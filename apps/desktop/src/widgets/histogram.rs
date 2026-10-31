// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::theme;
use eframe::egui::{Color32, CornerRadius, Mesh, Pos2, Rect, Ui, vec2};
use photonation_engine::Histogram;

/// RGB histogram drawn as translucent overlapping areas with a luminance outline.
pub fn histogram(ui: &mut Ui, histogram: Option<&Histogram>) {
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), 84.0),
        eframe::egui::Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(theme::CONTROL_RADIUS),
        theme::WINDOW,
    );
    let Some(h) = histogram else { return };
    let peak = h.peak() as f32;
    let area = rect.shrink2(vec2(4.0, 6.0));
    for (bins, color) in [
        (&h.red, Color32::from_rgba_unmultiplied(255, 70, 70, 90)),
        (&h.green, Color32::from_rgba_unmultiplied(70, 220, 90, 90)),
        (&h.blue, Color32::from_rgba_unmultiplied(70, 130, 255, 90)),
        (
            &h.luminance,
            Color32::from_rgba_unmultiplied(220, 220, 225, 60),
        ),
    ] {
        painter.add(area_mesh(area, bins, peak, color));
    }
}

/// Filled area under a 256-bin curve (square-root scaled so faint tones stay visible).
pub(super) fn area_mesh(rect: Rect, bins: &[u32], peak: f32, color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    let last = (bins.len() - 1) as f32;
    for (i, &count) in bins.iter().enumerate() {
        let x = rect.left() + rect.width() * i as f32 / last;
        let height = ((count as f32 / peak).min(1.0)).sqrt() * rect.height();
        mesh.colored_vertex(Pos2::new(x, rect.bottom()), color);
        mesh.colored_vertex(Pos2::new(x, rect.bottom() - height), color);
        if i > 0 {
            let base = (i as u32 - 1) * 2;
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 2, base + 3);
        }
    }
    mesh
}
