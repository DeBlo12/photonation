// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Where the photo sits on screen: converts between screen points and mask
//! coordinates (`photonation_develop::MaskSpace`). The mask handles and the
//! GPU overlay both use it, so they always agree.

use eframe::egui::{Pos2, Rect, pos2};
use photonation_develop::MaskSpace;
use photonation_image::NormRect;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhotoFrame {
    /// The displayed image on screen, in points. It shows `crop` of the photo.
    pub image_rect: Rect,
    pub crop: NormRect,
    pub space: MaskSpace,
}

impl PhotoFrame {
    /// Screen point of a mask position (normalized photo coordinates).
    pub fn to_screen(self, position: [f64; 2]) -> Pos2 {
        let (crop_w, crop_h) = self.crop_size();
        let u = (position[0] - self.crop.left) / crop_w;
        let v = (position[1] - self.crop.top) / crop_h;
        pos2(
            self.image_rect.left() + u as f32 * self.image_rect.width(),
            self.image_rect.top() + v as f32 * self.image_rect.height(),
        )
    }

    /// Mask position under a screen point.
    pub fn to_position(self, screen: Pos2) -> [f64; 2] {
        let (crop_w, crop_h) = self.crop_size();
        let u = ((screen.x - self.image_rect.left()) / self.image_rect.width().max(f32::EPSILON))
            as f64;
        let v = ((screen.y - self.image_rect.top()) / self.image_rect.height().max(f32::EPSILON))
            as f64;
        [self.crop.left + u * crop_w, self.crop.top + v * crop_h]
    }

    pub fn metric_to_screen(self, metric: [f64; 2]) -> Pos2 {
        self.to_screen(self.space.from_metric(metric))
    }

    pub fn screen_to_metric(self, screen: Pos2) -> [f64; 2] {
        self.space.to_metric(self.to_position(screen))
    }

    fn crop_size(self) -> (f64, f64) {
        (
            (self.crop.right - self.crop.left).max(f64::EPSILON),
            (self.crop.bottom - self.crop.top).max(f64::EPSILON),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(crop: NormRect) -> PhotoFrame {
        PhotoFrame {
            image_rect: Rect::from_min_size(pos2(100.0, 50.0), eframe::egui::vec2(300.0, 200.0)),
            crop,
            space: MaskSpace::new(6000.0, 4000.0),
        }
    }

    #[test]
    fn screen_and_positions_round_trip() {
        let f = frame(NormRect::FULL);
        assert_eq!(f.to_screen([0.0, 0.0]), pos2(100.0, 50.0));
        assert_eq!(f.to_screen([1.0, 1.0]), pos2(400.0, 250.0));
        let back = f.to_position(f.to_screen([0.3, 0.7]));
        assert!((back[0] - 0.3).abs() < 1e-6 && (back[1] - 0.7).abs() < 1e-6);
        let metric = f.screen_to_metric(pos2(400.0, 250.0));
        assert!((metric[0] - 1.0).abs() < 1e-6 && (metric[1] - 4000.0 / 6000.0).abs() < 1e-6);
    }

    #[test]
    fn cropped_photos_keep_masks_on_their_content() {
        let left_half = frame(NormRect {
            left: 0.0,
            top: 0.0,
            right: 0.5,
            bottom: 1.0,
        });
        // The photo's centre is at the right edge of the cropped image.
        assert_eq!(left_half.to_screen([0.5, 0.5]), pos2(400.0, 150.0));
    }
}
