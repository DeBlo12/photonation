// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! On-canvas mask editing while the masks panel is open: the selected
//! shape's lines or ellipse with draggable handles, hollow pins that select
//! the selected mask's other shapes, and solid pins that select the other
//! masks.

use crate::app::MaskSelection;
use crate::photo_frame::PhotoFrame;
use crate::theme;
use eframe::egui::{self, Painter, Pos2, Response, Shape, Stroke, Ui, Vec2};
use photonation_develop::{LinearGradient, Mask, MaskShape, RadialGradient};

const HANDLE_RADIUS: f32 = 5.5;
/// How close (points) the pointer must be to grab a handle or a pin.
const GRAB_DISTANCE: f32 = 12.0;
const PIN_RADIUS: f32 = 6.5;
/// Pins of the selected mask's other shapes are smaller rings.
const SHAPE_PIN_RADIUS: f32 = 5.0;
const ELLIPSE_SEGMENTS: usize = 72;
/// Smallest radius a drag can set, in long-edge units.
const MIN_RADIUS: f64 = 0.005;
/// Half-length of the gradient lines, in long-edge units: past any edge.
const LINE_REACH: f64 = 3.0;

/// The part of the selected mask a drag moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    LinearStart,
    LinearEnd,
    /// The middle line: moves the whole gradient.
    LinearMiddle,
    RadialCenter,
    /// An edge handle on the ellipse's (rotated) x or y axis.
    RadialEdgeX,
    RadialEdgeY,
}

/// What a pin on the canvas selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pin {
    /// Another mask (its first shape's anchor).
    Mask(usize),
    /// Another shape of the selected mask.
    Component(usize),
}

/// Handles pointer input on the canvas: drags on the selected shape's
/// handles, clicks on pins. Returns whether a handle drag owns the pointer,
/// so the viewer doesn't also pan.
pub fn interact(
    masks: &mut [Mask],
    selection: &mut Option<MaskSelection>,
    ui: &Ui,
    response: &Response,
    frame: &PhotoFrame,
) -> bool {
    let drag_id = response.id.with("mask-handle");
    let mut active: Option<Handle> = ui.ctx().data(|d| d.get_temp(drag_id));

    if response.drag_started() {
        let press = ui.input(|i| i.pointer.press_origin());
        active = selected_shape(masks, *selection)
            .zip(press)
            .and_then(|(shape, press)| handle_at(shape, frame, press));
    }
    if !response.dragged() {
        active = None;
    }
    if let (Some(handle), Some(pointer), Some(shape)) = (
        active,
        response.interact_pointer_pos(),
        selected_shape_mut(masks, *selection),
    ) {
        let keep_round = ui.input(|i| i.modifiers.shift);
        drag(
            shape,
            handle,
            pointer,
            response.drag_delta(),
            frame,
            keep_round,
        );
    }
    ui.ctx().data_mut(|d| match active {
        Some(handle) => {
            d.insert_temp(drag_id, handle);
        }
        None => d.remove::<Handle>(drag_id),
    });

    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some(pin) = pin_at(masks, *selection, frame, pointer)
    {
        *selection = Some(match (pin, *selection) {
            (Pin::Component(component), Some(current)) => MaskSelection {
                component,
                ..current
            },
            (Pin::Mask(mask), _) | (Pin::Component(mask), None) => MaskSelection::mask(mask),
        });
    }
    let hovering_handle = response.hover_pos().is_some_and(|pointer| {
        selected_shape(masks, *selection)
            .is_some_and(|shape| handle_at(shape, frame, pointer).is_some())
            || pin_at(masks, *selection, frame, pointer).is_some()
    });
    if active.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if hovering_handle {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    active.is_some()
}

fn selected_shape(masks: &[Mask], selection: Option<MaskSelection>) -> Option<&MaskShape> {
    let selection = selection?;
    let mask = masks.get(selection.mask)?;
    Some(&mask.components.get(selection.component)?.shape)
}

fn selected_shape_mut(
    masks: &mut [Mask],
    selection: Option<MaskSelection>,
) -> Option<&mut MaskShape> {
    let selection = selection?;
    let mask = masks.get_mut(selection.mask)?;
    Some(&mut mask.components.get_mut(selection.component)?.shape)
}

/// Every pin on the canvas and where it sits.
fn pins(masks: &[Mask], selection: Option<MaskSelection>, frame: &PhotoFrame) -> Vec<(Pin, Pos2)> {
    let selected_mask = selection.map(|s| s.mask);
    let mut pins: Vec<(Pin, Pos2)> = masks
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != selected_mask)
        .filter_map(|(index, mask)| {
            let first = mask.components.first()?;
            Some((Pin::Mask(index), frame.to_screen(anchor(&first.shape))))
        })
        .collect();
    if let Some(selection) = selection
        && let Some(mask) = masks.get(selection.mask)
    {
        pins.extend(
            mask.components
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != selection.component)
                .map(|(index, component)| {
                    (
                        Pin::Component(index),
                        frame.to_screen(anchor(&component.shape)),
                    )
                }),
        );
    }
    pins
}

/// Draws the selected shape's lines and handles, and the pins.
pub fn paint(
    masks: &[Mask],
    selection: Option<MaskSelection>,
    painter: &Painter,
    frame: &PhotoFrame,
) {
    let painter = painter.with_clip_rect(frame.image_rect.intersect(painter.clip_rect()));
    for (pin, position) in pins(masks, selection, frame) {
        match pin {
            Pin::Mask(index) => {
                let fill = if masks[index].enabled {
                    theme::CARD
                } else {
                    theme::CARD.gamma_multiply(0.5)
                };
                painter.circle_filled(position, PIN_RADIUS + 1.5, theme::HANDLE_SHADOW);
                painter.circle(position, PIN_RADIUS, fill, Stroke::new(1.5, theme::KNOB));
            }
            Pin::Component(_) => {
                painter.circle_stroke(
                    position,
                    SHAPE_PIN_RADIUS,
                    Stroke::new(3.5, theme::HANDLE_SHADOW),
                );
                painter.circle_stroke(position, SHAPE_PIN_RADIUS, Stroke::new(2.0, theme::ACCENT));
            }
        }
    }
    let Some(shape) = selected_shape(masks, selection) else {
        return;
    };
    match shape {
        MaskShape::Linear(linear) => paint_linear(&painter, frame, linear),
        MaskShape::Radial(radial) => paint_radial(&painter, frame, radial),
    }
    for (handle, position) in handles(shape, frame) {
        let fill = if matches!(handle, Handle::LinearMiddle | Handle::RadialCenter) {
            theme::ACCENT
        } else {
            theme::KNOB
        };
        painter.circle_filled(position, HANDLE_RADIUS + 1.5, theme::HANDLE_SHADOW);
        painter.circle_filled(position, HANDLE_RADIUS, fill);
    }
}

/// Where a mask's pin sits: the radial centre, or the gradient's middle.
fn anchor(shape: &MaskShape) -> [f64; 2] {
    match shape {
        MaskShape::Linear(linear) => midpoint(linear),
        MaskShape::Radial(radial) => radial.center,
    }
}

fn midpoint(linear: &LinearGradient) -> [f64; 2] {
    [
        (linear.start[0] + linear.end[0]) / 2.0,
        (linear.start[1] + linear.end[1]) / 2.0,
    ]
}

/// The ellipse's x and y axes as unit vectors in metric space.
fn radial_axes(radial: &RadialGradient) -> ([f64; 2], [f64; 2]) {
    let (sin, cos) = f64::from(radial.rotation).to_radians().sin_cos();
    ([cos, sin], [-sin, cos])
}

fn offset(point: [f64; 2], direction: [f64; 2], distance: f64) -> [f64; 2] {
    [
        point[0] + direction[0] * distance,
        point[1] + direction[1] * distance,
    ]
}

/// Screen positions of every handle of a shape.
fn handles(shape: &MaskShape, frame: &PhotoFrame) -> Vec<(Handle, Pos2)> {
    match shape {
        MaskShape::Linear(linear) => vec![
            (Handle::LinearStart, frame.to_screen(linear.start)),
            (Handle::LinearEnd, frame.to_screen(linear.end)),
            (Handle::LinearMiddle, frame.to_screen(midpoint(linear))),
        ],
        MaskShape::Radial(radial) => {
            let center = frame.space.to_metric(radial.center);
            let (axis_x, axis_y) = radial_axes(radial);
            let mut handles = vec![(Handle::RadialCenter, frame.metric_to_screen(center))];
            for sign in [1.0, -1.0] {
                handles.push((
                    Handle::RadialEdgeX,
                    frame.metric_to_screen(offset(center, axis_x, sign * radial.radius[0])),
                ));
                handles.push((
                    Handle::RadialEdgeY,
                    frame.metric_to_screen(offset(center, axis_y, sign * radial.radius[1])),
                ));
            }
            handles
        }
    }
}

/// The handle nearest `pointer`, if within grabbing distance.
fn handle_at(shape: &MaskShape, frame: &PhotoFrame, pointer: Pos2) -> Option<Handle> {
    handles(shape, frame)
        .into_iter()
        .map(|(handle, position)| (handle, position.distance(pointer)))
        .filter(|(_, distance)| *distance <= GRAB_DISTANCE)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(handle, _)| handle)
}

/// The pin under `pointer`, if any.
fn pin_at(
    masks: &[Mask],
    selection: Option<MaskSelection>,
    frame: &PhotoFrame,
    pointer: Pos2,
) -> Option<Pin> {
    pins(masks, selection, frame)
        .into_iter()
        .find(|(_, position)| position.distance(pointer) <= GRAB_DISTANCE)
        .map(|(pin, _)| pin)
}

/// Applies one frame of dragging `handle` to `pointer` (moved by `delta`).
fn drag(
    shape: &mut MaskShape,
    handle: Handle,
    pointer: Pos2,
    delta: Vec2,
    frame: &PhotoFrame,
    keep_round: bool,
) {
    let previous = frame.to_position(pointer - delta);
    let current = frame.to_position(pointer);
    let moved = [current[0] - previous[0], current[1] - previous[1]];
    let shift = |point: &mut [f64; 2]| {
        point[0] += moved[0];
        point[1] += moved[1];
    };
    match (shape, handle) {
        (MaskShape::Linear(linear), Handle::LinearStart) => linear.start = current,
        (MaskShape::Linear(linear), Handle::LinearEnd) => linear.end = current,
        (MaskShape::Linear(linear), Handle::LinearMiddle) => {
            shift(&mut linear.start);
            shift(&mut linear.end);
        }
        (MaskShape::Radial(radial), Handle::RadialCenter) => shift(&mut radial.center),
        (MaskShape::Radial(radial), edge @ (Handle::RadialEdgeX | Handle::RadialEdgeY)) => {
            let center = frame.space.to_metric(radial.center);
            let point = frame.screen_to_metric(pointer);
            let from_center = [point[0] - center[0], point[1] - center[1]];
            let (axis_x, axis_y) = radial_axes(radial);
            let (axis, which) = if edge == Handle::RadialEdgeX {
                (axis_x, 0)
            } else {
                (axis_y, 1)
            };
            let radius = (from_center[0] * axis[0] + from_center[1] * axis[1])
                .abs()
                .max(MIN_RADIUS);
            if keep_round {
                radial.radius = [radius; 2];
            } else {
                radial.radius[which] = radius;
            }
        }
        _ => {} // a handle of the other shape kind: the mask changed mid-drag
    }
}

/// A line drawn twice (shadow, then white) so it reads on any photo.
fn outlined(painter: &Painter, points: Vec<Pos2>, width: f32, closed: bool, dashed: bool) {
    for (color, stroke_width) in [(theme::HANDLE_SHADOW, width + 2.0), (theme::KNOB, width)] {
        let stroke = Stroke::new(stroke_width, color);
        if dashed {
            painter.extend(Shape::dashed_line(&points, stroke, 6.0, 5.0));
        } else if closed {
            painter.add(Shape::closed_line(points.clone(), stroke));
        } else {
            painter.add(Shape::line(points.clone(), stroke));
        }
    }
}

fn paint_linear(painter: &Painter, frame: &PhotoFrame, linear: &LinearGradient) {
    let start = frame.space.to_metric(linear.start);
    let end = frame.space.to_metric(linear.end);
    let direction = [end[0] - start[0], end[1] - start[1]];
    let length = direction[0].hypot(direction[1]).max(f64::EPSILON);
    let across = [-direction[1] / length, direction[0] / length];
    let line_through = |point: [f64; 2]| {
        vec![
            frame.metric_to_screen(offset(point, across, -LINE_REACH)),
            frame.metric_to_screen(offset(point, across, LINE_REACH)),
        ]
    };
    let middle = frame.space.to_metric(midpoint(linear));
    outlined(painter, line_through(start), 1.5, false, false);
    outlined(painter, line_through(middle), 1.0, false, true);
    outlined(painter, line_through(end), 1.5, false, false);
}

fn paint_radial(painter: &Painter, frame: &PhotoFrame, radial: &RadialGradient) {
    let center = frame.space.to_metric(radial.center);
    let (axis_x, axis_y) = radial_axes(radial);
    let ellipse = |scale: f64| -> Vec<Pos2> {
        (0..ELLIPSE_SEGMENTS)
            .map(|step| {
                let angle = std::f64::consts::TAU * step as f64 / ELLIPSE_SEGMENTS as f64;
                let x = angle.cos() * radial.radius[0] * scale;
                let y = angle.sin() * radial.radius[1] * scale;
                frame.metric_to_screen(offset(offset(center, axis_x, x), axis_y, y))
            })
            .collect()
    };
    outlined(painter, ellipse(1.0), 1.5, true, false);
    let fade_start = 1.0 - f64::from(radial.feather) / 100.0;
    if fade_start > 0.02 && fade_start < 0.98 {
        let mut inner = ellipse(fade_start);
        inner.push(inner[0]); // close the dashed outline
        outlined(painter, inner, 1.0, true, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Rect, pos2, vec2};
    use photonation_develop::{CombineMode, MaskComponent, MaskSpace};
    use photonation_image::NormRect;

    fn frame() -> PhotoFrame {
        PhotoFrame {
            image_rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0)),
            crop: NormRect::FULL,
            space: MaskSpace::new(6000.0, 4000.0),
        }
    }

    fn circle() -> MaskShape {
        MaskShape::Radial(RadialGradient {
            center: [0.5, 0.5],
            radius: [0.1, 0.1],
            rotation: 0.0,
            feather: 50.0,
        })
    }

    #[test]
    fn handles_sit_on_the_shape() {
        let positions = handles(&circle(), &frame());
        assert_eq!(positions[0], (Handle::RadialCenter, pos2(300.0, 200.0)));
        // 0.1 long edges = 60 points on a 600-point-wide image.
        assert_eq!(positions[1], (Handle::RadialEdgeX, pos2(360.0, 200.0)));
        assert_eq!(positions[2], (Handle::RadialEdgeY, pos2(300.0, 260.0)));
        assert_eq!(
            handle_at(&circle(), &frame(), pos2(357.0, 203.0)),
            Some(Handle::RadialEdgeX)
        );
        assert_eq!(handle_at(&circle(), &frame(), pos2(330.0, 230.0)), None);
    }

    #[test]
    fn dragging_an_edge_resizes_along_its_axis() {
        let mut shape = circle();
        drag(
            &mut shape,
            Handle::RadialEdgeX,
            pos2(420.0, 230.0),
            vec2(60.0, 30.0),
            &frame(),
            false,
        );
        let MaskShape::Radial(radial) = shape else {
            unreachable!()
        };
        assert!((radial.radius[0] - 0.2).abs() < 1e-6, "{:?}", radial.radius);
        assert_eq!(radial.radius[1], 0.1);

        let mut round = circle();
        drag(
            &mut round,
            Handle::RadialEdgeY,
            pos2(300.0, 290.0),
            vec2(0.0, 30.0),
            &frame(),
            true,
        );
        let MaskShape::Radial(radial) = round else {
            unreachable!()
        };
        assert!((radial.radius[0] - 0.15).abs() < 1e-6 && radial.radius[0] == radial.radius[1]);
    }

    #[test]
    fn dragging_the_middle_moves_the_whole_gradient() {
        let mut shape = MaskShape::Linear(LinearGradient {
            start: [0.5, 0.2],
            end: [0.5, 0.6],
        });
        drag(
            &mut shape,
            Handle::LinearMiddle,
            pos2(360.0, 200.0),
            vec2(60.0, 40.0),
            &frame(),
            false,
        );
        let MaskShape::Linear(linear) = shape else {
            unreachable!()
        };
        assert!((linear.start[0] - 0.6).abs() < 1e-6 && (linear.start[1] - 0.3).abs() < 1e-6);
        assert!((linear.end[0] - 0.6).abs() < 1e-6 && (linear.end[1] - 0.7).abs() < 1e-6);
    }

    #[test]
    fn pins_select_other_masks_and_the_selected_masks_other_shapes() {
        let linear = MaskShape::Linear(LinearGradient::default());
        let mut a = Mask::new("A", circle());
        a.components
            .push(MaskComponent::new(linear, CombineMode::Subtract));
        let masks = vec![a, Mask::new("B", linear)];
        let linear_pin = frame().to_screen(midpoint(&LinearGradient::default()));
        let circle_pin = pos2(300.0, 200.0);
        let on_a = Some(MaskSelection::mask(0));
        assert_eq!(
            pin_at(&masks, on_a, &frame(), circle_pin),
            None,
            "the selected shape has handles, not a pin"
        );
        // B's pin and A's second shape sit at the same place; B comes first.
        assert_eq!(
            pin_at(&masks, on_a, &frame(), linear_pin),
            Some(Pin::Mask(1))
        );
        let on_b = Some(MaskSelection::mask(1));
        assert_eq!(
            pin_at(&masks, on_b, &frame(), circle_pin),
            Some(Pin::Mask(0))
        );
        let on_a_linear = Some(MaskSelection {
            mask: 0,
            component: 1,
        });
        let masks = vec![masks[0].clone()];
        assert_eq!(
            pin_at(&masks, on_a_linear, &frame(), circle_pin),
            Some(Pin::Component(0))
        );
    }
}
