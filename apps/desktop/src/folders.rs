// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Moving around inside the folder the user opened: which folders lie below
//! the open one, and going down into them and back up. The overview and the
//! breadcrumb are drawn by `panels::folders`. See docs/DESKTOP.md.

use crate::app::PhotonationApp;
use eframe::egui;
use photonation_develop::load_photo;
use photonation_engine::{RenderedImage, Subfolder};
use std::path::{Path, PathBuf};

/// A folder below the open one, as a card in the overview.
pub struct FolderCard {
    pub subfolder: Subfolder,
    pub cover_image: Option<egui::TextureHandle>,
    /// The cover is a render, not the embedded preview it starts as.
    cover_is_render: bool,
}

/// A move between folders. Panels ask for one and it happens as the next
/// frame starts, so a frame draws one folder from top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FolderRequest {
    Open(PathBuf),
    /// From the overview back to the photos of a folder that has both.
    ShowPhotos,
    /// From the photos of a folder that has both to its overview.
    ShowSubfolders,
}

#[derive(Default)]
pub struct FolderBrowser {
    /// The folder the user chose: the breadcrumb goes no higher.
    root: Option<PathBuf>,
    /// The folders with raw files directly inside the open one, by name.
    pub subfolders: Vec<FolderCard>,
    /// Still looking for more of them.
    pub scanning: bool,
    /// The overview is showing in place of the open folder's photos.
    showing_over_photos: bool,
    /// The photo that was open when the overview took over.
    photo_to_return_to: Option<usize>,
    request: Option<FolderRequest>,
}

impl FolderBrowser {
    /// `folder` has been opened: a folder inside the current root is a step
    /// within it, any other is a new root. Its subfolders are not known yet.
    pub fn folder_opened(&mut self, folder: &Path) {
        let inside_root = self.root.as_ref().is_some_and(|r| folder.starts_with(r));
        if !inside_root {
            self.root = Some(folder.to_path_buf());
        }
        self.subfolders.clear();
        self.scanning = true;
        self.showing_over_photos = false;
        self.photo_to_return_to = None;
    }

    pub fn request(&mut self, request: FolderRequest) {
        self.request = Some(request);
    }

    pub fn showing_over_photos(&self) -> bool {
        self.showing_over_photos
    }

    /// The photos below the open folder, as far as the scan has got.
    pub fn photos_below(&self) -> usize {
        self.subfolders
            .iter()
            .map(|card| card.subfolder.photo_count)
            .sum()
    }

    /// The folders from the root down to `folder`, for the breadcrumb.
    pub fn trail_to(&self, folder: &Path) -> Vec<PathBuf> {
        let Some(root) = &self.root else {
            return Vec::new();
        };
        let mut trail: Vec<PathBuf> = folder
            .ancestors()
            .take_while(|ancestor| ancestor != root)
            .map(Path::to_path_buf)
            .collect();
        trail.push(root.clone());
        trail.reverse();
        trail
    }

    /// Every card found has its cover, for the screenshot aid.
    pub fn settled(&self) -> bool {
        !self.scanning && self.subfolders.iter().all(|c| c.cover_image.is_some())
    }

    /// Keeps the cards in name order. The same folder found again replaces
    /// its card, in case a scan of this folder was still reporting when it
    /// was opened a second time.
    fn add(&mut self, subfolder: Subfolder) {
        let card = FolderCard {
            subfolder,
            cover_image: None,
            cover_is_render: false,
        };
        let position = self
            .subfolders
            .binary_search_by(|other| other.subfolder.path.cmp(&card.subfolder.path));
        match position {
            Ok(index) => self.subfolders[index] = card,
            Err(index) => self.subfolders.insert(index, card),
        }
    }

    /// Shows `image` on the card whose cover is `path`, if there is one.
    pub fn cover_arrived(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        image: &RenderedImage,
        rendered: bool,
    ) {
        let card = self
            .subfolders
            .iter_mut()
            .find(|card| card.subfolder.cover == path);
        let Some(card) = card else {
            return;
        };
        // A render is never replaced by the embedded preview.
        if card.cover_is_render && !rendered {
            return;
        }
        card.cover_image = Some(crate::app::thumbnail_texture(
            ctx,
            format!("cover-{}", path.display()),
            image,
        ));
        card.cover_is_render = rendered;
    }
}

impl PhotonationApp {
    /// The overview takes the window: a folder with no photos of its own
    /// always shows it, and says so when there is nothing below it either.
    pub fn showing_folder_overview(&self) -> bool {
        self.folder.is_some() && (self.photos.is_empty() || self.folders.showing_over_photos)
    }

    /// Carries out the move a panel asked for during the last frame. Like
    /// the grid, it waits while an editing tool is open.
    pub fn apply_folder_request(&mut self) {
        let Some(request) = self.folders.request.take() else {
            return;
        };
        if self.tool_blocks_library() {
            return;
        }
        match request {
            FolderRequest::Open(folder) => self.open_folder(folder),
            FolderRequest::ShowSubfolders => {
                self.folders.photo_to_return_to = self.current;
                self.folders.showing_over_photos = true;
                self.close_photo();
            }
            FolderRequest::ShowPhotos => {
                self.folders.showing_over_photos = false;
                let photo = self.folders.photo_to_return_to.take().unwrap_or(0);
                self.select(photo);
            }
        }
    }

    /// The scan of `parent` found a subfolder; asks for its cover.
    pub fn subfolder_found(&mut self, parent: &Path, subfolder: Subfolder) {
        if self.folder.as_deref() != Some(parent) {
            return;
        }
        let cover = subfolder.cover.clone();
        self.folders.add(subfolder);
        let settings = load_photo(&cover)
            .unwrap_or_default()
            .settings
            .map(|loaded| loaded.settings)
            .unwrap_or_default();
        self.engine.request_thumbnail(cover, settings);
    }

    pub fn subfolders_scanned(&mut self, parent: &Path) {
        if self.folder.as_deref() == Some(parent) {
            self.folders.scanning = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    const TIMEOUT: Duration = Duration::from_secs(30);

    /// A folder with these (unreadable) files; parents are made as needed.
    fn folder_with(files: &[&str]) -> TempDir {
        let root = tempfile::tempdir().expect("temporary folder");
        for file in files {
            let path = root.path().join(file);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create folders");
            std::fs::write(path, b"not a raw").expect("write photo");
        }
        root
    }

    struct Browsing {
        ctx: egui::Context,
        app: PhotonationApp,
        root: TempDir,
    }

    impl Browsing {
        fn open(files: &[&str]) -> Browsing {
            let root = folder_with(files);
            let ctx = egui::Context::default();
            let mut app = PhotonationApp::empty(&ctx);
            app.open_folder(root.path().to_path_buf());
            let mut browsing = Browsing { ctx, app, root };
            browsing.wait_for_the_scan();
            browsing
        }

        fn wait_for_the_scan(&mut self) {
            let started = Instant::now();
            while self.app.folders.scanning {
                assert!(started.elapsed() < TIMEOUT, "the scan ends");
                self.app.handle_events(&self.ctx);
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn go(&mut self, request: FolderRequest) {
            self.app.folders.request(request);
            self.app.apply_folder_request();
            self.wait_for_the_scan();
        }

        fn subfolder_names(&self) -> Vec<String> {
            self.app
                .folders
                .subfolders
                .iter()
                .map(|card| {
                    let name = card.subfolder.path.file_name().expect("a name");
                    name.to_string_lossy().into_owned()
                })
                .collect()
        }
    }

    #[test]
    fn a_folder_of_only_subfolders_opens_on_the_overview() {
        let browsing = Browsing::open(&["2026_07/b.CR3", "2026_06/a.CR3", "2026_06/c.CR3"]);
        assert!(browsing.app.showing_folder_overview());
        assert_eq!(browsing.subfolder_names(), ["2026_06", "2026_07"]);
        assert_eq!(browsing.app.folders.photos_below(), 3);
    }

    #[test]
    fn a_folder_of_only_photos_opens_on_them() {
        let browsing = Browsing::open(&["a.CR3"]);
        assert!(!browsing.app.showing_folder_overview());
        assert_eq!(browsing.app.current, Some(0));
        assert!(browsing.app.folders.subfolders.is_empty());
    }

    #[test]
    fn a_folder_with_nothing_inside_still_shows_the_overview_empty() {
        let browsing = Browsing::open(&["exports/a.jpg"]);
        assert!(browsing.app.showing_folder_overview());
        assert!(browsing.app.folders.subfolders.is_empty());
        assert!(!browsing.app.folders.scanning);
    }

    #[test]
    fn a_folder_with_both_opens_on_its_photos_and_comes_back_to_the_same_one() {
        let mut browsing = Browsing::open(&["a.CR3", "b.CR3", "month/c.CR3"]);
        assert!(!browsing.app.showing_folder_overview());
        assert_eq!(browsing.subfolder_names(), ["month"]);
        browsing.app.select(1);

        browsing.go(FolderRequest::ShowSubfolders);
        assert!(browsing.app.showing_folder_overview());
        assert_eq!(browsing.app.current, None, "no hidden photo takes the keys");

        browsing.go(FolderRequest::ShowPhotos);
        assert!(!browsing.app.showing_folder_overview());
        assert_eq!(browsing.app.current, Some(1));
    }

    #[test]
    fn going_down_and_back_up_keeps_the_trail_to_the_folder_first_opened() {
        let mut browsing = Browsing::open(&["2026/06/a.CR3"]);
        let root = browsing.root.path().to_path_buf();
        let month = root.join("2026/06");

        browsing.go(FolderRequest::Open(month.clone()));
        assert_eq!(browsing.app.current, Some(0));
        assert_eq!(
            browsing.app.folders.trail_to(&month),
            [root.clone(), root.join("2026"), month.clone()]
        );

        browsing.go(FolderRequest::Open(root.join("2026")));
        assert_eq!(browsing.app.current, None);
        assert_eq!(browsing.subfolder_names(), ["06"]);
        assert_eq!(
            browsing.app.folders.trail_to(&root.join("2026")),
            [root.clone(), root.join("2026")]
        );
    }

    #[test]
    fn opening_a_folder_elsewhere_starts_a_new_trail() {
        let mut browsing = Browsing::open(&["month/a.CR3"]);
        let elsewhere = folder_with(&["b.CR3"]);
        browsing.go(FolderRequest::Open(elsewhere.path().to_path_buf()));
        assert_eq!(
            browsing.app.folders.trail_to(elsewhere.path()),
            [elsewhere.path().to_path_buf()]
        );
    }

    #[test]
    fn leaving_a_photo_for_an_empty_folder_leaves_nothing_of_it_to_undo() {
        let mut browsing = Browsing::open(&["a.CR3", "month/deeper/b.CR3"]);
        browsing.app.settings.exposure = 1.0;
        browsing.app.history.commit(&browsing.app.settings);
        assert!(browsing.app.history.can_undo());

        let month = browsing.root.path().join("month");
        browsing.go(FolderRequest::Open(month));
        assert_eq!(browsing.app.current, None);
        assert!(!browsing.app.history.can_undo());
        assert_eq!(browsing.app.settings, Default::default());
    }

    #[test]
    fn an_editing_tool_holds_back_a_move() {
        let mut browsing = Browsing::open(&["a.CR3", "month/b.CR3"]);
        browsing.app.view.toggle_cropping();
        browsing.go(FolderRequest::ShowSubfolders);
        assert!(!browsing.app.showing_folder_overview());
        assert_eq!(browsing.app.current, Some(0));
    }

    #[test]
    fn subfolders_of_a_folder_no_longer_open_are_ignored() {
        let mut browsing = Browsing::open(&["a.CR3"]);
        let cover = browsing.root.path().join("stale/b.CR3");
        browsing.app.subfolder_found(
            Path::new("/somewhere/else"),
            Subfolder {
                path: browsing.root.path().join("stale"),
                photo_count: 1,
                cover,
            },
        );
        assert!(browsing.app.folders.subfolders.is_empty());
        browsing.wait_for_the_scan();
    }
}
