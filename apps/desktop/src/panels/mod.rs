// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The window's regions: toolbar (top), filmstrip (bottom), adjustments,
//! masks, crop or remove sidebar (right), viewer (centre) and the export
//! sheet.

pub mod adjustments;
pub mod crop;
pub mod export;
pub mod filmstrip;
pub mod help;
pub mod mask_handles;
pub mod mask_thumbnails;
pub mod masks;
pub mod menu_bar;
pub mod point_colors;
pub mod removal_brush;
pub mod removals;
pub mod settings;
pub mod toolbar;
pub mod viewer;

use crate::theme;
use crate::widgets::{Icon, paint_icon};
use eframe::egui::{self, Atom, Button, IdSalt, Response, RichText, Ui};

/// Brand icons in toolbar buttons, in points.
pub const ICON_SIZE: f32 = 18.0;

/// Borderless toolbar button with an optional "on" state.
pub fn tool_button(ui: &mut Ui, label: &str, active: bool, tooltip: &str) -> Response {
    let text =
        RichText::new(label)
            .size(13.0)
            .color(if active { theme::ACCENT } else { theme::TEXT });
    let fill = if active {
        theme::ACCENT_SOFT
    } else {
        egui::Color32::TRANSPARENT
    };
    let response = ui.add(
        Button::new(text)
            .fill(fill)
            .corner_radius(theme::CONTROL_RADIUS)
            .min_size(egui::vec2(30.0, 26.0)),
    );
    hover_tip(response, tooltip)
}

/// [`tool_button`] showing a brand icon, before `label` if there is one.
pub fn icon_button(
    ui: &mut Ui,
    icon: Icon,
    label: Option<&str>,
    active: bool,
    tooltip: &str,
) -> Response {
    let color = if active { theme::ACCENT } else { theme::TEXT };
    let fill = if active {
        theme::ACCENT_SOFT
    } else {
        egui::Color32::TRANSPARENT
    };
    let id = IdSalt::new(("icon", icon));
    let atom = Atom::custom(id, egui::Vec2::splat(ICON_SIZE));
    let button = match label {
        Some(label) => Button::new((atom, RichText::new(label).size(13.0).color(color))),
        None => Button::new(atom),
    };
    let response = button
        .fill(fill)
        .corner_radius(theme::CONTROL_RADIUS)
        .min_size(egui::vec2(30.0, 26.0))
        .atom_ui(ui);
    if let Some(rect) = response.rect(id) {
        paint_icon(ui.painter(), rect, icon, color);
    }
    hover_tip(response.response, tooltip)
}

/// Shows `tip` while the pointer is on `response`, also when the control
/// is disabled, so a greyed-out tool still says what it is. The first line
/// is the control's name; any further lines explain it.
pub fn hover_tip(response: Response, tip: &str) -> Response {
    if tip.is_empty() {
        return response;
    }
    let (name, detail) = tip.split_once('\n').unwrap_or((tip, ""));
    let show = move |ui: &mut Ui| {
        ui.set_max_width(TIP_WIDTH);
        ui.label(
            RichText::new(name)
                .font(theme::title(12.5))
                .color(theme::TEXT),
        );
        if !detail.is_empty() {
            ui.label(
                RichText::new(detail)
                    .font(theme::body(12.0))
                    .color(theme::TEXT_SECONDARY),
            );
        }
    };
    response.on_hover_ui(show).on_disabled_hover_ui(show)
}

const TIP_WIDTH: f32 = 260.0;
