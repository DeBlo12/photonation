// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! A render described as plain data: every decision and table the band
//! graph would use, for executors other than the graph (the wgpu path in
//! `photonation-gpu`). It is built by the same code that builds the graph's
//! stages, so both render with identical parameters; only the per-pixel
//! maths is transcribed.
//!
//! Plans describe every stage of the graph, so [`PlanError::Unsupported`]
//! is reserved for stages added to the graph before their executors.

use crate::band::Source;
use crate::masks::{LocalAmounts, MaskWeight};
use crate::retouch::{RemovalWeight, RetouchPatch};
use crate::{
    Demosaic, Framing, RemovalFills, RenderOptions, camera::CameraModel, denoise::Denoise,
    develop::Develop, encode::Encode, local_contrast::LocalContrast, masks, resize::Resize,
    retouch, scaled_size, sharpen::Sharpen, source::SensorSource,
};
use photonation_develop::Settings;
use photonation_image::{Orientation, Rect, SensorImage};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct RenderPlan {
    /// The sensor area read (shrunk to whole quads for half-size demosaicing).
    pub region: Rect,
    pub demosaic: Demosaic,
    /// Demosaiced size: the region, or half of it.
    pub source_size: (usize, usize),
    /// Output size, stored orientation.
    pub target: (usize, usize),
    /// Output pixels are written in display orientation.
    pub orientation: Orientation,
    /// Per colour channel: a raw sample normalizes to
    /// `min((raw − black) × gain, clip)` (gain = white balance ÷ (white − black);
    /// clip 1, or the channel's multiplier when highlights are reconstructed).
    pub black: [f32; 3],
    pub gain: [f32; 3],
    pub clip: [f32; 3],
    /// Removal fills blended in before developing; `None` without any.
    pub retouch: Option<RetouchPlan>,
    pub develop: DevelopPlan,
    /// Noise reduction, at the source's size; `None` when it is off.
    pub denoise: Option<DenoisePlan>,
    /// Texture and Clarity, at the source's size; `None` when both are 0.
    pub local_contrast: Option<LocalContrastPlan>,
    /// A turned crop frame resampled upright; `None` without an angle.
    pub straighten: Option<StraightenPlan>,
    /// `None` when the source (or the straightened frame) already has the
    /// output size.
    pub resize: Option<ResizePlan>,
    /// The output transfer curve, sampled at `i / (len − 1)`.
    pub encode: Vec<f32>,
    pub sharpen: Option<SharpenPlan>,
}

/// The develop stage's global steps (see `develop.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct DevelopPlan {
    /// Highlight reconstruction (process V2), before everything else.
    pub reconstruction: Option<HighlightPlan>,
    /// White-balanced camera RGB → working RGB, exposure folded in.
    pub camera_to_working: [[f32; 3]; 3],
    pub working_luminance: [f32; 3],
    pub haze: f32,
    /// Fractions of ±100.
    pub highlights: f32,
    pub shadows: f32,
    pub mid_grey: f32,
    pub max_tone_shift_ev: f32,
    pub tone: TonePlan,
    /// Working RGB → LMS, LMS′ → Lab, Lab → LMS′, LMS → working RGB.
    pub oklab: [[[f32; 3]; 3]; 4],
    pub color: Option<ColorPlan>,
    /// The masks' local adjustments; `None` without active masks.
    pub local: Option<LocalPlan>,
    pub working_to_output: [[f32; 3]; 3],
    pub output_luminance: [f32; 3],
}

/// Highlight reconstruction (see `highlights.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HighlightPlan {
    /// Each channel's clip level in white-balanced camera RGB.
    pub clip: [f32; 3],
    /// Fraction of its clip where a channel starts to count as clipped.
    pub soft_start: f32,
}

/// Per-channel tone tables over log2 of the scene value (see `tone.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct TonePlan {
    pub min_ev: f32,
    pub max_ev: f32,
    /// Red, green, blue; equally long.
    pub tables: [Vec<f32>; 3],
}

/// The Oklab colour step (see `color.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorPlan {
    /// Oklab hue (degrees) at the centre of each mixer band.
    pub band_hues: [f32; 8],
    pub mixer: Option<[MixerBand; 8]>,
    pub grading: Option<GradingPlan>,
    pub saturation: f32,
    pub vibrance: f32,
    pub point_colors: Vec<SelectionPlan>,
    /// The point colour whose selection is shown instead of the edit.
    pub overlay: Option<SelectionPlan>,
}

/// One colour mixer band at full weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixerBand {
    /// Degrees.
    pub hue_shift: f32,
    /// Chroma factor before clamping at 0.
    pub chroma_gain: f32,
    /// Oklab lightness change for a fully coloured pixel.
    pub lightness_shift: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradingPlan {
    /// Shadows, midtones, highlights and global: Oklab `[a, b, lightness]` offsets.
    pub wheels: [[f32; 3]; 4],
    pub pivot: f32,
    pub transition: f32,
}

/// One point colour's selection and shifts (see `point_color.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionPlan {
    /// Lightness, chroma, hue (degrees).
    pub target: [f32; 3],
    pub hue_relevance: f32,
    pub hue_reach: f32,
    pub chroma_reach: f32,
    pub lightness_reach: f32,
    /// Share of each reach that is fully selected.
    pub solid_share: f32,
    /// Below this chroma a colour counts as grey.
    pub grey_chroma: f32,
    pub hue_shift: f32,
    pub chroma_gain: f32,
    pub lightness_shift: f32,
}

/// Every active mask and where each source pixel sits in mask space (see
/// `masks.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct LocalPlan {
    /// Metric mask point of source pixel (x, y):
    /// `origin + x · step_x + y · step_y`.
    pub origin: [f32; 2],
    pub step_x: [f32; 2],
    pub step_y: [f32; 2],
    pub masks: Vec<MaskPlan>,
    pub limits: LocalLimits,
}

/// One mask: its selection and its adjustments at full selection (amount
/// and group already applied).
#[derive(Clone, Debug, PartialEq)]
pub struct MaskPlan {
    pub weight: MaskWeight,
    pub amounts: LocalAmounts,
}

/// The develop stage's scales and caps for summed local amounts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalLimits {
    /// Haze removed at Dehaze 100 (also the global scale).
    pub max_haze: f32,
    /// Stops Temperature (red/blue) or Tint (green) move at 100.
    pub max_white_balance_ev: f32,
    /// Change of the contrast exponent at Contrast 100.
    pub max_contrast: f32,
    /// Caps on the summed exposure (stops) and tone amounts (fractions).
    pub max_exposure_ev: f32,
    pub max_tone_amount: f32,
    /// Saturation local Dehaze restores per unit.
    pub dehaze_saturation: f32,
}

/// Noise reduction (see `denoise.rs`): à trous wavelet shrinkage of
/// square-root luminance and colour differences.
#[derive(Clone, Debug, PartialEq)]
pub struct DenoisePlan {
    /// Per level, finest first (step `2^level`): the luminance and the
    /// colour threshold.
    pub thresholds: Vec<[f32; 2]>,
    /// Share of the luminance detail removed that is put back.
    pub detail_kept: f32,
}

/// Texture and Clarity (see `local_contrast.rs`): detail of √luma against
/// its blur, added back at two radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalContrastPlan {
    /// Strengths; 0 skips that blur.
    pub clarity: f32,
    pub texture: f32,
    /// Box radii in source pixels.
    pub clarity_radius: usize,
    pub texture_radius: usize,
    /// Box passes per axis (≈ Gaussian).
    pub box_passes: usize,
    pub luma: [f32; 3],
}

/// Removal fills (see `retouch.rs`): for each removal in order, its fill
/// blended in by the brush weight.
#[derive(Clone, Debug, PartialEq)]
pub struct RetouchPlan {
    /// Metric mask point of source pixel (x, y):
    /// `origin + x · step_x + y · step_y`.
    pub origin: [f32; 2],
    pub step_x: [f32; 2],
    pub step_y: [f32; 2],
    /// Sensor position of source pixel (x, y):
    /// `sensor_origin + (x + 0.5, y + 0.5) · sensor_step`.
    pub sensor_origin: [f32; 2],
    pub sensor_step: [f32; 2],
    /// A fill's sample becomes `min(sample × wb_multipliers, clip)`, as the
    /// source normalizes raw samples.
    pub wb_multipliers: [f32; 3],
    pub clip: [f32; 3],
    pub removals: Vec<RemovalPlan>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemovalPlan {
    pub weight: RemovalWeight,
    pub patch: Arc<RetouchPatch>,
}

/// Catmull–Rom resampling along a turned frame (see `straighten.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StraightenPlan {
    /// Output size, stored orientation.
    pub size: (usize, usize),
    /// Input position of output position (0, 0) (pixel `i`'s centre at
    /// `i + 0.5`), and the input distance one output pixel covers along x
    /// and along y.
    pub origin: [f32; 2],
    pub step_x: [f32; 2],
    pub step_y: [f32; 2],
}

/// Separable Lanczos-3 weights (see `resize.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct ResizePlan {
    pub horizontal: ResampleAxis,
    pub vertical: ResampleAxis,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResampleAxis {
    /// Per output coordinate: first input, offset into `weights`, tap count.
    pub taps: Vec<[usize; 3]>,
    pub weights: Vec<f32>,
}

/// Unsharp mask on encoded luma (see `sharpen.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct SharpenPlan {
    /// Fraction of the detail added back.
    pub amount: f32,
    /// Normalized 1D Gaussian, odd length.
    pub kernel: Vec<f32>,
    pub luma: [f32; 3],
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlanError {
    #[error("image has no pixels")]
    Empty,
    /// The render needs a stage plans don't describe yet.
    #[error("{0} are not supported by this renderer yet")]
    Unsupported(&'static str),
}

/// The plan of `render_with_fills(sensor, settings, fills, opts)`.
/// `opts.bit_depth` and `opts.band_rows` don't change it.
pub fn render_plan(
    sensor: &SensorImage,
    settings: &Settings,
    fills: &RemovalFills,
    opts: &RenderOptions,
) -> Result<RenderPlan, PlanError> {
    let settings = settings.sanitized();
    let framing = Framing::new(sensor, &settings);
    let region = framing.region();
    if region.w < 2 || region.h < 2 || !sensor.is_valid() {
        return Err(PlanError::Empty);
    }
    let target = scaled_size(framing.size(), opts.long_edge);
    let demosaic = crate::demosaic_for(sensor, &framing, target);
    let camera = CameraModel::new(sensor, &settings.white_balance);
    let source = SensorSource::new(
        sensor,
        region,
        camera.wb_multipliers,
        demosaic,
        settings.process.reconstructs_highlights(),
    );
    let (source_region, source_size) = (source.region(), source.size());
    let (black, gain, clip) = source.levels();
    let retouch = retouch::retouch_plan(
        sensor,
        &settings,
        fills,
        source_region,
        source_size,
        camera.wb_multipliers,
        clip,
    );
    let local = masks::LocalAdjust::new(
        &settings.masks,
        &settings.mask_groups,
        sensor,
        masks::PixelToMask::new(sensor, source_region, source_size),
    );
    let develop = Develop::new(
        &settings,
        &camera.camera_to_working,
        sensor.baseline_exposure,
        opts.color_space,
    )
    .with_highlight_reconstruction(&settings, clip)
    .with_local_adjustments(local)
    .with_point_color_overlay(&settings, opts.point_color_overlay);
    let straighten = match &framing {
        Framing::Straight(_) => None,
        Framing::Rotated { frame, size, .. } => {
            Some(crate::straighten_stage(sensor, frame, *size, source_region, source_size).plan())
        }
    };
    let developed_size = straighten.map_or(source_size, |straighten| straighten.size);
    Ok(RenderPlan {
        region: source_region,
        demosaic,
        source_size,
        target,
        orientation: sensor.orientation,
        black,
        gain,
        clip,
        retouch,
        develop: develop.plan(),
        denoise: Denoise::new(&settings.noise_reduction, demosaic.source_scale())
            .as_ref()
            .map(Denoise::plan),
        local_contrast: LocalContrast::new(settings.clarity, settings.texture, source_size)
            .as_ref()
            .map(LocalContrast::plan),
        straighten,
        resize: (developed_size != target).then(|| Resize::new(developed_size, target).plan()),
        encode: Encode::new(opts.color_space).table().to_vec(),
        sharpen: (opts.sharpen && settings.sharpening.amount > 0.0)
            .then(|| Sharpen::new(settings.sharpening.amount, settings.sharpening.radius).plan()),
    })
}
