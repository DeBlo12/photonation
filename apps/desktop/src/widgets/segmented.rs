// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::theme;
use eframe::egui::{Align2, CornerRadius, Rect, Response, Sense, Ui, pos2, vec2};

/// Segmented control; returns the response of the row and updates `selected`.
pub fn segmented(ui: &mut Ui, labels: &[&str], selected: &mut usize) -> Response {
    let height = 24.0;
    let (rect, mut response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(theme::CONTROL_RADIUS),
        theme::CONTROL,
    );
    let segment_width = rect.width() / labels.len() as f32;
    for (index, label) in labels.iter().enumerate() {
        let segment = Rect::from_min_size(
            pos2(rect.left() + segment_width * index as f32, rect.top()),
            vec2(segment_width, height),
        )
        .shrink(2.0);
        let is_selected = index == *selected;
        if is_selected {
            painter.rect_filled(
                segment,
                CornerRadius::same(theme::CONTROL_RADIUS - 1),
                theme::CONTROL_HOVER,
            );
        }
        let color = if is_selected {
            theme::TEXT
        } else {
            theme::TEXT_SECONDARY
        };
        painter.text(
            segment.center(),
            Align2::CENTER_CENTER,
            *label,
            theme::body(12.0),
            color,
        );
    }
    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let index = (((pointer.x - rect.left()) / segment_width) as usize).min(labels.len() - 1);
        if index != *selected {
            *selected = index;
            response.mark_changed();
        }
    }
    response
}
