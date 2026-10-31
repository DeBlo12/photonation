// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The brand's stroke icons, from the Photonation icon set: paths on a 24 u
//! grid (20 u live area) drawn with round ends. The stroke thins as icons
//! grow, so their weight looks the same at every size. Icons the set lacks
//! (undo, redo, compare, the gradient shapes) are drawn to the same rules.

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use kurbo::PathEl;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    // From the set.
    Library,
    Crop,
    Layers,
    Retouch,
    Rotate,
    Flip,
    Focus,
    // Drawn for Photonation in the set's style.
    Undo,
    Redo,
    Compare,
    Help,
    LinearGradient,
    RadialGradient,
    SplitHandle,
    Grid,
    Survey,
    Pick,
    Reject,
}

impl Icon {
    const COUNT: usize = 18;

    /// SVG path data on the 24 u grid.
    fn path(self) -> &'static str {
        match self {
            Icon::Library => {
                "M5.5 7.5h9q2 0 2 2v8.5q0 2 -2 2h-9q-2 0 -2 -2v-8.5q0 -2 2 -2z\
                 M7.5 4.5h10a3 3 0 0 1 3 3v10M4 17.5l3.5-3.5 3.5 3.5 2-2 3 3"
            }
            Icon::Crop => "M7 3v12a2 2 0 0 0 2 2h12M3 7h12a2 2 0 0 1 2 2v12",
            Icon::Layers => {
                "M12 3.5 3.5 8 12 12.5 20.5 8zM3.5 12 12 16.5 20.5 12M3.5 16 12 20.5 20.5 16"
            }
            Icon::Retouch => {
                "M4.5 19.5 14 10M13 7.5l3.5 3.5M17.5 3v3M16 4.5h3M20 9.5v2M19 10.5h2M9.5 4v2M8.5 5h2"
            }
            Icon::Rotate => "M19.5 12A7.5 7.5 0 1 1 17.3 6.7M17.3 2.9v3.8h-3.8",
            Icon::Flip => {
                "M4.5 12a7.5 7.5 0 0 1 13.4-4.6M18 4v3.5h-3.5M19.5 12a7.5 7.5 0 0 1-13.4 4.6M6 20v-3.5h3.5"
            }
            Icon::Focus => {
                "M4 9V6a2 2 0 0 1 2-2h3M15 4h3a2 2 0 0 1 2 2v3M20 15v3a2 2 0 0 1-2 2h-3\
                 M9 20H6a2 2 0 0 1-2-2v-3M12 12m-2.5 0a2.5 2.5 0 1 0 5 0a2.5 2.5 0 1 0 -5 0"
            }
            Icon::Undo => "M9 14.5 4.5 10 9 5.5M4.5 10h10a5 5 0 0 1 0 10H12",
            Icon::Redo => "M15 14.5 19.5 10 15 5.5M19.5 10h-10a5 5 0 0 0 0 10H12",
            Icon::Compare => "M6 4h12q2 0 2 2v12q0 2 -2 2h-12q-2 0 -2 -2v-12q0 -2 2 -2zM12 2v20",
            Icon::Help => {
                "M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0\
                 M9.6 9.4a2.5 2.5 0 1 1 3.2 2.4c-.5.2-.8.6-.8 1.2v.9M12 16.6h.01"
            }
            Icon::LinearGradient => {
                "M6 4h12q2 0 2 2v12q0 2 -2 2h-12q-2 0 -2 -2v-12q0 -2 2 -2zM4 9.5h16M4 14.5h16"
            }
            Icon::RadialGradient => "M12 12m-8 0a8 6 0 1 0 16 0a8 6 0 1 0 -16 0M12 12h.01",
            Icon::SplitHandle => "M9 7 4 12l5 5M15 7l5 5-5 5",
            Icon::Grid => {
                "M5.5 4h3q1.5 0 1.5 1.5v3q0 1.5 -1.5 1.5h-3q-1.5 0 -1.5 -1.5v-3q0 -1.5 1.5 -1.5z                 M15.5 4h3q1.5 0 1.5 1.5v3q0 1.5 -1.5 1.5h-3q-1.5 0 -1.5 -1.5v-3q0 -1.5 1.5 -1.5z                 M5.5 14h3q1.5 0 1.5 1.5v3q0 1.5 -1.5 1.5h-3q-1.5 0 -1.5 -1.5v-3q0 -1.5 1.5 -1.5z                 M15.5 14h3q1.5 0 1.5 1.5v3q0 1.5 -1.5 1.5h-3q-1.5 0 -1.5 -1.5v-3q0 -1.5 1.5 -1.5z"
            }
            Icon::Survey => {
                "M4.5 5h5q1.5 0 1.5 1.5v11q0 1.5 -1.5 1.5h-5q-1.5 0 -1.5 -1.5v-11q0 -1.5 1.5 -1.5z                 M14.5 5h5q1.5 0 1.5 1.5v11q0 1.5 -1.5 1.5h-5q-1.5 0 -1.5 -1.5v-11q0 -1.5 1.5 -1.5z"
            }
            Icon::Pick => "M6 21V4M6 4.5h11l-2.5 4 2.5 4H6",
            Icon::Reject => "M6.5 6.5l11 11M17.5 6.5l-11 11",
        }
    }
}

/// Stroke width in grid units for an icon `size` points wide: the set's
/// optical ladder (2.0 at 16, 1.75 at 20–24, 1.5 at 32, 1.25 at 48).
fn stroke_units(size: f32) -> f32 {
    match size {
        s if s <= 16.0 => 2.0,
        s if s <= 24.0 => 1.75,
        s if s <= 32.0 => 1.5,
        s if s <= 48.0 => 1.25,
        _ => 1.0,
    }
}

/// One flattened subpath in grid units.
struct Subpath {
    points: Vec<Pos2>,
    closed: bool,
}

/// The icon's subpaths, flattened once.
fn subpaths(icon: Icon) -> &'static [Subpath] {
    static FLATTENED: [OnceLock<Vec<Subpath>>; Icon::COUNT] =
        [const { OnceLock::new() }; Icon::COUNT];
    FLATTENED[icon as usize].get_or_init(|| flatten(icon.path()))
}

fn flatten(data: &str) -> Vec<Subpath> {
    let path = kurbo::BezPath::from_svg(data).expect("icon paths are valid SVG path data");
    let point = |p: kurbo::Point| pos2(p.x as f32, p.y as f32);
    let mut subpaths = Vec::new();
    let mut current: Option<Subpath> = None;
    kurbo::flatten(path, 0.02, |element| match element {
        PathEl::MoveTo(p) => {
            subpaths.extend(current.take());
            current = Some(Subpath {
                points: vec![point(p)],
                closed: false,
            });
        }
        PathEl::LineTo(p) => {
            if let Some(subpath) = &mut current {
                subpath.points.push(point(p));
            }
        }
        PathEl::ClosePath => {
            if let Some(mut subpath) = current.take() {
                subpath.closed = true;
                subpaths.push(subpath);
            }
        }
        // Flattening leaves only moves, lines and closes.
        PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
    });
    subpaths.extend(current);
    // A lone move (as before an `m` that starts a circle) draws nothing.
    subpaths.retain(|subpath| subpath.points.len() > 1);
    subpaths
}

/// Paints `icon` centred in `rect`, as large as fits.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let size = rect.width().min(rect.height());
    let scale = size / 24.0;
    let origin = rect.center() - vec2(12.0, 12.0) * scale;
    let width = stroke_units(size) * scale;
    let stroke = Stroke::new(width, color);
    for subpath in subpaths(icon) {
        let mut points: Vec<Pos2> = subpath
            .points
            .iter()
            .map(|p| origin + p.to_vec2() * scale)
            .collect();
        let first = points[0];
        if points.iter().all(|p| p.distance(first) < 0.05 * scale) {
            // A dot, like the centre of the radial gradient.
            painter.circle_filled(first, width / 2.0, color);
        } else if subpath.closed {
            painter.add(Shape::closed_line(points, stroke));
        } else {
            // Extend open ends by half the width, so they cover the round
            // ends the set draws without overlapping (and darkening) a
            // translucent stroke.
            extend_end(&mut points, false, width / 2.0);
            extend_end(&mut points, true, width / 2.0);
            painter.add(Shape::line(points, stroke));
        }
    }
}

fn extend_end(points: &mut [Pos2], last: bool, by: f32) {
    let n = points.len();
    let (end, next) = if last { (n - 1, n - 2) } else { (0, 1) };
    let direction = (points[end] - points[next]).normalized();
    if direction.is_finite() {
        points[end] += direction * by;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Icon; Icon::COUNT] = [
        Icon::Library,
        Icon::Crop,
        Icon::Layers,
        Icon::Retouch,
        Icon::Rotate,
        Icon::Flip,
        Icon::Focus,
        Icon::Undo,
        Icon::Redo,
        Icon::Compare,
        Icon::Help,
        Icon::LinearGradient,
        Icon::RadialGradient,
        Icon::SplitHandle,
        Icon::Grid,
        Icon::Survey,
        Icon::Pick,
        Icon::Reject,
    ];

    /// Every icon parses and stays within the 24 u grid.
    #[test]
    fn icons_parse_onto_the_grid() {
        for (index, icon) in ALL.into_iter().enumerate() {
            assert_eq!(icon as usize, index, "COUNT and the cache index agree");
            let subpaths = subpaths(icon);
            assert!(!subpaths.is_empty(), "{icon:?} draws something");
            for point in subpaths.iter().flat_map(|s| &s.points) {
                assert!(
                    (1.0..=23.0).contains(&point.x) && (1.0..=23.0).contains(&point.y),
                    "{icon:?} leaves the grid at {point:?}"
                );
            }
        }
    }
}
