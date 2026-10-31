// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The adjustments sidebar: histogram, then one collapsible card per
//! [`AdjustmentGroup`], in Lightroom's order (or, while masking or
//! cropping, the masks or crop panel).

use crate::app::{Eyedropper, MIXER_TABS, POINT_COLOR_TAB, PhotonationApp, ViewState};
use crate::theme;
use crate::widgets::{
    self, CurveChannel, Icon, SectionAction, SliderStyle, Track, adjustment_slider,
};
use eframe::egui::{self, Color32, RichText, Ui};
use photonation_develop::{
    AdjustmentGroup, BASIC_CONTROLS, ColorMixer, Control, DETAIL_CONTROLS, GRADING_BLEND_CONTROLS,
    GRADING_CONTROLS, MIXER_CONTROLS, PARAMETRIC_CURVE_CONTROLS, Settings, WhiteBalance,
};
use photonation_engine::Histogram;
use photonation_pipeline::curve::Parametric;

pub const CURVE_MODES: [&str; 5] = ["Parametric", "RGB", "Red", "Green", "Blue"];
const GRADING_WHEELS: [&str; 4] = ["Shadows", "Midtones", "Highlights", "Global"];
/// Swatch colours for the mixer bands (display only).
const BAND_SWATCHES: [Color32; 8] = [
    Color32::from_rgb(230, 64, 64),
    Color32::from_rgb(240, 140, 40),
    Color32::from_rgb(236, 206, 52),
    Color32::from_rgb(84, 190, 84),
    Color32::from_rgb(64, 198, 200),
    Color32::from_rgb(64, 112, 232),
    Color32::from_rgb(150, 84, 222),
    Color32::from_rgb(222, 72, 180),
];
const TEMPERATURE_RANGE: std::ops::RangeInclusive<f32> = 2000.0..=12000.0;
const TINT_RANGE: std::ops::RangeInclusive<f32> = -150.0..=150.0;

pub fn show(app: &mut PhotonationApp, ui: &mut Ui) {
    egui::Panel::right("adjustments")
        .resizable(true)
        .default_size(crate::screenshot::sidebar_width())
        .size_range(280.0..=440.0)
        .show_separator_line(false)
        .frame(
            egui::Frame::NONE
                .fill(theme::WINDOW)
                .inner_margin(egui::Margin {
                    left: 0,
                    right: 10,
                    top: 0,
                    bottom: 10,
                }),
        )
        .show(ui, |ui| {
            theme::inspector_frame().show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                inspector(app, ui);
            });
        });
}

fn inspector(app: &mut PhotonationApp, ui: &mut Ui) {
    widgets::histogram(ui, app.histogram.as_ref());
    ui.add_space(10.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            if app.view.cropping {
                crate::panels::crop::sidebar(app, ui);
                return;
            }
            if app.view.show_masks {
                crate::panels::masks::show(app, ui);
                return;
            }
            if app.view.show_removals {
                crate::panels::removals::show(app, ui);
                return;
            }
            ui.spacing_mut().item_spacing.y = 8.0;
            let as_shot = app.info.as_ref().map_or((5500.0, 0.0), |i| i.as_shot);
            let histogram = app.histogram.as_ref();
            for group in AdjustmentGroup::ALL {
                group_card(
                    ui,
                    &mut app.settings,
                    &mut app.view,
                    group,
                    |ui, settings, view| match group {
                        AdjustmentGroup::WhiteBalance => white_balance(ui, settings, view, as_shot),
                        AdjustmentGroup::Light => {
                            control_sliders(ui, settings, &BASIC_CONTROLS[0..6])
                        }
                        AdjustmentGroup::Presence => {
                            control_sliders(ui, settings, &BASIC_CONTROLS[6..9])
                        }
                        AdjustmentGroup::Color => color(ui, settings),
                        AdjustmentGroup::ToneCurve => curves(ui, settings, view, histogram),
                        AdjustmentGroup::ColorMixer => color_mixer(ui, settings, view),
                        AdjustmentGroup::ColorGrading => color_grading(ui, settings, view),
                        AdjustmentGroup::Detail => control_sliders(ui, settings, DETAIL_CONTROLS),
                    },
                );
            }
            ui.add_space(8.0);
            if ui.button("Reset All").clicked() {
                app.settings = Settings::default();
            }
        });
}

/// One collapsible card, wiring its switch and reset to the view state.
fn group_card(
    ui: &mut Ui,
    settings: &mut Settings,
    view: &mut ViewState,
    group: AdjustmentGroup,
    content: impl FnOnce(&mut Ui, &mut Settings, &mut ViewState),
) {
    let mut expanded = view.expanded.contains(&group);
    let mut enabled = !view.disabled.contains(&group);
    let modified = group.is_modified(settings);
    let action = widgets::section(
        ui,
        group.title(),
        &mut expanded,
        &mut enabled,
        modified,
        |ui| content(ui, settings, view),
    );
    if action == SectionAction::Reset {
        group.reset(settings);
        if group == AdjustmentGroup::Detail {
            settings.sharpening = Default::default();
        }
    }
    set_membership(&mut view.expanded, group, expanded);
    set_membership(&mut view.disabled, group, !enabled);
}

fn set_membership(
    set: &mut std::collections::HashSet<AdjustmentGroup>,
    group: AdjustmentGroup,
    member: bool,
) {
    if member {
        set.insert(group);
    } else {
        set.remove(&group);
    }
}

/// Slider bound to a [`Control`] spec of the photo's settings or a mask's
/// local adjustments.
pub fn control_slider<Target>(
    ui: &mut Ui,
    target: &mut Target,
    control: &Control<Target>,
    style: &SliderStyle,
) -> egui::Response {
    let value = control.get_mut(target);
    adjustment_slider(
        ui,
        control.label,
        value,
        control.min..=control.max,
        control.default,
        style,
    )
}

fn control_sliders(ui: &mut Ui, settings: &mut Settings, controls: &[Control]) {
    for control in controls {
        control_slider(ui, settings, control, &style_for(control.id));
    }
}

pub fn style_for(id: &str) -> SliderStyle {
    match id {
        "exposure" | "local-exposure" => SliderStyle {
            format: |v| format!("{v:+.2}").replace("+0.00", "0.00"),
            ..SliderStyle::default()
        },
        "sharpen-radius" => SliderStyle {
            format: |v| format!("{v:.1}"),
            ..SliderStyle::default()
        },
        "sharpen-amount" => SliderStyle {
            format: |v| format!("{v:.0}"),
            ..SliderStyle::default()
        },
        "local-temperature" => SliderStyle {
            track: temperature_track(),
            ..SliderStyle::default()
        },
        "local-tint" => SliderStyle {
            track: tint_track(),
            ..SliderStyle::default()
        },
        _ => SliderStyle::default(),
    }
}

/// Blue → white → amber, for temperature sliders.
fn temperature_track() -> Track {
    Track::Gradient(vec![
        Color32::from_rgb(70, 120, 230),
        Color32::from_rgb(200, 210, 230),
        Color32::from_rgb(240, 200, 90),
    ])
}

/// Green → grey → magenta, for tint sliders.
fn tint_track() -> Track {
    Track::Gradient(vec![
        Color32::from_rgb(70, 190, 90),
        Color32::from_rgb(210, 210, 210),
        Color32::from_rgb(210, 80, 200),
    ])
}

fn white_balance(ui: &mut Ui, settings: &mut Settings, view: &mut ViewState, as_shot: (f64, f64)) {
    let (temperature, tint) = match settings.white_balance {
        WhiteBalance::AsShot => as_shot,
        WhiteBalance::Custom { temperature, tint } => (temperature, tint),
    };
    ui.horizontal(|ui| {
        let current = match settings.white_balance {
            WhiteBalance::AsShot => "As Shot",
            WhiteBalance::Custom { temperature, tint } => WhiteBalance::PRESETS
                .iter()
                .find(|(_, t, ti)| *t == temperature && *ti == tint)
                .map_or("Custom", |(name, _, _)| name),
        };
        egui::ComboBox::from_id_salt("wb-preset")
            .selected_text(current)
            .width(ui.available_width() - 40.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(current == "As Shot", "As Shot")
                    .clicked()
                {
                    settings.white_balance = WhiteBalance::AsShot;
                }
                for (name, temperature, tint) in WhiteBalance::PRESETS {
                    if ui.selectable_label(current == name, name).clicked() {
                        settings.white_balance = WhiteBalance::Custom { temperature, tint };
                    }
                }
            });
        eyedropper_button(ui, view, Eyedropper::WhiteBalance);
    });
    ui.add_space(4.0);
    let mut temperature_value = temperature as f32;
    let mut tint_value = tint as f32;
    let temperature_style = SliderStyle {
        track: temperature_track(),
        format: |v| format!("{v:.0} K"),
    };
    let tint_style = SliderStyle {
        track: tint_track(),
        ..SliderStyle::default()
    };
    let changed_temperature = adjustment_slider(
        ui,
        "Temperature",
        &mut temperature_value,
        TEMPERATURE_RANGE,
        as_shot.0 as f32,
        &temperature_style,
    )
    .changed();
    let changed_tint = adjustment_slider(
        ui,
        "Tint",
        &mut tint_value,
        TINT_RANGE,
        as_shot.1 as f32,
        &tint_style,
    )
    .changed();
    if changed_temperature || changed_tint {
        settings.white_balance = WhiteBalance::Custom {
            temperature: temperature_value.round() as f64,
            tint: tint_value.round() as f64,
        };
    }
}

fn color(ui: &mut Ui, settings: &mut Settings) {
    let colorful = Track::Gradient(vec![
        Color32::from_rgb(128, 128, 128),
        Color32::from_rgb(220, 90, 90),
        Color32::from_rgb(230, 200, 60),
        Color32::from_rgb(80, 190, 120),
        Color32::from_rgb(70, 130, 230),
    ]);
    let style = SliderStyle {
        track: colorful,
        ..SliderStyle::default()
    };
    for control in &BASIC_CONTROLS[9..11] {
        control_slider(ui, settings, control, &style);
    }
}

fn curves(
    ui: &mut Ui,
    settings: &mut Settings,
    view: &mut ViewState,
    histogram: Option<&Histogram>,
) {
    widgets::segmented(ui, &CURVE_MODES, &mut view.curve_mode);
    ui.add_space(6.0);
    match view.curve_channel() {
        None => {
            let parametric = Parametric::new(&settings.tone_curve.parametric);
            let bins = histogram.map(|h| CurveChannel::Rgb.bins(h));
            widgets::curve_editor(ui, None, Some(&|v| parametric.eval(v)), bins, theme::TEXT);
            control_sliders(ui, settings, PARAMETRIC_CURVE_CONTROLS);
        }
        Some(channel) => {
            let curves = &mut settings.tone_curve;
            let curve = match channel {
                CurveChannel::Rgb => &mut curves.master,
                CurveChannel::Red => &mut curves.red,
                CurveChannel::Green => &mut curves.green,
                CurveChannel::Blue => &mut curves.blue,
            };
            let bins = histogram.map(|h| channel.bins(h));
            widgets::curve_editor(ui, Some(curve), None, bins, channel.color());
            ui.label(
                RichText::new("Click to add · drag to move · double-click to remove")
                    .size(11.0)
                    .color(theme::TEXT_TERTIARY),
            );
        }
    }
}

/// Toggles a picker; the viewer measures the next click on the photo.
fn eyedropper_button(ui: &mut Ui, view: &mut ViewState, purpose: Eyedropper) {
    let active = view.eyedropper == Some(purpose);
    if crate::panels::icon_button(ui, Icon::Focus, None, active, purpose.tooltip()).clicked() {
        view.eyedropper = (!active).then_some(purpose);
    }
}

/// The Color Mixer card: the eight-band mixer, or the point colours.
fn color_mixer(ui: &mut Ui, settings: &mut Settings, view: &mut ViewState) {
    ui.horizontal(|ui| {
        let picker_width = 36.0;
        ui.allocate_ui(
            egui::vec2(ui.available_width() - picker_width, 26.0),
            |ui| widgets::segmented(ui, &MIXER_TABS, &mut view.mixer_tab),
        );
        let purpose = if view.mixer_tab == POINT_COLOR_TAB {
            Eyedropper::PointColor
        } else {
            Eyedropper::MixerBand
        };
        eyedropper_button(ui, view, purpose);
    });
    ui.add_space(6.0);
    if view.mixer_tab == POINT_COLOR_TAB {
        crate::panels::point_colors::show(ui, settings, view);
    } else {
        mixer_bands(ui, settings, view);
    }
}

fn mixer_bands(ui: &mut Ui, settings: &mut Settings, view: &mut ViewState) {
    ui.horizontal(|ui| {
        let spacing = 6.0;
        ui.spacing_mut().item_spacing.x = spacing;
        let diameter = ((ui.available_width() - spacing * 7.0) / 8.0).min(26.0);
        for (index, swatch) in BAND_SWATCHES.iter().enumerate() {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(diameter, diameter), egui::Sense::click());
            let painter = ui.painter();
            painter.circle_filled(rect.center(), diameter / 2.0 - 3.0, *swatch);
            if view.mixer_band == index {
                painter.circle_stroke(
                    rect.center(),
                    diameter / 2.0 - 0.5,
                    egui::Stroke::new(2.0, theme::KNOB),
                );
            }
            if settings.color_mixer.bands()[index] != Default::default() {
                painter.circle_filled(
                    rect.center_bottom() + egui::vec2(0.0, 3.0),
                    2.0,
                    theme::ACCENT,
                );
            }
            if response
                .on_hover_text(ColorMixer::BAND_NAMES[index])
                .clicked()
            {
                view.mixer_band = index;
            }
        }
    });
    ui.add_space(4.0);
    let band = view.mixer_band;
    let swatch = BAND_SWATCHES[band];
    let neighbour = |offset: usize| BAND_SWATCHES[(band + offset) % 8];
    let styles = [
        SliderStyle {
            track: Track::Gradient(vec![neighbour(7), swatch, neighbour(1)]),
            ..SliderStyle::default()
        },
        SliderStyle {
            track: Track::Gradient(vec![Color32::from_gray(128), swatch]),
            ..SliderStyle::default()
        },
        SliderStyle {
            track: Track::Gradient(vec![
                swatch.gamma_multiply(0.25),
                swatch,
                swatch.lerp_to_gamma(Color32::WHITE, 0.6),
            ]),
            ..SliderStyle::default()
        },
    ];
    for (control, style) in MIXER_CONTROLS[band].iter().zip(&styles) {
        control_slider(ui, settings, control, style);
    }
}

fn color_grading(ui: &mut Ui, settings: &mut Settings, view: &mut ViewState) {
    widgets::segmented(ui, &GRADING_WHEELS, &mut view.grading_wheel);
    let grading = &mut settings.color_grading;
    let wheel = match view.grading_wheel {
        0 => &mut grading.shadows,
        1 => &mut grading.midtones,
        2 => &mut grading.highlights,
        _ => &mut grading.global,
    };
    widgets::color_wheel(ui, wheel, 150.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(format!(
                "Hue {:.0}°  ·  Saturation {:.0}",
                wheel.hue, wheel.saturation
            ))
            .size(11.5)
            .color(theme::TEXT_SECONDARY),
        );
    });
    let luminance = &GRADING_CONTROLS[view.grading_wheel][2];
    control_slider(ui, settings, luminance, &SliderStyle::default());
    ui.add_space(4.0);
    for control in GRADING_BLEND_CONTROLS {
        control_slider(
            ui,
            settings,
            control,
            &SliderStyle {
                format: |v| format!("{v:.0}"),
                ..SliderStyle::default()
            },
        );
    }
}
