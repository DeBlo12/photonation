// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Which of a photo's metadata an export writes, in Lightroom-like groups.
//! The chosen fields are written as EXIF by `photonation_export`; see
//! docs/ENGINE.md.

use photonation_image::CameraInfo;
use serde::{Deserialize, Serialize};

/// The metadata groups an export writes, and the creator and copyright the
/// user entered for them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataOptions {
    /// Camera and capture: make, model, lens, focal length, aperture,
    /// shutter speed, ISO, exposure compensation, capture date and time.
    pub camera: bool,
    /// Copyright and creator: the artist and copyright entered below, or
    /// the raw file's where nothing is entered.
    pub copyright: bool,
    /// GPS position. Off by default: it reveals where the photo was taken.
    pub location: bool,
    /// Written as the artist instead of the raw file's; blank keeps it.
    pub creator: String,
    /// Written as the copyright instead of the raw file's; blank keeps it.
    pub copyright_notice: String,
}

impl Default for MetadataOptions {
    fn default() -> Self {
        MetadataOptions {
            camera: true,
            copyright: true,
            location: false,
            creator: String::new(),
            copyright_notice: String::new(),
        }
    }
}

/// Shortcuts for the groups, as Lightroom's "Include" menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataPreset {
    /// Camera and capture plus copyright and creator. Location stays as
    /// chosen, like Lightroom's separate "Remove Location Info".
    All,
    CopyrightOnly,
    None,
}

impl MetadataPreset {
    pub const ALL: [MetadataPreset; 3] = [
        MetadataPreset::All,
        MetadataPreset::CopyrightOnly,
        MetadataPreset::None,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MetadataPreset::All => "All",
            MetadataPreset::CopyrightOnly => "Copyright Only",
            MetadataPreset::None => "None",
        }
    }
}

impl MetadataOptions {
    /// Sets the groups to `preset`'s; the entered creator and copyright
    /// are kept for later.
    pub fn apply(&mut self, preset: MetadataPreset) {
        match preset {
            MetadataPreset::All => {
                self.camera = true;
                self.copyright = true;
            }
            MetadataPreset::CopyrightOnly => {
                self.camera = false;
                self.copyright = true;
                self.location = false;
            }
            MetadataPreset::None => {
                self.camera = false;
                self.copyright = false;
                self.location = false;
            }
        }
    }

    /// The preset the groups match, if any (`None` for a custom choice).
    pub fn preset(&self) -> Option<MetadataPreset> {
        match (self.camera, self.copyright, self.location) {
            (true, true, _) => Some(MetadataPreset::All),
            (false, true, false) => Some(MetadataPreset::CopyrightOnly),
            (false, false, false) => Some(MetadataPreset::None),
            _ => None,
        }
    }

    /// The metadata to write for a photo whose raw file recorded
    /// `recorded`: the chosen groups' fields, with the entered creator and
    /// copyright in place of the file's. Everything else is left empty.
    pub fn select(&self, recorded: &CameraInfo) -> CameraInfo {
        let mut selected = CameraInfo::default();
        if self.camera {
            selected = CameraInfo {
                artist: None,
                copyright: None,
                gps: None,
                ..recorded.clone()
            };
        }
        if self.copyright {
            let entered = |text: &str| Some(text.trim().to_string()).filter(|t| !t.is_empty());
            selected.artist = entered(&self.creator).or_else(|| recorded.artist.clone());
            selected.copyright =
                entered(&self.copyright_notice).or_else(|| recorded.copyright.clone());
        }
        if self.location {
            selected.gps = recorded.gps;
        }
        selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonation_image::GpsPosition;

    fn recorded() -> CameraInfo {
        CameraInfo {
            make: "Canon".into(),
            model: "Canon EOS 250D".into(),
            lens: Some("Canon EF 50mm f/1.8 STM".into()),
            iso: Some(200),
            date_time: Some("2026:06:20 18:02:23".into()),
            artist: Some("Camera Owner".into()),
            copyright: Some("© Camera Owner".into()),
            gps: Some(GpsPosition {
                latitude: 48.1,
                longitude: 11.6,
                altitude: None,
            }),
            ..CameraInfo::default()
        }
    }

    #[test]
    fn location_is_off_by_default() {
        let selected = MetadataOptions::default().select(&recorded());
        assert_eq!(selected.model, "Canon EOS 250D");
        assert_eq!(selected.artist.as_deref(), Some("Camera Owner"));
        assert_eq!(selected.gps, None);
        assert_eq!(
            MetadataOptions::default().preset(),
            Some(MetadataPreset::All)
        );
    }

    #[test]
    fn entered_creator_replaces_the_files() {
        let options = MetadataOptions {
            creator: " Ada Lovelace ".into(),
            copyright_notice: "  ".into(),
            ..MetadataOptions::default()
        };
        let selected = options.select(&recorded());
        assert_eq!(selected.artist.as_deref(), Some("Ada Lovelace"));
        assert_eq!(selected.copyright.as_deref(), Some("© Camera Owner"));
    }

    #[test]
    fn presets_choose_groups() {
        let mut options = MetadataOptions {
            location: true,
            creator: "Ada".into(),
            ..MetadataOptions::default()
        };
        let everything = CameraInfo {
            artist: Some("Ada".into()),
            ..recorded()
        };
        assert_eq!(options.select(&recorded()), everything);

        options.apply(MetadataPreset::CopyrightOnly);
        assert_eq!(options.preset(), Some(MetadataPreset::CopyrightOnly));
        assert_eq!(
            options.select(&recorded()),
            CameraInfo {
                artist: Some("Ada".into()),
                copyright: Some("© Camera Owner".into()),
                ..CameraInfo::default()
            }
        );

        options.apply(MetadataPreset::None);
        assert_eq!(options.select(&recorded()), CameraInfo::default());
        assert_eq!(options.creator, "Ada", "entered text is kept");

        options.apply(MetadataPreset::All);
        assert!(options.camera && options.copyright && !options.location);
        options.copyright = false;
        assert_eq!(options.preset(), None, "custom");
    }
}
