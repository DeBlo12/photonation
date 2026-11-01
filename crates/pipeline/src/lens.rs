// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Lens corrections: the lens profile from the Lensfun database
//! (`photonation-lens`), scaled by the settings' amounts, plus the manual
//! corrections, as
//!
//! - a [`LensMap`] from the corrected photo to the photo as shot
//!   (distortion, and red and blue apart for chromatic aberration), which
//!   the resampling stage follows (`straighten.rs`), and
//! - a [`Vignette`] gain on the photo as shot, which `Develop` applies.
//!
//! Both are radial about the photo's centre, with radius 1 at its corners.

use photonation_develop::LensCorrection;
use photonation_image::CameraInfo;
use photonation_lens::{Profile, Shot};

/// Shrinking of the corner radius at manual Distortion +100 (the photo's
/// corners come from that much nearer the centre; −100 grows it as much).
const MANUAL_DISTORTION: f64 = 0.15;
/// Brightening of the corners at manual Vignetting +100, in stops.
const MANUAL_VIGNETTING_EV: f32 = 2.0;
/// Radii sampled from the edges' middles to the corners when fitting the
/// corrected photo into its frame.
const ZOOM_SAMPLES: usize = 64;

/// The Lensfun profile for a photo's camera and lens, if the database has
/// them.
pub fn lens_profile(info: &CameraInfo) -> Option<Profile> {
    photonation_lens::profile(&Shot {
        make: &info.make,
        model: &info.model,
        lens: info.lens.as_deref(),
        focal_length: info.focal_length,
        f_number: info.f_number,
    })
}

/// Where the corrected photo's points lie on the photo as shot. Positions
/// are normalized on the displayed photo (0..1 across each side).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LensMap {
    /// The displayed photo's size in pixels.
    size: [f64; 2],
    /// Corrected positions scale about the centre by this first, so the
    /// corrected photo fills its frame with no empty edges.
    zoom: f64,
    /// The radius factor `F(ρ) = Σ cᵢ ρⁱ`: a corrected point at radius ρ
    /// (after the zoom) lies at radius `ρ · F(ρ)` on the photo as shot.
    distortion: [f64; 7],
    /// Red's and blue's radius factors `v + c ρ + b ρ²` on the photo as
    /// shot, relative to green.
    tca: Option<[[f64; 3]; 2]>,
}

/// A polynomial's value at `r` (coefficients from the constant term up).
fn polynomial(coefficients: &[f64], r: f64) -> f64 {
    coefficients.iter().rev().fold(0.0, |sum, c| sum * r + c)
}

/// The `t` in `0..` where the increasing `f` reaches `target`.
fn inverse(f: impl Fn(f64) -> f64, target: f64) -> f64 {
    let mut high = 1.0;
    while f(high) < target && high < 64.0 {
        high *= 2.0;
    }
    let mut low = 0.0;
    for _ in 0..60 {
        let middle = (low + high) / 2.0;
        if f(middle) < target {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0
}

impl LensMap {
    /// The map the settings ask for on a photo of `size` (displayed, in
    /// pixels), `None` when it moves nothing.
    pub fn new(
        size: [f64; 2],
        settings: &LensCorrection,
        profile: Option<&Profile>,
    ) -> Option<LensMap> {
        let mut distortion = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        if settings.profile
            && let Some(profile) = profile.and_then(|p| p.distortion)
        {
            let amount = settings.distortion as f64 / 100.0;
            for (i, c) in profile.0.iter().enumerate() {
                distortion[i] = amount * c + if i == 0 { 1.0 - amount } else { 0.0 };
            }
        }
        let manual = settings.manual_distortion as f64 / 100.0 * MANUAL_DISTORTION;
        if manual != 0.0 {
            // Times 1 − manual · ρ².
            for i in (2..7).rev() {
                distortion[i] -= manual * distortion[i - 2];
            }
        }
        let tca = settings
            .chromatic_aberration
            .then(|| profile.and_then(|p| p.tca))
            .flatten()
            .map(|tca| [tca.red, tca.blue].map(|radial| [radial.0[0], radial.0[1], radial.0[2]]));
        if distortion == [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] && tca.is_none() {
            return None;
        }
        let mut map = LensMap {
            size: size.map(|side| side.max(1.0)),
            zoom: 1.0,
            distortion,
            tca,
        };
        map.zoom = map.filling_zoom();
        Some(map)
    }

    /// The largest zoom whose corrected photo (each channel's) stays on the
    /// photo as shot: at every radius from the edges' middles to the
    /// corners, the point must come from no farther out.
    fn filling_zoom(&self) -> f64 {
        let nearest_edge = self.size[0].min(self.size[1]) / self.size[0].hypot(self.size[1]);
        let mut zoom = f64::INFINITY;
        for i in 0..=ZOOM_SAMPLES {
            let edge = nearest_edge + (1.0 - nearest_edge) * i as f64 / ZOOM_SAMPLES as f64;
            for channel in 0..3 {
                let reached = inverse(|t| self.shot_radius(t, channel), edge);
                zoom = zoom.min(reached / edge);
            }
        }
        if zoom.is_finite() && zoom > 0.0 {
            zoom
        } else {
            1.0
        }
    }

    /// Radius on the photo as shot of `channel` at zoomed radius `rho`.
    fn shot_radius(&self, rho: f64, channel: usize) -> f64 {
        let green = rho * polynomial(&self.distortion, rho);
        green * self.tca_factor(channel, green)
    }

    fn tca_factor(&self, channel: usize, rho: f64) -> f64 {
        match (self.tca, channel) {
            (Some([red, _]), 0) => polynomial(&red, rho),
            (Some([_, blue]), 2) => polynomial(&blue, rho),
            _ => 1.0,
        }
    }

    /// Pixels per unit radius: half the diagonal.
    fn radius(&self) -> f64 {
        self.size[0].hypot(self.size[1]) / 2.0
    }

    /// Where the photo as shot shows a corrected point, for each channel.
    pub fn to_shot_channels(&self, point: [f64; 2]) -> [[f64; 2]; 3] {
        let offset = [
            (point[0] - 0.5) * self.size[0] * self.zoom,
            (point[1] - 0.5) * self.size[1] * self.zoom,
        ];
        let rho = offset[0].hypot(offset[1]) / self.radius();
        let green = polynomial(&self.distortion, rho);
        std::array::from_fn(|channel| {
            let factor = green * self.tca_factor(channel, rho * green);
            [
                0.5 + offset[0] * factor / self.size[0],
                0.5 + offset[1] * factor / self.size[1],
            ]
        })
    }

    /// Where the photo as shot shows a corrected point (green).
    pub fn to_shot(&self, point: [f64; 2]) -> [f64; 2] {
        self.to_shot_channels(point)[1]
    }

    /// The corrected point showing a point of the photo as shot (green):
    /// the inverse of [`Self::to_shot`].
    pub fn from_shot(&self, point: [f64; 2]) -> [f64; 2] {
        let offset = [
            (point[0] - 0.5) * self.size[0],
            (point[1] - 0.5) * self.size[1],
        ];
        let shot = offset[0].hypot(offset[1]) / self.radius();
        if shot == 0.0 {
            return [0.5, 0.5];
        }
        let corrected = inverse(|rho| self.shot_radius(rho, 1), shot);
        let scale = corrected / shot / self.zoom;
        [
            0.5 + offset[0] * scale / self.size[0],
            0.5 + offset[1] * scale / self.size[1],
        ]
    }

    /// The displayed photo's size in pixels.
    pub fn size(&self) -> [f64; 2] {
        self.size
    }

    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    pub fn distortion(&self) -> [f64; 7] {
        self.distortion
    }

    pub fn tca(&self) -> Option<[[f64; 3]; 2]> {
        self.tca
    }
}

/// The vignetting correction: the photo as shot, at radius `r` from its
/// centre, brightens by `gain(r²)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vignette {
    /// The profile's falloff `1 + k₁r² + k₂r⁴ + k₃r⁶`…
    pub falloff: [f32; 3],
    /// …is divided out to this power (its amount, 0..2)…
    pub amount: f32,
    /// …and the corners brighten by this many stops more (`r³` falloff).
    pub manual_ev: f32,
}

impl Vignette {
    pub fn new(settings: &LensCorrection, profile: Option<&Profile>) -> Option<Vignette> {
        let falloff = profile
            .filter(|_| settings.profile && settings.vignetting > 0.0)
            .and_then(|p| p.vignetting);
        let manual_ev = settings.manual_vignetting / 100.0 * MANUAL_VIGNETTING_EV;
        if falloff.is_none() && manual_ev == 0.0 {
            return None;
        }
        Some(Vignette {
            falloff: falloff.map_or([0.0; 3], |k| k.map(|k| k as f32)),
            amount: if falloff.is_some() {
                settings.vignetting / 100.0
            } else {
                0.0
            },
            manual_ev,
        })
    }

    #[inline(always)]
    pub fn gain(&self, r2: f32) -> f32 {
        let [k1, k2, k3] = self.falloff;
        let falloff = 1.0 + r2 * (k1 + r2 * (k2 + r2 * k3));
        let stops = self.manual_ev * r2 * r2.sqrt() - self.amount * falloff.max(1e-3).log2();
        stops.exp2()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(width: usize, height: usize) -> [f64; 2] {
        [width as f64, height as f64]
    }

    fn manual(distortion: f32) -> LensCorrection {
        LensCorrection {
            manual_distortion: distortion,
            ..LensCorrection::default()
        }
    }

    #[test]
    fn nothing_to_correct_is_no_map() {
        let size = photo(60, 40);
        assert!(LensMap::new(size, &LensCorrection::default(), None).is_none());
        // The profile switched on without one (an unknown lens): nothing.
        let on = LensCorrection {
            profile: true,
            chromatic_aberration: true,
            ..LensCorrection::default()
        };
        assert!(LensMap::new(size, &on, None).is_none());
        assert!(Vignette::new(&on, None).is_none());
    }

    #[test]
    fn corrected_photos_fill_their_frame() {
        let size = photo(600, 400);
        for amount in [-100.0, -30.0, 40.0, 100.0] {
            let map = LensMap::new(size, &manual(amount), None).expect("moves");
            // Every edge point comes from the photo, and the tightest one
            // from its very edge.
            let mut farthest = 0.0f64;
            for i in 0..=100 {
                let t = i as f64 / 100.0;
                for point in [[t, 0.0], [0.0, t], [t, 1.0], [1.0, t]] {
                    let [x, y] = map.to_shot(point);
                    let reach = (x - 0.5).abs().max((y - 0.5).abs()) * 2.0;
                    assert!(reach <= 1.0 + 1e-9, "{amount}: {point:?} from {x}, {y}");
                    farthest = farthest.max(reach);
                }
            }
            assert!(farthest > 0.999, "{amount}: {farthest}");
        }
    }

    #[test]
    fn from_shot_inverts_to_shot() {
        let size = photo(600, 400);
        let map = LensMap::new(size, &manual(70.0), None).expect("moves");
        for point in [[0.1, 0.2], [0.5, 0.5], [0.93, 0.61], [0.0, 1.0]] {
            let back = map.from_shot(map.to_shot(point));
            assert!((back[0] - point[0]).abs() < 1e-9 && (back[1] - point[1]).abs() < 1e-9);
        }
        // Positive manual distortion straightens barrel distortion: edge
        // middles come from nearer the centre than corners do.
        let middle = map.to_shot([1.0, 0.5])[0] - 0.5;
        let corner = map.to_shot([1.0, 1.0])[0] - 0.5;
        assert!(middle > corner, "{middle} {corner}");
    }

    #[test]
    fn vignetting_brightens_corners_only() {
        let up = Vignette::new(
            &LensCorrection {
                manual_vignetting: 50.0,
                ..LensCorrection::default()
            },
            None,
        )
        .expect("manual");
        assert_eq!(up.gain(0.0), 1.0);
        assert!((up.gain(1.0) - 2.0).abs() < 1e-6, "one stop at the corner");
        assert!(up.gain(0.25) > 1.0 && up.gain(0.25) < up.gain(1.0));
    }
}
