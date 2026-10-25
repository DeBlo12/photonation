// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Colour science primitives.
//!
//! Setup maths (matrix derivation, chromatic adaptation, white balance
//! solving) runs in `f64`; the per-pixel hot paths receive `f32` matrices
//! via [`Mat3::to_f32`].

mod mat;
mod oklab;
mod spaces;
mod temperature;
mod transfer;

pub use mat::{Mat3, apply3};
pub use oklab::Oklab;
pub use spaces::{ColorSpace, D50, D65, Primaries, Xy, bradford, xy_to_xyz};
pub use temperature::{TINT_SCALE, cct_to_xy, temp_tint_to_xy, xy_to_temp_tint};
pub use transfer::Transfer;
