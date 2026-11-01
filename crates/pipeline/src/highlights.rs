// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Highlight reconstruction (process V2).
//!
//! After white balance a neutral has equal channels, but each channel
//! clips at its own level: its multiplier (the smallest is 1, usually
//! green's). Process V1 clips every channel at 1, which keeps blown areas
//! neutral but throws away what red and blue recorded above that. V2 keeps
//! each channel up to its own clip and rebuilds the clipped ones per pixel:
//! a clipped channel is at least what it shows, and is raised toward the
//! channels that still hold detail. Clipped channels can't be told apart,
//! so they also rise to the highest clip among them: green clipped at 1
//! next to blue clipped at 1.5 is not greener-than-blue information.
//!
//! A clipped pixel's brightness is recovered, but its colour is a guess:
//! in a sky that clips one channel after another, the guesses form bands
//! of colour. So where channels clip, the colour (distance from the
//! pixel's grey) is held to what clipping every channel at 1 left of it,
//! as V1 does, while the brightness and hue come from the reconstruction.
//! Neutral gradients come out exact up to the highest clip, colours that
//! don't clip keep all of theirs, and a pixel clipped in every channel
//! turns neutral at the brightest clip instead of magenta.

use crate::plan::HighlightPlan;

/// Fraction of its clip level where a channel starts to count as clipped.
/// Below it a channel is trusted fully; the soft zone keeps the result
/// continuous and absorbs demosaicing near clipped edges.
const SOFT_CLIP_START: f32 = 0.9;

/// Rebuilds clipped channels of white-balanced camera RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HighlightReconstruction {
    /// Per channel: the white-balanced value of a clipped sample.
    clip: [f32; 3],
}

impl HighlightReconstruction {
    /// `clip`: each channel's clip level (its white-balance multiplier).
    pub fn new(clip: [f32; 3]) -> HighlightReconstruction {
        HighlightReconstruction { clip }
    }

    pub fn plan(&self) -> HighlightPlan {
        HighlightPlan {
            clip: self.clip,
            soft_start: SOFT_CLIP_START,
        }
    }

    #[inline(always)]
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        if (0..3).all(|c| rgb[c] <= SOFT_CLIP_START * self.clip[c]) {
            return rgb;
        }
        let clipped = [0, 1, 2].map(|c| {
            let clip = self.clip[c];
            crate::color::smoothstep(SOFT_CLIP_START * clip, clip, rgb[c])
        });
        // What the trusted channels say about the pixel's brightness; with
        // every channel clipped, the brightest is the best lower bound.
        let trust = clipped.map(|k| 1.0 - k);
        let trust_sum = trust[0] + trust[1] + trust[2];
        let reference = if trust_sum > 1e-6 {
            (trust[0] * rgb[0] + trust[1] * rgb[1] + trust[2] * rgb[2]) / trust_sum
        } else {
            rgb[0].max(rgb[1]).max(rgb[2])
        };
        let clipped_floor = (0..3).fold(0.0f32, |floor, c| floor.max(clipped[c] * rgb[c]));
        let target = reference.max(clipped_floor);
        let raised = [0, 1, 2].map(|c| rgb[c] + clipped[c] * (target - rgb[c]).max(0.0));

        let (grey, colour) = grey_and_colour(raised);
        let colour_norm = norm(colour);
        if colour_norm <= 1e-6 {
            return raised;
        }
        let (_, common_clip_colour) = grey_and_colour(rgb.map(|v| v.min(1.0)));
        let limit = (norm(common_clip_colour) / colour_norm).min(1.0);
        let most_clipped = clipped[0].max(clipped[1]).max(clipped[2]);
        let keep = 1.0 - most_clipped * (1.0 - limit);
        colour.map(|v| grey + v * keep)
    }
}

/// A pixel's grey (its mean) and its distance from it.
#[inline(always)]
fn grey_and_colour(rgb: [f32; 3]) -> (f32, [f32; 3]) {
    let grey = (rgb[0] + rgb[1] + rgb[2]) / 3.0;
    (grey, rgb.map(|v| v - grey))
}

#[inline(always)]
fn norm(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Daylight-like multipliers: green clips first, then blue, then red.
    fn daylight() -> HighlightReconstruction {
        HighlightReconstruction::new([2.0, 1.0, 1.5])
    }

    #[test]
    fn unclipped_pixels_are_unchanged() {
        let r = daylight();
        for rgb in [[0.2, 0.5, 0.1], [1.7, 0.8, 1.2], [0.0; 3], [1.0, 0.85, 1.3]] {
            assert_eq!(r.apply(rgb), rgb);
        }
    }

    #[test]
    fn neutral_gradient_survives_up_to_the_highest_clip() {
        let r = daylight();
        for step in 0..=40 {
            let level = 0.5 + step as f32 * 0.0375; // 0.5 ..= 2.0
            // What the sensor recorded: each channel stops at its clip.
            let recorded = [level.min(2.0), level.min(1.0), level.min(1.5)];
            let out = r.apply(recorded);
            for value in out {
                assert!((value - level).abs() < 1e-5, "{level}: {out:?}");
            }
        }
    }

    #[test]
    fn fully_clipped_pixels_are_neutral() {
        let out = daylight().apply([2.0, 1.0, 1.5]);
        assert_eq!(out, [2.0; 3]);
    }

    fn mean(rgb: [f32; 3]) -> f32 {
        (rgb[0] + rgb[1] + rgb[2]) / 3.0
    }

    #[test]
    fn clipped_pixels_never_get_darker() {
        let r = daylight();
        for rgb in [
            [2.0, 0.9, 0.3],
            [0.5, 1.0, 1.2],
            [1.95, 1.0, 0.2],
            [0.1, 0.95, 1.5],
        ] {
            let out = r.apply(rgb);
            assert!(mean(out) >= mean(rgb) - 1e-6, "{rgb:?} → {out:?}");
        }
    }

    #[test]
    fn clipped_channels_rise_together() {
        // A bright sky: green and blue clipped, red not. Green must not
        // stay below blue only because its clip is lower (that turns pink);
        // with nothing left to say what colour it was, it turns grey.
        let out = daylight().apply([1.2, 1.0, 1.5]);
        for value in out {
            assert!((value - 1.4).abs() < 1e-5, "{out:?}");
        }
    }

    #[test]
    fn clipped_colours_keep_their_hue_and_no_more_colour_than_v1() {
        // A sunset whose red clipped: brighter than V1's, still orange,
        // and no more colourful than V1's (every channel clipped at 1).
        let v1 = [1.0, 0.9, 0.3];
        let out = daylight().apply([2.0, 0.9, 0.3]);
        assert!(out[0] > out[1] && out[1] > out[2], "{out:?}");
        assert!(mean(out) > mean(v1));
        let spread = |rgb: [f32; 3]| norm(grey_and_colour(rgb).1);
        assert!(spread(out) <= spread(v1) + 1e-5, "{out:?}");
    }

    #[test]
    fn colours_that_do_not_clip_keep_all_of_it() {
        // Red above 1 but below its own clip: V1 clipped it, V2 keeps it.
        let rgb = [1.6, 0.5, 0.2];
        assert_eq!(daylight().apply(rgb), rgb);
    }

    #[test]
    fn result_is_continuous_across_the_soft_zone() {
        let r = daylight();
        let mut previous = r.apply([1.4, 0.85, 1.2]);
        for step in 1..=200 {
            let green = 0.85 + step as f32 * 0.001;
            let out = r.apply([1.4, green, 1.2]);
            for c in 0..3 {
                assert!((out[c] - previous[c]).abs() < 0.01, "{green}: {out:?}");
            }
            previous = out;
        }
    }
}
