// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Choices remembered between launches, in `preferences.json` in the
//! engine's data directory: what Settings holds, and the export sheet's
//! metadata choice. A missing or unreadable file means defaults; a missing
//! field means its default, so older files stay readable.

use crate::gpu::GpuSetting;
use photonation_color::ColorSpace;
use photonation_engine::{ExportOptions, MetadataOptions, data_directory};
use photonation_export::Format;
use photonation_image::BitDepth;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// The export metadata groups and the creator and copyright entered
    /// in the export sheet or Settings.
    pub export_metadata: MetadataOptions,
    /// What the export sheet starts with.
    pub export: ExportDefaults,
    /// Where exports go: the folder dialog opens here, or, with
    /// `export_without_asking`, files go straight here.
    pub export_folder: Option<PathBuf>,
    pub export_without_asking: bool,
    /// The graphics adapter, chosen at start-up.
    pub gpu: GpuSetting,
}

/// The export sheet's starting options, except metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportDefaults {
    pub format: ExportFormat,
    pub jpeg_quality: u8,
    /// 16 bits per channel for PNG and TIFF.
    pub sixteen_bit: bool,
    pub color_space: ColorSpace,
    /// `None`: the original size.
    pub long_edge: Option<usize>,
    pub file_name: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportFormat {
    #[default]
    Jpeg,
    Png,
    Tiff,
}

impl Default for ExportDefaults {
    fn default() -> Self {
        ExportDefaults::from_options(&ExportOptions::default())
    }
}

impl ExportDefaults {
    pub fn from_options(options: &ExportOptions) -> ExportDefaults {
        let (format, jpeg_quality) = match options.format {
            Format::Jpeg { quality } => (ExportFormat::Jpeg, quality),
            Format::Png => (ExportFormat::Png, DEFAULT_QUALITY),
            Format::Tiff => (ExportFormat::Tiff, DEFAULT_QUALITY),
        };
        ExportDefaults {
            format,
            jpeg_quality,
            sixteen_bit: options.bit_depth == BitDepth::Sixteen,
            color_space: options.color_space,
            long_edge: options.long_edge,
            file_name: options.file_name.clone(),
        }
    }

    pub fn options(&self, metadata: MetadataOptions) -> ExportOptions {
        let format = match self.format {
            ExportFormat::Jpeg => Format::Jpeg {
                quality: self.jpeg_quality.clamp(1, 100),
            },
            ExportFormat::Png => Format::Png,
            ExportFormat::Tiff => Format::Tiff,
        };
        let sixteen = self.sixteen_bit && self.format != ExportFormat::Jpeg;
        ExportOptions {
            format,
            bit_depth: if sixteen {
                BitDepth::Sixteen
            } else {
                BitDepth::Eight
            },
            color_space: self.color_space,
            long_edge: self.long_edge,
            file_name: self.file_name.clone(),
            metadata,
        }
    }
}

const DEFAULT_QUALITY: u8 = 92;

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

    /// The options a new export starts with.
    pub fn export_options(&self) -> ExportOptions {
        self.export.options(self.export_metadata.clone())
    }

    /// The folder exports go to without a dialog, if Settings asks for
    /// that and the folder still exists.
    pub fn fixed_export_folder(&self) -> Option<&PathBuf> {
        self.export_folder
            .as_ref()
            .filter(|folder| self.export_without_asking && folder.is_dir())
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
        assert_eq!(partial.export, ExportDefaults::default());
        assert_eq!(partial.gpu, GpuSetting::Automatic);
        assert!(!partial.export_without_asking);
        let empty: Preferences = serde_json::from_str("{}").expect("parse");
        assert_eq!(empty, Preferences::default());
    }

    #[test]
    fn export_defaults_round_trip_through_options() {
        let defaults = ExportDefaults {
            format: ExportFormat::Tiff,
            jpeg_quality: 80,
            sixteen_bit: true,
            color_space: ColorSpace::DisplayP3,
            long_edge: Some(3000),
            file_name: "{name}-web".into(),
        };
        let options = defaults.options(MetadataOptions::default());
        assert_eq!(options.format, Format::Tiff);
        assert_eq!(options.bit_depth, BitDepth::Sixteen);
        let back = ExportDefaults::from_options(&options);
        // The JPEG quality is not part of TIFF options.
        assert_eq!(
            back,
            ExportDefaults {
                jpeg_quality: DEFAULT_QUALITY,
                ..defaults
            }
        );
        assert_eq!(
            ExportDefaults::default().options(MetadataOptions::default()),
            ExportOptions::default()
        );
    }

    #[test]
    fn settings_survive_saving_and_loading() {
        let preferences = Preferences {
            export_folder: Some(PathBuf::from("/photos/exports")),
            export_without_asking: true,
            gpu: GpuSetting::Adapter("NVIDIA GeForce RTX 5080".into()),
            ..Preferences::default()
        };
        let text = serde_json::to_string(&preferences).expect("serialize");
        let loaded: Preferences = serde_json::from_str(&text).expect("parse");
        assert_eq!(loaded, preferences);
    }
}
