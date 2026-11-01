// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Copying settings between photos, and presets: named settings that
//! apply only the parts they hold, so a preset that sets Exposure leaves a
//! photo's Contrast alone. Presets are JSON files in a folder; Lightroom
//! `.xmp` presets import into that format. Documented in
//! docs/EDIT-MODEL.md.

use crate::{AdjustmentGroup, MAX_SIDECAR_BYTES, Settings, XmpError, import_lightroom_preset};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

const FORMAT: &str = "photonation.preset";
/// Presets are read from `*.json` files in the presets folder.
pub const PRESET_EXTENSION: &str = "json";
pub const MAX_PRESET_NAME_CHARS: usize = 64;
/// Where presets without a group are listed.
pub const DEFAULT_PRESET_GROUP: &str = "User Presets";

/// A part of a photo's settings that can be copied on its own, as in
/// Lightroom's Copy Settings dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SettingsPart {
    WhiteBalance,
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Texture,
    Clarity,
    Dehaze,
    Vibrance,
    Saturation,
    ToneCurve,
    ColorMixer,
    PointColors,
    ColorGrading,
    Sharpening,
    Crop,
    Masks,
    Removals,
}

impl SettingsPart {
    pub const ALL: [SettingsPart; 20] = [
        SettingsPart::WhiteBalance,
        SettingsPart::Exposure,
        SettingsPart::Contrast,
        SettingsPart::Highlights,
        SettingsPart::Shadows,
        SettingsPart::Whites,
        SettingsPart::Blacks,
        SettingsPart::Texture,
        SettingsPart::Clarity,
        SettingsPart::Dehaze,
        SettingsPart::Vibrance,
        SettingsPart::Saturation,
        SettingsPart::ToneCurve,
        SettingsPart::ColorMixer,
        SettingsPart::PointColors,
        SettingsPart::ColorGrading,
        SettingsPart::Sharpening,
        SettingsPart::Crop,
        SettingsPart::Masks,
        SettingsPart::Removals,
    ];

    /// The name preset files use.
    pub fn id(self) -> &'static str {
        match self {
            SettingsPart::WhiteBalance => "white_balance",
            SettingsPart::Exposure => "exposure",
            SettingsPart::Contrast => "contrast",
            SettingsPart::Highlights => "highlights",
            SettingsPart::Shadows => "shadows",
            SettingsPart::Whites => "whites",
            SettingsPart::Blacks => "blacks",
            SettingsPart::Texture => "texture",
            SettingsPart::Clarity => "clarity",
            SettingsPart::Dehaze => "dehaze",
            SettingsPart::Vibrance => "vibrance",
            SettingsPart::Saturation => "saturation",
            SettingsPart::ToneCurve => "tone_curve",
            SettingsPart::ColorMixer => "color_mixer",
            SettingsPart::PointColors => "point_colors",
            SettingsPart::ColorGrading => "color_grading",
            SettingsPart::Sharpening => "sharpening",
            SettingsPart::Crop => "crop",
            SettingsPart::Masks => "masks",
            SettingsPart::Removals => "removals",
        }
    }

    pub fn from_id(id: &str) -> Option<SettingsPart> {
        SettingsPart::ALL.into_iter().find(|part| part.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingsPart::WhiteBalance => "White Balance",
            SettingsPart::Exposure => "Exposure",
            SettingsPart::Contrast => "Contrast",
            SettingsPart::Highlights => "Highlights",
            SettingsPart::Shadows => "Shadows",
            SettingsPart::Whites => "Whites",
            SettingsPart::Blacks => "Blacks",
            SettingsPart::Texture => "Texture",
            SettingsPart::Clarity => "Clarity",
            SettingsPart::Dehaze => "Dehaze",
            SettingsPart::Vibrance => "Vibrance",
            SettingsPart::Saturation => "Saturation",
            SettingsPart::ToneCurve => "Curves",
            SettingsPart::ColorMixer => "Mixer Bands",
            SettingsPart::PointColors => "Point Colors",
            SettingsPart::ColorGrading => "Color Grading",
            SettingsPart::Sharpening => "Sharpening",
            SettingsPart::Crop => "Crop and Straighten",
            SettingsPart::Masks => "Masks",
            SettingsPart::Removals => "Removals",
        }
    }

    /// The adjustment card the part is on; `None` for the tools (crop,
    /// masks, removals).
    pub fn group(self) -> Option<AdjustmentGroup> {
        Some(match self {
            SettingsPart::WhiteBalance => AdjustmentGroup::WhiteBalance,
            SettingsPart::Exposure
            | SettingsPart::Contrast
            | SettingsPart::Highlights
            | SettingsPart::Shadows
            | SettingsPart::Whites
            | SettingsPart::Blacks => AdjustmentGroup::Light,
            SettingsPart::Texture | SettingsPart::Clarity | SettingsPart::Dehaze => {
                AdjustmentGroup::Presence
            }
            SettingsPart::Vibrance | SettingsPart::Saturation => AdjustmentGroup::Color,
            SettingsPart::ToneCurve => AdjustmentGroup::ToneCurve,
            SettingsPart::ColorMixer | SettingsPart::PointColors => AdjustmentGroup::ColorMixer,
            SettingsPart::ColorGrading => AdjustmentGroup::ColorGrading,
            SettingsPart::Sharpening => AdjustmentGroup::Detail,
            SettingsPart::Crop | SettingsPart::Masks | SettingsPart::Removals => return None,
        })
    }

    /// The parts on `group`'s card, in display order.
    pub fn in_group(group: AdjustmentGroup) -> impl Iterator<Item = SettingsPart> {
        SettingsPart::ALL
            .into_iter()
            .filter(move |part| part.group() == Some(group))
    }

    /// Sets this part of `to` to `from`'s.
    pub fn copy(self, from: &Settings, to: &mut Settings) {
        match self {
            SettingsPart::WhiteBalance => to.white_balance = from.white_balance,
            SettingsPart::Exposure => to.exposure = from.exposure,
            SettingsPart::Contrast => to.contrast = from.contrast,
            SettingsPart::Highlights => to.highlights = from.highlights,
            SettingsPart::Shadows => to.shadows = from.shadows,
            SettingsPart::Whites => to.whites = from.whites,
            SettingsPart::Blacks => to.blacks = from.blacks,
            SettingsPart::Texture => to.texture = from.texture,
            SettingsPart::Clarity => to.clarity = from.clarity,
            SettingsPart::Dehaze => to.dehaze = from.dehaze,
            SettingsPart::Vibrance => to.vibrance = from.vibrance,
            SettingsPart::Saturation => to.saturation = from.saturation,
            SettingsPart::ToneCurve => to.tone_curve = from.tone_curve.clone(),
            SettingsPart::ColorMixer => to.color_mixer = from.color_mixer,
            SettingsPart::PointColors => to.point_colors = from.point_colors.clone(),
            SettingsPart::ColorGrading => to.color_grading = from.color_grading,
            SettingsPart::Sharpening => to.sharpening = from.sharpening,
            SettingsPart::Crop => {
                to.crop = from.crop;
                to.crop_angle = from.crop_angle;
            }
            SettingsPart::Masks => {
                to.masks = from.masks.clone();
                to.mask_groups = from.mask_groups.clone();
            }
            SettingsPart::Removals => to.removals = from.removals.clone(),
        }
    }

    /// Whether `settings` has this part at its default.
    pub fn is_default(self, settings: &Settings) -> bool {
        let mut reset = settings.clone();
        self.copy(&Settings::default(), &mut reset);
        reset == *settings
    }
}

/// A set of [`SettingsPart`]s.
pub type SettingsParts = BTreeSet<SettingsPart>;

/// What Copy Settings starts with: everything but the crop, masks and
/// removals, which belong to one photo's framing and content.
pub fn default_copy_parts() -> SettingsParts {
    SettingsPart::ALL
        .into_iter()
        .filter(|part| part.group().is_some())
        .collect()
}

/// What a new preset starts with: the look, without the photo's own white
/// balance and sampled point colours.
pub fn default_preset_parts() -> SettingsParts {
    default_copy_parts()
        .into_iter()
        .filter(|part| !matches!(part, SettingsPart::WhiteBalance | SettingsPart::PointColors))
        .collect()
}

/// Sets `parts` of `to` to `from`'s and leaves the rest.
pub fn copy_parts(from: &Settings, to: &mut Settings, parts: &SettingsParts) {
    for part in parts {
        part.copy(from, to);
    }
}

/// Named settings that apply only their `parts`.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub name: String,
    /// The heading it is listed under.
    pub group: String,
    pub parts: SettingsParts,
    /// Defaults outside `parts`.
    pub settings: Settings,
}

impl Preset {
    /// A preset of `parts` of `settings`.
    pub fn new(name: &str, group: &str, settings: &Settings, parts: SettingsParts) -> Preset {
        let mut kept = Settings::default();
        copy_parts(&settings.sanitized(), &mut kept, &parts);
        Preset {
            name: clean_name(name, "Preset"),
            group: clean_name(group, DEFAULT_PRESET_GROUP),
            parts,
            settings: kept,
        }
    }

    /// `settings` with this preset applied.
    pub fn applied_to(&self, settings: &Settings) -> Settings {
        let mut applied = settings.clone();
        copy_parts(&self.settings, &mut applied, &self.parts);
        applied
    }

    pub fn to_json(&self) -> String {
        let stored = StoredPreset {
            format: FORMAT.into(),
            name: self.name.clone(),
            group: self.group.clone(),
            parts: self.parts.iter().map(|part| part.id().to_owned()).collect(),
            settings: self.settings.clone(),
        };
        let mut text = serde_json::to_string_pretty(&stored).unwrap_or_default();
        text.push('\n');
        text
    }

    /// Reads a preset file's text. Part names this build doesn't know
    /// are skipped, so newer presets apply what they can.
    pub fn from_json(text: &str) -> Result<Preset, PresetFileError> {
        let stored: StoredPreset = serde_json::from_str(text)?;
        if stored.format != FORMAT {
            return Err(PresetFileError::Format(stored.format));
        }
        let parts = stored
            .parts
            .iter()
            .filter_map(|id| SettingsPart::from_id(id))
            .collect();
        Ok(Preset::new(
            &stored.name,
            &stored.group,
            &stored.settings,
            parts,
        ))
    }
}

/// A preset file's contents; unknown fields are ignored.
#[derive(Serialize, Deserialize)]
struct StoredPreset {
    format: String,
    name: String,
    #[serde(default)]
    group: String,
    parts: Vec<String>,
    #[serde(default)]
    settings: Settings,
}

/// Trims, removes control characters and caps at
/// [`MAX_PRESET_NAME_CHARS`]; `fallback` when nothing is left.
fn clean_name(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_PRESET_NAME_CHARS)
        .collect();
    match cleaned.trim() {
        "" => fallback.to_owned(),
        trimmed => trimmed.to_owned(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PresetFileError {
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("not a Photonation preset (format {0:?})")]
    Format(String),
    #[error("unreadable Lightroom preset: {0}")]
    Xmp(#[from] XmpError),
    #[error("holds no develop settings")]
    Empty,
}

#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: larger than {MAX_SIDECAR_BYTES} bytes")]
    TooLarge { path: PathBuf },
    #[error("{path}: {source}")]
    File {
        path: PathBuf,
        source: PresetFileError,
    },
}

/// A preset in the presets folder.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredPresetFile {
    pub path: PathBuf,
    pub preset: Preset,
}

/// The presets in `folder`, sorted by group and name, and the files that
/// could not be read. A missing folder holds none.
pub fn load_presets(folder: &Path) -> (Vec<StoredPresetFile>, Vec<PresetError>) {
    let (mut presets, mut errors) = (Vec::new(), Vec::new());
    let Ok(entries) = std::fs::read_dir(folder) else {
        return (presets, errors);
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.extension().and_then(|e| e.to_str()) != Some(PRESET_EXTENSION) {
            continue;
        }
        let preset = read_capped(&path).and_then(|text| {
            Preset::from_json(&text).map_err(|source| PresetError::File {
                path: path.clone(),
                source,
            })
        });
        match preset {
            Ok(preset) => presets.push(StoredPresetFile { path, preset }),
            Err(error) => errors.push(error),
        }
    }
    presets.sort_by(|a, b| {
        let key =
            |p: &StoredPresetFile| (p.preset.group.to_lowercase(), p.preset.name.to_lowercase());
        key(a).cmp(&key(b))
    });
    (presets, errors)
}

/// Writes `preset` to a new file in `folder` (created if missing), named
/// after the preset, and returns its path.
pub fn save_new_preset(folder: &Path, preset: &Preset) -> Result<PathBuf, PresetError> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| PresetError::Io { path, source }
    };
    std::fs::create_dir_all(folder).map_err(io(folder))?;
    let path = unused_file_name(folder, &preset.name);
    crate::sidecar::write_atomic(&path, &preset.to_json()).map_err(io(&path))?;
    Ok(path)
}

/// Rewrites a preset file in place.
pub fn save_preset(path: &Path, preset: &Preset) -> Result<(), PresetError> {
    crate::sidecar::write_atomic(path, &preset.to_json()).map_err(|source| PresetError::Io {
        path: path.into(),
        source,
    })
}

/// A preset from a file the user picked: a Photonation preset (`.json`)
/// or a Lightroom one (`.xmp`), named after the file when it has no name.
/// Also returns the Lightroom edits it had to leave out.
pub fn import_preset_file(path: &Path) -> Result<(Preset, Vec<&'static str>), PresetError> {
    let text = read_capped(path)?;
    let file_error = |source| PresetError::File {
        path: path.into(),
        source,
    };
    let is_xmp = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("xmp"));
    if !is_xmp {
        return Preset::from_json(&text)
            .map(|preset| (preset, Vec::new()))
            .map_err(file_error);
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let imported = import_lightroom_preset(&text, &stem)
        .map_err(|e| file_error(e.into()))?
        .ok_or_else(|| file_error(PresetFileError::Empty))?;
    Ok((imported.preset, imported.not_imported))
}

/// `folder/<name>.json`, or `<name> 2.json` and so on if taken. Characters
/// that aren't safe in file names on every platform become `_`.
fn unused_file_name(folder: &Path, name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '(' | ')' | '&' | '+') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = match safe.trim().trim_start_matches('.') {
        "" => "Preset".to_owned(),
        trimmed => trimmed.to_owned(),
    };
    (1..)
        .map(|n| match n {
            1 => folder.join(format!("{safe}.{PRESET_EXTENSION}")),
            n => folder.join(format!("{safe} {n}.{PRESET_EXTENSION}")),
        })
        .find(|path| !path.exists())
        .expect("an unused name")
}

/// A file's text, read only up to [`MAX_SIDECAR_BYTES`].
fn read_capped(path: &Path) -> Result<String, PresetError> {
    let io = |source| PresetError::Io {
        path: path.into(),
        source,
    };
    let file = std::fs::File::open(path).map_err(io)?;
    let mut text = String::new();
    file.take(MAX_SIDECAR_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(io)?;
    if text.len() as u64 > MAX_SIDECAR_BYTES {
        return Err(PresetError::TooLarge { path: path.into() });
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edited() -> Settings {
        let mut s = Settings {
            exposure: 0.7,
            contrast: 25.0,
            saturation: -40.0,
            crop: Some(photonation_image::NormRect {
                left: 0.1,
                top: 0.1,
                right: 0.9,
                bottom: 0.9,
            }),
            crop_angle: 3.0,
            ..Settings::default()
        };
        s.color_mixer.blue.saturation = -30.0;
        s
    }

    #[test]
    fn every_part_round_trips_its_id_and_copies_something() {
        for part in SettingsPart::ALL {
            assert_eq!(SettingsPart::from_id(part.id()), Some(part));
        }
        let mut target = Settings::default();
        copy_parts(
            &edited(),
            &mut target,
            &SettingsPart::ALL.into_iter().collect(),
        );
        assert_eq!(target, edited());
    }

    #[test]
    fn copying_leaves_other_parts_alone() {
        let mut target = Settings {
            contrast: -10.0,
            highlights: -50.0,
            ..Settings::default()
        };
        copy_parts(&edited(), &mut target, &[SettingsPart::Exposure].into());
        assert_eq!(target.exposure, 0.7);
        assert_eq!(target.contrast, -10.0);
        assert_eq!(target.highlights, -50.0);
    }

    #[test]
    fn default_copies_leave_out_the_tools() {
        let mut target = Settings::default();
        copy_parts(&edited(), &mut target, &default_copy_parts());
        assert_eq!(target.crop, None);
        assert_eq!(target.crop_angle, 0.0);
        assert_eq!(target.color_mixer.blue.saturation, -30.0);
        assert!(!default_preset_parts().contains(&SettingsPart::WhiteBalance));
    }

    #[test]
    fn presets_keep_only_their_parts() {
        let preset = Preset::new(
            "  Moody\n",
            "",
            &edited(),
            [SettingsPart::Saturation, SettingsPart::ColorMixer].into(),
        );
        assert_eq!(preset.name, "Moody");
        assert_eq!(preset.group, DEFAULT_PRESET_GROUP);
        assert_eq!(preset.settings.exposure, 0.0, "not a part");
        let photo = Settings {
            exposure: -1.0,
            ..Settings::default()
        };
        let applied = preset.applied_to(&photo);
        assert_eq!(applied.exposure, -1.0);
        assert_eq!(applied.saturation, -40.0);
        assert_eq!(applied.color_mixer.blue.saturation, -30.0);
    }

    #[test]
    fn preset_files_round_trip_and_skip_unknown_parts() {
        let preset = Preset::new("Warm", "Mine", &edited(), default_preset_parts());
        assert_eq!(Preset::from_json(&preset.to_json()).expect("read"), preset);

        let newer = r#"{"format": "photonation.preset", "name": "Future", "parts": ["exposure", "grain"],
                        "settings": {"exposure": 1.5}, "comment": "ignored"}"#;
        let read = Preset::from_json(newer).expect("read");
        assert_eq!(read.parts, [SettingsPart::Exposure].into());
        assert_eq!(read.settings.exposure, 1.5);
        assert!(matches!(
            Preset::from_json(r#"{"format": "photonation.develop", "name": "x", "parts": []}"#),
            Err(PresetFileError::Format(_))
        ));
    }

    #[test]
    fn presets_are_saved_under_unused_safe_names() {
        let folder = tempfile::tempdir().expect("temp dir");
        let preset = Preset::new("B/W: high", "", &edited(), default_preset_parts());
        let first = save_new_preset(folder.path(), &preset).expect("save");
        let second = save_new_preset(folder.path(), &preset).expect("save");
        assert_eq!(first.file_name().expect("file name"), "B_W_ high.json");
        assert_eq!(second.file_name().expect("file name"), "B_W_ high 2.json");
        std::fs::write(folder.path().join("broken.json"), "{").expect("write");
        std::fs::write(folder.path().join("notes.txt"), "not a preset").expect("write");
        let (presets, errors) = load_presets(folder.path());
        assert_eq!(presets.len(), 2);
        assert_eq!(presets[0].preset, preset);
        assert_eq!(errors.len(), 1, "the broken file, not the text file");
        assert!(load_presets(&folder.path().join("missing")).0.is_empty());
    }
}
