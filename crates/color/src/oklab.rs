// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use crate::{Mat3, Primaries, apply3};

/// Oklab conversion for an arbitrary linear RGB working space.
#[derive(Clone, Debug)]
pub struct Oklab {
    to_lms: [[f32; 3]; 3],
    lms_to_lab: [[f32; 3]; 3],
    lab_to_lms: [[f32; 3]; 3],
    from_lms: [[f32; 3]; 3],
}

// Ottosson's matrices, defined on linear sRGB.
const M1_SRGB: Mat3 = Mat3([
    [0.4122214708, 0.5363325363, 0.0514459929],
    [0.2119034982, 0.6806995451, 0.1073969566],
    [0.0883024619, 0.2817188376, 0.6299787005],
]);
const M2: Mat3 = Mat3([
    [0.2104542553, 0.7936177850, -0.0040720468],
    [1.9779984951, -2.4285922050, 0.4505937099],
    [0.0259040371, 0.7827717662, -0.8086757660],
]);

impl Oklab {
    pub fn new(working: Primaries) -> Oklab {
        let working_to_srgb = Primaries::REC709.from_xyz() * working.to_xyz();
        let to_lms = M1_SRGB * working_to_srgb;
        Oklab {
            to_lms: to_lms.to_f32(),
            lms_to_lab: M2.to_f32(),
            lab_to_lms: M2.inverse().unwrap_or(Mat3::IDENTITY).to_f32(),
            from_lms: to_lms.inverse().unwrap_or(Mat3::IDENTITY).to_f32(),
        }
    }

    /// RGB → LMS, LMS′ → Lab, Lab → LMS′ and LMS → RGB (LMS′ is the
    /// cube root of LMS), for other implementations of the conversion.
    pub fn matrices(&self) -> [[[f32; 3]; 3]; 4] {
        [self.to_lms, self.lms_to_lab, self.lab_to_lms, self.from_lms]
    }

    #[inline]
    pub fn from_rgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        let lms = apply3(&self.to_lms, rgb).map(f32::cbrt);
        apply3(&self.lms_to_lab, lms)
    }

    #[inline]
    pub fn to_rgb(&self, lab: [f32; 3]) -> [f32; 3] {
        let lms = apply3(&self.lab_to_lms, lab).map(|v| v * v * v);
        apply3(&self.from_lms, lms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_has_no_chroma() {
        let ok = Oklab::new(Primaries::REC2020);
        let lab = ok.from_rgb([1.0, 1.0, 1.0]);
        assert!(
            (lab[0] - 1.0).abs() < 1e-3 && lab[1].abs() < 1e-3 && lab[2].abs() < 1e-3,
            "{lab:?}"
        );
    }

    #[test]
    fn round_trip() {
        let ok = Oklab::new(Primaries::REC2020);
        let rgb = [0.3, 0.6, 0.1];
        let back = ok.to_rgb(ok.from_rgb(rgb));
        for i in 0..3 {
            assert!((back[i] - rgb[i]).abs() < 1e-4);
        }
    }
}
