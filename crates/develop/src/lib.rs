// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The edit model.
//!
//! Edits are never applied to the original file. A photo's look is its
//! [`Settings`], stored next to the original as a JSON sidecar, and rendered
//! on demand by `photonation-pipeline`. Every setting has a [`Control`]
//! describing its id, range and default, so UIs, the CLI and future
//! automation share one definition. [`Mask`]s add local adjustments on
//! top of the global ones; [`PointColor`]s edit colours sampled from the
//! photo.

mod controls;
mod groups;
mod masks;
mod point_colors;
mod settings;
mod sidecar;

pub use controls::{
    BASIC_CONTROLS, Control, DETAIL_CONTROLS, GRADING_BLEND_CONTROLS, GRADING_CONTROLS,
    LOCAL_CONTROLS, MIXER_CONTROLS, PARAMETRIC_CURVE_CONTROLS, POINT_COLOR_CONTROLS, all_controls,
};
pub use groups::AdjustmentGroup;
pub use masks::{
    CombineMode, LinearGradient, LocalAdjustments, MAX_MASK_COMPONENTS, MAX_MASK_NAME_CHARS,
    MAX_MASKS, Mask, MaskComponent, MaskShape, MaskSpace, RadialGradient,
};
pub use point_colors::{MAX_POINT_COLORS, MAX_SAMPLED_CHROMA, OklabLch, PointColor};
pub use settings::{
    ColorGrading, ColorMixer, GradingWheel, HslAdjustment, ParametricCurve, PointCurve,
    ProcessVersion, Settings, Sharpening, ToneCurve, WhiteBalance,
};
pub use sidecar::{MAX_SIDECAR_BYTES, SIDECAR_EXTENSION, SidecarError, sidecar_path};
