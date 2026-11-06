// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The menus in the macOS menu bar, built with muda from the data in `menus`.
//! See docs/DESKTOP.md.

use crate::menus::{Action, Entry, Item, Menu, SystemItem};
use eframe::egui;
use muda::accelerator::{Accelerator, Code, Modifiers};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};

/// The live menu bar, kept in step with the menus' data.
pub struct NativeMenus {
    /// The menu bar; macOS uses it for as long as it exists.
    bar: muda::Menu,
    shown: Vec<Menu>,
    handles: Vec<Handle>,
    actions: HashMap<muda::MenuId, Action>,
    chosen: Receiver<muda::MenuId>,
}

/// An item as macOS has it, in the same place as its entry in the data.
enum Handle {
    Plain(muda::MenuItem),
    Check(muda::CheckMenuItem),
    Submenu(muda::Submenu, Vec<Handle>),
    Fixed,
}

impl NativeMenus {
    /// Replaces winit's default menu bar. A choice wakes the app to act on it.
    pub fn install(ctx: &egui::Context, menus: Vec<Menu>) -> NativeMenus {
        let (sender, chosen) = channel();
        let ctx = ctx.clone();
        muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
            sender.send(event.id).ok();
            ctx.request_repaint();
        }));
        let (bar, handles) = build(&menus);
        let actions = actions_by_id(&menus);
        NativeMenus {
            bar,
            shown: menus,
            handles,
            actions,
            chosen,
        }
    }

    /// The actions chosen since the last call.
    pub fn take_chosen(&self) -> Vec<Action> {
        self.chosen
            .try_iter()
            .filter_map(|id| self.actions.get(&id).cloned())
            .collect()
    }

    /// Brings the menu bar up to date. Most frames nothing has changed. Items
    /// change in place, so that a menu someone has open stays open; only a
    /// different list of items, as when presets are imported, means a new bar.
    pub fn update(&mut self, menus: Vec<Menu>) {
        if menus == self.shown {
            return;
        }
        if same_shape(&menus, &self.shown) {
            for (handles, menu) in self.handles.iter().zip(&menus) {
                let Handle::Submenu(_, handles) = handles else {
                    continue;
                };
                update_entries(handles, &menu.entries);
            }
        } else {
            let (bar, handles) = build(&menus);
            self.bar = bar;
            self.handles = handles;
        }
        self.actions = actions_by_id(&menus);
        self.shown = menus;
    }
}

/// Whether the two have the same items in the same places, whatever they say.
fn same_shape(left: &[Menu], right: &[Menu]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| same_entries(&left.entries, &right.entries))
}

fn same_entries(left: &[Entry], right: &[Entry]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|pair| match pair {
            (Entry::Item(left), Entry::Item(right)) => {
                left.checked.is_some() == right.checked.is_some()
                    && accelerator(left) == accelerator(right)
            }
            (Entry::Separator, Entry::Separator) => true,
            (Entry::System(left), Entry::System(right)) => left == right,
            (Entry::Submenu { entries: left, .. }, Entry::Submenu { entries: right, .. }) => {
                same_entries(left, right)
            }
            _ => false,
        })
}

fn update_entries(handles: &[Handle], entries: &[Entry]) {
    for (handle, entry) in handles.iter().zip(entries) {
        match (handle, entry) {
            (Handle::Plain(native), Entry::Item(item)) => {
                native.set_text(title(item));
                native.set_enabled(item.enabled);
            }
            (Handle::Check(native), Entry::Item(item)) => {
                native.set_text(title(item));
                native.set_enabled(item.enabled);
                native.set_checked(item.checked.unwrap_or(false));
            }
            (
                Handle::Submenu(native, handles),
                Entry::Submenu {
                    title,
                    enabled,
                    entries,
                },
            ) => {
                native.set_text(title);
                native.set_enabled(*enabled);
                update_entries(handles, entries);
            }
            _ => {}
        }
    }
}

fn build(menus: &[Menu]) -> (muda::Menu, Vec<Handle>) {
    let bar = muda::Menu::new();
    let mut handles = Vec::with_capacity(menus.len());
    for (index, menu) in menus.iter().enumerate() {
        let native = muda::Submenu::new(menu.title, true);
        let children = append_entries(&native, &menu.entries, &index.to_string());
        bar.append(&native).expect("a menu joins the menu bar");
        match menu.title {
            "Window" => native.set_as_windows_menu_for_nsapp(),
            "Help" => native.set_as_help_menu_for_nsapp(),
            _ => {}
        }
        handles.push(Handle::Submenu(native, children));
    }
    bar.init_for_nsapp();
    (bar, handles)
}

fn append_entries(parent: &muda::Submenu, entries: &[Entry], path: &str) -> Vec<Handle> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let id = entry_id(path, index);
            match entry {
                Entry::Item(item) if item.checked.is_some() => {
                    let native = muda::CheckMenuItem::with_id(
                        id,
                        title(item),
                        item.enabled,
                        item.checked.unwrap_or(false),
                        accelerator(item),
                    );
                    parent.append(&native).expect("an item joins its menu");
                    Handle::Check(native)
                }
                Entry::Item(item) => {
                    let native =
                        muda::MenuItem::with_id(id, title(item), item.enabled, accelerator(item));
                    parent.append(&native).expect("an item joins its menu");
                    Handle::Plain(native)
                }
                Entry::Separator => {
                    parent
                        .append(&muda::PredefinedMenuItem::separator())
                        .expect("a separator joins its menu");
                    Handle::Fixed
                }
                Entry::System(system) => {
                    parent
                        .append(&system_item(*system))
                        .expect("a system item joins its menu");
                    Handle::Fixed
                }
                Entry::Submenu {
                    title,
                    enabled,
                    entries,
                } => {
                    let native = muda::Submenu::new(title, *enabled);
                    let children = append_entries(&native, entries, &id);
                    parent.append(&native).expect("a submenu joins its menu");
                    Handle::Submenu(native, children)
                }
            }
        })
        .collect()
}

fn system_item(item: SystemItem) -> muda::PredefinedMenuItem {
    use muda::PredefinedMenuItem as Predefined;
    match item {
        SystemItem::Services => Predefined::services(None),
        // By default it says the executable's name.
        SystemItem::Hide => Predefined::hide(Some("Hide Photonation")),
        SystemItem::HideOthers => Predefined::hide_others(None),
        SystemItem::ShowAll => Predefined::show_all(None),
        SystemItem::Minimize => Predefined::minimize(None),
        SystemItem::Zoom => Predefined::maximize(None),
        SystemItem::BringAllToFront => Predefined::bring_all_to_front(None),
        SystemItem::FullScreen => Predefined::fullscreen(None),
    }
}

/// An item's place in the menus, which is its id: the same item keeps it when
/// what it says changes.
fn entry_id(path: &str, index: usize) -> String {
    format!("{path}.{index}")
}

fn actions_by_id(menus: &[Menu]) -> HashMap<muda::MenuId, Action> {
    fn collect(entries: &[Entry], path: &str, actions: &mut HashMap<muda::MenuId, Action>) {
        for (index, entry) in entries.iter().enumerate() {
            let id = entry_id(path, index);
            match entry {
                Entry::Item(item) => {
                    actions.insert(muda::MenuId::new(id), item.action.clone());
                }
                Entry::Submenu { entries, .. } => collect(entries, &id, actions),
                Entry::Separator | Entry::System(_) => {}
            }
        }
    }
    let mut actions = HashMap::new();
    for (index, menu) in menus.iter().enumerate() {
        collect(&menu.entries, &index.to_string(), &mut actions);
    }
    actions
}

/// What an item says. A key macOS doesn't handle for it goes in the text, the way
/// tooltips show them.
fn title(item: &Item) -> String {
    if item.shortcut.is_empty() || accelerator(item).is_some() {
        item.label.clone()
    } else {
        format!("{} ({})", item.label, item.shortcut)
    }
}

/// The shortcut macOS should handle for an item, from the text the app shows.
///
/// Only ⌘ shortcuts qualify. A macOS key equivalent acts before the focused
/// view sees the key, so a plain key would fire while typing a preset's name,
/// and ⌘A would select photos instead of the text in a field. The app's own
/// shortcut handling keeps those, and knows when someone is typing.
fn accelerator(item: &Item) -> Option<Accelerator> {
    if item.action == Action::SelectAll {
        return None;
    }
    let key = item.shortcut.strip_suffix(|_: char| true)?;
    let code = key_code(item.shortcut.chars().last()?)?;
    let modifiers = match key {
        "⌘" => Modifiers::META,
        "⇧⌘" => Modifiers::META | Modifiers::SHIFT,
        _ => return None,
    };
    Some(Accelerator::new(modifiers, code))
}

fn key_code(key: char) -> Option<Code> {
    Some(match key {
        'C' => Code::KeyC,
        'E' => Code::KeyE,
        'O' => Code::KeyO,
        'Q' => Code::KeyQ,
        'V' => Code::KeyV,
        'Z' => Code::KeyZ,
        '0' => Code::Digit0,
        ',' => Code::Comma,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::{self, Action};
    use crate::theme;

    fn item(shortcut: &'static str, action: Action) -> Item {
        Item {
            label: "Name".into(),
            shortcut,
            action,
            enabled: true,
            checked: None,
        }
    }

    #[test]
    fn every_command_shortcut_the_app_shows_is_understood() {
        for shortcut in [
            theme::SHORTCUT_OPEN,
            theme::SHORTCUT_OPEN_PHOTO,
            theme::SHORTCUT_EXPORT,
            theme::SHORTCUT_FIT,
            theme::SHORTCUT_UNDO,
            theme::SHORTCUT_REDO,
            theme::SHORTCUT_SETTINGS,
            theme::SHORTCUT_QUIT,
            theme::SHORTCUT_COPY_SETTINGS,
            theme::SHORTCUT_PASTE_SETTINGS,
        ] {
            assert!(
                accelerator(&item(shortcut, Action::Undo)).is_some(),
                "{shortcut}"
            );
        }
    }

    #[test]
    fn keys_the_app_handles_itself_go_in_the_text() {
        for (shortcut, action) in [
            ("G", Action::ToggleGrid),
            ("←", Action::PreviousPhoto),
            ("F1", Action::Help),
            (theme::SHORTCUT_SELECT_ALL, Action::SelectAll),
        ] {
            let item = item(shortcut, action);
            assert_eq!(accelerator(&item), None, "{shortcut}");
            assert_eq!(title(&item), format!("Name ({shortcut})"));
        }
    }

    #[test]
    fn a_shortcut_macos_handles_stays_out_of_the_text() {
        assert_eq!(title(&item(theme::SHORTCUT_UNDO, Action::Undo)), "Name");
    }

    #[test]
    fn changed_text_keeps_the_shape_and_changed_items_do_not() {
        let app = crate::app::PhotonationApp::empty(&egui::Context::default());
        let before = menus::menus(&app, true);
        let mut renamed = before.clone();
        let Entry::Item(first) = &mut renamed[1].entries[0] else {
            panic!("File starts with an item");
        };
        first.label = "Something Else".into();
        first.enabled = false;
        assert!(same_shape(&before, &renamed));
        let mut longer = before.clone();
        longer[1].entries.push(Entry::Separator);
        assert!(!same_shape(&before, &longer));
    }

    #[test]
    fn every_item_has_an_action_under_its_own_id() {
        let app = crate::app::PhotonationApp::empty(&egui::Context::default());
        let menus = menus::menus(&app, true);
        let actions = actions_by_id(&menus);
        assert_eq!(actions.get(&muda::MenuId::new("0.0")), Some(&Action::About));
        fn items(entries: &[Entry]) -> usize {
            entries
                .iter()
                .map(|entry| match entry {
                    Entry::Item(_) => 1,
                    Entry::Submenu { entries, .. } => items(entries),
                    _ => 0,
                })
                .sum()
        }
        let total: usize = menus.iter().map(|menu| items(&menu.entries)).sum();
        assert_eq!(actions.len(), total);
    }
}
