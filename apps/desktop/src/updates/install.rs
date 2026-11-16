// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Putting a downloaded, verified installer in place. On macOS the app
//! bundle that is running is replaced with the one in the disk image; on
//! Windows the setup program runs, which upgrades an installation in place.
//! See docs/DESKTOP.md.

use crate::updates::release::Platform;
use crate::updates::transfer::hide_console_window;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The app as the disk image holds it (packaging/macos/build-installer.sh).
const BUNDLE_IN_DISK_IMAGE: &str = "Photonation.app";
/// `CFBundleExecutable` in packaging/macos/Info.plist.
const BUNDLE_EXECUTABLE: &str = "Contents/MacOS/photonation-desktop";
const BUNDLE_INFORMATION: &str = "Contents/Info.plist";
/// What Inno Setup leaves beside an installed program.
const WINDOWS_UNINSTALLER: &str = "unins000.exe";

/// How this copy of the app can take an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    /// It runs from this app bundle, which the new one replaces.
    ReplaceBundle(PathBuf),
    /// The setup program installed it and can install over it.
    RunSetup,
    /// It was built from source, or runs from somewhere no installer put it.
    Unavailable,
}

impl Method {
    pub fn current() -> Method {
        match std::env::current_exe() {
            Ok(executable) => Method::for_executable(Platform::current(), &executable),
            Err(_) => Method::Unavailable,
        }
    }

    fn for_executable(platform: Platform, executable: &Path) -> Method {
        match platform {
            Platform::MacOs { .. } => {
                bundle_containing(executable).map_or(Method::Unavailable, Method::ReplaceBundle)
            }
            Platform::Windows => {
                let installed = executable
                    .parent()
                    .is_some_and(|folder| folder.join(WINDOWS_UNINSTALLER).is_file());
                if installed {
                    Method::RunSetup
                } else {
                    Method::Unavailable
                }
            }
            Platform::Other => Method::Unavailable,
        }
    }
}

/// The `….app` whose `Contents/MacOS` holds `executable`.
fn bundle_containing(executable: &Path) -> Option<PathBuf> {
    let executables = executable.parent()?;
    let contents = executables.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = executables.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_owned())
}

/// What is left to do once the installer is ready, when the user says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finish {
    /// The new bundle is in place: start it once this copy has quit.
    Relaunch { bundle: PathBuf },
    /// Start the setup program and quit, so it can replace the running one.
    RunSetup { setup: PathBuf },
}

impl Finish {
    /// Starts what comes after this copy; the caller then quits.
    pub fn start(&self) -> io::Result<()> {
        match self {
            Finish::Relaunch { bundle } => relaunch_once_quit(bundle),
            Finish::RunSetup { setup } => Command::new(setup).spawn().map(drop),
        }
    }
}

/// Waits for this process to end, then opens `bundle`. Opening it sooner
/// would only bring the copy that is still running to the front. The path
/// reaches the shell as an argument, never as part of the script.
fn relaunch_once_quit(bundle: &Path) -> io::Result<()> {
    const SCRIPT: &str = "tries=0\n\
        while kill -0 \"$1\" 2>/dev/null && [ \"$tries\" -lt 150 ]; do\n\
            tries=$((tries + 1)); /bin/sleep 0.2\n\
        done\n\
        exec /usr/bin/open \"$2\"";
    Command::new("/bin/sh")
        .args(["-c", SCRIPT, "photonation-relaunch"])
        .arg(std::process::id().to_string())
        .arg(bundle)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

/// Shows a downloaded installer to the user, to install by hand: macOS
/// opens the disk image, Windows shows the setup program in its folder.
pub fn show_installer(installer: &Path) -> io::Result<()> {
    let mut command = if cfg!(windows) {
        let mut explorer = Command::new("explorer");
        explorer.arg("/select,").arg(installer);
        explorer
    } else {
        let mut open = Command::new("/usr/bin/open");
        open.arg(installer);
        open
    };
    command.spawn().map(drop)
}

/// Runs a system tool to its end; its own words are the error.
fn run(program: &str, configure: impl FnOnce(&mut Command)) -> Result<(), String> {
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    hide_console_window(&mut command);
    configure(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("Couldn't start {program}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let said: String = said.trim().chars().take(300).collect();
    Err(if said.is_empty() {
        format!("{program} stopped with {}", output.status)
    } else {
        format!("{program}: {said}")
    })
}

/// Replaces `bundle` with the app in `disk_image`, which is mounted out of
/// sight in `work_folder` for as long as that takes.
pub fn replace_bundle(disk_image: &Path, bundle: &Path, work_folder: &Path) -> Result<(), String> {
    let mount_point = work_folder.join("mounted");
    fs::create_dir_all(&mount_point)
        .map_err(|error| format!("Couldn't make a folder to open the disk image in: {error}"))?;
    run("/usr/bin/hdiutil", |command| {
        command
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&mount_point)
            .arg(disk_image);
    })?;
    let replaced = replace_bundle_with(&mount_point.join(BUNDLE_IN_DISK_IMAGE), bundle);
    // Left mounted, the image only stays until the next restart.
    run("/usr/bin/hdiutil", |command| {
        command.args(["detach", "-force"]).arg(&mount_point);
    })
    .ok();
    replaced
}

/// Copies the app at `source` next to `bundle`, then swaps the two, so that
/// `bundle` is never half of each.
fn replace_bundle_with(source: &Path, bundle: &Path) -> Result<(), String> {
    if !is_app_bundle(source) {
        return Err("The disk image doesn't hold Photonation.app.".into());
    }
    let staged = beside(bundle, "update").ok_or("The app has no folder around it.")?;
    remove_leftover(&staged).map_err(|error| could_not_replace(bundle, &error))?;
    let copied = run("/usr/bin/ditto", |command| {
        command.arg(source).arg(&staged);
    })
    .and_then(|()| {
        if is_app_bundle(&staged) {
            swap_in(&staged, bundle).map_err(|error| could_not_replace(bundle, &error))
        } else {
            Err("The copy of the new app is incomplete.".into())
        }
    });
    if copied.is_err() {
        remove_leftover(&staged).ok();
    }
    copied
}

fn could_not_replace(bundle: &Path, error: &io::Error) -> String {
    format!(
        "Couldn't replace {}: {error}. If macOS asked whether Photonation may update apps, \
         allow it under System Settings ▸ Privacy & Security ▸ App Management and try again, \
         or install from the disk image by hand.",
        bundle.display()
    )
}

/// A folder that has the two files every Photonation.app has, and is not a
/// link to somewhere else.
fn is_app_bundle(path: &Path) -> bool {
    let is_real_folder = fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir());
    is_real_folder
        && [BUNDLE_INFORMATION, BUNDLE_EXECUTABLE]
            .iter()
            .all(|file| path.join(file).is_file())
}

/// A hidden neighbour of `bundle`, on the same disk, so that moving between
/// the two is a rename: `/Applications/.Photonation.app.update`.
fn beside(bundle: &Path, purpose: &str) -> Option<PathBuf> {
    let name = bundle.file_name()?.to_string_lossy();
    Some(bundle.parent()?.join(format!(".{name}.{purpose}")))
}

/// Removes what an update that didn't finish left at `path`.
fn remove_leftover(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Moves `bundle` aside, `staged` into its place, and deletes the old one.
/// If the new one can't move in, the old one moves back.
fn swap_in(staged: &Path, bundle: &Path) -> io::Result<()> {
    let previous = beside(bundle, "previous")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no folder around the app"))?;
    remove_leftover(&previous)?;
    fs::rename(bundle, &previous)?;
    if let Err(error) = fs::rename(staged, bundle) {
        fs::rename(&previous, bundle).ok();
        return Err(error);
    }
    // The update is in place either way; a leftover goes with the next one.
    remove_leftover(&previous).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: Platform = Platform::MacOs {
        architecture: "arm64",
    };

    /// A folder with the files of an app bundle, and `marker` to tell it by.
    fn bundle_at(path: &Path, marker: &str) {
        fs::create_dir_all(path.join("Contents/MacOS")).expect("folders");
        fs::write(path.join(BUNDLE_INFORMATION), "<plist/>").expect("Info.plist");
        fs::write(path.join(BUNDLE_EXECUTABLE), marker).expect("executable");
    }

    fn marker_of(bundle: &Path) -> String {
        fs::read_to_string(bundle.join(BUNDLE_EXECUTABLE)).expect("executable")
    }

    fn names_in(folder: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(folder)
            .expect("the folder")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_app_bundle_is_found_from_its_executable() {
        assert_eq!(
            bundle_containing(Path::new(
                "/Applications/Photonation.app/Contents/MacOS/photonation-desktop"
            )),
            Some(PathBuf::from("/Applications/Photonation.app"))
        );
        for elsewhere in [
            "/Users/someone/photonation/target/release/photonation-desktop",
            "/Applications/Photonation.app/Contents/Resources/photonation-desktop",
            "/Applications/Photonation/Contents/MacOS/photonation-desktop",
            "photonation-desktop",
        ] {
            assert_eq!(bundle_containing(Path::new(elsewhere)), None, "{elsewhere}");
        }
    }

    #[test]
    fn only_an_installed_copy_can_take_an_update() {
        let installed =
            Path::new("/Applications/Photonation.app/Contents/MacOS/photonation-desktop");
        assert_eq!(
            Method::for_executable(MAC, installed),
            Method::ReplaceBundle("/Applications/Photonation.app".into())
        );
        let built = Path::new("/src/photonation/target/release/photonation-desktop");
        assert_eq!(Method::for_executable(MAC, built), Method::Unavailable);
        assert_eq!(
            Method::for_executable(Platform::Other, installed),
            Method::Unavailable
        );

        let folder = tempfile::tempdir().expect("a folder");
        let program = folder.path().join("photonation-desktop.exe");
        assert_eq!(
            Method::for_executable(Platform::Windows, &program),
            Method::Unavailable
        );
        fs::write(folder.path().join(WINDOWS_UNINSTALLER), "").expect("the uninstaller");
        assert_eq!(
            Method::for_executable(Platform::Windows, &program),
            Method::RunSetup
        );
    }

    #[test]
    fn the_new_app_takes_the_old_ones_place_and_nothing_is_left_over() {
        let folder = tempfile::tempdir().expect("a folder");
        let bundle = folder.path().join("Photonation.app");
        bundle_at(&bundle, "old");
        fs::write(bundle.join("Contents/only-in-the-old-one"), "").expect("a file");
        let staged = beside(&bundle, "update").expect("a neighbour");
        bundle_at(&staged, "new");
        // What an update that was interrupted left behind.
        bundle_at(&beside(&bundle, "previous").expect("a neighbour"), "older");

        swap_in(&staged, &bundle).expect("swapped");

        assert_eq!(marker_of(&bundle), "new");
        assert!(!bundle.join("Contents/only-in-the-old-one").exists());
        assert_eq!(names_in(folder.path()), ["Photonation.app"]);
    }

    #[test]
    fn the_old_app_stays_when_there_is_no_new_one_to_move_in() {
        let folder = tempfile::tempdir().expect("a folder");
        let bundle = folder.path().join("Photonation.app");
        bundle_at(&bundle, "old");
        let missing = beside(&bundle, "update").expect("a neighbour");

        assert!(swap_in(&missing, &bundle).is_err());

        assert_eq!(marker_of(&bundle), "old");
        assert_eq!(names_in(folder.path()), ["Photonation.app"]);
    }

    #[test]
    fn a_folder_that_is_not_the_app_is_not_installed() {
        let folder = tempfile::tempdir().expect("a folder");
        let bundle = folder.path().join("Photonation.app");
        bundle_at(&bundle, "old");
        let not_an_app = folder.path().join("source/Photonation.app");
        fs::create_dir_all(not_an_app.join("Contents")).expect("folders");

        assert!(replace_bundle_with(&not_an_app, &bundle).is_err());
        assert!(replace_bundle_with(&folder.path().join("missing"), &bundle).is_err());

        assert_eq!(marker_of(&bundle), "old");
        assert_eq!(names_in(folder.path()), ["Photonation.app", "source"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_an_app_is_not_installed() {
        let folder = tempfile::tempdir().expect("a folder");
        let real = folder.path().join("Real.app");
        bundle_at(&real, "real");
        let link = folder.path().join("Photonation.app");
        std::os::unix::fs::symlink(&real, &link).expect("a link");
        assert!(is_app_bundle(&real));
        assert!(!is_app_bundle(&link));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_app_is_copied_from_a_folder_and_swapped_in() {
        let folder = tempfile::tempdir().expect("a folder");
        let bundle = folder.path().join("installed/Photonation.app");
        bundle_at(&bundle, "old");
        let source = folder.path().join("mounted/Photonation.app");
        bundle_at(&source, "new");

        replace_bundle_with(&source, &bundle).expect("replaced");

        assert_eq!(marker_of(&bundle), "new");
        assert_eq!(marker_of(&source), "new");
        assert_eq!(
            names_in(&folder.path().join("installed")),
            ["Photonation.app"]
        );
    }

    /// The whole macOS install, from a real disk image. Not run by default:
    /// it mounts an image, which is slow and can't be done in every sandbox.
    /// `cargo test -p photonation-desktop -- --ignored disk_image`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "creates and mounts a disk image"]
    fn the_app_in_a_disk_image_replaces_the_installed_one() {
        let folder = tempfile::tempdir().expect("a folder");
        let contents = folder.path().join("contents");
        bundle_at(&contents.join(BUNDLE_IN_DISK_IMAGE), "new");
        let disk_image = folder.path().join("photonation.dmg");
        run("/usr/bin/hdiutil", |command| {
            command
                .args([
                    "create",
                    "-quiet",
                    "-volname",
                    "Photonation",
                    "-format",
                    "UDZO",
                ])
                .arg("-srcfolder")
                .arg(&contents)
                .arg(&disk_image);
        })
        .expect("a disk image");
        let bundle = folder.path().join("installed/Photonation.app");
        bundle_at(&bundle, "old");
        let work_folder = folder.path().join("work");
        fs::create_dir(&work_folder).expect("a folder");

        replace_bundle(&disk_image, &bundle, &work_folder).expect("replaced");

        assert_eq!(marker_of(&bundle), "new");
        assert_eq!(
            names_in(&folder.path().join("installed")),
            ["Photonation.app"]
        );
        // Detached again: nothing is mounted there any more.
        assert!(names_in(&work_folder.join("mounted")).is_empty());
    }
}
