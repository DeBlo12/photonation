// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Development aid: with `PHOTONATION_SCREENSHOT=<file.png>` the app saves
//! a capture of its window once the photo and thumbnails have rendered,
//! then quits. Used to check the UI and to produce documentation images.
//! `PHOTONATION_SCREENSHOT_PANELS=Curves,Color Grading` chooses which
//! adjustment cards are open; `PHOTONATION_SCREENSHOT_MASKS=1` opens the
//! masks panel with the first mask selected.

use eframe::egui;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Give up waiting for everything to settle after this long.
const TIMEOUT: Duration = Duration::from_secs(20);

pub struct Screenshot {
    path: PathBuf,
    started: Instant,
    requested: bool,
}

impl Screenshot {
    pub fn from_env() -> Option<Screenshot> {
        let path = std::env::var_os("PHOTONATION_SCREENSHOT")?;
        Some(Screenshot {
            path: path.into(),
            started: Instant::now(),
            requested: false,
        })
    }

    /// Applies the requested panel layout.
    pub fn prepare(&self, view: &mut crate::app::ViewState) {
        if std::env::var_os("PHOTONATION_SCREENSHOT_MASKS").is_some() {
            view.show_masks = true;
            view.selected_mask = Some(crate::app::MaskSelection::mask(0));
        }
        let Ok(panels) = std::env::var("PHOTONATION_SCREENSHOT_PANELS") else {
            return;
        };
        let wanted: Vec<&str> = panels.split(',').map(str::trim).collect();
        view.expanded = photonation_develop::AdjustmentGroup::ALL
            .into_iter()
            .filter(|g| wanted.contains(&g.title()))
            .collect();
    }

    pub fn update(&mut self, ctx: &egui::Context, settled: bool) {
        if !self.requested && (settled || self.started.elapsed() > TIMEOUT) {
            self.requested = true;
            let (path, ctx_for_close) = (self.path.clone(), ctx.clone());
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                egui::ScreenshotCallback::new(move |image| {
                    if let Err(e) = save_png(&path, &image) {
                        eprintln!("screenshot: {e}");
                    }
                    ctx_for_close.send_viewport_cmd(egui::ViewportCommand::Close);
                    ctx_for_close.request_repaint();
                }),
            ));
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

fn save_png(
    path: &std::path::Path,
    image: &egui::ColorImage,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, image.size[0] as u32, image.size[1] as u32);
    encoder.set_color(png::ColorType::Rgba);
    let mut writer = encoder.write_header()?;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    writer.write_image_data(&bytes)?;
    Ok(())
}
