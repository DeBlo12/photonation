// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! A minimal EXIF writer. EXIF is a set of TIFF directories: IFD0 (camera,
//! creator, orientation), the Exif IFD (exposure, capture time, colour
//! space) and the GPS IFD. JPEG carries them as a TIFF stream in an APP1
//! segment, PNG as the same stream in an `eXIf` chunk, and a TIFF file
//! holds them as directories of its own.

use photonation_color::ColorSpace;
use photonation_image::{CameraInfo, GpsPosition, OutputImage};

/// Prefix of the JPEG APP1 segment that holds EXIF.
pub const JPEG_APP1_PREFIX: &[u8] = b"Exif\0\0";
/// Longest text field written, in bytes (a JPEG segment holds 64 KiB in all).
const MAX_TEXT_BYTES: usize = 2000;
/// The `Software` (EXIF) and `CreatorTool` (XMP) of every export.
pub const SOFTWARE: &str = concat!("Photonation ", env!("CARGO_PKG_VERSION"));

/// IFD0 tags.
pub mod tag {
    pub const MAKE: u16 = 0x010F;
    pub const MODEL: u16 = 0x0110;
    pub const ORIENTATION: u16 = 0x0112;
    pub const SOFTWARE: u16 = 0x0131;
    pub const ARTIST: u16 = 0x013B;
    pub const COPYRIGHT: u16 = 0x8298;
    pub const EXIF_IFD: u16 = 0x8769;
    pub const GPS_IFD: u16 = 0x8825;
    // Exif IFD
    pub const EXPOSURE_TIME: u16 = 0x829A;
    pub const F_NUMBER: u16 = 0x829D;
    pub const ISO: u16 = 0x8827;
    pub const EXIF_VERSION: u16 = 0x9000;
    pub const DATE_TIME_ORIGINAL: u16 = 0x9003;
    pub const DATE_TIME_DIGITIZED: u16 = 0x9004;
    pub const OFFSET_TIME_ORIGINAL: u16 = 0x9011;
    pub const OFFSET_TIME_DIGITIZED: u16 = 0x9012;
    pub const EXPOSURE_BIAS: u16 = 0x9204;
    pub const FOCAL_LENGTH: u16 = 0x920A;
    pub const COLOR_SPACE: u16 = 0xA001;
    pub const PIXEL_X_DIMENSION: u16 = 0xA002;
    pub const PIXEL_Y_DIMENSION: u16 = 0xA003;
    pub const LENS_MODEL: u16 = 0xA434;
    // GPS IFD
    pub const GPS_VERSION: u16 = 0x0000;
    pub const GPS_LATITUDE_REF: u16 = 0x0001;
    pub const GPS_LATITUDE: u16 = 0x0002;
    pub const GPS_LONGITUDE_REF: u16 = 0x0003;
    pub const GPS_LONGITUDE: u16 = 0x0004;
    pub const GPS_ALTITUDE_REF: u16 = 0x0005;
    pub const GPS_ALTITUDE: u16 = 0x0006;
}

/// One TIFF field value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// NUL-terminated text. Written as UTF-8, which readers accept in
    /// ASCII fields, so names and "©" survive.
    Ascii(String),
    Byte(Vec<u8>),
    Short(u16),
    Long(u32),
    Rational(Vec<(u32, u32)>),
    SRational(i32, i32),
    Undefined(Vec<u8>),
}

impl Value {
    fn type_code(&self) -> u16 {
        match self {
            Value::Byte(_) => 1,
            Value::Ascii(_) => 2,
            Value::Short(_) => 3,
            Value::Long(_) => 4,
            Value::Rational(_) => 5,
            Value::Undefined(_) => 7,
            Value::SRational(..) => 10,
        }
    }

    fn count(&self) -> u32 {
        let count = match self {
            Value::Ascii(text) => text.len() + 1,
            Value::Byte(bytes) | Value::Undefined(bytes) => bytes.len(),
            Value::Short(_) | Value::Long(_) | Value::SRational(..) => 1,
            Value::Rational(values) => values.len(),
        };
        count as u32
    }

    /// The value's bytes in the given byte order.
    fn bytes(&self, order: ByteOrder) -> Vec<u8> {
        let u16_bytes = |v: u16| match order {
            ByteOrder::Little => v.to_le_bytes(),
            ByteOrder::Big => v.to_be_bytes(),
        };
        let u32_bytes = |v: u32| match order {
            ByteOrder::Little => v.to_le_bytes(),
            ByteOrder::Big => v.to_be_bytes(),
        };
        match self {
            Value::Ascii(text) => [text.as_bytes(), &[0]].concat(),
            Value::Byte(bytes) | Value::Undefined(bytes) => bytes.clone(),
            Value::Short(v) => u16_bytes(*v).to_vec(),
            Value::Long(v) => u32_bytes(*v).to_vec(),
            Value::Rational(values) => values
                .iter()
                .flat_map(|&(n, d)| [u32_bytes(n), u32_bytes(d)])
                .flatten()
                .collect(),
            Value::SRational(n, d) => [u32_bytes(*n as u32), u32_bytes(*d as u32)].concat(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOrder {
    Little,
    Big,
}

impl ByteOrder {
    pub fn native() -> ByteOrder {
        if cfg!(target_endian = "big") {
            ByteOrder::Big
        } else {
            ByteOrder::Little
        }
    }
}

/// Fields of one directory, sorted by tag as TIFF requires.
pub type Fields = Vec<(u16, Value)>;

/// The EXIF directories for one export. The IFD0 pointers to the Exif
/// and GPS directories are added by whoever lays out the file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Directories {
    pub primary: Fields,
    pub exif: Fields,
    pub gps: Fields,
}

impl Directories {
    /// The fields to write for `metadata` on `img`, or `None` when there is
    /// nothing to record (an export without metadata carries no EXIF).
    ///
    /// Orientation is always "normal" (1): exports are rendered upright.
    /// The colour space is sRGB (1) or "uncalibrated" (0xFFFF), which
    /// tells readers to use the embedded ICC profile.
    pub fn new(metadata: &CameraInfo, img: &OutputImage) -> Option<Directories> {
        if *metadata == CameraInfo::default() {
            return None;
        }
        let mut primary = Fields::new();
        let mut exif = Fields::new();
        let text = |fields: &mut Fields, tag: u16, value: Option<&str>| {
            if let Some(value) = value.and_then(field_text) {
                fields.push((tag, Value::Ascii(value)));
            }
        };
        text(&mut primary, tag::MAKE, Some(&metadata.make));
        text(&mut primary, tag::MODEL, Some(&metadata.model));
        primary.push((tag::ORIENTATION, Value::Short(1)));
        text(&mut primary, tag::SOFTWARE, Some(SOFTWARE));
        text(&mut primary, tag::ARTIST, metadata.artist.as_deref());
        text(&mut primary, tag::COPYRIGHT, metadata.copyright.as_deref());

        let rational = |value: Option<f64>| Some(Value::Rational(vec![unsigned_rational(value?)?]));
        if let Some(value) = rational(metadata.exposure_time) {
            exif.push((tag::EXPOSURE_TIME, value));
        }
        if let Some(value) = rational(metadata.f_number) {
            exif.push((tag::F_NUMBER, value));
        }
        if let Some(iso) = metadata.iso {
            exif.push((tag::ISO, Value::Short(iso.min(u16::MAX.into()) as u16)));
        }
        exif.push((tag::EXIF_VERSION, Value::Undefined(b"0232".to_vec())));
        text(
            &mut exif,
            tag::DATE_TIME_ORIGINAL,
            metadata.date_time.as_deref(),
        );
        text(
            &mut exif,
            tag::DATE_TIME_DIGITIZED,
            metadata.date_time.as_deref(),
        );
        if metadata.date_time.is_some() {
            let offset = metadata.time_offset.as_deref();
            text(&mut exif, tag::OFFSET_TIME_ORIGINAL, offset);
            text(&mut exif, tag::OFFSET_TIME_DIGITIZED, offset);
        }
        if let Some((n, d)) = metadata.exposure_bias.and_then(signed_rational) {
            exif.push((tag::EXPOSURE_BIAS, Value::SRational(n, d)));
        }
        if let Some(value) = rational(metadata.focal_length) {
            exif.push((tag::FOCAL_LENGTH, value));
        }
        let color_space = match img.color_space {
            ColorSpace::Srgb => 1,
            _ => 0xFFFF,
        };
        exif.push((tag::COLOR_SPACE, Value::Short(color_space)));
        exif.push((tag::PIXEL_X_DIMENSION, Value::Long(img.width as u32)));
        exif.push((tag::PIXEL_Y_DIMENSION, Value::Long(img.height as u32)));
        text(&mut exif, tag::LENS_MODEL, metadata.lens.as_deref());

        let gps = metadata.gps.map(gps_fields).unwrap_or_default();
        Some(Directories { primary, exif, gps })
    }

    /// A complete little-endian TIFF stream: header, IFD0, Exif IFD and
    /// GPS IFD, each followed by the values too long to sit in its entries.
    /// Offsets count from the header, as JPEG APP1 and PNG `eXIf` expect.
    pub fn tiff_stream(&self) -> Vec<u8> {
        let order = ByteOrder::Little;
        let mut primary = self.primary.clone();
        let exif_offset = HEADER_LEN + ifd_len(&primary) + self.pointer_count() * ENTRY_LEN;
        let gps_offset = exif_offset + ifd_len(&self.exif);
        primary.push((tag::EXIF_IFD, Value::Long(exif_offset)));
        if !self.gps.is_empty() {
            primary.push((tag::GPS_IFD, Value::Long(gps_offset)));
        }
        primary.sort_by_key(|(tag, _)| *tag);

        let mut out = b"II\x2A\0".to_vec();
        out.extend(HEADER_LEN.to_le_bytes());
        write_ifd(&mut out, &primary, order);
        write_ifd(&mut out, &self.exif, order);
        if !self.gps.is_empty() {
            write_ifd(&mut out, &self.gps, order);
        }
        out
    }

    /// How many directory pointers IFD0 gains (Exif, and GPS if present).
    fn pointer_count(&self) -> u32 {
        1 + u32::from(!self.gps.is_empty())
    }
}

const HEADER_LEN: u32 = 8;
const ENTRY_LEN: u32 = 12;

/// Bytes a directory takes: entry count, entries, next-IFD link, and the
/// values longer than four bytes (each padded to an even length).
fn ifd_len(fields: &Fields) -> u32 {
    let values: u32 = fields
        .iter()
        .map(|(_, value)| value.bytes(ByteOrder::Little).len() as u32)
        .filter(|&len| len > 4)
        .map(|len| len + len % 2)
        .sum();
    2 + fields.len() as u32 * ENTRY_LEN + 4 + values
}

/// Appends one directory, its long values right after it. `out` starts at
/// the TIFF header, so its length is the current offset.
fn write_ifd(out: &mut Vec<u8>, fields: &Fields, order: ByteOrder) {
    debug_assert!(
        out.len().is_multiple_of(2),
        "directories start at even offsets"
    );
    let u16_bytes = |v: u16| match order {
        ByteOrder::Little => v.to_le_bytes(),
        ByteOrder::Big => v.to_be_bytes(),
    };
    let u32_bytes = |v: u32| match order {
        ByteOrder::Little => v.to_le_bytes(),
        ByteOrder::Big => v.to_be_bytes(),
    };
    let start = out.len() as u32;
    let mut value_offset = start + 2 + fields.len() as u32 * ENTRY_LEN + 4;
    let mut long_values = Vec::new();
    out.extend(u16_bytes(fields.len() as u16));
    for (tag, value) in fields {
        let bytes = value.bytes(order);
        out.extend(u16_bytes(*tag));
        out.extend(u16_bytes(value.type_code()));
        out.extend(u32_bytes(value.count()));
        if bytes.len() <= 4 {
            let mut inline = [0u8; 4];
            inline[..bytes.len()].copy_from_slice(&bytes);
            out.extend(inline);
        } else {
            out.extend(u32_bytes(value_offset));
            value_offset += (bytes.len() + bytes.len() % 2) as u32;
            long_values.extend(&bytes);
            if bytes.len() % 2 == 1 {
                long_values.push(0);
            }
        }
    }
    out.extend(u32_bytes(0)); // no next directory
    out.extend(long_values);
}

/// Writes the Exif and GPS directories into a TIFF file being encoded, as
/// extra directories, and returns the IFD0 fields (with the pointers to
/// them) for the image's own directory, which comes after.
pub fn write_tiff_sub_directories<W, K>(
    encoder: &mut tiff::encoder::TiffEncoder<W, K>,
    directories: &Directories,
) -> tiff::TiffResult<Fields>
where
    W: std::io::Write + std::io::Seek,
    K: tiff::encoder::TiffKind,
{
    let mut primary = directories.primary.clone();
    let mut sub_directory = |fields: &Fields| -> tiff::TiffResult<u32> {
        let mut directory = encoder.extra_directory()?;
        write_tiff_fields(&mut directory, fields)?;
        let offset = directory.finish_with_offsets()?.pointer.0;
        u32::try_from(offset).map_err(|_| tiff::TiffError::LimitsExceeded)
    };
    primary.push((
        tag::EXIF_IFD,
        Value::Long(sub_directory(&directories.exif)?),
    ));
    if !directories.gps.is_empty() {
        primary.push((tag::GPS_IFD, Value::Long(sub_directory(&directories.gps)?)));
    }
    Ok(primary)
}

/// Adds `fields` to a directory the tiff encoder is writing (in its byte
/// order, the platform's).
pub fn write_tiff_fields<W, K>(
    directory: &mut tiff::encoder::DirectoryEncoder<'_, W, K>,
    fields: &Fields,
) -> tiff::TiffResult<()>
where
    W: std::io::Write + std::io::Seek,
    K: tiff::encoder::TiffKind,
{
    use tiff::tags::{Tag, Type};
    let mut entries = Vec::with_capacity(fields.len());
    for (tag, value) in fields {
        let field_type = match value {
            Value::Byte(_) => Type::BYTE,
            Value::Ascii(_) => Type::ASCII,
            Value::Short(_) => Type::SHORT,
            Value::Long(_) => Type::LONG,
            Value::Rational(_) => Type::RATIONAL,
            Value::Undefined(_) => Type::UNDEFINED,
            Value::SRational(..) => Type::SRATIONAL,
        };
        let entry = directory.write_entry_bytes(field_type, &value.bytes(ByteOrder::native()))?;
        entries.push((Tag::from_u16_exhaustive(*tag), entry));
    }
    directory.extend_from(&entries.into_iter().collect());
    Ok(())
}

fn gps_fields(position: GpsPosition) -> Fields {
    let mut fields = vec![(tag::GPS_VERSION, Value::Byte(vec![2, 3, 0, 0]))];
    let hemisphere = |value: f64, positive: &str, negative: &str| {
        Value::Ascii(if value < 0.0 { negative } else { positive }.into())
    };
    let Some(latitude) = degrees_minutes_seconds(position.latitude) else {
        return Fields::new();
    };
    let Some(longitude) = degrees_minutes_seconds(position.longitude) else {
        return Fields::new();
    };
    fields.push((
        tag::GPS_LATITUDE_REF,
        hemisphere(position.latitude, "N", "S"),
    ));
    fields.push((tag::GPS_LATITUDE, latitude));
    fields.push((
        tag::GPS_LONGITUDE_REF,
        hemisphere(position.longitude, "E", "W"),
    ));
    fields.push((tag::GPS_LONGITUDE, longitude));
    if let Some(altitude) = position.altitude
        && let Some(metres) = unsigned_rational(altitude.abs())
    {
        // Reference 1: below sea level.
        fields.push((
            tag::GPS_ALTITUDE_REF,
            Value::Byte(vec![u8::from(altitude < 0.0)]),
        ));
        fields.push((tag::GPS_ALTITUDE, Value::Rational(vec![metres])));
    }
    fields
}

/// `|degrees|` as EXIF's three rationals: whole degrees, whole minutes and
/// seconds to 1/10000.
fn degrees_minutes_seconds(degrees: f64) -> Option<Value> {
    let degrees = degrees.abs();
    if !degrees.is_finite() || degrees > 180.0 {
        return None;
    }
    // Rounded once, then split, so seconds never round up to 60.
    const STEPS_PER_SECOND: u32 = 10_000;
    let steps = (degrees * 3600.0 * STEPS_PER_SECOND as f64).round() as u64;
    let seconds = (steps % (60 * STEPS_PER_SECOND as u64)) as u32;
    let minutes = steps / (60 * STEPS_PER_SECOND as u64);
    Some(Value::Rational(vec![
        ((minutes / 60) as u32, 1),
        ((minutes % 60) as u32, 1),
        (seconds, STEPS_PER_SECOND),
    ]))
}

/// Text for a field: blank text is left out, NULs (which would end it
/// early) are dropped, and long text is cut at a character boundary.
pub fn field_text(text: &str) -> Option<String> {
    let text: String = text.trim().chars().filter(|&c| c != '\0').collect();
    if text.is_empty() {
        return None;
    }
    let mut end = text.len().min(MAX_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(text[..end].to_string())
}

fn unsigned_rational(value: f64) -> Option<(u32, u32)> {
    let (n, d) = nearest_fraction(value, u32::MAX.into())?;
    Some((n as u32, d as u32))
}

fn signed_rational(value: f64) -> Option<(i32, i32)> {
    let (n, d) = nearest_fraction(value.abs(), i32::MAX as u64)?;
    let n = n as i32;
    Some((if value < 0.0 { -n } else { n }, d as i32))
}

/// The simplest fraction n/d (both at most `max`) within a billionth of
/// `value` ≥ 0, from its continued fraction: 1/160 s stays 1/160 and −⅔ EV
/// stays −2/3 instead of becoming a decimal approximation.
fn nearest_fraction(value: f64, max: u64) -> Option<(u64, u64)> {
    if !value.is_finite() || value < 0.0 || value > max as f64 {
        return None;
    }
    // Convergents h/k, starting from h₋₂/k₋₂ = 0/1 and h₋₁/k₋₁ = 1/0.
    let (mut h, mut h_previous) = (1u64, 0u64);
    let (mut k, mut k_previous) = (0u64, 1u64);
    let mut remainder = value;
    loop {
        let whole = remainder.floor();
        let next_h = (whole as u64)
            .checked_mul(h)
            .and_then(|v| v.checked_add(h_previous));
        let next_k = (whole as u64)
            .checked_mul(k)
            .and_then(|v| v.checked_add(k_previous));
        let (Some(next_h), Some(next_k)) = (next_h, next_k) else {
            break;
        };
        if next_h > max || next_k > max {
            break;
        }
        (h_previous, h) = (h, next_h);
        (k_previous, k) = (k, next_k);
        let fraction = remainder - whole;
        let close = (h as f64 / k as f64 - value).abs() <= value * 1e-9;
        if fraction < 1e-12 || close {
            break;
        }
        remainder = 1.0 / fraction;
    }
    (k > 0).then_some((h, k))
}

/// Reads EXIF back from a TIFF stream, for tests here and in `lib.rs`.
#[cfg(test)]
pub mod read_back {
    use super::{ByteOrder, tag};
    use photonation_image::{CameraInfo, GpsPosition};
    use std::collections::BTreeMap;

    /// A field as read: its type code and raw bytes (in the stream's order).
    #[derive(Clone, Debug)]
    pub struct RawField {
        pub type_code: u16,
        pub bytes: Vec<u8>,
        order: ByteOrder,
    }

    impl RawField {
        pub fn text(&self) -> String {
            let end = self
                .bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(self.bytes.len());
            String::from_utf8_lossy(&self.bytes[..end]).into_owned()
        }

        pub fn u32_at(&self, index: usize) -> u32 {
            let b: [u8; 4] = self.bytes[index * 4..index * 4 + 4]
                .try_into()
                .expect("4 bytes");
            match self.order {
                ByteOrder::Little => u32::from_le_bytes(b),
                ByteOrder::Big => u32::from_be_bytes(b),
            }
        }

        pub fn short(&self) -> u16 {
            let b: [u8; 2] = self.bytes[..2].try_into().expect("2 bytes");
            match self.order {
                ByteOrder::Little => u16::from_le_bytes(b),
                ByteOrder::Big => u16::from_be_bytes(b),
            }
        }

        /// Rational number `index` as a float.
        pub fn rational(&self, index: usize) -> f64 {
            if self.type_code == 10 {
                self.u32_at(2 * index) as i32 as f64 / self.u32_at(2 * index + 1) as i32 as f64
            } else {
                self.u32_at(2 * index) as f64 / self.u32_at(2 * index + 1) as f64
            }
        }
    }

    /// The fields of IFD0, the Exif IFD and the GPS IFD (empty maps for
    /// directories that are absent).
    pub struct Exif {
        pub primary: BTreeMap<u16, RawField>,
        pub exif: BTreeMap<u16, RawField>,
        pub gps: BTreeMap<u16, RawField>,
    }

    /// Every field filled, with a non-ASCII copyright and a position in
    /// the western hemisphere below sea level.
    pub fn sample_metadata() -> CameraInfo {
        CameraInfo {
            make: "Canon".into(),
            model: "Canon EOS 250D".into(),
            lens: Some("Canon EF 50mm f/1.8 STM".into()),
            iso: Some(200),
            exposure_time: Some(1.0 / 4000.0),
            f_number: Some(2.2),
            focal_length: Some(50.0),
            exposure_bias: Some(-2.0 / 3.0),
            date_time: Some("2026:06:20 18:02:23".into()),
            time_offset: Some("+01:00".into()),
            artist: Some("Ada Lovelace".into()),
            copyright: Some("© 2026 Ada Lovelace".into()),
            gps: Some(GpsPosition {
                latitude: 48.137_154,
                longitude: -11.576_124,
                altitude: Some(-12.5),
            }),
        }
    }

    pub fn read(tiff: &[u8]) -> Exif {
        let order = match &tiff[..4] {
            b"II\x2A\0" => ByteOrder::Little,
            b"MM\0\x2A" => ByteOrder::Big,
            other => panic!("not a TIFF stream: {other:?}"),
        };
        let u16_at = |at: usize| {
            let b = [tiff[at], tiff[at + 1]];
            match order {
                ByteOrder::Little => u16::from_le_bytes(b),
                ByteOrder::Big => u16::from_be_bytes(b),
            }
        };
        let u32_at = |at: usize| {
            let b = [tiff[at], tiff[at + 1], tiff[at + 2], tiff[at + 3]];
            match order {
                ByteOrder::Little => u32::from_le_bytes(b),
                ByteOrder::Big => u32::from_be_bytes(b),
            }
        };
        let directory = |offset: usize| {
            let mut fields = BTreeMap::new();
            let mut previous_tag = None;
            for i in 0..u16_at(offset) as usize {
                let entry = offset + 2 + i * 12;
                let tag = u16_at(entry);
                assert!(previous_tag < Some(tag), "tags must ascend");
                previous_tag = Some(tag);
                let type_code = u16_at(entry + 2);
                let count = u32_at(entry + 4);
                let size = match type_code {
                    1 | 2 | 7 => 1,
                    3 => 2,
                    4 | 13 => 4,
                    5 | 10 => 8,
                    other => panic!("unexpected type {other}"),
                } * count as usize;
                let start = if size <= 4 {
                    entry + 8
                } else {
                    u32_at(entry + 8) as usize
                };
                let bytes = tiff[start..start + size].to_vec();
                fields.insert(
                    tag,
                    RawField {
                        type_code,
                        bytes,
                        order,
                    },
                );
            }
            fields
        };
        let primary = directory(u32_at(4) as usize);
        let linked = |pointer: u16| {
            primary
                .get(&pointer)
                .map(|field| directory(field.u32_at(0) as usize))
                .unwrap_or_default()
        };
        Exif {
            exif: linked(tag::EXIF_IFD),
            gps: linked(tag::GPS_IFD),
            primary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonation_image::Pixels;

    fn image(color_space: ColorSpace) -> OutputImage {
        OutputImage {
            width: 30,
            height: 20,
            color_space,
            pixels: Pixels::U8(vec![0; 30 * 20 * 3]),
        }
    }

    #[test]
    fn fractions_stay_exact_where_cameras_record_them() {
        assert_eq!(
            nearest_fraction(1.0 / 160.0, u32::MAX.into()),
            Some((1, 160))
        );
        assert_eq!(nearest_fraction(2.2, u32::MAX.into()), Some((11, 5)));
        assert_eq!(nearest_fraction(30.0, u32::MAX.into()), Some((30, 1)));
        assert_eq!(nearest_fraction(0.0, u32::MAX.into()), Some((0, 1)));
        assert_eq!(signed_rational(-2.0 / 3.0), Some((-2, 3)));
        assert_eq!(nearest_fraction(f64::NAN, u32::MAX.into()), None);
        assert_eq!(nearest_fraction(-1.0, u32::MAX.into()), None);
        assert_eq!(nearest_fraction(1e12, u32::MAX.into()), None);
        let (n, d) = nearest_fraction(std::f64::consts::PI, u32::MAX.into()).expect("pi");
        assert!((n as f64 / d as f64 - std::f64::consts::PI).abs() < 1e-9);
    }

    #[test]
    fn coordinates_never_have_sixty_seconds() {
        let Some(Value::Rational(dms)) = degrees_minutes_seconds(10.999_999_999) else {
            panic!("degrees");
        };
        assert_eq!(dms, vec![(11, 1), (0, 1), (0, 10_000)]);
        assert_eq!(degrees_minutes_seconds(f64::NAN), None);
    }

    #[test]
    fn empty_metadata_writes_nothing() {
        assert_eq!(
            Directories::new(&CameraInfo::default(), &image(ColorSpace::Srgb)),
            None
        );
    }

    #[test]
    fn stream_round_trips_every_field() {
        let metadata = read_back::sample_metadata();
        let directories =
            Directories::new(&metadata, &image(ColorSpace::DisplayP3)).expect("fields");
        let exif = read_back::read(&directories.tiff_stream());
        assert_eq!(exif.primary[&tag::MAKE].text(), "Canon");
        assert_eq!(exif.primary[&tag::MODEL].text(), "Canon EOS 250D");
        assert_eq!(exif.primary[&tag::ORIENTATION].short(), 1);
        assert!(
            exif.primary[&tag::SOFTWARE]
                .text()
                .starts_with("Photonation ")
        );
        assert_eq!(exif.primary[&tag::ARTIST].text(), "Ada Lovelace");
        assert_eq!(exif.primary[&tag::COPYRIGHT].text(), "© 2026 Ada Lovelace");

        assert_eq!(exif.exif[&tag::EXPOSURE_TIME].rational(0), 1.0 / 4000.0);
        assert_eq!(exif.exif[&tag::F_NUMBER].rational(0), 2.2);
        assert_eq!(exif.exif[&tag::ISO].short(), 200);
        assert_eq!(exif.exif[&tag::EXPOSURE_BIAS].rational(0), -2.0 / 3.0);
        assert_eq!(exif.exif[&tag::FOCAL_LENGTH].rational(0), 50.0);
        assert_eq!(
            exif.exif[&tag::DATE_TIME_ORIGINAL].text(),
            "2026:06:20 18:02:23"
        );
        assert_eq!(exif.exif[&tag::OFFSET_TIME_ORIGINAL].text(), "+01:00");
        assert_eq!(
            exif.exif[&tag::LENS_MODEL].text(),
            "Canon EF 50mm f/1.8 STM"
        );
        assert_eq!(exif.exif[&tag::COLOR_SPACE].short(), 0xFFFF);
        assert_eq!(exif.exif[&tag::PIXEL_X_DIMENSION].u32_at(0), 30);

        let gps = &exif.gps;
        assert_eq!(gps[&tag::GPS_LATITUDE_REF].text(), "N");
        assert_eq!(gps[&tag::GPS_LONGITUDE_REF].text(), "W");
        let degrees = |field: &read_back::RawField| {
            field.rational(0) + field.rational(1) / 60.0 + field.rational(2) / 3600.0
        };
        assert!((degrees(&gps[&tag::GPS_LATITUDE]) - 48.137_154).abs() < 1e-6);
        assert!((degrees(&gps[&tag::GPS_LONGITUDE]) - 11.576_124).abs() < 1e-6);
        assert_eq!(gps[&tag::GPS_ALTITUDE_REF].bytes, [1]);
        assert_eq!(gps[&tag::GPS_ALTITUDE].rational(0), 12.5);
    }

    #[test]
    fn partial_metadata_leaves_the_rest_out() {
        let copyright_only = CameraInfo {
            copyright: Some("© Ada".into()),
            artist: Some("  ".into()),
            ..CameraInfo::default()
        };
        let directories =
            Directories::new(&copyright_only, &image(ColorSpace::Srgb)).expect("fields");
        let exif = read_back::read(&directories.tiff_stream());
        assert_eq!(exif.primary[&tag::COPYRIGHT].text(), "© Ada");
        for absent in [tag::MAKE, tag::MODEL, tag::ARTIST, tag::GPS_IFD] {
            assert!(!exif.primary.contains_key(&absent), "{absent:#x}");
        }
        assert!(!exif.exif.contains_key(&tag::DATE_TIME_ORIGINAL));
        assert_eq!(exif.exif[&tag::COLOR_SPACE].short(), 1);
        assert!(exif.gps.is_empty());
    }

    #[test]
    fn long_text_is_cut_to_fit_a_jpeg_segment() {
        let metadata = CameraInfo {
            artist: Some("é".repeat(5000)),
            copyright: Some("x\0y".into()),
            ..CameraInfo::default()
        };
        let directories = Directories::new(&metadata, &image(ColorSpace::Srgb)).expect("fields");
        let exif = read_back::read(&directories.tiff_stream());
        assert_eq!(exif.primary[&tag::ARTIST].text().len(), MAX_TEXT_BYTES);
        assert_eq!(exif.primary[&tag::COPYRIGHT].text(), "xy");
    }
}
