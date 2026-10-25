// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Unsharp mask on luminance, applied to display-encoded values so halos
//! are perceptually even and colours don't fringe.

use crate::band::{Band, Stage, clamp_index};
use std::ops::Range;

pub struct Sharpen {
    amount: f32,
    /// Normalized 1D Gaussian, `2 * radius + 1` taps.
    kernel: Vec<f32>,
}

impl Sharpen {
    /// `amount` on the 0..150 slider scale; `radius` is the Gaussian sigma in pixels.
    pub fn new(amount: f32, radius: f32) -> Sharpen {
        let sigma = radius.max(0.3) as f64;
        let half_width = (sigma * 3.0).ceil() as isize;
        let mut kernel: Vec<f32> = (-half_width..=half_width)
            .map(|offset| (-(offset * offset) as f64 / (2.0 * sigma * sigma)).exp() as f32)
            .collect();
        let sum: f32 = kernel.iter().sum();
        kernel.iter_mut().for_each(|k| *k /= sum);
        Sharpen {
            amount: amount / 100.0,
            kernel,
        }
    }

    fn radius(&self) -> usize {
        self.kernel.len() / 2
    }

    /// Kernel-weighted sum of `sample(i)` for `i` around `center`, clamped to `0..len`.
    #[inline]
    fn convolve(&self, center: usize, len: usize, sample: impl Fn(usize) -> f32) -> f32 {
        let radius = self.radius() as isize;
        self.kernel
            .iter()
            .enumerate()
            .map(|(k, weight)| {
                sample(clamp_index(center as isize + k as isize - radius, len)) * weight
            })
            .sum()
    }
}

const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

impl Stage for Sharpen {
    fn size(&self, input: (usize, usize)) -> (usize, usize) {
        input
    }

    fn input_rows(&self, out: Range<usize>, input_height: usize) -> Range<usize> {
        let radius = self.radius();
        out.start.saturating_sub(radius)..(out.end + radius).min(input_height)
    }

    fn process(&self, input: Band, out: Range<usize>) -> Band {
        let width = input.width;
        let luma: Vec<f32> = input
            .data
            .chunks_exact(3)
            .map(|pixel| pixel[0] * LUMA[0] + pixel[1] * LUMA[1] + pixel[2] * LUMA[2])
            .collect();
        let mut blurred_rows = vec![0.0f32; luma.len()];
        for (line, blurred_line) in luma
            .chunks_exact(width)
            .zip(blurred_rows.chunks_exact_mut(width))
        {
            for (x, blurred) in blurred_line.iter_mut().enumerate() {
                *blurred = self.convolve(x, width, |i| line[i]);
            }
        }
        let mut band = Band::new(out.start, width, out.len());
        for y in out {
            let band_y = y - input.y0;
            let source_row = input.row(y);
            let target_row = band.row_mut(y);
            for x in 0..width {
                let blurred =
                    self.convolve(band_y, input.rows, |row| blurred_rows[row * width + x]);
                let detail = self.amount * (luma[band_y * width + x] - blurred);
                for channel in x * 3..x * 3 + 3 {
                    target_row[channel] = (source_row[channel] + detail).clamp(0.0, 1.0);
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
    fn flat_image_is_unchanged() {
        let s = Sharpen::new(100.0, 1.0);
        let mut input = Band::new(0, 16, 16);
        input.data.iter_mut().for_each(|v| *v = 0.5);
        let out = s.process(input, 4..8);
        assert!(out.data.iter().all(|v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn edges_gain_contrast() {
        let s = Sharpen::new(100.0, 1.0);
        let mut input = Band::new(0, 16, 8);
        for y in 0..8 {
            for (x, p) in input.row_mut(y).chunks_exact_mut(3).enumerate() {
                p.fill(if x < 8 { 0.3 } else { 0.7 });
            }
        }
        let out = s.process(input, 0..8);
        let row = out.row(4);
        assert!(row[7 * 3] < 0.3 && row[8 * 3] > 0.7);
    }
}
