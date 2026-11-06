// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Painting a mask's brush on the canvas, while it is the selected shape. A
//! drag paints a stroke and a click dabs; Alt does the opposite of the panel's
//! Paint or Erase. The stroke being painted renders with the mask
//! (`with_stroke_in_progress`), so the adjustments follow it, and joins the
//! settings on release, as one undo step. See docs/DESKTOP.md.

use crate::app::{MaskBrushTool, PhotonationApp, ViewState};
use crate::panels::mask_handles::{select_pin, selected_shape, selected_shape_mut};
use crate::panels::removal_brush::{extend, paint_stroke, screen_radius};
use crate::photo_frame::PhotoFrame;
use crate::theme;
use eframe::egui::{self, Color32, Key, Modifiers, Painter, Pos2, Response, Stroke, Ui, vec2};
use photonation_develop::{
    BrushStroke, MAX_BRUSH_RADIUS, MAX_STROKE_POINTS, MIN_BRUSH_RADIUS, MaskBrush, MaskShape,
    MaskStroke, Settings,
};

/// The stroke being erased with, where painting shows in the overlay's red.
const ERASING: Color32 = Color32::from_rgba_premultiplied(0x50, 0x50, 0x58, 0x50);
/// Each press of [ or ] sizes the brush by this factor.
const SIZE_STEP: f32 = 1.2;
/// Half the length of the plus or minus in the brush's outline, in points.
const SIGN_REACH: f32 = 4.0;

/// Whether the selected shape is a brush, so the canvas paints.
pub fn in_hand(app: &PhotonationApp) -> bool {
    matches!(
        selected_shape(&app.settings.masks, app.view.selected_mask),
        Some(MaskShape::Brush(_))
    )
}

/// `settings` with the stroke being painted on the selected mask.
pub fn with_stroke_in_progress(settings: &mut Settings, view: &ViewState) {
    if let Some(stroke) = &view.mask_stroke
        && let Some(MaskShape::Brush(brush)) =
            selected_shape_mut(&mut settings.masks, view.selected_mask)
    {
        brush.strokes.push(stroke.clone());
    }
}

/// [ and ] size the brush while one is in hand.
pub fn size_shortcuts(app: &mut PhotonationApp, ctx: &egui::Context) {
    if !(app.view.show_masks && in_hand(app)) {
        return;
    }
    let (smaller, larger) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::OpenBracket),
            i.consume_key(Modifiers::NONE, Key::CloseBracket),
        )
    });
    let tool = &mut app.view.mask_brush;
    if smaller {
        tool.size_percent = stepped_size(tool.size_percent / SIZE_STEP);
    }
    if larger {
        tool.size_percent = stepped_size(tool.size_percent * SIZE_STEP);
    }
}

fn stepped_size(percent: f32) -> f32 {
    percent.clamp(
        (MIN_BRUSH_RADIUS * 100.0) as f32,
        (MAX_BRUSH_RADIUS * 100.0) as f32,
    )
}

/// Handles painting. Returns whether a stroke owns the pointer, so the viewer
/// doesn't also pan.
pub fn interact(
    app: &mut PhotonationApp,
    ui: &Ui,
    response: &Response,
    frame: &PhotoFrame,
) -> bool {
    let view = &mut app.view;
    let tool = view.mask_brush;
    let opposite = ui.input(|i| i.modifiers.alt);
    if response.drag_started()
        && let Some(press) = ui.input(|i| i.pointer.press_origin())
        && frame.image_rect.contains(press)
    {
        view.mask_stroke = Some(new_stroke(tool, opposite, press, frame));
    }
    if response.dragged() {
        if let Some(pointer) = response.interact_pointer_pos()
            && view.mask_stroke.is_some()
        {
            continue_stroke(&mut app.settings.masks, view, pointer, frame);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            return true;
        }
        return false;
    }
    if let Some(stroke) = view.mask_stroke.take() {
        commit(&mut app.settings.masks, view, stroke);
    } else if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
        && frame.image_rect.contains(pointer)
        && !select_pin(&app.settings.masks, &mut view.selected_mask, frame, pointer)
    {
        let dab = new_stroke(tool, opposite, pointer, frame);
        commit(&mut app.settings.masks, view, dab);
    }
    if response
        .hover_pos()
        .is_some_and(|p| frame.image_rect.contains(p))
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    false
}

fn new_stroke(
    tool: MaskBrushTool,
    opposite: bool,
    pointer: Pos2,
    frame: &PhotoFrame,
) -> MaskStroke {
    MaskStroke {
        path: BrushStroke {
            points: vec![rounded(frame.to_position(pointer))],
            radius: tool.radius(),
        },
        feather: tool.feather,
        flow: tool.flow,
        erase: tool.erase != opposite,
    }
}

/// Five decimals place a point within a tenth of a pixel on any camera's
/// photo, and sidecars with thousands of points stay small.
fn rounded(position: [f64; 2]) -> [f64; 2] {
    position.map(|v| (v * 1e5).round() / 1e5)
}

/// Extends the stroke. One that reached the point limit joins the settings
/// and goes on as a new stroke, so a long drag with a tiny brush never stops.
fn continue_stroke(
    masks: &mut [photonation_develop::Mask],
    view: &mut ViewState,
    pointer: Pos2,
    frame: &PhotoFrame,
) {
    let Some(stroke) = &mut view.mask_stroke else {
        return;
    };
    let position = rounded(frame.to_position(pointer));
    if stroke.path.points.len() >= MAX_STROKE_POINTS {
        let mut next = stroke.clone();
        next.path.points = next.path.points.split_off(MAX_STROKE_POINTS - 1);
        let full = std::mem::replace(stroke, next);
        commit(masks, view, full);
    }
    if let Some(stroke) = &mut view.mask_stroke {
        extend(&mut stroke.path, position, frame);
    }
}

/// Adds a finished stroke to the selected brush, if it still fits.
fn commit(masks: &mut [photonation_develop::Mask], view: &ViewState, stroke: MaskStroke) {
    if let Some(MaskShape::Brush(brush)) = selected_shape_mut(masks, view.selected_mask) {
        push_stroke(brush, stroke);
    }
}

fn push_stroke(brush: &mut MaskBrush, stroke: MaskStroke) {
    if brush.has_room_for(&stroke) {
        brush.strokes.push(stroke);
    }
}

/// Draws the stroke being painted, where the overlay can't show it yet, and
/// the brush's outline under the pointer.
pub fn paint(
    view: &ViewState,
    ui: &Ui,
    response: &Response,
    painter: &Painter,
    frame: &PhotoFrame,
) {
    let painter = painter.with_clip_rect(frame.image_rect.intersect(painter.clip_rect()));
    let tool = view.mask_brush;
    if view.mask_overlay
        && let Some(stroke) = &view.mask_stroke
    {
        let color = if stroke.erase {
            ERASING
        } else {
            theme::MASK_OVERLAY
        };
        paint_stroke(&painter, frame, &stroke.path, color);
    }
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let Some(pointer) = pointer.filter(|p| frame.image_rect.contains(*p)) else {
        return;
    };
    let (radius, erase) = match &view.mask_stroke {
        Some(stroke) => (stroke.path.radius, stroke.erase),
        None => (tool.radius(), tool.erase != ui.input(|i| i.modifiers.alt)),
    };
    let outer = screen_radius(frame, frame.to_position(pointer), radius);
    painter.circle_stroke(pointer, outer, Stroke::new(2.5, theme::HANDLE_SHADOW));
    painter.circle_stroke(pointer, outer, Stroke::new(1.2, theme::KNOB));
    // Where the feather starts, while there's room to tell the rings apart.
    let inner = outer * (1.0 - tool.feather / 100.0);
    if outer - inner > 3.0 && inner > 3.0 {
        painter.circle_stroke(pointer, inner, Stroke::new(1.0, theme::HANDLE_SHADOW));
    }
    // Plus or minus: what this stroke does.
    let line = |from: Pos2, to: Pos2| {
        painter.line_segment([from, to], Stroke::new(2.5, theme::HANDLE_SHADOW));
        painter.line_segment([from, to], Stroke::new(1.2, theme::KNOB));
    };
    line(
        pointer - vec2(SIGN_REACH, 0.0),
        pointer + vec2(SIGN_REACH, 0.0),
    );
    if !erase {
        line(
            pointer - vec2(0.0, SIGN_REACH),
            pointer + vec2(0.0, SIGN_REACH),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::MaskSelection;
    use eframe::egui::{Rect, pos2};
    use photonation_develop::{Mask, MaskSpace};
    use photonation_image::CropFrame;

    fn frame() -> PhotoFrame {
        PhotoFrame {
            image_rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0)),
            crop: CropFrame::full([6000.0, 4000.0]),
            space: MaskSpace::new(6000.0, 4000.0),
            lens: None,
        }
    }

    fn brush_mask() -> Mask {
        Mask::new("Painted", MaskShape::Brush(MaskBrush::default()))
    }

    fn strokes_of(mask: &Mask) -> &[MaskStroke] {
        match &mask.components[0].shape {
            MaskShape::Brush(brush) => &brush.strokes,
            other => panic!("not a brush: {other:?}"),
        }
    }

    #[test]
    fn strokes_take_the_tool_and_alt_flips_the_mode() {
        let tool = MaskBrushTool {
            size_percent: 5.0,
            feather: 20.0,
            flow: 30.0,
            erase: false,
        };
        let stroke = new_stroke(tool, false, pos2(300.0, 200.0), &frame());
        assert_eq!(stroke.path.points, vec![[0.5, 0.5]]);
        assert_eq!(stroke.path.radius, 0.05);
        assert_eq!(
            (stroke.feather, stroke.flow, stroke.erase),
            (20.0, 30.0, false)
        );
        assert!(new_stroke(tool, true, pos2(0.0, 0.0), &frame()).erase);
        let eraser = MaskBrushTool {
            erase: true,
            ..tool
        };
        assert!(new_stroke(eraser, false, pos2(0.0, 0.0), &frame()).erase);
        assert!(!new_stroke(eraser, true, pos2(0.0, 0.0), &frame()).erase);
    }

    #[test]
    fn positions_are_rounded_for_the_sidecar() {
        assert_eq!(rounded([0.123_456_789, 0.5]), [0.123_46, 0.5]);
    }

    #[test]
    fn a_stroke_at_the_point_limit_goes_on_as_a_new_one() {
        let mut masks = vec![brush_mask()];
        let mut view = ViewState {
            selected_mask: Some(MaskSelection::mask(0)),
            ..ViewState::default()
        };
        let mut stroke = new_stroke(view.mask_brush, false, pos2(0.0, 200.0), &frame());
        stroke.path.points = vec![[0.1, 0.5]; MAX_STROKE_POINTS];
        view.mask_stroke = Some(stroke);
        continue_stroke(&mut masks, &mut view, pos2(300.0, 200.0), &frame());
        assert_eq!(strokes_of(&masks[0]).len(), 1, "the full stroke joined");
        let going_on = view.mask_stroke.expect("still painting");
        assert_eq!(
            going_on.path.points,
            vec![[0.1, 0.5], [0.5, 0.5]],
            "connected to where the last one stopped"
        );
    }

    #[test]
    fn the_stroke_in_progress_renders_on_the_selected_brush_only() {
        let gradient = Mask::new("Gradient", MaskShape::default());
        let mut settings = Settings {
            masks: vec![gradient, brush_mask()],
            ..Settings::default()
        };
        let mut view = ViewState {
            selected_mask: Some(MaskSelection::mask(1)),
            ..ViewState::default()
        };
        with_stroke_in_progress(&mut settings, &view);
        assert!(strokes_of(&settings.masks[1]).is_empty(), "not painting");
        view.mask_stroke = Some(new_stroke(view.mask_brush, false, pos2(1.0, 1.0), &frame()));
        with_stroke_in_progress(&mut settings, &view);
        assert_eq!(strokes_of(&settings.masks[1]).len(), 1);
        view.selected_mask = Some(MaskSelection::mask(0));
        with_stroke_in_progress(&mut settings, &view);
        assert_eq!(strokes_of(&settings.masks[1]).len(), 1, "a gradient is out");
    }

    #[test]
    fn a_full_brush_turns_strokes_away() {
        let mut brush = MaskBrush {
            strokes: vec![MaskStroke::default(); photonation_develop::MAX_MASK_BRUSH_STROKES],
        };
        push_stroke(&mut brush, MaskStroke::default());
        assert_eq!(
            brush.strokes.len(),
            photonation_develop::MAX_MASK_BRUSH_STROKES
        );
    }

    #[test]
    fn size_steps_stay_in_range() {
        assert_eq!(stepped_size(1e9), (MAX_BRUSH_RADIUS * 100.0) as f32);
        assert_eq!(stepped_size(0.0), (MIN_BRUSH_RADIUS * 100.0) as f32);
    }
}
