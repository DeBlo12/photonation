// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The Help popover: what the editor does to a photo and which setting
//! changes what. The text follows the code (`photonation_pipeline`,
//! `photonation_develop::controls` and the panels); update it with them.

use crate::panels::icon_button;
use crate::theme;
use crate::widgets::{Icon, paint_brand_mark};
use eframe::egui::{self, Id, Popup, PopupCloseBehavior, RectAlign, Sense, Ui, text::LayoutJob};

const WIDTH: f32 = 420.0;
const MAX_HEIGHT: f32 = 560.0;
const SCROLL_BAR_ROOM: f32 = 12.0;

/// A heading and its entries: a term in semibold, then what it does.
struct Topic {
    title: &'static str,
    entries: Vec<(&'static str, String)>,
}

fn popover_id() -> Id {
    Id::unique("help-popover")
}

/// Opens the popover, as a click on the Help button does.
pub fn open(ctx: &egui::Context) {
    Popup::open_id(ctx, popover_id());
}

/// The toolbar's Help button; it opens and closes the popover beneath it.
pub fn button(ui: &mut Ui) {
    let id = popover_id();
    let open = Popup::is_id_open(ui.ctx(), id);
    let response = icon_button(
        ui,
        Icon::Help,
        None,
        open,
        "Help\nHow Photonation develops a photo and what each setting changes.",
    );
    Popup::from_toggle_button_response(&response)
        .id(id)
        .align(RectAlign::BOTTOM_END)
        .gap(6.0)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .frame(theme::inspector_frame())
        .width(WIDTH)
        .show(|ui| {
            ui.set_width(WIDTH);
            header(ui);
            ui.add_space(6.0);
            // As tall as the window allows below the toolbar, up to MAX_HEIGHT.
            let height = (ui.ctx().content_rect().height() - theme::TOOLBAR_HEIGHT - 90.0)
                .clamp(160.0, MAX_HEIGHT);
            egui::ScrollArea::vertical()
                .min_scrolled_height(height)
                .max_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for topic in topics() {
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new(topic.title)
                                .font(theme::title(14.0))
                                .color(theme::TEXT),
                        );
                        ui.add_space(2.0);
                        for (term, text) in &topic.entries {
                            entry(ui, term, text);
                        }
                    }
                    ui.add_space(4.0);
                });
        });
}

fn header(ui: &mut Ui) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::hover());
        paint_brand_mark(ui.painter(), rect.center(), 22.0, theme::SIDEBAR);
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("How Photonation works")
                .font(theme::display(17.0))
                .color(theme::TEXT),
        );
    });
}

fn entry(ui: &mut Ui, term: &str, text: &str) {
    let mut job = LayoutJob::default();
    // Clear of the scroll bar, which floats over the content's right edge.
    job.wrap.max_width = ui.available_width() - SCROLL_BAR_ROOM;
    if !term.is_empty() {
        job.append(term, 0.0, format(theme::title(12.5), theme::TEXT));
        job.append(" ", 0.0, format(theme::body(12.5), theme::TEXT));
    }
    job.append(text, 0.0, format(theme::body(12.5), theme::TEXT_SECONDARY));
    ui.label(job);
    ui.add_space(3.0);
}

fn format(font_id: egui::FontId, color: egui::Color32) -> egui::TextFormat {
    egui::TextFormat {
        font_id,
        color,
        ..Default::default()
    }
}

fn topics() -> Vec<Topic> {
    let shift = theme::SHIFT_KEY;
    let alt = theme::ALT_KEY;
    vec![
        Topic {
            title: "Working with photos",
            entries: vec![
                (
                    "Open",
                    format!(
                        "a folder ({}) to see its raw photos in the filmstrip at the bottom. \
                         Point at the strip to enlarge it; ← and → step through the folder.",
                        theme::SHORTCUT_OPEN
                    ),
                ),
                (
                    "Edits are non-destructive.",
                    "The raw file is never changed. Settings are saved beside it \
                     (IMG_0001.CR3 → IMG_0001.CR3.photonation.json) shortly after each \
                     edit. A photo with a Lightroom .xmp and no Photonation file opens \
                     with its Lightroom develop settings and crop."
                        .into(),
                ),
                (
                    "Undo and redo",
                    format!(
                        "({} / {}) step through the history; a whole drag is one step.",
                        theme::SHORTCUT_UNDO,
                        theme::SHORTCUT_REDO
                    ),
                ),
                (
                    "Compare (\\)",
                    "shows the unedited original left of a handle you drag across the \
                     photo."
                        .into(),
                ),
                (
                    "Zoom:",
                    format!(
                        "Fit ({}) shows the whole photo, 1:1 one photo pixel on each screen \
                         pixel. Pinch or {}-scroll zooms around the pointer; scrolling \
                         pans. Zoomed previews render at the resolution the zoom needs.",
                        theme::SHORTCUT_FIT,
                        theme::COMMAND_KEY
                    ),
                ),
            ],
        },
        Topic {
            title: "How a photo is developed",
            entries: vec![
                (
                    "",
                    "Every preview and export runs the same steps in this order, so \
                     what you see is what you export:"
                        .into(),
                ),
                (
                    "1. Decode and demosaic.",
                    "Raw sensor values are scaled by the white balance and \
                     demosaiced (half size when a preview needs no more)."
                        .into(),
                ),
                (
                    "2. Removals",
                    "are blended in here, so every later adjustment applies to the \
                     fill too."
                        .into(),
                ),
                (
                    "3. Camera colour.",
                    "The camera's colour matrix, adapted to the chosen white point, \
                     converts to a wide working space (Rec. 2020), with Exposure \
                     applied."
                        .into(),
                ),
                (
                    "4. Light:",
                    "Dehaze, then Highlights and Shadows, then one tone curve made of \
                     Blacks, Whites, Contrast and the Curves card."
                        .into(),
                ),
                (
                    "5. Colour",
                    "in Oklab: Color Mixer, Color Grading, Vibrance and Saturation, \
                     Point Colors. Masks add their local amounts within steps 3–5."
                        .into(),
                ),
                (
                    "6. Detail and geometry:",
                    "Texture and Clarity, then straighten and crop, resizing to the \
                     output size, the output colour space, and finally Sharpening."
                        .into(),
                ),
            ],
        },
        Topic {
            title: "Adjustment cards",
            entries: vec![
                (
                    "Cards",
                    "fold open with a click on their title. The switch turns a card \
                     off without losing its values; the amber dot and the reset arrow \
                     appear once a card differs from its defaults."
                        .into(),
                ),
                (
                    "Sliders:",
                    format!(
                        "drag anywhere on the row, hold {alt} for fine control, \
                         double-click to reset. The amber fill runs from the default \
                         value, marked by a tick."
                    ),
                ),
            ],
        },
        Topic {
            title: "White Balance",
            entries: vec![
                (
                    "As Shot",
                    "uses the neutral the camera recorded. The presets and the \
                     Temperature (K) and Tint sliders set a custom white point; \
                     the camera matrix is interpolated for it."
                        .into(),
                ),
                (
                    "Eyedropper:",
                    "click something that should be grey; Photonation averages a small \
                     patch there and makes it neutral."
                        .into(),
                ),
            ],
        },
        Topic {
            title: "Light",
            entries: vec![
                (
                    "Exposure",
                    "(±5 stops) multiplies the scene light before any tone step, like \
                     a longer or shorter exposure. It adds to the file's own baseline \
                     exposure (DNG)."
                        .into(),
                ),
                (
                    "Highlights and Shadows",
                    "shift exposure by how far each pixel lies above or below mid-grey, \
                     so bright or dark areas move without the rest."
                        .into(),
                ),
                (
                    "Whites and Blacks",
                    "move the white point (where the soft highlight shoulder reaches \
                     white) and the black point of the tone curve."
                        .into(),
                ),
                (
                    "Contrast",
                    "bends an S-curve around mid-grey; mid-grey itself stays put.".into(),
                ),
            ],
        },
        Topic {
            title: "Presence",
            entries: vec![
                (
                    "Texture",
                    "boosts or softens fine detail (a small blur radius).".into(),
                ),
                (
                    "Clarity",
                    "does the same at a large radius, weighted to the midtones: local \
                     contrast and punch. Both scale with the photo, so a preview and an \
                     export match."
                        .into(),
                ),
                (
                    "Dehaze",
                    "removes a neutral veil (negative values add one) and restores some \
                     of the colour haze washes out."
                        .into(),
                ),
            ],
        },
        Topic {
            title: "Color",
            entries: vec![
                (
                    "Vibrance",
                    "raises the saturation of muted colours more than of strong ones.".into(),
                ),
                (
                    "Saturation",
                    "scales every colour's chroma equally. Colours that leave the output \
                     space are pulled toward their brightness, keeping their hue."
                        .into(),
                ),
            ],
        },
        Topic {
            title: "Curves",
            entries: vec![
                (
                    "Parametric",
                    "raises or lowers four tonal regions (Shadows, Darks, Lights, \
                     Highlights) whose borders you can move."
                        .into(),
                ),
                (
                    "RGB, Red, Green, Blue:",
                    "point curves; click to add a point, drag to shape. They apply \
                     after the Light card's tone curve, combined into one curve per \
                     channel."
                        .into(),
                ),
            ],
        },
        Topic {
            title: "Color Mixer",
            entries: vec![
                (
                    "Mixer:",
                    "Hue, Saturation and Luminance for eight colour bands. A pixel \
                     takes a blend of its two nearest bands, so edits stay smooth; \
                     Luminance affects coloured pixels only. The eyedropper picks the \
                     band of a colour in the photo."
                        .into(),
                ),
                (
                    "Point Color:",
                    "pick a colour in the photo, then shift its hue, saturation and \
                     lightness. The range sliders set how far in hue, chroma and \
                     lightness the selection reaches; Visualize range greys out \
                     everything it does not select."
                        .into(),
                ),
            ],
        },
        Topic {
            title: "Color Grading",
            entries: vec![
                (
                    "Wheels",
                    "tint the shadows, midtones, highlights or the whole photo \
                     (hue, saturation, luminance)."
                        .into(),
                ),
                (
                    "Balance",
                    "moves the border between shadows and highlights; Blending sets how \
                     softly the three ranges overlap."
                        .into(),
                ),
            ],
        },
        Topic {
            title: "Detail",
            entries: vec![(
                "Sharpening",
                "is an unsharp mask on brightness only (no colour fringes), applied \
                 last, at the output size. Radius sets the size of the detail it \
                 enhances."
                    .into(),
            )],
        },
        Topic {
            title: "Masks (M)",
            entries: vec![
                (
                    "Shapes:",
                    format!(
                        "+ Linear and + Radial add a gradient; drag its handles on the \
                         photo ({shift} keeps a circle round). Further shapes Add, \
                         Subtract or Intersect, and Invert flips the mask."
                    ),
                ),
                (
                    "Local adjustments",
                    "(Temperature, Tint, Exposure, Contrast, Highlights, Shadows, \
                     Dehaze, Saturation) apply where the mask selects, scaled by its \
                     Amount and its group's. They add to the global sliders."
                        .into(),
                ),
                (
                    "Layers:",
                    "drag rows to reorder, right-click for more; groups switch, fold \
                     and scale several masks together. The red tint shows the selected \
                     mask."
                        .into(),
                ),
            ],
        },
        Topic {
            title: "Crop and straighten (R)",
            entries: vec![(
                "Frame:",
                "drag inside to move, a corner or edge to resize, outside to turn. \
                 Aspect keeps a ratio; Angle straightens and shrinks the frame to stay \
                 on the photo. Return or Esc closes the tool."
                    .into(),
            )],
        },
        Topic {
            title: "Remove objects (Q)",
            entries: vec![
                (
                    "Paint",
                    format!(
                        "over what should go; a click dabs, {shift} adds to the \
                         selected removal. The brush size is a share of the long edge."
                    ),
                ),
                (
                    "Fill:",
                    "Generative AI (a neural inpainting model you install) or \
                     Content-aware (patches from the rest of the photo; Try Another \
                     Fill picks again). Fills compute in the background; Feather softens \
                     the edge."
                        .into(),
                ),
            ],
        },
        Topic {
            title: "Export",
            entries: vec![
                (
                    "Format",
                    "JPEG (with Quality), PNG or TIFF; Size is the original or a long \
                     edge; Color Space is sRGB, Display P3, Adobe RGB or ProPhoto RGB, \
                     embedded as an ICC profile."
                        .into(),
                ),
                (
                    "File name:",
                    "{name} is the original name and {n} the number in a batch.".into(),
                ),
                (
                    "Metadata:",
                    "camera and capture details, creator and copyright (blank fields use \
                     the raw file's) and, only if ticked, the GPS position. The choice is \
                     remembered."
                        .into(),
                ),
            ],
        },
        Topic {
            title: "Settings",
            entries: vec![
                (
                    "Graphics processor",
                    "draws the interface, previews and mask overlay; choose one in \
                     Settings and it is used from the next start. Developing and \
                     exporting run on the CPU."
                        .into(),
                ),
                (
                    "Export folder:",
                    "where the export's folder dialog starts, or, if you choose, where \
                     exports go without asking."
                        .into(),
                ),
                (
                    "Export defaults:",
                    "the format, size, colour space, file name and metadata every new \
                     export starts with."
                        .into(),
                ),
            ],
        },
    ]
}
