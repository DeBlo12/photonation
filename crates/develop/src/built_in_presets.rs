// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The presets that come with Photonation. Each sets the whole look
//! (contrast, presence, colour, curves, mixer and grading) and leaves the
//! photo's own corrections alone: white balance, exposure, highlights,
//! shadows, whites, blacks, dehaze, sharpening and the tools. So switching
//! from one to another replaces the look instead of piling up.

use crate::{GradingWheel, PointCurve, Preset, Settings, SettingsPart, SettingsParts};

/// The heading the built-in presets are listed under.
pub const BUILT_IN_PRESET_GROUP: &str = "Photonation";

/// The parts every built-in preset sets.
fn look_parts() -> SettingsParts {
    [
        SettingsPart::Contrast,
        SettingsPart::Texture,
        SettingsPart::Clarity,
        SettingsPart::Vibrance,
        SettingsPart::Saturation,
        SettingsPart::ToneCurve,
        SettingsPart::ColorMixer,
        SettingsPart::ColorGrading,
    ]
    .into()
}

fn curve(points: &[[f32; 2]]) -> PointCurve {
    PointCurve(points.to_vec())
}

fn wheel(hue: f32, saturation: f32) -> GradingWheel {
    GradingWheel {
        hue,
        saturation,
        ..GradingWheel::default()
    }
}

/// Raised blacks and lowered whites: the faded look of matte prints.
fn matte(lift: f32) -> PointCurve {
    curve(&[
        [0.0, lift],
        [0.3, 0.3],
        [0.7, 0.71],
        [1.0, 1.0 - lift * 0.6],
    ])
}

pub fn built_in_presets() -> Vec<Preset> {
    let preset = |name: &str, settings: Settings| {
        Preset::new(name, BUILT_IN_PRESET_GROUP, &settings, look_parts())
    };
    let base = Settings::default;

    let mut warm = Settings {
        contrast: -5.0,
        vibrance: -8.0,
        ..base()
    };
    warm.tone_curve.master = matte(0.05);
    warm.color_grading.shadows = wheel(195.0, 16.0);
    warm.color_grading.highlights = wheel(45.0, 24.0);

    let mut cool = Settings {
        vibrance: 5.0,
        ..base()
    };
    cool.color_grading.shadows = wheel(220.0, 20.0);
    cool.color_grading.highlights = wheel(205.0, 12.0);

    let mut punchy = Settings {
        contrast: 30.0,
        clarity: 12.0,
        vibrance: 12.0,
        ..base()
    };
    punchy.tone_curve.master = curve(&[[0.0, 0.0], [0.25, 0.21], [0.75, 0.8], [1.0, 1.0]]);

    let mut faded = Settings {
        contrast: -8.0,
        saturation: -12.0,
        ..base()
    };
    faded.tone_curve.master = matte(0.07);

    let monochrome = Settings {
        saturation: -100.0,
        contrast: 10.0,
        ..base()
    };
    // Darker skies, brighter skin, as a red-orange filter would give.
    let mut monochrome_contrast = Settings {
        saturation: -100.0,
        contrast: 40.0,
        clarity: 18.0,
        ..base()
    };
    let mixer = &mut monochrome_contrast.color_mixer;
    mixer.red.luminance = 10.0;
    mixer.orange.luminance = 15.0;
    mixer.yellow.luminance = 10.0;
    mixer.aqua.luminance = -20.0;
    mixer.blue.luminance = -40.0;
    let mut monochrome_soft = Settings {
        saturation: -100.0,
        contrast: -15.0,
        ..base()
    };
    monochrome_soft.tone_curve.master = matte(0.06);

    vec![
        preset("Natural", base()),
        preset(
            "Vivid",
            Settings {
                contrast: 15.0,
                clarity: 5.0,
                vibrance: 35.0,
                saturation: 5.0,
                ..base()
            },
        ),
        preset("Punchy", punchy),
        preset(
            "Soft",
            Settings {
                contrast: -20.0,
                texture: -8.0,
                clarity: -12.0,
                vibrance: 5.0,
                ..base()
            },
        ),
        preset("Faded", faded),
        preset("Warm Film", warm),
        preset("Cool", cool),
        preset("Black & White", monochrome),
        preset("Black & White Contrast", monochrome_contrast),
        preset("Black & White Soft", monochrome_soft),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_ins_are_valid_and_keep_the_photos_corrections() {
        let presets = built_in_presets();
        let photo = Settings {
            exposure: 0.5,
            shadows: 30.0,
            contrast: 99.0,
            ..Settings::default()
        };
        for preset in &presets {
            assert_eq!(
                preset.settings,
                preset.settings.sanitized(),
                "{}",
                preset.name
            );
            let applied = preset.applied_to(&photo);
            assert_eq!((applied.exposure, applied.shadows), (0.5, 30.0));
            assert_ne!(applied.contrast, 99.0, "{} sets the look", preset.name);
        }
        assert_eq!(
            presets[0].applied_to(&photo).contrast,
            0.0,
            "Natural resets it"
        );
    }
}
