// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The filmstrip: thumbnails of the folder's photos along the bottom.
//! It rests as a thin strip of tiny thumbnails so the photo gets the room,
//! and grows over the bottom of the viewer while the pointer is on it or
//! just after switching photos (see docs/DESKTOP.md).

use crate::app::{Photo, PhotonationApp};
use crate::theme;
use eframe::egui::{self, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

/// Thumbnail height when the strip is open.
const THUMB_HEIGHT: f32 = 72.0;
/// Thumbnail height when it rests.
const RESTING_THUMB_HEIGHT: f32 = 16.0;
const PLACEHOLDER_ASPECT: f32 = 1.5;
/// Gap plus stroke of the ring around the current photo.
const SELECTION_RING: f32 = 4.0;
/// Stay open this long after the pointer leaves, so brushing past the
/// edge doesn't make it flicker.
const LEAVE_DELAY: f64 = 0.5;
/// Open this long after the photo changes (arrow keys, opening a folder),
/// to show where in the folder the new photo is.
const NAVIGATION_REVEAL: f64 = 1.2;
/// Seconds the strip takes to open or close.
const ANIMATION_TIME: f32 = 0.18;

/// When the filmstrip is open. Time is egui's input time in seconds.
#[derive(Default)]
pub struct FilmstripReveal {
    open: bool,
    open_until: f64,
    shown_photo: Option<usize>,
}

impl FilmstripReveal {
    /// Whether the strip is open this frame. `pointer_over` is whether the
    /// pointer is on the strip as it was drawn last frame.
    pub fn update(&mut self, now: f64, pointer_over: bool, current_photo: Option<usize>) -> bool {
        if pointer_over {
            self.open_until = self.open_until.max(now + LEAVE_DELAY);
        }
        if current_photo != self.shown_photo {
            self.shown_photo = current_photo;
            self.open_until = self.open_until.max(now + NAVIGATION_REVEAL);
        }
        self.open = now < self.open_until;
        self.open
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Seconds until the strip closes by itself, if it is open.
    pub fn closes_in(&self, now: f64) -> Option<f64> {
        self.open.then(|| (self.open_until - now).max(0.0))
    }
}

pub fn show(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    // Reserve only the resting strip; the open strip floats over the viewer
    // so the photo never changes size (and never re-renders) as it opens.
    let resting = egui::Panel::bottom("filmstrip")
        .exact_size(theme::FILMSTRIP_RESTING_HEIGHT)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(theme::WINDOW))
        .show(ui, |_| {})
        .response
        .rect;
    let ctx = ui.ctx().clone();
    let open_rect = Rect::from_min_max(
        pos2(resting.left(), resting.bottom() - theme::FILMSTRIP_HEIGHT),
        resting.right_bottom(),
    );

    let (now, pointer) = ctx.input(|i| (i.time, i.pointer.hover_pos()));
    let hover_zone = if app.view.filmstrip.is_open() {
        open_rect
    } else {
        resting
    };
    let open = app.view.filmstrip.update(
        now,
        pointer.is_some_and(|p| hover_zone.contains(p)),
        app.current,
    );
    if let Some(seconds) = app.view.filmstrip.closes_in(now) {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(seconds));
    }
    let openness =
        ctx.animate_bool_with_time(egui::Id::unique("filmstrip-open"), open, ANIMATION_TIME);
    let height = egui::lerp(
        theme::FILMSTRIP_RESTING_HEIGHT..=theme::FILMSTRIP_HEIGHT,
        openness,
    );
    let thumb_height = egui::lerp(RESTING_THUMB_HEIGHT..=THUMB_HEIGHT, openness);
    let strip = Rect::from_min_max(pos2(resting.left(), resting.bottom() - height), resting.max);

    egui::Area::new(egui::Id::unique("filmstrip-strip"))
        .order(egui::Order::Foreground)
        .fixed_pos(strip.min)
        .show(&ctx, |ui| {
            ui.painter().rect_filled(strip, 0.0, theme::WINDOW);
            let margin = vec2(14.0, (height - thumb_height) / 2.0);
            let content = strip.shrink2(margin);
            ui.set_clip_rect(strip);
            let mut content_ui = ui.new_child(egui::UiBuilder::new().max_rect(content));
            thumbnails(app, &mut content_ui, thumb_height, openness);
        });
}

fn thumbnails(app: &mut PhotonationApp, ui: &mut egui::Ui, thumb_height: f32, openness: f32) {
    let navigated = ui
        .ctx()
        .input(|i| i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowRight));
    let mut clicked = None;
    egui::ScrollArea::horizontal()
        .auto_shrink([false; 2])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            // Rows are at least `interact_size.y` tall, more than a
            // resting thumbnail: the strip would cut it off.
            ui.spacing_mut().interact_size.y = thumb_height;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = egui::lerp(3.0..=8.0, openness);
                // Room for the selection ring, which the scroll area clips.
                ui.add_space(SELECTION_RING);
                for (index, photo) in app.photos.iter().enumerate() {
                    let selected = app.current == Some(index);
                    let response = thumbnail(ui, photo, selected, thumb_height, openness);
                    if selected && navigated {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.on_hover_text(&photo.name).clicked() {
                        clicked = Some(index);
                    }
                }
            });
        });
    if let Some(index) = clicked {
        app.select(index);
    }
}

/// One thumbnail; `openness` (0 resting, 1 open) fades in the details
/// that don't fit a tiny thumbnail.
fn thumbnail(
    ui: &mut egui::Ui,
    photo: &Photo,
    selected: bool,
    thumb_height: f32,
    openness: f32,
) -> egui::Response {
    let aspect = photo
        .thumbnail
        .as_ref()
        .map_or(PLACEHOLDER_ASPECT, |t| t.aspect_ratio());
    let (rect, response) =
        ui.allocate_exact_size(vec2(thumb_height * aspect, thumb_height), Sense::click());
    let painter = ui.painter();
    let corner = egui::lerp(2.0..=f32::from(theme::CONTROL_RADIUS), openness);
    let radius = CornerRadius::same(corner.round() as u8);
    match &photo.thumbnail {
        Some(texture) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.add(
                egui::epaint::RectShape::filled(rect, radius, egui::Color32::WHITE)
                    .with_texture(texture.id(), uv),
            );
        }
        None => {
            painter.rect_filled(rect, radius, theme::CARD);
            if photo.unsupported && openness > 0.5 {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Unsupported",
                    theme::body(10.5),
                    theme::TEXT_TERTIARY,
                );
            }
        }
    }
    if selected {
        painter.rect_stroke(
            rect.expand(SELECTION_RING / 2.0),
            CornerRadius::same(radius.nw + 2),
            Stroke::new(SELECTION_RING / 2.0, theme::ACCENT),
            StrokeKind::Outside,
        );
    } else {
        let dim = if response.hovered() { 20 } else { 70 };
        painter.rect_filled(rect, radius, egui::Color32::from_black_alpha(dim));
    }
    if photo.has_edits && openness > 0.5 {
        painter.circle_filled(
            pos2(rect.right() - 8.0, rect.top() + 8.0),
            3.5,
            theme::ACCENT,
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_while_the_pointer_is_on_it_and_shortly_after() {
        let mut reveal = FilmstripReveal::default();
        reveal.update(0.0, false, Some(0));
        assert!(!reveal.update(10.0, false, Some(0)), "rests by default");
        assert!(reveal.update(11.0, true, Some(0)));
        assert!(reveal.update(11.0 + LEAVE_DELAY - 0.1, false, Some(0)));
        assert!(!reveal.update(11.0 + LEAVE_DELAY + 0.1, false, Some(0)));
    }

    #[test]
    fn opens_briefly_when_the_photo_changes() {
        let mut reveal = FilmstripReveal::default();
        reveal.update(0.0, false, Some(0));
        assert!(!reveal.update(10.0, false, Some(0)));
        assert!(reveal.update(20.0, false, Some(1)));
        let closes_in = reveal.closes_in(20.0).expect("open");
        assert!((closes_in - NAVIGATION_REVEAL).abs() < 1e-9);
        assert!(!reveal.update(20.0 + NAVIGATION_REVEAL + 0.1, false, Some(1)));
        assert_eq!(reveal.closes_in(30.0), None);
    }
}
