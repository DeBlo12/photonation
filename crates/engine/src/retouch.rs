// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Removal fills: choosing an inpainter per removal, computing fills in
//! order, and caching them so renders only blend. Control flow is in
//! docs/RETOUCH.md.

use crate::disk_cache::{mark_used, trim_directory, write_whole};
use photonation_develop::{RemovalMethod, Settings};
use photonation_image::{Rect, SensorImage};
use photonation_pipeline::{RemovalFills, RemovalJob, RetouchPatch, removal_canvas, removal_jobs};
use photonation_retouch::{ContentAwareFill, Inpainter};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// Environment variable naming the inpainting model file, overriding the
/// default location ([`default_model_path`]).
pub const MODEL_PATH_VARIABLE: &str = "PHOTONATION_INPAINT_MODEL";
/// Photos whose fills stay cached; older ones are dropped first.
const CACHED_PHOTOS: usize = 8;
/// The disk cache is trimmed to [`DISK_CACHE_TRIMMED_BYTES`] when it grows past
/// this: about a hundred large fills, or thousands of small ones.
const DISK_CACHE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DISK_CACHE_TRIMMED_BYTES: u64 = 1536 * 1024 * 1024;
/// Bump when cached fills must be redone or the file changes. How fills are
/// computed is covered by the crate version, which is part of every key.
const DISK_CACHE_FORMAT: u32 = 1;
const DISK_CACHE_MAGIC: &[u8; 4] = b"PNFL";
/// The header: magic, then rectangle, width and height as `u32`s.
const DISK_CACHE_HEADER_BYTES: usize = 4 + 6 * 4;
/// Larger files are not fills Photonation wrote: a canvas is at most
/// `MAX_CANVAS_EDGE` square, three `f32`s a pixel.
const MAX_DISK_FILL_BYTES: u64 = 64 * 1024 * 1024;

/// Where the "Generative AI" removal method stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelStatus {
    /// Not loaded yet; loading starts with the first generative removal.
    NotLoaded,
    /// This build has no neural inpainting (the `ai` feature is off).
    NotBuilt,
    /// No model file at the expected place.
    NotInstalled {
        expected: Option<PathBuf>,
    },
    Ready {
        name: String,
    },
    Failed {
        message: String,
    },
}

impl ModelStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, ModelStatus::Ready { .. })
    }
}

/// `$PHOTONATION_INPAINT_MODEL`, or `models/inpaint.onnx` in the
/// [`data_directory`].
pub fn default_model_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(MODEL_PATH_VARIABLE) {
        return Some(PathBuf::from(path));
    }
    data_directory().map(|directory| directory.join("models").join("inpaint.onnx"))
}

/// The platform's place for Photonation's own files (models, preferences):
/// `~/Library/Application Support/Photonation` on macOS,
/// `$XDG_DATA_HOME/photonation` or `~/.local/share/photonation` on Linux,
/// `%APPDATA%\Photonation` on Windows.
pub fn data_directory() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.map(|home| home.join("Library/Application Support/Photonation"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("Photonation"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|home| home.join(".local/share")))
            .map(|data| data.join("photonation"))
    }
}

/// The neural inpainter, loaded on first use.
enum Generative {
    NotLoaded,
    Ready(Arc<dyn Inpainter>),
    Unavailable(ModelStatus),
}

/// Fills removals and remembers the results. Shared by all workers.
pub struct Retoucher {
    content_aware: ContentAwareFill,
    model_path: Option<PathBuf>,
    generative: Mutex<Generative>,
    /// Per photo, fills by removal chain fingerprint; `order` is oldest
    /// first.
    cache: Mutex<FillCache>,
    /// Fills from earlier sessions, in this folder.
    disk: Option<PathBuf>,
}

#[derive(Default)]
struct FillCache {
    photos: HashMap<PathBuf, RemovalFills>,
    order: Vec<PathBuf>,
}

impl FillCache {
    fn photo(&mut self, path: &Path) -> &mut RemovalFills {
        if let Some(position) = self.order.iter().position(|cached| cached == path) {
            let recent = self.order.remove(position);
            self.order.push(recent);
        } else {
            self.order.push(path.to_path_buf());
            if self.order.len() > CACHED_PHOTOS {
                let oldest = self.order.remove(0);
                self.photos.remove(&oldest);
            }
        }
        self.photos.entry(path.to_path_buf()).or_default()
    }
}

impl Retoucher {
    /// A retoucher whose generative method loads the model at `model_path`
    /// (when built with the `ai` feature).
    pub fn new(model_path: Option<PathBuf>) -> Retoucher {
        Retoucher {
            content_aware: ContentAwareFill::default(),
            model_path,
            generative: Mutex::new(Generative::NotLoaded),
            cache: Mutex::new(FillCache::default()),
            disk: None,
        }
    }

    /// The retoucher this machine works with: its installed model, and fills
    /// kept in `fills` under the data directory.
    pub fn installed() -> Retoucher {
        Retoucher::new(default_model_path())
            .with_disk_cache(data_directory().map(|data| data.join("fills")))
    }

    /// Also keeps fills in `directory`, so reopening a photo doesn't compute
    /// them again.
    pub fn with_disk_cache(mut self, directory: Option<PathBuf>) -> Retoucher {
        self.disk = directory;
        self
    }

    /// A retoucher with an inpainter already at hand for the generative
    /// method (tests, or a front end that loads models itself).
    pub fn with_generative(inpainter: Arc<dyn Inpainter>) -> Retoucher {
        let retoucher = Retoucher::new(None);
        *lock(&retoucher.generative) = Generative::Ready(inpainter);
        retoucher
    }

    pub fn model_status(&self) -> ModelStatus {
        match &*lock(&self.generative) {
            Generative::NotLoaded => ModelStatus::NotLoaded,
            Generative::Ready(inpainter) => ModelStatus::Ready {
                name: inpainter.name().to_string(),
            },
            Generative::Unavailable(status) => status.clone(),
        }
    }

    /// Loads the model if that has not been tried yet, or if no model was
    /// installed then (seconds for a large model), and returns the
    /// resulting status. A model put in place while the app runs is found
    /// the next time.
    pub fn ensure_model(&self) -> ModelStatus {
        let mut generative = lock(&self.generative);
        if matches!(
            *generative,
            Generative::NotLoaded | Generative::Unavailable(ModelStatus::NotInstalled { .. })
        ) {
            *generative = match load_model(self.model_path.as_deref()) {
                Ok(inpainter) => {
                    // Fills made while the model was missing used the
                    // fallback; redo them with the model.
                    *lock(&self.cache) = FillCache::default();
                    Generative::Ready(inpainter)
                }
                Err(status) => Generative::Unavailable(status),
            };
        }
        drop(generative);
        self.model_status()
    }

    /// The inpainter for `method`: generative falls back to content-aware
    /// while no model is ready.
    fn inpainter(&self, method: RemovalMethod) -> Arc<dyn Inpainter> {
        if method == RemovalMethod::Generative
            && let Generative::Ready(inpainter) = &*lock(&self.generative)
        {
            return inpainter.clone();
        }
        Arc::new(self.content_aware)
    }

    /// The cached fills for `settings` of the photo at `path`.
    pub fn cached_fills(&self, path: &Path, settings: &Settings) -> RemovalFills {
        let mut cache = lock(&self.cache);
        let photo = cache.photo(path);
        removal_jobs(settings)
            .into_iter()
            .filter_map(|job| Some((job.key, photo.get(&job.key)?.clone())))
            .collect()
    }

    /// Active removals of `settings` without a cached fill.
    pub fn missing(&self, path: &Path, settings: &Settings) -> Vec<RemovalJob> {
        let mut cache = lock(&self.cache);
        let photo = cache.photo(path);
        removal_jobs(settings)
            .into_iter()
            .filter(|job| !photo.contains_key(&job.key))
            .collect()
    }

    /// Fills every active removal of `settings` that has no cached fill,
    /// in order (each canvas includes the fills before it), calling
    /// `on_filled` after each. Returns all fills for `settings`.
    pub fn fill_missing(
        &self,
        path: &Path,
        sensor: &SensorImage,
        settings: &Settings,
        mut on_filled: impl FnMut(),
    ) -> Result<RemovalFills, String> {
        let settings = settings.sanitized();
        if self.missing(path, &settings).is_empty() {
            return Ok(self.cached_fills(path, &settings));
        }
        if settings
            .removals
            .iter()
            .any(|removal| removal.is_active() && removal.method == RemovalMethod::Generative)
        {
            self.ensure_model();
        }
        let mut fills = self.cached_fills(path, &settings);
        for job in removal_jobs(&settings) {
            if fills.contains_key(&job.key) {
                continue;
            }
            let removal = &settings.removals[job.index];
            let entry = self.disk_entry(path, &settings, &job);
            let patch = match entry.as_deref().and_then(load_fill) {
                Some(patch) => patch,
                None => {
                    let Some(canvas) = removal_canvas(sensor, &settings, job.index, &fills) else {
                        continue; // entirely off the photo: nothing to fill
                    };
                    let filled = self
                        .inpainter(removal.method)
                        .inpaint(&canvas.canvas, removal.seed)
                        .map_err(|error| error.to_string())?;
                    let patch = canvas.patch(&filled);
                    if let Some(entry) = &entry {
                        store_fill(entry, &patch);
                    }
                    patch
                }
            };
            let patch = Arc::new(patch);
            lock(&self.cache).photo(path).insert(job.key, patch.clone());
            fills.insert(job.key, patch);
            on_filled();
        }
        Ok(fills)
    }

    /// The file holding `job`'s fill, if the photo can be read and there is a
    /// cache. The name comes from a hash of all a fill depends on: the photo's
    /// path, size and modification time, the chain of removals, the crate
    /// version, and the model if the chain uses it, since a generative
    /// removal made without one used the fallback.
    fn disk_entry(&self, path: &Path, settings: &Settings, job: &RemovalJob) -> Option<PathBuf> {
        let directory = self.disk.as_ref()?;
        let metadata = std::fs::metadata(path).ok()?;
        let mut hasher = DefaultHasher::new();
        DISK_CACHE_FORMAT.hash(&mut hasher);
        env!("CARGO_PKG_VERSION").hash(&mut hasher);
        path.hash(&mut hasher);
        metadata.len().hash(&mut hasher);
        metadata.modified().ok().hash(&mut hasher);
        job.key.hash(&mut hasher);
        let uses_model = settings.removals[..=job.index]
            .iter()
            .any(|removal| removal.is_active() && removal.method == RemovalMethod::Generative);
        if uses_model {
            self.model_identity().hash(&mut hasher);
        }
        Some(directory.join(format!("{:016x}.fill", hasher.finish())))
    }

    /// What tells one model from another: its name, and its file's size and
    /// modification time. `None` while there's no model and the fallback fills.
    fn model_identity(&self) -> Option<(String, Option<u64>, Option<std::time::SystemTime>)> {
        let Generative::Ready(inpainter) = &*lock(&self.generative) else {
            return None;
        };
        let metadata = self
            .model_path
            .as_deref()
            .and_then(installed_model)
            .and_then(|model| std::fs::metadata(model).ok());
        Some((
            inpainter.name().to_string(),
            metadata.as_ref().map(std::fs::Metadata::len),
            metadata.and_then(|metadata| metadata.modified().ok()),
        ))
    }
}

/// A fill from the disk cache. Anything unexpected in the file just means
/// computing it again.
fn load_fill(entry: &Path) -> Option<RetouchPatch> {
    let mut bytes = Vec::new();
    std::fs::File::open(entry)
        .ok()?
        .take(MAX_DISK_FILL_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    let patch = parse_fill(&bytes)?;
    mark_used(entry);
    Some(patch)
}

fn parse_fill(bytes: &[u8]) -> Option<RetouchPatch> {
    if bytes.len() as u64 > MAX_DISK_FILL_BYTES {
        return None;
    }
    let (header, samples) = bytes.split_at_checked(DISK_CACHE_HEADER_BYTES)?;
    let (magic, fields) = header.split_at(DISK_CACHE_MAGIC.len());
    if magic != DISK_CACHE_MAGIC {
        return None;
    }
    let mut fields = fields
        .chunks_exact(4)
        .filter_map(|field| Some(u32::from_le_bytes(field.try_into().ok()?) as usize));
    let mut field = || fields.next();
    let sensor_rect = Rect::new(field()?, field()?, field()?, field()?);
    let (width, height) = (field()?, field()?);
    if width == 0 || height == 0 || sensor_rect.w == 0 || sensor_rect.h == 0 {
        return None;
    }
    let expected = width
        .checked_mul(height)?
        .checked_mul(3 * size_of::<f32>())?;
    if samples.len() != expected {
        return None;
    }
    let rgb: Vec<f32> = samples
        .chunks_exact(size_of::<f32>())
        .filter_map(|sample| Some(f32::from_le_bytes(sample.try_into().ok()?)))
        .collect();
    rgb.iter()
        .all(|sample| sample.is_finite())
        .then_some(RetouchPatch {
            sensor_rect,
            width,
            height,
            rgb,
        })
}

/// Keeps a fill; failures only cost computing it again next time.
fn store_fill(entry: &Path, patch: &RetouchPatch) {
    let rect = patch.sensor_rect;
    let mut bytes =
        Vec::with_capacity(DISK_CACHE_HEADER_BYTES + patch.rgb.len() * size_of::<f32>());
    bytes.extend_from_slice(DISK_CACHE_MAGIC);
    for field in [rect.x, rect.y, rect.w, rect.h, patch.width, patch.height] {
        bytes.extend_from_slice(&(field as u32).to_le_bytes());
    }
    for sample in &patch.rgb {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    write_whole(entry, &bytes);
    if let Some(directory) = entry.parent() {
        trim_directory(directory, DISK_CACHE_MAX_BYTES, DISK_CACHE_TRIMMED_BYTES);
    }
}

/// A poisoned lock only means another job panicked mid-update; the
/// caches stay usable (worst case a fill is computed again).
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The model file for `expected`: that file, or else the only ONNX file
/// in its folder, so a download can keep its own name (`lama_fp32.onnx`).
#[cfg_attr(not(feature = "ai"), allow(dead_code))]
fn installed_model(expected: &Path) -> Option<PathBuf> {
    if expected.is_file() {
        return Some(expected.to_path_buf());
    }
    let mut models = std::fs::read_dir(expected.parent()?)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("onnx"))
        });
    let model = models.next()?;
    models.next().is_none().then_some(model)
}

#[cfg(feature = "ai")]
fn load_model(path: Option<&Path>) -> Result<Arc<dyn Inpainter>, ModelStatus> {
    let Some(path) = path.and_then(installed_model) else {
        return Err(ModelStatus::NotInstalled {
            expected: path.map(Path::to_path_buf),
        });
    };
    let path = path.as_path();
    // The network sets the structure at its low resolution; content-aware
    // fill then adds the photo's own texture at the canvas's resolution.
    photonation_retouch::OnnxInpainter::load(path)
        .map(|inpainter| {
            Arc::new(photonation_retouch::Refined {
                inner: inpainter,
                detail: ContentAwareFill::default(),
            }) as Arc<dyn Inpainter>
        })
        .map_err(|error| ModelStatus::Failed {
            message: error.to_string(),
        })
}

#[cfg(not(feature = "ai"))]
fn load_model(_: Option<&Path>) -> Result<Arc<dyn Inpainter>, ModelStatus> {
    Err(ModelStatus::NotBuilt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonation_develop::{BrushStroke, Removal};
    use photonation_image::Orientation;
    use photonation_retouch::{Canvas, InpaintError};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Paints the hole flat grey and counts its calls.
    struct CountingInpainter(AtomicUsize);

    impl Inpainter for CountingInpainter {
        fn name(&self) -> &str {
            "Counting"
        }

        fn inpaint(&self, canvas: &Canvas, _: u32) -> Result<Vec<f32>, InpaintError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(vec![0.5; canvas.rgb.len()])
        }
    }

    fn settings_with(methods: &[RemovalMethod]) -> Settings {
        Settings {
            removals: methods
                .iter()
                .enumerate()
                .map(|(index, method)| {
                    Removal::new(
                        *method,
                        BrushStroke {
                            points: vec![[0.2 + 0.3 * index as f64, 0.5]],
                            radius: 0.05,
                        },
                    )
                })
                .collect(),
            ..Settings::default()
        }
    }

    #[test]
    fn fills_are_computed_once_and_reused() {
        let counter = Arc::new(CountingInpainter(AtomicUsize::new(0)));
        let retoucher = Retoucher::with_generative(counter.clone());
        let sensor = SensorImage::test_pattern(96, 64, Orientation::Normal);
        let path = Path::new("a.cr3");
        let settings = settings_with(&[RemovalMethod::Generative, RemovalMethod::Generative]);
        assert_eq!(retoucher.missing(path, &settings).len(), 2);
        let mut progress = 0;
        let fills = retoucher
            .fill_missing(path, &sensor, &settings, || progress += 1)
            .expect("fill");
        assert_eq!((fills.len(), progress), (2, 2));
        assert!(retoucher.missing(path, &settings).is_empty());
        retoucher
            .fill_missing(path, &sensor, &settings, || {})
            .expect("fill");
        assert_eq!(
            counter.0.load(Ordering::SeqCst),
            2,
            "cached, not recomputed"
        );
        // Changing a slider keeps the fills.
        let brighter = Settings {
            exposure: 1.0,
            ..settings.clone()
        };
        assert_eq!(retoucher.cached_fills(path, &brighter).len(), 2);
    }

    /// A photo that exists, so it has a size and modification time to key on.
    struct DiskCacheFixture {
        folder: tempfile::TempDir,
        photo: PathBuf,
        counter: Arc<CountingInpainter>,
    }

    impl DiskCacheFixture {
        fn new() -> DiskCacheFixture {
            let folder = tempfile::tempdir().expect("temporary folder");
            let photo = folder.path().join("a.cr3");
            std::fs::write(&photo, b"raw").expect("write photo");
            DiskCacheFixture {
                folder,
                photo,
                counter: Arc::new(CountingInpainter(AtomicUsize::new(0))),
            }
        }

        /// A new session: nothing in memory, the disk as it was.
        fn session(&self) -> Retoucher {
            Retoucher::with_generative(self.counter.clone())
                .with_disk_cache(Some(self.folder.path().join("fills")))
        }

        fn fill(&self, retoucher: &Retoucher, settings: &Settings) -> RemovalFills {
            let sensor = SensorImage::test_pattern(96, 64, Orientation::Normal);
            retoucher
                .fill_missing(&self.photo, &sensor, settings, || {})
                .expect("fill")
        }

        fn computed(&self) -> usize {
            self.counter.0.load(Ordering::SeqCst)
        }

        fn files(&self) -> Vec<PathBuf> {
            std::fs::read_dir(self.folder.path().join("fills"))
                .map(|entries| entries.filter_map(|e| Some(e.ok()?.path())).collect())
                .unwrap_or_default()
        }
    }

    #[test]
    fn fills_come_back_from_disk_in_a_new_session_exactly() {
        let fixture = DiskCacheFixture::new();
        let settings = settings_with(&[RemovalMethod::Generative, RemovalMethod::Generative]);
        let first = fixture.fill(&fixture.session(), &settings);
        assert_eq!(fixture.computed(), 2);
        assert_eq!(fixture.files().len(), 2);

        let later = fixture.session();
        let again = fixture.fill(&later, &settings);
        assert_eq!(fixture.computed(), 2, "nothing computed again");
        assert_eq!(first.len(), again.len());
        for (key, patch) in &first {
            assert_eq!(**patch, *again[key], "the same fill, to the bit");
        }
        assert!(
            later.missing(&fixture.photo, &settings).is_empty(),
            "in memory now"
        );
    }

    #[test]
    fn a_changed_removal_refills_only_from_there_on() {
        let fixture = DiskCacheFixture::new();
        let settings = settings_with(&[RemovalMethod::Generative, RemovalMethod::Generative]);
        fixture.fill(&fixture.session(), &settings);
        let mut changed = settings.clone();
        changed.removals[1].seed = 9;
        fixture.fill(&fixture.session(), &changed);
        assert_eq!(fixture.computed(), 3, "the first came from disk");
        let mut earlier = settings.clone();
        earlier.removals[0].seed = 9;
        fixture.fill(&fixture.session(), &earlier);
        assert_eq!(fixture.computed(), 5, "the second sees the first, so both");
    }

    #[test]
    fn a_changed_photo_doesnt_get_its_old_fills() {
        let fixture = DiskCacheFixture::new();
        let settings = settings_with(&[RemovalMethod::Generative]);
        fixture.fill(&fixture.session(), &settings);
        std::fs::write(&fixture.photo, b"another raw").expect("write photo");
        fixture.fill(&fixture.session(), &settings);
        assert_eq!(fixture.computed(), 2);
    }

    #[test]
    fn a_fill_made_without_the_model_isnt_kept_for_when_there_is_one() {
        let fixture = DiskCacheFixture::new();
        let settings = settings_with(&[RemovalMethod::Generative]);
        let without_model = Retoucher::new(Some(fixture.folder.path().join("none.onnx")))
            .with_disk_cache(Some(fixture.folder.path().join("fills")));
        fixture.fill(&without_model, &settings);
        assert_eq!(fixture.computed(), 0, "the fallback filled it");
        fixture.fill(&fixture.session(), &settings);
        assert_eq!(fixture.computed(), 1, "the model fills it when it's there");
        // A content-aware removal doesn't depend on any model.
        let content_aware = settings_with(&[RemovalMethod::ContentAware]);
        let sessions = [fixture.session(), without_model];
        fixture.fill(&sessions[0], &content_aware);
        let files = fixture.files().len();
        fixture.fill(&sessions[1], &content_aware);
        assert_eq!(fixture.files().len(), files, "the same file either way");
    }

    #[test]
    fn a_damaged_file_is_computed_again_and_replaced() {
        let fixture = DiskCacheFixture::new();
        let settings = settings_with(&[RemovalMethod::Generative]);
        let good = fixture.fill(&fixture.session(), &settings);
        let entry = fixture.files().pop().expect("a file");
        for damage in [
            &b""[..],
            b"PNFL",
            b"not a fill at all, but long enough to read",
        ] {
            std::fs::write(&entry, damage).expect("write");
            let before = fixture.computed();
            let again = fixture.fill(&fixture.session(), &settings);
            assert_eq!(fixture.computed(), before + 1);
            assert_eq!(again.len(), good.len());
        }
        assert!(
            parse_fill(&std::fs::read(&entry).expect("read")).is_some(),
            "replaced"
        );
    }

    #[test]
    fn nonsense_in_a_fill_file_is_refused() {
        let patch = RetouchPatch {
            sensor_rect: Rect::new(1, 2, 30, 40),
            width: 2,
            height: 1,
            rgb: vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6],
        };
        let folder = tempfile::tempdir().expect("temporary folder");
        let entry = folder.path().join("x.fill");
        store_fill(&entry, &patch);
        let bytes = std::fs::read(&entry).expect("read");
        assert_eq!(parse_fill(&bytes), Some(patch));
        assert_eq!(parse_fill(&bytes[..bytes.len() - 1]), None, "cut short");
        let mut wrong_magic = bytes.clone();
        wrong_magic[0] = b'X';
        assert_eq!(parse_fill(&wrong_magic), None);
        let mut no_pixels = bytes.clone();
        no_pixels[20..24].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(parse_fill(&no_pixels), None, "no width");
        let mut huge = bytes.clone();
        huge[20..28].copy_from_slice(&[0xff; 8]);
        assert_eq!(parse_fill(&huge), None, "sizes that overflow");
        let mut not_a_number = bytes;
        not_a_number[28..32].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(parse_fill(&not_a_number), None);
    }

    #[test]
    fn without_a_cache_folder_or_a_readable_photo_nothing_is_kept() {
        let fixture = DiskCacheFixture::new();
        let settings = settings_with(&[RemovalMethod::Generative]);
        let no_cache = Retoucher::with_generative(fixture.counter.clone());
        fixture.fill(&no_cache, &settings);
        assert!(fixture.files().is_empty());
        let sensor = SensorImage::test_pattern(96, 64, Orientation::Normal);
        fixture
            .session()
            .fill_missing(Path::new("/nonexistent/a.cr3"), &sensor, &settings, || {})
            .expect("still fills");
        assert!(fixture.files().is_empty());
    }

    #[test]
    fn generative_falls_back_to_content_aware_without_a_model() {
        let retoucher = Retoucher::new(Some(PathBuf::from("/nonexistent/inpaint.onnx")));
        let sensor = SensorImage::test_pattern(96, 64, Orientation::Normal);
        let settings = settings_with(&[RemovalMethod::Generative]);
        let fills = retoucher
            .fill_missing(Path::new("a.cr3"), &sensor, &settings, || {})
            .expect("fill");
        assert_eq!(fills.len(), 1);
        assert!(matches!(
            retoucher.model_status(),
            ModelStatus::NotInstalled { .. } | ModelStatus::NotBuilt
        ));
    }

    #[test]
    fn a_model_is_looked_for_again_until_one_is_installed() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let expected = folder.path().join("inpaint.onnx");
        let retoucher = Retoucher::new(Some(expected.clone()));
        let first = retoucher.ensure_model();
        if first == ModelStatus::NotBuilt {
            return;
        }
        assert!(matches!(first, ModelStatus::NotInstalled { .. }));
        // A download under its own name is found; this one is not a
        // model, so loading it fails, and that is not tried again.
        std::fs::write(folder.path().join("lama_fp32.onnx"), b"not a model").expect("write");
        assert!(matches!(
            retoucher.ensure_model(),
            ModelStatus::Failed { .. }
        ));
    }

    #[test]
    fn the_installed_model_is_the_expected_file_or_the_only_onnx_file() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let expected = folder.path().join("inpaint.onnx");
        assert_eq!(installed_model(&expected), None);
        std::fs::write(folder.path().join("notes.txt"), b"").expect("write");
        let lama = folder.path().join("lama_fp32.onnx");
        std::fs::write(&lama, b"").expect("write");
        assert_eq!(installed_model(&expected), Some(lama));
        std::fs::write(folder.path().join("other.ONNX"), b"").expect("write");
        assert_eq!(installed_model(&expected), None, "two models: neither");
        std::fs::write(&expected, b"").expect("write");
        assert_eq!(installed_model(&expected), Some(expected));
    }

    #[test]
    fn the_cache_forgets_the_oldest_photos() {
        let mut cache = FillCache::default();
        for index in 0..CACHED_PHOTOS + 2 {
            cache.photo(Path::new(&format!("{index}.cr3")));
        }
        assert_eq!(cache.photos.len(), CACHED_PHOTOS);
        assert!(!cache.photos.contains_key(Path::new("0.cr3")));
    }
}
