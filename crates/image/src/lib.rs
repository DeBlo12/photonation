// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Image data types that cross crate boundaries: the decoded sensor mosaic
//! a raw decoder produces, the display-referred image the pipeline renders,
//! and the geometry (rectangles, EXIF orientation) relating the two.

mod geometry;
mod output;
mod sensor;
#[cfg(any(test, feature = "test-util"))]
mod test_util;

pub use geometry::{NormRect, Orientation, Rect};
pub use output::{BitDepth, OutputImage, Pixels};
pub use sensor::{
    CalibrationMatrix, CameraColor, CameraInfo, CfaColor, CfaPattern, SensorImage, SensorLayout,
};
