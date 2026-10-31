// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! A [`RenderPlan`] as the shaders' uniform and storage buffers. The
//! layouts mirror the WGSL structs field by field; every field is a
//! 16-byte row, so no padding rules come into play.

use photonation_develop::CombineMode;
use photonation_image::{Orientation, SensorLayout};
use photonation_pipeline::Demosaic;
use photonation_pipeline::masks::ShapeWeight;
use photonation_pipeline::plan::{ColorPlan, DevelopPlan, RenderPlan, SelectionPlan};

/// 32-bit words for a GPU buffer.
#[derive(Default)]
pub struct Words(Vec<u32>);

impl Words {
    fn floats(&mut self, values: [f32; 4]) {
        self.0.extend(values.map(f32::to_bits));
    }

    fn uints(&mut self, values: [u32; 4]) {
        self.0.extend(values);
    }

    fn ints(&mut self, values: [i32; 4]) {
        self.0.extend(values.map(|v| v as u32));
    }

    fn vec3(&mut self, v: [f32; 3]) {
        self.floats([v[0], v[1], v[2], 0.0]);
    }

    fn matrix(&mut self, rows: &[[f32; 3]; 3]) {
        rows.iter().for_each(|row| self.vec3(*row));
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.0.iter().flat_map(|word| word.to_le_bytes()).collect()
    }
}

/// The develop shader's `Sensor` uniform.
pub fn sensor(plan: &RenderPlan, layout: &SensorLayout) -> Words {
    let mode = match plan.demosaic {
        Demosaic::Mhc => 0,
        Demosaic::HalfSize => 1,
        Demosaic::NeighbourAverage => 2,
        Demosaic::Linear => 3,
    };
    let (pattern_size, colors) = match layout {
        SensorLayout::Cfa(pattern) => {
            let (width, height) = pattern.size();
            let colors: Vec<u32> = (0..height)
                .flat_map(|row| (0..width).map(move |col| (row, col)))
                .map(|(row, col)| pattern.color_at(row, col) as u32)
                .collect();
            ((width as u32, height as u32), colors)
        }
        SensorLayout::LinearRgb => ((1, 1), vec![0]),
    };
    let region = plan.region;
    let mut words = Words::default();
    words.uints([region.x, region.y, region.w, region.h].map(|v| v as u32));
    words.uints([mode, pattern_size.0, pattern_size.1, 0]);
    words.vec3(plan.black);
    words.vec3(plan.gain);
    let mut cfa = [0u32; 36];
    cfa[..colors.len().min(36)].copy_from_slice(&colors[..colors.len().min(36)]);
    for chunk in cfa.chunks_exact(4) {
        words.uints([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    words
}

/// Whether the shader's 6×6 CFA table holds `layout`'s pattern.
pub fn pattern_fits(layout: &SensorLayout) -> bool {
    match layout {
        SensorLayout::Cfa(pattern) => pattern.size().0 * pattern.size().1 <= 36,
        SensorLayout::LinearRgb => true,
    }
}

fn selection(words: &mut Words, selection: &SelectionPlan) {
    let [lightness, chroma, hue] = selection.target;
    words.floats([lightness, chroma, hue, selection.hue_relevance]);
    words.floats([
        selection.hue_reach,
        selection.chroma_reach,
        selection.lightness_reach,
        selection.solid_share,
    ]);
    words.floats([
        selection.hue_shift,
        selection.chroma_gain,
        selection.lightness_shift,
        selection.grey_chroma,
    ]);
}

/// The develop shader's `Develop` uniform.
pub fn develop(plan: &DevelopPlan) -> Words {
    let mut words = Words::default();
    words.matrix(&plan.camera_to_working);
    words.matrix(&plan.working_to_output);
    words.vec3(plan.working_luminance);
    words.vec3(plan.output_luminance);
    words.floats([plan.haze, plan.highlights, plan.shadows, plan.mid_grey]);
    words.floats([
        plan.max_tone_shift_ev,
        plan.tone.min_ev,
        plan.tone.max_ev,
        plan.tone.tables[0].len() as f32,
    ]);
    let empty = ColorPlan {
        band_hues: [0.0; 8],
        mixer: None,
        grading: None,
        saturation: 0.0,
        vibrance: 0.0,
        point_colors: Vec::new(),
        overlay: None,
    };
    let color = plan.color.as_ref().unwrap_or(&empty);
    words.uints([
        u32::from(plan.color.is_some()),
        u32::from(color.mixer.is_some()),
        u32::from(color.grading.is_some()),
        color.point_colors.len() as u32,
    ]);
    words.floats([
        color.saturation,
        color.vibrance,
        if color.overlay.is_some() { 1.0 } else { 0.0 },
        0.0,
    ]);
    plan.oklab.iter().for_each(|matrix| words.matrix(matrix));
    for hues in color.band_hues.chunks_exact(4) {
        words.floats([hues[0], hues[1], hues[2], hues[3]]);
    }
    for index in 0..8 {
        let band = color.mixer.map(|bands| bands[index]);
        words.vec3(band.map_or([0.0; 3], |band| {
            [band.hue_shift, band.chroma_gain, band.lightness_shift]
        }));
    }
    let grading = color.grading.as_ref();
    for wheel in 0..4 {
        words.vec3(grading.map_or([0.0; 3], |grading| grading.wheels[wheel]));
    }
    words.floats([
        grading.map_or(0.0, |grading| grading.pivot),
        grading.map_or(0.0, |grading| grading.transition),
        0.0,
        0.0,
    ]);
    match &color.overlay {
        Some(overlay) => selection(&mut words, overlay),
        None => (0..3).for_each(|_| words.floats([0.0; 4])),
    }
    match &plan.local {
        Some(local) => {
            let limits = &local.limits;
            words.uints([1, local.masks.len() as u32, 0, 0]);
            words.floats([
                local.origin[0],
                local.origin[1],
                local.step_x[0],
                local.step_x[1],
            ]);
            words.floats([local.step_y[0], local.step_y[1], 0.0, 0.0]);
            words.floats([
                limits.max_haze,
                limits.max_white_balance_ev,
                limits.max_contrast,
                limits.dehaze_saturation,
            ]);
            words.floats([limits.max_exposure_ev, limits.max_tone_amount, 0.0, 0.0]);
        }
        None => (0..5).for_each(|_| words.uints([0; 4])),
    }
    words
}

/// The develop shader's `masks` and `shapes` arrays (never empty).
pub fn masks(plan: &DevelopPlan) -> (Words, Words) {
    let (mut masks, mut shapes) = (Words::default(), Words::default());
    let mut shape_count = 0;
    for mask in plan.local.iter().flat_map(|local| &local.masks) {
        let components = &mask.weight.components;
        masks.uints([
            shape_count,
            components.len() as u32,
            u32::from(mask.weight.invert),
            0,
        ]);
        let a = &mask.amounts;
        masks.floats([a.exposure, a.temperature, a.tint, a.contrast]);
        masks.floats([a.highlights, a.shadows, a.dehaze, a.saturation]);
        for (shape, mode) in components {
            shape_words(&mut shapes, shape, *mode);
        }
        shape_count += components.len() as u32;
    }
    for words in [&mut masks, &mut shapes] {
        if words.0.is_empty() {
            (0..3).for_each(|_| words.uints([0; 4]));
        }
    }
    (masks, shapes)
}

fn shape_words(words: &mut Words, shape: &ShapeWeight, mode: CombineMode) {
    let mode = match mode {
        CombineMode::Add => 0,
        CombineMode::Subtract => 1,
        CombineMode::Intersect => 2,
    };
    match *shape {
        ShapeWeight::Linear { start, ramp } => {
            words.floats([start[0], start[1], ramp[0], ramp[1]]);
            words.floats([0.0; 4]);
            words.uints([0, mode, 0, 0]);
        }
        ShapeWeight::Radial {
            center,
            axis_x,
            axis_y,
            fade_start,
        } => {
            words.floats([center[0], center[1], fade_start, 0.0]);
            words.floats([axis_x[0], axis_x[1], axis_y[0], axis_y[1]]);
            words.uints([1, mode, 0, 0]);
        }
    }
}

/// The develop shader's `point_colors` array (never empty: a binding
/// needs at least one element).
pub fn point_colors(plan: &DevelopPlan) -> Words {
    let mut words = Words::default();
    let points = plan
        .color
        .as_ref()
        .map_or(&[][..], |color| &color.point_colors);
    points.iter().for_each(|point| selection(&mut words, point));
    if points.is_empty() {
        (0..3).for_each(|_| words.floats([0.0; 4]));
    }
    words
}

pub fn floats(values: &[f32]) -> Words {
    Words(values.iter().map(|v| v.to_bits()).collect())
}

/// Resampling taps as the resize shader's `vec4<u32>` entries.
pub fn taps(taps: &[[usize; 3]]) -> Words {
    let mut words = Words::default();
    for &[first, offset, count] in taps {
        words.uints([first as u32, offset as u32, count as u32, 0]);
    }
    words
}

/// The finish shader's `Finish` uniform.
pub fn finish(plan: &RenderPlan) -> Words {
    let (stored_w, stored_h) = plan.target;
    let display_w = plan.orientation.display_size(stored_w, stored_h).0;
    let mut words = Words::default();
    let (radius, sharpen) = match &plan.sharpen {
        Some(sharpen) => (
            (sharpen.kernel.len() / 2) as u32,
            [
                sharpen.luma[0],
                sharpen.luma[1],
                sharpen.luma[2],
                sharpen.amount,
            ],
        ),
        None => (0, [0.0; 4]),
    };
    words.uints([radius, 0, 0, 0]);
    words.floats(sharpen);
    for row in to_stored(plan.orientation, plan.target) {
        words.ints([row[0], row[1], row[2], 0]);
    }
    words.uints([display_w as u32, (stored_w * stored_h * 3) as u32, 0, 0]);
    words
}

/// The affine map from display pixels to stored pixels of an image of
/// `stored` size: stored = (row · (x, y) + offset) per row.
fn to_stored(orientation: Orientation, (width, height): (usize, usize)) -> [[i32; 3]; 2] {
    // Orientations map pixels by a signed permutation plus an offset; probe
    // the permutation on a 3×3 image, the offset on the real one.
    let probe = |x, y| {
        let (display_x, display_y) = orientation.to_display(x, y, 3, 3);
        [display_x as i32, display_y as i32]
    };
    let difference = |a: [i32; 2], b: [i32; 2]| [a[0] - b[0], a[1] - b[1]];
    let origin = probe(0, 0);
    let along_x = difference(probe(1, 0), origin);
    let along_y = difference(probe(0, 1), origin);
    let (offset_x, offset_y) = orientation.to_display(0, 0, width, height);
    let offset = [offset_x as i32, offset_y as i32];
    // display = A · stored + offset with A's columns along_x and along_y;
    // A is orthogonal, so stored = Aᵀ · (display − offset).
    let row = |column: [i32; 2]| {
        [
            column[0],
            column[1],
            -(column[0] * offset[0] + column[1] * offset[1]),
        ]
    };
    [row(along_x), row(along_y)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_stored_inverts_every_orientation() {
        let (width, height) = (5, 3);
        for orientation in [
            Orientation::Normal,
            Orientation::FlipH,
            Orientation::Rotate180,
            Orientation::FlipV,
            Orientation::Transpose,
            Orientation::Rotate90,
            Orientation::Transverse,
            Orientation::Rotate270,
        ] {
            let rows = to_stored(orientation, (width, height));
            for y in 0..height {
                for x in 0..width {
                    let (dx, dy) = orientation.to_display(x, y, width, height);
                    let d = [dx as i32, dy as i32];
                    let back = rows.map(|r| r[0] * d[0] + r[1] * d[1] + r[2]);
                    assert_eq!(back, [x as i32, y as i32], "{orientation:?}");
                }
            }
        }
    }
}
