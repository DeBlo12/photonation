// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use super::*;

/// The shape of a Lightroom Classic 14 sidecar for a portrait (EXIF 8)
/// Canon raw, trimmed: settings as attributes, curves as `rdf:Seq`, one
/// mask whose local values must not leak into the global settings.
const PORTRAIT_SIDECAR: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:tiff="http://ns.adobe.com/tiff/1.0/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   tiff:Orientation="8"
   tiff:ImageWidth="6000"
   tiff:ImageLength="4000"
   crs:Version="18.4"
   crs:ProcessVersion="15.4"
   crs:WhiteBalance="Custom"
   crs:Temperature="5150"
   crs:Tint="+12"
   crs:Exposure2012="-0.54"
   crs:Contrast2012="+8"
   crs:Shadows2012="+28"
   crs:Blacks2012="+61"
   crs:Texture="+5"
   crs:Dehaze="+3"
   crs:ParametricLights="-10"
   crs:ParametricMidtoneSplit="45"
   crs:Sharpness="40"
   crs:SharpenRadius="+1.0"
   crs:LuminanceSmoothing="20"
   crs:SaturationAdjustmentRed="+50"
   crs:SaturationAdjustmentBlue="-42"
   crs:LuminanceAdjustmentMagenta="-11"
   crs:HueAdjustmentAqua="+7"
   crs:SplitToningShadowHue="220"
   crs:SplitToningShadowSaturation="18"
   crs:SplitToningHighlightHue="45"
   crs:SplitToningHighlightSaturation="9"
   crs:SplitToningBalance="-20"
   crs:ColorGradeMidtoneHue="30"
   crs:ColorGradeMidtoneSat="4"
   crs:ColorGradeShadowLum="-6"
   crs:ColorGradeBlending="70"
   crs:LensProfileEnable="0"
   crs:HasSettings="True"
   crs:CropTop="0.09873"
   crs:CropLeft="0.052519"
   crs:CropBottom="0.90127"
   crs:CropRight="0.947481"
   crs:CropAngle="-2.818159"
   crs:HasCrop="True">
   <crs:ToneCurvePV2012>
    <rdf:Seq>
     <rdf:li>0, 0</rdf:li>
     <rdf:li>92, 112</rdf:li>
     <rdf:li>255, 153</rdf:li>
    </rdf:Seq>
   </crs:ToneCurvePV2012>
   <crs:ToneCurvePV2012Red>
    <rdf:Seq>
     <rdf:li>0, 0</rdf:li>
     <rdf:li>255, 255</rdf:li>
    </rdf:Seq>
   </crs:ToneCurvePV2012Red>
   <crs:MaskGroupBasedCorrections>
    <rdf:Seq>
     <rdf:li>
      <rdf:Description
       crs:What="Correction"
       crs:CorrectionAmount="1"
       crs:LocalExposure2012="0.179911"
       crs:Exposure2012="+3.00"/>
     </rdf:li>
    </rdf:Seq>
   </crs:MaskGroupBasedCorrections>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
"#;

fn import(text: &str) -> LightroomImport {
    import_lightroom_xmp(text)
        .expect("parses")
        .expect("has settings")
}

#[test]
fn global_settings_are_imported() {
    let imported = import(PORTRAIT_SIDECAR);
    let s = &imported.settings;
    assert_eq!(
        s.white_balance,
        WhiteBalance::Custom {
            temperature: 5150.0,
            tint: 12.0
        }
    );
    assert_eq!(s.exposure, -0.54);
    assert_eq!((s.contrast, s.shadows, s.blacks), (8.0, 28.0, 61.0));
    assert_eq!((s.texture, s.dehaze), (5.0, 3.0));
    assert_eq!(s.highlights, 0.0, "missing values keep their defaults");
    assert_eq!(s.sharpening.amount, 40.0);
    assert_eq!(s.tone_curve.parametric.lights, -10.0);
    assert_eq!(s.tone_curve.parametric.midtone_split, 45.0);
}

#[test]
fn mask_values_stay_out_of_the_global_settings() {
    // The mask's own crs:Exposure2012 is +3; the photo's is −0.54.
    assert_eq!(import(PORTRAIT_SIDECAR).settings.exposure, -0.54);
}

#[test]
fn curves_mixer_and_grading_are_imported() {
    let s = import(PORTRAIT_SIDECAR).settings;
    let master = &s.tone_curve.master.0;
    assert_eq!(master.len(), 3);
    assert!((master[1][0] - 92.0 / 255.0).abs() < 1e-6);
    assert!((master[2][1] - 153.0 / 255.0).abs() < 1e-6);
    assert!(s.tone_curve.red.is_identity());
    assert_eq!(s.color_mixer.red.saturation, 50.0);
    assert_eq!(s.color_mixer.blue.saturation, -42.0);
    assert_eq!(s.color_mixer.magenta.luminance, -11.0);
    assert_eq!(s.color_mixer.aqua.hue, 7.0);
    let grading = s.color_grading;
    assert_eq!(
        grading.shadows,
        GradingWheel {
            hue: 220.0,
            saturation: 18.0,
            luminance: -6.0
        }
    );
    assert_eq!(
        (grading.highlights.hue, grading.highlights.saturation),
        (45.0, 9.0)
    );
    assert_eq!(
        (grading.midtones.hue, grading.midtones.saturation),
        (30.0, 4.0)
    );
    assert_eq!((grading.blending, grading.balance), (70.0, -20.0));
}

#[test]
fn crops_are_turned_to_the_displayed_image() {
    let s = import(PORTRAIT_SIDECAR).settings;
    // Rotating for display (EXIF 8) keeps the turn's direction.
    assert!((s.crop_angle - -2.818159).abs() < 1e-5);
    // Lightroom exported this crop at 3470 × 5205.
    let frame = CropFrame::new([4000.0, 6000.0], s.crop.expect("crop"), s.crop_angle as f64);
    let [width, height] = frame.size();
    assert!((width - 3470.0).abs() < 1.0 && (height - 5205.0).abs() < 1.0);
}

#[test]
fn unsupported_edits_are_reported() {
    assert_eq!(
        import(PORTRAIT_SIDECAR).not_imported,
        vec!["masks", "noise reduction"]
    );
}

#[test]
fn as_shot_white_balance_ignores_the_temperature() {
    let text = PORTRAIT_SIDECAR.replace(
        r#"crs:WhiteBalance="Custom""#,
        r#"crs:WhiteBalance="As Shot""#,
    );
    assert_eq!(import(&text).settings.white_balance, WhiteBalance::AsShot);
}

#[test]
fn element_properties_are_read_too() {
    // XMP allows every property as an element instead of an attribute.
    let text = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
     <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
      <rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/">
       <crs:ProcessVersion>15.4</crs:ProcessVersion>
       <crs:Exposure2012>+1.25</crs:Exposure2012>
       <crs:HasCrop>False</crs:HasCrop>
      </rdf:Description>
     </rdf:RDF>
    </x:xmpmeta>"#;
    let imported = import(text);
    assert_eq!(imported.settings.exposure, 1.25);
    assert_eq!(imported.settings.crop, None);
    assert!(imported.not_imported.is_empty());
}

#[test]
fn metadata_only_sidecars_have_no_settings() {
    let text = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
     <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
      <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="4"/>
     </rdf:RDF>
    </x:xmpmeta>"#;
    assert_eq!(import_lightroom_xmp(text).expect("parses"), None);
}

/// A metadata-only sidecar with `rating` and `label` as attributes.
fn rated_sidecar(rating: &str, label: &str) -> String {
    format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
     <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
      <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/"
       xmp:Rating="{rating}" xmp:Label="{label}"/>
     </rdf:RDF>
    </x:xmpmeta>"#
    )
}

#[test]
fn ratings_and_labels_are_imported() {
    let info = import_lightroom_library(&rated_sidecar("4", "Green")).expect("parses");
    assert_eq!(
        info,
        LibraryInfo {
            rating: 4,
            flag: Flag::None,
            label: Some(ColorLabel::Green),
        }
    );
    let rejected = import_lightroom_library(&rated_sidecar("-1", "")).expect("parses");
    assert_eq!(rejected.flag, Flag::Reject);
    assert_eq!(rejected.rating, 0);
    assert_eq!(rejected.label, None);
    let wild = import_lightroom_library(&rated_sidecar("12", "Orange")).expect("parses");
    assert_eq!((wild.rating, wild.label), (MAX_RATING, None));
    // Develop sidecars without a rating have none.
    let info = import_lightroom_library(PORTRAIT_SIDECAR).expect("parses");
    assert!(info.is_empty());
}

#[test]
fn out_of_range_and_broken_values_are_repaired() {
    let text = PORTRAIT_SIDECAR
        .replace(r#"crs:Exposure2012="-0.54""#, r#"crs:Exposure2012="+40""#)
        .replace(r#"crs:Contrast2012="+8""#, r#"crs:Contrast2012="lots""#);
    let s = import(&text).settings;
    assert_eq!(s.exposure, 5.0);
    assert_eq!(s.contrast, 0.0);
}

#[test]
fn malformed_xml_is_an_error() {
    assert!(import_lightroom_xmp("<x:xmpmeta><rdf:RDF>").is_err());
    // No entity expansion: a DTD is refused outright.
    let billion_laughs = r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "aaaa">]><x>&a;</x>"#;
    assert!(import_lightroom_xmp(billion_laughs).is_err());
}

#[test]
fn sidecar_names_follow_lightroom() {
    assert_eq!(
        lightroom_xmp_path(Path::new("/photos/IMG_0001.CR3")),
        PathBuf::from("/photos/IMG_0001.xmp")
    );
}
