// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The GPU path against the CPU pipeline, the reference. Renders need a
//! graphics adapter (software ones count here); without one they are
//! skipped with a note. The shaders are validated everywhere.

use super::*;
use photonation_color::ColorSpace;
use photonation_develop::{
    CombineMode, GradingWheel, LinearGradient, LocalAdjustments, Mask, MaskComponent, MaskGroup,
    MaskShape, OklabLch, PointColor, PointCurve, RadialGradient,
};
use photonation_image::{CfaColor, CfaPattern, Orientation, Rect};
use std::sync::{Mutex, OnceLock};

/// Largest difference of any 8-bit sample: transcendental functions and
/// fused multiply-adds round differently on the GPU, which can tip a
/// value over a quantization step.
const MAX_SAMPLE_DIFFERENCE: u8 = 2;
/// Mean difference over all samples.
const MAX_MEAN_DIFFERENCE: f64 = 0.01;

#[test]
fn shaders_validate() {
    for (name, source) in [
        ("develop", DEVELOP_SHADER),
        ("resize", RESIZE_SHADER),
        ("finish", FINISH_SHADER),
        ("local_contrast", LOCAL_CONTRAST_SHADER),
    ] {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{name}.wgsl: {}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}.wgsl: {e:?}"));
    }
}

/// One renderer for every test (creating a device is slow); `None`
/// without an adapter.
fn renderer() -> Option<std::sync::MutexGuard<'static, GpuRenderer>> {
    static RENDERER: OnceLock<Option<Mutex<GpuRenderer>>> = OnceLock::new();
    let renderer = RENDERER.get_or_init(|| match GpuRenderer::create(true) {
        Ok(renderer) => {
            eprintln!("GPU tests run on {}", renderer.adapter());
            Some(Mutex::new(renderer))
        }
        Err(error) => {
            eprintln!("skipping GPU renders: {error}");
            None
        }
    });
    let renderer = renderer.as_ref()?;
    Some(
        renderer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

/// A mosaic whose channels vary independently, so the photo has real
/// colours (the shared test pattern is nearly neutral).
fn colorful(width: usize, height: usize, layout: SensorLayout) -> SensorImage {
    let mut sensor = SensorImage::test_pattern(width, height, Orientation::Normal);
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % 300) as f32
    };
    let value = |color: usize, x: usize, y: usize| {
        let (u, v) = (x as f32 / width as f32, y as f32 / height as f32);
        let level = match color {
            0 => 0.5 + 0.5 * (7.0 * u).sin() * (3.0 * v).cos(),
            1 => (u * v * 1.6).min(1.0),
            _ => 0.5 + 0.5 * (5.0 * (u + v)).cos(),
        };
        // Deep shadows up to clipped highlights.
        512.0 + level * level * 16_000.0
    };
    sensor.data = match &layout {
        SensorLayout::Cfa(pattern) => (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| {
                let color = pattern.color_at(y, x) as usize;
                (value(color, x, y) + noise()).min(16_383.0) as u16
            })
            .collect(),
        SensorLayout::LinearRgb => (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .flat_map(|(x, y)| [0, 1, 2].map(|color| (color, x, y)))
            .map(|(color, x, y)| (value(color, x, y) + noise()).min(16_383.0) as u16)
            .collect(),
    };
    sensor.white = 15_000.0;
    sensor.layout = layout;
    sensor
}

fn bayer(width: usize, height: usize) -> SensorImage {
    colorful(width, height, SensorLayout::Cfa(CfaPattern::rggb()))
}

fn xtrans(width: usize, height: usize) -> SensorImage {
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
    let pattern = CfaPattern::new(6, 6, colors).expect("X-Trans pattern");
    colorful(width, height, SensorLayout::Cfa(pattern))
}

/// Settings that switch on every step the GPU path covers.
fn busy_settings() -> Settings {
    let mut s = Settings {
        exposure: 0.4,
        contrast: 25.0,
        highlights: -60.0,
        shadows: 45.0,
        whites: 15.0,
        blacks: -20.0,
        dehaze: 25.0,
        vibrance: 30.0,
        saturation: -10.0,
        ..Settings::default()
    };
    s.tone_curve.master = PointCurve(vec![[0.0, 0.05], [0.4, 0.35], [1.0, 1.0]]);
    s.tone_curve.blue = PointCurve(vec![[0.0, 0.0], [0.5, 0.56], [1.0, 1.0]]);
    s.color_mixer.orange.hue = 40.0;
    s.color_mixer.blue.saturation = -50.0;
    s.color_mixer.green.luminance = 35.0;
    s.color_grading.shadows = GradingWheel {
        hue: 220.0,
        saturation: 40.0,
        luminance: -10.0,
    };
    s.color_grading.highlights = GradingWheel {
        hue: 50.0,
        saturation: 30.0,
        luminance: 5.0,
    };
    s.color_grading.balance = 20.0;
    s.point_colors.push(PointColor {
        hue: 40.0,
        saturation: -30.0,
        luminance: 20.0,
        ..PointColor::new(OklabLch {
            lightness: 0.6,
            chroma: 0.08,
            hue: 60.0,
        })
    });
    s
}

fn samples(image: &OutputImage) -> &[u8] {
    match &image.pixels {
        Pixels::U8(samples) => samples,
        Pixels::U16(_) => panic!("8-bit output"),
    }
}

/// Renders on both paths and checks that they agree.
fn assert_matches_cpu(sensor: SensorImage, settings: &Settings, opts: &RenderOptions) {
    let Some(mut gpu) = renderer() else { return };
    let sensor = Arc::new(sensor);
    let cpu = photonation_pipeline::render(&sensor, settings, opts).expect("CPU render");
    let ours = gpu
        .render(&sensor, settings, &RemovalFills::new(), opts)
        .expect("GPU render");
    assert_eq!((ours.width, ours.height), (cpu.width, cpu.height));
    assert_eq!(ours.color_space, cpu.color_space);
    let (expected, got) = (samples(&cpu), samples(&ours));
    assert_eq!(got.len(), expected.len());
    let mut worst = (0u8, 0usize);
    let mut total = 0u64;
    for (index, (a, b)) in expected.iter().zip(got).enumerate() {
        let difference = a.abs_diff(*b);
        total += u64::from(difference);
        if difference > worst.0 {
            worst = (difference, index);
        }
    }
    let mean = total as f64 / expected.len() as f64;
    let (difference, index) = worst;
    let pixel = index / 3;
    assert!(
        difference <= MAX_SAMPLE_DIFFERENCE && mean <= MAX_MEAN_DIFFERENCE,
        "GPU differs by up to {difference} (CPU {}, GPU {} at pixel ({}, {}) channel {}), \
         mean {mean:.4}",
        expected[index],
        got[index],
        pixel % cpu.width,
        pixel / cpu.width,
        index % 3,
    );
}

fn full_size() -> RenderOptions {
    RenderOptions::default()
}

fn preview(long_edge: usize) -> RenderOptions {
    RenderOptions {
        long_edge: Some(long_edge),
        ..RenderOptions::default()
    }
}

#[test]
fn default_settings_match_at_full_size() {
    assert_matches_cpu(bayer(120, 84), &Settings::default(), &full_size());
}

#[test]
fn half_size_previews_match() {
    // Half size, then Lanczos down to the preview.
    assert_matches_cpu(bayer(240, 160), &Settings::default(), &preview(70));
}

#[test]
fn every_global_adjustment_matches() {
    for opts in [full_size(), preview(80)] {
        assert_matches_cpu(bayer(160, 110), &busy_settings(), &opts);
    }
}

#[test]
fn white_balance_and_crops_match() {
    let s = Settings {
        white_balance: photonation_develop::WhiteBalance::Custom {
            temperature: 3800.0,
            tint: 12.0,
        },
        crop: Some(photonation_image::NormRect {
            left: 0.13,
            top: 0.21,
            right: 0.77,
            bottom: 0.9,
        }),
        ..busy_settings()
    };
    for opts in [full_size(), preview(50)] {
        assert_matches_cpu(bayer(150, 100), &s, &opts);
    }
}

#[test]
fn every_orientation_matches() {
    for exif in 1..=8 {
        let mut sensor = bayer(90, 60);
        sensor.orientation = Orientation::from_exif(exif);
        assert_matches_cpu(sensor, &busy_settings(), &preview(40));
    }
}

#[test]
fn xtrans_matches() {
    assert_matches_cpu(xtrans(96, 72), &busy_settings(), &full_size());
}

#[test]
fn linear_dng_matches() {
    let mut sensor = colorful(80, 60, SensorLayout::LinearRgb);
    sensor.crop = Rect::new(0, 0, 80, 60);
    sensor.baseline_exposure = 0.5;
    assert_matches_cpu(sensor, &busy_settings(), &full_size());
}

#[test]
fn output_spaces_and_unsharpened_renders_match() {
    for color_space in [ColorSpace::DisplayP3, ColorSpace::ProphotoRgb] {
        let opts = RenderOptions {
            color_space,
            sharpen: false,
            ..preview(60)
        };
        assert_matches_cpu(bayer(130, 90), &busy_settings(), &opts);
    }
}

#[test]
fn point_color_overlay_matches() {
    let opts = RenderOptions {
        point_color_overlay: Some(0),
        ..preview(60)
    };
    assert_matches_cpu(bayer(130, 90), &busy_settings(), &opts);
}

#[test]
fn new_photos_replace_the_uploaded_mosaic() {
    // Same size, different content: a stale upload would show.
    assert_matches_cpu(bayer(100, 70), &Settings::default(), &full_size());
    let mut other = bayer(100, 70);
    other.data.iter_mut().for_each(|v| *v = 16_383 - *v);
    assert_matches_cpu(other, &Settings::default(), &full_size());
}

#[test]
fn unsupported_renders_say_so() {
    let Some(mut gpu) = renderer() else { return };
    let sensor = Arc::new(bayer(64, 48));
    let fills = RemovalFills::new();
    let sixteen = RenderOptions {
        bit_depth: BitDepth::Sixteen,
        ..RenderOptions::default()
    };
    let error = gpu
        .render(&sensor, &Settings::default(), &fills, &sixteen)
        .expect_err("16-bit");
    assert!(error.is_unsupported(), "{error}");
    let straightened = Settings {
        crop_angle: 3.0,
        ..Settings::default()
    };
    let error = gpu
        .render(&sensor, &straightened, &fills, &full_size())
        .expect_err("straightened");
    assert!(error.is_unsupported(), "{error}");
}

#[test]
fn texture_and_clarity_match() {
    for (clarity, texture) in [(60.0, 0.0), (0.0, 70.0), (-50.0, 40.0), (100.0, -100.0)] {
        let settings = Settings {
            clarity,
            texture,
            ..busy_settings()
        };
        // Large enough that the radii exceed one pixel; previews blur at
        // half size.
        for opts in [full_size(), preview(150)] {
            assert_matches_cpu(bayer(640, 420), &settings, &opts);
        }
    }
}

#[test]
fn texture_and_clarity_with_masks_and_orientation_match() {
    let settings = Settings {
        clarity: 45.0,
        texture: 25.0,
        ..masked_settings()
    };
    for exif in [1, 6] {
        let mut sensor = bayer(500, 330);
        sensor.orientation = Orientation::from_exif(exif);
        assert_matches_cpu(sensor, &settings, &preview(200));
    }
}

fn radial(center: [f64; 2], radius: [f64; 2], rotation: f32, feather: f32) -> MaskShape {
    MaskShape::Radial(RadialGradient {
        center,
        radius,
        rotation,
        feather,
    })
}

fn linear(start: [f64; 2], end: [f64; 2]) -> MaskShape {
    MaskShape::Linear(LinearGradient { start, end })
}

/// Masks of every kind, combination and local adjustment, overlapping.
fn masked_settings() -> Settings {
    let mut settings = busy_settings();
    let mut sky = Mask::new("Sky", linear([0.5, 0.1], [0.45, 0.55]));
    sky.adjustments = LocalAdjustments {
        exposure: -0.8,
        temperature: -30.0,
        highlights: -40.0,
        dehaze: 35.0,
        saturation: 25.0,
        ..LocalAdjustments::default()
    };
    let mut ring = Mask::new("Ring", radial([0.45, 0.55], [0.35, 0.25], 30.0, 60.0));
    ring.components.push(MaskComponent::new(
        radial([0.45, 0.55], [0.12, 0.12], 0.0, 0.0),
        CombineMode::Subtract,
    ));
    ring.adjustments = LocalAdjustments {
        exposure: 0.6,
        tint: 40.0,
        contrast: 50.0,
        shadows: 60.0,
        ..LocalAdjustments::default()
    };
    ring.amount = 70.0;
    let mut lens = Mask::new("Lens", radial([0.3, 0.4], [0.3, 0.3], 0.0, 40.0));
    lens.components.push(MaskComponent::new(
        linear([0.2, 0.2], [0.6, 0.7]),
        CombineMode::Intersect,
    ));
    lens.components.push(MaskComponent::new(
        radial([0.8, 0.8], [0.1, 0.2], 10.0, 0.0),
        CombineMode::Add,
    ));
    lens.invert = true;
    lens.group = Some(0);
    lens.adjustments = LocalAdjustments {
        temperature: 50.0,
        contrast: -40.0,
        dehaze: -30.0,
        saturation: -60.0,
        ..LocalAdjustments::default()
    };
    let mut group = MaskGroup::new("Background");
    group.amount = 60.0;
    settings.masks = vec![sky, ring, lens];
    settings.mask_groups = vec![group];
    settings
}

#[test]
fn masks_match() {
    for opts in [full_size(), preview(70)] {
        assert_matches_cpu(bayer(160, 110), &masked_settings(), &opts);
    }
    // The masks do change the photo, so the match means something.
    let sensor = bayer(160, 110);
    let render = |settings| photonation_pipeline::render(&sensor, settings, &full_size());
    let (masked, plain) = (render(&masked_settings()), render(&busy_settings()));
    assert_ne!(masked.expect("masked").pixels, plain.expect("plain").pixels);
}

#[test]
fn masks_follow_orientation_and_crop() {
    let settings = Settings {
        crop: Some(photonation_image::NormRect {
            left: 0.1,
            top: 0.05,
            right: 0.85,
            bottom: 0.8,
        }),
        ..masked_settings()
    };
    for exif in [1, 3, 6, 8] {
        let mut sensor = bayer(120, 80);
        sensor.orientation = Orientation::from_exif(exif);
        assert_matches_cpu(sensor, &settings, &preview(50));
    }
}

#[test]
fn masks_alone_match() {
    // Masks without global colour work: the local saturation round trip
    // is the only Oklab step.
    let settings = Settings {
        masks: masked_settings().masks,
        ..Settings::default()
    };
    assert_matches_cpu(xtrans(96, 72), &settings, &full_size());
}

#[test]
fn piled_up_masks_match() {
    // Overlapping masks past the caps on summed amounts.
    let mut settings = Settings::default();
    let mut strong = Mask::new("Strong", radial([0.5, 0.5], [0.4, 0.4], 0.0, 80.0));
    strong.adjustments = LocalAdjustments {
        exposure: 4.0,
        highlights: 100.0,
        shadows: -100.0,
        dehaze: 100.0,
        ..LocalAdjustments::default()
    };
    settings.masks = vec![strong; 8];
    assert_matches_cpu(bayer(100, 70), &settings, &full_size());
}

/// Wall-clock times at a real photo's size, both paths (run with
/// `cargo test --release -p photonation-gpu -- --ignored --nocapture`).
#[test]
#[ignore = "timing, not a check"]
fn timings_at_24_megapixels() {
    let Some(mut gpu) = renderer() else { return };
    let sensor = Arc::new(bayer(6024, 4020));
    let fills = RemovalFills::new();
    let time = |label: &str, render: &mut dyn FnMut() -> OutputImage| {
        render(); // warm up: uploads, caches
        let start = std::time::Instant::now();
        let runs = 5;
        for _ in 0..runs {
            render();
        }
        eprintln!(
            "{label}: {:.1} ms",
            start.elapsed().as_secs_f64() * 1e3 / runs as f64
        );
    };
    let clarity = Settings {
        clarity: 40.0,
        texture: 30.0,
        ..masked_settings()
    };
    for (edits, settings) in [
        ("global", busy_settings()),
        ("3 masks", masked_settings()),
        ("3 masks, Texture, Clarity", clarity),
    ] {
        for (size, opts) in [("full size", full_size()), ("1600 px", preview(1600))] {
            time(&format!("CPU {size}, {edits}"), &mut || {
                photonation_pipeline::render(&sensor, &settings, &opts).expect("CPU")
            });
            time(&format!("GPU {size}, {edits}"), &mut || {
                gpu.render(&sensor, &settings, &fills, &opts).expect("GPU")
            });
        }
    }
}
