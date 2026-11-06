// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The menus as data, and what choosing an item does. `panels::menu_bar` draws
//! them in the window; on macOS `native_menu` puts them in the system menu bar.
//! Every item does what its toolbar button or shortcut does; the menus make them
//! discoverable and list the shortcuts. See docs/DESKTOP.md.

use crate::app::{PhotonationApp, Zoom};
use crate::library::{EditedFilter, FlagFilter, LibraryChange, SortOrder};
use crate::panels::help;
use crate::panels::library_bar::BIN_NAME;
use crate::panels::presets;
use crate::theme;
use crate::widgets::rating::label_key;
use eframe::egui;
use photonation_develop::{ColorLabel, Flag, MAX_RATING, Settings};

#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub title: &'static str,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Item(Item),
    Separator,
    Submenu {
        title: String,
        enabled: bool,
        entries: Vec<Entry>,
    },
    /// An item macOS provides and handles itself.
    System(SystemItem),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    /// As shown, such as "⌘O" or "G"; empty for none.
    pub shortcut: &'static str,
    pub action: Action,
    pub enabled: bool,
    /// Whether its mode is on, for items that have one.
    pub checked: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemItem {
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Minimize,
    Zoom,
    BringAllToFront,
    FullScreen,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    OpenFolder,
    OpenPhoto,
    Export,
    ExportBatch,
    Settings,
    Quit,
    Undo,
    Redo,
    CopySettings,
    PasteSettings,
    ApplyPreset { group: String, name: String },
    NewPreset,
    ImportPresets,
    ResetAdjustments,
    FitToWindow,
    ActualPixels,
    ToggleCompare,
    ToggleFullScreen,
    ToggleWindowFullScreen,
    ToggleMasks,
    ToggleCropping,
    ToggleRemovals,
    PreviousPhoto,
    NextPhoto,
    ToggleGrid,
    ToggleSurvey,
    Library(LibraryChange),
    ToggleAutoAdvance,
    FilterMinRating(u8),
    FilterFlag(FlagFilter),
    FilterLabel(ColorLabel),
    FilterEdited(EditedFilter),
    ClearFilter,
    Sort(SortOrder),
    SelectAll,
    SelectNone,
    MoveRejectedToBin,
    Help,
    KeyboardShortcuts,
    About,
}

fn item(label: impl Into<String>, shortcut: &'static str, action: Action, enabled: bool) -> Entry {
    Entry::Item(Item {
        label: label.into(),
        shortcut,
        action,
        enabled,
        checked: None,
    })
}

fn toggle(
    label: impl Into<String>,
    shortcut: &'static str,
    action: Action,
    on: bool,
    enabled: bool,
) -> Entry {
    Entry::Item(Item {
        label: label.into(),
        shortcut,
        action,
        enabled,
        checked: Some(on),
    })
}

fn submenu(title: &str, enabled: bool, entries: Vec<Entry>) -> Entry {
    Entry::Submenu {
        title: title.into(),
        enabled,
        entries,
    }
}

/// The menus as they are now. `native` arranges them as macOS expects: an
/// application menu with About, Settings and Quit, and a Window menu.
pub fn menus(app: &PhotonationApp, native: bool) -> Vec<Menu> {
    let mut menus = Vec::new();
    if native {
        menus.push(application());
    }
    menus.extend([
        Menu {
            title: "File",
            entries: file(app, native),
        },
        Menu {
            title: "Edit",
            entries: edit(app),
        },
        Menu {
            title: "View",
            entries: view(app, native),
        },
        Menu {
            title: "Library",
            entries: library(app),
        },
    ]);
    if native {
        menus.push(window());
    }
    menus.push(Menu {
        title: "Help",
        entries: help_menu(native),
    });
    if app.view.dialog_open() {
        for menu in &mut menus {
            disable_for_dialog(&mut menu.entries);
        }
    }
    menus
}

/// A dialog in the window has to be answered first. The window's own menus are
/// behind it already; the system's are always within reach.
fn disable_for_dialog(entries: &mut [Entry]) {
    for entry in entries {
        match entry {
            Entry::Item(item) => item.enabled &= item.action == Action::Quit,
            Entry::Submenu {
                enabled, entries, ..
            } => {
                *enabled = false;
                disable_for_dialog(entries);
            }
            Entry::Separator | Entry::System(_) => {}
        }
    }
}

fn about() -> Entry {
    item("About Photonation", "", Action::About, true)
}

fn settings() -> Entry {
    item(
        "Settings…",
        theme::SHORTCUT_SETTINGS,
        Action::Settings,
        true,
    )
}

fn quit() -> Entry {
    item("Quit", theme::SHORTCUT_QUIT, Action::Quit, true)
}

fn application() -> Menu {
    Menu {
        // macOS shows the app's name here, whatever this says.
        title: "Photonation",
        entries: vec![
            about(),
            Entry::Separator,
            settings(),
            Entry::Separator,
            Entry::System(SystemItem::Services),
            Entry::Separator,
            Entry::System(SystemItem::Hide),
            Entry::System(SystemItem::HideOthers),
            Entry::System(SystemItem::ShowAll),
            Entry::Separator,
            item("Quit Photonation", theme::SHORTCUT_QUIT, Action::Quit, true),
        ],
    }
}

fn window() -> Menu {
    Menu {
        title: "Window",
        entries: vec![
            Entry::System(SystemItem::Minimize),
            Entry::System(SystemItem::Zoom),
            Entry::Separator,
            Entry::System(SystemItem::BringAllToFront),
        ],
    }
}

/// Also the labels of the buttons that do the same.
pub const OPEN_FOLDER: &str = "Open Folder…";
pub const OPEN_PHOTO: &str = "Open Photo…";

fn file(app: &PhotonationApp, native: bool) -> Vec<Entry> {
    let (batch, kind) = app.export_batch();
    let mut entries = vec![
        item(OPEN_FOLDER, theme::SHORTCUT_OPEN, Action::OpenFolder, true),
        item(
            OPEN_PHOTO,
            theme::SHORTCUT_OPEN_PHOTO,
            Action::OpenPhoto,
            true,
        ),
        Entry::Separator,
        item(
            "Export…",
            theme::SHORTCUT_EXPORT,
            Action::Export,
            app.current.is_some(),
        ),
        item(kind.menu_label(), "", Action::ExportBatch, batch.len() > 1),
    ];
    if !native {
        entries.extend([Entry::Separator, settings(), Entry::Separator, quit()]);
    }
    entries
}

fn edit(app: &PhotonationApp) -> Vec<Entry> {
    let has_photo = app.current.is_some();
    let targets = app.rating_targets().len();
    let paste = if targets > 1 {
        format!("Paste Settings to {targets} Photos")
    } else {
        "Paste Settings".into()
    };
    vec![
        item(
            "Undo",
            theme::SHORTCUT_UNDO,
            Action::Undo,
            app.history.can_undo(),
        ),
        item(
            "Redo",
            theme::SHORTCUT_REDO,
            Action::Redo,
            app.history.can_redo(),
        ),
        Entry::Separator,
        item(
            "Copy Settings…",
            theme::SHORTCUT_COPY_SETTINGS,
            Action::CopySettings,
            has_photo,
        ),
        item(
            paste,
            theme::SHORTCUT_PASTE_SETTINGS,
            Action::PasteSettings,
            has_photo && app.view.presets.copied.is_some(),
        ),
        Entry::Separator,
        submenu("Apply Preset", has_photo, presets::apply_entries(app)),
        item("New Preset…", "", Action::NewPreset, has_photo),
        item("Import Presets…", "", Action::ImportPresets, true),
        Entry::Separator,
        item(
            "Reset All Adjustments",
            "",
            Action::ResetAdjustments,
            has_photo,
        ),
    ]
}

fn view(app: &PhotonationApp, native: bool) -> Vec<Entry> {
    let has_photo = app.current.is_some();
    let shown = &app.view.library.shown;
    let position = app
        .current
        .and_then(|current| shown.iter().position(|&i| i == current));
    let last = shown.len().saturating_sub(1);
    let mut entries = vec![
        item(
            "Fit to Window",
            theme::SHORTCUT_FIT,
            Action::FitToWindow,
            has_photo,
        ),
        item("Actual Pixels", "", Action::ActualPixels, has_photo),
        toggle(
            "Compare with Original",
            "\\",
            Action::ToggleCompare,
            app.view.compare,
            has_photo,
        ),
        toggle(
            "Full Screen Photo",
            theme::SHORTCUT_FULL_SCREEN,
            Action::ToggleFullScreen,
            app.view.full_screen.is_some(),
            has_photo,
        ),
        Entry::Separator,
        toggle(
            "Masks",
            "M",
            Action::ToggleMasks,
            app.view.show_masks,
            has_photo,
        ),
        toggle(
            "Crop and Straighten",
            "R",
            Action::ToggleCropping,
            app.view.cropping,
            has_photo,
        ),
        toggle(
            "Remove Objects",
            "Q",
            Action::ToggleRemovals,
            app.view.show_removals,
            has_photo,
        ),
        Entry::Separator,
        item(
            "Previous Photo",
            "←",
            Action::PreviousPhoto,
            position.is_some_and(|p| p > 0),
        ),
        item(
            "Next Photo",
            "→",
            Action::NextPhoto,
            position.is_some_and(|p| p < last),
        ),
    ];
    // macOS has its own, with its own shortcut.
    entries.extend([
        Entry::Separator,
        if native {
            Entry::System(SystemItem::FullScreen)
        } else {
            item(
                "Full Screen Window",
                "F11",
                Action::ToggleWindowFullScreen,
                true,
            )
        },
    ]);
    entries
}

fn library(app: &PhotonationApp) -> Vec<Entry> {
    let has_photo = app.current.is_some();
    let library = &app.view.library;
    let info = app
        .current
        .map(|index| app.photos[index].library)
        .unwrap_or_default();
    let change = |change| Action::Library(change);

    let mut ratings: Vec<Entry> = (0..=MAX_RATING)
        .map(|stars| {
            let label = match stars {
                0 => "No Rating".to_string(),
                1 => "1 Star".to_string(),
                n => format!("{n} Stars"),
            };
            toggle(
                label,
                RATING_KEYS[usize::from(stars)],
                change(LibraryChange::Rating(stars)),
                info.rating == stars,
                true,
            )
        })
        .collect();
    ratings.extend([
        Entry::Separator,
        item(
            "Increase Rating",
            "]",
            change(LibraryChange::RatingStep(1)),
            info.rating < MAX_RATING,
        ),
        item(
            "Decrease Rating",
            "[",
            change(LibraryChange::RatingStep(-1)),
            info.rating > 0,
        ),
    ]);

    let mut labels: Vec<Entry> = ColorLabel::ALL
        .into_iter()
        .map(|label| {
            toggle(
                label.name(),
                label_key(label).unwrap_or(""),
                change(LibraryChange::ToggleLabel(label)),
                info.label == Some(label),
                true,
            )
        })
        .collect();
    labels.extend([
        Entry::Separator,
        item(
            "No Label",
            "",
            change(LibraryChange::ClearLabel),
            info.label.is_some(),
        ),
    ]);

    let mut entries = vec![
        toggle(
            "Grid",
            "G",
            Action::ToggleGrid,
            library.grid && !library.survey,
            has_photo,
        ),
        toggle(
            "Survey",
            "N",
            Action::ToggleSurvey,
            library.survey,
            has_photo,
        ),
        Entry::Separator,
        submenu("Rating", has_photo, ratings),
        submenu("Label", has_photo, labels),
    ];
    for (label, shortcut, flag) in [
        ("Pick", "P", Flag::Pick),
        ("Reject", "X", Flag::Reject),
        ("Unflag", "U", Flag::None),
    ] {
        entries.push(toggle(
            label,
            shortcut,
            change(LibraryChange::Flag(flag)),
            flag != Flag::None && info.flag == flag,
            has_photo,
        ));
    }
    entries.extend([
        toggle(
            "Auto-Advance",
            "",
            Action::ToggleAutoAdvance,
            library.auto_advance,
            true,
        ),
        Entry::Separator,
        submenu("Filter", true, filter_entries(app)),
        submenu(
            "Sort By",
            true,
            SortOrder::ALL
                .into_iter()
                .map(|order| {
                    toggle(
                        order.label(),
                        "",
                        Action::Sort(order),
                        library.sort == order,
                        true,
                    )
                })
                .collect(),
        ),
        Entry::Separator,
        item(
            "Select All",
            theme::SHORTCUT_SELECT_ALL,
            Action::SelectAll,
            has_photo,
        ),
        item(
            "Select None",
            "",
            Action::SelectNone,
            !library.selection.is_empty(),
        ),
        Entry::Separator,
        item(
            format!("Move Rejected Photos to {BIN_NAME}…"),
            "",
            Action::MoveRejectedToBin,
            !app.rejected_photos().is_empty(),
        ),
    ]);
    entries
}

/// The keys that set each rating, from none to [`MAX_RATING`].
const RATING_KEYS: [&str; 6] = ["0", "1", "2", "3", "4", "5"];

fn filter_entries(app: &PhotonationApp) -> Vec<Entry> {
    let filter = &app.view.library.filter;
    let mut entries: Vec<Entry> = (0..=MAX_RATING)
        .map(|stars| {
            let label = match stars {
                0 => "Any Rating".to_string(),
                5 => "5 Stars".to_string(),
                n => format!("{n} Stars or More"),
            };
            toggle(
                label,
                "",
                Action::FilterMinRating(stars),
                filter.min_rating == stars,
                true,
            )
        })
        .collect();
    entries.push(Entry::Separator);
    entries.extend(FlagFilter::ALL.into_iter().map(|flag| {
        toggle(
            flag.label(),
            "",
            Action::FilterFlag(flag),
            filter.flag == flag,
            true,
        )
    }));
    entries.push(Entry::Separator);
    entries.extend(ColorLabel::ALL.into_iter().map(|label| {
        toggle(
            label.name(),
            "",
            Action::FilterLabel(label),
            filter.labels.contains(&label),
            true,
        )
    }));
    entries.push(Entry::Separator);
    entries.extend(EditedFilter::ALL.into_iter().map(|edited| {
        toggle(
            edited.label(),
            "",
            Action::FilterEdited(edited),
            filter.edited == edited,
            true,
        )
    }));
    entries.extend([
        Entry::Separator,
        item("Clear Filter", "", Action::ClearFilter, filter.is_active()),
    ]);
    entries
}

fn help_menu(native: bool) -> Vec<Entry> {
    let mut entries = vec![
        item("Photonation Help", "F1", Action::Help, true),
        item("Keyboard Shortcuts", "", Action::KeyboardShortcuts, true),
    ];
    if !native {
        entries.extend([Entry::Separator, about()]);
    }
    entries
}

impl PhotonationApp {
    /// Does what a menu item says.
    pub fn perform(&mut self, action: &Action, ctx: &egui::Context) {
        match action {
            Action::OpenFolder => self.pick_folder(),
            Action::OpenPhoto => self.pick_photo(),
            Action::Export => {
                self.view.export_all = false;
                self.view.show_export = true;
            }
            Action::ExportBatch => {
                self.view.export_all = true;
                self.view.show_export = true;
            }
            Action::Settings => self.view.show_settings = true,
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::CopySettings => self.view.presets.show_copy = true,
            Action::PasteSettings => self.paste_settings(),
            Action::ApplyPreset { group, name } => {
                if let Some(preset) = presets::find(self, group, name) {
                    self.apply_preset(&preset);
                }
            }
            Action::NewPreset => self.open_new_preset(),
            Action::ImportPresets => self.import_presets(),
            Action::ResetAdjustments => self.settings = Settings::default(),
            Action::FitToWindow => {
                self.view.zoom = Zoom::Fit;
                self.view.pan = egui::Vec2::ZERO;
            }
            Action::ActualPixels => {
                self.view.zoom = Zoom::Scale(1.0);
                self.view.pan = egui::Vec2::ZERO;
            }
            Action::ToggleCompare => self.view.compare = !self.view.compare,
            Action::ToggleFullScreen => self.toggle_full_screen(ctx),
            Action::ToggleWindowFullScreen => self.toggle_window_full_screen(ctx),
            Action::ToggleMasks => self.view.toggle_masks(),
            Action::ToggleCropping => self.view.toggle_cropping(),
            Action::ToggleRemovals => self.view.toggle_removals(),
            Action::PreviousPhoto => self.step(-1),
            Action::NextPhoto => self.step(1),
            Action::ToggleGrid => self.toggle_grid(),
            Action::ToggleSurvey => self.toggle_survey(),
            Action::Library(change) => self.change_library(*change),
            Action::ToggleAutoAdvance => {
                let advance = !self.view.library.auto_advance;
                self.view.library.auto_advance = advance;
                self.preferences.library.auto_advance = advance;
                self.preferences.save();
            }
            Action::FilterMinRating(stars) => self.view.library.filter.min_rating = *stars,
            Action::FilterFlag(flag) => self.view.library.filter.flag = *flag,
            Action::FilterLabel(label) => {
                let labels = &mut self.view.library.filter.labels;
                if !labels.remove(label) {
                    labels.insert(*label);
                }
            }
            Action::FilterEdited(edited) => self.view.library.filter.edited = *edited,
            Action::ClearFilter => self.view.library.filter = Default::default(),
            Action::Sort(order) => {
                self.view.library.sort = *order;
                self.preferences.library.sort = *order;
                self.preferences.save();
            }
            Action::SelectAll => self.select_all(),
            Action::SelectNone => {
                self.view.library.selection.clear();
                self.view.library.survey = false;
            }
            Action::MoveRejectedToBin => self.view.library.confirm_trash = true,
            Action::Help => help::open(&mut self.view),
            Action::KeyboardShortcuts => help::open_page(&mut self.view, help::Page::Shortcuts),
            Action::About => self.view.show_about = true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(entries: &[Entry]) -> Vec<&str> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Item(item) => Some(item.label.as_str()),
                Entry::Submenu { title, .. } => Some(title.as_str()),
                _ => None,
            })
            .collect()
    }

    fn titled<'a>(menus: &'a [Menu], title: &str) -> &'a Menu {
        menus
            .iter()
            .find(|menu| menu.title == title)
            .expect("the menu")
    }

    #[test]
    fn the_file_menu_opens_a_folder_or_a_photo_at_any_time() {
        let ctx = egui::Context::default();
        let app = PhotonationApp::empty(&ctx);
        for native in [false, true] {
            let menus = menus(&app, native);
            let file = &titled(&menus, "File").entries;
            for (label, action) in [
                (OPEN_FOLDER, Action::OpenFolder),
                (OPEN_PHOTO, Action::OpenPhoto),
            ] {
                let item = all_items(file)
                    .into_iter()
                    .find(|item| item.label == label)
                    .expect("the item");
                assert_eq!(item.action, action);
                assert!(item.enabled, "{label} needs nothing open");
            }
        }
    }

    fn all_items(entries: &[Entry]) -> Vec<&Item> {
        entries
            .iter()
            .flat_map(|entry| match entry {
                Entry::Item(item) => vec![item],
                Entry::Submenu { entries, .. } => all_items(entries),
                _ => Vec::new(),
            })
            .collect()
    }

    fn app() -> PhotonationApp {
        PhotonationApp::empty(&egui::Context::default())
    }

    #[test]
    fn in_the_window_the_menus_stay_as_they_were() {
        let menus = menus(&app(), false);
        let titles: Vec<_> = menus.iter().map(|menu| menu.title).collect();
        assert_eq!(titles, ["File", "Edit", "View", "Library", "Help"]);
        let file = labels(&titled(&menus, "File").entries);
        assert!(file.contains(&"Settings…") && file.contains(&"Quit"));
        assert!(labels(&titled(&menus, "Help").entries).contains(&"About Photonation"));
    }

    #[test]
    fn on_macos_the_menus_follow_its_conventions() {
        let menus = menus(&app(), true);
        let titles: Vec<_> = menus.iter().map(|menu| menu.title).collect();
        assert_eq!(
            titles,
            [
                "Photonation",
                "File",
                "Edit",
                "View",
                "Library",
                "Window",
                "Help"
            ]
        );
        let application = labels(&menus[0].entries);
        assert!(application.contains(&"About Photonation"));
        assert!(application.contains(&"Settings…"));
        assert!(application.contains(&"Quit Photonation"));
        let file = labels(&titled(&menus, "File").entries);
        assert!(!file.contains(&"Settings…") && !file.contains(&"Quit"));
        assert!(!labels(&titled(&menus, "Help").entries).contains(&"About Photonation"));
    }

    #[test]
    fn a_dialog_disables_everything_but_quit() {
        let mut app = app();
        app.view.show_settings = true;
        let menus = menus(&app, true);
        for item in menus.iter().flat_map(|menu| all_items(&menu.entries)) {
            assert_eq!(item.enabled, item.action == Action::Quit, "{}", item.label);
        }
    }

    #[test]
    fn items_for_a_photo_wait_for_one() {
        let menus = menus(&app(), true);
        let edit = all_items(&titled(&menus, "Edit").entries);
        let find = |label: &str| edit.iter().find(|item| item.label == label).expect(label);
        assert!(!find("Copy Settings…").enabled);
        assert!(find("Import Presets…").enabled);
    }

    #[test]
    fn choosing_items_changes_what_they_say() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.perform(&Action::ToggleCompare, &ctx);
        assert!(app.view.compare);
        app.perform(&Action::FilterLabel(ColorLabel::Red), &ctx);
        assert!(app.view.library.filter.labels.contains(&ColorLabel::Red));
        app.perform(&Action::FilterLabel(ColorLabel::Red), &ctx);
        assert!(app.view.library.filter.labels.is_empty());
        app.perform(&Action::Settings, &ctx);
        assert!(app.view.show_settings);
        app.perform(&Action::About, &ctx);
        assert!(app.view.show_about);
    }
}
