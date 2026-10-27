// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Point colours in Oklab: each selects the colours near its sampled
//! colour (by hue, chroma and lightness) and shifts them. Selections are
//! measured on the colour before any point colour applies, and their shifts
//! add up, so the order of the point colours does not matter.

use crate::color::smoothstep;
use photonation_develop::{OklabLch, PointColor};

/// Hue change at Hue Shift ±100, degrees.
pub const MAX_POINT_HUE_SHIFT: f32 = 60.0;
/// Lightness change at Luminance Shift ±100.
pub const MAX_POINT_LIGHTNESS: f32 = 0.3;
/// Hue distance (degrees) where the selection ends, at Hue Range 0 and 100.
const HUE_REACH: [f32; 2] = [8.0, 90.0];
/// Chroma distance where the selection ends, at Saturation Range 0 and 100.
const CHROMA_REACH: [f32; 2] = [0.02, 0.25];
/// Lightness distance where the selection ends, at Luminance Range 0 and 100.
const LIGHTNESS_REACH: [f32; 2] = [0.05, 0.7];
/// Full selection up to this share of the reach, then a smooth fade.
const SOLID_SHARE: f32 = 0.35;
/// Below this chroma a colour counts as grey: its hue is noise.
const GREY_CHROMA: f32 = 0.02;

fn lerp(range: [f32; 2], amount: f32) -> f32 {
    range[0] + (range[1] - range[0]) * amount
}

/// 1 within the solid part of `reach`, fading to 0 at `reach`.
#[inline(always)]
fn falloff(distance: f32, reach: f32) -> f32 {
    1.0 - smoothstep(reach * SOLID_SHARE, reach, distance)
}

/// One point colour, prepared for evaluation per pixel.
#[derive(Clone, Copy, Debug)]
pub struct PointSelection {
    target: OklabLch,
    /// Weight given to the hue distance: 0 for a grey target, whose hue
    /// means nothing.
    hue_relevance: f32,
    hue_reach: f32,
    chroma_reach: f32,
    lightness_reach: f32,
    /// Shifts at full selection.
    hue_shift: f32,
    chroma_gain: f32,
    lightness_shift: f32,
}

impl PointSelection {
    pub fn new(point: &PointColor) -> PointSelection {
        PointSelection {
            target: point.target,
            hue_relevance: smoothstep(0.0, GREY_CHROMA, point.target.chroma),
            hue_reach: lerp(HUE_REACH, point.hue_range / 100.0),
            chroma_reach: lerp(CHROMA_REACH, point.saturation_range / 100.0),
            lightness_reach: lerp(LIGHTNESS_REACH, point.luminance_range / 100.0),
            hue_shift: point.hue / 100.0 * MAX_POINT_HUE_SHIFT,
            chroma_gain: point.saturation / 100.0,
            lightness_shift: point.luminance / 100.0 * MAX_POINT_LIGHTNESS,
        }
    }

    /// How strongly a colour is selected, 0..1.
    #[inline]
    pub fn weight(&self, color: &OklabLch) -> f32 {
        let lightness = falloff(
            (color.lightness - self.target.lightness).abs(),
            self.lightness_reach,
        );
        let chroma = falloff((color.chroma - self.target.chroma).abs(), self.chroma_reach);
        if lightness == 0.0 || chroma == 0.0 {
            return 0.0;
        }
        let hue_distance = {
            let difference = (color.hue - self.target.hue).rem_euclid(360.0);
            difference.min(360.0 - difference)
        };
        // A grey pixel's hue is noise: for coloured targets it counts as
        // far away, so greys stay out of colour selections. Grey targets
        // ignore hue altogether.
        let pixel_hue_relevance = smoothstep(0.0, GREY_CHROMA, color.chroma);
        let hue = self.hue_relevance * pixel_hue_relevance * falloff(hue_distance, self.hue_reach)
            + (1.0 - self.hue_relevance);
        lightness * chroma * hue
    }
}

/// Every active point colour of a render.
#[derive(Clone, Debug)]
pub struct PointColors {
    selections: Vec<PointSelection>,
    /// Shows this point colour's selection instead of the edit: selected
    /// colours keep their colour, the rest turns grey.
    overlay: Option<PointSelection>,
}

impl PointColors {
    /// `None` when nothing would change: no active point colour and no
    /// selection to show. `overlay` indexes `points`.
    pub fn new(points: &[PointColor], overlay: Option<usize>) -> Option<PointColors> {
        let selections: Vec<PointSelection> = points
            .iter()
            .filter(|point| point.is_active())
            .map(PointSelection::new)
            .collect();
        let overlay = overlay
            .and_then(|index| points.get(index))
            .map(PointSelection::new);
        (!selections.is_empty() || overlay.is_some()).then_some(PointColors {
            selections,
            overlay,
        })
    }

    /// Applies the shifts to an Oklab colour.
    #[inline]
    pub fn apply(&self, lab: &mut [f32; 3]) {
        let original = OklabLch::from_lab(*lab);
        let mut color = original;
        for selection in &self.selections {
            let weight = selection.weight(&original);
            if weight > 0.0 {
                color.hue += selection.hue_shift * weight;
                color.chroma *= (1.0 + selection.chroma_gain * weight).max(0.0);
                color.lightness += selection.lightness_shift * weight;
            }
        }
        if let Some(overlay) = &self.overlay {
            color.chroma *= overlay.weight(&original);
        }
        *lab = color.to_lab();
    }
}

/// The colour a point colour turns its own target into (for swatches).
pub fn shifted_target(point: &PointColor) -> OklabLch {
    let mut lab = point.target.to_lab();
    PointColors {
        selections: vec![PointSelection::new(point)],
        overlay: None,
    }
    .apply(&mut lab);
    OklabLch::from_lab(lab)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKY: OklabLch = OklabLch {
        lightness: 0.7,
        chroma: 0.1,
        hue: 240.0,
    };

    fn point(adjust: impl FnOnce(&mut PointColor)) -> PointColor {
        let mut point = PointColor::new(SKY);
        adjust(&mut point);
        point
    }

    fn apply(points: &[PointColor], color: OklabLch) -> OklabLch {
        let mut lab = color.to_lab();
        PointColors::new(points, None)
            .expect("active")
            .apply(&mut lab);
        OklabLch::from_lab(lab)
    }

    #[test]
    fn untouched_points_skip_the_step() {
        assert!(PointColors::new(&[PointColor::new(SKY)], None).is_none());
        assert!(PointColors::new(&[PointColor::new(SKY)], Some(0)).is_some());
    }

    #[test]
    fn target_is_fully_selected_and_far_colours_not_at_all() {
        let selection = PointSelection::new(&PointColor::new(SKY));
        assert_eq!(selection.weight(&SKY), 1.0);
        let foliage = OklabLch { hue: 140.0, ..SKY };
        assert_eq!(selection.weight(&foliage), 0.0);
        let grey = OklabLch { chroma: 0.0, ..SKY };
        assert_eq!(selection.weight(&grey), 0.0);
    }

    #[test]
    fn wider_ranges_select_more() {
        let near = OklabLch { hue: 215.0, ..SKY };
        let narrow = PointSelection::new(&point(|p| p.hue_range = 10.0));
        let wide = PointSelection::new(&point(|p| p.hue_range = 90.0));
        assert!(wide.weight(&near) > narrow.weight(&near) + 0.3);
    }

    #[test]
    fn hue_wraps_around_red() {
        let red = OklabLch {
            lightness: 0.6,
            chroma: 0.15,
            hue: 2.0,
        };
        let selection = PointSelection::new(&PointColor::new(red));
        assert!(selection.weight(&OklabLch { hue: 358.0, ..red }) > 0.99);
    }

    #[test]
    fn shifts_move_the_target_colour() {
        let out = apply(&[point(|p| p.hue = 50.0)], SKY);
        assert!((out.hue - (SKY.hue + 30.0)).abs() < 0.01, "{out:?}");
        let out = apply(&[point(|p| p.saturation = -100.0)], SKY);
        assert!(out.chroma < 1e-6, "{out:?}");
        let out = apply(&[point(|p| p.luminance = 100.0)], SKY);
        assert!((out.lightness - (SKY.lightness + MAX_POINT_LIGHTNESS)).abs() < 1e-6);
    }

    #[test]
    fn grey_target_edits_greys_only() {
        let grey = OklabLch {
            lightness: 0.5,
            chroma: 0.0,
            hue: 0.0,
        };
        let darker = point(|p| {
            p.target = grey;
            p.luminance = -50.0;
        });
        assert!(apply(&[darker], grey).lightness < 0.4);
        let red = OklabLch {
            chroma: 0.2,
            hue: 30.0,
            ..grey
        };
        assert_eq!(apply(&[darker], red).lightness, red.lightness);
    }

    #[test]
    fn overlay_greys_out_unselected_colours() {
        let points = [PointColor::new(SKY)];
        let overlay = PointColors::new(&points, Some(0)).expect("overlay");
        let foliage = OklabLch { hue: 140.0, ..SKY };
        let mut lab = foliage.to_lab();
        overlay.apply(&mut lab);
        assert!(OklabLch::from_lab(lab).chroma < 1e-6);
        let mut lab = SKY.to_lab();
        overlay.apply(&mut lab);
        assert!((OklabLch::from_lab(lab).chroma - SKY.chroma).abs() < 1e-6);
    }

    #[test]
    fn shifted_target_previews_the_edit() {
        let warmer = shifted_target(&point(|p| p.hue = -100.0));
        assert!((warmer.hue - (SKY.hue - MAX_POINT_HUE_SHIFT)).abs() < 0.01);
    }
}
