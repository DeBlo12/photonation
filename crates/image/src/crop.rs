// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Crop and straighten geometry. Documented in docs/EDIT-MODEL.md.

use crate::{NormRect, Orientation};

/// A crop frame on a photo, possibly rotated: what a render shows, upright.
///
/// Settings store a frame as the normalized photo positions of its top-left
/// and bottom-right corners plus its clockwise rotation on the photo (the
/// photo then appears rotated the other way), the way Lightroom stores a
/// crop. Rotation needs equal units on both axes, so the frame itself works
/// in photo pixels (display orientation). Without rotation the corners are
/// simply the crop rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropFrame {
    /// Photo size in pixels.
    photo: [f64; 2],
    /// Frame centre in photo pixels.
    center: [f64; 2],
    /// Width and height along the frame's own axes, in photo pixels.
    size: [f64; 2],
    /// Clockwise rotation on the photo, in degrees.
    angle: f64,
}

/// The smallest frame side, in photo pixels.
const MIN_SIDE: f64 = 1.0;
/// Rounding slack when testing whether a frame fits, in photo pixels.
const FIT_TOLERANCE: f64 = 1e-6;

impl CropFrame {
    /// The frame with `corners` (top-left and bottom-right, normalized)
    /// rotated by `angle` degrees clockwise on a `photo`-sized photo.
    pub fn new(photo: [f64; 2], corners: NormRect, angle: f64) -> CropFrame {
        let top_left = [corners.left * photo[0], corners.top * photo[1]];
        let bottom_right = [corners.right * photo[0], corners.bottom * photo[1]];
        let diagonal = [bottom_right[0] - top_left[0], bottom_right[1] - top_left[1]];
        let along_axes = rotate(diagonal, -angle);
        CropFrame {
            photo,
            center: [
                (top_left[0] + bottom_right[0]) / 2.0,
                (top_left[1] + bottom_right[1]) / 2.0,
            ],
            size: [
                along_axes[0].abs().max(MIN_SIDE),
                along_axes[1].abs().max(MIN_SIDE),
            ],
            angle,
        }
    }

    /// A frame from its centre and size in photo pixels.
    pub fn from_center(photo: [f64; 2], center: [f64; 2], size: [f64; 2], angle: f64) -> CropFrame {
        CropFrame {
            photo,
            center,
            size: [size[0].max(MIN_SIDE), size[1].max(MIN_SIDE)],
            angle,
        }
    }

    /// The whole photo, unrotated.
    pub fn full(photo: [f64; 2]) -> CropFrame {
        CropFrame::new(photo, NormRect::FULL, 0.0)
    }

    /// The largest frame with `aspect` (width ÷ height) and `angle` that
    /// fits the photo, centred on it.
    pub fn largest(photo: [f64; 2], aspect: f64, angle: f64) -> CropFrame {
        let (sin, cos) = angle.to_radians().sin_cos();
        let (sin, cos) = (sin.abs(), cos.abs());
        let height = (photo[0] / (aspect * cos + sin)).min(photo[1] / (aspect * sin + cos));
        CropFrame::from_center(
            photo,
            [photo[0] / 2.0, photo[1] / 2.0],
            [aspect * height, height],
            angle,
        )
    }

    /// The settings' representation: normalized top-left and bottom-right corners.
    pub fn corners(&self) -> NormRect {
        let half_diagonal = rotate([self.size[0] / 2.0, self.size[1] / 2.0], self.angle);
        NormRect {
            left: (self.center[0] - half_diagonal[0]) / self.photo[0],
            top: (self.center[1] - half_diagonal[1]) / self.photo[1],
            right: (self.center[0] + half_diagonal[0]) / self.photo[0],
            bottom: (self.center[1] + half_diagonal[1]) / self.photo[1],
        }
    }

    pub fn photo_size(&self) -> [f64; 2] {
        self.photo
    }

    pub fn center(&self) -> [f64; 2] {
        self.center
    }

    /// Width and height in photo pixels: the size of a full-resolution render.
    pub fn size(&self) -> [f64; 2] {
        self.size
    }

    /// Clockwise rotation on the photo, in degrees.
    pub fn angle(&self) -> f64 {
        self.angle
    }

    pub fn with_angle(self, angle: f64) -> CropFrame {
        CropFrame { angle, ..self }
    }

    /// Normalized photo position of a point given normalized to the frame
    /// (0..1 across its width and height).
    pub fn to_photo(&self, frame_point: [f64; 2]) -> [f64; 2] {
        let offset = rotate(
            [
                (frame_point[0] - 0.5) * self.size[0],
                (frame_point[1] - 0.5) * self.size[1],
            ],
            self.angle,
        );
        [
            (self.center[0] + offset[0]) / self.photo[0],
            (self.center[1] + offset[1]) / self.photo[1],
        ]
    }

    /// Normalized frame position of a normalized photo point (the inverse
    /// of [`Self::to_photo`]).
    pub fn to_frame(&self, photo_point: [f64; 2]) -> [f64; 2] {
        let offset = rotate(
            [
                photo_point[0] * self.photo[0] - self.center[0],
                photo_point[1] * self.photo[1] - self.center[1],
            ],
            -self.angle,
        );
        [
            offset[0] / self.size[0] + 0.5,
            offset[1] / self.size[1] + 0.5,
        ]
    }

    /// The frame's corners in normalized photo coordinates, clockwise from
    /// its top-left.
    pub fn corner_points(&self) -> [[f64; 2]; 4] {
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|corner| self.to_photo(corner))
    }

    /// How far the frame reaches from its centre along the photo's axes.
    fn half_extent(&self) -> [f64; 2] {
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let (half_w, half_h) = (self.size[0] / 2.0, self.size[1] / 2.0);
        [
            (half_w * cos).abs() + (half_h * sin).abs(),
            (half_w * sin).abs() + (half_h * cos).abs(),
        ]
    }

    /// Whether the whole frame lies on the photo.
    pub fn fits(&self) -> bool {
        let extent = self.half_extent();
        (0..2).all(|axis| {
            self.center[axis] - extent[axis] >= -FIT_TOLERANCE
                && self.center[axis] + extent[axis] <= self.photo[axis] + FIT_TOLERANCE
        })
    }

    /// The frame shrunk around its centre until it is no larger than the
    /// photo allows at its angle, then moved onto the photo.
    pub fn constrained(self) -> CropFrame {
        let extent = self.half_extent();
        let scale = (0..2)
            .map(|axis| self.photo[axis] / 2.0 / extent[axis].max(f64::EPSILON))
            .fold(1.0f64, f64::min);
        let shrunk = CropFrame {
            size: [
                (self.size[0] * scale).max(MIN_SIDE),
                (self.size[1] * scale).max(MIN_SIDE),
            ],
            ..self
        };
        let extent = shrunk.half_extent();
        let clamp_center = |axis: usize| {
            let (low, high) = (extent[axis], self.photo[axis] - extent[axis]);
            if low <= high {
                self.center[axis].clamp(low, high)
            } else {
                self.photo[axis] / 2.0
            }
        };
        CropFrame {
            center: [clamp_center(0), clamp_center(1)],
            ..shrunk
        }
    }

    /// This frame, given on the stored image, as seen on the displayed one.
    pub fn oriented(self, orientation: Orientation) -> CropFrame {
        let swap = |[a, b]: [f64; 2]| {
            if orientation.swaps_axes() {
                [b, a]
            } else {
                [a, b]
            }
        };
        let photo = swap(self.photo);
        let (center_x, center_y) = orientation.to_display_norm(
            self.center[0] / self.photo[0],
            self.center[1] / self.photo[1],
        );
        CropFrame {
            photo,
            center: [center_x * photo[0], center_y * photo[1]],
            size: swap(self.size),
            angle: if orientation.mirrors() {
                -self.angle
            } else {
                self.angle
            },
        }
    }

    /// Smallest normalized rectangle (axis-aligned on the photo) holding the frame.
    pub fn bounds(&self) -> NormRect {
        let corners = self.corner_points();
        let along = |axis: usize, pick: fn(f64, f64) -> f64, start: f64| {
            corners.iter().map(|c| c[axis]).fold(start, pick)
        };
        NormRect {
            left: along(0, f64::min, f64::INFINITY),
            top: along(1, f64::min, f64::INFINITY),
            right: along(0, f64::max, f64::NEG_INFINITY),
            bottom: along(1, f64::max, f64::NEG_INFINITY),
        }
    }
}

/// `vector` rotated `degrees` clockwise (y points down).
fn rotate(vector: [f64; 2], degrees: f64) -> [f64; 2] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [
        vector[0] * cos - vector[1] * sin,
        vector[0] * sin + vector[1] * cos,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHOTO: [f64; 2] = [6000.0, 4000.0];

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn unrotated_frames_are_the_crop_rectangle() {
        let crop = NormRect {
            left: 0.1,
            top: 0.2,
            right: 0.6,
            bottom: 0.9,
        };
        let frame = CropFrame::new(PHOTO, crop, 0.0);
        assert_eq!(frame.size(), [3000.0, 2800.0]);
        let back = frame.corners();
        for (a, b) in [
            (back.left, crop.left),
            (back.top, crop.top),
            (back.right, crop.right),
            (back.bottom, crop.bottom),
        ] {
            assert!(close(a, b, 1e-12));
        }
        assert_eq!(frame.to_photo([0.0, 0.0]), [0.1, 0.2]);
        assert!(frame.fits());
    }

    #[test]
    fn lightroom_crops_give_lightroom_sizes() {
        // From a Lightroom export ("Garage1.jpg", 3838 × 2559): the stored
        // corners and angle of a Canon EOS 250D raw (6000 × 4000).
        let frame = CropFrame::new(
            PHOTO,
            NormRect {
                left: 0.100976,
                top: 0.23349,
                right: 0.731315,
                bottom: 0.893875,
            },
            1.242087,
        );
        let [width, height] = frame.size();
        assert!(close(width, 3838.0, 1.0) && close(height, 2559.0, 1.0));
        assert!(frame.fits());
    }

    #[test]
    fn oriented_frames_cover_the_same_content() {
        let stored = CropFrame::from_center(PHOTO, [3400.0, 1700.0], [2400.0, 1500.0], 6.0);
        for orientation in [
            Orientation::Normal,
            Orientation::FlipH,
            Orientation::Rotate180,
            Orientation::FlipV,
            Orientation::Transpose,
            Orientation::Rotate90,
            Orientation::Transverse,
            Orientation::Rotate270,
        ] {
            let displayed = stored.oriented(orientation);
            // Every displayed corner is the display position of a stored one.
            let stored_corners = stored
                .corner_points()
                .map(|[x, y]| orientation.to_display_norm(x, y));
            for [x, y] in displayed.corner_points() {
                assert!(
                    stored_corners
                        .iter()
                        .any(|(sx, sy)| close(*sx, x, 1e-9) && close(*sy, y, 1e-9)),
                    "{orientation:?}"
                );
            }
        }
    }

    #[test]
    fn portrait_lightroom_crops_give_lightroom_sizes() {
        // "Straßburg Tramstation.jpg" (3470 × 5205) from a raw shown
        // rotated (EXIF 8); Lightroom stores the crop on the stored image.
        let stored = CropFrame::new(
            PHOTO,
            NormRect {
                left: 0.052519,
                top: 0.09873,
                right: 0.947481,
                bottom: 0.90127,
            },
            -2.818159,
        );
        let [width, height] = stored.oriented(Orientation::Rotate270).size();
        assert!(close(width, 3470.0, 1.0) && close(height, 5205.0, 1.0));
    }

    #[test]
    fn corners_and_points_round_trip() {
        let frame = CropFrame::from_center(PHOTO, [3100.0, 1900.0], [2400.0, 1600.0], -7.5);
        let again = CropFrame::new(PHOTO, frame.corners(), frame.angle());
        for axis in 0..2 {
            assert!(close(again.center()[axis], frame.center()[axis], 1e-9));
            assert!(close(again.size()[axis], frame.size()[axis], 1e-9));
        }
        let point = [0.3, 0.8];
        let back = frame.to_frame(frame.to_photo(point));
        assert!(close(back[0], point[0], 1e-12) && close(back[1], point[1], 1e-12));
    }

    #[test]
    fn positive_angles_turn_the_frame_clockwise() {
        // The frame's top edge runs down to the right on the photo.
        let frame = CropFrame::from_center(PHOTO, [3000.0, 2000.0], [2000.0, 1000.0], 10.0);
        let [top_left, top_right, ..] = frame.corner_points();
        assert!(top_right[1] > top_left[1]);
    }

    #[test]
    fn largest_frames_touch_the_photo_edges() {
        for angle in [-30.0, -4.0, 0.0, 2.5, 45.0] {
            let frame = CropFrame::largest(PHOTO, 1.5, angle);
            assert!(frame.fits(), "{angle}");
            let grown = CropFrame::from_center(
                PHOTO,
                frame.center(),
                [frame.size()[0] * 1.001, frame.size()[1] * 1.001],
                angle,
            );
            assert!(!grown.fits(), "{angle}");
        }
        assert_eq!(CropFrame::largest(PHOTO, 1.5, 0.0).size(), PHOTO);
    }

    #[test]
    fn constrained_frames_fit() {
        let too_big = CropFrame::from_center(PHOTO, [5500.0, 300.0], [6000.0, 4000.0], 12.0);
        let fitted = too_big.constrained();
        assert!(fitted.fits());
        // The aspect ratio is kept.
        assert!(close(fitted.size()[0] / fitted.size()[1], 1.5, 1e-9));
        // A frame that already fits stays put.
        let inside = CropFrame::from_center(PHOTO, [3000.0, 2000.0], [1000.0, 800.0], 5.0);
        assert_eq!(inside.constrained(), inside);
    }

    #[test]
    fn bounds_hold_every_corner() {
        let frame = CropFrame::from_center(PHOTO, [3000.0, 2000.0], [3000.0, 2000.0], 20.0);
        let bounds = frame.bounds();
        for [x, y] in frame.corner_points() {
            assert!(bounds.left <= x && x <= bounds.right);
            assert!(bounds.top <= y && y <= bounds.bottom);
        }
    }
}
