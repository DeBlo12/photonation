// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Removals: brushed areas whose content is replaced by a plausible fill
//! (a fly in front of a cow, a passer-by, a fence post). The fill itself is
//! computed outside the edit model; a removal only records where to remove
//! and how. See docs/RETOUCH.md.

use crate::MaskSpace;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// Most removals one photo keeps. Each one costs a fill computation and
/// time at every pixel its bounds cover.
pub const MAX_REMOVALS: usize = 64;
/// Most strokes one removal combines.
pub const MAX_REMOVAL_STROKES: usize = 32;
/// Most points one stroke keeps; the editor thins strokes to stay below it.
pub const MAX_STROKE_POINTS: usize = 1024;
/// Brush radius limits, in units of the photo's long edge.
pub const MIN_BRUSH_RADIUS: f64 = 0.001;
pub const MAX_BRUSH_RADIUS: f64 = 0.25;

/// How a removal's area is filled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemovalMethod {
    /// A neural inpainting model invents content that fits the scene.
    /// Falls back to [`RemovalMethod::ContentAware`] when no model is
    /// installed.
    #[default]
    Generative,
    /// Patches copied from the rest of the photo (PatchMatch). Always
    /// available, deterministic, best on texture (grass, sky, walls).
    ContentAware,
}

impl RemovalMethod {
    pub const ALL: [RemovalMethod; 2] = [RemovalMethod::Generative, RemovalMethod::ContentAware];

    pub fn label(self) -> &'static str {
        match self {
            RemovalMethod::Generative => "Generative AI",
            RemovalMethod::ContentAware => "Content-aware",
        }
    }
}

/// One brush stroke: a polyline swept by a round brush. Positions are in
/// [`MaskSpace`] (normalized, display orientation, before the user's crop),
/// the radius in units of the long edge, like mask radii.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrushStroke {
    pub points: Vec<[f64; 2]>,
    pub radius: f64,
}

impl Default for BrushStroke {
    fn default() -> Self {
        BrushStroke {
            points: Vec::new(),
            radius: 0.02,
        }
    }
}

/// A brushed area to remove. Removals apply in order: each one's fill sees
/// the photo with the earlier removals already applied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Removal {
    /// Disabled removals are kept but not rendered.
    pub enabled: bool,
    pub method: RemovalMethod,
    /// Their union is the area replaced.
    pub strokes: Vec<BrushStroke>,
    /// Width of the soft edge as a share of the brush radius, 0..100.
    pub feather: f32,
    /// Varies the fill: a different seed gives a different result for
    /// methods that have a random component.
    pub seed: u32,
}

impl Default for Removal {
    fn default() -> Self {
        Removal {
            enabled: true,
            method: RemovalMethod::default(),
            strokes: Vec::new(),
            feather: 25.0,
            seed: 0,
        }
    }
}

impl Removal {
    pub fn new(method: RemovalMethod, stroke: BrushStroke) -> Removal {
        Removal {
            method,
            strokes: vec![stroke],
            ..Removal::default()
        }
    }

    /// Whether rendering this removal changes anything.
    pub fn is_active(&self) -> bool {
        self.enabled && self.strokes.iter().any(|stroke| !stroke.points.is_empty())
    }

    /// Bounds of everything the brush touches, in [`MaskSpace`]:
    /// `[left, top, right, bottom]`, or `None` without points.
    pub fn bounds(&self, space: MaskSpace) -> Option<[f64; 4]> {
        let scale = space.scale();
        let mut bounds: Option<[f64; 4]> = None;
        for stroke in &self.strokes {
            // The radius is in long-edge units; convert per axis.
            let reach = [stroke.radius / scale[0], stroke.radius / scale[1]];
            for point in &stroke.points {
                let [left, top, right, bottom] = bounds.get_or_insert([
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ]);
                *left = left.min(point[0] - reach[0]);
                *top = top.min(point[1] - reach[1]);
                *right = right.max(point[0] + reach[0]);
                *bottom = bottom.max(point[1] + reach[1]);
            }
        }
        bounds
    }

    /// A hash of everything that changes this removal's fill (not
    /// `enabled`), for in-memory fill caches. Not stable across builds, so
    /// never persist it.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.method.hash(&mut hasher);
        self.seed.hash(&mut hasher);
        self.feather.to_bits().hash(&mut hasher);
        for stroke in &self.strokes {
            stroke.radius.to_bits().hash(&mut hasher);
            stroke.points.len().hash(&mut hasher);
            for point in &stroke.points {
                point[0].to_bits().hash(&mut hasher);
                point[1].to_bits().hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// Caps strokes and points, clamps every value and repairs non-finite
    /// input. The caps bound the per-pixel cost a hostile sidecar can cause.
    pub fn sanitized(&self) -> Removal {
        let mut removal = self.clone();
        removal.strokes.truncate(MAX_REMOVAL_STROKES);
        for stroke in &mut removal.strokes {
            stroke.points.truncate(MAX_STROKE_POINTS);
            stroke
                .points
                .retain(|point| point.iter().all(|v| v.is_finite()));
            for point in &mut stroke.points {
                *point = point.map(|v| v.clamp(-1.0, 2.0));
            }
            stroke.radius = if stroke.radius.is_finite() {
                stroke.radius.clamp(MIN_BRUSH_RADIUS, MAX_BRUSH_RADIUS)
            } else {
                BrushStroke::default().radius
            };
        }
        removal.strokes.retain(|stroke| !stroke.points.is_empty());
        removal.feather = if removal.feather.is_finite() {
            removal.feather.clamp(0.0, 100.0)
        } else {
            Removal::default().feather
        };
        removal
    }
}

/// Fingerprints of a removal chain: entry `i` covers removal `i` and every
/// active removal before it, because a removal's fill sees the earlier
/// ones applied. Inactive removals get `None` and do not affect later ones.
pub fn removal_chain_fingerprints(removals: &[Removal]) -> Vec<Option<u64>> {
    let mut chain = 0u64;
    removals
        .iter()
        .map(|removal| {
            removal.is_active().then(|| {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                chain.hash(&mut hasher);
                removal.fingerprint().hash(&mut hasher);
                chain = hasher.finish();
                chain
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(points: &[[f64; 2]], radius: f64) -> BrushStroke {
        BrushStroke {
            points: points.to_vec(),
            radius,
        }
    }

    #[test]
    fn json_round_trip_and_defaults() {
        let removal = Removal::new(
            RemovalMethod::ContentAware,
            stroke(&[[0.2, 0.3], [0.25, 0.35]], 0.01),
        );
        let json = serde_json::to_string(&removal).expect("serialize");
        assert!(json.contains(r#""method":"content-aware""#), "{json}");
        let back: Removal = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, removal);
        let minimal: Removal = serde_json::from_str("{}").expect("parse");
        assert!(minimal.enabled && minimal.method == RemovalMethod::Generative);
        assert!(!minimal.is_active(), "nothing brushed yet");
    }

    #[test]
    fn bounds_cover_the_brush_on_both_axes() {
        let space = MaskSpace::new(6000.0, 4000.0);
        let removal = Removal::new(RemovalMethod::Generative, stroke(&[[0.5, 0.5]], 0.1));
        let [left, top, right, bottom] = removal.bounds(space).expect("bounds");
        assert!((right - left - 0.2).abs() < 1e-12, "width spans the radius");
        // The short axis is 2/3 of the long edge, so the same radius covers
        // a larger share of it.
        assert!((bottom - top - 0.3).abs() < 1e-12);
        assert_eq!(Removal::default().bounds(space), None);
    }

    #[test]
    fn fingerprint_ignores_enabled_but_not_geometry() {
        let removal = Removal::new(RemovalMethod::Generative, stroke(&[[0.5, 0.5]], 0.1));
        let mut disabled = removal.clone();
        disabled.enabled = false;
        assert_eq!(removal.fingerprint(), disabled.fingerprint());
        let mut moved = removal.clone();
        moved.strokes[0].points[0][0] = 0.51;
        assert_ne!(removal.fingerprint(), moved.fingerprint());
        let mut reseeded = removal.clone();
        reseeded.seed = 1;
        assert_ne!(removal.fingerprint(), reseeded.fingerprint());
    }

    #[test]
    fn chain_fingerprints_depend_on_earlier_active_removals() {
        let first = Removal::new(RemovalMethod::Generative, stroke(&[[0.2, 0.2]], 0.05));
        let second = Removal::new(RemovalMethod::Generative, stroke(&[[0.7, 0.7]], 0.05));
        let alone = removal_chain_fingerprints(std::slice::from_ref(&second));
        let after_first = removal_chain_fingerprints(&[first.clone(), second.clone()]);
        assert_ne!(alone[0], after_first[1]);
        let mut first_off = first;
        first_off.enabled = false;
        let after_disabled = removal_chain_fingerprints(&[first_off, second]);
        assert_eq!(after_disabled[0], None);
        assert_eq!(after_disabled[1], alone[0], "inactive removals don't count");
    }

    #[test]
    fn sanitize_repairs_hostile_values() {
        let mut removal = Removal::new(
            RemovalMethod::Generative,
            stroke(&[[f64::NAN, 0.5], [1e300, -1e300]], f64::INFINITY),
        );
        removal.strokes[0]
            .points
            .extend(std::iter::repeat_n([0.5, 0.5], MAX_STROKE_POINTS * 2));
        removal.strokes.push(stroke(&[], 0.1));
        removal.feather = f32::NAN;
        let clean = removal.sanitized();
        assert_eq!(clean.strokes.len(), 1, "empty strokes are dropped");
        let points = &clean.strokes[0].points;
        assert_eq!(points.len(), MAX_STROKE_POINTS - 1, "NaN point dropped");
        assert_eq!(points[0], [2.0, -1.0]);
        assert_eq!(clean.strokes[0].radius, BrushStroke::default().radius);
        assert_eq!(clean.feather, Removal::default().feather);
    }
}
