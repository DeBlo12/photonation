// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::theme;
use eframe::egui::{Response, Sense, Ui, Vec2, pos2, vec2};

/// How much room a switch takes.
pub const TOGGLE_SIZE: Vec2 = vec2(30.0, 17.0);

/// iOS-style switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(TOGGLE_SIZE, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool(response.id, *on);
    let track = theme::CONTROL.lerp_to_gamma(theme::ACCENT, t);
    let painter = ui.painter();
    painter.rect_filled(rect, rect.height() / 2.0, track);
    let radius = rect.height() / 2.0 - 2.0;
    let x = egui_lerp(rect.left() + radius + 2.0, rect.right() - radius - 2.0, t);
    painter.circle_filled(pos2(x, rect.center().y), radius, theme::KNOB);
    response
}

fn egui_lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
