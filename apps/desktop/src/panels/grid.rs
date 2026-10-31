// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The grid: every shown photo as a thumbnail with its stars, flag and
//! label, for culling a folder. Only the rows in view are drawn, so
//! folders of thousands stay cheap. See docs/DESKTOP.md.

use crate::app::{Photo, PhotonationApp};
use crate::library::capture_time_label;
use crate::panels::tool_button;
use crate::theme;
use crate::widgets::paint_badges;
use eframe::egui::{
    self, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2,
};

const GAP: f32 = 10.0;
/// Space around the thumbnail inside its cell.
const CELL_PADDING: f32 = 8.0;
const PLACEHOLDER_ASPECT: f32 = 1.5;

pub fn show(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    egui::CentralPanel::no_frame()
        .frame(
            egui::Frame::NONE
                .fill(theme::WINDOW)
                .inner_margin(egui::Margin::symmetric(16, 8)),
        )
        .show(ui, |ui| {
            let shown = app.view.library.shown.clone();
            if shown.is_empty() {
                nothing_shown(app, ui);
                return;
            }
            let width = ui.available_width();
            let wanted = app.view.library.grid_size;
            let columns = (((width + GAP) / (wanted + GAP)).floor() as usize).max(1);
            let cell = (width - GAP * (columns - 1) as f32) / columns as f32;
            let row_height = cell + GAP;
            let rows = shown.len().div_ceil(columns);
            let library = &mut app.view.library;
            library.grid_columns = columns;

            // Keep the current photo in view when it moves by keyboard.
            let current_row = app
                .current
                .and_then(|current| shown.iter().position(|&i| i == current))
                .map(|position| position / columns);
            let mut scroll_area = egui::ScrollArea::vertical().auto_shrink([false; 2]);
            if library.grid_followed != app.current
                && let Some(row) = current_row
            {
                let (offset, height) = library.grid_viewport;
                let top = row as f32 * row_height;
                let target = if top < offset || height <= 0.0 {
                    top
                } else if top + row_height > offset + height {
                    top + row_height - height
                } else {
                    offset
                };
                scroll_area = scroll_area.vertical_scroll_offset(target.max(0.0));
            }
            library.grid_followed = app.current;

            let mut clicked = None;
            let mut opened = None;
            ui.spacing_mut().item_spacing.y = 0.0;
            let output = scroll_area.show_rows(ui, row_height, rows, |ui, row_range| {
                for row in row_range {
                    let (row_rect, _) =
                        ui.allocate_exact_size(vec2(width, row_height), Sense::hover());
                    for column in 0..columns {
                        let Some(&index) = shown.get(row * columns + column) else {
                            break;
                        };
                        let rect = Rect::from_min_size(
                            pos2(
                                row_rect.left() + column as f32 * (cell + GAP),
                                row_rect.top(),
                            ),
                            vec2(cell, cell),
                        );
                        let marked = if app.current == Some(index) {
                            Marked::Current
                        } else if app.view.library.selection.contains(&index) {
                            Marked::Selected
                        } else {
                            Marked::None
                        };
                        let response = cell_ui(ui, index, rect, &app.photos[index], marked);
                        if response.double_clicked() {
                            opened = Some(index);
                        } else if response.clicked() {
                            clicked = Some((index, ui.input(|i| i.modifiers)));
                        }
                    }
                }
            });
            app.view.library.grid_viewport =
                (output.state.clamped_offset().y, output.inner_rect.height());
            if let Some((index, modifiers)) = clicked {
                app.click_photo(index, modifiers);
                // It is in view: don't scroll to it.
                app.view.library.grid_followed = app.current;
            }
            if let Some(index) = opened {
                app.click_photo(index, egui::Modifiers::NONE);
                app.view.library.grid = false;
            }
        });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Marked {
    None,
    Selected,
    Current,
}

fn cell_ui(
    ui: &egui::Ui,
    index: usize,
    cell: Rect,
    photo: &Photo,
    marked: Marked,
) -> egui::Response {
    let response = ui.interact(cell, egui::Id::unique(("grid-cell", index)), Sense::click());
    let painter = ui.painter();
    let radius = CornerRadius::same(theme::CARD_RADIUS);
    let background = match marked {
        Marked::Current => theme::CONTROL,
        Marked::Selected => theme::CARD,
        Marked::None if response.hovered() => theme::CARD,
        Marked::None => theme::SIDEBAR,
    };
    painter.rect_filled(cell, radius, background);
    match marked {
        Marked::Current => {
            painter.rect_stroke(
                cell,
                radius,
                Stroke::new(2.0, theme::ACCENT),
                StrokeKind::Inside,
            );
        }
        Marked::Selected => {
            painter.rect_stroke(
                cell,
                radius,
                Stroke::new(1.5, theme::TEXT_SECONDARY),
                StrokeKind::Inside,
            );
        }
        Marked::None => {}
    }

    let room = cell.shrink(CELL_PADDING);
    let aspect = photo
        .thumbnail
        .as_ref()
        .map_or(PLACEHOLDER_ASPECT, |t| t.aspect_ratio());
    let size = if aspect >= room.width() / room.height() {
        vec2(room.width(), room.width() / aspect)
    } else {
        vec2(room.height() * aspect, room.height())
    };
    let image = Rect::from_center_size(room.center(), size);
    let corner = CornerRadius::same(3);
    match &photo.thumbnail {
        Some(texture) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.add(
                egui::epaint::RectShape::filled(image, corner, Color32::WHITE)
                    .with_texture(texture.id(), uv),
            );
        }
        None => {
            painter.rect_filled(image, corner, theme::CARD);
            if photo.unsupported {
                painter.text(
                    image.center(),
                    egui::Align2::CENTER_CENTER,
                    "Unsupported",
                    theme::body(11.0),
                    theme::TEXT_TERTIARY,
                );
            }
        }
    }
    if photo.library.flag == photonation_develop::Flag::Reject {
        painter.rect_filled(image, corner, Color32::from_black_alpha(140));
    }
    paint_badges(painter, image, photo.library, 1.0);
    if photo.has_edits {
        painter.circle_filled(
            pos2(image.right() - 8.0, image.top() + 8.0),
            3.5,
            theme::ACCENT,
        );
    }
    let tip = match photo.captured.as_deref().and_then(capture_time_label) {
        Some(date) => format!("{}\n{date}", photo.name),
        None => photo.name.clone(),
    };
    response.on_hover_text(tip)
}

fn nothing_shown(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);
        ui.label(
            RichText::new("No photos match the filter")
                .size(15.0)
                .color(theme::TEXT_SECONDARY),
        );
        ui.add_space(8.0);
        if tool_button(
            ui,
            "Clear Filter",
            false,
            "Clear the filter\nShows every photo again.",
        )
        .clicked()
        {
            app.view.library.filter = Default::default();
        }
    });
}
