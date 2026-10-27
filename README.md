# Photonation

[![CI](https://github.com/DeBlo12/photonation/actions/workflows/ci.yml/badge.svg)](https://github.com/DeBlo12/photonation/actions/workflows/ci.yml)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

A fast, memory-efficient, non-destructive raw photo editor written in pure Rust —
a Lightroom-style develop module with a Photomator-inspired interface.

- **Raw development** for Bayer cameras (Canon, Nikon, Sony, Panasonic, Olympus/OM,
  Pentax, Leica…), Fujifilm X-Trans and linear DNG (Apple ProRAW), via
  [rawler](https://github.com/dnglab/dnglab).
- **Lightroom's light and colour tools**: white balance (presets, eyedropper,
  temperature/tint), exposure, contrast, highlights, shadows, whites, blacks,
  texture, clarity, dehaze, vibrance, saturation, parametric and point tone curves
  (RGB and per channel), an 8-band colour mixer, point colours (pick a colour in
  the photo and shift its hue, saturation and luminance), colour grading wheels
  (shadows/midtones/highlights/global with blending and balance), sharpening.
- **Non-destructive**: originals are never touched; edits live in a JSON sidecar
  next to each photo (`IMG_0001.CR3.photonation.json`), with undo/redo.
- **Fast and lean**: a band-parallel pipeline renders a 24 MP raw to full-size
  JPEG in ~170 ms (Apple M1 Pro) without ever holding a full-size float image;
  slider drags re-render the preview in ~40 ms.
- **Export** to JPEG, PNG (8/16-bit) and TIFF (8/16-bit) in sRGB, Display P3,
  Adobe RGB or ProPhoto RGB with embedded ICC profiles.
- **Desktop app** (macOS first; egui, so the same UI can later run in the browser
  via WebAssembly) and a **CLI** for batch export.

> Status: early. The core pipeline, CLI and desktop editor work; see
> [docs/ROADMAP.md](docs/ROADMAP.md) for what's next.

## Quick start

Requires Rust 1.95+ (the pinned toolchain in `rust-toolchain.toml` installs
automatically with rustup). On macOS the Xcode licence must be accepted
once (`sudo xcodebuild -license`).

```bash
cargo run --release -p photonation-desktop -- ~/Pictures/Raw
```

Opens the editor on a folder (or pass a single raw file to open it directly).

```bash
cargo run --release -p photonation-cli -- export ~/Pictures/Raw -o ~/Pictures/Export --long-edge 2048
```

Batch-exports every raw in a folder, applying each photo's sidecar edits.

### CLI

| Command | What it does |
|---|---|
| `photonation info <files…>` | Camera, lens, exposure, size, as-shot white balance |
| `photonation export <files or folders…>` | Develop and write JPEG/PNG/TIFF (`--format`, `--quality`, `--long-edge`, `--color-space`, `--depth`, slider overrides such as `--exposure 0.5 --shadows 30`) |
| `photonation edit <files…> --exposure 0.3 …` | Create or update sidecars without exporting |

### Desktop shortcuts

| Key | Action |
|---|---|
| ⌘O | Open folder |
| ← / → | Previous / next photo |
| ⌘Z / ⇧⌘Z | Undo / redo |
| `\` | Compare with original (drag the divider) |
| ⌘0, double-click | Fit / actual pixels |
| ⇧⌘E | Export |
| Double-click a slider | Reset it; hold ⌥ while dragging for fine control |

## Documentation

| Document | Contents |
|---|---|
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crates, layers, data model, key decisions |
| [PIPELINE.md](docs/PIPELINE.md) | The render path step by step: band engine, demosaic, colour, tone |
| [EDIT-MODEL.md](docs/EDIT-MODEL.md) | Settings, controls, sidecars, process versions |
| [ENGINE.md](docs/ENGINE.md) | Background workers, events, export |
| [DESKTOP.md](docs/DESKTOP.md) | The editor's layout and per-frame control flow |
| [DEVELOPMENT.md](docs/DEVELOPMENT.md) | Building, testing, conventions, adding a control |
| [ROADMAP.md](docs/ROADMAP.md) | What's done and what's next |

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) and the
[Code of Conduct](CODE_OF_CONDUCT.md). Report security issues privately as
described in [SECURITY.md](SECURITY.md).

## License

Photonation is free software, licensed under the
[GNU General Public License v3.0 or later](LICENSE). You may use, study, share
and modify it; distributed copies and derivatives must stay under the GPL with
their source available. Third-party components and their licences are listed in
[NOTICE](NOTICE).
