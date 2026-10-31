// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Collapsible adjustment card with an on/off switch and a reset button,
//! like Photomator's adjustment list.

use crate::theme;
use crate::widgets::{Icon, paint_icon, toggle};
use eframe::egui::{self, Align2, Sense, Ui, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionAction {
    None,
    Reset,
}

/// Draws a card titled `title`. `expanded` and `enabled` are updated from
/// clicks; `content` runs only when expanded.
pub fn section(
    ui: &mut Ui,
    title: &str,
    expanded: &mut bool,
    enabled: &mut bool,
    modified: bool,
    content: impl FnOnce(&mut Ui),
) -> SectionAction {
    let mut action = SectionAction::None;
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let header_height = 22.0;
        let (header, header_response) =
            ui.allocate_exact_size(vec2(ui.available_width(), header_height), Sense::click());
        let openness = ui.ctx().animate_bool(header_response.id, *expanded);

        // Chevron, rotating as the card opens.
        let painter = ui.painter();
        let chevron_center = pos2(header.left() + 6.0, header.center().y);
        let angle = openness * std::f32::consts::FRAC_PI_2;
        let (sin, cos) = angle.sin_cos();
        let rotate = |x: f32, y: f32| chevron_center + vec2(x * cos - y * sin, x * sin + y * cos);
        let chevron_color = if header_response.hovered() {
            theme::TEXT
        } else {
            theme::TEXT_SECONDARY
        };
        painter.line(
            vec![rotate(-2.0, -4.0), rotate(2.5, 0.0), rotate(-2.0, 4.0)],
            egui::Stroke::new(1.6, chevron_color),
        );

        let title_color = if *enabled {
            theme::TEXT
        } else {
            theme::TEXT_TERTIARY
        };
        let title_rect = painter.text(
            pos2(header.left() + 18.0, header.center().y),
            Align2::LEFT_CENTER,
            title,
            theme::body(14.0),
            title_color,
        );
        if modified && *enabled {
            painter.circle_filled(
                pos2(title_rect.right() + 7.0, header.center().y),
                3.0,
                theme::ACCENT,
            );
        }

        // Switch and reset on the right edge of the header.
        let mut controls = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    pos2(header.right() - 90.0, header.top()),
                    header.max,
                ))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        toggle(&mut controls, enabled).on_hover_text(if *enabled { "Turn off" } else { "Turn on" });
        if modified {
            let icon_id = egui::IdSalt::new("reset-icon");
            let reset = egui::Button::new(egui::Atom::custom(icon_id, vec2(14.0, 14.0)))
                .frame(false)
                .atom_ui(&mut controls);
            if let Some(rect) = reset.rect(icon_id) {
                let color = if reset.hovered() {
                    theme::TEXT
                } else {
                    theme::TEXT_SECONDARY
                };
                paint_icon(controls.painter(), rect, Icon::Rotate, color);
            }
            if reset.response.on_hover_text("Reset").clicked() {
                action = SectionAction::Reset;
            }
        }
        let over_controls = controls
            .min_rect()
            .contains(header_response.interact_pointer_pos().unwrap_or_default());
        if header_response.clicked() && !over_controls {
            *expanded = !*expanded;
        }

        if openness > 0.0 {
            ui.add_space(8.0 * openness);
            ui.scope(|ui| {
                ui.set_opacity(openness);
                if !*enabled {
                    ui.disable();
                }
                content(ui);
            });
        }
    });
    action
}
