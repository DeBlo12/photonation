// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use std::collections::VecDeque;
use std::fs::DirEntry;
use std::path::{Path, PathBuf};

/// Folders on removable and system disks that never hold the user's photos.
const SYSTEM_FOLDERS: [&str; 2] = ["$RECYCLE.BIN", "System Volume Information"];
/// Folders that are one document to the user (macOS shows them as files),
/// such as a photo library, whose insides are not theirs to browse.
const PACKAGE_EXTENSIONS: [&str; 6] = [
    "app",
    "aplibrary",
    "cocatalog",
    "lrdata",
    "lrlibrary",
    "photoslibrary",
];

/// A folder below the one opened that has raw files somewhere inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subfolder {
    pub path: PathBuf,
    /// Raw files inside it, at any depth.
    pub photo_count: usize,
    /// The photo that stands for it: the first by name, nearest the top.
    pub cover: PathBuf,
}

/// Raw files directly inside `folder`, sorted by name.
pub fn scan_folder(folder: &Path) -> std::io::Result<Vec<PathBuf>> {
    Ok(read_folder(folder)?.photos)
}

/// Calls `found` for each folder directly inside `folder` that has raw files
/// somewhere within, in name order. A folder that can't be read counts as
/// empty. `superseded` is asked before each folder read; once it says yes
/// the walk stops and this returns false, else true when the walk is done.
pub(crate) fn scan_subfolders(
    folder: &Path,
    mut superseded: impl FnMut() -> bool,
    mut found: impl FnMut(Subfolder),
) -> bool {
    let Ok(contents) = read_folder(folder) else {
        return true;
    };
    for subfolder in contents.subfolders {
        match summarize_subfolder(subfolder, &mut superseded) {
            Walked::Superseded => return false,
            Walked::Summary(Some(summary)) => found(summary),
            Walked::Summary(None) => {}
        }
    }
    true
}

enum Walked {
    Superseded,
    /// `None` when it has no raw files.
    Summary(Option<Subfolder>),
}

/// Walks a folder level by level, so its cover is the photo nearest the top.
fn summarize_subfolder(path: PathBuf, superseded: &mut impl FnMut() -> bool) -> Walked {
    let mut waiting = VecDeque::from([path.clone()]);
    let mut photo_count = 0;
    let mut cover = None;
    while let Some(folder) = waiting.pop_front() {
        if superseded() {
            return Walked::Superseded;
        }
        let Ok(contents) = read_folder(&folder) else {
            continue;
        };
        photo_count += contents.photos.len();
        if cover.is_none() {
            cover = contents.photos.into_iter().next();
        }
        waiting.extend(contents.subfolders);
    }
    Walked::Summary(cover.map(|cover| Subfolder {
        path,
        photo_count,
        cover,
    }))
}

struct FolderContents {
    photos: Vec<PathBuf>,
    subfolders: Vec<PathBuf>,
}

/// What is directly inside `folder`, both lists sorted by name.
fn read_folder(folder: &Path) -> std::io::Result<FolderContents> {
    let mut contents = FolderContents {
        photos: Vec::new(),
        subfolders: Vec::new(),
    };
    for entry in std::fs::read_dir(folder)?.flatten() {
        let path = entry.path();
        if is_browsable_folder(&entry) {
            contents.subfolders.push(path);
        } else if path.is_file() && photonation_raw::is_supported(&path) {
            contents.photos.push(path);
        }
    }
    contents.photos.sort();
    contents.subfolders.sort();
    Ok(contents)
}

/// A real folder worth looking in. Symbolic links don't count, because one
/// pointing back up would make the walk endless; a linked raw file still
/// counts as a photo.
fn is_browsable_folder(entry: &DirEntry) -> bool {
    entry.file_type().is_ok_and(|kind| kind.is_dir()) && is_browsable_name(&entry.path())
}

fn is_browsable_name(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let is_package = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            let extension = extension.to_ascii_lowercase();
            PACKAGE_EXTENSIONS.contains(&extension.as_str())
        });
    !name.starts_with('.') && !SYSTEM_FOLDERS.contains(&name.as_ref()) && !is_package
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// A folder with these files; their parent folders are made as needed.
    fn folder_with(files: &[&str]) -> TempDir {
        let root = tempfile::tempdir().expect("temporary folder");
        for file in files {
            let path = root.path().join(file);
            fs::create_dir_all(path.parent().expect("parent")).expect("create folders");
            fs::write(path, b"not a raw").expect("write file");
        }
        root
    }

    fn subfolders_of(root: &Path) -> Vec<Subfolder> {
        let mut subfolders = Vec::new();
        let finished = scan_subfolders(root, || false, |found| subfolders.push(found));
        assert!(finished);
        subfolders
    }

    #[test]
    fn scanning_a_folder_lists_only_its_own_raw_files() {
        let root = folder_with(&["b.CR3", "a.dng", "notes.txt", "month/c.CR3"]);
        let photos = scan_folder(root.path()).expect("scan");
        assert_eq!(
            photos,
            [root.path().join("a.dng"), root.path().join("b.CR3")]
        );
    }

    #[test]
    fn subfolders_count_raw_files_at_any_depth_in_name_order() {
        let root = folder_with(&[
            "2026_07/a.CR3",
            "2026_06/day1/b.CR3",
            "2026_06/day2/c.CR3",
            "2026_06/d.CR3",
        ]);
        assert_eq!(
            subfolders_of(root.path()),
            [
                Subfolder {
                    path: root.path().join("2026_06"),
                    photo_count: 3,
                    // Nearest the top, though b.CR3 sorts first.
                    cover: root.path().join("2026_06/d.CR3"),
                },
                Subfolder {
                    path: root.path().join("2026_07"),
                    photo_count: 1,
                    cover: root.path().join("2026_07/a.CR3"),
                },
            ]
        );
    }

    #[test]
    fn folders_without_raw_files_are_left_out() {
        let root = folder_with(&["exports/a.jpg", "empty/deeper/notes.txt"]);
        assert_eq!(subfolders_of(root.path()), []);
    }

    #[test]
    fn hidden_system_and_package_folders_are_not_browsed() {
        let root = folder_with(&[
            ".hidden/a.CR3",
            "$RECYCLE.BIN/b.CR3",
            "Photos Library.photoslibrary/originals/c.CR3",
            "Catalog.LRDATA/d.CR3",
            "kept/hidden/e.CR3",
            "kept/.cache/f.CR3",
        ]);
        let subfolders = subfolders_of(root.path());
        assert_eq!(subfolders.len(), 1);
        assert_eq!(subfolders[0].path, root.path().join("kept"));
        assert_eq!(subfolders[0].photo_count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_back_up_does_not_make_the_walk_endless() {
        let root = folder_with(&["month/a.CR3"]);
        std::os::unix::fs::symlink(root.path(), root.path().join("month/back-up"))
            .expect("link folder");
        std::os::unix::fs::symlink(root.path().join("month"), root.path().join("linked"))
            .expect("link folder");
        let subfolders = subfolders_of(root.path());
        assert_eq!(subfolders.len(), 1);
        assert_eq!(subfolders[0].photo_count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_raw_file_counts_as_a_photo() {
        let root = folder_with(&["originals.txt", "month/placeholder.txt"]);
        let original = folder_with(&["a.CR3"]);
        std::os::unix::fs::symlink(
            original.path().join("a.CR3"),
            root.path().join("month/a.CR3"),
        )
        .expect("link file");
        assert_eq!(subfolders_of(root.path())[0].photo_count, 1);
    }

    #[test]
    fn a_missing_folder_has_no_subfolders() {
        let root = folder_with(&[]);
        assert_eq!(subfolders_of(&root.path().join("gone")), []);
    }

    #[test]
    fn a_superseded_walk_stops_and_says_so() {
        let root = folder_with(&["a/1.CR3", "b/2.CR3", "c/3.CR3"]);
        let mut reads = 0;
        let mut seen = Vec::new();
        let finished = scan_subfolders(
            root.path(),
            || {
                reads += 1;
                reads > 1
            },
            |found| seen.push(found.path),
        );
        assert!(!finished);
        assert_eq!(seen, [root.path().join("a")]);
    }
}
