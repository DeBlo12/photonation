// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Background workers. Control flow is documented in docs/ENGINE.md.
//!
//! * Preview worker: one thread, keeps the current photo decoded, and for
//!   each burst of requests renders only the newest one per kind
//!   (latest-wins), so dragging a slider never queues stale renders.
//! * Thumbnail workers: a small pool showing embedded previews or cached
//!   renders at once, then rendering small previews (`thumbnails.rs`).
//! * Export worker: one thread running export batches in order.
//! * Retouch worker: one thread filling removals in the background (a
//!   neural fill takes seconds), so previews never wait for them; it loads
//!   the generative model too.
//!
//! Every job runs under [`isolate_panics`]: a decoder or pipeline bug hit
//! by an unusual file becomes an [`Event::Error`] instead of silently
//! killing the worker thread for the rest of the session.

use crate::preview::PreviewRenderer;
use crate::thumbnails::{THUMBNAIL_WORKERS, ThumbnailCache, ThumbnailQueue, thumbnail_worker};
use crate::{
    Decoder, ExportOptions, Histogram, ModelStatus, RawDecoder, Retoucher, data_directory,
    default_model_path, export_photo_with, output_path, unused_path,
};
use photonation_develop::{OklabLch, Settings, WhiteBalance};
use photonation_image::{CameraInfo, Pixels, SensorImage};
use photonation_pipeline::{CameraModel, RemovalFills, RenderOptions};
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

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
    /// A small image of a photo: its embedded preview or a cached render
    /// at first (`rendered` says which), then a render with its settings.
    Thumbnail {
        path: PathBuf,
        image: RenderedImage,
        rendered: bool,
    },
    /// When a photo was taken ("YYYY:MM:DD HH:MM:SS"), read by the
    /// thumbnail's quick pass.
    CaptureTime {
        path: PathBuf,
        captured: String,
    },
    /// A large image of a photo for comparing several side by side.
    SurveyImage {
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
    /// More removal fills are ready for this photo; previews rendered
    /// before lack them, so request another.
    RemovalsFilled {
        path: PathBuf,
    },
    /// The generative removal model's state, after a status request or
    /// when it changes.
    ModelStatus(ModelStatus),
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

enum RetouchJob {
    Fill {
        path: PathBuf,
        sensor: Arc<SensorImage>,
        settings: Box<Settings>,
    },
    /// Load the model if needed and report its status.
    Status,
}

struct ExportJob {
    photos: Vec<(PathBuf, Option<Settings>)>,
    directory: PathBuf,
    options: ExportOptions,
}

/// Sends events to the front end and wakes it up.
#[derive(Clone)]
pub(crate) struct EventSink {
    sender: Sender<Event>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl EventSink {
    pub(crate) fn send(&self, event: Event) {
        // A closed channel means the UI is gone; there is nobody to tell.
        if self.sender.send(event).is_ok() {
            (self.notify)();
        }
    }
}

pub struct RenderService {
    previews: Sender<PreviewJob>,
    thumbnails: Arc<ThumbnailQueue>,
    exports: Sender<ExportJob>,
    retouches: Sender<RetouchJob>,
    events: Receiver<Event>,
}

impl RenderService {
    /// `notify` is called (from worker threads) whenever an event is ready.
    /// Previews render on the GPU where possible (see [`crate::GPU_RENDER_VARIABLE`]).
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> RenderService {
        let retoucher = Arc::new(Retoucher::new(default_model_path()));
        RenderService::start(Arc::new(RawDecoder), retoucher, disk_cache(), true, notify)
    }

    /// Like [`RenderService::new`], but previews render on the CPU and no
    /// GPU device is opened.
    pub fn on_cpu(notify: impl Fn() + Send + Sync + 'static) -> RenderService {
        let retoucher = Arc::new(Retoucher::new(default_model_path()));
        RenderService::start(Arc::new(RawDecoder), retoucher, disk_cache(), false, notify)
    }

    /// A service that reads photos with `decoder` instead of the raw decoder.
    pub fn with_decoder(
        decoder: Arc<dyn Decoder>,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> RenderService {
        let retoucher = Arc::new(Retoucher::new(default_model_path()));
        RenderService::with_parts(decoder, retoucher, notify)
    }

    /// A service with its own decoder and removal filler. Its previews
    /// render on the CPU, the reference, and its thumbnails are not cached
    /// on disk.
    pub fn with_parts(
        decoder: Arc<dyn Decoder>,
        retoucher: Arc<Retoucher>,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> RenderService {
        RenderService::start(decoder, retoucher, ThumbnailCache::default(), false, notify)
    }

    fn start(
        decoder: Arc<dyn Decoder>,
        retoucher: Arc<Retoucher>,
        cache: ThumbnailCache,
        gpu_previews: bool,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> RenderService {
        let (event_sender, events) = channel();
        let sink = EventSink {
            sender: event_sender,
            notify: Arc::new(notify),
        };

        let (retouches, retouch_jobs) = channel();
        let (retouch_sink, retouch_retoucher) = (sink.clone(), retoucher.clone());
        std::thread::Builder::new()
            .name("retouch".into())
            .spawn(move || retouch_worker(retouch_jobs, &retouch_retoucher, retouch_sink))
            .expect("spawning the retouch thread");

        let (previews, preview_jobs) = channel();
        let preview = PreviewContext {
            decoder: decoder.clone(),
            retoucher: retoucher.clone(),
            retouches: retouches.clone(),
            sink: sink.clone(),
        };
        std::thread::Builder::new()
            .name("preview".into())
            .spawn(move || {
                let renderer = if gpu_previews {
                    PreviewRenderer::with_gpu()
                } else {
                    PreviewRenderer::cpu()
                };
                preview_worker(preview_jobs, &preview, renderer)
            })
            .expect("spawning the preview thread");

        let thumbnails = Arc::new(ThumbnailQueue::default());
        for index in 0..THUMBNAIL_WORKERS {
            let (queue, thumbnail_sink, cache) = (thumbnails.clone(), sink.clone(), cache.clone());
            let (thumbnail_decoder, thumbnail_retoucher) = (decoder.clone(), retoucher.clone());
            std::thread::Builder::new()
                .name(format!("thumbnail-{index}"))
                .spawn(move || {
                    thumbnail_worker(
                        &queue,
                        &cache,
                        &*thumbnail_decoder,
                        &thumbnail_retoucher,
                        thumbnail_sink,
                    )
                })
                .expect("spawning a thumbnail thread");
        }
        // Trimming lists the whole cache; keep it off the workers' path.
        std::thread::Builder::new()
            .name("thumbnail-cache".into())
            .spawn(move || cache.trim_to_limit())
            .expect("spawning the thumbnail cache thread");

        let (exports, export_jobs) = channel();
        std::thread::Builder::new()
            .name("export".into())
            .spawn(move || export_worker(export_jobs, &*decoder, &retoucher, sink))
            .expect("spawning the export thread");

        RenderService {
            previews,
            thumbnails,
            exports,
            retouches,
            events,
        }
    }

    pub fn request_preview(&self, request: PreviewRequest) {
        self.previews.send(PreviewJob::Render(request)).ok();
    }

    /// A thumbnail for a photo of a folder just opened: its cached render
    /// or embedded preview first ([`Event::Thumbnail`]), with its capture
    /// time ([`Event::CaptureTime`]), then a render if none was cached.
    pub fn request_thumbnail(&self, path: PathBuf, settings: Settings) {
        self.thumbnails.request(path, settings);
    }

    /// Renders a photo's thumbnail again after an edit, ahead of other
    /// thumbnails.
    pub fn refresh_thumbnail(&self, path: PathBuf, settings: Settings) {
        self.thumbnails.refresh(path, settings);
    }

    /// Forgets thumbnails asked for but not started (another folder opened).
    pub fn cancel_thumbnails(&self) {
        self.thumbnails.cancel();
    }

    /// A `long_edge` image of a photo for the survey view
    /// ([`Event::SurveyImage`]): its embedded preview while it is
    /// unedited, else a render.
    pub fn request_survey_image(&self, path: PathBuf, settings: Settings, long_edge: usize) {
        self.thumbnails.survey(path, settings, long_edge);
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

    /// Loads the generative removal model in the background if needed;
    /// answers with [`Event::ModelStatus`].
    pub fn request_model_status(&self) {
        self.retouches.send(RetouchJob::Status).ok();
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
pub(crate) fn isolate_panics<T>(job: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
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

pub(crate) fn to_rendered(image: photonation_image::OutputImage) -> RenderedImage {
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
pub(crate) fn render_rgb8(
    sensor: &SensorImage,
    settings: &Settings,
    fills: &RemovalFills,
    options: &RenderOptions,
) -> Result<RenderedImage, String> {
    photonation_pipeline::render_with_fills(sensor, settings, fills, options)
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

/// What the preview worker works with.
struct PreviewContext {
    decoder: Arc<dyn Decoder>,
    retoucher: Arc<Retoucher>,
    /// Missing removal fills are requested here.
    retouches: Sender<RetouchJob>,
    sink: EventSink,
}

fn preview_worker(
    jobs: Receiver<PreviewJob>,
    context: &PreviewContext,
    mut renderer: PreviewRenderer,
) {
    let (decoder, sink) = (&*context.decoder, &context.sink);
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
                let sensor = load_photo(&mut current, &pick.path, decoder, sink)?;
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
                let sensor = load_photo(&mut current, &request.path, decoder, sink)?;
                // Render with the fills at hand; missing ones are filled in
                // the background and announced with RemovalsFilled.
                let retoucher = &context.retoucher;
                if !retoucher
                    .missing(&request.path, &request.settings)
                    .is_empty()
                {
                    let fill = RetouchJob::Fill {
                        path: request.path.clone(),
                        sensor: sensor.clone(),
                        settings: Box::new(request.settings.clone()),
                    };
                    context.retouches.send(fill).ok();
                }
                let fills = retoucher.cached_fills(&request.path, &request.settings);
                let options = RenderOptions {
                    long_edge: request.long_edge,
                    point_color_overlay: request.point_color_overlay,
                    ..RenderOptions::default()
                };
                renderer.render(&sensor, &request.settings, &fills, &options)
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

impl Drop for RenderService {
    fn drop(&mut self) {
        // The other workers end when their channels close; the thumbnail
        // queue is shared, so tell its workers.
        self.thumbnails.close();
    }
}

/// The thumbnail cache in the data directory (none without one).
fn disk_cache() -> ThumbnailCache {
    ThumbnailCache::in_directory(data_directory().map(|data| data.join("thumbnails")))
}

fn export_worker(
    jobs: Receiver<ExportJob>,
    decoder: &dyn Decoder,
    retoucher: &Retoucher,
    sink: EventSink,
) {
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
                    retoucher,
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

/// Fills removals in the background: for each burst, only the newest
/// request (older ones describe settings the user has moved on from).
fn retouch_worker(jobs: Receiver<RetouchJob>, retoucher: &Retoucher, sink: EventSink) {
    let mut reported: Option<ModelStatus> = None;
    while let Ok(first) = jobs.recv() {
        let mut latest_fill = None;
        let mut status_requested = false;
        for job in std::iter::once(first).chain(jobs.try_iter()) {
            match job {
                RetouchJob::Fill {
                    path,
                    sensor,
                    settings,
                } => latest_fill = Some((path, sensor, settings)),
                RetouchJob::Status => status_requested = true,
            }
        }
        if status_requested {
            let status = isolate_panics(|| Ok(retoucher.ensure_model()))
                .unwrap_or_else(|message| ModelStatus::Failed { message });
            sink.send(Event::ModelStatus(status.clone()));
            reported = Some(status);
        }
        if let Some((path, sensor, settings)) = latest_fill {
            let filled = isolate_panics(|| {
                retoucher.fill_missing(&path, &sensor, &settings, || {
                    sink.send(Event::RemovalsFilled { path: path.clone() })
                })
            });
            if let Err(message) = filled {
                sink.send(Event::Error {
                    path: Some(path),
                    message,
                });
            }
            let status = retoucher.model_status();
            if reported.as_ref() != Some(&status) {
                sink.send(Event::ModelStatus(status.clone()));
                reported = Some(status);
            }
        }
    }
}

#[cfg(test)]
mod tests;
