// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Photonation desktop editor.
//!
//! Layout and control flow are described in docs/DESKTOP.md.

mod app;
mod gpu;
mod panels;
mod photo_frame;
mod preferences;
mod screenshot;
mod theme;
mod widgets;

fn main() -> eframe::Result {
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Photonation")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([980.0, 620.0])
        .with_fullsize_content_view(true)
        .with_title_shown(false)
        .with_titlebar_shown(false);
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: gpu::wgpu_configuration(gpu::GpuPreference::from_env()),
        // The mask overlay's pipeline assumes neither (gpu/mask_overlay.rs).
        multisampling: 0,
        depth_buffer: 0,
        ..Default::default()
    };
    eframe::run_native(
        "Photonation",
        options,
        Box::new(|cc| Ok(Box::new(app::PhotonationApp::new(cc)))),
    )
}
