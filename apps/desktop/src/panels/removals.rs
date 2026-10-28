// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The Remove panel. While removing it takes the place of the adjustment
//! cards: how new removals are filled and the brush size, the generative
//! model's status, the list of removals, and the selected removal's
//! settings. Painting happens in the viewer (`removal_brush.rs`). See
//! docs/DESKTOP.md and docs/RETOUCH.md.

use crate::app::{PhotonationApp, RemovalBrush};
use crate::panels::masks::hint;
use crate::panels::tool_button;
use crate::theme;
use crate::widgets::{self, SliderStyle, adjustment_slider};
use eframe::egui::{self, Align, Layout, RichText, Sense, Ui, vec2};
use photonation_develop::{
    MAX_BRUSH_RADIUS, MAX_REMOVALS, MIN_BRUSH_RADIUS, Removal, RemovalMethod,
};
use photonation_engine::{MODEL_PATH_VARIABLE, ModelStatus};

const ROW_HEIGHT: f32 = 32.0;

pub fn show(app: &mut PhotonationApp, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 8.0;
    card(ui, |ui| {
        header(app, ui);
        brush(&mut app.view.removal_brush, app.model_status.as_ref(), ui);
    });
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
        ui.label(RichText::new("Remove").size(14.0).color(theme::TEXT));
        ui.label(
            RichText::new(format!("{} / {MAX_REMOVALS}", app.settings.removals.len()))
                .size(11.0)
                .color(theme::TEXT_TERTIARY),
        );
        if app.filling_removals {
            ui.add(egui::Spinner::new().size(12.0).color(theme::TEXT_SECONDARY));
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

fn brush(brush: &mut RemovalBrush, model: Option<&ModelStatus>, ui: &mut Ui) {
    let mut method = method_index(brush.method);
    widgets::segmented(ui, &method_labels(), &mut method);
    brush.method = RemovalMethod::ALL[method];
    if brush.method == RemovalMethod::Generative {
        model_status(model, ui);
    }
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
}

/// One line on whether "Generative AI" really runs a model.
fn model_status(model: Option<&ModelStatus>, ui: &mut Ui) {
    let text = match model {
        Some(ModelStatus::Ready { name }) => format!("Model: {name}"),
        None | Some(ModelStatus::NotLoaded) => "Loading the model…".into(),
        Some(ModelStatus::NotBuilt) => {
            "This build has no AI support; content-aware fill is used.".into()
        }
        Some(ModelStatus::NotInstalled { expected }) => format!(
            "No model installed; content-aware fill is used. Put a LaMa or MI-GAN \
             ONNX file at {} or set {MODEL_PATH_VARIABLE}.",
            expected
                .as_ref()
                .map_or("the models folder".into(), |path| path
                    .display()
                    .to_string())
        ),
        Some(ModelStatus::Failed { message }) => {
            format!("The model could not be loaded ({message}); content-aware fill is used.")
        }
    };
    hint(ui, &text);
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
                    theme::CONTROL
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
                .color(theme::TEXT),
        );
        row.label(
            RichText::new(summary(removal))
                .size(11.0)
                .color(theme::TEXT_TERTIARY),
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
