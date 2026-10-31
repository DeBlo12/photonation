// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Thumbnails for culling a folder quickly. A quick pass per photo reads
//! its capture time and shows a cached render or, failing that, the
//! preview the camera embedded in the file; the photo is then rendered
//! with its settings in the background and the render kept in a disk
//! cache, so the next visit shows it at once. Large survey images take
//! the embedded preview of unedited photos and render edited ones.
//! Documented in docs/ENGINE.md.

use crate::service::{EventSink, isolate_panics, render_rgb8};
use crate::{Decoder, Event, Histogram, RenderedImage, Retoucher};
use photonation_develop::Settings;
use photonation_pipeline::RenderOptions;
use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

pub(crate) const THUMBNAIL_LONG_EDGE: usize = 320;
pub(crate) const THUMBNAIL_WORKERS: usize = 2;
/// The disk cache is trimmed to [`CACHE_TRIMMED_BYTES`] when it grows past this.
const CACHE_MAX_BYTES: u64 = 512 * 1024 * 1024;
const CACHE_TRIMMED_BYTES: u64 = 384 * 1024 * 1024;
/// Bump when cached thumbnails must be redone (the rendering itself is
/// covered by the crate version, which is part of every key).
const CACHE_FORMAT: u32 = 1;
const CACHE_JPEG_QUALITY: u8 = 88;
/// Larger cache files are not thumbnails Photonation wrote.
const MAX_CACHE_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// What a thumbnail worker does next.
#[derive(Debug, PartialEq)]
pub(crate) enum ThumbnailJob {
    /// Capture time, then a cached render or the embedded preview; a
    /// render follows unless the cache had it.
    Quick { path: PathBuf, settings: Settings },
    /// Render with the newest settings asked for the photo.
    Render { path: PathBuf, settings: Settings },
    /// A large image for the survey view.
    Survey {
        path: PathBuf,
        settings: Settings,
        long_edge: usize,
    },
}

#[derive(Default)]
struct Pending {
    survey: VecDeque<(PathBuf, Settings, usize)>,
    quick: VecDeque<PathBuf>,
    /// Each photo at most once; the settings are looked up when it starts.
    render: VecDeque<PathBuf>,
    /// The newest settings asked for each photo with work queued or running.
    settings: HashMap<PathBuf, Settings>,
    closed: bool,
}

/// The thumbnail workers' jobs: survey images first (someone is looking
/// at them), then quick passes (a folder just opened), then renders.
#[derive(Default)]
pub(crate) struct ThumbnailQueue {
    pending: Mutex<Pending>,
    ready: Condvar,
}

impl ThumbnailQueue {
    fn with_pending(&self, change: impl FnOnce(&mut Pending)) {
        if let Ok(mut pending) = self.pending.lock() {
            change(&mut pending);
        }
        self.ready.notify_all();
    }

    /// A quick pass, then a render, for a photo in a folder just opened.
    pub fn request(&self, path: PathBuf, settings: Settings) {
        self.with_pending(|pending| {
            pending.settings.insert(path.clone(), settings);
            pending.quick.push_back(path);
        });
    }

    /// Render the photo again with new settings, before other renders.
    pub fn refresh(&self, path: PathBuf, settings: Settings) {
        self.with_pending(|pending| {
            pending.settings.insert(path.clone(), settings);
            pending.render.retain(|queued| *queued != path);
            pending.render.push_front(path);
        });
    }

    pub fn survey(&self, path: PathBuf, settings: Settings, long_edge: usize) {
        self.with_pending(|pending| pending.survey.push_back((path, settings, long_edge)));
    }

    /// Drop the quick passes and renders not started yet (another folder
    /// was opened).
    pub fn cancel(&self) {
        self.with_pending(|pending| {
            pending.quick.clear();
            pending.render.clear();
            pending.settings.clear();
        });
    }

    /// Wake the workers and let them end.
    pub fn close(&self) {
        self.with_pending(|pending| pending.closed = true);
    }

    /// Queue a render for a photo whose quick pass found no cached one.
    fn queue_render(&self, path: &Path) {
        self.with_pending(|pending| {
            if pending.settings.contains_key(path) && !pending.render.iter().any(|p| p == path) {
                pending.render.push_back(path.into());
            }
        });
    }

    /// Whether `settings` are still the newest asked for the photo; a
    /// render for older ones is out of date.
    fn is_latest(&self, path: &Path, settings: &Settings) -> bool {
        self.pending
            .lock()
            .is_ok_and(|pending| pending.settings.get(path) == Some(settings))
    }

    /// The photo needs no more work unless asked again.
    fn finished(&self, path: &Path, settings: &Settings) {
        self.with_pending(|pending| {
            let queued = pending
                .quick
                .iter()
                .chain(&pending.render)
                .any(|p| p == path);
            if !queued && pending.settings.get(path) == Some(settings) {
                pending.settings.remove(path);
            }
        });
    }

    /// Blocks for the next job; `None` once the queue is closed.
    pub fn next(&self) -> Option<ThumbnailJob> {
        let mut pending = self.pending.lock().ok()?;
        loop {
            if pending.closed {
                return None;
            }
            if let Some((path, settings, long_edge)) = pending.survey.pop_front() {
                return Some(ThumbnailJob::Survey {
                    path,
                    settings,
                    long_edge,
                });
            }
            while let Some(path) = pending.quick.pop_front() {
                if let Some(settings) = pending.settings.get(&path).cloned() {
                    return Some(ThumbnailJob::Quick { path, settings });
                }
            }
            while let Some(path) = pending.render.pop_front() {
                if let Some(settings) = pending.settings.get(&path).cloned() {
                    return Some(ThumbnailJob::Render { path, settings });
                }
            }
            pending = self.ready.wait(pending).ok()?;
        }
    }
}

/// Rendered thumbnails as JPEG files in the data directory, named by a
/// hash of the photo's path, size, modification time and settings, so an
/// edit or a changed file never shows a stale one. Least recently used
/// files go first once it outgrows [`CACHE_MAX_BYTES`].
#[derive(Clone, Debug, Default)]
pub(crate) struct ThumbnailCache {
    directory: Option<PathBuf>,
}

impl ThumbnailCache {
    pub fn in_directory(directory: Option<PathBuf>) -> ThumbnailCache {
        ThumbnailCache { directory }
    }

    /// The cache file for `path` rendered with `settings`, if the photo
    /// can be read and there is a cache.
    fn entry(&self, path: &Path, settings: &Settings) -> Option<PathBuf> {
        let directory = self.directory.as_ref()?;
        let metadata = std::fs::metadata(path).ok()?;
        let mut hasher = DefaultHasher::new();
        CACHE_FORMAT.hash(&mut hasher);
        env!("CARGO_PKG_VERSION").hash(&mut hasher);
        THUMBNAIL_LONG_EDGE.hash(&mut hasher);
        path.hash(&mut hasher);
        metadata.len().hash(&mut hasher);
        metadata.modified().ok().hash(&mut hasher);
        // Debug output names every field and value; the crate version
        // above covers changes to its shape.
        format!("{settings:?}").hash(&mut hasher);
        Some(directory.join(format!("{:016x}.jpg", hasher.finish())))
    }

    fn load(&self, path: &Path, settings: &Settings) -> Option<RenderedImage> {
        let entry = self.entry(path, settings)?;
        let file = std::fs::File::open(&entry).ok()?;
        let mut bytes = Vec::new();
        file.take(MAX_CACHE_FILE_BYTES)
            .read_to_end(&mut bytes)
            .ok()?;
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
            .ok()?
            .to_rgb8();
        // Mark it used, so trimming keeps it.
        if let Ok(file) = std::fs::File::options().append(true).open(&entry) {
            file.set_modified(std::time::SystemTime::now()).ok();
        }
        Some(rendered(
            image.width() as usize,
            image.height() as usize,
            image.into_raw(),
        ))
    }

    /// Keeps a render; failures only cost a re-render next time.
    fn store(&self, path: &Path, settings: &Settings, image: &RenderedImage) {
        let Some(entry) = self.entry(path, settings) else {
            return;
        };
        let (Ok(width), Ok(height)) = (u16::try_from(image.width), u16::try_from(image.height))
        else {
            return;
        };
        let mut jpeg = Vec::new();
        let encoder = jpeg_encoder::Encoder::new(&mut jpeg, CACHE_JPEG_QUALITY);
        if encoder
            .encode(&image.rgb, width, height, jpeg_encoder::ColorType::Rgb)
            .is_err()
        {
            return;
        }
        let Some(directory) = entry.parent() else {
            return;
        };
        if std::fs::create_dir_all(directory).is_err() {
            return;
        }
        // Written beside it and renamed, so a reader never sees half a file.
        let partial = entry.with_extension(format!("{:?}.partial", std::thread::current().id()));
        if std::fs::write(&partial, &jpeg).is_ok() && std::fs::rename(&partial, &entry).is_err() {
            std::fs::remove_file(&partial).ok();
        }
    }

    /// Deletes the least recently used files while the cache is larger
    /// than `max_bytes`, down to `trimmed_bytes`.
    pub fn trim(&self, max_bytes: u64, trimmed_bytes: u64) {
        let Some(directory) = &self.directory else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let metadata = entry.metadata().ok()?;
                metadata.is_file().then(|| {
                    let used = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
                    (used, metadata.len(), entry.path())
                })
            })
            .collect();
        let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
        if total <= max_bytes {
            return;
        }
        files.sort();
        for (_, size, path) in files {
            if total <= trimmed_bytes {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total -= size;
            }
        }
    }

    /// [`ThumbnailCache::trim`] with the cache's own limits.
    pub fn trim_to_limit(&self) {
        self.trim(CACHE_MAX_BYTES, CACHE_TRIMMED_BYTES);
    }
}

fn rendered(width: usize, height: usize, rgb: Vec<u8>) -> RenderedImage {
    let histogram = Histogram::from_rgb8(&rgb);
    RenderedImage {
        width,
        height,
        rgb,
        histogram,
    }
}

pub(crate) fn thumbnail_worker(
    queue: &ThumbnailQueue,
    cache: &ThumbnailCache,
    decoder: &dyn Decoder,
    retoucher: &Retoucher,
    sink: EventSink,
) {
    while let Some(job) = queue.next() {
        match job {
            ThumbnailJob::Quick { path, settings } => {
                let captured = isolate_panics(|| Ok(decoder.capture_time(&path)));
                if let Ok(Some(captured)) = captured {
                    sink.send(Event::CaptureTime {
                        path: path.clone(),
                        captured,
                    });
                }
                if let Some(image) = cache.load(&path, &settings) {
                    sink.send(Event::Thumbnail {
                        path: path.clone(),
                        image,
                        rendered: true,
                    });
                    queue.finished(&path, &settings);
                    continue;
                }
                let embedded =
                    isolate_panics(|| Ok(decoder.embedded_preview(&path, THUMBNAIL_LONG_EDGE)));
                if let Ok(Some(image)) = embedded {
                    sink.send(Event::Thumbnail {
                        path: path.clone(),
                        image,
                        rendered: false,
                    });
                }
                queue.queue_render(&path);
            }
            ThumbnailJob::Render { path, settings } => {
                let options = RenderOptions {
                    long_edge: Some(THUMBNAIL_LONG_EDGE),
                    sharpen: false,
                    ..RenderOptions::default()
                };
                match render(decoder, retoucher, &path, &settings, &options) {
                    Ok(image) => {
                        cache.store(&path, &settings, &image);
                        // A newer refresh is queued; this one is out of date.
                        if queue.is_latest(&path, &settings) {
                            sink.send(Event::Thumbnail {
                                path: path.clone(),
                                image,
                                rendered: true,
                            });
                        }
                    }
                    Err(message) => sink.send(Event::Error {
                        path: Some(path.clone()),
                        message,
                    }),
                }
                queue.finished(&path, &settings);
            }
            ThumbnailJob::Survey {
                path,
                settings,
                long_edge,
            } => {
                let embedded = if settings == Settings::default() {
                    isolate_panics(|| Ok(decoder.embedded_preview(&path, long_edge)))
                        .ok()
                        .flatten()
                } else {
                    None
                };
                let image = match embedded {
                    Some(image) => Ok(image),
                    None => {
                        let options = RenderOptions {
                            long_edge: Some(long_edge),
                            ..RenderOptions::default()
                        };
                        render(decoder, retoucher, &path, &settings, &options)
                    }
                };
                match image {
                    Ok(image) => sink.send(Event::SurveyImage { path, image }),
                    Err(message) => sink.send(Event::Error {
                        path: Some(path),
                        message,
                    }),
                }
            }
        }
    }
}

fn render(
    decoder: &dyn Decoder,
    retoucher: &Retoucher,
    path: &Path,
    settings: &Settings,
    options: &RenderOptions,
) -> Result<RenderedImage, String> {
    isolate_panics(|| {
        let sensor = decoder.decode(path).map_err(|e| e.to_string())?;
        let fills = retoucher.fill_missing(path, &sensor, settings, || {})?;
        render_rgb8(&sensor, settings, &fills, options)
    })
}

/// An embedded preview as the image events carry.
pub(crate) fn embedded_image(preview: photonation_raw::EmbeddedPreview) -> RenderedImage {
    rendered(preview.width, preview.height, preview.rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edited(exposure: f32) -> Settings {
        Settings {
            exposure,
            ..Settings::default()
        }
    }

    #[test]
    fn survey_images_go_first_then_quick_passes_then_renders() {
        let queue = ThumbnailQueue::default();
        queue.request("a".into(), Settings::default());
        queue.refresh("b".into(), edited(1.0));
        queue.survey("c".into(), Settings::default(), 1600);
        assert!(matches!(queue.next(), Some(ThumbnailJob::Survey { .. })));
        assert!(
            matches!(queue.next(), Some(ThumbnailJob::Quick { path, .. }) if path == Path::new("a"))
        );
        assert!(
            matches!(queue.next(), Some(ThumbnailJob::Render { path, .. }) if path == Path::new("b"))
        );
    }

    #[test]
    fn renders_use_the_newest_settings_and_run_once() {
        let queue = ThumbnailQueue::default();
        queue.request("a".into(), Settings::default());
        let Some(ThumbnailJob::Quick { path, .. }) = queue.next() else {
            panic!("quick pass first");
        };
        queue.queue_render(&path);
        queue.refresh("a".into(), edited(1.0));
        queue.queue_render(&path);
        assert_eq!(
            queue.next(),
            Some(ThumbnailJob::Render {
                path: "a".into(),
                settings: edited(1.0)
            })
        );
        assert!(!queue.is_latest(Path::new("a"), &Settings::default()));
        assert!(queue.is_latest(Path::new("a"), &edited(1.0)));
        queue.finished(Path::new("a"), &edited(1.0));
        queue.close();
        assert_eq!(queue.next(), None, "nothing left but the close");
    }

    #[test]
    fn cancelling_drops_what_has_not_started() {
        let queue = ThumbnailQueue::default();
        queue.request("a".into(), Settings::default());
        queue.refresh("b".into(), edited(1.0));
        queue.cancel();
        queue.survey("c".into(), Settings::default(), 1600);
        assert!(matches!(queue.next(), Some(ThumbnailJob::Survey { .. })));
        queue.close();
        assert_eq!(queue.next(), None);
    }

    #[test]
    fn the_cache_keeps_renders_per_settings_and_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let photo = dir.path().join("a.cr3");
        std::fs::write(&photo, b"raw").expect("write photo");
        let cache = ThumbnailCache::in_directory(Some(dir.path().join("cache")));
        let image = rendered(4, 2, (0..24).map(|i| (i * 10) as u8).collect());
        assert!(cache.load(&photo, &Settings::default()).is_none());
        cache.store(&photo, &Settings::default(), &image);
        let loaded = cache.load(&photo, &Settings::default()).expect("cached");
        assert_eq!((loaded.width, loaded.height), (4, 2));
        assert!(cache.load(&photo, &edited(1.0)).is_none(), "other settings");
        std::fs::write(&photo, b"another raw").expect("rewrite photo");
        assert!(
            cache.load(&photo, &Settings::default()).is_none(),
            "changed file"
        );
    }

    #[test]
    fn trimming_removes_the_least_recently_used_first() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = ThumbnailCache::in_directory(Some(dir.path().into()));
        let day = std::time::Duration::from_secs(86_400);
        let now = std::time::SystemTime::now();
        for (name, age) in [("old", 3), ("middle", 2), ("new", 1)] {
            let path = dir.path().join(name);
            std::fs::write(&path, [0u8; 100]).expect("write");
            let file = std::fs::File::options()
                .append(true)
                .open(&path)
                .expect("open");
            file.set_modified(now - day * age).expect("age");
        }
        cache.trim(250, 150);
        let left: Vec<String> = std::fs::read_dir(dir.path())
            .expect("list")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into())
            .collect();
        assert_eq!(left, ["new"]);
    }
}
