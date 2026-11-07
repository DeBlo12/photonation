// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The neural networks Photonation can use, where they are installed, and
//! loading them on first use. Photonation ships no weights: people download
//! a model themselves and put it in the models folder. Models run on this
//! computer only. See docs/RETOUCH.md.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// Environment variable naming the inpainting model file, overriding the
/// models folder.
pub const MODEL_PATH_VARIABLE: &str = "PHOTONATION_INPAINT_MODEL";
/// Environment variable naming the people model file, overriding the
/// models folder.
pub const PEOPLE_MODEL_PATH_VARIABLE: &str = "PHOTONATION_PEOPLE_MODEL";

/// What a model is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelKind {
    /// Fills removals ("Generative AI"): LaMa or MI-GAN.
    Inpainting,
    /// Finds people to select with a click: an RF-DETR segmentation model.
    People,
}

impl ModelKind {
    pub const ALL: [ModelKind; 2] = [ModelKind::Inpainting, ModelKind::People];

    /// The name the model is looked for under in the models folder.
    pub fn file_name(self) -> &'static str {
        match self {
            ModelKind::Inpainting => "inpaint.onnx",
            ModelKind::People => "people.onnx",
        }
    }

    /// The environment variable that names the file instead.
    pub fn path_variable(self) -> &'static str {
        match self {
            ModelKind::Inpainting => MODEL_PATH_VARIABLE,
            ModelKind::People => PEOPLE_MODEL_PATH_VARIABLE,
        }
    }

    /// The model file's expected place: the environment variable, or
    /// [`ModelKind::file_name`] in `models` of the [`data_directory`].
    pub fn default_path(self) -> Option<PathBuf> {
        if let Some(path) = std::env::var_os(self.path_variable()) {
            return Some(PathBuf::from(path));
        }
        data_directory().map(|directory| directory.join("models").join(self.file_name()))
    }

    /// Whether an ONNX file kept under its download name is this kind's:
    /// RF-DETR exports (`rfdetr-seg-medium.onnx`) find people, and any other
    /// file is taken for an inpainting model, as before people models.
    fn claims(self, file_stem: &str) -> bool {
        let people = file_stem.to_ascii_lowercase().starts_with("rfdetr");
        people == (self == ModelKind::People)
    }
}

/// Where a model stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelStatus {
    /// Not loaded yet; loading starts with its first use.
    NotLoaded,
    /// This build has no neural networks (the `ai` feature is off).
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
    ModelKind::Inpainting.default_path()
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

/// The model file for `expected`: that file, or else the only ONNX file in
/// its folder that `kind` claims, so a download can keep its own name
/// (`lama_fp32.onnx`, `rfdetr-seg-medium.onnx`).
#[cfg_attr(not(feature = "ai"), allow(dead_code))]
pub(crate) fn installed_model(kind: ModelKind, expected: &Path) -> Option<PathBuf> {
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
                && path
                    .file_stem()
                    .is_some_and(|stem| kind.claims(&stem.to_string_lossy()))
        });
    let model = models.next()?;
    models.next().is_none().then_some(model)
}

/// A model loaded on first use and kept, shared by all workers.
pub(crate) struct LazyModel<T: ?Sized> {
    kind: ModelKind,
    path: Option<PathBuf>,
    state: Mutex<Slot<T>>,
}

enum Slot<T: ?Sized> {
    NotLoaded,
    Ready { model: Arc<T>, name: String },
    Unavailable(ModelStatus),
}

impl<T: ?Sized> LazyModel<T> {
    pub(crate) fn new(kind: ModelKind, path: Option<PathBuf>) -> LazyModel<T> {
        LazyModel {
            kind,
            path,
            state: Mutex::new(Slot::NotLoaded),
        }
    }

    /// A model already at hand (tests, or a front end that loads its own).
    pub(crate) fn ready(kind: ModelKind, model: Arc<T>, name: String) -> LazyModel<T> {
        LazyModel {
            kind,
            path: None,
            state: Mutex::new(Slot::Ready { model, name }),
        }
    }

    pub(crate) fn status(&self) -> ModelStatus {
        match &*lock(&self.state) {
            Slot::NotLoaded => ModelStatus::NotLoaded,
            Slot::Ready { name, .. } => ModelStatus::Ready { name: name.clone() },
            Slot::Unavailable(status) => status.clone(),
        }
    }

    /// The model, if loaded.
    pub(crate) fn get(&self) -> Option<Arc<T>> {
        match &*lock(&self.state) {
            Slot::Ready { model, .. } => Some(model.clone()),
            _ => None,
        }
    }

    /// Loads the model with `load` if that has not been tried yet, or if no
    /// model was installed then (seconds for a large one), so a model put in
    /// place while the app runs is found the next time. Returns whether it
    /// was loaded just now.
    pub(crate) fn ensure(
        &self,
        load: impl FnOnce(&Path) -> Result<(Arc<T>, String), String>,
    ) -> bool {
        let mut state = lock(&self.state);
        if !matches!(
            *state,
            Slot::NotLoaded | Slot::Unavailable(ModelStatus::NotInstalled { .. })
        ) {
            return false;
        }
        if !cfg!(feature = "ai") {
            *state = Slot::Unavailable(ModelStatus::NotBuilt);
            return false;
        }
        let Some(file) = self
            .path
            .as_deref()
            .and_then(|path| installed_model(self.kind, path))
        else {
            *state = Slot::Unavailable(ModelStatus::NotInstalled {
                expected: self.path.clone(),
            });
            return false;
        };
        *state = match load(&file) {
            Ok((model, name)) => Slot::Ready { model, name },
            Err(message) => Slot::Unavailable(ModelStatus::Failed { message }),
        };
        matches!(*state, Slot::Ready { .. })
    }

    /// What tells one model file from another: its name, size and
    /// modification time. `None` while no model is loaded.
    pub(crate) fn identity(&self) -> Option<(String, Option<u64>, Option<std::time::SystemTime>)> {
        let ModelStatus::Ready { name } = self.status() else {
            return None;
        };
        let metadata = self
            .path
            .as_deref()
            .and_then(|path| installed_model(self.kind, path))
            .and_then(|model| std::fs::metadata(model).ok());
        Some((
            name,
            metadata.as_ref().map(std::fs::Metadata::len),
            metadata.and_then(|metadata| metadata.modified().ok()),
        ))
    }
}

/// A poisoned lock only means another job panicked mid-update; the state
/// stays usable (worst case a model loads again).
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_finds_its_own_download_but_not_the_others() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let inpaint = folder.path().join(ModelKind::Inpainting.file_name());
        let people = folder.path().join(ModelKind::People.file_name());
        assert_eq!(installed_model(ModelKind::Inpainting, &inpaint), None);
        std::fs::write(folder.path().join("notes.txt"), b"").expect("write");
        let lama = folder.path().join("lama_fp32.onnx");
        std::fs::write(&lama, b"").expect("write");
        let rfdetr = folder.path().join("rfdetr-seg-medium.onnx");
        std::fs::write(&rfdetr, b"").expect("write");
        assert_eq!(installed_model(ModelKind::Inpainting, &inpaint), Some(lama));
        assert_eq!(installed_model(ModelKind::People, &people), Some(rfdetr));
        std::fs::write(folder.path().join("other.ONNX"), b"").expect("write");
        assert_eq!(
            installed_model(ModelKind::Inpainting, &inpaint),
            None,
            "two inpainting models: neither"
        );
        std::fs::write(&inpaint, b"").expect("write");
        assert_eq!(
            installed_model(ModelKind::Inpainting, &inpaint),
            Some(inpaint)
        );
    }

    #[test]
    fn a_model_is_looked_for_again_until_one_is_installed() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let expected = folder.path().join("people.onnx");
        let model: LazyModel<str> = LazyModel::new(ModelKind::People, Some(expected.clone()));
        let load = |_: &Path| Err::<(Arc<str>, String), _>("not a model".to_string());
        model.ensure(load);
        if model.status() == ModelStatus::NotBuilt {
            return;
        }
        assert!(matches!(model.status(), ModelStatus::NotInstalled { .. }));
        std::fs::write(&expected, b"").expect("write");
        model.ensure(load);
        assert!(matches!(model.status(), ModelStatus::Failed { .. }));
        let loaded = model.ensure(|_| Ok((Arc::from("model"), "model".to_string())));
        assert!(!loaded, "a model that failed is not tried again");
    }
}
