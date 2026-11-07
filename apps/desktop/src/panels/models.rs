// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The neural networks a tool needs: whether one is installed, and, while
//! none is, what to download. Every detail people might want to weigh (what
//! the model does, who made it, what it learned from, its size, licence and
//! checksum) shows before anything is downloaded, and the download itself
//! only starts with the Download button, in the browser. Photonation ships
//! no weights and never downloads on its own. Used by the Remove panel and
//! the Masks panel; see docs/RETOUCH.md.

use crate::panels::hover_tip;
use crate::panels::masks::hint;
use crate::theme;
use eframe::egui::{self, RichText, Ui};
use photonation_engine::{ModelKind, ModelStatus};
use std::path::Path;

/// A model people can download, as docs/RETOUCH.md checked it.
pub struct ModelDownload {
    /// "LaMa", "RF-DETR Seg Medium".
    pub name: &'static str,
    /// What it does in Photonation.
    pub purpose: &'static str,
    pub made_by: &'static str,
    pub trained_on: &'static str,
    pub file_name: &'static str,
    pub bytes: u64,
    pub licence: &'static str,
    /// What the licence allows, in a few words.
    pub licence_summary: &'static str,
    pub licence_url: &'static str,
    /// The page the file is on.
    pub page_url: &'static str,
    pub download_url: &'static str,
    pub sha256: &'static str,
}

const APACHE_SUMMARY: &str = "Free to use, also commercially; whoever passes the file on keeps \
                              the licence and notices with it.";

pub const INPAINTING: ModelDownload = ModelDownload {
    name: "LaMa",
    purpose: "Fills removals with content that fits the scene (Generative AI).",
    made_by: "Samsung AI Center (LaMa); converted to ONNX by Carve",
    trained_on: "Places2 (photos of scenes)",
    file_name: "lama_fp32.onnx",
    bytes: 208_044_816,
    licence: "Apache-2.0",
    licence_summary: APACHE_SUMMARY,
    licence_url: "https://github.com/advimman/lama/blob/main/LICENSE",
    page_url: "https://huggingface.co/Carve/LaMa-ONNX",
    download_url: "https://huggingface.co/Carve/LaMa-ONNX/resolve/main/lama_fp32.onnx",
    sha256: "1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6",
};

pub const PEOPLE: ModelDownload = ModelDownload {
    name: "RF-DETR Seg Medium",
    purpose: "Finds the people in a photo, so a click selects one.",
    made_by: "Roboflow (RF-DETR); converted to ONNX by besit, weights unchanged",
    trained_on: "COCO (everyday photos from Flickr)",
    file_name: "rfdetr-seg-medium.onnx",
    bytes: 138_376_531,
    licence: "Apache-2.0",
    licence_summary: APACHE_SUMMARY,
    licence_url: "https://huggingface.co/besit/rfdetr-onnx/blob/main/LICENSE",
    page_url: "https://huggingface.co/besit/rfdetr-onnx",
    download_url: "https://huggingface.co/besit/rfdetr-onnx/resolve/main/rfdetr-seg-medium.onnx",
    sha256: "33f97a068ea7de3892e4cbc3825e15c5418be818506d7f39f44e1fb1f7ab6abb",
};

pub fn download(kind: ModelKind) -> &'static ModelDownload {
    match kind {
        ModelKind::Inpainting => &INPAINTING,
        ModelKind::People => &PEOPLE,
    }
}

/// What the model card asked for.
pub enum ModelAction {
    /// Look for the model again (it may have been put in place).
    CheckAgain(ModelKind),
    Failed(String),
}

/// "208 MB".
pub fn size_label(bytes: u64) -> String {
    format!("{} MB", (bytes as f64 / 1_000_000.0).round())
}

/// One line on the model `kind` while it works; while it doesn't, why, and
/// a button that shows what to download. `fallback` says what happens
/// without the model. `expanded` keeps the details open.
pub fn status(
    ui: &mut Ui,
    kind: ModelKind,
    status: Option<&ModelStatus>,
    fallback: &str,
    expanded: &mut bool,
) -> Option<ModelAction> {
    let model = download(kind);
    let problem = match status {
        Some(ModelStatus::Ready { name }) => {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                hint(ui, &format!("Model: {name} ·"));
                licence_link(ui, model);
                hint(ui, "· runs on this computer only");
            });
            return None;
        }
        None | Some(ModelStatus::NotLoaded) => {
            hint(ui, "Loading the model…");
            return None;
        }
        Some(ModelStatus::NotBuilt) => {
            hint(ui, &format!("This build has no AI support. {fallback}"));
            return None;
        }
        Some(ModelStatus::NotInstalled { .. }) => {
            format!("No model installed. {fallback}")
        }
        Some(ModelStatus::Failed { message }) => {
            format!("The model could not be loaded ({message}). {fallback}")
        }
    };
    hint(ui, &problem);
    let expected = match status {
        Some(ModelStatus::NotInstalled { expected }) => expected.as_deref(),
        _ => None,
    };
    if !*expanded {
        let get = ui.button(format!("Get the {} Model…", model.name));
        if hover_tip(
            get,
            &format!(
                "What to download\nShows the model's details, licence and size first. \
                 Nothing downloads until you click Download ({}).",
                size_label(model.bytes)
            ),
        )
        .clicked()
        {
            *expanded = true;
        }
        return None;
    }
    details(ui, kind, expected, expanded)
}

fn licence_link(ui: &mut Ui, model: &ModelDownload) {
    let link = ui.hyperlink_to(
        RichText::new(format!("{} licence", model.licence)).size(11.0),
        model.licence_url,
    );
    hover_tip(
        link,
        &format!("{} licence\n{}", model.licence, model.licence_summary),
    );
}

/// The model's details, then the steps: download, put in place, check.
fn details(
    ui: &mut Ui,
    kind: ModelKind,
    expected: Option<&Path>,
    expanded: &mut bool,
) -> Option<ModelAction> {
    let model = download(kind);
    let mut action = None;
    egui::Frame::NONE
        .fill(theme::palette().control)
        .corner_radius(theme::CONTROL_RADIUS)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(
                RichText::new(format!("{} — what you would download", model.name))
                    .font(theme::title(12.5))
                    .color(theme::palette().text),
            );
            egui::Grid::new(("model details", kind as u8))
                .num_columns(2)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    let row = |ui: &mut Ui, name: &str, value: &str| {
                        field_name(ui, name);
                        field_value(ui, value);
                        ui.end_row();
                    };
                    row(ui, "Does", model.purpose);
                    row(ui, "Made by", model.made_by);
                    row(ui, "Learned from", model.trained_on);
                    row(
                        ui,
                        "File",
                        &format!("{}, {}", model.file_name, size_label(model.bytes)),
                    );
                    field_name(ui, "Licence");
                    ui.vertical(|ui| {
                        licence_link(ui, model);
                        field_value(ui, model.licence_summary);
                    });
                    ui.end_row();
                    field_name(ui, "From");
                    ui.hyperlink_to(RichText::new(model.page_url).size(11.0), model.page_url);
                    ui.end_row();
                    field_name(ui, "SHA-256");
                    ui.horizontal(|ui| {
                        field_value(ui, &format!("{}…", &model.sha256[..16]));
                        let copy = ui.small_button("Copy");
                        if hover_tip(
                            copy,
                            &format!(
                                "Copy the checksum\nCompare it with the downloaded file's: \
                                 shasum -a 256 {0} on a Mac, Get-FileHash {0} in PowerShell.",
                                model.file_name
                            ),
                        )
                        .clicked()
                        {
                            ui.ctx().copy_text(model.sha256.into());
                        }
                    });
                    ui.end_row();
                    row(
                        ui,
                        "Privacy",
                        "Runs on this computer only. Photonation doesn't upload photos or \
                         connect to the internet; your browser downloads the file.",
                    );
                });
            ui.separator();
            let download = ui.add(
                egui::Button::new(
                    RichText::new(format!(
                        "1. Download {} ({})",
                        model.file_name,
                        size_label(model.bytes)
                    ))
                    .color(theme::ON_ACCENT),
                )
                .fill(theme::ACCENT),
            );
            if hover_tip(
                download,
                &format!(
                    "Download in your browser\nFrom {}. Photonation itself downloads nothing.",
                    model.page_url
                ),
            )
            .clicked()
            {
                ui.ctx()
                    .open_url(egui::OpenUrl::new_tab(model.download_url));
            }
            let variable = kind.path_variable();
            let place = match expected {
                Some(_) if std::env::var_os(variable).is_none() => {
                    "2. Move the file into the models folder; it can keep its name.".to_string()
                }
                Some(path) => format!("2. Put it at {} ({variable}).", path.display()),
                None => format!("2. Set {variable} to the model file."),
            };
            hint(ui, &place);
            ui.horizontal_wrapped(|ui| {
                if let Some(folder) = expected.and_then(Path::parent) {
                    let open = ui.button("Open Models Folder");
                    if hover_tip(open, &folder.display().to_string()).clicked()
                        && let Err(error) = open_folder(folder)
                    {
                        action = Some(ModelAction::Failed(format!(
                            "Could not open {}: {error}",
                            folder.display()
                        )));
                    }
                }
                if ui.button("3. Check Again").clicked() {
                    action = Some(ModelAction::CheckAgain(kind));
                }
                if ui.button("Hide").clicked() {
                    *expanded = false;
                }
            });
        });
    action
}

fn field_name(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(11.0)
            .color(theme::palette().text_tertiary),
    );
}

fn field_value(ui: &mut Ui, text: &str) {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(11.0)
                .color(theme::palette().text_secondary),
        )
        .wrap(),
    );
}

/// Shows `folder` in the system's file manager, creating it first.
fn open_folder(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(folder)
        .spawn()
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_downloads_are_the_models_the_docs_checked() {
        let retouch = include_str!("../../../../docs/RETOUCH.md");
        for kind in ModelKind::ALL {
            let model = download(kind);
            assert_eq!(model.sha256.len(), 64);
            assert!(
                retouch.contains(model.sha256),
                "RETOUCH.md lists {}'s SHA-256",
                model.name
            );
            assert!(
                model
                    .download_url
                    .ends_with(&format!("/{}", model.file_name))
            );
            assert!(model.download_url.starts_with(model.page_url));
            assert!(
                retouch.contains(model.page_url),
                "RETOUCH.md links {}",
                model.page_url
            );
        }
    }

    #[test]
    fn sizes_read_in_megabytes() {
        assert_eq!(size_label(INPAINTING.bytes), "208 MB");
        assert_eq!(size_label(PEOPLE.bytes), "138 MB");
    }
}
