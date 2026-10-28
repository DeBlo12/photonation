// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Removal fills: choosing an inpainter per removal, computing fills in
//! order, and caching them so renders only blend. Control flow is in
//! docs/RETOUCH.md.

use photonation_develop::{RemovalMethod, Settings};
use photonation_image::SensorImage;
use photonation_pipeline::{RemovalFills, RemovalJob, removal_canvas, removal_jobs};
use photonation_retouch::{ContentAwareFill, Inpainter};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// Environment variable naming the inpainting model file, overriding the
/// default location ([`default_model_path`]).
pub const MODEL_PATH_VARIABLE: &str = "PHOTONATION_INPAINT_MODEL";
/// Photos whose fills stay cached; older ones are dropped first.
const CACHED_PHOTOS: usize = 8;

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
        }
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

    /// Loads the model if that has not been tried yet (seconds for a large
    /// model), and returns the resulting status.
    pub fn ensure_model(&self) -> ModelStatus {
        let mut generative = lock(&self.generative);
        if matches!(*generative, Generative::NotLoaded) {
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
            let Some(canvas) = removal_canvas(sensor, &settings, job.index, &fills) else {
                continue; // entirely off the photo: nothing to fill
            };
            let filled = self
                .inpainter(removal.method)
                .inpaint(&canvas.canvas, removal.seed)
                .map_err(|error| error.to_string())?;
            let patch = Arc::new(canvas.patch(&filled));
            lock(&self.cache).photo(path).insert(job.key, patch.clone());
            fills.insert(job.key, patch);
            on_filled();
        }
        Ok(fills)
    }
}

/// A poisoned lock only means another job panicked mid-update; the
/// caches stay usable (worst case a fill is computed again).
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(feature = "ai")]
fn load_model(path: Option<&Path>) -> Result<Arc<dyn Inpainter>, ModelStatus> {
    let Some(path) = path.filter(|path| path.is_file()) else {
        return Err(ModelStatus::NotInstalled {
            expected: path.map(Path::to_path_buf),
        });
    };
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
    fn the_cache_forgets_the_oldest_photos() {
        let mut cache = FillCache::default();
        for index in 0..CACHED_PHOTOS + 2 {
            cache.photo(Path::new(&format!("{index}.cr3")));
        }
        assert_eq!(cache.photos.len(), CACHED_PHOTOS);
        assert!(!cache.photos.contains_key(Path::new("0.cr3")));
    }
}
