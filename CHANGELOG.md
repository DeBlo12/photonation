# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/). Each merged pull request that
adds features is a minor release and each fix-only one a patch release;
changes to CI alone are not released.

## [Unreleased]

## [0.15.1] - 2026-10-31

### Changed

- Without a generative model, the Remove panel links the LaMa download, shows
  its SHA-256 to check it against, opens the models folder and checks again
  without a restart. The download can keep its own name
  (`lama_fp32.onnx`) when it is the only model in the folder.

### Fixed

- With the masks, crop or Remove tool open, the grid button (or G, N, the
  filter chip) flashed the grid for a moment and closed it again. Now the
  grid waits: a message names the tool to finish first, and its panel is
  outlined. ([#20](https://github.com/DeBlo12/photonation/pull/20))

## [0.15.0] - 2026-10-31

### Added

- Culling: rate photos 0–5 stars (0–5, `]` `[`), pick (P), reject (X) or
  unflag (U) them and give them a colour label (6–9), from the keyboard,
  the stars next to the photo name or the new Library menu. Ratings are
  saved in the sidecar at once and are not edits. Ratings and labels from
  Lightroom XMP sidecars come along.
- Auto-advance moves to the next photo after a rating or flag.
- Filter (stars, flag, label, edited) and sort (file name, capture time,
  rating, edited first); the filmstrip, ← → and batch export follow them.
- Grid view (G) with Ctrl/⌘- and Shift-click selection, Select All and a
  thumbnail size slider; ratings apply to every selected photo.
- Survey view (N): 2–8 selected photos side by side, rated one at a time.
- Library › Move Rejected Photos to Recycle Bin (Trash on a Mac) moves the
  rejects and their sidecars to the bin after asking.
- The filmstrip shows stars, flags and labels while open, and a scroll bar
  while it moves: drag it to cross a long folder, with the photo's name and
  date shown as you go.
- Export can cover the selected or the shown photos.

### Changed

- Thumbnails show the camera's embedded preview at once, then
  Photonation's rendering, which is kept in a disk cache (up to 512 MB) so
  folders open with their renders next time.
  ([#19](https://github.com/DeBlo12/photonation/pull/19))

## [0.14.0] - 2026-10-31

### Added

- Curves draw the photo's histogram behind the curve: brightness behind the
  parametric and RGB curves, the channel itself behind the red, green and
  blue ones.

### Fixed

- Curve points follow the pointer for the whole drag; before, they stopped
  after the first few pixels. A quick drag on a point moves it instead of
  adding a new one, and dragging a point placed right next to another no
  longer crashes the editor. ([#18](https://github.com/DeBlo12/photonation/pull/18))

## [0.13.0] - 2026-10-31

### Added

- Help ▸ Keyboard Shortcuts lists every shortcut.

### Changed

- The Help popover is replaced by a Help window: the ? button, Help ▸
  Photonation Help or F1 opens a window with one page per feature, listed
  on the left: getting started, how a photo is developed (the order of the
  steps), each adjustment card, masks and mask groups, crop and straighten,
  Remove, compare and zoom, export (formats, colour spaces, file name
  templates, metadata), Settings, Lightroom import and the keyboard
  shortcuts. It opens on the page of the tool in use (masks, crop, Remove,
  the export sheet or Settings) and otherwise where it was left; Esc
  closes it. ([#16](https://github.com/DeBlo12/photonation/pull/16))

## [0.12.0] - 2026-10-31

### Added

- GPU previews: the editor renders previews with wgpu compute shaders,
  about 30–40 times faster than the CPU on a 24 MP photo (≈10 ms at
  1600 px, with or without masks; ≈16 ms with Texture and Clarity),
  identical to the CPU render but for rare one-level differences.
  Straightened crops and removals still render on the CPU, as do exports.
  `PHOTONATION_GPU_RENDER=off` turns it off.
  ([#15](https://github.com/DeBlo12/photonation/pull/15))

## [0.11.0] - 2026-10-31

### Added

- Photonation look: the editor keeps Photomator's layout (photo first, a
  rounded inspector floating at the right) in the brand's colours and type.
  Surfaces lean toward Ink, Photon amber marks what is on (tools, switches,
  slider fills, selections, the Export button, with Ink text on amber), text
  is Paper, and the bundled Inter and Sora fonts replace the system font on
  every platform. The welcome screen shows the iris and the wordmark.
- Help: a ? button in the toolbar opens a popover that explains how a photo
  is developed (the order of the steps) and what each card, slider, mask,
  crop, removal and export setting changes. Toolbar tooltips now say what
  each tool does, and appear on disabled tools as well.
- Settings (File ▸ Settings…, Ctrl+, / ⌘,): choose the graphics processor
  (automatic, integrated first, software, or a specific adapter; applied at
  the next start), a default export folder (where the folder dialog starts,
  or where exports go without asking), and the format, size, colour space,
  file name and metadata new exports start with. Saved between launches.
- Menu bar with File, Edit, View and Help menus listing every action and its
  shortcut, and an About window.
  ([#14](https://github.com/DeBlo12/photonation/pull/14))

## [0.10.0] - 2026-10-31

### Added

- Windows installer: `packaging\windows\build-installer.ps1` builds
  `photonation-<version>-windows-x64-setup.exe` (Inno Setup), a per-user
  install with a Start menu entry, an optional desktop shortcut, the CLI
  optionally on `PATH`, and an uninstaller. On Windows the editor runs
  without a console window, the toolbar no longer leaves room for macOS
  window buttons, and tooltips and hints show Ctrl and Shift.
- Brand icons: the editor's toolbar, colour pickers, crop, mask and card
  controls draw the Photonation stroke icons (library, crop, layers,
  retouch, rotate, flip, focus) instead of Unicode symbols, crisp at any
  size and the same on every platform. Undo, redo, compare and the gradient
  shapes, which the set lacks, are drawn to its rules.
- App icon: the Photonation iris (Ink) in the title bar, taskbar and Dock,
  embedded in the Windows executable and the installer, with an `.icns`
  for the macOS app bundle. `assets/icon` holds the SVG masters of all four
  variants (Ink, Paper, Photon, Mono); `packaging/icons` regenerates them.
  ([#13](https://github.com/DeBlo12/photonation/pull/13))

## [0.9.0] - 2026-10-28

### Added

- Export metadata: exported JPEG, PNG and TIFF files carry EXIF and XMP with
  the camera, lens, exposure and capture time, the creator and copyright
  (from the raw file or entered in the export sheet, remembered between
  launches) and, only if chosen, the GPS position. The export sheet offers
  All / Copyright Only / None and a checkbox per group;
  `photonation export` takes `--metadata`, `--location`, `--creator` and
  `--copyright`. Orientation is written as normal and the ICC profile is
  kept. `photonation info` also shows exposure compensation, artist,
  copyright and location.
  ([#12](https://github.com/DeBlo12/photonation/pull/12))

## [0.8.0] - 2026-10-28

### Added

- Mask groups: gather masks in collapsible folders in the masks panel, each
  with a name, a visibility switch and an amount that scale every mask in it.
  Drag masks into, out of and between groups, reorder groups, ungroup or delete
  them. Existing sidecars load unchanged. Mask thumbnails and the overlay now
  show a mask's amount (and its group's) and dim while it is hidden.
  ([#11](https://github.com/DeBlo12/photonation/pull/11))

## [0.7.1] - 2026-10-28

### Fixed

- In the masks panel, the selected row's highlight no longer covers the next
  row and the hint below the list; long mask names end in "…" instead of
  running out of the row.
  ([#10](https://github.com/DeBlo12/photonation/pull/10))

## [0.7.0] - 2026-10-28

### Added

- Export file names: the export sheet and `photonation export --name` take
  a name, or a template with `{name}` (original name) and `{n}` (number in
  the batch); the sheet previews the resulting file name.
  ([#9](https://github.com/DeBlo12/photonation/pull/9))

## [0.6.0] - 2026-10-28

### Added

- Object removal: paint over a distraction (Remove panel, `Q`) and it is
  replaced by a content-aware fill (PatchMatch) or, with an installed LaMa
  or MI-GAN ONNX model, a generative fill. Fills are computed in the
  background and cached; removals are stored in the sidecar.
  ([#8](https://github.com/DeBlo12/photonation/pull/8))

## [0.5.1] - 2026-10-28

### Fixed

- The filmstrip scrolls with the mouse wheel and trackpad, anywhere on the
  strip, and keeps the selected photo in view when ← and → change it.
  ([#7](https://github.com/DeBlo12/photonation/pull/7))

## [0.5.0] - 2026-10-27

### Added

- Crop and straighten: a crop tool (R) with aspect ratios, an angle slider and
  dragging outside the frame to turn it. Turned crops render with bicubic
  resampling; masks, the overlay and the eyedropper follow them.
- Photos edited in Lightroom open with their edits: white balance, light,
  presence, colour, curves, colour mixer, colour grading, sharpening and crop
  are read from the `.xmp` sidecar (never written). The editor names edits it
  could not bring along, such as masks.
  ([#6](https://github.com/DeBlo12/photonation/pull/6))

### Fixed

- Compare (◐) shows the original with the edit's crop, so the two line up.

## [0.4.0] - 2026-10-27

### Added

- Point colours: pick up to eight colours in the photo and shift the hue,
  saturation and luminance of everything similar, with adjustable hue,
  saturation and luminance ranges and a "Visualize range" overlay (the Color
  Mixer's Point Color tab). The Mixer tab gets a picker that selects the band
  of a colour clicked in the photo.
  ([#5](https://github.com/DeBlo12/photonation/pull/5))

### Fixed

- Texture and Clarity no longer change by a rounding step depending on how the
  render is split into bands.

## [0.3.0] - 2026-10-27

### Added

- Mask layers: a mask combines up to eight linear and radial shapes with
  Add, Subtract or Intersect, then applies invert and an amount. Sidecars
  with a single shape load unchanged.
  ([#4](https://github.com/DeBlo12/photonation/pull/4))

### Changed

- A layer panel replaces the floating mask window and takes the sidebar's
  place while masking (◎ / `M`): rows with a tinted thumbnail, a summary and
  a visibility switch; drag to reorder, right-click to invert, duplicate,
  move or delete. On the photo, rings mark the selected mask's other shapes
  and pins the other masks.
- The filmstrip rests as a thin strip of tiny thumbnails and opens to full
  size, over the photo, while the pointer is on it or just after the photo
  changes.

## [0.2.0] - 2026-10-27

### Added

- Masks: linear and radial gradients with invert, and local temperature, tint,
  exposure, contrast, highlights, shadows, dehaze and saturation. The desktop
  editor has a mask window (M), draggable handles and a GPU-drawn overlay.
- The desktop editor prefers a discrete GPU, falling back to integrated
  graphics and then software rendering; `PHOTONATION_GPU` overrides the order.
  ([#3](https://github.com/DeBlo12/photonation/pull/3))

### Fixed

- A panic while decoding or rendering one file no longer stops previews,
  thumbnails or exports for the rest of the session.
- Zooming to 1:1 on photos larger than the GPU's texture limit no longer crashes.
- Batch exports from the editor no longer replace existing files.

### Security

- Atomic writes (sidecars, exports) no longer follow symlinks planted at a
  predictable temporary path.
- Sidecar reads are capped at 4 MiB, masks at 32 per photo, and raw file
  dimensions are checked for overflow.

## [0.1.0] - 2026-10-25

### Added

- Raw decoding for Bayer cameras, Fujifilm X-Trans and linear DNG (Apple ProRAW),
  with EXIF orientation and DNG baseline exposure.
- Band-parallel develop pipeline: white balance, exposure, contrast,
  highlights, shadows, whites, blacks, texture, clarity, dehaze, vibrance,
  saturation, parametric and point tone curves, colour mixer, colour grading,
  sharpening.
- Export to JPEG, PNG and TIFF (8/16-bit) in sRGB, Display P3, Adobe RGB and
  ProPhoto RGB with ICC profiles.
- JSON develop sidecars with process versions; undo/redo.
- Desktop editor with viewer, compare, filmstrip, adjustment cards and export.
- `photonation` CLI: `info`, `export`, `edit`.

### Changed

- Licensed under GPL-3.0-or-later.

[Unreleased]: https://github.com/DeBlo12/photonation/compare/v0.15.1...HEAD
[0.15.1]: https://github.com/DeBlo12/photonation/compare/v0.15.0...v0.15.1
[0.15.0]: https://github.com/DeBlo12/photonation/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/DeBlo12/photonation/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/DeBlo12/photonation/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/DeBlo12/photonation/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/DeBlo12/photonation/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/DeBlo12/photonation/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/DeBlo12/photonation/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/DeBlo12/photonation/compare/v0.7.1...v0.8.0
[0.7.1]: https://github.com/DeBlo12/photonation/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/DeBlo12/photonation/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/DeBlo12/photonation/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/DeBlo12/photonation/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/DeBlo12/photonation/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/DeBlo12/photonation/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/DeBlo12/photonation/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/DeBlo12/photonation/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/DeBlo12/photonation/releases/tag/v0.1.0
