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
