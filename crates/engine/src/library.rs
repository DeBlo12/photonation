// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use std::path::{Path, PathBuf};

/// Raw files directly inside `folder`, sorted by name.
pub fn scan_folder(folder: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut photos: Vec<PathBuf> = std::fs::read_dir(folder)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_file() && photonation_raw::is_supported(path))
        .collect();
    photos.sort();
    Ok(photos)
}
