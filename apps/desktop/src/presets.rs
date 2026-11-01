// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Copy and paste of settings, and presets: the built-in ones and the
//! user's, kept as files in the `presets` folder of the engine's data
//! directory. Both apply to the current photo and the selected ones, like
//! ratings. The panels are in `panels::presets`.

use crate::app::PhotonationApp;
use photonation_develop::{
    BUILT_IN_PRESET_GROUP, Preset, Settings, SettingsParts, StoredPresetFile, built_in_presets,
    default_copy_parts, default_preset_parts, import_preset_file, load_photo, load_presets,
    save_new_preset, save_photo,
};
use std::path::PathBuf;

/// The New Preset dialog's fields.
#[derive(Clone, Debug, PartialEq)]
pub struct NewPreset {
    pub name: String,
    pub group: String,
    pub parts: SettingsParts,
}

/// Presets and copied settings (not saved with a photo).
pub struct PresetsView {
    pub built_in: Vec<Preset>,
    /// The user's presets, by group and name.
    pub user: Vec<StoredPresetFile>,
    /// Settings copied with Copy Settings, and the parts Paste applies.
    pub copied: Option<(Settings, SettingsParts)>,
    /// The parts Copy Settings offers; remembered for the next copy.
    pub copy_parts: SettingsParts,
    /// The Copy Settings dialog is open.
    pub show_copy: bool,
    /// The New Preset dialog, while open.
    pub new_preset: Option<NewPreset>,
    /// The parts the last new preset was saved with.
    pub preset_parts: SettingsParts,
    /// The settings shown while the pointer is over a preset, for this
    /// frame only.
    pub hover: Option<Settings>,
    /// The Presets card is open.
    pub expanded: bool,
    /// Groups folded in the Presets card.
    pub folded: std::collections::BTreeSet<String>,
    /// A preset file whose deletion waits for confirmation.
    pub confirm_delete: Option<StoredPresetFile>,
}

impl Default for PresetsView {
    fn default() -> Self {
        PresetsView {
            built_in: built_in_presets(),
            user: Vec::new(),
            copied: None,
            copy_parts: default_copy_parts(),
            show_copy: false,
            new_preset: None,
            preset_parts: default_preset_parts(),
            hover: None,
            expanded: false,
            folded: Default::default(),
            confirm_delete: None,
        }
    }
}

impl PresetsView {
    /// Every preset, in listing order: the user's groups, then the
    /// built-in ones.
    pub fn groups(&self) -> Vec<(String, Vec<PresetRef<'_>>)> {
        let mut groups: Vec<(String, Vec<PresetRef<'_>>)> = Vec::new();
        let user = self.user.iter().map(PresetRef::User);
        let built_in = self.built_in.iter().map(PresetRef::BuiltIn);
        for preset in user.chain(built_in) {
            let group = &preset.preset().group;
            match groups.last_mut() {
                Some((name, members)) if name == group => members.push(preset),
                _ => groups.push((group.clone(), vec![preset])),
            }
        }
        groups
    }
}

/// A preset as listed: built in, or a file of the user's.
#[derive(Clone, Copy, Debug)]
pub enum PresetRef<'a> {
    BuiltIn(&'a Preset),
    User(&'a StoredPresetFile),
}

impl<'a> PresetRef<'a> {
    pub fn preset(self) -> &'a Preset {
        match self {
            PresetRef::BuiltIn(preset) => preset,
            PresetRef::User(stored) => &stored.preset,
        }
    }
}

/// The folder user presets are kept in.
pub fn presets_folder() -> Option<PathBuf> {
    photonation_engine::data_directory().map(|data| data.join("presets"))
}

/// Where Lightroom Classic and Camera Raw keep develop presets, if there.
fn lightroom_presets_folder() -> Option<PathBuf> {
    let folder = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home).join("Library/Application Support/Adobe/CameraRaw/Settings")
        })
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(|appdata| PathBuf::from(appdata).join("Adobe/CameraRaw/Settings"))
    } else {
        None
    };
    folder.filter(|folder| folder.is_dir())
}

fn photos(count: usize) -> String {
    if count == 1 {
        "1 photo".into()
    } else {
        format!("{count} photos")
    }
}

impl PhotonationApp {
    /// Reads the user's presets from the presets folder.
    pub fn load_presets(&mut self) {
        let Some(folder) = presets_folder() else {
            return;
        };
        let (presets, errors) = load_presets(&folder);
        self.view.presets.user = presets;
        if let Some(error) = errors.first() {
            self.notify(format!("A preset could not be read: {error}"));
        }
    }

    /// Applies `change` to the current photo and the selected ones (see
    /// `rating_targets`). The current photo's change can be undone; the
    /// others' sidecars are written at once. Returns how many changed.
    fn change_settings(&mut self, change: impl Fn(&Settings) -> Settings) -> usize {
        let targets = self.rating_targets();
        let mut changed = 0;
        for index in targets {
            if self.current == Some(index) {
                let new = change(&self.settings);
                if new != self.settings {
                    if new.removals != self.settings.removals {
                        self.filling_removals = new.removals.iter().any(|r| r.is_active());
                    }
                    self.settings = new;
                    self.history.commit(&self.settings);
                    self.edited();
                    changed += 1;
                }
                continue;
            }
            let path = self.photos[index].path.clone();
            // A sidecar that can't be read is left alone, not replaced.
            let result = load_photo(&path).and_then(|loaded| {
                let old = loaded.settings.map(|s| s.settings).unwrap_or_default();
                let new = change(&old);
                if new == old {
                    return Ok(None);
                }
                save_photo(&path, &new, &loaded.library).map(|()| Some(new))
            });
            match result {
                Ok(Some(new)) => {
                    self.photos[index].has_edits = new != Settings::default();
                    self.engine.refresh_thumbnail(path.clone(), new);
                    self.survey_images.remove(&path);
                    self.survey_requested.remove(&path);
                    changed += 1;
                }
                Ok(None) => {}
                Err(e) => self.notify(e.to_string()),
            }
        }
        changed
    }

    /// Applies `preset` to the current and selected photos.
    pub fn apply_preset(&mut self, preset: &Preset) {
        let targets = self.rating_targets().len();
        self.change_settings(|settings| preset.applied_to(settings));
        if targets > 1 {
            self.notify(format!("Applied “{}” to {}", preset.name, photos(targets)));
        }
    }

    /// Shows the photo with `preset` applied while the pointer rests on it.
    pub fn preview_preset(&mut self, preset: &Preset) {
        if self.current.is_some() {
            self.view.presets.hover = Some(preset.applied_to(&self.effective_settings()));
        }
    }

    /// Remembers `parts` of the current photo's settings for Paste.
    pub fn copy_settings(&mut self, parts: SettingsParts) {
        if self.current.is_none() {
            return;
        }
        self.view.presets.copy_parts = parts.clone();
        self.view.presets.copied = Some((self.settings.clone(), parts));
        self.notify("Settings copied");
    }

    /// Pastes the copied settings onto the current and selected photos.
    pub fn paste_settings(&mut self) {
        let Some((copied, parts)) = self.view.presets.copied.clone() else {
            return;
        };
        let targets = self.rating_targets().len();
        self.change_settings(|settings| {
            let mut pasted = settings.clone();
            photonation_develop::copy_parts(&copied, &mut pasted, &parts);
            pasted
        });
        if targets > 1 {
            self.notify(format!("Pasted settings to {}", photos(targets)));
        }
    }

    /// Opens the New Preset dialog with the parts used last time.
    pub fn open_new_preset(&mut self) {
        if self.current.is_none() {
            return;
        }
        let presets = &mut self.view.presets;
        presets.new_preset = Some(NewPreset {
            name: String::new(),
            group: presets
                .user
                .last()
                .map_or(photonation_develop::DEFAULT_PRESET_GROUP.into(), |p| {
                    p.preset.group.clone()
                }),
            parts: presets.preset_parts.clone(),
        });
    }

    /// Saves the current photo's settings as a new preset.
    pub fn save_preset(&mut self, new: &NewPreset) {
        let Some(folder) = presets_folder() else {
            self.notify("Presets need a data folder, and none was found");
            return;
        };
        let preset = Preset::new(&new.name, &new.group, &self.settings, new.parts.clone());
        if preset.group == BUILT_IN_PRESET_GROUP {
            self.notify(format!(
                "“{BUILT_IN_PRESET_GROUP}” is kept for the built-in presets"
            ));
            return;
        }
        self.view.presets.preset_parts = new.parts.clone();
        match save_new_preset(&folder, &preset) {
            Ok(_) => {
                self.view.presets.folded.remove(&preset.group);
                self.load_presets();
                self.notify(format!("Saved the preset “{}”", preset.name));
            }
            Err(e) => self.notify(e.to_string()),
        }
    }

    /// Asks for Lightroom (`.xmp`) or Photonation (`.json`) preset files
    /// and copies them into the presets folder.
    pub fn import_presets(&mut self) {
        let Some(folder) = presets_folder() else {
            self.notify("Presets need a data folder, and none was found");
            return;
        };
        let mut dialog = rfd::FileDialog::new()
            .set_title("Import presets")
            .add_filter("Lightroom or Photonation presets", &["xmp", "json"]);
        if let Some(lightroom) = lightroom_presets_folder() {
            dialog = dialog.set_directory(lightroom);
        }
        let Some(files) = dialog.pick_files() else {
            return;
        };
        let (mut imported, mut partial, mut failed) = (0, 0, Vec::new());
        for file in files {
            let result = import_preset_file(&file).and_then(|(preset, not_imported)| {
                save_new_preset(&folder, &preset).map(|_| not_imported)
            });
            match result {
                Ok(not_imported) => {
                    imported += 1;
                    partial += usize::from(!not_imported.is_empty());
                }
                Err(e) => failed.push(e.to_string()),
            }
        }
        self.load_presets();
        let presets = |n: usize| {
            if n == 1 {
                "1 preset".into()
            } else {
                format!("{n} presets")
            }
        };
        let mut message = format!("Imported {}", presets(imported));
        if partial > 0 {
            message.push_str(&format!(
                "; {} use Lightroom features Photonation lacks, like profiles, and look different",
                presets(partial)
            ));
        }
        if let Some(first) = failed.first() {
            message.push_str(&format!("; {} failed: {first}", failed.len()));
        }
        self.notify(message);
    }

    /// Moves a user preset's file to the system's bin.
    pub fn delete_preset(&mut self, stored: &StoredPresetFile) {
        match trash::delete(&stored.path) {
            Ok(()) => {
                self.load_presets();
                self.notify(format!(
                    "Moved the preset “{}” to the {}",
                    stored.preset.name,
                    crate::panels::library_bar::BIN_NAME
                ));
            }
            Err(e) => self.notify(format!("Could not delete the preset: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, Modifiers};
    use photonation_develop::SettingsPart;

    /// An app on a folder of `count` (unreadable) raw files.
    fn app_on_folder(count: usize) -> (PhotonationApp, tempfile::TempDir) {
        let folder = tempfile::tempdir().expect("temporary folder");
        for i in 0..count {
            std::fs::write(folder.path().join(format!("IMG_{i}.CR3")), b"not a raw")
                .expect("write photo");
        }
        let mut app = PhotonationApp::empty(&egui::Context::default());
        app.open_folder(folder.path().to_path_buf());
        app.arrange_photos();
        (app, folder)
    }

    fn saved(app: &PhotonationApp, index: usize) -> Settings {
        load_photo(&app.photos[index].path)
            .expect("read sidecar")
            .settings
            .map(|s| s.settings)
            .unwrap_or_default()
    }

    #[test]
    fn pasting_applies_the_copied_parts_to_the_selection() {
        let (mut app, _folder) = app_on_folder(4);
        app.settings.exposure = 1.0;
        app.settings.contrast = 20.0;
        app.copy_settings([SettingsPart::Exposure].into());

        app.click_photo(1, Modifiers::NONE);
        app.settings.contrast = -30.0;
        app.edited();
        app.click_photo(2, Modifiers::COMMAND);
        app.paste_settings();
        assert_eq!(app.settings.exposure, 1.0, "the current photo");
        assert!(app.history.can_undo(), "the current photo's paste undoes");
        let other = saved(&app, 1);
        assert_eq!(
            (other.exposure, other.contrast),
            (1.0, -30.0),
            "contrast not copied"
        );
        assert!(app.photos[1].has_edits);
        assert_eq!(saved(&app, 3).exposure, 0.0, "not selected");
    }

    #[test]
    fn presets_apply_to_the_selection_and_preview_without_editing() {
        let (mut app, _folder) = app_on_folder(3);
        let vivid = app
            .view
            .presets
            .built_in
            .iter()
            .find(|p| p.name == "Vivid")
            .cloned()
            .expect("built in");
        app.settings.exposure = 0.5;
        app.preview_preset(&vivid);
        let shown = app.view.presets.hover.clone().expect("previewed");
        assert_eq!((shown.exposure, shown.vibrance), (0.5, 35.0));
        assert_eq!(app.settings.vibrance, 0.0, "a preview doesn't edit");

        app.click_photo(1, Modifiers::COMMAND);
        app.apply_preset(&vivid);
        assert_eq!(app.settings.vibrance, 35.0);
        assert_eq!(saved(&app, 0).vibrance, 35.0, "selected");
        assert_eq!(saved(&app, 2).vibrance, 0.0, "not selected");
    }

    #[test]
    fn groups_list_user_presets_before_built_in_ones() {
        let mut view = PresetsView::default();
        let mine = Preset::new(
            "Mine",
            "Travel",
            &Settings::default(),
            default_preset_parts(),
        );
        view.user.push(StoredPresetFile {
            path: PathBuf::from("Mine.json"),
            preset: mine,
        });
        let groups = view.groups();
        assert_eq!(groups[0].0, "Travel");
        assert_eq!(groups[1].0, BUILT_IN_PRESET_GROUP);
        assert_eq!(groups[1].1.len(), view.built_in.len());
    }
}
