// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Background workers. Control flow is documented in docs/ENGINE.md.
//!
//! * Preview worker: one thread, keeps the current photo decoded, and for
//!   each burst of requests renders only the newest one per kind
//!   (latest-wins), so dragging a slider never queues stale renders.
//! * Thumbnail workers: a small pool decoding and rendering small previews.
//! * Export worker: one thread running export batches in order.
//!
//! Every job runs under [`isolate_panics`]: a decoder or pipeline bug hit
//! by an unusual file becomes an [`Event::Error`] instead of silently
//! killing the worker thread for the rest of the session.

use crate::{
    Decoder, ExportOptions, Histogram, RawDecoder, export_photo_with, output_path, unused_path,
};
use photonation_develop::{OklabLch, Settings, WhiteBalance};
use photonation_image::{CameraInfo, Pixels, SensorImage};
use photonation_pipeline::{CameraModel, RenderOptions};
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

const THUMBNAIL_LONG_EDGE: usize = 320;
const THUMBNAIL_WORKERS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreviewKind {
    /// The photo with its current settings.
    Edited,
    /// The photo with default settings, for before/after comparison.
    Before,
}

#[derive(Clone, Debug)]
pub struct PreviewRequest {
    pub path: PathBuf,
    pub settings: Settings,
    pub long_edge: Option<usize>,
    /// Increases with every edit; lets the UI ignore out-of-date results.
    pub generation: u64,
    pub kind: PreviewKind,
    /// Show this point colour's selection instead of the edit (see
    /// [`RenderOptions::point_color_overlay`]).
    pub point_color_overlay: Option<usize>,
}

/// An 8-bit sRGB render ready for display.
#[derive(Clone, Debug)]
pub struct RenderedImage {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
    pub histogram: Histogram,
}

#[derive(Clone, Debug)]
pub struct PhotoInfo {
    pub path: PathBuf,
    pub camera: CameraInfo,
    /// As-shot white balance (kelvin, tint).
    pub as_shot: (f64, f64),
    /// Full-resolution size of the default crop, display orientation.
    pub size: (usize, usize),
}

#[derive(Debug)]
pub enum Event {
    PhotoLoaded(PhotoInfo),
    Preview {
        path: PathBuf,
        generation: u64,
        kind: PreviewKind,
        image: RenderedImage,
    },
    Thumbnail {
        path: PathBuf,
        image: RenderedImage,
    },
    WhiteBalancePicked {
        path: PathBuf,
        temperature: f64,
        tint: f64,
    },
    /// The colour under a colour picker, as point colours see it.
    ColorPicked {
        path: PathBuf,
        color: OklabLch,
    },
    ExportProgress {
        done: usize,
        total: usize,
    },
    /// A batch finished. Files are written under unused names
    /// ([`crate::unused_path`]), so nothing in the directory is replaced.
    ExportFinished {
        exported: usize,
        failures: Vec<(PathBuf, String)>,
    },
    Error {
        path: Option<PathBuf>,
        message: String,
    },
}

enum PreviewJob {
    Render(PreviewRequest),
    Pick(PickJob),
}

/// What a picker measures at a point of the photo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickKind {
    WhiteBalance,
    Color,
}

struct PickJob {
    path: PathBuf,
    settings: Settings,
    /// Normalized display coordinates of the rendered image.
    point: [f64; 2],
    kind: PickKind,
}

struct ThumbnailJob {
    path: PathBuf,
    settings: Settings,
}

struct ExportJob {
    photos: Vec<(PathBuf, Option<Settings>)>,
    directory: PathBuf,
    options: ExportOptions,
}

/// Sends events to the front end and wakes it up.
#[derive(Clone)]
struct EventSink {
    sender: Sender<Event>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl EventSink {
    fn send(&self, event: Event) {
        // A closed channel means the UI is gone; there is nobody to tell.
        if self.sender.send(event).is_ok() {
            (self.notify)();
        }
    }
}

pub struct RenderService {
    previews: Sender<PreviewJob>,
    thumbnails: Sender<ThumbnailJob>,
    exports: Sender<ExportJob>,
    events: Receiver<Event>,
}

impl RenderService {
    /// `notify` is called (from worker threads) whenever an event is ready.
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> RenderService {
        RenderService::with_decoder(Arc::new(RawDecoder), notify)
    }

    /// A service that reads photos with `decoder` instead of the raw decoder.
    pub fn with_decoder(
        decoder: Arc<dyn Decoder>,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> RenderService {
        let (event_sender, events) = channel();
        let sink = EventSink {
            sender: event_sender,
            notify: Arc::new(notify),
        };

        let (previews, preview_jobs) = channel();
        let (preview_sink, preview_decoder) = (sink.clone(), decoder.clone());
        std::thread::Builder::new()
            .name("preview".into())
            .spawn(move || preview_worker(preview_jobs, &*preview_decoder, preview_sink))
            .expect("spawning the preview thread");

        let (thumbnails, thumbnail_jobs) = channel::<ThumbnailJob>();
        let thumbnail_jobs = Arc::new(Mutex::new(thumbnail_jobs));
        for index in 0..THUMBNAIL_WORKERS {
            let (jobs, thumbnail_sink) = (thumbnail_jobs.clone(), sink.clone());
            let thumbnail_decoder = decoder.clone();
            std::thread::Builder::new()
                .name(format!("thumbnail-{index}"))
                .spawn(move || thumbnail_worker(jobs, &*thumbnail_decoder, thumbnail_sink))
                .expect("spawning a thumbnail thread");
        }

        let (exports, export_jobs) = channel();
        std::thread::Builder::new()
            .name("export".into())
            .spawn(move || export_worker(export_jobs, &*decoder, sink))
            .expect("spawning the export thread");

        RenderService {
            previews,
            thumbnails,
            exports,
            events,
        }
    }

    pub fn request_preview(&self, request: PreviewRequest) {
        self.previews.send(PreviewJob::Render(request)).ok();
    }

    pub fn request_thumbnail(&self, path: PathBuf, settings: Settings) {
        self.thumbnails.send(ThumbnailJob { path, settings }).ok();
    }

    /// White balance eyedropper: answers with [`Event::WhiteBalancePicked`].
    pub fn pick_white_balance(&self, path: PathBuf, settings: Settings, point: [f64; 2]) {
        self.pick(path, settings, point, PickKind::WhiteBalance);
    }

    /// Colour picker: answers with [`Event::ColorPicked`].
    pub fn pick_color(&self, path: PathBuf, settings: Settings, point: [f64; 2]) {
        self.pick(path, settings, point, PickKind::Color);
    }

    fn pick(&self, path: PathBuf, settings: Settings, point: [f64; 2], kind: PickKind) {
        self.previews
            .send(PreviewJob::Pick(PickJob {
                path,
                settings,
                point,
                kind,
            }))
            .ok();
    }

    /// Exports photos (with explicit settings, or `None` to use each sidecar).
    pub fn export(
        &self,
        photos: Vec<(PathBuf, Option<Settings>)>,
        directory: PathBuf,
        options: ExportOptions,
    ) {
        self.exports
            .send(ExportJob {
                photos,
                directory,
                options,
            })
            .ok();
    }

    /// Events that arrived since the last call.
    pub fn poll(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.try_iter()
    }
}

/// Runs one job, turning a panic into an error message so the worker
/// survives and keeps serving later jobs. Requires `panic = "unwind"`
/// (the default; don't switch the release profile to abort).
fn isolate_panics<T>(job: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(job)).unwrap_or_else(|payload| Err(panic_message(&*payload)))
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    let detail = payload
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown cause".into());
    format!("internal error ({detail}); please report this file")
}

fn to_rendered(image: photonation_image::OutputImage) -> RenderedImage {
    let rgb = match image.pixels {
        Pixels::U8(rgb) => rgb,
        Pixels::U16(samples) => samples.iter().map(|v| (v >> 8) as u8).collect(),
    };
    let histogram = Histogram::from_rgb8(&rgb);
    RenderedImage {
        width: image.width,
        height: image.height,
        rgb,
        histogram,
    }
}

/// Renders 8-bit sRGB; `options` sets size, sharpening and overlays.
fn render_rgb8(
    sensor: &SensorImage,
    settings: &Settings,
    options: &RenderOptions,
) -> Result<RenderedImage, String> {
    photonation_pipeline::render(sensor, settings, options)
        .map(to_rendered)
        .map_err(|e| e.to_string())
}

/// The decoded photo the preview worker is working on.
struct CurrentPhoto {
    path: PathBuf,
    sensor: Arc<SensorImage>,
}

/// The decoded photo at `path`: the cached one, or decoded now (which
/// announces it with [`Event::PhotoLoaded`]).
fn load_photo(
    current: &mut Option<CurrentPhoto>,
    path: &Path,
    decoder: &dyn Decoder,
    sink: &EventSink,
) -> Result<Arc<SensorImage>, String> {
    if let Some(photo) = current.as_ref().filter(|p| p.path == path) {
        return Ok(photo.sensor.clone());
    }
    *current = None; // free the previous mosaic before decoding the next
    let sensor = decoder.decode(path).map_err(|e| e.to_string())?;
    let camera = CameraModel::new(&sensor, &WhiteBalance::AsShot);
    let size = sensor
        .orientation
        .display_size(sensor.crop.w, sensor.crop.h);
    sink.send(Event::PhotoLoaded(PhotoInfo {
        path: path.into(),
        camera: sensor.info.clone(),
        as_shot: (camera.temperature, camera.tint),
        size,
    }));
    let sensor = Arc::new(sensor);
    *current = Some(CurrentPhoto {
        path: path.into(),
        sensor: sensor.clone(),
    });
    Ok(sensor)
}

fn preview_worker(jobs: Receiver<PreviewJob>, decoder: &dyn Decoder, sink: EventSink) {
    let mut current: Option<CurrentPhoto> = None;
    while let Ok(first) = jobs.recv() {
        // Collapse the burst: newest render per kind, every picker click.
        let mut latest_edited = None;
        let mut latest_before = None;
        let mut picks = Vec::new();
        for job in std::iter::once(first).chain(jobs.try_iter()) {
            match job {
                PreviewJob::Render(r) if r.kind == crate::PreviewKind::Edited => {
                    latest_edited = Some(r)
                }
                PreviewJob::Render(r) => latest_before = Some(r),
                PreviewJob::Pick(pick) => picks.push(pick),
            }
        }
        for pick in picks {
            let path = pick.path.clone();
            let picked = isolate_panics(|| {
                let sensor = load_photo(&mut current, &pick.path, decoder, &sink)?;
                Ok(measure(&sensor, pick))
            });
            match picked {
                Ok(Some(event)) => sink.send(event),
                Ok(None) => {} // nothing measurable under the pointer
                Err(message) => sink.send(Event::Error {
                    path: Some(path),
                    message,
                }),
            }
        }
        for request in [latest_edited, latest_before].into_iter().flatten() {
            let rendered = isolate_panics(|| {
                let sensor = load_photo(&mut current, &request.path, decoder, &sink)?;
                let options = RenderOptions {
                    long_edge: request.long_edge,
                    point_color_overlay: request.point_color_overlay,
                    ..RenderOptions::default()
                };
                render_rgb8(&sensor, &request.settings, &options)
            });
            match rendered {
                Ok(image) => sink.send(Event::Preview {
                    path: request.path,
                    generation: request.generation,
                    kind: request.kind,
                    image,
                }),
                Err(message) => sink.send(Event::Error {
                    path: Some(request.path),
                    message,
                }),
            }
        }
    }
}

/// Answers one picker click; `None` when there is nothing to measure.
fn measure(sensor: &SensorImage, pick: PickJob) -> Option<Event> {
    match pick.kind {
        PickKind::WhiteBalance => {
            let (temperature, tint) =
                photonation_pipeline::white_balance_at(sensor, &pick.settings, pick.point)?;
            Some(Event::WhiteBalancePicked {
                path: pick.path,
                temperature,
                tint,
            })
        }
        PickKind::Color => {
            let color = photonation_pipeline::color_at(sensor, &pick.settings, pick.point)?;
            Some(Event::ColorPicked {
                path: pick.path,
                color,
            })
        }
    }
}

fn thumbnail_worker(
    jobs: Arc<Mutex<Receiver<ThumbnailJob>>>,
    decoder: &dyn Decoder,
    sink: EventSink,
) {
    loop {
        // Hold the lock only while taking a job.
        let job = match jobs.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => return,
        };
        let Ok(job) = job else { return };
        let rendered = isolate_panics(|| {
            let sensor = decoder.decode(&job.path).map_err(|e| e.to_string())?;
            let options = RenderOptions {
                long_edge: Some(THUMBNAIL_LONG_EDGE),
                sharpen: false,
                ..RenderOptions::default()
            };
            render_rgb8(&sensor, &job.settings, &options)
        });
        match rendered {
            Ok(image) => sink.send(Event::Thumbnail {
                path: job.path,
                image,
            }),
            Err(message) => sink.send(Event::Error {
                path: Some(job.path),
                message,
            }),
        }
    }
}

fn export_worker(jobs: Receiver<ExportJob>, decoder: &dyn Decoder, sink: EventSink) {
    for job in jobs {
        let total = job.photos.len();
        let mut failures = Vec::new();
        for (done, (path, settings)) in job.photos.iter().enumerate() {
            sink.send(Event::ExportProgress { done, total });
            let named = output_path(path, &job.directory, &job.options, done + 1, total);
            let target = unused_path(&named);
            let exported = isolate_panics(|| {
                export_photo_with(
                    decoder,
                    path,
                    &target,
                    settings.as_ref(),
                    &job.options,
                    |_, _| {},
                )
                .map(drop)
                .map_err(|e| e.to_string())
            });
            if let Err(message) = exported {
                failures.push((path.clone(), message));
            }
        }
        sink.send(Event::ExportFinished {
            exported: total - failures.len(),
            failures,
        });
    }
}

#[cfg(test)]
mod tests;
