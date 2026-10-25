// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Background workers. Control flow is documented in docs/ENGINE.md.
//!
//! * Preview worker: one thread, keeps the current photo decoded, and for
//!   each burst of requests renders only the newest one per kind
//!   (latest-wins), so dragging a slider never queues stale renders.
//! * Thumbnail workers: a small pool decoding and rendering small previews.
//! * Export worker: one thread running export batches in order.

use crate::{ExportOptions, Histogram, export_photo, output_path};
use photonation_develop::{Settings, WhiteBalance};
use photonation_image::{CameraInfo, Pixels, SensorImage};
use photonation_pipeline::{CameraModel, RenderOptions};
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
    ExportProgress {
        done: usize,
        total: usize,
    },
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
    PickWhiteBalance {
        path: PathBuf,
        settings: Settings,
        point: [f64; 2],
    },
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
        let (event_sender, events) = channel();
        let sink = EventSink {
            sender: event_sender,
            notify: Arc::new(notify),
        };

        let (previews, preview_jobs) = channel();
        let preview_sink = sink.clone();
        std::thread::Builder::new()
            .name("preview".into())
            .spawn(move || preview_worker(preview_jobs, preview_sink))
            .expect("spawning the preview thread");

        let (thumbnails, thumbnail_jobs) = channel::<ThumbnailJob>();
        let thumbnail_jobs = Arc::new(Mutex::new(thumbnail_jobs));
        for index in 0..THUMBNAIL_WORKERS {
            let (jobs, thumbnail_sink) = (thumbnail_jobs.clone(), sink.clone());
            std::thread::Builder::new()
                .name(format!("thumbnail-{index}"))
                .spawn(move || thumbnail_worker(jobs, thumbnail_sink))
                .expect("spawning a thumbnail thread");
        }

        let (exports, export_jobs) = channel();
        std::thread::Builder::new()
            .name("export".into())
            .spawn(move || export_worker(export_jobs, sink))
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

    /// Eyedropper: answers with [`Event::WhiteBalancePicked`].
    pub fn pick_white_balance(&self, path: PathBuf, settings: Settings, point: [f64; 2]) {
        self.previews
            .send(PreviewJob::PickWhiteBalance {
                path,
                settings,
                point,
            })
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

fn render_rgb8(
    sensor: &SensorImage,
    settings: &Settings,
    long_edge: Option<usize>,
    sharpen: bool,
) -> Result<RenderedImage, String> {
    let options = RenderOptions {
        long_edge,
        sharpen,
        ..RenderOptions::default()
    };
    photonation_pipeline::render(sensor, settings, &options)
        .map(to_rendered)
        .map_err(|e| e.to_string())
}

/// The decoded photo the preview worker is working on.
struct CurrentPhoto {
    path: PathBuf,
    sensor: Arc<SensorImage>,
}

fn load_photo(
    current: &mut Option<CurrentPhoto>,
    path: &Path,
    sink: &EventSink,
) -> Option<Arc<SensorImage>> {
    if let Some(photo) = current.as_ref().filter(|p| p.path == path) {
        return Some(photo.sensor.clone());
    }
    *current = None; // free the previous mosaic before decoding the next
    match photonation_raw::decode_file(path) {
        Ok(sensor) => {
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
            Some(sensor)
        }
        Err(e) => {
            sink.send(Event::Error {
                path: Some(path.into()),
                message: e.to_string(),
            });
            None
        }
    }
}

fn preview_worker(jobs: Receiver<PreviewJob>, sink: EventSink) {
    let mut current: Option<CurrentPhoto> = None;
    while let Ok(first) = jobs.recv() {
        // Collapse the burst: newest render per kind, every eyedropper pick.
        let mut latest_edited = None;
        let mut latest_before = None;
        let mut picks = Vec::new();
        for job in std::iter::once(first).chain(jobs.try_iter()) {
            match job {
                PreviewJob::Render(r) if r.kind == crate::PreviewKind::Edited => {
                    latest_edited = Some(r)
                }
                PreviewJob::Render(r) => latest_before = Some(r),
                PreviewJob::PickWhiteBalance {
                    path,
                    settings,
                    point,
                } => picks.push((path, settings, point)),
            }
        }
        for (path, settings, point) in picks {
            let Some(sensor) = load_photo(&mut current, &path, &sink) else {
                continue;
            };
            if let Some((temperature, tint)) =
                photonation_pipeline::white_balance_at(&sensor, &settings, point)
            {
                sink.send(Event::WhiteBalancePicked {
                    path,
                    temperature,
                    tint,
                });
            }
        }
        for request in [latest_edited, latest_before].into_iter().flatten() {
            let Some(sensor) = load_photo(&mut current, &request.path, &sink) else {
                continue;
            };
            match render_rgb8(&sensor, &request.settings, request.long_edge, true) {
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

fn thumbnail_worker(jobs: Arc<Mutex<Receiver<ThumbnailJob>>>, sink: EventSink) {
    loop {
        // Hold the lock only while taking a job.
        let job = match jobs.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => return,
        };
        let Ok(job) = job else { return };
        let rendered = photonation_raw::decode_file(&job.path)
            .map_err(|e| e.to_string())
            .and_then(|sensor| {
                render_rgb8(&sensor, &job.settings, Some(THUMBNAIL_LONG_EDGE), false)
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

fn export_worker(jobs: Receiver<ExportJob>, sink: EventSink) {
    for job in jobs {
        let total = job.photos.len();
        let mut failures = Vec::new();
        for (done, (path, settings)) in job.photos.iter().enumerate() {
            sink.send(Event::ExportProgress { done, total });
            let target = output_path(path, &job.directory, job.options.format);
            if let Err(e) = export_photo(path, &target, settings.as_ref(), &job.options, |_, _| {})
            {
                failures.push((path.clone(), e.to_string()));
            }
        }
        sink.send(Event::ExportFinished {
            exported: total - failures.len(),
            failures,
        });
    }
}
