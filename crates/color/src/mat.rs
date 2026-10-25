// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use std::ops::Mul;

/// Row-major 3×3 matrix used for colour conversions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3(pub [[f64; 3]; 3]);

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    pub fn diag(v: [f64; 3]) -> Mat3 {
        Mat3([[v[0], 0.0, 0.0], [0.0, v[1], 0.0], [0.0, 0.0, v[2]]])
    }

    pub fn from_cols(c0: [f64; 3], c1: [f64; 3], c2: [f64; 3]) -> Mat3 {
        Mat3([
            [c0[0], c1[0], c2[0]],
            [c0[1], c1[1], c2[1]],
            [c0[2], c1[2], c2[2]],
        ])
    }

    pub fn apply(&self, v: [f64; 3]) -> [f64; 3] {
        let m = &self.0;
        [
            m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
        ]
    }

    pub fn scale(&self, s: f64) -> Mat3 {
        let mut out = self.0;
        out.iter_mut().flatten().for_each(|x| *x *= s);
        Mat3(out)
    }

    pub fn lerp(&self, other: &Mat3, t: f64) -> Mat3 {
        let mut out = [[0.0; 3]; 3];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, x) in row.iter_mut().enumerate() {
                *x = self.0[r][c] * (1.0 - t) + other.0[r][c] * t;
            }
        }
        Mat3(out)
    }

    pub fn determinant(&self) -> f64 {
        let m = &self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// Inverse, or `None` when the matrix is (numerically) singular.
    pub fn inverse(&self) -> Option<Mat3> {
        let det = self.determinant();
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        let m = &self.0;
        let inv_det = 1.0 / det;
        let cof = |r0: usize, r1: usize, c0: usize, c1: usize| {
            m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
        };
        Some(Mat3([
            [
                cof(1, 2, 1, 2) * inv_det,
                -cof(0, 2, 1, 2) * inv_det,
                cof(0, 1, 1, 2) * inv_det,
            ],
            [
                -cof(1, 2, 0, 2) * inv_det,
                cof(0, 2, 0, 2) * inv_det,
                -cof(0, 1, 0, 2) * inv_det,
            ],
            [
                cof(1, 2, 0, 1) * inv_det,
                -cof(0, 2, 0, 1) * inv_det,
                cof(0, 1, 0, 1) * inv_det,
            ],
        ]))
    }

    pub fn to_f32(&self) -> [[f32; 3]; 3] {
        self.0.map(|row| row.map(|x| x as f32))
    }
}

impl Mul for Mat3 {
    type Output = Mat3;

    fn mul(self, rhs: Mat3) -> Mat3 {
        let mut out = [[0.0; 3]; 3];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, x) in row.iter_mut().enumerate() {
                *x = (0..3).map(|k| self.0[r][k] * rhs.0[k][c]).sum();
            }
        }
        Mat3(out)
    }
}

/// Per-pixel matrix application.
#[inline(always)]
pub fn apply3(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_round_trips() {
        let m = Mat3([
            [0.9079, -0.1923, -0.1236],
            [-0.4677, 1.2454, 0.2492],
            [-0.0922, 0.2319, 0.5565],
        ]);
        let id = m * m.inverse().expect("invertible");
        for r in 0..3 {
            for c in 0..3 {
                let want = if r == c { 1.0 } else { 0.0 };
                assert!((id.0[r][c] - want).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn singular_has_no_inverse() {
        assert!(
            Mat3([[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 0.0, 1.0]])
                .inverse()
                .is_none()
        );
    }
}
