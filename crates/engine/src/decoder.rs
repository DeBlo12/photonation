// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::RenderedImage;
use photonation_image::SensorImage;
use photonation_raw::RawError;
use std::path::Path;

/// Turns a photo path into a decoded sensor image.
///
/// The engine decodes with [`RawDecoder`] unless told otherwise. Tests use
/// synthetic images, and a future web build reading files through the
/// browser's APIs can plug in its own.
pub trait Decoder: Send + Sync {
    fn decode(&self, path: &Path) -> Result<SensorImage, RawError>;

    /// The preview the camera embedded in the file, at most `long_edge`
    /// pixels long and upright; `None` when there is none to read.
    fn embedded_preview(&self, _path: &Path, _long_edge: usize) -> Option<RenderedImage> {
        None
    }

    /// When the photo was taken ("YYYY:MM:DD HH:MM:SS", local time).
    fn capture_time(&self, _path: &Path) -> Option<String> {
        None
    }
}

/// Camera raw files, through `photonation-raw`.
#[derive(Clone, Copy, Debug, Default)]
pub struct RawDecoder;

impl Decoder for RawDecoder {
    fn decode(&self, path: &Path) -> Result<SensorImage, RawError> {
        photonation_raw::decode_file(path)
    }

    fn embedded_preview(&self, path: &Path, long_edge: usize) -> Option<RenderedImage> {
        photonation_raw::embedded_preview(path, long_edge)
            .ok()
            .flatten()
            .map(crate::thumbnails::embedded_image)
    }

    fn capture_time(&self, path: &Path) -> Option<String> {
        photonation_raw::capture_time(path)
    }
}
