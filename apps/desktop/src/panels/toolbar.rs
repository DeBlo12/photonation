// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::app::{PhotonationApp, Zoom};
use crate::panels::tool_button;
use crate::theme;
use eframe::egui::{self, Align, Button, Layout, RichText};

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
                if tool_button(ui, "📁  Open", false, "Open a folder (⌘O)").clicked() {
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
                    if export.on_hover_text("Export (⇧⌘E)").clicked() {
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
                    if tool_button(ui, "Fit", app.view.zoom == Zoom::Fit, "Fit to window (⌘0)")
                        .clicked()
                    {
                        app.view.zoom = Zoom::Fit;
                        app.view.pan = egui::Vec2::ZERO;
                    }
                    ui.add_space(8.0);
                    if tool_button(ui, "◐", app.view.compare, "Compare with original (\\)")
                        .clicked()
                    {
                        app.view.compare = !app.view.compare;
                    }
                    if ui
                        .add_enabled_ui(has_photo, |ui| {
                            tool_button(ui, "◎", app.view.show_masks, "Masks (M)")
                        })
                        .inner
                        .clicked()
                    {
                        app.view.show_masks = !app.view.show_masks;
                    }
                    ui.add_space(8.0);
                    if ui
                        .add_enabled(app.history.can_redo(), tool_button_widget("↷"))
                        .on_hover_text("Redo (⇧⌘Z)")
                        .clicked()
                    {
                        app.redo();
                    }
                    if ui
                        .add_enabled(app.history.can_undo(), tool_button_widget("↶"))
                        .on_hover_text("Undo (⌘Z)")
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

fn tool_button_widget(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).size(15.0))
        .fill(egui::Color32::TRANSPARENT)
        .min_size(egui::vec2(30.0, 26.0))
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
