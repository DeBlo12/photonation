// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Photonation desktop editor.
//!
//! Layout and control flow are described in docs/DESKTOP.md.

// A GUI app on Windows: no console window next to the editor. Debug builds
// keep the console for log output.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

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
        .with_icon(icon())
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([980.0, 620.0])
        .with_fullsize_content_view(true)
        .with_title_shown(false)
        .with_titlebar_shown(false);
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: gpu::wgpu_configuration(&preferences::Preferences::load().gpu),
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

/// The app icon for the title bar, taskbar and, unbundled, the Dock. On
/// macOS it sits on Apple's icon grid like the other Dock icons. Made by
/// packaging/icons from the brand design.
fn icon() -> eframe::egui::IconData {
    let png: &[u8] = if cfg!(target_os = "macos") {
        include_bytes!("../../../assets/icon/photonation-macos-512.png")
    } else {
        include_bytes!("../../../assets/icon/photonation-256.png")
    };
    eframe::icon_data::from_png_bytes(png).unwrap_or_default()
}
