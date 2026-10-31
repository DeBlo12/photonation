// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::app::{PhotonationApp, ViewState, Zoom};
use crate::panels::{hover_tip, icon_button, tool_button};
use crate::theme;
use crate::widgets::{Icon, rating_control};
use eframe::egui::{self, Align, Layout, RichText};

/// A tool with its own sidebar: icon, whether it is open, tooltip, toggle.
type SidebarTool = (Icon, bool, &'static str, fn(&mut ViewState));

pub fn show(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    egui::Panel::top("toolbar")
        .exact_size(theme::TOOLBAR_HEIGHT)
        .show_separator_line(false)
        .frame(
            egui::Frame::NONE
                .fill(theme::WINDOW)
                .inner_margin(egui::Margin::symmetric(12, 8)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                if icon_button(
                    ui,
                    Icon::Library,
                    Some("Open"),
                    false,
                    &format!(
                        "Open a folder ({})\nShows its raw photos in the filmstrip. Edits are \
                         saved beside each photo; the raw file is never changed.",
                        theme::SHORTCUT_OPEN
                    ),
                )
                .clicked()
                {
                    app.pick_folder();
                }
                library_buttons(app, ui);
                ui.add_space(12.0);
                if let Some(index) = app.current {
                    ui.label(
                        RichText::new(&app.photos[index].name)
                            .size(13.5)
                            .color(theme::TEXT),
                    );
                    ui.add_space(6.0);
                    if let Some(change) = rating_control(ui, app.photos[index].library) {
                        app.change_library(change);
                    }
                    ui.add_space(6.0);
                    if let Some(info) = &app.info {
                        ui.label(
                            RichText::new(camera_summary(info))
                                .size(12.0)
                                .color(theme::TEXT_SECONDARY),
                        );
                    }
                }

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
                                .color(theme::TEXT_SECONDARY),
                        );
                        ui.add(egui::Spinner::new().size(14.0));
                    }
                });
            });
        });
}

/// The grid and, while a filter hides photos, a reminder of it.
fn library_buttons(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    if app.photos.is_empty() {
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
