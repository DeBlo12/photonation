// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::{ColorGrading, ColorMixer, Settings, Sharpening, ToneCurve, WhiteBalance};

/// The adjustment panels, in display order. Each can be reset or
/// temporarily disabled as a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AdjustmentGroup {
    WhiteBalance,
    Light,
    Presence,
    Color,
    ToneCurve,
    ColorMixer,
    ColorGrading,
    Detail,
}

impl AdjustmentGroup {
    pub const ALL: [AdjustmentGroup; 8] = [
        AdjustmentGroup::WhiteBalance,
        AdjustmentGroup::Light,
        AdjustmentGroup::Presence,
        AdjustmentGroup::Color,
        AdjustmentGroup::ToneCurve,
        AdjustmentGroup::ColorMixer,
        AdjustmentGroup::ColorGrading,
        AdjustmentGroup::Detail,
    ];

    pub fn title(self) -> &'static str {
        match self {
            AdjustmentGroup::WhiteBalance => "White Balance",
            AdjustmentGroup::Light => "Light",
            AdjustmentGroup::Presence => "Presence",
            AdjustmentGroup::Color => "Color",
            AdjustmentGroup::ToneCurve => "Curves",
            AdjustmentGroup::ColorMixer => "Color Mixer",
            AdjustmentGroup::ColorGrading => "Color Grading",
            AdjustmentGroup::Detail => "Detail",
        }
    }

    /// Restores this group's settings to their defaults.
    pub fn reset(self, s: &mut Settings) {
        let defaults = Settings::default();
        match self {
            AdjustmentGroup::WhiteBalance => s.white_balance = WhiteBalance::AsShot,
            AdjustmentGroup::Light => {
                s.exposure = defaults.exposure;
                s.contrast = defaults.contrast;
                s.highlights = defaults.highlights;
                s.shadows = defaults.shadows;
                s.whites = defaults.whites;
                s.blacks = defaults.blacks;
            }
            AdjustmentGroup::Presence => {
                s.texture = defaults.texture;
                s.clarity = defaults.clarity;
                s.dehaze = defaults.dehaze;
            }
            AdjustmentGroup::Color => {
                s.vibrance = defaults.vibrance;
                s.saturation = defaults.saturation;
            }
            AdjustmentGroup::ToneCurve => s.tone_curve = ToneCurve::default(),
            AdjustmentGroup::ColorMixer => {
                s.color_mixer = ColorMixer::default();
                s.point_colors.clear();
            }
            AdjustmentGroup::ColorGrading => s.color_grading = ColorGrading::default(),
            // Disabling detail turns sharpening off rather than back to its default.
            AdjustmentGroup::Detail => {
                s.sharpening = Sharpening {
                    amount: 0.0,
                    ..Sharpening::default()
                }
            }
        }
    }

    /// Whether the group differs from its defaults.
    pub fn is_modified(self, s: &Settings) -> bool {
        let mut reset = s.clone();
        match self {
            AdjustmentGroup::Detail => reset.sharpening = Sharpening::default(),
            other => other.reset(&mut reset),
        }
        reset != *s
    }

    /// Settings with the given groups switched off.
    pub fn apply_disabled<'a>(
        s: &Settings,
        disabled: impl IntoIterator<Item = &'a AdjustmentGroup>,
    ) -> Settings {
        let mut effective = s.clone();
        for group in disabled {
            group.reset(&mut effective);
        }
        effective
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_clears_modifications() {
        let mut s = Settings {
            exposure: 1.0,
            clarity: 20.0,
            ..Settings::default()
        };
        s.color_mixer.red.hue = 10.0;
        assert!(AdjustmentGroup::Light.is_modified(&s));
        for group in AdjustmentGroup::ALL {
            group.reset(&mut s);
        }
        assert!(
            AdjustmentGroup::ALL
                .iter()
                .all(|g| !g.is_modified(&s) || *g == AdjustmentGroup::Detail)
        );
    }

    #[test]
    fn disabled_groups_do_not_touch_others() {
        let s = Settings {
            exposure: 1.0,
            vibrance: 30.0,
            ..Settings::default()
        };
        let effective = AdjustmentGroup::apply_disabled(&s, &[AdjustmentGroup::Color]);
        assert_eq!(effective.exposure, 1.0);
        assert_eq!(effective.vibrance, 0.0);
    }
}
