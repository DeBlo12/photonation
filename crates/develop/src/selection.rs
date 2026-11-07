// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Selections found for the user (a person a model outlined) and the brush
//! strokes that cover them. A click turns a selection into ordinary strokes,
//! so a removal or mask made this way saves, renders and edits like one
//! painted by hand, and needs no model afterwards. See docs/RETOUCH.md.

use crate::{BrushStroke, MAX_BRUSH_RADIUS, MAX_STROKE_POINTS, MIN_BRUSH_RADIUS, MaskSpace};

/// Grid cells, stretched over the whole photo in mask space (normalized
/// positions on the default crop, display orientation), that something
/// occupies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub width: usize,
    pub height: usize,
    /// Row-major, `width × height`.
    pub cells: Vec<bool>,
}

/// One row's run of selected cells, `left..=right`.
#[derive(Clone, Copy, Debug)]
struct Run {
    row: usize,
    left: usize,
    right: usize,
}

impl Selection {
    /// The cells where `probabilities` (a `width × height` grid) reach 0.5,
    /// sampled `upsample` times finer with bilinear interpolation so outlines
    /// follow the probabilities between cells instead of their steps.
    pub fn from_probabilities(
        width: usize,
        height: usize,
        probabilities: &[f32],
        upsample: usize,
    ) -> Selection {
        let upsample = upsample.max(1);
        if width == 0 || height == 0 || probabilities.len() != width * height {
            return Selection {
                width: 0,
                height: 0,
                cells: Vec::new(),
            };
        }
        let (fine_width, fine_height) = (width * upsample, height * upsample);
        let source = |fine: usize, length: usize| {
            let position =
                ((fine as f32 + 0.5) / upsample as f32 - 0.5).clamp(0.0, (length - 1) as f32);
            let low = position.floor() as usize;
            (low, (low + 1).min(length - 1), position - low as f32)
        };
        let at = |x: usize, y: usize| probabilities[y * width + x];
        let mut cells = Vec::with_capacity(fine_width * fine_height);
        for y in 0..fine_height {
            let (y0, y1, fy) = source(y, height);
            for x in 0..fine_width {
                let (x0, x1, fx) = source(x, width);
                let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
                let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
                cells.push(top + (bottom - top) * fy >= 0.5);
            }
        }
        Selection {
            width: fine_width,
            height: fine_height,
            cells,
        }
    }

    /// A cell's larger side, in long-edge units.
    pub fn cell_size(&self, space: MaskSpace) -> f64 {
        let [scale_x, scale_y] = space.scale();
        (scale_x / self.width.max(1) as f64).max(scale_y / self.height.max(1) as f64)
    }

    /// How many cells are selected.
    pub fn area(&self) -> usize {
        self.cells.iter().filter(|cell| **cell).count()
    }

    /// Whether the cell under a mask position is selected.
    pub fn contains(&self, position: [f64; 2]) -> bool {
        let cell = |value: f64, length: usize| {
            (0.0..1.0)
                .contains(&value)
                .then(|| ((value * length as f64) as usize).min(length - 1))
        };
        if self.width == 0 || self.height == 0 {
            return false;
        }
        match (
            cell(position[0], self.width),
            cell(position[1], self.height),
        ) {
            (Some(x), Some(y)) => self.cells[y * self.width + x],
            _ => false,
        }
    }

    /// Strokes covering every selected cell, reaching `grow` (long-edge
    /// units) further: at most `max_strokes`, the largest parts first, each
    /// within [`MAX_STROKE_POINTS`].
    ///
    /// Each stroke zigzags along the rows of one connected part, end to end
    /// (two points a row), with a radius of half a cell's diagonal plus
    /// `grow`, so even the outer corners of a row's end cells are covered.
    pub fn to_strokes(&self, space: MaskSpace, grow: f64, max_strokes: usize) -> Vec<BrushStroke> {
        let [scale_x, scale_y] = space.scale();
        let cell_width = scale_x / self.width.max(1) as f64;
        let cell_height = scale_y / self.height.max(1) as f64;
        let radius = (0.5 * cell_width.hypot(cell_height) + grow.max(0.0))
            .clamp(MIN_BRUSH_RADIUS, MAX_BRUSH_RADIUS);
        let centre = |column: usize, row: usize| {
            [
                (column as f64 + 0.5) / self.width as f64,
                (row as f64 + 0.5) / self.height as f64,
            ]
        };
        let mut parts: Vec<(usize, BrushStroke)> = self
            .parts()
            .into_iter()
            .flat_map(|part| {
                // Two points a row; longer parts continue in another stroke.
                part.chunks(MAX_STROKE_POINTS / 2)
                    .map(|runs| {
                        let cells = runs.iter().map(|run| run.right - run.left + 1).sum();
                        let points = runs
                            .iter()
                            .enumerate()
                            .flat_map(|(index, run)| {
                                let (left, right) =
                                    (centre(run.left, run.row), centre(run.right, run.row));
                                if index % 2 == 0 {
                                    [left, right]
                                } else {
                                    [right, left]
                                }
                            })
                            .collect();
                        (cells, BrushStroke { points, radius })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        // Stable: equal parts stay top to bottom.
        parts.sort_by_key(|(cells, _)| std::cmp::Reverse(*cells));
        parts
            .into_iter()
            .take(max_strokes)
            .map(|(_, stroke)| stroke)
            .collect()
    }

    /// The selected cells as runs, grouped into parts: each run continues
    /// the part whose run on the row above touches it.
    fn parts(&self) -> Vec<Vec<Run>> {
        let mut parts: Vec<Vec<Run>> = Vec::new();
        for row in 0..self.height {
            let cells = &self.cells[row * self.width..(row + 1) * self.width];
            let mut column = 0;
            // Parts already continued on this row: one run each.
            let mut continued = vec![false; parts.len()];
            while column < self.width {
                if !cells[column] {
                    column += 1;
                    continue;
                }
                let left = column;
                while column < self.width && cells[column] {
                    column += 1;
                }
                let run = Run {
                    row,
                    left,
                    right: column - 1,
                };
                let above = parts.iter().enumerate().position(|(index, part)| {
                    let last = part.last().expect("parts are never empty");
                    !continued.get(index).copied().unwrap_or(true)
                        && last.row + 1 == row
                        && last.left <= run.right + 1
                        && run.left <= last.right + 1
                });
                match above {
                    Some(index) => {
                        parts[index].push(run);
                        continued[index] = true;
                    }
                    None => parts.push(vec![run]),
                }
            }
        }
        parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(rows: &[&str]) -> Selection {
        Selection {
            width: rows[0].len(),
            height: rows.len(),
            cells: rows
                .iter()
                .flat_map(|row| row.chars().map(|cell| cell == '#'))
                .collect(),
        }
    }

    /// Whether a stroke's capsules cover a position (metric distance).
    fn covers(strokes: &[BrushStroke], space: MaskSpace, position: [f64; 2]) -> bool {
        let point = space.to_metric(position);
        strokes.iter().any(|stroke| {
            let ends = stroke.points.windows(2).map(|pair| (pair[0], pair[1]));
            let single = (stroke.points.len() == 1).then(|| (stroke.points[0], stroke.points[0]));
            ends.chain(single).any(|(a, b)| {
                let (a, b) = (space.to_metric(a), space.to_metric(b));
                let along = [b[0] - a[0], b[1] - a[1]];
                let length = along[0] * along[0] + along[1] * along[1];
                let t = if length > 0.0 {
                    (((point[0] - a[0]) * along[0] + (point[1] - a[1]) * along[1]) / length)
                        .clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let nearest = [a[0] + t * along[0], a[1] + t * along[1]];
                (point[0] - nearest[0]).hypot(point[1] - nearest[1]) <= stroke.radius
            })
        })
    }

    #[test]
    fn every_selected_cell_centre_and_corner_is_covered() {
        let shape = selection(&["..##....", ".####...", ".#..#...", ".#..#.#.", "##..###."]);
        let space = MaskSpace::new(8.0, 5.0);
        let strokes = shape.to_strokes(space, 0.0, 32);
        for row in 0..shape.height {
            for column in 0..shape.width {
                if !shape.cells[row * shape.width + column] {
                    continue;
                }
                for (dx, dy) in [(0.5, 0.5), (0.05, 0.05), (0.95, 0.95), (0.05, 0.95)] {
                    let position = [
                        (column as f64 + dx) / shape.width as f64,
                        (row as f64 + dy) / shape.height as f64,
                    ];
                    assert!(
                        covers(&strokes, space, position),
                        "cell {column},{row} at {dx},{dy}"
                    );
                }
            }
        }
    }

    #[test]
    fn separate_parts_get_separate_strokes_largest_first() {
        let shape = selection(&["#....###", "#....###"]);
        let strokes = shape.to_strokes(MaskSpace::new(8.0, 2.0), 0.0, 32);
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[0].points[0][0], 5.5 / 8.0, "the bigger part first");
        let only_one = shape.to_strokes(MaskSpace::new(8.0, 2.0), 0.0, 1);
        assert_eq!(only_one, strokes[..1]);
    }

    #[test]
    fn a_tall_part_continues_in_strokes_within_the_point_cap() {
        let rows = vec!["#"; MAX_STROKE_POINTS];
        let strokes = selection(&rows).to_strokes(MaskSpace::new(1.0, 10.0), 0.0, 32);
        assert_eq!(strokes.len(), 2);
        assert!(
            strokes
                .iter()
                .all(|stroke| stroke.points.len() <= MAX_STROKE_POINTS)
        );
    }

    #[test]
    fn growing_widens_the_radius() {
        let rows = vec!["####################"; 20];
        let shape = selection(&rows);
        let space = MaskSpace::new(20.0, 20.0);
        let plain = shape.to_strokes(space, 0.0, 1)[0].radius;
        let grown = shape.to_strokes(space, 0.01, 1)[0].radius;
        assert!((grown - plain - 0.01).abs() < 1e-12);
    }

    #[test]
    fn probabilities_are_thresholded_on_a_finer_grid() {
        let coarse = Selection::from_probabilities(2, 1, &[1.0, 0.0], 2);
        assert_eq!((coarse.width, coarse.height), (4, 2));
        assert_eq!(
            coarse.cells,
            [true, true, false, false, true, true, false, false]
        );
        assert!(coarse.contains([0.1, 0.5]));
        assert!(!coarse.contains([0.9, 0.5]));
        assert!(!coarse.contains([1.2, 0.5]));
        assert_eq!(coarse.area(), 4);
        let square = MaskSpace::new(1.0, 1.0);
        assert_eq!(coarse.cell_size(square), 0.5, "the taller side");
    }
}
