// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The mask overlay: while a mask is selected, the photo is tinted where it
//! applies. A fragment shader (`mask_overlay.wgsl`) evaluates the mask at
//! every screen pixel on the GPU, so the tint follows handle drags at once,
//! without waiting for the photo to re-render.

use crate::photo_frame::PhotoFrame;
use eframe::egui::{self, Color32, Rgba, pos2};
use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, RenderState, ScreenDescriptor};
use eframe::wgpu;
use photonation_develop::{CombineMode, MAX_MASK_COMPONENTS};
use photonation_pipeline::brush_map::{BrushMap, CELLS_PER_LONG_EDGE};
use photonation_pipeline::masks::{MaskWeight, ShapeWeight};
use std::num::NonZeroU64;
use std::sync::Arc;

const SHADER: &str = include_str!("mask_overlay.wgsl");
/// Words in the shader's `Overlay` header: seven 16-byte rows.
const HEADER_WORDS: usize = 28;
/// Words per entry of its `components` array: three 16-byte rows.
const COMPONENT_WORDS: usize = 12;
/// The shader's whole `Overlay` uniform, in 32-bit words.
const UNIFORM_WORDS: usize = HEADER_WORDS + MAX_MASK_COMPONENTS * COMPONENT_WORDS;

/// GPU objects shared by every overlay draw, kept in egui's callback resources.
struct OverlayPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// The brush map in the bind group's texture. A mask has at most one
    /// brush; while none is drawn, the last one just stays there unused.
    brush: Option<Arc<BrushMap>>,
    /// The framebuffer encodes sRGB itself, so colours must be linear.
    linear_framebuffer: bool,
}

/// Creates the overlay's pipeline on eframe's device. eframe renders without
/// multisampling or a depth buffer (see `main.rs`), so neither is used here.
pub fn install(render_state: &RenderState) {
    let device = &render_state.device;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("mask overlay"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let uniform_size = (UNIFORM_WORDS * 4) as u64;
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mask overlay uniforms"),
        size: uniform_size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("mask overlay"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(uniform_size),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    // Bilinear with edges clamped, as `BrushMap::at` samples.
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("mask overlay brush"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..wgpu::SamplerDescriptor::default()
    });
    let bind_group = brush_bind_group(
        device,
        &render_state.queue,
        &layout,
        &uniforms,
        &sampler,
        &BrushMap::empty(),
    );
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("mask overlay"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mask overlay"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: render_state.target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache: None,
    });
    render_state
        .renderer
        .write()
        .callback_resources
        .insert(OverlayPipeline {
            pipeline,
            layout,
            bind_group,
            uniforms,
            sampler,
            brush: None,
            linear_framebuffer: render_state.target_format.is_srgb(),
        });
}

/// A bind group whose texture holds `map`. Eight bits are plenty for a tint,
/// and unlike 32-bit floats this format can be filtered on every device.
fn brush_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    map: &BrushMap,
) -> wgpu::BindGroup {
    let size = wgpu::Extent3d {
        width: map.width as u32,
        height: map.height as u32,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mask overlay brush"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes: Vec<u8> = map
        .weights
        .iter()
        .map(|weight| (weight * 255.0).round() as u8)
        .collect();
    queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(map.width as u32),
            rows_per_image: Some(map.height as u32),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mask overlay"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// One overlay draw: a mask's weight over the photo, as a tint.
pub struct MaskOverlay {
    pub frame: PhotoFrame,
    /// At most [`MAX_MASK_COMPONENTS`] shapes are drawn; sanitized masks
    /// have no more.
    pub weight: MaskWeight,
    /// Tint at full weight.
    pub color: Color32,
}

impl MaskOverlay {
    /// Draws over the photo, clipped like everything else `painter` draws.
    pub fn paint(self, painter: &egui::Painter) {
        let rect = self.frame.image_rect;
        painter.add(egui_wgpu::Callback::new_paint_callback(rect, self));
    }

    /// The mask's brush, which the shader reads from a texture. Sanitized
    /// masks have at most one.
    fn brush_map(&self) -> Option<&Arc<BrushMap>> {
        self.weight
            .components
            .iter()
            .find_map(|(shape, _)| match shape {
                ShapeWeight::Brush(map) => Some(map),
                _ => None,
            })
    }

    /// The shader's `Overlay` uniform, as 32-bit words.
    fn uniform_words(&self, pixels_per_point: f32, linear_framebuffer: bool) -> Vec<u32> {
        // Framebuffer pixel p (with its centre at p + 0.5, as the shader
        // sees it) is screen point p / pixels_per_point.
        // The mapping to the corrected photo is affine (a straightened crop
        // turns it), so the origin and one step along each axis describe
        // it; the shader follows the lens map from there.
        let origin = self.frame.to_corrected(pos2(0.0, 0.0));
        let one_pixel = 1.0 / pixels_per_point;
        let step_along = |screen| {
            let next = self.frame.to_corrected(screen);
            [next[0] - origin[0], next[1] - origin[1]]
        };
        let step_x = step_along(pos2(one_pixel, 0.0));
        let step_y = step_along(pos2(0.0, one_pixel));
        let color = if linear_framebuffer {
            Rgba::from(self.color).to_array()
        } else {
            self.color.to_normalized_gamma_f32()
        };
        let components =
            &self.weight.components[..self.weight.components.len().min(MAX_MASK_COMPONENTS)];
        let mut words = Vec::with_capacity(UNIFORM_WORDS);
        words.extend([origin[0], origin[1], step_x[0], step_x[1]].map(|v| (v as f32).to_bits()));
        words.extend([step_y[0], step_y[1]].map(|v| (v as f32).to_bits()));
        words.extend([components.len() as u32, u32::from(self.weight.invert)]);
        words.extend(color.map(f32::to_bits));
        let scale = self.frame.space.scale();
        let floats = |values: [f64; 4]| values.map(|v| (v as f32).to_bits());
        match &self.frame.lens {
            Some(lens) => {
                let [width, height] = lens.size();
                let zoom = lens.zoom();
                words.extend(floats([
                    width * zoom,
                    height * zoom,
                    2.0 / width.hypot(height),
                    1.0,
                ]));
                words.extend(floats([1.0 / width, 1.0 / height, scale[0], scale[1]]));
                let d = lens.distortion();
                words.extend(floats([d[0], d[1], d[2], d[3]]));
                words.extend(floats([d[4], d[5], d[6], 0.0]));
            }
            None => {
                words.extend(floats([0.0; 4]));
                words.extend(floats([0.0, 0.0, scale[0], scale[1]]));
                words.extend([0; 8]);
            }
        }
        for (shape, mode) in components {
            words.extend(component_words(shape, *mode));
        }
        words.resize(UNIFORM_WORDS, 0);
        words
    }
}

/// One entry of the shader's `components` array.
fn component_words(shape: &ShapeWeight, mode: CombineMode) -> [u32; COMPONENT_WORDS] {
    let (kind, fade_start, a, b) = match *shape {
        ShapeWeight::Brush(ref map) => (
            2,
            0.0,
            [map.origin[0], map.origin[1], CELLS_PER_LONG_EDGE, 0.0],
            [map.width as f32, map.height as f32, 0.0, 0.0],
        ),
        ShapeWeight::Linear { start, ramp } => {
            (0, 0.0, [start[0], start[1], ramp[0], ramp[1]], [0.0; 4])
        }
        ShapeWeight::Radial {
            center,
            axis_x,
            axis_y,
            fade_start,
        } => (
            1,
            fade_start,
            [center[0], center[1], 0.0, 0.0],
            [axis_x[0], axis_x[1], axis_y[0], axis_y[1]],
        ),
    };
    let mode = match mode {
        CombineMode::Add => 0,
        CombineMode::Subtract => 1,
        CombineMode::Intersect => 2,
    };
    let mut words = [0; COMPONENT_WORDS];
    for (word, value) in words.iter_mut().zip(a.iter().chain(&b)) {
        *word = value.to_bits();
    }
    words[8..].copy_from_slice(&[kind, mode, fade_start.to_bits(), 0]);
    words
}

impl CallbackTrait for MaskOverlay {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(overlay) = resources.get_mut::<OverlayPipeline>() {
            if let Some(map) = self.brush_map()
                && !overlay
                    .brush
                    .as_ref()
                    .is_some_and(|uploaded| Arc::ptr_eq(uploaded, map))
            {
                overlay.bind_group = brush_bind_group(
                    device,
                    queue,
                    &overlay.layout,
                    &overlay.uniforms,
                    &overlay.sampler,
                    map,
                );
                overlay.brush = Some(map.clone());
            }
            let words = self.uniform_words(screen.pixels_per_point, overlay.linear_framebuffer);
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
            queue.write_buffer(&overlay.uniforms, 0, &bytes);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if let Some(overlay) = resources.get::<OverlayPipeline>() {
            render_pass.set_pipeline(&overlay.pipeline);
            render_pass.set_bind_group(0, &overlay.bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Rect, vec2};
    use photonation_develop::{
        BrushStroke, LensCorrection, LinearGradient, Mask, MaskBrush, MaskComponent, MaskShape,
        MaskSpace, MaskStroke, RadialGradient,
    };
    use photonation_image::{CropFrame, NormRect};
    use photonation_pipeline::LensMap;

    #[test]
    fn shader_compiles_and_validates() {
        let module = naga::front::wgsl::parse_str(SHADER).expect("WGSL parses");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("WGSL validates");
        let size = module
            .global_variables
            .iter()
            .find(|(_, global)| global.name.as_deref() == Some("overlay"))
            .map(|(_, global)| module.types[global.ty].inner.size(module.to_ctx()))
            .expect("overlay uniform");
        assert_eq!(size as usize, UNIFORM_WORDS * 4, "uniform layout matches");
    }

    /// A bilinear texture lookup at `uv`, clamped to the edge. Reads the
    /// same weights the texture holds, before they round to 8 bits.
    fn brush_twin(map: &BrushMap, uv: [f32; 2]) -> f32 {
        let x = (uv[0] * map.width as f32 - 0.5).clamp(0.0, (map.width - 1) as f32);
        let y = (uv[1] * map.height as f32 - 0.5).clamp(0.0, (map.height - 1) as f32);
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(map.width - 1), (y0 + 1).min(map.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| map.weights[y * map.width + x];
        let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
        let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
        top + (bottom - top) * fy
    }

    /// The shader's `weight`, transcribed, reading the packed uniform: checks
    /// that the packing and the pixel mapping reproduce the pipeline's weights.
    fn shader_twin(words: &[u32], map: Option<&Arc<BrushMap>>, pixel_centre: [f32; 2]) -> f32 {
        let f = |index: usize| f32::from_bits(words[index]);
        let corrected = [
            f(0) + pixel_centre[0] * f(2) + pixel_centre[1] * f(4),
            f(1) + pixel_centre[0] * f(3) + pixel_centre[1] * f(5),
        ];
        let shot = if f(15) != 0.0 {
            let offset = [(corrected[0] - 0.5) * f(12), (corrected[1] - 0.5) * f(13)];
            let rho = (offset[0] * offset[0] + offset[1] * offset[1]).sqrt() * f(14);
            let factor = (20..27).rev().fold(0.0, |sum, i| sum * rho + f(i));
            [
                0.5 + offset[0] * factor * f(16),
                0.5 + offset[1] * factor * f(17),
            ]
        } else {
            corrected
        };
        let point = [shot[0] * f(18), shot[1] * f(19)];
        let ramp = |edge0: f32, edge1: f32, x: f32| {
            let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let count = (words[6] as usize).min(MAX_MASK_COMPONENTS);
        let mut selected = 0.0;
        for index in 0..count {
            let base = HEADER_WORDS + index * COMPONENT_WORDS;
            let c = |offset: usize| f(base + offset);
            let offset = [point[0] - c(0), point[1] - c(1)];
            let w = if words[base + 8] == 0 {
                1.0 - ramp(0.0, 1.0, offset[0] * c(2) + offset[1] * c(3))
            } else if words[base + 8] == 2 {
                // Hardware bilinear filtering of the texture.
                let uv = [
                    (offset[0] * c(2) + 0.5) / c(4),
                    (offset[1] * c(2) + 0.5) / c(5),
                ];
                brush_twin(map.expect("a brush has a map"), uv)
            } else {
                let x = offset[0] * c(4) + offset[1] * c(5);
                let y = offset[0] * c(6) + offset[1] * c(7);
                let distance = (x * x + y * y).sqrt();
                if c(10) >= 1.0 {
                    f32::from(distance <= 1.0)
                } else {
                    1.0 - ramp(c(10), 1.0, distance)
                }
            };
            selected = match (index, words[base + 9]) {
                (0, _) => w,
                (_, 0) => f32::max(selected, w),
                (_, 1) => f32::min(selected, 1.0 - w),
                _ => f32::min(selected, w),
            };
        }
        if words[7] != 0 {
            1.0 - selected
        } else {
            selected
        }
    }

    #[test]
    fn packed_uniforms_reproduce_the_pipeline_weights() {
        let space = MaskSpace::new(6000.0, 4000.0);
        let cropped = CropFrame::new(
            [6000.0, 4000.0],
            NormRect {
                left: 0.1,
                top: 0.0,
                right: 0.9,
                bottom: 1.0,
            },
            0.0,
        );
        let straightened = CropFrame::largest([6000.0, 4000.0], 1.5, -8.0);
        let lens = LensMap::new(
            [6000.0, 4000.0],
            &LensCorrection {
                manual_distortion: -70.0,
                ..LensCorrection::default()
            },
            None,
        );
        for (crop, lens) in [
            (cropped, None),
            (straightened, None),
            (cropped, lens),
            (straightened, lens),
        ] {
            packed_uniforms_match(PhotoFrame {
                image_rect: Rect::from_min_size(pos2(40.0, 30.0), vec2(600.0, 400.0)),
                crop,
                space,
                lens,
            });
        }
    }

    fn packed_uniforms_match(frame: PhotoFrame) {
        let space = frame.space;
        let pixels_per_point = 2.0;
        let linear = MaskShape::Linear(LinearGradient {
            start: [0.3, 0.2],
            end: [0.6, 0.7],
        });
        let radial = MaskShape::Radial(RadialGradient {
            center: [0.45, 0.55],
            radius: [0.25, 0.1],
            rotation: 30.0,
            feather: 40.0,
        });
        let brush = MaskShape::Brush(MaskBrush {
            strokes: vec![MaskStroke {
                path: BrushStroke {
                    points: vec![[0.2, 0.3], [0.4, 0.35], [0.5, 0.6], [0.8, 0.7]],
                    radius: 0.09,
                },
                ..MaskStroke::default()
            }],
        });
        let mut masks = vec![
            Mask::new("Linear", linear.clone()),
            Mask::new("Radial", radial.clone()),
            Mask::new("Brush", brush.clone()),
        ];
        for mode in CombineMode::ALL {
            let mut combined = Mask::new("Combined", radial.clone());
            combined
                .components
                .push(MaskComponent::new(linear.clone(), mode));
            masks.push(combined);
            let mut brushed = Mask::new("Brushed", linear.clone());
            brushed
                .components
                .push(MaskComponent::new(brush.clone(), mode));
            masks.push(brushed);
        }
        for mut mask in masks {
            for invert in [false, true] {
                mask.invert = invert;
                let overlay = MaskOverlay {
                    frame,
                    weight: MaskWeight::new(&mask, space),
                    color: Color32::RED,
                };
                let words = overlay.uniform_words(pixels_per_point, false);
                assert_eq!(words.len(), UNIFORM_WORDS);
                for pixel in [
                    [100.0, 80.0],
                    [500.5, 520.5],
                    [640.5, 470.5],
                    [1200.0, 820.0],
                ] {
                    let centre = [pixel[0] + 0.5, pixel[1] + 0.5];
                    let screen = pos2(centre[0] / pixels_per_point, centre[1] / pixels_per_point);
                    let metric = frame.screen_to_metric(screen).map(|v| v as f32);
                    let expected = overlay.weight.at(metric);
                    let got = shader_twin(&words, overlay.brush_map(), centre);
                    assert!(
                        (got - expected).abs() < 1e-3,
                        "{mask:?} {pixel:?}: {got} vs {expected}"
                    );
                }
            }
        }
    }
}
