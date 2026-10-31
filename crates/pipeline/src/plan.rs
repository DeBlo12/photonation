// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! A render described as plain data: every decision and table the band
//! graph would use, for executors other than the graph (the wgpu path in
//! `photonation-gpu`). It is built by the same code that builds the graph's
//! stages, so both render with identical parameters; only the per-pixel
//! maths is transcribed.
//!
//! Stages a plan cannot describe yet make [`render_plan`] return
//! [`PlanError::Unsupported`]; callers render those on the CPU.

use crate::band::Source;
use crate::masks::{LocalAmounts, MaskWeight};
use crate::{
    Demosaic, Framing, RemovalFills, RenderOptions, camera::CameraModel, develop::Develop,
    encode::Encode, local_contrast::LocalContrast, masks, resize::Resize, retouch, scaled_size,
    sharpen::Sharpen, source::SensorSource,
};
use photonation_develop::Settings;
use photonation_image::{Orientation, Rect, SensorImage};

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
    /// `min((raw − black) × gain, 1)` (gain = white balance ÷ (white − black)).
    pub black: [f32; 3],
    pub gain: [f32; 3],
    pub develop: DevelopPlan,
    /// `None` when the source already has the output size.
    pub resize: Option<ResizePlan>,
    /// The output transfer curve, sampled at `i / (len − 1)`.
    pub encode: Vec<f32>,
    pub sharpen: Option<SharpenPlan>,
}

/// The develop stage's global steps (see `develop.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct DevelopPlan {
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
    if matches!(framing, Framing::Rotated { .. }) {
        return Err(PlanError::Unsupported("straightened crops"));
    }
    let target = scaled_size(framing.size(), opts.long_edge);
    let demosaic = crate::demosaic_for(sensor, &framing, target);
    let camera = CameraModel::new(sensor, &settings.white_balance);
    let source = SensorSource::new(sensor, region, camera.wb_multipliers, demosaic);
    let (source_region, source_size) = (source.region(), source.size());
    if retouch::applies(&settings.removals, fills) {
        return Err(PlanError::Unsupported("removals"));
    }
    let local = masks::LocalAdjust::new(
        &settings.masks,
        &settings.mask_groups,
        sensor,
        masks::PixelToMask::new(sensor, source_region, source_size),
    );
    if LocalContrast::new(settings.clarity, settings.texture, source_size).is_some() {
        return Err(PlanError::Unsupported("Texture and Clarity"));
    }
    let develop = Develop::new(
        &settings,
        &camera.camera_to_working,
        sensor.baseline_exposure,
        opts.color_space,
    )
    .with_local_adjustments(local)
    .with_point_color_overlay(&settings, opts.point_color_overlay);
    let (black, gain) = source.levels();
    Ok(RenderPlan {
        region: source_region,
        demosaic,
        source_size,
        target,
        orientation: sensor.orientation,
        black,
        gain,
        develop: develop.plan(),
        resize: (source_size != target).then(|| Resize::new(source_size, target).plan()),
        encode: Encode::new(opts.color_space).table().to_vec(),
        sharpen: (opts.sharpen && settings.sharpening.amount > 0.0)
            .then(|| Sharpen::new(settings.sharpening.amount, settings.sharpening.radius).plan()),
    })
}
