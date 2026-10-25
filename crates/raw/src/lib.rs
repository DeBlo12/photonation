// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Camera raw decoding.
//!
//! The only crate that talks to the decoder library. Everything downstream
//! sees a [`SensorImage`], so the decoder can be swapped (or a second one
//! added for formats rawler lacks) without touching the pipeline.

use photonation_color::Mat3;
use photonation_image::{
    CalibrationMatrix, CameraColor, CameraInfo, CfaColor, CfaPattern, Orientation, Rect,
    SensorImage, SensorLayout,
};
use rawler::decoders::{RawDecodeParams, RawMetadata};
use rawler::imgop::xyz::Illuminant;
use rawler::rawimage::{RawImage, RawPhotometricInterpretation};
use rawler::rawsource::RawSource;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum RawError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Decode { path: PathBuf, message: String },
    #[error("{path}: unsupported image layout ({what})")]
    Unsupported { path: PathBuf, what: String },
}

/// File extensions (lowercase, no dot) the decoder recognizes.
pub fn supported_extensions() -> impl Iterator<Item = String> {
    rawler::decoders::supported_extensions()
        .iter()
        .map(|e| e.to_ascii_lowercase())
}

pub fn is_supported(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    supported_extensions().any(|e| e == ext)
}

/// Decodes a raw file into a sensor mosaic with its colour metadata.
pub fn decode_file(path: &Path) -> Result<SensorImage, RawError> {
    let decode_err = |message: String| RawError::Decode {
        path: path.into(),
        message,
    };
    let source = RawSource::new(path).map_err(|source| RawError::Io {
        path: path.into(),
        source,
    })?;
    let params = RawDecodeParams::default();
    // rawler::decode already isolates decoder panics; metadata goes through
    // the decoder directly, so guard it the same way.
    let raw = rawler::decode(&source, &params).map_err(|e| decode_err(e.to_string()))?;
    let metadata = catch_unwind(AssertUnwindSafe(|| {
        rawler::get_decoder(&source).and_then(|d| d.raw_metadata(&source, &params))
    }))
    .ok()
    .and_then(Result::ok);
    let mut sensor = convert(raw, metadata.as_ref(), path)?;
    sensor.baseline_exposure = dng_baseline_exposure(path).unwrap_or(0.0);
    Ok(sensor)
}

/// DNG `BaselineExposure` (stops), read from the root IFD.
fn dng_baseline_exposure(path: &Path) -> Option<f32> {
    use rawler::formats::tiff::reader::TiffReader;
    let is_dng = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("dng"));
    if !is_dng {
        return None;
    }
    let mut file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let tiff = rawler::formats::tiff::GenericTiffReader::new(&mut file, 0, 0, None, &[]).ok()?;
    let value = tiff
        .root_ifd()
        .get_entry(rawler::tags::DngTag::BaselineExposure)?
        .value
        .force_f32(0);
    value.is_finite().then_some(value)
}

fn convert(
    raw: RawImage,
    meta: Option<&RawMetadata>,
    path: &Path,
) -> Result<SensorImage, RawError> {
    let unsupported = |what: String| RawError::Unsupported {
        path: path.into(),
        what,
    };

    let layout = match (&raw.photometric, raw.cpp) {
        (RawPhotometricInterpretation::Cfa(config), 1) => {
            SensorLayout::Cfa(cfa_pattern(config).ok_or_else(|| {
                unsupported(format!(
                    "{}×{} CFA {}",
                    config.cfa.width, config.cfa.height, config.cfa.name
                ))
            })?)
        }
        (RawPhotometricInterpretation::LinearRaw, 3) => SensorLayout::LinearRgb,
        (photometric, cpp) => {
            return Err(unsupported(format!(
                "{photometric:?} with {cpp} samples per pixel"
            )));
        }
    };

    let (width, height) = (raw.width, raw.height);
    let full = Rect::new(0, 0, width, height);
    let to_rect = |r: &rawler::imgop::Rect| Rect::new(r.p.x, r.p.y, r.d.w, r.d.h);
    let crop = raw
        .crop_area
        .as_ref()
        .or(raw.active_area.as_ref())
        .map(to_rect)
        .filter(|r| crop_fits(*r, width, height))
        .unwrap_or(full);

    let white = raw
        .whitelevel
        .0
        .iter()
        .copied()
        .min()
        .unwrap_or(u16::MAX as u32) as f32;
    // Levels come straight from the file; keep them finite and below white
    // so normalization never divides by zero or produces NaN.
    let black = black_per_channel(&raw, &layout).map(|level| sane_black_level(level, white));
    let color = camera_color(&raw);
    // Some formats (e.g. Canon CR3) only record orientation in EXIF.
    let orientation = meta
        .and_then(|m| m.exif.orientation)
        .map(Orientation::from_exif)
        .unwrap_or_else(|| Orientation::from_exif(raw.orientation.to_u16()));
    let info = camera_info(&raw, meta);

    let (data, black, white) = match raw.data {
        rawler::RawImageData::Integer(samples) => (samples, black, white),
        // Float raws (some DNGs): rescale to the full u16 range.
        rawler::RawImageData::Float(samples) => {
            let scale = u16::MAX as f32 / white.max(f32::EPSILON);
            let data = samples
                .iter()
                .map(|v| (v * scale).clamp(0.0, u16::MAX as f32) as u16)
                .collect();
            (data, black.map(|b| b * scale), u16::MAX as f32)
        }
    };
    if let Err(message) = check_dimensions(width, height, layout.samples_per_pixel(), data.len()) {
        return Err(RawError::Decode {
            path: path.into(),
            message,
        });
    }

    Ok(SensorImage {
        width,
        height,
        layout,
        data,
        black,
        white,
        crop,
        orientation,
        color,
        baseline_exposure: 0.0,
        info,
    })
}

/// Largest sensor accepted, in pixels. Medium-format backs reach ~150 MP
/// (400 MP in pixel-shift modes); anything far beyond is a corrupt header.
const MAX_SENSOR_PIXELS: usize = 1 << 30;

/// Dimensions read from a file are untrusted: products are checked so a
/// crafted header cannot wrap around and pass the sample-count check.
fn check_dimensions(
    width: usize,
    height: usize,
    samples_per_pixel: usize,
    sample_count: usize,
) -> Result<(), String> {
    let pixels = width
        .checked_mul(height)
        .filter(|&pixels| pixels > 0 && pixels <= MAX_SENSOR_PIXELS)
        .ok_or_else(|| format!("implausible sensor size {width}×{height}"))?;
    match pixels.checked_mul(samples_per_pixel) {
        Some(expected) if expected == sample_count => Ok(()),
        _ => Err("sample count does not match dimensions".into()),
    }
}

/// A black level from the file, made finite and kept below `white` (which
/// may itself be bogus, even 0) so normalization never divides by zero.
fn sane_black_level(level: f32, white: f32) -> f32 {
    if level.is_finite() {
        level.clamp(0.0, (white - 1.0).max(0.0))
    } else {
        0.0
    }
}

/// Whether a crop rectangle from the file is non-empty and inside the image.
fn crop_fits(crop: Rect, width: usize, height: usize) -> bool {
    let inside = |start: usize, len: usize, limit: usize| {
        len > 0 && start.checked_add(len).is_some_and(|end| end <= limit)
    };
    inside(crop.x, crop.w, width) && inside(crop.y, crop.h, height)
}

fn cfa_pattern(config: &rawler::rawimage::CFAConfig) -> Option<CfaPattern> {
    let (width, height) = (config.cfa.width, config.cfa.height);
    let mut colors = Vec::with_capacity(width * height);
    for row in 0..height {
        for col in 0..width {
            let plane = config.cfa.color_at(row, col);
            let color = match format!("{:?}", config.colors.colors.get(plane)?).as_str() {
                "RED" => CfaColor::Red,
                "GREEN" => CfaColor::Green,
                "BLUE" => CfaColor::Blue,
                _ => return None,
            };
            colors.push(color);
        }
    }
    CfaPattern::new(width, height, colors)
}

/// rawler reports black levels per CFA position (or per channel for linear
/// data); the pipeline uses one level per colour channel.
fn black_per_channel(raw: &RawImage, layout: &SensorLayout) -> [f32; 3] {
    let levels = raw.blacklevel.as_vec();
    let level = |i: usize| levels.get(i).or(levels.first()).copied().unwrap_or(0.0);
    match layout {
        SensorLayout::LinearRgb => [level(0), level(1), level(2)],
        SensorLayout::Cfa(pattern) => {
            let (bw, bh) = (raw.blacklevel.width.max(1), raw.blacklevel.height.max(1));
            let mut sums = [0.0f32; 3];
            let mut counts = [0usize; 3];
            for row in 0..bh {
                for col in 0..bw {
                    let color = pattern.color_at(row, col) as usize;
                    sums[color] += level(row * bw + col);
                    counts[color] += 1;
                }
            }
            std::array::from_fn(|c| {
                if counts[c] > 0 {
                    sums[c] / counts[c] as f32
                } else {
                    level(0)
                }
            })
        }
    }
}

fn illuminant_temperature(i: Illuminant) -> Option<f64> {
    Some(match i {
        Illuminant::A | Illuminant::Tungsten => 2856.0,
        Illuminant::IsoStudioTungsten => 3200.0,
        Illuminant::WhiteFluorescent => 3450.0,
        Illuminant::CoolWhiteFluorescent | Illuminant::Fluorescent => 4150.0,
        Illuminant::B => 4874.0,
        Illuminant::D50 => 5003.0,
        Illuminant::Daylight | Illuminant::FineWeather | Illuminant::Flash => 5500.0,
        Illuminant::D55 => 5503.0,
        Illuminant::DaylightWhiteFluorescent => 5000.0,
        Illuminant::D65 | Illuminant::CloudyWeather => 6504.0,
        Illuminant::DaylightFluorescent => 6430.0,
        Illuminant::C => 6774.0,
        Illuminant::D75 | Illuminant::Shade => 7504.0,
        Illuminant::Unknown => return None,
    })
}

fn camera_color(raw: &RawImage) -> CameraColor {
    let mut matrices: Vec<CalibrationMatrix> = raw
        .color_matrix
        .iter()
        .filter(|(_, m)| m.len() >= 9)
        .filter_map(|(illuminant, m)| {
            let temperature = illuminant_temperature(*illuminant)?;
            let xyz_to_camera = Mat3([
                [m[0] as f64, m[1] as f64, m[2] as f64],
                [m[3] as f64, m[4] as f64, m[5] as f64],
                [m[6] as f64, m[7] as f64, m[8] as f64],
            ]);
            xyz_to_camera.inverse()?;
            Some(CalibrationMatrix {
                temperature,
                xyz_to_camera,
            })
        })
        .collect();
    matrices.sort_by(|a, b| a.temperature.total_cmp(&b.temperature));
    matrices.dedup_by(|a, b| (a.temperature - b.temperature).abs() < 1.0);
    // The DNG model interpolates between at most two calibrations: keep the
    // extremes.
    if matrices.len() > 2 {
        let last = matrices.pop();
        matrices.truncate(1);
        matrices.extend(last);
    }

    let wb = raw.wb_coeffs;
    let as_shot_neutral = (wb[..3].iter().all(|v| v.is_finite() && *v > 0.0)).then(|| {
        [
            wb[1] as f64 / wb[0] as f64,
            1.0,
            wb[1] as f64 / wb[2] as f64,
        ]
    });
    CameraColor {
        as_shot_neutral,
        matrices,
    }
}

fn camera_info(raw: &RawImage, meta: Option<&RawMetadata>) -> CameraInfo {
    let rational = |r: &Option<rawler::formats::tiff::Rational>| {
        r.as_ref()
            .filter(|r| r.d != 0)
            .map(|r| r.n as f64 / r.d as f64)
    };
    let exif = meta.map(|m| &m.exif);
    CameraInfo {
        make: raw.clean_make.clone(),
        model: raw.clean_model.clone(),
        lens: meta.and_then(|m| m.lens.as_ref()).map(|l| {
            format!("{} {}", l.lens_make, l.lens_model)
                .trim()
                .to_string()
        }),
        iso: exif.and_then(|e| e.iso_speed.or(e.iso_speed_ratings.map(u32::from))),
        exposure_time: exif.and_then(|e| rational(&e.exposure_time)),
        f_number: exif.and_then(|e| rational(&e.fnumber)),
        focal_length: exif.and_then(|e| rational(&e.focal_length)),
        date_time: exif.and_then(|e| e.date_time_original.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_products_cannot_wrap() {
        assert!(check_dimensions(6000, 4000, 1, 24_000_000).is_ok());
        assert!(check_dimensions(10, 10, 3, 300).is_ok());
        assert!(check_dimensions(10, 10, 3, 100).is_err());
        assert!(check_dimensions(0, 4000, 1, 0).is_err());
        // 2^32 × 2^32 wraps to 0 in unchecked 64-bit arithmetic.
        assert!(check_dimensions(1 << 32, 1 << 32, 1, 0).is_err());
        assert!(check_dimensions(usize::MAX, 2, 1, usize::MAX - 1).is_err());
    }

    #[test]
    fn crops_must_fit_without_overflow() {
        assert!(crop_fits(Rect::new(0, 0, 100, 50), 100, 50));
        assert!(crop_fits(Rect::new(10, 5, 80, 40), 100, 50));
        assert!(!crop_fits(Rect::new(10, 5, 91, 40), 100, 50));
        assert!(!crop_fits(Rect::new(0, 0, 0, 50), 100, 50));
        assert!(!crop_fits(Rect::new(usize::MAX, 0, 2, 50), 100, 50));
    }

    #[test]
    fn black_levels_are_finite_and_below_white() {
        assert_eq!(sane_black_level(512.0, 15000.0), 512.0);
        assert_eq!(sane_black_level(f32::NAN, 15000.0), 0.0);
        assert_eq!(sane_black_level(-3.0, 15000.0), 0.0);
        assert_eq!(sane_black_level(20000.0, 15000.0), 14999.0);
        // A zero white level must not make the clamp range inverted (a panic).
        assert_eq!(sane_black_level(512.0, 0.0), 0.0);
    }

    #[test]
    fn extensions_are_matched_case_insensitively() {
        assert!(is_supported(Path::new("/photos/IMG_0001.CR3")));
        assert!(is_supported(Path::new("img_0001.cr3")));
        assert!(!is_supported(Path::new("notes.txt")));
        assert!(!is_supported(Path::new("no-extension")));
    }
}
