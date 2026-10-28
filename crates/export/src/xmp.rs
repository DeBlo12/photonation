// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! A minimal XMP packet beside the EXIF. Some readers take the creator,
//! copyright and lens only from XMP (macOS Spotlight and Finder among
//! them), so these, the capture time and the creating tool are written
//! there as well, the way Lightroom does. JPEG carries the packet in an
//! APP1 segment, PNG in an `iTXt` chunk and TIFF in tag 700.

use crate::exif::{SOFTWARE, field_text};
use photonation_image::CameraInfo;

/// Prefix of the JPEG APP1 segment that holds XMP.
pub const JPEG_APP1_PREFIX: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
/// Keyword of the PNG `iTXt` chunk that holds XMP.
pub const PNG_KEYWORD: &str = "XML:com.adobe.xmp";
/// TIFF tag that holds XMP.
pub const TIFF_TAG: u16 = 700;

/// The XMP packet for `metadata`, or `None` when it records nothing.
pub fn packet(metadata: &CameraInfo) -> Option<String> {
    if *metadata == CameraInfo::default() {
        return None;
    }
    let mut attributes = vec![("xmp:CreatorTool", SOFTWARE.to_string())];
    if let Some(date) = metadata
        .date_time
        .as_deref()
        .and_then(|date| iso_8601(date, metadata.time_offset.as_deref()))
    {
        attributes.push(("xmp:CreateDate", date.clone()));
        attributes.push(("photoshop:DateCreated", date));
    }
    if let Some(lens) = metadata.lens.as_deref().and_then(field_text) {
        attributes.push(("aux:Lens", lens.clone()));
        attributes.push(("exifEX:LensModel", lens));
    }
    let mut elements = String::new();
    if let Some(artist) = metadata.artist.as_deref().and_then(field_text) {
        elements.push_str(&format!(
            "   <dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>\n",
            escape(&artist)
        ));
    }
    if let Some(copyright) = metadata.copyright.as_deref().and_then(field_text) {
        elements.push_str(&format!(
            "   <dc:rights><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:rights>\n",
            escape(&copyright)
        ));
    }
    let attributes: String = attributes
        .iter()
        .map(|(name, value)| format!("\n    {name}=\"{}\"", escape(value)))
        .collect();
    Some(format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n \
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  \
         <rdf:Description rdf:about=\"\"\n    \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n    \
         xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n    \
         xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\"\n    \
         xmlns:aux=\"http://ns.adobe.com/exif/1.0/aux/\"\n    \
         xmlns:exifEX=\"http://cipa.jp/exif/1.0/\"{attributes}>\n\
         {elements}  \
         </rdf:Description>\n \
         </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>"
    ))
}

/// EXIF's "YYYY:MM:DD HH:MM:SS" (plus an optional "+HH:MM" offset) as
/// XMP's ISO 8601 date, or `None` if it is not in that shape.
fn iso_8601(date_time: &str, offset: Option<&str>) -> Option<String> {
    let bytes = date_time.as_bytes();
    let shaped = bytes.len() >= 19
        && bytes[..19].iter().enumerate().all(|(i, b)| match i {
            4 | 7 | 13 | 16 => *b == b':',
            10 => *b == b' ',
            _ => b.is_ascii_digit(),
        });
    if !shaped {
        return None;
    }
    let date = date_time[..10].replace(':', "-");
    let time = &date_time[11..19];
    let offset = offset
        .map(str::trim)
        .filter(|o| {
            let b = o.as_bytes();
            b.len() == 6
                && matches!(b[0], b'+' | b'-')
                && b[3] == b':'
                && [1, 2, 4, 5].iter().all(|&i| b[i].is_ascii_digit())
        })
        .unwrap_or("");
    Some(format!("{date}T{time}{offset}"))
}

/// Text as XML character data or attribute value. Control characters XML
/// cannot hold are dropped.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\t' | '\n' | '\r' => escaped.push(c),
            c if c.is_control() => {}
            c => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_holds_creator_copyright_lens_and_date() {
        let packet = packet(&crate::exif::read_back::sample_metadata()).expect("packet");
        assert!(packet.starts_with("<?xpacket begin="));
        assert!(packet.ends_with("<?xpacket end=\"w\"?>"));
        assert!(packet.contains("<rdf:li>Ada Lovelace</rdf:li>"));
        assert!(packet.contains(">© 2026 Ada Lovelace</rdf:li>"));
        assert!(packet.contains("aux:Lens=\"Canon EF 50mm f/1.8 STM\""));
        assert!(packet.contains("xmp:CreateDate=\"2026-06-20T18:02:23+01:00\""));
        assert!(packet.contains("xmp:CreatorTool=\"Photonation "));
    }

    #[test]
    fn text_is_escaped_and_missing_fields_left_out() {
        let metadata = CameraInfo {
            artist: Some("Tom & \"Jerry\" <studio>\u{7}".into()),
            date_time: Some("not a date".into()),
            ..CameraInfo::default()
        };
        let packet = packet(&metadata).expect("packet");
        assert!(packet.contains("<rdf:li>Tom &amp; &quot;Jerry&quot; &lt;studio&gt;</rdf:li>"));
        assert!(!packet.contains("dc:rights>"));
        assert!(!packet.contains("CreateDate"));
        assert!(!packet.contains("aux:Lens"));
        assert_eq!(super::packet(&CameraInfo::default()), None);
    }

    #[test]
    fn dates_become_iso_8601() {
        assert_eq!(
            iso_8601("2026:06:20 18:02:23", Some("+01:00")).as_deref(),
            Some("2026-06-20T18:02:23+01:00")
        );
        assert_eq!(
            iso_8601("2026:06:20 18:02:23", Some("bogus")).as_deref(),
            Some("2026-06-20T18:02:23")
        );
        assert_eq!(iso_8601("2026:06:20", None), None);
    }
}
