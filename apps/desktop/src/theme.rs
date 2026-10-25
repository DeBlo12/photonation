// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Visual language: a dark, image-first look in the spirit of Photomator.
//! All colours and metrics live here so the widgets stay consistent.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke,
};
use std::sync::Arc;

pub const WINDOW: Color32 = Color32::from_rgb(0x12, 0x12, 0x14);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x1e);
pub const CARD: Color32 = Color32::from_rgb(0x24, 0x24, 0x27);
pub const CONTROL: Color32 = Color32::from_rgb(0x33, 0x33, 0x37);
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0x3d, 0x3d, 0x42);
pub const SEPARATOR: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x32);
pub const TRACK: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3f);
pub const TEXT: Color32 = Color32::from_rgb(0xec, 0xec, 0xf0);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0x9a, 0x9a, 0xa2);
pub const TEXT_TERTIARY: Color32 = Color32::from_rgb(0x66, 0x66, 0x6d);
pub const ACCENT: Color32 = Color32::from_rgb(0x0a, 0x84, 0xff);
/// Accent at 25 % opacity (premultiplied).
pub const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(0x03, 0x21, 0x40, 0x40);
pub const KNOB: Color32 = Color32::from_rgb(0xf5, 0xf5, 0xf7);
/// Tint over the area the selected mask affects (premultiplied).
pub const MASK_OVERLAY: Color32 = Color32::from_rgba_premultiplied(0x8c, 0x14, 0x1e, 0x8c);
/// Under mask lines and handles, so they stay visible on bright photos.
pub const HANDLE_SHADOW: Color32 = Color32::from_black_alpha(140);

pub const CARD_RADIUS: u8 = 10;
pub const CONTROL_RADIUS: u8 = 6;
pub const SIDEBAR_WIDTH: f32 = 320.0;
pub const TOOLBAR_HEIGHT: f32 = 46.0;
pub const FILMSTRIP_HEIGHT: f32 = 104.0;
/// Room for the macOS traffic lights in the transparent title bar.
pub const TRAFFIC_LIGHT_INSET: f32 = 78.0;

pub fn body(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(CARD)
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(12))
}

/// Installs fonts and widget visuals.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = SIDEBAR;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = WINDOW;
    visuals.faint_bg_color = CARD;
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, TEXT);
    visuals.hyperlink_color = ACCENT;
    visuals.window_corner_radius = CornerRadius::same(CARD_RADIUS + 2);
    visuals.window_stroke = Stroke::new(1.0, SEPARATOR);
    let radius = CornerRadius::same(CONTROL_RADIUS);
    for (widget, fill) in [
        (&mut visuals.widgets.noninteractive, CARD),
        (&mut visuals.widgets.inactive, CONTROL),
        (&mut visuals.widgets.hovered, CONTROL_HOVER),
        (&mut visuals.widgets.active, CONTROL_HOVER),
        (&mut visuals.widgets.open, CONTROL),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.corner_radius = radius;
        widget.bg_stroke = Stroke::NONE;
        widget.fg_stroke = Stroke::new(1.0, TEXT);
    }
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_SECONDARY);
    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.text_styles.insert(egui::TextStyle::Body, body(13.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, body(13.0));
        style.text_styles.insert(egui::TextStyle::Small, body(11.0));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, body(15.0));
    });
}

/// Uses the macOS system font when present (loaded at runtime, never
/// bundled); otherwise egui's built-in fonts.
fn install_fonts(ctx: &egui::Context) {
    const SYSTEM_FONTS: [&str; 2] = [
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
    ];
    let mut fonts = FontDefinitions::default();
    if let Some(bytes) = SYSTEM_FONTS
        .iter()
        .find_map(|path| std::fs::read(path).ok())
    {
        fonts
            .font_data
            .insert("system".into(), Arc::new(FontData::from_owned(bytes)));
        if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
            family.insert(0, "system".into());
        }
    }
    ctx.set_fonts(fonts);
}
