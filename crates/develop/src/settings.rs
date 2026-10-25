// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use photonation_image::NormRect;
use serde::{Deserialize, Serialize};

/// The rendering process a set of settings is interpreted by.
///
/// A change that alters how existing settings render ships as a new
/// version; old versions stay frozen so earlier edits keep their look.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub enum ProcessVersion {
    #[default]
    V1,
}

impl ProcessVersion {
    pub const LATEST: ProcessVersion = ProcessVersion::V1;

    pub fn number(self) -> u32 {
        match self {
            ProcessVersion::V1 => 1,
        }
    }
}

impl From<ProcessVersion> for u32 {
    fn from(p: ProcessVersion) -> u32 {
        p.number()
    }
}

impl TryFrom<u32> for ProcessVersion {
    type Error = String;

    fn try_from(v: u32) -> Result<Self, Self::Error> {
        match v {
            1 => Ok(ProcessVersion::V1),
            _ => Err(format!(
                "unsupported process version {v}; this build supports up to {}",
                Self::LATEST.number()
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum WhiteBalance {
    /// The camera's recorded white balance.
    #[default]
    AsShot,
    /// Kelvin and tint (positive = magenta).
    Custom { temperature: f64, tint: f64 },
}

impl WhiteBalance {
    /// Lightroom-style presets (name, kelvin, tint).
    pub const PRESETS: [(&'static str, f64, f64); 6] = [
        ("Daylight", 5500.0, 10.0),
        ("Cloudy", 6500.0, 10.0),
        ("Shade", 7500.0, 10.0),
        ("Tungsten", 2850.0, 0.0),
        ("Fluorescent", 3800.0, 21.0),
        ("Flash", 5500.0, 0.0),
    ];
}

/// Region sliders of the parametric tone curve (−100..100) and the
/// boundaries between regions (0..100, in curve input).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParametricCurve {
    pub highlights: f32,
    pub lights: f32,
    pub darks: f32,
    pub shadows: f32,
    pub shadow_split: f32,
    pub midtone_split: f32,
    pub highlight_split: f32,
}

impl Default for ParametricCurve {
    fn default() -> Self {
        ParametricCurve {
            highlights: 0.0,
            lights: 0.0,
            darks: 0.0,
            shadows: 0.0,
            shadow_split: 25.0,
            midtone_split: 50.0,
            highlight_split: 75.0,
        }
    }
}

/// A curve through control points `[input, output]`, both 0..1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PointCurve(pub Vec<[f32; 2]>);

impl Default for PointCurve {
    fn default() -> Self {
        PointCurve(vec![[0.0, 0.0], [1.0, 1.0]])
    }
}

impl PointCurve {
    pub fn is_identity(&self) -> bool {
        self.0.iter().all(|[x, y]| (x - y).abs() < 1e-6)
    }

    /// Sorted by input, clamped to 0..1, duplicates removed, at least two points.
    pub fn sanitized(&self) -> PointCurve {
        let mut points: Vec<[f32; 2]> = self
            .0
            .iter()
            .filter(|p| p[0].is_finite() && p[1].is_finite())
            .map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)])
            .collect();
        points.sort_by(|a, b| a[0].total_cmp(&b[0]));
        points.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-4);
        if points.len() < 2 {
            PointCurve::default()
        } else {
            PointCurve(points)
        }
    }
}

/// Lightroom's tone curve panel: parametric regions plus point curves for
/// the composite and for each channel.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneCurve {
    pub parametric: ParametricCurve,
    pub master: PointCurve,
    pub red: PointCurve,
    pub green: PointCurve,
    pub blue: PointCurve,
}

/// Hue shift, saturation and luminance for one colour band (−100..100).
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HslAdjustment {
    pub hue: f32,
    pub saturation: f32,
    pub luminance: f32,
}

/// The colour mixer (HSL) with Lightroom's eight bands.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorMixer {
    pub red: HslAdjustment,
    pub orange: HslAdjustment,
    pub yellow: HslAdjustment,
    pub green: HslAdjustment,
    pub aqua: HslAdjustment,
    pub blue: HslAdjustment,
    pub purple: HslAdjustment,
    pub magenta: HslAdjustment,
}

impl ColorMixer {
    pub const BAND_NAMES: [&'static str; 8] = [
        "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta",
    ];

    pub fn bands(&self) -> [HslAdjustment; 8] {
        [
            self.red,
            self.orange,
            self.yellow,
            self.green,
            self.aqua,
            self.blue,
            self.purple,
            self.magenta,
        ]
    }

    pub fn bands_mut(&mut self) -> [&mut HslAdjustment; 8] {
        [
            &mut self.red,
            &mut self.orange,
            &mut self.yellow,
            &mut self.green,
            &mut self.aqua,
            &mut self.blue,
            &mut self.purple,
            &mut self.magenta,
        ]
    }

    pub fn is_neutral(&self) -> bool {
        self.bands().iter().all(|b| *b == HslAdjustment::default())
    }
}

/// One colour-grading wheel: tint hue (degrees), its strength and a
/// luminance offset.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GradingWheel {
    /// 0..360.
    pub hue: f32,
    /// 0..100.
    pub saturation: f32,
    /// −100..100.
    pub luminance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorGrading {
    pub shadows: GradingWheel,
    pub midtones: GradingWheel,
    pub highlights: GradingWheel,
    pub global: GradingWheel,
    /// Overlap between the tonal ranges, 0..100.
    pub blending: f32,
    /// Moves the shadow/highlight split, −100..100.
    pub balance: f32,
}

impl Default for ColorGrading {
    fn default() -> Self {
        ColorGrading {
            shadows: GradingWheel::default(),
            midtones: GradingWheel::default(),
            highlights: GradingWheel::default(),
            global: GradingWheel::default(),
            blending: 50.0,
            balance: 0.0,
        }
    }
}

impl ColorGrading {
    pub fn is_neutral(&self) -> bool {
        [self.shadows, self.midtones, self.highlights, self.global]
            .iter()
            .all(|w| w.saturation == 0.0 && w.luminance == 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sharpening {
    /// 0..150.
    pub amount: f32,
    /// Gaussian sigma in output pixels, 0.5..3.
    pub radius: f32,
}

impl Default for Sharpening {
    fn default() -> Self {
        Sharpening {
            amount: 40.0,
            radius: 1.0,
        }
    }
}

/// Develop settings. Slider values use the familiar −100..100 scale unless
/// noted; missing fields deserialize to their defaults so sidecars written
/// by older builds stay readable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub process: ProcessVersion,
    pub white_balance: WhiteBalance,
    /// Stops, −5..5.
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    /// Fine local contrast.
    pub texture: f32,
    /// Mid-scale local contrast.
    pub clarity: f32,
    /// Removes (positive) or adds (negative) atmospheric haze.
    pub dehaze: f32,
    pub vibrance: f32,
    pub saturation: f32,
    pub tone_curve: ToneCurve,
    pub color_mixer: ColorMixer,
    pub color_grading: ColorGrading,
    pub sharpening: Sharpening,
    /// Crop on the displayed (oriented) image, inside the camera's default crop.
    pub crop: Option<NormRect>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            process: ProcessVersion::LATEST,
            white_balance: WhiteBalance::AsShot,
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            tone_curve: ToneCurve::default(),
            color_mixer: ColorMixer::default(),
            color_grading: ColorGrading::default(),
            sharpening: Sharpening::default(),
            crop: None,
        }
    }
}

impl Settings {
    /// Clamps every value into its control's range and repairs non-finite input.
    pub fn sanitized(&self) -> Settings {
        let mut s = self.clone();
        for c in crate::all_controls() {
            let v = c.get_mut(&mut s);
            *v = c.clamp(*v);
        }
        if let WhiteBalance::Custom { temperature, tint } = &mut s.white_balance {
            *temperature = if temperature.is_finite() {
                temperature.clamp(2000.0, 25000.0)
            } else {
                5500.0
            };
            *tint = if tint.is_finite() {
                tint.clamp(-150.0, 150.0)
            } else {
                0.0
            };
        }
        s.color_grading = {
            let mut grading = s.color_grading;
            for wheel in [
                &mut grading.shadows,
                &mut grading.midtones,
                &mut grading.highlights,
                &mut grading.global,
            ] {
                wheel.hue = if wheel.hue.is_finite() {
                    wheel.hue.rem_euclid(360.0)
                } else {
                    0.0
                };
            }
            grading
        };
        let curves = &mut s.tone_curve;
        for curve in [
            &mut curves.master,
            &mut curves.red,
            &mut curves.green,
            &mut curves.blue,
        ] {
            *curve = curve.sanitized();
        }
        let p = &mut curves.parametric;
        p.shadow_split = p.shadow_split.clamp(5.0, 45.0);
        p.midtone_split = p.midtone_split.clamp(p.shadow_split + 5.0, 90.0);
        p.highlight_split = p.highlight_split.clamp(p.midtone_split + 5.0, 95.0);
        s.crop = s.crop.map(NormRect::clamped);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let s = Settings {
            exposure: 0.7,
            white_balance: WhiteBalance::Custom {
                temperature: 5200.0,
                tint: 8.0,
            },
            crop: Some(NormRect {
                left: 0.1,
                top: 0.0,
                right: 0.9,
                bottom: 1.0,
            }),
            ..Settings::default()
        };
        let back: Settings =
            serde_json::from_str(&serde_json::to_string(&s).expect("serialize")).expect("parse");
        assert_eq!(s, back);
    }

    #[test]
    fn missing_fields_default() {
        let s: Settings = serde_json::from_str(r#"{"exposure": 1.5}"#).expect("parse");
        assert_eq!(s.exposure, 1.5);
        assert_eq!(s.sharpening, Sharpening::default());
        assert_eq!(s.process, ProcessVersion::V1);
    }

    #[test]
    fn future_process_is_rejected() {
        let err = serde_json::from_str::<Settings>(r#"{"process": 99}"#).expect_err("must fail");
        assert!(err.to_string().contains("unsupported process version"));
    }

    #[test]
    fn point_curve_sanitize_sorts_and_dedups() {
        let c = PointCurve(vec![[1.0, 1.0], [0.5, 0.6], [0.5, 0.7], [-1.0, 0.0]]).sanitized();
        assert_eq!(c.0, vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]]);
        assert!(PointCurve::default().is_identity());
    }

    #[test]
    fn sanitize_clamps() {
        let s = Settings {
            exposure: 42.0,
            contrast: f32::NAN,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s.exposure, 5.0);
        assert_eq!(s.contrast, 0.0);
    }
}
