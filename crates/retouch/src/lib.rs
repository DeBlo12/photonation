// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Inpainting: filling a hole in an image with content that fits.
//!
//! An [`Inpainter`] takes a [`Canvas`] (an RGB image plus a hole mask) and
//! returns the image with the hole filled. Two are provided:
//!
//! * [`ContentAwareFill`]: multi-scale PatchMatch (Wexler et al., Barnes
//!   et al.), copying texture from the rest of the canvas. Pure Rust,
//!   deterministic for a given seed, always available.
//! * [`OnnxInpainter`] (feature `onnx`): a neural network (LaMa or
//!   MI-GAN) that can invent structure found nowhere else in the photo
//!   (the cow behind the fly, the field behind the fence).
//!
//! Nothing here knows about raw files or the develop pipeline; the
//! pipeline cuts canvases out of the photo and blends fills back in. See
//! docs/RETOUCH.md.

mod canvas;
#[cfg(feature = "onnx")]
mod onnx;
mod patch_match;

pub use canvas::Canvas;
#[cfg(feature = "onnx")]
pub use onnx::{ModelLayout, OnnxInpainter};
pub use patch_match::{ContentAwareFill, Refined};

#[derive(Debug, thiserror::Error)]
pub enum InpaintError {
    #[error("the canvas is empty or its buffers do not match its size")]
    InvalidCanvas,
    #[error("could not load the inpainting model: {0}")]
    Model(String),
    #[error("inpainting failed: {0}")]
    Inference(String),
}

/// Fills the hole of a canvas.
pub trait Inpainter: Send + Sync {
    /// Short name for logs and the UI ("Content-aware", "LaMa").
    fn name(&self) -> &str;

    /// The canvas's pixels with every hole pixel replaced; pixels outside
    /// the hole come back unchanged. `seed` varies fills that have a random
    /// component; the same seed gives the same fill.
    fn inpaint(&self, canvas: &Canvas, seed: u32) -> Result<Vec<f32>, InpaintError>;
}
