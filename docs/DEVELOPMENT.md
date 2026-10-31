# Development

## Build and test

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets
```

```bash
cargo run --release -p photonation-desktop -- ~/Pictures/Raw
```

Dependencies and the per-pixel crates build optimized even in dev profiles
(`Cargo.toml`), so debug builds are usable for image work.

## Windows installer

On Windows (x64), with Rust, the Visual Studio C++ build tools and
[Inno Setup 6](https://jrsoftware.org/isinfo.php) (`winget install JRSoftware.InnoSetup`):

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
```

It builds the release binaries and writes
`target\installer\photonation-<version>-windows-x64-setup.exe`. The installer
(`packaging/windows/photonation.iss`) installs per user without
administrator rights, or for all users if chosen; adds a Start menu entry, an
optional desktop shortcut and, optionally, the `photonation` CLI to `PATH`; and
registers an uninstaller. Release builds of the editor run without a console
window, so its log output is only visible from debug builds.

## App icon

`assets/icon` holds the app icon: SVG masters of the four variants (Ink is
the app icon on every platform; Paper, Photon and Mono are for light
surfaces, marketing and system tinting), the PNGs the desktop app sets as its
window icon (`photonation-256.png`; `photonation-macos-512.png`, on Apple's
icon grid), `photonation.ico` (embedded in the Windows executable by
`apps/desktop/build.rs` and used by the installer) and `photonation.icns` (for
the macOS app bundle). Don't edit them by hand; change the design in
`packaging/icons/src/main.rs` and regenerate them all:

```bash
cargo run --release --manifest-path packaging/icons/Cargo.toml
```

Interface icons come from the same brand: `apps/desktop/src/widgets/icon.rs`
holds the stroke icons as SVG path data on a 24 u grid, drawn as vectors with
the set's optical stroke ladder. Add an icon there (from the set, or drawn to
its rules: 20 u live area, 1.75 stroke, round ends, 2 u corners) rather than
using a Unicode symbol, which depends on the platform's fonts.

## Git workflow

- `main` is always releasable; work on a branch (`feature/…`, `fix/…`) and open a
  pull request. CI must pass (fmt, clippy, tests on macOS and Linux, licence and
  advisory checks).
- Write commit subjects in the imperative mood ("Add crop tool"), ≤ 72
  characters, with a body explaining *why* when it isn't obvious.
- Note user-visible changes in [CHANGELOG.md](../CHANGELOG.md) under *Unreleased*.
- Every source file starts with an SPDX header:
  `// SPDX-License-Identifier: GPL-3.0-or-later`.
- New dependencies must have a licence allowed in `deny.toml`
  (`cargo deny check` locally).

## Conventions

- **Layering**: a crate depends only on lower layers ([ARCHITECTURE.md](ARCHITECTURE.md)).
  Nothing below `apps/` uses egui.
- **DRY**: one definition per concept. Slider ranges live in `Control` specs,
  export goes through `engine::export_photo`, and per-bit-depth or per-format
  code is generic rather than duplicated.
- **Names say what a value holds** (`band_rows`, `wb_multipliers`,
  `points_per_image_pixel`); single letters only for coordinates and tight
  maths kernels.
- **Docs travel with code**: behaviour and control flow are described in
  `docs/*.md`; update the matching document in the same change. Code comments
  explain *why*, not what.
- **No panics on user data**: decoding and rendering return errors; `unwrap` is
  linted.
- **Pipeline invariants are tested**: band-size invariance, neutral stays
  neutral, monotone curves. Add a test when adding a stage.

## Adding a slider

1. Add the field to `Settings` (and its `Default`) in `crates/develop/src/settings.rs`.
2. Add a `control!` entry to the right group in `controls.rs`, and handle it in
   `AdjustmentGroup::reset`.
3. Implement it in the pipeline (usually `develop.rs` or `color.rs`) with a test.
4. Show it in `apps/desktop/src/panels/adjustments.rs` (most groups render their
   control list automatically).
5. If it changes how existing settings render, add a new `ProcessVersion`
   ([EDIT-MODEL.md](EDIT-MODEL.md)).
6. Update [PIPELINE.md](PIPELINE.md).
