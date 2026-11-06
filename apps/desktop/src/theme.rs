// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Visual language: Photomator's image-first layout in the Photonation
//! brand, dark or light as the system is. All colours, type and metrics live
//! here so the widgets stay consistent.
//!
//! Dark surfaces are near-neutral greys leaning slightly toward the brand's
//! Ink; light ones are its warm Paper. In light mode the photo sits on a
//! neutral mid-grey instead, so the surfaces do not skew how it is judged. Photon amber is reserved for "on": active tools, changed values,
//! switches and the primary button. Text on amber is Ink, because Paper on
//! amber fails contrast, and amber text is unreadable on Paper, so text and
//! thin marks on a surface use [`Palette::accent_strong`].
//!
//! [`palette`] holds what changes with the mode. The constants do not change:
//! brand colours, and whatever is drawn on the photo rather than on a surface.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Margin,
    Stroke, Theme, epaint::text::VariationCoords,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The colours that differ between dark and light mode.
pub struct Palette {
    pub window: Color32,
    /// Behind the photo in the viewer.
    pub viewer: Color32,
    /// The floating inspector on the right.
    pub sidebar: Color32,
    pub card: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub separator: Color32,
    pub track: Color32,
    /// Under text on glass, on cards and bars: as light a veil as keeps it readable on
    /// anything behind the window (see `glass_cards_keep_text_readable`).
    pub glass_card: Color32,
    /// Around a slider's knob, which is pale like the light surfaces.
    pub knob_edge: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_tertiary: Color32,
    /// Marks on [`Palette::viewer`], such as the rendering spinner.
    pub on_viewer: Color32,
    /// The accent as text, icons and thin marks on a surface.
    pub accent_strong: Color32,
    /// The iris's blades, in the order of `widgets::brand_mark`.
    pub brand_blades: [Color32; 6],
}

const DARK: Palette = Palette {
    window: Color32::from_rgb(0x0e, 0x10, 0x15),
    viewer: Color32::from_rgb(0x0e, 0x10, 0x15),
    sidebar: Color32::from_rgb(0x17, 0x1a, 0x21),
    card: Color32::from_rgb(0x1f, 0x23, 0x2b),
    control: Color32::from_rgb(0x2b, 0x30, 0x3b),
    control_hover: Color32::from_rgb(0x36, 0x3c, 0x49),
    separator: Color32::from_rgb(0x27, 0x2c, 0x37),
    track: Color32::from_rgb(0x3a, 0x40, 0x4e),
    glass_card: Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0x85),
    knob_edge: Color32::TRANSPARENT,
    text: PAPER,
    text_secondary: Color32::from_rgb(0xa7, 0xae, 0xc0),
    text_tertiary: Color32::from_rgb(0x85, 0x8b, 0xa0),
    on_viewer: Color32::from_rgb(0xa7, 0xae, 0xc0),
    accent_strong: ACCENT,
    brand_blades: [
        Color32::from_rgb(0xd6, 0xcf, 0xc1),
        Color32::from_rgb(0xea, 0xe5, 0xda),
        Color32::from_rgb(0xff, 0xfd, 0xf8),
        Color32::from_rgb(0xf6, 0xf2, 0xea),
        Color32::from_rgb(0xe2, 0xdc, 0xd0),
        Color32::from_rgb(0xcd, 0xc5, 0xb6),
    ],
};

const LIGHT: Palette = Palette {
    window: Color32::from_rgb(0xe7, 0xe5, 0xdf),
    // About 18 % grey, the photographer's middle grey.
    viewer: Color32::from_rgb(0x77, 0x77, 0x77),
    sidebar: PAPER,
    card: Color32::WHITE,
    control: Color32::from_rgb(0xda, 0xd8, 0xd1),
    control_hover: Color32::from_rgb(0xce, 0xcb, 0xc3),
    separator: Color32::from_rgb(0xd9, 0xd7, 0xd0),
    track: Color32::from_rgb(0xcf, 0xcc, 0xc4),
    glass_card: Color32::from_rgba_premultiplied(0x78, 0x78, 0x78, 0x78),
    knob_edge: Color32::from_black_alpha(90),
    text: INK,
    text_secondary: Color32::from_rgb(0x4a, 0x50, 0x61),
    text_tertiary: Color32::from_rgb(0x6a, 0x6f, 0x7d),
    on_viewer: PAPER,
    accent_strong: Color32::from_rgb(0x88, 0x51, 0x00),
    brand_blades: [
        Color32::from_rgb(0x17, 0x1b, 0x2a),
        Color32::from_rgb(0x26, 0x2c, 0x3e),
        Color32::from_rgb(0x3a, 0x42, 0x58),
        Color32::from_rgb(0x2f, 0x36, 0x4a),
        Color32::from_rgb(0x1e, 0x23, 0x34),
        Color32::from_rgb(0x11, 0x14, 0x1f),
    ],
};

/// Whether this frame is in light mode. Set once per frame by [`follow_mode`];
/// painting code often has no `Context` at hand to ask.
static LIGHT_MODE: AtomicBool = AtomicBool::new(false);

const fn palette_of(theme: Theme) -> &'static Palette {
    match theme {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

/// The colours of the mode the app is in.
pub fn palette() -> &'static Palette {
    palette_of(if LIGHT_MODE.load(Ordering::Relaxed) {
        Theme::Light
    } else {
        Theme::Dark
    })
}

/// Picks up the mode egui resolved from the system. Call at the start of a
/// frame, before anything paints.
pub fn follow_mode(ctx: &egui::Context) {
    LIGHT_MODE.store(ctx.theme() == Theme::Light, Ordering::Relaxed);
}

/// Whether the panes are on glass (glass.rs). Set once at startup.
static GLASS: AtomicBool = AtomicBool::new(false);

/// Tells the theme that the panes have glass behind them.
#[cfg(target_os = "macos")]
pub fn use_glass() {
    GLASS.store(true, Ordering::Relaxed);
}

/// Whether the panes are on glass and must leave their area see-through.
pub fn glass() -> bool {
    GLASS.load(Ordering::Relaxed)
}

/// Behind toolbars and library views: a veil on glass, for their text, else
/// the window colour.
pub fn window_fill() -> Color32 {
    if glass() {
        palette().glass_card
    } else {
        palette().window
    }
}

/// Brand Ink.
pub const INK: Color32 = Color32::from_rgb(0x0e, 0x13, 0x22);
/// Brand Paper.
pub const PAPER: Color32 = Color32::from_rgb(0xf6, 0xf5, 0xf1);
/// Brand Photon amber, as a fill or on the photo. On a surface, text and thin
/// marks use [`Palette::accent_strong`].
pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0xb2, 0x24);
/// Accent at 25 % opacity (premultiplied).
pub const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(0x40, 0x2c, 0x09, 0x40);
/// Text and icons on an [`ACCENT`] fill: brand Ink.
pub const ON_ACCENT: Color32 = INK;
pub const KNOB: Color32 = PAPER;
/// Text and icons drawn on the photo or on a dark band over it.
pub const ON_PHOTO: Color32 = PAPER;
pub const MASK_OVERLAY: Color32 = Color32::from_rgba_premultiplied(0x8c, 0x14, 0x1e, 0x8c);
/// Colour labels (Lightroom's five).
pub const LABEL_RED: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
pub const LABEL_YELLOW: Color32 = Color32::from_rgb(0xf2, 0xcc, 0x3d);
pub const LABEL_GREEN: Color32 = Color32::from_rgb(0x46, 0xb8, 0x6b);
pub const LABEL_BLUE: Color32 = Color32::from_rgb(0x4a, 0x90, 0xe2);
pub const LABEL_PURPLE: Color32 = Color32::from_rgb(0xa3, 0x6b, 0xdb);
/// The reject mark.
pub const REJECT: Color32 = LABEL_RED;
/// Inside the mask pins on the photo.
pub const PIN_FILL: Color32 = Color32::from_rgb(0x1f, 0x23, 0x2b);
/// Under mask lines and handles, so they stay visible on bright photos.
pub const HANDLE_SHADOW: Color32 = Color32::from_black_alpha(140);

pub const CARD_RADIUS: u8 = 10;
pub const INSPECTOR_RADIUS: u8 = 14;
pub const CONTROL_RADIUS: u8 = 6;
pub const SIDEBAR_WIDTH: f32 = 320.0;
pub const TOOLBAR_HEIGHT: f32 = 46.0;
/// Around a pane floating on glass, to the window's edge and the viewer.
pub const GLASS_PANE_MARGIN: f32 = 10.0;
/// The solid border around the photo on glass.
pub const PHOTO_BORDER: f32 = 6.0;
/// The filmstrip while the pointer is on it.
pub const FILMSTRIP_HEIGHT: f32 = 104.0;
/// The filmstrip at rest: a thin row of tiny thumbnails.
pub const FILMSTRIP_RESTING_HEIGHT: f32 = 26.0;
/// Room for the macOS traffic lights in the transparent title bar. Other
/// platforms keep their native title bar above the toolbar.
pub const MENU_BAR_HEIGHT: f32 = 26.0;

/// Shortcut labels for tooltips, in the platform's notation.
pub const SHORTCUT_OPEN: &str = shortcut("⌘O", "Ctrl+O");
pub const SHORTCUT_EXPORT: &str = shortcut("⇧⌘E", "Ctrl+Shift+E");
pub const SHORTCUT_FIT: &str = shortcut("⌘0", "Ctrl+0");
pub const SHORTCUT_UNDO: &str = shortcut("⌘Z", "Ctrl+Z");
pub const SHORTCUT_REDO: &str = shortcut("⇧⌘Z", "Ctrl+Shift+Z");
pub const SHORTCUT_SETTINGS: &str = shortcut("⌘,", "Ctrl+,");
pub const SHORTCUT_QUIT: &str = shortcut("⌘Q", "Alt+F4");
pub const SHORTCUT_COPY_SETTINGS: &str = shortcut("⇧⌘C", "Ctrl+Shift+C");
pub const SHORTCUT_PASTE_SETTINGS: &str = shortcut("⇧⌘V", "Ctrl+Shift+V");
pub const SHORTCUT_SELECT_ALL: &str = shortcut("⌘A", "Ctrl+A");
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
        .fill(if glass() {
            palette().glass_card
        } else {
            palette().card
        })
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(12))
}

/// Photomator's inspector: a rounded panel floating over the window.
pub fn inspector_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(if glass() {
            Color32::TRANSPARENT
        } else {
            palette().sidebar
        })
        .stroke(Stroke::new(1.0, palette().separator))
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

/// Installs fonts and the widget visuals of both modes. The app then follows
/// the system's mode, also when it changes while running.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::ThemePreference::System);
    for theme in [Theme::Dark, Theme::Light] {
        ctx.set_visuals_of(theme, visuals_of(theme));
    }
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

/// egui's own visuals for `theme`, in the brand's colours.
fn visuals_of(theme: Theme) -> egui::Visuals {
    let colors = palette_of(theme);
    let mut visuals = match theme {
        Theme::Dark => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
    };
    visuals.panel_fill = colors.sidebar;
    visuals.window_fill = colors.card;
    visuals.extreme_bg_color = colors.window;
    visuals.faint_bg_color = colors.card;
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, ON_ACCENT);
    visuals.hyperlink_color = colors.accent_strong;
    visuals.window_corner_radius = CornerRadius::same(CARD_RADIUS + 2);
    visuals.window_stroke = Stroke::new(1.0, colors.separator);
    let radius = CornerRadius::same(CONTROL_RADIUS);
    for (widget, fill) in [
        (&mut visuals.widgets.noninteractive, colors.card),
        (&mut visuals.widgets.inactive, colors.control),
        (&mut visuals.widgets.hovered, colors.control_hover),
        (&mut visuals.widgets.active, colors.control_hover),
        (&mut visuals.widgets.open, colors.control),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.corner_radius = radius;
        widget.bg_stroke = Stroke::NONE;
        widget.fg_stroke = Stroke::new(1.0, colors.text);
    }
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, colors.text_secondary);
    visuals
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

    #[test]
    fn the_app_follows_the_system_mode() {
        for (system, palette) in [(Theme::Light, &LIGHT), (Theme::Dark, &DARK)] {
            let ctx = egui::Context::default();
            install(&ctx);
            let input = egui::RawInput {
                system_theme: Some(system),
                ..Default::default()
            };
            ctx.run_ui(input, |_| {}).textures_delta.clear();
            assert_eq!(ctx.theme(), system);
            assert_eq!(
                ctx.global_style().visuals.widgets.inactive.weak_bg_fill,
                palette.control
            );
        }
    }

    /// WCAG contrast ratio of two opaque colours.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |color: Color32| {
            let [red, green, blue, _] = egui::Rgba::from(color).to_array();
            0.2126 * red + 0.7152 * green + 0.0722 * blue
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// A veil of premultiplied `veil` over the opaque `backdrop`.
    fn over(veil: Color32, backdrop: Color32) -> Color32 {
        let [red, green, blue, alpha] = veil.to_array();
        let through = |channel: u8, base: u8| {
            channel + (u32::from(base) * u32::from(255 - alpha) / 255) as u8
        };
        Color32::from_rgb(
            through(red, backdrop.r()),
            through(green, backdrop.g()),
            through(blue, backdrop.b()),
        )
    }

    /// Glass as measured on macOS 27 over the hardest thing behind the window:
    /// a black one in light mode, a white one in dark mode.
    #[test]
    fn glass_cards_keep_text_readable() {
        for (palette, hardest_glass) in [
            (&LIGHT, Color32::from_gray(0x9a)),
            (&DARK, Color32::from_gray(0x82)),
        ] {
            let card = over(palette.glass_card, hardest_glass);
            assert!(contrast(palette.text, card) >= 4.5);
            assert!(contrast(palette.text_secondary, card) >= 4.5);
            assert!(contrast(palette.text_tertiary, card) >= 3.0);
        }
    }

    #[test]
    fn text_stays_readable_on_every_surface_in_both_modes() {
        for palette in [&DARK, &LIGHT] {
            for surface in [
                palette.window,
                palette.sidebar,
                palette.card,
                palette.control,
            ] {
                assert!(contrast(palette.text, surface) >= 4.5);
                assert!(contrast(palette.text_secondary, surface) >= 4.5);
                assert!(contrast(palette.accent_strong, surface) >= 4.5);
                assert!(contrast(palette.text_tertiary, surface) >= 3.0);
            }
            assert!(contrast(palette.on_viewer, palette.viewer) >= 3.0);
            // The library bar puts controls straight on the window.
            assert!(contrast(palette.control, palette.window) >= 1.1);
        }
        assert!(contrast(ON_ACCENT, ACCENT) >= 4.5);
    }
}
