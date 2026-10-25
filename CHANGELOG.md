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

### Changed

- Licensed under GPL-3.0-or-later.
