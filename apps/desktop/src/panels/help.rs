// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The Help window (the ? button, Help ▸ Photonation Help, F1): one page
//! per feature, listed on the left. It opens on the page of the tool in
//! use and otherwise where it was last left. The text follows the code
//! (`photonation_pipeline`, `photonation_develop::controls`, the panels)
//! and docs/*.md; update it with them.

use crate::app::ViewState;
use crate::panels::{icon_button, tool_button};
use crate::theme;
use crate::widgets::{Icon, paint_brand_mark, paint_icon};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Id, LayerId, Layout, Order, RichText, Sense, Ui,
    text::LayoutJob, vec2,
};
use photonation_engine::{NAME_TOKEN, NUMBER_TOKEN};

const DEFAULT_SIZE: egui::Vec2 = vec2(920.0, 680.0);
const MIN_SIZE: egui::Vec2 = vec2(640.0, 420.0);
const NAV_WIDTH: f32 = 216.0;
const NAV_ROW_HEIGHT: f32 = 28.0;
/// Lines longer than this are hard to read.
const TEXT_WIDTH: f32 = 640.0;
const SCROLL_BAR_ROOM: f32 = 14.0;
const KEYS_WIDTH: f32 = 190.0;

/// Where Photonation keeps its preferences and models.
const DATA_FOLDER: &str = if cfg!(target_os = "macos") {
    "~/Library/Application Support/Photonation"
} else if cfg!(windows) {
    "%APPDATA%\\Photonation"
} else {
    "~/.local/share/photonation"
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Page {
    #[default]
    GettingStarted,
    Library,
    Developing,
    WhiteBalance,
    Light,
    Presence,
    Color,
    Curves,
    ColorMixer,
    ColorGrading,
    Detail,
    Masks,
    Crop,
    Remove,
    CompareZoom,
    Export,
    Settings,
    Lightroom,
    Shortcuts,
}

/// The navigation list: a group label and its pages.
const CHAPTERS: [(&str, &[Page]); 4] = [
    (
        "Basics",
        &[Page::GettingStarted, Page::Library, Page::Developing],
    ),
    (
        "Adjustments",
        &[
            Page::WhiteBalance,
            Page::Light,
            Page::Presence,
            Page::Color,
            Page::Curves,
            Page::ColorMixer,
            Page::ColorGrading,
            Page::Detail,
        ],
    ),
    (
        "Tools",
        &[Page::Masks, Page::Crop, Page::Remove, Page::CompareZoom],
    ),
    (
        "Saving and more",
        &[
            Page::Export,
            Page::Settings,
            Page::Lightroom,
            Page::Shortcuts,
        ],
    ),
];

impl Page {
    pub const ALL: [Page; 19] = [
        Page::GettingStarted,
        Page::Library,
        Page::Developing,
        Page::WhiteBalance,
        Page::Light,
        Page::Presence,
        Page::Color,
        Page::Curves,
        Page::ColorMixer,
        Page::ColorGrading,
        Page::Detail,
        Page::Masks,
        Page::Crop,
        Page::Remove,
        Page::CompareZoom,
        Page::Export,
        Page::Settings,
        Page::Lightroom,
        Page::Shortcuts,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Page::GettingStarted => "Getting started",
            Page::Library => "Rating and culling",
            Page::Developing => "How a photo is developed",
            Page::WhiteBalance => "White Balance",
            Page::Light => "Light",
            Page::Presence => "Presence",
            Page::Color => "Color",
            Page::Curves => "Curves",
            Page::ColorMixer => "Color Mixer and Point Color",
            Page::ColorGrading => "Color Grading",
            Page::Detail => "Detail",
            Page::Masks => "Masks and mask groups",
            Page::Crop => "Crop and straighten",
            Page::Remove => "Remove",
            Page::CompareZoom => "Compare and zoom",
            Page::Export => "Export",
            Page::Settings => "Settings",
            Page::Lightroom => "Lightroom import",
            Page::Shortcuts => "Keyboard shortcuts",
        }
    }

    /// The toolbar's icon for the page's tool, if it has one.
    fn icon(self) -> Option<Icon> {
        match self {
            Page::GettingStarted => Some(Icon::Library),
            Page::Library => Some(Icon::Grid),
            Page::Detail => Some(Icon::Focus),
            Page::Masks => Some(Icon::Layers),
            Page::Crop => Some(Icon::Crop),
            Page::Remove => Some(Icon::Retouch),
            Page::CompareZoom => Some(Icon::Compare),
            _ => None,
        }
    }

    /// The page by its title, ignoring case (for screenshot mode).
    pub fn from_title(title: &str) -> Option<Page> {
        Page::ALL
            .into_iter()
            .find(|page| page.title().eq_ignore_ascii_case(title.trim()))
    }
}

/// Whether the Help window is open and which page it shows. View state:
/// the page is remembered while the app runs, not saved.
#[derive(Debug, Default)]
pub struct HelpView {
    pub open: bool,
    pub page: Page,
    /// Bring the window to the front on the next frame (it was opened, or
    /// asked for while it may be behind a sheet).
    raise: bool,
    /// Frames left in which the list scrolls to the selected page (the
    /// window's first frame only measures it).
    reveal: u8,
}

/// The page about the tool or sheet in use, if any.
pub fn tool_page(view: &ViewState) -> Option<Page> {
    if view.show_export {
        Some(Page::Export)
    } else if view.show_settings {
        Some(Page::Settings)
    } else if view.show_masks {
        Some(Page::Masks)
    } else if view.cropping {
        Some(Page::Crop)
    } else if view.show_removals {
        Some(Page::Remove)
    } else if view.library.grid || view.library.survey {
        Some(Page::Library)
    } else {
        None
    }
}

/// Opens Help on the page of the tool in use, else where it was left.
pub fn open(view: &mut ViewState) {
    if let Some(page) = tool_page(view) {
        view.help.page = page;
    }
    view.help.open = true;
    view.help.raise = true;
    view.help.reveal = 2;
}

/// Opens Help on `page`.
pub fn open_page(view: &mut ViewState, page: Page) {
    view.help.page = page;
    view.help.open = true;
    view.help.raise = true;
    view.help.reveal = 2;
}

/// The toolbar's Help button; it opens and closes the window.
pub fn button(ui: &mut Ui, view: &mut ViewState) {
    let tip = "Help (F1)\nWhat each tool and setting does, one page per feature. Opens on \
               the page of the tool you are using.";
    if icon_button(ui, Icon::Help, None, view.help.open, tip).clicked() {
        if view.help.open {
            view.help.open = false;
        } else {
            open(view);
        }
    }
}

fn window_id() -> Id {
    Id::unique("help-window")
}

pub fn show(view: &mut ViewState, ctx: &egui::Context) {
    let help = &mut view.help;
    let id = window_id();
    if std::mem::take(&mut help.raise) {
        // Above the export sheet or Settings when asked for from there.
        ctx.move_to_top(LayerId::new(Order::Foreground, id));
    }
    let screen = ctx.content_rect();
    let size = DEFAULT_SIZE
        .min(screen.size() - vec2(48.0, 48.0))
        .max(MIN_SIZE);
    let mut close = false;
    egui::Window::new("Photonation Help")
        .id(id)
        .order(Order::Foreground)
        .title_bar(false)
        .frame(
            egui::Frame::NONE
                .fill(theme::SIDEBAR)
                .stroke(egui::Stroke::new(1.0, theme::SEPARATOR))
                .corner_radius(CornerRadius::same(theme::INSPECTOR_RADIUS))
                .inner_margin(egui::Margin::same(14))
                .shadow(ctx.global_style().visuals.window_shadow),
        )
        .pivot(Align2::CENTER_CENTER)
        .default_pos(screen.center())
        .default_size(size)
        .min_size(MIN_SIZE)
        .resizable(true)
        .collapsible(false)
        .constrain(true)
        .show(ctx, |ui| {
            header(ui, &mut close);
            ui.add_space(10.0);
            let height = ui.available_height();
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    vec2(NAV_WIDTH, height),
                    Layout::top_down(Align::Min),
                    |ui| navigation(ui, &mut help.page, help.reveal > 0),
                );
                ui.add_space(18.0);
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), height),
                    Layout::top_down(Align::Min),
                    |ui| page(ui, &mut help.page),
                );
            });
        });
    help.reveal = help.reveal.saturating_sub(1);
    if close {
        help.open = false;
    }
}

fn header(ui: &mut Ui, close: &mut bool) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
        paint_brand_mark(ui.painter(), rect.center(), 24.0, theme::SIDEBAR);
        ui.add_space(2.0);
        ui.label(
            RichText::new("Photonation Help")
                .font(theme::display(17.0))
                .color(theme::TEXT),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if tool_button(ui, "×", false, "Close Help (Esc)").clicked() {
                *close = true;
            }
        });
    });
}

fn navigation(ui: &mut Ui, selected: &mut Page, reveal: bool) {
    egui::Frame::NONE
        .fill(theme::WINDOW)
        .corner_radius(CornerRadius::same(theme::CARD_RADIUS))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("help-navigation")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (index, (chapter, pages)) in CHAPTERS.iter().enumerate() {
                        ui.add_space(if index == 0 { 4.0 } else { 10.0 });
                        ui.label(
                            RichText::new(*chapter)
                                .font(theme::title(11.0))
                                .color(theme::TEXT_TERTIARY),
                        );
                        for &page in *pages {
                            let row = nav_row(ui, page, page == *selected);
                            if reveal && page == *selected {
                                row.scroll_to_me(None);
                            }
                            if row.clicked() {
                                *selected = page;
                            }
                        }
                    }
                });
        });
}

/// A page in the navigation list; the selected one is amber with Ink text.
fn nav_row(ui: &mut Ui, page: Page, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, NAV_ROW_HEIGHT), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    let (fill, color) = if selected {
        (theme::ACCENT, theme::ON_ACCENT)
    } else if response.hovered() {
        (theme::CONTROL, theme::TEXT)
    } else {
        (Color32::TRANSPARENT, theme::TEXT_SECONDARY)
    };
    painter.rect_filled(rect, CornerRadius::same(theme::CONTROL_RADIUS), fill);
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 17.0, rect.center().y),
        vec2(16.0, 16.0),
    );
    if let Some(icon) = page.icon() {
        paint_icon(painter, icon_rect, icon, color);
    }
    let font = if selected {
        theme::title(13.0)
    } else {
        theme::body(13.0)
    };
    painter.text(
        egui::pos2(rect.left() + 34.0, rect.center().y),
        Align2::LEFT_CENTER,
        page.title(),
        font,
        color,
    );
    response
}

fn page(ui: &mut Ui, current: &mut Page) {
    let content = content(*current);
    let mut next = None;
    // One scroll position per page, so each opens where it was left.
    egui::ScrollArea::vertical()
        .id_salt(("help-page", *current))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = (ui.available_width() - SCROLL_BAR_ROOM).min(TEXT_WIDTH);
            ui.set_max_width(width);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if let Some(icon) = current.icon() {
                    let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                    paint_icon(ui.painter(), rect, icon, theme::ACCENT);
                    ui.add_space(2.0);
                }
                ui.label(
                    RichText::new(current.title())
                        .font(theme::title(22.0))
                        .color(theme::TEXT),
                );
            });
            ui.add_space(6.0);
            paragraph(ui, &content.lede, theme::body(14.0), theme::TEXT);
            ui.add_space(4.0);
            let mut step = 0;
            for block in &content.blocks {
                match block {
                    Block::Heading(text) => {
                        ui.add_space(14.0);
                        ui.label(
                            RichText::new(*text)
                                .font(theme::title(15.0))
                                .color(theme::TEXT),
                        );
                        ui.add_space(2.0);
                    }
                    Block::Text(text) => {
                        paragraph(ui, text, theme::body(13.0), theme::TEXT_SECONDARY);
                        ui.add_space(4.0);
                    }
                    Block::Term(term, text) => {
                        term_paragraph(ui, term, text, ui.available_width());
                        ui.add_space(5.0);
                    }
                    Block::Step(term, text) => {
                        step += 1;
                        numbered(ui, step, term, text);
                        ui.add_space(6.0);
                    }
                    Block::Keys(keys, action) => key_row(ui, keys, action),
                    Block::Note(text) => {
                        ui.add_space(8.0);
                        note(ui, text);
                        ui.add_space(4.0);
                    }
                }
            }
            if !content.see_also.is_empty() {
                ui.add_space(16.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("See also")
                            .font(theme::title(12.5))
                            .color(theme::TEXT_TERTIARY),
                    );
                    for &page in content.see_also {
                        let link = ui.link(RichText::new(page.title()).font(theme::body(13.0)));
                        if link.clicked() {
                            next = Some(page);
                        }
                    }
                });
            }
            ui.add_space(12.0);
        });
    if let Some(page) = next {
        *current = page;
    }
}

fn paragraph(ui: &mut Ui, text: &str, font_id: egui::FontId, color: Color32) {
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    job.append(text, 0.0, format(font_id, color));
    ui.label(job);
}

/// A term in semibold, then what it does, wrapped at `width`.
fn term_paragraph(ui: &mut Ui, term: &str, text: &str, width: f32) {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    job.append(term, 0.0, format(theme::title(13.0), theme::TEXT));
    job.append(" ", 0.0, format(theme::body(13.0), theme::TEXT));
    job.append(text, 0.0, format(theme::body(13.0), theme::TEXT_SECONDARY));
    ui.label(job);
}

/// A step with its number in an amber disc.
fn numbered(ui: &mut Ui, number: usize, term: &str, text: &str) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 10.0, theme::ACCENT);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            number.to_string(),
            theme::title(11.5),
            theme::ON_ACCENT,
        );
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.add_space(2.0);
            term_paragraph(ui, term, text, ui.available_width());
        });
    });
}

/// Key caps on the left, what they do on the right.
fn key_row(ui: &mut Ui, keys: &[String], action: &str) {
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            vec2(KEYS_WIDTH, 24.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (index, key) in keys.iter().enumerate() {
                    if index > 0 {
                        ui.label(
                            RichText::new("or")
                                .font(theme::body(11.5))
                                .color(theme::TEXT_TERTIARY),
                        );
                    }
                    egui::Frame::NONE
                        .fill(theme::CONTROL)
                        .stroke(egui::Stroke::new(1.0, theme::TRACK))
                        .corner_radius(CornerRadius::same(5))
                        .inner_margin(egui::Margin::symmetric(7, 2))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(key)
                                    .font(theme::body(12.0))
                                    .color(theme::TEXT),
                            );
                        });
                }
            },
        );
        ui.vertical(|ui| {
            ui.add_space(3.0);
            paragraph(ui, action, theme::body(13.0), theme::TEXT_SECONDARY);
        });
    });
    ui.add_space(3.0);
}

/// A tip on a card, marked in amber.
fn note(ui: &mut Ui, text: &str) {
    egui::Frame::NONE
        .fill(theme::CARD)
        .corner_radius(CornerRadius::same(theme::CARD_RADIUS))
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut job = LayoutJob::default();
            job.wrap.max_width = ui.available_width();
            job.append("Tip ", 0.0, format(theme::title(13.0), theme::ACCENT));
            job.append(text, 0.0, format(theme::body(13.0), theme::TEXT_SECONDARY));
            ui.label(job);
        });
}

fn format(font_id: egui::FontId, color: Color32) -> egui::TextFormat {
    egui::TextFormat {
        font_id,
        color,
        ..Default::default()
    }
}

/// A page: an opening line, then its blocks, then related pages.
struct Content {
    lede: String,
    blocks: Vec<Block>,
    see_also: &'static [Page],
}

enum Block {
    Heading(&'static str),
    Text(String),
    /// A term in semibold, then what it does.
    Term(&'static str, String),
    /// A numbered step: its name, then what happens.
    Step(&'static str, String),
    /// Alternative keys (or gestures) and what they do.
    Keys(Vec<String>, String),
    Note(String),
}

fn text(text: &str) -> Block {
    Block::Text(text.into())
}

fn term(term: &'static str, text: impl Into<String>) -> Block {
    Block::Term(term, text.into())
}

fn step(term: &'static str, text: &str) -> Block {
    Block::Step(term, text.into())
}

fn keys(keys: &[&str], action: &str) -> Block {
    Block::Keys(
        keys.iter().map(|key| (*key).to_owned()).collect(),
        action.into(),
    )
}

fn tip(text: impl Into<String>) -> Block {
    Block::Note(text.into())
}

fn content(page: Page) -> Content {
    let shift = theme::SHIFT_KEY;
    let alt = theme::ALT_KEY;
    let command = theme::COMMAND_KEY;
    match page {
        Page::GettingStarted => Content {
            lede: "Photonation develops raw photos. Open a folder, pick a photo in the \
                   filmstrip and adjust it in the inspector on the right. Your raw files \
                   are never changed."
                .into(),
            blocks: vec![
                Block::Heading("Open a folder"),
                term(
                    "Open",
                    format!(
                        "({}) or File › Open Folder… chooses a folder. Photonation lists \
                         its raw photos (Bayer cameras, Fujifilm X-Trans and linear DNG such \
                         as Apple ProRAW) in the filmstrip and shows the first one.",
                        theme::SHORTCUT_OPEN
                    ),
                ),
                Block::Heading("The filmstrip"),
                term(
                    "At rest",
                    "it is a thin row of tiny thumbnails along the bottom, so the photo gets \
                     the room. Point at it and it grows; click a thumbnail to open that photo, \
                     scroll or swipe to move along the folder.",
                ),
                term(
                    "← and →",
                    "step to the previous and next photo. The strip opens for a moment to \
                     show where in the folder you are.",
                ),
                term(
                    "An amber dot",
                    "marks photos you have edited; stars, a pick or reject mark and a \
                     coloured line show how you rated them (see Rating and culling). Files \
                     Photonation cannot read say Unsupported.",
                ),
                term(
                    "The scroll bar",
                    "along the bottom of the strip shows while it moves. Drag it to cross \
                     a long folder quickly; the photo under the middle of the strip is named \
                     above it as you drag.",
                ),
                Block::Heading("Edits are non-destructive"),
                term(
                    "Sidecars:",
                    "every edit is saved beside the raw file, in a small file named after \
                     it (IMG_0001.CR3 → IMG_0001.CR3.photonation.json), a moment after you \
                     stop. Keep the two together when you copy or move photos. A photo \
                     reset to its defaults needs no sidecar, so its file is removed.",
                ),
                term(
                    "Undo and redo",
                    format!(
                        "({} / {}) step through the photo's history; a whole slider drag is \
                         one step. Edit › Reset All Adjustments returns the photo to its \
                         defaults, and undo brings the edit back.",
                        theme::SHORTCUT_UNDO,
                        theme::SHORTCUT_REDO
                    ),
                ),
                Block::Heading("The inspector"),
                term(
                    "Cards",
                    "fold open with a click on their title. A card's switch turns it off \
                     without losing its values, to see the photo with and without it; the \
                     amber dot and the reset arrow appear once a card differs from its \
                     defaults.",
                ),
                term(
                    "Sliders:",
                    format!(
                        "drag anywhere on the row, hold {alt} for fine control, double-click \
                         to reset. The amber fill runs from the default value, marked by a \
                         tick."
                    ),
                ),
                term(
                    "The histogram",
                    "at the top shows the tones of the edited photo, from black at the left \
                     to white at the right.",
                ),
                tip(
                    "Point at any toolbar button to see its name, shortcut and what it does. \
                     Greyed-out tools explain themselves too.",
                ),
            ],
            see_also: &[Page::Developing, Page::Shortcuts, Page::Lightroom],
        },
        Page::Library => Content {
            lede: "Go through a folder quickly: rate, pick and reject photos with single \
                   keys, compare a burst side by side, then show only the keepers."
                .into(),
            blocks: vec![
                Block::Heading("Rating, flags and labels"),
                term(
                    "Stars:",
                    "0 to 5 set the rating (0 clears it); ] and [ add or take away a star. \
                     The stars in the toolbar do the same with a click.",
                ),
                term(
                    "Flags:",
                    "P picks a keeper, X rejects a photo, U removes the flag. Rejected \
                     photos are dimmed.",
                ),
                term(
                    "Labels:",
                    "6, 7, 8 and 9 put a red, yellow, green or blue label on the photo (the \
                     same key again takes it off); purple is in the toolbar and Library › \
                     Label.",
                ),
                term(
                    "Auto-advance",
                    "(Library › Auto-Advance, or the button above the grid) moves to the \
                     next photo after you rate or flag one, so a folder is one key per photo.",
                ),
                text(
                    "Ratings are saved in the photo's sidecar at once. They are not edits: \
                     a rated photo has no amber dot, and undo leaves ratings alone. Ratings \
                     and labels from Lightroom's XMP sidecars come along (a Lightroom reject \
                     is a reject here).",
                ),
                Block::Heading("Grid and survey"),
                term(
                    "Grid",
                    "(G, or the grid button in the toolbar) shows every photo at once. \
                     Click to choose one; Ctrl-click (⌘ on a Mac) adds or removes a photo, \
                     Shift-click selects a run. Ratings apply to every selected photo. \
                     Arrow keys move around; E, Return or a double-click opens the photo in \
                     the editor.",
                ),
                term(
                    "Survey",
                    "(N) puts the selected photos side by side, as large as fits, to find \
                     the best of a burst. ← → move between them and ratings apply to the one \
                     outlined; the × on a photo takes it out. Esc goes back to the grid.",
                ),
                Block::Heading("Filter and sort"),
                text(
                    "The bar above the grid (and Library › Filter) shows only photos with \
                     enough stars, a flag, a label or edits. The filmstrip, ← → and the grid \
                     all follow the filter and the sort order (file name, capture time, \
                     rating, or edited first). The photo you are on stays until you move \
                     on, even when a new rating no longer matches.",
                ),
                Block::Heading("Throwing rejects away"),
                text(&format!(
                    "Library › Move Rejected Photos to {bin}… moves every rejected photo, \
                     with its sidecars, to the {bin}, after asking. They can be put back \
                     from there.",
                    bin = crate::panels::library_bar::BIN_NAME
                )),
                tip(
                    "Thumbnails first show the preview the camera stored in the raw file, \
                     then Photonation's own rendering once it is ready; renders are kept, so \
                     the folder opens at once next time.",
                ),
            ],
            see_also: &[Page::GettingStarted, Page::Export, Page::Shortcuts],
        },
        Page::Developing => Content {
            lede: "Every preview, thumbnail and export runs the same steps in the same \
                   order, so what you see is what you export. The order explains how the \
                   settings work together."
                .into(),
            blocks: vec![
                step(
                    "Read and white-balance.",
                    "The part of the sensor inside the crop is read, scaled by the white \
                     balance and clipped evenly, so blown highlights stay neutral, then \
                     demosaiced (at half size when a preview needs no more).",
                ),
                step(
                    "Removals",
                    "are blended in here, before any adjustment, so every later setting \
                     applies to the fill exactly as to the rest of the photo.",
                ),
                step(
                    "Camera colour and Exposure.",
                    "The camera's colour matrix, adapted to the chosen white point, converts \
                     to a wide working space (linear Rec. 2020); Exposure scales the light \
                     here. Masks add their local Exposure, Temperature and Tint.",
                ),
                step(
                    "Dehaze, Highlights and Shadows,",
                    "in that order, with the masks' local amounts and local Contrast.",
                ),
                step(
                    "The tone curve:",
                    "one curve per channel made of Blacks, Whites (a soft highlight \
                     shoulder) and Contrast, then the Curves card: the parametric curve, \
                     the RGB curve and the red, green and blue curves.",
                ),
                step(
                    "Colour, in Oklab:",
                    "Color Mixer, Color Grading, Vibrance and Saturation, then Point Colors. \
                     Masks add their local Saturation.",
                ),
                step(
                    "The output colour space.",
                    "Colours outside it are pulled toward their brightness, keeping their \
                     hue, instead of clipping into a different colour.",
                ),
                step(
                    "Texture and Clarity,",
                    "local contrast at a small and a large radius.",
                ),
                step(
                    "Straighten and resize.",
                    "A turned crop is resampled upright, then the photo is scaled to the \
                     preview or export size.",
                ),
                step(
                    "Encode and sharpen:",
                    "the output space's tone response, then Sharpening, at the output size.",
                ),
                Block::Heading("What follows from the order"),
                term(
                    "White Balance and Exposure",
                    "act on the scene's light before any tone step, so they behave like \
                     changing the shot rather than editing a finished picture.",
                ),
                term(
                    "Masks",
                    "add their local amounts within steps 3 to 6, on top of the global \
                     sliders.",
                ),
                term(
                    "Sharpening",
                    "comes last, at the size you export, so a small export is sharpened for \
                     its own pixels.",
                ),
                term(
                    "Texture and Clarity",
                    "scale with the photo, so a preview and a full-size export look alike.",
                ),
                term(
                    "Previews",
                    "render in the background at the size the viewer needs (more when \
                     zoomed in). The newest change always wins, so dragging a slider stays \
                     smooth.",
                ),
                tip(
                    "Each sidecar records the rendering process it was made with. A change \
                     to how settings render ships as a new process, so earlier edits keep \
                     their look.",
                ),
            ],
            see_also: &[
                Page::WhiteBalance,
                Page::Light,
                Page::ColorMixer,
                Page::Masks,
            ],
        },
        Page::WhiteBalance => Content {
            lede: "White Balance sets which colour of light counts as neutral: the colour \
                   cast you remove, or keep."
                .into(),
            blocks: vec![
                term(
                    "As Shot",
                    "uses the neutral the camera recorded when you took the photo.",
                ),
                term(
                    "Presets",
                    "(Daylight, Cloudy, Shade, Tungsten, Fluorescent, Flash) set the typical \
                     temperature and tint of that light. Moving a slider makes the white \
                     balance Custom.",
                ),
                term(
                    "Temperature",
                    "(2000 to 12000 K) is the colour of the light in the scene. A lower \
                     value cools the photo, as for warm tungsten light; a higher one warms \
                     it.",
                ),
                term(
                    "Tint",
                    "corrects along the green–magenta axis, for fluorescent light or a \
                     green cast from foliage.",
                ),
                term(
                    "Eyedropper:",
                    "click its button, then something in the photo that should be neutral \
                     grey: a wall, a cloud, a grey card. Photonation averages a small patch \
                     there and sets the temperature and tint that make it neutral. Esc \
                     cancels the picker.",
                ),
                Block::Heading("How it works"),
                text(
                    "The white balance scales the raw channels before demosaicing, as a \
                     camera does. The camera's colour matrix is interpolated for the chosen \
                     white point between the file's calibrations, as DNG converters do, so \
                     colours stay right at any temperature.",
                ),
                tip(
                    "Masks have their own Temperature and Tint, to warm or cool part of the photo.",
                ),
            ],
            see_also: &[Page::Developing, Page::Masks],
        },
        Page::Light => Content {
            lede: "The Light card sets the photo's brightness and tonal range: how bright \
                   it is, how much contrast it has and where its blacks and whites sit."
                .into(),
            blocks: vec![
                term(
                    "Exposure",
                    "(±5 stops) multiplies the scene's light before any tone step, like a \
                     longer or shorter exposure. It adds to the file's own baseline \
                     exposure (DNG).",
                ),
                term(
                    "Contrast",
                    "bends an S-curve around mid-grey: positive values darken the darks and \
                     brighten the lights. Mid-grey itself stays put.",
                ),
                term(
                    "Highlights",
                    "darkens or brightens the bright areas only. Negative values bring back \
                     detail in a bright sky.",
                ),
                term(
                    "Shadows",
                    "lifts or deepens the dark areas only, to open up shade without \
                     brightening the rest.",
                ),
                term(
                    "Whites",
                    "moves the white point, where the soft highlight shoulder reaches pure \
                     white: raise it for brighter whites, lower it to keep the brightest \
                     tones from clipping.",
                ),
                term(
                    "Blacks",
                    "moves the black point: lower for deeper blacks, higher for a faded \
                     look.",
                ),
                Block::Heading("How they work together"),
                text(
                    "Highlights and Shadows shift exposure by how many stops each pixel lies \
                     above or below mid-grey, so bright or dark areas move without the \
                     rest. Blacks, Whites and Contrast then form one tone curve together \
                     with the Curves card. Bright tones roll off into white along a soft \
                     shoulder instead of clipping hard.",
                ),
                tip(
                    "Watch the histogram while you adjust: tones piled against its left or \
                     right edge are clipping to black or white.",
                ),
            ],
            see_also: &[Page::Curves, Page::Presence, Page::Developing],
        },
        Page::Presence => Content {
            lede: "Presence controls local contrast and haze: how crisp, punchy or soft the \
                   photo looks, rather than how bright it is."
                .into(),
            blocks: vec![
                term(
                    "Texture",
                    "boosts or smooths fine detail (skin, foliage, fabric) at a small \
                     radius, leaving larger shapes alone.",
                ),
                term(
                    "Clarity",
                    "does the same at a large radius, weighted to the midtones: local \
                     contrast and punch. Negative values give a soft glow.",
                ),
                term(
                    "Dehaze",
                    "removes a veil of haze, fog or glare and restores some of the colour it \
                     washed out. Negative values add a veil.",
                ),
                Block::Heading("How it works"),
                text(
                    "Texture and Clarity compare each pixel's brightness with a blurred copy \
                     and add back the difference, so colours are unaffected. Their radii \
                     are a share of the photo's long edge, so a preview and a full-size \
                     export match. Dehaze runs early, right after Exposure, and masks can \
                     add their own.",
                ),
            ],
            see_also: &[Page::Light, Page::Detail],
        },
        Page::Color => Content {
            lede: "The Color card changes how colourful the whole photo is.".into(),
            blocks: vec![
                term(
                    "Vibrance",
                    "raises the saturation of muted colours more than of strong ones, so \
                     skin and already vivid colours don't overdo it. Negative values mute \
                     the quiet colours first.",
                ),
                term(
                    "Saturation",
                    "scales every colour's intensity equally. −100 gives black and white.",
                ),
                Block::Heading("How it works"),
                text(
                    "Both work in Oklab, a perceptual colour space, so changing \
                     colourfulness leaves brightness and hue alone. Colours pushed outside \
                     the output colour space are pulled toward their brightness, keeping \
                     their hue, instead of clipping into a different colour.",
                ),
                tip(
                    "For one colour, use the Color Mixer or Point Color; for part of the \
                     photo, a mask's Saturation.",
                ),
            ],
            see_also: &[Page::ColorMixer, Page::ColorGrading, Page::Masks],
        },
        Page::Curves => Content {
            lede: "Curves reshape the tones precisely, on top of the Light card: the \
                   brightness of everything, or of the red, green and blue channels on \
                   their own."
                .into(),
            blocks: vec![
                term(
                    "Parametric",
                    "raises or lowers four tonal regions, Highlights, Lights, Darks and \
                     Shadows, with sliders. Its smooth shape is hard to overdo.",
                ),
                term(
                    "RGB",
                    "is a point curve for all three channels: click the curve to add a \
                     point, drag points to shape it, double-click a point to remove it. \
                     Moving the end points sets the black and white points.",
                ),
                term(
                    "Red, Green, Blue",
                    "are point curves for one channel each, to tint the shadows or the \
                     highlights. Raising blue in the shadows cools them; lowering it warms \
                     them.",
                ),
                Block::Heading("How it works"),
                text(
                    "All curves combine with the Light card's Blacks, Whites and Contrast \
                     into one lookup per channel: the base curve, then the parametric \
                     curve, then the RGB curve, then the channel's own curve. Point curves \
                     are smooth splines that never overshoot between their points.",
                ),
                tip("Lightroom's parametric and point curves import into this card."),
            ],
            see_also: &[Page::Light, Page::ColorGrading],
        },
        Page::ColorMixer => Content {
            lede: "The Color Mixer changes colours by their hue: all the blues at once, or \
                   one exact colour you pick in the photo."
                .into(),
            blocks: vec![
                Block::Heading("Mixer"),
                term(
                    "Eight bands",
                    "(Red, Orange, Yellow, Green, Aqua, Blue, Purple, Magenta) each have Hue, \
                     Saturation and Luminance. Hue shifts the band toward its neighbours \
                     (greens toward yellow, say), Saturation makes it more or less \
                     colourful, Luminance brighter or darker.",
                ),
                term(
                    "Smooth blends:",
                    "a pixel takes a blend of its two nearest bands, so edits never break \
                     into steps. Luminance changes coloured pixels only, never greys.",
                ),
                term(
                    "Eyedropper:",
                    "click it, then a colour in the photo, to select that colour's band.",
                ),
                Block::Heading("Point Color"),
                term(
                    "Pick",
                    "a colour in the photo with the Point Color tab's picker, up to 8 per \
                     photo. It selects that colour and colours like it; the swatch shows it \
                     before and after.",
                ),
                term(
                    "Shifts:",
                    "Hue Shift (up to ±60°), Saturation Shift and Luminance Shift change \
                     what is selected.",
                ),
                term(
                    "Ranges:",
                    "Hue Range, Saturation Range and Luminance Range set how far from the \
                     picked colour the selection reaches. It is full close to the colour \
                     and fades smoothly to nothing at the range's edge.",
                ),
                term(
                    "Visualize range",
                    "greys out everything the selected point colour leaves alone, so you can \
                     see and tune what it picks.",
                ),
                text(
                    "Greys are never selected by a coloured point colour. Every point colour \
                     looks at the photo before any of them applies, so their order doesn't \
                     matter.",
                ),
                tip(
                    "Point Color is the precise tool for one colour, like a jacket or a sky; \
                     the Mixer is quicker for whole families of hues. The card's reset \
                     clears both.",
                ),
            ],
            see_also: &[Page::Color, Page::ColorGrading, Page::Masks],
        },
        Page::ColorGrading => Content {
            lede: "Color Grading tints the shadows, midtones and highlights separately: the \
                   split-toned colour looks of film and cinema."
                .into(),
            blocks: vec![
                term(
                    "Wheels:",
                    "Shadows, Midtones, Highlights and Global. Drag on a wheel: the \
                     direction picks the hue, the distance from the centre the strength. \
                     Double-click a wheel to clear it. The Luminance slider under it \
                     brightens or darkens that range.",
                ),
                term(
                    "Balance",
                    "moves the border between shadows and highlights, giving more of the \
                     photo to the highlights' tint (positive) or the shadows' (negative).",
                ),
                term(
                    "Blending",
                    "sets how softly the three ranges overlap; higher values blend the tints \
                     into each other.",
                ),
                text(
                    "The ranges are measured on perceptual lightness (Oklab L); Global \
                     applies to every tone.",
                ),
                tip(
                    "Cool shadows and warm highlights at low strength give a classic look. \
                     Lightroom's split toning and colour grading import here.",
                ),
            ],
            see_also: &[Page::Curves, Page::ColorMixer],
        },
        Page::Detail => Content {
            lede: "Detail sharpens the finished photo.".into(),
            blocks: vec![
                term("Sharpening", "(0 to 150, default 40) sets the strength."),
                term(
                    "Radius",
                    "(0.5 to 3 pixels) sets the size of the detail it enhances: small for \
                     fine texture, larger for bolder edges.",
                ),
                Block::Heading("How it works"),
                text(
                    "Sharpening is an unsharp mask on brightness only, so it adds no colour \
                     fringes. It runs last, after the photo is scaled to its output size, so \
                     a small web export and a full-size print are each sharpened for their \
                     own pixels.",
                ),
                tip("Judge sharpening at 1:1, where one photo pixel is one screen pixel."),
            ],
            see_also: &[Page::CompareZoom, Page::Export, Page::Presence],
        },
        Page::Masks => Content {
            lede: "Masks apply adjustments to part of the photo: darken a sky, brighten a \
                   face, warm a sunset. Open them with the layers button or M; Done or M \
                   brings back the adjustment cards."
                .into(),
            blocks: vec![
                Block::Heading("Shapes"),
                term(
                    "+ Linear",
                    "adds a gradient: full effect up to the first line, fading to none at \
                     the second. Drag the lines and their handles on the photo.",
                ),
                term(
                    "+ Radial",
                    format!(
                        "adds an ellipse that fades toward its edge. Drag its centre and \
                         handles ({shift} keeps a circle round); Feather sets how much of it \
                         fades, Rotation turns it."
                    ),
                ),
                term(
                    "Combining:",
                    "a mask holds up to 8 shapes. Each further shape Adds to, Subtracts from \
                     or Intersects with what the mask selects so far, and Invert flips the \
                     result to adjust everything outside it.",
                ),
                term(
                    "On the photo",
                    "the selected shape shows its handles, hollow amber rings mark the \
                     mask's other shapes, and pins the other masks. Click one to select it.",
                ),
                term(
                    "The overlay",
                    "tints red where the selected mask applies, as strongly as it applies. \
                     It has its own switch in the panel.",
                ),
                Block::Heading("Adjustments"),
                term(
                    "Local sliders:",
                    "Temperature, Tint, Exposure, Contrast, Highlights, Shadows, Dehaze and \
                     Saturation apply where the mask selects, in addition to the global \
                     sliders.",
                ),
                term(
                    "Amount",
                    "(0–100 %) scales all of a mask's adjustments at once.",
                ),
                text(
                    "Masks add up: where two overlap, both apply, so their order in the list \
                     doesn't change the result.",
                ),
                Block::Heading("Layers and groups"),
                term(
                    "The layer list",
                    "shows each mask with a picture of what it selects, its name and a \
                     summary. Click a row to select it, drag it to reorder, switch it off to \
                     hide it, right-click to invert, duplicate, move, group or delete it. A \
                     photo holds up to 32 masks.",
                ),
                term(
                    "+ Group",
                    "puts the selected mask in a new group. A group's switch and Amount \
                     apply to all its masks at once (a mask renders at its amount times its \
                     group's). Click the arrow to fold a group, double-click its name to \
                     rename it, and drop a mask on its header to add it.",
                ),
                text(
                    "Masks sit on the whole photo, before the crop, so they stay on their \
                     subject when you change the crop.",
                ),
            ],
            see_also: &[Page::Developing, Page::Crop, Page::Remove],
        },
        Page::Crop => Content {
            lede: "The crop tool frames and straightens the photo. Open it with the crop \
                   button or R; Return or Esc closes it."
                .into(),
            blocks: vec![
                text(
                    "While it is open you see the whole photo, with the crop frame over it, \
                     a rule-of-thirds grid and the outside dimmed.",
                ),
                term(
                    "Move:",
                    "drag inside the frame; it slides along the photo's edges.",
                ),
                term(
                    "Resize:",
                    "drag a corner or an edge. The opposite side stays put and the frame \
                     stops at the photo's edge.",
                ),
                term(
                    "Straighten:",
                    "drag outside the frame to turn it around its centre, or use the Angle \
                     slider (±45°). The frame shrinks to stay on the photo, and turning back \
                     gives it its size back.",
                ),
                term(
                    "Aspect",
                    "keeps a ratio: Original (the default), Free, 1:1, 4:5, 5:7, 2:3 or \
                     9:16, in the frame's orientation. The swap button turns portrait into \
                     landscape and back.",
                ),
                term("Reset", "removes the crop and the angle."),
                text(
                    "Masks and removals stay on their content when the crop changes, and \
                     Compare frames the original like the edit. Lightroom crops import \
                     exactly, angle included.",
                ),
            ],
            see_also: &[Page::CompareZoom, Page::Masks, Page::Export],
        },
        Page::Remove => Content {
            lede: "Remove paints out what should not be in the photo, like a passer-by, a \
                   fence post or a sensor speck, and fills it in. Open it with the retouch \
                   button or Q."
                .into(),
            blocks: vec![
                Block::Heading("Painting"),
                term(
                    "Paint",
                    format!(
                        "over what should go: drag to paint, click to dab. Each stroke is a \
                         new removal; {shift}-paint adds to the selected one."
                    ),
                ),
                term(
                    "Brush Size",
                    "is a share of the photo's long edge, so it covers the same part of the \
                     photo at any zoom.",
                ),
                term(
                    "The list",
                    "shows each removal: click to select it, switch it off to compare, × to \
                     delete it. The selected removal is drawn red on the photo, the others \
                     grey. A photo holds up to 64.",
                ),
                Block::Heading("Fills"),
                term(
                    "Generative AI",
                    "has a neural network (LaMa or MI-GAN) invent content that fits, then \
                     rebuilds it from the photo's own texture. Best for structure: fences, \
                     people, large areas.",
                ),
                term(
                    "Content-aware",
                    "copies patches from the rest of the photo. Best for texture: grass, \
                     sky, water, walls. Try Another Fill picks different patches.",
                ),
                term("Feather", "softens the removal's edge."),
                text(
                    "Fills are computed in the background, while a spinner shows by the \
                     panel's title, and kept in memory; reopening a photo computes them \
                     again. The sidecar stores only the brush strokes. Removals come before \
                     every adjustment, so the fill matches the rest of the photo.",
                ),
                Block::Heading("Installing a model"),
                Block::Text(format!(
                    "Photonation ships no model. Put a LaMa or MI-GAN ONNX file at \
                     {DATA_FOLDER}{sep}models{sep}inpaint.onnx, or set \
                     PHOTONATION_INPAINT_MODEL to its path. The panel shows whether it \
                     loaded; without a model, Generative AI uses content-aware fill.",
                    sep = std::path::MAIN_SEPARATOR
                )),
                tip(
                    "Removals apply in order and each sees the ones before it, so changing \
                     an early removal fills the later ones again.",
                ),
            ],
            see_also: &[Page::Developing, Page::Masks],
        },
        Page::CompareZoom => Content {
            lede: "Compare your edit with the original, and zoom in to judge detail.".into(),
            blocks: vec![
                Block::Heading("Compare"),
                term(
                    "Compare (\\)",
                    "splits the photo: the unedited original left of the handle, your edit \
                     right of it. Drag the handle across the photo. The original is framed \
                     like the edit.",
                ),
                term(
                    "A card's switch",
                    "turns that card off, to compare just one adjustment.",
                ),
                Block::Heading("Zoom"),
                term(
                    "Fit",
                    format!("({}) shows the whole photo.", theme::SHORTCUT_FIT),
                ),
                term(
                    "1:1",
                    "shows one photo pixel per screen pixel, to judge focus, noise and \
                     sharpening (View › Actual Pixels).",
                ),
                term("Double-click", "the photo to switch between Fit and 1:1."),
                term(
                    "Pinch",
                    format!("or {command}-scroll to zoom in or out around the pointer."),
                ),
                term("Drag or scroll", "to move around a zoomed photo."),
                text(
                    "Zoomed previews render at the resolution the zoom needs, up to full \
                     size. While you drag a slider they stay at the fitted size, for speed, \
                     and sharpen when you let go.",
                ),
            ],
            see_also: &[Page::Detail, Page::Crop],
        },
        Page::Export => Content {
            lede: format!(
                "Export saves the edited photo as a new file: the Export button, File › \
                 Export… or {}. The raw file and its edits stay as they are.",
                theme::SHORTCUT_EXPORT
            ),
            blocks: vec![
                Block::Heading("Format and size"),
                term(
                    "JPEG",
                    "with Quality from 50 to 100 (default 92): small files for sharing.",
                ),
                term(
                    "PNG and TIFF",
                    "are lossless, in 8 or 16 bits per channel (TIFF starts at 16). 16 bits \
                     keeps gradients smooth for further editing.",
                ),
                term(
                    "Size:",
                    "Original, or Long Edge in pixels (256 to 20 000, default 2048). \
                     Sharpening is applied at this size.",
                ),
                Block::Heading("Colour spaces"),
                term("sRGB", "for the web and most screens: the safe choice."),
                term(
                    "Display P3",
                    "for wide-gamut screens such as recent phones, tablets and Macs.",
                ),
                term("Adobe RGB (1998)", "for print work."),
                term(
                    "ProPhoto RGB",
                    "is the widest, for further editing in 16 bits. It looks dull in apps \
                     that ignore colour profiles.",
                ),
                text(
                    "The colour space's profile is embedded in every file, so \
                     colour-managed apps show it correctly.",
                ),
                Block::Heading("File names"),
                term(
                    NAME_TOKEN,
                    "is the original file name without its extension.",
                ),
                term(
                    NUMBER_TOKEN,
                    format!(
                        "is the photo's number in the batch, padded to the batch's size: \
                         Trip {NUMBER_TOKEN} gives Trip 01 to Trip 12."
                    ),
                ),
                Block::Text(format!(
                    "A batch named without either gets -{NUMBER_TOKEN} added, so the files \
                     stay apart. Exports never replace a file: a clash adds -2, -3 and so \
                     on. The sheet previews the resulting name."
                )),
                Block::Heading("Metadata"),
                term(
                    "All, Copyright Only or None",
                    "choose quickly, or tick the groups yourself:",
                ),
                term(
                    "Camera and capture:",
                    "make, model, lens, exposure and capture time.",
                ),
                term(
                    "Copyright and creator:",
                    "what you enter in the two fields, else the raw file's own.",
                ),
                term(
                    "Location (GPS):",
                    "where the photo was taken. Left out unless ticked, for privacy.",
                ),
                text("The choice and what you enter are remembered between launches."),
                Block::Heading("Where files go"),
                text(
                    "Export asks for a folder, starting in the one chosen in Settings, or, \
                     if Settings says so, exports straight to it. With more than one photo \
                     in the folder, the sheet can export them all (File › Export All \
                     Photos…): the selected photos while several are selected, else the \
                     ones the filter shows, so exporting the picks takes a filter and one \
                     click. Exports run in the background, and the toolbar shows their \
                     progress.",
                ),
                tip("New exports start with the defaults in Settings › Export."),
            ],
            see_also: &[Page::Settings, Page::Detail],
        },
        Page::Settings => Content {
            lede: format!(
                "Settings (File › Settings…, {}) holds the choices that apply to every \
                 photo. They are saved when you close the window, in preferences.json in \
                 {DATA_FOLDER}.",
                theme::SHORTCUT_SETTINGS
            ),
            blocks: vec![
                Block::Heading("General"),
                term(
                    "Graphics processor",
                    "draws the interface, the previews and the mask overlay: Automatic \
                     (a dedicated graphics card first), integrated graphics first, software \
                     rendering, or one adapter by name. The one in use is shown; a change \
                     applies when Photonation starts again. Developing and exporting run on \
                     the CPU, on all its cores.",
                ),
                text(
                    "The PHOTONATION_GPU environment variable (integrated or software), if \
                     set, decides instead.",
                ),
                term(
                    "Export folder",
                    "is where the export's folder dialog starts. With Export straight to \
                     this folder, exports go there without asking; Clear forgets it.",
                ),
                Block::Heading("Export"),
                term(
                    "New exports start with",
                    "this format, quality or bit depth, size, colour space, file name and \
                     metadata. The export sheet can still change any of them for one \
                     export.",
                ),
                tip(
                    "If the window draws wrongly or Photonation doesn't start on a laptop, \
                     choose integrated graphics or software rendering.",
                ),
            ],
            see_also: &[Page::Export],
        },
        Page::Lightroom => Content {
            lede: "Photos you edited in Lightroom open with their Lightroom settings, so \
                   you can carry on where you left off."
                .into(),
            blocks: vec![
                term(
                    "When:",
                    "a raw file has a Lightroom XMP sidecar beside it (IMG_0001.xmp) and no \
                     Photonation sidecar. Photonation only reads the XMP, never writes it.",
                ),
                term(
                    "Imported:",
                    "white balance, the Light, Presence and Color sliders, parametric and \
                     point curves, the colour mixer, split toning and colour grading, \
                     sharpening, and the crop with its angle.",
                ),
                term(
                    "Not imported:",
                    "masks, spot removal, noise reduction, lens corrections, Upright, \
                     effects and black & white. A message names what was left out when the \
                     photo opens.",
                ),
                text(
                    "The sliders share Lightroom's scales, but the rendering differs, so \
                     imported photos look similar rather than identical. Crops match to the \
                     pixel.",
                ),
                term(
                    "Afterwards",
                    "your first edit saves a Photonation sidecar, which takes over from the \
                     XMP. Resetting such a photo keeps a sidecar with the defaults, so the \
                     Lightroom edits don't come back.",
                ),
                tip(
                    "Lightroom Classic writes XMP sidecars with Metadata › Save Metadata to \
                     File, or by itself with Automatically write changes into XMP in its \
                     catalog settings.",
                ),
            ],
            see_also: &[Page::GettingStarted, Page::Crop],
        },
        Page::Shortcuts => Content {
            lede: "Letter and arrow keys work whenever you are not typing in a text field.".into(),
            blocks: vec![
                Block::Heading("Photos"),
                keys(&[theme::SHORTCUT_OPEN], "Open a folder"),
                keys(&["←", "→"], "Previous or next photo"),
                keys(&["G"], "Grid"),
                keys(&["N"], "Survey the selected photos"),
                keys(&["E", "Return"], "Open the photo in the editor"),
                keys(&[theme::SHORTCUT_EXPORT], "Export"),
                keys(&[theme::SHORTCUT_SETTINGS], "Settings"),
                keys(&[theme::SHORTCUT_QUIT], "Quit"),
                Block::Heading("Rating"),
                keys(
                    &["0", "1", "2", "3", "4", "5"],
                    "Set the stars (0 clears them)",
                ),
                keys(&["]", "["], "One star more or fewer"),
                keys(&["P"], "Pick"),
                keys(&["X"], "Reject"),
                keys(&["U"], "Remove the flag"),
                keys(&["6", "7", "8", "9"], "Red, yellow, green or blue label"),
                keys(
                    &[theme::SHORTCUT_SELECT_ALL],
                    "Select every shown photo (grid)",
                ),
                keys(&["↑", "↓"], "Move a row up or down (grid)"),
                keys(&["Esc"], "Leave the survey, or clear the selection"),
                Block::Heading("Editing"),
                keys(&[theme::SHORTCUT_UNDO], "Undo"),
                keys(&[theme::SHORTCUT_REDO], "Redo"),
                Block::Heading("View"),
                keys(&["\\"], "Compare with the original"),
                keys(&[theme::SHORTCUT_FIT], "Fit to window"),
                Block::Heading("Tools"),
                keys(&["M"], "Masks"),
                keys(&["R"], "Crop and straighten"),
                keys(&["Return", "Esc"], "Close the crop tool"),
                keys(&["Q"], "Remove"),
                keys(&["Esc"], "Cancel an eyedropper"),
                Block::Heading("Help"),
                keys(&["F1"], "Open Help, on the page of the tool in use"),
                keys(&["Esc"], "Close Help"),
                Block::Heading("Mouse and trackpad"),
                keys(
                    &[format!("{alt}-drag").as_str()],
                    "Fine control on a slider",
                ),
                keys(&["Double-click"], "Reset a slider or a colour wheel"),
                keys(&["Double-click"], "Switch the photo between Fit and 1:1"),
                keys(
                    &["Pinch", format!("{command}-scroll").as_str()],
                    "Zoom around the pointer",
                ),
                keys(&["Drag", "Scroll"], "Move around a zoomed photo"),
                keys(
                    &[format!("{shift}-drag").as_str()],
                    "Keep a radial mask round",
                ),
                keys(
                    &[format!("{shift}-paint").as_str()],
                    "Add to the selected removal",
                ),
            ],
            see_also: &[Page::GettingStarted],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_is_listed_once() {
        let listed: Vec<Page> = CHAPTERS
            .iter()
            .flat_map(|(_, pages)| pages.iter().copied())
            .collect();
        assert_eq!(listed, Page::ALL);
    }

    #[test]
    fn every_page_has_text_and_links_elsewhere() {
        for page in Page::ALL {
            let content = content(page);
            assert!(!content.lede.is_empty(), "{page:?} has an opening line");
            assert!(!content.blocks.is_empty(), "{page:?} has content");
            assert!(
                !content.see_also.contains(&page),
                "{page:?} does not link to itself"
            );
            assert_eq!(Page::from_title(page.title()), Some(page));
        }
    }

    #[test]
    fn help_opens_on_the_tool_in_use() {
        let mut view = ViewState::default();
        view.help.page = Page::Curves;
        open(&mut view);
        assert!(view.help.open);
        assert_eq!(
            view.help.page,
            Page::Curves,
            "the last page, without a tool"
        );
        view.help.open = false;
        view.toggle_cropping();
        open(&mut view);
        assert_eq!(view.help.page, Page::Crop);
        view.show_export = true;
        open(&mut view);
        assert_eq!(view.help.page, Page::Export, "the sheet is in front");
    }
}
