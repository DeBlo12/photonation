# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
- Masks: linear and radial gradients with invert, and local temperature, tint,
  exposure, contrast, highlights, shadows, dehaze and saturation. The desktop
  editor has a mask window (M), draggable handles and a GPU-drawn overlay.
- Crop and straighten: a crop tool (R) with aspect ratios, an angle slider and
  dragging outside the frame to turn it. Turned crops render with bicubic
  resampling; masks, the overlay and the eyedropper follow them.
- The desktop editor prefers a discrete GPU, falling back to integrated
  graphics and then software rendering; `PHOTONATION_GPU` overrides the order.

### Fixed

- Compare (◐) shows the original with the edit's crop, so the two line up.

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
