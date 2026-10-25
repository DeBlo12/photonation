// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::app::PhotonationApp;
use crate::theme;
use eframe::egui::{self, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

const THUMB_HEIGHT: f32 = 72.0;
const PLACEHOLDER_ASPECT: f32 = 1.5;

pub fn show(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    egui::Panel::bottom("filmstrip")
        .exact_size(theme::FILMSTRIP_HEIGHT)
        .show_separator_line(false)
        .frame(
            egui::Frame::NONE
                .fill(theme::WINDOW)
                .inner_margin(egui::Margin::symmetric(14, 12)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::horizontal()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let mut clicked = None;
                        for (index, photo) in app.photos.iter().enumerate() {
                            let aspect = photo
                                .thumbnail
                                .as_ref()
                                .map_or(PLACEHOLDER_ASPECT, |t| t.aspect_ratio());
                            let (rect, response) = ui.allocate_exact_size(
                                vec2(THUMB_HEIGHT * aspect, THUMB_HEIGHT),
                                Sense::click(),
                            );
                            let selected = app.current == Some(index);
                            if selected
                                && ui.ctx().input(|i| {
                                    i.key_pressed(egui::Key::ArrowLeft)
                                        || i.key_pressed(egui::Key::ArrowRight)
                                })
                            {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            let painter = ui.painter();
                            let radius = CornerRadius::same(theme::CONTROL_RADIUS);
                            match &photo.thumbnail {
                                Some(texture) => {
                                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                                    painter.add(
                                        egui::epaint::RectShape::filled(
                                            rect,
                                            radius,
                                            egui::Color32::WHITE,
                                        )
                                        .with_texture(texture.id(), uv),
                                    );
                                }
                                None => {
                                    painter.rect_filled(rect, radius, theme::CARD);
                                    if photo.unsupported {
                                        painter.text(
                                            rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "Unsupported",
                                            theme::body(10.5),
                                            theme::TEXT_TERTIARY,
                                        );
                                    }
                                }
                            }
                            if !selected {
                                let dim = if response.hovered() { 20 } else { 70 };
                                painter.rect_filled(
                                    rect,
                                    radius,
                                    egui::Color32::from_black_alpha(dim),
                                );
                            }
                            if selected {
                                painter.rect_stroke(
                                    rect.expand(2.0),
                                    CornerRadius::same(theme::CONTROL_RADIUS + 2),
                                    Stroke::new(2.0, theme::ACCENT),
                                    StrokeKind::Outside,
                                );
                            }
                            if photo.has_edits {
                                painter.circle_filled(
                                    pos2(rect.right() - 8.0, rect.top() + 8.0),
                                    3.5,
                                    theme::ACCENT,
                                );
                            }
                            if response.on_hover_text(&photo.name).clicked() {
                                clicked = Some(index);
                            }
                        }
                        if let Some(index) = clicked {
                            app.select(index);
                        }
                    });
                });
        });
}
