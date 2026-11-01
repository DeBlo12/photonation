// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use super::*;

fn shot<'a>(lens: &'a str, focal: f64, f_number: f64) -> Shot<'a> {
    Shot {
        make: "Canon",
        model: "Canon EOS 250D",
        lens: Some(lens),
        focal_length: Some(focal),
        f_number: Some(f_number),
    }
}

#[test]
fn the_whole_database_parses() {
    let database = database();
    assert!(
        database.cameras.len() > 1000,
        "{} cameras",
        database.cameras.len()
    );
    assert!(
        database.lenses.len() > 1500,
        "{} lenses",
        database.lenses.len()
    );
    assert!(database.mounts.len() > 40);
}

#[test]
fn a_kit_zoom_on_an_aps_c_body() {
    let profile = profile(&shot("Canon EF-S 18-55mm f/3.5-5.6 III", 18.0, 3.5)).expect("found");
    assert_eq!(profile.camera, "Canon EOS 250D");
    assert_eq!(profile.lens, "Canon EF-S 18-55mm f/3.5-5.6 III");
    // Barrel distortion at 18 mm: the corner shows what belongs farther out.
    let distortion = profile.distortion.expect("calibrated");
    assert!(distortion.at(1.0) < 0.99, "{}", distortion.at(1.0));
    assert_eq!(distortion.at(0.0), 1.0);
    // The camera's own spelling finds it too.
    let tight = profile_of("Canon EF-S18-55mm f/3.5-5.6 III", 30.0);
    assert_eq!(tight.lens, "Canon EF-S 18-55mm f/3.5-5.6 III");
}

fn profile_of(lens: &str, focal: f64) -> Profile {
    profile(&shot(lens, focal, 5.6)).expect("found")
}

#[test]
fn a_prime_with_full_frame_and_aps_c_calibrations() {
    let profile = profile(&shot("Canon EF 50mm f/1.8 STM", 50.0, 1.8)).expect("found");
    assert_eq!(profile.lens, "Canon EF 50mm f/1.8 STM");
    let [k1, ..] = profile.vignetting.expect("calibrated");
    assert!(k1 < 0.0, "darker corners: {k1}");
    let gain = 1.0 + profile.vignetting.map_or(0.0, |k| k[0] + k[1] + k[2]);
    assert!(gain > 0.3 && gain < 1.0, "{gain}");
    assert!(profile.distortion.is_some());
}

#[test]
fn interpolation_passes_through_the_calibrations() {
    let at = |focal| {
        profile_of("Canon EF-S 18-55mm f/3.5-5.6 III", focal)
            .distortion
            .expect("calibrated")
    };
    let (low, middle, high) = (at(24.0), at(25.5), at(27.0));
    // Between two calibrations the corner lands between theirs.
    let corner = [low, middle, high].map(|d| d.at(1.0));
    assert!(corner[1] > corner[0].min(corner[2]) - 0.005);
    assert!(corner[1] < corner[0].max(corner[2]) + 0.005);
    // Close to a calibration, close to its value.
    assert!((at(24.001).at(1.0) - corner[0]).abs() < 1e-4);
}

#[test]
fn unknown_cameras_and_lenses_find_nothing() {
    assert!(
        profile(&Shot {
            make: "Apple",
            model: "iPhone 16 Pro",
            lens: Some("iPhone 16 Pro back triple camera 6.765mm f/1.78"),
            ..Shot::default()
        })
        .is_none()
    );
    assert!(profile(&shot("Canon EF 50mm f/1.9 Imaginary", 50.0, 2.0)).is_none());
    // A lens named on the 250D but not in the database: nothing.
    assert!(
        profile(&Shot {
            lens: None,
            ..shot("", 50.0, 2.0)
        })
        .is_none()
    );
}

#[test]
fn hermite_is_linear_without_outer_points() {
    assert!((hermite(None, 1.0, 3.0, None, 0.25) - 1.5).abs() < 1e-12);
    assert_eq!(
        Radial([1.0, 2.0, 3.0, 0.0, 0.5]).at(2.0),
        1.0 + 4.0 + 12.0 + 8.0
    );
}
