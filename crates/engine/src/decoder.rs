// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

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
}

/// Camera raw files, through `photonation-raw`.
#[derive(Clone, Copy, Debug, Default)]
pub struct RawDecoder;

impl Decoder for RawDecoder {
    fn decode(&self, path: &Path) -> Result<SensorImage, RawError> {
        photonation_raw::decode_file(path)
    }
}
