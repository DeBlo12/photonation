// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The folder overview, in place of the editor: a card for each folder below
//! the open one, with a cover photo and how many photos it holds. Also the
//! breadcrumb in the toolbar that leads back up. See docs/DESKTOP.md.

use crate::app::PhotonationApp;
use crate::folders::{FolderCard, FolderRequest};
use crate::panels::grid::{GAP, columns_of};
use crate::panels::tool_button;
use crate::theme;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Rect, RichText, Sense, Stroke,
    StrokeKind, TextWrapMode, pos2, vec2,
};
use std::path::Path;

/// The cover's width to its height.
const COVER_ASPECT: f32 = 1.5;
/// Space around the cover and the text inside a card.
const CARD_PADDING: f32 = 8.0;
/// The folder's name and its count below the cover.
const CARD_TEXT_HEIGHT: f32 = 42.0;
/// Folder names longer than this shorten in the breadcrumb.
const MAX_CRUMB_CHARS: usize = 22;
/// The size of a breadcrumb button's text (see `tool_button`).
const CRUMB_TEXT_SIZE: f32 = 13.0;
/// A `tool_button` is at least this wide.
const MIN_CRUMB_WIDTH: f32 = 30.0;
const SEPARATOR: &str = "›";
/// Stands for the folders the breadcrumb has no room to name.
const ELLIPSIS: &str = "…";
/// Goes up one folder when the breadcrumb has no room for any names.
const UP: &str = "‹";

pub fn show(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    egui::CentralPanel::no_frame()
        .frame(
            egui::Frame::NONE
                .fill(theme::WINDOW)
                .inner_margin(egui::Margin::symmetric(16, 8)),
        )
        .show(ui, |ui| {
            let canvas = ui.max_rect();
            header(app, ui);
            ui.add_space(8.0);
            if app.folders.subfolders.is_empty() {
                if !app.folders.scanning {
                    nothing_found(app, ui);
                }
            } else {
                cards(app, ui);
            }
            crate::panels::viewer::toast(app, ui, canvas);
        });
}

/// The folder's name, what is below it and whether more is still turning up.
fn header(app: &PhotonationApp, ui: &mut egui::Ui) {
    let folders = &app.folders;
    ui.horizontal(|ui| {
        if let Some(folder) = &app.folder {
            ui.label(
                RichText::new(folder_name(folder))
                    .font(theme::title(15.0))
                    .color(theme::TEXT),
            );
        }
        let found = folders.subfolders.len();
        if found > 0 {
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!(
                    "{}  ·  {}",
                    count_label(found, "folder"),
                    count_label(folders.photos_below(), "photo"),
                ))
                .font(theme::body(12.5))
                .color(theme::TEXT_SECONDARY),
            );
        }
        if folders.scanning {
            ui.add_space(6.0);
            ui.add(egui::Spinner::new().size(14.0));
            if found == 0 {
                ui.label(
                    RichText::new("Looking for raw photos…")
                        .font(theme::body(12.5))
                        .color(theme::TEXT_SECONDARY),
                );
            }
        }
    });
}

fn nothing_found(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);
        ui.label(
            RichText::new("No raw photos in this folder or the folders inside it")
                .size(15.0)
                .color(theme::TEXT_SECONDARY),
        );
        ui.add_space(12.0);
        if ui.add(theme::primary_button("Open Folder…")).clicked() {
            app.pick_folder();
        }
    });
}

fn cards(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (columns, card_width) = columns_of(width, app.view.library.grid_size);
    let cover_height = (card_width - 2.0 * CARD_PADDING) / COVER_ASPECT;
    let card_height = CARD_PADDING + cover_height + CARD_TEXT_HEIGHT;
    let row_height = card_height + GAP;
    let rows = app.folders.subfolders.len().div_ceil(columns);
    let mut opened = None;
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show_rows(ui, row_height, rows, |ui, row_range| {
            for row in row_range {
                let (row_rect, _) = ui.allocate_exact_size(vec2(width, row_height), Sense::hover());
                let in_row = app.folders.subfolders.iter().skip(row * columns);
                for (column, card) in in_row.take(columns).enumerate() {
                    let left = row_rect.left() + column as f32 * (card_width + GAP);
                    let rect = Rect::from_min_size(
                        pos2(left, row_rect.top()),
                        vec2(card_width, card_height),
                    );
                    if card_ui(ui, rect, card).clicked() {
                        opened = Some(card.subfolder.path.clone());
                    }
                }
            }
        });
    if let Some(folder) = opened {
        app.folders.request(FolderRequest::Open(folder));
    }
}

fn card_ui(ui: &egui::Ui, rect: Rect, card: &FolderCard) -> egui::Response {
    let subfolder = &card.subfolder;
    let id = ui.scope_id().with(("folder-card", &subfolder.path));
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let painter = ui.painter();
    let radius = CornerRadius::same(theme::CARD_RADIUS);
    let hovered = response.hovered();
    painter.rect_filled(
        rect,
        radius,
        if hovered { theme::CARD } else { theme::SIDEBAR },
    );
    if hovered {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.5, theme::TEXT_SECONDARY),
            StrokeKind::Inside,
        );
    }

    let inner_width = rect.width() - 2.0 * CARD_PADDING;
    let cover = Rect::from_min_size(
        rect.min + vec2(CARD_PADDING, CARD_PADDING),
        vec2(inner_width, inner_width / COVER_ASPECT),
    );
    let corner = CornerRadius::same(3);
    match &card.cover_image {
        Some(texture) => {
            let uv = filling_uv(texture.aspect_ratio(), COVER_ASPECT);
            painter.add(
                egui::epaint::RectShape::filled(cover, corner, Color32::WHITE)
                    .with_texture(texture.id(), uv),
            );
        }
        None => {
            painter.rect_filled(cover, corner, theme::CONTROL);
        }
    }

    let name = folder_name(&subfolder.path);
    let name_galley = ui.painter().layout_job(single_line(
        name.clone(),
        theme::title(13.0),
        theme::TEXT,
        inner_width,
    ));
    let name_top = cover.bottom() + 7.0;
    painter.galley(pos2(cover.left(), name_top), name_galley, theme::TEXT);
    painter.text(
        pos2(cover.left(), name_top + 18.0),
        Align2::LEFT_TOP,
        count_label(subfolder.photo_count, "photo"),
        theme::body(12.0),
        theme::TEXT_SECONDARY,
    );
    response.on_hover_text(name)
}

fn single_line(text: String, font: FontId, color: Color32, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
    job.wrap = egui::text::TextWrapping::from_wrap_mode_and_width(TextWrapMode::Truncate, width);
    job
}

/// The middle of a texture that fills a frame of another shape without
/// stretching, as texture coordinates.
fn filling_uv(texture_aspect: f32, frame_aspect: f32) -> Rect {
    let (width, height) = if texture_aspect > frame_aspect {
        (frame_aspect / texture_aspect, 1.0)
    } else {
        (1.0, texture_aspect / frame_aspect)
    };
    Rect::from_center_size(pos2(0.5, 0.5), vec2(width, height))
}

/// The way from the folder first opened down to the open one, then a button
/// for the folders inside it when it has photos of its own too. Draws
/// nothing at the folder first opened unless that button is there: the
/// overview's heading already names it, and there is nowhere higher. Within
/// `room`, it drops the folders nearest the top first (a `…` leads to them),
/// then every name, leaving a `‹` that goes up one folder.
pub fn breadcrumb(app: &mut PhotonationApp, ui: &mut egui::Ui, room: f32) {
    let Some(folder) = app.folder.clone() else {
        return;
    };
    let trail = app.folders.trail_to(&folder);
    let subfolder_count = app.folders.subfolders.len();
    let has_both = subfolder_count > 0 && !app.photos.is_empty();
    if trail.len() < 2 && !has_both {
        return;
    }
    let chip = has_both.then(|| count_label(subfolder_count, "subfolder"));
    let spacing = ui.spacing().item_spacing.x;
    let chip_width = chip
        .as_ref()
        .map_or(0.0, |label| crumb_width(ui, label) + spacing);
    let labels: Vec<String> = trail
        .iter()
        .map(|path| shortened(&folder_name(path)))
        .collect();
    let widths: Vec<f32> = labels.iter().map(|label| crumb_width(ui, label)).collect();
    let separator = text_width(ui, SEPARATOR);
    let lead = crumb_width(ui, ELLIPSIS);
    let names_room = room - chip_width;

    ui.add_space(6.0);
    let parent = trail.len().checked_sub(2).map(|index| trail[index].clone());
    let mut request = None;
    match first_crumb_that_fits(
        &widths,
        separator + 2.0 * spacing,
        lead + spacing,
        names_room,
    ) {
        Some(first) => {
            if first > 0 {
                let tip = format!("Go up\nShows {}.", labels[first - 1]);
                if tool_button(ui, ELLIPSIS, false, &tip).clicked() {
                    request = Some(FolderRequest::Open(trail[first - 1].clone()));
                }
                separator_label(ui);
            }
            let last = trail.len() - 1;
            for (index, path) in trail.iter().enumerate().skip(first) {
                if index < last {
                    let tip = format!("Go up\nShows {}.", folder_name(path));
                    if tool_button(ui, &labels[index], false, &tip).clicked() {
                        request = Some(FolderRequest::Open(path.clone()));
                    }
                    separator_label(ui);
                } else if app.folders.showing_over_photos() {
                    let tip = "Back to the photos\nShows this folder's own photos again.";
                    if tool_button(ui, &labels[index], false, tip).clicked() {
                        request = Some(FolderRequest::ShowPhotos);
                    }
                } else {
                    ui.label(
                        RichText::new(&labels[index])
                            .size(CRUMB_TEXT_SIZE)
                            .color(theme::TEXT_SECONDARY),
                    );
                }
            }
        }
        None => {
            if let Some(parent) = parent.filter(|_| names_room >= MIN_CRUMB_WIDTH) {
                let tip = format!("Go up\nShows {}.", folder_name(&parent));
                if tool_button(ui, UP, false, &tip).clicked() {
                    request = Some(FolderRequest::Open(parent));
                }
            }
        }
    }
    if let Some(chip) = chip.filter(|_| room >= chip_width) {
        let showing = app.folders.showing_over_photos();
        let tip = if showing {
            "Back to the photos\nShows this folder's own photos again."
        } else {
            "Folders inside this one\nShows them with a cover and how many photos they hold."
        };
        if tool_button(ui, &chip, showing, tip).clicked() {
            request = Some(if showing {
                FolderRequest::ShowPhotos
            } else {
                FolderRequest::ShowSubfolders
            });
        }
    }
    if let Some(request) = request {
        app.folders.request(request);
    }
}

fn separator_label(ui: &mut egui::Ui) {
    ui.label(
        RichText::new(SEPARATOR)
            .size(CRUMB_TEXT_SIZE)
            .color(theme::TEXT_TERTIARY),
    );
}

/// The topmost crumb from which the rest fit in `room`, each with its
/// `separator_width` and, when some are left out, the `lead_width` of what
/// stands for them. `None` when not even the last one fits.
fn first_crumb_that_fits(
    widths: &[f32],
    separator_width: f32,
    lead_width: f32,
    room: f32,
) -> Option<usize> {
    (0..widths.len()).find(|&first| {
        let shown = &widths[first..];
        let separators = (shown.len() - 1) as f32 * separator_width;
        let lead = if first > 0 {
            lead_width + separator_width
        } else {
            0.0
        };
        shown.iter().sum::<f32>() + separators + lead <= room
    })
}

fn text_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            FontId::proportional(CRUMB_TEXT_SIZE),
            theme::TEXT,
        )
        .size()
        .x
}

/// How wide a `tool_button` with this label comes out.
fn crumb_width(ui: &egui::Ui, label: &str) -> f32 {
    (text_width(ui, label) + 2.0 * ui.spacing().button_padding.x).max(MIN_CRUMB_WIDTH)
}

fn folder_name(folder: &Path) -> String {
    folder.file_name().map_or_else(
        || folder.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn shortened(name: &str) -> String {
    if name.chars().count() <= MAX_CRUMB_CHARS {
        return name.to_owned();
    }
    let kept: String = name.chars().take(MAX_CRUMB_CHARS - 1).collect();
    format!("{kept}{ELLIPSIS}")
}

fn count_label(count: usize, noun: &str) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {noun}{plural}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, RawInput};
    use photonation_engine::Subfolder;

    /// One frame of the overview, as `folders::show` draws it in the window.
    fn frame(app: &mut PhotonationApp, ctx: &egui::Context, events: Vec<Event>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 700.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| show(app, ui));
        output.textures_delta.clear();
    }

    #[test]
    fn clicking_a_card_opens_its_folder() {
        let root = tempfile::tempdir().expect("temporary folder");
        let month = root.path().join("2026_06");
        std::fs::create_dir(&month).expect("create folder");
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);
        app.open_folder(root.path().to_path_buf());
        app.subfolder_found(
            root.path(),
            Subfolder {
                path: month.clone(),
                photo_count: 1,
                cover: month.join("a.CR3"),
            },
        );

        // Well inside the first card, below the header.
        let on_the_card = pos2(80.0, 120.0);
        let button = |pressed| Event::PointerButton {
            pos: on_the_card,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut app, &ctx, vec![Event::PointerMoved(on_the_card)]);
        frame(&mut app, &ctx, vec![button(true)]);
        frame(&mut app, &ctx, vec![button(false)]);
        frame(&mut app, &ctx, Vec::new());
        app.apply_folder_request();
        assert_eq!(app.folder.as_deref(), Some(month.as_path()));
    }

    /// Clicks along the breadcrumb until a click asks for a move, and carries
    /// it out.
    fn click_along_the_breadcrumb(app: &mut PhotonationApp, ctx: &egui::Context) {
        let folder_before = app.folder.clone();
        let overview_before = app.showing_folder_overview();
        for x in (10..500).step_by(6) {
            let pos = pos2(x as f32, 20.0);
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            for events in [
                vec![Event::PointerMoved(pos)],
                vec![button(true)],
                vec![button(false)],
            ] {
                let mut output = ctx.run_ui(
                    RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        ui.horizontal(|ui| breadcrumb(app, ui, 800.0));
                    },
                );
                output.textures_delta.clear();
            }
            app.apply_folder_request();
            if app.folder != folder_before || app.showing_folder_overview() != overview_before {
                return;
            }
        }
        panic!("no click on the breadcrumb did anything");
    }

    #[test]
    fn the_breadcrumb_leads_back_up() {
        let root = tempfile::tempdir().expect("temporary folder");
        let month = root.path().join("2026_06");
        std::fs::create_dir(&month).expect("create folder");
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);
        app.open_folder(root.path().to_path_buf());
        app.open_folder(month);
        click_along_the_breadcrumb(&mut app, &ctx);
        assert_eq!(app.folder.as_deref(), Some(root.path()));
    }

    #[test]
    fn the_subfolders_button_shows_the_overview_and_goes_back() {
        let root = tempfile::tempdir().expect("temporary folder");
        std::fs::write(root.path().join("a.CR3"), b"not a raw").expect("write photo");
        let month = root.path().join("2026_06");
        std::fs::create_dir(&month).expect("create folder");
        let ctx = egui::Context::default();
        let mut app = PhotonationApp::empty(&ctx);
        app.open_folder(root.path().to_path_buf());
        app.subfolder_found(
            root.path(),
            Subfolder {
                path: month.clone(),
                photo_count: 1,
                cover: month.join("b.CR3"),
            },
        );
        assert!(!app.showing_folder_overview());
        click_along_the_breadcrumb(&mut app, &ctx);
        assert!(app.showing_folder_overview());
        assert_eq!(app.current, None);
        click_along_the_breadcrumb(&mut app, &ctx);
        assert!(!app.showing_folder_overview());
        assert_eq!(app.current, Some(0));
    }

    #[test]
    fn counts_read_as_english() {
        assert_eq!(count_label(1, "photo"), "1 photo");
        assert_eq!(count_label(0, "photo"), "0 photos");
        assert_eq!(count_label(890, "photo"), "890 photos");
    }

    #[test]
    fn long_names_shorten_by_whole_characters() {
        assert_eq!(shortened("2026_06"), "2026_06");
        let long = "Straßburg im Winter und Würzburg";
        let short = shortened(long);
        assert_eq!(short.chars().count(), MAX_CRUMB_CHARS);
        assert!(short.starts_with("Straßburg") && short.ends_with(ELLIPSIS));
    }

    #[test]
    fn the_breadcrumb_drops_folders_from_the_top() {
        let widths = [50.0, 50.0, 50.0];
        let (separator, lead) = (10.0, 20.0);
        let fits = |room| first_crumb_that_fits(&widths, separator, lead, room);
        assert_eq!(fits(170.0), Some(0));
        assert_eq!(fits(169.0), Some(1), "… › b › c");
        assert_eq!(fits(140.0), Some(1));
        assert_eq!(fits(139.0), Some(2), "… › c");
        assert_eq!(fits(80.0), Some(2));
        assert_eq!(fits(79.0), None);
    }

    #[test]
    fn a_cover_fills_its_frame_without_stretching() {
        let close = |rect: Rect, width: f32, height: f32| {
            (rect.width() - width).abs() < 1e-4
                && (rect.height() - height).abs() < 1e-4
                && (rect.center() - pos2(0.5, 0.5)).length() < 1e-4
        };
        assert!(close(filling_uv(1.5, 1.5), 1.0, 1.0));
        assert!(close(filling_uv(3.0, 1.5), 0.5, 1.0), "wide photo");
        assert!(close(filling_uv(0.75, 1.5), 1.0, 0.5), "upright photo");
    }
}
