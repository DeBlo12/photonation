// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Application state and the per-frame update loop.

use crate::panels;
use crate::theme;
use crate::widgets::CurveChannel;
use eframe::egui::{self, TextureHandle, TextureOptions};
use photonation_develop::{
    AdjustmentGroup, BrushStroke, MAX_POINT_COLORS, OklabLch, PointColor, RemovalMethod, Settings,
    SettingsOrigin, lightroom_xmp_path, load_photo_settings, sidecar_path,
};
use photonation_engine::{
    Event, ExportOptions, Histogram, History, ModelStatus, PhotoInfo, PreviewKind, PreviewRequest,
    RenderService,
};
use photonation_image::NormRect;
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

/// The mask being edited and which of its shapes the handles move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaskSelection {
    /// Index into `settings.masks`.
    pub mask: usize,
    /// Index into that mask's `components`.
    pub component: usize,
}

impl MaskSelection {
    /// A mask, with its first shape.
    pub fn mask(mask: usize) -> MaskSelection {
        MaskSelection { mask, component: 0 }
    }

    /// The selection still valid for `masks` (undo, "Reset All" or a
    /// deleted shape can remove what was selected).
    pub fn validated(
        selection: Option<MaskSelection>,
        masks: &[photonation_develop::Mask],
    ) -> Option<MaskSelection> {
        let selection = selection?;
        let shapes = masks.get(selection.mask)?.components.len();
        Some(MaskSelection {
            component: selection.component.min(shapes.saturating_sub(1)),
            ..selection
        })
    }
}

/// What a click on the photo measures while a picker is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eyedropper {
    /// A neutral grey, for the white balance.
    WhiteBalance,
    /// A colour to add as a new point colour.
    PointColor,
    /// A colour whose mixer band to select.
    MixerBand,
}

impl Eyedropper {
    /// Hint shown on the picker's button.
    pub fn tooltip(self) -> &'static str {
        match self {
            Eyedropper::WhiteBalance => "Pick a neutral grey in the photo",
            Eyedropper::PointColor => "Pick a color in the photo to edit",
            Eyedropper::MixerBand => "Pick a color in the photo to select its band",
        }
    }
}

/// The Color Mixer card's tabs.
pub const MIXER_TABS: [&str; 2] = ["Mixer", "Point Color"];
pub const POINT_COLOR_TAB: usize = 1;

/// How new removals are painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RemovalBrush {
    pub method: RemovalMethod,
    /// Brush radius as a percentage of the photo's long edge.
    pub size_percent: f32,
}

impl Default for RemovalBrush {
    fn default() -> Self {
        RemovalBrush {
            method: RemovalMethod::Generative,
            size_percent: 2.0,
        }
    }
}

impl RemovalBrush {
    /// Radius in long-edge units, as removals store it.
    pub fn radius(self) -> f64 {
        f64::from(self.size_percent) / 100.0
    }
}

/// Transient view state (not saved with the photo).
pub struct ViewState {
    pub expanded: HashSet<AdjustmentGroup>,
    pub disabled: HashSet<AdjustmentGroup>,
    pub mixer_band: usize,
    /// Index into [`MIXER_TABS`].
    pub mixer_tab: usize,
    /// Index into `settings.point_colors`.
    pub selected_point_color: usize,
    /// Show the selected point colour's range in the viewer.
    pub point_color_overlay: bool,
    pub grading_wheel: usize,
    /// Index into [`panels::adjustments::CURVE_MODES`].
    pub curve_mode: usize,
    pub compare: bool,
    /// Position of the before/after divider, 0..1 across the image.
    pub compare_split: f32,
    /// The active picker, if any.
    pub eyedropper: Option<Eyedropper>,
    pub zoom: Zoom,
    pub pan: egui::Vec2,
    pub show_export: bool,
    pub export_options: ExportOptions,
    pub export_all: bool,
    /// The masks panel replaces the adjustments, and the selected shape's
    /// handles show on the photo.
    pub show_masks: bool,
    pub selected_mask: Option<MaskSelection>,
    /// The selected mask group, and which groups the layer list folds.
    pub mask_groups: panels::masks::groups::GroupView,
    /// Tint the area the selected mask affects.
    pub mask_overlay: bool,
    /// The crop tool: the whole photo with the crop frame over it, and the
    /// crop card in place of the adjustments.
    pub cropping: bool,
    pub crop_tool: panels::crop::CropTool,
    pub filmstrip: panels::filmstrip::FilmstripReveal,
    pub filmstrip_scroll: panels::filmstrip::FilmstripScroll,
    pub mask_thumbnails: panels::mask_thumbnails::MaskThumbnails,
    /// The Remove panel replaces the adjustments, and dragging on the photo
    /// paints removals.
    pub show_removals: bool,
    /// Index into `settings.removals`.
    pub selected_removal: Option<usize>,
    pub removal_brush: RemovalBrush,
    /// The stroke being painted; it joins the settings on release.
    pub painting: Option<BrushStroke>,
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
            mixer_tab: 0,
            selected_point_color: 0,
            point_color_overlay: false,
            grading_wheel: 0,
            curve_mode: 1,
            compare: false,
            compare_split: 0.5,
            eyedropper: None,
            zoom: Zoom::Fit,
            pan: egui::Vec2::ZERO,
            show_export: false,
            export_options: ExportOptions::default(),
            export_all: false,
            show_masks: false,
            selected_mask: None,
            mask_groups: Default::default(),
            mask_overlay: true,
            cropping: false,
            crop_tool: Default::default(),
            filmstrip: Default::default(),
            filmstrip_scroll: Default::default(),
            mask_thumbnails: Default::default(),
            show_removals: false,
            selected_removal: None,
            removal_brush: RemovalBrush::default(),
            painting: None,
        }
    }
}

impl ViewState {
    pub fn curve_channel(&self) -> Option<CurveChannel> {
        self.curve_mode.checked_sub(1).map(|i| CurveChannel::ALL[i])
    }

    /// Opens or closes the masks panel; it replaces the other tools.
    pub fn toggle_masks(&mut self) {
        let open = !self.show_masks;
        self.close_tools();
        self.show_masks = open;
    }

    /// Opens or closes the crop tool; it replaces the other tools.
    pub fn toggle_cropping(&mut self) {
        let open = !self.cropping;
        self.close_tools();
        self.cropping = open;
    }

    /// Opens or closes the Remove panel; it replaces the other tools.
    pub fn toggle_removals(&mut self) {
        let open = !self.show_removals;
        self.close_tools();
        self.show_removals = open;
    }

    /// Masks, crop and Remove share the sidebar and the photo's pointer
    /// input, so at most one is open.
    fn close_tools(&mut self) {
        self.show_masks = false;
        self.cropping = false;
        self.show_removals = false;
        self.painting = None;
    }

    /// The point colour whose range the viewer shows, if any: only while
    /// its tab is open and the colour mixer is switched on.
    pub fn point_color_overlay(&self, settings: &Settings) -> Option<usize> {
        let tab_visible = !self.show_masks
            && !self.cropping
            && !self.show_removals
            && self.mixer_tab == POINT_COLOR_TAB
            && self.expanded.contains(&AdjustmentGroup::ColorMixer)
            && !self.disabled.contains(&AdjustmentGroup::ColorMixer);
        (self.point_color_overlay
            && tab_visible
            && self.selected_point_color < settings.point_colors.len())
        .then_some(self.selected_point_color)
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
    /// Generation, long edge and point colour overlay of the last request.
    last_requested: Option<(u64, Option<usize>, Option<usize>)>,
    /// What the colour picker in flight was for.
    pending_color_pick: Option<Eyedropper>,
    /// The crop the "before" preview was requested with, if any.
    before_requested: Option<(Option<NormRect>, f32)>,
    pending_save: Option<Instant>,
    /// Latest message for the user and when it was posted.
    pub status: Option<(String, Instant)>,
    /// The status message is about the photo shown and goes when it does.
    status_is_about_photo: bool,
    pub export_progress: Option<(usize, usize)>,
    thumbnails_by_path: HashMap<PathBuf, usize>,
    screenshot: Option<crate::screenshot::Screenshot>,
    /// The graphics adapter in use, e.g. "AMD Radeon Pro 5500M (discrete GPU, Metal)".
    pub gpu: Option<String>,
    /// The GPU mask overlay is installed (wgpu is rendering).
    pub mask_overlay_ready: bool,
    /// Whether "Generative AI" removals run a model (None until reported).
    pub model_status: Option<ModelStatus>,
    /// Removals changed and their fills have not all arrived yet.
    pub filling_removals: bool,
}

impl PhotonationApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> PhotonationApp {
        let mut app = PhotonationApp::empty(&cc.egui_ctx);
        app.screenshot = crate::screenshot::Screenshot::from_env();
        // Load the inpainting model in the background, so the first
        // generative removal doesn't wait for it.
        app.engine.request_model_status();
        if let Some(render_state) = &cc.wgpu_render_state {
            crate::gpu::mask_overlay::install(render_state);
            let gpu = crate::gpu::describe(&render_state.adapter.get_info());
            eprintln!("Photonation: rendering on {gpu}");
            app.gpu = Some(gpu);
            app.mask_overlay_ready = true;
        }
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

    /// No folder open, no GPU and no command line: what `new` starts from,
    /// and what tests draw panels with.
    pub fn empty(ctx: &egui::Context) -> PhotonationApp {
        theme::install(ctx);
        let ctx = ctx.clone();
        PhotonationApp {
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
            pending_color_pick: None,
            before_requested: None,
            pending_save: None,
            status: None,
            status_is_about_photo: false,
            export_progress: None,
            thumbnails_by_path: HashMap::new(),
            screenshot: None,
            gpu: None,
            mask_overlay_ready: false,
            model_status: None,
            filling_removals: false,
        }
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
                        has_edits: false,
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
                for photo in &mut self.photos {
                    let settings = load_photo_settings(&photo.path)
                        .ok()
                        .flatten()
                        .map(|loaded| loaded.settings)
                        .unwrap_or_default();
                    photo.has_edits = settings != Settings::default();
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
        if self.status_is_about_photo {
            self.status = None;
        }
        self.current = Some(index);
        let path = &self.photos[index].path;
        self.settings = match load_photo_settings(path) {
            Ok(Some(loaded)) => {
                if let SettingsOrigin::Lightroom { not_imported } = loaded.origin {
                    self.notify(lightroom_notice(&not_imported));
                    self.status_is_about_photo = true;
                }
                loaded.settings
            }
            Ok(None) => Settings::default(),
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
        self.before_requested = None;
        self.view.zoom = Zoom::Fit;
        self.view.pan = egui::Vec2::ZERO;
        self.view.disabled.clear();
        self.view.selected_mask = None;
        self.view.mask_groups = Default::default();
        self.view.selected_point_color = 0;
        self.view.eyedropper = None;
        self.pending_color_pick = None;
        self.view.selected_removal = None;
        self.view.painting = None;
        self.filling_removals = false;
        self.refresh();
    }

    pub fn notify(&mut self, message: impl Into<String>) {
        self.status = Some((message.into(), Instant::now()));
        self.status_is_about_photo = false;
    }

    pub fn current_path(&self) -> Option<&PathBuf> {
        self.current.map(|i| &self.photos[i].path)
    }

    /// Call after any change to `settings`: re-render and save the sidecar.
    pub fn edited(&mut self) {
        self.refresh();
        self.mark_unsaved();
    }

    /// Save the sidecar soon, without re-rendering (the change doesn't
    /// show, like a crop edit while the crop tool shows the whole photo).
    fn mark_unsaved(&mut self) {
        self.pending_save = Some(Instant::now());
    }

    /// Re-render without marking the photo as changed.
    fn refresh(&mut self) {
        self.generation += 1;
    }

    /// Asks the engine for the colour at `point` (normalized display
    /// coordinates); `purpose` decides what the answer does.
    pub fn pick_color(&mut self, point: [f64; 2], purpose: Eyedropper) {
        let Some(path) = self.current_path().cloned() else {
            return;
        };
        // `point` is on the photo as shown: whole while the crop tool is open.
        let as_shown = |settings: Settings| {
            if self.view.cropping {
                panels::crop::uncropped(settings)
            } else {
                settings
            }
        };
        match purpose {
            Eyedropper::WhiteBalance => {
                let settings = as_shown(self.settings.clone());
                self.engine.pick_white_balance(path, settings, point)
            }
            Eyedropper::PointColor | Eyedropper::MixerBand => {
                self.pending_color_pick = Some(purpose);
                let settings = as_shown(self.effective_settings());
                self.engine.pick_color(path, settings, point);
            }
        }
    }

    /// Applies a picked colour as the pending picker asked.
    fn color_picked(&mut self, color: OklabLch) {
        match self.pending_color_pick.take() {
            Some(Eyedropper::PointColor) => {
                if self.settings.point_colors.len() >= MAX_POINT_COLORS {
                    self.notify(format!(
                        "A photo keeps at most {MAX_POINT_COLORS} point colors"
                    ));
                    return;
                }
                self.settings.point_colors.push(PointColor::new(color));
                self.view.selected_point_color = self.settings.point_colors.len() - 1;
                self.view.mixer_tab = POINT_COLOR_TAB;
                self.history.commit(&self.settings);
                self.edited();
            }
            Some(Eyedropper::MixerBand) => {
                self.view.mixer_band = photonation_pipeline::nearest_mixer_band(color.hue);
                self.view.mixer_tab = 0;
            }
            Some(Eyedropper::WhiteBalance) | None => {}
        }
    }

    /// Settings with disabled panels switched off — what is actually rendered.
    pub fn effective_settings(&self) -> Settings {
        AdjustmentGroup::apply_disabled(&self.settings, &self.view.disabled)
    }

    /// What the viewer shows: the crop tool shows the whole photo.
    fn preview_settings(&self) -> Settings {
        let settings = self.effective_settings();
        if self.view.cropping {
            panels::crop::uncropped(settings)
        } else {
            settings
        }
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
        // Without a sidecar, Lightroom's edits would come back next time.
        let resets_lightroom_edits = lightroom_xmp_path(&path).exists();
        let result = if self.settings == Settings::default() && !resets_lightroom_edits {
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
                Event::RemovalsFilled { path } if Some(&path) == self.current_path() => {
                    // Same generation, so the next frame asks again and the
                    // new preview includes the fill.
                    self.last_requested = None;
                    self.filling_removals = false;
                }
                Event::ModelStatus(status) => self.model_status = Some(status),
                Event::ColorPicked { path, color } if Some(&path) == self.current_path() => {
                    self.color_picked(color)
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
                        self.filling_removals = false;
                        self.notify(message);
                    }
                }
                _ => {}
            }
        }
    }

    /// Asks for a new preview when the settings or the needed size changed.
    /// `max_texture_side` is the GPU's limit (egui's `max_texture_side`).
    pub fn request_previews(
        &mut self,
        viewer_long_edge_px: usize,
        interacting: bool,
        max_texture_side: usize,
    ) {
        let Some(path) = self.current_path().cloned() else {
            return;
        };
        let full_long_edge = self.info.as_ref().map(|i| i.size.0.max(i.size.1));
        let long_edge = preview_long_edge(
            viewer_long_edge_px,
            self.view.zoom,
            full_long_edge,
            interacting,
            max_texture_side,
        );
        let point_color_overlay = self.view.point_color_overlay(&self.settings);
        let key = (self.generation, long_edge, point_color_overlay);
        if self.last_requested != Some(key) {
            if self
                .last_requested
                .is_some_and(|(_, _, overlay)| overlay != point_color_overlay)
            {
                // A new look: newer than any preview already shown.
                self.generation += 1;
            }
            self.last_requested = Some((self.generation, long_edge, point_color_overlay));
            self.engine.request_preview(PreviewRequest {
                path: path.clone(),
                settings: self.preview_settings(),
                long_edge,
                generation: self.generation,
                kind: PreviewKind::Edited,
                point_color_overlay,
            });
        }
        // The original, framed like the edit so the two line up.
        let (crop, crop_angle) = if self.view.cropping {
            (None, 0.0)
        } else {
            (self.settings.crop, self.settings.crop_angle)
        };
        if self.view.compare && self.before_requested != Some((crop, crop_angle)) {
            self.before_requested = Some((crop, crop_angle));
            self.engine.request_preview(PreviewRequest {
                path,
                settings: Settings {
                    crop,
                    crop_angle,
                    ..Settings::default()
                },
                long_edge: Some(viewer_long_edge_px.min(max_texture_side.max(1))),
                generation: self.generation,
                kind: PreviewKind::Before,
                point_color_overlay: None,
            });
        }
    }

    pub fn is_rendering(&self) -> bool {
        self.current.is_some() && self.rendered_generation < self.generation
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        // Plain keys belong to a focused text field (e.g. a mask's name).
        let typing = ctx.egui_wants_keyboard_input();
        let (
            undo,
            redo,
            prev,
            next,
            compare,
            masks,
            crop,
            removals,
            leave_crop,
            export,
            open,
            reset_zoom,
        ) = ctx.input_mut(|i| {
            let mut plain = |key| !typing && i.consume_key(Modifiers::NONE, key);
            let (prev, next, compare, masks, crop, removals) = (
                plain(Key::ArrowLeft),
                plain(Key::ArrowRight),
                plain(Key::Backslash),
                plain(Key::M),
                plain(Key::R),
                plain(Key::Q),
            );
            let leave_crop = self.view.cropping && (plain(Key::Enter) || plain(Key::Escape));
            (
                i.consume_key(Modifiers::COMMAND, Key::Z),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                prev,
                next,
                compare,
                masks,
                crop,
                removals,
                leave_crop,
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
        if masks && self.current.is_some() {
            self.view.toggle_masks();
        }
        if (crop && self.current.is_some()) || leave_crop {
            self.view.toggle_cropping();
        }
        if removals && self.current.is_some() {
            self.view.toggle_removals();
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

/// Tells the user a photo opened with its Lightroom edits, and which of
/// them Photonation could not bring along.
fn lightroom_notice(not_imported: &[&str]) -> String {
    match not_imported {
        [] => "Opened with its Lightroom edits".into(),
        [only] => format!("Opened with its Lightroom edits, without {only}"),
        [first @ .., last] => format!(
            "Opened with its Lightroom edits, without {} and {last}",
            first.join(", ")
        ),
    }
}

/// Preview resolution: the viewer size when fitted, more when zoomed in,
/// never more than the GPU can hold in one texture (a larger upload fails
/// and takes the app down). While a control is being dragged, stay at fit
/// size for responsiveness. `None` means full resolution.
fn preview_long_edge(
    viewer_long_edge_px: usize,
    zoom: Zoom,
    full_long_edge: Option<usize>,
    interacting: bool,
    max_texture_side: usize,
) -> Option<usize> {
    // Round up so small window resizes don't trigger re-renders.
    let fit = viewer_long_edge_px.div_ceil(256) * 256;
    let wanted = match (zoom, full_long_edge) {
        (Zoom::Scale(scale), Some(full)) if !interacting => {
            let wanted = ((full as f32 * scale).ceil() as usize).max(fit);
            if wanted >= full {
                None
            } else {
                Some(wanted.div_ceil(256) * 256)
            }
        }
        _ => Some(fit),
    };
    let limit = max_texture_side.max(1);
    match wanted {
        Some(edge) if edge > limit => Some(limit),
        None if full_long_edge.is_some_and(|full| full > limit) => Some(limit),
        other => other,
    }
}

impl eframe::App for PhotonationApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_events(&ctx);
        self.shortcuts(&ctx);

        let settings_before = self.settings.clone();
        let disabled_before = self.view.disabled.clone();
        let cropping_before = self.view.cropping;

        panels::toolbar::show(self, ui);
        if !self.photos.is_empty() {
            panels::filmstrip::show(self, ui);
        }
        if self.current.is_some() {
            panels::adjustments::show(self, ui);
        }
        panels::viewer::show(self, ui);
        self.view.selected_mask =
            MaskSelection::validated(self.view.selected_mask, &self.settings.masks);
        self.view
            .mask_groups
            .validate(self.settings.mask_groups.len());
        self.view.selected_removal = self
            .view
            .selected_removal
            .filter(|index| *index < self.settings.removals.len());
        if self.view.show_export {
            panels::export::show(self, &ctx);
        }

        if self.settings.removals != settings_before.removals {
            self.filling_removals = self.settings.removals.iter().any(|r| r.is_active());
        }
        if self.view.disabled != disabled_before
            || (self.settings != settings_before
                && !(self.view.cropping
                    && panels::crop::only_crop_differs(&self.settings, &settings_before)))
        {
            self.edited();
        } else if self.settings != settings_before {
            self.mark_unsaved();
        }
        if self.view.cropping != cropping_before {
            self.refresh();
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

#[cfg(test)]
mod tests {
    use super::*;

    const GPU_LIMIT: usize = 8192;

    #[test]
    fn lightroom_notices_list_what_was_left_out() {
        assert_eq!(lightroom_notice(&[]), "Opened with its Lightroom edits");
        assert_eq!(
            lightroom_notice(&["masks", "noise reduction", "effects"]),
            "Opened with its Lightroom edits, without masks, noise reduction and effects"
        );
    }

    #[test]
    fn fitted_previews_follow_the_viewer() {
        assert_eq!(
            preview_long_edge(1500, Zoom::Fit, Some(6000), false, GPU_LIMIT),
            Some(1536)
        );
    }

    #[test]
    fn zoomed_previews_use_full_resolution_unless_dragging() {
        assert_eq!(
            preview_long_edge(1500, Zoom::Scale(1.0), Some(6000), false, GPU_LIMIT),
            None
        );
        assert_eq!(
            preview_long_edge(1500, Zoom::Scale(0.5), Some(6000), false, GPU_LIMIT),
            Some(3072)
        );
        assert_eq!(
            preview_long_edge(1500, Zoom::Scale(1.0), Some(6000), true, GPU_LIMIT),
            Some(1536)
        );
    }

    #[test]
    fn previews_never_exceed_the_gpu_texture_limit() {
        // A 61 MP Sony (9504 px) at 1:1 would need a texture the GPU lacks.
        assert_eq!(
            preview_long_edge(1500, Zoom::Scale(1.0), Some(9504), false, GPU_LIMIT),
            Some(GPU_LIMIT)
        );
        assert_eq!(
            preview_long_edge(1500, Zoom::Scale(0.95), Some(9504), false, GPU_LIMIT),
            Some(GPU_LIMIT)
        );
        // A tiny software rasterizer limit caps even fitted previews.
        assert_eq!(
            preview_long_edge(3000, Zoom::Fit, Some(6000), false, 2048),
            Some(2048)
        );
    }
}
