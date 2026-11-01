// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The Lensfun database's cameras, mounts and lenses, read from its XML.

use crate::matching::{guessed_parameters, normalized};

pub struct Database {
    pub cameras: Vec<Camera>,
    pub mounts: Vec<Mount>,
    pub lenses: Vec<Lens>,
}

pub struct Camera {
    /// The first model name, for display.
    pub name: String,
    /// Every maker and model name (all languages), normalized.
    pub makers: Vec<String>,
    pub models: Vec<String>,
    pub mount: String,
    pub crop: f64,
}

pub struct Mount {
    pub name: String,
    pub compatible: Vec<String>,
}

pub struct Lens {
    /// The first model name, for display.
    pub name: String,
    /// Every model name (all languages), as written.
    pub models: Vec<String>,
    pub mounts: Vec<String>,
    /// Crop factor and aspect ratio of the sensor it was calibrated on.
    pub crop: f64,
    pub aspect: f64,
    /// Not a fisheye or panoramic lens.
    pub rectilinear: bool,
    /// Shortest and longest focal length, 0 when unknown.
    pub focal: [f64; 2],
    /// Widest and narrowest aperture (f-numbers), 0 when unknown.
    pub aperture: [f64; 2],
    pub distortion: Vec<Distortion>,
    pub tca: Vec<TcaSample>,
    pub vignetting: Vec<Vignetting>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistortionModel {
    /// `r · (a r³ + b r² + c r + 1 − a − b − c)`
    PtLens,
    /// `r · (1 − k₁ + k₁ r²)`
    Poly3,
    /// `r · (1 + k₁ r² + k₂ r⁴)`
    Poly5,
}

pub struct Distortion {
    pub model: DistortionModel,
    pub focal: f64,
    /// a, b, c; or k₁ (and k₂).
    pub terms: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcaModel {
    /// Red and blue scaled by `kr`, `kb`.
    Linear,
    /// `r · (b r² + c r + v)` for red and blue.
    Poly3,
}

pub struct TcaSample {
    pub model: TcaModel,
    pub focal: f64,
    /// vr, vb, cr, cb, br, bb (kr, kb for the linear model).
    pub terms: [f64; 6],
}

/// The "pa" model: brightness `1 + k₁r² + k₂r⁴ + k₃r⁶` times the true one.
pub struct Vignetting {
    pub focal: f64,
    pub aperture: f64,
    /// Metres.
    pub distance: f64,
    pub terms: [f64; 3],
}

impl Database {
    /// Reads every file; a file that doesn't parse is left out.
    pub fn parse(files: &[(&str, &str)]) -> Database {
        let mut database = Database {
            cameras: Vec::new(),
            mounts: Vec::new(),
            lenses: Vec::new(),
        };
        for (_, text) in files {
            let options = roxmltree::ParsingOptions {
                allow_dtd: true,
                ..roxmltree::ParsingOptions::default()
            };
            let Ok(document) = roxmltree::Document::parse_with_options(text, options) else {
                continue;
            };
            for node in document
                .root_element()
                .children()
                .filter(|n| n.is_element())
            {
                match node.tag_name().name() {
                    "camera" => database.cameras.extend(camera(node)),
                    "mount" => database.mounts.extend(mount(node)),
                    "lens" => database.lenses.extend(lens(node)),
                    _ => {}
                }
            }
        }
        database
    }

    /// The camera named `make` `model` (some cameras repeat the make in the
    /// model, some don't).
    pub fn camera(&self, make: &str, model: &str) -> Option<&Camera> {
        let model = normalized(model);
        let make = normalized(make);
        let with_make = format!("{make} {model}");
        let named = |camera: &&Camera| {
            camera
                .models
                .iter()
                .any(|name| *name == model || *name == with_make)
        };
        let mut found = self.cameras.iter().filter(named);
        let first = found.next()?;
        Some(
            std::iter::once(first)
                .chain(found)
                .find(|camera| camera.makers.contains(&make))
                .unwrap_or(first),
        )
    }

    /// The mounts a camera with `mount` takes lenses of, besides its own.
    pub fn compatible(&self, mount: &str) -> &[String] {
        self.mounts
            .iter()
            .find(|m| m.name == mount)
            .map_or(&[], |m| m.compatible.as_slice())
    }
}

fn texts<'a>(node: roxmltree::Node<'a, 'a>, tag: &str) -> Vec<&'a str> {
    node.children()
        .filter(|child| child.has_tag_name(tag))
        .filter_map(|child| child.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect()
}

fn number(node: roxmltree::Node, attribute: &str) -> Option<f64> {
    node.attribute(attribute)?.trim().parse().ok()
}

fn camera(node: roxmltree::Node) -> Option<Camera> {
    let models = texts(node, "model");
    Some(Camera {
        name: models.first()?.to_string(),
        makers: texts(node, "maker").into_iter().map(normalized).collect(),
        models: models.into_iter().map(normalized).collect(),
        mount: texts(node, "mount").first()?.to_string(),
        crop: texts(node, "cropfactor").first()?.parse().ok()?,
    })
}

fn mount(node: roxmltree::Node) -> Option<Mount> {
    Some(Mount {
        name: texts(node, "name").first()?.to_string(),
        compatible: texts(node, "compat")
            .into_iter()
            .map(String::from)
            .collect(),
    })
}

/// "3:2", "4:3" or a plain ratio; Lensfun assumes 3:2.
fn aspect(text: Option<&&str>) -> f64 {
    let parsed = text.and_then(|text| match text.split_once(':') {
        Some((w, h)) => Some(w.trim().parse::<f64>().ok()? / h.trim().parse::<f64>().ok()?),
        None => text.parse().ok(),
    });
    match parsed {
        Some(ratio) if ratio.is_finite() && ratio > 0.0 => ratio.max(1.0 / ratio),
        _ => 1.5,
    }
}

fn lens(node: roxmltree::Node) -> Option<Lens> {
    let models: Vec<String> = texts(node, "model").into_iter().map(String::from).collect();
    let crop: f64 = texts(node, "cropfactor").first()?.parse().ok()?;
    let rectilinear = texts(node, "type")
        .first()
        .is_none_or(|kind| *kind == "rectilinear");
    let mut lens = Lens {
        name: models.first()?.clone(),
        mounts: texts(node, "mount").into_iter().map(String::from).collect(),
        crop,
        aspect: aspect(texts(node, "aspect-ratio").first()),
        rectilinear,
        focal: [0.0; 2],
        aperture: [0.0; 2],
        distortion: Vec::new(),
        tca: Vec::new(),
        vignetting: Vec::new(),
        models,
    };
    for child in node.children() {
        match child.tag_name().name() {
            "focal" => {
                lens.focal = [number(child, "min"), number(child, "max")].map(|f| f.unwrap_or(0.0));
                if let Some(value) = number(child, "value") {
                    lens.focal = [value; 2];
                }
            }
            "aperture" => {
                lens.aperture =
                    [number(child, "min"), number(child, "max")].map(|f| f.unwrap_or(0.0));
            }
            "calibration" => calibration(child, &mut lens),
            _ => {}
        }
    }
    guess_ranges(&mut lens);
    Some(lens)
}

fn calibration(node: roxmltree::Node, lens: &mut Lens) {
    for child in node.children().filter(|n| n.is_element()) {
        let Some(focal) = number(child, "focal") else {
            continue;
        };
        let term = |name: &str| number(child, name).unwrap_or(0.0);
        match (child.tag_name().name(), child.attribute("model")) {
            ("distortion", Some(model)) => {
                let (model, terms) = match model {
                    "ptlens" => (DistortionModel::PtLens, [term("a"), term("b"), term("c")]),
                    "poly3" => (DistortionModel::Poly3, [term("k1"), 0.0, 0.0]),
                    "poly5" => (DistortionModel::Poly5, [term("k1"), term("k2"), 0.0]),
                    _ => continue,
                };
                lens.distortion.push(Distortion {
                    model,
                    focal,
                    terms,
                });
            }
            ("tca", Some(model)) => {
                let (model, terms) = match model {
                    "linear" => (
                        TcaModel::Linear,
                        [term("kr"), term("kb"), 0.0, 0.0, 0.0, 0.0],
                    ),
                    "poly3" => {
                        let one = |name: &str| number(child, name).unwrap_or(1.0);
                        (
                            TcaModel::Poly3,
                            [
                                one("vr"),
                                one("vb"),
                                term("cr"),
                                term("cb"),
                                term("br"),
                                term("bb"),
                            ],
                        )
                    }
                    _ => continue,
                };
                lens.tca.push(TcaSample {
                    model,
                    focal,
                    terms,
                });
            }
            ("vignetting", Some("pa")) => {
                let (Some(aperture), Some(distance)) =
                    (number(child, "aperture"), number(child, "distance"))
                else {
                    continue;
                };
                lens.vignetting.push(Vignetting {
                    focal,
                    aperture,
                    distance,
                    terms: [term("k1"), term("k2"), term("k3")],
                });
            }
            _ => {}
        }
    }
}

/// Fills unknown focal lengths and apertures as Lensfun does: from the
/// name, then from the calibrations.
fn guess_ranges(lens: &mut Lens) {
    if lens.focal[0] > 0.0 && lens.aperture[0] > 0.0 {
        return;
    }
    let guessed = guessed_parameters(&lens.name);
    let mut focal = [guessed.focal[0], guessed.focal[1]];
    let mut aperture = [guessed.aperture, 0.0];
    let focals = lens
        .distortion
        .iter()
        .map(|d| d.focal)
        .chain(lens.tca.iter().map(|t| t.focal))
        .chain(lens.vignetting.iter().map(|v| v.focal));
    for f in focals {
        focal[0] = if focal[0] > 0.0 { focal[0].min(f) } else { f };
        focal[1] = focal[1].max(f);
    }
    for v in &lens.vignetting {
        aperture[0] = if aperture[0] > 0.0 {
            aperture[0].min(v.aperture)
        } else {
            v.aperture
        };
        aperture[1] = aperture[1].max(v.aperture);
    }
    for (known, guess) in lens.focal.iter_mut().zip(focal) {
        if *known == 0.0 {
            *known = guess;
        }
    }
    for (known, guess) in lens.aperture.iter_mut().zip(aperture) {
        if *known == 0.0 {
            *known = guess;
        }
    }
}
