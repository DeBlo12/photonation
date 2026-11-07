// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Picking a person the model found with a click on the photo. The panel
//! section (the people model's status, the button that starts picking, and
//! what the search found) is shared by the Remove and Masks panels. On the
//! photo everyone found is outlined, and the one under the pointer stands
//! out, so it is clear what a click selects. What a click does is up to the
//! tool: the Remove panel removes the person, the Masks panel masks them.

use crate::app::PhotonationApp;
use crate::panels::masks::hint;
use crate::panels::models::{self, ModelAction};
use crate::panels::removal_brush::paint_stroke;
use crate::panels::tool_button;
use crate::people::{Person, Search, person_at};
use crate::photo_frame::PhotoFrame;
use crate::theme;
use eframe::egui::{
    self, Align, Color32, CursorIcon, Layout, Painter, Pos2, Response, RichText, Ui,
};
use photonation_engine::ModelKind;

/// What picking people does in a tool, in its own words.
pub struct PeopleTool {
    /// What a click and a ⇧-click do.
    pub click: &'static str,
    /// The Select People button's tooltip.
    pub tooltip: &'static str,
    /// What to do while there is no people model.
    pub without_model: &'static str,
}

/// The People section of a panel: its title and the button that starts
/// picking, the people model's card, and what the search found. Starts the
/// search for the open photo while picking.
pub fn section(app: &mut PhotonationApp, ui: &mut Ui, tool: &PeopleTool) -> Option<ModelAction> {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("People")
                .size(13.0)
                .color(theme::palette().text),
        );
        let ready = app
            .models
            .status(ModelKind::People)
            .is_some_and(|status| status.is_ready());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let picking = app.view.selecting_people;
            let label = if picking { "Done" } else { "Select People" };
            if ui
                .add_enabled_ui(ready || picking, |ui| {
                    tool_button(ui, label, picking, tool.tooltip)
                })
                .inner
                .clicked()
            {
                app.view.selecting_people = !picking;
            }
        });
    });
    if app.models.status(ModelKind::People).is_none() {
        // Load it now, so the first click doesn't wait.
        app.request_model_status(ModelKind::People);
    }
    let expanded = app.view.model_details.entry(ModelKind::People).or_default();
    let action = models::status(
        ui,
        ModelKind::People,
        app.models.status(ModelKind::People),
        tool.without_model,
        expanded,
    );
    if !app.view.selecting_people {
        return action;
    }
    let Some(path) = app.current_path().cloned() else {
        return action;
    };
    if app.people.of(&path).is_none() {
        app.find_people(path.clone());
    }
    match app.people.of(&path) {
        Some(Search::Searching) | None => {
            ui.horizontal(|ui| {
                ui.add(
                    egui::Spinner::new()
                        .size(12.0)
                        .color(theme::palette().text_secondary),
                );
                hint(ui, "Finding people…");
            });
        }
        Some(Search::Found(found)) if found.is_empty() => {
            hint(ui, "No people found in this photo.");
        }
        Some(Search::Found(found)) => {
            let count = found.len();
            let people = if count == 1 { "person" } else { "people" };
            hint(ui, &format!("{count} {people} found. {}", tool.click));
        }
        Some(Search::Failed(message)) => hint(ui, &format!("Couldn't look for people: {message}")),
    }
    action
}

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
