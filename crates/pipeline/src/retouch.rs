// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Removals (docs/RETOUCH.md): brushed areas replaced by a fill.
//!
//! Filling is non-local (the content comes from elsewhere in the photo or
//! from a neural network), so it cannot run band by band. It runs once per
//! removal, outside the render:
//!
//! ```text
//! removal_canvas   sensor area around the brush → display-like RGB + hole
//! Inpainter        (photonation-retouch) fills the hole
//! RemovalCanvas::patch   filled RGB → linear camera RGB before white balance
//! ```
//!
//! The resulting [`RetouchPatch`]es are cached by the caller, keyed by the
//! removal chain's fingerprints, and every render blends them in with the
//! [`Retouch`] point stage right after the source, so all adjustments
//! (white balance included) apply to the fill exactly as to the rest of the
//! photo, and a slider drag never re-runs a fill.

use crate::band::{Graph, Point, PointOp};
use crate::masks::{PixelToMask, mask_space};
use crate::resize::Resize;
use crate::source::{Demosaic, SensorSource};
use crate::{CameraModel, WORKING};
use photonation_color::{Mat3, Primaries, apply3};
use photonation_develop::{MaskSpace, Removal, Settings, WhiteBalance, removal_chain_fingerprints};
use photonation_image::{NormRect, Rect, SensorImage};
use photonation_retouch::Canvas;
use std::collections::HashMap;
use std::sync::Arc;

/// Longest edge of a removal's canvas. Larger areas are filled at this
/// resolution and scaled up.
pub const MAX_CANVAS_EDGE: usize = 1536;
/// Context around the brushed area, as a multiple of its larger side, so
/// the fill has something to match.
const CONTEXT_MARGIN: f64 = 1.0;
/// Display gamma of canvases (only the inpainter sees it).
const CANVAS_GAMMA: f32 = 2.2;

/// Fills keyed by removal chain fingerprint
/// ([`photonation_develop::removal_chain_fingerprints`] of the sanitized
/// removals).
pub type RemovalFills = HashMap<u64, Arc<RetouchPatch>>;

/// A filled area: linear camera RGB before white balance (1.0 = sensor
/// clip), interleaved, covering `sensor_rect` of the stored mosaic.
#[derive(Clone, Debug, PartialEq)]
pub struct RetouchPatch {
    pub sensor_rect: Rect,
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<f32>,
}

impl RetouchPatch {
    /// Bilinear sample at a sensor position (pixel-centre convention).
    #[inline]
    fn sample(&self, sensor_x: f32, sensor_y: f32) -> [f32; 3] {
        let rect = self.sensor_rect;
        let u = ((sensor_x - rect.x as f32) / rect.w as f32 * self.width as f32 - 0.5)
            .clamp(0.0, (self.width - 1) as f32);
        let v = ((sensor_y - rect.y as f32) / rect.h as f32 * self.height as f32 - 0.5)
            .clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (u as usize, v as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (u - x0 as f32, v - y0 as f32);
        let at = |x: usize, y: usize, channel: usize| self.rgb[(y * self.width + x) * 3 + channel];
        std::array::from_fn(|channel| {
            let top = at(x0, y0, channel) + (at(x1, y0, channel) - at(x0, y0, channel)) * fx;
            let bottom = at(x0, y1, channel) + (at(x1, y1, channel) - at(x0, y1, channel)) * fx;
            top + (bottom - top) * fy
        })
    }
}

/// A removal's brushed area, prepared for evaluation at metric points of
/// its [`MaskSpace`]: full weight within `radius · (1 − feather)` of a
/// stroke, fading to none at the radius.
#[derive(Clone, Debug, PartialEq)]
pub struct RemovalWeight {
    strokes: Vec<StrokeWeight>,
    /// Metric bounds `[left, top, right, bottom]`; zero weight outside.
    bounds: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
struct StrokeWeight {
    points: Vec<[f32; 2]>,
    radius: f32,
    fade_start: f32,
}

impl RemovalWeight {
    pub fn new(removal: &Removal, space: MaskSpace) -> RemovalWeight {
        let fade = removal.feather / 100.0;
        let strokes: Vec<StrokeWeight> = removal
            .strokes
            .iter()
            .filter(|stroke| !stroke.points.is_empty())
            .map(|stroke| StrokeWeight {
                points: stroke
                    .points
                    .iter()
                    .map(|point| space.to_metric(*point).map(|v| v as f32))
                    .collect(),
                radius: stroke.radius as f32,
                fade_start: stroke.radius as f32 * (1.0 - fade),
            })
            .collect();
        let mut bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for stroke in &strokes {
            for point in &stroke.points {
                bounds[0] = bounds[0].min(point[0] - stroke.radius);
                bounds[1] = bounds[1].min(point[1] - stroke.radius);
                bounds[2] = bounds[2].max(point[0] + stroke.radius);
                bounds[3] = bounds[3].max(point[1] + stroke.radius);
            }
        }
        RemovalWeight { strokes, bounds }
    }

    /// Removal strength, 0..1, at a metric point.
    #[inline]
    pub fn at(&self, point: [f32; 2]) -> f32 {
        let [left, top, right, bottom] = self.bounds;
        if point[0] < left || point[0] > right || point[1] < top || point[1] > bottom {
            return 0.0;
        }
        let mut weight: f32 = 0.0;
        for stroke in &self.strokes {
            let distance = distance_to_polyline(point, &stroke.points);
            let stroke_weight = if distance <= stroke.fade_start {
                1.0
            } else if distance >= stroke.radius {
                0.0
            } else {
                1.0 - crate::color::smoothstep(stroke.fade_start, stroke.radius, distance)
            };
            weight = weight.max(stroke_weight);
            if weight >= 1.0 {
                break;
            }
        }
        weight
    }
}

fn distance_to_polyline(point: [f32; 2], points: &[[f32; 2]]) -> f32 {
    if points.len() == 1 {
        return (point[0] - points[0][0]).hypot(point[1] - points[0][1]);
    }
    points
        .windows(2)
        .map(|segment| {
            let (a, b) = (segment[0], segment[1]);
            let direction = [b[0] - a[0], b[1] - a[1]];
            let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
            let along = if length_squared > 0.0 {
                (((point[0] - a[0]) * direction[0] + (point[1] - a[1]) * direction[1])
                    / length_squared)
                    .clamp(0.0, 1.0)
            } else {
                0.0
            };
            let closest = [a[0] + along * direction[0], a[1] + along * direction[1]];
            (point[0] - closest[0]).hypot(point[1] - closest[1])
        })
        .fold(f32::INFINITY, f32::min)
}

/// One removal ready to blend: where it applies and what goes there.
struct AppliedRemoval {
    weight: RemovalWeight,
    patch: Arc<RetouchPatch>,
}

/// The point stage blending fills into the white-balanced camera RGB the
/// source produces.
pub struct Retouch {
    removals: Vec<AppliedRemoval>,
    positions: PixelToMask,
    /// Stage pixel → sensor position: `origin + (x + 0.5) · step`.
    origin: [f32; 2],
    step: [f32; 2],
    wb_multipliers: [f32; 3],
}

impl Retouch {
    /// The stage for the removals `0..count` of `removals` that have a
    /// fill in `fills`, or `None` when nothing applies. `region` is the
    /// sensor area the source reads and `size` the stage's pixel grid.
    fn new(
        sensor: &SensorImage,
        removals: &[Removal],
        fills: &RemovalFills,
        region: Rect,
        size: (usize, usize),
        wb_multipliers: [f32; 3],
    ) -> Option<Retouch> {
        let space = mask_space(sensor);
        let applied: Vec<AppliedRemoval> = removals
            .iter()
            .zip(removal_chain_fingerprints(removals))
            .filter_map(|(removal, key)| {
                let patch = fills.get(&key?)?.clone();
                Some(AppliedRemoval {
                    weight: RemovalWeight::new(removal, space),
                    patch,
                })
            })
            .collect();
        if applied.is_empty() {
            return None;
        }
        Some(Retouch {
            removals: applied,
            positions: PixelToMask::new(sensor, region, size),
            origin: [region.x as f32, region.y as f32],
            step: [
                region.w as f32 / size.0.max(1) as f32,
                region.h as f32 / size.1.max(1) as f32,
            ],
            wb_multipliers,
        })
    }
}

impl PointOp for Retouch {
    fn apply(&self, y: usize, row: &mut [f32]) {
        let sensor_y = self.origin[1] + (y as f32 + 0.5) * self.step[1];
        for (x, pixel) in row.chunks_exact_mut(3).enumerate() {
            let point = self.positions.at(x, y);
            for removal in &self.removals {
                let weight = removal.weight.at(point);
                if weight <= 0.0 {
                    continue;
                }
                let sensor_x = self.origin[0] + (x as f32 + 0.5) * self.step[0];
                let fill = removal.patch.sample(sensor_x, sensor_y);
                for channel in 0..3 {
                    // White balance and clip exactly as the source does.
                    let value = (fill[channel] * self.wb_multipliers[channel]).min(1.0);
                    pixel[channel] += (value - pixel[channel]) * weight;
                }
            }
        }
    }
}

/// Adds the retouch stage to a graph that holds only its source.
pub(crate) fn push_retouch(
    graph: &mut Graph<'_>,
    sensor: &SensorImage,
    settings: &Settings,
    fills: &RemovalFills,
    source_region: Rect,
    wb_multipliers: [f32; 3],
) {
    if fills.is_empty() {
        return;
    }
    if let Some(stage) = Retouch::new(
        sensor,
        &settings.removals,
        fills,
        source_region,
        graph.size(),
        wb_multipliers,
    ) {
        graph.push(Box::new(Point(stage)));
    }
}

/// A removal still to fill: its index in the (sanitized) settings and the
/// key its fill is cached under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemovalJob {
    pub index: usize,
    pub key: u64,
}

/// The active removals of `settings`, in order. Fill them in this order:
/// each one's canvas includes the fills before it.
pub fn removal_jobs(settings: &Settings) -> Vec<RemovalJob> {
    let settings = settings.sanitized();
    removal_chain_fingerprints(&settings.removals)
        .into_iter()
        .enumerate()
        .filter_map(|(index, key)| Some(RemovalJob { index, key: key? }))
        .collect()
}

/// What an inpainter sees of one removal, and how to turn its answer back
/// into a [`RetouchPatch`].
#[derive(Clone, Debug)]
pub struct RemovalCanvas {
    pub canvas: Canvas,
    sensor_rect: Rect,
    /// As-shot white balance, so the canvas looks natural to a network.
    display_balance: [f32; 3],
    /// Linear sRGB → white-balanced camera RGB: the canvas is sRGB because
    /// neural inpainters were trained on ordinary photos, and camera
    /// colours make them paint odd hues.
    from_display: [[f32; 3]; 3],
    /// Exposure that puts the context's highlights near 1.
    display_gain: f32,
}

impl RemovalCanvas {
    /// The patch for a filled canvas (`filled` as returned by an
    /// `Inpainter`).
    pub fn patch(&self, filled: &[f32]) -> RetouchPatch {
        let rgb = filled
            .chunks_exact(3)
            .flat_map(|pixel| {
                let linear = std::array::from_fn(|channel| {
                    pixel[channel].clamp(0.0, 1.0).powf(CANVAS_GAMMA) / self.display_gain
                });
                let balanced = apply3(&self.from_display, linear);
                (0..3).map(move |channel| balanced[channel] / self.display_balance[channel])
            })
            .collect();
        RetouchPatch {
            sensor_rect: self.sensor_rect,
            width: self.canvas.width,
            height: self.canvas.height,
            rgb,
        }
    }
}

/// Cuts the canvas for removal `index` of `settings` (sanitized here):
/// the sensor area around the brush plus context, rendered without any
/// adjustment but with the earlier removals' `fills` applied, white
/// balanced as shot, converted to sRGB, brightened to use the full range
/// and gamma encoded. `None` when the removal is inactive or entirely off
/// the photo.
pub fn removal_canvas(
    sensor: &SensorImage,
    settings: &Settings,
    index: usize,
    fills: &RemovalFills,
) -> Option<RemovalCanvas> {
    let settings = settings.sanitized();
    let removal = settings.removals.get(index).filter(|r| r.is_active())?;
    let space = mask_space(sensor);
    let [left, top, right, bottom] = removal.bounds(space)?;
    // The margin is measured in metric units (so it is the same on both
    // axes), then converted back to each axis's normalized units.
    let scale = space.scale();
    let margin = ((right - left) * scale[0]).max((bottom - top) * scale[1]) * CONTEXT_MARGIN;
    let (margin_x, margin_y) = (margin / scale[0], margin / scale[1]);
    let context = NormRect {
        left: left - margin_x,
        top: top - margin_y,
        right: right + margin_x,
        bottom: bottom + margin_y,
    }
    .clamped();
    if context.right <= context.left || context.bottom <= context.top {
        return None;
    }
    let region = sensor
        .crop
        .sub(sensor.orientation.display_rect_to_stored(context));
    if region.w < 2 || region.h < 2 {
        return None;
    }

    let longest = region.w.max(region.h);
    let target = if longest > MAX_CANVAS_EDGE {
        let shrink = MAX_CANVAS_EDGE as f64 / longest as f64;
        (
            ((region.w as f64 * shrink).round() as usize).max(2),
            ((region.h as f64 * shrink).round() as usize).max(2),
        )
    } else {
        (region.w, region.h)
    };
    let half_size_is_enough = target.0 * 2 <= region.w && target.1 * 2 <= region.h;
    let demosaic = Demosaic::choose(&sensor.layout, half_size_is_enough);
    // Demosaic white balanced as shot, like a render: demosaicing does not
    // commute with per-channel scaling, so balancing afterwards would not
    // match the photo. The patch divides the balance back out.
    let display_balance = CameraModel::new(sensor, &WhiteBalance::AsShot).wb_multipliers;
    let source = SensorSource::new(sensor, region, display_balance, demosaic);
    let source_region = source.region();
    let mut graph = Graph::new(Box::new(source));
    let earlier = Settings {
        removals: settings.removals[..index].to_vec(),
        ..Settings::default()
    };
    push_retouch(
        &mut graph,
        sensor,
        &earlier,
        fills,
        source_region,
        display_balance,
    );
    if graph.size() != target {
        graph.push(Box::new(Resize::new(graph.size(), target)));
    }
    let (width, height) = graph.size();
    let balanced = graph.render_rows(0..height).data;

    let positions = PixelToMask::new(sensor, source_region, (width, height));
    let weight = RemovalWeight::new(removal, space);
    let hole: Vec<f32> = (0..width * height)
        .map(|pixel| weight.at(positions.at(pixel % width, pixel / width)))
        .collect();
    let camera = CameraModel::new(sensor, &WhiteBalance::AsShot);
    let to_display = Primaries::REC709.from_xyz() * WORKING.to_xyz() * camera.camera_to_working;
    let from_display = to_display.inverse().unwrap_or(to_display);
    let as_f32 = |matrix: Mat3| matrix.0.map(|row| row.map(|v| v as f32));
    let (to_display, from_display) = (as_f32(to_display), as_f32(from_display));
    let display: Vec<f32> = balanced
        .chunks_exact(3)
        .flat_map(|pixel| apply3(&to_display, [pixel[0], pixel[1], pixel[2]]))
        .collect();
    let display_gain = highlight_gain(&display, &hole);
    let rgb = display
        .iter()
        .map(|value| {
            (value * display_gain)
                .clamp(0.0, 1.0)
                .powf(1.0 / CANVAS_GAMMA)
        })
        .collect();
    Some(RemovalCanvas {
        canvas: Canvas {
            width,
            height,
            rgb,
            hole,
        },
        sensor_rect: source_region,
        display_balance,
        from_display,
        display_gain,
    })
}

/// Gain that maps the 99th percentile of the known pixels' brightest
/// channel to 0.95, so the canvas uses the inpainter's range without
/// clipping much of the context.
fn highlight_gain(display: &[f32], hole: &[f32]) -> f32 {
    let mut brightest: Vec<f32> = display
        .chunks_exact(3)
        .zip(hole)
        .filter(|(_, weight)| **weight <= 0.0)
        .map(|(pixel, _)| pixel.iter().copied().fold(0.0, f32::max))
        .collect();
    if brightest.is_empty() {
        return 1.0;
    }
    let rank = (brightest.len() * 99 / 100).min(brightest.len() - 1);
    let percentile = *brightest.select_nth_unstable_by(rank, f32::total_cmp).1;
    (0.95 / percentile.max(1e-4)).clamp(0.25, 256.0)
}
