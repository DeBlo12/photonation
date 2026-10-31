// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The bar above the grid and survey views: the two views, how many
//! photos show, the filter (stars, flag, labels, edited), the sort order,
//! auto-advance and the grid's cell size. Also the confirmation before
//! rejected photos go to the bin.

use crate::app::PhotonationApp;
use crate::library::{EditedFilter, FlagFilter, GRID_SIZES, SortOrder};
use crate::panels::{hover_tip, icon_button, tool_button};
use crate::theme;
use crate::widgets::{Icon, label_color};
use eframe::egui::{self, Align, Layout, RichText, Sense, Stroke, vec2};
use photonation_develop::{ColorLabel, MAX_RATING};

pub const LIBRARY_BAR_HEIGHT: f32 = 38.0;

/// What the system calls its bin.
pub const BIN_NAME: &str = if cfg!(target_os = "macos") {
    "Trash"
} else {
    "Recycle Bin"
};

pub fn show(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    let remembered = crate::preferences::LibraryPreferences::from_view(&app.view.library);
    egui::Panel::top("library-bar")
        .exact_size(LIBRARY_BAR_HEIGHT)
        .show_separator_line(false)
        .frame(
            egui::Frame::NONE
                .fill(theme::WINDOW)
                .inner_margin(egui::Margin::symmetric(12, 4)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                views(app, ui);
                ui.add_space(10.0);
                count(app, ui);
                ui.add_space(10.0);
                filter(app, ui);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if app.view.library.grid && !app.view.library.survey {
                        ui.add_sized(
                            vec2(110.0, 20.0),
                            egui::Slider::new(&mut app.view.library.grid_size, GRID_SIZES)
                                .show_value(false),
                        )
                        .on_hover_text("Thumbnail size");
                        ui.add_space(8.0);
                    }
                    let library = &mut app.view.library;
                    let advance_tip = "Auto-advance\nMoves to the next photo after you rate, \
                                       pick or reject one, so a folder can be culled with \
                                       one key per photo.";
                    if tool_button(ui, "Auto-advance", library.auto_advance, advance_tip).clicked()
                    {
                        library.auto_advance = !library.auto_advance;
                    }
                    ui.add_space(6.0);
                    egui::ComboBox::from_id_salt("library-sort")
                        .selected_text(format!("Sort: {}", library.sort.label()))
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for order in SortOrder::ALL {
                                ui.selectable_value(&mut library.sort, order, order.label());
                            }
                        });
                });
            });
        });
    let now = crate::preferences::LibraryPreferences::from_view(&app.view.library);
    if now != remembered {
        app.preferences.library = now;
        app.preferences.save();
    }
}

fn views(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    let library = &app.view.library;
    let (grid, survey) = (library.grid && !library.survey, library.survey);
    let grid_tip = "Grid (G)\nAll shown photos at once. Click to choose one, Ctrl- or \
                    Shift-click to select several; double-click or E opens it in the editor.";
    if icon_button(ui, Icon::Grid, Some("Grid"), grid, grid_tip).clicked() {
        app.view.library.grid = true;
        app.view.library.survey = false;
    }
    let survey_tip = "Survey (N)\nThe selected photos side by side, to choose the best of a \
                      burst. ← → move between them; ratings and flags apply to the one \
                      outlined.";
    if icon_button(ui, Icon::Survey, Some("Survey"), survey, survey_tip).clicked() && !survey {
        app.toggle_survey();
    }
}

fn count(app: &PhotonationApp, ui: &mut egui::Ui) {
    let library = &app.view.library;
    let total = app.photos.len();
    let shown = library.shown.len();
    let mut text = if library.filter.is_active() {
        format!("{shown} of {total} photos")
    } else {
        format!("{total} photos")
    };
    let selected = library.selection.len() + usize::from(app.current.is_some());
    if selected > 1 {
        text.push_str(&format!(" · {selected} selected"));
    }
    ui.label(RichText::new(text).size(12.5).color(theme::TEXT_SECONDARY));
}

fn filter(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    let filter = &mut app.view.library.filter;
    let rating_text = |stars: u8| match stars {
        0 => "Any rating".to_string(),
        5 => "5 stars".to_string(),
        n => format!("{n}+ stars"),
    };
    egui::ComboBox::from_id_salt("library-rating")
        .selected_text(rating_text(filter.min_rating))
        .width(100.0)
        .show_ui(ui, |ui| {
            for stars in 0..=MAX_RATING {
                ui.selectable_value(&mut filter.min_rating, stars, rating_text(stars));
            }
        });
    egui::ComboBox::from_id_salt("library-flag")
        .selected_text(filter.flag.label())
        .width(110.0)
        .show_ui(ui, |ui| {
            for flag in FlagFilter::ALL {
                ui.selectable_value(&mut filter.flag, flag, flag.label());
            }
        });
    ui.add_space(4.0);
    for label in ColorLabel::ALL {
        let (rect, response) = ui.allocate_exact_size(vec2(16.0, 22.0), Sense::click());
        let on = filter.labels.contains(&label);
        let color = label_color(label);
        let radius = if on || response.hovered() { 5.5 } else { 4.0 };
        ui.painter().circle_filled(
            rect.center(),
            radius,
            if on { color } else { color.gamma_multiply(0.5) },
        );
        if on {
            ui.painter()
                .circle_stroke(rect.center(), radius + 1.5, Stroke::new(1.0, theme::TEXT));
        }
        let tip = format!(
            "{} label\nShows photos with this label (or any of the labels switched on).",
            label.name()
        );
        if hover_tip(response, &tip).clicked() && !filter.labels.remove(&label) {
            filter.labels.insert(label);
        }
    }
    ui.add_space(4.0);
    egui::ComboBox::from_id_salt("library-edited")
        .selected_text(filter.edited.label())
        .width(110.0)
        .show_ui(ui, |ui| {
            for edited in EditedFilter::ALL {
                ui.selectable_value(&mut filter.edited, edited, edited.label());
            }
        });
    if filter.is_active() {
        ui.add_space(4.0);
        if tool_button(
            ui,
            "Clear",
            false,
            "Clear the filter\nShows every photo again.",
        )
        .clicked()
        {
            *filter = Default::default();
        }
    }
}

/// Asks before moving the rejected photos to the bin.
pub fn confirm_trash(app: &mut PhotonationApp, ctx: &egui::Context) {
    let rejected = app.rejected_photos().len();
    let mut close = rejected == 0;
    let mut confirmed = false;
    let modal = egui::Modal::new(egui::Id::unique("confirm-trash")).show(ctx, |ui| {
        ui.set_width(360.0);
        let photos = if rejected == 1 {
            "1 rejected photo".to_string()
        } else {
            format!("{rejected} rejected photos")
        };
        ui.label(
            RichText::new(format!("Move {photos} to the {BIN_NAME}?"))
                .font(theme::title(16.0))
                .color(theme::TEXT),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!(
                "The raw files go to the {BIN_NAME} with their Photonation and Lightroom \
                 sidecars. You can put them back from there."
            ))
            .size(12.5)
            .color(theme::TEXT_SECONDARY),
        );
        ui.add_space(14.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add(
                    theme::primary_button(&format!("Move to {BIN_NAME}"))
                        .min_size(vec2(84.0, 28.0)),
                )
                .clicked()
            {
                confirmed = true;
            }
            if ui
                .add(egui::Button::new("Cancel").min_size(vec2(72.0, 28.0)))
                .clicked()
            {
                close = true;
            }
        });
    });
    if confirmed {
        app.trash_rejected();
        close = true;
    }
    if close || modal.should_close() {
        app.view.library.confirm_trash = false;
    }
}
