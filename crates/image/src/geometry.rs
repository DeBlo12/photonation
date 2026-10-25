// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use serde::{Deserialize, Serialize};

/// Integer pixel rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub fn new(x: usize, y: usize, w: usize, h: usize) -> Rect {
        Rect { x, y, w, h }
    }

    /// Sub-rectangle given in coordinates normalized to this rectangle.
    /// Always at least 1×1 and inside `self`.
    pub fn sub(&self, n: NormRect) -> Rect {
        let n = n.clamped();
        let x0 = (n.left * self.w as f64).round() as usize;
        let y0 = (n.top * self.h as f64).round() as usize;
        let x1 = ((n.right * self.w as f64).round() as usize).clamp(x0 + 1, self.w.max(1));
        let y1 = ((n.bottom * self.h as f64).round() as usize).clamp(y0 + 1, self.h.max(1));
        let x0 = x0.min(x1 - 1);
        let y0 = y0.min(y1 - 1);
        Rect::new(self.x + x0, self.y + y0, x1 - x0, y1 - y0)
    }
}

/// Rectangle in coordinates normalized to an image (0..1 on both axes).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NormRect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl NormRect {
    pub const FULL: NormRect = NormRect {
        left: 0.0,
        top: 0.0,
        right: 1.0,
        bottom: 1.0,
    };

    pub fn clamped(self) -> NormRect {
        let c = |v: f64| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let (l, r) = (c(self.left), c(self.right));
        let (t, b) = (c(self.top), c(self.bottom));
        NormRect {
            left: l.min(r),
            top: t.min(b),
            right: l.max(r),
            bottom: t.max(b),
        }
    }
}

/// EXIF orientation: how the stored pixels must be transformed for display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Normal,
    FlipH,
    Rotate180,
    FlipV,
    Transpose,
    /// Rotate 90° clockwise for display (EXIF 6).
    Rotate90,
    Transverse,
    /// Rotate 270° clockwise for display (EXIF 8).
    Rotate270,
}

impl Orientation {
    pub fn from_exif(v: u16) -> Orientation {
        match v {
            2 => Orientation::FlipH,
            3 => Orientation::Rotate180,
            4 => Orientation::FlipV,
            5 => Orientation::Transpose,
            6 => Orientation::Rotate90,
            7 => Orientation::Transverse,
            8 => Orientation::Rotate270,
            _ => Orientation::Normal,
        }
    }

    pub fn swaps_axes(self) -> bool {
        matches!(
            self,
            Orientation::Transpose
                | Orientation::Rotate90
                | Orientation::Transverse
                | Orientation::Rotate270
        )
    }

    /// Display size of a stored `w × h` image.
    pub fn display_size(self, w: usize, h: usize) -> (usize, usize) {
        if self.swaps_axes() { (h, w) } else { (w, h) }
    }

    /// Display position of stored pixel `(x, y)` in a stored `w × h` image.
    #[inline]
    pub fn to_display(self, x: usize, y: usize, w: usize, h: usize) -> (usize, usize) {
        match self {
            Orientation::Normal => (x, y),
            Orientation::FlipH => (w - 1 - x, y),
            Orientation::Rotate180 => (w - 1 - x, h - 1 - y),
            Orientation::FlipV => (x, h - 1 - y),
            Orientation::Transpose => (y, x),
            Orientation::Rotate90 => (h - 1 - y, x),
            Orientation::Transverse => (h - 1 - y, w - 1 - x),
            Orientation::Rotate270 => (y, w - 1 - x),
        }
    }

    /// Stored-space normalized point for a display-space normalized point.
    fn to_stored_norm(self, u: f64, v: f64) -> (f64, f64) {
        match self {
            Orientation::Normal => (u, v),
            Orientation::FlipH => (1.0 - u, v),
            Orientation::Rotate180 => (1.0 - u, 1.0 - v),
            Orientation::FlipV => (u, 1.0 - v),
            Orientation::Transpose => (v, u),
            Orientation::Rotate90 => (v, 1.0 - u),
            Orientation::Transverse => (1.0 - v, 1.0 - u),
            Orientation::Rotate270 => (1.0 - v, u),
        }
    }

    /// Maps a crop drawn on the displayed image to stored coordinates.
    pub fn display_rect_to_stored(self, r: NormRect) -> NormRect {
        let (a0, a1) = self.to_stored_norm(r.left, r.top);
        let (b0, b1) = self.to_stored_norm(r.right, r.bottom);
        NormRect {
            left: a0.min(b0),
            top: a1.min(b1),
            right: a0.max(b0),
            bottom: a1.max(b1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Orientation; 8] = [
        Orientation::Normal,
        Orientation::FlipH,
        Orientation::Rotate180,
        Orientation::FlipV,
        Orientation::Transpose,
        Orientation::Rotate90,
        Orientation::Transverse,
        Orientation::Rotate270,
    ];

    #[test]
    fn to_display_is_a_bijection() {
        let (w, h) = (5, 3);
        for o in ALL {
            let (dw, dh) = o.display_size(w, h);
            let mut seen = vec![false; dw * dh];
            for y in 0..h {
                for x in 0..w {
                    let (dx, dy) = o.to_display(x, y, w, h);
                    assert!(dx < dw && dy < dh, "{o:?}");
                    assert!(!seen[dy * dw + dx], "{o:?} hits twice");
                    seen[dy * dw + dx] = true;
                }
            }
        }
    }

    #[test]
    fn crop_mapping_agrees_with_pixel_mapping() {
        // The top-left display quadrant must contain the display image of
        // the stored pixels in the mapped stored rect.
        let (w, h) = (8, 4);
        let crop = NormRect {
            left: 0.0,
            top: 0.0,
            right: 0.5,
            bottom: 0.5,
        };
        for o in ALL {
            let s = o.display_rect_to_stored(crop);
            let (dw, dh) = o.display_size(w, h);
            let r = Rect::new(0, 0, w, h).sub(s);
            for y in r.y..r.y + r.h {
                for x in r.x..r.x + r.w {
                    let (dx, dy) = o.to_display(x, y, w, h);
                    assert!(dx < dw / 2 && dy < dh / 2, "{o:?} ({x},{y}) -> ({dx},{dy})");
                }
            }
        }
    }

    #[test]
    fn sub_rect_is_never_empty() {
        let r = Rect::new(10, 10, 100, 50).sub(NormRect {
            left: 0.5,
            top: 0.5,
            right: 0.5,
            bottom: 0.5,
        });
        assert!(r.w >= 1 && r.h >= 1);
    }
}
