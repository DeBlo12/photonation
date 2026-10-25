// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use super::*;
use photonation_color::Mat3;
use photonation_image::{
    CalibrationMatrix, CameraColor, CameraInfo, CfaColor, CfaPattern, Orientation, SensorLayout,
};

/// Deterministic synthetic mosaic: smooth gradients plus pseudo-random noise.
fn synthetic(w: usize, h: usize, orientation: Orientation) -> SensorImage {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut data = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let noise = (state % 400) as f32;
            let base = 600.0 + 9000.0 * (x as f32 / w as f32) * (0.5 + 0.5 * (y as f32 / h as f32));
            data.push((base + noise).min(15000.0) as u16);
        }
    }
    SensorImage {
        width: w,
        height: h,
        data,
        layout: SensorLayout::Cfa(CfaPattern::rggb()),
        black: [512.0; 3],
        white: 15000.0,
        crop: Rect::new(2, 2, w - 4, h - 4),
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

fn render_with(
    img: &SensorImage,
    s: &Settings,
    long_edge: Option<usize>,
    band_rows: usize,
) -> OutputImage {
    let opts = RenderOptions {
        long_edge,
        band_rows: Some(band_rows),
        ..RenderOptions::default()
    };
    render(img, s, &opts).expect("render")
}

#[test]
fn band_size_does_not_change_output() {
    let img = synthetic(97, 71, Orientation::Normal);
    let mut s = Settings {
        highlights: -40.0,
        shadows: 30.0,
        vibrance: 20.0,
        clarity: 50.0,
        texture: 30.0,
        dehaze: 20.0,
        ..Settings::default()
    };
    s.color_mixer.orange.hue = 20.0;
    s.color_grading.shadows.saturation = 30.0;
    for long_edge in [None, Some(60), Some(30)] {
        let whole = render_with(&img, &s, long_edge, 10_000);
        for band_rows in [1, 3, 7, 16] {
            assert_eq!(
                whole,
                render_with(&img, &s, long_edge, band_rows),
                "long_edge {long_edge:?} band {band_rows}"
            );
        }
    }
}

#[test]
fn orientation_rotates_output() {
    let normal = render_with(
        &synthetic(40, 24, Orientation::Normal),
        &Settings::default(),
        None,
        8,
    );
    let rotated = render_with(
        &synthetic(40, 24, Orientation::Rotate90),
        &Settings::default(),
        None,
        8,
    );
    assert_eq!((normal.width, normal.height), (36, 20));
    assert_eq!((rotated.width, rotated.height), (20, 36));
    let (Pixels::U8(a), Pixels::U8(b)) = (&normal.pixels, &rotated.pixels) else {
        panic!("8-bit")
    };
    // Stored (x, y) lands at display (h-1-y, x).
    for y in 0..20 {
        for x in 0..36 {
            let (dx, dy) = (19 - y, x);
            assert_eq!(a[(y * 36 + x) * 3..][..3], b[(dy * 20 + dx) * 3..][..3]);
        }
    }
}

#[test]
fn long_edge_and_crop_set_size() {
    let img = synthetic(204, 104, Orientation::Normal);
    let s = Settings {
        crop: Some(NormRect {
            left: 0.0,
            top: 0.0,
            right: 0.5,
            bottom: 1.0,
        }),
        ..Settings::default()
    };
    assert_eq!(output_size(&img, &s, None), (100, 100));
    let out = render_with(&img, &Settings::default(), Some(50), 64);
    assert_eq!((out.width, out.height), (50, 25));
}

#[test]
fn exposure_brightens() {
    let img = synthetic(64, 48, Orientation::Normal);
    let mean = |o: &OutputImage| match &o.pixels {
        Pixels::U8(p) => p.iter().map(|&v| v as f64).sum::<f64>() / p.len() as f64,
        Pixels::U16(_) => unreachable!(),
    };
    let base = mean(&render_with(&img, &Settings::default(), None, 16));
    let bright = mean(&render_with(
        &img,
        &Settings {
            exposure: 1.0,
            ..Settings::default()
        },
        None,
        16,
    ));
    assert!(bright > base + 10.0, "{base} -> {bright}");
}

#[test]
fn sixteen_bit_matches_eight_bit() {
    let img = synthetic(48, 32, Orientation::Normal);
    let o8 = render(&img, &Settings::default(), &RenderOptions::default()).expect("render");
    let o16 = render(
        &img,
        &Settings::default(),
        &RenderOptions {
            bit_depth: BitDepth::Sixteen,
            ..RenderOptions::default()
        },
    )
    .expect("render");
    let (Pixels::U8(a), Pixels::U16(b)) = (&o8.pixels, &o16.pixels) else {
        panic!("depths")
    };
    for (x, y) in a.iter().zip(b) {
        assert!((*x as i32 - (*y as f64 / 257.0).round() as i32).abs() <= 1);
    }
}

#[test]
fn eyedropper_on_neutral_patch_returns_as_shot() {
    // A flat patch whose raw values equal the as-shot neutral is grey under as-shot WB.
    let mut img = synthetic(32, 32, Orientation::Normal);
    let neutral = img.color.as_shot_neutral.expect("neutral");
    for y in 0..32 {
        for x in 0..32 {
            let color = match &img.layout {
                SensorLayout::Cfa(pattern) => pattern.color_at(y, x) as usize,
                SensorLayout::LinearRgb => unreachable!(),
            };
            img.data[y * 32 + x] = (512.0 + 4000.0 * neutral[color]) as u16;
        }
    }
    let (temp, tint) = white_balance_at(&img, &Settings::default(), [0.5, 0.5]).expect("sample");
    let shot = CameraModel::new(&img, &photonation_develop::WhiteBalance::AsShot);
    assert!(
        (temp - shot.temperature).abs() < 50.0,
        "{temp} vs {}",
        shot.temperature
    );
    assert!((tint - shot.tint).abs() < 2.0);
}

fn xtrans() -> CfaPattern {
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
    CfaPattern::new(6, 6, colors).expect("valid pattern")
}

#[test]
fn xtrans_renders_and_is_band_invariant() {
    let mut img = synthetic(61, 45, Orientation::Normal);
    img.layout = SensorLayout::Cfa(xtrans());
    let whole = render_with(&img, &Settings::default(), None, 10_000);
    assert_eq!(whole, render_with(&img, &Settings::default(), None, 5));
    assert_eq!((whole.width, whole.height), (57, 41));
}

#[test]
fn linear_rgb_renders_neutral_grey_as_grey() {
    let (w, h) = (24, 16);
    let neutral = [0.55, 1.0, 0.63];
    let mut img = synthetic(w, h, Orientation::Normal);
    img.layout = SensorLayout::LinearRgb;
    img.data = (0..w * h)
        .flat_map(|_| neutral.map(|n| (512.0 + 3000.0 * n) as u16))
        .collect();
    let out = render_with(&img, &Settings::default(), None, 4);
    let Pixels::U8(px) = &out.pixels else {
        panic!("8-bit")
    };
    let center = &px[(8 * out.width + 10) * 3..][..3];
    assert!(
        center
            .iter()
            .all(|&v| (v as i32 - center[1] as i32).abs() <= 2),
        "{center:?}"
    );
}
