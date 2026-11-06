// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::app::{PhotonationApp, ViewState, Zoom};
use crate::panels::{hover_tip, icon_button, tool_button};
use crate::theme;
use crate::widgets::{Icon, rating_control, rating_control_width};
use eframe::egui::{self, Align, Layout, RichText};

/// Room for a camera's file name, such as IMG_2200.CR3. Below this the colour
/// labels make way; they are still in the Library menu.
const MIN_NAME_WIDTH: f32 = 96.0;
/// Below this the camera summary would be little more than an ellipsis.
const MIN_SUMMARY_WIDTH: f32 = 80.0;
const DETAILS_GAP: f32 = 6.0;

/// A tool with its own sidebar: icon, whether it is open, tooltip, toggle.
type SidebarTool = (Icon, bool, &'static str, fn(&mut ViewState));

pub fn show(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    egui::Panel::top("toolbar")
        .exact_size(theme::TOOLBAR_HEIGHT)
        .show_separator_line(false)
        .frame(
            egui::Frame::NONE
                .fill(theme::window_fill())
                .inner_margin(egui::Margin::symmetric(12, 8)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                // The right-hand controls go first so the photo details on the left get
                // only the space they leave, and truncate instead of running under them.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let has_photo = app.current.is_some();
                    let export = ui.add_enabled(
                        has_photo,
                        theme::primary_button("Export").min_size(egui::vec2(76.0, 26.0)),
                    );
                    let export_tip = format!(
                        "Export ({})\nSaves the edited photo as JPEG, PNG or TIFF, at full or \
                         reduced size, in the colour space and with the metadata you choose.",
                        theme::SHORTCUT_EXPORT
                    );
                    if hover_tip(export, &export_tip).clicked() {
                        app.view.show_export = true;
                    }
                    ui.add_space(4.0);
                    crate::panels::help::button(ui, &mut app.view);
                    ui.add_space(4.0);
                    let is_actual =
                        matches!(app.view.zoom, Zoom::Scale(s) if (s - 1.0).abs() < 1e-3);
                    let actual_tip = "Actual pixels\nOne photo pixel per screen pixel, to judge \
                                      focus, noise and sharpening.";
                    if tool_button(ui, "1:1", is_actual, actual_tip).clicked() {
                        app.view.zoom = if is_actual {
                            Zoom::Fit
                        } else {
                            Zoom::Scale(1.0)
                        };
                        app.view.pan = egui::Vec2::ZERO;
                    }
                    let fit_tooltip = format!(
                        "Fit to window ({})\nShows the whole photo. Pinch or {}-scroll zooms \
                         around the pointer; scrolling pans a zoomed photo.",
                        theme::SHORTCUT_FIT,
                        theme::COMMAND_KEY
                    );
                    if tool_button(ui, "Fit", app.view.zoom == Zoom::Fit, &fit_tooltip).clicked() {
                        app.view.zoom = Zoom::Fit;
                        app.view.pan = egui::Vec2::ZERO;
                    }
                    ui.add_space(8.0);
                    let compare = icon_button(
                        ui,
                        Icon::Compare,
                        None,
                        app.view.compare,
                        "Compare with the original (\\)\nSplits the photo: the unedited \
                         original left of the handle, the edit right of it. Drag the handle.",
                    );
                    if compare.clicked() {
                        app.view.compare = !app.view.compare;
                    }
                    let tools: [SidebarTool; 3] = [
                        (
                            Icon::Layers,
                            app.view.show_masks,
                            "Masks (M)\nLocal adjustments inside linear and radial gradients, \
                             combined into masks and groups.",
                            ViewState::toggle_masks,
                        ),
                        (
                            Icon::Crop,
                            app.view.cropping,
                            "Crop and straighten (R)\nDrag the frame or its edges to crop, \
                             outside it to straighten. Return or Esc closes the tool.",
                            ViewState::toggle_cropping,
                        ),
                        (
                            Icon::Retouch,
                            app.view.show_removals,
                            "Remove objects (Q)\nPaint over something to remove it; \
                             generative AI or content-aware fill replaces it.",
                            ViewState::toggle_removals,
                        ),
                    ];
                    for (icon, active, tooltip, toggle) in tools {
                        if ui
                            .add_enabled_ui(has_photo, |ui| {
                                icon_button(ui, icon, None, active, tooltip)
                            })
                            .inner
                            .clicked()
                        {
                            toggle(&mut app.view);
                        }
                    }
                    ui.add_space(8.0);
                    let redo_tooltip = format!(
                        "Redo ({})\nBrings back the last edit you undid.",
                        theme::SHORTCUT_REDO
                    );
                    if ui
                        .add_enabled_ui(app.history.can_redo(), |ui| {
                            icon_button(ui, Icon::Redo, None, false, &redo_tooltip)
                        })
                        .inner
                        .clicked()
                    {
                        app.redo();
                    }
                    let undo_tooltip = format!(
                        "Undo ({})\nSteps back one edit. A whole slider drag is one step.",
                        theme::SHORTCUT_UNDO
                    );
                    if ui
                        .add_enabled_ui(app.history.can_undo(), |ui| {
                            icon_button(ui, Icon::Undo, None, false, &undo_tooltip)
                        })
                        .inner
                        .clicked()
                    {
                        app.undo();
                    }
                    if let Some((done, total)) = app.export_progress {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(format!("Exporting {}/{}", done + 1, total))
                                .size(12.0)
                                .color(theme::palette().text_secondary),
                        );
                        ui.add(egui::Spinner::new().size(14.0));
                    }
                    ui.add_space(12.0);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        photo_details(app, ui);
                    });
                });
            });
        });
}

/// Open, the grid, and the current photo's name, rating and camera settings,
/// within the space `ui` has. As it narrows, the camera settings go first, then
/// the file name shortens, then the colour labels go.
fn photo_details(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    // Too narrow even for that (a filter and an export in progress in the
    // smallest window), the rest is cut off: drawn over the controls on the
    // right, it would also take their clicks.
    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
    if icon_button(
        ui,
        Icon::Library,
        Some("Open"),
        false,
        &format!(
            "Open a folder ({})\nShows its raw photos in the filmstrip, or the folders \
             inside it that have some. Edits are saved beside each photo; the raw file is \
             never changed.",
            theme::SHORTCUT_OPEN
        ),
    )
    .clicked()
    {
        app.pick_folder();
    }
    library_buttons(app, ui);
    let name_gap = ui.spacing().item_spacing.x + DETAILS_GAP;
    // The photo's name and stars come first; the path has what they leave.
    let photo_room = app.current.map_or(0.0, |_| {
        MIN_NAME_WIDTH + name_gap + rating_control_width(ui, false)
    });
    crate::panels::folders::breadcrumb(app, ui, ui.available_width() - photo_room);
    ui.add_space(12.0);
    let Some(index) = app.current else {
        return;
    };
    let available = ui.available_width();
    let with_labels = available >= MIN_NAME_WIDTH + name_gap + rating_control_width(ui, true);
    let name_width = available - name_gap - rating_control_width(ui, with_labels);
    ui.allocate_ui(
        egui::vec2(name_width.max(0.0), ui.available_height()),
        |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(&app.photos[index].name)
                        .size(13.5)
                        .color(theme::palette().text),
                )
                .truncate(),
            );
        },
    );
    ui.add_space(DETAILS_GAP);
    if let Some(change) = rating_control(ui, app.photos[index].library, with_labels) {
        app.change_library(change);
    }
    let summary_fits = ui.available_width() >= DETAILS_GAP + MIN_SUMMARY_WIDTH;
    if let Some(info) = app.info.as_ref().filter(|_| summary_fits) {
        ui.add_space(DETAILS_GAP);
        ui.add(
            egui::Label::new(
                RichText::new(camera_summary(info))
                    .size(12.0)
                    .color(theme::palette().text_secondary),
            )
            .truncate(),
        );
    }
}

/// The grid and, while a filter hides photos, a reminder of it.
fn library_buttons(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    if app.photos.is_empty() || app.showing_folder_overview() {
        return;
    }
    let library = &app.view.library;
    let grid_tip = "Grid (G)\nAll the folder's photos at once, to rate, flag, label and compare \
                    them. E or a double-click opens one in the editor.";
    if icon_button(
        ui,
        Icon::Grid,
        None,
        library.grid || library.survey,
        grid_tip,
    )
    .clicked()
    {
        app.toggle_grid();
    }
    let library = &app.view.library;
    if library.filter.is_active() && !library.grid && !library.survey {
        let text = format!("{} of {}", library.shown.len(), app.photos.len());
        let tip = "Filtered\nThe filmstrip and ← → go through the photos the filter shows. \
                   Click to change the filter in the grid.";
        if tool_button(ui, &text, true, tip).clicked() {
            app.open_grid();
        }
    }
}

fn camera_summary(info: &photonation_engine::PhotoInfo) -> String {
    let c = &info.camera;
    let mut parts = vec![format!("{} {}", c.make, c.model)];
    if let Some(t) = c.exposure_time {
        parts.push(if t < 1.0 {
            format!("1/{:.0}s", 1.0 / t)
        } else {
            format!("{t:.1}s")
        });
    }
    if let Some(f) = c.f_number {
        parts.push(format!("ƒ/{f:.1}"));
    }
    if let Some(iso) = c.iso {
        parts.push(format!("ISO {iso}"));
    }
    if let Some(focal) = c.focal_length {
        parts.push(format!("{focal:.0} mm"));
    }
    parts.join("  ·  ")
}

/// The details drawn headless, in a space as wide as the toolbar would leave.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Photo;
    use eframe::egui::{Event, PointerButton, Pos2, RawInput, Rect, UiBuilder, pos2};

    const HEIGHT: f32 = 40.0;
    /// Past Open and the grid button, which would open a folder dialog or the grid.
    const PAST_THE_BUTTONS: f32 = 150.0;
    /// The narrowest space that holds the buttons, the start of a name, the
    /// stars and flags. The toolbar leaves more than this in the smallest window.
    const NARROWEST_FIT: usize = 350;
    /// How near a widget the pointer counts as on it (egui's `interact_radius`).
    const POINTER_TOLERANCE: usize = 5;
    const LONG_NAME: &str = "A very long file name that could never fit in the toolbar.CR3";

    struct Details {
        ctx: egui::Context,
        app: PhotonationApp,
        time: f64,
        _folder: tempfile::TempDir,
    }

    impl Details {
        fn with_photo_named(name: &str) -> Self {
            let folder = tempfile::tempdir().expect("temp folder");
            let ctx = egui::Context::default();
            let mut app = PhotonationApp::empty(&ctx);
            app.photos = vec![Photo::new(folder.path().join(name))];
            app.current = Some(0);
            Details {
                ctx,
                app,
                time: 0.0,
                _folder: folder,
            }
        }

        /// One frame with the details `width` wide; returns the space they used.
        fn frame(&mut self, width: f32, events: Vec<Event>) -> Rect {
            const DT: f32 = 1.0 / 60.0;
            self.time += f64::from(DT);
            let input = RawInput {
                screen_rect: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(1600.0, 800.0))),
                time: Some(self.time),
                predicted_dt: DT,
                events,
                ..Default::default()
            };
            let (app, mut used) = (&mut self.app, Rect::NOTHING);
            let mut output = self.ctx.run_ui(input, |ui| {
                let space = Rect::from_min_max(pos2(0.0, 0.0), pos2(width, HEIGHT));
                let builder = UiBuilder::new()
                    .max_rect(space)
                    .layout(Layout::left_to_right(Align::Center));
                used = ui
                    .scope_builder(builder, |ui| photo_details(app, ui))
                    .response
                    .rect;
            });
            output.textures_delta.clear();
            used
        }

        /// Clicks at `x` and says whether that changed the photo's rating, flag or label.
        fn click_changes_the_photo(&mut self, width: f32, x: f32) -> bool {
            let before = self.app.photos[0].library;
            let pos = pos2(x, HEIGHT / 2.0);
            let button = |pos: Pos2, pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(width, vec![Event::PointerMoved(pos)]);
            self.frame(width, vec![button(pos, true)]);
            self.frame(width, vec![button(pos, false)]);
            self.frame(width, Vec::new());
            self.app.photos[0].library != before
        }
    }

    #[test]
    fn the_rating_control_is_as_wide_as_it_says() {
        for with_labels in [false, true] {
            let ctx = egui::Context::default();
            let (mut expected, mut measured) = (0.0, 0.0);
            let info = photonation_develop::LibraryInfo::default();
            let mut output = ctx.run_ui(RawInput::default(), |ui| {
                expected = rating_control_width(ui, with_labels);
                measured = ui
                    .horizontal(|ui| rating_control(ui, info, with_labels))
                    .response
                    .rect
                    .width();
            });
            output.textures_delta.clear();
            assert!(
                (expected - measured).abs() < 0.5,
                "{expected} and {measured}"
            );
        }
    }

    #[test]
    fn details_stay_in_their_space_however_long_the_name() {
        for name in ["IMG_2200.CR3", LONG_NAME] {
            let mut details = Details::with_photo_named(name);
            for width in (NARROWEST_FIT..=1200).step_by(10) {
                let width = width as f32;
                let used = details.frame(width, Vec::new());
                assert!(
                    used.right() <= width + 0.5,
                    "{name}: {} in {width}",
                    used.right()
                );
            }
        }
    }

    #[test]
    fn details_stay_in_their_space_with_a_long_path_to_the_folder() {
        let mut details = Details::with_photo_named("IMG_2200.CR3");
        let root = details._folder.path().to_path_buf();
        let folder = root.join("A folder with quite a long name indeed/2026/06");
        details.app.folders.folder_opened(&root);
        details.app.folders.folder_opened(&folder);
        details.app.folder = Some(folder);
        for width in (NARROWEST_FIT..=1200).step_by(10) {
            let width = width as f32;
            let used = details.frame(width, Vec::new());
            assert!(used.right() <= width + 0.5, "{} in {width}", used.right());
        }
    }

    #[test]
    fn labels_show_only_where_they_fit_beside_the_name() {
        let mut details = Details::with_photo_named("IMG_2200.CR3");
        let without_labels = details.frame(450.0, Vec::new()).width();
        let with_labels = details.frame(700.0, Vec::new()).width();
        assert!(
            with_labels - without_labels > 100.0,
            "{without_labels} then {with_labels}"
        );
    }

    #[test]
    fn what_does_not_fit_takes_no_clicks() {
        // The space ends among the stars; the rest falls outside it.
        let width = 200.0;
        let mut details = Details::with_photo_named("IMG_2200.CR3");
        assert!(
            (PAST_THE_BUTTONS as usize..width as usize)
                .step_by(4)
                .any(|x| details.click_changes_the_photo(width, x as f32)),
            "what shows still works"
        );
        for x in (width as usize + POINTER_TOLERANCE + 1..width as usize + 300).step_by(4) {
            assert!(
                !details.click_changes_the_photo(width, x as f32),
                "a click at {x}, outside the {width} of space"
            );
        }
    }
}
