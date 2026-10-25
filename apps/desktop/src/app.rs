// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Application state and the per-frame update loop.

use crate::panels;
use crate::theme;
use crate::widgets::CurveChannel;
use eframe::egui::{self, TextureHandle, TextureOptions};
use photonation_develop::{AdjustmentGroup, Settings, sidecar_path};
use photonation_engine::{
    Event, ExportOptions, Histogram, History, PhotoInfo, PreviewKind, PreviewRequest, RenderService,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Wait this long after the last edit before writing the sidecar.
const SIDECAR_DEBOUNCE: Duration = Duration::from_millis(400);

pub struct Photo {
    pub path: PathBuf,
    pub name: String,
    pub thumbnail: Option<TextureHandle>,
    pub has_edits: bool,
    /// The decoder could not read this file.
    pub unsupported: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    Fit,
    /// Physical screen pixels per image pixel.
    Scale(f32),
}

/// A rendered image on the GPU.
pub struct Preview {
    pub texture: TextureHandle,
    pub size: [usize; 2],
}

/// Transient view state (not saved with the photo).
pub struct ViewState {
    pub expanded: HashSet<AdjustmentGroup>,
    pub disabled: HashSet<AdjustmentGroup>,
    pub mixer_band: usize,
    pub grading_wheel: usize,
    /// Index into [`panels::adjustments::CURVE_MODES`].
    pub curve_mode: usize,
    pub compare: bool,
    /// Position of the before/after divider, 0..1 across the image.
    pub compare_split: f32,
    pub eyedropper: bool,
    pub zoom: Zoom,
    pub pan: egui::Vec2,
    pub show_export: bool,
    pub export_options: ExportOptions,
    pub export_all: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            expanded: [
                AdjustmentGroup::WhiteBalance,
                AdjustmentGroup::Light,
                AdjustmentGroup::Color,
            ]
            .into(),
            disabled: HashSet::new(),
            mixer_band: 0,
            grading_wheel: 0,
            curve_mode: 1,
            compare: false,
            compare_split: 0.5,
            eyedropper: false,
            zoom: Zoom::Fit,
            pan: egui::Vec2::ZERO,
            show_export: false,
            export_options: ExportOptions::default(),
            export_all: false,
        }
    }
}

impl ViewState {
    pub fn curve_channel(&self) -> Option<CurveChannel> {
        self.curve_mode.checked_sub(1).map(|i| CurveChannel::ALL[i])
    }
}

pub struct PhotonationApp {
    pub engine: RenderService,
    pub folder: Option<PathBuf>,
    pub photos: Vec<Photo>,
    pub current: Option<usize>,
    pub settings: Settings,
    pub history: History,
    pub info: Option<PhotoInfo>,
    pub view: ViewState,
    pub preview: Option<Preview>,
    pub before: Option<Preview>,
    pub histogram: Option<Histogram>,
    /// Bumped on every edit; previews carry the generation they were made for.
    generation: u64,
    rendered_generation: u64,
    last_requested: Option<(u64, Option<usize>)>,
    before_requested: bool,
    pending_save: Option<Instant>,
    /// Latest message for the user and when it was posted.
    pub status: Option<(String, Instant)>,
    pub export_progress: Option<(usize, usize)>,
    thumbnails_by_path: HashMap<PathBuf, usize>,
    screenshot: Option<crate::screenshot::Screenshot>,
}

impl PhotonationApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> PhotonationApp {
        theme::install(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let mut app = PhotonationApp {
            engine: RenderService::new(move || ctx.request_repaint()),
            folder: None,
            photos: Vec::new(),
            current: None,
            settings: Settings::default(),
            history: History::default(),
            info: None,
            view: ViewState::default(),
            preview: None,
            before: None,
            histogram: None,
            generation: 0,
            rendered_generation: 0,
            last_requested: None,
            before_requested: false,
            pending_save: None,
            status: None,
            export_progress: None,
            thumbnails_by_path: HashMap::new(),
            screenshot: crate::screenshot::Screenshot::from_env(),
        };
        // `Photonation <folder>` or `Photonation <photo>` (opens its folder).
        if let Some(target) = std::env::args().nth(1).map(PathBuf::from) {
            let folder = if target.is_dir() {
                Some(target.clone())
            } else {
                target.parent().map(PathBuf::from)
            };
            if let Some(folder) = folder {
                app.open_folder(folder);
                if let Some(index) = app.photos.iter().position(|p| p.path == target) {
                    app.select(index);
                }
            }
        }
        if let Some(screenshot) = &app.screenshot {
            screenshot.prepare(&mut app.view);
        }
        app
    }

    pub fn open_folder(&mut self, folder: PathBuf) {
        self.save_now();
        match photonation_engine::scan_folder(&folder) {
            Ok(paths) => {
                self.photos = paths
                    .into_iter()
                    .map(|path| Photo {
                        name: path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        has_edits: sidecar_path(&path).exists(),
                        thumbnail: None,
                        unsupported: false,
                        path,
                    })
                    .collect();
                self.thumbnails_by_path = self
                    .photos
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (p.path.clone(), i))
                    .collect();
                for photo in &self.photos {
                    let settings = Settings::load(&sidecar_path(&photo.path))
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    self.engine.request_thumbnail(photo.path.clone(), settings);
                }
                if self.photos.is_empty() {
                    self.notify("No supported raw files in this folder");
                }
                self.folder = Some(folder);
                self.current = None;
                self.select(0);
            }
            Err(e) => self.notify(format!("Could not open folder: {e}")),
        }
    }

    pub fn select(&mut self, index: usize) {
        if index >= self.photos.len() || self.current == Some(index) {
            return;
        }
        self.save_now();
        self.current = Some(index);
        let path = &self.photos[index].path;
        self.settings = match Settings::load(&sidecar_path(path)) {
            Ok(settings) => settings.unwrap_or_default(),
            Err(e) => {
                self.notify(e.to_string());
                Settings::default()
            }
        };
        self.history = History::new(self.settings.clone());
        self.info = None;
        self.preview = None;
        self.before = None;
        self.histogram = None;
        self.before_requested = false;
        self.view.zoom = Zoom::Fit;
        self.view.pan = egui::Vec2::ZERO;
        self.view.disabled.clear();
        self.refresh();
    }

    pub fn notify(&mut self, message: impl Into<String>) {
        self.status = Some((message.into(), Instant::now()));
    }

    pub fn current_path(&self) -> Option<&PathBuf> {
        self.current.map(|i| &self.photos[i].path)
    }

    /// Call after any change to `settings`: re-render and save the sidecar.
    pub fn edited(&mut self) {
        self.refresh();
        self.pending_save = Some(Instant::now());
    }

    /// Re-render without marking the photo as changed.
    fn refresh(&mut self) {
        self.generation += 1;
    }

    /// Settings with disabled panels switched off — what is actually rendered.
    pub fn effective_settings(&self) -> Settings {
        AdjustmentGroup::apply_disabled(&self.settings, &self.view.disabled)
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.history.undo() {
            self.settings = previous;
            self.edited();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.history.redo() {
            self.settings = next;
            self.edited();
        }
    }

    pub fn save_now(&mut self) {
        let (Some(_), Some(path)) = (self.pending_save.take(), self.current_path().cloned()) else {
            return;
        };
        let sidecar = sidecar_path(&path);
        let result = if self.settings == Settings::default() {
            // Nothing to remember: don't leave empty sidecars around.
            match std::fs::remove_file(&sidecar) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            }
        } else {
            self.settings.save(&sidecar).map_err(|e| e.to_string())
        };
        match result {
            Ok(()) => {
                if let Some(index) = self.current {
                    self.photos[index].has_edits = self.settings != Settings::default();
                    self.engine.request_thumbnail(path, self.settings.clone());
                }
            }
            Err(message) => self.notify(message),
        }
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        let events: Vec<Event> = self.engine.poll().collect();
        for event in events {
            match event {
                Event::PhotoLoaded(info) if Some(&info.path) == self.current_path() => {
                    self.info = Some(info)
                }
                Event::Preview {
                    path,
                    generation,
                    kind,
                    image,
                } if Some(&path) == self.current_path() => {
                    let color_image =
                        egui::ColorImage::from_rgb([image.width, image.height], &image.rgb);
                    let slot = match kind {
                        PreviewKind::Edited => {
                            if generation < self.rendered_generation {
                                continue;
                            }
                            self.rendered_generation = generation;
                            self.histogram = Some(image.histogram);
                            &mut self.preview
                        }
                        PreviewKind::Before => &mut self.before,
                    };
                    let size = [image.width, image.height];
                    match slot {
                        Some(preview) => {
                            preview.texture.set(color_image, TextureOptions::LINEAR);
                            preview.size = size;
                        }
                        None => {
                            let texture = ctx.load_texture(
                                format!("{kind:?}"),
                                color_image,
                                TextureOptions::LINEAR,
                            );
                            *slot = Some(Preview { texture, size });
                        }
                    }
                }
                Event::Thumbnail { path, image } => {
                    if let Some(&index) = self.thumbnails_by_path.get(&path) {
                        let color_image =
                            egui::ColorImage::from_rgb([image.width, image.height], &image.rgb);
                        let texture = ctx.load_texture(
                            format!("thumb-{index}"),
                            color_image,
                            TextureOptions::LINEAR,
                        );
                        self.photos[index].thumbnail = Some(texture);
                    }
                }
                Event::WhiteBalancePicked {
                    path,
                    temperature,
                    tint,
                } if Some(&path) == self.current_path() => {
                    self.settings.white_balance = photonation_develop::WhiteBalance::Custom {
                        temperature: temperature.round(),
                        tint: tint.round(),
                    };
                    self.history.commit(&self.settings);
                    self.edited();
                }
                Event::ExportProgress { done, total } => self.export_progress = Some((done, total)),
                Event::ExportFinished { exported, failures } => {
                    self.export_progress = None;
                    self.notify(if failures.is_empty() {
                        format!("Exported {exported} photo(s)")
                    } else {
                        format!(
                            "Exported {exported}, {} failed: {}",
                            failures.len(),
                            failures[0].1
                        )
                    });
                }
                Event::Error { path, message } => {
                    // Background thumbnail failures only mark the photo; tell
                    // the user about errors for the photo they are looking at.
                    if let Some(&index) = path.as_ref().and_then(|p| self.thumbnails_by_path.get(p))
                    {
                        self.photos[index].unsupported = true;
                    }
                    if path.is_none() || path.as_ref() == self.current_path() {
                        self.notify(message);
                    }
                }
                _ => {}
            }
        }
    }

    /// Asks for a new preview when the settings or the needed size changed.
    pub fn request_previews(&mut self, viewer_long_edge_px: usize, interacting: bool) {
        let Some(path) = self.current_path().cloned() else {
            return;
        };
        let long_edge = self.needed_long_edge(viewer_long_edge_px, interacting);
        let key = (self.generation, long_edge);
        if self.last_requested != Some(key) {
            self.last_requested = Some(key);
            self.engine.request_preview(PreviewRequest {
                path: path.clone(),
                settings: self.effective_settings(),
                long_edge,
                generation: self.generation,
                kind: PreviewKind::Edited,
            });
        }
        if self.view.compare && !self.before_requested {
            self.before_requested = true;
            self.engine.request_preview(PreviewRequest {
                path,
                settings: Settings::default(),
                long_edge: Some(viewer_long_edge_px),
                generation: self.generation,
                kind: PreviewKind::Before,
            });
        }
    }

    /// Preview resolution: the viewer size when fitted, more when zoomed in.
    /// While a control is being dragged, stay at fit size for responsiveness.
    fn needed_long_edge(&self, viewer_long_edge_px: usize, interacting: bool) -> Option<usize> {
        // Round up so small window resizes don't trigger re-renders.
        let fit = viewer_long_edge_px.div_ceil(256) * 256;
        let full = self.info.as_ref().map(|i| i.size.0.max(i.size.1));
        match (self.view.zoom, full) {
            (Zoom::Scale(scale), Some(full)) if !interacting => {
                let wanted = ((full as f32 * scale).ceil() as usize).max(fit);
                if wanted >= full {
                    None
                } else {
                    Some(wanted.div_ceil(256) * 256)
                }
            }
            _ => Some(fit),
        }
    }

    pub fn is_rendering(&self) -> bool {
        self.current.is_some() && self.rendered_generation < self.generation
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let (undo, redo, prev, next, compare, export, open, reset_zoom) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Z),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                i.consume_key(Modifiers::NONE, Key::ArrowRight),
                i.consume_key(Modifiers::NONE, Key::Backslash),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::E),
                i.consume_key(Modifiers::COMMAND, Key::O),
                i.consume_key(Modifiers::COMMAND, Key::Num0),
            )
        });
        if redo {
            self.redo();
        } else if undo {
            self.undo();
        }
        if let Some(current) = self.current {
            if prev && current > 0 {
                self.select(current - 1);
            }
            if next {
                self.select(current + 1);
            }
        }
        if compare {
            self.view.compare = !self.view.compare;
        }
        if export && self.current.is_some() {
            self.view.show_export = true;
        }
        if open {
            self.pick_folder();
        }
        if reset_zoom {
            self.view.zoom = Zoom::Fit;
            self.view.pan = egui::Vec2::ZERO;
        }
    }

    pub fn pick_folder(&mut self) {
        if let Some(folder) = rfd::FileDialog::new()
            .set_title("Open a folder of raw photos")
            .pick_folder()
        {
            self.open_folder(folder);
        }
    }
}

impl eframe::App for PhotonationApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_events(&ctx);
        self.shortcuts(&ctx);

        let settings_before = self.settings.clone();
        let disabled_before = self.view.disabled.clone();

        panels::toolbar::show(self, ui);
        if !self.photos.is_empty() {
            panels::filmstrip::show(self, ui);
        }
        if self.current.is_some() {
            panels::adjustments::show(self, ui);
        }
        panels::viewer::show(self, ui);
        if self.view.show_export {
            panels::export::show(self, &ctx);
        }

        if self.settings != settings_before || self.view.disabled != disabled_before {
            self.edited();
        }
        // One undo step per interaction: commit once the pointer is released.
        if !ctx.input(|i| i.pointer.any_down()) {
            self.history.commit(&self.settings);
        }
        let settled = !self.is_rendering()
            && self.preview.is_some()
            && self
                .photos
                .iter()
                .all(|p| p.thumbnail.is_some() || p.unsupported);
        if let Some(screenshot) = &mut self.screenshot {
            screenshot.update(&ctx, settled);
        }
        if self
            .pending_save
            .is_some_and(|t| t.elapsed() > SIDECAR_DEBOUNCE)
            && !ctx.input(|i| i.pointer.any_down())
        {
            self.save_now();
        } else if self.pending_save.is_some() {
            ctx.request_repaint_after(SIDECAR_DEBOUNCE);
        }
    }

    fn on_exit(&mut self) {
        self.save_now();
    }
}
