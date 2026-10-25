// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::{Mat3, Transfer};
use serde::{Deserialize, Serialize};

/// CIE 1931 chromaticity.
pub type Xy = [f64; 2];

pub const D65: Xy = [0.3127, 0.3290];
pub const D50: Xy = [0.3457, 0.3585];

/// XYZ with Y = 1 for a chromaticity.
pub fn xy_to_xyz(xy: Xy) -> [f64; 3] {
    [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primaries {
    pub r: Xy,
    pub g: Xy,
    pub b: Xy,
    pub white: Xy,
}

impl Primaries {
    pub const REC709: Primaries = Primaries {
        r: [0.64, 0.33],
        g: [0.30, 0.60],
        b: [0.15, 0.06],
        white: D65,
    };
    pub const DISPLAY_P3: Primaries = Primaries {
        r: [0.680, 0.320],
        g: [0.265, 0.690],
        b: [0.150, 0.060],
        white: D65,
    };
    pub const ADOBE_RGB: Primaries = Primaries {
        r: [0.64, 0.33],
        g: [0.21, 0.71],
        b: [0.15, 0.06],
        white: D65,
    };
    pub const REC2020: Primaries = Primaries {
        r: [0.708, 0.292],
        g: [0.170, 0.797],
        b: [0.131, 0.046],
        white: D65,
    };
    pub const PROPHOTO: Primaries = Primaries {
        r: [0.734699, 0.265301],
        g: [0.159597, 0.840403],
        b: [0.036598, 0.000105],
        white: D50,
    };

    /// Linear RGB → XYZ (relative to this space's own white, Y_white = 1).
    pub fn to_xyz(&self) -> Mat3 {
        let p = Mat3::from_cols(xy_to_xyz(self.r), xy_to_xyz(self.g), xy_to_xyz(self.b));
        // Primaries are linearly independent for every space defined here.
        let s = p
            .inverse()
            .unwrap_or(Mat3::IDENTITY)
            .apply(xy_to_xyz(self.white));
        p * Mat3::diag(s)
    }

    pub fn from_xyz(&self) -> Mat3 {
        self.to_xyz().inverse().unwrap_or(Mat3::IDENTITY)
    }

    /// Luminance weights (the Y row of [`Primaries::to_xyz`]).
    pub fn luminance(&self) -> [f64; 3] {
        self.to_xyz().0[1]
    }
}

/// Bradford chromatic adaptation between two whites given as XYZ.
pub fn bradford(src_white: [f64; 3], dst_white: [f64; 3]) -> Mat3 {
    const M: Mat3 = Mat3([
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ]);
    let m_inv = M.inverse().unwrap_or(Mat3::IDENTITY);
    let s = M.apply(src_white);
    let d = M.apply(dst_white);
    m_inv * Mat3::diag([d[0] / s[0], d[1] / s[1], d[2] / s[2]]) * M
}

/// Output (display-referred) colour spaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ColorSpace {
    #[default]
    Srgb,
    DisplayP3,
    AdobeRgb,
    ProphotoRgb,
}

impl ColorSpace {
    pub const ALL: [ColorSpace; 4] = [
        ColorSpace::Srgb,
        ColorSpace::DisplayP3,
        ColorSpace::AdobeRgb,
        ColorSpace::ProphotoRgb,
    ];

    pub fn primaries(self) -> Primaries {
        match self {
            ColorSpace::Srgb => Primaries::REC709,
            ColorSpace::DisplayP3 => Primaries::DISPLAY_P3,
            ColorSpace::AdobeRgb => Primaries::ADOBE_RGB,
            ColorSpace::ProphotoRgb => Primaries::PROPHOTO,
        }
    }

    /// Encoding curve; matches the ICC profiles written by `photonation-export`.
    pub fn transfer(self) -> Transfer {
        match self {
            ColorSpace::Srgb | ColorSpace::DisplayP3 => Transfer::Srgb,
            ColorSpace::AdobeRgb => Transfer::Gamma(563.0 / 256.0),
            ColorSpace::ProphotoRgb => Transfer::Gamma(1.8),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ColorSpace::Srgb => "sRGB",
            ColorSpace::DisplayP3 => "Display P3",
            ColorSpace::AdobeRgb => "Adobe RGB (1998)",
            ColorSpace::ProphotoRgb => "ProPhoto RGB",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_matrix_matches_reference() {
        let m = Primaries::REC709.to_xyz();
        let want = [
            [0.4124, 0.3576, 0.1805],
            [0.2126, 0.7152, 0.0722],
            [0.0193, 0.1192, 0.9505],
        ];
        for (got_row, want_row) in m.0.iter().zip(want) {
            for (got, want) in got_row.iter().zip(want_row) {
                assert!((got - want).abs() < 2e-4, "{got} vs {want}");
            }
        }
    }

    #[test]
    fn bradford_same_white_is_identity() {
        let w = xy_to_xyz(D65);
        let m = bradford(w, w);
        for r in 0..3 {
            for c in 0..3 {
                let want = if r == c { 1.0 } else { 0.0 };
                assert!((m.0[r][c] - want).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn bradford_maps_white_to_white() {
        let (a, b) = (xy_to_xyz(D50), xy_to_xyz(D65));
        let got = bradford(a, b).apply(a);
        for i in 0..3 {
            assert!((got[i] - b[i]).abs() < 1e-9);
        }
    }
}
