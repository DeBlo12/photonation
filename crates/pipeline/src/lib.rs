// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The develop pipeline.
//!
//! ```text
//! sensor u16 ─▶ normalize + WB + demosaic ─▶ [removal fills] ─▶ camera→Rec.2020 · exposure · tone · colour
//!            ─▶ output primaries ─▶ [resize] ─▶ encode ─▶ [sharpen] ─▶ u8/u16 (oriented)
//! ```
//!
//! Everything runs band by band (see [`band`]); this CPU path is the
//! reference implementation any future GPU path is checked against.

pub mod band;
mod camera;
mod color;
pub mod curve;
mod develop;
mod encode;
mod local_contrast;
pub mod masks;
mod resize;
pub mod retouch;
mod sharpen;
mod source;
mod tone;

pub use camera::{CameraModel, WORKING, neutral_to_temp_tint};
pub use retouch::{
    RemovalCanvas, RemovalFills, RemovalJob, RetouchPatch, removal_canvas, removal_jobs,
};
pub use source::Demosaic;
pub use tone::BASELINE_EXPOSURE;

use band::{Graph, Point};
use photonation_color::ColorSpace;
use photonation_develop::Settings;
use photonation_image::{BitDepth, NormRect, Orientation, OutputImage, Pixels, Rect, SensorImage};

#[derive(Clone, Debug, PartialEq)]
pub struct RenderOptions {
    /// Longest output edge in pixels; `None` renders at full resolution.
    pub long_edge: Option<usize>,
    pub color_space: ColorSpace,
    pub bit_depth: BitDepth,
    /// Apply the settings' sharpening.
    pub sharpen: bool,
    /// Output rows per band; `None` picks a size from the image.
    pub band_rows: Option<usize>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            long_edge: None,
            color_space: ColorSpace::Srgb,
            bit_depth: BitDepth::Eight,
            sharpen: true,
            band_rows: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("image has no pixels")]
    Empty,
}

/// Region of the sensor a render covers: the camera crop, narrowed by the
/// user's crop (given on the displayed image).
pub fn render_region(sensor: &SensorImage, settings: &Settings) -> Rect {
    let crop = settings.crop.unwrap_or(NormRect::FULL);
    sensor
        .crop
        .sub(sensor.orientation.display_rect_to_stored(crop))
}

/// Output size (display orientation) of a render, without rendering.
pub fn output_size(
    sensor: &SensorImage,
    settings: &Settings,
    long_edge: Option<usize>,
) -> (usize, usize) {
    let (width, height) = scaled_size(render_region(sensor, settings), long_edge);
    sensor.orientation.display_size(width, height)
}

fn scaled_size(region: Rect, long_edge: Option<usize>) -> (usize, usize) {
    let longest = region.w.max(region.h);
    match long_edge {
        Some(limit) if limit < longest => {
            let scale = limit as f64 / longest as f64;
            let scaled = |len: usize| ((len as f64 * scale).round() as usize).max(1);
            (scaled(region.w), scaled(region.h))
        }
        _ => (region.w, region.h),
    }
}

/// White balance that makes the area around `point` (normalized display
/// coordinates of the rendered image) neutral: the eyedropper.
pub fn white_balance_at(
    sensor: &SensorImage,
    settings: &Settings,
    point: [f64; 2],
) -> Option<(f64, f64)> {
    let region = render_region(sensor, settings);
    let stored = sensor.orientation.display_rect_to_stored(NormRect {
        left: point[0],
        top: point[1],
        right: point[0],
        bottom: point[1],
    });
    const SAMPLE_RADIUS: usize = 4;
    let center_x = region.x + ((stored.left * region.w as f64) as usize).min(region.w - 1);
    let center_y = region.y + ((stored.top * region.h as f64) as usize).min(region.h - 1);
    let mut sums = [0.0f64; 3];
    let mut counts = [0usize; 3];
    let rows =
        center_y.saturating_sub(SAMPLE_RADIUS)..(center_y + SAMPLE_RADIUS).min(sensor.height);
    for y in rows {
        for x in
            center_x.saturating_sub(SAMPLE_RADIUS)..(center_x + SAMPLE_RADIUS).min(sensor.width)
        {
            for (color, raw) in sensor.samples_at(x, y) {
                sums[color] += (raw as f64 - sensor.black[color] as f64).max(0.0);
                counts[color] += 1;
            }
        }
    }
    let means: Vec<f64> = (0..3).map(|c| sums[c] / counts[c].max(1) as f64).collect();
    if means.iter().any(|m| *m <= 0.0) {
        return None;
    }
    let neutral = [means[0] / means[1], 1.0, means[2] / means[1]];
    Some(neutral_to_temp_tint(&sensor.color, neutral))
}

/// Renders without removal fills: removals show the original photo.
pub fn render(
    sensor: &SensorImage,
    settings: &Settings,
    opts: &RenderOptions,
) -> Result<OutputImage, RenderError> {
    render_with_fills(sensor, settings, &RemovalFills::new(), opts)
}

/// Renders with the removal fills in `fills` (see [`retouch`]); removals
/// without a fill there show the original photo.
pub fn render_with_fills(
    sensor: &SensorImage,
    settings: &Settings,
    fills: &RemovalFills,
    opts: &RenderOptions,
) -> Result<OutputImage, RenderError> {
    let settings = settings.sanitized();
    let region = render_region(sensor, &settings);
    if region.w < 2 || region.h < 2 || !sensor.is_valid() {
        return Err(RenderError::Empty);
    }
    let target = scaled_size(region, opts.long_edge);
    let graph = build_graph(sensor, region, target, &settings, fills, opts);

    // Aim for ~256 source rows per band: big enough that halos are cheap,
    // small enough to stay cache- and memory-friendly.
    let source_height = graph.source_size().1.max(1);
    let band_rows = opts
        .band_rows
        .unwrap_or_else(|| (256 * target.1 / source_height).clamp(16, 256));
    let layout = OutputLayout::new(target, sensor.orientation);
    let pixels = match opts.bit_depth {
        BitDepth::Eight => Pixels::U8(render_samples(&graph, band_rows, &layout)),
        BitDepth::Sixteen => Pixels::U16(render_samples(&graph, band_rows, &layout)),
    };
    let (width, height) = layout.display_size;
    Ok(OutputImage {
        width,
        height,
        color_space: opts.color_space,
        pixels,
    })
}

/// Assembles the stage chain for one render (see docs/PIPELINE.md).
fn build_graph<'a>(
    sensor: &'a SensorImage,
    region: Rect,
    target: (usize, usize),
    settings: &Settings,
    fills: &RemovalFills,
    opts: &RenderOptions,
) -> Graph<'a> {
    let half_size_is_enough = target.0 * 2 <= region.w && target.1 * 2 <= region.h;
    let demosaic = Demosaic::choose(&sensor.layout, half_size_is_enough);
    let camera = CameraModel::new(sensor, &settings.white_balance);
    let source = source::SensorSource::new(sensor, region, camera.wb_multipliers, demosaic);
    let source_region = source.region();

    let mut graph = Graph::new(Box::new(source));
    retouch::push_retouch(
        &mut graph,
        sensor,
        settings,
        fills,
        source_region,
        camera.wb_multipliers,
    );
    let local = masks::LocalAdjust::new(
        &settings.masks,
        sensor,
        masks::PixelToMask::new(sensor, source_region, graph.size()),
    );
    graph.push(Box::new(Point(
        develop::Develop::new(
            settings,
            &camera.camera_to_working,
            sensor.baseline_exposure,
            opts.color_space,
        )
        .with_local_adjustments(local),
    )));
    if let Some(local) =
        local_contrast::LocalContrast::new(settings.clarity, settings.texture, graph.size())
    {
        graph.push(Box::new(local));
    }
    if graph.size() != target {
        graph.push(Box::new(resize::Resize::new(graph.size(), target)));
    }
    graph.push(Box::new(Point(encode::Encode::new(opts.color_space))));
    if opts.sharpen && settings.sharpening.amount > 0.0 {
        graph.push(Box::new(sharpen::Sharpen::new(
            settings.sharpening.amount,
            settings.sharpening.radius,
        )));
    }
    graph
}

/// Where each rendered (stored-orientation) pixel lands in the output buffer.
struct OutputLayout {
    stored_size: (usize, usize),
    display_size: (usize, usize),
    orientation: Orientation,
}

impl OutputLayout {
    fn new(stored_size: (usize, usize), orientation: Orientation) -> OutputLayout {
        let display_size = orientation.display_size(stored_size.0, stored_size.1);
        OutputLayout {
            stored_size,
            display_size,
            orientation,
        }
    }

    /// Index of the first channel of stored pixel `(x, y)`.
    #[inline(always)]
    fn offset(&self, x: usize, y: usize) -> usize {
        let (stored_w, stored_h) = self.stored_size;
        let (display_x, display_y) = self.orientation.to_display(x, y, stored_w, stored_h);
        (display_y * self.display_size.0 + display_x) * 3
    }
}

/// An integer output sample type.
trait Quantize: Copy + Default + Send {
    fn quantize(value: f32) -> Self;
}

impl Quantize for u8 {
    #[inline(always)]
    fn quantize(value: f32) -> u8 {
        (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
    }
}

impl Quantize for u16 {
    #[inline(always)]
    fn quantize(value: f32) -> u16 {
        (value.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16
    }
}

/// Runs the graph band-parallel, quantizing each band straight into the
/// oriented output buffer.
fn render_samples<T: Quantize>(graph: &Graph, band_rows: usize, layout: &OutputLayout) -> Vec<T> {
    let (display_w, display_h) = layout.display_size;
    let mut buffer = vec![T::default(); display_w * display_h * 3];
    let output = SharedOut::new(&mut buffer);
    graph.run(band_rows, |band| {
        for y in band.row_range() {
            for (x, pixel) in band.row(y).chunks_exact(3).enumerate() {
                let offset = layout.offset(x, y);
                for (channel, &value) in pixel.iter().enumerate() {
                    // SAFETY: orientation maps stored pixels to display pixels
                    // one-to-one (see its tests) and bands cover disjoint rows,
                    // so no two writes target the same index.
                    unsafe { output.write(offset + channel, T::quantize(value)) }
                }
            }
        }
    });
    buffer
}

/// Output buffer written concurrently by bands at disjoint indices.
struct SharedOut<T> {
    ptr: *mut T,
    len: usize,
}

// SAFETY: access is only through `write`, whose callers guarantee disjoint indices.
unsafe impl<T: Send> Sync for SharedOut<T> {}

impl<T> SharedOut<T> {
    fn new(buffer: &mut [T]) -> SharedOut<T> {
        SharedOut {
            ptr: buffer.as_mut_ptr(),
            len: buffer.len(),
        }
    }

    /// # Safety
    /// No other thread may write index `index` concurrently.
    #[inline(always)]
    unsafe fn write(&self, index: usize, value: T) {
        assert!(index < self.len);
        // SAFETY: in bounds (checked above); exclusivity guaranteed by the caller.
        unsafe { self.ptr.add(index).write(value) }
    }
}

#[cfg(test)]
mod tests;
