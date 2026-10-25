// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Pull-based band execution.
//!
//! The image is rendered as horizontal bands of output rows, in parallel.
//! For each band, every stage reports which input rows it needs (its
//! "halo"), and the chain is evaluated backwards from the output to the
//! source for exactly those rows. Point stages run in place. Nothing
//! full-size exists between stages, so peak memory is the source mosaic,
//! the output buffer and a few band-sized scratch buffers per thread.

use rayon::prelude::*;
use std::ops::Range;

/// A strip of interleaved RGB `f32` rows `y0 .. y0 + rows`.
#[derive(Clone, Debug)]
pub struct Band {
    pub y0: usize,
    pub width: usize,
    pub rows: usize,
    pub data: Vec<f32>,
}

impl Band {
    pub fn new(y0: usize, width: usize, rows: usize) -> Band {
        Band {
            y0,
            width,
            rows,
            data: vec![0.0; width * rows * 3],
        }
    }

    pub fn row_range(&self) -> Range<usize> {
        self.y0..self.y0 + self.rows
    }

    /// Row by absolute index.
    #[inline]
    pub fn row(&self, y: usize) -> &[f32] {
        let start = (y - self.y0) * self.width * 3;
        &self.data[start..start + self.width * 3]
    }

    #[inline]
    pub fn row_mut(&mut self, y: usize) -> &mut [f32] {
        let row_len = self.width * 3;
        let start = (y - self.y0) * row_len;
        &mut self.data[start..start + row_len]
    }
}

pub trait Source: Send + Sync {
    fn size(&self) -> (usize, usize);
    fn render(&self, rows: Range<usize>) -> Band;
}

pub trait Stage: Send + Sync {
    /// Output size given the input size.
    fn size(&self, input: (usize, usize)) -> (usize, usize);
    /// Input rows needed to produce `out` (clamped to the input height).
    fn input_rows(&self, out: Range<usize>, input_height: usize) -> Range<usize>;
    fn process(&self, input: Band, out: Range<usize>) -> Band;
}

/// A per-pixel stage; adapts to [`Stage`] and runs in place.
pub trait PointOp: Send + Sync {
    fn apply(&self, px: &mut [f32]);
}

pub struct Point<P>(pub P);

impl<P: PointOp> Stage for Point<P> {
    fn size(&self, input: (usize, usize)) -> (usize, usize) {
        input
    }

    fn input_rows(&self, out: Range<usize>, _: usize) -> Range<usize> {
        out
    }

    fn process(&self, mut input: Band, _: Range<usize>) -> Band {
        self.0.apply(&mut input.data);
        input
    }
}

pub struct Graph<'a> {
    source: Box<dyn Source + 'a>,
    stages: Vec<Box<dyn Stage + 'a>>,
    sizes: Vec<(usize, usize)>,
}

impl<'a> Graph<'a> {
    pub fn new(source: Box<dyn Source + 'a>) -> Graph<'a> {
        let sizes = vec![source.size()];
        Graph {
            source,
            stages: Vec::new(),
            sizes,
        }
    }

    pub fn push(&mut self, stage: Box<dyn Stage + 'a>) {
        self.sizes.push(stage.size(self.size()));
        self.stages.push(stage);
    }

    /// Output size of the last stage.
    pub fn size(&self) -> (usize, usize) {
        *self.sizes.last().unwrap_or(&(0, 0))
    }

    pub fn source_size(&self) -> (usize, usize) {
        self.sizes[0]
    }

    pub fn render_rows(&self, rows: Range<usize>) -> Band {
        self.render_level(self.stages.len(), rows)
    }

    fn render_level(&self, level: usize, rows: Range<usize>) -> Band {
        if level == 0 {
            return self.source.render(rows);
        }
        let stage = &self.stages[level - 1];
        let input_height = self.sizes[level - 1].1;
        let needed_rows = stage.input_rows(rows.clone(), input_height);
        let input = self.render_level(level - 1, needed_rows);
        stage.process(input, rows)
    }

    /// Renders all rows in bands of `band_rows`, calling `sink` (from any
    /// thread) with each finished band.
    pub fn run<F: Fn(Band) + Sync>(&self, band_rows: usize, sink: F) {
        let height = self.size().1;
        let band_rows = band_rows.max(1);
        (0..height.div_ceil(band_rows))
            .into_par_iter()
            .for_each(|band_index| {
                let start = band_index * band_rows;
                sink(self.render_rows(start..(start + band_rows).min(height)));
            });
    }
}

/// Clamped index for edge handling in spatial stages.
#[inline(always)]
pub fn clamp_index(index: isize, len: usize) -> usize {
    index.clamp(0, len as isize - 1) as usize
}
