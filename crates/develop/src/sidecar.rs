// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::Settings;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const SIDECAR_EXTENSION: &str = "photonation.json";
const FORMAT: &str = "photonation.develop";
/// Largest sidecar read. Real ones are a few kilobytes; the limit keeps a
/// corrupt or hostile file from exhausting memory when a folder opens.
pub const MAX_SIDECAR_BYTES: u64 = 4 * 1024 * 1024;

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
    #[error("{path}: sidecar is larger than {MAX_SIDECAR_BYTES} bytes")]
    TooLarge { path: PathBuf },
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
        let io = |source| SidecarError::Io {
            path: path.into(),
            source,
        };
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(io(source)),
        };
        // Read one byte past the limit to tell "at the limit" from "over it".
        let mut text = String::new();
        file.take(MAX_SIDECAR_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(io)?;
        if text.len() as u64 > MAX_SIDECAR_BYTES {
            return Err(SidecarError::TooLarge { path: path.into() });
        }
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

    /// Writes settings atomically so a crash never leaves a truncated
    /// sidecar: the text goes to a new temporary file beside it (created
    /// exclusively under a random name, so a file or symlink planted in a
    /// shared folder is never written through), is flushed to disk, then
    /// renamed over the sidecar.
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
        let io = |source| SidecarError::Io {
            path: path.into(),
            source,
        };
        let directory = path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::Builder::new();
        temporary.prefix(".photonation-").suffix(".tmp");
        // Same permissions a plain create would give (0o666 minus umask)
        // instead of tempfile's owner-only default.
        #[cfg(unix)]
        temporary.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666));
        let mut file = temporary.tempfile_in(directory).map_err(io)?;
        file.write_all(text.as_bytes()).map_err(io)?;
        file.as_file().sync_all().map_err(io)?;
        file.persist(path).map_err(|e| io(e.error))?;
        Ok(())
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
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("x.photonation.json");
        let s = Settings {
            exposure: 1.25,
            ..Settings::default()
        };
        s.save(&path).expect("save");
        assert_eq!(Settings::load(&path).expect("load"), Some(s));
        assert_eq!(
            Settings::load(&dir.path().join("missing.json")).expect("load"),
            None
        );
        // Only the sidecar remains: the temporary file was renamed.
        assert_eq!(std::fs::read_dir(dir.path()).expect("list").count(), 1);
    }

    #[test]
    fn oversized_sidecar_is_rejected() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("big.photonation.json");
        let file = std::fs::File::create(&path).expect("create");
        file.set_len(MAX_SIDECAR_BYTES + 1).expect("grow");
        assert!(matches!(
            Settings::load(&path),
            Err(SidecarError::TooLarge { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn save_never_writes_through_planted_symlinks() {
        // In a shared folder another user could point the sidecar path, or
        // the predictable temporary name older builds used, at one of the
        // victim's files. Saving must replace links, never follow them.
        let dir = tempfile::tempdir().expect("temp dir");
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "precious").expect("write victim");
        let path = dir.path().join("x.photonation.json");
        std::os::unix::fs::symlink(&victim, &path).expect("symlink");
        std::os::unix::fs::symlink(&victim, path.with_extension("json.tmp")).expect("symlink");
        Settings::default().save(&path).expect("save");
        assert_eq!(
            std::fs::read_to_string(&victim).expect("read victim"),
            "precious"
        );
        assert!(!std::fs::symlink_metadata(&path).expect("stat").is_symlink());
    }
}
