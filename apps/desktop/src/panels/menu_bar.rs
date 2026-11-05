// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The row above the toolbar. On macOS it is only room for the window buttons,
//! as the menus are in the system's menu bar (`native_menu`); elsewhere it holds
//! the menus (`menus`).

use crate::app::PhotonationApp;
use crate::menus::{self, Action, Entry, Item};
use crate::theme;
use eframe::egui::{self, Button, Ui};

pub fn show(app: &mut PhotonationApp, ui: &mut Ui) {
    egui::Panel::top("menu-bar")
        .exact_size(theme::MENU_BAR_HEIGHT)
        .show_separator_line(false)
        .frame(
            egui::Frame::NONE
                .fill(theme::palette().window)
                .inner_margin(egui::Margin::symmetric(8, 2)),
        )
        .show(ui, |ui| {
            if !cfg!(target_os = "macos") {
                draw(app, ui);
            }
        });
}

fn draw(app: &mut PhotonationApp, ui: &mut Ui) {
    let menus = menus::menus(app, false);
    let mut chosen = None;
    egui::MenuBar::new().ui(ui, |ui| {
        for menu in &menus {
            ui.menu_button(menu.title, |ui| {
                draw_entries(app, ui, &menu.entries, &mut chosen);
            });
        }
    });
    if let Some(action) = chosen {
        let ctx = ui.ctx().clone();
        app.perform(&action, &ctx);
    }
}

fn draw_entries(
    app: &mut PhotonationApp,
    ui: &mut Ui,
    entries: &[Entry],
    chosen: &mut Option<Action>,
) {
    for entry in entries {
        match entry {
            Entry::Item(item) => {
                let response = item_button(ui, item);
                if let Action::ApplyPreset { group, name } = &item.action
                    && response.hovered()
                    && let Some(preset) = crate::panels::presets::find(app, group, name)
                {
                    app.preview_preset(&preset);
                }
                if response.clicked() {
                    *chosen = Some(item.action.clone());
                    if matches!(item.action, Action::ApplyPreset { .. }) {
                        ui.close();
                    }
                }
            }
            Entry::Separator => {
                ui.separator();
            }
            Entry::Submenu {
                title,
                enabled,
                entries,
            } => {
                ui.add_enabled_ui(*enabled, |ui| {
                    ui.menu_button(title, |ui| draw_entries(app, ui, entries, chosen));
                });
            }
            // macOS's own items, in its menu bar only.
            Entry::System(_) => {}
        }
    }
}

/// A menu item with its shortcut, showing whether its mode is on if it has one.
fn item_button(ui: &mut Ui, item: &Item) -> egui::Response {
    let button = match item.checked {
        Some(on) => Button::selectable(on, &item.label),
        None => Button::new(&item.label),
    };
    ui.add_enabled(item.enabled, button.shortcut_text(item.shortcut))
}

/// Drawn headless, as on Windows and Linux.
#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, RawInput, Rect, pos2};

    fn entry(label: &str, action: Action, enabled: bool) -> Entry {
        Entry::Item(Item {
            label: label.into(),
            shortcut: "",
            action,
            enabled,
            checked: None,
        })
    }

    /// Clicks down a drawn list and returns what the clicks chose.
    fn chosen_by_clicking_down(entries: &[Entry]) -> Vec<Action> {
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);
        let (mut chosen, mut time) = (Vec::new(), 0.0);
        let mut frame = |events: Vec<Event>| {
            const DT: f32 = 1.0 / 60.0;
            time += f64::from(DT);
            let input = RawInput {
                screen_rect: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0))),
                time: Some(time),
                predicted_dt: DT,
                events,
                ..Default::default()
            };
            let mut this_frame = None;
            let mut output = ctx.run_ui(input, |ui| {
                draw_entries(&mut app, ui, entries, &mut this_frame);
            });
            output.textures_delta.clear();
            chosen.extend(this_frame);
        };
        for y in (2..120).step_by(4) {
            let pos = pos2(20.0, y as f32);
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(vec![Event::PointerMoved(pos)]);
            frame(vec![button(true)]);
            frame(vec![button(false)]);
        }
        chosen
    }

    #[test]
    fn clicking_an_item_chooses_its_action_unless_it_is_disabled() {
        let chosen = chosen_by_clicking_down(&[
            entry("Disabled", Action::Export, false),
            Entry::Separator,
            entry("Enabled", Action::About, true),
        ]);
        assert!(chosen.contains(&Action::About), "{chosen:?}");
        assert!(!chosen.contains(&Action::Export), "{chosen:?}");
    }
}
