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
use photonation_pipeline::masks::ShapeWeight;
use std::num::NonZeroU64;

const SHADER: &str = include_str!("mask_overlay.wgsl");
/// The shader's `Overlay` uniform: five 16-byte rows of 32-bit words.
const UNIFORM_WORDS: usize = 20;

/// GPU objects shared by every overlay draw, kept in egui's callback resources.
struct OverlayPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
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
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(uniform_size),
            },
            count: None,
        }],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mask overlay"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }],
    });
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
            bind_group,
            uniforms,
            linear_framebuffer: render_state.target_format.is_srgb(),
        });
}

/// One overlay draw: a mask's weight over the photo, as a tint.
pub struct MaskOverlay {
    pub frame: PhotoFrame,
    pub shape: ShapeWeight,
    pub invert: bool,
    /// Tint at full weight.
    pub color: Color32,
}

impl MaskOverlay {
    /// Draws over the photo, clipped like everything else `painter` draws.
    pub fn paint(self, painter: &egui::Painter) {
        let rect = self.frame.image_rect;
        painter.add(egui_wgpu::Callback::new_paint_callback(rect, self));
    }

    /// The shader's `Overlay` uniform, as 32-bit words.
    fn uniform_words(&self, pixels_per_point: f32, linear_framebuffer: bool) -> Vec<u32> {
        // Framebuffer pixel p (with its centre at p + 0.5, as the shader
        // sees it) is screen point p / pixels_per_point.
        let origin = self.frame.screen_to_metric(pos2(0.0, 0.0));
        let one_pixel = 1.0 / pixels_per_point;
        let next = self.frame.screen_to_metric(pos2(one_pixel, one_pixel));
        let step = [next[0] - origin[0], next[1] - origin[1]];
        let (kind, fade_start, a, b) = match self.shape {
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
        let color = if linear_framebuffer {
            Rgba::from(self.color).to_array()
        } else {
            self.color.to_normalized_gamma_f32()
        };
        let mut words = Vec::with_capacity(UNIFORM_WORDS);
        words.extend([origin[0], origin[1], step[0], step[1]].map(|v| (v as f32).to_bits()));
        words.extend([kind, u32::from(self.invert), f32::to_bits(fade_start), 0]);
        words.extend(a.iter().chain(&b).chain(&color).map(|v| v.to_bits()));
        words
    }
}

impl CallbackTrait for MaskOverlay {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(overlay) = resources.get::<OverlayPipeline>() {
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
    use photonation_develop::{LinearGradient, MaskShape, MaskSpace, RadialGradient};
    use photonation_image::NormRect;

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

    /// The shader's `weight`, transcribed, reading the packed uniform: checks
    /// that the packing and the pixel mapping reproduce the pipeline's weights.
    fn shader_twin(words: &[u32], pixel_centre: [f32; 2]) -> f32 {
        let f = |index: usize| f32::from_bits(words[index]);
        let point = [f(0) + pixel_centre[0] * f(2), f(1) + pixel_centre[1] * f(3)];
        let ramp = |edge0: f32, edge1: f32, x: f32| {
            let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let offset = [point[0] - f(8), point[1] - f(9)];
        let w = if words[4] == 0 {
            1.0 - ramp(0.0, 1.0, offset[0] * f(10) + offset[1] * f(11))
        } else {
            let x = offset[0] * f(12) + offset[1] * f(13);
            let y = offset[0] * f(14) + offset[1] * f(15);
            let distance = (x * x + y * y).sqrt();
            if f(6) >= 1.0 {
                f32::from(distance <= 1.0)
            } else {
                1.0 - ramp(f(6), 1.0, distance)
            }
        };
        if words[5] != 0 { 1.0 - w } else { w }
    }

    #[test]
    fn packed_uniforms_reproduce_the_pipeline_weights() {
        let space = MaskSpace::new(6000.0, 4000.0);
        let frame = PhotoFrame {
            image_rect: Rect::from_min_size(pos2(40.0, 30.0), vec2(600.0, 400.0)),
            crop: NormRect {
                left: 0.1,
                top: 0.0,
                right: 0.9,
                bottom: 1.0,
            },
            space,
        };
        let pixels_per_point = 2.0;
        let shapes = [
            MaskShape::Linear(LinearGradient {
                start: [0.3, 0.2],
                end: [0.6, 0.7],
            }),
            MaskShape::Radial(RadialGradient {
                center: [0.45, 0.55],
                radius: [0.25, 0.1],
                rotation: 30.0,
                feather: 40.0,
            }),
        ];
        for shape in shapes {
            for invert in [false, true] {
                let overlay = MaskOverlay {
                    frame,
                    shape: ShapeWeight::new(&shape, space),
                    invert,
                    color: Color32::RED,
                };
                let words = overlay.uniform_words(pixels_per_point, false);
                assert_eq!(words.len(), UNIFORM_WORDS);
                for pixel in [[100.0, 80.0], [640.5, 470.5], [1200.0, 820.0]] {
                    let centre = [pixel[0] + 0.5, pixel[1] + 0.5];
                    let screen = pos2(centre[0] / pixels_per_point, centre[1] / pixels_per_point);
                    let metric = frame.screen_to_metric(screen).map(|v| v as f32);
                    let expected = overlay.shape.at(metric);
                    let expected = if invert { 1.0 - expected } else { expected };
                    let got = shader_twin(&words, centre);
                    assert!(
                        (got - expected).abs() < 1e-3,
                        "{shape:?} {pixel:?}: {got} vs {expected}"
                    );
                }
            }
        }
    }
}
