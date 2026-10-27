// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The develop pipeline.
//!
//! ```text
//! sensor u16 ─▶ normalize + WB + demosaic ─▶ camera→Rec.2020 · exposure · tone · colour
//!            ─▶ output primaries ─▶ [straighten] ─▶ [resize] ─▶ encode ─▶ [sharpen]
//!            ─▶ u8/u16 (oriented)
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
pub mod point_color;
mod resize;
mod sharpen;
mod source;
mod straighten;
mod tone;

pub use camera::{CameraModel, WORKING, neutral_to_temp_tint};
pub use color::nearest_mixer_band;
pub use source::Demosaic;
pub use tone::BASELINE_EXPOSURE;

use band::{Graph, Point, Source};
use photonation_color::{ColorSpace, Oklab};
use photonation_develop::{OklabLch, Settings};
use photonation_image::{
    BitDepth, CropFrame, NormRect, Orientation, OutputImage, Pixels, Rect, SensorImage,
};

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
    /// Show this point colour's selection instead of the edit (an index
    /// into `settings.point_colors`): what it selects keeps its colour,
    /// the rest turns grey.
    pub point_color_overlay: Option<usize>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            long_edge: None,
            color_space: ColorSpace::Srgb,
            bit_depth: BitDepth::Eight,
            sharpen: true,
            band_rows: None,
            point_color_overlay: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("image has no pixels")]
    Empty,
}

/// What part of the sensor a render reads, and how its output maps onto it.
enum Framing {
    /// An unrotated crop is a rectangle on the sensor, rendered as it is.
    Straight(Rect),
    /// A rotated crop frame: the sensor is read around it, then resampled.
    Rotated {
        region: Rect,
        frame: CropFrame,
        /// Full-resolution output size, stored orientation.
        size: (usize, usize),
    },
}

/// Photo pixels read around a rotated frame for the bicubic resampling.
const STRAIGHTEN_MARGIN: f64 = 3.0;

impl Framing {
    /// `settings` must be sanitized.
    fn new(sensor: &SensorImage, settings: &Settings) -> Framing {
        let orientation = sensor.orientation;
        if settings.crop_angle == 0.0 {
            let crop = settings.crop.unwrap_or(NormRect::FULL);
            return Framing::Straight(sensor.crop.sub(orientation.display_rect_to_stored(crop)));
        }
        let (photo_w, photo_h) = orientation.display_size(sensor.crop.w, sensor.crop.h);
        let photo = [photo_w as f64, photo_h as f64];
        let frame = crop_frame(photo, settings.crop, settings.crop_angle);
        let bounds = frame.bounds();
        let (margin_x, margin_y) = (STRAIGHTEN_MARGIN / photo[0], STRAIGHTEN_MARGIN / photo[1]);
        let around = NormRect {
            left: bounds.left - margin_x,
            top: bounds.top - margin_y,
            right: bounds.right + margin_x,
            bottom: bounds.bottom + margin_y,
        }
        .clamped();
        let [frame_w, frame_h] = frame.size().map(|side| (side.round() as usize).max(1));
        Framing::Rotated {
            region: sensor.crop.sub(orientation.display_rect_to_stored(around)),
            frame,
            // Swapping axes is its own inverse: display → stored size.
            size: orientation.display_size(frame_w, frame_h),
        }
    }

    /// The sensor area the render reads.
    fn region(&self) -> Rect {
        match self {
            Framing::Straight(region) | Framing::Rotated { region, .. } => *region,
        }
    }

    /// Full-resolution output size, stored orientation.
    fn size(&self) -> (usize, usize) {
        match self {
            Framing::Straight(region) => (region.w, region.h),
            Framing::Rotated { size, .. } => *size,
        }
    }
}

/// The frame a render shows of a `photo`-sized photo (pixels, display
/// orientation): the crop's corners turned by `angle` and kept on the photo
/// (Lightroom does the same; it also tames hand-edited sidecars). Without a
/// crop, the whole photo, or when turned the largest frame of its aspect.
pub fn crop_frame(photo: [f64; 2], crop: Option<NormRect>, angle: f32) -> CropFrame {
    let angle = angle as f64;
    match crop {
        Some(corners) if angle == 0.0 => CropFrame::new(photo, corners.clamped(), 0.0),
        Some(corners) => CropFrame::new(photo, corners.coordinates_clamped(), angle).constrained(),
        None if angle == 0.0 => CropFrame::full(photo),
        None => CropFrame::largest(photo, photo[0] / photo[1], angle),
    }
}

/// Region of the sensor a render reads: the camera crop, narrowed to the
/// user's crop (given on the displayed image), or around it when rotated.
pub fn render_region(sensor: &SensorImage, settings: &Settings) -> Rect {
    Framing::new(sensor, &settings.sanitized()).region()
}

/// Output size (display orientation) of a render, without rendering.
pub fn output_size(
    sensor: &SensorImage,
    settings: &Settings,
    long_edge: Option<usize>,
) -> (usize, usize) {
    let (width, height) = scaled_size(
        Framing::new(sensor, &settings.sanitized()).size(),
        long_edge,
    );
    sensor.orientation.display_size(width, height)
}

fn scaled_size((width, height): (usize, usize), long_edge: Option<usize>) -> (usize, usize) {
    let longest = width.max(height);
    match long_edge {
        Some(limit) if limit < longest => {
            let scale = limit as f64 / longest as f64;
            let scaled = |len: usize| ((len as f64 * scale).round() as usize).max(1);
            (scaled(width), scaled(height))
        }
        _ => (width, height),
    }
}

/// Radius, in sensor pixels, of the patch the pickers average.
const PICK_RADIUS: usize = 4;

/// The sensor patch the pickers average around `point` (normalized
/// display coordinates of the rendered image), inside the rendered region
/// (a turned crop's point is found through its frame). `None` for an empty
/// region.
fn pick_patch(sensor: &SensorImage, settings: &Settings, point: [f64; 2]) -> Option<Rect> {
    let orientation = sensor.orientation;
    let (region, stored) = match Framing::new(sensor, &settings.sanitized()) {
        Framing::Straight(region) => (region, orientation.to_stored_norm(point[0], point[1])),
        Framing::Rotated { frame, .. } => {
            let [photo_x, photo_y] = frame.to_photo(point);
            (sensor.crop, orientation.to_stored_norm(photo_x, photo_y))
        }
    };
    if region.w == 0 || region.h == 0 {
        return None;
    }
    let at = |offset: usize, fraction: f64, len: usize| {
        offset + ((fraction.max(0.0) * len as f64) as usize).min(len - 1)
    };
    let center_x = at(region.x, stored.0, region.w);
    let center_y = at(region.y, stored.1, region.h);
    let left = center_x.saturating_sub(PICK_RADIUS).max(region.x);
    let top = center_y.saturating_sub(PICK_RADIUS).max(region.y);
    let right = (center_x + PICK_RADIUS).min(region.x + region.w);
    let bottom = (center_y + PICK_RADIUS).min(region.y + region.h);
    Some(Rect::new(left, top, right - left, bottom - top))
}

/// White balance that makes the area around `point` (normalized display
/// coordinates of the rendered image) neutral: the eyedropper.
pub fn white_balance_at(
    sensor: &SensorImage,
    settings: &Settings,
    point: [f64; 2],
) -> Option<(f64, f64)> {
    let patch = pick_patch(sensor, settings, point)?;
    let mut sums = [0.0f64; 3];
    let mut counts = [0usize; 3];
    for y in patch.y..patch.y + patch.h {
        for x in patch.x..patch.x + patch.w {
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

/// The colour around `point` (normalized display coordinates of the
/// rendered image) as point colours see it: developed with every setting
/// except the point colours themselves, before the output conversion.
/// Spatial stages (Texture, Clarity, sharpening) don't take part.
pub fn color_at(sensor: &SensorImage, settings: &Settings, point: [f64; 2]) -> Option<OklabLch> {
    let mut settings = settings.sanitized();
    settings.point_colors.clear();
    if !sensor.is_valid() {
        return None;
    }
    let patch = pick_patch(sensor, &settings, point)?;
    let camera = CameraModel::new(sensor, &settings.white_balance);
    let source = source::SensorSource::new(
        sensor,
        patch,
        camera.wb_multipliers,
        Demosaic::choose(&sensor.layout, false),
    );
    let size = source.size();
    let local = masks::LocalAdjust::new(
        &settings.masks,
        sensor,
        masks::PixelToMask::new(sensor, source.region(), size),
    );
    let develop = develop::Develop::new(
        &settings,
        &camera.camera_to_working,
        sensor.baseline_exposure,
        ColorSpace::Srgb,
    )
    .with_local_adjustments(local);
    let band = source.render(0..size.1);
    let mut sum = [0.0f32; 3];
    for y in band.row_range() {
        for (x, pixel) in band.row(y).chunks_exact(3).enumerate() {
            let working = develop.working_at(x, y, [pixel[0], pixel[1], pixel[2]]);
            for channel in 0..3 {
                sum[channel] += working[channel];
            }
        }
    }
    let count = (size.0 * size.1) as f32;
    if count == 0.0 {
        return None;
    }
    let mean = sum.map(|v| (v / count).max(0.0));
    Some(OklabLch::from_lab(Oklab::new(WORKING).from_rgb(mean)).sanitized())
}

pub fn render(
    sensor: &SensorImage,
    settings: &Settings,
    opts: &RenderOptions,
) -> Result<OutputImage, RenderError> {
    let settings = settings.sanitized();
    let framing = Framing::new(sensor, &settings);
    let region = framing.region();
    if region.w < 2 || region.h < 2 || !sensor.is_valid() {
        return Err(RenderError::Empty);
    }
    let target = scaled_size(framing.size(), opts.long_edge);
    let graph = build_graph(sensor, &framing, target, &settings, opts);

    // Aim for ~256 source rows per band: big enough that halos are cheap,
    // small enough to stay cache- and memory-friendly.
    let source_height = graph.source_size().1.max(1);
    let band_rows = opts.band_rows.unwrap_or_else(|| {
        let rows = (256 * target.1 / source_height).clamp(16, 256);
        match framing {
            Framing::Straight(_) => rows,
            // A tilted frame's bands read input rows that overlap their
            // neighbours' by the tilt; one band per thread keeps the
            // overlap, which is computed twice, small.
            Framing::Rotated { .. } => rows.max(target.1.div_ceil(rayon::current_num_threads())),
        }
    });
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
    framing: &Framing,
    target: (usize, usize),
    settings: &Settings,
    opts: &RenderOptions,
) -> Graph<'a> {
    let region = framing.region();
    let full_size = framing.size();
    let half_size_is_enough = target.0 * 2 <= full_size.0 && target.1 * 2 <= full_size.1;
    let demosaic = Demosaic::choose(&sensor.layout, half_size_is_enough);
    let camera = CameraModel::new(sensor, &settings.white_balance);
    let source = source::SensorSource::new(sensor, region, camera.wb_multipliers, demosaic);
    let source_region = source.region();

    let mut graph = Graph::new(Box::new(source));
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
        .with_local_adjustments(local)
        .with_point_color_overlay(settings, opts.point_color_overlay),
    )));
    if let Some(local) =
        local_contrast::LocalContrast::new(settings.clarity, settings.texture, graph.size())
    {
        graph.push(Box::new(local));
    }
    if let Framing::Rotated { frame, size, .. } = framing {
        graph.push(Box::new(straighten_stage(
            sensor,
            frame,
            *size,
            source_region,
            graph.size(),
        )));
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

/// Resamples the developed source (`input` pixels covering `source_region`
/// of the sensor) along `frame`, at the source's scale.
fn straighten_stage(
    sensor: &SensorImage,
    frame: &CropFrame,
    full_size: (usize, usize),
    source_region: Rect,
    input: (usize, usize),
) -> straighten::Straighten {
    // Input pixels per sensor pixel (½ after half-size demosaicing).
    let input_scale = [
        input.0 as f64 / source_region.w.max(1) as f64,
        input.1 as f64 / source_region.h.max(1) as f64,
    ];
    let out = (
        ((full_size.0 as f64 * input_scale[0]).round() as usize).max(1),
        ((full_size.1 as f64 * input_scale[1]).round() as usize).max(1),
    );
    let (orientation, crop) = (sensor.orientation, sensor.crop);
    // Output (stored) → output displayed → photo displayed → photo stored
    // → sensor → input; every step is affine.
    straighten::Straighten::new(out, |[x, y]| {
        let (display_x, display_y) =
            orientation.to_display_norm(x / out.0 as f64, y / out.1 as f64);
        let [photo_x, photo_y] = frame.to_photo([display_x, display_y]);
        let (stored_x, stored_y) = orientation.to_stored_norm(photo_x, photo_y);
        let sensor_x = crop.x as f64 + stored_x * crop.w as f64;
        let sensor_y = crop.y as f64 + stored_y * crop.h as f64;
        [
            (sensor_x - source_region.x as f64) * input_scale[0],
            (sensor_y - source_region.y as f64) * input_scale[1],
        ]
    })
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
