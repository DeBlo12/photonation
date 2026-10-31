// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The develop pipeline on the GPU, as wgpu compute passes.
//!
//! ```text
//! mosaic (u16 texture, uploaded once per photo)
//!   ─▶ develop.wgsl: normalize + WB + demosaic + develop (with masks) + output gamut
//!   ─▶ local_contrast.wgsl: √luma, box blurs, detail added back
//!                                                  (Texture or Clarity ≠ 0)
//!   ─▶ resize.wgsl: Lanczos-3 rows, columns        (when the size changes)
//!   ─▶ finish.wgsl: encode ─▶ sharpen rows, columns (when sharpening)
//!                  ─▶ pack into RGB8, display orientation ─▶ read back
//! ```
//!
//! Every parameter and table comes from [`photonation_pipeline::render_plan`],
//! so the GPU renders with exactly the CPU's settings; the shaders transcribe
//! the per-pixel maths. The CPU pipeline is the reference: tests compare
//! the two. Renders a plan can't describe yet (straightened crops,
//! removals) and 16-bit output return
//! [`GpuError::Unsupported`]; callers render those on the CPU.

mod params;

use params::Words;
use photonation_develop::Settings;
use photonation_image::{BitDepth, OutputImage, Pixels, SensorImage, SensorLayout};
use photonation_pipeline::plan::LocalContrastPlan;
use photonation_pipeline::{PlanError, RemovalFills, RenderOptions, RenderPlan, render_plan};
use std::sync::{Arc, Weak};

const DEVELOP_SHADER: &str = include_str!("develop.wgsl");
const RESIZE_SHADER: &str = include_str!("resize.wgsl");
const FINISH_SHADER: &str = include_str!("finish.wgsl");
const LOCAL_CONTRAST_SHADER: &str = include_str!("local_contrast.wgsl");

/// Side of the 2D passes' square workgroups.
const TILE: u32 = 16;
/// Invocations per workgroup of the pack pass.
const PACK_GROUP: u32 = 256;
/// Invocations per workgroup of the box blur passes.
const BOX_GROUP: u32 = 256;
/// Largest box radius the blur passes' shared memory holds (Clarity on a
/// 24 000-pixel long edge); keep in step with `local_contrast.wgsl`.
const MAX_BOX_RADIUS: usize = 192;

#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("no graphics adapter for GPU rendering")]
    NoAdapter,
    #[error("{0} are not supported by the GPU renderer yet")]
    Unsupported(&'static str),
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error("GPU error: {0}")]
    Device(String),
}

impl GpuError {
    /// The render itself is fine; this renderer just can't do it (yet).
    pub fn is_unsupported(&self) -> bool {
        matches!(
            self,
            GpuError::Unsupported(_) | GpuError::Plan(PlanError::Unsupported(_))
        )
    }
}

struct Pipelines {
    develop: wgpu::ComputePipeline,
    resize_horizontal: wgpu::ComputePipeline,
    resize_vertical: wgpu::ComputePipeline,
    encode: wgpu::ComputePipeline,
    sharpen_rows: wgpu::ComputePipeline,
    sharpen_columns: wgpu::ComputePipeline,
    pack: wgpu::ComputePipeline,
    lightness: wgpu::ComputePipeline,
    box_rows: wgpu::ComputePipeline,
    box_columns: wgpu::ComputePipeline,
    combine: wgpu::ComputePipeline,
}

impl Pipelines {
    fn new(device: &wgpu::Device) -> Pipelines {
        let module = |label, source: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };
        let develop = module("develop", DEVELOP_SHADER);
        let resize = module("resize", RESIZE_SHADER);
        let finish = module("finish", FINISH_SHADER);
        let local_contrast = module("local contrast", LOCAL_CONTRAST_SHADER);
        let pipeline = |module: &wgpu::ShaderModule, entry_point| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry_point),
                layout: None,
                module,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        Pipelines {
            develop: pipeline(&develop, "main"),
            resize_horizontal: pipeline(&resize, "horizontal"),
            resize_vertical: pipeline(&resize, "vertical"),
            encode: pipeline(&finish, "encode"),
            sharpen_rows: pipeline(&finish, "sharpen_rows"),
            sharpen_columns: pipeline(&finish, "sharpen_columns"),
            pack: pipeline(&finish, "pack"),
            lightness: pipeline(&local_contrast, "lightness"),
            box_rows: pipeline(&local_contrast, "box_rows"),
            box_columns: pipeline(&local_contrast, "box_columns"),
            combine: pipeline(&local_contrast, "combine"),
        }
    }
}

/// The current photo's mosaic on the GPU.
struct UploadedSensor {
    /// Identifies the photo; a `Weak` keeps its allocation, so no other
    /// photo can turn up at the same address while this one is cached.
    sensor: Weak<SensorImage>,
    view: wgpu::TextureView,
}

/// Renders on one GPU device. Keeps the last photo's mosaic uploaded, so
/// re-rendering it with new settings uploads only parameters.
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: String,
    pipelines: Pipelines,
    uploaded: Option<UploadedSensor>,
}

impl GpuRenderer {
    /// A renderer on the fastest hardware adapter. Software rasterizers
    /// are refused: they are slower than the CPU pipeline.
    pub fn new() -> Result<GpuRenderer, GpuError> {
        GpuRenderer::create(false)
    }

    fn create(allow_software: bool) -> Result<GpuRenderer, GpuError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..wgpu::RequestAdapterOptions::default()
        }))
        .map_err(|_| GpuError::NoAdapter)?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu && !allow_software {
            return Err(GpuError::NoAdapter);
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("photonation render"),
            // Full-resolution renders need large textures.
            required_limits: adapter.limits(),
            ..wgpu::DeviceDescriptor::default()
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;
        // Errors inside a render are caught by its error scopes; anything
        // else is logged rather than taking the process down.
        device.on_uncaptured_error(Arc::new(|error| eprintln!("GPU renderer: {error}")));
        let pipelines = Pipelines::new(&device);
        Ok(GpuRenderer {
            device,
            queue,
            adapter: format!("{} ({:?})", info.name, info.backend),
            pipelines,
            uploaded: None,
        })
    }

    /// The adapter's name and backend, for logs.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// Renders like [`photonation_pipeline::render_with_fills`] (8-bit only).
    pub fn render(
        &mut self,
        sensor: &Arc<SensorImage>,
        settings: &Settings,
        fills: &RemovalFills,
        opts: &RenderOptions,
    ) -> Result<OutputImage, GpuError> {
        if opts.bit_depth != BitDepth::Eight {
            return Err(GpuError::Unsupported("16-bit renders"));
        }
        let plan = render_plan(sensor, settings, fills, opts)?;
        if !params::pattern_fits(&sensor.layout) {
            return Err(GpuError::Unsupported("colour filter patterns this large"));
        }
        let max_side = self.device.limits().max_texture_dimension_2d as usize;
        let largest = [plan.source_size, plan.target, (sensor.width, sensor.height)]
            .iter()
            .map(|&(width, height)| width.max(height))
            .max()
            .unwrap_or(0);
        if largest > max_side {
            return Err(GpuError::Unsupported("images this large"));
        }
        if let Some(contrast) = &plan.local_contrast
            && contrast.clarity_radius.max(contrast.texture_radius) > MAX_BOX_RADIUS
        {
            return Err(GpuError::Unsupported(
                "Texture and Clarity on images this large",
            ));
        }

        let out_of_memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mosaic = self.mosaic(sensor);
        let readback = self.run(&plan, &sensor.layout, &mosaic);
        let errors = [validation.pop(), out_of_memory.pop()].map(pollster::block_on);
        if let Some(error) = errors.into_iter().flatten().next() {
            // A failed upload must not be reused.
            self.uploaded = None;
            return Err(GpuError::Device(error.to_string()));
        }
        let (width, height) = plan.orientation.display_size(plan.target.0, plan.target.1);
        let mut rgb = self.read(&readback)?;
        rgb.truncate(width * height * 3);
        Ok(OutputImage {
            width,
            height,
            color_space: opts.color_space,
            pixels: Pixels::U8(rgb),
        })
    }

    /// The mosaic texture of `sensor`, uploaded unless it is the cached one.
    fn mosaic(&mut self, sensor: &Arc<SensorImage>) -> wgpu::TextureView {
        if let Some(uploaded) = &self.uploaded
            && Weak::ptr_eq(&uploaded.sensor, &Arc::downgrade(sensor))
        {
            return uploaded.view.clone();
        }
        self.uploaded = None; // free the previous mosaic first
        let (format, texel_bytes, bytes) = match sensor.layout {
            SensorLayout::Cfa(_) => (
                wgpu::TextureFormat::R16Uint,
                2,
                sensor
                    .data
                    .iter()
                    .flat_map(|sample| sample.to_le_bytes())
                    .collect::<Vec<u8>>(),
            ),
            // There is no three-channel format; pad to four.
            SensorLayout::LinearRgb => (
                wgpu::TextureFormat::Rgba16Uint,
                8,
                sensor
                    .data
                    .chunks_exact(3)
                    .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 0])
                    .flat_map(|sample| sample.to_le_bytes())
                    .collect::<Vec<u8>>(),
            ),
        };
        let size = extent(sensor.width, sensor.height);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mosaic"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((sensor.width * texel_bytes) as u32),
                rows_per_image: Some(sensor.height as u32),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.uploaded = Some(UploadedSensor {
            sensor: Arc::downgrade(sensor),
            view: view.clone(),
        });
        view
    }

    /// Records and submits every pass of `plan`; returns the buffer the
    /// packed RGB8 output lands in.
    fn run(
        &self,
        plan: &RenderPlan,
        layout: &SensorLayout,
        mosaic: &wgpu::TextureView,
    ) -> wgpu::Buffer {
        let device = &self.device;
        let pipelines = &self.pipelines;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render"),
        });

        let developed = self.image("developed", plan.source_size, RGBA);
        let tone_tables: Vec<f32> = plan.develop.tone.tables.concat();
        let (masks, shapes) = params::masks(&plan.develop);
        let develop_group = self.bind_group(
            &pipelines.develop,
            &[
                (0, Bind::View(mosaic.clone())),
                (1, self.uniform("sensor", &params::sensor(plan, layout))),
                (2, self.uniform("develop", &params::develop(&plan.develop))),
                (3, self.storage("tone", &params::floats(&tone_tables))),
                (
                    4,
                    self.storage("point colours", &params::point_colors(&plan.develop)),
                ),
                (5, view(&developed)),
                (6, self.storage("masks", &masks)),
                (7, self.storage("mask shapes", &shapes)),
            ],
        );
        dispatch(
            &mut encoder,
            &pipelines.develop,
            &develop_group,
            tiles(plan.source_size),
        );
        let developed = match &plan.local_contrast {
            None => developed,
            Some(contrast) => {
                self.local_contrast(&mut encoder, contrast, plan.source_size, &developed)
            }
        };

        let linear = match &plan.resize {
            None => developed,
            Some(resize) => {
                let rows_size = (plan.target.0, plan.source_size.1);
                let rows = self.image("resized rows", rows_size, RGBA);
                let resized = self.image("resized", plan.target, RGBA);
                for (pipeline, axis, input, output, size) in [
                    (
                        &pipelines.resize_horizontal,
                        &resize.horizontal,
                        &developed,
                        &rows,
                        rows_size,
                    ),
                    (
                        &pipelines.resize_vertical,
                        &resize.vertical,
                        &rows,
                        &resized,
                        plan.target,
                    ),
                ] {
                    let group = self.bind_group(
                        pipeline,
                        &[
                            (0, view(input)),
                            (1, view(output)),
                            (2, self.storage("taps", &params::taps(&axis.taps))),
                            (3, self.storage("weights", &params::floats(&axis.weights))),
                        ],
                    );
                    dispatch(&mut encoder, pipeline, &group, tiles(size));
                }
                resized
            }
        };

        let finish = self.uniform("finish", &params::finish(plan));
        let encoded = self.image("encoded", plan.target, RGBA);
        let encode_group = self.bind_group(
            &pipelines.encode,
            &[
                (1, view(&linear)),
                (2, self.storage("transfer", &params::floats(&plan.encode))),
                (3, view(&encoded)),
            ],
        );
        dispatch(
            &mut encoder,
            &pipelines.encode,
            &encode_group,
            tiles(plan.target),
        );

        let finished = match &plan.sharpen {
            None => encoded,
            Some(sharpen) => {
                let kernel = self.storage("kernel", &params::floats(&sharpen.kernel));
                let blurred_rows = self.image("blurred rows", plan.target, LUMA);
                let sharpened = self.image("sharpened", plan.target, RGBA);
                let rows_group = self.bind_group(
                    &pipelines.sharpen_rows,
                    &[
                        (0, finish.clone()),
                        (4, view(&encoded)),
                        (5, kernel.clone()),
                        (6, view(&blurred_rows)),
                    ],
                );
                dispatch(
                    &mut encoder,
                    &pipelines.sharpen_rows,
                    &rows_group,
                    tiles(plan.target),
                );
                let columns_group = self.bind_group(
                    &pipelines.sharpen_columns,
                    &[
                        (0, finish.clone()),
                        (4, view(&encoded)),
                        (5, kernel),
                        (7, view(&blurred_rows)),
                        (8, view(&sharpened)),
                    ],
                );
                dispatch(
                    &mut encoder,
                    &pipelines.sharpen_columns,
                    &columns_group,
                    tiles(plan.target),
                );
                sharpened
            }
        };

        let bytes = plan.target.0 * plan.target.1 * 3;
        let words = bytes.div_ceil(4) as u32;
        let buffer_size = u64::from(words.max(1)) * 4;
        let packed = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("packed"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let pack_group = self.bind_group(
            &pipelines.pack,
            &[
                (0, finish),
                (9, view(&finished)),
                (10, Bind::Buffer(packed.clone())),
            ],
        );
        // One dimension holds at most 65 535 workgroups; spill into a second.
        let groups = words.div_ceil(PACK_GROUP).max(1);
        let groups_x = groups.min(u32::from(u16::MAX));
        dispatch(
            &mut encoder,
            &pipelines.pack,
            &pack_group,
            (groups_x, groups.div_ceil(groups_x)),
        );

        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&packed, 0, &readback, 0, buffer_size);
        self.queue.submit([encoder.finish()]);
        readback
    }

    /// Records Texture and Clarity on `developed` (`size` pixels); returns
    /// the result.
    fn local_contrast(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        contrast: &LocalContrastPlan,
        size: (usize, usize),
        developed: &wgpu::Texture,
    ) -> wgpu::Texture {
        let pipelines = &self.pipelines;
        let amounts = self.uniform("local contrast", &params::local_contrast(contrast));
        let lightness = self.image("lightness", size, LUMA);
        let lightness_group = self.bind_group(
            &pipelines.lightness,
            &[
                (0, amounts.clone()),
                (2, view(developed)),
                (4, view(&lightness)),
            ],
        );
        dispatch(encoder, &pipelines.lightness, &lightness_group, tiles(size));

        // Box passes along rows, then columns, ping-ponging between two planes.
        let (width, height) = (size.0 as u32, size.1 as u32);
        let blur = |encoder: &mut wgpu::CommandEncoder, radius: usize| {
            let radius = self.uniform("box radius", &params::box_radius(radius));
            let planes = [
                self.image("blur", size, LUMA),
                self.image("blur", size, LUMA),
            ];
            let rows = (width.div_ceil(BOX_GROUP), height);
            let columns = (width, height.div_ceil(BOX_GROUP));
            let passes =
                std::iter::repeat_n((&pipelines.box_rows, rows), contrast.box_passes).chain(
                    std::iter::repeat_n((&pipelines.box_columns, columns), contrast.box_passes),
                );
            let mut input = &lightness;
            for (index, (pipeline, groups)) in passes.enumerate() {
                let output = &planes[index % 2];
                let group = self.bind_group(
                    pipeline,
                    &[(1, radius.clone()), (3, view(input)), (4, view(output))],
                );
                dispatch(encoder, pipeline, &group, groups);
                input = output;
            }
            input.clone()
        };
        let clarity_base =
            (contrast.clarity != 0.0).then(|| blur(encoder, contrast.clarity_radius));
        let texture_base =
            (contrast.texture != 0.0).then(|| blur(encoder, contrast.texture_radius));

        let contrasted = self.image("local contrast", size, RGBA);
        let combine_group = self.bind_group(
            &pipelines.combine,
            &[
                (0, amounts),
                (2, view(developed)),
                (5, view(&lightness)),
                // An unused base is never read; any plane will do.
                (6, view(clarity_base.as_ref().unwrap_or(&lightness))),
                (7, view(texture_base.as_ref().unwrap_or(&lightness))),
                (8, view(&contrasted)),
            ],
        );
        dispatch(encoder, &pipelines.combine, &combine_group, tiles(size));
        contrasted
    }

    /// Waits for the render and copies its output to the CPU.
    fn read(&self, readback: &wgpu::Buffer) -> Result<Vec<u8>, GpuError> {
        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).ok();
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Device(e.to_string()))?;
        receiver
            .recv()
            .map_err(|e| GpuError::Device(e.to_string()))?
            .map_err(|e| GpuError::Device(e.to_string()))?;
        let bytes = slice
            .get_mapped_range()
            .map_err(|e| GpuError::Device(e.to_string()))?
            .to_vec();
        readback.unmap();
        Ok(bytes)
    }

    /// An intermediate image, written by one pass and read by the next.
    fn image(
        &self,
        label: &str,
        (width, height): (usize, usize),
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    }

    fn uniform(&self, label: &str, words: &Words) -> Bind {
        self.buffer(label, words, wgpu::BufferUsages::UNIFORM)
    }

    fn storage(&self, label: &str, words: &Words) -> Bind {
        self.buffer(label, words, wgpu::BufferUsages::STORAGE)
    }

    fn buffer(&self, label: &str, words: &Words, usage: wgpu::BufferUsages) -> Bind {
        let bytes = words.bytes();
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.len().max(4) as u64,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, &bytes);
        Bind::Buffer(buffer)
    }

    fn bind_group(
        &self,
        pipeline: &wgpu::ComputePipeline,
        entries: &[(u32, Bind)],
    ) -> wgpu::BindGroup {
        let entries: Vec<wgpu::BindGroupEntry> = entries
            .iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: match resource {
                    Bind::Buffer(buffer) => buffer.as_entire_binding(),
                    Bind::View(view) => wgpu::BindingResource::TextureView(view),
                },
            })
            .collect();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        })
    }
}

const RGBA: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const LUMA: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

fn extent(width: usize, height: usize) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: width as u32,
        height: height as u32,
        depth_or_array_layers: 1,
    }
}

/// A resource for one binding of a bind group.
#[derive(Clone)]
enum Bind {
    Buffer(wgpu::Buffer),
    View(wgpu::TextureView),
}

fn view(texture: &wgpu::Texture) -> Bind {
    Bind::View(texture.create_view(&wgpu::TextureViewDescriptor::default()))
}

/// Workgroups covering an image with `TILE`-sided tiles.
fn tiles((width, height): (usize, usize)) -> (u32, u32) {
    (
        (width as u32).div_ceil(TILE),
        (height as u32).div_ceil(TILE),
    )
}

fn dispatch(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    group: &wgpu::BindGroup,
    (x, y): (u32, u32),
) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.dispatch_workgroups(x, y, 1);
}

#[cfg(test)]
mod tests;
