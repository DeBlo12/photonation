// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Camera colour model (DNG style): white balance and camera → working space.

use photonation_color::{Mat3, Primaries, bradford, temp_tint_to_xy, xy_to_temp_tint, xy_to_xyz};
use photonation_develop::WhiteBalance;
use photonation_image::{CameraColor, SensorImage};

/// The scene-referred working space: linear Rec.2020, D65.
pub const WORKING: Primaries = Primaries::REC2020;

#[derive(Clone, Debug)]
pub struct CameraModel {
    /// Multipliers applied to raw camera RGB (smallest = 1).
    pub wb_multipliers: [f32; 3],
    /// White-balanced camera RGB → working RGB; (1,1,1) maps to (1,1,1).
    pub camera_to_working: Mat3,
    /// The white point in effect, as temperature (K) and tint.
    pub temperature: f64,
    pub tint: f64,
}

fn xyz_to_camera_at(color: &CameraColor, temperature: f64) -> Mat3 {
    match color.matrices.as_slice() {
        [] => Primaries::REC709.from_xyz(),
        [only] => only.xyz_to_camera,
        [lo, .., hi] => {
            // Interpolate linearly in mired between the calibrations.
            let (m, m_lo, m_hi) = (
                1.0 / temperature,
                1.0 / lo.temperature,
                1.0 / hi.temperature,
            );
            let g = ((m - m_hi) / (m_lo - m_hi)).clamp(0.0, 1.0);
            hi.xyz_to_camera.lerp(&lo.xyz_to_camera, g)
        }
    }
}

fn xyz_to_xy(xyz: [f64; 3]) -> [f64; 2] {
    let s = xyz[0] + xyz[1] + xyz[2];
    if s.abs() < 1e-12 {
        photonation_color::D65
    } else {
        [xyz[0] / s, xyz[1] / s]
    }
}

/// Temperature and tint of a camera neutral (DNG "AsShotNeutral" solve).
pub fn neutral_to_temp_tint(color: &CameraColor, neutral: [f64; 3]) -> (f64, f64) {
    let mut temp = 5000.0;
    let mut tint = 0.0;
    for _ in 0..8 {
        let Some(inv) = xyz_to_camera_at(color, temp).inverse() else {
            break;
        };
        (temp, tint) = xy_to_temp_tint(xyz_to_xy(inv.apply(neutral)));
    }
    (temp, tint)
}

impl CameraModel {
    pub fn new(img: &SensorImage, wb: &WhiteBalance) -> CameraModel {
        let color = &img.color;
        let (neutral, temperature, tint) = match *wb {
            WhiteBalance::Custom { temperature, tint } => {
                let white = xy_to_xyz(temp_tint_to_xy(temperature, tint));
                let n = xyz_to_camera_at(color, temperature).apply(white);
                (n.map(|v| v / n[1]), temperature, tint)
            }
            WhiteBalance::AsShot => {
                let n = color.as_shot_neutral.unwrap_or_else(|| {
                    let n =
                        xyz_to_camera_at(color, 6504.0).apply(xy_to_xyz(photonation_color::D65));
                    n.map(|v| v / n[1])
                });
                let (t, ti) = neutral_to_temp_tint(color, n);
                (n, t, ti)
            }
        };
        let neutral = neutral.map(|v| if v.is_finite() && v > 1e-6 { v } else { 1.0 });

        let inv = neutral.map(|v| 1.0 / v);
        let min = inv.iter().copied().fold(f64::INFINITY, f64::min);
        let mult = inv.map(|v| v / min);

        let camera_to_xyz = xyz_to_camera_at(color, temperature)
            .inverse()
            .unwrap_or(Mat3::IDENTITY);
        let white = camera_to_xyz.apply(neutral);
        let white = white.map(|v| v / white[1]);
        let adapt = bradford(white, xy_to_xyz(WORKING.white));
        // Undo the multipliers (camera = wb_camera / mult), go to XYZ, adapt
        // the shot white to D65, into the working space.
        let m = WORKING.from_xyz() * adapt * camera_to_xyz * Mat3::diag(mult.map(|v| 1.0 / v));
        let w = m.apply([1.0; 3]);
        let lum = WORKING.luminance();
        let y = w[0] * lum[0] + w[1] * lum[1] + w[2] * lum[2];
        let camera_to_working = m.scale(1.0 / y);

        CameraModel {
            wb_multipliers: mult.map(|v| v as f32),
            camera_to_working,
            temperature,
            tint,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonation_image::CalibrationMatrix;

    fn color() -> CameraColor {
        // Canon EOS 250D calibrations.
        let d65 = Mat3([
            [0.9079, -0.1923, -0.1236],
            [-0.4677, 1.2454, 0.2492],
            [-0.0922, 0.2319, 0.5565],
        ]);
        let a = Mat3([
            [1.0253, -0.3687, -0.0541],
            [-0.3827, 1.1485, 0.2672],
            [-0.0369, 0.1557, 0.572],
        ]);
        CameraColor {
            as_shot_neutral: Some([1.0 / 1.8418, 1.0, 1.0 / 1.583]),
            matrices: vec![
                CalibrationMatrix {
                    temperature: 2856.0,
                    xyz_to_camera: a,
                },
                CalibrationMatrix {
                    temperature: 6504.0,
                    xyz_to_camera: d65,
                },
            ],
        }
    }

    #[test]
    fn custom_white_balance_round_trips_through_neutral() {
        let c = color();
        for (temp, tint) in [(3200.0, 0.0), (5500.0, 10.0), (7500.0, -15.0)] {
            let white = xy_to_xyz(temp_tint_to_xy(temp, tint));
            let n = xyz_to_camera_at(&c, temp).apply(white);
            let (t, ti) = neutral_to_temp_tint(&c, n.map(|v| v / n[1]));
            assert!((t - temp).abs() / temp < 0.01, "{temp} -> {t}");
            assert!((ti - tint).abs() < 1.0, "{tint} -> {ti}");
        }
    }

    #[test]
    fn as_shot_daylight_is_plausible() {
        let (t, tint) = neutral_to_temp_tint(&color(), color().as_shot_neutral.expect("neutral"));
        assert!((4000.0..7000.0).contains(&t), "{t}");
        assert!(tint.abs() < 40.0, "{tint}");
    }
}
