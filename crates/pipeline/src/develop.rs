// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The fused per-pixel develop stage: camera RGB in, display-linear RGB in
//! the output primaries out. Masks' local adjustments join the global ones
//! at each step. See docs/PIPELINE.md for the order of operations.

use crate::band::PointOp;
use crate::camera::WORKING;
use crate::color::{ColorAdjust, DEHAZE_SATURATION, smoothstep};
use crate::masks::{LocalAdjust, LocalAmounts};
use crate::plan::DevelopPlan;
use crate::tone::{BASELINE_EXPOSURE, MID_GREY, ToneLuts};
use photonation_color::{ColorSpace, Mat3, Oklab, apply3, bradford, xy_to_xyz};
use photonation_develop::Settings;

/// Haze level removed (or added) at Dehaze ±100, in scene-linear units.
const MAX_HAZE: f32 = 0.03;
/// Exposure change Highlights/Shadows apply at ±100, stops.
const MAX_TONE_SHIFT_EV: f32 = 1.5;
/// Stops the red/blue balance (Temperature) or green (Tint) moves at local ±100.
const MAX_LOCAL_WHITE_BALANCE_EV: f32 = 1.0;
/// Change of the scene-contrast exponent around mid-grey at local Contrast ±100.
const MAX_LOCAL_CONTRAST: f32 = 0.5;
/// Overlapping masks add up; past these totals more masks change nothing.
const MAX_LOCAL_EXPOSURE_EV: f32 = 10.0;
const MAX_TONE_AMOUNT: f32 = 2.0;

pub struct Develop {
    camera_to_working: [[f32; 3]; 3],
    working_luminance: [f32; 3],
    haze: f32,
    highlights: f32,
    shadows: f32,
    tone: ToneLuts,
    color: Option<ColorAdjust>,
    working_to_output: [[f32; 3]; 3],
    output_luminance: [f32; 3],
    local: Option<LocalAdjust>,
    /// For local saturation.
    oklab: Oklab,
}

#[inline(always)]
fn dot(weights: &[f32; 3], rgb: &[f32; 3]) -> f32 {
    weights[0] * rgb[0] + weights[1] * rgb[1] + weights[2] * rgb[2]
}

impl Develop {
    /// `file_exposure` is the stops the raw file itself asks for (DNG BaselineExposure).
    pub fn new(
        s: &Settings,
        camera_to_working: &Mat3,
        file_exposure: f32,
        output: ColorSpace,
    ) -> Develop {
        let exposure = 2f64.powf((s.exposure + BASELINE_EXPOSURE + file_exposure) as f64);
        let out = output.primaries();
        let adapt = bradford(xy_to_xyz(WORKING.white), xy_to_xyz(out.white));
        let working_to_output = out.from_xyz() * adapt * WORKING.to_xyz();
        Develop {
            camera_to_working: camera_to_working.scale(exposure).to_f32(),
            working_luminance: WORKING.luminance().map(|v| v as f32),
            haze: s.dehaze / 100.0 * MAX_HAZE,
            highlights: s.highlights / 100.0,
            shadows: s.shadows / 100.0,
            tone: ToneLuts::new(s),
            color: ColorAdjust::new(s, WORKING, None),
            working_to_output: working_to_output.to_f32(),
            output_luminance: out.luminance().map(|v| v as f32),
            local: None,
            oklab: Oklab::new(WORKING),
        }
    }

    /// Shows point colour `overlay`'s selection instead of the edit:
    /// colours it selects keep their colour, everything else turns grey.
    pub fn with_point_color_overlay(mut self, s: &Settings, overlay: Option<usize>) -> Develop {
        if overlay.is_some() {
            self.color = ColorAdjust::new(s, WORKING, overlay);
        }
        self
    }

    /// Adds the masks' local adjustments (`None`: no active masks).
    pub fn with_local_adjustments(mut self, local: Option<LocalAdjust>) -> Develop {
        self.local = local;
        self
    }

    /// The global steps as data. Local adjustments aren't part of it.
    pub fn plan(&self) -> DevelopPlan {
        DevelopPlan {
            camera_to_working: self.camera_to_working,
            working_luminance: self.working_luminance,
            haze: self.haze,
            highlights: self.highlights,
            shadows: self.shadows,
            mid_grey: MID_GREY as f32,
            max_tone_shift_ev: MAX_TONE_SHIFT_EV,
            tone: self.tone.plan(),
            color: self.color.as_ref().map(ColorAdjust::plan),
            working_to_output: self.working_to_output,
            output_luminance: self.output_luminance,
        }
    }

    /// Positive haze is subtracted smoothly (`x² / (x + h)` never goes
    /// negative); negative haze lays a neutral veil over the image.
    #[inline(always)]
    fn dehaze(rgb: [f32; 3], haze: f32) -> [f32; 3] {
        if haze > 0.0 {
            rgb.map(|v| {
                let v = v.max(0.0);
                v * v / (v + haze) * (1.0 + haze)
            })
        } else {
            let veil = -haze;
            rgb.map(|v| v * (1.0 - veil) + veil)
        }
    }

    /// Highlights/Shadows (global plus local, as fractions of ±100): an
    /// exposure shift weighted by where the pixel's luminance sits relative
    /// to mid-grey (in stops).
    #[inline(always)]
    fn highlights_shadows(&self, rgb: [f32; 3], highlights: f32, shadows: f32) -> [f32; 3] {
        let luminance = dot(&self.working_luminance, &rgb).max(1e-6);
        let stops = (luminance / MID_GREY as f32).log2();
        let shadow_weight = 1.0 - smoothstep(-4.0, 0.0, stops);
        let highlight_weight = smoothstep(-0.5, 2.5, stops);
        let shift = MAX_TONE_SHIFT_EV * (shadows * shadow_weight + highlights * highlight_weight);
        let gain = shift.exp2();
        rgb.map(|v| v * gain)
    }

    /// Local Temperature/Tint as channel gains in the working space (warmer:
    /// more red, less blue; magenta: less green), scaled so a neutral keeps
    /// its luminance, combined with local exposure.
    #[inline(always)]
    fn local_light(&self, rgb: [f32; 3], local: &LocalAmounts) -> [f32; 3] {
        let exposure = local
            .exposure
            .clamp(-MAX_LOCAL_EXPOSURE_EV, MAX_LOCAL_EXPOSURE_EV)
            .exp2();
        if local.temperature == 0.0 && local.tint == 0.0 {
            return rgb.map(|v| v * exposure);
        }
        let warm = local.temperature * MAX_LOCAL_WHITE_BALANCE_EV / 2.0;
        let magenta = local.tint * MAX_LOCAL_WHITE_BALANCE_EV;
        let balance = [warm.exp2(), (-magenta).exp2(), (-warm).exp2()];
        let scale = exposure / dot(&self.working_luminance, &balance);
        [0, 1, 2].map(|channel| rgb[channel] * balance[channel] * scale)
    }

    /// Local Contrast: steepens (or flattens) scene luminance around
    /// mid-grey in log space, keeping each pixel's colour.
    #[inline(always)]
    fn local_contrast(&self, rgb: [f32; 3], contrast: f32) -> [f32; 3] {
        let luminance = dot(&self.working_luminance, &rgb).max(1e-6);
        let exponent = (1.0 + contrast * MAX_LOCAL_CONTRAST).clamp(0.25, 2.0);
        let grey = MID_GREY as f32;
        let gain = grey * (luminance / grey).powf(exponent) / luminance;
        rgb.map(|v| v * gain)
    }

    /// Local Saturation: scales Oklab chroma, like the global slider.
    #[inline(always)]
    fn saturate(&self, rgb: [f32; 3], amount: f32) -> [f32; 3] {
        let mut lab = self.oklab.from_rgb(rgb.map(|v| v.max(0.0)));
        let gain = (1.0 + amount).max(0.0);
        lab[1] *= gain;
        lab[2] *= gain;
        self.oklab.to_rgb(lab)
    }

    /// Converts to the output primaries, pulling out-of-gamut colours toward
    /// their luminance instead of clipping channels independently (which
    /// would shift hue).
    #[inline(always)]
    fn to_output_gamut(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut out = apply3(&self.working_to_output, rgb);
        let luminance = dot(&self.output_luminance, &out).clamp(0.0, 1.0);
        let pull_toward_luminance =
            |out: [f32; 3], t: f32| out.map(|v| luminance + (v - luminance) * t);
        let min = out[0].min(out[1]).min(out[2]);
        if min < 0.0 {
            out = pull_toward_luminance(
                out,
                if luminance > 0.0 {
                    luminance / (luminance - min)
                } else {
                    0.0
                },
            );
        }
        let max = out[0].max(out[1]).max(out[2]);
        if max > 1.0 {
            out = pull_toward_luminance(
                out,
                if max > luminance {
                    (1.0 - luminance) / (max - luminance)
                } else {
                    0.0
                },
            );
        }
        out.map(|v| v.clamp(0.0, 1.0))
    }

    /// One pixel, with `local` the masks' summed adjustments there (`None`
    /// where no mask applies).
    #[cfg(test)]
    fn pixel(&self, camera_rgb: [f32; 3], local: Option<&LocalAmounts>) -> [f32; 3] {
        let working = match local {
            None => self.working_pixel::<false>(camera_rgb, &LocalAmounts::NONE),
            Some(local) => self.working_pixel::<true>(camera_rgb, local),
        };
        self.to_output_gamut(working)
    }

    /// Stage pixel `(x, y)` developed up to the output conversion: display
    /// linear, in the working space. What the colour pickers sample.
    pub fn working_at(&self, x: usize, y: usize, camera_rgb: [f32; 3]) -> [f32; 3] {
        match &self.local {
            None => self.working_pixel::<false>(camera_rgb, &LocalAmounts::NONE),
            Some(local) => self.working_pixel::<true>(camera_rgb, &local.at(x, y)),
        }
    }

    /// The develop steps for one pixel, before the output conversion.
    /// `MASKED` is false for renders without active masks: that copy is
    /// compiled separately with every local step removed, so unmasked
    /// renders cost what they did before masks existed. (Merely passing
    /// zero amounts is measurably slower: the compiler must keep
    /// `x + 0.0`, which isn't `x` for `-0.0`.)
    #[inline(always)]
    fn working_pixel<const MASKED: bool>(
        &self,
        camera_rgb: [f32; 3],
        local: &LocalAmounts,
    ) -> [f32; 3] {
        let mut rgb = apply3(&self.camera_to_working, camera_rgb);
        let (haze, highlights, shadows) = if MASKED {
            if local.exposure != 0.0 || local.temperature != 0.0 || local.tint != 0.0 {
                rgb = self.local_light(rgb, local);
            }
            let tone_amount = |v: f32| v.clamp(-MAX_TONE_AMOUNT, MAX_TONE_AMOUNT);
            (
                (self.haze + local.dehaze * MAX_HAZE)
                    .clamp(-MAX_TONE_AMOUNT * MAX_HAZE, MAX_TONE_AMOUNT * MAX_HAZE),
                tone_amount(self.highlights + local.highlights),
                tone_amount(self.shadows + local.shadows),
            )
        } else {
            (self.haze, self.highlights, self.shadows)
        };
        if haze != 0.0 {
            rgb = Self::dehaze(rgb, haze);
        }
        if highlights != 0.0 || shadows != 0.0 {
            rgb = self.highlights_shadows(rgb, highlights, shadows);
        }
        if MASKED && local.contrast != 0.0 {
            rgb = self.local_contrast(rgb, local.contrast);
        }
        rgb = [0, 1, 2].map(|channel| self.tone.eval(channel, rgb[channel]));
        if let Some(color) = &self.color {
            rgb = color.apply(rgb);
        }
        if MASKED {
            // Like the global slider, local Dehaze restores washed-out colour.
            let saturation = local.saturation + DEHAZE_SATURATION * local.dehaze;
            if saturation != 0.0 {
                rgb = self.saturate(rgb, saturation);
            }
        }
        rgb
    }
}

impl PointOp for Develop {
    fn apply(&self, y: usize, row: &mut [f32]) {
        let pixels = row.chunks_exact_mut(3);
        match &self.local {
            None => pixels.for_each(|pixel| {
                let working = self
                    .working_pixel::<false>([pixel[0], pixel[1], pixel[2]], &LocalAmounts::NONE);
                pixel.copy_from_slice(&self.to_output_gamut(working));
            }),
            Some(local) => pixels.enumerate().for_each(|(x, pixel)| {
                let working =
                    self.working_pixel::<true>([pixel[0], pixel[1], pixel[2]], &local.at(x, y));
                pixel.copy_from_slice(&self.to_output_gamut(working));
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pixel with no masks covering it.
    fn global(develop: &Develop, rgb: [f32; 3]) -> [f32; 3] {
        develop.pixel(rgb, None)
    }

    fn plain() -> Develop {
        Develop::new(&Settings::default(), &Mat3::IDENTITY, 0.0, ColorSpace::Srgb)
    }

    fn local(amounts: impl FnOnce(&mut LocalAmounts)) -> LocalAmounts {
        let mut local = LocalAmounts::NONE;
        amounts(&mut local);
        local
    }

    fn luma(rgb: [f32; 3]) -> f32 {
        0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
    }

    #[test]
    fn local_exposure_matches_global_exposure() {
        let global_stop = Develop::new(
            &Settings {
                exposure: 1.0,
                ..Settings::default()
            },
            &Mat3::IDENTITY,
            0.0,
            ColorSpace::Srgb,
        );
        let one_stop = local(|l| l.exposure = 1.0);
        for v in [0.01, 0.1, 0.3] {
            let a = global(&global_stop, [v; 3]);
            let b = plain().pixel([v; 3], Some(&one_stop));
            assert!((a[1] - b[1]).abs() < 1e-5, "{v}: {a:?} vs {b:?}");
        }
    }

    #[test]
    fn local_temperature_warms_and_tint_adds_magenta() {
        let develop = plain();
        let grey = [0.1f32; 3];
        let warm = develop.pixel(grey, Some(&local(|l| l.temperature = 0.6)));
        assert!(warm[0] > warm[2] + 0.02, "{warm:?}");
        let magenta = develop.pixel(grey, Some(&local(|l| l.tint = 0.6)));
        assert!(
            magenta[1] < magenta[0] && magenta[1] < magenta[2],
            "{magenta:?}"
        );
        // The balance shift keeps brightness roughly where it was.
        let neutral = global(&develop, grey);
        assert!(
            (luma(warm) - luma(neutral)).abs() < 0.03,
            "{warm:?} vs {neutral:?}"
        );
    }

    #[test]
    fn local_contrast_spreads_tones_around_mid_grey() {
        let develop = plain();
        let more = local(|l| l.contrast = 1.0);
        let spread = |amounts: Option<&LocalAmounts>| {
            develop.pixel([0.6; 3], amounts)[1] - develop.pixel([0.03; 3], amounts)[1]
        };
        assert!(spread(Some(&more)) > spread(None) + 0.02);
        let less = local(|l| l.contrast = -1.0);
        assert!(spread(Some(&less)) < spread(None) - 0.02);
    }

    #[test]
    fn local_saturation_minus_100_is_grey() {
        let out = plain().pixel([0.4, 0.1, 0.05], Some(&local(|l| l.saturation = -1.0)));
        assert!(
            (out[0] - out[1]).abs() < 2e-3 && (out[1] - out[2]).abs() < 2e-3,
            "{out:?}"
        );
    }

    #[test]
    fn piled_up_masks_stay_finite() {
        let extreme = LocalAmounts {
            exposure: 400.0,
            temperature: 32.0,
            tint: -32.0,
            contrast: 32.0,
            highlights: 32.0,
            shadows: -32.0,
            dehaze: -32.0,
            saturation: 32.0,
        };
        for v in [0.0, 0.01, 1.0, 50.0] {
            let out = plain().pixel([v; 3], Some(&extreme));
            assert!(
                out.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c)),
                "{out:?}"
            );
        }
    }

    #[test]
    fn neutral_stays_neutral() {
        let d = Develop::new(&Settings::default(), &Mat3::IDENTITY, 0.0, ColorSpace::Srgb);
        for v in [0.01, 0.1, 0.5, 1.0, 4.0] {
            let out = d.pixel([v; 3], None);
            assert!(
                (out[0] - out[1]).abs() < 1e-4 && (out[1] - out[2]).abs() < 1e-4,
                "{v}: {out:?}"
            );
        }
    }

    #[test]
    fn saturation_minus_100_is_grey() {
        let s = Settings {
            saturation: -100.0,
            ..Settings::default()
        };
        let d = Develop::new(&s, &Mat3::IDENTITY, 0.0, ColorSpace::Srgb);
        let out = d.pixel([0.4, 0.1, 0.05], None);
        assert!(
            (out[0] - out[1]).abs() < 2e-3 && (out[1] - out[2]).abs() < 2e-3,
            "{out:?}"
        );
    }

    #[test]
    fn dehaze_darkens_shadows_and_keeps_order() {
        let plain = Develop::new(&Settings::default(), &Mat3::IDENTITY, 0.0, ColorSpace::Srgb);
        let dehazed = Develop::new(
            &Settings {
                dehaze: 80.0,
                ..Settings::default()
            },
            &Mat3::IDENTITY,
            0.0,
            ColorSpace::Srgb,
        );
        assert!(global(&dehazed, [0.02; 3])[1] < global(&plain, [0.02; 3])[1]);
        assert!(global(&dehazed, [0.03; 3])[1] > global(&dehazed, [0.02; 3])[1]);
    }

    #[test]
    fn shadows_lift_dark_tones_more_than_bright() {
        let plain = Develop::new(&Settings::default(), &Mat3::IDENTITY, 0.0, ColorSpace::Srgb);
        let lifted = Develop::new(
            &Settings {
                shadows: 100.0,
                ..Settings::default()
            },
            &Mat3::IDENTITY,
            0.0,
            ColorSpace::Srgb,
        );
        let gain = |v: f32| global(&lifted, [v; 3])[1] - global(&plain, [v; 3])[1];
        assert!(gain(0.01) > 0.01);
        assert!(gain(1.0).abs() < 0.01);
    }
}
