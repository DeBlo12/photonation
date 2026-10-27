// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::{LocalAdjustments, PointColor, Settings};

/// Description of one slider over a `Target`: the photo's [`Settings`],
/// a mask's [`LocalAdjustments`] or a [`PointColor`].
#[derive(Debug)]
pub struct Control<Target = Settings> {
    /// Stable id used by the CLI, sidecars and UIs.
    pub id: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    accessor: fn(&mut Target) -> &mut f32,
}

// Manual impls: a derive would require `Target: Copy`, but only the
// function pointer is copied.
impl<Target> Clone for Control<Target> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Target> Copy for Control<Target> {}

impl<Target> Control<Target> {
    pub fn clamp(&self, v: f32) -> f32 {
        if v.is_finite() {
            v.clamp(self.min, self.max)
        } else {
            self.default
        }
    }

    pub fn get_mut<'a>(&self, target: &'a mut Target) -> &'a mut f32 {
        (self.accessor)(target)
    }
}

impl<Target: Clone> Control<Target> {
    pub fn get(&self, target: &Target) -> f32 {
        // The accessor needs `&mut`; reading through a copy keeps `get` usable
        // on shared values. Both targets are small, so this is cheap.
        let mut copy = target.clone();
        *(self.accessor)(&mut copy)
    }
}

impl Control {
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

/// The crop tool's straighten angle.
pub static CROP_CONTROLS: &[Control] = &[control!(
    "crop-angle",
    "Angle",
    -45.0,
    45.0,
    0.0,
    crop_angle
)];

/// A mask's local adjustments, in the order the mask editor shows them.
pub static LOCAL_CONTROLS: &[Control<LocalAdjustments>] = &[
    control!(
        "local-temperature",
        "Temperature",
        -100.0,
        100.0,
        0.0,
        temperature
    ),
    control!("local-tint", "Tint", -100.0, 100.0, 0.0, tint),
    control!("local-exposure", "Exposure", -4.0, 4.0, 0.0, exposure),
    control!("local-contrast", "Contrast", -100.0, 100.0, 0.0, contrast),
    control!(
        "local-highlights",
        "Highlights",
        -100.0,
        100.0,
        0.0,
        highlights
    ),
    control!("local-shadows", "Shadows", -100.0, 100.0, 0.0, shadows),
    control!("local-dehaze", "Dehaze", -100.0, 100.0, 0.0, dehaze),
    control!(
        "local-saturation",
        "Saturation",
        -100.0,
        100.0,
        0.0,
        saturation
    ),
];

/// A point colour's shifts, then its ranges, in the order the Point Color
/// tab shows them.
pub static POINT_COLOR_CONTROLS: &[Control<PointColor>] = &[
    control!("point-hue", "Hue Shift", -100.0, 100.0, 0.0, hue),
    control!(
        "point-saturation",
        "Saturation Shift",
        -100.0,
        100.0,
        0.0,
        saturation
    ),
    control!(
        "point-luminance",
        "Luminance Shift",
        -100.0,
        100.0,
        0.0,
        luminance
    ),
    control!("point-hue-range", "Hue Range", 0.0, 100.0, 50.0, hue_range),
    control!(
        "point-saturation-range",
        "Saturation Range",
        0.0,
        100.0,
        50.0,
        saturation_range
    ),
    control!(
        "point-luminance-range",
        "Luminance Range",
        0.0,
        100.0,
        50.0,
        luminance_range
    ),
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
        .chain(CROP_CONTROLS)
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
    fn local_defaults_match_local_adjustments_default() {
        let adjustments = LocalAdjustments::default();
        for c in LOCAL_CONTROLS {
            assert_eq!(c.get(&adjustments), c.default, "{}", c.id);
        }
    }

    #[test]
    fn point_color_defaults_match_point_color_default() {
        let point = PointColor::default();
        for c in POINT_COLOR_CONTROLS {
            assert_eq!(c.get(&point), c.default, "{}", c.id);
        }
    }

    #[test]
    fn ids_are_unique() {
        let ids: Vec<&str> = all_controls()
            .map(|c| c.id)
            .chain(LOCAL_CONTROLS.iter().map(|c| c.id))
            .chain(POINT_COLOR_CONTROLS.iter().map(|c| c.id))
            .collect();
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[i + 1..].contains(id), "{id}");
        }
    }
}
