// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Visual language: Photomator's dark, image-first layout in the
//! Photonation brand. All colours, type and metrics live here so the widgets
//! stay consistent.
//!
//! Surfaces are near-neutral greys leaning slightly toward the brand's Ink,
//! so the window reads as Photonation without tinting how a photo is
//! judged. Photon amber is reserved for "on": active tools, changed values,
//! switches and the primary button. Text on amber is Ink, because Paper on
//! amber fails contrast.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Margin,
    Stroke, epaint::text::VariationCoords,
};
use std::sync::Arc;

pub const WINDOW: Color32 = Color32::from_rgb(0x0e, 0x10, 0x15);
/// The floating inspector on the right.
pub const SIDEBAR: Color32 = Color32::from_rgb(0x17, 0x1a, 0x21);
pub const CARD: Color32 = Color32::from_rgb(0x1f, 0x23, 0x2b);
pub const CONTROL: Color32 = Color32::from_rgb(0x2b, 0x30, 0x3b);
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0x36, 0x3c, 0x49);
pub const SEPARATOR: Color32 = Color32::from_rgb(0x27, 0x2c, 0x37);
pub const TRACK: Color32 = Color32::from_rgb(0x3a, 0x40, 0x4e);
/// Brand Paper.
pub const TEXT: Color32 = Color32::from_rgb(0xf6, 0xf5, 0xf1);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0xa7, 0xae, 0xc0);
pub const TEXT_TERTIARY: Color32 = Color32::from_rgb(0x6b, 0x72, 0x87);
/// Brand Photon amber.
pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0xb2, 0x24);
/// Accent at 25 % opacity (premultiplied).
pub const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(0x40, 0x2c, 0x09, 0x40);
/// Text and icons on an [`ACCENT`] fill: brand Ink.
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x0e, 0x13, 0x22);
pub const KNOB: Color32 = TEXT;
pub const MASK_OVERLAY: Color32 = Color32::from_rgba_premultiplied(0x8c, 0x14, 0x1e, 0x8c);
/// Under mask lines and handles, so they stay visible on bright photos.
pub const HANDLE_SHADOW: Color32 = Color32::from_black_alpha(140);

pub const CARD_RADIUS: u8 = 10;
pub const INSPECTOR_RADIUS: u8 = 14;
pub const CONTROL_RADIUS: u8 = 6;
pub const SIDEBAR_WIDTH: f32 = 320.0;
pub const TOOLBAR_HEIGHT: f32 = 46.0;
/// The filmstrip while the pointer is on it.
pub const FILMSTRIP_HEIGHT: f32 = 104.0;
/// The filmstrip at rest: a thin row of tiny thumbnails.
pub const FILMSTRIP_RESTING_HEIGHT: f32 = 26.0;
/// Room for the macOS traffic lights in the transparent title bar. Other
/// platforms keep their native title bar above the toolbar.
pub const MENU_BAR_HEIGHT: f32 = 26.0;
pub const TRAFFIC_LIGHT_INSET: f32 = if cfg!(target_os = "macos") { 78.0 } else { 0.0 };

/// Shortcut labels for tooltips, in the platform's notation.
pub const SHORTCUT_OPEN: &str = shortcut("⌘O", "Ctrl+O");
pub const SHORTCUT_EXPORT: &str = shortcut("⇧⌘E", "Ctrl+Shift+E");
pub const SHORTCUT_FIT: &str = shortcut("⌘0", "Ctrl+0");
pub const SHORTCUT_UNDO: &str = shortcut("⌘Z", "Ctrl+Z");
pub const SHORTCUT_REDO: &str = shortcut("⇧⌘Z", "Ctrl+Shift+Z");
pub const SHORTCUT_SETTINGS: &str = shortcut("⌘,", "Ctrl+,");
pub const SHORTCUT_QUIT: &str = shortcut("⌘Q", "Alt+F4");
pub const SHIFT_KEY: &str = shortcut("⇧", "Shift");
pub const ALT_KEY: &str = shortcut("Option", "Alt");
pub const COMMAND_KEY: &str = shortcut("⌘", "Ctrl");

const fn shortcut(mac: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        mac
    } else {
        other
    }
}

/// Inter, the interface face.
pub fn body(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Inter semibold, for section titles.
pub fn title(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// Sora semibold, the wordmark and display face.
pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(DISPLAY.into()))
}

const SEMIBOLD: &str = "semibold";
const DISPLAY: &str = "display";

pub fn card_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(CARD)
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(12))
}

/// Photomator's inspector: a rounded panel floating over the window.
pub fn inspector_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(SIDEBAR)
        .stroke(Stroke::new(1.0, SEPARATOR))
        .corner_radius(CornerRadius::same(INSPECTOR_RADIUS))
        .inner_margin(Margin::same(12))
}

/// The primary action: an amber pill with Ink text.
pub fn primary_button(label: &str) -> egui::Button<'_> {
    egui::Button::new(
        egui::RichText::new(label)
            .color(ON_ACCENT)
            .font(title(13.0)),
    )
    .fill(ACCENT)
    .corner_radius(CornerRadius::same(u8::MAX))
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
    visuals.selection.stroke = Stroke::new(1.0, ON_ACCENT);
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

/// Bundles the brand faces (variable fonts, instanced by weight): Inter for
/// the interface, Sora for display. egui's built-in fonts stay behind them
/// as fallbacks. Icons are drawn as vectors (`widgets::icon`), not from
/// fonts.
fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(brand_fonts());
}

fn brand_fonts() -> FontDefinitions {
    const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter.ttf");
    const SORA: &[u8] = include_bytes!("../../../assets/fonts/Sora.ttf");
    let weighted = |bytes: &'static [u8], weight: f32| {
        Arc::new(FontData::from_static(bytes).tweak(FontTweak {
            coords: VariationCoords::new([(*b"wght", weight)]),
            ..Default::default()
        }))
    };
    let mut fonts = FontDefinitions::default();
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (name, data, family) in [
        ("inter", weighted(INTER, 400.0), FontFamily::Proportional),
        (
            "inter-semibold",
            weighted(INTER, 600.0),
            FontFamily::Name(SEMIBOLD.into()),
        ),
        (
            "sora-semibold",
            weighted(SORA, 600.0),
            FontFamily::Name(DISPLAY.into()),
        ),
    ] {
        fonts.font_data.insert(name.into(), data);
        let mut faces = vec![name.to_owned()];
        if family != FontFamily::Proportional {
            faces.extend(fallbacks.iter().cloned());
        }
        fonts
            .families
            .entry(family)
            .or_default()
            .splice(0..0, faces);
    }
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    /// Every non-ASCII character inside a string literal in the app's
    /// sources (comment lines skipped). Labels and icons come from these.
    fn ui_characters() -> BTreeSet<char> {
        fn visit(dir: &Path, found: &mut BTreeSet<char>) {
            for entry in std::fs::read_dir(dir).expect("read src") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    visit(&path, found);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let source = std::fs::read_to_string(&path).expect("read source");
                    for line in source.lines() {
                        if line.trim_start().starts_with("//") {
                            continue;
                        }
                        // Odd pieces between quotes are inside literals.
                        for literal in line.split('"').skip(1).step_by(2) {
                            found.extend(literal.chars().filter(|c| !c.is_ascii()));
                        }
                    }
                }
            }
        }
        let mut found = BTreeSet::new();
        visit(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut found,
        );
        found
    }

    /// Symbols outside the bundled fonts, drawn through eframe's system
    /// font fallback: only the macOS shortcut keys. Each was checked in a
    /// screenshot on macOS; some symbols come out as stacked bars instead
    /// (the fullwidth `＋` and `✕` did), so a new one goes here only after
    /// it was seen drawn. Icons are brand vectors (`widgets::Icon`).
    const SYSTEM_FONT_ICONS: &[char] = &['⇧', '⌘'];

    /// A label or icon character in neither the bundled fonts nor
    /// [`SYSTEM_FONT_ICONS`] may draw as an empty box or as garbage.
    #[test]
    fn every_ui_character_has_a_checked_glyph() {
        let ctx = egui::Context::default();
        ctx.set_fonts(brand_fonts());
        // Fonts load on the first pass; its texture upload is not needed.
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        let font = body(13.0);
        let unchecked: Vec<char> = ctx.fonts_mut(|fonts| {
            ui_characters()
                .into_iter()
                .filter(|&c| !fonts.has_glyph(&font, c) && !SYSTEM_FONT_ICONS.contains(&c))
                .collect()
        });
        assert!(unchecked.is_empty(), "no checked glyph for {unchecked:?}");
    }
}
