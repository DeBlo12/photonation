// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Small pictures of what each mask selects, for the layer list: a tint
//! drawn over the photo's thumbnail. They are computed on the CPU at
//! thumbnail size (a few thousand pixels) and cached until the mask's
//! shapes, the crop or the size changes; how strongly they are drawn
//! follows the mask's amount and visibility ([`tint_strength`]).

use crate::photo_frame::PhotoFrame;
use crate::theme;
use eframe::egui::{self, Color32, ColorImage, Rect, TextureHandle, TextureOptions, pos2, vec2};
use photonation_develop::Mask;
use photonation_pipeline::masks::MaskWeight;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

/// Pictures of masks that don't render (switched off, or in a group that
/// is) are drawn this much as strongly.
pub const HIDDEN_OPACITY: f32 = 0.4;
/// The faintest tint, so a mask at 0 % still shows what it selects.
const MIN_TINT_STRENGTH: f32 = 0.25;

/// How strongly to draw a mask's tint, in its thumbnail and as the overlay
/// on the photo, for the mask as it renders ([`Mask::rendered`]): by its
/// amount (its own times its group's), and fainter when it is hidden.
pub fn tint_strength(rendered: &Mask) -> f32 {
    let amount = (rendered.amount / 100.0).clamp(0.0, 1.0);
    let strength = MIN_TINT_STRENGTH + (1.0 - MIN_TINT_STRENGTH) * amount;
    if rendered.enabled {
        strength
    } else {
        strength * HIDDEN_OPACITY
    }
}

/// Tint textures by what they show. Textures not asked for during a frame
/// are dropped at its end.
#[derive(Default)]
pub struct MaskThumbnails {
    cached: HashMap<u64, TextureHandle>,
    used: HashMap<u64, TextureHandle>,
}

impl MaskThumbnails {
    /// The tint for `mask` over a photo shown as `frame`, `size` pixels
    /// large (the frame's `image_rect` is ignored).
    pub fn tint(
        &mut self,
        ctx: &egui::Context,
        mask: &Mask,
        frame: PhotoFrame,
        size: [usize; 2],
    ) -> egui::TextureId {
        let key = tint_key(mask, frame, size);
        let texture = match self.cached.remove(&key) {
            Some(texture) => texture,
            None => ctx.load_texture(
                "mask-thumbnail",
                tint_image(mask, frame, size),
                TextureOptions::LINEAR,
            ),
        };
        let id = texture.id();
        self.used.insert(key, texture);
        id
    }

    /// Call once per frame, after the layer list.
    pub fn end_frame(&mut self) {
        self.cached = std::mem::take(&mut self.used);
    }
}

fn tint_key(mask: &Mask, frame: PhotoFrame, size: [usize; 2]) -> u64 {
    let mut hasher = DefaultHasher::new();
    // Floats don't implement Hash; their Debug text identifies them.
    format!(
        "{:?} {} {:?} {:?}",
        mask.components, mask.invert, frame.crop, frame.space
    )
    .hash(&mut hasher);
    size.hash(&mut hasher);
    hasher.finish()
}

/// The mask overlay colour scaled by the mask's weight at each pixel.
pub fn tint_image(mask: &Mask, frame: PhotoFrame, size: [usize; 2]) -> ColorImage {
    let weight = MaskWeight::new(&mask.sanitized(), frame.space);
    let frame = PhotoFrame {
        image_rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(size[0] as f32, size[1] as f32)),
        ..frame
    };
    let tint = theme::MASK_OVERLAY.to_array();
    let pixels = (0..size[1])
        .flat_map(|y| (0..size[0]).map(move |x| (x, y)))
        .map(|(x, y)| {
            let centre = pos2(x as f32 + 0.5, y as f32 + 0.5);
            let point = frame.screen_to_metric(centre).map(|v| v as f32);
            let strength = weight.at(point);
            let [r, g, b, a] = tint.map(|channel| (f32::from(channel) * strength).round() as u8);
            Color32::from_rgba_premultiplied(r, g, b, a)
        })
        .collect();
    ColorImage::new(size, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonation_develop::{MaskShape, MaskSpace, RadialGradient};
    use photonation_image::{CropFrame, NormRect};

    #[test]
    fn tint_follows_the_mask_and_the_crop() {
        let mask = Mask::new(
            "Left",
            MaskShape::Radial(RadialGradient {
                center: [0.25, 0.5],
                radius: [0.1, 0.1],
                rotation: 0.0,
                feather: 0.0,
            }),
        );
        let frame = PhotoFrame {
            image_rect: Rect::NOTHING,
            crop: CropFrame::full([400.0, 200.0]),
            space: MaskSpace::new(400.0, 200.0),
        };
        let image = tint_image(&mask, frame, [40, 20]);
        assert_eq!(image[(10, 10)], theme::MASK_OVERLAY, "inside the circle");
        assert_eq!(image[(30, 10)], Color32::TRANSPARENT, "outside");

        // Cropped to the left half, the circle is in the middle.
        let cropped = PhotoFrame {
            crop: CropFrame::new(
                [400.0, 200.0],
                NormRect {
                    right: 0.5,
                    ..NormRect::FULL
                },
                0.0,
            ),
            ..frame
        };
        let image = tint_image(&mask, cropped, [20, 20]);
        assert_eq!(image[(10, 10)], theme::MASK_OVERLAY);
        assert_eq!(image[(1, 1)], Color32::TRANSPARENT);
    }
}
