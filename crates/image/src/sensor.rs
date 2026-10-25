// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::{Orientation, Rect};
use photonation_color::Mat3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CfaColor {
    Red = 0,
    Green = 1,
    Blue = 2,
}

/// Colour filter array: the repeating pattern of filter colours over the
/// sensor, indexed by `(row % height, col % width)` of the stored data.
/// 2×2 for Bayer sensors, 6×6 for Fujifilm X-Trans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CfaPattern {
    width: usize,
    height: usize,
    colors: Vec<CfaColor>,
}

impl CfaPattern {
    /// `colors` is row-major, `width * height` entries; every colour must occur.
    pub fn new(width: usize, height: usize, colors: Vec<CfaColor>) -> Option<CfaPattern> {
        let complete = [CfaColor::Red, CfaColor::Green, CfaColor::Blue]
            .iter()
            .all(|c| colors.contains(c));
        (width > 0 && height > 0 && colors.len() == width * height && complete).then_some(
            CfaPattern {
                width,
                height,
                colors,
            },
        )
    }

    pub fn rggb() -> CfaPattern {
        CfaPattern {
            width: 2,
            height: 2,
            colors: vec![
                CfaColor::Red,
                CfaColor::Green,
                CfaColor::Green,
                CfaColor::Blue,
            ],
        }
    }

    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    #[inline(always)]
    pub fn color_at(&self, row: usize, col: usize) -> CfaColor {
        self.colors[(row % self.height) * self.width + col % self.width]
    }

    /// The 2×2 table, when this is a Bayer pattern (one red, two green, one blue).
    pub fn bayer(&self) -> Option<[[CfaColor; 2]; 2]> {
        if self.size() != (2, 2) {
            return None;
        }
        let count = |c| self.colors.iter().filter(|&&x| x == c).count();
        (count(CfaColor::Red) == 1 && count(CfaColor::Green) == 2).then(|| {
            [
                [self.colors[0], self.colors[1]],
                [self.colors[2], self.colors[3]],
            ]
        })
    }
}

/// How samples are laid out in [`SensorImage::data`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SensorLayout {
    /// One sample per pixel behind a colour filter array (needs demosaicing).
    Cfa(CfaPattern),
    /// Three interleaved samples per pixel, already demosaiced (linear DNG,
    /// e.g. Apple ProRAW).
    LinearRgb,
}

impl SensorLayout {
    pub fn samples_per_pixel(&self) -> usize {
        match self {
            SensorLayout::Cfa(_) => 1,
            SensorLayout::LinearRgb => 3,
        }
    }
}

/// A colour matrix measured under one calibration illuminant.
#[derive(Clone, Debug, PartialEq)]
pub struct CalibrationMatrix {
    /// Correlated colour temperature of the calibration illuminant, kelvin.
    pub temperature: f64,
    /// XYZ → camera RGB for scenes lit by that illuminant (DNG `ColorMatrix`).
    pub xyz_to_camera: Mat3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CameraColor {
    /// Camera RGB of a neutral under the shot illuminant (green = 1), when
    /// the file records one.
    pub as_shot_neutral: Option<[f64; 3]>,
    /// One or two calibration matrices (DNG dual-illuminant model).
    pub matrices: Vec<CalibrationMatrix>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraInfo {
    pub make: String,
    pub model: String,
    pub lens: Option<String>,
    pub iso: Option<u32>,
    /// Seconds.
    pub exposure_time: Option<f64>,
    pub f_number: Option<f64>,
    /// Millimetres.
    pub focal_length: Option<f64>,
    pub date_time: Option<String>,
}

/// Decoded raw sensor data plus everything needed to develop it.
///
/// Kept as compact `u16` samples: the pipeline normalizes on the fly, band
/// by band, so no full-size float copy of the sensor ever exists.
#[derive(Clone, Debug)]
pub struct SensorImage {
    pub width: usize,
    pub height: usize,
    pub layout: SensorLayout,
    /// Row-major samples, `width * height * layout.samples_per_pixel()`.
    pub data: Vec<u16>,
    /// Black level per colour channel (R, G, B).
    pub black: [f32; 3],
    pub white: f32,
    /// The camera's recommended crop (excludes masked and edge pixels).
    pub crop: Rect,
    pub orientation: Orientation,
    pub color: CameraColor,
    /// Exposure the file asks renderers to add, in stops (DNG
    /// `BaselineExposure`; Apple ProRAW relies on it). 0 for other formats.
    pub baseline_exposure: f32,
    pub info: CameraInfo,
}

impl SensorImage {
    /// Sample `channel` of pixel `(x, y)`; for CFA data the channel is
    /// implied by the pattern and `channel` must be 0.
    #[inline(always)]
    pub fn sample(&self, x: usize, y: usize, channel: usize) -> u16 {
        let samples = self.layout.samples_per_pixel();
        self.data[(y * self.width + x) * samples + channel]
    }

    /// Colour channel and raw value of every sample at pixel `(x, y)`.
    pub fn samples_at(&self, x: usize, y: usize) -> impl Iterator<Item = (usize, u16)> + '_ {
        let channels: &[usize] = match &self.layout {
            SensorLayout::Cfa(_) => &[0],
            SensorLayout::LinearRgb => &[0, 1, 2],
        };
        channels.iter().map(move |&channel| {
            let color = match &self.layout {
                SensorLayout::Cfa(pattern) => pattern.color_at(y, x) as usize,
                SensorLayout::LinearRgb => channel,
            };
            (color, self.sample(x, y, channel))
        })
    }

    pub fn is_valid(&self) -> bool {
        self.data.len() >= self.width * self.height * self.layout.samples_per_pixel()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rggb_is_bayer() {
        assert!(CfaPattern::rggb().bayer().is_some());
    }

    #[test]
    fn xtrans_is_not_bayer_but_valid() {
        use CfaColor::{Blue as B, Green as G, Red as R};
        #[rustfmt::skip]
        let colors = vec![
            G, G, R, G, G, B,
            G, G, B, G, G, R,
            B, R, G, R, B, G,
            G, G, B, G, G, R,
            G, G, R, G, G, B,
            R, B, G, B, R, G,
        ];
        let pattern = CfaPattern::new(6, 6, colors).expect("valid");
        assert!(pattern.bayer().is_none());
        assert_eq!(pattern.color_at(7, 2), CfaColor::Blue);
    }

    #[test]
    fn incomplete_pattern_is_rejected() {
        assert!(CfaPattern::new(1, 1, vec![CfaColor::Green]).is_none());
    }
}
