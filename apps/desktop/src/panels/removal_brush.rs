// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Painting removals on the canvas while the Remove panel is open. A drag
//! paints a stroke; it joins the settings only on release, so the fill is
//! computed once per stroke instead of for every intermediate shape.
//! ⇧-drag adds the stroke to the selected removal.

use crate::app::ViewState;
use crate::photo_frame::PhotoFrame;
use crate::theme;
use eframe::egui::{self, Color32, Painter, Pos2, Response, Shape, Stroke, Ui};
use photonation_develop::{
    BrushStroke, MAX_REMOVAL_STROKES, MAX_REMOVALS, MAX_STROKE_POINTS, Removal,
};

/// New points are added once the pointer moved this share of the radius.
const POINT_SPACING: f64 = 0.25;
/// Brushed areas of removals other than the selected one.
const OTHER_REMOVAL: Color32 = Color32::from_rgba_premultiplied(0x50, 0x50, 0x58, 0x50);

/// Handles painting. Returns whether a stroke owns the pointer, so the
/// viewer doesn't also pan.
pub fn interact(
    removals: &mut Vec<Removal>,
    view: &mut ViewState,
    ui: &Ui,
    response: &Response,
    frame: &PhotoFrame,
) -> bool {
    let radius = view.removal_brush.radius();
    if response.drag_started()
        && let Some(press) = ui.input(|i| i.pointer.press_origin())
        && frame.image_rect.contains(press)
    {
        view.painting = Some(BrushStroke {
            points: vec![frame.to_position(press)],
            radius,
        });
    }
    if response.dragged() {
        if let (Some(stroke), Some(pointer)) = (&mut view.painting, response.interact_pointer_pos())
        {
            extend(stroke, frame.to_position(pointer), frame);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            return true;
        }
        return false;
    }
    let finished = view.painting.take().or_else(|| {
        // A click without a drag dabs the brush once.
        let pointer = response.interact_pointer_pos()?;
        (response.clicked() && frame.image_rect.contains(pointer)).then(|| BrushStroke {
            points: vec![frame.to_position(pointer)],
            radius,
        })
    });
    if let Some(stroke) = finished {
        let add_to_selected = ui.input(|i| i.modifiers.shift);
        commit(removals, view, stroke, add_to_selected);
    }
    if response
        .hover_pos()
        .is_some_and(|p| frame.image_rect.contains(p))
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    false
}

/// Appends `position` once the pointer has moved far enough.
fn extend(stroke: &mut BrushStroke, position: [f64; 2], frame: &PhotoFrame) {
    if stroke.points.len() >= MAX_STROKE_POINTS {
        return;
    }
    let Some(&last) = stroke.points.last() else {
        stroke.points.push(position);
        return;
    };
    let (from, to) = (frame.space.to_metric(last), frame.space.to_metric(position));
    if (to[0] - from[0]).hypot(to[1] - from[1]) >= stroke.radius * POINT_SPACING {
        stroke.points.push(position);
    }
}

/// Adds a finished stroke: to the selected removal, or as a new removal
/// (which becomes the selection).
fn commit(
    removals: &mut Vec<Removal>,
    view: &mut ViewState,
    stroke: BrushStroke,
    add_to_selected: bool,
) {
    if add_to_selected
        && let Some(removal) = view
            .selected_removal
            .and_then(|index| removals.get_mut(index))
        && removal.strokes.len() < MAX_REMOVAL_STROKES
    {
        removal.strokes.push(stroke);
        return;
    }
    if removals.len() < MAX_REMOVALS {
        removals.push(Removal::new(view.removal_brush.method, stroke));
        view.selected_removal = Some(removals.len() - 1);
    }
}

/// Draws the brushed areas (the selected removal and the stroke being
/// painted in red, others in grey) and the brush outline under the pointer.
pub fn paint(
    removals: &[Removal],
    view: &ViewState,
    painter: &Painter,
    frame: &PhotoFrame,
    hover: Option<Pos2>,
) {
    let painter = painter.with_clip_rect(frame.image_rect.intersect(painter.clip_rect()));
    for (index, removal) in removals.iter().enumerate() {
        let color = if view.selected_removal == Some(index) {
            theme::MASK_OVERLAY
        } else {
            OTHER_REMOVAL
        };
        let color = if removal.enabled {
            color
        } else {
            color.gamma_multiply(0.4)
        };
        for stroke in &removal.strokes {
            paint_stroke(&painter, frame, stroke, color);
        }
    }
    if let Some(stroke) = &view.painting {
        paint_stroke(&painter, frame, stroke, theme::MASK_OVERLAY);
    }
    if view.painting.is_none()
        && let Some(pointer) = hover.filter(|p| frame.image_rect.contains(*p))
    {
        let radius = screen_radius(
            frame,
            frame.to_position(pointer),
            view.removal_brush.radius(),
        );
        painter.circle_stroke(pointer, radius, Stroke::new(2.5, theme::HANDLE_SHADOW));
        painter.circle_stroke(pointer, radius, Stroke::new(1.2, theme::KNOB));
    }
}

fn paint_stroke(painter: &Painter, frame: &PhotoFrame, stroke: &BrushStroke, color: Color32) {
    let Some(&first) = stroke.points.first() else {
        return;
    };
    let radius = screen_radius(frame, first, stroke.radius);
    let points: Vec<Pos2> = stroke.points.iter().map(|p| frame.to_screen(*p)).collect();
    if points.len() == 1 {
        painter.circle_filled(points[0], radius, color);
        return;
    }
    // One path, so overlapping segments don't darken; round caps.
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(radius * 2.0, color),
    ));
    for end in [points[0], points[points.len() - 1]] {
        painter.circle_filled(end, radius, color);
    }
}

/// A brush radius (long-edge units) in screen points at `position`.
fn screen_radius(frame: &PhotoFrame, position: [f64; 2], radius: f64) -> f32 {
    let centre = frame.space.to_metric(position);
    let edge = frame.metric_to_screen([centre[0] + radius, centre[1]]);
    (edge - frame.to_screen(position)).length()
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Rect, pos2, vec2};
    use photonation_develop::MaskSpace;
    use photonation_image::NormRect;

    fn frame() -> PhotoFrame {
        PhotoFrame {
            image_rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0)),
            crop: NormRect::FULL,
            space: MaskSpace::new(6000.0, 4000.0),
        }
    }

    #[test]
    fn strokes_thin_out_points_closer_than_a_quarter_radius() {
        let mut stroke = BrushStroke {
            points: vec![[0.5, 0.5]],
            radius: 0.04,
        };
        extend(&mut stroke, [0.505, 0.5], &frame());
        assert_eq!(stroke.points.len(), 1, "0.005 < 0.01");
        extend(&mut stroke, [0.52, 0.5], &frame());
        assert_eq!(stroke.points.len(), 2);
    }

    #[test]
    fn shift_strokes_join_the_selected_removal() {
        let mut view = ViewState::default();
        let mut removals = Vec::new();
        let stroke = || BrushStroke {
            points: vec![[0.3, 0.3]],
            radius: 0.02,
        };
        commit(&mut removals, &mut view, stroke(), true);
        assert_eq!((removals.len(), view.selected_removal), (1, Some(0)));
        commit(&mut removals, &mut view, stroke(), true);
        assert_eq!(removals.len(), 1);
        assert_eq!(removals[0].strokes.len(), 2);
        commit(&mut removals, &mut view, stroke(), false);
        assert_eq!((removals.len(), view.selected_removal), (2, Some(1)));
    }

    #[test]
    fn brush_radius_is_measured_on_the_long_edge() {
        // 0.1 of a 600-point-wide landscape photo is 60 points.
        let radius = screen_radius(&frame(), [0.5, 0.5], 0.1);
        assert!((radius - 60.0).abs() < 1e-3, "{radius}");
    }
}
