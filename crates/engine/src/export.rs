// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::{Decoder, MetadataOptions, RawDecoder, Retoucher, default_model_path};
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
    /// Which of the photo's metadata the file carries.
    pub metadata: MetadataOptions,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            format: Format::Jpeg { quality: 92 },
            bit_depth: BitDepth::Eight,
            color_space: ColorSpace::Srgb,
            long_edge: None,
            file_name: NAME_TOKEN.into(),
            metadata: MetadataOptions::default(),
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
    #[error("removal failed: {0}")]
    Retouch(String),
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
/// sidecar (or the edits imported from Lightroom's); `adjust` runs on the
/// settings after decoding (CLI overrides). Removals are filled with the
/// default model location ([`default_model_path`]); batch callers should
/// share one [`Retoucher`] through [`export_photo_with`] so the model loads
/// once.
pub fn export_photo(
    original: &Path,
    target: &Path,
    settings: Option<&Settings>,
    options: &ExportOptions,
    adjust: impl FnOnce(&mut Settings, &photonation_image::SensorImage),
) -> Result<ExportReport, ExportError> {
    let retoucher = Retoucher::new(default_model_path());
    export_photo_with(
        &RawDecoder,
        &retoucher,
        original,
        target,
        settings,
        options,
        adjust,
    )
}

/// [`export_photo`] with the photo decoded by `decoder` and removals filled
/// (or taken from the cache) by `retoucher`.
pub fn export_photo_with(
    decoder: &dyn Decoder,
    retoucher: &Retoucher,
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
    let fills = retoucher
        .fill_missing(original, &sensor, &settings, || {})
        .map_err(ExportError::Retouch)?;
    let image =
        photonation_pipeline::render_with_fills(&sensor, &settings, &fills, &render_options)?;
    let render = started.elapsed();
    let metadata = options.metadata.select(&sensor.info);
    drop(sensor);

    let started = Instant::now();
    photonation_export::write(target, &image, options.format, &metadata)?;
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

    /// Decodes every path to a small test pattern shot with a Canon.
    struct CanonDecoder;

    impl Decoder for CanonDecoder {
        fn decode(
            &self,
            _: &Path,
        ) -> Result<photonation_image::SensorImage, photonation_raw::RawError> {
            let mut sensor = photonation_image::SensorImage::test_pattern(
                48,
                32,
                photonation_image::Orientation::Normal,
            );
            sensor.info = photonation_image::CameraInfo {
                make: "Canon".into(),
                model: "Canon EOS 250D".into(),
                date_time: Some("2026:06:20 18:02:23".into()),
                ..Default::default()
            };
            Ok(sensor)
        }
    }

    #[test]
    fn exports_carry_the_chosen_metadata() {
        let dir = tempfile::tempdir().expect("temp dir");
        let export = |metadata: MetadataOptions| {
            let target = dir.path().join("out.jpg");
            let options = ExportOptions {
                metadata,
                ..ExportOptions::default()
            };
            export_photo_with(
                &CanonDecoder,
                &Retoucher::new(None),
                Path::new("IMG_0001.CR3"),
                &target,
                Some(&Settings::default()),
                &options,
                |_, _| {},
            )
            .expect("export");
            std::fs::read(target).expect("read")
        };
        let contains =
            |bytes: &[u8], text: &str| bytes.windows(text.len()).any(|w| w == text.as_bytes());

        let all = export(MetadataOptions {
            creator: "Ada Lovelace".into(),
            ..MetadataOptions::default()
        });
        assert!(contains(&all, "Exif\0\0"));
        assert!(contains(&all, "Canon EOS 250D"));
        assert!(contains(&all, "2026:06:20 18:02:23"));
        assert!(contains(&all, "Ada Lovelace"));

        let mut copyright_only = MetadataOptions {
            creator: "Ada Lovelace".into(),
            ..MetadataOptions::default()
        };
        copyright_only.apply(crate::MetadataPreset::CopyrightOnly);
        let bytes = export(copyright_only);
        assert!(contains(&bytes, "Ada Lovelace"));
        assert!(!contains(&bytes, "Canon EOS 250D"));

        let mut none = MetadataOptions::default();
        none.apply(crate::MetadataPreset::None);
        let bytes = export(none);
        assert!(!contains(&bytes, "Exif\0\0"));
        assert!(contains(&bytes, "ICC_PROFILE"), "the profile stays");
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
