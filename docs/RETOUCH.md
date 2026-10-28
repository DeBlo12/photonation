# Removing objects

Paint over something that should not be in the photo (a fly in front of a
cow, a passer-by, a fence post, a sensor speck) and Photonation replaces it
with a plausible fill. Like every other edit this is non-destructive: the
sidecar stores only *where* to remove and *how*. The fill is computed from
the raw file when needed and cached in memory.

## The pieces

| Crate | File | Role |
|---|---|---|
| `photonation-develop` | `removals.rs` | The edit model: `Removal`, `BrushStroke`, `RemovalMethod`, caps, fingerprints |
| `photonation-retouch` | `patch_match.rs` | `ContentAwareFill`: multi-scale PatchMatch, pure Rust; `Refined`: a network's fill refined by it |
| `photonation-retouch` | `onnx.rs` (feature `onnx`) | `OnnxInpainter`: LaMa or MI-GAN through tract |
| `photonation-pipeline` | `retouch.rs` | Cutting a removal's canvas, converting fills to patches, the `Retouch` blend stage |
| `photonation-engine` | `retouch.rs` | `Retoucher`: picks the inpainter, fills in order, caches; the retouch worker |
| `apps/desktop` | `panels/removals.rs`, `panels/removal_brush.rs` | The Remove panel and painting on the photo |

## Edit model

`settings.removals` is a list of at most `MAX_REMOVALS` (64). Each `Removal`:

| Field | Meaning |
|---|---|
| `enabled` | Off: kept but not rendered |
| `method` | `generative` (neural model) or `content-aware` (PatchMatch) |
| `strokes` | Up to 32 `BrushStroke`s: polylines (≤ 1024 points) in mask space, with a `radius` in long-edge units (0.001..0.25) |
| `feather` | Soft edge, as a share of the radius (0..100) |
| `seed` | Varies content-aware fills ("Try Another Fill") |

Positions use the masks' coordinate frame (`MaskSpace`, see
[EDIT-MODEL.md](EDIT-MODEL.md)), so a removal stays on its content when the
crop changes. The area removed is the union of the strokes; the weight is 1
within `radius × (1 − feather)` of a stroke and fades to 0 at the radius.

**Order matters.** Removals apply in order, and each one's fill sees the
photo with the earlier removals already applied. A fill is therefore keyed
by its *chain fingerprint*: a hash of the removal and every active one before
it (`removal_chain_fingerprints`). Changing an early removal refills the later
ones; changing a slider refills nothing.

## Control flow

```text
desktop: stroke released ─▶ settings.removals changes ─▶ preview request
engine preview worker:
  render with the fills already cached (missing removals show the original)
  any missing? ─▶ RetouchJob::Fill to the retouch worker
engine retouch worker (latest request wins):
  for each active removal without a cached fill, in order:
    removal_canvas   context around the brush, as-shot white balance, sRGB,
                     earlier fills applied, gain to fill the range, gamma 2.2
    Inpainter        generative model refined by content-aware fill, or
                     content-aware fill alone
    RemovalCanvas::patch   back to linear camera RGB before white balance
    cache it, send Event::RemovalsFilled ─▶ desktop asks for a new preview
render (preview, thumbnail, export):
  sensor ─▶ normalize + WB + demosaic ─▶ Retouch (blend patches) ─▶ develop …
```

Thumbnails and exports fill missing removals synchronously on their own
threads; previews never wait.

### The canvas (`removal_canvas`)

1. The removal's bounds plus a margin as large as its larger side, clipped
   to the photo, mapped to a sensor rectangle.
2. That area demosaiced, scaled so its long edge is at most 1536 px
   (`MAX_CANVAS_EDGE`). Demosaicing uses the as-shot white balance, like a
   render does; demosaicing does not commute with per-channel scaling, so
   balancing afterwards would not match the photo.
3. Earlier removals' fills blended in.
4. Converted to linear sRGB with the as-shot camera matrix. Networks were
   trained on ordinary photos; fed camera colours, LaMa painted purple and
   yellow blotches.
5. A gain that puts the context's 99th-percentile highlight at 0.95, then
   gamma 2.2: a natural-looking image for the inpainter.
6. The hole is every pixel with removal weight above 0, including the
   feathered edge. (Counting only the solid core leaves an object's soft
   outline outside the hole, and networks spread it back in.)

`RemovalCanvas::patch` inverts steps 2, 4 and 5 exactly, so a canvas returned
unchanged reproduces the photo (`tests.rs: an_unchanged_canvas_round_trips…`).
Patches are stored *before* white balance and blended *before* every
adjustment, so white balance, exposure, curves and masks apply to the fill
exactly as to the rest of the photo.

### Content-aware fill (`patch_match.rs`)

Multi-scale PatchMatch inpainting (Barnes et al. 2009; Wexler et al. 2007):

1. Halve the canvas until every hole pixel is within a patch radius of
   known pixels (7×7 patches), but not below 64 px on the short side, so
   the coarsest level still has plenty of source patches.
2. Coarsest level: fill the hole from its rim inwards with neighbour means.
   This smooth guess counts only 0.2× in patch distances there; at full
   weight it attracts flat areas and the fill stays flat.
3. For each level, coarse to fine, start from the upsampled fill and
   nearest-neighbour field, then repeat expectation–maximization rounds:
   PatchMatch (propagation in alternating scan order, random search at
   halving radii) finds for every patch touching the hole the most similar
   patch entirely outside it; voting sets each hole pixel to the weighted
   mean of what the overlapping matches put there (weights fall off with
   patch distance, scaled by the lower quartile). The last round of each
   level instead takes the single best patch, which keeps grainy texture
   such as grass from averaging into grey.

It is deterministic for a seed, needs no model, and works best on texture:
grass, sky, water, walls, foliage. It cannot invent structure that appears
nowhere else in the photo.

### Generative fill (`onnx.rs`)

A neural network invents content that fits the scene, which handles fences,
people in front of structure and other large or structured areas better.
Photonation runs ONNX models with [tract](https://github.com/sonos/tract)
(pure Rust, CPU, MIT OR Apache-2.0) and recognises two input layouts:

| Model | Licence | Inputs | Output |
|---|---|---|---|
| [LaMa](https://github.com/advimman/lama) | Apache-2.0 | `image` 1×3×H×W (0..1), `mask` 1×1×H×W (1 = hole) | 1×3×H×W, 0..1 or 0..255 |
| [MI-GAN](https://github.com/Picsart-AI-Research/MI-GAN) | MIT | one 1×4×H×W tensor: `known − 0.5`, RGB (−1..1) × `known` | 1×3×H×W, −1..1 |

Fixed-size models (512×512 is typical) get the canvas resized to their size
and the fill resized back; only hole pixels change. The hole is blanked
(zero) in the image input, so the network never sees what it removes.

**Refinement** (`Refined`). A 512 px fill of a large area is soft once scaled
up. So the engine passes every network fill to
`ContentAwareFill::refine`, which runs PatchMatch on the full canvas with a
three-level pyramid whose coarsest level starts from the network's fill at
full weight. The network decides *what* goes there (more grass, the rest of
the rock); PatchMatch rebuilds it from the photo's own texture at up to
1536 px. Exporting a 24 MP photo with three such removals takes about 15 s on the CPU (about 2.5 s with content-aware fill alone).

**Verifying a model.** `lama_fp32.onnx` from
[Carve/LaMa-ONNX](https://huggingface.co/Carve/LaMa-ONNX) has SHA-256
`1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6`, the
same hash an Internet Archive snapshot of the page from November 2025
lists (before Hugging Face's July 2026 breach). Run on the repository's own
sample image, tract's output matches onnxruntime's (mean difference 0.5 of
255) and the published result (2.1). Check the hash before installing:
`shasum -a 256 inpaint.onnx`.

**Installing a model.** Photonation ships no weights. Put an ONNX export at

- macOS: `~/Library/Application Support/Photonation/models/inpaint.onnx`
- Linux: `$XDG_DATA_HOME/photonation/models/inpaint.onnx` (default `~/.local/share/...`)
- Windows: `%APPDATA%\Photonation\models\inpaint.onnx`

or point `PHOTONATION_INPAINT_MODEL` at the file. The desktop app loads it in
the background at start-up and shows its status in the Remove panel. Without
a model (or in a build without the `ai` feature) generative removals use
content-aware fill; once a model loads, cached fills are dropped so they are
redone with it. Model licences are listed in [NOTICE](../NOTICE).

## Limits and next steps

- Fills live in memory (8 photos); reopening a photo recomputes them.
  A disk cache keyed by file and chain fingerprint would make that instant.
- Canvases are capped at 1536 px; removals larger than that in the photo
  are refined at that resolution and scaled up. Tiled refinement at full
  resolution would remove the cap.
- Photonation does not verify model files itself yet; pinning known-good
  hashes needs a SHA-256 implementation (a new dependency).
- Removals are painted by hand. Next: one-click selection of people (a
  segmentation model), dust spot detection, and wire/fence detection.
- `fill_missing` runs removals one after another; independent removals
  (non-overlapping canvases) could run in parallel.
- Note for distributors: the PatchMatch algorithm has been the subject of
  US patents (Adobe). Several open-source projects ship implementations;
  check the current status before distributing binaries commercially.
