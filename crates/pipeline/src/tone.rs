// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Process V1 tone mapping: the base scene→display curve (with Whites,
//! Blacks and Contrast) composed with the user's tone curves into one
//! lookup table per channel.

use crate::curve::{Parametric, Spline};
use crate::plan::TonePlan;
use photonation_develop::Settings;

/// Exposure added to every image so a normally exposed raw lands on a
/// pleasant mid-grey (cameras leave ~3 stops of highlight headroom).
pub const BASELINE_EXPOSURE: f32 = 1.2;
/// Scene value (after exposure) the default curve maps to display white.
const WHITE_POINT: f64 = 3.0;
/// Contrast of the default curve.
const BASE_CONTRAST: f64 = 1.45;
pub const MID_GREY: f64 = 0.18;
/// Exponent of the perceptual domain curves and contrast operate in.
const PERCEPTUAL_GAMMA: f64 = 2.2;

const LUT_MIN_EV: f64 = -16.0;
const LUT_MAX_EV: f64 = 6.0;
const LUT_SIZE: usize = 4096;

/// Base curve: scene-linear → display-linear, both ≥ 0.
fn base_curve(s: &Settings) -> impl Fn(f64) -> f64 {
    let black_point = -(s.blacks as f64 / 100.0) * 0.006;
    let white = WHITE_POINT * 2f64.powf(-(s.whites as f64 / 100.0) * 1.5);
    let contrast = BASE_CONTRAST * 2f64.powf(s.contrast as f64 / 100.0);
    let pivot = MID_GREY.powf(1.0 / PERCEPTUAL_GAMMA);
    move |scene: f64| {
        let x = ((scene - black_point) / (1.0 - black_point)).max(0.0);
        // Extended Reinhard: linear toe, smooth shoulder reaching 1 at `white`.
        let display = (x * (1.0 + x / (white * white)) / (1.0 + x)).min(1.0);
        // Contrast S-curve around mid-grey in the perceptual domain.
        let v = display.powf(1.0 / PERCEPTUAL_GAMMA);
        let v = if v < pivot {
            pivot * (v / pivot).powf(contrast)
        } else {
            1.0 - (1.0 - pivot) * ((1.0 - v) / (1.0 - pivot)).powf(contrast)
        };
        v.powf(PERCEPTUAL_GAMMA)
    }
}

/// Per-channel scene-linear → display-linear curves, sampled in log2.
pub struct ToneLuts {
    channels: [Vec<f32>; 3],
}

impl ToneLuts {
    pub fn new(s: &Settings) -> ToneLuts {
        let base = base_curve(s);
        let curves = &s.tone_curve;
        let parametric = Parametric::new(&curves.parametric);
        let master = Spline::new(&curves.master);
        let channel_splines = [
            Spline::new(&curves.red),
            Spline::new(&curves.green),
            Spline::new(&curves.blue),
        ];
        let channels = std::array::from_fn(|channel| {
            (0..LUT_SIZE)
                .map(|i| {
                    let ev =
                        LUT_MIN_EV + (LUT_MAX_EV - LUT_MIN_EV) * i as f64 / (LUT_SIZE - 1) as f64;
                    let display = base(2f64.powf(ev));
                    let perceptual = display.powf(1.0 / PERCEPTUAL_GAMMA) as f32;
                    let shaped =
                        channel_splines[channel].eval(master.eval(parametric.eval(perceptual)));
                    (shaped as f64).powf(PERCEPTUAL_GAMMA) as f32
                })
                .collect()
        });
        ToneLuts { channels }
    }

    pub fn plan(&self) -> TonePlan {
        TonePlan {
            min_ev: LUT_MIN_EV as f32,
            max_ev: LUT_MAX_EV as f32,
            tables: self.channels.clone(),
        }
    }

    #[inline(always)]
    pub fn eval(&self, channel: usize, scene: f32) -> f32 {
        let lut = &self.channels[channel];
        const LUT_MIN: f32 = 1.0 / 65536.0; // 2^LUT_MIN_EV
        if scene <= LUT_MIN {
            return scene.max(0.0) * (lut[0] / LUT_MIN);
        }
        let position = (scene.log2() - LUT_MIN_EV as f32)
            * ((LUT_SIZE - 1) as f32 / (LUT_MAX_EV - LUT_MIN_EV) as f32);
        if position >= (LUT_SIZE - 1) as f32 {
            return lut[LUT_SIZE - 1];
        }
        let index = position as usize;
        let fraction = position - index as f32;
        lut[index] + (lut[index + 1] - lut[index]) * fraction
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonation_develop::PointCurve;

    fn luts(s: Settings) -> ToneLuts {
        ToneLuts::new(&s)
    }

    #[test]
    fn default_curve_is_monotonic_and_bounded() {
        let c = luts(Settings::default());
        let mut prev = -1.0;
        for i in 0..2000 {
            let x = i as f32 / 500.0;
            let y = c.eval(1, x);
            assert!(y >= prev - 1e-6 && (0.0..=1.0).contains(&y), "x={x} y={y}");
            prev = y;
        }
        assert_eq!(c.eval(1, 0.0), 0.0);
        assert!(c.eval(1, WHITE_POINT as f32) > 0.999);
    }

    #[test]
    fn mid_grey_stays_mid_grey() {
        let y = luts(Settings::default()).eval(1, MID_GREY as f32);
        assert!((0.12..0.2).contains(&y), "{y}");
    }

    #[test]
    fn contrast_steepens_around_mid_grey() {
        let span = |c: &ToneLuts| c.eval(1, 0.3) - c.eval(1, 0.1);
        let flat = luts(Settings::default());
        let steep = luts(Settings {
            contrast: 80.0,
            ..Settings::default()
        });
        assert!(span(&steep) > span(&flat));
    }

    #[test]
    fn red_curve_only_changes_red() {
        let mut s = Settings::default();
        s.tone_curve.red = PointCurve(vec![[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]]);
        let (plain, tinted) = (luts(Settings::default()), luts(s));
        assert!(tinted.eval(0, 0.18) > plain.eval(0, 0.18) + 0.02);
        assert_eq!(tinted.eval(1, 0.18), plain.eval(1, 0.18));
        assert_eq!(tinted.eval(2, 0.18), plain.eval(2, 0.18));
    }
}
