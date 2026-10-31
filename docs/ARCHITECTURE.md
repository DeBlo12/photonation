# Architecture

Photonation is a Cargo workspace of small crates arranged in layers. A crate may
depend only on crates in lower layers; nothing below the apps knows about a UI
toolkit, so the same engine can serve the desktop app, the CLI and (later) a
WebAssembly build.

## Layers

| Layer | Crate | Role |
|---|---|---|
| L0 | `photonation-color` | Matrices, colour spaces, Bradford adaptation, transfer curves, temperature/tint, Oklab |
| L0 | `photonation-image` | Shared data types: `SensorImage` (decoded raw), `OutputImage` (rendered), geometry, EXIF orientation, crop frames |
| L1 | `photonation-develop` | The edit model: `Settings`, control specs, adjustment groups, sidecar files, Lightroom XMP import |
| L1 | `photonation-raw` | Raw decoding (adapter over rawler) → `SensorImage` |
| L1 | `photonation-export` | JPEG/PNG/TIFF encoders with ICC profiles (moxcms) and EXIF/XMP metadata |
| L1 | `photonation-retouch` | Inpainting for removals: content-aware fill (PatchMatch) and, with feature `onnx`, neural models via tract. See [RETOUCH.md](RETOUCH.md) |
| L2 | `photonation-pipeline` | The develop pipeline: `SensorImage` + `Settings` → `OutputImage`; `render_plan` describes a render as data |
| L3 | `photonation-gpu` | The pipeline as wgpu compute shaders, run from a `RenderPlan` and checked against the CPU |
| L4 | `photonation-engine` | Session services: background preview/thumbnail/export workers, history, histogram |
| L5 | `apps/desktop` | The egui editor (`photonation-desktop` binary) |
| L5 | `apps/cli` | Batch export and inspection (`photonation` binary) |

```text
apps/desktop ──┐        apps/cli
               ▼           │
        photonation-engine ◀┘
          │      │      │   ╲
          │      │      │    ▼
          │      │      │   gpu        (previews; feature `gpu`)
          ▼      ▼      ▼  ╱
   pipeline   raw   export
          │      │      │
          ▼      ▼      ▼
       develop ─▶ image ─▶ color
          ▲
   pipeline ─▶ retouch      (inpainting; no photo types)
```

## Data model

- **`SensorImage`** — the decoded raw, kept as compact `u16` samples with its
  layout (Bayer/other CFA pattern, or linear RGB), black/white levels, camera crop,
  orientation, colour calibration (as-shot neutral, one or two XYZ→camera
  matrices) and DNG baseline exposure.
- **`Settings`** — every adjustment of one photo; serialized as the sidecar. See
  [EDIT-MODEL.md](EDIT-MODEL.md).
- **`OutputImage`** — display-referred, encoded, interleaved RGB (8 or 16 bit) in
  an output colour space, already oriented for display.

## Key decisions

- **Pure Rust, GPL-3.0-or-later.** Photonation is copyleft so improvements stay
  open. Dependencies are permissive or LGPL (all GPL-compatible, enforced by
  `cargo deny` in CI). The raw decoder, rawler, is only touched by
  `photonation-raw`, so swapping or adding a decoder changes one crate.
- **Scene-referred pipeline.** Adjustments that model light (white balance,
  exposure, highlights/shadows, dehaze) work on linear Rec.2020 values; the tone
  curve then maps to display, and colour work happens in Oklab.
- **Band-parallel rendering.** The image is rendered in horizontal bands, pulled
  through the stage chain with exactly the halo each stage needs. No full-size
  intermediate exists, so a 24 MP render peaks at roughly the raw mosaic (≈50 MB)
  plus the output buffer. See [PIPELINE.md](PIPELINE.md).
- **CPU reference first.** The CPU pipeline is the specification. The GPU
  (wgpu) path renders from a plan the CPU code builds and must match it;
  tests compare the two (see [PIPELINE.md](PIPELINE.md) §8). Exports stay
  on the CPU.
- **Process versions.** Each sidecar records the rendering process it was made
  with, so improving the look never changes existing edits.
- **Non-destructive, file-based.** Originals are read-only; sidecars sit beside
  them, written atomically.
- **One UI codebase.** egui runs natively and in the browser, which keeps the
  path to a web app short (see [ROADMAP.md](ROADMAP.md)).
