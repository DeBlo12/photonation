// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Encoders for rendered images. Every file carries the ICC profile of its
//! colour space so viewers display it correctly, and the photo's metadata
//! as EXIF (see [`exif`]) and XMP (see [`xmp`]).

mod exif;
mod xmp;

use photonation_color::ColorSpace;
use photonation_image::{BitDepth, CameraInfo, OutputImage, Pixels};
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

/// Writes `img` to `path` atomically: it is encoded into a new temporary
/// file beside `path` (created exclusively under a random name, so a file
/// or symlink planted in a shared folder is never written through),
/// flushed to disk, then renamed over `path`. A failed export leaves
/// nothing behind.
///
/// `metadata` is written as EXIF and XMP; fields left empty are left out,
/// and an empty `metadata` writes neither.
pub fn write(
    path: &Path,
    img: &OutputImage,
    format: Format,
    metadata: &CameraInfo,
) -> Result<(), ExportError> {
    if !format.supports(img.bit_depth()) {
        return Err(ExportError::Depth {
            format,
            depth: img.bit_depth(),
        });
    }
    let io_error = |source| ExportError::Io {
        path: path.into(),
        source,
    };
    let directory = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::Builder::new();
    temporary.prefix(".photonation-").suffix(".partial");
    // Same permissions a plain create would give (0o666 minus umask), so
    // exports stay readable as before, instead of tempfile's owner-only default.
    #[cfg(unix)]
    temporary.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666));
    let mut file = temporary.tempfile_in(directory).map_err(io_error)?;
    encode_to(file.as_file_mut(), path, img, format, metadata)?;
    file.persist(path).map_err(|e| io_error(e.error))?;
    Ok(())
}

/// Encodes into `file`; `path` (the final destination) names errors.
fn encode_to(
    file: &mut File,
    path: &Path,
    img: &OutputImage,
    format: Format,
    metadata: &CameraInfo,
) -> Result<(), ExportError> {
    let io_error = |source| ExportError::Io {
        path: path.into(),
        source,
    };
    let encode_error = |e: &dyn std::fmt::Display| ExportError::Encode {
        path: path.into(),
        message: e.to_string(),
    };
    let icc = icc_profile(img.color_space);
    let exif = exif::Directories::new(metadata, img);
    let xmp = xmp::packet(metadata);
    let mut out = BufWriter::new(file);

    match (format, &img.pixels) {
        (Format::Jpeg { quality }, Pixels::U8(px)) => {
            let (w, h) =
                dims_u16(img).ok_or_else(|| encode_error(&"JPEG is limited to 65535×65535"))?;
            let mut e = jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100));
            e.set_optimized_huffman_tables(true);
            if quality >= 90 {
                e.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1);
            }
            // The APP1 segments (EXIF, then XMP) go before the APP2 ICC
            // profile segments.
            if let Some(exif) = &exif {
                let segment = [exif::JPEG_APP1_PREFIX, &exif.tiff_stream()].concat();
                e.add_app_segment(1, segment)
                    .map_err(|e| encode_error(&e))?;
            }
            if let Some(xmp) = &xmp {
                let segment = [xmp::JPEG_APP1_PREFIX, xmp.as_bytes()].concat();
                e.add_app_segment(1, segment)
                    .map_err(|e| encode_error(&e))?;
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
            info.exif_metadata = exif.as_ref().map(|exif| Cow::Owned(exif.tiff_stream()));
            if let Some(xmp) = &xmp {
                info.utf8_text
                    .push(png::text_metadata::ITXtChunk::new(xmp::PNG_KEYWORD, xmp));
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
        (Format::Tiff, Pixels::U8(px)) => write_tiff::<_, tiff::encoder::colortype::RGB8>(
            &mut out,
            img,
            px,
            &icc,
            exif.as_ref(),
            xmp.as_deref(),
        )
        .map_err(|e| encode_error(&e))?,
        (Format::Tiff, Pixels::U16(px)) => write_tiff::<_, tiff::encoder::colortype::RGB16>(
            &mut out,
            img,
            px,
            &icc,
            exif.as_ref(),
            xmp.as_deref(),
        )
        .map_err(|e| encode_error(&e))?,
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
    exif: Option<&exif::Directories>,
    xmp: Option<&str>,
) -> tiff::TiffResult<()>
where
    W: Write + std::io::Seek,
    C: tiff::encoder::colortype::ColorType,
    [C::Inner]: tiff::encoder::TiffValue,
{
    let mut encoder = tiff::encoder::TiffEncoder::new(out)?
        .with_compression(tiff::encoder::Compression::Deflate(Default::default()));
    // The Exif and GPS directories are written first, so the image's own
    // directory (IFD0) can point to them.
    let mut primary = exif
        .map(|exif| exif::write_tiff_sub_directories(&mut encoder, exif))
        .transpose()?;
    if let (Some(primary), Some(xmp)) = (&mut primary, xmp) {
        primary.push((xmp::TIFF_TAG, exif::Value::Byte(xmp.as_bytes().to_vec())));
    }
    let mut image = encoder.new_image::<C>(img.width as u32, img.height as u32)?;
    if !icc.is_empty() {
        image
            .encoder()
            .write_tag(tiff::tags::Tag::IccProfile, icc)?;
    }
    if let Some(primary) = &primary {
        exif::write_tiff_fields(image.encoder(), primary)?;
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
        let temp = tempfile::tempdir().expect("temp dir");
        let dir = temp.path();
        for (format, depth) in [
            (Format::Jpeg { quality: 90 }, BitDepth::Eight),
            (Format::Png, BitDepth::Eight),
            (Format::Png, BitDepth::Sixteen),
            (Format::Tiff, BitDepth::Sixteen),
        ] {
            let path = dir.join(format!("t-{depth:?}.{}", format.extension()));
            write(&path, &tiny(depth), format, &CameraInfo::default()).expect("write");
            assert!(std::fs::metadata(&path).expect("exists").len() > 100);
        }
        assert!(matches!(
            write(
                &dir.join("x.jpg"),
                &tiny(BitDepth::Sixteen),
                Format::Jpeg { quality: 90 },
                &CameraInfo::default()
            ),
            Err(ExportError::Depth { .. })
        ));
        // Only the finished files remain; no temporary files are left over.
        assert_eq!(std::fs::read_dir(dir).expect("list").count(), 4);
    }

    /// What an exported file embeds; `None` where it has none.
    struct Embedded {
        /// The EXIF TIFF stream.
        exif: Option<Vec<u8>>,
        xmp: Option<String>,
        icc: Option<Vec<u8>>,
    }

    fn embedded(bytes: &[u8], format: Format) -> Embedded {
        let utf8 = |bytes: &[u8]| String::from_utf8(bytes.to_vec()).expect("UTF-8 XMP");
        match format {
            Format::Jpeg { .. } => {
                let (mut exif, mut xmp, mut icc) = (None, None, Vec::new());
                let mut at = 2; // after SOI
                while bytes[at] == 0xFF && (0xE0..=0xEF).contains(&bytes[at + 1]) {
                    let length = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
                    let data = &bytes[at + 4..at + 2 + length];
                    if let Some(stream) = data.strip_prefix(exif::JPEG_APP1_PREFIX) {
                        exif = Some(stream.to_vec());
                    }
                    if let Some(packet) = data.strip_prefix(xmp::JPEG_APP1_PREFIX) {
                        xmp = Some(utf8(packet));
                    }
                    if let Some(chunk) = data.strip_prefix(b"ICC_PROFILE\0") {
                        icc.extend(&chunk[2..]); // after sequence number and count
                    }
                    at += 2 + length;
                }
                Embedded {
                    exif,
                    xmp,
                    icc: (!icc.is_empty()).then_some(icc),
                }
            }
            Format::Png => {
                let reader = png::Decoder::new(std::io::Cursor::new(bytes))
                    .read_info()
                    .expect("png");
                let info = reader.info();
                Embedded {
                    exif: info.exif_metadata.as_ref().map(|e| e.to_vec()),
                    xmp: info
                        .utf8_text
                        .iter()
                        .find(|chunk| chunk.keyword == xmp::PNG_KEYWORD)
                        .map(|chunk| chunk.get_text().expect("text")),
                    icc: info.icc_profile.as_ref().map(|p| p.to_vec()),
                }
            }
            Format::Tiff => {
                let mut decoder =
                    tiff::decoder::Decoder::new(std::io::Cursor::new(bytes)).expect("tiff");
                assert_eq!(decoder.dimensions().expect("size"), (6, 4));
                decoder.read_image().expect("pixels decode");
                // The file itself is the TIFF stream.
                let primary = exif::read_back::read(bytes).primary;
                Embedded {
                    exif: primary
                        .contains_key(&exif::tag::EXIF_IFD)
                        .then(|| bytes.to_vec()),
                    xmp: primary.get(&xmp::TIFF_TAG).map(|field| utf8(&field.bytes)),
                    icc: primary
                        .get(&ICC_PROFILE_TAG)
                        .map(|field| field.bytes.clone()),
                }
            }
        }
    }

    const ICC_PROFILE_TAG: u16 = 34675;

    /// Equal ICC profiles, ignoring the creation time in the header
    /// (bytes 24..36), which is stamped when the profile is encoded.
    fn same_profile(a: &[u8], b: &[u8]) -> bool {
        a.len() == b.len() && a[..24] == b[..24] && a[36..] == b[36..]
    }

    const EVERY_FORMAT: [(Format, BitDepth); 4] = [
        (Format::Jpeg { quality: 90 }, BitDepth::Eight),
        (Format::Png, BitDepth::Sixteen),
        (Format::Tiff, BitDepth::Eight),
        (Format::Tiff, BitDepth::Sixteen),
    ];

    #[test]
    fn every_format_carries_metadata_and_keeps_its_icc_profile() {
        use exif::tag;
        let temp = tempfile::tempdir().expect("temp dir");
        let metadata = exif::read_back::sample_metadata();
        for (format, depth) in EVERY_FORMAT {
            let path = temp
                .path()
                .join(format!("m-{depth:?}.{}", format.extension()));
            write(&path, &tiny(depth), format, &metadata).expect("write");
            let found = embedded(&std::fs::read(&path).expect("read"), format);
            let icc = found.icc.expect("ICC present");
            assert!(
                same_profile(&icc, &icc_profile(ColorSpace::DisplayP3)),
                "{format:?}"
            );

            let exif = exif::read_back::read(&found.exif.expect("EXIF present"));
            let primary = &exif.primary;
            assert_eq!(primary[&tag::MODEL].text(), "Canon EOS 250D", "{format:?}");
            assert_eq!(primary[&tag::ORIENTATION].short(), 1, "{format:?}");
            assert_eq!(primary[&tag::ARTIST].text(), "Ada Lovelace");
            assert_eq!(primary[&tag::COPYRIGHT].text(), "© 2026 Ada Lovelace");
            let lens = exif.exif[&tag::LENS_MODEL].text();
            assert_eq!(lens, "Canon EF 50mm f/1.8 STM");
            let taken = exif.exif[&tag::DATE_TIME_ORIGINAL].text();
            assert_eq!(taken, "2026:06:20 18:02:23");
            assert_eq!(exif.exif[&tag::EXPOSURE_TIME].rational(0), 1.0 / 4000.0);
            assert_eq!(exif.gps[&tag::GPS_LATITUDE_REF].text(), "N", "{format:?}");

            let xmp = found.xmp.expect("XMP present");
            assert!(xmp.contains("<rdf:li>Ada Lovelace</rdf:li>"), "{format:?}");
            assert!(xmp.contains("aux:Lens=\"Canon EF 50mm f/1.8 STM\""));
        }
    }

    #[test]
    fn exports_without_metadata_carry_none() {
        let temp = tempfile::tempdir().expect("temp dir");
        for (format, depth) in EVERY_FORMAT {
            let path = temp
                .path()
                .join(format!("plain-{depth:?}.{}", format.extension()));
            write(&path, &tiny(depth), format, &CameraInfo::default()).expect("write");
            let found = embedded(&std::fs::read(&path).expect("read"), format);
            assert!(found.exif.is_none(), "{format:?}");
            assert!(found.xmp.is_none(), "{format:?}");
            assert!(found.icc.is_some(), "{format:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn never_writes_through_planted_symlinks() {
        // Older builds encoded into the predictable `<name>.partial`; a link
        // planted there (or at the target) must not redirect the write.
        let temp = tempfile::tempdir().expect("temp dir");
        let victim = temp.path().join("victim.txt");
        std::fs::write(&victim, "precious").expect("write victim");
        let target = temp.path().join("out.png");
        std::os::unix::fs::symlink(&victim, &target).expect("symlink");
        std::os::unix::fs::symlink(&victim, temp.path().join("out.png.partial")).expect("symlink");
        write(
            &target,
            &tiny(BitDepth::Eight),
            Format::Png,
            &CameraInfo::default(),
        )
        .expect("write");
        assert_eq!(
            std::fs::read_to_string(&victim).expect("read victim"),
            "precious"
        );
        assert!(
            !std::fs::symlink_metadata(&target)
                .expect("stat")
                .is_symlink()
        );
    }

    #[test]
    fn failed_export_leaves_no_temporary_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let too_wide = OutputImage {
            width: 70_000,
            height: 1,
            color_space: ColorSpace::Srgb,
            pixels: Pixels::U8(vec![0; 70_000 * 3]),
        };
        let target = temp.path().join("wide.jpg");
        assert!(matches!(
            write(
                &target,
                &too_wide,
                Format::Jpeg { quality: 90 },
                &CameraInfo::default()
            ),
            Err(ExportError::Encode { .. })
        ));
        assert_eq!(std::fs::read_dir(temp.path()).expect("list").count(), 0);
    }
}
