// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Encoders for rendered images. Every file carries the ICC profile of its
//! colour space so viewers display it correctly.

use photonation_color::ColorSpace;
use photonation_image::{BitDepth, OutputImage, Pixels};
use std::borrow::Cow;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg { quality: u8 },
    Png,
    Tiff,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpeg { .. } => "jpg",
            Format::Png => "png",
            Format::Tiff => "tif",
        }
    }

    /// Whether the format can store images of this bit depth.
    pub fn supports(self, depth: BitDepth) -> bool {
        !matches!((self, depth), (Format::Jpeg { .. }, BitDepth::Sixteen))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Encode { path: PathBuf, message: String },
    #[error("{format:?} cannot store {depth:?} images")]
    Depth { format: Format, depth: BitDepth },
}

pub fn icc_profile(space: ColorSpace) -> Vec<u8> {
    let profile = match space {
        ColorSpace::Srgb => moxcms::ColorProfile::new_srgb(),
        ColorSpace::DisplayP3 => moxcms::ColorProfile::new_display_p3(),
        ColorSpace::AdobeRgb => moxcms::ColorProfile::new_adobe_rgb(),
        ColorSpace::ProphotoRgb => moxcms::ColorProfile::new_pro_photo_rgb(),
    };
    // Encoding the built-in profiles cannot fail; an empty profile only
    // means a file without embedded colour information.
    profile.encode().unwrap_or_default()
}

/// Writes `img` to `path` atomically (temporary file, then rename).
pub fn write(path: &Path, img: &OutputImage, format: Format) -> Result<(), ExportError> {
    if !format.supports(img.bit_depth()) {
        return Err(ExportError::Depth {
            format,
            depth: img.bit_depth(),
        });
    }
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".partial");
    let tmp = path.with_file_name(tmp_name);
    let result = encode_to(&tmp, img, format).and_then(|()| {
        std::fs::rename(&tmp, path).map_err(|source| ExportError::Io {
            path: path.into(),
            source,
        })
    });
    if result.is_err() {
        std::fs::remove_file(&tmp).ok();
    }
    result
}

fn encode_to(path: &Path, img: &OutputImage, format: Format) -> Result<(), ExportError> {
    let io_error = |source| ExportError::Io {
        path: path.into(),
        source,
    };
    let encode_error = |e: &dyn std::fmt::Display| ExportError::Encode {
        path: path.into(),
        message: e.to_string(),
    };
    let icc = icc_profile(img.color_space);
    let mut out = BufWriter::new(File::create(path).map_err(io_error)?);

    match (format, &img.pixels) {
        (Format::Jpeg { quality }, Pixels::U8(px)) => {
            let (w, h) =
                dims_u16(img).ok_or_else(|| encode_error(&"JPEG is limited to 65535×65535"))?;
            let mut e = jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100));
            e.set_optimized_huffman_tables(true);
            if quality >= 90 {
                e.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1);
            }
            if !icc.is_empty() {
                e.add_icc_profile(&icc).map_err(|e| encode_error(&e))?;
            }
            e.encode(px, w, h, jpeg_encoder::ColorType::Rgb)
                .map_err(|e| encode_error(&e))?;
        }
        (Format::Png, pixels) => {
            let mut info = png::Info::with_size(img.width as u32, img.height as u32);
            info.color_type = png::ColorType::Rgb;
            if !icc.is_empty() {
                info.icc_profile = Some(Cow::Borrowed(&icc));
            }
            let data: Cow<[u8]> = match pixels {
                Pixels::U8(px) => {
                    info.bit_depth = png::BitDepth::Eight;
                    Cow::Borrowed(px)
                }
                Pixels::U16(px) => {
                    info.bit_depth = png::BitDepth::Sixteen;
                    Cow::Owned(px.iter().flat_map(|v| v.to_be_bytes()).collect())
                }
            };
            let mut e = png::Encoder::with_info(&mut out, info).map_err(|e| encode_error(&e))?;
            e.set_compression(png::Compression::Fast);
            let mut w = e.write_header().map_err(|e| encode_error(&e))?;
            w.write_image_data(&data).map_err(|e| encode_error(&e))?;
            w.finish().map_err(|e| encode_error(&e))?;
        }
        (Format::Tiff, Pixels::U8(px)) => {
            write_tiff::<_, tiff::encoder::colortype::RGB8>(&mut out, img, px, &icc)
                .map_err(|e| encode_error(&e))?
        }
        (Format::Tiff, Pixels::U16(px)) => {
            write_tiff::<_, tiff::encoder::colortype::RGB16>(&mut out, img, px, &icc)
                .map_err(|e| encode_error(&e))?
        }
        (format, _) => {
            return Err(ExportError::Depth {
                format,
                depth: img.bit_depth(),
            });
        }
    }
    out.flush().map_err(io_error)?;
    out.into_inner()
        .map_err(|e| io_error(e.into_error()))?
        .sync_all()
        .map_err(io_error)
}

fn write_tiff<W, C>(
    out: W,
    img: &OutputImage,
    samples: &[C::Inner],
    icc: &[u8],
) -> tiff::TiffResult<()>
where
    W: Write + std::io::Seek,
    C: tiff::encoder::colortype::ColorType,
    [C::Inner]: tiff::encoder::TiffValue,
{
    let mut encoder = tiff::encoder::TiffEncoder::new(out)?
        .with_compression(tiff::encoder::Compression::Deflate(Default::default()));
    let mut image = encoder.new_image::<C>(img.width as u32, img.height as u32)?;
    if !icc.is_empty() {
        image
            .encoder()
            .write_tag(tiff::tags::Tag::IccProfile, icc)?;
    }
    image.write_data(samples)
}

fn dims_u16(img: &OutputImage) -> Option<(u16, u16)> {
    Some((
        u16::try_from(img.width).ok()?,
        u16::try_from(img.height).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny(depth: BitDepth) -> OutputImage {
        let n = 6 * 4 * 3;
        let pixels = match depth {
            BitDepth::Eight => Pixels::U8((0..n).map(|i| (i * 3) as u8).collect()),
            BitDepth::Sixteen => Pixels::U16((0..n).map(|i| (i * 900) as u16).collect()),
        };
        OutputImage {
            width: 6,
            height: 4,
            color_space: ColorSpace::DisplayP3,
            pixels,
        }
    }

    #[test]
    fn profiles_encode() {
        for cs in ColorSpace::ALL {
            let icc = icc_profile(cs);
            assert!(icc.len() > 128, "{cs:?}");
            assert_eq!(&icc[36..40], b"acsp");
        }
    }

    #[test]
    fn writes_every_format() {
        let dir = std::env::temp_dir().join(format!("photonation-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        for (format, depth) in [
            (Format::Jpeg { quality: 90 }, BitDepth::Eight),
            (Format::Png, BitDepth::Eight),
            (Format::Png, BitDepth::Sixteen),
            (Format::Tiff, BitDepth::Sixteen),
        ] {
            let path = dir.join(format!("t-{depth:?}.{}", format.extension()));
            write(&path, &tiny(depth), format).expect("write");
            assert!(std::fs::metadata(&path).expect("exists").len() > 100);
        }
        assert!(matches!(
            write(
                &dir.join("x.jpg"),
                &tiny(BitDepth::Sixteen),
                Format::Jpeg { quality: 90 }
            ),
            Err(ExportError::Depth { .. })
        ));
        std::fs::remove_dir_all(&dir).ok();
    }
}
