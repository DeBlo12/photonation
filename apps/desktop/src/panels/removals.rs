// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The Remove panel. While removing it takes the place of the adjustment
//! cards: how new removals are filled and the brush size, the generative
//! model's status, the list of removals, and the selected removal's
//! settings. Painting happens in the viewer (`removal_brush.rs`). See
//! docs/DESKTOP.md and docs/RETOUCH.md.

use crate::app::{PhotonationApp, RemovalBrush};
use crate::panels::masks::hint;
use crate::panels::{hover_tip, tool_button};
use crate::theme;
use crate::widgets::{self, SliderStyle, adjustment_slider};
use eframe::egui::{self, Align, Layout, RichText, Sense, Ui, vec2};
use photonation_develop::{
    MAX_BRUSH_RADIUS, MAX_REMOVALS, MIN_BRUSH_RADIUS, Removal, RemovalMethod,
};
use photonation_engine::{MODEL_PATH_VARIABLE, ModelStatus};
use std::path::Path;

const ROW_HEIGHT: f32 = 32.0;
/// The LaMa export docs/RETOUCH.md checked, and its SHA-256 there.
const LAMA_DOWNLOAD: &str = "https://huggingface.co/Carve/LaMa-ONNX/resolve/main/lama_fp32.onnx";
const LAMA_SHA256: &str = "1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6";

/// What the model status asked for.
enum ModelAction {
    CheckAgain,
    Failed(String),
}

pub fn show(app: &mut PhotonationApp, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 8.0;
    card(ui, |ui| {
        header(app, ui);
        match brush(&mut app.view.removal_brush, app.model_status.as_ref(), ui) {
            Some(ModelAction::CheckAgain) => {
                app.model_status = Some(ModelStatus::NotLoaded);
                app.engine.request_model_status();
            }
            Some(ModelAction::Failed(message)) => app.notify(message),
            None => {}
        }
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

fn brush(
    brush: &mut RemovalBrush,
    model: Option<&ModelStatus>,
    ui: &mut Ui,
) -> Option<ModelAction> {
    let mut method = method_index(brush.method);
    widgets::segmented(ui, &method_labels(), &mut method);
    brush.method = RemovalMethod::ALL[method];
    let action = if brush.method == RemovalMethod::Generative {
        model_status(model, ui)
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

/// One line on whether "Generative AI" really runs a model, and without
/// one, where to get it.
fn model_status(model: Option<&ModelStatus>, ui: &mut Ui) -> Option<ModelAction> {
    let text = match model {
        Some(ModelStatus::Ready { name }) => format!("Model: {name}"),
        None | Some(ModelStatus::NotLoaded) => "Loading the model…".into(),
        Some(ModelStatus::NotBuilt) => {
            "This build has no AI support; content-aware fill is used.".into()
        }
        Some(ModelStatus::NotInstalled { expected }) => {
            return model_download(expected.as_deref(), ui);
        }
        Some(ModelStatus::Failed { message }) => {
            format!("The model could not be loaded ({message}); content-aware fill is used.")
        }
    };
    hint(ui, &text);
    None
}

/// Where to download the LaMa model, how to check it and where it goes.
fn model_download(expected: Option<&Path>, ui: &mut Ui) -> Option<ModelAction> {
    hint(
        ui,
        "No model installed, so content-aware fill is used. Generative AI needs \
         an inpainting model, which Photonation doesn't ship:",
    );
    let download = ui.hyperlink_to("Download LaMa (lama_fp32.onnx)", LAMA_DOWNLOAD);
    hover_tip(
        download,
        "LaMa inpainting model, Apache-2.0\nFrom Hugging Face (Carve/LaMa-ONNX); the \
         download is large. Photonation runs it on this computer only.",
    );
    ui.horizontal_wrapped(|ui| {
        hint(ui, &format!("SHA-256 {}…", &LAMA_SHA256[..12]));
        let copy = ui.small_button("Copy");
        if hover_tip(
            copy,
            "Copy the model's checksum\nCompare it before using the file, e.g. in \
             PowerShell: Get-FileHash lama_fp32.onnx (shasum -a 256 on a Mac).",
        )
        .clicked()
        {
            ui.ctx().copy_text(LAMA_SHA256.into());
        }
    });
    let place = match expected {
        Some(_) if std::env::var_os(MODEL_PATH_VARIABLE).is_none() => {
            "Put the file in the models folder, then click Check Again.".to_string()
        }
        Some(path) => format!("Put it at {} ({MODEL_PATH_VARIABLE}).", path.display()),
        None => format!("Set {MODEL_PATH_VARIABLE} to the model file."),
    };
    hint(ui, &place);
    let mut action = None;
    ui.horizontal(|ui| {
        if let Some(folder) = expected.and_then(Path::parent) {
            let open = ui.button("Open Models Folder");
            if hover_tip(open, &folder.display().to_string()).clicked()
                && let Err(error) = open_folder(folder)
            {
                action = Some(ModelAction::Failed(format!(
                    "Could not open {}: {error}",
                    folder.display()
                )));
            }
        }
        if ui.button("Check Again").clicked() {
            action = Some(ModelAction::CheckAgain);
        }
    });
    action
}

/// Shows `folder` in the system's file manager, creating it first.
fn open_folder(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(folder)
        .spawn()
        .map(drop)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_download_is_the_model_the_docs_checked() {
        let retouch = include_str!("../../../../docs/RETOUCH.md");
        assert_eq!(LAMA_SHA256.len(), 64);
        assert!(
            retouch.contains(LAMA_SHA256),
            "RETOUCH.md lists its SHA-256"
        );
        assert!(LAMA_DOWNLOAD.ends_with("/lama_fp32.onnx"));
        assert!(retouch.contains("huggingface.co/Carve/LaMa-ONNX"));
    }
}
