// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Straightening and lens geometry: resamples the developed image along a
//! rotated crop frame and through the lens corrections' radial map
//! (bicubic, Catmull–Rom), so the frame's content comes out upright and
//! undistorted. With chromatic aberration corrected, red and blue are
//! sampled apart from green.

use crate::band::{Band, Stage, clamp_index};
use crate::plan::{LensPlan, StraightenPlan};
use std::ops::Range;

/// Maps output pixels to input positions: an affine transform, then
/// optionally the lens map.
pub struct Straighten {
    out: (usize, usize),
    /// Input position of output position (0, 0). Positions are continuous,
    /// with pixel `i`'s centre at `i + 0.5`.
    origin: [f64; 2],
    /// Input distance covered by one output pixel along x, and along y.
    step_x: [f64; 2],
    step_y: [f64; 2],
    lens: Option<StageLens>,
}

/// The lens map in input pixels (see [`LensPlan`]).
#[derive(Clone, Copy, Debug)]
pub struct StageLens {
    pub centre: [f64; 2],
    pub inverse_radius: [f64; 2],
    pub distortion: [f64; 7],
    pub tca: Option<[[f64; 3]; 2]>,
}

impl StageLens {
    /// Where green (and red and blue) of `position` lie.
    #[inline(always)]
    fn map(&self, position: [f64; 2]) -> [[f64; 2]; 3] {
        let offset = [position[0] - self.centre[0], position[1] - self.centre[1]];
        let rho = (offset[0] * self.inverse_radius[0]).hypot(offset[1] * self.inverse_radius[1]);
        let factor = polynomial(&self.distortion, rho);
        let at = |factor: f64| {
            [
                self.centre[0] + offset[0] * factor,
                self.centre[1] + offset[1] * factor,
            ]
        };
        let green = at(factor);
        match self.tca {
            None => [green; 3],
            Some([red, blue]) => {
                let shot = rho * factor;
                [
                    at(factor * polynomial(&red, shot)),
                    green,
                    at(factor * polynomial(&blue, shot)),
                ]
            }
        }
    }
}

#[inline(always)]
fn polynomial(coefficients: &[f64], r: f64) -> f64 {
    coefficients.iter().rev().fold(0.0, |sum, c| sum * r + c)
}

/// Input rows and columns read on each side of a sample's nearest pixel.
const KERNEL_REACH: f64 = 2.0;

impl Straighten {
    /// `to_input` maps continuous output positions to continuous input
    /// positions and must be affine (rotation, scale and offset); `lens`
    /// then moves those positions.
    pub fn new(
        out: (usize, usize),
        to_input: impl Fn([f64; 2]) -> [f64; 2],
        lens: Option<StageLens>,
    ) -> Straighten {
        let origin = to_input([0.0, 0.0]);
        let along = |point: [f64; 2]| [point[0] - origin[0], point[1] - origin[1]];
        Straighten {
            out,
            origin,
            step_x: along(to_input([1.0, 0.0])),
            step_y: along(to_input([0.0, 1.0])),
            lens,
        }
    }

    pub fn plan(&self) -> StraightenPlan {
        let narrow = |v: [f64; 2]| v.map(|v| v as f32);
        StraightenPlan {
            size: self.out,
            origin: narrow(self.origin),
            step_x: narrow(self.step_x),
            step_y: narrow(self.step_y),
            lens: self.lens.map(|lens| LensPlan {
                centre: narrow(lens.centre),
                inverse_radius: narrow(lens.inverse_radius),
                distortion: lens.distortion.map(|c| c as f32),
                tca: lens.tca.map(|tca| tca.map(|row| row.map(|c| c as f32))),
            }),
        }
    }

    #[inline(always)]
    fn affine_position(&self, x: f64, y: f64) -> [f64; 2] {
        [
            self.origin[0] + x * self.step_x[0] + y * self.step_y[0],
            self.origin[1] + x * self.step_x[1] + y * self.step_y[1],
        ]
    }

    /// Input positions of red, green and blue at output position `(x, y)`.
    #[inline(always)]
    fn input_positions(&self, x: f64, y: f64) -> [[f64; 2]; 3] {
        let position = self.affine_position(x, y);
        match &self.lens {
            None => [position; 3],
            Some(lens) => lens.map(position),
        }
    }

    /// The lowest and highest input y any channel of output rows `out`
    /// samples at.
    fn input_y_span(&self, out: &Range<usize>) -> (f64, f64) {
        let width = self.out.0 as f64;
        let (top, bottom) = (out.start as f64, out.end as f64);
        if self.lens.is_none() {
            let ys = [(0.0, top), (width, top), (0.0, bottom), (width, bottom)]
                .map(|(x, y)| self.affine_position(x, y)[1]);
            let lowest = ys.iter().copied().fold(f64::INFINITY, f64::min);
            let highest = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            return (lowest, highest);
        }
        // The lens map is radial and monotonic, so the band's pixel centres
        // map inside what its outermost ones map to (to within a fraction of
        // a pixel between neighbouring centres, which the kernel's reach
        // covers).
        let (first_y, last_y) = (top + 0.5, bottom - 0.5);
        let (first_x, last_x) = (0.5, width - 0.5);
        let rows = (0..self.out.0).flat_map(|x| {
            let x = x as f64 + 0.5;
            [(x, first_y), (x, last_y)]
        });
        let columns = out.clone().flat_map(|y| {
            let y = y as f64 + 0.5;
            [(first_x, y), (last_x, y)]
        });
        rows.chain(columns)
            .flat_map(|(x, y)| self.input_positions(x, y))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), [_, y]| {
                (low.min(y), high.max(y))
            })
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

/// The bicubic sample of `input` at continuous position `[x, y]`.
#[inline(always)]
fn sample(input: &Band, [input_x, input_y]: [f64; 2]) -> [f32; 3] {
    let first_row = input.y0 as isize;
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
    sum
}

impl Stage for Straighten {
    fn size(&self, _: (usize, usize)) -> (usize, usize) {
        self.out
    }

    fn input_rows(&self, out: Range<usize>, input_height: usize) -> Range<usize> {
        if out.is_empty() {
            return 0..0;
        }
        let (lowest, highest) = self.input_y_span(&out);
        let (lowest, highest) = (lowest - 0.5, highest - 0.5);
        let first = (lowest.floor() - KERNEL_REACH).max(0.0) as usize;
        let end = ((highest.ceil() + KERNEL_REACH + 1.0).max(0.0) as usize).min(input_height);
        first.min(end.saturating_sub(1))..end
    }

    fn process(&self, input: Band, out: Range<usize>) -> Band {
        let mut band = Band::new(out.start, self.out.0, out.len());
        let split_channels = self.lens.is_some_and(|lens| lens.tca.is_some());
        for y in out {
            let row = band.row_mut(y);
            for (x, pixel) in row.chunks_exact_mut(3).enumerate() {
                let positions = self.input_positions(x as f64 + 0.5, y as f64 + 0.5);
                let value = if split_channels {
                    [
                        sample(&input, positions[0])[0],
                        sample(&input, positions[1])[1],
                        sample(&input, positions[2])[2],
                    ]
                } else {
                    sample(&input, positions[1])
                };
                // Bicubic lobes overshoot at edges; keep values displayable.
                for (target, value) in pixel.iter_mut().zip(value) {
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
                pixel[0] *= 0.9;
            }
        }
        band
    }

    #[test]
    fn identity_copies_the_input() {
        let stage = Straighten::new((12, 9), |p| p, None);
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
        let stage = Straighten::new(
            (20, 14),
            |[x, y]| [20.0 + x * cos - y * sin, 4.0 + x * sin + y * cos],
            None,
        );
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

    /// A strong barrel correction with chromatic aberration, about the
    /// input's centre.
    fn lens(width: f64, height: f64) -> StageLens {
        let radius = width.hypot(height) / 2.0;
        StageLens {
            centre: [width / 2.0, height / 2.0],
            inverse_radius: [1.0 / radius; 2],
            distortion: [1.0, 0.0, -0.2, 0.05, 0.0, 0.0, 0.0],
            tca: Some([[1.002, 0.0, 0.001], [0.998, 0.0, 0.0]]),
        }
    }

    #[test]
    fn requested_rows_cover_every_tap() {
        // A 30° turn reaches far up and down, and the lens map bends the
        // rows; the rows asked for must hold every pixel the kernel reads,
        // so a band matches a whole render.
        let (sin, cos) = 30f64.to_radians().sin_cos();
        let turned = |[x, y]: [f64; 2]| [25.0 + x * cos - y * sin, 5.0 + x * sin + y * cos];
        let input = ramp_band(80, 60);
        for lens in [None, Some(lens(80.0, 60.0))] {
            let stage = Straighten::new((30, 30), turned, lens);
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

    #[test]
    fn the_lens_map_moves_red_and_blue_apart() {
        let lens = lens(80.0, 60.0);
        let [red, green, blue] = lens.map([75.0, 30.0]);
        // Barrel correction: an edge point comes from nearer the centre.
        assert!(green[0] < 75.0 && green[1] == 30.0);
        assert!(red[0] > green[0] && blue[0] < green[0]);
        assert_eq!(lens.map([40.0, 30.0]), [[40.0, 30.0]; 3]);
    }
}
