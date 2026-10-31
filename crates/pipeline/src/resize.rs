// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Separable Lanczos-3 resampling in linear light.

use crate::band::{Band, Stage};
use crate::plan::{ResampleAxis, ResizePlan};
use std::ops::Range;

/// The input samples (and their weights) that make up one output sample.
#[derive(Clone, Copy)]
struct Taps {
    first_input: usize,
    weight_offset: usize,
    count: usize,
}

/// Filter taps for every output coordinate along one axis.
struct Axis {
    taps: Vec<Taps>,
    /// All weights, each output's `count` weights stored contiguously.
    weights: Vec<f32>,
}

fn lanczos3(x: f64) -> f64 {
    let x = x.abs();
    if x < 1e-9 {
        1.0
    } else if x < 3.0 {
        let px = std::f64::consts::PI * x;
        3.0 * px.sin() * (px / 3.0).sin() / (px * px)
    } else {
        0.0
    }
}

impl Axis {
    fn new(input_len: usize, output_len: usize) -> Axis {
        let scale = output_len as f64 / input_len as f64;
        // When shrinking, widen the kernel so it low-pass filters (no aliasing).
        let stretch = (1.0 / scale).max(1.0);
        let support = 3.0 * stretch;
        let mut taps = Vec::with_capacity(output_len);
        let mut weights = Vec::new();
        for output in 0..output_len {
            let center = (output as f64 + 0.5) / scale - 0.5;
            let first = ((center - support).floor() as isize).max(0) as usize;
            let last = ((center + support).ceil() as isize)
                .min(input_len as isize - 1)
                .max(first as isize) as usize;
            let raw: Vec<f64> = (first..=last)
                .map(|input| lanczos3((input as f64 - center) / stretch))
                .collect();
            let sum: f64 = raw.iter().sum();
            let normalize = if sum.abs() > 1e-12 { 1.0 / sum } else { 1.0 };
            taps.push(Taps {
                first_input: first,
                weight_offset: weights.len(),
                count: raw.len(),
            });
            weights.extend(raw.iter().map(|w| (w * normalize) as f32));
        }
        Axis { taps, weights }
    }

    fn weights(&self, taps: Taps) -> &[f32] {
        &self.weights[taps.weight_offset..taps.weight_offset + taps.count]
    }

    fn plan(&self) -> ResampleAxis {
        ResampleAxis {
            taps: self
                .taps
                .iter()
                .map(|taps| [taps.first_input, taps.weight_offset, taps.count])
                .collect(),
            weights: self.weights.clone(),
        }
    }

    fn input_range(&self, out: Range<usize>) -> Range<usize> {
        if out.is_empty() {
            return 0..0;
        }
        let last = self.taps[out.end - 1];
        self.taps[out.start].first_input..last.first_input + last.count
    }
}

pub struct Resize {
    out: (usize, usize),
    horizontal: Axis,
    vertical: Axis,
}

impl Resize {
    pub fn new(input: (usize, usize), out: (usize, usize)) -> Resize {
        Resize {
            out,
            horizontal: Axis::new(input.0, out.0),
            vertical: Axis::new(input.1, out.1),
        }
    }

    pub fn plan(&self) -> ResizePlan {
        ResizePlan {
            horizontal: self.horizontal.plan(),
            vertical: self.vertical.plan(),
        }
    }
}

impl Stage for Resize {
    fn size(&self, _: (usize, usize)) -> (usize, usize) {
        self.out
    }

    fn input_rows(&self, out: Range<usize>, _: usize) -> Range<usize> {
        self.vertical.input_range(out)
    }

    fn process(&self, input: Band, out: Range<usize>) -> Band {
        let out_width = self.out.0;
        // Horizontal pass over every input row of the band.
        let mut narrowed = Band::new(input.y0, out_width, input.rows);
        for y in input.row_range() {
            let source_row = input.row(y);
            let target_row = narrowed.row_mut(y);
            for (target, &taps) in target_row.chunks_exact_mut(3).zip(&self.horizontal.taps) {
                let mut sum = [0.0f32; 3];
                for (k, weight) in self.horizontal.weights(taps).iter().enumerate() {
                    let source = &source_row[(taps.first_input + k) * 3..][..3];
                    for channel in 0..3 {
                        sum[channel] += source[channel] * weight;
                    }
                }
                target.copy_from_slice(&sum);
            }
        }
        // Vertical pass for the requested output rows only.
        let mut band = Band::new(out.start, out_width, out.len());
        for y in out {
            let taps = self.vertical.taps[y];
            let target_row = band.row_mut(y);
            for (k, weight) in self.vertical.weights(taps).iter().enumerate() {
                let source_row = narrowed.row(taps.first_input + k);
                for (target, source) in target_row.iter_mut().zip(source_row) {
                    *target += source * weight;
                }
            }
            // Lanczos lobes overshoot at edges; keep values displayable.
            target_row.iter_mut().for_each(|v| *v = v.clamp(0.0, 1.0));
        }
        band
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_sum_to_one() {
        for (src, dst) in [(100, 37), (37, 100), (6000, 2048), (10, 10)] {
            let a = Axis::new(src, dst);
            for &taps in &a.taps {
                let sum: f32 = a.weights(taps).iter().sum();
                assert!((sum - 1.0).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn constant_stays_constant() {
        let r = Resize::new((40, 30), (13, 9));
        let mut input = Band::new(0, 40, 30);
        input.data.iter_mut().for_each(|v| *v = 0.25);
        let out = r.process(input, 0..9);
        assert!(out.data.iter().all(|v| (v - 0.25).abs() < 1e-5));
    }
}
