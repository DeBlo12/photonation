// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Colour-grading wheel: hue around the circle, strength from the centre.

use crate::theme;
use crate::widgets::hsv;
use eframe::egui::{Mesh, Pos2, Response, Sense, Stroke, Ui, Vec2, vec2};
use photonation_develop::GradingWheel;

const RINGS: usize = 8;
const SEGMENTS: usize = 72;

pub fn color_wheel(ui: &mut Ui, wheel: &mut GradingWheel, diameter: f32) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(
        vec2(ui.available_width(), diameter + 8.0),
        Sense::click_and_drag(),
    );
    let center = rect.center();
    let radius = diameter / 2.0;

    let before = *wheel;
    if response.double_clicked() {
        wheel.hue = 0.0;
        wheel.saturation = 0.0;
    } else if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let offset = pointer - center;
        // Screen y grows downward; hue grows counter-clockwise.
        wheel.hue = (-offset.y).atan2(offset.x).to_degrees().rem_euclid(360.0);
        wheel.saturation = (offset.length() / radius * 100.0).clamp(0.0, 100.0);
    }
    if *wheel != before {
        response.mark_changed();
    }

    let painter = ui.painter();
    painter.add(wheel_mesh(center, radius));
    painter.circle_stroke(center, radius, Stroke::new(1.0, theme::SEPARATOR));
    let angle = wheel.hue.to_radians();
    let puck = center + Vec2::new(angle.cos(), -angle.sin()) * radius * wheel.saturation / 100.0;
    painter.circle_filled(
        puck + vec2(0.0, 1.0),
        8.0,
        eframe::egui::Color32::from_black_alpha(100),
    );
    painter.circle_filled(
        puck,
        7.0,
        hsv(wheel.hue / 360.0, wheel.saturation / 100.0 * 0.8, 0.95),
    );
    painter.circle_stroke(puck, 7.0, Stroke::new(2.0, theme::KNOB));
    response
}

fn wheel_mesh(center: Pos2, radius: f32) -> Mesh {
    let mut mesh = Mesh::default();
    let point = |ring: usize, segment: usize| {
        let r = radius * ring as f32 / RINGS as f32;
        let a = segment as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        center + Vec2::new(a.cos(), -a.sin()) * r
    };
    for ring in 0..=RINGS {
        for segment in 0..=SEGMENTS {
            let saturation = ring as f32 / RINGS as f32 * 0.75;
            mesh.colored_vertex(
                point(ring, segment),
                hsv(segment as f32 / SEGMENTS as f32, saturation, 0.62),
            );
        }
    }
    let stride = (SEGMENTS + 1) as u32;
    for ring in 0..RINGS as u32 {
        for segment in 0..SEGMENTS as u32 {
            let a = ring * stride + segment;
            let b = a + stride;
            mesh.add_triangle(a, b, a + 1);
            mesh.add_triangle(a + 1, b, b + 1);
        }
    }
    mesh
}
