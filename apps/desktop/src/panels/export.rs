// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Export sheet: format, size, colour and file name options, then a
//! folder picker.

use crate::app::PhotonationApp;
use crate::theme;
use crate::widgets::{self, SliderStyle, adjustment_slider};
use eframe::egui::{self, RichText};
use photonation_color::ColorSpace;
use photonation_engine::{NAME_TOKEN, NUMBER_TOKEN, export_file_stem};
use photonation_export::Format;
use photonation_image::BitDepth;

const FORMATS: [&str; 3] = ["JPEG", "PNG", "TIFF"];
const SIZES: [&str; 2] = ["Original", "Long Edge"];
const DEFAULT_LONG_EDGE: usize = 2048;

pub fn show(app: &mut PhotonationApp, ctx: &egui::Context) {
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("export-sheet")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(RichText::new("Export").size(17.0).color(theme::TEXT));
        ui.add_space(10.0);
        let options = &mut app.view.export_options;

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

        ui.add_space(10.0);
        let photo_count = app.photos.len();
        if photo_count > 1 {
            ui.checkbox(
                &mut app.view.export_all,
                format!("Export all {photo_count} photos in this folder"),
            );
        }

        ui.add_space(6.0);
        label(ui, "File Name");
        let total = if app.view.export_all { photo_count } else { 1 };
        file_name(ui, app, total);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let export = egui::Button::new(RichText::new("Export…").color(theme::TEXT))
                    .fill(theme::ACCENT)
                    .min_size(egui::vec2(84.0, 28.0));
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

/// The name field, what the tokens mean, and the name the current photo
/// will get (before the extension; existing files are never replaced, so
/// a clash adds `-2`, `-3`, …).
fn file_name(ui: &mut egui::Ui, app: &mut PhotonationApp, total: usize) {
    let options = &mut app.view.export_options;
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
    if let Some(index) = app.current {
        // The current photo's place in the batch (it is alone otherwise).
        let number = if total > 1 { index + 1 } else { 1 };
        let stem = export_file_stem(&options.file_name, &app.photos[index].path, number, total);
        ui.label(
            RichText::new(format!("{stem}.{}", options.format.extension()))
                .size(12.0)
                .color(theme::TEXT_SECONDARY),
        );
    }
}

fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(theme::TEXT_SECONDARY));
}

fn start_export(app: &mut PhotonationApp) {
    let Some(directory) = rfd::FileDialog::new().set_title("Export to").pick_folder() else {
        return;
    };
    app.save_now();
    let photos = if app.view.export_all {
        // Sidecars hold every photo's settings (the current one was just saved).
        app.photos.iter().map(|p| (p.path.clone(), None)).collect()
    } else {
        app.current_path()
            .map(|p| vec![(p.clone(), Some(app.effective_settings()))])
            .unwrap_or_default()
    };
    app.engine
        .export(photos, directory, app.view.export_options.clone());
}
