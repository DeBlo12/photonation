// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Display encoding (OETF) through a dense lookup table.

use crate::band::PointOp;
use photonation_color::ColorSpace;

const LUT_SIZE: usize = 1 << 16;

pub struct Encode {
    lut: Vec<f32>,
}

impl Encode {
    pub fn new(space: ColorSpace) -> Encode {
        let transfer = space.transfer();
        let lut = (0..=LUT_SIZE)
            .map(|i| transfer.encode(i as f64 / LUT_SIZE as f64) as f32)
            .collect();
        Encode { lut }
    }

    /// The transfer curve at `i / LUT_SIZE`, `LUT_SIZE + 1` entries.
    pub fn table(&self) -> &[f32] {
        &self.lut
    }
}

impl PointOp for Encode {
    fn apply(&self, _y: usize, values: &mut [f32]) {
        for value in values {
            let position = value.clamp(0.0, 1.0) * LUT_SIZE as f32;
            let index = (position as usize).min(LUT_SIZE - 1);
            let fraction = position - index as f32;
            *value = self.lut[index] + (self.lut[index + 1] - self.lut[index]) * fraction;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_transfer_function() {
        let encode = Encode::new(ColorSpace::Srgb);
        let mut values = [0.0, 0.001, 0.18, 0.5, 1.0, 2.0];
        encode.apply(0, &mut values);
        let transfer = ColorSpace::Srgb.transfer();
        for (got, linear) in values.iter().zip([0.0, 0.001, 0.18, 0.5, 1.0, 1.0]) {
            assert!((*got as f64 - transfer.encode(linear)).abs() < 1e-4);
        }
    }
}
