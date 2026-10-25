// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Masks: shapes that select part of the photo, each with the local
//! adjustments applied there. See docs/EDIT-MODEL.md.

use crate::LOCAL_CONTROLS;
use serde::{Deserialize, Serialize};

/// Most masks one photo keeps. Every mask costs time at every pixel, so the
/// cap also stops a hostile sidecar from making renders arbitrarily slow.
pub const MAX_MASKS: usize = 32;
/// Longest mask name kept, in characters.
pub const MAX_MASK_NAME_CHARS: usize = 64;

/// The coordinate frame masks are drawn in.
///
/// Positions are normalized to the photo: 0..1 across its width and height,
/// in display orientation, on the camera's default crop but before the
/// user's crop, so a mask stays on the same content when the crop changes.
/// Radii are in units of the photo's long edge, so a circle stays round
/// whatever the aspect ratio. "Metric" points are positions scaled the same
/// way (`x · width / long edge`, `y · height / long edge`), where distances
/// can be measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskSpace {
    /// Width and height divided by the long edge.
    scale: [f64; 2],
}

impl MaskSpace {
    /// The mask space of a photo `width × height` (display orientation;
    /// any unit, only the aspect ratio matters).
    pub fn new(width: f64, height: f64) -> MaskSpace {
        let long_edge = width.max(height);
        if !(long_edge.is_finite() && long_edge > 0.0) {
            return MaskSpace { scale: [1.0, 1.0] };
        }
        MaskSpace {
            scale: [width / long_edge, height / long_edge],
        }
    }

    /// Width and height over the long edge (the larger one is 1).
    pub fn scale(self) -> [f64; 2] {
        self.scale
    }

    pub fn to_metric(self, position: [f64; 2]) -> [f64; 2] {
        [position[0] * self.scale[0], position[1] * self.scale[1]]
    }

    pub fn from_metric(self, metric: [f64; 2]) -> [f64; 2] {
        [metric[0] / self.scale[0], metric[1] / self.scale[1]]
    }
}

/// Lightroom's linear gradient: full effect up to the line through `start`
/// (perpendicular to start → end), fading smoothly to none at the parallel
/// line through `end`. Both are positions in [`MaskSpace`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LinearGradient {
    pub start: [f64; 2],
    pub end: [f64; 2],
}

impl Default for LinearGradient {
    /// From the top towards the middle: a typical sky gradient.
    fn default() -> Self {
        LinearGradient {
            start: [0.5, 0.15],
            end: [0.5, 0.5],
        }
    }
}

/// An ellipse, full effect inside, fading out towards its edge.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RadialGradient {
    /// Position in [`MaskSpace`].
    pub center: [f64; 2],
    /// Half-axes in units of the long edge, before rotation.
    pub radius: [f64; 2],
    /// Clockwise, degrees.
    pub rotation: f32,
    /// How much of the radius fades out, 0..100 (0 = hard edge).
    pub feather: f32,
}

impl Default for RadialGradient {
    fn default() -> Self {
        RadialGradient {
            center: [0.5, 0.5],
            radius: [0.2, 0.2],
            rotation: 0.0,
            feather: 50.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MaskShape {
    Linear(LinearGradient),
    Radial(RadialGradient),
}

impl Default for MaskShape {
    fn default() -> Self {
        MaskShape::Radial(RadialGradient::default())
    }
}

/// What a mask changes where it applies. Values use the global sliders'
/// scales (−100..100, stops for exposure); temperature and tint shift
/// relative to the photo's white balance.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalAdjustments {
    /// Warmer (positive) or cooler.
    pub temperature: f32,
    /// Towards magenta (positive) or green.
    pub tint: f32,
    /// Stops, −4..4.
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub dehaze: f32,
    pub saturation: f32,
}

impl LocalAdjustments {
    pub fn is_neutral(&self) -> bool {
        *self == LocalAdjustments::default()
    }
}

/// A local adjustment: a shape selecting part of the photo, optionally
/// inverted to select everything else, and what to change there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mask {
    pub name: String,
    /// Disabled masks are kept but not rendered.
    pub enabled: bool,
    pub shape: MaskShape,
    /// Apply the adjustments outside the shape instead of inside.
    pub invert: bool,
    pub adjustments: LocalAdjustments,
}

impl Default for Mask {
    fn default() -> Self {
        Mask::new("", MaskShape::default())
    }
}

impl Mask {
    pub fn new(name: impl Into<String>, shape: MaskShape) -> Mask {
        Mask {
            name: name.into(),
            enabled: true,
            shape,
            invert: false,
            adjustments: LocalAdjustments::default(),
        }
    }

    /// Whether rendering this mask changes anything.
    pub fn is_active(&self) -> bool {
        self.enabled && !self.adjustments.is_neutral()
    }

    /// Clamps every value into range and repairs non-finite input.
    pub fn sanitized(&self) -> Mask {
        let mut mask = self.clone();
        if let Some((cut, _)) = mask.name.char_indices().nth(MAX_MASK_NAME_CHARS) {
            mask.name.truncate(cut);
        }
        for control in LOCAL_CONTROLS {
            let value = control.get_mut(&mut mask.adjustments);
            *value = control.clamp(*value);
        }
        mask.shape = match mask.shape {
            MaskShape::Linear(linear) => {
                let start = sanitize_position(linear.start);
                let mut end = sanitize_position(linear.end);
                // A zero-length gradient has no direction; make it a hard
                // edge pointing down.
                if (end[0] - start[0]).hypot(end[1] - start[1]) < MIN_GRADIENT_LENGTH {
                    end = [start[0], start[1] + MIN_GRADIENT_LENGTH];
                }
                MaskShape::Linear(LinearGradient { start, end })
            }
            MaskShape::Radial(radial) => MaskShape::Radial(RadialGradient {
                center: sanitize_position(radial.center),
                radius: radial.radius.map(|r| {
                    if r.is_finite() {
                        r.clamp(MIN_RADIUS, MAX_RADIUS)
                    } else {
                        RadialGradient::default().radius[0]
                    }
                }),
                rotation: if radial.rotation.is_finite() {
                    radial.rotation.rem_euclid(360.0)
                } else {
                    0.0
                },
                feather: if radial.feather.is_finite() {
                    radial.feather.clamp(0.0, 100.0)
                } else {
                    RadialGradient::default().feather
                },
            }),
        };
        mask
    }
}

/// Positions may lie off the photo (a gradient starting above the frame),
/// but not arbitrarily far.
const POSITION_RANGE: std::ops::RangeInclusive<f64> = -1.0..=2.0;
const MIN_GRADIENT_LENGTH: f64 = 1e-3;
const MIN_RADIUS: f64 = 1e-3;
const MAX_RADIUS: f64 = 2.0;

fn sanitize_position(position: [f64; 2]) -> [f64; 2] {
    position.map(|v| {
        if v.is_finite() {
            v.clamp(*POSITION_RANGE.start(), *POSITION_RANGE.end())
        } else {
            0.5
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_space_keeps_circles_round() {
        let landscape = MaskSpace::new(6000.0, 4000.0);
        assert_eq!(landscape.scale(), [1.0, 4000.0 / 6000.0]);
        let point = [0.25, 0.75];
        let back = landscape.from_metric(landscape.to_metric(point));
        assert!((back[0] - point[0]).abs() < 1e-12 && (back[1] - point[1]).abs() < 1e-12);
        // One long-edge unit is the full width of a landscape photo.
        assert_eq!(landscape.to_metric([1.0, 0.0]), [1.0, 0.0]);
        assert_eq!(MaskSpace::new(0.0, 0.0).scale(), [1.0, 1.0]);
    }

    #[test]
    fn json_round_trip_with_tagged_shapes() {
        let mut sky = Mask::new("Sky", MaskShape::Linear(LinearGradient::default()));
        sky.adjustments.exposure = -0.7;
        let mut face = Mask::new("Face", MaskShape::Radial(RadialGradient::default()));
        face.invert = true;
        let json = serde_json::to_string(&vec![sky.clone(), face.clone()]).expect("serialize");
        assert!(json.contains(r#""kind":"linear""#), "{json}");
        let back: Vec<Mask> = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, vec![sky, face]);
    }

    #[test]
    fn missing_mask_fields_default() {
        let mask: Mask =
            serde_json::from_str(r#"{"shape": {"kind": "radial", "center": [0.3, 0.4]}}"#)
                .expect("parse");
        assert!(mask.enabled, "masks are on unless switched off");
        let MaskShape::Radial(radial) = mask.shape else {
            panic!("radial")
        };
        assert_eq!(radial.center, [0.3, 0.4]);
        assert_eq!(radial.feather, RadialGradient::default().feather);
        assert!(mask.adjustments.is_neutral() && !mask.invert);
    }

    #[test]
    fn sanitize_repairs_hostile_values() {
        let mut mask = Mask::new(
            "x".repeat(10_000),
            MaskShape::Radial(RadialGradient {
                center: [f64::NAN, 1e300],
                radius: [0.0, f64::INFINITY],
                rotation: -90.0,
                feather: 400.0,
            }),
        );
        mask.adjustments.exposure = 99.0;
        mask.adjustments.contrast = f32::NAN;
        let clean = mask.sanitized();
        assert_eq!(clean.name.chars().count(), MAX_MASK_NAME_CHARS);
        assert_eq!(clean.adjustments.exposure, 4.0);
        assert_eq!(clean.adjustments.contrast, 0.0);
        let MaskShape::Radial(radial) = clean.shape else {
            panic!("radial")
        };
        assert_eq!(radial.center, [0.5, 2.0]);
        assert_eq!(radial.radius, [MIN_RADIUS, 0.2]);
        assert_eq!((radial.rotation, radial.feather), (270.0, 100.0));
    }

    #[test]
    fn sanitize_gives_zero_length_gradients_a_direction() {
        let mask = Mask::new(
            "Edge",
            MaskShape::Linear(LinearGradient {
                start: [0.5, 0.5],
                end: [0.5, 0.5],
            }),
        )
        .sanitized();
        let MaskShape::Linear(linear) = mask.shape else {
            panic!("linear")
        };
        assert!(linear.end[1] > linear.start[1]);
    }

    #[test]
    fn name_truncation_respects_character_boundaries() {
        let clean = Mask::new("é".repeat(100), MaskShape::default()).sanitized();
        assert_eq!(clean.name.chars().count(), MAX_MASK_NAME_CHARS);
    }
}
