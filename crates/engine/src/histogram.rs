// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

/// 256-bin histograms of an 8-bit RGB image (per channel and luminance).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Histogram {
    pub red: Vec<u32>,
    pub green: Vec<u32>,
    pub blue: Vec<u32>,
    pub luminance: Vec<u32>,
}

impl Histogram {
    pub fn from_rgb8(pixels: &[u8]) -> Histogram {
        let mut h = Histogram {
            red: vec![0; 256],
            green: vec![0; 256],
            blue: vec![0; 256],
            luminance: vec![0; 256],
        };
        for pixel in pixels.chunks_exact(3) {
            let (r, g, b) = (pixel[0] as usize, pixel[1] as usize, pixel[2] as usize);
            h.red[r] += 1;
            h.green[g] += 1;
            h.blue[b] += 1;
            h.luminance[(r * 54 + g * 183 + b * 19) >> 8] += 1;
        }
        h
    }

    /// Largest bin, ignoring the two clipped end bins so a blown sky
    /// doesn't flatten the rest of the graph.
    pub fn peak(&self) -> u32 {
        [&self.red, &self.green, &self.blue, &self.luminance]
            .iter()
            .flat_map(|channel| channel[1..255].iter().copied())
            .max()
            .unwrap_or(0)
            .max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_pixels() {
        let h = Histogram::from_rgb8(&[255, 0, 0, 255, 255, 255]);
        assert_eq!(h.red[255], 2);
        assert_eq!(h.green[0], 1);
        assert_eq!(h.luminance[254] + h.luminance[255], 1);
    }
}
