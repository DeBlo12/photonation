// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The window's regions: toolbar (top), filmstrip (bottom), adjustments,
//! masks or crop sidebar (right), viewer (centre) and the export sheet.

pub mod adjustments;
pub mod crop;
pub mod export;
pub mod filmstrip;
pub mod mask_handles;
pub mod mask_thumbnails;
pub mod masks;
pub mod toolbar;
pub mod viewer;

use crate::theme;
use eframe::egui::{self, Button, Response, RichText, Ui};

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
