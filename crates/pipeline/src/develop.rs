// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The fused per-pixel develop stage: camera RGB in, display-linear RGB in
//! the output primaries out. See docs/PIPELINE.md for the order of operations.

use crate::band::PointOp;
use crate::camera::WORKING;
use crate::color::{ColorAdjust, smoothstep};
use crate::tone::{BASELINE_EXPOSURE, MID_GREY, ToneLuts};
use photonation_color::{ColorSpace, Mat3, apply3, bradford, xy_to_xyz};
use photonation_develop::Settings;

/// Haze level removed (or added) at Dehaze ±100, in scene-linear units.
const MAX_HAZE: f32 = 0.03;
/// Exposure change Highlights/Shadows apply at ±100, stops.
const MAX_TONE_SHIFT_EV: f32 = 1.5;

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
            color: ColorAdjust::new(s, WORKING),
            working_to_output: working_to_output.to_f32(),
            output_luminance: out.luminance().map(|v| v as f32),
        }
    }

    /// Positive haze is subtracted smoothly (`x² / (x + h)` never goes
    /// negative); negative haze lays a neutral veil over the image.
    #[inline(always)]
    fn dehaze(&self, rgb: [f32; 3]) -> [f32; 3] {
        if self.haze > 0.0 {
            rgb.map(|v| {
                let v = v.max(0.0);
                v * v / (v + self.haze) * (1.0 + self.haze)
            })
        } else {
            let veil = -self.haze;
            rgb.map(|v| v * (1.0 - veil) + veil)
        }
    }

    /// Global Highlights/Shadows: an exposure shift weighted by where the
    /// pixel's luminance sits relative to mid-grey (in stops).
    #[inline(always)]
    fn highlights_shadows(&self, rgb: [f32; 3]) -> [f32; 3] {
        let luminance = dot(&self.working_luminance, &rgb).max(1e-6);
        let stops = (luminance / MID_GREY as f32).log2();
        let shadow_weight = 1.0 - smoothstep(-4.0, 0.0, stops);
        let highlight_weight = smoothstep(-0.5, 2.5, stops);
        let shift =
            MAX_TONE_SHIFT_EV * (self.shadows * shadow_weight + self.highlights * highlight_weight);
        let gain = shift.exp2();
        rgb.map(|v| v * gain)
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

    #[inline(always)]
    fn pixel(&self, camera_rgb: [f32; 3]) -> [f32; 3] {
        let mut rgb = apply3(&self.camera_to_working, camera_rgb);
        if self.haze != 0.0 {
            rgb = self.dehaze(rgb);
        }
        if self.highlights != 0.0 || self.shadows != 0.0 {
            rgb = self.highlights_shadows(rgb);
        }
        rgb = [0, 1, 2].map(|channel| self.tone.eval(channel, rgb[channel]));
        if let Some(color) = &self.color {
            rgb = color.apply(rgb);
        }
        self.to_output_gamut(rgb)
    }
}

impl PointOp for Develop {
    fn apply(&self, pixels: &mut [f32]) {
        for pixel in pixels.chunks_exact_mut(3) {
            let out = self.pixel([pixel[0], pixel[1], pixel[2]]);
            pixel.copy_from_slice(&out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_stays_neutral() {
        let d = Develop::new(&Settings::default(), &Mat3::IDENTITY, 0.0, ColorSpace::Srgb);
        for v in [0.01, 0.1, 0.5, 1.0, 4.0] {
            let out = d.pixel([v, v, v]);
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
        let out = d.pixel([0.4, 0.1, 0.05]);
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
        assert!(dehazed.pixel([0.02; 3])[1] < plain.pixel([0.02; 3])[1]);
        assert!(dehazed.pixel([0.03; 3])[1] > dehazed.pixel([0.02; 3])[1]);
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
        let gain = |v: f32| lifted.pixel([v; 3])[1] - plain.pixel([v; 3])[1];
        assert!(gain(0.01) > 0.01);
        assert!(gain(1.0).abs() < 0.01);
    }
}
