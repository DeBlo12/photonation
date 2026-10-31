# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- GPU previews: the editor renders previews with wgpu compute shaders,
  about 25 times faster than the CPU on a 24 MP photo (≈10 ms at 1600 px),
  identical to the CPU render but for rare one-level differences. Masks,
  Texture and Clarity, straightened crops and removals still render on the
  CPU, as do exports. `PHOTONATION_GPU_RENDER=off` turns it off.

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
- Export metadata: exported JPEG, PNG and TIFF files carry EXIF and XMP with
  the camera, lens, exposure and capture time, the creator and copyright
  (from the raw file or entered in the export sheet, remembered between
  launches) and, only if chosen, the GPS position. The export sheet offers
  All / Copyright Only / None and a checkbox per group;
  `photonation export` takes `--metadata`, `--location`, `--creator` and
  `--copyright`. Orientation is written as normal and the ICC profile is
  kept. `photonation info` also shows exposure compensation, artist,
  copyright and location.
- Export file names: the export sheet and `photonation export --name` take
  a name, or a template with `{name}` (original name) and `{n}` (number in
  the batch); the sheet previews the resulting file name.
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
- Masks: linear and radial gradients with invert, and local temperature, tint,
  exposure, contrast, highlights, shadows, dehaze and saturation. The desktop
  editor has a mask window (M), draggable handles and a GPU-drawn overlay.
- Crop and straighten: a crop tool (R) with aspect ratios, an angle slider and
  dragging outside the frame to turn it. Turned crops render with bicubic
  resampling; masks, the overlay and the eyedropper follow them.
- Photos edited in Lightroom open with their edits: white balance, light,
  presence, colour, curves, colour mixer, colour grading, sharpening and crop
  are read from the `.xmp` sidecar (never written). The editor names edits it
  could not bring along, such as masks.
- Point colours: pick up to eight colours in the photo and shift the hue,
  saturation and luminance of everything similar, with adjustable hue,
  saturation and luminance ranges and a "Visualize range" overlay (the Color
  Mixer's Point Color tab). The Mixer tab gets a picker that selects the band
  of a colour clicked in the photo.
- Object removal: paint over a distraction (Remove panel, `Q`) and it is
  replaced by a content-aware fill (PatchMatch) or, with an installed LaMa
  or MI-GAN ONNX model, a generative fill. Fills are computed in the
  background and cached; removals are stored in the sidecar.
- Mask groups: gather masks in collapsible folders in the masks panel, each
  with a name, a visibility switch and an amount that scale every mask in it.
  Drag masks into, out of and between groups, reorder groups, ungroup or delete
  them. Existing sidecars load unchanged. Mask thumbnails and the overlay now
  show a mask's amount (and its group's) and dim while it is hidden.
- The desktop editor prefers a discrete GPU, falling back to integrated
  graphics and then software rendering; `PHOTONATION_GPU` overrides the order.

### Fixed

- Compare (◐) shows the original with the edit's crop, so the two line up.
- Texture and Clarity no longer change by a rounding step depending on how the
  render is split into bands.

- A panic while decoding or rendering one file no longer stops previews,
  thumbnails or exports for the rest of the session.
- Zooming to 1:1 on photos larger than the GPU's texture limit no longer crashes.
- Batch exports from the editor no longer replace existing files.

### Security

- Atomic writes (sidecars, exports) no longer follow symlinks planted at a
  predictable temporary path.
- Sidecar reads are capped at 4 MiB, masks at 32 per photo, and raw file
  dimensions are checked for overflow.

### Changed

- Licensed under GPL-3.0-or-later.
