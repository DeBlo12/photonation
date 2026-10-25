// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Synthetic sensor images for tests in this and downstream crates
//! (enabled with the `test-util` feature). Raw sample files are
//! copyrighted, so tests render these instead.

use crate::{
    CalibrationMatrix, CameraColor, CameraInfo, CfaPattern, Orientation, Rect, SensorImage,
    SensorLayout,
};
use photonation_color::Mat3;

impl SensorImage {
    /// Deterministic RGGB mosaic: smooth gradients plus pseudo-random
    /// noise, with Canon-like levels and a D65 calibration. The camera
    /// crop leaves a 2-pixel border.
    pub fn test_pattern(width: usize, height: usize, orientation: Orientation) -> SensorImage {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut data = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let noise = (state % 400) as f32;
                let base = 600.0
                    + 9000.0 * (x as f32 / width as f32) * (0.5 + 0.5 * (y as f32 / height as f32));
                data.push((base + noise).min(15000.0) as u16);
            }
        }
        SensorImage {
            width,
            height,
            data,
            layout: SensorLayout::Cfa(CfaPattern::rggb()),
            black: [512.0; 3],
            white: 15000.0,
            crop: Rect::new(2, 2, width - 4, height - 4),
            orientation,
            color: CameraColor {
                as_shot_neutral: Some([0.55, 1.0, 0.63]),
                matrices: vec![CalibrationMatrix {
                    temperature: 6504.0,
                    xyz_to_camera: Mat3([
                        [0.9079, -0.1923, -0.1236],
                        [-0.4677, 1.2454, 0.2492],
                        [-0.0922, 0.2319, 0.5565],
                    ]),
                }],
            },
            baseline_exposure: 0.0,
            info: CameraInfo::default(),
        }
    }
}
