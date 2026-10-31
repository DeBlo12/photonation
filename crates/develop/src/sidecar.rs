// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::{
    LibraryInfo, Settings, import_lightroom_library, import_lightroom_xmp, lightroom_xmp_path,
};
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
    #[error("{path}: unreadable Lightroom sidecar: {source}")]
    Xmp {
        path: PathBuf,
        source: crate::XmpError,
    },
}

/// Where a photo's settings were read from.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsOrigin {
    /// Its Photonation sidecar.
    Photonation,
    /// Imported from Lightroom's XMP sidecar, leaving out `not_imported`.
    Lightroom { not_imported: Vec<&'static str> },
}

/// A photo's develop settings and where they came from.
#[derive(Clone, Debug, PartialEq)]
pub struct PhotoSettings {
    pub settings: Settings,
    pub origin: SettingsOrigin,
}

/// Everything a photo's sidecars say about it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhotoSidecar {
    /// Its develop settings, if it has any.
    pub settings: Option<PhotoSettings>,
    /// Its rating, flag and label.
    pub library: LibraryInfo,
}

/// What a photo's sidecars hold: its Photonation sidecar, else what a
/// Lightroom XMP sidecar next to it has (develop settings, rating and
/// label); empty when it has neither. Once the photo is edited or rated
/// here, the Photonation sidecar takes over.
pub fn load_photo(original: &Path) -> Result<PhotoSidecar, SidecarError> {
    if let Some(sidecar) = read_sidecar(&sidecar_path(original))? {
        return Ok(PhotoSidecar {
            settings: Some(PhotoSettings {
                settings: sidecar.settings,
                origin: SettingsOrigin::Photonation,
            }),
            library: sidecar.library.sanitized(),
        });
    }
    let xmp = lightroom_xmp_path(original);
    let Some(text) = read_capped(&xmp)? else {
        return Ok(PhotoSidecar::default());
    };
    let xmp_error = |source| SidecarError::Xmp {
        path: xmp.clone(),
        source,
    };
    let imported = import_lightroom_xmp(&text).map_err(xmp_error)?;
    let library = import_lightroom_library(&text).map_err(xmp_error)?;
    Ok(PhotoSidecar {
        settings: imported.map(|imported| PhotoSettings {
            settings: imported.settings,
            origin: SettingsOrigin::Lightroom {
                not_imported: imported.not_imported,
            },
        }),
        library,
    })
}

/// The settings a photo opens with (see [`load_photo`]); `Ok(None)` when
/// it has none.
pub fn load_photo_settings(original: &Path) -> Result<Option<PhotoSettings>, SidecarError> {
    Ok(load_photo(original)?.settings)
}

/// Writes a photo's sidecar, or removes it when there is nothing to
/// remember: default settings and no rating, flag or label. A Lightroom
/// sidecar next to the photo keeps it, or Lightroom's edits and rating
/// would come back.
pub fn save_photo(
    original: &Path,
    settings: &Settings,
    library: &LibraryInfo,
) -> Result<(), SidecarError> {
    let path = sidecar_path(original);
    let nothing_to_remember = *settings == Settings::default() && library.is_empty();
    if nothing_to_remember && !lightroom_xmp_path(original).exists() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(SidecarError::Io { path, source: e })
            }
            _ => Ok(()),
        };
    }
    write_sidecar(&path, settings, library)
}

/// A sidecar's text; `Ok(None)` when it doesn't exist. Reads at most
/// [`MAX_SIDECAR_BYTES`].
fn read_capped(path: &Path) -> Result<Option<String>, SidecarError> {
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
    Ok(Some(text))
}

#[derive(Serialize, Deserialize)]
struct Sidecar {
    format: String,
    settings: Settings,
    #[serde(default, skip_serializing_if = "LibraryInfo::is_empty")]
    library: LibraryInfo,
}

/// A Photonation sidecar's contents; `Ok(None)` when it doesn't exist.
fn read_sidecar(path: &Path) -> Result<Option<Sidecar>, SidecarError> {
    let Some(text) = read_capped(path)? else {
        return Ok(None);
    };
    let sidecar: Sidecar = serde_json::from_str(&text).map_err(|source| SidecarError::Parse {
        path: path.into(),
        source,
    })?;
    if sidecar.format != FORMAT {
        return Err(SidecarError::Format {
            path: path.into(),
            found: sidecar.format,
        });
    }
    Ok(Some(sidecar))
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
        Ok(read_sidecar(path)?.map(|sidecar| sidecar.settings))
    }

    /// Writes settings to a sidecar, keeping the rating, flag and label it
    /// already holds (see [`write_sidecar`] for how).
    pub fn save(&self, path: &Path) -> Result<(), SidecarError> {
        let library = read_sidecar(path)
            .ok()
            .flatten()
            .map(|sidecar| sidecar.library)
            .unwrap_or_default();
        write_sidecar(path, self, &library)
    }
}

/// Writes a sidecar atomically so a crash never leaves a truncated one:
/// the text goes to a new temporary file beside it (created exclusively
/// under a random name, so a file or symlink planted in a shared folder is
/// never written through), is flushed to disk, then renamed over the
/// sidecar.
fn write_sidecar(
    path: &Path,
    settings: &Settings,
    library: &LibraryInfo,
) -> Result<(), SidecarError> {
    let sidecar = Sidecar {
        format: FORMAT.into(),
        settings: settings.clone(),
        library: *library,
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

    #[test]
    fn photonation_sidecars_take_precedence_over_lightroom() {
        let dir = tempfile::tempdir().expect("temp dir");
        let photo = dir.path().join("IMG_0001.CR3");
        assert_eq!(load_photo_settings(&photo).expect("load"), None);
        std::fs::write(
            lightroom_xmp_path(&photo),
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
               <rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
                crs:ProcessVersion="15.4" crs:Exposure2012="+0.70"/></rdf:RDF></x:xmpmeta>"#,
        )
        .expect("write xmp");
        let imported = load_photo_settings(&photo)
            .expect("load")
            .expect("imported");
        assert_eq!(imported.settings.exposure, 0.7);
        assert!(matches!(imported.origin, SettingsOrigin::Lightroom { .. }));

        let edited = Settings {
            exposure: -1.0,
            ..Settings::default()
        };
        edited.save(&sidecar_path(&photo)).expect("save");
        let loaded = load_photo_settings(&photo).expect("load").expect("ours");
        assert_eq!(loaded.settings, edited);
        assert_eq!(loaded.origin, SettingsOrigin::Photonation);
    }

    fn rated(rating: u8) -> LibraryInfo {
        LibraryInfo {
            rating,
            ..LibraryInfo::default()
        }
    }

    #[test]
    fn ratings_live_beside_the_settings() {
        let dir = tempfile::tempdir().expect("temp dir");
        let photo = dir.path().join("IMG_0001.CR3");
        save_photo(&photo, &Settings::default(), &rated(3)).expect("save");
        let loaded = load_photo(&photo).expect("load");
        assert_eq!(loaded.library, rated(3));
        // A rated photo is not an edited one.
        let settings = loaded.settings.expect("sidecar").settings;
        assert_eq!(settings, Settings::default());
        // Saving settings alone (the CLI's `edit`) keeps the rating.
        let edited = Settings {
            exposure: 0.5,
            ..Settings::default()
        };
        edited.save(&sidecar_path(&photo)).expect("save");
        assert_eq!(load_photo(&photo).expect("load").library, rated(3));
    }

    #[test]
    fn unrated_sidecars_leave_the_library_out() {
        let dir = tempfile::tempdir().expect("temp dir");
        let photo = dir.path().join("IMG_0001.CR3");
        let edited = Settings {
            exposure: 0.5,
            ..Settings::default()
        };
        save_photo(&photo, &edited, &LibraryInfo::default()).expect("save");
        let text = std::fs::read_to_string(sidecar_path(&photo)).expect("read");
        assert!(!text.contains("library"), "{text}");
    }

    #[test]
    fn sidecars_with_nothing_to_remember_are_removed() {
        let dir = tempfile::tempdir().expect("temp dir");
        let photo = dir.path().join("IMG_0001.CR3");
        save_photo(&photo, &Settings::default(), &rated(2)).expect("save");
        assert!(sidecar_path(&photo).exists());
        save_photo(&photo, &Settings::default(), &LibraryInfo::default()).expect("save");
        assert!(!sidecar_path(&photo).exists());
        // Unless Lightroom's rating would come back without it.
        std::fs::write(
            lightroom_xmp_path(&photo),
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
               <rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="5"/></rdf:RDF></x:xmpmeta>"#,
        )
        .expect("write xmp");
        assert_eq!(load_photo(&photo).expect("load").library, rated(5));
        assert_eq!(load_photo(&photo).expect("load").settings, None);
        save_photo(&photo, &Settings::default(), &LibraryInfo::default()).expect("save");
        assert!(sidecar_path(&photo).exists());
        assert!(load_photo(&photo).expect("load").library.is_empty());
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
