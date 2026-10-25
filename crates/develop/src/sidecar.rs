// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::Settings;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SIDECAR_EXTENSION: &str = "photonation.json";
const FORMAT: &str = "photonation.develop";

#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("{path}: not a Photonation sidecar (format {found:?})")]
    Format { path: PathBuf, found: String },
}

#[derive(Serialize, Deserialize)]
struct Sidecar {
    format: String,
    settings: Settings,
}

/// `IMG_0001.CR3` → `IMG_0001.CR3.photonation.json`.
pub fn sidecar_path(original: &Path) -> PathBuf {
    let mut name = original.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(SIDECAR_EXTENSION);
    original.with_file_name(name)
}

impl Settings {
    /// Reads settings from a sidecar file; `Ok(None)` when it doesn't exist.
    pub fn load(path: &Path) -> Result<Option<Settings>, SidecarError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(SidecarError::Io {
                    path: path.into(),
                    source,
                });
            }
        };
        let sidecar: Sidecar =
            serde_json::from_str(&text).map_err(|source| SidecarError::Parse {
                path: path.into(),
                source,
            })?;
        if sidecar.format != FORMAT {
            return Err(SidecarError::Format {
                path: path.into(),
                found: sidecar.format,
            });
        }
        Ok(Some(sidecar.settings))
    }

    /// Writes settings atomically (temp file + rename) so a crash never
    /// leaves a truncated sidecar.
    pub fn save(&self, path: &Path) -> Result<(), SidecarError> {
        let sidecar = Sidecar {
            format: FORMAT.into(),
            settings: self.clone(),
        };
        let mut text =
            serde_json::to_string_pretty(&sidecar).map_err(|source| SidecarError::Parse {
                path: path.into(),
                source,
            })?;
        text.push('\n');
        let tmp = path.with_extension("json.tmp");
        let io = |source| SidecarError::Io {
            path: path.into(),
            source,
        };
        std::fs::write(&tmp, text).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_appends_extension() {
        assert_eq!(
            sidecar_path(Path::new("/a/IMG_1.CR3")),
            PathBuf::from("/a/IMG_1.CR3.photonation.json")
        );
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("photonation-sidecar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("x.photonation.json");
        let s = Settings {
            exposure: 1.25,
            ..Settings::default()
        };
        s.save(&path).expect("save");
        assert_eq!(Settings::load(&path).expect("load"), Some(s));
        assert_eq!(
            Settings::load(&dir.join("missing.json")).expect("load"),
            None
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
