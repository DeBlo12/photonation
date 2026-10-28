// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Neural inpainting with an ONNX model, run by tract (pure Rust, CPU).
//!
//! Two input conventions are recognised from the model's inputs:
//!
//! | Layout | Inputs | Output |
//! |---|---|---|
//! | [`ModelLayout::ImageAndMask`] (LaMa) | `image` 1×3×H×W in 0..1, hole pixels 0; `mask` 1×1×H×W, 1 = hole | 1×3×H×W, 0..1 or 0..255 (detected) |
//! | [`ModelLayout::Stacked`] (MI-GAN) | one 1×4×H×W tensor: `known − 0.5`, then RGB in −1..1 times `known` | 1×3×H×W in −1..1 |
//!
//! Models with a fixed input size (most exports are 512×512) get the canvas
//! resized to it and the fill resized back; only hole pixels change.

use crate::canvas::resize_interleaved;
use crate::{Canvas, InpaintError, Inpainter};
use std::path::Path;
use tract_onnx::prelude::*;
use tract_onnx::tract_hir::infer::Factoid;

/// Input size used when the model does not fix one.
const DEFAULT_MODEL_SIZE: usize = 512;

/// How a model expects the image and hole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelLayout {
    /// Separate image and mask inputs (LaMa and most exports).
    ImageAndMask {
        image_input: usize,
        mask_input: usize,
    },
    /// A single tensor of mask and masked image (MI-GAN).
    Stacked,
}

pub struct OnnxInpainter {
    name: String,
    layout: ModelLayout,
    /// Model input width and height.
    size: (usize, usize),
    plan: Arc<TypedRunnableModel>,
}

impl OnnxInpainter {
    /// Loads and optimizes a model; the layout and input size are read from
    /// its inputs. Optimizing a large model takes a few seconds, so load
    /// once and keep the inpainter.
    pub fn load(path: &Path) -> Result<OnnxInpainter, InpaintError> {
        let model_error = |error: TractError| InpaintError::Model(format!("{error:#}"));
        let model = tract_onnx::onnx()
            .model_for_path(path)
            .map_err(model_error)?;
        let inputs = model.input_outlets().map_err(model_error)?.len();
        let channels_of = |index: usize| -> Option<i64> {
            let fact = model.input_fact(index).ok()?;
            fact.shape.dims().nth(1)?.concretize()?.as_i64()
        };
        let layout = match inputs {
            1 if channels_of(0) == Some(4) => ModelLayout::Stacked,
            2 => {
                let mask_input = if channels_of(0) == Some(1) { 0 } else { 1 };
                ModelLayout::ImageAndMask {
                    image_input: 1 - mask_input,
                    mask_input,
                }
            }
            _ => {
                return Err(InpaintError::Model(format!(
                    "unrecognised inputs ({inputs}); expected image + mask, or one 4-channel tensor"
                )));
            }
        };
        let fixed_dimension = |axis: usize| -> Option<usize> {
            let fact = model.input_fact(0).ok()?;
            let length = fact.shape.dims().nth(axis)?.concretize()?.as_i64()?;
            usize::try_from(length).ok().filter(|length| *length > 0)
        };
        let size = (
            fixed_dimension(3).unwrap_or(DEFAULT_MODEL_SIZE),
            fixed_dimension(2).unwrap_or(DEFAULT_MODEL_SIZE),
        );
        let (width, height) = size;
        let model = match layout {
            ModelLayout::Stacked => model
                .with_input_fact(0, f32::fact([1, 4, height, width]).into())
                .map_err(model_error)?,
            ModelLayout::ImageAndMask {
                image_input,
                mask_input,
            } => model
                .with_input_fact(image_input, f32::fact([1, 3, height, width]).into())
                .and_then(|model| {
                    model.with_input_fact(mask_input, f32::fact([1, 1, height, width]).into())
                })
                .map_err(model_error)?,
        };
        let plan = model
            .into_optimized()
            .and_then(|model| model.into_runnable())
            .map_err(model_error)?;
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "ONNX model".into());
        Ok(OnnxInpainter {
            name,
            layout,
            size,
            plan,
        })
    }

    pub fn layout(&self) -> ModelLayout {
        self.layout
    }

    /// Runs the model on a canvas already at the model's size; returns
    /// planar RGB in 0..1.
    fn run(&self, canvas: &Canvas) -> Result<Vec<f32>, InpaintError> {
        let (width, height) = self.size;
        let pixels = width * height;
        let inference_error = |error: TractError| InpaintError::Inference(format!("{error:#}"));
        let planar_rgb = |transform: &dyn Fn(f32, f32) -> f32| -> Vec<f32> {
            let mut planes = vec![0.0; pixels * 3];
            for pixel in 0..pixels {
                let hole = f32::from(canvas.is_hole(pixel));
                for channel in 0..3 {
                    let value = canvas.rgb[pixel * 3 + channel].clamp(0.0, 1.0);
                    planes[channel * pixels + pixel] = transform(value, hole);
                }
            }
            planes
        };
        let tensor = |channels: usize, data: Vec<f32>| {
            Tensor::from_shape(&[1, channels, height, width], &data).map_err(inference_error)
        };
        let (inputs, decode): (TVec<TValue>, fn(f32) -> f32) = match self.layout {
            ModelLayout::ImageAndMask {
                image_input,
                mask_input,
            } => {
                // The hole is blanked: the network must not see what it removes
                // (exports differ in whether they mask internally).
                let image = tensor(3, planar_rgb(&|value, hole| value * (1.0 - hole)))?;
                let mask = tensor(
                    1,
                    (0..pixels).map(|p| f32::from(canvas.is_hole(p))).collect(),
                )?;
                let mut ordered = [None, None];
                ordered[image_input] = Some(image.into_tvalue());
                ordered[mask_input] = Some(mask.into_tvalue());
                (ordered.into_iter().flatten().collect(), |value| value)
            }
            ModelLayout::Stacked => {
                let mut data: Vec<f32> = (0..pixels)
                    .map(|p| 0.5 - f32::from(canvas.is_hole(p)))
                    .collect();
                data.extend(planar_rgb(&|value, hole| {
                    (value * 2.0 - 1.0) * (1.0 - hole)
                }));
                (tvec![tensor(4, data)?.into_tvalue()], |value| {
                    (value + 1.0) * 0.5
                })
            }
        };
        let outputs = self.plan.run(inputs).map_err(inference_error)?;
        let output = outputs
            .first()
            .ok_or_else(|| InpaintError::Inference("the model returned nothing".into()))?;
        // Logical (row-major) order, whatever the tensor's memory layout.
        let values: Vec<f32> = output
            .to_plain_array_view::<f32>()
            .map_err(inference_error)?
            .iter()
            .copied()
            .collect();
        if values.len() < pixels * 3 {
            return Err(InpaintError::Inference(format!(
                "expected a 3×{height}×{width} output, got {:?}",
                output.shape()
            )));
        }
        let values = &values[..pixels * 3];
        // LaMa exports differ: some end in 0..255, others in 0..1.
        let range_scale = if matches!(self.layout, ModelLayout::ImageAndMask { .. })
            && values.iter().any(|value| *value > 2.0)
        {
            1.0 / 255.0
        } else {
            1.0
        };
        Ok(values
            .iter()
            .map(|value| decode(value * range_scale).clamp(0.0, 1.0))
            .collect())
    }
}

impl Inpainter for OnnxInpainter {
    fn name(&self) -> &str {
        &self.name
    }

    /// Networks are deterministic; the seed is ignored.
    fn inpaint(&self, canvas: &Canvas, _seed: u32) -> Result<Vec<f32>, InpaintError> {
        if !canvas.is_valid() {
            return Err(InpaintError::InvalidCanvas);
        }
        let (width, height) = self.size;
        let planar = self.run(&canvas.resized(width, height))?;
        let pixels = width * height;
        let interleaved: Vec<f32> = (0..pixels)
            .flat_map(|pixel| {
                let planar = &planar;
                (0..3).map(move |channel| planar[channel * pixels + pixel])
            })
            .collect();
        let fill = resize_interleaved(
            &interleaved,
            (width, height),
            (canvas.width, canvas.height),
            3,
        );
        let mut filled = canvas.rgb.clone();
        for pixel in 0..canvas.width * canvas.height {
            if canvas.is_hole(pixel) {
                filled[pixel * 3..pixel * 3 + 3].copy_from_slice(&fill[pixel * 3..pixel * 3 + 3]);
            }
        }
        Ok(filled)
    }
}
