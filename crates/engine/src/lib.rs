// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Session services shared by every front end (desktop, CLI, later web):
//! background rendering with latest-wins coalescing, thumbnails, export
//! jobs, removal fills, folder scanning, undo history and histograms. Nothing here knows
//! about a UI toolkit; results arrive as [`Event`]s.

mod decoder;
mod export;
mod histogram;
mod history;
mod library;
mod retouch;
mod service;

pub use decoder::{Decoder, RawDecoder};
pub use export::{
    ExportError, ExportOptions, ExportReport, export_photo, export_photo_with, output_path,
    unused_path,
};
pub use histogram::Histogram;
pub use history::History;
pub use library::scan_folder;
pub use retouch::{MODEL_PATH_VARIABLE, ModelStatus, Retoucher, default_model_path};
pub use service::{Event, PhotoInfo, PreviewKind, PreviewRequest, RenderService, RenderedImage};
