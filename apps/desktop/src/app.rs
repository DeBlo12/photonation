// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Application state and the per-frame update loop.

use crate::panels;
use crate::theme;
use crate::widgets::CurveChannel;
use eframe::egui::{self, TextureHandle, TextureOptions};
use photonation_develop::{
    AdjustmentGroup, BrushStroke, LibraryInfo, MAX_POINT_COLORS, MaskStroke, OklabLch, PointColor,
    RemovalMethod, Settings, SettingsOrigin, load_photo, save_photo,
};
use photonation_engine::{
    Event, ExportOptions, Histogram, History, ModelStatus, PhotoInfo, PreviewKind, PreviewRequest,
    RenderService,
};
use photonation_image::NormRect;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Wait this long after the last edit before writing the sidecar.
const SIDECAR_DEBOUNCE: Duration = Duration::from_millis(400);

pub struct Photo {
    pub path: PathBuf,
    pub name: String,
    pub thumbnail: Option<TextureHandle>,
    /// The thumbnail is a render with the photo's settings, not the
    /// camera's embedded preview.
    pub thumbnail_rendered: bool,
    pub has_edits: bool,
    /// Rating, flag and label.
    pub library: LibraryInfo,
    /// Capture time as EXIF writes it, once the thumbnail pass read it.
    pub captured: Option<String>,
    /// The decoder could not read this file.
    pub unsupported: bool,
}

impl Photo {
    /// A photo not read yet: no thumbnail, edits or rating.
    pub fn new(path: PathBuf) -> Photo {
        Photo {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path,
            thumbnail: None,
            thumbnail_rendered: false,
            has_edits: false,
            library: LibraryInfo::default(),
            captured: None,
            unsupported: false,
        }
    }
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

/// The photo alone on the whole screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhotoFullScreen {
    /// The window was already full screen, so it stays that way on leaving.
    pub window_was_full: bool,
    /// The system has reported the window full screen, so once it stops, the
    /// user left through the system (the green button, ⌃⌘F) and this ends too.
    pub window_reached: bool,
}

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

/// How new mask strokes are painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskBrushTool {
    /// Brush radius as a percentage of the photo's long edge.
    pub size_percent: f32,
    pub feather: f32,
    pub flow: f32,
    /// The panel's Paint or Erase; holding Alt does the opposite.
    pub erase: bool,
}

impl Default for MaskBrushTool {
    fn default() -> Self {
        let stroke = MaskStroke::default();
        MaskBrushTool {
            size_percent: 2.0,
            feather: stroke.feather,
            flow: stroke.flow,
            erase: stroke.erase,
        }
    }
}

impl MaskBrushTool {
    /// Radius in long-edge units, as strokes store it.
    pub fn radius(self) -> f64 {
        f64::from(self.size_percent) / 100.0
    }
}

/// Transient view state (not saved with the photo).
pub struct ViewState {
    /// Where the inspector is this frame, if shown.
    pub inspector_rect: Option<egui::Rect>,
    /// Where the filmstrip is this frame, if shown.
    pub filmstrip_rect: Option<egui::Rect>,
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
    /// The Settings window, and its tab (index into `panels::settings::TABS`).
    pub show_settings: bool,
    pub settings_tab: usize,
    pub show_about: bool,
    /// The Help window and its page.
    pub help: panels::help::HelpView,
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
    pub filmstrip_bar: panels::filmstrip::FilmstripScrollBar,
    pub mask_thumbnails: panels::mask_thumbnails::MaskThumbnails,
    /// The Remove panel replaces the adjustments, and dragging on the photo
    /// paints removals.
    pub show_removals: bool,
    /// Filter, sort, selection, and the grid and survey views.
    pub library: crate::library::LibraryView,
    /// When the open tool's panel was last outlined because the grid or
    /// survey waited for it to close.
    pub tool_warning: Option<Instant>,
    /// Index into `settings.removals`.
    pub selected_removal: Option<usize>,
    pub full_screen: Option<PhotoFullScreen>,
    pub removal_brush: RemovalBrush,
    /// A gradient just added, while it is still the selected shape: the next
    /// drag on the photo places it.
    pub shape_to_draw: Option<MaskSelection>,
    pub mask_brush: MaskBrushTool,
    /// The mask stroke being painted. It renders like any stroke, but joins
    /// the settings only on release: one undo step, and the interface never
    /// waits for a weight map while the pointer moves.
    pub mask_stroke: Option<MaskStroke>,
    /// The stroke being painted; it joins the settings on release.
    pub painting: Option<BrushStroke>,
    /// Presets, copied settings and their dialogs.
    pub presets: crate::presets::PresetsView,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            inspector_rect: None,
            filmstrip_rect: None,
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
            show_settings: false,
            settings_tab: 0,
            show_about: false,
            help: Default::default(),
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
            filmstrip_bar: Default::default(),
            mask_thumbnails: Default::default(),
            show_removals: false,
            library: Default::default(),
            tool_warning: None,
            selected_removal: None,
            full_screen: None,
            removal_brush: RemovalBrush::default(),
            shape_to_draw: None,
            mask_brush: MaskBrushTool::default(),
            mask_stroke: None,
            painting: None,
            presets: Default::default(),
        }
    }
}

impl ViewState {
    /// Whether something open needs the panels that full screen hides.
    pub fn needs_panels(&self) -> bool {
        self.show_masks
            || self.cropping
            || self.show_removals
            || self.library.grid
            || self.library.survey
    }

    pub fn curve_channel(&self) -> Option<CurveChannel> {
        self.curve_mode.checked_sub(1).map(|i| CurveChannel::ALL[i])
    }

    /// Opens or closes the masks panel; it replaces the other tools.
    /// Whether a dialog that has to be answered first is open.
    pub fn dialog_open(&self) -> bool {
        self.show_export
            || self.show_settings
            || self.show_about
            || self.library.confirm_trash
            || self.presets.show_copy
            || self.presets.new_preset.is_some()
            || self.presets.confirm_delete.is_some()
    }

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
        self.crop_tool.reset();
    }

    /// Opens or closes the Remove panel; it replaces the other tools.
    pub fn toggle_removals(&mut self) {
        let open = !self.show_removals;
        self.close_tools();
        self.show_removals = open;
    }

    /// Masks, crop and Remove share the sidebar and the photo's pointer
    /// input, so at most one is open. They need the editor, so the grid
    /// and survey close too.
    pub(crate) fn close_tools(&mut self) {
        self.show_masks = false;
        self.cropping = false;
        self.show_removals = false;
        self.painting = None;
        self.mask_stroke = None;
        self.library.grid = false;
        self.library.survey = false;
        self.tool_warning = None;
    }

    /// The open editing tool's name and the key that closes it.
    pub fn open_tool(&self) -> Option<(&'static str, &'static str)> {
        if self.show_masks {
            Some(("Masks", "M"))
        } else if self.cropping {
            Some(("Crop", "R"))
        } else if self.show_removals {
            Some(("Remove", "Q"))
        } else {
            None
        }
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

/// The open photo as the panels found it: what differs from this once they
/// have drawn is an edit.
#[derive(Default)]
struct PhotoBeforePanels {
    settings: Settings,
    disabled: HashSet<AdjustmentGroup>,
}

pub struct PhotonationApp {
    pub engine: RenderService,
    pub folder: Option<PathBuf>,
    pub folders: crate::folders::FolderBrowser,
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
    photo_before_panels: PhotoBeforePanels,
    /// Latest message for the user and when it was posted.
    pub status: Option<(String, Instant)>,
    /// The status message is about the photo shown and goes when it does.
    status_is_about_photo: bool,
    pub export_progress: Option<(usize, usize)>,
    thumbnails_by_path: HashMap<PathBuf, usize>,
    screenshot: Option<crate::screenshot::Screenshot>,
    #[cfg(target_os = "macos")]
    glass: Option<crate::glass::GlassPanes>,
    #[cfg(target_os = "macos")]
    native_menus: Option<crate::native_menu::NativeMenus>,
    /// The graphics adapter in use, e.g. "AMD Radeon Pro 5500M (discrete GPU, Metal)".
    pub gpu: Option<String>,
    /// Settings and remembered choices (`preferences.json`).
    pub preferences: crate::preferences::Preferences,
    /// The GPU setting the window was created with; a different one takes
    /// effect at the next start.
    pub gpu_setting_at_launch: crate::gpu::GpuSetting,
    /// The GPU mask overlay is installed (wgpu is rendering).
    pub mask_overlay_ready: bool,
    /// Whether "Generative AI" removals run a model (None until reported).
    pub model_status: Option<ModelStatus>,
    /// Removals changed and their fills have not all arrived yet.
    pub filling_removals: bool,
    /// Large images for the survey view, and the size each was asked at.
    pub survey_images: HashMap<PathBuf, Preview>,
    pub survey_requested: HashMap<PathBuf, usize>,
}

impl PhotonationApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> PhotonationApp {
        let repaint = cc.egui_ctx.clone();
        let engine = RenderService::new(move || repaint.request_repaint());
        let mut app = PhotonationApp::with_engine(&cc.egui_ctx, engine);
        app.screenshot = crate::screenshot::Screenshot::from_env();
        #[cfg(target_os = "macos")]
        {
            app.glass = crate::glass::GlassPanes::install();
            if let Some(glass) = &app.glass {
                theme::use_glass();
                if let Some(forced_theme) = crate::screenshot::forced_theme() {
                    crate::glass::force_appearance(Some(forced_theme));
                }
                if let Some(screenshot) = &mut app.screenshot
                    && let Some(region) = glass.front_for_capture()
                {
                    screenshot.capture_screen_region(region);
                }
            }
            let menus = crate::menus::menus(&app, true);
            app.native_menus = Some(crate::native_menu::NativeMenus::install(
                &cc.egui_ctx,
                menus,
            ));
        }
        app.preferences = crate::preferences::Preferences::load();
        if crate::screenshot::forced_theme().is_none() {
            theme::set_appearance(&cc.egui_ctx, app.preferences.appearance);
        }
        app.gpu_setting_at_launch = app.preferences.gpu.clone();
        app.view.export_options = app.preferences.export_options();
        app.preferences.library.apply(&mut app.view.library);
        app.load_presets();
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
            if target.is_dir() {
                app.open_folder(target);
            } else {
                app.open_photo(&target);
            }
        }
        if let Some(screenshot) = &app.screenshot {
            screenshot.prepare(&mut app.view);
        }
        app
    }

    /// No folder open, no GPU and no command line: what tests draw panels
    /// with. Previews render on the CPU, so tests never open a GPU device
    /// (a preview thread still setting one up when the test process exits
    /// can crash it).
    #[cfg(test)]
    pub fn empty(ctx: &egui::Context) -> PhotonationApp {
        let repaint = ctx.clone();
        let engine = RenderService::on_cpu(move || repaint.request_repaint());
        PhotonationApp::with_engine(ctx, engine)
    }

    /// No folder open and no command line, rendering with `engine`: what
    /// `new` starts from.
    fn with_engine(ctx: &egui::Context, engine: RenderService) -> PhotonationApp {
        theme::install(ctx);
        if let Some(forced_theme) = crate::screenshot::forced_theme() {
            ctx.set_theme(forced_theme);
        }
        PhotonationApp {
            engine,
            folder: None,
            folders: Default::default(),
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
            photo_before_panels: Default::default(),
            status: None,
            status_is_about_photo: false,
            export_progress: None,
            thumbnails_by_path: HashMap::new(),
            screenshot: None,
            #[cfg(target_os = "macos")]
            glass: None,
            #[cfg(target_os = "macos")]
            native_menus: None,
            gpu: None,
            preferences: Default::default(),
            gpu_setting_at_launch: Default::default(),
            mask_overlay_ready: false,
            model_status: None,
            filling_removals: false,
            survey_images: HashMap::new(),
            survey_requested: HashMap::new(),
        }
    }

    pub fn open_folder(&mut self, folder: PathBuf) {
        self.save_now();
        match photonation_engine::scan_folder(&folder) {
            Ok(paths) => {
                self.engine.cancel_thumbnails();
                self.photos = paths.into_iter().map(Photo::new).collect();
                self.view.library.selection.clear();
                self.view.library.survey = false;
                self.survey_images.clear();
                self.survey_requested.clear();
                self.thumbnails_by_path = self
                    .photos
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (p.path.clone(), i))
                    .collect();
                for photo in &mut self.photos {
                    let loaded = load_photo(&photo.path).unwrap_or_default();
                    let settings = loaded.settings.map(|s| s.settings).unwrap_or_default();
                    photo.has_edits = settings != Settings::default();
                    photo.library = loaded.library;
                    self.engine.request_thumbnail(photo.path.clone(), settings);
                }
                self.folders.folder_opened(&folder);
                self.engine.scan_subfolders(folder.clone());
                self.folder = Some(folder);
                self.close_photo();
                self.select(0);
            }
            Err(e) => self.notify(format!("Could not open folder: {e}")),
        }
    }

    /// Opens the folder a photo is in, on that photo, so the filmstrip and the
    /// grid work as they do for any folder.
    pub fn open_photo(&mut self, photo: &Path) {
        // A bare file name on the command line has an empty parent.
        let photo = std::path::absolute(photo).unwrap_or_else(|_| photo.to_path_buf());
        let Some(folder) = photo.parent() else {
            return;
        };
        self.open_folder(folder.to_path_buf());
        if let Some(index) = self.photos.iter().position(|p| p.path == photo) {
            self.select(index);
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
        self.settings = match load_photo(path).map(|loaded| loaded.settings) {
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
        self.forget_rendered_photo();
        self.view.zoom = Zoom::Fit;
        self.view.pan = egui::Vec2::ZERO;
        self.view.disabled.clear();
        self.view.selected_mask = None;
        self.view.shape_to_draw = None;
        self.view.mask_groups = Default::default();
        self.view.selected_point_color = 0;
        self.view.eyedropper = None;
        self.pending_color_pick = None;
        self.view.selected_removal = None;
        self.view.painting = None;
        self.view.mask_stroke = None;
        self.filling_removals = false;
        self.remember_photo_before_panels();
        self.refresh();
    }

    /// Saves and closes the current photo, leaving none open: nothing of it
    /// stays behind to undo or to show.
    pub fn close_photo(&mut self) {
        self.save_now();
        if self.status_is_about_photo {
            self.status = None;
        }
        self.current = None;
        self.settings = Settings::default();
        self.history = History::default();
        self.forget_rendered_photo();
        self.remember_photo_before_panels();
    }

    fn forget_rendered_photo(&mut self) {
        self.info = None;
        self.preview = None;
        self.before = None;
        self.histogram = None;
        self.before_requested = None;
    }

    /// Turns the photo alone on the whole screen on or off. The tools need
    /// their panels, so they close; the sheets open over it as usual.
    pub fn toggle_full_screen(&mut self, ctx: &egui::Context) {
        if self.view.full_screen.is_some() {
            self.leave_full_screen(ctx);
        } else if self.current.is_some() && !self.showing_folder_overview() {
            self.view.close_tools();
            self.view.eyedropper = None;
            let window_is_full = ctx.input(|i| i.viewport().fullscreen) == Some(true);
            self.view.full_screen = Some(PhotoFullScreen {
                window_was_full: window_is_full,
                window_reached: window_is_full,
            });
            self.set_window_full_screen(ctx, true);
            self.notify("Press F or Esc to leave full screen");
        }
    }

    /// The window alone, with the panels still in it (F11 where the system
    /// offers no full screen of its own).
    pub fn toggle_window_full_screen(&self, ctx: &egui::Context) {
        let window_is_full = ctx.input(|i| i.viewport().fullscreen) == Some(true);
        self.set_window_full_screen(ctx, !window_is_full);
    }

    fn leave_full_screen(&mut self, ctx: &egui::Context) {
        if let Some(state) = self.view.full_screen.take()
            && !state.window_was_full
        {
            self.set_window_full_screen(ctx, false);
        }
    }

    /// A screenshot run leaves the window alone: going full screen would move
    /// whoever is working at the computer to another Space.
    fn set_window_full_screen(&self, ctx: &egui::Context, full: bool) {
        if self.screenshot.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(full));
        }
    }

    /// Ends full screen when something needs the panels back, a key opened a
    /// tool or the grid, say, or when the system took the window out of it.
    fn follow_full_screen(&mut self, ctx: &egui::Context) {
        let Some(state) = &mut self.view.full_screen else {
            return;
        };
        match ctx.input(|i| i.viewport().fullscreen) {
            Some(true) => state.window_reached = true,
            Some(false) if state.window_reached => {
                self.view.full_screen = None;
                return;
            }
            _ => {}
        }
        if self.current.is_none() || self.view.needs_panels() {
            self.leave_full_screen(ctx);
        }
    }

    pub fn notify(&mut self, message: impl Into<String>) {
        self.status = Some((message.into(), Instant::now()));
        self.status_is_about_photo = false;
    }

    pub fn current_path(&self) -> Option<&PathBuf> {
        self.current.map(|i| &self.photos[i].path)
    }

    /// Taken before the panels draw and again whenever a photo opens or
    /// closes, which panels can do midway through a frame: a photo's own
    /// settings replacing the last one's are not an edit of it.
    fn remember_photo_before_panels(&mut self) {
        self.photo_before_panels = PhotoBeforePanels {
            settings: self.settings.clone(),
            disabled: self.view.disabled.clone(),
        };
    }

    /// Acts on what the panels changed since `remember_photo_before_panels`.
    fn notice_panel_edits(&mut self) {
        let before = &self.photo_before_panels;
        if self.settings.removals != before.settings.removals {
            self.filling_removals = self.settings.removals.iter().any(|r| r.is_active());
        }
        let settings_changed = self.settings != before.settings;
        let disabled_changed = self.view.disabled != before.disabled;
        let only_hidden_crop_changed =
            self.view.cropping && panels::crop::only_crop_differs(&self.settings, &before.settings);
        if disabled_changed || (settings_changed && !only_hidden_crop_changed) {
            self.edited();
        } else if settings_changed {
            self.mark_unsaved();
        }
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
    pub(crate) fn refresh(&mut self) {
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

    /// What the viewer shows: the crop tool shows the whole photo, and a
    /// preset under the pointer shows on the photo.
    fn preview_settings(&self) -> Settings {
        let mut settings = self
            .view
            .presets
            .hover
            .clone()
            .unwrap_or_else(|| self.effective_settings());
        panels::mask_brush::with_stroke_in_progress(&mut settings, &self.view);
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
        let (Some(_), Some(index)) = (self.pending_save.take(), self.current) else {
            return;
        };
        let path = self.photos[index].path.clone();
        // Removes the sidecar when there is nothing to remember.
        match save_photo(&path, &self.settings, &self.photos[index].library) {
            Ok(()) => {
                self.photos[index].has_edits = self.settings != Settings::default();
                self.engine
                    .refresh_thumbnail(path.clone(), self.settings.clone());
                self.survey_images.remove(&path);
                self.survey_requested.remove(&path);
            }
            Err(e) => self.notify(e.to_string()),
        }
    }

    /// Writes photo `index`'s sidecar after its rating, flag or label
    /// changed. Other photos keep the settings their sidecars hold; the
    /// current one is saved with its edits, if any are waiting.
    pub fn save_library(&mut self, index: usize) {
        if self.current == Some(index) && self.pending_save.is_some() {
            self.save_now();
            return;
        }
        let path = self.photos[index].path.clone();
        let library = self.photos[index].library;
        let result = if self.current == Some(index) {
            save_photo(&path, &self.settings, &library)
        } else {
            // A sidecar that can't be read is left alone, not replaced.
            load_photo(&path).and_then(|loaded| {
                let settings = loaded.settings.map(|s| s.settings).unwrap_or_default();
                save_photo(&path, &settings, &library)
            })
        };
        if let Err(e) = result {
            self.notify(e.to_string());
        }
    }

    pub(crate) fn handle_events(&mut self, ctx: &egui::Context) {
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
                    if kind == PreviewKind::Edited && self.view.cropping {
                        self.view.crop_tool.rendered(generation, image.rgb, size);
                    }
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
                Event::Thumbnail {
                    path,
                    image,
                    rendered,
                } => {
                    if let Some(&index) = self.thumbnails_by_path.get(&path) {
                        let photo = &mut self.photos[index];
                        // A render is never replaced by the embedded preview.
                        if photo.thumbnail_rendered && !rendered {
                            continue;
                        }
                        let name = format!("thumb-{index}");
                        photo.thumbnail = Some(thumbnail_texture(ctx, name, &image));
                        photo.thumbnail_rendered = rendered;
                    } else {
                        self.folders.cover_arrived(ctx, &path, &image, rendered);
                    }
                }
                Event::SubfolderFound { parent, subfolder } => {
                    self.subfolder_found(&parent, subfolder);
                }
                Event::SubfoldersScanned { parent } => self.subfolders_scanned(&parent),
                Event::CaptureTime { path, captured } => {
                    if let Some(&index) = self.thumbnails_by_path.get(&path) {
                        self.photos[index].captured = Some(captured);
                    }
                }
                Event::SurveyImage { path, image } if self.survey_requested.contains_key(&path) => {
                    let color_image =
                        egui::ColorImage::from_rgb([image.width, image.height], &image.rgb);
                    let texture = ctx.load_texture(
                        format!("survey-{}", path.display()),
                        color_image,
                        TextureOptions::LINEAR,
                    );
                    let size = [image.width, image.height];
                    self.survey_images.insert(path, Preview { texture, size });
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
                Event::ModelStatus(status) => {
                    // A model found after start-up redoes the fills made
                    // without it (the engine dropped them).
                    let was_ready = self
                        .model_status
                        .as_ref()
                        .is_some_and(ModelStatus::is_ready);
                    if status.is_ready() && !was_ready && !self.settings.removals.is_empty() {
                        self.refresh();
                    }
                    self.model_status = Some(status);
                }
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
            if self.view.cropping {
                self.view.crop_tool.requested(self.generation);
            }
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
            open_photo,
            open,
            reset_zoom,
            settings,
            help,
            close_help,
            copy_settings,
            paste_settings,
            full_screen,
            leave_full_screen,
            window_full_screen,
        ) = ctx.input_mut(|i| {
            // Before the crop tool and the sheets see Esc: Help is in front.
            let close_help =
                self.view.help.open && !typing && i.consume_key(Modifiers::NONE, Key::Escape);
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
            // A sheet in front gets Esc first.
            let sheet_open = self.view.show_export || self.view.show_settings;
            let full_screen = plain(Key::F);
            let leave_full_screen =
                self.view.full_screen.is_some() && !sheet_open && plain(Key::Escape);
            let window_full_screen = cfg!(not(target_os = "macos")) && plain(Key::F11);
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
                // Before ⌘O, which also matches with Shift held.
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::O),
                i.consume_key(Modifiers::COMMAND, Key::O),
                i.consume_key(Modifiers::COMMAND, Key::Num0),
                i.consume_key(Modifiers::COMMAND, Key::Comma),
                i.consume_key(Modifiers::NONE, Key::F1),
                close_help,
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::C),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::V),
                full_screen,
                leave_full_screen,
                window_full_screen,
            )
        });
        if full_screen || leave_full_screen {
            self.toggle_full_screen(ctx);
        }
        if window_full_screen {
            self.toggle_window_full_screen(ctx);
        }
        if redo {
            self.redo();
        } else if undo {
            self.undo();
        }
        if prev {
            self.step(-1);
        }
        if next {
            self.step(1);
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
        if open_photo {
            self.pick_photo();
        }
        if open {
            self.pick_folder();
        }
        if settings {
            self.view.show_settings = true;
        }
        if help {
            panels::help::open(&mut self.view);
        } else if close_help {
            self.view.help.open = false;
        }
        if reset_zoom {
            self.view.zoom = Zoom::Fit;
            self.view.pan = egui::Vec2::ZERO;
        }
        if copy_settings && self.current.is_some() {
            self.view.presets.show_copy = true;
        }
        if paste_settings {
            self.paste_settings();
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

    pub fn pick_photo(&mut self) {
        // Linux's dialogs match extensions by case, and cameras write capitals.
        let extensions: Vec<String> = photonation_engine::raw_extensions()
            .flat_map(|extension| [extension.to_ascii_uppercase(), extension])
            .collect();
        if let Some(photo) = rfd::FileDialog::new()
            .set_title("Open a raw photo")
            .add_filter("Raw photos", &extensions)
            .pick_file()
        {
            self.open_photo(&photo);
        }
    }
}

/// A thumbnail or folder cover as a texture.
pub fn thumbnail_texture(
    ctx: &egui::Context,
    name: String,
    image: &photonation_engine::RenderedImage,
) -> egui::TextureHandle {
    let color_image = egui::ColorImage::from_rgb([image.width, image.height], &image.rgb);
    ctx.load_texture(name, color_image, TextureOptions::LINEAR)
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

#[cfg(target_os = "macos")]
impl PhotonationApp {
    fn perform_native_menu_choices(&mut self, ctx: &egui::Context) {
        let chosen = self
            .native_menus
            .as_ref()
            .map(crate::native_menu::NativeMenus::take_chosen)
            .unwrap_or_default();
        for action in chosen {
            self.perform(&action, ctx);
        }
    }

    fn update_native_menus(&mut self) {
        let menus = crate::menus::menus(self, true);
        if let Some(native) = &mut self.native_menus {
            native.update(menus);
        }
    }
}

impl eframe::App for PhotonationApp {
    /// See-through wherever nothing is painted, for the glass panes. A window
    /// without glass is opaque and ignores the alpha.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        theme::follow_mode(&ctx);
        self.view.inspector_rect = None;
        self.view.filmstrip_rect = None;
        self.handle_events(&ctx);
        self.apply_folder_request();
        self.arrange_photos();
        #[cfg(target_os = "macos")]
        self.perform_native_menu_choices(&ctx);
        crate::library::shortcuts(self, &ctx);
        self.shortcuts(&ctx);
        self.follow_full_screen(&ctx);
        let photo_only = self.view.full_screen.is_some();

        self.remember_photo_before_panels();
        // The panels set it again while the pointer stays on a preset.
        let preset_hover_before = self.view.presets.hover.take();
        let cropping_before = self.view.cropping;

        if !photo_only {
            panels::menu_bar::show(self, ui);
            panels::toolbar::show(self, ui);
        }
        if self.showing_folder_overview() {
            panels::folders::show(self, ui);
        } else if self.view.library.survey {
            panels::library_bar::show(self, ui);
            panels::survey::show(self, ui);
        } else if self.view.library.grid {
            panels::library_bar::show(self, ui);
            panels::grid::show(self, ui);
        } else {
            // The inspector takes the full height; the filmstrip runs beside it.
            if !photo_only {
                if self.current.is_some() {
                    panels::adjustments::show(self, ui);
                }
                if !self.photos.is_empty() {
                    panels::filmstrip::show(self, ui);
                }
            }
            panels::viewer::show(self, ui);
        }
        if self.view.library.confirm_trash {
            panels::library_bar::confirm_trash(self, &ctx);
        }
        self.view.selected_mask =
            MaskSelection::validated(self.view.selected_mask, &self.settings.masks);
        // Selecting anything else, or closing the panel, ends the wait for good,
        // so coming back to the shape later doesn't make a drag redraw it.
        if !self.view.show_masks || self.view.shape_to_draw != self.view.selected_mask {
            self.view.shape_to_draw = None;
        }
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
        if self.view.show_settings {
            panels::settings::show(self, &ctx);
        }
        if self.view.show_about {
            panels::settings::about(self, &ctx);
        }
        if self.view.help.open {
            panels::help::show(&mut self.view, &ctx);
        }
        panels::presets::dialogs(self, &ctx);
        if self.view.presets.hover != preset_hover_before {
            self.refresh();
        }

        self.notice_panel_edits();
        if self.view.cropping != cropping_before {
            self.refresh();
        }
        // One undo step per interaction: commit once the pointer is released.
        if !ctx.input(|i| i.pointer.any_down()) {
            self.history.commit(&self.settings);
        }
        // The folder overview, grid and survey views draw no preview.
        let overview = self.showing_folder_overview();
        let library_view = overview || self.view.library.grid || self.view.library.survey;
        let settled = (library_view || (!self.is_rendering() && self.preview.is_some()))
            && (!overview || self.folders.settled())
            && (!self.view.library.survey || self.survey_images.len() >= 2)
            && self
                .photos
                .iter()
                .all(|p| p.thumbnail.is_some() || p.unsupported);
        #[cfg(target_os = "macos")]
        self.update_native_menus();
        #[cfg(target_os = "macos")]
        if let Some(glass) = &mut self.glass {
            glass.place(self.view.inspector_rect, self.view.filmstrip_rect);
        }
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
    use photonation_develop::{lightroom_xmp_path, sidecar_path};
    use std::path::Path;

    const GPU_LIMIT: usize = 8192;

    /// One whole frame of the window, as eframe runs it.
    fn window_frame(app: &mut PhotonationApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            eframe::App::ui(app, ui, &mut eframe::Frame::_new_kittest());
        });
        output.textures_delta.clear();
    }

    /// Clicks the grid's second photo, well inside its cell.
    fn click_second_photo_in_grid(app: &mut PhotonationApp, ctx: &egui::Context) {
        let on_the_cell = egui::pos2(300.0, 200.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: on_the_cell,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        app.view.library.grid = true;
        window_frame(app, ctx, Vec::new());
        window_frame(app, ctx, vec![egui::Event::PointerMoved(on_the_cell)]);
        window_frame(app, ctx, vec![button(true)]);
        window_frame(app, ctx, vec![button(false)]);
        assert_eq!(app.current, Some(1), "the click lands on the second photo");
    }

    /// A frame after any save the last ones scheduled would have happened.
    fn window_frame_after_the_debounce(app: &mut PhotonationApp, ctx: &egui::Context) {
        std::thread::sleep(SIDECAR_DEBOUNCE + Duration::from_millis(50));
        window_frame(app, ctx, Vec::new());
    }

    fn write_lightroom_edits(photo: &Path) {
        std::fs::write(
            lightroom_xmp_path(photo),
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
               <rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
                crs:ProcessVersion="15.4" crs:Exposure2012="+0.70"/></rdf:RDF></x:xmpmeta>"#,
        )
        .expect("write xmp");
    }

    /// Pretends the current photo has loaded, so the viewer draws it.
    fn show_a_decoded_photo(app: &mut PhotonationApp, ctx: &egui::Context, path: PathBuf) {
        app.info = Some(PhotoInfo {
            path,
            camera: Default::default(),
            as_shot: (5000.0, 0.0),
            size: (6000, 4000),
        });
        let image = egui::ColorImage::new([60, 40], vec![egui::Color32::GRAY; 60 * 40]);
        app.preview = Some(Preview {
            texture: ctx.load_texture("test photo", image, TextureOptions::LINEAR),
            size: [60, 40],
        });
    }

    fn pointer_button(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    /// A press at `from`, a drag along `to`, and a release: frame by frame,
    /// as the window runs them.
    fn drag_on_the_photo(
        app: &mut PhotonationApp,
        ctx: &egui::Context,
        from: egui::Pos2,
        to: &[egui::Pos2],
        modifiers: egui::Modifiers,
    ) {
        window_frame(app, ctx, vec![egui::Event::ModifiersChanged(modifiers)]);
        window_frame(app, ctx, vec![egui::Event::PointerMoved(from)]);
        window_frame(app, ctx, vec![pointer_button(from, true, modifiers)]);
        for point in to {
            window_frame(app, ctx, vec![egui::Event::PointerMoved(*point)]);
        }
        let end = to.last().copied().unwrap_or(from);
        window_frame(app, ctx, vec![pointer_button(end, false, modifiers)]);
        let released = egui::Event::ModifiersChanged(egui::Modifiers::NONE);
        window_frame(app, ctx, vec![released]);
    }

    fn painted_strokes(app: &PhotonationApp) -> &[photonation_develop::MaskStroke] {
        match &app.settings.masks[0].components[0].shape {
            photonation_develop::MaskShape::Brush(brush) => &brush.strokes,
            other => panic!("not a brush: {other:?}"),
        }
    }

    #[test]
    fn painting_on_the_photo_makes_one_undoable_stroke_that_renders_as_it_grows() {
        use egui::{Modifiers, pos2};
        use photonation_develop::{Mask, MaskBrush, MaskShape};
        let folder = folder_of(&["a.CR3"]);
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);
        app.open_folder(folder.path().to_path_buf());
        app.current = Some(0);
        window_frame(&mut app, &ctx, Vec::new());
        show_a_decoded_photo(&mut app, &ctx, folder.path().join("a.CR3"));
        app.settings.masks = vec![Mask::new("Painted", MaskShape::Brush(MaskBrush::default()))];
        app.view.selected_mask = Some(MaskSelection::mask(0));
        app.view.show_masks = true;
        window_frame(&mut app, &ctx, Vec::new());
        app.history.commit(&app.settings);
        assert!(painted_strokes(&app).is_empty());

        let path = [
            pos2(300.0, 300.0),
            pos2(340.0, 300.0),
            pos2(380.0, 320.0),
            pos2(420.0, 320.0),
        ];
        let generation = app.generation;
        window_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(path[0])]);
        window_frame(
            &mut app,
            &ctx,
            vec![pointer_button(path[0], true, Modifiers::NONE)],
        );
        for point in &path[1..] {
            window_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(*point)]);
        }
        assert!(painted_strokes(&app).is_empty(), "not until the release");
        let growing = app.view.mask_stroke.as_ref().expect("painting");
        assert!(growing.path.points.len() >= 3, "{:?}", growing.path.points);
        assert!(
            app.generation > generation,
            "the stroke renders as it grows"
        );
        let rendered = app.preview_settings();
        match &rendered.masks[0].components[0].shape {
            MaskShape::Brush(brush) => assert_eq!(brush.strokes.len(), 1),
            other => panic!("not a brush: {other:?}"),
        }

        window_frame(
            &mut app,
            &ctx,
            vec![pointer_button(path[3], false, Modifiers::NONE)],
        );
        window_frame(&mut app, &ctx, Vec::new());
        assert!(app.view.mask_stroke.is_none());
        assert_eq!(painted_strokes(&app).len(), 1);
        assert!(!painted_strokes(&app)[0].erase);
        assert!(app.pending_save.is_some(), "the stroke is an edit to save");

        // With Alt the brush erases, and each stroke is its own undo step.
        drag_on_the_photo(
            &mut app,
            &ctx,
            pos2(300.0, 350.0),
            &[pos2(340.0, 350.0), pos2(380.0, 350.0)],
            Modifiers::ALT,
        );
        window_frame(&mut app, &ctx, Vec::new());
        assert_eq!(painted_strokes(&app).len(), 2);
        assert!(painted_strokes(&app)[1].erase);
        app.undo();
        assert_eq!(painted_strokes(&app).len(), 1, "undo takes back one stroke");
        app.undo();
        assert!(painted_strokes(&app).is_empty());

        // A click dabs, and the viewer doesn't take it for a zoom.
        let spot = pos2(350.0, 300.0);
        window_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(spot)]);
        window_frame(
            &mut app,
            &ctx,
            vec![pointer_button(spot, true, Modifiers::NONE)],
        );
        window_frame(
            &mut app,
            &ctx,
            vec![pointer_button(spot, false, Modifiers::NONE)],
        );
        window_frame(&mut app, &ctx, Vec::new());
        assert_eq!(painted_strokes(&app).len(), 1);
        assert_eq!(painted_strokes(&app)[0].path.points.len(), 1);
    }

    /// The centre of the topmost widget labelled `label`, from what a frame
    /// tells assistive technology.
    fn widget_labelled(app: &mut PhotonationApp, ctx: &egui::Context, label: &str) -> egui::Pos2 {
        ctx.enable_accesskit();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            eframe::App::ui(app, ui, &mut eframe::Frame::_new_kittest());
        });
        output.textures_delta.clear();
        let update = output
            .platform_output
            .accesskit_update
            .expect("accesskit is on");
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some(label))
            .filter_map(|(_, node)| node.bounds())
            .min_by(|a, b| a.y0.total_cmp(&b.y0))
            .map(|bounds| {
                egui::pos2(
                    ((bounds.x0 + bounds.x1) / 2.0) as f32,
                    ((bounds.y0 + bounds.y1) / 2.0) as f32,
                )
            })
            .unwrap_or_else(|| panic!("no widget labelled {label}"))
    }

    fn click(app: &mut PhotonationApp, ctx: &egui::Context, at: egui::Pos2) {
        use egui::Modifiers;
        window_frame(app, ctx, vec![egui::Event::PointerMoved(at)]);
        window_frame(app, ctx, vec![pointer_button(at, true, Modifiers::NONE)]);
        window_frame(app, ctx, vec![pointer_button(at, false, Modifiers::NONE)]);
        window_frame(app, ctx, Vec::new());
    }

    /// Where the viewer draws the photo: two dabs reveal the mapping from
    /// screen to photo positions, then undo removes them.
    fn photo_rect_from_dabs(app: &mut PhotonationApp, ctx: &egui::Context) -> egui::Rect {
        use egui::pos2;
        let spots = [pos2(200.0, 200.0), pos2(400.0, 400.0)];
        for spot in spots {
            click(app, ctx, spot);
            app.history.commit(&app.settings);
        }
        let strokes = painted_strokes(app);
        let (a, b) = (strokes[0].path.points[0], strokes[1].path.points[0]);
        let width = (spots[1].x - spots[0].x) as f64 / (b[0] - a[0]);
        let height = (spots[1].y - spots[0].y) as f64 / (b[1] - a[1]);
        let min = pos2(
            (spots[0].x as f64 - a[0] * width) as f32,
            (spots[0].y as f64 - a[1] * height) as f32,
        );
        app.undo();
        app.undo();
        egui::Rect::from_min_size(min, egui::vec2(width as f32, height as f32))
    }

    #[test]
    fn a_drag_on_the_compare_divider_moves_only_the_divider() {
        use egui::{Modifiers, pos2};
        use photonation_develop::{MaskBrush, MaskShape};
        let folder = folder_of(&["a.CR3"]);
        let ctx = egui::Context::default();
        let brush = MaskShape::Brush(MaskBrush::default());
        let mut app = app_with_mask(&ctx, &folder, brush);
        let rect = photo_rect_from_dabs(&mut app, &ctx);
        app.view.compare = true;
        let image = egui::ColorImage::new([60, 40], vec![egui::Color32::BLACK; 60 * 40]);
        app.before = Some(Preview {
            texture: ctx.load_texture("test original", image, TextureOptions::LINEAR),
            size: [60, 40],
        });
        let knob = rect.center();
        let moved_to = knob + egui::vec2(60.0, 0.0);
        drag_on_the_photo(
            &mut app,
            &ctx,
            knob,
            &[knob + egui::vec2(30.0, 0.0), moved_to],
            Modifiers::NONE,
        );
        let expected = (moved_to.x - rect.left()) / rect.width();
        assert!(
            (app.view.compare_split - expected).abs() < 1e-3,
            "{} vs {expected}",
            app.view.compare_split
        );
        assert!(painted_strokes(&app).is_empty(), "the brush stayed out");

        // On the line but away from its knob, the brush paints across it.
        let split = app.view.compare_split;
        let on_the_line = pos2(moved_to.x, rect.top() + 40.0);
        drag_on_the_photo(
            &mut app,
            &ctx,
            on_the_line,
            &[
                on_the_line + egui::vec2(30.0, 0.0),
                on_the_line + egui::vec2(60.0, 0.0),
            ],
            Modifiers::NONE,
        );
        assert_eq!(painted_strokes(&app).len(), 1);
        assert_eq!(app.view.compare_split, split, "the divider stayed put");
    }

    #[test]
    fn without_a_tool_the_whole_compare_line_moves_the_divider_and_nothing_pans() {
        use egui::{Modifiers, pos2};
        use photonation_develop::{MaskBrush, MaskShape};
        let folder = folder_of(&["a.CR3"]);
        let ctx = egui::Context::default();
        let brush = MaskShape::Brush(MaskBrush::default());
        let mut app = app_with_mask(&ctx, &folder, brush);
        let rect = photo_rect_from_dabs(&mut app, &ctx);
        app.view.show_masks = false;
        app.view.compare = true;
        let image = egui::ColorImage::new([60, 40], vec![egui::Color32::BLACK; 60 * 40]);
        app.before = Some(Preview {
            texture: ctx.load_texture("test original", image, TextureOptions::LINEAR),
            size: [60, 40],
        });
        let on_the_line = pos2(rect.center().x, rect.top() + 40.0);
        drag_on_the_photo(
            &mut app,
            &ctx,
            on_the_line,
            &[
                on_the_line + egui::vec2(30.0, 0.0),
                on_the_line + egui::vec2(60.0, 0.0),
            ],
            Modifiers::NONE,
        );
        assert!(app.view.compare_split > 0.5);
        assert_eq!(app.view.pan, egui::Vec2::ZERO);
    }

    #[test]
    fn adding_a_gradient_from_the_panel_lets_a_drag_place_it() {
        use egui::{Modifiers, pos2};
        use photonation_develop::{MaskShape, RadialGradient};
        let folder = folder_of(&["a.CR3"]);
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);
        app.open_folder(folder.path().to_path_buf());
        app.current = Some(0);
        window_frame(&mut app, &ctx, Vec::new());
        show_a_decoded_photo(&mut app, &ctx, folder.path().join("a.CR3"));
        app.view.show_masks = true;
        window_frame(&mut app, &ctx, Vec::new());

        let button = widget_labelled(&mut app, &ctx, "+ Radial");
        click(&mut app, &ctx, button);
        assert_eq!(app.settings.masks.len(), 1, "the button adds a mask");
        assert_eq!(app.view.shape_to_draw, Some(MaskSelection::mask(0)));
        let MaskShape::Radial(radial) = app.settings.masks[0].components[0].shape else {
            panic!("radial")
        };
        assert_eq!(radial, RadialGradient::default(), "ready to use as it is");

        drag_on_the_photo(
            &mut app,
            &ctx,
            pos2(150.0, 200.0),
            &[pos2(190.0, 200.0), pos2(230.0, 200.0)],
            Modifiers::NONE,
        );
        let MaskShape::Radial(placed) = app.settings.masks[0].components[0].shape else {
            panic!("radial")
        };
        assert_ne!(placed.center, radial.center);
        assert_eq!(placed.radius[0], placed.radius[1]);
        assert_eq!(app.view.shape_to_draw, None);

        // A brush paints, so nothing waits to be placed (the topmost button
        // adds a mask; the one in the Shapes card, a shape).
        let button = widget_labelled(&mut app, &ctx, "+ Brush");
        click(&mut app, &ctx, button);
        assert_eq!(app.settings.masks.len(), 2);
        assert_eq!(app.view.selected_mask, Some(MaskSelection::mask(1)));
        assert_eq!(app.view.shape_to_draw, None);
    }

    /// An app showing a photo, with the masks panel open on `shape`.
    fn app_with_mask(
        ctx: &egui::Context,
        folder: &tempfile::TempDir,
        shape: photonation_develop::MaskShape,
    ) -> PhotonationApp {
        let mut app = PhotonationApp::empty(ctx);
        app.open_folder(folder.path().to_path_buf());
        app.current = Some(0);
        window_frame(&mut app, ctx, Vec::new());
        show_a_decoded_photo(&mut app, ctx, folder.path().join("a.CR3"));
        app.settings.masks = vec![photonation_develop::Mask::new("Mask", shape)];
        app.view.selected_mask = Some(MaskSelection::mask(0));
        app.view.show_masks = true;
        window_frame(&mut app, ctx, Vec::new());
        app.history.commit(&app.settings);
        app
    }

    #[test]
    fn a_new_gradient_is_placed_by_the_next_drag_and_only_that_one() {
        use egui::{Modifiers, pos2};
        use photonation_develop::{LinearGradient, MaskShape};
        let folder = folder_of(&["a.CR3"]);
        let ctx = egui::Context::default();
        let original = LinearGradient::default();
        let mut app = app_with_mask(&ctx, &folder, MaskShape::Linear(original));
        let linear = |app: &PhotonationApp| match &app.settings.masks[0].components[0].shape {
            MaskShape::Linear(linear) => *linear,
            other => panic!("not linear: {other:?}"),
        };

        // Nothing waits: a drag off the handles leaves the shape alone.
        let path = [pos2(250.0, 300.0), pos2(250.0, 340.0), pos2(250.0, 400.0)];
        drag_on_the_photo(&mut app, &ctx, path[0], &path[1..], Modifiers::NONE);
        assert_eq!(linear(&app), original, "no shape was waiting");

        app.view.shape_to_draw = app.view.selected_mask;
        drag_on_the_photo(&mut app, &ctx, path[0], &path[1..], Modifiers::NONE);
        window_frame(&mut app, &ctx, Vec::new());
        let placed = linear(&app);
        assert_ne!(placed, original);
        assert!(placed.end[1] > placed.start[1], "from the press downward");
        assert_eq!(app.view.shape_to_draw, None, "one drag places it");

        // The next drag has nothing waiting either, and one undo step goes.
        drag_on_the_photo(
            &mut app,
            &ctx,
            pos2(260.0, 200.0),
            &[pos2(300.0, 200.0), pos2(340.0, 200.0)],
            Modifiers::NONE,
        );
        assert_eq!(linear(&app), placed);
        app.undo();
        assert_eq!(linear(&app), original);
    }

    #[test]
    fn a_shape_stops_waiting_to_be_drawn_once_another_is_selected() {
        use egui::{Modifiers, pos2};
        use photonation_develop::{CombineMode, LinearGradient, MaskComponent, MaskShape};
        let folder = folder_of(&["a.CR3"]);
        let ctx = egui::Context::default();
        let original = LinearGradient::default();
        let mut app = app_with_mask(&ctx, &folder, MaskShape::Linear(original));
        app.settings.masks[0].components.push(MaskComponent::new(
            MaskShape::Linear(original),
            CombineMode::Add,
        ));
        app.view.shape_to_draw = Some(MaskSelection {
            mask: 0,
            component: 1,
        });
        drag_on_the_photo(
            &mut app,
            &ctx,
            pos2(250.0, 300.0),
            &[pos2(250.0, 340.0), pos2(250.0, 400.0)],
            Modifiers::NONE,
        );
        let unchanged = |index: usize| match &app.settings.masks[0].components[index].shape {
            MaskShape::Linear(linear) => *linear == original,
            other => panic!("not linear: {other:?}"),
        };
        assert!(unchanged(0) && unchanged(1), "shape 0 is selected, not 1");
        assert_eq!(app.view.shape_to_draw, None, "and coming back won't redraw");
    }

    #[test]
    fn brackets_size_the_mask_brush_instead_of_rating_the_photo() {
        use photonation_develop::{Mask, MaskBrush, MaskShape};
        let folder = folder_of(&["a.CR3"]);
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);
        app.open_folder(folder.path().to_path_buf());
        app.current = Some(0);
        window_frame(&mut app, &ctx, Vec::new());
        app.settings.masks = vec![Mask::new("Painted", MaskShape::Brush(MaskBrush::default()))];
        app.view.selected_mask = Some(MaskSelection::mask(0));
        app.view.show_masks = true;
        let size = app.view.mask_brush.size_percent;
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        window_frame(&mut app, &ctx, vec![key(egui::Key::CloseBracket)]);
        assert!(app.view.mask_brush.size_percent > size);
        window_frame(&mut app, &ctx, vec![key(egui::Key::OpenBracket)]);
        assert!((app.view.mask_brush.size_percent - size).abs() < 1e-4);
    }

    /// One frame; returns what it told the window to do.
    fn window_commands(
        app: &mut PhotonationApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Vec<egui::ViewportCommand> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            eframe::App::ui(app, ui, &mut eframe::Frame::_new_kittest());
        });
        output.textures_delta.clear();
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .clone()
    }

    fn pressed(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn app_showing_a_photo(ctx: &egui::Context, folder: &tempfile::TempDir) -> PhotonationApp {
        let mut app = PhotonationApp::empty(ctx);
        app.open_folder(folder.path().to_path_buf());
        app.current = Some(0);
        window_frame(&mut app, ctx, Vec::new());
        show_a_decoded_photo(&mut app, ctx, folder.path().join("a.CR3"));
        window_frame(&mut app, ctx, Vec::new());
        app
    }

    #[test]
    fn f_and_esc_enter_and_leave_full_screen_with_the_panels_hidden() {
        use egui::ViewportCommand::Fullscreen;
        let folder = folder_of(&["a.CR3"]);
        let ctx = egui::Context::default();
        let mut app = app_showing_a_photo(&ctx, &folder);
        app.view.show_masks = true;
        window_frame(&mut app, &ctx, Vec::new());
        assert!(app.view.inspector_rect.is_some(), "the panels are there");

        let commands = window_commands(&mut app, &ctx, vec![pressed(egui::Key::F)]);
        assert!(commands.contains(&Fullscreen(true)), "{commands:?}");
        assert!(app.view.full_screen.is_some());
        assert!(!app.view.show_masks, "the tools need their panels");
        assert_eq!(app.view.inspector_rect, None, "no inspector");
        assert_eq!(app.view.filmstrip_rect, None, "no filmstrip");
        assert!(app.status.is_some(), "it says how to get out");

        let commands = window_commands(&mut app, &ctx, vec![pressed(egui::Key::Escape)]);
        assert!(commands.contains(&Fullscreen(false)), "{commands:?}");
        assert_eq!(app.view.full_screen, None);
        window_frame(&mut app, &ctx, Vec::new());
        assert!(app.view.inspector_rect.is_some(), "the panels are back");

        window_frame(&mut app, &ctx, vec![pressed(egui::Key::F)]);
        let commands = window_commands(&mut app, &ctx, vec![pressed(egui::Key::F)]);
        assert!(commands.contains(&Fullscreen(false)), "F leaves too");
    }

    #[test]
    fn a_tool_or_the_grid_ends_full_screen_and_shows_its_panels() {
        let folder = folder_of(&["a.CR3"]);
        let ctx = egui::Context::default();
        let mut app = app_showing_a_photo(&ctx, &folder);
        for (key, opened) in [
            (
                egui::Key::M,
                (&|view: &ViewState| view.show_masks) as &dyn Fn(&ViewState) -> bool,
            ),
            (egui::Key::R, &|view: &ViewState| view.cropping),
            (egui::Key::Q, &|view: &ViewState| view.show_removals),
            (egui::Key::G, &|view: &ViewState| view.library.grid),
        ] {
            window_frame(&mut app, &ctx, vec![pressed(egui::Key::F)]);
            assert!(app.view.full_screen.is_some(), "{key:?}");
            let commands = window_commands(&mut app, &ctx, vec![pressed(key)]);
            assert!(opened(&app.view), "{key:?} opens");
            assert_eq!(app.view.full_screen, None, "{key:?} leaves");
            assert!(
                commands.contains(&egui::ViewportCommand::Fullscreen(false)),
                "{key:?}: {commands:?}"
            );
            app.view.close_tools();
            window_frame(&mut app, &ctx, Vec::new());
        }
    }

    #[test]
    fn full_screen_ends_with_the_systems_and_keeps_a_window_already_full() {
        let folder = folder_of(&["a.CR3"]);
        let ctx = egui::Context::default();
        let mut app = app_showing_a_photo(&ctx, &folder);
        let frame_with_window = |app: &mut PhotonationApp, ctx: &egui::Context, full, events| {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                events,
                ..Default::default()
            };
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .fullscreen = Some(full);
            let mut output = ctx.run_ui(input, |ui| {
                eframe::App::ui(app, ui, &mut eframe::Frame::_new_kittest());
            });
            output.textures_delta.clear();
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .clone()
        };
        // Still on its way there: not yet a sign the user left.
        frame_with_window(&mut app, &ctx, false, vec![pressed(egui::Key::F)]);
        frame_with_window(&mut app, &ctx, false, Vec::new());
        assert!(app.view.full_screen.is_some(), "while the window animates");
        frame_with_window(&mut app, &ctx, true, Vec::new());
        // The system took the window out (the green button): nothing to undo.
        let commands = frame_with_window(&mut app, &ctx, false, Vec::new());
        assert_eq!(app.view.full_screen, None);
        assert!(
            !commands.contains(&egui::ViewportCommand::Fullscreen(false)),
            "{commands:?}"
        );

        // Full screen by the system first: leaving keeps the window so.
        frame_with_window(&mut app, &ctx, true, vec![pressed(egui::Key::F)]);
        assert!(app.view.full_screen.is_some());
        let commands = frame_with_window(&mut app, &ctx, true, vec![pressed(egui::Key::Escape)]);
        assert_eq!(app.view.full_screen, None);
        assert!(
            !commands.contains(&egui::ViewportCommand::Fullscreen(false)),
            "{commands:?}"
        );
    }

    #[test]
    fn full_screen_needs_a_photo_and_esc_goes_to_a_sheet_first() {
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);
        window_frame(&mut app, &ctx, vec![pressed(egui::Key::F)]);
        assert_eq!(app.view.full_screen, None, "nothing to show");

        let folder = folder_of(&["a.CR3"]);
        let mut app = app_showing_a_photo(&ctx, &folder);
        window_frame(&mut app, &ctx, vec![pressed(egui::Key::F)]);
        app.view.show_export = true;
        window_frame(&mut app, &ctx, vec![pressed(egui::Key::Escape)]);
        assert!(app.view.full_screen.is_some(), "the sheet was in front");
    }

    /// A folder with these (unreadable) raw files.
    fn folder_of(names: &[&str]) -> tempfile::TempDir {
        let folder = tempfile::tempdir().expect("temporary folder");
        for name in names {
            std::fs::write(folder.path().join(name), b"not a raw").expect("write photo");
        }
        folder
    }

    #[test]
    fn opening_a_photo_opens_its_folder_on_that_photo() {
        let folder = folder_of(&["a.CR3", "b.CR3", "c.CR3"]);
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);

        app.open_photo(&folder.path().join("b.CR3"));

        assert_eq!(app.photos.len(), 3, "the rest of its folder is there too");
        let current = app.current.expect("a photo is open");
        assert_eq!(app.photos[current].path, folder.path().join("b.CR3"));
    }

    #[test]
    fn opening_a_file_that_is_not_a_photo_still_opens_its_folder() {
        let folder = folder_of(&["a.CR3", "notes.txt"]);
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);

        app.open_photo(&folder.path().join("notes.txt"));

        assert_eq!(app.photos.len(), 1);
        assert_eq!(app.current, Some(0));
    }

    #[test]
    fn viewing_a_photo_with_lightroom_edits_leaves_no_sidecar() {
        let folder = folder_of(&["a.CR3", "b.CR3"]);
        let viewed = folder.path().join("b.CR3");
        write_lightroom_edits(&viewed);
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);
        app.open_folder(folder.path().to_path_buf());

        click_second_photo_in_grid(&mut app, &ctx);
        let generation_when_opened = app.generation;
        assert_ne!(app.settings, Settings::default(), "Lightroom's edits show");
        assert!(app.pending_save.is_none(), "viewing is not an edit");

        window_frame_after_the_debounce(&mut app, &ctx);
        assert!(!sidecar_path(&viewed).exists());
        assert_eq!(app.generation, generation_when_opened, "no second render");
    }

    #[test]
    fn viewing_a_photo_whose_sidecar_cannot_be_read_leaves_it_alone() {
        let folder = folder_of(&["a.CR3", "b.CR3"]);
        // Settings to leave behind, so the unreadable photo's defaults differ.
        write_lightroom_edits(&folder.path().join("a.CR3"));
        let sidecar = sidecar_path(&folder.path().join("b.CR3"));
        std::fs::write(&sidecar, "not a sidecar").expect("write sidecar");
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);
        app.open_folder(folder.path().to_path_buf());

        click_second_photo_in_grid(&mut app, &ctx);
        window_frame_after_the_debounce(&mut app, &ctx);
        assert_eq!(
            std::fs::read_to_string(&sidecar).expect("the sidecar is still there"),
            "not a sidecar"
        );
    }

    #[test]
    fn panels_switched_off_on_the_last_photo_are_not_an_edit_of_the_next() {
        let folder = folder_of(&["a.CR3", "b.CR3"]);
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);
        app.open_folder(folder.path().to_path_buf());
        app.view.disabled.insert(AdjustmentGroup::Light);

        click_second_photo_in_grid(&mut app, &ctx);
        assert!(app.view.disabled.is_empty());
        assert!(app.pending_save.is_none());
    }

    #[test]
    fn opening_a_folder_while_the_panels_draw_is_not_an_edit() {
        let first = folder_of(&["a.CR3"]);
        let second = folder_of(&["b.CR3"]);
        write_lightroom_edits(&second.path().join("b.CR3"));
        let mut app = PhotonationApp::empty(&egui::Context::default());
        app.open_folder(first.path().to_path_buf());

        app.remember_photo_before_panels();
        app.open_folder(second.path().to_path_buf());
        app.notice_panel_edits();
        assert_ne!(app.settings, Settings::default());
        assert!(app.pending_save.is_none());
    }

    #[test]
    fn closing_the_photo_while_the_panels_draw_is_not_an_edit() {
        let folder = folder_of(&["a.CR3"]);
        write_lightroom_edits(&folder.path().join("a.CR3"));
        let mut app = PhotonationApp::empty(&egui::Context::default());
        app.open_folder(folder.path().to_path_buf());

        app.remember_photo_before_panels();
        app.close_photo();
        app.notice_panel_edits();
        assert!(app.pending_save.is_none());
    }

    #[test]
    fn an_edit_is_still_noticed_on_a_photo_opened_while_the_panels_draw() {
        let folder = folder_of(&["a.CR3", "b.CR3"]);
        let mut app = PhotonationApp::empty(&egui::Context::default());
        app.open_folder(folder.path().to_path_buf());

        app.remember_photo_before_panels();
        app.select(1);
        app.settings.exposure = 1.0;
        app.notice_panel_edits();
        assert!(app.pending_save.is_some());
    }

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
