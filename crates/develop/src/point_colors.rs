// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Point colours: colours sampled from the photo, each with its own hue,
//! saturation and luminance shift applied to similar colours. See
//! docs/EDIT-MODEL.md.

use crate::POINT_COLOR_CONTROLS;
use serde::{Deserialize, Serialize};

/// Most point colours one photo keeps (Lightroom's limit). Each one costs
/// time at every pixel, so the cap also bounds a hostile sidecar.
pub const MAX_POINT_COLORS: usize = 8;
/// Largest chroma a sampled colour keeps; Oklab chroma of real colours
/// stays well below this.
pub const MAX_SAMPLED_CHROMA: f32 = 0.5;

/// A colour in Oklab's polar form, the space the colour stage works in.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OklabLch {
    /// 0 (black) .. 1 (white).
    pub lightness: f32,
    /// 0 for greys; saturated colours reach about 0.3.
    pub chroma: f32,
    /// Degrees, 0..360.
    pub hue: f32,
}

impl OklabLch {
    /// From Oklab `[L, a, b]`.
    pub fn from_lab(lab: [f32; 3]) -> OklabLch {
        OklabLch {
            lightness: lab[0],
            chroma: lab[1].hypot(lab[2]),
            hue: lab[2].atan2(lab[1]).to_degrees().rem_euclid(360.0),
        }
    }

    /// Oklab `[L, a, b]`.
    pub fn to_lab(self) -> [f32; 3] {
        let angle = self.hue.to_radians();
        [
            self.lightness,
            self.chroma * angle.cos(),
            self.chroma * angle.sin(),
        ]
    }

    /// Clamped into range, with non-finite values replaced by grey.
    pub fn sanitized(self) -> OklabLch {
        let finite_or = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        OklabLch {
            lightness: finite_or(self.lightness, 0.5).clamp(0.0, 1.0),
            chroma: finite_or(self.chroma, 0.0).clamp(0.0, MAX_SAMPLED_CHROMA),
            hue: finite_or(self.hue, 0.0).rem_euclid(360.0),
        }
    }
}

/// One sampled colour and what to do with colours like it.
///
/// Shifts use the familiar −100..100 scale. The ranges (0..100) set how far
/// a pixel's hue, saturation and luminance may sit from the sampled colour
/// and still be affected; the selection fades out smoothly towards their
/// edges.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PointColor {
    /// The colour picked in the photo.
    pub target: OklabLch,
    pub hue: f32,
    pub saturation: f32,
    pub luminance: f32,
    pub hue_range: f32,
    pub saturation_range: f32,
    pub luminance_range: f32,
}

impl Default for PointColor {
    fn default() -> Self {
        PointColor::new(OklabLch::default())
    }
}

impl PointColor {
    /// A point colour for `target` that changes nothing yet.
    pub fn new(target: OklabLch) -> PointColor {
        PointColor {
            target,
            hue: 0.0,
            saturation: 0.0,
            luminance: 0.0,
            hue_range: 50.0,
            saturation_range: 50.0,
            luminance_range: 50.0,
        }
    }

    /// Whether it changes any colour.
    pub fn is_active(&self) -> bool {
        self.hue != 0.0 || self.saturation != 0.0 || self.luminance != 0.0
    }

    pub fn sanitized(&self) -> PointColor {
        let mut point = *self;
        point.target = point.target.sanitized();
        for control in POINT_COLOR_CONTROLS {
            let value = control.get_mut(&mut point);
            *value = control.clamp(*value);
        }
        point
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lch_round_trips_through_lab() {
        let color = OklabLch {
            lightness: 0.6,
            chroma: 0.12,
            hue: 250.0,
        };
        let back = OklabLch::from_lab(color.to_lab());
        assert!((back.lightness - color.lightness).abs() < 1e-6);
        assert!((back.chroma - color.chroma).abs() < 1e-6);
        assert!((back.hue - color.hue).abs() < 1e-3);
    }

    #[test]
    fn sanitize_repairs_hostile_values() {
        let point = PointColor {
            target: OklabLch {
                lightness: f32::NAN,
                chroma: 9.0,
                hue: -30.0,
            },
            hue: 500.0,
            hue_range: f32::INFINITY,
            ..PointColor::default()
        }
        .sanitized();
        assert_eq!(point.target.lightness, 0.5);
        assert_eq!(point.target.chroma, MAX_SAMPLED_CHROMA);
        assert_eq!(point.target.hue, 330.0);
        assert_eq!(point.hue, 100.0);
        assert_eq!(point.hue_range, 50.0);
    }

    #[test]
    fn a_fresh_point_changes_nothing() {
        assert!(!PointColor::default().is_active());
    }
}
