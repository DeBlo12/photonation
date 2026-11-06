// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Development aid: with `PHOTONATION_SCREENSHOT=<file.png>` the app saves
//! a capture of its window once the photo and thumbnails have rendered,
//! then quits. Used to check the UI and to produce documentation images.
//! `PHOTONATION_SCREENSHOT_PANELS=Curves,Color Grading` chooses which
//! adjustment cards are open; `PHOTONATION_SCREENSHOT_MASKS=1` opens the
//! masks panel with the first mask selected (`=group`: the first mask
//! group); `PHOTONATION_SCREENSHOT_CROP=1`
//! opens the crop tool; `PHOTONATION_SCREENSHOT_POINT_COLOR=1` opens the
//! Color Mixer's Point Color tab (`=range` also shows the first point
//! colour's range); `PHOTONATION_SCREENSHOT_SIDEBAR=280` starts the sidebar
//! at that width; `PHOTONATION_SCREENSHOT_EXPORT=1` opens the export sheet;
//! `PHOTONATION_SCREENSHOT_HELP=1` opens the Help window (`=Masks and mask
//! groups`, or any page's title, opens that page);
//! `PHOTONATION_SCREENSHOT_SETTINGS=general` (or `=export`) opens Settings;
//! `PHOTONATION_SCREENSHOT_LIBRARY=grid` shows the grid, `=survey` the
//! first three photos in the survey view.
//! `PHOTONATION_SCREENSHOT_PRESETS=1` opens the Presets card (`=copy`
//! also opens Copy Settings, `=new` the New Preset dialog).
//! `PHOTONATION_SCREENSHOT_FILMSTRIP=1` keeps the filmstrip open.
//! `PHOTONATION_SCREENSHOT_THEME=light` (or `=dark`) overrides the system's
//! mode. With glass panes the window comes to the front, without the keyboard,
//! and macOS's `screencapture` takes that part of the screen, since the glass
//! and what shows through it are not in what the app draws.

use eframe::egui;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Give up waiting for everything to settle after this long.
const TIMEOUT: Duration = Duration::from_secs(20);
/// Time for a settled frame to reach the screen before capturing the window.
#[cfg(target_os = "macos")]
const SCREEN_CAPTURE_DELAY: Duration = Duration::from_millis(600);

/// The sidebar's starting width: `PHOTONATION_SCREENSHOT_SIDEBAR`, so
/// narrow and wide layouts can be checked, else the usual default.
pub fn sidebar_width() -> f32 {
    static WIDTH: OnceLock<f32> = OnceLock::new();
    *WIDTH.get_or_init(|| {
        std::env::var("PHOTONATION_SCREENSHOT_SIDEBAR")
            .ok()
            .and_then(|width| width.trim().parse().ok())
            .unwrap_or(crate::theme::SIDEBAR_WIDTH)
    })
}

/// The mode asked for by `PHOTONATION_SCREENSHOT_THEME`, if any.
pub fn forced_theme() -> Option<egui::Theme> {
    match std::env::var("PHOTONATION_SCREENSHOT_THEME").ok()?.as_str() {
        "light" => Some(egui::Theme::Light),
        "dark" => Some(egui::Theme::Dark),
        _ => None,
    }
}

pub struct Screenshot {
    path: PathBuf,
    started: Instant,
    requested: bool,
    /// Where on the screen to capture, instead of what the app drew.
    #[cfg(target_os = "macos")]
    screen_region: Option<[f64; 4]>,
    #[cfg(target_os = "macos")]
    settled_at: Option<Instant>,
}

impl Screenshot {
    pub fn from_env() -> Option<Screenshot> {
        let path = std::env::var_os("PHOTONATION_SCREENSHOT")?;
        Some(Screenshot {
            path: path.into(),
            started: Instant::now(),
            requested: false,
            #[cfg(target_os = "macos")]
            screen_region: None,
            #[cfg(target_os = "macos")]
            settled_at: None,
        })
    }

    /// Captures this part of the screen when the time comes.
    #[cfg(target_os = "macos")]
    pub fn capture_screen_region(&mut self, region: [f64; 4]) {
        self.screen_region = Some(region);
    }

    /// Applies the requested panel layout.
    pub fn prepare(&self, view: &mut crate::app::ViewState) {
        if std::env::var_os("PHOTONATION_SCREENSHOT_FILMSTRIP").is_some() {
            view.filmstrip.keep_open();
        }
        if let Some(library) = std::env::var_os("PHOTONATION_SCREENSHOT_LIBRARY") {
            view.library.grid = true;
            if library == "survey" {
                view.library.survey = true;
                view.library.selection = [1, 2].into();
            }
        }
        if let Some(tab) = std::env::var_os("PHOTONATION_SCREENSHOT_SETTINGS") {
            view.show_settings = true;
            view.settings_tab = usize::from(tab == "export");
        }
        if std::env::var_os("PHOTONATION_SCREENSHOT_CROP").is_some() {
            view.cropping = true;
        }
        if std::env::var_os("PHOTONATION_SCREENSHOT_COMPARE").is_some() {
            view.compare = true;
        }
        if std::env::var_os("PHOTONATION_SCREENSHOT_REMOVE").is_some() {
            view.show_removals = true;
        }
        if let Some(presets) = std::env::var_os("PHOTONATION_SCREENSHOT_PRESETS") {
            view.presets.expanded = true;
            view.presets.show_copy = presets == "copy";
            if presets == "new" {
                view.presets.new_preset = Some(crate::presets::NewPreset {
                    name: "Golden Hour".into(),
                    group: photonation_develop::DEFAULT_PRESET_GROUP.into(),
                    parts: view.presets.preset_parts.clone(),
                });
            }
        }
        if std::env::var_os("PHOTONATION_SCREENSHOT_EXPORT").is_some() {
            view.show_export = true;
        }
        if let Some(masks) = std::env::var_os("PHOTONATION_SCREENSHOT_MASKS") {
            view.show_masks = true;
            if masks == "group" {
                view.mask_groups.selected = Some(0);
            } else {
                view.selected_mask = Some(crate::app::MaskSelection::mask(0));
            }
        }
        if let Some(point_color) = std::env::var_os("PHOTONATION_SCREENSHOT_POINT_COLOR") {
            view.mixer_tab = crate::app::POINT_COLOR_TAB;
            view.expanded
                .insert(photonation_develop::AdjustmentGroup::ColorMixer);
            view.point_color_overlay = point_color == "range";
        }
        // After the tools, so Help opens on the page of the one shown.
        if let Ok(page) = std::env::var("PHOTONATION_SCREENSHOT_HELP") {
            use crate::panels::help;
            match help::Page::from_title(&page) {
                Some(page) => help::open_page(view, page),
                None => help::open(view),
            }
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
        #[cfg(target_os = "macos")]
        if let Some(region) = self.screen_region {
            self.update_screen_capture(ctx, settled, region);
            return;
        }
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

#[cfg(target_os = "macos")]
impl Screenshot {
    fn update_screen_capture(&mut self, ctx: &egui::Context, settled: bool, region: [f64; 4]) {
        if settled || self.started.elapsed() > TIMEOUT {
            let settled_at = *self.settled_at.get_or_insert_with(Instant::now);
            if !self.requested && settled_at.elapsed() > SCREEN_CAPTURE_DELAY {
                self.requested = true;
                let captured = std::process::Command::new("/usr/sbin/screencapture")
                    .args([
                        "-x",
                        &format!("-R{},{},{},{}", region[0], region[1], region[2], region[3]),
                    ])
                    .arg(&self.path)
                    .status();
                if let Err(e) = captured {
                    eprintln!("screenshot: {e}");
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
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
