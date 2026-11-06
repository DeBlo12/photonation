// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Masks: layers that each select part of the photo with one or more
//! shapes, and apply their local adjustments there. See docs/EDIT-MODEL.md.

use crate::{BrushStroke, LOCAL_CONTROLS};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// Most masks one photo keeps. Every mask costs time at every pixel, so the
/// cap also stops a hostile sidecar from making renders arbitrarily slow.
pub const MAX_MASKS: usize = 32;
/// Longest mask name kept, in characters.
pub const MAX_MASK_NAME_CHARS: usize = 64;
/// Most shapes one mask combines. The desktop overlay evaluates them in a
/// fixed-size GPU uniform, and each one costs time at every pixel.
pub const MAX_MASK_COMPONENTS: usize = 8;
/// Most strokes one [`MaskBrush`] keeps.
pub const MAX_MASK_BRUSH_STROKES: usize = 1024;
/// Most points one [`MaskBrush`] keeps across its strokes. Painting whole
/// photos at a small size stays well below it; it bounds the time a hostile
/// sidecar can make a brush's weight map take.
pub const MAX_MASK_BRUSH_POINTS: usize = 32_768;

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

/// One stroke of a [`MaskBrush`]: a path swept by a round brush.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaskStroke {
    /// Positions in [`MaskSpace`], radius in units of the long edge.
    #[serde(flatten)]
    pub path: BrushStroke,
    /// How much of the radius fades out, 0..100 (0 = hard edge).
    pub feather: f32,
    /// How much of the selection one stroke paints or erases, 0..100.
    pub flow: f32,
    /// Takes away from the selection instead of adding to it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub erase: bool,
}

impl Default for MaskStroke {
    fn default() -> Self {
        MaskStroke {
            path: BrushStroke::default(),
            feather: 50.0,
            flow: 100.0,
            erase: false,
        }
    }
}

impl MaskStroke {
    /// The selection after this stroke, where its own weight (from its
    /// radius and feather) is `weight`. Painting over a selection builds up
    /// towards full; at full flow a stroke's core always reaches it.
    #[inline(always)]
    pub fn combine(erase: bool, flow: f32, selected: f32, weight: f32) -> f32 {
        let strength = flow * weight;
        if erase {
            selected * (1.0 - strength)
        } else {
            selected + (1.0 - selected) * strength
        }
    }
}

/// Painted strokes, applied in order to a selection that starts empty.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaskBrush {
    pub strokes: Vec<MaskStroke>,
}

impl MaskBrush {
    /// Whether `stroke` still fits under the caps.
    pub fn has_room_for(&self, stroke: &MaskStroke) -> bool {
        self.strokes.len() < MAX_MASK_BRUSH_STROKES
            && self.point_count() + stroke.path.points.len() <= MAX_MASK_BRUSH_POINTS
    }

    fn point_count(&self) -> usize {
        self.strokes
            .iter()
            .map(|stroke| stroke.path.points.len())
            .sum()
    }

    /// A hash of everything that changes what the brush selects, for
    /// in-memory caches. Not stable across builds, so never persist it.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for stroke in &self.strokes {
            stroke.path.hash_path(&mut hasher);
            stroke.feather.to_bits().hash(&mut hasher);
            stroke.flow.to_bits().hash(&mut hasher);
            stroke.erase.hash(&mut hasher);
        }
        hasher.finish()
    }

    fn sanitize(&mut self) {
        self.strokes.truncate(MAX_MASK_BRUSH_STROKES);
        for stroke in &mut self.strokes {
            stroke.path.sanitize();
            stroke.feather = percent_or(stroke.feather, MaskStroke::default().feather);
            stroke.flow = percent_or(stroke.flow, MaskStroke::default().flow);
        }
        self.strokes.retain(|stroke| !stroke.path.points.is_empty());
        let mut points = 0;
        let kept = self
            .strokes
            .iter()
            .take_while(|stroke| {
                points += stroke.path.points.len();
                points <= MAX_MASK_BRUSH_POINTS
            })
            .count();
        self.strokes.truncate(kept);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MaskShape {
    Linear(LinearGradient),
    Radial(RadialGradient),
    Brush(MaskBrush),
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

/// How a shape combines with the shapes above it in its mask. Weights are
/// 0..1; the first shape is the starting selection, whatever its mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CombineMode {
    /// Union: the larger weight.
    #[default]
    Add,
    /// Remove this shape's selection: the smaller of the weight and
    /// one minus the shape's.
    Subtract,
    /// Keep only what both select: the smaller weight.
    Intersect,
}

impl CombineMode {
    pub const ALL: [CombineMode; 3] = [
        CombineMode::Add,
        CombineMode::Subtract,
        CombineMode::Intersect,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CombineMode::Add => "Add",
            CombineMode::Subtract => "Subtract",
            CombineMode::Intersect => "Intersect",
        }
    }

    /// The selection after combining `selected` (so far) with a shape's
    /// `weight`.
    #[inline(always)]
    pub fn combine(self, selected: f32, weight: f32) -> f32 {
        match self {
            CombineMode::Add => selected.max(weight),
            CombineMode::Subtract => selected.min(1.0 - weight),
            CombineMode::Intersect => selected.min(weight),
        }
    }
}

/// One shape of a mask and how it combines with the shapes before it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaskComponent {
    pub shape: MaskShape,
    pub mode: CombineMode,
}

impl MaskComponent {
    pub fn new(shape: MaskShape, mode: CombineMode) -> MaskComponent {
        MaskComponent { shape, mode }
    }
}

/// A mask layer: shapes that together select part of the photo
/// (optionally inverted to select everything else), how strongly the layer
/// applies, and what it changes there. Layers add up, so their order only
/// matters for display. A mask may sit in a group
/// ([`MaskGroup`](crate::MaskGroup)), whose switch and amount also apply.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "StoredMask")]
pub struct Mask {
    pub name: String,
    /// Disabled masks are kept but not rendered.
    pub enabled: bool,
    /// Combined in order; see [`CombineMode`].
    pub components: Vec<MaskComponent>,
    /// Apply the adjustments outside the selection instead of inside.
    pub invert: bool,
    /// How much of the adjustments apply, 0..100 %.
    pub amount: f32,
    pub adjustments: LocalAdjustments,
    /// Index into `Settings::mask_groups`; `None` for a mask outside any
    /// group, which sidecars leave out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<usize>,
}

/// A mask as sidecars store it: also accepts the single `shape` that masks
/// had before they could combine shapes.
#[derive(Deserialize)]
#[serde(default)]
struct StoredMask {
    name: String,
    enabled: bool,
    shape: Option<MaskShape>,
    components: Vec<MaskComponent>,
    invert: bool,
    amount: f32,
    adjustments: LocalAdjustments,
    group: Option<usize>,
}

impl Default for StoredMask {
    fn default() -> Self {
        let mask = Mask::default();
        StoredMask {
            name: mask.name,
            enabled: mask.enabled,
            shape: None,
            components: Vec::new(),
            invert: mask.invert,
            amount: mask.amount,
            adjustments: mask.adjustments,
            group: mask.group,
        }
    }
}

impl From<StoredMask> for Mask {
    fn from(stored: StoredMask) -> Mask {
        let components = match (stored.components.is_empty(), stored.shape) {
            (true, Some(shape)) => vec![MaskComponent::new(shape, CombineMode::Add)],
            _ => stored.components,
        };
        Mask {
            name: stored.name,
            enabled: stored.enabled,
            components,
            invert: stored.invert,
            amount: stored.amount,
            adjustments: stored.adjustments,
            group: stored.group,
        }
    }
}

impl Default for Mask {
    fn default() -> Self {
        Mask::new("", MaskShape::default())
    }
}

impl Mask {
    /// A mask of one shape, at full amount, adjusting nothing yet.
    pub fn new(name: impl Into<String>, shape: MaskShape) -> Mask {
        Mask {
            name: name.into(),
            enabled: true,
            components: vec![MaskComponent::new(shape, CombineMode::Add)],
            invert: false,
            amount: 100.0,
            adjustments: LocalAdjustments::default(),
            group: None,
        }
    }

    /// Whether rendering this mask changes anything.
    pub fn is_active(&self) -> bool {
        self.enabled
            && self.amount > 0.0
            && !self.components.is_empty()
            && !self.adjustments.is_neutral()
    }

    /// Clamps every value into range and repairs non-finite input.
    pub fn sanitized(&self) -> Mask {
        let mut mask = self.clone();
        truncate_name(&mut mask.name);
        for control in LOCAL_CONTROLS {
            let value = control.get_mut(&mut mask.adjustments);
            *value = control.clamp(*value);
        }
        // A brush's selection is a weight map in memory, so the number of
        // masks also bounds what a hostile sidecar can allocate. A second
        // brush would add nothing: one brush paints and erases on its own.
        let mut seen_brush = false;
        mask.components.retain(|component| {
            let is_brush = matches!(component.shape, MaskShape::Brush(_));
            !(is_brush && std::mem::replace(&mut seen_brush, true))
        });
        mask.components.truncate(MAX_MASK_COMPONENTS);
        for component in &mut mask.components {
            sanitize_shape(&mut component.shape);
        }
        mask.amount = if mask.amount.is_finite() {
            mask.amount.clamp(0.0, 100.0)
        } else {
            100.0
        };
        mask
    }
}

/// Cuts `name` to [`MAX_MASK_NAME_CHARS`] characters.
pub(crate) fn truncate_name(name: &mut String) {
    if let Some((cut, _)) = name.char_indices().nth(MAX_MASK_NAME_CHARS) {
        name.truncate(cut);
    }
}

/// Positions may lie off the photo (a gradient starting above the frame),
/// but not arbitrarily far.
const POSITION_RANGE: std::ops::RangeInclusive<f64> = -1.0..=2.0;
const MIN_GRADIENT_LENGTH: f64 = 1e-3;
const MIN_RADIUS: f64 = 1e-3;
const MAX_RADIUS: f64 = 2.0;

fn sanitize_shape(shape: &mut MaskShape) {
    match shape {
        MaskShape::Linear(linear) => {
            linear.start = sanitize_position(linear.start);
            linear.end = sanitize_position(linear.end);
            let (start, end) = (linear.start, linear.end);
            // A zero-length gradient has no direction; make it a hard
            // edge pointing down.
            if (end[0] - start[0]).hypot(end[1] - start[1]) < MIN_GRADIENT_LENGTH {
                linear.end = [start[0], start[1] + MIN_GRADIENT_LENGTH];
            }
        }
        MaskShape::Radial(radial) => {
            radial.center = sanitize_position(radial.center);
            radial.radius = radial.radius.map(|r| {
                if r.is_finite() {
                    r.clamp(MIN_RADIUS, MAX_RADIUS)
                } else {
                    RadialGradient::default().radius[0]
                }
            });
            radial.rotation = if radial.rotation.is_finite() {
                radial.rotation.rem_euclid(360.0)
            } else {
                0.0
            };
            radial.feather = percent_or(radial.feather, RadialGradient::default().feather);
        }
        MaskShape::Brush(brush) => brush.sanitize(),
    }
}

/// `value` clamped to 0..100, or `default` if it isn't finite.
fn percent_or(value: f32, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        default
    }
}

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
        face.components.push(MaskComponent::new(
            MaskShape::Linear(LinearGradient::default()),
            CombineMode::Subtract,
        ));
        face.amount = 40.0;
        let json = serde_json::to_string(&vec![sky.clone(), face.clone()]).expect("serialize");
        assert!(json.contains(r#""kind":"linear""#), "{json}");
        assert!(json.contains(r#""mode":"subtract""#), "{json}");
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert!(
            value[0].get("shape").is_none(),
            "only the legacy reader knows `shape`"
        );
        let back: Vec<Mask> = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, vec![sky, face]);
    }

    #[test]
    fn single_shape_masks_from_older_sidecars_load_as_one_component() {
        let mask: Mask =
            serde_json::from_str(r#"{"shape": {"kind": "radial", "center": [0.3, 0.4]}}"#)
                .expect("parse");
        assert!(mask.enabled, "masks are on unless switched off");
        assert_eq!(mask.amount, 100.0, "full amount unless set");
        assert_eq!(mask.components.len(), 1);
        assert_eq!(mask.components[0].mode, CombineMode::Add);
        let MaskShape::Radial(radial) = mask.components[0].shape else {
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
        mask.amount = f32::NAN;
        mask.components = vec![mask.components[0].clone(); MAX_MASK_COMPONENTS * 4];
        let clean = mask.sanitized();
        assert_eq!(clean.name.chars().count(), MAX_MASK_NAME_CHARS);
        assert_eq!(clean.adjustments.exposure, 4.0);
        assert_eq!(clean.adjustments.contrast, 0.0);
        assert_eq!(clean.amount, 100.0);
        assert_eq!(clean.components.len(), MAX_MASK_COMPONENTS);
        let MaskShape::Radial(radial) = clean.components[0].shape else {
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
        let MaskShape::Linear(linear) = mask.components[0].shape else {
            panic!("linear")
        };
        assert!(linear.end[1] > linear.start[1]);
    }

    #[test]
    fn combine_modes_follow_fuzzy_set_rules() {
        use CombineMode::{Add, Intersect, Subtract};
        assert_eq!(Add.combine(0.25, 0.75), 0.75);
        assert_eq!(Subtract.combine(0.75, 0.75), 0.25);
        assert_eq!(Subtract.combine(0.75, 0.0), 0.75);
        assert_eq!(Intersect.combine(0.25, 0.75), 0.25);
        // Combining a selection with itself keeps it.
        for weight in [0.0, 0.3, 1.0] {
            assert_eq!(Add.combine(weight, weight), weight);
            assert_eq!(Intersect.combine(weight, weight), weight);
        }
    }

    #[test]
    fn empty_or_zero_amount_masks_do_nothing() {
        let mut mask = Mask::new("Sky", MaskShape::default());
        mask.adjustments.exposure = 1.0;
        assert!(mask.is_active());
        mask.amount = 0.0;
        assert!(!mask.is_active());
        mask.amount = 100.0;
        mask.components.clear();
        assert!(!mask.is_active());
    }

    fn stroke(points: Vec<[f64; 2]>) -> MaskStroke {
        MaskStroke {
            path: BrushStroke {
                points,
                radius: 0.05,
            },
            ..MaskStroke::default()
        }
    }

    fn brush_mask(strokes: Vec<MaskStroke>) -> Mask {
        Mask::new("Painted", MaskShape::Brush(MaskBrush { strokes }))
    }

    fn brush_of(mask: &Mask) -> &MaskBrush {
        match &mask.components[0].shape {
            MaskShape::Brush(brush) => brush,
            other => panic!("not a brush: {other:?}"),
        }
    }

    #[test]
    fn brush_strokes_round_trip_and_stay_compact() {
        let mut erased = stroke(vec![[0.1, 0.2], [0.3, 0.4]]);
        erased.erase = true;
        erased.flow = 40.0;
        let mask = brush_mask(vec![stroke(vec![[0.5, 0.5]]), erased]);
        let json = serde_json::to_string(&mask).expect("serialize");
        assert!(json.contains(r#""kind":"brush""#), "{json}");
        assert!(json.contains(r#""radius":0.05"#), "flat: {json}");
        assert_eq!(
            json.matches("erase").count(),
            1,
            "painting strokes leave it out: {json}"
        );
        let back: Mask = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, mask);
    }

    #[test]
    fn brush_stroke_fields_default_when_missing() {
        let mask: Mask = serde_json::from_str(
            r#"{"components": [{"shape": {"kind": "brush", "strokes": [{"points": [[0.5, 0.5]]}]}}]}"#,
        )
        .expect("parse");
        let stroke = &brush_of(&mask).strokes[0];
        assert_eq!(stroke.path.radius, BrushStroke::default().radius);
        assert_eq!((stroke.feather, stroke.flow), (50.0, 100.0));
        assert!(!stroke.erase);
    }

    #[test]
    fn sanitize_repairs_and_caps_brushes() {
        let mut hostile = stroke(vec![[f64::NAN, 0.5], [9.0, -9.0]]);
        hostile.path.radius = f64::INFINITY;
        hostile.feather = f32::NAN;
        hostile.flow = 1000.0;
        let clean = brush_mask(vec![hostile, stroke(vec![[f64::NAN, f64::NAN]])]).sanitized();
        let strokes = &brush_of(&clean).strokes;
        assert_eq!(strokes.len(), 1, "a stroke left without points goes");
        assert_eq!(strokes[0].path.points, vec![[2.0, -1.0]]);
        assert_eq!(strokes[0].path.radius, BrushStroke::default().radius);
        assert_eq!((strokes[0].feather, strokes[0].flow), (50.0, 100.0));

        let many = vec![stroke(vec![[0.5, 0.5]]); MAX_MASK_BRUSH_STROKES * 2];
        let clean = brush_mask(many).sanitized();
        assert_eq!(brush_of(&clean).strokes.len(), MAX_MASK_BRUSH_STROKES);

        let long = vec![stroke(vec![[0.5, 0.5]; crate::MAX_STROKE_POINTS]); 40];
        let clean = brush_mask(long).sanitized();
        let brush = brush_of(&clean);
        assert_eq!(
            brush.point_count(),
            MAX_MASK_BRUSH_POINTS,
            "whole strokes up to the cap"
        );
        assert!(!brush.has_room_for(&stroke(vec![[0.5, 0.5]])));
    }

    #[test]
    fn sanitize_keeps_one_brush_per_mask() {
        let mut mask = brush_mask(vec![stroke(vec![[0.1, 0.1]])]);
        mask.components
            .push(MaskComponent::new(MaskShape::default(), CombineMode::Add));
        mask.components.push(MaskComponent::new(
            MaskShape::Brush(MaskBrush::default()),
            CombineMode::Subtract,
        ));
        let clean = mask.sanitized();
        assert_eq!(clean.components.len(), 2);
        assert!(matches!(clean.components[1].shape, MaskShape::Radial(_)));
    }

    #[test]
    fn brush_fingerprints_follow_what_it_selects() {
        let base = MaskBrush {
            strokes: vec![stroke(vec![[0.5, 0.5], [0.6, 0.5]])],
        };
        let changed = |edit: fn(&mut MaskStroke)| {
            let mut other = base.clone();
            edit(&mut other.strokes[0]);
            other.fingerprint()
        };
        assert_eq!(base.fingerprint(), base.clone().fingerprint());
        assert_ne!(base.fingerprint(), changed(|s| s.path.points[1][0] = 0.7));
        assert_ne!(base.fingerprint(), changed(|s| s.path.radius = 0.1));
        assert_ne!(base.fingerprint(), changed(|s| s.feather = 10.0));
        assert_ne!(base.fingerprint(), changed(|s| s.flow = 10.0));
        assert_ne!(base.fingerprint(), changed(|s| s.erase = true));
    }

    #[test]
    fn strokes_build_up_and_erase() {
        assert_eq!(MaskStroke::combine(false, 1.0, 0.25, 1.0), 1.0, "full flow");
        assert_eq!(MaskStroke::combine(false, 0.5, 0.5, 1.0), 0.75, "build-up");
        assert_eq!(MaskStroke::combine(false, 0.5, 0.5, 0.0), 0.5, "untouched");
        assert_eq!(MaskStroke::combine(true, 1.0, 0.75, 1.0), 0.0, "erased");
        assert_eq!(MaskStroke::combine(true, 0.5, 0.5, 1.0), 0.25, "half");
        assert_eq!(MaskStroke::combine(true, 1.0, 0.5, 0.0), 0.5, "untouched");
    }

    #[test]
    fn name_truncation_respects_character_boundaries() {
        let clean = Mask::new("é".repeat(100), MaskShape::default()).sanitized();
        assert_eq!(clean.name.chars().count(), MAX_MASK_NAME_CHARS);
    }
}
