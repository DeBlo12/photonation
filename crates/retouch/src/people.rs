// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Finding people in a photo with an RF-DETR segmentation model (ONNX, run
//! by tract), so a click can select one. See docs/RETOUCH.md.
//!
//! The model takes one `1×3×S×S` image (RGB in 0..1, ImageNet normalized,
//! the photo stretched to a square) and returns, for each of its queries,
//! a box (`1×Q×4`), class logits (`1×Q×91`, sparse COCO ids, 1 = person)
//! and mask logits over the whole image (`1×Q×S/4×S/4`). Outputs are told
//! apart by their shapes, so their names and order don't matter.

use crate::canvas::{RgbImage, resize_interleaved};
use std::path::Path;
use tract_onnx::prelude::*;
use tract_onnx::tract_hir::infer::Factoid;

/// COCO's category id for a person, the index into the class logits.
const PERSON_CLASS: usize = 1;
/// Lowest class probability that counts as a person. On the test photos
/// every real person scored 0.84 or more, and a signpost 0.40.
const MIN_PERSON_SCORE: f32 = 0.5;
/// Two queries whose masks overlap this much are the same person.
const DUPLICATE_OVERLAP: f32 = 0.7;
/// ImageNet statistics the backbone was trained with.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STANDARD_DEVIATION: [f32; 3] = [0.229, 0.224, 0.225];

#[derive(Debug, thiserror::Error)]
pub enum PeopleError {
    #[error("the image is empty or its buffer does not match its size")]
    InvalidImage,
    #[error("could not load the people model: {0}")]
    Model(String),
    #[error("finding people failed: {0}")]
    Inference(String),
}

/// One person found: how sure the model is, and where they are as a grid of
/// probabilities stretched over the whole image (row-major, 0..1).
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub score: f32,
    pub width: usize,
    pub height: usize,
    pub mask: Vec<f32>,
}

impl Person {
    /// Grid cells that belong to the person (probability at least 0.5).
    pub fn cells(&self) -> impl Iterator<Item = bool> + '_ {
        self.mask.iter().map(|probability| *probability >= 0.5)
    }
}

pub struct PeopleDetector {
    name: String,
    /// The model's square input edge.
    size: usize,
    plan: Arc<TypedRunnableModel>,
}

impl PeopleDetector {
    /// Loads and optimizes a model (a few seconds); keep the detector.
    pub fn load(path: &Path) -> Result<PeopleDetector, PeopleError> {
        let model_error = |error: TractError| PeopleError::Model(format!("{error:#}"));
        let model = tract_onnx::onnx()
            .model_for_path(path)
            .map_err(model_error)?;
        if model.input_outlets().map_err(model_error)?.len() != 1 {
            return Err(PeopleError::Model(
                "expected one image input (an RF-DETR segmentation export)".into(),
            ));
        }
        let dimension = |axis: usize| -> Option<usize> {
            let fact = model.input_fact(0).ok()?;
            let length = fact.shape.dims().nth(axis)?.concretize()?.as_i64()?;
            usize::try_from(length).ok().filter(|length| *length > 0)
        };
        let size = match (dimension(1), dimension(2), dimension(3)) {
            (Some(3), Some(height), Some(width)) if height == width => width,
            _ => {
                return Err(PeopleError::Model(
                    "expected a fixed 1×3×S×S image input (an RF-DETR segmentation export)".into(),
                ));
            }
        };
        let plan = model
            .with_input_fact(0, f32::fact([1, 3, size, size]).into())
            .and_then(|model| model.into_optimized())
            .and_then(|model| model.into_runnable())
            .map_err(model_error)?;
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "ONNX model".into());
        Ok(PeopleDetector { name, size, plan })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The people in `image`, most certain first.
    pub fn find(&self, image: &RgbImage) -> Result<Vec<Person>, PeopleError> {
        if !image.is_valid() {
            return Err(PeopleError::InvalidImage);
        }
        let inference_error = |error: TractError| PeopleError::Inference(format!("{error:#}"));
        let size = self.size;
        let square = resize_interleaved(&image.rgb, (image.width, image.height), (size, size), 3);
        let pixels = size * size;
        let mut planes = vec![0.0; pixels * 3];
        for (pixel, rgb) in square.chunks_exact(3).enumerate() {
            for channel in 0..3 {
                planes[channel * pixels + pixel] =
                    (rgb[channel].clamp(0.0, 1.0) - MEAN[channel]) / STANDARD_DEVIATION[channel];
            }
        }
        let input = Tensor::from_shape(&[1, 3, size, size], &planes).map_err(inference_error)?;
        let outputs = self
            .plan
            .run(tvec![input.into_tvalue()])
            .map_err(inference_error)?;
        let shaped = |rank: usize, last: Option<usize>| {
            outputs.iter().find(|output| {
                output.rank() == rank && last.is_none_or(|last| output.shape()[rank - 1] == last)
            })
        };
        let masks = shaped(4, None)
            .ok_or_else(|| PeopleError::Inference("the model returned no masks".into()))?;
        let labels = outputs
            .iter()
            .find(|output| {
                output.rank() == 3 && output.shape()[2] > PERSON_CLASS && output.shape()[2] != 4
            })
            .ok_or_else(|| PeopleError::Inference("the model returned no class scores".into()))?;
        let plain = |tensor: &TValue| -> Result<Vec<f32>, PeopleError> {
            Ok(tensor
                .to_plain_array_view::<f32>()
                .map_err(inference_error)?
                .iter()
                .copied()
                .collect())
        };
        let (queries, classes) = (labels.shape()[1], labels.shape()[2]);
        let (mask_queries, mask_height, mask_width) =
            (masks.shape()[1], masks.shape()[2], masks.shape()[3]);
        if mask_queries != queries {
            return Err(PeopleError::Inference(format!(
                "{queries} class scores but {mask_queries} masks"
            )));
        }
        let (labels, masks) = (plain(labels)?, plain(masks)?);
        let cells = mask_width * mask_height;
        let mut people: Vec<Person> = (0..queries)
            .filter_map(|query| {
                let score = sigmoid(labels[query * classes + PERSON_CLASS]);
                (score >= MIN_PERSON_SCORE).then(|| Person {
                    score,
                    width: mask_width,
                    height: mask_height,
                    mask: masks[query * cells..(query + 1) * cells]
                        .iter()
                        .map(|logit| sigmoid(*logit))
                        .collect(),
                })
            })
            .filter(|person| person.cells().any(|inside| inside))
            .collect();
        people.sort_by(|a, b| b.score.total_cmp(&a.score));
        Ok(without_duplicates(people))
    }
}

fn sigmoid(logit: f32) -> f32 {
    1.0 / (1.0 + (-logit).exp())
}

/// Drops people whose mask mostly repeats a more certain one's.
fn without_duplicates(people: Vec<Person>) -> Vec<Person> {
    let mut kept: Vec<Person> = Vec::with_capacity(people.len());
    for person in people {
        let duplicate = kept
            .iter()
            .any(|other| overlap(other, &person) >= DUPLICATE_OVERLAP);
        if !duplicate {
            kept.push(person);
        }
    }
    kept
}

/// Intersection over union of two people's cells.
fn overlap(a: &Person, b: &Person) -> f32 {
    let (mut both, mut either) = (0usize, 0usize);
    for (in_a, in_b) in a.cells().zip(b.cells()) {
        both += usize::from(in_a && in_b);
        either += usize::from(in_a || in_b);
    }
    if either == 0 {
        0.0
    } else {
        both as f32 / either as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(score: f32, cells: &[usize]) -> Person {
        let mut mask = vec![0.0; 16];
        for cell in cells {
            mask[*cell] = 0.9;
        }
        Person {
            score,
            width: 4,
            height: 4,
            mask,
        }
    }

    #[test]
    fn a_mask_repeating_a_more_certain_one_is_dropped() {
        let people = vec![
            person(0.9, &[0, 1, 4, 5]),
            person(0.8, &[0, 1, 4, 5, 8]),
            person(0.7, &[10, 11, 14, 15]),
        ];
        let kept = without_duplicates(people);
        assert_eq!(kept.iter().map(|p| p.score).collect::<Vec<_>>(), [0.9, 0.7]);
    }

    #[test]
    fn an_empty_image_is_refused_before_the_model_runs() {
        let image = RgbImage {
            width: 2,
            height: 2,
            rgb: vec![0.0; 3],
        };
        assert!(!image.is_valid());
    }
}
