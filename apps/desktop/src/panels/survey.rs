// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The survey view: the current and selected photos side by side, as
//! large as the window allows, to choose the best of a burst. Unedited
//! photos show the camera's embedded preview, edited ones a render (see
//! docs/ENGINE.md); the thumbnail stands in until it arrives.

use crate::app::PhotonationApp;
use crate::theme;
use crate::widgets::{Icon, paint_icon, paint_stars};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2,
};
use photonation_develop::{Flag, load_photo_settings};

const GAP: f32 = 12.0;
const FOOTER: f32 = 28.0;
/// Survey images are asked for in steps of this many pixels, so resizing
/// the window doesn't ask again for every pixel.
const SIZE_STEP: usize = 256;
const MAX_LONG_EDGE: usize = 2560;

pub fn show(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    egui::CentralPanel::no_frame()
        .frame(
            egui::Frame::NONE
                .fill(theme::WINDOW)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ui, |ui| {
            let photos = app.survey_photos();
            if photos.len() < 2 {
                app.view.library.survey = false;
                return;
            }
            let area = ui.max_rect();
            let (columns, rows) = match photos.len() {
                2 => (2, 1),
                3 => (3, 1),
                4 => (2, 2),
                5 | 6 => (3, 2),
                _ => (4, 2),
            };
            let cell_size = vec2(
                (area.width() - GAP * (columns - 1) as f32) / columns as f32,
                (area.height() - GAP * (rows - 1) as f32) / rows as f32,
            );
            let pixels_per_point = ui.ctx().pixels_per_point();
            let mut focus = None;
            let mut remove = None;
            for (slot, &index) in photos.iter().enumerate() {
                let origin = area.min
                    + vec2(
                        (slot % columns) as f32 * (cell_size.x + GAP),
                        (slot / columns) as f32 * (cell_size.y + GAP),
                    );
                let cell = Rect::from_min_size(origin, cell_size);
                let room = Rect::from_min_max(cell.min, pos2(cell.right(), cell.bottom() - FOOTER));
                let long_edge = ((room.width().max(room.height()) * pixels_per_point) as usize)
                    .div_ceil(SIZE_STEP)
                    * SIZE_STEP;
                request_image(app, index, long_edge.min(MAX_LONG_EDGE));
                let response =
                    ui.interact(cell, egui::Id::unique(("survey", index)), Sense::click());
                let current = app.current == Some(index);
                paint_photo(app, ui, index, room, current);
                footer(app, ui, index, cell, current);
                if response.hovered() && close_button(ui, room, index) {
                    remove = Some(index);
                } else if response.clicked() {
                    focus = Some(index);
                }
            }
            if let Some(index) = remove {
                app.remove_from_survey(index);
            } else if let Some(index) = focus {
                app.focus_in_survey(index);
            }
        });
}

/// Asks for the photo's survey image if none as large has been asked for.
fn request_image(app: &mut PhotonationApp, index: usize, long_edge: usize) {
    let path = app.photos[index].path.clone();
    if app
        .survey_requested
        .get(&path)
        .is_some_and(|&asked| asked >= long_edge)
    {
        return;
    }
    let settings = if app.current == Some(index) {
        app.settings.clone()
    } else {
        load_photo_settings(&path)
            .ok()
            .flatten()
            .map(|loaded| loaded.settings)
            .unwrap_or_default()
    };
    app.survey_requested.insert(path.clone(), long_edge);
    app.engine.request_survey_image(path, settings, long_edge);
}

fn paint_photo(app: &PhotonationApp, ui: &egui::Ui, index: usize, room: Rect, current: bool) {
    let photo = &app.photos[index];
    let texture = app
        .survey_images
        .get(&photo.path)
        .map(|preview| &preview.texture)
        .or(photo.thumbnail.as_ref());
    let Some(texture) = texture else {
        ui.painter().rect_filled(room, 4.0, theme::CARD);
        return;
    };
    let aspect = texture.aspect_ratio();
    let size = if aspect >= room.width() / room.height() {
        vec2(room.width(), room.width() / aspect)
    } else {
        vec2(room.height() * aspect, room.height())
    };
    let image = Rect::from_center_size(room.center(), size);
    let painter = ui.painter();
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter.add(
        egui::epaint::RectShape::filled(image, CornerRadius::same(2), Color32::WHITE)
            .with_texture(texture.id(), uv),
    );
    if photo.library.flag == Flag::Reject {
        painter.rect_filled(image, 2.0, Color32::from_black_alpha(140));
    }
    if current {
        painter.rect_stroke(
            image.expand(3.0),
            CornerRadius::same(4),
            Stroke::new(2.0, theme::ACCENT),
            StrokeKind::Outside,
        );
    }
}

/// Name on the left, stars and flag on the right.
fn footer(app: &PhotonationApp, ui: &egui::Ui, index: usize, cell: Rect, current: bool) {
    let photo = &app.photos[index];
    let painter = ui.painter();
    let y = cell.bottom() - FOOTER / 2.0 + 2.0;
    painter.text(
        pos2(cell.left() + 2.0, y),
        Align2::LEFT_CENTER,
        &photo.name,
        theme::body(12.0),
        if current {
            theme::TEXT
        } else {
            theme::TEXT_SECONDARY
        },
    );
    let flag_rect = Rect::from_center_size(pos2(cell.right() - 9.0, y), vec2(14.0, 14.0));
    match photo.library.flag {
        Flag::Pick => paint_icon(painter, flag_rect, Icon::Pick, theme::ACCENT),
        Flag::Reject => paint_icon(painter, flag_rect, Icon::Reject, theme::REJECT),
        Flag::None => {}
    }
    let star = 12.0;
    let stars_left = flag_rect.left() - 8.0 - star * 5.0;
    paint_stars(
        painter,
        pos2(stars_left, y),
        star,
        photo.library.rating,
        true,
        theme::TEXT_SECONDARY,
    );
}

/// The × in the photo's corner that takes it out of the survey; true
/// when clicked.
fn close_button(ui: &egui::Ui, room: Rect, index: usize) -> bool {
    let rect = Rect::from_center_size(room.right_top() + vec2(-16.0, 16.0), vec2(22.0, 22.0));
    let id = egui::Id::unique(("survey-close", index));
    let response = ui.interact(rect, id, Sense::click());
    let painter = ui.painter();
    painter.circle_filled(
        rect.center(),
        11.0,
        Color32::from_black_alpha(if response.hovered() { 200 } else { 140 }),
    );
    paint_icon(painter, rect.shrink(5.0), Icon::Reject, theme::TEXT);
    crate::panels::hover_tip(response, "Remove from the survey").clicked()
}
