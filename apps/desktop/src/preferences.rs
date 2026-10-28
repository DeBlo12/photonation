// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Choices remembered between launches, in `preferences.json` in the
//! engine's data directory. A missing or unreadable file means defaults.

use photonation_engine::{MetadataOptions, data_directory};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// The export sheet's metadata groups and the creator and copyright
    /// entered there.
    pub export_metadata: MetadataOptions,
}

impl Preferences {
    fn path() -> Option<PathBuf> {
        data_directory().map(|directory| directory.join("preferences.json"))
    }

    pub fn load() -> Preferences {
        Preferences::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Writes the preferences; failing only means they are not remembered.
    pub fn save(&self) {
        let Some(path) = Preferences::path() else {
            return;
        };
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
                std::fs::write(&path, text)
            });
        if let Err(e) = written {
            eprintln!("Photonation: could not save {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_or_partial_files_fall_back_to_defaults() {
        let partial: Preferences =
            serde_json::from_str(r#"{"export_metadata": {"creator": "Ada"}}"#).expect("parse");
        assert_eq!(partial.export_metadata.creator, "Ada");
        assert!(partial.export_metadata.camera);
        assert!(!partial.export_metadata.location);
        let empty: Preferences = serde_json::from_str("{}").expect("parse");
        assert_eq!(empty, Preferences::default());
    }
}
