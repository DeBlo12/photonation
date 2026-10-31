// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::app::{PhotonationApp, ViewState, Zoom};
use crate::panels::{icon_button, tool_button};
use crate::theme;
use crate::widgets::Icon;
use eframe::egui::{self, Align, Button, Layout, RichText};

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
                ui.add_space(theme::TRAFFIC_LIGHT_INSET);
                if icon_button(
                    ui,
                    Icon::Library,
                    Some("Open"),
                    false,
                    &format!("Open a folder ({})", theme::SHORTCUT_OPEN),
                )
                .clicked()
                {
                    app.pick_folder();
                }
                ui.add_space(12.0);
                if let Some(index) = app.current {
                    ui.label(
                        RichText::new(&app.photos[index].name)
                            .size(13.5)
                            .color(theme::TEXT),
                    );
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
                        Button::new(RichText::new("Export").size(13.0).color(theme::TEXT))
                            .fill(theme::ACCENT)
                            .corner_radius(theme::CONTROL_RADIUS)
                            .min_size(egui::vec2(72.0, 26.0)),
                    );
                    if export
                        .on_hover_text(format!("Export ({})", theme::SHORTCUT_EXPORT))
                        .clicked()
                    {
                        app.view.show_export = true;
                    }
                    ui.add_space(8.0);
                    let is_actual =
                        matches!(app.view.zoom, Zoom::Scale(s) if (s - 1.0).abs() < 1e-3);
                    if tool_button(ui, "1:1", is_actual, "Actual pixels").clicked() {
                        app.view.zoom = if is_actual {
                            Zoom::Fit
                        } else {
                            Zoom::Scale(1.0)
                        };
                        app.view.pan = egui::Vec2::ZERO;
                    }
                    let fit_tooltip = format!("Fit to window ({})", theme::SHORTCUT_FIT);
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
                        "Compare with original (\\)",
                    );
                    if compare.clicked() {
                        app.view.compare = !app.view.compare;
                    }
                    let tools: [SidebarTool; 3] = [
                        (
                            Icon::Layers,
                            app.view.show_masks,
                            "Masks (M)",
                            ViewState::toggle_masks,
                        ),
                        (
                            Icon::Crop,
                            app.view.cropping,
                            "Crop and straighten (R)",
                            ViewState::toggle_cropping,
                        ),
                        (
                            Icon::Retouch,
                            app.view.show_removals,
                            "Remove objects (Q)",
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
                    let redo_tooltip = format!("Redo ({})", theme::SHORTCUT_REDO);
                    if ui
                        .add_enabled_ui(app.history.can_redo(), |ui| {
                            icon_button(ui, Icon::Redo, None, false, &redo_tooltip)
                        })
                        .inner
                        .clicked()
                    {
                        app.redo();
                    }
                    let undo_tooltip = format!("Undo ({})", theme::SHORTCUT_UNDO);
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
