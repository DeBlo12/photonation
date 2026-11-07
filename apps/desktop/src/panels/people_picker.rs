// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Picking a person the model found with a click on the photo. Everyone
//! found is outlined; the one under the pointer stands out, so it is clear
//! what a click selects. What a click does is up to the tool (the Remove
//! panel removes the person).

use crate::panels::removal_brush::paint_stroke;
use crate::people::{Person, person_at};
use crate::photo_frame::PhotoFrame;
use eframe::egui::{Color32, CursorIcon, Painter, Pos2, Response, Ui};

/// Everyone found, faintly.
const FOUND: Color32 = Color32::from_rgba_premultiplied(0x40, 0x40, 0x40, 0x40);
/// The person a click would select: the accent, half opaque.
const UNDER_POINTER: Color32 = Color32::from_rgba_premultiplied(0x80, 0x59, 0x12, 0x80);

/// The person under `pointer` ([`person_at`]).
pub fn under(people: &[Person], frame: &PhotoFrame, pointer: Option<Pos2>) -> Option<usize> {
    let pointer = pointer.filter(|p| frame.image_rect.contains(*p))?;
    person_at(people, frame.to_position(pointer))
}

/// A click on a person: their index, and whether ⇧ was held (add to the
/// selection rather than start a new one).
pub fn clicked(
    people: &[Person],
    ui: &Ui,
    response: &Response,
    frame: &PhotoFrame,
) -> Option<(usize, bool)> {
    let hovered = under(people, frame, response.hover_pos());
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if !response.clicked() {
        return None;
    }
    let index = under(people, frame, response.interact_pointer_pos())?;
    Some((index, ui.input(|i| i.modifiers.shift)))
}

/// Outlines everyone found, the person under the pointer in the accent.
pub fn paint(people: &[Person], painter: &Painter, frame: &PhotoFrame, pointer: Option<Pos2>) {
    let painter = painter.with_clip_rect(frame.image_rect.intersect(painter.clip_rect()));
    let hovered = under(people, frame, pointer);
    for (index, person) in people.iter().enumerate() {
        let color = if hovered == Some(index) {
            UNDER_POINTER
        } else {
            FOUND
        };
        for stroke in &person.outline {
            paint_stroke(&painter, frame, stroke, color);
        }
    }
}
