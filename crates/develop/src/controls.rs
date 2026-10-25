// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::Settings;

/// Description of one slider.
#[derive(Clone, Copy, Debug)]
pub struct Control {
    /// Stable id used by the CLI, sidecars and UIs.
    pub id: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    accessor: fn(&mut Settings) -> &mut f32,
}

impl Control {
    pub fn clamp(&self, v: f32) -> f32 {
        if v.is_finite() {
            v.clamp(self.min, self.max)
        } else {
            self.default
        }
    }

    pub fn get_mut<'a>(&self, s: &'a mut Settings) -> &'a mut f32 {
        (self.accessor)(s)
    }

    pub fn get(&self, s: &Settings) -> f32 {
        // The accessor needs `&mut`; reading through a copy keeps `get` usable
        // on shared settings. Settings are small, so this is cheap.
        let mut copy = s.clone();
        *(self.accessor)(&mut copy)
    }

    pub fn find(id: &str) -> Option<&'static Control> {
        all_controls().find(|c| c.id == id)
    }
}

macro_rules! control {
    ($id:expr, $label:literal, $min:expr, $max:expr, $default:expr, $($field:ident).+) => {
        Control { id: $id, label: $label, min: $min, max: $max, default: $default, accessor: |s| &mut s.$($field).+ }
    };
}

macro_rules! hsl_band {
    ($band:ident, $name:literal) => {
        [
            control!(
                concat!("mixer-", $name, "-hue"),
                "Hue",
                -100.0,
                100.0,
                0.0,
                color_mixer.$band.hue
            ),
            control!(
                concat!("mixer-", $name, "-saturation"),
                "Saturation",
                -100.0,
                100.0,
                0.0,
                color_mixer.$band.saturation
            ),
            control!(
                concat!("mixer-", $name, "-luminance"),
                "Luminance",
                -100.0,
                100.0,
                0.0,
                color_mixer.$band.luminance
            ),
        ]
    };
}

macro_rules! grading_wheel {
    ($wheel:ident, $name:literal) => {
        [
            control!(
                concat!("grading-", $name, "-hue"),
                "Hue",
                0.0,
                360.0,
                0.0,
                color_grading.$wheel.hue
            ),
            control!(
                concat!("grading-", $name, "-saturation"),
                "Saturation",
                0.0,
                100.0,
                0.0,
                color_grading.$wheel.saturation
            ),
            control!(
                concat!("grading-", $name, "-luminance"),
                "Luminance",
                -100.0,
                100.0,
                0.0,
                color_grading.$wheel.luminance
            ),
        ]
    };
}

/// Basic panel: light, presence and colour.
pub static BASIC_CONTROLS: &[Control] = &[
    control!("exposure", "Exposure", -5.0, 5.0, 0.0, exposure),
    control!("contrast", "Contrast", -100.0, 100.0, 0.0, contrast),
    control!("highlights", "Highlights", -100.0, 100.0, 0.0, highlights),
    control!("shadows", "Shadows", -100.0, 100.0, 0.0, shadows),
    control!("whites", "Whites", -100.0, 100.0, 0.0, whites),
    control!("blacks", "Blacks", -100.0, 100.0, 0.0, blacks),
    control!("texture", "Texture", -100.0, 100.0, 0.0, texture),
    control!("clarity", "Clarity", -100.0, 100.0, 0.0, clarity),
    control!("dehaze", "Dehaze", -100.0, 100.0, 0.0, dehaze),
    control!("vibrance", "Vibrance", -100.0, 100.0, 0.0, vibrance),
    control!("saturation", "Saturation", -100.0, 100.0, 0.0, saturation),
];

pub static PARAMETRIC_CURVE_CONTROLS: &[Control] = &[
    control!(
        "curve-highlights",
        "Highlights",
        -100.0,
        100.0,
        0.0,
        tone_curve.parametric.highlights
    ),
    control!(
        "curve-lights",
        "Lights",
        -100.0,
        100.0,
        0.0,
        tone_curve.parametric.lights
    ),
    control!(
        "curve-darks",
        "Darks",
        -100.0,
        100.0,
        0.0,
        tone_curve.parametric.darks
    ),
    control!(
        "curve-shadows",
        "Shadows",
        -100.0,
        100.0,
        0.0,
        tone_curve.parametric.shadows
    ),
];

pub static MIXER_CONTROLS: [[Control; 3]; 8] = [
    hsl_band!(red, "red"),
    hsl_band!(orange, "orange"),
    hsl_band!(yellow, "yellow"),
    hsl_band!(green, "green"),
    hsl_band!(aqua, "aqua"),
    hsl_band!(blue, "blue"),
    hsl_band!(purple, "purple"),
    hsl_band!(magenta, "magenta"),
];

/// Shadows, midtones, highlights, global.
pub static GRADING_CONTROLS: [[Control; 3]; 4] = [
    grading_wheel!(shadows, "shadows"),
    grading_wheel!(midtones, "midtones"),
    grading_wheel!(highlights, "highlights"),
    grading_wheel!(global, "global"),
];

pub static GRADING_BLEND_CONTROLS: &[Control] = &[
    control!(
        "grading-blending",
        "Blending",
        0.0,
        100.0,
        50.0,
        color_grading.blending
    ),
    control!(
        "grading-balance",
        "Balance",
        -100.0,
        100.0,
        0.0,
        color_grading.balance
    ),
];

pub static DETAIL_CONTROLS: &[Control] = &[
    control!(
        "sharpen-amount",
        "Sharpening",
        0.0,
        150.0,
        40.0,
        sharpening.amount
    ),
    control!("sharpen-radius", "Radius", 0.5, 3.0, 1.0, sharpening.radius),
];

/// Every slider, in panel order.
pub fn all_controls() -> impl Iterator<Item = &'static Control> {
    BASIC_CONTROLS
        .iter()
        .chain(PARAMETRIC_CURVE_CONTROLS)
        .chain(MIXER_CONTROLS.iter().flatten())
        .chain(GRADING_CONTROLS.iter().flatten())
        .chain(GRADING_BLEND_CONTROLS)
        .chain(DETAIL_CONTROLS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_settings_default() {
        let s = Settings::default();
        for c in all_controls() {
            assert_eq!(c.get(&s), c.default, "{}", c.id);
        }
    }

    #[test]
    fn ids_are_unique() {
        let ids: Vec<&str> = all_controls().map(|c| c.id).collect();
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[i + 1..].contains(id), "{id}");
        }
    }
}
