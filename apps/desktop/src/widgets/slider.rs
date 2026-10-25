// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The adjustment slider: label and value above a thin track whose fill
//! grows from the default (centre for bipolar sliders). Drag anywhere on
//! the row; hold Option for fine control; double-click to reset.

use crate::theme;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, Mesh, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2,
};

/// How the track is painted.
#[derive(Clone)]
pub enum Track {
    /// Neutral track with an accent fill from the default value.
    Plain,
    /// Colour gradient along the track (temperature, tint, hue…).
    Gradient(Vec<Color32>),
}

#[derive(Clone)]
pub struct SliderStyle {
    pub track: Track,
    /// Formats the displayed value.
    pub format: fn(f32) -> String,
}

impl Default for SliderStyle {
    fn default() -> Self {
        SliderStyle {
            track: Track::Plain,
            format: |v| format!("{:+.0}", v).replace("+0", "0"),
        }
    }
}

const ROW_HEIGHT: f32 = 36.0;
const TRACK_HEIGHT: f32 = 4.0;
const KNOB_RADIUS: f32 = 7.0;
const FINE_FACTOR: f32 = 0.1;

pub fn adjustment_slider(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    style: &SliderStyle,
) -> Response {
    let (min, max) = (*range.start(), *range.end());
    let width = ui.available_width();
    let (rect, mut response) =
        ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click_and_drag());
    let track_rect = Rect::from_min_max(
        pos2(
            rect.left() + KNOB_RADIUS,
            rect.bottom() - 10.0 - TRACK_HEIGHT / 2.0,
        ),
        pos2(
            rect.right() - KNOB_RADIUS,
            rect.bottom() - 10.0 + TRACK_HEIGHT / 2.0,
        ),
    );
    let to_x = |v: f32| track_rect.left() + (v - min) / (max - min) * track_rect.width();

    let before = *value;
    if response.double_clicked() {
        *value = default;
    } else if response.dragged() {
        let fine = ui.input(|i| i.modifiers.alt);
        if response.drag_started() && !fine {
            if let Some(pointer) = response.interact_pointer_pos() {
                *value = min
                    + ((pointer.x - track_rect.left()) / track_rect.width()).clamp(0.0, 1.0)
                        * (max - min);
            }
        } else {
            let scale = if fine { FINE_FACTOR } else { 1.0 };
            *value += response.drag_delta().x / track_rect.width() * (max - min) * scale;
        }
        *value = value.clamp(min, max);
    }
    if *value != before {
        response.mark_changed();
    }

    let painter = ui.painter_at(rect.expand(2.0));
    let active = response.hovered() || response.dragged();
    let label_color = if active {
        theme::TEXT
    } else {
        theme::TEXT_SECONDARY
    };
    painter.text(
        pos2(rect.left(), rect.top() + 9.0),
        Align2::LEFT_CENTER,
        label,
        theme::body(12.5),
        label_color,
    );
    let value_color = if (*value - default).abs() > f32::EPSILON {
        theme::TEXT
    } else {
        theme::TEXT_TERTIARY
    };
    painter.text(
        pos2(rect.right(), rect.top() + 9.0),
        Align2::RIGHT_CENTER,
        (style.format)(*value),
        theme::body(12.5),
        value_color,
    );

    let radius = CornerRadius::same((TRACK_HEIGHT / 2.0) as u8);
    match &style.track {
        Track::Plain => {
            painter.rect_filled(track_rect, radius, theme::TRACK);
            let (a, b) = (to_x(default.clamp(min, max)), to_x(*value));
            let fill = Rect::from_min_max(
                pos2(a.min(b), track_rect.top()),
                pos2(a.max(b), track_rect.bottom()),
            );
            painter.rect_filled(fill, radius, theme::ACCENT);
        }
        Track::Gradient(colors) => paint_gradient(&painter, track_rect, colors),
    }
    // Tick at the default so the neutral position is always visible.
    let tick_x = to_x(default.clamp(min, max));
    painter.line_segment(
        [
            pos2(tick_x, track_rect.top() - 3.0),
            pos2(tick_x, track_rect.bottom() + 3.0),
        ],
        Stroke::new(1.0, theme::TEXT_TERTIARY),
    );

    let knob = pos2(to_x(*value), track_rect.center().y);
    painter.circle_filled(
        knob + vec2(0.0, 1.0),
        KNOB_RADIUS + 0.5,
        Color32::from_black_alpha(90),
    );
    painter.circle_filled(
        knob,
        if active {
            KNOB_RADIUS + 0.5
        } else {
            KNOB_RADIUS
        },
        theme::KNOB,
    );
    response
}

/// Horizontal multi-stop gradient as a mesh.
pub fn paint_gradient(painter: &egui::Painter, rect: Rect, colors: &[Color32]) {
    if colors.len() < 2 {
        return;
    }
    let mut mesh = Mesh::default();
    for (i, &color) in colors.iter().enumerate() {
        let x = rect.left() + rect.width() * i as f32 / (colors.len() - 1) as f32;
        mesh.colored_vertex(Pos2::new(x, rect.top()), color);
        mesh.colored_vertex(Pos2::new(x, rect.bottom()), color);
        if i > 0 {
            let base = (i as u32 - 1) * 2;
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 2, base + 3);
        }
    }
    painter.add(mesh);
}
