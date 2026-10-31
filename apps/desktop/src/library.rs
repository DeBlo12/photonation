// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Culling: the rating, flag and label of each photo, the filter and sort
//! that decide which photos the filmstrip, grid and arrow keys go through
//! and in which order, and the selection ratings apply to. The panels are
//! `panels/grid.rs`, `panels/survey.rs` and `panels/library_bar.rs`; see
//! docs/DESKTOP.md.

use crate::app::{Photo, PhotonationApp};
use eframe::egui::{self, Key, Modifiers};
use photonation_develop::{
    ColorLabel, Flag, LibraryInfo, MAX_RATING, lightroom_xmp_path, sidecar_path,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Most photos the survey view compares at once.
pub const MAX_SURVEY: usize = 8;
pub const GRID_SIZES: std::ops::RangeInclusive<f32> = 96.0..=320.0;
const DEFAULT_GRID_SIZE: f32 = 168.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlagFilter {
    #[default]
    Any,
    Picked,
    Unflagged,
    /// Everything but rejects: the usual view while culling.
    NotRejected,
    Rejected,
}

impl FlagFilter {
    pub const ALL: [FlagFilter; 5] = [
        FlagFilter::Any,
        FlagFilter::Picked,
        FlagFilter::Unflagged,
        FlagFilter::NotRejected,
        FlagFilter::Rejected,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FlagFilter::Any => "Any flag",
            FlagFilter::Picked => "Picked",
            FlagFilter::Unflagged => "Unflagged",
            FlagFilter::NotRejected => "Not rejected",
            FlagFilter::Rejected => "Rejected",
        }
    }

    fn matches(self, flag: Flag) -> bool {
        match self {
            FlagFilter::Any => true,
            FlagFilter::Picked => flag == Flag::Pick,
            FlagFilter::Unflagged => flag == Flag::None,
            FlagFilter::NotRejected => flag != Flag::Reject,
            FlagFilter::Rejected => flag == Flag::Reject,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditedFilter {
    #[default]
    Any,
    Edited,
    Unedited,
}

impl EditedFilter {
    pub const ALL: [EditedFilter; 3] = [
        EditedFilter::Any,
        EditedFilter::Edited,
        EditedFilter::Unedited,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EditedFilter::Any => "Edited or not",
            EditedFilter::Edited => "Edited",
            EditedFilter::Unedited => "Unedited",
        }
    }
}

/// Which photos show. Every part must match.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LibraryFilter {
    /// At least this many stars; 0 shows unrated photos too.
    pub min_rating: u8,
    pub flag: FlagFilter,
    /// Photos with one of these labels; empty shows every photo.
    pub labels: BTreeSet<ColorLabel>,
    pub edited: EditedFilter,
}

impl LibraryFilter {
    pub fn matches(&self, photo: &Photo) -> bool {
        let info = photo.library;
        info.rating >= self.min_rating
            && self.flag.matches(info.flag)
            && (self.labels.is_empty() || info.label.is_some_and(|l| self.labels.contains(&l)))
            && match self.edited {
                EditedFilter::Any => true,
                EditedFilter::Edited => photo.has_edits,
                EditedFilter::Unedited => !photo.has_edits,
            }
    }

    pub fn is_active(&self) -> bool {
        *self != LibraryFilter::default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SortOrder {
    #[default]
    FileName,
    CaptureTime,
    /// Most stars first.
    Rating,
    /// Edited photos first.
    Edited,
}

impl SortOrder {
    pub const ALL: [SortOrder; 4] = [
        SortOrder::FileName,
        SortOrder::CaptureTime,
        SortOrder::Rating,
        SortOrder::Edited,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortOrder::FileName => "File name",
            SortOrder::CaptureTime => "Capture time",
            SortOrder::Rating => "Rating",
            SortOrder::Edited => "Edited first",
        }
    }
}

/// The culling state of the open folder (view state: not saved with any
/// photo). Photo indices refer to `PhotonationApp::photos`.
pub struct LibraryView {
    pub filter: LibraryFilter,
    pub sort: SortOrder,
    /// The photos shown, in order: those the filter lets through, plus the
    /// current photo even when it no longer matches, so rating it away
    /// doesn't pull it from under the user. Rebuilt every frame.
    pub shown: Vec<usize>,
    /// Photos selected besides the current one (Ctrl- or Shift-click).
    pub selection: BTreeSet<usize>,
    /// The grid replaces the editor.
    pub grid: bool,
    /// The current and selected photos side by side, in place of the grid
    /// or the editor.
    pub survey: bool,
    /// Grid cell size, points.
    pub grid_size: f32,
    /// Columns the grid had last frame, for ↑ ↓.
    pub grid_columns: usize,
    /// The photo the grid last scrolled to show; when the current photo
    /// differs (it moved by keyboard), the grid scrolls to it.
    pub grid_followed: Option<usize>,
    /// The grid's scroll offset and height last frame.
    pub grid_viewport: (f32, f32),
    /// Move to the next photo after rating or flagging one.
    pub auto_advance: bool,
    /// Asking whether to move the rejected photos to the bin.
    pub confirm_trash: bool,
}

impl Default for LibraryView {
    fn default() -> Self {
        LibraryView {
            filter: LibraryFilter::default(),
            sort: SortOrder::default(),
            shown: Vec::new(),
            selection: BTreeSet::new(),
            grid: false,
            survey: false,
            grid_size: DEFAULT_GRID_SIZE,
            grid_columns: 1,
            grid_followed: None,
            grid_viewport: (0.0, 0.0),
            auto_advance: false,
            confirm_trash: false,
        }
    }
}

/// The photos `filter` lets through (and `current`), in `sort` order.
/// Ties keep file name order, as the folder was scanned.
pub fn arrange(
    photos: &[Photo],
    filter: &LibraryFilter,
    sort: SortOrder,
    current: Option<usize>,
) -> Vec<usize> {
    let mut shown: Vec<usize> = (0..photos.len())
        .filter(|&i| Some(i) == current || filter.matches(&photos[i]))
        .collect();
    match sort {
        SortOrder::FileName => {}
        // Photos without a capture time (not read yet) go last.
        SortOrder::CaptureTime => shown.sort_by(|&a, &b| {
            let key = |i: usize| (photos[i].captured.is_none(), photos[i].captured.as_deref());
            key(a).cmp(&key(b))
        }),
        SortOrder::Rating => shown.sort_by_key(|&i| std::cmp::Reverse(photos[i].library.rating)),
        SortOrder::Edited => shown.sort_by_key(|&i| !photos[i].has_edits),
    }
    shown
}

/// Photos from `from` to `to` in shown order, both included.
fn range(shown: &[usize], from: usize, to: usize) -> Vec<usize> {
    let position = |index| shown.iter().position(|&i| i == index);
    match (position(from), position(to)) {
        (Some(a), Some(b)) => shown[a.min(b)..=a.max(b)].to_vec(),
        _ => vec![to],
    }
}

/// "2026:06:20 18:02:23" as "20 Jun 2026, 18:02".
pub fn capture_time_label(captured: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (date, time) = captured.trim().split_once(' ')?;
    let mut parts = date.split(':');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    let month = MONTHS.get(month.parse::<usize>().ok()?.checked_sub(1)?)?;
    let day = day.parse::<u32>().ok()?;
    Some(format!("{day} {month} {year}, {}", time.get(..5)?))
}

/// A change to the rating, flag or label of the photos it applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryChange {
    Rating(u8),
    /// One star more (+1) or fewer (-1).
    RatingStep(i8),
    Flag(Flag),
    /// Sets the label, or clears it on photos that already have it.
    ToggleLabel(ColorLabel),
    ClearLabel,
}

impl LibraryChange {
    fn apply(self, info: &mut LibraryInfo) {
        match self {
            LibraryChange::Rating(rating) => info.rating = rating.min(MAX_RATING),
            LibraryChange::RatingStep(step) => {
                info.rating = info.rating.saturating_add_signed(step).min(MAX_RATING)
            }
            LibraryChange::Flag(flag) => info.flag = flag,
            LibraryChange::ToggleLabel(label) => {
                info.label = (info.label != Some(label)).then_some(label)
            }
            LibraryChange::ClearLabel => info.label = None,
        }
    }

    /// Ratings and flags move on with auto-advance; labels don't.
    fn advances(self) -> bool {
        matches!(
            self,
            LibraryChange::Rating(_) | LibraryChange::RatingStep(_) | LibraryChange::Flag(_)
        )
    }
}

impl PhotonationApp {
    /// Rebuilds the shown photos for this frame.
    pub fn arrange_photos(&mut self) {
        let library = &mut self.view.library;
        library.shown = arrange(&self.photos, &library.filter, library.sort, self.current);
        library.selection.retain(|&i| i < self.photos.len());
        if self.photos.is_empty() {
            library.grid = false;
            library.survey = false;
        }
    }

    /// The photos a rating applies to: the current one, and in the grid
    /// and filmstrip the selected ones too. The survey view rates only the
    /// current photo, so one of a burst can be rejected.
    pub fn rating_targets(&self) -> Vec<usize> {
        let Some(current) = self.current else {
            return Vec::new();
        };
        let mut targets: BTreeSet<usize> = [current].into();
        if !self.view.library.survey {
            targets.extend(&self.view.library.selection);
        }
        targets.into_iter().collect()
    }

    /// The current and selected photos in shown order: what the survey
    /// view compares (at most [`MAX_SURVEY`]).
    pub fn survey_photos(&self) -> Vec<usize> {
        let library = &self.view.library;
        let in_survey = |i: &usize| Some(*i) == self.current || library.selection.contains(i);
        let mut photos: Vec<usize> = library.shown.iter().copied().filter(in_survey).collect();
        photos.truncate(MAX_SURVEY);
        photos
    }

    /// Applies `change` to the rating targets, saves their sidecars, and
    /// moves on if auto-advance is on and one photo was rated.
    pub fn change_library(&mut self, change: LibraryChange) {
        let targets = self.rating_targets();
        for &index in &targets {
            let before = self.photos[index].library;
            change.apply(&mut self.photos[index].library);
            if self.photos[index].library != before {
                self.save_library(index);
            }
        }
        if self.view.library.auto_advance && change.advances() && targets.len() == 1 {
            self.step(1);
        }
    }

    /// Moves `delta` photos through the shown photos (the survey's photos
    /// in the survey view), clearing the selection outside the survey.
    pub fn step(&mut self, delta: isize) {
        let Some(current) = self.current else {
            return;
        };
        let order = if self.view.library.survey {
            self.survey_photos()
        } else {
            self.view.library.shown.clone()
        };
        let Some(position) = order.iter().position(|&i| i == current) else {
            return;
        };
        let target = position.saturating_add_signed(delta).min(order.len() - 1);
        if target != position {
            if !self.view.library.survey {
                self.view.library.selection.clear();
            }
            self.select(order[target]);
        }
    }

    /// A click on a photo in the grid or filmstrip: a plain click makes it
    /// the current photo alone, Ctrl (⌘ on a Mac) adds or removes it, and
    /// Shift selects the shown photos from the current one to it.
    pub fn click_photo(&mut self, index: usize, modifiers: Modifiers) {
        let library = &mut self.view.library;
        let Some(current) = self.current else {
            self.select(index);
            return;
        };
        if modifiers.shift {
            library.selection = range(&library.shown, current, index).into_iter().collect();
            library.selection.remove(&current);
        } else if modifiers.command {
            if index == current {
                // The current photo leaves the selection: another takes over.
                if let Some(&next) = library.selection.iter().next() {
                    library.selection.remove(&next);
                    self.select(next);
                }
            } else if !library.selection.remove(&index) {
                library.selection.insert(current);
                library.selection.remove(&index);
                self.select(index);
            }
        } else {
            library.selection.clear();
            self.select(index);
        }
    }

    /// Makes a photo of the survey the current one; the others stay.
    pub fn focus_in_survey(&mut self, index: usize) {
        let Some(current) = self.current.filter(|&current| current != index) else {
            return;
        };
        let selection = &mut self.view.library.selection;
        selection.insert(current);
        selection.remove(&index);
        self.select(index);
    }

    /// Takes a photo out of the survey (and the selection), which ends
    /// once fewer than two are left.
    pub fn remove_from_survey(&mut self, index: usize) {
        if self.current == Some(index) {
            let next = self.survey_photos().into_iter().find(|&i| i != index);
            if let Some(next) = next {
                self.view.library.selection.remove(&next);
                self.select(next);
            }
        } else {
            self.view.library.selection.remove(&index);
        }
        if self.survey_photos().len() < 2 {
            self.view.library.survey = false;
        }
    }

    /// Every shown photo selected.
    pub fn select_all(&mut self) {
        let library = &mut self.view.library;
        library.selection = library.shown.iter().copied().collect();
        if let Some(current) = self.current {
            library.selection.remove(&current);
        } else if let Some(&first) = library.shown.first() {
            self.select(first);
        }
    }

    /// Shows or leaves the grid; the editor's tools close with it.
    pub fn toggle_grid(&mut self) {
        let library = &mut self.view.library;
        library.grid = !library.grid && !self.photos.is_empty();
        library.survey = false;
    }

    /// Shows or leaves the survey view. It needs two photos or more.
    pub fn toggle_survey(&mut self) {
        if self.view.library.survey {
            self.view.library.survey = false;
        } else if self.survey_photos().len() >= 2 {
            self.view.library.survey = true;
        } else {
            self.notify(format!(
                "Select 2 to {MAX_SURVEY} photos to compare them: {}-click or Shift-click",
                crate::theme::COMMAND_KEY
            ));
        }
    }

    /// The rejected photos of the folder.
    pub fn rejected_photos(&self) -> Vec<usize> {
        (0..self.photos.len())
            .filter(|&i| self.photos[i].library.flag == Flag::Reject)
            .collect()
    }

    /// Moves the rejected photos to the system's bin with their sidecars
    /// (a Lightroom sidecar only when no photo that stays shares it), then
    /// reads the folder again, on the current photo if it stayed or the
    /// next one shown that did.
    pub fn trash_rejected(&mut self) {
        let Some(folder) = self.folder.clone() else {
            return;
        };
        self.save_now();
        let rejected = self.rejected_photos();
        let staying_xmp: BTreeSet<PathBuf> = (0..self.photos.len())
            .filter(|i| !rejected.contains(i))
            .map(|i| lightroom_xmp_path(&self.photos[i].path))
            .collect();
        let mut files = Vec::new();
        for &index in &rejected {
            let path = &self.photos[index].path;
            files.push(path.clone());
            let xmp = lightroom_xmp_path(path);
            for sidecar in [sidecar_path(path), xmp.clone()] {
                if sidecar.exists() && (sidecar != xmp || !staying_xmp.contains(&xmp)) {
                    files.push(sidecar);
                }
            }
        }
        files.dedup();
        // The photo to land on: the current one, or the next shown keeper.
        let shown = &self.view.library.shown;
        let start = self
            .current
            .and_then(|current| shown.iter().position(|&i| i == current))
            .unwrap_or(0);
        let land_on = shown
            .iter()
            .skip(start)
            .chain(shown.iter().take(start).rev())
            .find(|i| !rejected.contains(i))
            .map(|&i| self.photos[i].path.clone());
        let bin = crate::panels::library_bar::BIN_NAME;
        match trash::delete_all(&files) {
            Ok(()) => {
                self.open_folder(folder);
                let moved = rejected.len();
                self.notify(if moved == 1 {
                    format!("Moved 1 photo to the {bin}")
                } else {
                    format!("Moved {moved} photos to the {bin}")
                });
            }
            Err(e) => {
                // Some files may have gone; show the folder as it is now.
                self.open_folder(folder);
                self.notify(format!(
                    "Could not move every rejected photo to the {bin}: {e}"
                ));
            }
        }
        if let Some(index) =
            land_on.and_then(|path| self.photos.iter().position(|p| p.path == path))
        {
            self.select(index);
        }
    }
}

/// Culling keys: stars, flags, labels, the grid and survey views and
/// selection. They act whenever no text field has the keyboard.
pub fn shortcuts(app: &mut PhotonationApp, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() || app.current.is_none() {
        return;
    }
    let grid = app.view.library.grid && !app.view.library.survey;
    let survey = app.view.library.survey;
    let columns = app.view.library.grid_columns.max(1) as isize;
    let mut changes = Vec::new();
    let mut steps = Vec::new();
    let (mut toggle_grid, mut toggle_survey, mut open, mut select_all, mut escape) =
        (false, false, false, false, false);
    ctx.input_mut(|i| {
        let mut plain = |key| i.consume_key(Modifiers::NONE, key);
        let ratings = [
            Key::Num0,
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
        ];
        for (stars, key) in ratings.into_iter().enumerate() {
            if plain(key) {
                changes.push(LibraryChange::Rating(stars as u8));
            }
        }
        let labels = [Key::Num6, Key::Num7, Key::Num8, Key::Num9];
        for (label, key) in ColorLabel::ALL.into_iter().zip(labels) {
            if plain(key) {
                changes.push(LibraryChange::ToggleLabel(label));
            }
        }
        for (key, change) in [
            (Key::P, LibraryChange::Flag(Flag::Pick)),
            (Key::X, LibraryChange::Flag(Flag::Reject)),
            (Key::U, LibraryChange::Flag(Flag::None)),
            (Key::CloseBracket, LibraryChange::RatingStep(1)),
            (Key::OpenBracket, LibraryChange::RatingStep(-1)),
        ] {
            if plain(key) {
                changes.push(change);
            }
        }
        toggle_grid = plain(Key::G);
        toggle_survey = plain(Key::N);
        open = plain(Key::E) || ((grid || survey) && plain(Key::Enter));
        escape = (grid || survey) && plain(Key::Escape);
        if grid || survey {
            // The editor's ← → are handled with its other keys.
            for (key, delta) in [(Key::ArrowLeft, -1), (Key::ArrowRight, 1)] {
                if plain(key) {
                    steps.push(delta);
                }
            }
        }
        if grid {
            for (key, delta) in [(Key::ArrowUp, -columns), (Key::ArrowDown, columns)] {
                if plain(key) {
                    steps.push(delta);
                }
            }
            select_all = i.consume_key(Modifiers::COMMAND, Key::A);
        }
    });
    for change in changes {
        app.change_library(change);
    }
    for delta in steps {
        app.step(delta);
    }
    if select_all {
        app.select_all();
    }
    if toggle_grid {
        app.toggle_grid();
    }
    if toggle_survey {
        app.toggle_survey();
    }
    if escape {
        if survey {
            app.view.library.survey = false;
        } else {
            app.view.library.selection.clear();
        }
    }
    if open {
        app.view.library.grid = false;
        app.view.library.survey = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(name: &str, rating: u8, flag: Flag, captured: Option<&str>) -> Photo {
        Photo {
            library: LibraryInfo {
                rating,
                flag,
                label: None,
            },
            captured: captured.map(String::from),
            ..Photo::new(name.into())
        }
    }

    fn folder() -> Vec<Photo> {
        vec![
            photo("a.CR3", 0, Flag::None, Some("2026:06:20 18:02:30")),
            photo("b.CR3", 4, Flag::Pick, Some("2026:06:20 18:02:10")),
            photo("c.CR3", 2, Flag::Reject, None),
            photo("d.CR3", 5, Flag::None, Some("2026:06:19 09:00:00")),
        ]
    }

    #[test]
    fn filters_combine() {
        let photos = folder();
        let filter = LibraryFilter {
            min_rating: 2,
            flag: FlagFilter::NotRejected,
            ..LibraryFilter::default()
        };
        assert_eq!(arrange(&photos, &filter, SortOrder::FileName, None), [1, 3]);
        let rejects = LibraryFilter {
            flag: FlagFilter::Rejected,
            ..LibraryFilter::default()
        };
        assert_eq!(arrange(&photos, &rejects, SortOrder::FileName, None), [2]);
    }

    #[test]
    fn the_current_photo_stays_shown() {
        let photos = folder();
        let picks = LibraryFilter {
            flag: FlagFilter::Picked,
            ..LibraryFilter::default()
        };
        assert_eq!(
            arrange(&photos, &picks, SortOrder::FileName, Some(0)),
            [0, 1]
        );
    }

    #[test]
    fn sorts_keep_file_order_for_ties() {
        let photos = folder();
        let all = LibraryFilter::default();
        assert_eq!(
            arrange(&photos, &all, SortOrder::CaptureTime, None),
            [3, 1, 0, 2],
            "unknown capture times last"
        );
        assert_eq!(
            arrange(&photos, &all, SortOrder::Rating, None),
            [3, 1, 2, 0]
        );
    }

    #[test]
    fn labels_filter_by_any_of_them() {
        let mut photos = folder();
        photos[1].library.label = Some(ColorLabel::Red);
        photos[2].library.label = Some(ColorLabel::Blue);
        let filter = LibraryFilter {
            labels: [ColorLabel::Red, ColorLabel::Green].into(),
            ..LibraryFilter::default()
        };
        assert_eq!(arrange(&photos, &filter, SortOrder::FileName, None), [1]);
    }

    #[test]
    fn changes_stay_in_range_and_labels_toggle() {
        let mut info = LibraryInfo::default();
        LibraryChange::RatingStep(-1).apply(&mut info);
        assert_eq!(info.rating, 0);
        LibraryChange::Rating(9).apply(&mut info);
        assert_eq!(info.rating, MAX_RATING);
        LibraryChange::RatingStep(1).apply(&mut info);
        assert_eq!(info.rating, MAX_RATING);
        LibraryChange::ToggleLabel(ColorLabel::Red).apply(&mut info);
        assert_eq!(info.label, Some(ColorLabel::Red));
        LibraryChange::ToggleLabel(ColorLabel::Blue).apply(&mut info);
        assert_eq!(info.label, Some(ColorLabel::Blue));
        LibraryChange::ToggleLabel(ColorLabel::Blue).apply(&mut info);
        assert_eq!(info.label, None);
    }

    #[test]
    fn ranges_follow_the_shown_order() {
        assert_eq!(range(&[3, 1, 0, 2], 1, 2), [1, 0, 2]);
        assert_eq!(range(&[3, 1, 0, 2], 2, 3), [3, 1, 0, 2]);
        assert_eq!(range(&[3, 1], 1, 7), [7], "a photo not shown");
    }

    #[test]
    fn capture_times_read_as_dates() {
        assert_eq!(
            capture_time_label("2026:06:20 18:02:23").as_deref(),
            Some("20 Jun 2026, 18:02")
        );
        assert_eq!(capture_time_label("2026:13:20 18:02:23"), None);
        assert_eq!(capture_time_label("garbage"), None);
    }
}
