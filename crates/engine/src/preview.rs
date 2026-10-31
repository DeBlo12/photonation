// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The preview worker's renderer: on the GPU where it can (feature `gpu`,
//! see `photonation-gpu`), else on the CPU pipeline. Renders the GPU path
//! doesn't cover yet (Texture and Clarity, straightened crops, removals) go
//! to the CPU one by one; a GPU failure moves every later
//! preview to the CPU.

use crate::service::{RenderedImage, render_rgb8};
use photonation_develop::Settings;
use photonation_image::SensorImage;
use photonation_pipeline::{RemovalFills, RenderOptions};
use std::sync::Arc;

/// Environment variable: `off` renders previews on the CPU only.
pub const GPU_RENDER_VARIABLE: &str = "PHOTONATION_GPU_RENDER";

pub(crate) struct PreviewRenderer {
    #[cfg(feature = "gpu")]
    gpu: Option<photonation_gpu::GpuRenderer>,
}

impl PreviewRenderer {
    /// A CPU-only renderer.
    pub fn cpu() -> PreviewRenderer {
        PreviewRenderer {
            #[cfg(feature = "gpu")]
            gpu: None,
        }
    }

    /// Uses the GPU when the `gpu` feature is on, an adapter is available
    /// and [`GPU_RENDER_VARIABLE`] doesn't say `off`. Creating the device
    /// takes a moment; call this on the worker thread.
    pub fn with_gpu() -> PreviewRenderer {
        #[cfg(feature = "gpu")]
        {
            let disabled = std::env::var(GPU_RENDER_VARIABLE)
                .is_ok_and(|value| value.trim().eq_ignore_ascii_case("off"));
            if !disabled {
                match photonation_gpu::GpuRenderer::new() {
                    Ok(gpu) => {
                        eprintln!("Previews render on {}", gpu.adapter());
                        return PreviewRenderer { gpu: Some(gpu) };
                    }
                    Err(error) => eprintln!("Previews render on the CPU: {error}"),
                }
            }
        }
        PreviewRenderer::cpu()
    }

    pub fn render(
        &mut self,
        sensor: &Arc<SensorImage>,
        settings: &Settings,
        fills: &RemovalFills,
        options: &RenderOptions,
    ) -> Result<RenderedImage, String> {
        #[cfg(feature = "gpu")]
        if let Some(gpu) = &mut self.gpu {
            match gpu.render(sensor, settings, fills, options) {
                Ok(image) => return Ok(crate::service::to_rendered(image)),
                Err(error @ photonation_gpu::GpuError::Device(_)) => {
                    eprintln!("GPU rendering failed; previews render on the CPU from now: {error}");
                    self.gpu = None;
                }
                // Not supported yet, or nothing to render: the CPU renders
                // it or reports why not.
                Err(_) => {}
            }
        }
        render_rgb8(sensor, settings, fills, options)
    }
}
