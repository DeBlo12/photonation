// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::{Decoder, RawDecoder};
use photonation_color::ColorSpace;
use photonation_develop::{Settings, sidecar_path};
use photonation_export::Format;
use photonation_image::BitDepth;
use photonation_pipeline::RenderOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct ExportOptions {
    pub format: Format,
    pub bit_depth: BitDepth,
    pub color_space: ColorSpace,
    pub long_edge: Option<usize>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            format: Format::Jpeg { quality: 92 },
            bit_depth: BitDepth::Eight,
            color_space: ColorSpace::Srgb,
            long_edge: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Raw(#[from] photonation_raw::RawError),
    #[error(transparent)]
    Sidecar(#[from] photonation_develop::SidecarError),
    #[error(transparent)]
    Render(#[from] photonation_pipeline::RenderError),
    #[error(transparent)]
    Encode(#[from] photonation_export::ExportError),
}

/// Timings and size of one export.
#[derive(Clone, Copy, Debug)]
pub struct ExportReport {
    pub size: (usize, usize),
    pub decode: Duration,
    pub render: Duration,
    pub encode: Duration,
}

/// `<dir>/<stem>.<ext>`.
pub fn output_path(original: &Path, dir: &Path, format: Format) -> PathBuf {
    let stem = original.file_stem().unwrap_or_default().to_string_lossy();
    dir.join(format!("{stem}.{}", format.extension()))
}

/// `target`, or the first of `<stem>-2.<ext>`, `<stem>-3.<ext>`, … that
/// does not exist yet, so an export never replaces an earlier file (nor a
/// photo with the same name from another camera format).
pub fn unused_path(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }
    let stem = target.file_stem().unwrap_or_default().to_string_lossy();
    let extension = target
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|n| target.with_file_name(format!("{stem}-{n}{extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| target.to_path_buf())
}

/// Decodes, develops and writes one photo. `settings` overrides the
/// sidecar; `adjust` runs on the settings after decoding (CLI overrides).
pub fn export_photo(
    original: &Path,
    target: &Path,
    settings: Option<&Settings>,
    options: &ExportOptions,
    adjust: impl FnOnce(&mut Settings, &photonation_image::SensorImage),
) -> Result<ExportReport, ExportError> {
    export_photo_with(&RawDecoder, original, target, settings, options, adjust)
}

/// [`export_photo`] with the photo decoded by `decoder`.
pub fn export_photo_with(
    decoder: &dyn Decoder,
    original: &Path,
    target: &Path,
    settings: Option<&Settings>,
    options: &ExportOptions,
    adjust: impl FnOnce(&mut Settings, &photonation_image::SensorImage),
) -> Result<ExportReport, ExportError> {
    let started = Instant::now();
    let sensor = decoder.decode(original)?;
    let decode = started.elapsed();

    let mut settings = match settings {
        Some(s) => s.clone(),
        None => Settings::load(&sidecar_path(original))?.unwrap_or_default(),
    };
    adjust(&mut settings, &sensor);

    let started = Instant::now();
    let render_options = RenderOptions {
        long_edge: options.long_edge,
        color_space: options.color_space,
        bit_depth: options.bit_depth,
        ..RenderOptions::default()
    };
    let image = photonation_pipeline::render(&sensor, &settings, &render_options)?;
    let render = started.elapsed();
    drop(sensor);

    let started = Instant::now();
    photonation_export::write(target, &image, options.format)?;
    Ok(ExportReport {
        size: (image.width, image.height),
        decode,
        render,
        encode: started.elapsed(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unused_path_never_returns_an_existing_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let target = dir.path().join("IMG_0001.jpg");
        assert_eq!(unused_path(&target), target);
        std::fs::write(&target, "first").expect("write");
        let second = unused_path(&target);
        assert_eq!(second, dir.path().join("IMG_0001-2.jpg"));
        std::fs::write(&second, "second").expect("write");
        assert_eq!(unused_path(&target), dir.path().join("IMG_0001-3.jpg"));
    }

    #[test]
    fn output_path_uses_the_stem_and_format_extension() {
        assert_eq!(
            output_path(
                Path::new("/photos/IMG_0001.CR3"),
                Path::new("/out"),
                Format::Png
            ),
            PathBuf::from("/out/IMG_0001.png")
        );
    }
}
