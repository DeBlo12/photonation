// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Lens profiles from the Lensfun database (<https://lensfun.github.io>,
//! CC BY-SA 3.0, embedded unchanged from `lensfun/db`).
//!
//! [`profile`] finds the camera and lens of a shot the way Lensfun does
//! (camera by name; lens by fuzzy name, crop factor, mount, focal range and
//! aperture), interpolates the lens's calibrations to the shot's focal
//! length (and aperture, for vignetting) and returns them as polynomials in
//! one radius: `r = 1` at the corner of the full image, wherever the
//! calibration measured it.

mod database;
mod matching;

use database::{Database, DistortionModel, Lens, TcaModel};
use std::sync::{Mutex, OnceLock};

mod files {
    include!(concat!(env!("OUT_DIR"), "/database.rs"));
}

/// The database, parsed on first use.
fn database() -> &'static Database {
    static DATABASE: OnceLock<Database> = OnceLock::new();
    DATABASE.get_or_init(|| Database::parse(files::FILES))
}

/// What a photo's metadata says about how it was taken.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shot<'a> {
    pub make: &'a str,
    pub model: &'a str,
    /// The lens's name as the camera recorded it; `None` for a fixed lens
    /// the camera doesn't name.
    pub lens: Option<&'a str>,
    /// Millimetres.
    pub focal_length: Option<f64>,
    pub f_number: Option<f64>,
}

/// A polynomial `c₀ + c₁r + c₂r² + c₃r³ + c₄r⁴` in the radius `r`, which
/// is 1 at the corner of the image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radial(pub [f64; 5]);

impl Radial {
    pub const IDENTITY: Radial = Radial([1.0, 0.0, 0.0, 0.0, 0.0]);

    pub fn at(&self, r: f64) -> f64 {
        self.0.iter().rev().fold(0.0, |sum, c| sum * r + c)
    }
}

/// Lateral chromatic aberration: how much farther from the centre than
/// green the red and blue channels are found, as a factor of the radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tca {
    pub red: Radial,
    pub blue: Radial,
}

/// The lens's corrections for one shot.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// The camera's name in the database.
    pub camera: String,
    /// The lens's name in the database.
    pub lens: String,
    /// Where the photo shows what belongs at radius `r`: at `r · factor(r)`.
    /// `None` without a calibration, and for fisheyes.
    pub distortion: Option<Radial>,
    pub tca: Option<Tca>,
    /// Light falloff: the photo is `1 + k₁r² + k₂r⁴ + k₃r⁶` times as bright
    /// as it should be at radius `r`.
    pub vignetting: Option<[f64; 3]>,
}

impl Profile {
    /// Whether the profile corrects anything at all.
    pub fn is_empty(&self) -> bool {
        self.distortion.is_none() && self.tca.is_none() && self.vignetting.is_none()
    }
}

/// The lens profile for `shot`, when the database knows its camera and
/// lens. Recent answers are remembered: every render asks.
pub fn profile(shot: &Shot) -> Option<Profile> {
    type Key = (String, String, Option<String>, Option<u64>, Option<u64>);
    static RECENT: Mutex<Vec<(Key, Option<Profile>)>> = Mutex::new(Vec::new());
    const REMEMBERED: usize = 8;
    let key = (
        shot.make.to_owned(),
        shot.model.to_owned(),
        shot.lens.map(str::to_owned),
        shot.focal_length.map(f64::to_bits),
        shot.f_number.map(f64::to_bits),
    );
    if let Ok(recent) = RECENT.lock()
        && let Some((_, profile)) = recent.iter().find(|(k, _)| *k == key)
    {
        return profile.clone();
    }
    let profile = find(shot);
    if let Ok(mut recent) = RECENT.lock() {
        if recent.len() == REMEMBERED {
            recent.remove(0);
        }
        recent.push((key, profile.clone()));
    }
    profile
}

fn find(shot: &Shot) -> Option<Profile> {
    let database = database();
    let camera = database.camera(shot.make, shot.model)?;
    let lenses = matching::lenses(database, camera, shot.lens);
    let first = lenses.first()?;
    let crop = camera.crop;
    // Lensfun uses a calibration made on a sensor no smaller than the
    // camera's (within 4 %), the one closest in size.
    let closest = |has: &dyn Fn(&Lens) -> bool| {
        lenses
            .iter()
            .copied()
            .filter(|lens| has(lens) && crop / lens.crop >= 0.96)
            .min_by(|a, b| (crop / a.crop).total_cmp(&(crop / b.crop)))
    };
    let focal = shot.focal_length.filter(|f| *f > 0.0).or_else(|| {
        (first.focal[0] > 0.0 && first.focal[0] == first.focal[1]).then_some(first.focal[0])
    });
    let distortion = focal.and_then(|focal| {
        let lens = closest(&|lens| lens.rectilinear && !lens.distortion.is_empty())?;
        distortion(lens, crop, focal)
    });
    let tca = focal.and_then(|focal| {
        let lens = closest(&|lens| !lens.tca.is_empty())?;
        tca(lens, crop, focal)
    });
    let vignetting = focal.and_then(|focal| {
        let lens = closest(&|lens| !lens.vignetting.is_empty())?;
        let aperture = shot
            .f_number
            .filter(|f| *f > 0.0)
            .or(Some(lens.aperture[0]))?;
        vignetting(lens, crop, focal, aperture)
    });
    Some(Profile {
        camera: camera.name.clone(),
        lens: first.name.clone(),
        distortion,
        tca,
        vignetting,
    })
}

/// Hugin's radius (1 at half the calibration sensor's short side) per unit
/// of ours (1 at the corner of the image).
fn hugin_scale(lens: &Lens, camera_crop: f64) -> f64 {
    lens.crop * lens.aspect.hypot(1.0) / camera_crop
}

fn distortion(lens: &Lens, camera_crop: f64, focal: f64) -> Option<Radial> {
    let model = lens.distortion[0].model;
    let samples: Vec<_> = lens
        .distortion
        .iter()
        .filter(|sample| sample.model == model)
        .map(|sample| (sample.focal, sample.terms))
        .collect();
    // Every term follows roughly 1/f; interpolating `term · f` is smoother.
    let [a, b, c] = interpolate(&samples, focal, |_| true)?;
    let s = hugin_scale(lens, camera_crop);
    // Hugin: r_source = r · P(r) in its units, with P(0) = d chosen so r = 1
    // stays put. Like Lensfun, keep the centre's scale instead: the
    // corrected radius r·d (then ours, r·s) maps to r·P(r).
    Some(Radial(match model {
        DistortionModel::PtLens => {
            let d = 1.0 - a - b - c;
            [
                1.0,
                c * s / d.powi(2),
                b * s * s / d.powi(3),
                a * s.powi(3) / d.powi(4),
                0.0,
            ]
        }
        DistortionModel::Poly3 => [1.0, 0.0, a * s * s / (1.0 - a).powi(3), 0.0, 0.0],
        DistortionModel::Poly5 => [1.0, 0.0, a * s * s, 0.0, b * s.powi(4)],
    }))
}

fn tca(lens: &Lens, camera_crop: f64, focal: f64) -> Option<Tca> {
    let model = lens.tca[0].model;
    let samples: Vec<_> = lens
        .tca
        .iter()
        .filter(|sample| sample.model == model)
        .map(|sample| (sample.focal, sample.terms))
        .collect();
    // The constant terms stay put; the others follow roughly 1/f.
    let [vr, vb, cr, cb, br, bb] = interpolate(&samples, focal, |term| term >= 2)?;
    let s = hugin_scale(lens, camera_crop);
    let radial = |v: f64, c: f64, b: f64| Radial([v, c * s, b * s * s, 0.0, 0.0]);
    Some(match model {
        TcaModel::Linear => Tca {
            red: radial(vr, 0.0, 0.0),
            blue: radial(vb, 0.0, 0.0),
        },
        TcaModel::Poly3 => Tca {
            red: radial(vr, cr, br),
            blue: radial(vb, cb, bb),
        },
    })
}

/// Lensfun's distance for vignetting when the shot's is unknown: far away.
const FOCUS_DISTANCE: f64 = 1000.0;

/// Inverse distance weighting over focal length, 1/aperture and
/// 1/distance, as Lensfun does.
fn vignetting(lens: &Lens, camera_crop: f64, focal: f64, aperture: f64) -> Option<[f64; 3]> {
    let span = lens.focal[1] - lens.focal[0];
    let along = |f: f64| {
        if span > 0.0 {
            (f - lens.focal[0]) / span
        } else {
            f - lens.focal[0]
        }
    };
    let place = |f: f64, a: f64, d: f64| [along(f), 4.0 / a, 0.1 / d];
    let target = place(focal, aperture, FOCUS_DISTANCE);
    let mut sum = [0.0; 3];
    let mut total = 0.0;
    let mut nearest = f64::INFINITY;
    for sample in &lens.vignetting {
        let at = place(sample.focal, sample.aperture, sample.distance);
        let distance = (0..3)
            .map(|i| (at[i] - target[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        if distance < 1e-4 {
            return Some(scaled_vignetting(sample.terms, lens, camera_crop));
        }
        nearest = nearest.min(distance);
        let weight = distance.powf(-3.5);
        for (sum, term) in sum.iter_mut().zip(sample.terms) {
            *sum += weight * term;
        }
        total += weight;
    }
    if nearest > 1.0 || total <= 0.0 {
        return None;
    }
    Some(scaled_vignetting(
        sum.map(|term| term / total),
        lens,
        camera_crop,
    ))
}

/// The "pa" model's radius is 1 at the calibration sensor's corner.
fn scaled_vignetting(terms: [f64; 3], lens: &Lens, camera_crop: f64) -> [f64; 3] {
    let q2 = (lens.crop / camera_crop).powi(2);
    [terms[0] * q2, terms[1] * q2 * q2, terms[2] * q2 * q2 * q2]
}

/// Lensfun's spline through the two calibrations on either side of `focal`
/// (Hermite, with the outer ones setting the tangents), each term
/// multiplied by its focal length first when `by_focal(term)`.
fn interpolate<const N: usize>(
    samples: &[(f64, [f64; N])],
    focal: f64,
    by_focal: impl Fn(usize) -> bool,
) -> Option<[f64; N]> {
    if let Some((_, terms)) = samples.iter().find(|(f, _)| *f == focal) {
        return Some(*terms);
    }
    let mut below: Vec<_> = samples.iter().filter(|(f, _)| *f < focal).collect();
    let mut above: Vec<_> = samples.iter().filter(|(f, _)| *f > focal).collect();
    below.sort_by(|a, b| b.0.total_cmp(&a.0));
    above.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (inner_below, inner_above) = match (below.first(), above.first()) {
        (Some(low), Some(high)) => (*low, *high),
        (Some(only), None) | (None, Some(only)) => return Some(only.1),
        (None, None) => return None,
    };
    let (outer_below, outer_above) = (below.get(1), above.get(1));
    let t = (focal - inner_below.0) / (inner_above.0 - inner_below.0);
    Some(std::array::from_fn(|term| {
        let scale = |f: f64| if by_focal(term) { f } else { 1.0 };
        let value = |(f, terms): &(f64, [f64; N])| terms[term] * scale(*f);
        hermite(
            outer_below.map(|s| value(s)),
            value(inner_below),
            value(inner_above),
            outer_above.map(|s| value(s)),
            t,
        ) / scale(focal)
    }))
}

fn hermite(y0: Option<f64>, y1: f64, y2: f64, y3: Option<f64>, t: f64) -> f64 {
    let tangent1 = y0.map_or(y2 - y1, |y0| (y2 - y0) / 2.0);
    let tangent2 = y3.map_or(y2 - y1, |y3| (y3 - y1) / 2.0);
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y1
        + (t3 - 2.0 * t2 + t) * tangent1
        + (-2.0 * t3 + 3.0 * t2) * y2
        + (t3 - t2) * tangent2
}

#[cfg(test)]
mod tests;
