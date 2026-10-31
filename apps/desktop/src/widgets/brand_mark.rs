// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The Photonation iris: six paper blades around an amber photon, drawn as
//! vectors from the app icon's geometry (`packaging/icons`), so it stays
//! sharp at any size.

use crate::theme;
use eframe::egui::{Color32, Painter, Pos2, Shape, Stroke, Vec2, vec2};

/// The icon's 100 u grid: blades reach this radius around (50, 50).
const RADIUS: f32 = 34.0;
const CENTER: Vec2 = vec2(50.0, 50.0);
/// Each blade's inner corner, where its rim arc starts, and its shade.
const BLADES: [([f32; 2], [f32; 2], Color32); 6] = [
    (
        [62.07, 53.24],
        [34.86, 80.45],
        Color32::from_rgb(0xd6, 0xcf, 0xc1),
    ),
    (
        [53.24, 62.07],
        [16.07, 52.11],
        Color32::from_rgb(0xea, 0xe5, 0xda),
    ),
    (
        [41.16, 58.84],
        [31.20, 21.67],
        Color32::from_rgb(0xff, 0xfd, 0xf8),
    ),
    (
        [37.93, 46.76],
        [65.14, 19.55],
        Color32::from_rgb(0xf6, 0xf2, 0xea),
    ),
    (
        [46.76, 37.93],
        [83.93, 47.89],
        Color32::from_rgb(0xe2, 0xdc, 0xd0),
    ),
    (
        [58.84, 41.16],
        [68.80, 78.33],
        Color32::from_rgb(0xcd, 0xc5, 0xb6),
    ),
];
const ARC_STEPS: usize = 16;

/// Paints the iris `diameter` points across at `center`, on a surface of
/// colour `background` (the gaps between blades and the aperture show it).
pub fn paint_brand_mark(painter: &Painter, center: Pos2, diameter: f32, background: Color32) {
    let scale = diameter / (2.0 * RADIUS);
    let to_screen = |[x, y]: [f32; 2]| center + (vec2(x, y) - CENTER) * scale;
    let angle_of = |[x, y]: [f32; 2]| (y - CENTER.y).atan2(x - CENTER.x);
    let on_rim = |angle: f32| center + vec2(angle.cos(), angle.sin()) * RADIUS * scale;
    let blade_angle = std::f32::consts::TAU / BLADES.len() as f32;
    let mut aperture = Vec::with_capacity(BLADES.len());
    let mut seams = Vec::with_capacity(BLADES.len());
    for (inner, rim, color) in BLADES {
        let start = angle_of(rim);
        let mut points = vec![to_screen(inner)];
        points.extend(
            (0..=ARC_STEPS).map(|i| on_rim(start - blade_angle * i as f32 / ARC_STEPS as f32)),
        );
        painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
        aperture.push(to_screen(inner));
        seams.push([to_screen(inner), on_rim(start - blade_angle)]);
    }
    let seam = Stroke::new(0.9 * scale, background);
    for [from, to] in seams {
        painter.line_segment([from, to], seam);
    }
    painter.add(Shape::convex_polygon(aperture, background, Stroke::NONE));
    painter.circle_filled(center, 5.0 * scale, theme::ACCENT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    /// The blades' rim points must sit on the icon's circle, or the mark
    /// no longer matches the app icon.
    #[test]
    fn blade_rims_lie_on_the_circle() {
        for (_, [x, y], _) in BLADES {
            let distance = (pos2(x, y) - CENTER.to_pos2()).length();
            assert!((distance - RADIUS).abs() < 0.05, "{distance}");
        }
    }
}
