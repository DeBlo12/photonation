// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The window's regions: toolbar (top), filmstrip (bottom), adjustments,
//! masks, crop or remove sidebar (right), viewer (centre) and the export
//! sheet.

pub mod adjustments;
pub mod crop;
pub mod export;
pub mod filmstrip;
pub mod mask_handles;
pub mod mask_thumbnails;
pub mod masks;
pub mod point_colors;
pub mod removal_brush;
pub mod removals;
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
    ui.add(
        Button::new(text)
            .fill(fill)
            .corner_radius(theme::CONTROL_RADIUS)
            .min_size(egui::vec2(30.0, 26.0)),
    )
    .on_hover_text(tooltip)
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
    response.response.on_hover_text(tooltip)
}
