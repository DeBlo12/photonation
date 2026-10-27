// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use super::*;
use photonation_image::{CfaColor, CfaPattern, Orientation, Pixels, SensorLayout};

fn synthetic(w: usize, h: usize, orientation: Orientation) -> SensorImage {
    SensorImage::test_pattern(w, h, orientation)
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
    s.point_colors.push(photonation_develop::PointColor {
        hue: 40.0,
        saturation: -30.0,
        ..photonation_develop::PointColor::new(photonation_develop::OklabLch {
            lightness: 0.5,
            chroma: 0.05,
            hue: 60.0,
        })
    });
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

/// A flat 32 × 32 patch: the as-shot neutral scaled by `tint` per channel
/// (all ones: grey under as-shot white balance).
fn flat_patch(tint: [f64; 3]) -> SensorImage {
    let mut img = synthetic(32, 32, Orientation::Normal);
    let neutral = img.color.as_shot_neutral.expect("neutral");
    for y in 0..32 {
        for x in 0..32 {
            let color = match &img.layout {
                SensorLayout::Cfa(pattern) => pattern.color_at(y, x) as usize,
                SensorLayout::LinearRgb => unreachable!(),
            };
            img.data[y * 32 + x] = (512.0 + 4000.0 * neutral[color] * tint[color]) as u16;
        }
    }
    img
}

/// The rendered pixel at the centre (8-bit display values).
fn centre_pixel(img: &SensorImage, s: &Settings, opts: &RenderOptions) -> [u8; 3] {
    let out = render(img, s, opts).expect("render");
    let Pixels::U8(pixels) = &out.pixels else {
        panic!("8-bit")
    };
    let offset = (out.height / 2 * out.width + out.width / 2) * 3;
    [pixels[offset], pixels[offset + 1], pixels[offset + 2]]
}

fn channel_spread(rgb: [u8; 3]) -> u8 {
    rgb.iter().max().unwrap_or(&0) - rgb.iter().min().unwrap_or(&0)
}

#[test]
fn eyedropper_on_neutral_patch_returns_as_shot() {
    // A flat patch whose raw values equal the as-shot neutral is grey under as-shot WB.
    let img = flat_patch([1.0; 3]);
    let (temp, tint) = white_balance_at(&img, &Settings::default(), [0.5, 0.5]).expect("sample");
    let shot = CameraModel::new(&img, &photonation_develop::WhiteBalance::AsShot);
    assert!(
        (temp - shot.temperature).abs() < 50.0,
        "{temp} vs {}",
        shot.temperature
    );
    assert!((tint - shot.tint).abs() < 2.0);
}

#[test]
fn color_picker_sees_grey_as_grey_and_red_as_red() {
    let grey = color_at(&flat_patch([1.0; 3]), &Settings::default(), [0.5, 0.5]).expect("grey");
    assert!(grey.chroma < 0.01, "{grey:?}");
    let red = color_at(
        &flat_patch([1.6, 0.8, 0.7]),
        &Settings::default(),
        [0.5, 0.5],
    )
    .expect("red");
    assert!(red.chroma > 0.05, "{red:?}");
    assert!(red.hue < 60.0 || red.hue > 340.0, "{red:?}");
}

#[test]
fn sampled_point_color_edits_the_sampled_area() {
    let img = flat_patch([1.6, 0.8, 0.7]);
    let target = color_at(&img, &Settings::default(), [0.5, 0.5]).expect("sample");
    let opts = RenderOptions::default();
    assert!(channel_spread(centre_pixel(&img, &Settings::default(), &opts)) > 30);
    let desaturated = Settings {
        point_colors: vec![photonation_develop::PointColor {
            saturation: -100.0,
            ..photonation_develop::PointColor::new(target)
        }],
        ..Settings::default()
    };
    assert!(channel_spread(centre_pixel(&img, &desaturated, &opts)) <= 2);
}

#[test]
fn point_color_overlay_greys_out_what_it_does_not_select() {
    let img = flat_patch([1.6, 0.8, 0.7]);
    let elsewhere = Settings {
        point_colors: vec![photonation_develop::PointColor::new(
            photonation_develop::OklabLch {
                lightness: 0.6,
                chroma: 0.1,
                hue: 240.0,
            },
        )],
        ..Settings::default()
    };
    let overlay = RenderOptions {
        point_color_overlay: Some(0),
        ..RenderOptions::default()
    };
    assert!(channel_spread(centre_pixel(&img, &elsewhere, &overlay)) <= 2);
    // Without the overlay, an untouched point colour changes nothing.
    assert_eq!(
        centre_pixel(&img, &elsewhere, &RenderOptions::default()),
        centre_pixel(&img, &Settings::default(), &RenderOptions::default())
    );
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

mod masks {
    use super::*;
    use photonation_develop::{LinearGradient, Mask, MaskShape, RadialGradient};
    use std::ops::Range;

    fn radial_mask(feather: f32) -> Mask {
        Mask::new(
            "Centre",
            MaskShape::Radial(RadialGradient {
                center: [0.5, 0.5],
                radius: [0.15, 0.15],
                rotation: 0.0,
                feather,
            }),
        )
    }

    /// Mean of the 8-bit samples in display rows `rows`, columns `columns`.
    fn mean(image: &OutputImage, rows: Range<usize>, columns: Range<usize>) -> f64 {
        let Pixels::U8(samples) = &image.pixels else {
            panic!("8-bit")
        };
        let mut sum = 0.0;
        let mut count = 0;
        for y in rows {
            for x in columns.clone() {
                for channel in 0..3 {
                    sum += samples[(y * image.width + x) * 3 + channel] as f64;
                    count += 1;
                }
            }
        }
        sum / count as f64
    }

    #[test]
    fn masks_keep_band_size_invariance() {
        let img = synthetic(97, 71, Orientation::Rotate90);
        let mut sky = Mask::new("Sky", MaskShape::Linear(LinearGradient::default()));
        sky.adjustments.exposure = -0.8;
        sky.adjustments.temperature = -30.0;
        sky.adjustments.saturation = 40.0;
        let mut subject = radial_mask(60.0);
        subject.invert = true;
        subject.adjustments.contrast = 30.0;
        subject.adjustments.shadows = 50.0;
        subject.adjustments.dehaze = 20.0;
        let s = Settings {
            masks: vec![sky, subject],
            ..Settings::default()
        };
        // Full size (MHC) and the half-size path, whose pixels are quads.
        for long_edge in [None, Some(40)] {
            let whole = render_with(&img, &s, long_edge, 10_000);
            for band_rows in [1, 5, 16] {
                assert_eq!(
                    whole,
                    render_with(&img, &s, long_edge, band_rows),
                    "long_edge {long_edge:?} band {band_rows}"
                );
            }
        }
    }

    #[test]
    fn masks_without_effect_change_nothing() {
        let img = synthetic(64, 48, Orientation::Normal);
        let plain = render_with(&img, &Settings::default(), None, 16);
        let mut disabled = radial_mask(50.0);
        disabled.adjustments.exposure = 2.0;
        disabled.enabled = false;
        let mut inverted_neutral = radial_mask(0.0);
        inverted_neutral.invert = true;
        let s = Settings {
            masks: vec![disabled, inverted_neutral, Mask::default()],
            ..Settings::default()
        };
        assert_eq!(plain, render_with(&img, &s, None, 16));
    }

    #[test]
    fn radial_exposure_brightens_only_inside() {
        let img = synthetic(104, 104, Orientation::Normal);
        let plain = render_with(&img, &Settings::default(), None, 16);
        let mut mask = radial_mask(0.0);
        mask.adjustments.exposure = 1.5;
        let masked = render_with(
            &img,
            &Settings {
                masks: vec![mask.clone()],
                ..Settings::default()
            },
            None,
            16,
        );
        let centre = (45..55, 45..55);
        let corner = (0..10, 0..10);
        assert!(
            mean(&masked, centre.0.clone(), centre.1.clone())
                > mean(&plain, centre.0.clone(), centre.1.clone()) + 15.0
        );
        assert_eq!(
            mean(&masked, corner.0.clone(), corner.1.clone()),
            mean(&plain, corner.0.clone(), corner.1.clone())
        );

        mask.invert = true;
        let inverted = render_with(
            &img,
            &Settings {
                masks: vec![mask],
                ..Settings::default()
            },
            None,
            16,
        );
        assert_eq!(
            mean(&inverted, centre.0.clone(), centre.1.clone()),
            mean(&plain, centre.0.clone(), centre.1)
        );
        assert!(
            mean(&inverted, corner.0.clone(), corner.1.clone())
                > mean(&plain, corner.0, corner.1) + 15.0
        );
    }

    #[test]
    fn masks_are_placed_on_the_displayed_photo() {
        // A rotated camera: the gradient across the displayed top must
        // brighten the displayed top rows, whatever the stored layout.
        let img = synthetic(84, 64, Orientation::Rotate90);
        let plain = render_with(&img, &Settings::default(), None, 16);
        let (width, height) = (plain.width, plain.height);
        assert!(height > width, "portrait after rotation");
        let mut top = Mask::new(
            "Top",
            MaskShape::Linear(LinearGradient {
                start: [0.5, 0.2],
                end: [0.5, 0.3],
            }),
        );
        top.adjustments.exposure = 2.0;
        let masked = render_with(
            &img,
            &Settings {
                masks: vec![top],
                ..Settings::default()
            },
            None,
            16,
        );
        let top_rows = 0..height / 6;
        let bottom_rows = height * 2 / 3..height;
        assert!(
            mean(&masked, top_rows.clone(), 0..width) > mean(&plain, top_rows, 0..width) + 20.0
        );
        assert_eq!(
            mean(&masked, bottom_rows.clone(), 0..width),
            mean(&plain, bottom_rows, 0..width)
        );
    }

    #[test]
    fn masks_stay_on_the_content_when_cropping() {
        // The radial mask sits on the photo's centre; cropping to the left
        // half moves that centre to the crop's right edge.
        let img = synthetic(104, 104, Orientation::Normal);
        let mut mask = radial_mask(0.0);
        mask.adjustments.exposure = 1.5;
        let crop = Some(NormRect {
            left: 0.0,
            top: 0.0,
            right: 0.5,
            bottom: 1.0,
        });
        let plain = render_with(
            &img,
            &Settings {
                crop,
                ..Settings::default()
            },
            None,
            16,
        );
        let masked = render_with(
            &img,
            &Settings {
                crop,
                masks: vec![mask],
                ..Settings::default()
            },
            None,
            16,
        );
        let (width, height) = (masked.width, masked.height);
        let right_middle = (height / 2 - 3..height / 2 + 3, width - 6..width);
        let left_middle = (height / 2 - 3..height / 2 + 3, 0..6);
        assert!(
            mean(&masked, right_middle.0.clone(), right_middle.1.clone())
                > mean(&plain, right_middle.0, right_middle.1) + 15.0
        );
        assert_eq!(
            mean(&masked, left_middle.0.clone(), left_middle.1.clone()),
            mean(&plain, left_middle.0, left_middle.1)
        );
    }
}
