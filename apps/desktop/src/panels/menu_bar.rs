// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The menu bar above the toolbar: File, Edit, View, Library and Help.
//! Every item does what its toolbar button or shortcut does; the menus
//! make them discoverable and list the shortcuts.

use crate::app::{PhotonationApp, Zoom};
use crate::library::{EditedFilter, FlagFilter, LibraryChange, SortOrder};
use crate::panels::help;
use crate::panels::library_bar::BIN_NAME;
use crate::theme;
use crate::widgets::rating::label_key;
use eframe::egui::{self, Button, Ui};
use photonation_develop::{ColorLabel, Flag, MAX_RATING, Settings};

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
                ui.menu_button("Library", |ui| library(app, ui));
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
    let (batch, kind) = app.export_batch();
    if item(ui, kind.menu_label(), "", batch.len() > 1) {
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
    let shown = &app.view.library.shown;
    let position = app
        .current
        .and_then(|current| shown.iter().position(|&i| i == current));
    let last = shown.len().saturating_sub(1);
    if item(ui, "Previous Photo", "←", position.is_some_and(|p| p > 0)) {
        app.step(-1);
    }
    if item(ui, "Next Photo", "→", position.is_some_and(|p| p < last)) {
        app.step(1);
    }
}

fn library(app: &mut PhotonationApp, ui: &mut Ui) {
    let has_photo = app.current.is_some();
    let (grid, survey) = (app.view.library.grid, app.view.library.survey);
    if toggle_item(ui, "Grid", "G", grid && !survey, has_photo) {
        app.toggle_grid();
    }
    if toggle_item(ui, "Survey", "N", survey, has_photo) {
        app.toggle_survey();
    }
    ui.separator();
    let info = app
        .current
        .map(|index| app.photos[index].library)
        .unwrap_or_default();
    let mut change = None;
    ui.add_enabled_ui(has_photo, |ui| {
        ui.menu_button("Rating", |ui| {
            for stars in 0..=MAX_RATING {
                let label = match stars {
                    0 => "No Rating".to_string(),
                    1 => "1 Star".to_string(),
                    n => format!("{n} Stars"),
                };
                if toggle_item(ui, &label, &stars.to_string(), info.rating == stars, true) {
                    change = Some(LibraryChange::Rating(stars));
                }
            }
            ui.separator();
            if item(ui, "Increase Rating", "]", info.rating < MAX_RATING) {
                change = Some(LibraryChange::RatingStep(1));
            }
            if item(ui, "Decrease Rating", "[", info.rating > 0) {
                change = Some(LibraryChange::RatingStep(-1));
            }
        });
        ui.menu_button("Label", |ui| {
            for label in ColorLabel::ALL {
                let on = info.label == Some(label);
                if toggle_item(ui, label.name(), label_key(label).unwrap_or(""), on, true) {
                    change = Some(LibraryChange::ToggleLabel(label));
                }
            }
            ui.separator();
            if item(ui, "No Label", "", info.label.is_some()) {
                change = Some(LibraryChange::ClearLabel);
            }
        });
    });
    for (label, shortcut, flag) in [
        ("Pick", "P", Flag::Pick),
        ("Reject", "X", Flag::Reject),
        ("Unflag", "U", Flag::None),
    ] {
        let on = flag != Flag::None && info.flag == flag;
        if toggle_item(ui, label, shortcut, on, has_photo) {
            change = Some(LibraryChange::Flag(flag));
        }
    }
    if let Some(change) = change {
        app.change_library(change);
    }
    let advance = app.view.library.auto_advance;
    if toggle_item(ui, "Auto-Advance", "", advance, true) {
        app.view.library.auto_advance = !advance;
        app.preferences.library.auto_advance = !advance;
        app.preferences.save();
    }
    ui.separator();
    ui.menu_button("Filter", |ui| filter_menu(app, ui));
    ui.menu_button("Sort By", |ui| {
        for order in SortOrder::ALL {
            if toggle_item(ui, order.label(), "", app.view.library.sort == order, true) {
                app.view.library.sort = order;
                app.preferences.library.sort = order;
                app.preferences.save();
            }
        }
    });
    ui.separator();
    if item(ui, "Select All", theme::SHORTCUT_SELECT_ALL, has_photo) {
        app.select_all();
    }
    if item(
        ui,
        "Select None",
        "",
        !app.view.library.selection.is_empty(),
    ) {
        app.view.library.selection.clear();
        app.view.library.survey = false;
    }
    ui.separator();
    let rejected = app.rejected_photos().len();
    let trash_label = format!("Move Rejected Photos to {BIN_NAME}…");
    if item(ui, &trash_label, "", rejected > 0) {
        app.view.library.confirm_trash = true;
    }
}

fn filter_menu(app: &mut PhotonationApp, ui: &mut Ui) {
    let filter = &mut app.view.library.filter;
    for stars in 0..=MAX_RATING {
        let label = match stars {
            0 => "Any Rating".to_string(),
            5 => "5 Stars".to_string(),
            n => format!("{n} Stars or More"),
        };
        if toggle_item(ui, &label, "", filter.min_rating == stars, true) {
            filter.min_rating = stars;
        }
    }
    ui.separator();
    for flag in FlagFilter::ALL {
        if toggle_item(ui, flag.label(), "", filter.flag == flag, true) {
            filter.flag = flag;
        }
    }
    ui.separator();
    for label in ColorLabel::ALL {
        let on = filter.labels.contains(&label);
        if toggle_item(ui, label.name(), "", on, true) && !filter.labels.remove(&label) {
            filter.labels.insert(label);
        }
    }
    ui.separator();
    for edited in EditedFilter::ALL {
        if toggle_item(ui, edited.label(), "", filter.edited == edited, true) {
            filter.edited = edited;
        }
    }
    ui.separator();
    if item(ui, "Clear Filter", "", filter.is_active()) {
        *filter = Default::default();
    }
}

fn help(app: &mut PhotonationApp, ui: &mut Ui) {
    if item(ui, "Photonation Help", "F1", true) {
        help::open(&mut app.view);
    }
    if item(ui, "Keyboard Shortcuts", "", true) {
        help::open_page(&mut app.view, help::Page::Shortcuts);
    }
    ui.separator();
    if item(ui, "About Photonation", "", true) {
        app.view.show_about = true;
    }
}
