// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Lightroom (Adobe Camera Raw) develop settings from `.xmp` sidecars, so
//! photos edited in Lightroom open with their edits. Import only: the XMP
//! file is never written. Documented in docs/EDIT-MODEL.md.

use crate::{GradingWheel, HslAdjustment, PointCurve, Settings, WhiteBalance};
use photonation_image::{CropFrame, NormRect, Orientation};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CAMERA_RAW_NAMESPACE: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";
const TIFF_NAMESPACE: &str = "http://ns.adobe.com/tiff/1.0/";
const EXIF_NAMESPACE: &str = "http://ns.adobe.com/exif/1.0/";
const RDF_NAMESPACE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// Lightroom's sidecar for a raw file: `IMG_0001.CR3` → `IMG_0001.xmp`.
pub fn lightroom_xmp_path(original: &Path) -> PathBuf {
    original.with_extension("xmp")
}

/// The develop settings Lightroom saved for a photo, as far as Photonation
/// has equivalents, and the Lightroom features it had to leave out.
#[derive(Clone, Debug, PartialEq)]
pub struct LightroomImport {
    pub settings: Settings,
    /// User-facing names of edits that were not imported, e.g. "masks".
    pub not_imported: Vec<&'static str>,
}

/// The top-level properties of an XMP packet: `crs:`, `tiff:` and `exif:`
/// values, keyed by namespace and local name.
struct Properties<'input> {
    values: HashMap<(&'input str, &'input str), &'input str>,
    /// `rdf:Seq` lists (tone curves), keyed like `values`.
    lists: HashMap<(&'input str, &'input str), Vec<&'input str>>,
    /// Properties holding nested structures (masks, spot removal…).
    structures: Vec<(&'input str, &'input str)>,
}

impl<'input> Properties<'input> {
    /// Reads the properties written as attributes of, or as elements under,
    /// each `rdf:Description`. Nested structures stay unread apart from
    /// their names, so a mask's local `crs:Exposure2012` is never taken
    /// for the photo's.
    fn read(document: &'input roxmltree::Document<'input>) -> Properties<'input> {
        let mut properties = Properties {
            values: HashMap::new(),
            lists: HashMap::new(),
            structures: Vec::new(),
        };
        let descriptions = document
            .descendants()
            .filter(|node| node.has_tag_name((RDF_NAMESPACE, "Description")))
            .filter(|node| {
                node.parent()
                    .is_some_and(|parent| parent.has_tag_name((RDF_NAMESPACE, "RDF")))
            });
        for description in descriptions {
            for attribute in description.attributes() {
                if let Some(namespace) = attribute.namespace() {
                    properties
                        .values
                        .insert((namespace, attribute.name()), attribute.value());
                }
            }
            for property in description.children().filter(|node| node.is_element()) {
                let key = (
                    property.tag_name().namespace().unwrap_or_default(),
                    property.tag_name().name(),
                );
                let list = property
                    .children()
                    .find(|node| node.has_tag_name((RDF_NAMESPACE, "Seq")));
                if let Some(list) = list {
                    let items = list
                        .children()
                        .filter(|node| node.has_tag_name((RDF_NAMESPACE, "li")))
                        .filter_map(|item| item.text())
                        .collect();
                    properties.lists.insert(key, items);
                } else if property.children().any(|node| node.is_element())
                    || property.attributes().len() > 0
                {
                    properties.structures.push(key);
                } else if let Some(text) = property.text() {
                    properties.values.insert(key, text.trim());
                }
            }
        }
        properties
    }

    fn text(&self, namespace: &str, name: &str) -> Option<&'input str> {
        self.values.get(&(namespace, name)).copied()
    }

    fn crs(&self, name: &str) -> Option<&'input str> {
        self.text(CAMERA_RAW_NAMESPACE, name)
    }

    /// A finite number (Lightroom writes signs: "+0.50").
    fn number(&self, name: &str) -> Option<f64> {
        self.crs(name)?
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
    }

    fn flag(&self, name: &str) -> bool {
        self.crs(name)
            .is_some_and(|value| value.eq_ignore_ascii_case("true") || value == "1")
    }

    /// Sets `target` when the property is present.
    fn read_into(&self, name: &str, target: &mut f32) {
        if let Some(value) = self.number(name) {
            *target = value as f32;
        }
    }

    fn has_structure(&self, name: &str) -> bool {
        let key = (CAMERA_RAW_NAMESPACE, name);
        self.structures.contains(&key)
            || self.lists.get(&key).is_some_and(|items| !items.is_empty())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct XmpError(String);

/// Parses the text of a Lightroom `.xmp` sidecar. `Ok(None)` when it holds
/// no develop settings (Lightroom also writes XMP for ratings and keywords).
pub fn import_lightroom_xmp(text: &str) -> Result<Option<LightroomImport>, XmpError> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        ..roxmltree::ParsingOptions::default()
    };
    let document = roxmltree::Document::parse_with_options(text, options)
        .map_err(|error| XmpError(error.to_string()))?;
    let properties = Properties::read(&document);
    if properties.crs("ProcessVersion").is_none() && !properties.flag("HasSettings") {
        return Ok(None);
    }
    let mut settings = Settings::default();
    basic(&properties, &mut settings);
    tone_curve(&properties, &mut settings);
    color_mixer(&properties, &mut settings);
    color_grading(&properties, &mut settings);
    crop(&properties, &mut settings);
    Ok(Some(LightroomImport {
        settings: settings.sanitized(),
        not_imported: not_imported(&properties),
    }))
}

fn basic(properties: &Properties, settings: &mut Settings) {
    let white_balance = properties.crs("WhiteBalance").unwrap_or("As Shot");
    if let (false, Some(temperature), Some(tint)) = (
        white_balance == "As Shot",
        properties.number("Temperature"),
        properties.number("Tint"),
    ) {
        settings.white_balance = WhiteBalance::Custom { temperature, tint };
    }
    // Process 2012 and later; older processes used other scales.
    for (name, target) in [
        ("Exposure2012", &mut settings.exposure),
        ("Contrast2012", &mut settings.contrast),
        ("Highlights2012", &mut settings.highlights),
        ("Shadows2012", &mut settings.shadows),
        ("Whites2012", &mut settings.whites),
        ("Blacks2012", &mut settings.blacks),
        ("Texture", &mut settings.texture),
        ("Clarity2012", &mut settings.clarity),
        ("Dehaze", &mut settings.dehaze),
        ("Vibrance", &mut settings.vibrance),
        ("Saturation", &mut settings.saturation),
        ("Sharpness", &mut settings.sharpening.amount),
        ("SharpenRadius", &mut settings.sharpening.radius),
    ] {
        properties.read_into(name, target);
    }
}

fn tone_curve(properties: &Properties, settings: &mut Settings) {
    let parametric = &mut settings.tone_curve.parametric;
    for (name, target) in [
        ("ParametricShadows", &mut parametric.shadows),
        ("ParametricDarks", &mut parametric.darks),
        ("ParametricLights", &mut parametric.lights),
        ("ParametricHighlights", &mut parametric.highlights),
        ("ParametricShadowSplit", &mut parametric.shadow_split),
        ("ParametricMidtoneSplit", &mut parametric.midtone_split),
        ("ParametricHighlightSplit", &mut parametric.highlight_split),
    ] {
        properties.read_into(name, target);
    }
    let curves = &mut settings.tone_curve;
    for (name, target) in [
        ("ToneCurvePV2012", &mut curves.master),
        ("ToneCurvePV2012Red", &mut curves.red),
        ("ToneCurvePV2012Green", &mut curves.green),
        ("ToneCurvePV2012Blue", &mut curves.blue),
    ] {
        if let Some(curve) = properties
            .lists
            .get(&(CAMERA_RAW_NAMESPACE, name))
            .and_then(|points| point_curve(points))
        {
            *target = curve;
        }
    }
}

/// Lightroom curve points are "input, output" on 0..255.
fn point_curve(points: &[&str]) -> Option<PointCurve> {
    let points = points
        .iter()
        .map(|point| {
            let (input, output) = point.split_once(',')?;
            let coordinate = |text: &str| text.trim().parse::<f32>().ok().map(|v| v / 255.0);
            Some([coordinate(input)?, coordinate(output)?])
        })
        .collect::<Option<Vec<_>>>()?;
    (points.len() >= 2).then(|| PointCurve(points).sanitized())
}

fn color_mixer(properties: &Properties, settings: &mut Settings) {
    const BANDS: [&str; 8] = [
        "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta",
    ];
    for (band, adjustment) in BANDS.iter().zip(settings.color_mixer.bands_mut()) {
        let HslAdjustment {
            hue,
            saturation,
            luminance,
        } = adjustment;
        properties.read_into(&format!("HueAdjustment{band}"), hue);
        properties.read_into(&format!("SaturationAdjustment{band}"), saturation);
        properties.read_into(&format!("LuminanceAdjustment{band}"), luminance);
    }
}

fn color_grading(properties: &Properties, settings: &mut Settings) {
    let grading = &mut settings.color_grading;
    // Lightroom keeps the shadow and highlight wheels under their
    // split-toning names from before Color Grading.
    let wheels: [(&mut GradingWheel, [&str; 3]); 4] = [
        (
            &mut grading.shadows,
            [
                "SplitToningShadowHue",
                "SplitToningShadowSaturation",
                "ColorGradeShadowLum",
            ],
        ),
        (
            &mut grading.midtones,
            [
                "ColorGradeMidtoneHue",
                "ColorGradeMidtoneSat",
                "ColorGradeMidtoneLum",
            ],
        ),
        (
            &mut grading.highlights,
            [
                "SplitToningHighlightHue",
                "SplitToningHighlightSaturation",
                "ColorGradeHighlightLum",
            ],
        ),
        (
            &mut grading.global,
            [
                "ColorGradeGlobalHue",
                "ColorGradeGlobalSat",
                "ColorGradeGlobalLum",
            ],
        ),
    ];
    for (wheel, [hue, saturation, luminance]) in wheels {
        properties.read_into(hue, &mut wheel.hue);
        properties.read_into(saturation, &mut wheel.saturation);
        properties.read_into(luminance, &mut wheel.luminance);
    }
    properties.read_into("ColorGradeBlending", &mut grading.blending);
    properties.read_into("SplitToningBalance", &mut grading.balance);
}

/// Lightroom stores the crop frame's corners and angle on the stored (not
/// yet rotated for display) image; Photonation stores them as displayed.
fn crop(properties: &Properties, settings: &mut Settings) {
    if !properties.flag("HasCrop") {
        return;
    }
    let edge = |name: &str, default: f64| properties.number(name).unwrap_or(default);
    let corners = NormRect {
        left: edge("CropLeft", 0.0),
        top: edge("CropTop", 0.0),
        right: edge("CropRight", 1.0),
        bottom: edge("CropBottom", 1.0),
    };
    let orientation = properties
        .text(TIFF_NAMESPACE, "Orientation")
        .and_then(|value| value.trim().parse().ok())
        .map_or(Orientation::Normal, Orientation::from_exif);
    let displayed = CropFrame::new(stored_size(properties), corners, edge("CropAngle", 0.0))
        .oriented(orientation);
    settings.crop_angle = displayed.angle() as f32;
    settings.crop = Some(displayed.corners());
}

/// The stored image's pixel size: only its aspect ratio matters, to rotate
/// the frame. Falls back to 3:2, the most common raw format.
fn stored_size(properties: &Properties) -> [f64; 2] {
    let dimension = |namespace, name| {
        properties
            .text(namespace, name)
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| *value >= 1.0)
    };
    let pairs = [
        (
            dimension(TIFF_NAMESPACE, "ImageWidth"),
            dimension(TIFF_NAMESPACE, "ImageLength"),
        ),
        (
            dimension(EXIF_NAMESPACE, "PixelXDimension"),
            dimension(EXIF_NAMESPACE, "PixelYDimension"),
        ),
    ];
    pairs
        .into_iter()
        .find_map(|pair| match pair {
            (Some(width), Some(height)) => Some([width, height]),
            _ => None,
        })
        .unwrap_or([3.0, 2.0])
}

/// Lightroom edits Photonation has no equivalent for yet.
fn not_imported(properties: &Properties) -> Vec<&'static str> {
    let nonzero = |name: &str| properties.number(name).is_some_and(|value| value != 0.0);
    let checks: [(&'static str, bool); 7] = [
        (
            "masks",
            [
                "MaskGroupBasedCorrections",
                "GradientBasedCorrections",
                "CircularGradientBasedCorrections",
                "PaintBasedCorrections",
            ]
            .iter()
            .any(|name| properties.has_structure(name)),
        ),
        ("spot removal", properties.has_structure("RetouchAreas")),
        ("noise reduction", nonzero("LuminanceSmoothing")),
        (
            "lens corrections",
            properties.flag("LensProfileEnable")
                || nonzero("LensManualDistortionAmount")
                || nonzero("VignetteAmount")
                || nonzero("AutoLateralCA"),
        ),
        (
            "upright and transform",
            properties
                .crs("PerspectiveUpright")
                .is_some_and(|value| value != "0")
                || [
                    "PerspectiveVertical",
                    "PerspectiveHorizontal",
                    "PerspectiveRotate",
                    "PerspectiveAspect",
                    "PerspectiveX",
                    "PerspectiveY",
                ]
                .iter()
                .any(|name| nonzero(name)),
        ),
        (
            "effects",
            nonzero("PostCropVignetteAmount") || nonzero("GrainAmount"),
        ),
        ("black & white", properties.flag("ConvertToGrayscale")),
    ];
    checks
        .into_iter()
        .filter(|(_, present)| *present)
        .map(|(name, _)| name)
        .collect()
}

#[cfg(test)]
mod tests;
