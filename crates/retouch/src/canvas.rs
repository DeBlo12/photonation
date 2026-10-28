// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

/// An image to inpaint: interleaved RGB samples (display-encoded, nominally
/// 0..1) and a hole mask, one value per pixel, where anything above 0 is to
/// be filled.
#[derive(Clone, Debug, PartialEq)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<f32>,
    pub hole: Vec<f32>,
}

impl Canvas {
    pub fn is_valid(&self) -> bool {
        let pixels = self.width * self.height;
        pixels > 0 && self.rgb.len() == pixels * 3 && self.hole.len() == pixels
    }

    #[inline(always)]
    pub fn is_hole(&self, index: usize) -> bool {
        self.hole[index] > 0.0
    }

    pub fn hole_pixels(&self) -> usize {
        self.hole.iter().filter(|weight| **weight > 0.0).count()
    }

    /// Bounding box of the hole, `(left, top, right, bottom)` exclusive, or
    /// `None` without a hole.
    pub fn hole_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        let mut bounds: Option<(usize, usize, usize, usize)> = None;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.is_hole(y * self.width + x) {
                    let (left, top, right, bottom) = bounds.get_or_insert((x, y, x + 1, y + 1));
                    *left = (*left).min(x);
                    *top = (*top).min(y);
                    *right = (*right).max(x + 1);
                    *bottom = (*bottom).max(y + 1);
                }
            }
        }
        bounds
    }

    /// This canvas resampled to `width × height` (bilinear on the pixel
    /// centres). The hole grows rather than shrinks: a target pixel is a
    /// hole if any source pixel it touches is.
    pub fn resized(&self, width: usize, height: usize) -> Canvas {
        let rgb = resize_interleaved(&self.rgb, (self.width, self.height), (width, height), 3);
        let hole = resize_max(&self.hole, (self.width, self.height), (width, height));
        Canvas {
            width,
            height,
            rgb,
            hole,
        }
    }
}

/// Bilinear resampling of an interleaved image with `channels` per pixel.
pub fn resize_interleaved(
    samples: &[f32],
    from: (usize, usize),
    to: (usize, usize),
    channels: usize,
) -> Vec<f32> {
    let (from_w, from_h) = from;
    let (to_w, to_h) = to;
    let mut out = vec![0.0; to_w * to_h * channels];
    if from_w == 0 || from_h == 0 {
        return out;
    }
    let scale_x = from_w as f32 / to_w as f32;
    let scale_y = from_h as f32 / to_h as f32;
    let source_coordinate = |target: usize, scale: f32, len: usize| {
        let position = ((target as f32 + 0.5) * scale - 0.5).clamp(0.0, (len - 1) as f32);
        let low = position.floor() as usize;
        (low, (low + 1).min(len - 1), position - low as f32)
    };
    for y in 0..to_h {
        let (y0, y1, fy) = source_coordinate(y, scale_y, from_h);
        for x in 0..to_w {
            let (x0, x1, fx) = source_coordinate(x, scale_x, from_w);
            for channel in 0..channels {
                let at = |sx: usize, sy: usize| samples[(sy * from_w + sx) * channels + channel];
                let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
                let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
                out[(y * to_w + x) * channels + channel] = top + (bottom - top) * fy;
            }
        }
    }
    out
}

/// Resampling that keeps, for each target pixel, the largest source value
/// its footprint touches (so thin holes survive shrinking).
fn resize_max(values: &[f32], from: (usize, usize), to: (usize, usize)) -> Vec<f32> {
    let (from_w, from_h) = from;
    let (to_w, to_h) = to;
    let span = |target: usize, from_len: usize, to_len: usize| {
        let start = target * from_len / to_len;
        let end = ((target + 1) * from_len).div_ceil(to_len).max(start + 1);
        start.min(from_len - 1)..end.min(from_len)
    };
    let mut out = vec![0.0; to_w * to_h];
    for y in 0..to_h {
        let rows = span(y, from_h, to_h);
        for x in 0..to_w {
            let columns = span(x, from_w, to_w);
            out[y * to_w + x] = rows
                .clone()
                .flat_map(|sy| columns.clone().map(move |sx| values[sy * from_w + sx]))
                .fold(0.0, f32::max);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrinking_keeps_thin_holes() {
        let mut hole = vec![0.0; 16 * 16];
        hole[5 * 16 + 7] = 1.0; // a single pixel
        let canvas = Canvas {
            width: 16,
            height: 16,
            rgb: vec![0.5; 16 * 16 * 3],
            hole,
        };
        let small = canvas.resized(4, 4);
        assert_eq!(small.hole_pixels(), 1);
        assert!(small.rgb.iter().all(|v| (v - 0.5).abs() < 1e-6));
        assert_eq!(canvas.hole_bounds(), Some((7, 5, 8, 6)));
    }

    #[test]
    fn resizing_to_the_same_size_is_exact() {
        let rgb: Vec<f32> = (0..5 * 3 * 3).map(|i| i as f32 / 45.0).collect();
        assert_eq!(resize_interleaved(&rgb, (5, 3), (5, 3), 3), rgb);
    }
}
