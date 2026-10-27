// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Colour adjustments in Oklab: colour mixer (HSL), colour grading,
//! vibrance, saturation and point colours. Oklab keeps hue and lightness perceptually
//! separate, so each slider does one thing.

use crate::point_color::PointColors;
use photonation_color::{Oklab, Primaries};
use photonation_develop::{ColorGrading, GradingWheel, HslAdjustment, Settings};

/// Oklab hue (degrees) at the centre of each mixer band: red, orange,
/// yellow, green, aqua, blue, purple, magenta.
const BAND_HUES: [f32; 8] = [25.0, 55.0, 100.0, 142.0, 195.0, 255.0, 300.0, 340.0];
/// Hue shift at ±100, degrees.
const MAX_HUE_SHIFT: f32 = 30.0;
/// Lightness change at ±100 for a fully saturated colour.
const MAX_BAND_LIGHTNESS: f32 = 0.25;
/// Grading wheel hue 0 (red) in Oklab hue degrees.
const GRADING_HUE_OFFSET: f32 = 25.0;
/// Chroma added by a grading wheel at saturation 100.
const MAX_GRADING_CHROMA: f32 = 0.06;
/// Lightness offset of a grading wheel at ±100.
const MAX_GRADING_LIGHTNESS: f32 = 0.15;
/// Saturation Dehaze restores per unit (washed-out colour comes back with the contrast).
pub const DEHAZE_SATURATION: f32 = 0.3;

#[inline(always)]
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A grading wheel as an Oklab offset.
#[derive(Clone, Copy, Default)]
struct WheelOffset {
    a: f32,
    b: f32,
    lightness: f32,
}

impl WheelOffset {
    fn new(wheel: &GradingWheel) -> WheelOffset {
        let angle = (wheel.hue + GRADING_HUE_OFFSET).to_radians();
        let chroma = wheel.saturation / 100.0 * MAX_GRADING_CHROMA;
        WheelOffset {
            a: chroma * angle.cos(),
            b: chroma * angle.sin(),
            lightness: wheel.luminance / 100.0 * MAX_GRADING_LIGHTNESS,
        }
    }
}

struct Grading {
    shadows: WheelOffset,
    midtones: WheelOffset,
    highlights: WheelOffset,
    global: WheelOffset,
    /// Lightness where shadows hand over to highlights.
    pivot: f32,
    /// Half-width of the transition around the pivot.
    transition: f32,
}

impl Grading {
    fn new(g: &ColorGrading) -> Grading {
        Grading {
            shadows: WheelOffset::new(&g.shadows),
            midtones: WheelOffset::new(&g.midtones),
            highlights: WheelOffset::new(&g.highlights),
            global: WheelOffset::new(&g.global),
            pivot: 0.5 - 0.2 * g.balance / 100.0,
            transition: 0.15 + 0.35 * g.blending / 100.0,
        }
    }

    fn apply(&self, lab: &mut [f32; 3]) {
        let toward_highlights = smoothstep(
            self.pivot - self.transition,
            self.pivot + self.transition,
            lab[0],
        );
        let shadow_weight = (1.0 - toward_highlights).powi(2);
        let highlight_weight = toward_highlights.powi(2);
        let midtone_weight = 1.0 - shadow_weight - highlight_weight;
        for (offset, weight) in [
            (self.shadows, shadow_weight),
            (self.midtones, midtone_weight),
            (self.highlights, highlight_weight),
            (self.global, 1.0),
        ] {
            lab[0] += offset.lightness * weight;
            lab[1] += offset.a * weight;
            lab[2] += offset.b * weight;
        }
    }
}

/// Blended mixer adjustment for a hue: linear between the two nearest bands.
fn band_adjustment(bands: &[HslAdjustment; 8], hue: f32) -> HslAdjustment {
    let upper = BAND_HUES
        .iter()
        .position(|&center| center > hue)
        .unwrap_or(0);
    let lower = (upper + 7) % 8;
    let span = (BAND_HUES[upper] - BAND_HUES[lower]).rem_euclid(360.0);
    let t = smoothstep(0.0, 1.0, (hue - BAND_HUES[lower]).rem_euclid(360.0) / span);
    let mix = |a: f32, b: f32| a + (b - a) * t;
    let (lo, hi) = (bands[lower], bands[upper]);
    HslAdjustment {
        hue: mix(lo.hue, hi.hue),
        saturation: mix(lo.saturation, hi.saturation),
        luminance: mix(lo.luminance, hi.luminance),
    }
}

pub struct ColorAdjust {
    oklab: Oklab,
    mixer: Option<[HslAdjustment; 8]>,
    grading: Option<Grading>,
    saturation: f32,
    vibrance: f32,
    points: Option<PointColors>,
}

impl ColorAdjust {
    /// `None` when every colour slider is neutral (the stage is skipped).
    /// `point_color_overlay` shows that point colour's selection instead
    /// of the edit.
    pub fn new(
        s: &Settings,
        working: Primaries,
        point_color_overlay: Option<usize>,
    ) -> Option<ColorAdjust> {
        let mixer = (!s.color_mixer.is_neutral()).then(|| s.color_mixer.bands());
        let grading = (!s.color_grading.is_neutral()).then(|| Grading::new(&s.color_grading));
        // Dehaze also restores the saturation haze washes out.
        let saturation = s.saturation / 100.0 + DEHAZE_SATURATION * s.dehaze / 100.0;
        let vibrance = s.vibrance / 100.0;
        let points = PointColors::new(&s.point_colors, point_color_overlay);
        if mixer.is_none()
            && grading.is_none()
            && saturation == 0.0
            && vibrance == 0.0
            && points.is_none()
        {
            return None;
        }
        Some(ColorAdjust {
            oklab: Oklab::new(working),
            mixer,
            grading,
            saturation,
            vibrance,
            points,
        })
    }

    #[inline]
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut lab = self.oklab.from_rgb(rgb.map(|v| v.max(0.0)));
        if let Some(bands) = &self.mixer {
            apply_mixer(bands, &mut lab);
        }
        if let Some(grading) = &self.grading {
            grading.apply(&mut lab);
        }
        let chroma = lab[1].hypot(lab[2]);
        let gain =
            (1.0 + self.saturation) * (1.0 + self.vibrance * (1.0 - smoothstep(0.0, 0.2, chroma)));
        lab[1] *= gain.max(0.0);
        lab[2] *= gain.max(0.0);
        if let Some(points) = &self.points {
            points.apply(&mut lab);
        }
        self.oklab.to_rgb(lab)
    }
}

/// The mixer band (index into `ColorMixer::bands`) whose centre is
/// nearest to an Oklab hue in degrees.
pub fn nearest_mixer_band(hue: f32) -> usize {
    let distance = |center: f32| {
        let difference = (hue - center).rem_euclid(360.0);
        difference.min(360.0 - difference)
    };
    (0..BAND_HUES.len())
        .min_by(|&a, &b| distance(BAND_HUES[a]).total_cmp(&distance(BAND_HUES[b])))
        .unwrap_or(0)
}

fn apply_mixer(bands: &[HslAdjustment; 8], lab: &mut [f32; 3]) {
    let chroma = lab[1].hypot(lab[2]);
    if chroma < 1e-5 {
        return;
    }
    let hue = lab[2].atan2(lab[1]).to_degrees().rem_euclid(360.0);
    let adjustment = band_adjustment(bands, hue);
    let new_hue = (hue + adjustment.hue / 100.0 * MAX_HUE_SHIFT).to_radians();
    let new_chroma = chroma * (1.0 + adjustment.saturation / 100.0).max(0.0);
    // Lightness changes only make sense for coloured pixels; greys stay put.
    lab[0] += adjustment.luminance / 100.0 * MAX_BAND_LIGHTNESS * smoothstep(0.0, 0.1, chroma);
    lab[1] = new_chroma * new_hue.cos();
    lab[2] = new_chroma * new_hue.sin();
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonation_develop::ColorMixer;

    const WORKING: Primaries = Primaries::REC2020;

    #[test]
    fn neutral_settings_skip_the_stage() {
        assert!(ColorAdjust::new(&Settings::default(), WORKING, None).is_none());
    }

    #[test]
    fn band_weights_hit_centres() {
        let mut bands = ColorMixer::default().bands();
        bands[3].saturation = 100.0; // green
        assert!((band_adjustment(&bands, 142.0).saturation - 100.0).abs() < 1e-3);
        assert!(band_adjustment(&bands, 25.0).saturation.abs() < 1e-3);
        // Wraps around between magenta and red.
        assert!(band_adjustment(&bands, 5.0).saturation.abs() < 1e-3);
    }

    #[test]
    fn nearest_band_wraps_around_red() {
        assert_eq!(nearest_mixer_band(25.0), 0);
        assert_eq!(nearest_mixer_band(10.0), 0);
        assert_eq!(nearest_mixer_band(345.0), 7);
        assert_eq!(nearest_mixer_band(250.0), 5);
    }

    #[test]
    fn mixer_desaturates_only_its_band() {
        let mut s = Settings::default();
        s.color_mixer.blue.saturation = -100.0;
        let adjust = ColorAdjust::new(&s, WORKING, None).expect("active");
        let ok = Oklab::new(WORKING);
        let chroma = |rgb| {
            let lab = ok.from_rgb(rgb);
            lab[1].hypot(lab[2])
        };
        let blue = [0.05, 0.1, 0.5];
        let red = [0.5, 0.08, 0.05];
        assert!(chroma(adjust.apply(blue)) < 0.01);
        assert!((chroma(adjust.apply(red)) - chroma(red)).abs() < 1e-3);
    }

    #[test]
    fn grading_tints_shadows_more_than_highlights() {
        let mut s = Settings::default();
        s.color_grading.shadows = GradingWheel {
            hue: 220.0,
            saturation: 80.0,
            luminance: 0.0,
        };
        let adjust = ColorAdjust::new(&s, WORKING, None).expect("active");
        let ok = Oklab::new(WORKING);
        let shift = |grey: f32| {
            let lab = ok.from_rgb(adjust.apply([grey; 3]));
            lab[1].hypot(lab[2])
        };
        assert!(shift(0.01) > shift(0.8) + 0.01);
    }
}
