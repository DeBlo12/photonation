// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Masks and local adjustments (docs/PIPELINE.md §5). A mask's weight at a
//! pixel combines its shapes' weights in order ([`MaskWeight`]); it scales
//! the mask's adjustments times its amount. The weighted adjustments of all
//! masks add up, and the develop stage applies the sum with the global ones.
//!
//! [`MaskWeight::at`] is the reference for the mask overlay the desktop
//! editor draws on the GPU (`apps/desktop/src/gpu/mask_overlay.wgsl`);
//! keep the two in step.

use crate::color::smoothstep;
use photonation_develop::{CombineMode, LocalAdjustments, Mask, MaskShape, MaskSpace};
use photonation_image::{Rect, SensorImage};

/// The mask space of a photo: its default crop in display orientation.
pub fn mask_space(sensor: &SensorImage) -> MaskSpace {
    let (width, height) = sensor
        .orientation
        .display_size(sensor.crop.w, sensor.crop.h);
    MaskSpace::new(width as f64, height as f64)
}

/// A mask shape prepared for evaluation at metric points of its
/// [`MaskSpace`].
#[derive(Clone, Copy, Debug, PartialEq)]
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
    origin: [f32; 2],
    step_x: [f32; 2],
    step_y: [f32; 2],
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
    /// `None` when no mask is enabled and adjusts something, so renders
    /// without masks skip the per-pixel work entirely.
    pub fn new(
        masks: &[Mask],
        sensor: &SensorImage,
        positions: PixelToMask,
    ) -> Option<LocalAdjust> {
        let space = mask_space(sensor);
        let masks: Vec<WeightedMask> = masks
            .iter()
            .filter(|mask| mask.is_active())
            .map(|mask| {
                let full = LocalAmounts::from_adjustments(&mask.adjustments);
                let mut amounts = LocalAmounts::NONE;
                amounts.add_weighted(&full, mask.amount / 100.0);
                WeightedMask {
                    weight: MaskWeight::new(mask, space),
                    amounts,
                }
            })
            .collect();
        (!masks.is_empty()).then_some(LocalAdjust { masks, positions })
    }

    /// The summed adjustments at stage pixel `(x, y)`.
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> LocalAmounts {
        let point = self.positions.at(x, y);
        let mut amounts = LocalAmounts::NONE;
        for mask in &self.masks {
            let weight = mask.weight.at(point);
            if weight > 0.0 {
                amounts.add_weighted(&mask.amounts, weight);
            }
        }
        amounts
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
        let local = LocalAdjust::new(&[brighten, darken_outside, disabled], &sensor, positions)
            .expect("active masks");
        assert_eq!(local.at(50, 50).exposure, 1.0);
        assert_eq!(local.at(2, 2).exposure, -2.0);

        let idle = Mask::new("Nothing to do", MaskShape::default());
        assert!(LocalAdjust::new(&[idle], &sensor, positions).is_none());
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
        let local = LocalAdjust::new(&[mask], &sensor, positions).expect("active");
        let centre = local.at(50, 50);
        assert_eq!((centre.exposure, centre.contrast), (0.5, 0.125));
    }
}
