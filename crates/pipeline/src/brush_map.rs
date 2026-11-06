// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! A painted mask's selection as a grid of weights (docs/PIPELINE.md §5b).
//!
//! A brush can hold tens of thousands of points, so measuring the distance
//! to them at every pixel of every render is out of reach, least of all
//! while painting. Instead the strokes are rasterized once, at a resolution
//! tied to the photo and not to the render, and each pixel samples the grid
//! bilinearly. Previews and exports therefore select exactly the same area,
//! and the per-pixel cost doesn't depend on how much was painted.
//!
//! [`BrushMap::at`] is the reference for `crates/gpu/src/develop.wgsl` and
//! the desktop editor's mask overlay; keep them in step.

use crate::retouch::brush_falloff;
use photonation_develop::{MaskBrush, MaskSpace, MaskStroke};
use rayon::prelude::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

/// Grid cells per long edge of the photo. A 24 MP photo gets a cell every
/// three pixels: soft edges lose nothing, and a hard edge fades over a few
/// pixels at full size, which is what a brushed edge should do anyway.
pub const CELLS_PER_LONG_EDGE: f32 = 2048.0;
/// A stroke fades over at least this many cells, so even a hard edge
/// interpolates smoothly instead of showing the grid's staircase.
const MIN_FADE_CELLS: f32 = 1.5;
/// Empty cells kept around the painted area, so sampling just outside it
/// reads no weight, and where the photo cuts a stroke off, past the photo's
/// edge (a straightened render reads a few pixels beyond it).
const MARGIN_CELLS: i64 = 8;
/// How many bytes of maps stay cached. A map covering a whole 3:2 photo
/// takes 11 MB; most brushes cover far less.
const CACHE_BYTES: usize = 256 * 1024 * 1024;

/// A brush's selection, 0..1, on a grid aligned to the photo. It covers only
/// the painted area: elsewhere, nothing is selected.
#[derive(Clone, Debug, PartialEq)]
pub struct BrushMap {
    /// Metric position of the first cell's centre.
    pub origin: [f32; 2],
    pub width: usize,
    pub height: usize,
    /// `width × height` weights, row by row.
    pub weights: Vec<f32>,
    /// Metric `[left, top, right, bottom]` beyond which [`BrushMap::at`] reads
    /// nothing. Infinite on a side where a weight touches the map's edge, as
    /// clamping continues it from there.
    painted: [f32; 4],
}

/// A stroke ready to rasterize, in metric units.
struct PreparedStroke {
    points: Vec<[f32; 2]>,
    radius: f32,
    fade_start: f32,
    /// 0..1.
    flow: f32,
    erase: bool,
    /// `[left, top, right, bottom]` of everything it touches.
    bounds: [f32; 4],
}

impl PreparedStroke {
    fn new(stroke: &MaskStroke, space: MaskSpace) -> Option<PreparedStroke> {
        let radius = stroke.path.radius as f32;
        let min_fade = MIN_FADE_CELLS / CELLS_PER_LONG_EDGE;
        let fade_start = (radius * (1.0 - stroke.feather / 100.0)).min(radius - min_fade);
        let points: Vec<[f32; 2]> = stroke
            .path
            .points
            .iter()
            .map(|point| space.to_metric(*point).map(|v| v as f32))
            .collect();
        let mut bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for point in &points {
            bounds[0] = bounds[0].min(point[0] - radius);
            bounds[1] = bounds[1].min(point[1] - radius);
            bounds[2] = bounds[2].max(point[0] + radius);
            bounds[3] = bounds[3].max(point[1] + radius);
        }
        (!points.is_empty()).then_some(PreparedStroke {
            points,
            radius,
            fade_start,
            flow: stroke.flow / 100.0,
            erase: stroke.erase,
            bounds,
        })
    }

    /// Lowers `nearest` (squared distance to the path, for the cells of one
    /// row at `y`, the first at `left`) wherever this stroke's path is closer.
    fn measure_row(&self, y: f32, left: f32, nearest: &mut [f32]) {
        let last_cell = nearest.len() as f32 - 1.0;
        let cell_at = |x: f32| ((x - left) * CELLS_PER_LONG_EDGE).clamp(0.0, last_cell);
        let segments = self
            .points
            .windows(2)
            .map(|pair| (pair[0], pair[1]))
            // A dab is one point: a segment without length.
            .chain((self.points.len() == 1).then(|| (self.points[0], self.points[0])));
        for (a, b) in segments {
            if y < a[1].min(b[1]) - self.radius || y > a[1].max(b[1]) + self.radius {
                continue;
            }
            let first = cell_at(a[0].min(b[0]) - self.radius).floor() as usize;
            let last = cell_at(a[0].max(b[0]) + self.radius).ceil() as usize;
            let direction = [b[0] - a[0], b[1] - a[1]];
            let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
            for (index, nearest) in nearest.iter_mut().enumerate().take(last + 1).skip(first) {
                let x = left + index as f32 / CELLS_PER_LONG_EDGE;
                let offset = [x - a[0], y - a[1]];
                let along = if length_squared > 0.0 {
                    ((offset[0] * direction[0] + offset[1] * direction[1]) / length_squared)
                        .clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (dx, dy) = (
                    offset[0] - along * direction[0],
                    offset[1] - along * direction[1],
                );
                *nearest = nearest.min(dx * dx + dy * dy);
            }
        }
    }
}

impl BrushMap {
    /// Rasterizes `brush`. Rows run in parallel, and each computes the same
    /// values whichever thread gets it.
    pub fn new(brush: &MaskBrush, space: MaskSpace) -> BrushMap {
        let strokes: Vec<PreparedStroke> = brush
            .strokes
            .iter()
            .filter_map(|stroke| PreparedStroke::new(stroke, space))
            .collect();
        let Some((columns, rows)) = painted_cells(&strokes, space) else {
            return BrushMap::empty();
        };
        let (width, height) = ((columns.1 - columns.0) as usize, (rows.1 - rows.0) as usize);
        let origin = [columns.0, rows.0].map(|cell| (cell as f32 + 0.5) / CELLS_PER_LONG_EDGE);
        let mut weights = vec![0.0f32; width * height];
        weights.par_chunks_mut(width).enumerate().for_each_init(
            || vec![0.0f32; width],
            |nearest, (row, selected)| {
                let y = origin[1] + row as f32 / CELLS_PER_LONG_EDGE;
                for stroke in &strokes {
                    paint_row(stroke, y, origin[0], nearest, selected);
                }
            },
        );
        let painted = painted_area(&weights, width, height, origin);
        BrushMap {
            origin,
            width,
            height,
            weights,
            painted,
        }
    }

    /// A map that selects nothing.
    pub fn empty() -> BrushMap {
        BrushMap {
            origin: [0.0; 2],
            width: 1,
            height: 1,
            weights: vec![0.0],
            painted: [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ],
        }
    }

    /// Whether [`BrushMap::at`] reads nothing anywhere in a metric area
    /// `[left, top, right, bottom]`.
    pub fn selects_nothing_in(&self, area: [f32; 4]) -> bool {
        let [left, top, right, bottom] = self.painted;
        area[2] < left || area[0] > right || area[3] < top || area[1] > bottom
    }

    /// The map of `brush`, from a small cache shared by every render: a
    /// slider drag doesn't rasterize anything, and the GPU renderer and the
    /// overlay can tell by the `Arc` that they already hold this map.
    pub fn cached(brush: &MaskBrush, space: MaskSpace) -> Arc<BrushMap> {
        static CACHE: OnceLock<Mutex<VecDeque<CachedMap>>> = OnceLock::new();
        let cache = CACHE.get_or_init(Mutex::default);
        let key = (brush.fingerprint(), space.scale().map(f64::to_bits));
        if let Ok(mut cached) = cache.lock()
            && let Some(position) = cached.iter().position(|entry| entry.key == key)
            && let Some(entry) = cached.remove(position)
        {
            let map = entry.map.clone();
            cached.push_back(entry);
            return map;
        }
        // Not under the lock: other renders keep their maps meanwhile.
        let map = Arc::new(BrushMap::new(brush, space));
        if let Ok(mut cached) = cache.lock() {
            cached.push_back(CachedMap {
                key,
                map: map.clone(),
            });
            let mut bytes: usize = cached.iter().map(|entry| entry.map.byte_size()).sum();
            while bytes > CACHE_BYTES && cached.len() > 1 {
                bytes -= cached.pop_front().map_or(0, |entry| entry.map.byte_size());
            }
        }
        map
    }

    pub fn byte_size(&self) -> usize {
        self.weights.len() * size_of::<f32>()
    }

    /// Selection strength, 0..1, at a metric point. Clamping to the edge
    /// reads the empty margin outside the painted area, and past the photo's
    /// edge continues a stroke the photo cuts off.
    #[inline]
    pub fn at(&self, point: [f32; 2]) -> f32 {
        let (last_x, last_y) = (self.width - 1, self.height - 1);
        let x = ((point[0] - self.origin[0]) * CELLS_PER_LONG_EDGE).clamp(0.0, last_x as f32);
        let y = ((point[1] - self.origin[1]) * CELLS_PER_LONG_EDGE).clamp(0.0, last_y as f32);
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(last_x), (y0 + 1).min(last_y));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let weight = |x: usize, y: usize| self.weights[y * self.width + x];
        let top = weight(x0, y0) + (weight(x1, y0) - weight(x0, y0)) * fx;
        let bottom = weight(x0, y1) + (weight(x1, y1) - weight(x0, y1)) * fx;
        top + (bottom - top) * fy
    }
}

struct CachedMap {
    key: (u64, [u64; 2]),
    map: Arc<BrushMap>,
}

/// What [`BrushMap::painted`] says for these weights: one cell past the last
/// weight on each side, since sampling interpolates towards it.
fn painted_area(weights: &[f32], width: usize, height: usize, origin: [f32; 2]) -> [f32; 4] {
    let mut cells: Option<[usize; 4]> = None;
    for (row, row_weights) in weights.chunks_exact(width).enumerate() {
        let is_painted = |weight: &f32| *weight > 0.0;
        if let Some(first) = row_weights.iter().position(is_painted) {
            let last = row_weights.iter().rposition(is_painted).unwrap_or(first);
            let cells = cells.get_or_insert([first, row, last, row]);
            cells[0] = cells[0].min(first);
            cells[2] = cells[2].max(last);
            cells[3] = row;
        }
    }
    let Some([first_x, first_y, last_x, last_y]) = cells else {
        return BrushMap::empty().painted;
    };
    let low = |cell: usize, origin: f32| {
        if cell == 0 {
            f32::NEG_INFINITY
        } else {
            origin + (cell - 1) as f32 / CELLS_PER_LONG_EDGE
        }
    };
    let high = |cell: usize, end: usize, origin: f32| {
        if cell + 1 == end {
            f32::INFINITY
        } else {
            origin + (cell + 1) as f32 / CELLS_PER_LONG_EDGE
        }
    };
    [
        low(first_x, origin[0]),
        low(first_y, origin[1]),
        high(last_x, width, origin[0]),
        high(last_y, height, origin[1]),
    ]
}

/// A range of grid columns or rows: first, and one past the last.
type CellSpan = (i64, i64);

/// One row of one stroke: where it reaches the row, it paints or erases.
fn paint_row(
    stroke: &PreparedStroke,
    y: f32,
    left: f32,
    nearest: &mut [f32],
    selected: &mut [f32],
) {
    let [stroke_left, top, stroke_right, bottom] = stroke.bounds;
    if y < top || y > bottom {
        return;
    }
    let last_cell = (selected.len() - 1) as f32;
    let cell_at = |x: f32| ((x - left) * CELLS_PER_LONG_EDGE).clamp(0.0, last_cell);
    let first = cell_at(stroke_left).floor() as usize;
    let last = cell_at(stroke_right).ceil() as usize;
    nearest[first..=last].fill(f32::INFINITY);
    // The cells are measured from `left`, so the indices stay the row's.
    stroke.measure_row(y, left, nearest);
    let radius_squared = stroke.radius * stroke.radius;
    for (selected, nearest) in selected[first..=last]
        .iter_mut()
        .zip(&nearest[first..=last])
    {
        if *nearest < radius_squared {
            let weight = brush_falloff(nearest.sqrt(), stroke.fade_start, stroke.radius);
            *selected = MaskStroke::combine(stroke.erase, stroke.flow, *selected, weight);
        }
    }
}

/// The range of grid columns and rows to rasterize: what the painting (not
/// the erasing, which can only take away) reaches on the photo, with margins.
fn painted_cells(strokes: &[PreparedStroke], space: MaskSpace) -> Option<(CellSpan, CellSpan)> {
    let mut bounds: Option<[f32; 4]> = None;
    for stroke in strokes
        .iter()
        .filter(|stroke| !stroke.erase && stroke.flow > 0.0)
    {
        let [left, top, right, bottom] = bounds.get_or_insert(stroke.bounds);
        *left = left.min(stroke.bounds[0]);
        *top = top.min(stroke.bounds[1]);
        *right = right.max(stroke.bounds[2]);
        *bottom = bottom.max(stroke.bounds[3]);
    }
    let [left, top, right, bottom] = bounds?;
    let scale = space.scale();
    let range = |low: f32, high: f32, photo: f64| {
        let photo_cells = (photo as f32 * CELLS_PER_LONG_EDGE).ceil() as i64;
        let first = (low * CELLS_PER_LONG_EDGE).floor() as i64 - MARGIN_CELLS;
        let end = (high * CELLS_PER_LONG_EDGE).ceil() as i64 + MARGIN_CELLS;
        let range = (
            first.max(-MARGIN_CELLS),
            end.min(photo_cells + MARGIN_CELLS),
        );
        (range.0 < range.1).then_some(range)
    };
    Some((range(left, right, scale[0])?, range(top, bottom, scale[1])?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonation_develop::BrushStroke;

    /// On a square photo metric points equal positions.
    fn square() -> MaskSpace {
        MaskSpace::new(1.0, 1.0)
    }

    fn stroke(points: &[[f64; 2]], radius: f64, feather: f32) -> MaskStroke {
        MaskStroke {
            path: BrushStroke {
                points: points.to_vec(),
                radius,
            },
            feather,
            ..MaskStroke::default()
        }
    }

    fn map(strokes: Vec<MaskStroke>) -> BrushMap {
        BrushMap::new(&MaskBrush { strokes }, square())
    }

    #[test]
    fn a_stroke_is_full_along_its_path_and_fades_to_its_radius() {
        let map = map(vec![stroke(&[[0.2, 0.5], [0.8, 0.5]], 0.1, 50.0)]);
        assert_eq!(map.at([0.5, 0.5]), 1.0, "on the path");
        assert_eq!(map.at([0.5, 0.54]), 1.0, "inside the core");
        let fading = map.at([0.5, 0.575]);
        assert!(
            fading > 0.2 && fading < 0.8,
            "halfway down the fade: {fading}"
        );
        assert_eq!(map.at([0.5, 0.62]), 0.0, "past the radius");
        assert_eq!(map.at([0.5, 0.1]), 0.0, "far outside the painted area");
        // Round caps: the ends reach as far as the sides.
        assert_eq!(map.at([0.16, 0.5]), 1.0);
        assert_eq!(map.at([0.08, 0.5]), 0.0);
    }

    #[test]
    fn a_click_dabs_a_round_spot() {
        let map = map(vec![stroke(&[[0.5, 0.5]], 0.05, 0.0)]);
        assert_eq!(map.at([0.5, 0.5]), 1.0);
        assert_eq!(map.at([0.53, 0.53]), 1.0);
        assert_eq!(map.at([0.54, 0.54]), 0.0);
    }

    #[test]
    fn hard_edges_interpolate_instead_of_showing_the_grid() {
        let map = map(vec![stroke(&[[0.5, 0.5]], 0.1, 0.0)]);
        let samples: Vec<f32> = (0..200)
            .map(|step| map.at([0.5 + 0.095 + step as f32 * 5e-5, 0.5]))
            .collect();
        assert_eq!(samples[0], 1.0);
        assert_eq!(samples[199], 0.0);
        assert!(
            samples.windows(2).all(|pair| pair[1] <= pair[0]),
            "monotone"
        );
        let largest_step = samples
            .windows(2)
            .map(|pair| pair[0] - pair[1])
            .fold(0.0, f32::max);
        assert!(largest_step < 0.2, "no jump: {largest_step}");
    }

    #[test]
    fn strokes_apply_in_order() {
        let paint = stroke(&[[0.5, 0.5]], 0.1, 0.0);
        let erase = MaskStroke {
            erase: true,
            ..stroke(&[[0.5, 0.5]], 0.03, 0.0)
        };
        let hole = map(vec![paint.clone(), erase.clone()]);
        assert_eq!(hole.at([0.5, 0.5]), 0.0, "erased");
        assert_eq!(hole.at([0.55, 0.5]), 1.0, "still painted");
        let painted_over = map(vec![paint.clone(), erase, paint]);
        assert_eq!(painted_over.at([0.5, 0.5]), 1.0, "painted over again");
    }

    #[test]
    fn low_flow_builds_up_across_strokes_but_not_within_one() {
        let thin = MaskStroke {
            flow: 50.0,
            // Doubling back: one stroke, so one coat.
            ..stroke(&[[0.3, 0.5], [0.7, 0.5], [0.3, 0.5]], 0.1, 0.0)
        };
        assert_eq!(map(vec![thin.clone()]).at([0.5, 0.5]), 0.5);
        assert_eq!(map(vec![thin.clone(), thin]).at([0.5, 0.5]), 0.75);
    }

    #[test]
    fn the_map_matches_the_distance_measured_directly() {
        let path = [[0.2, 0.3], [0.35, 0.6], [0.6, 0.55], [0.8, 0.2]];
        let (radius, feather) = (0.08, 60.0);
        let map = map(vec![stroke(&path, radius, feather)]);
        let fade_start = radius as f32 * (1.0 - feather / 100.0);
        let segments: Vec<[[f32; 2]; 2]> = path
            .windows(2)
            .map(|pair| [pair[0].map(|v| v as f32), pair[1].map(|v| v as f32)])
            .collect();
        for step in 0..400 {
            // A fixed scatter over the photo.
            let point = [
                (step as f32 * 0.618_034).fract(),
                (step as f32 * 0.754_877).fract(),
            ];
            let distance = segments
                .iter()
                .map(|[a, b]| {
                    let direction = [b[0] - a[0], b[1] - a[1]];
                    let along = (((point[0] - a[0]) * direction[0]
                        + (point[1] - a[1]) * direction[1])
                        / (direction[0] * direction[0] + direction[1] * direction[1]))
                        .clamp(0.0, 1.0);
                    (point[0] - a[0] - along * direction[0])
                        .hypot(point[1] - a[1] - along * direction[1])
                })
                .fold(f32::INFINITY, f32::min);
            let expected = brush_falloff(distance, fade_start, radius as f32);
            let error = (map.at(point) - expected).abs();
            assert!(error < 0.02, "{point:?}: {} vs {expected}", map.at(point));
        }
    }

    #[test]
    fn strokes_stay_round_on_landscape_photos() {
        let space = MaskSpace::new(3.0, 2.0);
        let brush = MaskBrush {
            strokes: vec![stroke(&[[0.5, 0.5]], 0.1, 0.0)],
        };
        let map = BrushMap::new(&brush, space);
        let centre = space.to_metric([0.5, 0.5]).map(|v| v as f32);
        for offset in [[0.09, 0.0], [0.0, 0.09], [-0.09, 0.0], [0.0, -0.09]] {
            assert_eq!(map.at([centre[0] + offset[0], centre[1] + offset[1]]), 1.0);
        }
        assert_eq!(map.at([centre[0], centre[1] + 0.11]), 0.0);
    }

    #[test]
    fn the_map_covers_only_what_was_painted() {
        let small = map(vec![stroke(&[[0.5, 0.5]], 0.01, 50.0)]);
        let expected = 0.02 * CELLS_PER_LONG_EDGE + 2.0 * MARGIN_CELLS as f32 + 2.0;
        assert!(small.width as f32 <= expected, "{}", small.width);
        assert!(small.height as f32 <= expected, "{}", small.height);

        // Far off the photo: nothing to keep beyond the margins.
        let large = map(vec![stroke(&[[-1.0, 0.5], [2.0, 0.5]], 0.25, 50.0)]);
        let limit = CELLS_PER_LONG_EDGE as usize + 2 * MARGIN_CELLS as usize;
        assert_eq!(large.width, limit);
        assert_eq!(large.at([0.0, 0.5]), 1.0);
        assert_eq!(large.at([1.0, 0.5]), 1.0, "up to the photo's edge");
        assert_eq!(large.at([1.1, 0.5]), 1.0, "clamped past it");
    }

    #[test]
    fn nothing_painted_selects_nothing() {
        let erase_only = MaskStroke {
            erase: true,
            ..stroke(&[[0.5, 0.5]], 0.1, 0.0)
        };
        for strokes in [Vec::new(), vec![erase_only]] {
            let map = map(strokes);
            assert_eq!((map.width, map.height), (1, 1));
            assert_eq!(map.at([0.5, 0.5]), 0.0);
        }
        let off_the_photo = map(vec![stroke(&[[-0.9, -0.9]], 0.01, 0.0)]);
        assert_eq!(off_the_photo.at([0.5, 0.5]), 0.0);
    }

    #[test]
    fn the_painted_area_holds_everything_sampling_can_read() {
        let map = map(vec![stroke(&[[0.3, 0.4], [0.5, 0.45]], 0.05, 50.0)]);
        // A grid of areas, each small enough to find every weight in it.
        for row in 0..100 {
            for column in 0..100 {
                let (x, y) = (column as f32 / 100.0, row as f32 / 100.0);
                let area = [x, y, x + 0.01, y + 0.01];
                if map.selects_nothing_in(area) {
                    for step in 0..=4 {
                        let t = step as f32 / 4.0;
                        let point = [x + 0.01 * t, y + 0.01 * (1.0 - t)];
                        assert_eq!(map.at(point), 0.0, "{point:?}");
                    }
                }
            }
        }
        assert!(map.selects_nothing_in([0.6, 0.0, 1.0, 1.0]), "right of it");
        assert!(map.selects_nothing_in([0.0, 0.6, 1.0, 1.0]), "below it");
        assert!(!map.selects_nothing_in([0.0, 0.0, 1.0, 1.0]), "on it");
        assert!(BrushMap::empty().selects_nothing_in([0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn a_stroke_the_photo_cuts_off_reaches_past_the_edge() {
        let map = map(vec![stroke(&[[-0.5, 0.5], [1.5, 0.5]], 0.25, 50.0)]);
        assert_eq!(map.at([3.0, 0.5]), 1.0, "clamping continues it");
        assert!(!map.selects_nothing_in([2.9, 0.45, 3.0, 0.55]));
        assert!(map.selects_nothing_in([2.9, 0.9, 3.0, 1.0]));
    }

    #[test]
    fn maps_are_cached_by_what_they_select() {
        let brush = MaskBrush {
            strokes: vec![stroke(&[[0.123, 0.456]], 0.02, 0.0)],
        };
        let first = BrushMap::cached(&brush, square());
        assert!(Arc::ptr_eq(
            &first,
            &BrushMap::cached(&brush.clone(), square())
        ));
        let other_photo = BrushMap::cached(&brush, MaskSpace::new(3.0, 2.0));
        assert!(
            !Arc::ptr_eq(&first, &other_photo),
            "the aspect ratio counts"
        );
        let mut changed = brush.clone();
        changed.strokes[0].flow = 40.0;
        let changed = BrushMap::cached(&changed, square());
        assert_eq!(changed.at([0.123, 0.456]), 0.4);
    }
}
