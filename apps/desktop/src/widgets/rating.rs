// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Stars, flags and colour labels: drawn on thumbnails, and as a control
//! for the current photo in the toolbar.

use crate::library::LibraryChange;
use crate::panels::hover_tip;
use crate::theme;
use crate::widgets::{Icon, paint_icon};
use eframe::egui::{
    Color32, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, epaint::Mesh, pos2, vec2,
};
use photonation_develop::{ColorLabel, Flag, LibraryInfo, MAX_RATING};
use std::f32::consts::{FRAC_PI_2, TAU};

pub fn label_color(label: ColorLabel) -> Color32 {
    match label {
        ColorLabel::Red => theme::LABEL_RED,
        ColorLabel::Yellow => theme::LABEL_YELLOW,
        ColorLabel::Green => theme::LABEL_GREEN,
        ColorLabel::Blue => theme::LABEL_BLUE,
        ColorLabel::Purple => theme::LABEL_PURPLE,
    }
}

/// The key that toggles `label`, if it has one.
pub fn label_key(label: ColorLabel) -> Option<&'static str> {
    match label {
        ColorLabel::Red => Some("6"),
        ColorLabel::Yellow => Some("7"),
        ColorLabel::Green => Some("8"),
        ColorLabel::Blue => Some("9"),
        ColorLabel::Purple => None,
    }
}

/// A five-pointed star of outer `radius`, point up. Filled stars are a
/// triangle fan (a star is not convex, which egui's fills need) with an
/// outline over the seams.
pub fn paint_star(painter: &Painter, center: Pos2, radius: f32, color: Color32, filled: bool) {
    let points: Vec<Pos2> = (0..10)
        .map(|i| {
            let r = if i % 2 == 0 { radius } else { radius * 0.45 };
            let angle = -FRAC_PI_2 + i as f32 * TAU / 10.0;
            center + vec2(angle.cos(), angle.sin()) * r
        })
        .collect();
    if filled {
        let mut mesh = Mesh::default();
        mesh.colored_vertex(center, color);
        for point in &points {
            mesh.colored_vertex(*point, color);
        }
        for i in 0..10u32 {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % 10);
        }
        painter.add(Shape::mesh(mesh));
    }
    painter.add(Shape::closed_line(
        points,
        Stroke::new(if filled { 0.6 } else { 1.0 }, color),
    ));
}

/// `rating` stars in a row starting at `left_center`, `size` points each;
/// with `show_empty`, unearned stars as outlines. Returns the row's width.
pub fn paint_stars(
    painter: &Painter,
    left_center: Pos2,
    size: f32,
    rating: u8,
    show_empty: bool,
    color: Color32,
) -> f32 {
    let count = if show_empty { MAX_RATING } else { rating };
    for star in 0..count {
        let center = left_center + vec2(size * (star as f32 + 0.5), 0.0);
        let filled = star < rating;
        let star_color = if filled {
            color
        } else {
            color.gamma_multiply(0.45)
        };
        paint_star(painter, center, size * 0.45, star_color, filled);
    }
    size * f32::from(count)
}

/// The rating, flag and label on a thumbnail: stars bottom left on a dark
/// band, the flag bottom right, the label as a coloured bar along the
/// bottom edge. `scale` is 1 for a grid cell.
pub fn paint_badges(painter: &Painter, image: Rect, info: LibraryInfo, scale: f32) {
    if let Some(label) = info.label {
        let bar = Rect::from_min_max(pos2(image.left(), image.bottom() - 3.0 * scale), image.max);
        painter.rect_filled(bar, 0.0, label_color(label));
    }
    let band_height = 16.0 * scale;
    let star = 10.0 * scale;
    let y = image.bottom()
        - band_height / 2.0
        - if info.label.is_some() {
            3.0 * scale
        } else {
            0.0
        };
    if info.rating > 0 || info.flag != Flag::None {
        let band = Rect::from_min_max(
            pos2(image.left(), y - band_height / 2.0),
            pos2(image.right(), y + band_height / 2.0),
        );
        painter.rect_filled(band, 0.0, Color32::from_black_alpha(110));
    }
    if info.rating > 0 {
        paint_stars(
            painter,
            pos2(image.left() + 4.0 * scale, y),
            star,
            info.rating,
            false,
            theme::TEXT,
        );
    }
    let flag_rect = Rect::from_center_size(
        pos2(image.right() - 4.0 * scale - star / 2.0 - 1.0, y),
        vec2(star + 2.0, star + 2.0),
    );
    match info.flag {
        Flag::Pick => paint_icon(painter, flag_rect, Icon::Pick, theme::ACCENT),
        Flag::Reject => paint_icon(painter, flag_rect, Icon::Reject, theme::REJECT),
        Flag::None => {}
    }
}

/// The current photo's stars, pick and reject buttons and labels, as in
/// the toolbar. Returns the change the user made, if any.
pub fn rating_control(ui: &mut Ui, info: LibraryInfo) -> Option<LibraryChange> {
    let mut change = None;
    let star = 15.0;
    let (rect, response) = ui.allocate_exact_size(vec2(star * 5.0, 24.0), Sense::click());
    let hovered = response
        .hover_pos()
        .map(|pointer| (((pointer.x - rect.left()) / star).floor() as u8 + 1).min(MAX_RATING));
    let shown = hovered.unwrap_or(info.rating);
    let color = if hovered.is_some() {
        theme::ACCENT
    } else {
        theme::TEXT
    };
    paint_stars(
        ui.painter(),
        pos2(rect.left(), rect.center().y),
        star,
        shown,
        true,
        color,
    );
    let tip = "Rating (0–5)\nClick a star to set it, the same star again to clear it.";
    if hover_tip(response, tip).clicked()
        && let Some(stars) = hovered
    {
        change = Some(LibraryChange::Rating(if stars == info.rating {
            0
        } else {
            stars
        }));
    }
    ui.add_space(6.0);
    for (flag, icon, color, tip) in [
        (
            Flag::Pick,
            Icon::Pick,
            theme::ACCENT,
            "Pick (P)\nMarks a keeper. U removes the flag.",
        ),
        (
            Flag::Reject,
            Icon::Reject,
            theme::REJECT,
            "Reject (X)\nMarks a photo to throw away; the Library menu moves every \
             rejected photo to the bin at once. U removes the flag.",
        ),
    ] {
        let on = info.flag == flag;
        let (rect, response) = ui.allocate_exact_size(vec2(22.0, 24.0), Sense::click());
        if on {
            ui.painter()
                .rect_filled(rect.shrink(1.0), theme::CONTROL_RADIUS, theme::CONTROL);
        }
        let icon_color = if on {
            color
        } else if response.hovered() {
            theme::TEXT
        } else {
            theme::TEXT_TERTIARY
        };
        paint_icon(ui.painter(), rect.shrink(4.0), icon, icon_color);
        if hover_tip(response, tip).clicked() {
            change = Some(LibraryChange::Flag(if on { Flag::None } else { flag }));
        }
    }
    ui.add_space(6.0);
    for label in ColorLabel::ALL {
        let (rect, response) = ui.allocate_exact_size(vec2(14.0, 24.0), Sense::click());
        let on = info.label == Some(label);
        let radius = if on || response.hovered() { 5.0 } else { 3.5 };
        let color = label_color(label);
        ui.painter().circle_filled(
            rect.center(),
            radius,
            if on {
                color
            } else {
                color.gamma_multiply(0.55)
            },
        );
        if on {
            ui.painter()
                .circle_stroke(rect.center(), radius + 1.5, Stroke::new(1.0, theme::TEXT));
        }
        let tip = match label_key(label) {
            Some(key) => format!("{} label ({key})\nClick again to remove it.", label.name()),
            None => format!("{} label\nClick again to remove it.", label.name()),
        };
        if hover_tip(response, &tip).clicked() {
            change = Some(LibraryChange::ToggleLabel(label));
        }
    }
    change
}
