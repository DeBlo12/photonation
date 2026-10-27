// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Straightening: resamples the developed image along a rotated crop frame
//! (bicubic, Catmull–Rom), so the frame's content comes out upright.

use crate::band::{Band, Stage, clamp_index};
use std::ops::Range;

/// Maps output pixels to input positions with an affine transform.
pub struct Straighten {
    out: (usize, usize),
    /// Input position of output position (0, 0). Positions are continuous,
    /// with pixel `i`'s centre at `i + 0.5`.
    origin: [f64; 2],
    /// Input distance covered by one output pixel along x, and along y.
    step_x: [f64; 2],
    step_y: [f64; 2],
}

/// Input rows and columns read on each side of a sample's nearest pixel.
const KERNEL_REACH: f64 = 2.0;

impl Straighten {
    /// `to_input` maps continuous output positions to continuous input
    /// positions and must be affine (rotation, scale and offset).
    pub fn new(out: (usize, usize), to_input: impl Fn([f64; 2]) -> [f64; 2]) -> Straighten {
        let origin = to_input([0.0, 0.0]);
        let along = |point: [f64; 2]| [point[0] - origin[0], point[1] - origin[1]];
        Straighten {
            out,
            origin,
            step_x: along(to_input([1.0, 0.0])),
            step_y: along(to_input([0.0, 1.0])),
        }
    }

    #[inline(always)]
    fn input_position(&self, x: f64, y: f64) -> [f64; 2] {
        [
            self.origin[0] + x * self.step_x[0] + y * self.step_y[0],
            self.origin[1] + x * self.step_x[1] + y * self.step_y[1],
        ]
    }
}

/// Catmull–Rom weights for the four pixels around a sample at fraction `t`
/// past the second one.
#[inline(always)]
fn catmull_rom(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        (-t3 + 2.0 * t2 - t) / 2.0,
        (3.0 * t3 - 5.0 * t2 + 2.0) / 2.0,
        (-3.0 * t3 + 4.0 * t2 + t) / 2.0,
        (t3 - t2) / 2.0,
    ]
}

impl Stage for Straighten {
    fn size(&self, _: (usize, usize)) -> (usize, usize) {
        self.out
    }

    fn input_rows(&self, out: Range<usize>, input_height: usize) -> Range<usize> {
        if out.is_empty() {
            return 0..0;
        }
        let width = self.out.0 as f64;
        let (top, bottom) = (out.start as f64, out.end as f64);
        let ys = [(0.0, top), (width, top), (0.0, bottom), (width, bottom)]
            .map(|(x, y)| self.input_position(x, y)[1] - 0.5);
        let lowest = ys.iter().copied().fold(f64::INFINITY, f64::min);
        let highest = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let first = (lowest.floor() - KERNEL_REACH).max(0.0) as usize;
        let end = ((highest.ceil() + KERNEL_REACH + 1.0).max(0.0) as usize).min(input_height);
        first.min(end.saturating_sub(1))..end
    }

    fn process(&self, input: Band, out: Range<usize>) -> Band {
        let mut band = Band::new(out.start, self.out.0, out.len());
        let first_row = input.y0 as isize;
        for y in out {
            let row = band.row_mut(y);
            for (x, pixel) in row.chunks_exact_mut(3).enumerate() {
                let [input_x, input_y] = self.input_position(x as f64 + 0.5, y as f64 + 0.5);
                let (sample_x, sample_y) = (input_x - 0.5, input_y - 0.5);
                let (base_x, base_y) = (sample_x.floor(), sample_y.floor());
                let weights_x = catmull_rom((sample_x - base_x) as f32);
                let weights_y = catmull_rom((sample_y - base_y) as f32);
                let mut sum = [0.0f32; 3];
                for (j, weight_y) in weights_y.iter().enumerate() {
                    let row_index = base_y as isize - 1 + j as isize - first_row;
                    let source_row = input.row(input.y0 + clamp_index(row_index, input.rows));
                    for (i, weight_x) in weights_x.iter().enumerate() {
                        let column = clamp_index(base_x as isize - 1 + i as isize, input.width);
                        let weight = weight_x * weight_y;
                        for (channel, total) in sum.iter_mut().enumerate() {
                            *total += source_row[column * 3 + channel] * weight;
                        }
                    }
                }
                // Bicubic lobes overshoot at edges; keep values displayable.
                for (target, value) in pixel.iter_mut().zip(sum) {
                    *target = value.clamp(0.0, 1.0);
                }
            }
        }
        band
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp_band(width: usize, height: usize) -> Band {
        let mut band = Band::new(0, width, height);
        for y in 0..height {
            for (x, pixel) in band.row_mut(y).chunks_exact_mut(3).enumerate() {
                pixel.fill((x + y) as f32 / (width + height) as f32);
            }
        }
        band
    }

    #[test]
    fn identity_copies_the_input() {
        let stage = Straighten::new((12, 9), |p| p);
        let input = ramp_band(12, 9);
        assert_eq!(stage.input_rows(0..9, 9), 0..9);
        let out = stage.process(input.clone(), 0..9);
        for (a, b) in out.data.iter().zip(&input.data) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn constant_stays_constant_when_rotated() {
        let (sin, cos) = 10f64.to_radians().sin_cos();
        let stage = Straighten::new((20, 14), |[x, y]| {
            [20.0 + x * cos - y * sin, 4.0 + x * sin + y * cos]
        });
        let mut input = Band::new(0, 48, 40);
        input.data.fill(0.3);
        let rows = stage.input_rows(3..9, 40);
        let band = Band {
            y0: rows.start,
            width: 48,
            rows: rows.len(),
            data: input.data[rows.start * 48 * 3..rows.end * 48 * 3].to_vec(),
        };
        let out = stage.process(band, 3..9);
        assert!(out.data.iter().all(|v| (v - 0.3).abs() < 1e-5));
    }

    #[test]
    fn requested_rows_cover_every_tap() {
        // A 30° turn reaches far up and down; the rows asked for must hold
        // every pixel the kernel reads, so a band matches a whole render.
        let (sin, cos) = 30f64.to_radians().sin_cos();
        let stage = Straighten::new((30, 30), |[x, y]| {
            [25.0 + x * cos - y * sin, 5.0 + x * sin + y * cos]
        });
        let input = ramp_band(80, 60);
        let whole = stage.process(input.clone(), 0..30);
        for start in (0..30).step_by(7) {
            let out = start..(start + 7).min(30);
            let rows = stage.input_rows(out.clone(), 60);
            let band = Band {
                y0: rows.start,
                width: 80,
                rows: rows.len(),
                data: input.data[rows.start * 80 * 3..rows.end * 80 * 3].to_vec(),
            };
            let part = stage.process(band, out.clone());
            for y in out {
                assert_eq!(part.row(y), whole.row(y), "row {y}");
            }
        }
    }
}
