// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Curve construction: monotone splines through control points and the
//! parametric (region) tone curve. Curves operate on perceptual values
//! (display-linear raised to 1/2.2), like Lightroom's tone curve panel.

use photonation_develop::{ParametricCurve, PointCurve};

/// Monotone cubic (Fritsch–Carlson) interpolation through sorted points.
/// Monotone data never overshoots, so a well-behaved curve stays well-behaved.
pub struct Spline {
    xs: Vec<f32>,
    ys: Vec<f32>,
    slopes: Vec<f32>,
}

impl Spline {
    pub fn new(curve: &PointCurve) -> Spline {
        let points = curve.sanitized().0;
        let xs: Vec<f32> = points.iter().map(|p| p[0]).collect();
        let ys: Vec<f32> = points.iter().map(|p| p[1]).collect();
        let n = xs.len();
        let secants: Vec<f32> = (0..n - 1)
            .map(|i| (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]))
            .collect();
        let mut slopes = vec![0.0; n];
        slopes[0] = secants[0];
        slopes[n - 1] = secants[n - 2];
        for i in 1..n - 1 {
            slopes[i] = if secants[i - 1] * secants[i] <= 0.0 {
                0.0
            } else {
                (secants[i - 1] + secants[i]) / 2.0
            };
        }
        // Limit slopes so each segment stays monotone.
        for i in 0..n - 1 {
            if secants[i] == 0.0 {
                slopes[i] = 0.0;
                slopes[i + 1] = 0.0;
                continue;
            }
            let (a, b) = (slopes[i] / secants[i], slopes[i + 1] / secants[i]);
            let magnitude = a.hypot(b);
            if magnitude > 3.0 {
                let scale = 3.0 / magnitude;
                slopes[i] = scale * a * secants[i];
                slopes[i + 1] = scale * b * secants[i];
            }
        }
        Spline { xs, ys, slopes }
    }

    pub fn eval(&self, x: f32) -> f32 {
        let last = self.xs.len() - 1;
        if x <= self.xs[0] {
            return self.ys[0];
        }
        if x >= self.xs[last] {
            return self.ys[last];
        }
        let segment = self.xs.partition_point(|&knot| knot <= x) - 1;
        let width = self.xs[segment + 1] - self.xs[segment];
        let t = (x - self.xs[segment]) / width;
        let (t2, t3) = (t * t, t * t * t);
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        (h00 * self.ys[segment]
            + h10 * width * self.slopes[segment]
            + h01 * self.ys[segment + 1]
            + h11 * width * self.slopes[segment + 1])
            .clamp(0.0, 1.0)
    }
}

/// Lightroom-style region curve: each slider bends the curve inside its
/// tonal region (bounded by the split points); endpoints stay fixed.
pub struct Parametric {
    /// `(center, half_width, strength)` per region.
    regions: [(f32, f32, f32); 4],
}

/// Largest shift one region can apply at ±100, in perceptual units.
const REGION_STRENGTH: f32 = 0.18;

impl Parametric {
    pub fn new(p: &ParametricCurve) -> Parametric {
        let splits = [
            0.0,
            p.shadow_split / 100.0,
            p.midtone_split / 100.0,
            p.highlight_split / 100.0,
            1.0,
        ];
        let amounts = [p.shadows, p.darks, p.lights, p.highlights];
        let regions = std::array::from_fn(|i| {
            let (lo, hi) = (splits[i], splits[i + 1]);
            (
                (lo + hi) / 2.0,
                (hi - lo).max(0.05),
                amounts[i] / 100.0 * REGION_STRENGTH,
            )
        });
        Parametric { regions }
    }

    pub fn eval(&self, v: f32) -> f32 {
        // Envelope pins the endpoints so black and white never move.
        let envelope = (4.0 * v * (1.0 - v)).clamp(0.0, 1.0);
        let shift: f32 = self
            .regions
            .iter()
            .map(|&(center, half_width, strength)| {
                let distance = ((v - center) / half_width).abs();
                let bump = if distance < 1.0 {
                    (1.0 - distance * distance).powi(2)
                } else {
                    0.0
                };
                strength * bump
            })
            .sum();
        (v + shift * envelope).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_spline() {
        let s = Spline::new(&PointCurve::default());
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            assert!((s.eval(x) - x).abs() < 1e-6);
        }
    }

    #[test]
    fn spline_is_monotone_and_hits_points() {
        let s = Spline::new(&PointCurve(vec![
            [0.0, 0.0],
            [0.25, 0.15],
            [0.5, 0.55],
            [0.75, 0.9],
            [1.0, 1.0],
        ]));
        assert!((s.eval(0.5) - 0.55).abs() < 1e-6);
        let mut prev = 0.0;
        for i in 0..=1000 {
            let y = s.eval(i as f32 / 1000.0);
            assert!(y >= prev - 1e-6);
            prev = y;
        }
    }

    #[test]
    fn parametric_moves_only_its_region_and_keeps_endpoints() {
        let p = Parametric::new(&ParametricCurve {
            shadows: 100.0,
            ..ParametricCurve::default()
        });
        assert_eq!(p.eval(0.0), 0.0);
        assert_eq!(p.eval(1.0), 1.0);
        assert!(p.eval(0.12) > 0.13);
        assert!((p.eval(0.9) - 0.9).abs() < 1e-6);
    }
}
