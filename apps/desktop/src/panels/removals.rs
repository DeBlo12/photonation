// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The Remove panel. While removing it takes the place of the adjustment
//! cards: how new removals are filled and the brush size, the generative
//! model's status, selecting people with a click, the list of removals,
//! and the selected removal's settings. Painting and picking happen in the
//! viewer (`removal_brush.rs`, `people_picker.rs`). See docs/DESKTOP.md and
//! docs/RETOUCH.md.

use crate::app::{PhotonationApp, RemovalBrush};
use crate::panels::masks::hint;
use crate::panels::models::{self, ModelAction};
use crate::panels::people_picker::{self, PeopleTool};
use crate::panels::tool_button;
use crate::theme;
use crate::widgets::{self, SliderStyle, adjustment_slider};
use eframe::egui::{self, Align, Layout, RichText, Sense, Ui, vec2};
use photonation_develop::{
    MAX_BRUSH_RADIUS, MAX_REMOVAL_STROKES, MAX_REMOVALS, MIN_BRUSH_RADIUS, MaskSpace, Removal,
    RemovalMethod,
};
use photonation_engine::ModelKind;

const ROW_HEIGHT: f32 = 32.0;
/// How far a removal made from a person reaches past their outline, in
/// long-edge units: enough that no fringe of them is left for the fill to
/// copy.
const PERSON_MARGIN: f64 = 0.006;

pub fn show(app: &mut PhotonationApp, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 8.0;
    let mut actions = Vec::new();
    card(ui, |ui| {
        header(app, ui);
        let expanded = app
            .view
            .model_details
            .entry(ModelKind::Inpainting)
            .or_default();
        actions.extend(brush(
            &mut app.view.removal_brush,
            app.models.status(ModelKind::Inpainting),
            expanded,
            ui,
        ));
    });
    card(ui, |ui| {
        actions.extend(people_picker::section(
            app,
            ui,
            &PeopleTool {
                click: "Click one to remove them; ⇧-click adds them to the selected removal.",
                tooltip: "Select people with a click\nOutlines the people in the photo; click \
                          one to remove them.",
                without_model: "Paint over people with the brush instead.",
            },
        ))
    });
    for action in actions {
        app.model_action(action);
    }
    if !app.settings.removals.is_empty() {
        card(ui, |ui| removal_list(app, ui));
    }
    let Some(index) = app.view.selected_removal else {
        return;
    };
    let mut delete = false;
    if let Some(removal) = app.settings.removals.get_mut(index) {
        card(ui, |ui| delete = selected_removal(removal, ui));
    }
    if delete {
        app.settings.removals.remove(index);
        app.view.selected_removal = None;
    }
}

fn card(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        content(ui);
    });
}

fn header(app: &mut PhotonationApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Remove")
                .size(14.0)
                .color(theme::palette().text),
        );
        ui.label(
            RichText::new(format!("{} / {MAX_REMOVALS}", app.settings.removals.len()))
                .size(11.0)
                .color(theme::palette().text_tertiary),
        );
        if app.filling_removals {
            ui.add(
                egui::Spinner::new()
                    .size(12.0)
                    .color(theme::palette().text_secondary),
            );
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if tool_button(ui, "Done", false, "Back to the adjustments (Q)").clicked() {
                app.view.show_removals = false;
            }
        });
    });
    hint(
        ui,
        "Paint over what should go: a fly, a passer-by, a fence. \
         ⇧-paint adds to the selected removal.",
    );
}

fn brush(
    brush: &mut RemovalBrush,
    model: Option<&photonation_engine::ModelStatus>,
    expanded: &mut bool,
    ui: &mut Ui,
) -> Option<ModelAction> {
    let mut method = method_index(brush.method);
    widgets::segmented(ui, &method_labels(), &mut method);
    brush.method = RemovalMethod::ALL[method];
    let action = if brush.method == RemovalMethod::Generative {
        models::status(
            ui,
            ModelKind::Inpainting,
            model,
            "Content-aware fill is used instead.",
            expanded,
        )
    } else {
        None
    };
    let percent = SliderStyle {
        format: |v| format!("{v:.1} %"),
        ..SliderStyle::default()
    };
    adjustment_slider(
        ui,
        "Brush Size",
        &mut brush.size_percent,
        (MIN_BRUSH_RADIUS * 100.0) as f32..=(MAX_BRUSH_RADIUS * 100.0) as f32,
        RemovalBrush::default().size_percent,
        &percent,
    );
    action
}

/// Removes the person `index` found in the open photo: as a new removal
/// (selected), or added to the selected one. Their outline, with a margin,
/// becomes ordinary strokes, so the removal edits and saves like a painted
/// one.
pub fn remove_person(app: &mut PhotonationApp, index: usize, add_to_selected: bool) {
    let (Some(path), Some(info)) = (app.current_path(), app.info.as_ref()) else {
        return;
    };
    let space = MaskSpace::new(info.size.0 as f64, info.size.1 as f64);
    let Some(person) = app.people.found(path).get(index) else {
        return;
    };
    let mut strokes = person
        .found
        .selection
        .to_strokes(space, PERSON_MARGIN, MAX_REMOVAL_STROKES)
        .into_iter();
    let removals = &mut app.settings.removals;
    if add_to_selected
        && let Some(removal) = app
            .view
            .selected_removal
            .and_then(|selected| removals.get_mut(selected))
    {
        let room = MAX_REMOVAL_STROKES.saturating_sub(removal.strokes.len());
        removal.strokes.extend(strokes.take(room));
        return;
    }
    let Some(first) = strokes.next() else {
        return;
    };
    if removals.len() < MAX_REMOVALS {
        let mut removal = Removal::new(app.view.removal_brush.method, first);
        removal.strokes.extend(strokes);
        removals.push(removal);
        app.view.selected_removal = Some(removals.len() - 1);
    }
}

fn method_labels() -> [&'static str; 2] {
    RemovalMethod::ALL.map(RemovalMethod::label)
}

fn method_index(method: RemovalMethod) -> usize {
    RemovalMethod::ALL
        .iter()
        .position(|candidate| *candidate == method)
        .unwrap_or(0)
}

fn removal_list(app: &mut PhotonationApp, ui: &mut Ui) {
    let mut select = None;
    let mut delete = None;
    for (index, removal) in app.settings.removals.iter_mut().enumerate() {
        let selected = app.view.selected_removal == Some(index);
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
        if selected || response.hovered() {
            ui.painter().rect_filled(
                rect,
                theme::CONTROL_RADIUS,
                if selected {
                    theme::ACCENT_SOFT
                } else {
                    theme::palette().control
                },
            );
        }
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(vec2(8.0, 4.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        row.label(
            RichText::new(format!("Removal {}", index + 1))
                .size(12.5)
                .color(theme::palette().text),
        );
        row.label(
            RichText::new(summary(removal))
                .size(11.0)
                .color(theme::palette().text_tertiary),
        );
        row.with_layout(Layout::right_to_left(Align::Center), |row| {
            if tool_button(row, "×", false, "Delete this removal").clicked() {
                delete = Some(index);
            }
            widgets::toggle(row, &mut removal.enabled).on_hover_text("Show this removal");
        });
        if response.clicked() {
            select = Some(index);
        }
    }
    if let Some(index) = delete {
        app.settings.removals.remove(index);
        app.view.selected_removal = match app.view.selected_removal {
            Some(selected) if selected == index => None,
            Some(selected) if selected > index => Some(selected - 1),
            other => other,
        };
    } else if let Some(index) = select {
        app.view.selected_removal = Some(index);
    }
}

/// "Generative AI · 2 strokes".
fn summary(removal: &Removal) -> String {
    let strokes = removal.strokes.len();
    let plural = if strokes == 1 { "" } else { "s" };
    format!("{} · {strokes} stroke{plural}", removal.method.label())
}

/// The selected removal's method, edge and variation. Returns whether
/// Delete was clicked.
fn selected_removal(removal: &mut Removal, ui: &mut Ui) -> bool {
    let mut method = method_index(removal.method);
    widgets::segmented(ui, &method_labels(), &mut method);
    removal.method = RemovalMethod::ALL[method];
    adjustment_slider(
        ui,
        "Feather",
        &mut removal.feather,
        0.0..=100.0,
        Removal::default().feather,
        &SliderStyle {
            format: |v| format!("{v:.0}"),
            ..SliderStyle::default()
        },
    );
    let mut delete = false;
    ui.horizontal(|ui| {
        // Neural fills are deterministic; only content-aware varies by seed.
        let varies = removal.method == RemovalMethod::ContentAware;
        if ui
            .add_enabled_ui(varies, |ui| {
                tool_button(
                    ui,
                    "Try Another Fill",
                    false,
                    "Fill again with a different seed",
                )
            })
            .inner
            .clicked()
        {
            removal.seed = removal.seed.wrapping_add(1);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            delete = tool_button(ui, "Delete", false, "Delete this removal").clicked();
        });
    });
    delete
}
