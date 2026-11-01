// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The Presets card at the top of the inspector, the Edit menu's preset
//! items, and the Copy Settings, New Preset and delete dialogs. Resting
//! the pointer on a preset shows it on the photo; clicking applies it.
//! The state and actions are in `crate::presets`.

use crate::app::PhotonationApp;
use crate::presets::PresetRef;
use crate::theme;
use eframe::egui::{self, Align, Align2, Layout, RichText, Sense, Ui, pos2, vec2};
use photonation_develop::{
    AdjustmentGroup, MAX_PRESET_NAME_CHARS, Preset, SettingsPart, SettingsParts, StoredPresetFile,
};

/// What a click in a preset list asks for, applied after the list is drawn.
enum PresetAction {
    Apply(Preset),
    Delete(StoredPresetFile),
}

/// A listed preset: its settings and, for the user's, its file.
type Listed = (Preset, Option<StoredPresetFile>);

/// The presets by group, owned so the app can change while they are drawn.
fn listing(app: &PhotonationApp) -> Vec<(String, Vec<Listed>)> {
    app.view
        .presets
        .groups()
        .into_iter()
        .map(|(group, members)| {
            let members = members
                .into_iter()
                .map(|member| match member {
                    PresetRef::BuiltIn(preset) => (preset.clone(), None),
                    PresetRef::User(stored) => (stored.preset.clone(), Some(stored.clone())),
                })
                .collect();
            (group, members)
        })
        .collect()
}

/// The collapsible Presets card above the adjustment cards.
pub fn card(app: &mut PhotonationApp, ui: &mut Ui) {
    let mut action = None;
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let expanded = header(app, ui);
        if !expanded {
            return;
        }
        ui.add_space(6.0);
        let targets = app.rating_targets().len();
        if targets > 1 {
            ui.label(
                RichText::new(format!("Applies to the {targets} selected photos"))
                    .size(11.5)
                    .color(theme::TEXT_SECONDARY),
            );
        }
        for (group, members) in listing(app) {
            let folded = app.view.presets.folded.contains(&group);
            if group_heading(ui, &group, folded) {
                let folded_groups = &mut app.view.presets.folded;
                if !folded_groups.remove(&group) {
                    folded_groups.insert(group.clone());
                }
            }
            if folded {
                continue;
            }
            for (preset, stored) in members {
                let row = preset_row(ui, &preset.name);
                if row.hovered() {
                    app.preview_preset(&preset);
                }
                if row.clicked() {
                    action = Some(PresetAction::Apply(preset.clone()));
                }
                if let Some(stored) = stored {
                    row.context_menu(|ui| {
                        if ui.button("Delete Preset…").clicked() {
                            action = Some(PresetAction::Delete(stored.clone()));
                            ui.close();
                        }
                    });
                }
            }
        }
    });
    match action {
        Some(PresetAction::Apply(preset)) => app.apply_preset(&preset),
        Some(PresetAction::Delete(stored)) => app.view.presets.confirm_delete = Some(stored),
        None => {}
    }
}

/// The card's title row: folds the card, and holds the New Preset and
/// Import buttons. Returns whether the card is open.
fn header(app: &mut PhotonationApp, ui: &mut Ui) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let painter = ui.painter();
    let expanded = &mut app.view.presets.expanded;
    let color = if response.hovered() {
        theme::TEXT
    } else {
        theme::TEXT_SECONDARY
    };
    chevron(
        painter,
        pos2(rect.left() + 6.0, rect.center().y),
        *expanded,
        1.0,
        color,
    );
    painter.text(
        pos2(rect.left() + 18.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Presets",
        theme::title(13.5),
        theme::TEXT,
    );
    let mut buttons = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_max(
                pos2(rect.right() - 120.0, rect.top()),
                rect.max,
            ))
            .layout(Layout::right_to_left(Align::Center)),
    );
    let import = buttons
        .add(egui::Button::new(RichText::new("Import").size(11.5)).frame(false))
        .on_hover_text("Import Lightroom (.xmp) or Photonation presets");
    let new = buttons
        .add(egui::Button::new(RichText::new("+").size(15.0)).frame(false))
        .on_hover_text("Create a preset from this photo's settings");
    let on_buttons = buttons
        .min_rect()
        .contains(response.interact_pointer_pos().unwrap_or_default());
    if response.clicked() && !on_buttons {
        *expanded = !*expanded;
    }
    let expanded = *expanded;
    if new.clicked() {
        app.open_new_preset();
    }
    if import.clicked() {
        app.import_presets();
    }
    expanded
}

/// A group's name; true when clicked (to fold or unfold it).
fn group_heading(ui: &mut Ui, group: &str, folded: bool) -> bool {
    ui.add_space(2.0);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
    let color = if response.hovered() {
        theme::TEXT
    } else {
        theme::TEXT_SECONDARY
    };
    chevron(
        ui.painter(),
        pos2(rect.left() + 6.0, rect.center().y),
        !folded,
        0.75,
        color,
    );
    ui.painter().text(
        pos2(rect.left() + 16.0, rect.center().y),
        Align2::LEFT_CENTER,
        group,
        theme::body(11.5),
        color,
    );
    response.clicked()
}

/// A chevron pointing right, or down when `open`, `scale` times the
/// cards' size.
fn chevron(
    painter: &egui::Painter,
    center: egui::Pos2,
    open: bool,
    scale: f32,
    color: egui::Color32,
) {
    let points = if open {
        [(-4.0, -2.0), (0.0, 2.5), (4.0, -2.0)]
    } else {
        [(-2.0, -4.0), (2.5, 0.0), (-2.0, 4.0)]
    };
    let points = points
        .iter()
        .map(|(x, y)| center + vec2(x * scale, y * scale))
        .collect();
    painter.line(points, egui::Stroke::new(1.6, color));
}

/// One preset: a full-width row that highlights under the pointer.
fn preset_row(ui: &mut Ui, name: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(theme::CONTROL_RADIUS),
            theme::CONTROL_HOVER,
        );
    }
    ui.painter().text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        name,
        theme::body(12.5),
        theme::TEXT,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The Edit menu's Apply Preset submenu: one submenu per group. Resting
/// on a preset shows it on the photo.
pub fn apply_menu(app: &mut PhotonationApp, ui: &mut Ui) {
    let mut chosen = None;
    for (group, members) in listing(app) {
        ui.menu_button(group, |ui| {
            for (preset, _) in members {
                let item = ui.button(&preset.name);
                if item.hovered() {
                    app.preview_preset(&preset);
                }
                if item.clicked() {
                    chosen = Some(preset);
                    ui.close();
                }
            }
        });
    }
    if let Some(preset) = chosen {
        app.apply_preset(&preset);
    }
}

/// The open dialogs, if any.
pub fn dialogs(app: &mut PhotonationApp, ctx: &egui::Context) {
    if app.view.presets.show_copy {
        copy_dialog(app, ctx);
    }
    if app.view.presets.new_preset.is_some() {
        new_preset_dialog(app, ctx);
    }
    if app.view.presets.confirm_delete.is_some() {
        delete_dialog(app, ctx);
    }
}

fn dialog_title(ui: &mut Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .font(theme::title(16.0))
            .color(theme::TEXT),
    );
    ui.add_space(8.0);
}

/// OK and Cancel at the bottom right; returns (confirmed, cancelled).
fn dialog_buttons(ui: &mut Ui, confirm: &str, enabled: bool) -> (bool, bool) {
    let mut result = (false, false);
    ui.add_space(14.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let ok = ui.add_enabled(
            enabled,
            theme::primary_button(confirm).min_size(vec2(84.0, 28.0)),
        );
        result.0 = ok.clicked() || (enabled && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        result.1 = ui
            .add(egui::Button::new("Cancel").min_size(vec2(72.0, 28.0)))
            .clicked();
    });
    result
}

fn copy_dialog(app: &mut PhotonationApp, ctx: &egui::Context) {
    let mut parts = app.view.presets.copy_parts.clone();
    let mut done = None;
    let modal = egui::Modal::new(egui::Id::unique("copy-settings")).show(ctx, |ui| {
        ui.set_width(440.0);
        dialog_title(ui, "Copy Settings");
        parts_picker(ui, &mut parts);
        let (copy, cancel) = dialog_buttons(ui, "Copy", !parts.is_empty());
        if copy {
            done = Some(true);
        } else if cancel {
            done = Some(false);
        }
    });
    app.view.presets.copy_parts = parts.clone();
    if done == Some(true) {
        app.copy_settings(parts);
    }
    if done.is_some() || modal.should_close() {
        app.view.presets.show_copy = false;
    }
}

fn new_preset_dialog(app: &mut PhotonationApp, ctx: &egui::Context) {
    let Some(mut new) = app.view.presets.new_preset.take() else {
        return;
    };
    let groups: Vec<String> = app
        .view
        .presets
        .groups()
        .into_iter()
        .map(|(group, _)| group)
        .filter(|group| group != photonation_develop::BUILT_IN_PRESET_GROUP)
        .collect();
    let mut done = None;
    let modal = egui::Modal::new(egui::Id::unique("new-preset")).show(ctx, |ui| {
        ui.set_width(440.0);
        dialog_title(ui, "New Preset");
        egui::Grid::new("new-preset-fields")
            .num_columns(2)
            .spacing(vec2(10.0, 8.0))
            .show(ui, |ui| {
                ui.label("Name");
                let name = ui.add(
                    egui::TextEdit::singleline(&mut new.name)
                        .char_limit(MAX_PRESET_NAME_CHARS)
                        .desired_width(300.0)
                        .hint_text("e.g. Golden Hour"),
                );
                if ui.memory(|m| m.focused().is_none()) {
                    name.request_focus();
                }
                ui.end_row();
                ui.label("Group");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut new.group)
                            .char_limit(MAX_PRESET_NAME_CHARS)
                            .desired_width(220.0),
                    );
                    if !groups.is_empty() {
                        ui.menu_button("Existing", |ui| {
                            for group in &groups {
                                if ui.button(group).clicked() {
                                    new.group = group.clone();
                                    ui.close();
                                }
                            }
                        });
                    }
                });
                ui.end_row();
            });
        ui.add_space(10.0);
        ui.label(
            RichText::new("Settings to include")
                .size(12.0)
                .color(theme::TEXT_SECONDARY),
        );
        ui.add_space(4.0);
        parts_picker(ui, &mut new.parts);
        let ready = !new.name.trim().is_empty() && !new.parts.is_empty();
        let (save, cancel) = dialog_buttons(ui, "Save", ready);
        if save {
            done = Some(true);
        } else if cancel {
            done = Some(false);
        }
    });
    match done {
        Some(true) => app.save_preset(&new),
        Some(false) => {}
        None if modal.should_close() => {}
        None => app.view.presets.new_preset = Some(new),
    }
}

fn delete_dialog(app: &mut PhotonationApp, ctx: &egui::Context) {
    let Some(stored) = app.view.presets.confirm_delete.clone() else {
        return;
    };
    let mut done = None;
    let modal = egui::Modal::new(egui::Id::unique("delete-preset")).show(ctx, |ui| {
        ui.set_width(360.0);
        dialog_title(ui, &format!("Delete the preset “{}”?", stored.preset.name));
        ui.label(
            RichText::new(format!(
                "Its file goes to the {}. Photos it was applied to keep their settings.",
                crate::panels::library_bar::BIN_NAME
            ))
            .size(12.5)
            .color(theme::TEXT_SECONDARY),
        );
        let (delete, cancel) = dialog_buttons(ui, "Delete", true);
        if delete {
            done = Some(true);
        } else if cancel {
            done = Some(false);
        }
    });
    if done == Some(true) {
        app.delete_preset(&stored);
    }
    if done.is_some() || modal.should_close() {
        app.view.presets.confirm_delete = None;
    }
}

/// Checkboxes for every [`SettingsPart`], grouped by card as in
/// Lightroom's Copy Settings dialog, with Check All and Check None.
pub fn parts_picker(ui: &mut Ui, parts: &mut SettingsParts) {
    ui.horizontal(|ui| {
        if ui.small_button("Check All").clicked() {
            *parts = SettingsPart::ALL.into_iter().collect();
        }
        if ui.small_button("Check None").clicked() {
            parts.clear();
        }
    });
    ui.add_space(6.0);
    let (left, right) = AdjustmentGroup::ALL.split_at(4);
    ui.columns(2, |columns| {
        for group in left {
            group_checkboxes(&mut columns[0], parts, *group);
        }
        for group in right {
            group_checkboxes(&mut columns[1], parts, *group);
        }
        let tools = [
            SettingsPart::Crop,
            SettingsPart::Masks,
            SettingsPart::Removals,
        ];
        nested_checkboxes(&mut columns[1], parts, "Tools", &tools);
    });
}

fn group_checkboxes(ui: &mut Ui, parts: &mut SettingsParts, group: AdjustmentGroup) {
    let members: Vec<SettingsPart> = SettingsPart::in_group(group).collect();
    nested_checkboxes(ui, parts, group.title(), &members);
}

/// One checkbox for a single part, or a group checkbox (mixed while some
/// are on) over one per part.
fn nested_checkboxes(
    ui: &mut Ui,
    parts: &mut SettingsParts,
    title: &str,
    members: &[SettingsPart],
) {
    let set = |parts: &mut SettingsParts, part: SettingsPart, on: bool| {
        if on {
            parts.insert(part);
        } else {
            parts.remove(&part);
        }
    };
    if let [part] = members {
        let mut on = parts.contains(part);
        if ui.checkbox(&mut on, part.label()).changed() {
            set(parts, *part, on);
        }
        return;
    }
    let count = members.iter().filter(|part| parts.contains(part)).count();
    let mut all = count == members.len();
    let mixed = count > 0 && !all;
    let group = ui.add(egui::Checkbox::new(&mut all, title).indeterminate(mixed));
    if group.changed() {
        for part in members {
            set(parts, *part, all);
        }
    }
    ui.indent(title, |ui| {
        for part in members {
            let mut on = parts.contains(part);
            if ui.checkbox(&mut on, part.label()).changed() {
                set(parts, *part, on);
            }
        }
    });
}
