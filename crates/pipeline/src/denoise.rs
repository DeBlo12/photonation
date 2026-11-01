// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Noise reduction: wavelet shrinkage of luminance and colour.
//!
//! The developed pixel goes to a square-root domain (where noise is
//! roughly even from shadows to highlights) and splits into luminance and
//! two colour differences. An à trous wavelet (B3 spline, holes doubling
//! per level) separates each into detail at 1, 2, 4, … pixels and a
//! coarse rest; each level's detail shrinks toward zero by a threshold set
//! from the sliders and the noise expected at that level, and the levels
//! add back up. Grain lives in the fine luminance levels, colour blotches
//! across all the colour levels.
//!
//! The thresholds follow the source's pixel size, so a half-size preview
//! removes the same noise as a full-size export. Every pixel's sums run in
//! a fixed order with edges clamped, so bands of any height give the same
//! result, and the GPU (`crates/gpu/src/denoise.wgsl`) transcribes it; keep
//! the two in step.

use crate::band::{Band, Stage, clamp_index};
use crate::plan::DenoisePlan;
use photonation_develop::NoiseReduction;
use std::ops::Range;

/// Wavelet levels at full resolution: detail up to 2⁵ = 32 pixels across.
const LEVELS: usize = 5;
/// The B3 spline's taps, at offsets −2..2 times the level's step.
pub const KERNEL: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];
/// Luminance noise per level of detail, for grain of deviation 1 per
/// pixel. White noise would give 0.89, 0.20, 0.086, …; demosaicing spreads
/// grain over a few pixels, which leaves more of it at levels 1 and 2.
const LUMINANCE_LEVELS: [f32; LEVELS] = [0.80, 0.40, 0.18, 0.08, 0.04];
/// Colour noise per level of full-resolution detail, relative to the finest:
/// demosaicing leaves it blotchy, so it stays strong at coarse levels.
const COLOR_NOISE: [f32; LEVELS] = [1.0, 0.7, 0.5, 0.35, 0.25];
/// Luminance noise deviation (√ domain, per full-resolution pixel) that
/// Luminance 100 removes; strong high-ISO grain.
const LUMINANCE_NOISE: f32 = 0.07;
/// Colour threshold at the finest level at Color 100.
const COLOR_THRESHOLD: f32 = 0.06;
/// Share of the luminance detail smoothing removed that Detail 100 keeps.
const MAX_DETAIL_KEPT: f32 = 0.6;

pub struct Denoise {
    plan: DenoisePlan,
}

impl Denoise {
    /// `None` when the sliders are off. `scale` is how many sensor pixels
    /// one source pixel spans across (2 after half-size demosaicing).
    pub fn new(settings: &NoiseReduction, scale: usize) -> Option<Denoise> {
        if !settings.is_active() {
            return None;
        }
        // A half-size pixel averages four, so it starts a level up and
        // carries half the luminance noise.
        let skipped = scale.max(1).ilog2() as usize;
        let luminance = settings.luminance / 100.0 * LUMINANCE_NOISE / scale.max(1) as f32;
        let color = settings.color / 100.0 * COLOR_THRESHOLD;
        let thresholds = (0..LEVELS.saturating_sub(skipped))
            .map(|level| {
                [
                    luminance * LUMINANCE_LEVELS[level],
                    color * COLOR_NOISE[level + skipped],
                ]
            })
            .collect();
        Some(Denoise {
            plan: DenoisePlan {
                thresholds,
                detail_kept: settings.detail / 100.0 * MAX_DETAIL_KEPT,
            },
        })
    }

    pub fn plan(&self) -> DenoisePlan {
        self.plan.clone()
    }

    /// Rows each side a band needs: every level's kernel reaches two steps.
    fn halo(&self) -> usize {
        (0..self.plan.thresholds.len())
            .map(|level| 2 << level)
            .sum()
    }
}

/// Square-root luminance and colour differences of linear RGB.
#[inline]
pub fn to_opponent(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb.map(|v| v.max(0.0).sqrt());
    [(r + 2.0 * g + b) * 0.25, b - g, r - g]
}

/// Inverse of [`to_opponent`].
#[inline]
pub fn from_opponent([luminance, blue, red]: [f32; 3]) -> [f32; 3] {
    let g = luminance - 0.25 * (blue + red);
    [red + g, g, blue + g].map(|v| v.max(0.0) * v.max(0.0))
}

/// `detail` shrunk toward zero: barely below `threshold`, hardly above it.
#[inline]
pub fn shrink(detail: f32, threshold: f32) -> f32 {
    if threshold <= 0.0 {
        return detail;
    }
    let energy = detail * detail;
    detail * energy / (energy + threshold * threshold)
}

/// A plane of opponent pixels, `width × rows`.
struct Plane {
    pixels: Vec<[f32; 3]>,
    width: usize,
    rows: usize,
}

impl Plane {
    /// The B3 smoothing at `step`, along rows then columns, edges clamped.
    fn smoothed(&self, step: usize) -> Plane {
        let (width, rows) = (self.width, self.rows);
        let offsets = [-2isize, -1, 0, 1, 2].map(|k| k * step as isize);
        let weigh = |samples: [[f32; 3]; 5]| {
            let mut sum = [0.0f32; 3];
            for (sample, weight) in samples.iter().zip(KERNEL) {
                for channel in 0..3 {
                    sum[channel] += weight * sample[channel];
                }
            }
            sum
        };
        let mut along_rows = vec![[0.0f32; 3]; self.pixels.len()];
        for y in 0..rows {
            let row = &self.pixels[y * width..][..width];
            for x in 0..width {
                along_rows[y * width + x] =
                    weigh(offsets.map(|offset| row[clamp_index(x as isize + offset, width)]));
            }
        }
        let mut pixels = vec![[0.0f32; 3]; self.pixels.len()];
        for y in 0..rows {
            let sources = offsets.map(|offset| clamp_index(y as isize + offset, rows) * width);
            for x in 0..width {
                pixels[y * width + x] = weigh(sources.map(|start| along_rows[start + x]));
            }
        }
        Plane {
            pixels,
            width,
            rows,
        }
    }
}

impl Stage for Denoise {
    fn size(&self, input: (usize, usize)) -> (usize, usize) {
        input
    }

    fn input_rows(&self, out: Range<usize>, input_height: usize) -> Range<usize> {
        let halo = self.halo();
        out.start.saturating_sub(halo)..(out.end + halo).min(input_height)
    }

    fn process(&self, input: Band, out: Range<usize>) -> Band {
        let mut approximation = Plane {
            pixels: input
                .data
                .chunks_exact(3)
                .map(|p| to_opponent([p[0], p[1], p[2]]))
                .collect(),
            width: input.width,
            rows: input.rows,
        };
        let mut result = vec![[0.0f32; 3]; approximation.pixels.len()];
        let kept = self.plan.detail_kept;
        for (level, [luminance, color]) in self.plan.thresholds.iter().enumerate() {
            let coarser = approximation.smoothed(1 << level);
            for ((sum, fine), coarse) in result
                .iter_mut()
                .zip(&approximation.pixels)
                .zip(&coarser.pixels)
            {
                let detail = [0, 1, 2].map(|channel| fine[channel] - coarse[channel]);
                let smoothed = shrink(detail[0], *luminance);
                sum[0] += smoothed + kept * (detail[0] - smoothed);
                sum[1] += shrink(detail[1], *color);
                sum[2] += shrink(detail[2], *color);
            }
            approximation = coarser;
        }

        let mut band = Band::new(out.start, input.width, out.len());
        for y in out {
            let first = (y - input.y0) * input.width;
            let target = band.row_mut(y);
            for x in 0..input.width {
                let sum = result[first + x];
                let coarse = approximation.pixels[first + x];
                let rgb = from_opponent([0, 1, 2].map(|channel| sum[channel] + coarse[channel]));
                target[x * 3..x * 3 + 3].copy_from_slice(&rgb);
            }
        }
        band
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (usize, usize) = (96, 72);

    fn settings(luminance: f32, color: f32) -> NoiseReduction {
        NoiseReduction {
            luminance,
            color,
            ..NoiseReduction::default()
        }
    }

    /// Small deterministic noise, −amount..amount.
    fn noise(x: usize, y: usize, seed: u64, amount: f32) -> f32 {
        let mut hash = (x as u64) << 40 | (y as u64) << 8 | seed;
        // splitmix64's finalizer.
        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d049bb133111eb);
        hash ^= hash >> 31;
        ((hash >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0) * amount
    }

    fn band(pixel: impl Fn(usize, usize) -> [f32; 3]) -> Band {
        let mut band = Band::new(0, SIZE.0, SIZE.1);
        for y in 0..SIZE.1 {
            for (x, p) in band.row_mut(y).chunks_exact_mut(3).enumerate() {
                p.copy_from_slice(&pixel(x, y));
            }
        }
        band
    }

    /// Mean squared difference from `clean`.
    fn error(band: &Band, clean: impl Fn(usize, usize) -> [f32; 3]) -> f32 {
        let mut sum = 0.0;
        for y in 0..SIZE.1 {
            for (x, p) in band.row(y).chunks_exact(3).enumerate() {
                let c = clean(x, y);
                sum += (0..3).map(|i| (p[i] - c[i]).powi(2)).sum::<f32>();
            }
        }
        sum / (SIZE.0 * SIZE.1 * 3) as f32
    }

    #[test]
    fn opponent_round_trips() {
        for rgb in [[0.2, 0.5, 0.9], [1.0, 0.0, 0.3], [0.0, 0.0, 0.0]] {
            let back = from_opponent(to_opponent(rgb));
            for channel in 0..3 {
                assert!(
                    (back[channel] - rgb[channel]).abs() < 1e-6,
                    "{rgb:?} {back:?}"
                );
            }
        }
    }

    #[test]
    fn off_is_no_stage() {
        assert!(Denoise::new(&NoiseReduction::default(), 1).is_none());
        assert!(Denoise::new(&settings(0.0, 0.0), 2).is_none());
    }

    #[test]
    fn a_flat_photo_stays_flat() {
        let stage = Denoise::new(&settings(80.0, 80.0), 1).expect("active");
        let out = stage.process(band(|_, _| [0.3, 0.4, 0.5]), 0..SIZE.1);
        assert!(error(&out, |_, _| [0.3, 0.4, 0.5]) < 1e-12);
    }

    #[test]
    fn grain_is_smoothed_and_edges_kept() {
        // A step edge under grain: luminance noise on all channels alike.
        let clean = |x: usize, _: usize| if x < 48 { [0.1; 3] } else { [0.6; 3] };
        let noisy = |x: usize, y: usize| clean(x, y).map(|v| (v + noise(x, y, 0, 0.03)).max(0.0));
        let stage = Denoise::new(&settings(100.0, 0.0), 1).expect("active");
        let out = stage.process(band(noisy), 0..SIZE.1);
        let before = error(&band(noisy), clean);
        let after = error(&out, clean);
        assert!(after < before * 0.4, "{before} → {after}");
        // The edge stays sharp: the pixels either side keep most of the step.
        let row = out.row(36);
        assert!(
            row[50 * 3] - row[45 * 3] > 0.4,
            "{} {}",
            row[45 * 3],
            row[50 * 3]
        );
    }

    #[test]
    fn colour_speckles_go_and_luminance_stays() {
        let clean = |_: usize, _: usize| [0.3, 0.3, 0.3];
        // Opposite red and blue speckles: colour only, luminance about even.
        let noisy = |x: usize, y: usize| {
            let n = noise(x, y, 1, 0.06);
            [0.3 + n, 0.3, 0.3 - n]
        };
        let stage = Denoise::new(&settings(0.0, 100.0), 1).expect("active");
        let out = stage.process(band(noisy), 0..SIZE.1);
        assert!(error(&out, clean) < error(&band(noisy), clean) * 0.2);
    }

    #[test]
    fn half_size_skips_the_finest_level() {
        let full = Denoise::new(&settings(50.0, 50.0), 1)
            .expect("active")
            .plan();
        let half = Denoise::new(&settings(50.0, 50.0), 2)
            .expect("active")
            .plan();
        assert_eq!(full.thresholds.len(), LEVELS);
        assert_eq!(half.thresholds.len(), LEVELS - 1);
        // Colour blotches are the same size on the photo either way.
        assert_eq!(half.thresholds[0][1], full.thresholds[1][1]);
    }

    #[test]
    fn bands_need_every_level_reach() {
        let stage = Denoise::new(&settings(50.0, 0.0), 1).expect("active");
        assert_eq!(stage.halo(), 2 + 4 + 8 + 16 + 32);
        assert_eq!(stage.input_rows(100..110, 400), 38..172);
    }
}
