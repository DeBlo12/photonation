// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The Color Mixer card's Point Color tab: swatches of the colours picked
//! in the photo, and the selected one's shifts and ranges.

use crate::app::ViewState;
use crate::panels::adjustments::control_slider;
use crate::theme;
use crate::widgets::{self, SliderStyle, Track};
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Ui, vec2};
use photonation_color::{Oklab, Primaries, Transfer};
use photonation_develop::{OklabLch, POINT_COLOR_CONTROLS, PointColor, Settings};
use photonation_pipeline::point_color::{MAX_POINT_HUE_SHIFT, shifted_target};

const SWATCH_DIAMETER: f32 = 24.0;
/// Shifts first, then ranges (see [`POINT_COLOR_CONTROLS`]).
const SHIFT_CONTROLS: std::ops::Range<usize> = 0..3;
const RANGE_CONTROLS: std::ops::Range<usize> = 3..6;
/// Track gradients of colours too muted to read get at least this chroma.
const MIN_TRACK_CHROMA: f32 = 0.08;

pub fn show(ui: &mut Ui, settings: &mut Settings, view: &mut ViewState) {
    if settings.point_colors.is_empty() {
        ui.label(
            RichText::new("Pick a color in the photo with ⌖ to edit it and colors like it.")
                .size(11.5)
                .color(theme::TEXT_TERTIARY),
        );
        return;
    }
    view.selected_point_color = view
        .selected_point_color
        .min(settings.point_colors.len() - 1);
    swatches(ui, &settings.point_colors, &mut view.selected_point_color);
    ui.add_space(6.0);

    let index = view.selected_point_color;
    let mut remove = false;
    before_after(ui, &settings.point_colors[index], &mut remove);
    ui.add_space(4.0);

    let point = &mut settings.point_colors[index];
    let styles = shift_styles(point.target);
    for (control, style) in POINT_COLOR_CONTROLS[SHIFT_CONTROLS].iter().zip(&styles) {
        control_slider(ui, point, control, style);
    }
    ui.add_space(4.0);
    ui.label(
        RichText::new("Range")
            .size(11.5)
            .color(theme::TEXT_SECONDARY),
    );
    let plain = SliderStyle {
        format: |v| format!("{v:.0}"),
        ..SliderStyle::default()
    };
    for control in &POINT_COLOR_CONTROLS[RANGE_CONTROLS] {
        control_slider(ui, point, control, &plain);
    }
    ui.horizontal(|ui| {
        widgets::toggle(ui, &mut view.point_color_overlay)
            .on_hover_text("Grey out everything this point color leaves alone");
        ui.label(
            RichText::new("Visualize range")
                .size(12.0)
                .color(theme::TEXT_SECONDARY),
        );
    });

    if remove {
        settings.point_colors.remove(index);
        view.selected_point_color = index.saturating_sub(1);
    }
}

/// One round swatch per point colour; click to select.
fn swatches(ui: &mut Ui, points: &[PointColor], selected: &mut usize) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (index, point) in points.iter().enumerate() {
            let (rect, response) =
                ui.allocate_exact_size(vec2(SWATCH_DIAMETER, SWATCH_DIAMETER), Sense::click());
            let painter = ui.painter();
            painter.circle_filled(
                rect.center(),
                SWATCH_DIAMETER / 2.0 - 3.0,
                swatch_color(point.target),
            );
            if *selected == index {
                painter.circle_stroke(
                    rect.center(),
                    SWATCH_DIAMETER / 2.0 - 0.5,
                    Stroke::new(2.0, theme::KNOB),
                );
            }
            if point.is_active() {
                painter.circle_filled(rect.center_bottom() + vec2(0.0, 3.0), 2.0, theme::ACCENT);
            }
            if response
                .on_hover_text(format!("Point color {}", index + 1))
                .clicked()
            {
                *selected = index;
            }
        }
    });
}

/// The picked colour beside what the edit turns it into, with a remove
/// button.
fn before_after(ui: &mut Ui, point: &PointColor, remove: &mut bool) {
    ui.horizontal(|ui| {
        let width = ui.available_width() - 36.0;
        let (rect, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
        let (left, right) = rect.split_left_right_at_fraction(0.5);
        let radius = theme::CONTROL_RADIUS;
        let painter = ui.painter();
        painter.rect_filled(
            left,
            egui::CornerRadius {
                nw: radius,
                sw: radius,
                ..Default::default()
            },
            swatch_color(point.target),
        );
        painter.rect_filled(
            right,
            egui::CornerRadius {
                ne: radius,
                se: radius,
                ..Default::default()
            },
            swatch_color(shifted_target(point)),
        );
        if crate::panels::tool_button(ui, "×", false, "Remove this point color").clicked() {
            *remove = true;
        }
    });
}

/// Tracks that preview each shift on the picked colour.
fn shift_styles(target: OklabLch) -> [SliderStyle; 3] {
    let visible = OklabLch {
        chroma: target.chroma.max(MIN_TRACK_CHROMA),
        ..target
    };
    let with_hue = |offset: f32| {
        swatch_color(OklabLch {
            hue: visible.hue + offset,
            ..visible
        })
    };
    let with_lightness = |lightness: f32| {
        swatch_color(OklabLch {
            lightness,
            ..visible
        })
    };
    let grey = OklabLch {
        chroma: 0.0,
        ..target
    };
    [
        SliderStyle {
            track: Track::Gradient(vec![
                with_hue(-MAX_POINT_HUE_SHIFT),
                with_hue(0.0),
                with_hue(MAX_POINT_HUE_SHIFT),
            ]),
            ..SliderStyle::default()
        },
        SliderStyle {
            track: Track::Gradient(vec![
                swatch_color(grey),
                swatch_color(OklabLch {
                    chroma: visible.chroma * 2.0,
                    ..visible
                }),
            ]),
            ..SliderStyle::default()
        },
        SliderStyle {
            track: Track::Gradient(vec![
                with_lightness((target.lightness - 0.3).max(0.05)),
                swatch_color(visible),
                with_lightness((target.lightness + 0.3).min(0.98)),
            ]),
            ..SliderStyle::default()
        },
    ]
}

/// An Oklab colour as an sRGB swatch (display only; out-of-gamut channels
/// clip).
pub fn swatch_color(color: OklabLch) -> Color32 {
    let rgb = Oklab::new(Primaries::REC709).to_rgb(color.to_lab());
    let [red, green, blue] =
        rgb.map(|v| (Transfer::Srgb.encode(v.clamp(0.0, 1.0) as f64) * 255.0).round() as u8);
    Color32::from_rgb(red, green, blue)
}
