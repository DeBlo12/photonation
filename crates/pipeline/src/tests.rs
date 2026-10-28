// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use super::*;
use photonation_image::{CfaColor, CfaPattern, Orientation, SensorLayout};

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

mod removals {
    use super::*;
    use crate::retouch::{RemovalFills, removal_canvas, removal_jobs};
    use photonation_develop::{BrushStroke, Removal, RemovalMethod};
    use photonation_retouch::{ContentAwareFill, Inpainter};
    use std::sync::Arc;

    fn removal_at(center: [f64; 2], radius: f64) -> Removal {
        Removal::new(
            RemovalMethod::ContentAware,
            BrushStroke {
                points: vec![center, [center[0] + 0.05, center[1]]],
                radius,
            },
        )
    }

    /// Fills every active removal of `settings` in order, like the engine.
    fn fill_all(
        img: &SensorImage,
        settings: &Settings,
        fill: impl Fn(&[f32]) -> Vec<f32>,
    ) -> RemovalFills {
        let mut fills = RemovalFills::new();
        for job in removal_jobs(settings) {
            let canvas = removal_canvas(img, settings, job.index, &fills).expect("canvas");
            let patch = canvas.patch(&fill(&canvas.canvas.rgb));
            fills.insert(job.key, Arc::new(patch));
        }
        fills
    }

    fn differing_pixels(a: &OutputImage, b: &OutputImage) -> Vec<(usize, usize)> {
        let (Pixels::U8(a_samples), Pixels::U8(b_samples)) = (&a.pixels, &b.pixels) else {
            panic!("8-bit")
        };
        (0..a.width * a.height)
            .filter(|&pixel| {
                (0..3).any(|c| a_samples[pixel * 3 + c].abs_diff(b_samples[pixel * 3 + c]) > 2)
            })
            .map(|pixel| (pixel % a.width, pixel / a.width))
            .collect()
    }

    #[test]
    fn an_unchanged_canvas_round_trips_to_the_original_photo() {
        let img = synthetic(96, 80, Orientation::Rotate90);
        let settings = Settings {
            exposure: 0.4,
            removals: vec![removal_at([0.4, 0.5], 0.08)],
            ..Settings::default()
        };
        let fills = fill_all(&img, &settings, |rgb| rgb.to_vec());
        assert_eq!(fills.len(), 1);
        let opts = RenderOptions::default();
        let plain = render(&img, &settings, &opts).expect("render");
        let retouched = render_with_fills(&img, &settings, &fills, &opts).expect("render");
        assert!(
            differing_pixels(&plain, &retouched).is_empty(),
            "encode/decode must invert"
        );
    }

    #[test]
    fn a_fill_changes_only_the_brushed_area_and_keeps_band_invariance() {
        let img = synthetic(120, 90, Orientation::Normal);
        let removal = removal_at([0.5, 0.5], 0.06);
        let settings = Settings {
            removals: vec![removal],
            ..Settings::default()
        };
        // A flat mid-grey fill, easy to spot.
        let fills = fill_all(&img, &settings, |rgb| vec![0.5; rgb.len()]);
        let plain = render(&img, &settings, &RenderOptions::default()).expect("render");
        let mut previous: Option<OutputImage> = None;
        for band_rows in [1, 7, 10_000] {
            let opts = RenderOptions {
                band_rows: Some(band_rows),
                ..RenderOptions::default()
            };
            let retouched = render_with_fills(&img, &settings, &fills, &opts).expect("render");
            if let Some(previous) = &previous {
                assert_eq!(previous, &retouched, "band {band_rows}");
            }
            previous = Some(retouched);
        }
        let changed = differing_pixels(&plain, &previous.expect("rendered"));
        assert!(!changed.is_empty(), "the fill shows");
        // Mask space spans the default crop (116 × 86); the brush reaches
        // 0.06 · 116 ≈ 7 px around the stroke from x 0.5 to 0.55.
        let (width, height) = (116.0, 86.0);
        for (x, y) in changed {
            let (u, v) = ((x as f64 + 0.5) / width, (y as f64 + 0.5) / height);
            let along = u.clamp(0.5, 0.55);
            let distance = ((u - along) * width).hypot((v - 0.5) * height);
            assert!(
                distance <= 0.06 * width + 1.5,
                "({x}, {y}) is {distance} px away"
            );
        }
    }

    #[test]
    fn content_aware_fill_runs_end_to_end_and_disabling_restores_the_photo() {
        let img = synthetic(128, 96, Orientation::Normal);
        let mut settings = Settings {
            removals: vec![removal_at([0.3, 0.4], 0.04), removal_at([0.6, 0.6], 0.05)],
            ..Settings::default()
        };
        let fill = ContentAwareFill::default();
        let mut fills = RemovalFills::new();
        for job in removal_jobs(&settings) {
            let canvas = removal_canvas(&img, &settings, job.index, &fills).expect("canvas");
            assert!(canvas.canvas.hole_pixels() > 0);
            let filled = fill.inpaint(&canvas.canvas, 0).expect("fill");
            fills.insert(job.key, Arc::new(canvas.patch(&filled)));
        }
        assert_eq!(fills.len(), 2, "both removals filled");
        let opts = RenderOptions::default();
        let output = render_with_fills(&img, &settings, &fills, &opts).expect("render");
        assert_eq!((output.width, output.height), (124, 92));
        settings
            .removals
            .iter_mut()
            .for_each(|removal| removal.enabled = false);
        assert!(removal_jobs(&settings).is_empty());
        assert_eq!(
            render(&img, &settings, &opts).expect("render"),
            render_with_fills(&img, &settings, &fills, &opts).expect("render"),
            "disabled removals ignore cached fills"
        );
    }
}
