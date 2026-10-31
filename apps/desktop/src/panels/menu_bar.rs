// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The menu bar above the toolbar: File, Edit, View and Help. Every item
//! does what its toolbar button or shortcut does; the menus make them
//! discoverable and list the shortcuts.

use crate::app::{PhotonationApp, Zoom};
use crate::theme;
use eframe::egui::{self, Button, Ui};
use photonation_develop::Settings;

pub fn show(app: &mut PhotonationApp, ui: &mut Ui) {
    egui::Panel::top("menu-bar")
        .exact_size(theme::MENU_BAR_HEIGHT)
        .show_separator_line(false)
        .frame(
            egui::Frame::NONE
                .fill(theme::WINDOW)
                .inner_margin(egui::Margin::symmetric(8, 2)),
        )
        .show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                // On macOS the window buttons sit in this row.
                ui.add_space(theme::TRAFFIC_LIGHT_INSET);
                ui.menu_button("File", |ui| file(app, ui));
                ui.menu_button("Edit", |ui| edit(app, ui));
                ui.menu_button("View", |ui| view(app, ui));
                ui.menu_button("Help", |ui| help(app, ui));
            });
        });
}

/// A menu item with its shortcut; true when chosen.
fn item(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, Button::new(label).shortcut_text(shortcut))
        .clicked()
}

/// A menu item that shows whether its mode is on.
fn toggle_item(ui: &mut Ui, label: &str, shortcut: &str, on: bool, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        Button::selectable(on, label).shortcut_text(shortcut),
    )
    .clicked()
}

fn file(app: &mut PhotonationApp, ui: &mut Ui) {
    let has_photo = app.current.is_some();
    if item(ui, "Open Folder…", theme::SHORTCUT_OPEN, true) {
        app.pick_folder();
    }
    ui.separator();
    if item(ui, "Export…", theme::SHORTCUT_EXPORT, has_photo) {
        app.view.export_all = false;
        app.view.show_export = true;
    }
    if item(ui, "Export All Photos…", "", app.photos.len() > 1) {
        app.view.export_all = true;
        app.view.show_export = true;
    }
    ui.separator();
    if item(ui, "Settings…", theme::SHORTCUT_SETTINGS, true) {
        app.view.show_settings = true;
    }
    ui.separator();
    if item(ui, "Quit", theme::SHORTCUT_QUIT, true) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

fn edit(app: &mut PhotonationApp, ui: &mut Ui) {
    if item(ui, "Undo", theme::SHORTCUT_UNDO, app.history.can_undo()) {
        app.undo();
    }
    if item(ui, "Redo", theme::SHORTCUT_REDO, app.history.can_redo()) {
        app.redo();
    }
    ui.separator();
    if item(ui, "Reset All Adjustments", "", app.current.is_some()) {
        app.settings = Settings::default();
    }
}

fn view(app: &mut PhotonationApp, ui: &mut Ui) {
    let has_photo = app.current.is_some();
    if item(ui, "Fit to Window", theme::SHORTCUT_FIT, has_photo) {
        app.view.zoom = Zoom::Fit;
        app.view.pan = egui::Vec2::ZERO;
    }
    if item(ui, "Actual Pixels", "", has_photo) {
        app.view.zoom = Zoom::Scale(1.0);
        app.view.pan = egui::Vec2::ZERO;
    }
    let compare = app.view.compare;
    if toggle_item(ui, "Compare with Original", "\\", compare, has_photo) {
        app.view.compare = !compare;
    }
    ui.separator();
    if toggle_item(ui, "Masks", "M", app.view.show_masks, has_photo) {
        app.view.toggle_masks();
    }
    if toggle_item(ui, "Crop and Straighten", "R", app.view.cropping, has_photo) {
        app.view.toggle_cropping();
    }
    if toggle_item(ui, "Remove Objects", "Q", app.view.show_removals, has_photo) {
        app.view.toggle_removals();
    }
    ui.separator();
    let current = app.current.unwrap_or(0);
    if item(ui, "Previous Photo", "←", has_photo && current > 0) {
        app.select(current - 1);
    }
    if item(
        ui,
        "Next Photo",
        "→",
        has_photo && current + 1 < app.photos.len(),
    ) {
        app.select(current + 1);
    }
}

fn help(app: &mut PhotonationApp, ui: &mut Ui) {
    if item(ui, "How Photonation Works", "", true) {
        crate::panels::help::open(ui.ctx());
    }
    ui.separator();
    if item(ui, "About Photonation", "", true) {
        app.view.show_about = true;
    }
}
