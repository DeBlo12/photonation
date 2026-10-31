// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The Settings window (File ▸ Settings…): the graphics processor, where
//! exports are saved, and the options a new export starts with. Saved in
//! `preferences.json` when the window closes. Also the About window.

use crate::app::PhotonationApp;
use crate::gpu::{self, GpuSetting};
use crate::panels::export;
use crate::preferences::ExportDefaults;
use crate::theme;
use crate::widgets::{self, paint_brand_mark};
use eframe::egui::{self, RichText, Sense, Ui};

pub const TABS: [&str; 2] = ["General", "Export"];
const WIDTH: f32 = 380.0;

pub fn show(app: &mut PhotonationApp, ctx: &egui::Context) {
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("settings")).show(ctx, |ui| {
        ui.set_width(WIDTH);
        ui.label(
            RichText::new("Settings")
                .font(theme::title(17.0))
                .color(theme::TEXT),
        );
        ui.add_space(10.0);
        widgets::segmented(ui, &TABS, &mut app.view.settings_tab);
        ui.add_space(6.0);
        if app.view.settings_tab == 0 {
            general(app, ui);
        } else {
            export_defaults(app, ui);
        }
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(theme::primary_button("Done").min_size(egui::vec2(84.0, 28.0)))
                .clicked()
            {
                close = true;
            }
        });
    });
    if close || modal.should_close() {
        app.view.show_settings = false;
        app.preferences.save();
        app.view.export_options = app.preferences.export_options();
    }
}

fn general(app: &mut PhotonationApp, ui: &mut Ui) {
    heading(ui, "Graphics");
    export::label(ui, "Graphics processor");
    let preferences = &mut app.preferences;
    egui::ComboBox::from_id_salt("settings-gpu")
        .selected_text(preferences.gpu.label().to_owned())
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for choice in [
                GpuSetting::Automatic,
                GpuSetting::Integrated,
                GpuSetting::Software,
            ] {
                let label = choice.label().to_owned();
                ui.selectable_value(&mut preferences.gpu, choice, label);
            }
            let adapters = gpu::available_adapters();
            if !adapters.is_empty() {
                ui.separator();
            }
            for name in adapters {
                ui.selectable_value(
                    &mut preferences.gpu,
                    GpuSetting::Adapter(name.clone()),
                    name,
                );
            }
        });
    if let Some(in_use) = &app.gpu {
        note(ui, &format!("In use: {in_use}"));
    }
    if gpu::overridden_by_env() {
        attention(
            ui,
            "PHOTONATION_GPU is set and decides instead of this setting.",
        );
    } else if app.preferences.gpu != app.gpu_setting_at_launch {
        attention(ui, "Takes effect when Photonation starts again.");
    }
    note(
        ui,
        "It draws the interface, the previews and the mask overlay. Developing \
         and exporting photos runs on the CPU, on all its cores.",
    );

    ui.add_space(12.0);
    heading(ui, "Saving");
    export::label(ui, "Export folder");
    let preferences = &mut app.preferences;
    ui.horizontal(|ui| {
        let folder = preferences
            .export_folder
            .as_ref()
            .map_or("None: the export asks each time".to_owned(), |folder| {
                folder.display().to_string()
            });
        let choose = ui.button("Choose…");
        let clear = preferences.export_folder.is_some() && ui.button("Clear").clicked();
        ui.add(
            egui::Label::new(RichText::new(folder).color(theme::TEXT))
                .truncate()
                .sense(Sense::hover()),
        );
        if choose.clicked() {
            let mut dialog = rfd::FileDialog::new().set_title("Export folder");
            if let Some(folder) = &preferences.export_folder {
                dialog = dialog.set_directory(folder);
            }
            if let Some(folder) = dialog.pick_folder() {
                preferences.export_folder = Some(folder);
            }
        }
        if clear {
            preferences.export_folder = None;
            preferences.export_without_asking = false;
        }
    });
    ui.add_enabled(
        preferences.export_folder.is_some(),
        egui::Checkbox::new(
            &mut preferences.export_without_asking,
            "Export straight to this folder, without asking",
        ),
    );
    note(
        ui,
        "Otherwise the export asks for a folder, starting here. Edits are saved \
         beside each raw file as you work (IMG_0001.CR3.photonation.json); the \
         raw file itself is never changed.",
    );
}

fn export_defaults(app: &mut PhotonationApp, ui: &mut Ui) {
    heading(ui, "New exports start with");
    let mut options = app.preferences.export_options();
    export::format_controls(ui, &mut options);
    ui.add_space(6.0);
    export::label(ui, "File Name");
    export::file_name_field(ui, &mut options);
    ui.add_space(6.0);
    export::label(ui, "Metadata");
    export::metadata(ui, &mut options.metadata);
    app.preferences.export = ExportDefaults::from_options(&options);
    app.preferences.export_metadata = options.metadata;
    note(
        ui,
        "The export sheet can still change any of these for one export.",
    );
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(text)
            .font(theme::title(14.0))
            .color(theme::TEXT),
    );
    ui.add_space(2.0);
}

fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(theme::TEXT_TERTIARY));
}

fn attention(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(theme::ACCENT));
}

/// Help ▸ About Photonation.
pub fn about(app: &mut PhotonationApp, ctx: &egui::Context) {
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("about")).show(ctx, |ui| {
        ui.set_width(300.0);
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(72.0, 72.0), Sense::hover());
            paint_brand_mark(ui.painter(), rect.center(), 68.0, theme::CARD);
            ui.add_space(6.0);
            ui.label(
                RichText::new("photonation")
                    .font(theme::display(24.0))
                    .color(theme::TEXT),
            );
            ui.label(
                RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                    .color(theme::TEXT_SECONDARY),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "A raw photo editor. Free software under the GNU General Public \
                     License, version 3 or later.",
                )
                .size(12.0)
                .color(theme::TEXT_SECONDARY),
            );
            if let Some(gpu) = &app.gpu {
                ui.add_space(4.0);
                note(ui, &format!("Graphics: {gpu}"));
            }
            ui.add_space(12.0);
            if ui
                .add(theme::primary_button("Close").min_size(egui::vec2(84.0, 28.0)))
                .clicked()
            {
                close = true;
            }
        });
    });
    if close || modal.should_close() {
        app.view.show_about = false;
    }
}
