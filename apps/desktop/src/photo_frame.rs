// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Where the photo sits on screen: converts between screen points and mask
//! coordinates (`photonation_develop::MaskSpace`). The mask handles, the
//! GPU overlay and the crop tool all use it, so they always agree.

use eframe::egui::{Pos2, Rect, pos2};
use photonation_develop::MaskSpace;
use photonation_image::CropFrame;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhotoFrame {
    /// The displayed image on screen, in points. It shows what `crop`
    /// frames of the photo, upright.
    pub image_rect: Rect,
    pub crop: CropFrame,
    pub space: MaskSpace,
}

impl PhotoFrame {
    /// Screen point of a mask position (normalized photo coordinates).
    pub fn to_screen(self, position: [f64; 2]) -> Pos2 {
        let [u, v] = self.crop.to_frame(position);
        pos2(
            self.image_rect.left() + u as f32 * self.image_rect.width(),
            self.image_rect.top() + v as f32 * self.image_rect.height(),
        )
    }

    /// Mask position under a screen point.
    pub fn to_position(self, screen: Pos2) -> [f64; 2] {
        let u = ((screen.x - self.image_rect.left()) / self.image_rect.width().max(f32::EPSILON))
            as f64;
        let v = ((screen.y - self.image_rect.top()) / self.image_rect.height().max(f32::EPSILON))
            as f64;
        self.crop.to_photo([u, v])
    }

    pub fn metric_to_screen(self, metric: [f64; 2]) -> Pos2 {
        self.to_screen(self.space.from_metric(metric))
    }

    pub fn screen_to_metric(self, screen: Pos2) -> [f64; 2] {
        self.space.to_metric(self.to_position(screen))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonation_image::NormRect;

    const PHOTO: [f64; 2] = [6000.0, 4000.0];

    fn frame(crop: NormRect) -> PhotoFrame {
        PhotoFrame {
            image_rect: Rect::from_min_size(pos2(100.0, 50.0), eframe::egui::vec2(300.0, 200.0)),
            crop: CropFrame::new(PHOTO, crop, 0.0),
            space: MaskSpace::new(PHOTO[0], PHOTO[1]),
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

    #[test]
    fn straightened_photos_keep_masks_on_their_content() {
        let mut tilted = frame(NormRect::FULL);
        tilted.crop = CropFrame::largest(PHOTO, 1.5, 10.0);
        // The photo's centre stays in the middle; a point right of it on
        // the photo appears higher up, as the photo turns anticlockwise.
        assert!(tilted.to_screen([0.5, 0.5]).distance(pos2(250.0, 150.0)) < 1e-3);
        assert!(tilted.to_screen([0.7, 0.5]).y < 150.0);
        let back = tilted.to_position(tilted.to_screen([0.3, 0.6]));
        assert!((back[0] - 0.3).abs() < 1e-6 && (back[1] - 0.6).abs() < 1e-6);
    }
}
