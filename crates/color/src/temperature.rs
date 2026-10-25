// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Correlated colour temperature and tint.
//!
//! Tint follows the photo-editor convention: positive is magenta (below the
//! Planckian locus), negative is green. One tint unit is `1 / TINT_SCALE` of
//! Δuv in CIE 1960 UCS.

use crate::Xy;

pub const TINT_SCALE: f64 = 3000.0;

const T_MIN: f64 = 1667.0;
const T_MAX: f64 = 25000.0;

/// Chromaticity of a Planckian radiator (Kim et al. cubic spline, 1667–25000 K).
pub fn cct_to_xy(t: f64) -> Xy {
    let t = t.clamp(T_MIN, T_MAX);
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.2661239e9 / t3 - 0.2343589e6 / t2 + 0.8776956e3 / t + 0.179910
    } else {
        -3.0258469e9 / t3 + 2.1070379e6 / t2 + 0.2226347e3 / t + 0.240390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.1063814 * x3 - 1.34811020 * x2 + 2.18555832 * x - 0.20219683
    } else if t <= 4000.0 {
        -0.9549476 * x3 - 1.37418593 * x2 + 2.09137015 * x - 0.16748867
    } else {
        3.0817580 * x3 - 5.87338670 * x2 + 3.75112997 * x - 0.37001483
    };
    [x, y]
}

fn xy_to_uv(xy: Xy) -> [f64; 2] {
    let d = -2.0 * xy[0] + 12.0 * xy[1] + 3.0;
    [4.0 * xy[0] / d, 6.0 * xy[1] / d]
}

fn uv_to_xy(uv: [f64; 2]) -> Xy {
    let d = 2.0 * uv[0] - 8.0 * uv[1] + 4.0;
    [3.0 * uv[0] / d, 2.0 * uv[1] / d]
}

/// Unit normal to the locus at `t`, pointing to the green side (+Δuv).
fn locus_normal(t: f64) -> [f64; 2] {
    let a = xy_to_uv(cct_to_xy(t * 0.99));
    let b = xy_to_uv(cct_to_xy(t * 1.01));
    let (du, dv) = (b[0] - a[0], b[1] - a[1]);
    let len = du.hypot(dv).max(1e-12);
    let n = [-dv / len, du / len];
    if n[1] < 0.0 { [-n[0], -n[1]] } else { n }
}

/// White point for a temperature (K) and tint.
pub fn temp_tint_to_xy(temp: f64, tint: f64) -> Xy {
    let t = temp.clamp(T_MIN, T_MAX);
    let uv = xy_to_uv(cct_to_xy(t));
    let n = locus_normal(t);
    let duv = -tint / TINT_SCALE;
    uv_to_xy([uv[0] + n[0] * duv, uv[1] + n[1] * duv])
}

/// Inverse of [`temp_tint_to_xy`]: nearest locus point by golden-section
/// search in mired space, tint from the signed distance.
pub fn xy_to_temp_tint(xy: Xy) -> (f64, f64) {
    let target = xy_to_uv(xy);
    let dist = |mired: f64| {
        let uv = xy_to_uv(cct_to_xy(1e6 / mired));
        (uv[0] - target[0]).hypot(uv[1] - target[1])
    };
    let (mut lo, mut hi) = (1e6 / T_MAX, 1e6 / T_MIN);
    let phi = (5f64.sqrt() - 1.0) / 2.0;
    for _ in 0..80 {
        let a = hi - phi * (hi - lo);
        let b = lo + phi * (hi - lo);
        if dist(a) < dist(b) {
            hi = b;
        } else {
            lo = a;
        }
    }
    let t = 1e6 / ((lo + hi) / 2.0);
    let uv = xy_to_uv(cct_to_xy(t));
    let n = locus_normal(t);
    let duv = (target[0] - uv[0]) * n[0] + (target[1] - uv[1]) * n[1];
    (t, -duv * TINT_SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d65_is_near_6500k() {
        let (t, tint) = xy_to_temp_tint(crate::D65);
        assert!((t - 6504.0).abs() < 60.0, "{t}");
        assert!(tint.abs() < 15.0, "{tint}");
    }

    #[test]
    fn round_trip() {
        for temp in [2500.0, 3200.0, 4500.0, 5500.0, 6500.0, 9000.0] {
            for tint in [-40.0, 0.0, 25.0] {
                let (t, ti) = xy_to_temp_tint(temp_tint_to_xy(temp, tint));
                assert!((t - temp).abs() / temp < 0.005, "{temp} {tint} -> {t}");
                assert!((ti - tint).abs() < 0.5, "{temp} {tint} -> {ti}");
            }
        }
    }

    #[test]
    fn positive_tint_is_magenta() {
        // Magenta has less green: y drops below the locus.
        let neutral = temp_tint_to_xy(5000.0, 0.0);
        let magenta = temp_tint_to_xy(5000.0, 50.0);
        assert!(magenta[1] < neutral[1]);
    }
}
