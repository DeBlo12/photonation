// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::{Decoder, RawDecoder};
use photonation_color::ColorSpace;
use photonation_develop::{Settings, load_photo_settings};
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
    /// Output file name without extension, see [`export_file_stem`]:
    /// `{name}` is the original's name and `{n}` the photo's number in
    /// the batch.
    pub file_name: String,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            format: Format::Jpeg { quality: 92 },
            bit_depth: BitDepth::Eight,
            color_space: ColorSpace::Srgb,
            long_edge: None,
            file_name: NAME_TOKEN.into(),
        }
    }
}

/// Replaced by the original's file name without extension.
pub const NAME_TOKEN: &str = "{name}";
/// Replaced by the photo's number in the batch, from 1, zero-padded to the
/// batch size's width.
pub const NUMBER_TOKEN: &str = "{n}";
/// Longest file stem written, in characters.
const MAX_STEM_CHARS: usize = 200;

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

/// The file stem for photo `number` (from 1) of `total` exported with
/// `template`. Tokens are replaced (see [`NAME_TOKEN`], [`NUMBER_TOKEN`]);
/// a batch whose template names every photo alike gets `-{n}` appended.
/// The result is a single plain file name: path separators and control
/// characters become `-`, leading dots and surrounding spaces are
/// dropped, and an empty name falls back to the original's.
pub fn export_file_stem(template: &str, original: &Path, number: usize, total: usize) -> String {
    let original_stem = original.file_stem().unwrap_or_default().to_string_lossy();
    let width = total.max(1).to_string().len();
    let mut template = template.to_string();
    if total > 1 && !template.contains(NAME_TOKEN) && !template.contains(NUMBER_TOKEN) {
        template.push('-');
        template.push_str(NUMBER_TOKEN);
    }
    let named = template
        .replace(NAME_TOKEN, &original_stem)
        .replace(NUMBER_TOKEN, &format!("{number:0width$}"));
    let cleaned: String = named
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .take(MAX_STEM_CHARS)
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    if cleaned.is_empty() {
        original_stem.into_owned()
    } else {
        cleaned.to_string()
    }
}

/// `<dir>/<file stem>.<ext>` for photo `number` (from 1) of `total`.
pub fn output_path(
    original: &Path,
    dir: &Path,
    options: &ExportOptions,
    number: usize,
    total: usize,
) -> PathBuf {
    let stem = export_file_stem(&options.file_name, original, number, total);
    dir.join(format!("{stem}.{}", options.format.extension()))
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
/// sidecar (or the edits imported from Lightroom's); `adjust` runs on the settings after decoding (CLI overrides).
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
        None => load_photo_settings(original)?
            .map(|loaded| loaded.settings)
            .unwrap_or_default(),
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
    fn file_names_replace_tokens_and_stay_inside_the_folder() {
        let original = Path::new("/photos/IMG_0001.CR3");
        assert_eq!(export_file_stem("{name}", original, 1, 1), "IMG_0001");
        assert_eq!(export_file_stem("Sunset", original, 1, 1), "Sunset");
        assert_eq!(
            export_file_stem("Trip {n} {name}", original, 3, 12),
            "Trip 03 IMG_0001"
        );
        assert_eq!(
            export_file_stem("Trip", original, 7, 120),
            "Trip-007",
            "batches stay distinct"
        );
        assert_eq!(export_file_stem("../../etc/x", original, 1, 1), "-..-etc-x");
        assert_eq!(export_file_stem(" .hidden ", original, 1, 1), "hidden");
        assert_eq!(
            export_file_stem("  ", original, 1, 1),
            "IMG_0001",
            "empty falls back"
        );
        assert_eq!(
            export_file_stem(&"a".repeat(500), original, 1, 1).len(),
            MAX_STEM_CHARS
        );
    }

    #[test]
    fn output_path_uses_the_stem_and_format_extension() {
        assert_eq!(
            output_path(
                Path::new("/photos/IMG_0001.CR3"),
                Path::new("/out"),
                &ExportOptions {
                    format: Format::Png,
                    ..ExportOptions::default()
                },
                1,
                1
            ),
            PathBuf::from("/out/IMG_0001.png")
        );
    }
}
