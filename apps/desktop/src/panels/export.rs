// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Export sheet: format, size, colour, file name and metadata options,
//! then a folder picker (or the folder chosen in Settings). The option
//! controls are shared with the Settings window's export defaults.

use crate::app::PhotonationApp;
use crate::theme;
use crate::widgets::{self, SliderStyle, adjustment_slider};
use eframe::egui::{self, RichText};
use photonation_color::ColorSpace;
use photonation_engine::{
    ExportOptions, MetadataPreset, NAME_TOKEN, NUMBER_TOKEN, export_file_stem,
};
use photonation_export::Format;
use photonation_image::BitDepth;

const FORMATS: [&str; 3] = ["JPEG", "PNG", "TIFF"];
const SIZES: [&str; 2] = ["Original", "Long Edge"];
const DEFAULT_LONG_EDGE: usize = 2048;

pub fn show(app: &mut PhotonationApp, ctx: &egui::Context) {
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("export-sheet")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(
            RichText::new("Export")
                .font(theme::title(17.0))
                .color(theme::TEXT),
        );
        ui.add_space(10.0);
        format_controls(ui, &mut app.view.export_options);

        ui.add_space(10.0);
        let (batch, kind) = app.export_batch();
        if batch.len() > 1 {
            ui.checkbox(&mut app.view.export_all, kind.sheet_label(batch.len()));
        }

        ui.add_space(6.0);
        label(ui, "File Name");
        let batch = if app.view.export_all && batch.len() > 1 {
            batch
        } else {
            app.current.into_iter().collect()
        };
        file_name_field(ui, &mut app.view.export_options);
        file_name_preview(ui, app, &batch);
        ui.add_space(10.0);
        label(ui, "Metadata");
        metadata(ui, &mut app.view.export_options.metadata);
        ui.add_space(12.0);
        let fixed_folder = app.preferences.fixed_export_folder().cloned();
        if let Some(folder) = &fixed_folder {
            ui.label(
                RichText::new(format!("To {} (change in Settings)", folder.display()))
                    .size(11.0)
                    .color(theme::TEXT_TERTIARY),
            );
            ui.add_space(4.0);
        }
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let title = if fixed_folder.is_some() {
                    "Export"
                } else {
                    "Export…"
                };
                let export = theme::primary_button(title).min_size(egui::vec2(84.0, 28.0));
                if ui.add(export).clicked() {
                    start_export(app);
                    close = true;
                }
                if ui
                    .add(egui::Button::new("Cancel").min_size(egui::vec2(72.0, 28.0)))
                    .clicked()
                {
                    close = true;
                }
            });
        });
    });
    if close || modal.should_close() {
        app.view.show_export = false;
    }
}

/// Format with its quality or bit depth, size and colour space.
pub fn format_controls(ui: &mut egui::Ui, options: &mut ExportOptions) {
    let mut format_index = match options.format {
        Format::Jpeg { .. } => 0,
        Format::Png => 1,
        Format::Tiff => 2,
    };
    label(ui, "Format");
    if widgets::segmented(ui, &FORMATS, &mut format_index).changed() {
        options.format = match format_index {
            0 => Format::Jpeg { quality: 92 },
            1 => Format::Png,
            _ => Format::Tiff,
        };
        options.bit_depth = if format_index == 2 {
            BitDepth::Sixteen
        } else {
            BitDepth::Eight
        };
    }
    ui.add_space(6.0);
    match &mut options.format {
        Format::Jpeg { quality } => {
            let mut value = *quality as f32;
            let style = SliderStyle {
                format: |v| format!("{v:.0}"),
                ..SliderStyle::default()
            };
            adjustment_slider(ui, "Quality", &mut value, 50.0..=100.0, 92.0, &style);
            *quality = value.round() as u8;
        }
        Format::Png | Format::Tiff => {
            let mut sixteen = options.bit_depth == BitDepth::Sixteen;
            ui.checkbox(&mut sixteen, "16 bits per channel");
            options.bit_depth = if sixteen {
                BitDepth::Sixteen
            } else {
                BitDepth::Eight
            };
        }
    }

    ui.add_space(6.0);
    label(ui, "Size");
    let mut size_index = usize::from(options.long_edge.is_some());
    if widgets::segmented(ui, &SIZES, &mut size_index).changed() {
        options.long_edge = (size_index == 1).then_some(DEFAULT_LONG_EDGE);
    }
    if let Some(long_edge) = &mut options.long_edge {
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(long_edge)
                    .range(256..=20000)
                    .speed(16)
                    .suffix(" px"),
            );
            ui.label(RichText::new("long edge").color(theme::TEXT_SECONDARY));
        });
    }

    ui.add_space(6.0);
    label(ui, "Color Space");
    egui::ComboBox::from_id_salt("export-color-space")
        .selected_text(options.color_space.name())
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for space in ColorSpace::ALL {
                ui.selectable_value(&mut options.color_space, space, space.name());
            }
        });
}

/// The name field and what its tokens mean.
pub fn file_name_field(ui: &mut egui::Ui, options: &mut ExportOptions) {
    ui.add(
        egui::TextEdit::singleline(&mut options.file_name)
            .hint_text(NAME_TOKEN)
            .desired_width(f32::INFINITY),
    );
    ui.label(
        RichText::new(format!(
            "{NAME_TOKEN} original name · {NUMBER_TOKEN} number in the batch"
        ))
        .size(11.0)
        .color(theme::TEXT_TERTIARY),
    );
}

/// The name the current photo will get (before the extension; existing
/// files are never replaced, so a clash adds `-2`, `-3`, …).
/// The current photo's file name, numbered by its place in `batch`.
fn file_name_preview(ui: &mut egui::Ui, app: &PhotonationApp, batch: &[usize]) {
    let options = &app.view.export_options;
    if let Some(index) = app.current {
        let number = batch.iter().position(|&i| i == index).map_or(1, |p| p + 1);
        let total = batch.len().max(1);
        let stem = export_file_stem(&options.file_name, &app.photos[index].path, number, total);
        ui.label(
            RichText::new(format!("{stem}.{}", options.format.extension()))
                .size(12.0)
                .color(theme::TEXT_SECONDARY),
        );
    }
}

/// The preset shortcuts, a checkbox per group, and the creator and
/// copyright fields (remembered with the groups, see `preferences.rs`).
pub fn metadata(ui: &mut egui::Ui, options: &mut photonation_engine::MetadataOptions) {
    let names = MetadataPreset::ALL.map(MetadataPreset::name);
    // A custom choice of groups highlights no preset.
    let mut preset_index = options
        .preset()
        .and_then(|preset| MetadataPreset::ALL.iter().position(|&p| p == preset))
        .unwrap_or(names.len());
    if widgets::segmented(ui, &names, &mut preset_index).changed() {
        options.apply(MetadataPreset::ALL[preset_index]);
    }
    ui.add_space(4.0);
    ui.checkbox(&mut options.camera, "Camera and capture")
        .on_hover_text("Make, model, lens, exposure and capture time");
    ui.checkbox(&mut options.copyright, "Copyright and creator");
    if options.copyright {
        ui.indent("export-creator", |ui| {
            for (text, hint) in [
                (&mut options.creator, "Creator (else the camera's)"),
                (
                    &mut options.copyright_notice,
                    "Copyright (else the camera's)",
                ),
            ] {
                ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY),
                );
            }
        });
    }
    ui.checkbox(&mut options.location, "Location (GPS)")
        .on_hover_text("Where the photo was taken; left out unless ticked");
}

pub fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(theme::TEXT_SECONDARY));
}

/// Exports to the folder set in Settings, or asks for one (starting in
/// that folder).
fn start_export(app: &mut PhotonationApp) {
    let directory = match app.preferences.fixed_export_folder() {
        Some(folder) => folder.clone(),
        None => {
            let mut dialog = rfd::FileDialog::new().set_title("Export to");
            if let Some(folder) = &app.preferences.export_folder {
                dialog = dialog.set_directory(folder);
            }
            let Some(directory) = dialog.pick_folder() else {
                return;
            };
            directory
        }
    };
    app.save_now();
    app.preferences.export_metadata = app.view.export_options.metadata.clone();
    app.preferences.save();
    let (batch, _) = app.export_batch();
    let photos = if app.view.export_all && batch.len() > 1 {
        // Sidecars hold every photo's settings (the current one was just saved).
        batch
            .iter()
            .map(|&i| (app.photos[i].path.clone(), None))
            .collect()
    } else {
        app.current_path()
            .map(|p| vec![(p.clone(), Some(app.effective_settings()))])
            .unwrap_or_default()
    };
    app.engine
        .export(photos, directory, app.view.export_options.clone());
}
