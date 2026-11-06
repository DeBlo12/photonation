// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Masks and local adjustments (docs/PIPELINE.md §5). A mask's weight at a
//! pixel combines its shapes' weights in order ([`MaskWeight`]); it scales
//! the mask's adjustments times its amount (and its group's). The weighted
//! adjustments of all masks add up, and the develop stage applies the sum
//! with the global ones.
//!
//! [`MaskWeight::at`] and [`LocalAdjust::at`] are the reference for the
//! GPU, with [`BrushMap::at`]: the desktop editor's mask overlay
//! (`apps/desktop/src/gpu/mask_overlay.wgsl`) and the GPU renderer
//! (`crates/gpu/src/develop.wgsl`); keep them in step.

use crate::brush_map::BrushMap;
use crate::color::smoothstep;
use crate::plan::{LocalPlan, MaskPlan};
use photonation_develop::{CombineMode, LocalAdjustments, Mask, MaskGroup, MaskShape, MaskSpace};
use photonation_image::{Rect, SensorImage};
use std::sync::Arc;

/// The mask space of a photo: its default crop in display orientation.
pub fn mask_space(sensor: &SensorImage) -> MaskSpace {
    let (width, height) = sensor
        .orientation
        .display_size(sensor.crop.w, sensor.crop.h);
    MaskSpace::new(width as f64, height as f64)
}

/// A mask shape prepared for evaluation at metric points of its
/// [`MaskSpace`].
#[derive(Clone, Debug, PartialEq)]
pub enum ShapeWeight {
    Linear {
        start: [f32; 2],
        /// `(end − start) / |end − start|²`, so the dot product with
        /// `point − start` runs 0 → 1 from the start line to the end line.
        ramp: [f32; 2],
    },
    Radial {
        center: [f32; 2],
        /// The ellipse's rotated axes divided by their radii, so the
        /// projected point has length 1 on the edge.
        axis_x: [f32; 2],
        axis_y: [f32; 2],
        /// Normalized radius where the fade starts (1 − feather).
        fade_start: f32,
    },
    /// Painted strokes, already rasterized: too many to measure the distance
    /// to at every pixel.
    Brush(Arc<BrushMap>),
}

impl ShapeWeight {
    pub fn new(shape: &MaskShape, space: MaskSpace) -> ShapeWeight {
        let metric = |position: [f64; 2]| space.to_metric(position).map(|v| v as f32);
        match shape {
            MaskShape::Linear(linear) => {
                let start = metric(linear.start);
                let end = metric(linear.end);
                let direction = [end[0] - start[0], end[1] - start[1]];
                let length_squared = (direction[0] * direction[0] + direction[1] * direction[1])
                    .max(f32::MIN_POSITIVE);
                ShapeWeight::Linear {
                    start,
                    ramp: direction.map(|v| v / length_squared),
                }
            }
            MaskShape::Radial(radial) => {
                let (sin, cos) = radial.rotation.to_radians().sin_cos();
                let radius = radial.radius.map(|r| (r as f32).max(f32::MIN_POSITIVE));
                ShapeWeight::Radial {
                    center: metric(radial.center),
                    axis_x: [cos / radius[0], sin / radius[0]],
                    axis_y: [-sin / radius[1], cos / radius[1]],
                    fade_start: 1.0 - radial.feather / 100.0,
                }
            }
            MaskShape::Brush(brush) => ShapeWeight::Brush(BrushMap::cached(brush, space)),
        }
    }

    /// Whether the weight is exactly 0 everywhere in a metric area
    /// `[left, top, right, bottom]`.
    fn selects_nothing_in(&self, area: [f32; 4]) -> bool {
        match *self {
            // The ramp is linear, so its lowest value over the area is at a
            // corner, and it is past the fade from 1 on.
            ShapeWeight::Linear { start, ramp } => {
                let lowest = |low: f32, high: f32, origin: f32, slope: f32| {
                    (if slope >= 0.0 { low } else { high } - origin) * slope
                };
                lowest(area[0], area[2], start[0], ramp[0])
                    + lowest(area[1], area[3], start[1], ramp[1])
                    > 1.0
            }
            ShapeWeight::Radial {
                center,
                axis_x,
                axis_y,
                ..
            } => {
                // The ellipse's reach along each axis of the photo. The axes
                // are its own divided by their radii, so their inverse
                // lengths give the radii back.
                let radius = [axis_x, axis_y].map(|axis| 1.0 / axis[0].hypot(axis[1]));
                let (rotated_x, rotated_y) = (axis_x[0] * radius[0], axis_x[1] * radius[0]);
                let reach = [
                    (radius[0] * rotated_x).hypot(radius[1] * rotated_y),
                    (radius[0] * rotated_y).hypot(radius[1] * rotated_x),
                ];
                area[2] < center[0] - reach[0]
                    || area[0] > center[0] + reach[0]
                    || area[3] < center[1] - reach[1]
                    || area[1] > center[1] + reach[1]
            }
            ShapeWeight::Brush(ref map) => map.selects_nothing_in(area),
        }
    }

    /// Selection strength, 0..1, at a metric point.
    #[inline(always)]
    pub fn at(&self, point: [f32; 2]) -> f32 {
        match *self {
            ShapeWeight::Linear { start, ramp } => {
                let t = (point[0] - start[0]) * ramp[0] + (point[1] - start[1]) * ramp[1];
                1.0 - smoothstep(0.0, 1.0, t)
            }
            ShapeWeight::Radial {
                center,
                axis_x,
                axis_y,
                fade_start,
            } => {
                let offset = [point[0] - center[0], point[1] - center[1]];
                let x = offset[0] * axis_x[0] + offset[1] * axis_x[1];
                let y = offset[0] * axis_y[0] + offset[1] * axis_y[1];
                let distance = (x * x + y * y).sqrt();
                if fade_start >= 1.0 {
                    // No feather: a hard edge (smoothstep would divide by 0).
                    f32::from(distance <= 1.0)
                } else {
                    1.0 - smoothstep(fade_start, 1.0, distance)
                }
            }
            ShapeWeight::Brush(ref map) => map.at(point),
        }
    }
}

/// Local adjustment amounts at one pixel, summed over the masks covering
/// it: exposure in stops, everything else as fractions (slider ÷ 100).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalAmounts {
    pub exposure: f32,
    pub temperature: f32,
    pub tint: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub dehaze: f32,
    pub saturation: f32,
}

impl LocalAmounts {
    /// No local adjustment.
    pub const NONE: LocalAmounts = LocalAmounts {
        exposure: 0.0,
        temperature: 0.0,
        tint: 0.0,
        contrast: 0.0,
        highlights: 0.0,
        shadows: 0.0,
        dehaze: 0.0,
        saturation: 0.0,
    };

    fn from_adjustments(adjustments: &LocalAdjustments) -> LocalAmounts {
        LocalAmounts {
            exposure: adjustments.exposure,
            temperature: adjustments.temperature / 100.0,
            tint: adjustments.tint / 100.0,
            contrast: adjustments.contrast / 100.0,
            highlights: adjustments.highlights / 100.0,
            shadows: adjustments.shadows / 100.0,
            dehaze: adjustments.dehaze / 100.0,
            saturation: adjustments.saturation / 100.0,
        }
    }

    #[inline(always)]
    fn add_weighted(&mut self, other: &LocalAmounts, weight: f32) {
        self.exposure += other.exposure * weight;
        self.temperature += other.temperature * weight;
        self.tint += other.tint * weight;
        self.contrast += other.contrast * weight;
        self.highlights += other.highlights * weight;
        self.shadows += other.shadows * weight;
        self.dehaze += other.dehaze * weight;
        self.saturation += other.saturation * weight;
    }
}

/// Maps pixel `(x, y)` of a stage that covers sensor `region` at `size`
/// pixels to a metric point in the photo's [`MaskSpace`]. The map is
/// affine (orientation only swaps and mirrors axes), so it is stored as an
/// origin and one step per pixel in each direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelToMask {
    pub(crate) origin: [f32; 2],
    pub(crate) step_x: [f32; 2],
    pub(crate) step_y: [f32; 2],
}

impl PixelToMask {
    pub fn new(sensor: &SensorImage, region: Rect, size: (usize, usize)) -> PixelToMask {
        let crop = sensor.crop;
        let space = mask_space(sensor);
        let scale = (
            region.w as f64 / size.0.max(1) as f64,
            region.h as f64 / size.1.max(1) as f64,
        );
        // Centre of stage pixel (x, y) → stored position on the default
        // crop → displayed position → metric.
        let metric = |x: f64, y: f64| {
            let sensor_x = region.x as f64 + (x + 0.5) * scale.0;
            let sensor_y = region.y as f64 + (y + 0.5) * scale.1;
            let (u, v) = sensor.orientation.to_display_norm(
                (sensor_x - crop.x as f64) / crop.w.max(1) as f64,
                (sensor_y - crop.y as f64) / crop.h.max(1) as f64,
            );
            space.to_metric([u, v])
        };
        let origin = metric(0.0, 0.0);
        let along_x = metric(1.0, 0.0);
        let along_y = metric(0.0, 1.0);
        let difference = |a: [f64; 2]| [(a[0] - origin[0]) as f32, (a[1] - origin[1]) as f32];
        PixelToMask {
            origin: origin.map(|v| v as f32),
            step_x: difference(along_x),
            step_y: difference(along_y),
        }
    }

    #[inline(always)]
    pub fn at(&self, x: usize, y: usize) -> [f32; 2] {
        let (x, y) = (x as f32, y as f32);
        [
            self.origin[0] + x * self.step_x[0] + y * self.step_y[0],
            self.origin[1] + x * self.step_x[1] + y * self.step_y[1],
        ]
    }
}

/// A mask's selection, prepared for evaluation at metric points of its
/// [`MaskSpace`]: its shapes combined in order, then optionally inverted.
#[derive(Clone, Debug, PartialEq)]
pub struct MaskWeight {
    pub components: Vec<(ShapeWeight, CombineMode)>,
    pub invert: bool,
}

impl MaskWeight {
    pub fn new(mask: &Mask, space: MaskSpace) -> MaskWeight {
        MaskWeight {
            components: mask
                .components
                .iter()
                .map(|component| (ShapeWeight::new(&component.shape, space), component.mode))
                .collect(),
            invert: mask.invert,
        }
    }

    /// Whether the selection is exactly 0 everywhere in a metric area
    /// `[left, top, right, bottom]`. It answers no when it can't tell: an
    /// inverted mask selects what its shapes leave out.
    pub fn selects_nothing_in(&self, area: [f32; 4]) -> bool {
        if self.invert {
            return false;
        }
        let mut components = self.components.iter();
        let mut nothing = components
            .next()
            .is_none_or(|(shape, _)| shape.selects_nothing_in(area));
        for (shape, mode) in components {
            nothing = match mode {
                CombineMode::Add => nothing && shape.selects_nothing_in(area),
                // Takes away, so what was empty stays empty.
                CombineMode::Subtract => nothing,
                CombineMode::Intersect => nothing || shape.selects_nothing_in(area),
            };
        }
        nothing
    }

    /// Selection strength, 0..1, at a metric point. A mask without shapes
    /// selects nothing (everything, inverted).
    #[inline]
    pub fn at(&self, point: [f32; 2]) -> f32 {
        let mut components = self.components.iter();
        let mut selected = components.next().map_or(0.0, |(shape, _)| shape.at(point));
        for (shape, mode) in components {
            selected = mode.combine(selected, shape.at(point));
        }
        if self.invert {
            1.0 - selected
        } else {
            selected
        }
    }
}

struct WeightedMask {
    weight: MaskWeight,
    amounts: LocalAmounts,
}

/// Every active mask of a render, ready for the develop stage.
pub struct LocalAdjust {
    masks: Vec<WeightedMask>,
    positions: PixelToMask,
}

impl LocalAdjust {
    /// `None` when no mask is enabled (in an enabled group) and adjusts
    /// something, so renders without masks skip the per-pixel work
    /// entirely. Grouped masks render as [`Mask::rendered`] says.
    pub fn new(
        masks: &[Mask],
        groups: &[MaskGroup],
        sensor: &SensorImage,
        positions: PixelToMask,
    ) -> Option<LocalAdjust> {
        let space = mask_space(sensor);
        let masks: Vec<WeightedMask> = masks
            .iter()
            .map(|mask| mask.rendered(groups))
            .filter(|mask| mask.is_active())
            .map(|mask| {
                let full = LocalAmounts::from_adjustments(&mask.adjustments);
                let mut amounts = LocalAmounts::NONE;
                amounts.add_weighted(&full, mask.amount / 100.0);
                WeightedMask {
                    weight: MaskWeight::new(&mask, space),
                    amounts,
                }
            })
            .collect();
        (!masks.is_empty()).then_some(LocalAdjust { masks, positions })
    }

    /// The masks and pixel mapping as data. `limits` are the develop
    /// stage's caps on the summed amounts.
    pub fn plan(&self, limits: crate::plan::LocalLimits) -> LocalPlan {
        LocalPlan {
            origin: self.positions.origin,
            step_x: self.positions.step_x,
            step_y: self.positions.step_y,
            masks: self
                .masks
                .iter()
                .map(|mask| MaskPlan {
                    weight: mask.weight.clone(),
                    amounts: mask.amounts,
                })
                .collect(),
            limits,
        }
    }

    /// The summed adjustments at stage pixel `(x, y)`.
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> LocalAmounts {
        Self::sum(self.masks.iter(), self.positions.at(x, y)).unwrap_or(LocalAmounts::NONE)
    }

    /// The masks that can touch stage row `y`, `width` pixels long. Most
    /// masks cover part of a photo, and a pixel costs less the fewer are left
    /// to measure, so this leaves out those whose bounds miss the row. The
    /// result doesn't change: what's left out would have added nothing.
    pub fn masks_on_row(&self, y: usize, width: usize) -> LocalRow<'_> {
        let (first, last) = (
            self.positions.at(0, y),
            self.positions.at(width.saturating_sub(1), y),
        );
        // Rounding must not cut off a pixel on the very edge of a mask.
        let area = [
            first[0].min(last[0]) - ROW_AREA_MARGIN,
            first[1].min(last[1]) - ROW_AREA_MARGIN,
            first[0].max(last[0]) + ROW_AREA_MARGIN,
            first[1].max(last[1]) + ROW_AREA_MARGIN,
        ];
        LocalRow {
            y,
            positions: self.positions,
            masks: self
                .masks
                .iter()
                .filter(|mask| !mask.weight.selects_nothing_in(area))
                .collect(),
        }
    }

    /// The masks' adjustments at `point`, or `None` where none applies.
    #[inline(always)]
    fn sum<'a>(
        masks: impl Iterator<Item = &'a WeightedMask>,
        point: [f32; 2],
    ) -> Option<LocalAmounts> {
        let mut amounts = None;
        for mask in masks {
            let weight = mask.weight.at(point);
            if weight > 0.0 {
                amounts
                    .get_or_insert(LocalAmounts::NONE)
                    .add_weighted(&mask.amounts, weight);
            }
        }
        amounts
    }
}

/// How far (metric) a row's area reaches past its pixels' centres: well under
/// a pixel on any sensor, well over `f32` rounding at these magnitudes.
const ROW_AREA_MARGIN: f32 = 1e-5;

/// The masks that can touch one stage row.
pub struct LocalRow<'a> {
    y: usize,
    positions: PixelToMask,
    masks: Vec<&'a WeightedMask>,
}

impl LocalRow<'_> {
    pub fn is_empty(&self) -> bool {
        self.masks.is_empty()
    }

    /// The summed adjustments at pixel `x` of the row, or `None` where no
    /// mask applies, so the pixel can skip the local steps.
    #[inline]
    pub fn at(&self, x: usize) -> Option<LocalAmounts> {
        LocalAdjust::sum(self.masks.iter().copied(), self.positions.at(x, self.y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonation_develop::{LinearGradient, RadialGradient};
    use photonation_image::Orientation;

    /// On a square photo metric points equal positions.
    fn square() -> MaskSpace {
        MaskSpace::new(1.0, 1.0)
    }

    fn linear(start: [f64; 2], end: [f64; 2]) -> ShapeWeight {
        ShapeWeight::new(&MaskShape::Linear(LinearGradient { start, end }), square())
    }

    fn radial(center: [f64; 2], radius: [f64; 2], rotation: f32, feather: f32) -> ShapeWeight {
        ShapeWeight::new(
            &MaskShape::Radial(RadialGradient {
                center,
                radius,
                rotation,
                feather,
            }),
            square(),
        )
    }

    #[test]
    fn linear_gradient_fades_from_start_to_end() {
        let shape = linear([0.5, 0.2], [0.5, 0.6]);
        assert_eq!(shape.at([0.1, 0.0]), 1.0, "before the start line");
        assert_eq!(
            shape.at([0.9, 0.2]),
            1.0,
            "on the start line, anywhere along it"
        );
        assert!((shape.at([0.3, 0.4]) - 0.5).abs() < 1e-6, "halfway");
        assert_eq!(shape.at([0.5, 0.6]), 0.0, "on the end line");
        assert_eq!(shape.at([0.5, 1.0]), 0.0, "past the end line");
        let samples: Vec<f32> = (0..=20).map(|i| shape.at([0.5, i as f32 / 20.0])).collect();
        assert!(
            samples.windows(2).all(|pair| pair[1] <= pair[0]),
            "monotone"
        );
    }

    #[test]
    fn radial_gradient_is_full_inside_and_fades_over_the_feather() {
        let shape = radial([0.5, 0.5], [0.2, 0.2], 0.0, 50.0);
        assert_eq!(shape.at([0.5, 0.5]), 1.0);
        assert_eq!(shape.at([0.59, 0.5]), 1.0, "inside the unfeathered core");
        let fading = shape.at([0.65, 0.5]);
        assert!(fading > 0.0 && fading < 1.0, "{fading}");
        assert_eq!(shape.at([0.71, 0.5]), 0.0, "outside the edge");
        let hard = radial([0.5, 0.5], [0.2, 0.2], 0.0, 0.0);
        assert_eq!(hard.at([0.69, 0.5]), 1.0);
        assert_eq!(hard.at([0.71, 0.5]), 0.0);
    }

    #[test]
    fn radial_rotation_turns_the_ellipse_clockwise() {
        // Wide ellipse turned 90°: now tall.
        let shape = radial([0.5, 0.5], [0.3, 0.1], 90.0, 0.0);
        assert_eq!(shape.at([0.5, 0.75]), 1.0);
        assert_eq!(shape.at([0.75, 0.5]), 0.0);
    }

    #[test]
    fn circles_stay_round_on_landscape_photos() {
        let space = MaskSpace::new(3.0, 2.0);
        let circle = ShapeWeight::new(
            &MaskShape::Radial(RadialGradient {
                center: [0.5, 0.5],
                radius: [0.2, 0.2],
                rotation: 0.0,
                feather: 0.0,
            }),
            space,
        );
        // 0.19 long edges right of and below the centre, both inside.
        let center = space.to_metric([0.5, 0.5]).map(|v| v as f32);
        assert_eq!(circle.at([center[0] + 0.19, center[1]]), 1.0);
        assert_eq!(circle.at([center[0], center[1] + 0.19]), 1.0);
        assert_eq!(circle.at([center[0], center[1] + 0.21]), 0.0);
    }

    #[test]
    fn pixels_map_to_display_positions_for_every_orientation() {
        // The first stored pixel's display position, for a 40×24 crop.
        let mut sensor = SensorImage::test_pattern(44, 28, Orientation::Normal);
        let region = sensor.crop;
        let size = (region.w, region.h);
        for (orientation, expected_display_pixel) in [
            (Orientation::Normal, (0.5, 0.5)),
            (Orientation::Rotate90, (23.5, 0.5)),
            (Orientation::Rotate180, (39.5, 23.5)),
            (Orientation::Rotate270, (0.5, 39.5)),
        ] {
            sensor.orientation = orientation;
            let (display_w, display_h) = orientation.display_size(region.w, region.h);
            let long_edge = display_w.max(display_h) as f32;
            let point = PixelToMask::new(&sensor, region, size).at(0, 0);
            let expected = [
                expected_display_pixel.0 / long_edge,
                expected_display_pixel.1 / long_edge,
            ];
            assert!(
                (point[0] - expected[0]).abs() < 1e-5 && (point[1] - expected[1]).abs() < 1e-5,
                "{orientation:?}: {point:?} vs {expected:?}"
            );
        }
    }

    #[test]
    fn half_size_pixels_map_to_quad_centres() {
        let sensor = SensorImage::test_pattern(44, 28, Orientation::Normal);
        let region = Rect::new(2, 2, 40, 24);
        let full = PixelToMask::new(&sensor, region, (40, 24));
        let half = PixelToMask::new(&sensor, region, (20, 12));
        // Half-size pixel 3 covers full pixels 6 and 7; its centre is between.
        let between = (full.at(6, 0)[0] + full.at(7, 0)[0]) / 2.0;
        assert!((half.at(3, 0)[0] - between).abs() < 1e-6);
    }

    /// Every kind of shape, turned and offset, with areas all over a photo:
    /// whatever claims to select nothing there must read 0 at every sample.
    #[test]
    fn selecting_nothing_in_an_area_is_never_wrong() {
        let mut shapes = vec![
            linear([0.5, 0.2], [0.5, 0.6]),
            linear([0.7, 0.8], [0.2, 0.1]),
            linear([0.2, 0.2], [0.21, 0.2]),
            radial([0.4, 0.6], [0.2, 0.05], 30.0, 50.0),
            radial([0.6, 0.3], [0.1, 0.3], 130.0, 0.0),
            radial([0.5, 0.5], [0.15, 0.15], 0.0, 100.0),
        ];
        shapes.push(ShapeWeight::new(
            &MaskShape::Brush(photonation_develop::MaskBrush {
                strokes: vec![photonation_develop::MaskStroke {
                    path: photonation_develop::BrushStroke {
                        points: vec![[0.3, 0.3], [0.6, 0.5]],
                        radius: 0.05,
                    },
                    ..photonation_develop::MaskStroke::default()
                }],
            }),
            square(),
        ));
        let mut claims = 0;
        for shape in &shapes {
            for row in 0..40 {
                for column in 0..40 {
                    let (x, y) = (column as f32 / 40.0, row as f32 / 40.0);
                    let area = [x, y, x + 0.025, y + 0.025];
                    if shape.selects_nothing_in(area) {
                        claims += 1;
                        for i in 0..=5 {
                            for j in 0..=5 {
                                let point = [x + 0.005 * i as f32, y + 0.005 * j as f32];
                                assert_eq!(shape.at(point), 0.0, "{shape:?} at {point:?}");
                            }
                        }
                    }
                }
            }
        }
        // It also has to find something, or it would save nothing.
        assert!(claims > shapes.len() * 40 * 40 / 3, "{claims}");
        let far = [0.9, 0.9, 1.0, 1.0];
        assert!(radial([0.2, 0.2], [0.1, 0.1], 0.0, 50.0).selects_nothing_in(far));
        assert!(linear([0.5, 0.1], [0.5, 0.4]).selects_nothing_in(far));
        assert!(!linear([0.5, 0.4], [0.5, 0.1]).selects_nothing_in(far));
    }

    #[test]
    fn combined_shapes_select_nothing_only_where_the_result_does() {
        use photonation_develop::MaskComponent;
        let left = hard_circle([0.2, 0.5], 0.1);
        let right = hard_circle([0.8, 0.5], 0.1);
        let weight = |mode| {
            let mut mask = Mask::new("Pair", left.clone());
            mask.components
                .push(MaskComponent::new(right.clone(), mode));
            MaskWeight::new(&mask, square())
        };
        let on_right = [0.75, 0.45, 0.85, 0.55];
        let on_left = [0.15, 0.45, 0.25, 0.55];
        let elsewhere = [0.4, 0.0, 0.6, 0.1];
        let add = weight(CombineMode::Add);
        assert!(!add.selects_nothing_in(on_right) && !add.selects_nothing_in(on_left));
        assert!(add.selects_nothing_in(elsewhere));
        let subtract = weight(CombineMode::Subtract);
        assert!(subtract.selects_nothing_in(on_right));
        assert!(!subtract.selects_nothing_in(on_left));
        let intersect = weight(CombineMode::Intersect);
        assert!(intersect.selects_nothing_in(on_right));
        assert!(intersect.selects_nothing_in(on_left));
        let mut inverted = Mask::new("Inverted", left.clone());
        inverted.invert = true;
        let inverted = MaskWeight::new(&inverted, square());
        assert!(!inverted.selects_nothing_in(elsewhere), "it selects it");
        assert!(
            MaskWeight::new(
                &Mask {
                    components: Vec::new(),
                    ..Mask::default()
                },
                square()
            )
            .selects_nothing_in(elsewhere)
        );
    }

    #[test]
    fn a_row_gives_the_same_amounts_as_every_mask_measured() {
        for orientation in [Orientation::Normal, Orientation::Rotate90] {
            let sensor = SensorImage::test_pattern(104, 84, orientation);
            let size = (sensor.crop.w, sensor.crop.h);
            let positions = PixelToMask::new(&sensor, sensor.crop, size);
            let adjusted = |mut mask: Mask| {
                mask.adjustments.exposure = 1.0;
                mask.adjustments.saturation = 20.0;
                mask
            };
            let masks = [
                adjusted(Mask::new(
                    "Top",
                    MaskShape::Linear(LinearGradient {
                        start: [0.5, 0.1],
                        end: [0.5, 0.3],
                    }),
                )),
                adjusted(Mask::new("Spot", hard_circle([0.6, 0.7], 0.1))),
            ];
            let local = LocalAdjust::new(&masks, &[], &sensor, positions).expect("active");
            let (mut skipped, mut empty_rows) = (0, 0);
            for y in 0..size.1 {
                let row = local.masks_on_row(y, size.0);
                skipped += masks.len() - row.masks.len();
                empty_rows += usize::from(row.is_empty());
                for x in 0..size.0 {
                    let amounts = row.at(x).unwrap_or(LocalAmounts::NONE);
                    assert_eq!(amounts, local.at(x, y), "{orientation:?} {x} {y}");
                }
            }
            assert!(skipped > 0, "{orientation:?} leaves masks out");
            if orientation == Orientation::Normal {
                assert!(empty_rows > 0, "rows between the masks need none");
            }
        }
    }

    #[test]
    fn local_adjust_sums_weighted_masks_and_honours_invert() {
        let sensor = SensorImage::test_pattern(104, 104, Orientation::Normal);
        let positions = PixelToMask::new(&sensor, sensor.crop, (100, 100));
        let mut brighten = Mask::new(
            "Centre",
            MaskShape::Radial(RadialGradient {
                center: [0.5, 0.5],
                radius: [0.2, 0.2],
                rotation: 0.0,
                feather: 0.0,
            }),
        );
        brighten.adjustments.exposure = 1.0;
        let mut darken_outside = brighten.clone();
        darken_outside.invert = true;
        darken_outside.adjustments.exposure = -2.0;
        let mut disabled = brighten.clone();
        disabled.enabled = false;
        let local = LocalAdjust::new(
            &[brighten, darken_outside, disabled],
            &[],
            &sensor,
            positions,
        )
        .expect("active masks");
        assert_eq!(local.at(50, 50).exposure, 1.0);
        assert_eq!(local.at(2, 2).exposure, -2.0);

        let idle = Mask::new("Nothing to do", MaskShape::default());
        assert!(LocalAdjust::new(&[idle], &[], &sensor, positions).is_none());
    }

    fn hard_circle(center: [f64; 2], radius: f64) -> MaskShape {
        MaskShape::Radial(RadialGradient {
            center,
            radius: [radius, radius],
            rotation: 0.0,
            feather: 0.0,
        })
    }

    #[test]
    fn mask_weight_combines_shapes_in_order() {
        use photonation_develop::MaskComponent;
        let mut ring = Mask::new("Ring", hard_circle([0.5, 0.5], 0.3));
        ring.components.push(MaskComponent::new(
            hard_circle([0.5, 0.5], 0.1),
            CombineMode::Subtract,
        ));
        let weight = MaskWeight::new(&ring, square());
        assert_eq!(weight.at([0.5, 0.5]), 0.0, "the hole");
        assert_eq!(weight.at([0.7, 0.5]), 1.0, "the ring");
        assert_eq!(weight.at([0.95, 0.5]), 0.0, "outside");

        let mut lens = Mask::new("Lens", hard_circle([0.4, 0.5], 0.2));
        lens.components.push(MaskComponent::new(
            hard_circle([0.6, 0.5], 0.2),
            CombineMode::Intersect,
        ));
        let weight = MaskWeight::new(&lens, square());
        assert_eq!(weight.at([0.5, 0.5]), 1.0, "in both");
        assert_eq!(weight.at([0.25, 0.5]), 0.0, "in the first only");

        let mut pair = lens.clone();
        pair.components[1].mode = CombineMode::Add;
        pair.invert = true;
        let weight = MaskWeight::new(&pair, square());
        assert_eq!(weight.at([0.25, 0.5]), 0.0, "in either, inverted");
        assert_eq!(weight.at([0.05, 0.05]), 1.0, "in neither, inverted");

        let empty = Mask {
            components: Vec::new(),
            ..Mask::default()
        };
        assert_eq!(MaskWeight::new(&empty, square()).at([0.5, 0.5]), 0.0);
    }

    #[test]
    fn amount_scales_a_masks_adjustments() {
        let sensor = SensorImage::test_pattern(104, 104, Orientation::Normal);
        let positions = PixelToMask::new(&sensor, sensor.crop, (100, 100));
        let mut mask = Mask::new("Centre", hard_circle([0.5, 0.5], 0.2));
        mask.adjustments.exposure = 2.0;
        mask.adjustments.contrast = 50.0;
        mask.amount = 25.0;
        let local = LocalAdjust::new(&[mask], &[], &sensor, positions).expect("active");
        let centre = local.at(50, 50);
        assert_eq!((centre.exposure, centre.contrast), (0.5, 0.125));
    }

    #[test]
    fn groups_scale_and_hide_their_masks() {
        let sensor = SensorImage::test_pattern(104, 104, Orientation::Normal);
        let positions = PixelToMask::new(&sensor, sensor.crop, (100, 100));
        let mut grouped = Mask::new("Centre", hard_circle([0.5, 0.5], 0.2));
        grouped.adjustments.exposure = 2.0;
        grouped.amount = 50.0;
        grouped.group = Some(0);
        let mut ungrouped = grouped.clone();
        ungrouped.group = None;
        ungrouped.adjustments.exposure = 0.25;
        ungrouped.amount = 100.0;
        let masks = [grouped, ungrouped];
        let mut groups = vec![MaskGroup::new("People")];
        groups[0].amount = 25.0;
        let local = LocalAdjust::new(&masks, &groups, &sensor, positions).expect("active");
        // 2 stops × 50 % × 25 %, plus the ungrouped quarter stop.
        assert_eq!(local.at(50, 50).exposure, 0.5);

        groups[0].enabled = false;
        let local = LocalAdjust::new(&masks, &groups, &sensor, positions).expect("active");
        assert_eq!(
            local.at(50, 50).exposure,
            0.25,
            "the hidden group adds nothing"
        );
        assert!(
            LocalAdjust::new(&masks[..1], &groups, &sensor, positions).is_none(),
            "a hidden group's masks cost no per-pixel work"
        );
        groups[0].enabled = true;
        groups[0].amount = 0.0;
        assert!(LocalAdjust::new(&masks[..1], &groups, &sensor, positions).is_none());
    }
}
