// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Help ▸ Check for Updates…: asks GitHub for the latest release and, on a
//! click, downloads its installer, verifies it and puts it in place. Nothing
//! here runs until the user asks, and the requests run on a thread of their
//! own so the window stays live. Control flow is in docs/DESKTOP.md.

mod install;
mod release;
mod sha256;
mod transfer;

pub use install::{Finish, Method};
pub use release::{Installer, Release, Version};

use eframe::egui;
use release::Platform;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use transfer::Answer;

/// What asking for the latest release found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// The running version is the latest one, or newer.
    UpToDate,
    /// The project has not published a release.
    NoRelease,
    Available(Release),
}

/// Where an installation has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Downloading,
    Verifying,
    Installing,
}

/// An installation that stopped. A download that passed verification is
/// kept, so it can still be installed by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallFailure {
    pub message: String,
    pub verified_installer: Option<PathBuf>,
}

impl From<String> for InstallFailure {
    fn from(message: String) -> InstallFailure {
        InstallFailure {
            message,
            verified_installer: None,
        }
    }
}

/// What the window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Nothing has been asked yet.
    Idle,
    Checking,
    Checked(Check),
    CheckFailed(String),
    Installing {
        release: Release,
        stage: Stage,
        /// The file the installer downloads to; its size is the progress.
        download: PathBuf,
    },
    /// Everything but the restart is done.
    Ready {
        release: Release,
        finish: Finish,
    },
    InstallFailed {
        release: Release,
        failure: InstallFailure,
    },
}

/// What the thread doing the requests reports back.
enum Event {
    Checked(Result<Check, String>),
    Reached(Stage),
    Installed(Result<Finish, InstallFailure>),
}

/// The update check and installation, and where they have got to.
pub struct Updater {
    state: State,
    method: Method,
    sender: Sender<Event>,
    events: Receiver<Event>,
    /// Holds the download; deleted with this, or when the next one starts.
    work_folder: Option<tempfile::TempDir>,
}

impl Default for Updater {
    fn default() -> Updater {
        let (sender, events) = channel();
        Updater {
            state: State::Idle,
            method: Method::current(),
            sender,
            events,
            work_folder: None,
        }
    }
}

impl Updater {
    pub fn state(&self) -> &State {
        &self.state
    }

    /// How this copy of the app can take an update.
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// A request or an installation is under way, or one is waiting for the
    /// restart: asking again would throw it away.
    fn is_occupied(&self) -> bool {
        matches!(
            self.state,
            State::Checking | State::Installing { .. } | State::Ready { .. }
        )
    }

    /// Asks GitHub for the latest release, unless something is under way.
    pub fn check(&mut self, ctx: &egui::Context) {
        if self.is_occupied() {
            return;
        }
        self.state = State::Checking;
        self.in_background(ctx, |_| {
            Event::Checked(
                latest_release().and_then(|answer| {
                    check_from(&answer, Version::running(), Platform::current())
                }),
            )
        });
    }

    /// Downloads, verifies and installs the release that was found.
    pub fn install(&mut self, ctx: &egui::Context) {
        let release = match &self.state {
            State::Checked(Check::Available(release)) => release.clone(),
            State::InstallFailed { release, .. } => release.clone(),
            _ => return,
        };
        let Some(installer) = release.installer.clone() else {
            return;
        };
        let method = self.method.clone();
        if method == Method::Unavailable {
            return;
        }
        // The folder of an earlier attempt goes first.
        self.work_folder = None;
        let work_folder = match tempfile::Builder::new()
            .prefix("photonation-update-")
            .tempdir()
        {
            Ok(work_folder) => work_folder,
            Err(error) => {
                let message = format!("Couldn't make a folder for the download: {error}");
                self.state = State::InstallFailed {
                    release,
                    failure: message.into(),
                };
                return;
            }
        };
        let work_path = work_folder.path().to_owned();
        let download = work_path.join(&installer.file_name);
        self.work_folder = Some(work_folder);
        self.state = State::Installing {
            release,
            stage: Stage::Downloading,
            download: download.clone(),
        };
        self.in_background(ctx, move |report| {
            Event::Installed(download_and_install(
                &installer, &download, &work_path, &method, report,
            ))
        });
    }

    /// Runs `work` on a thread of its own. Whatever it reports, and what it
    /// ends with, reaches [`Updater::poll`] and wakes the window.
    fn in_background(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce(&dyn Fn(Stage)) -> Event + Send + 'static,
    ) {
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("updates".into())
            .spawn(move || {
                let send = |event| {
                    sender.send(event).ok();
                    ctx.request_repaint();
                };
                let last = work(&|stage| send(Event::Reached(stage)));
                send(last);
            });
        if let Err(error) = spawned {
            let message = format!("Couldn't start the update: {error}");
            self.state = match std::mem::replace(&mut self.state, State::Idle) {
                State::Installing { release, .. } => State::InstallFailed {
                    release,
                    failure: message.into(),
                },
                _ => State::CheckFailed(message),
            };
        }
    }

    /// Takes in what the background thread has reported since the last call.
    pub fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.state = next_state(std::mem::replace(&mut self.state, State::Idle), event);
        }
    }

    /// How much of the installer has arrived, 0..=1, while it downloads.
    pub fn download_progress(&self) -> Option<f32> {
        let State::Installing {
            release,
            stage: Stage::Downloading,
            download,
        } = &self.state
        else {
            return None;
        };
        let expected = release.installer.as_ref()?.bytes;
        let arrived = std::fs::metadata(download).map_or(0, |metadata| metadata.len());
        Some((arrived as f64 / expected.max(1) as f64).min(1.0) as f32)
    }

    /// Starts the last step (the new app, or the setup program). The caller
    /// then quits the app. Does nothing unless an update is ready.
    pub fn finish(&mut self) -> Result<(), String> {
        let State::Ready { finish, .. } = &self.state else {
            return Ok(());
        };
        if matches!(finish, Finish::RunSetup { .. })
            && let Some(work_folder) = self.work_folder.take()
        {
            // The setup program runs from it after this app has quit.
            let _kept = work_folder.keep();
        }
        finish
            .start()
            .map_err(|error| format!("Couldn't start the new version: {error}"))
    }

    /// Shows the verified download of a failed installation in the system's
    /// file manager, to install by hand.
    pub fn show_verified_installer(&self) -> Result<(), String> {
        let State::InstallFailed { failure, .. } = &self.state else {
            return Ok(());
        };
        match &failure.verified_installer {
            Some(installer) => install::show_installer(installer)
                .map_err(|error| format!("Couldn't open the download: {error}")),
            None => Ok(()),
        }
    }

    /// Puts the updater in `state`, for drawing the window in tests.
    #[cfg(test)]
    pub fn in_state(state: State, method: Method) -> Updater {
        Updater {
            state,
            method,
            ..Default::default()
        }
    }
}

/// The state after `event`. Reports that don't belong to what is under way
/// (there are none while one thread runs at a time) change nothing.
fn next_state(state: State, event: Event) -> State {
    match (state, event) {
        (State::Checking, Event::Checked(Ok(check))) => State::Checked(check),
        (State::Checking, Event::Checked(Err(message))) => State::CheckFailed(message),
        (
            State::Installing {
                release, download, ..
            },
            Event::Reached(stage),
        ) => State::Installing {
            release,
            stage,
            download,
        },
        (State::Installing { release, .. }, Event::Installed(Ok(finish))) => {
            State::Ready { release, finish }
        }
        (State::Installing { release, .. }, Event::Installed(Err(failure))) => {
            State::InstallFailed { release, failure }
        }
        (state, _) => state,
    }
}

fn latest_release() -> Result<Answer, String> {
    let url = release::latest_release_api_url()
        .ok_or("This build's repository isn't on GitHub, so there is nowhere to ask.")?;
    transfer::get(&url).map_err(|error| format!("Couldn't reach GitHub: {error}"))
}

/// What GitHub's answer means for the copy that is `running`.
fn check_from(answer: &Answer, running: Version, platform: Platform) -> Result<Check, String> {
    match answer.status {
        200 => {
            let release = Release::from_json(&answer.body, platform)?;
            Ok(if release.version > running {
                Check::Available(release)
            } else {
                Check::UpToDate
            })
        }
        // What GitHub says while a repository has no published release.
        404 => Ok(Check::NoRelease),
        403 | 429 => Err(
            "GitHub is limiting requests from this network at the moment. Try again later.".into(),
        ),
        status => Err(format!("GitHub answered with status {status}.")),
    }
}

/// The work of an installation, off the window's thread: download, verify,
/// and on macOS replace the app. `report` says which stage begins.
fn download_and_install(
    installer: &Installer,
    download: &Path,
    work_folder: &Path,
    method: &Method,
    report: &dyn Fn(Stage),
) -> Result<Finish, InstallFailure> {
    transfer::download(&installer.url, download, installer.bytes)
        .map_err(|error| format!("The download didn't finish: {error}"))?;
    report(Stage::Verifying);
    verify(download, installer)?;
    match method {
        Method::ReplaceBundle(bundle) => {
            report(Stage::Installing);
            install::replace_bundle(download, bundle, work_folder).map_err(|message| {
                InstallFailure {
                    message,
                    verified_installer: Some(download.to_owned()),
                }
            })?;
            Ok(Finish::Relaunch {
                bundle: bundle.clone(),
            })
        }
        Method::RunSetup => Ok(Finish::RunSetup {
            setup: download.to_owned(),
        }),
        Method::Unavailable => Err(String::from("This copy can't install updates.").into()),
    }
}

/// Holds the download to the size and checksum GitHub recorded for the
/// file. A download that differs is deleted.
fn verify(download: &Path, installer: &Installer) -> Result<(), String> {
    let matches = std::fs::metadata(download)
        .map(|metadata| metadata.len() == installer.bytes)
        .and_then(|right_size| Ok(right_size && sha256::of_file(download)? == installer.sha256))
        .map_err(|error| format!("Couldn't read the download: {error}"))?;
    if matches {
        Ok(())
    } else {
        std::fs::remove_file(download).ok();
        Err(
            "The download doesn't match the checksum GitHub has for it, so it was not installed."
                .into(),
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use release::tests::answer;

    const MAC: Platform = Platform::MacOs {
        architecture: "arm64",
    };

    fn version(text: &str) -> Version {
        Version::parse(text).expect(text)
    }

    fn answered(status: u16, body: String) -> Answer {
        Answer { status, body }
    }

    /// A release newer than any, with an installer for an Apple Silicon Mac.
    pub fn a_release() -> Release {
        Release::from_json(&answer("99.0.0"), MAC).expect("a release")
    }

    /// The real question, to the real GitHub. Not run by default: it needs
    /// the network. `cargo test -p photonation-desktop -- --ignored github`.
    #[test]
    #[ignore = "asks GitHub over the network"]
    fn github_answers_with_something_understood() {
        let answer = latest_release().expect("an answer");
        let check = check_from(&answer, Version::running(), Platform::current());
        println!("status {}: {check:?}", answer.status);
        assert!(check.is_ok(), "{check:?}");
    }

    #[test]
    fn only_a_newer_release_is_an_update() {
        let latest = answered(200, answer("0.30.0"));
        let check = |running| check_from(&latest, version(running), MAC).expect("a check");
        assert!(matches!(check("0.29.5"), Check::Available(release)
            if release.version == version("0.30.0")));
        assert_eq!(check("0.30.0"), Check::UpToDate);
        // A build from source ahead of the last release.
        assert_eq!(check("0.31.0"), Check::UpToDate);
    }

    #[test]
    fn no_published_release_is_not_an_error() {
        let nothing = answered(404, r#"{"message": "Not Found"}"#.into());
        assert_eq!(
            check_from(&nothing, version("0.1.0"), MAC),
            Ok(Check::NoRelease)
        );
    }

    #[test]
    fn other_answers_are_errors_that_say_what_happened() {
        for status in [403, 429] {
            let error = check_from(&answered(status, String::new()), version("0.1.0"), MAC)
                .expect_err("limited");
            assert!(error.contains("limiting"), "{error}");
        }
        let error =
            check_from(&answered(500, String::new()), version("0.1.0"), MAC).expect_err("broken");
        assert!(error.contains("500"), "{error}");
        assert!(check_from(&answered(200, "<html>".into()), version("0.1.0"), MAC).is_err());
    }

    #[test]
    fn a_download_is_held_to_its_size_and_checksum() {
        let folder = tempfile::tempdir().expect("a folder");
        let download = folder.path().join("installer");
        let installer = |bytes, sha256: &str| Installer {
            file_name: "installer".into(),
            url: String::new(),
            bytes,
            sha256: sha256.into(),
        };
        // SHA-256 of "abc".
        let checksum = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

        std::fs::write(&download, "abc").expect("written");
        assert_eq!(verify(&download, &installer(3, checksum)), Ok(()));
        assert!(download.exists());

        // The right size with other contents, and the right start with more.
        for (contents, bytes) in [("abd", 3), ("abcd", 3), ("abc", 4)] {
            std::fs::write(&download, contents).expect("written");
            let refused = verify(&download, &installer(bytes, checksum));
            assert!(refused.is_err(), "{contents:?} as {bytes} bytes");
            assert!(!download.exists(), "a download that differs is deleted");
        }
        assert!(verify(&download, &installer(3, checksum)).is_err());
    }

    #[test]
    fn reports_move_an_installation_along() {
        let release = a_release();
        let finish = Finish::Relaunch {
            bundle: "/Applications/Photonation.app".into(),
        };
        let installing = |stage| State::Installing {
            release: release.clone(),
            stage,
            download: "download".into(),
        };
        assert_eq!(
            next_state(
                installing(Stage::Downloading),
                Event::Reached(Stage::Verifying)
            ),
            installing(Stage::Verifying)
        );
        assert_eq!(
            next_state(
                installing(Stage::Installing),
                Event::Installed(Ok(finish.clone()))
            ),
            State::Ready {
                release: release.clone(),
                finish
            }
        );
        let failure = InstallFailure::from(String::from("no"));
        assert_eq!(
            next_state(
                installing(Stage::Downloading),
                Event::Installed(Err(failure.clone()))
            ),
            State::InstallFailed {
                release: release.clone(),
                failure
            }
        );
        assert_eq!(
            next_state(State::Checking, Event::Checked(Ok(Check::UpToDate))),
            State::Checked(Check::UpToDate)
        );
        assert_eq!(
            next_state(State::Checking, Event::Checked(Err("offline".into()))),
            State::CheckFailed("offline".into())
        );
        // A report that belongs to nothing under way changes nothing.
        assert_eq!(
            next_state(State::Idle, Event::Reached(Stage::Verifying)),
            State::Idle
        );
    }

    #[test]
    fn nothing_starts_over_what_is_under_way() {
        let ctx = egui::Context::default();
        let release = a_release();
        let ready = State::Ready {
            release: release.clone(),
            finish: Finish::Relaunch {
                bundle: "/nowhere/Photonation.app".into(),
            },
        };
        let mut updater = Updater::in_state(ready.clone(), Method::Unavailable);
        updater.check(&ctx);
        updater.install(&ctx);
        assert_eq!(updater.state(), &ready);

        // A copy that can't take updates never downloads one.
        let available = State::Checked(Check::Available(release));
        let mut updater = Updater::in_state(available.clone(), Method::Unavailable);
        updater.install(&ctx);
        assert_eq!(updater.state(), &available);
        assert_eq!(updater.download_progress(), None);
    }

    #[test]
    fn progress_is_the_share_of_the_installer_that_has_arrived() {
        let folder = tempfile::tempdir().expect("a folder");
        let download = folder.path().join("installer");
        let mut release = a_release();
        release.installer.as_mut().expect("an installer").bytes = 400;
        let installing = |stage| State::Installing {
            release: release.clone(),
            stage,
            download: download.clone(),
        };
        let progress =
            |stage| Updater::in_state(installing(stage), Method::Unavailable).download_progress();
        assert_eq!(progress(Stage::Downloading), Some(0.0));
        std::fs::write(&download, vec![0u8; 100]).expect("written");
        assert_eq!(progress(Stage::Downloading), Some(0.25));
        std::fs::write(&download, vec![0u8; 500]).expect("written");
        assert_eq!(progress(Stage::Downloading), Some(1.0));
        assert_eq!(progress(Stage::Verifying), None);
    }
}
