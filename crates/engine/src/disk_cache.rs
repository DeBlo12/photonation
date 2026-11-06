// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! What the disk caches (thumbnails, removal fills) share: files written
//! whole, and the least recently used ones going first. Failures only cost
//! recomputing later, so nothing here reports them.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Writes `entry` beside itself and renames it, so a reader never sees half a
/// file, creating its folder if needed.
pub(crate) fn write_whole(entry: &Path, bytes: &[u8]) {
    let Some(directory) = entry.parent() else {
        return;
    };
    if std::fs::create_dir_all(directory).is_err() {
        return;
    }
    let partial = entry.with_extension(format!("{:?}.partial", std::thread::current().id()));
    if std::fs::write(&partial, bytes).is_ok() && std::fs::rename(&partial, entry).is_err() {
        std::fs::remove_file(&partial).ok();
    }
}

/// Marks a file as just used, so trimming keeps it longer.
pub(crate) fn mark_used(entry: &Path) {
    if let Ok(file) = std::fs::File::options().append(true).open(entry) {
        file.set_modified(SystemTime::now()).ok();
    }
}

/// Deletes the least recently used files of `directory` while it holds more
/// than `max_bytes`, down to `trimmed_bytes`.
pub(crate) fn trim_directory(directory: &Path, max_bytes: u64, trimmed_bytes: u64) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(|| {
                let used = metadata.modified().unwrap_or(UNIX_EPOCH);
                (used, metadata.len(), entry.path())
            })
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    if total <= max_bytes {
        return;
    }
    files.sort();
    for (_, size, path) in files {
        if total <= trimmed_bytes {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total -= size;
        }
    }
}
