// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

/// Display encoding curves (OETF) and their inverses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transfer {
    Linear,
    /// IEC 61966-2-1 piecewise curve.
    Srgb,
    /// Pure power law, `encoded = linear^(1/gamma)`.
    Gamma(f64),
}

impl Transfer {
    pub fn encode(self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Transfer::Linear => x,
            Transfer::Srgb => {
                if x <= 0.0031308 {
                    12.92 * x
                } else {
                    1.055 * x.powf(1.0 / 2.4) - 0.055
                }
            }
            Transfer::Gamma(g) => x.powf(1.0 / g),
        }
    }

    pub fn decode(self, v: f64) -> f64 {
        let v = v.clamp(0.0, 1.0);
        match self {
            Transfer::Linear => v,
            Transfer::Srgb => {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            }
            Transfer::Gamma(g) => v.powf(g),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for t in [
            Transfer::Linear,
            Transfer::Srgb,
            Transfer::Gamma(2.2),
            Transfer::Gamma(1.8),
        ] {
            for i in 0..=100 {
                let x = i as f64 / 100.0;
                assert!((t.decode(t.encode(x)) - x).abs() < 1e-9, "{t:?} {x}");
            }
        }
    }

    #[test]
    fn srgb_mid_grey() {
        assert!((Transfer::Srgb.encode(0.18) - 0.4613).abs() < 1e-3);
    }
}
