// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Texture and Clarity: local contrast on luminance at two scales.
//!
//! Detail = perceptual luminance − its blur; adding a fraction of it back
//! raises local contrast. Clarity uses a large radius weighted toward
//! midtones; Texture a small one. Radii scale with the image, so a preview
//! and a full-size export look the same. Blurs are three box passes
//! (≈ Gaussian) with running sums: O(1) per pixel regardless of radius.
//! The GPU renderer transcribes it (`crates/gpu/src/local_contrast.wgsl`);
//! keep the two in step.

use crate::band::{Band, Stage, clamp_index};
use crate::plan::LocalContrastPlan;
use std::ops::Range;

/// Box radii as a fraction of the long edge.
const CLARITY_RADIUS: f64 = 0.008;
const TEXTURE_RADIUS: f64 = 0.0012;
const CLARITY_STRENGTH: f32 = 0.6;
const TEXTURE_STRENGTH: f32 = 0.8;
const BOX_PASSES: usize = 3;
/// Rec.709 luma; the stage sees display-linear output RGB.
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

pub struct LocalContrast {
    clarity: f32,
    texture: f32,
    clarity_radius: usize,
    texture_radius: usize,
}

impl LocalContrast {
    /// `None` when both sliders are zero.
    pub fn new(clarity: f32, texture: f32, image_size: (usize, usize)) -> Option<LocalContrast> {
        if clarity == 0.0 && texture == 0.0 {
            return None;
        }
        let long_edge = image_size.0.max(image_size.1) as f64;
        let radius = |fraction: f64| ((long_edge * fraction).round() as usize).max(1);
        Some(LocalContrast {
            clarity: clarity / 100.0 * CLARITY_STRENGTH,
            texture: texture / 100.0 * TEXTURE_STRENGTH,
            clarity_radius: radius(CLARITY_RADIUS),
            texture_radius: radius(TEXTURE_RADIUS),
        })
    }

    pub fn plan(&self) -> LocalContrastPlan {
        LocalContrastPlan {
            clarity: self.clarity,
            texture: self.texture,
            clarity_radius: self.clarity_radius,
            texture_radius: self.texture_radius,
            box_passes: BOX_PASSES,
            luma: LUMA,
        }
    }

    fn halo(&self) -> usize {
        BOX_PASSES * self.clarity_radius.max(self.texture_radius)
    }
}

/// Plane of `width × rows` values.
struct Plane {
    values: Vec<f32>,
    width: usize,
    rows: usize,
}

impl Plane {
    /// Approximate Gaussian blur: `BOX_PASSES` horizontal then vertical box passes.
    fn blurred(&self, radius: usize) -> Plane {
        let mut values = self.values.clone();
        let mut scratch = vec![0.0; values.len()];
        for _ in 0..BOX_PASSES {
            for (src, dst) in values
                .chunks_exact(self.width)
                .zip(scratch.chunks_exact_mut(self.width))
            {
                box_1d(dst, radius, src.len(), |i| src[i]);
            }
            std::mem::swap(&mut values, &mut scratch);
        }
        let mut column = vec![0.0; self.rows];
        for _ in 0..BOX_PASSES {
            for x in 0..self.width {
                box_1d(&mut column, radius, self.rows, |y| {
                    values[y * self.width + x]
                });
                for (y, v) in column.iter().enumerate() {
                    values[y * self.width + x] = *v;
                }
            }
        }
        Plane {
            values,
            width: self.width,
            rows: self.rows,
        }
    }
}

/// Fixed-point scale of the running sums: 2^30 steps per unit.
const SUM_SCALE: f64 = (1u64 << 30) as f64;

/// Running-sum box filter of `len` samples into `out`, edges clamped.
///
/// The sum runs in fixed point, where adding and removing samples is
/// exact: a floating-point running sum would carry rounding error that
/// depends on where the band starts, so band height would change the
/// output.
fn box_1d(out: &mut [f32], radius: usize, len: usize, sample: impl Fn(usize) -> f32) {
    let r = radius as isize;
    let fixed = |i: isize| (sample(clamp_index(i, len)) as f64 * SUM_SCALE).round() as i64;
    let norm = 1.0 / ((2 * radius + 1) as f64 * SUM_SCALE);
    let mut sum: i64 = (-r..=r).map(fixed).sum();
    for (i, slot) in out.iter_mut().enumerate().take(len) {
        *slot = (sum as f64 * norm) as f32;
        let i = i as isize;
        sum += fixed(i + r + 1) - fixed(i - r);
    }
}

impl Stage for LocalContrast {
    fn size(&self, input: (usize, usize)) -> (usize, usize) {
        input
    }

    fn input_rows(&self, out: Range<usize>, input_height: usize) -> Range<usize> {
        let halo = self.halo();
        out.start.saturating_sub(halo)..(out.end + halo).min(input_height)
    }

    fn process(&self, input: Band, out: Range<usize>) -> Band {
        // sqrt is a cheap perceptual lightness proxy.
        let lightness = Plane {
            values: input
                .data
                .chunks_exact(3)
                .map(|p| {
                    (p[0] * LUMA[0] + p[1] * LUMA[1] + p[2] * LUMA[2])
                        .max(0.0)
                        .sqrt()
                })
                .collect(),
            width: input.width,
            rows: input.rows,
        };
        let clarity_base = (self.clarity != 0.0).then(|| lightness.blurred(self.clarity_radius));
        let texture_base = (self.texture != 0.0).then(|| lightness.blurred(self.texture_radius));

        let mut band = Band::new(out.start, input.width, out.len());
        for y in out {
            let source_row = input.row(y);
            let target_row = band.row_mut(y);
            for x in 0..input.width {
                let index = (y - input.y0) * input.width + x;
                let original = lightness.values[index];
                let mut enhanced = original;
                if let Some(base) = &clarity_base {
                    let midtone_weight = (4.0 * original * (1.0 - original)).clamp(0.0, 1.0);
                    enhanced += self.clarity * (original - base.values[index]) * midtone_weight;
                }
                if let Some(base) = &texture_base {
                    enhanced += self.texture * (original - base.values[index]);
                }
                // Scale RGB by the luminance ratio so hue is untouched.
                let gain = if original > 1e-4 {
                    (enhanced.max(0.0) / original).powi(2)
                } else {
                    1.0
                };
                for channel in x * 3..x * 3 + 3 {
                    target_row[channel] = (source_row[channel] * gain).clamp(0.0, 1.0);
                }
            }
        }
        band
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_filter_of_constant_is_constant() {
        let mut out = vec![0.0; 10];
        box_1d(&mut out, 3, 10, |_| 0.4);
        assert!(out.iter().all(|v| (v - 0.4).abs() < 1e-6));
    }

    #[test]
    fn clarity_increases_edge_contrast() {
        let stage = LocalContrast::new(100.0, 0.0, (64, 32)).expect("active");
        let mut input = Band::new(0, 64, 32);
        for y in 0..32 {
            for (x, p) in input.row_mut(y).chunks_exact_mut(3).enumerate() {
                p.fill(if x < 32 { 0.15 } else { 0.4 });
            }
        }
        let out = stage.process(input, 0..32);
        let row = out.row(16);
        assert!(row[31 * 3] < 0.15 && row[32 * 3] > 0.4);
    }
}
