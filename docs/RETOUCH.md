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
| `photonation-develop` | `selection.rs` | `Selection`: a found person as grid cells, and the strokes that cover it |
| `photonation-retouch` | `patch_match.rs` | `ContentAwareFill`: multi-scale PatchMatch, pure Rust; `Refined`: a network's fill refined by it |
| `photonation-retouch` | `onnx.rs` (feature `onnx`) | `OnnxInpainter`: LaMa or MI-GAN through tract |
| `photonation-retouch` | `people.rs` (feature `onnx`) | `PeopleDetector`: an RF-DETR segmentation model through tract |
| `photonation-pipeline` | `retouch.rs` | Cutting a removal's canvas, converting fills to patches, the `Retouch` blend stage, the image people are looked for in (`as_shot_image`) |
| `photonation-engine` | `models.rs` | `ModelKind`, where models are installed, `ModelStatus`, loading on first use |
| `photonation-engine` | `retouch.rs` | `Retoucher`: picks the inpainter, fills in order, caches, finds people; the retouch worker |
| `apps/desktop` | `panels/removals.rs`, `panels/removal_brush.rs` | The Remove panel and painting on the photo |
| `apps/desktop` | `panels/models.rs` | The model card: status, and what to download, shown before anything is |
| `apps/desktop` | `people.rs`, `panels/people_picker.rs` | The people found in the open photo; the People section of the Remove and Masks panels, and picking one with a click |

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

**Installing a model** is covered for both models under
[Installing models](#installing-models). Without an inpainting model (or in
a build without the `ai` feature) generative removals use content-aware
fill; once a model loads, cached fills are dropped and the app renders again
so they are redone with it.

## Selecting people

In the Remove panel, **Select People** outlines everyone an RF-DETR
segmentation model finds in the photo; the person under the pointer is
drawn in the accent colour, and a click removes them (⇧-click adds them to
the selected removal). The Masks panel has the same section: there a click
makes a brush mask of the person ("Person 2"), and ⇧-click adds them to the
selected mask's brush. The model is needed only to find people: the person
becomes ordinary brush strokes, so the removal or mask saves, renders,
exports and edits like a painted one.

```text
desktop: Select People ─▶ RenderService::find_people(path)
engine preview worker (has the photo decoded):
  as_shot_image   the whole default crop, upright, as shot, ≤ 864 px,
                  encoded like a removal's canvas (steps 2, 4 and 5 above)
  ─▶ RetouchJob::FindPeople to the retouch worker (latest wins)
engine retouch worker:
  load the people model if needed (ModelStatus { kind: People, … })
  PeopleDetector::find ─▶ Event::PeopleFound { path, people }
desktop: outlines on the photo; click ─▶ Selection::to_strokes ─▶ Removal,
         or a brush mask's MaskStrokes
```

**The model** (`people.rs`). One `1×3×S×S` input (the photo stretched to a
square, RGB 0..1 normalized with ImageNet's mean and standard deviation),
and per query a box, 91 class logits (sparse COCO ids, 1 = person) and mask
logits over the whole image at `S/4`. Outputs are told apart by shape. A
query is a person when the person class's probability is at least 0.5 (every
real person in the test photos scored 0.84 or more, a signpost 0.40); a
person whose mask mostly repeats a more certain one's is dropped.

| Model | Licence | Input | Output |
|---|---|---|---|
| [RF-DETR](https://github.com/roboflow/rf-detr) Seg Medium | Apache-2.0 | `input` 1×3×432×432 | `dets` 1×200×4, `labels` 1×200×91, `masks` 1×200×108×108 |

**Verifying the model.** `rfdetr-seg-medium.onnx` from
[besit/rfdetr-onnx](https://huggingface.co/besit/rfdetr-onnx) has SHA-256
`33f97a068ea7de3892e4cbc3825e15c5418be818506d7f39f44e1fb1f7ab6abb`, the hash
its page lists. Its weights were compared with Roboflow's own checkpoint
([Roboflow/rf-detr-seg-medium](https://huggingface.co/Roboflow/rf-detr-seg-medium),
`model.safetensors`, SHA-256 `0b1656792d51e2ecfaa49434248c0093bac883f9b75723e04a1ba23f45666d64`):
every weight the model uses when it runs is in the export bit for bit. The
rest of the checkpoint (about 12 % of its values) is the extra query groups
Group DETR trains with and drops for inference. On an M1 Pro, preparing the
image takes about 0.1 s and the search about 1 s on one thread.

**From mask to strokes** (`selection.rs`). The person's mask is sampled
four times as fine (432 cells across) with bilinear interpolation and
thresholded at 0.5, so the outline follows the probabilities instead of the
model's grid steps; a step of the traced outline is then about 14 px on a
24 MP photo. Each connected part becomes one stroke that zigzags
along its rows, two points a row, with a radius of half a cell's diagonal,
so even the corners of a row's end cells are covered. A removal adds a
margin of 0.6 % of the long edge, so no fringe of the person is left for the
fill to copy. A removal keeps at most 32 strokes (the largest parts first);
a part taller than 512 rows continues in another stroke. A mask takes the
outline as painting strokes reaching one cell further, at most 64, that fade
over their whole radius (feather 100) at full flow: a soft edge about as
wide as the model's own mask is accurate, which hides the outline's steps.
(With feather 50 and no reach, each row's end showed as a tooth along
slanted edges.)

## Installing models

Photonation ships no weights and never downloads anything itself. Each tool
that needs a model shows its status; while none is installed, **Get the …
Model** opens a card that says, before anything is downloaded, what the
model does, who made it, a link to its makers' page (*About*), what it
learned from, the file and its size, the licence (with a link and what it
allows), where the file comes from, its SHA-256
(with Copy), and that it runs on this computer only. Only **Download** then
opens the file in the browser; **Open Models Folder** and **Check Again**
finish the job (`panels/models.rs`).

| Model | File | Size | Put it at (macOS) | Or set |
|---|---|---|---|---|
| Inpainting (LaMa) | `lama_fp32.onnx` | 208 MB | `~/Library/Application Support/Photonation/models/inpaint.onnx` | `PHOTONATION_INPAINT_MODEL` |
| People (RF-DETR) | `rfdetr-seg-medium.onnx` | 138 MB | `~/Library/Application Support/Photonation/models/people.onnx` | `PHOTONATION_PEOPLE_MODEL` |

The models folder is `$XDG_DATA_HOME/photonation/models` (default
`~/.local/share/...`) on Linux and `%APPDATA%\Photonation\models` on
Windows. A download can keep its own name: without `inpaint.onnx` or
`people.onnx`, the only ONNX file in the folder of that kind is used
(`models.rs: installed_model`). Files whose names start with `rfdetr` are
people models, any other is an inpainting model. The desktop app loads the
inpainting model in the background at start-up and the people model when
the Remove or Masks panel first opens; while a model is missing, every status request
(Check Again) and use looks again, so no restart is needed. Check a download
before installing it: `shasum -a 256 <file>`. Model licences are listed in
[NOTICE](../NOTICE).

## Caching fills

Fills stay in memory for the 8 most recent photos (`FillCache`) and on disk, so
reopening a photo, even after a restart, doesn't compute them again. Exports from
the desktop app and the CLI use the same files.

**On disk.** One file per fill in `fills` under the data directory: a small header
(magic, sensor rectangle, size) and the samples as they are, `f32` little-endian, so
a fill comes back to the bit. Files are written whole and renamed into place. A file
that is damaged, or larger than a canvas can be, just means computing the fill again.
When the folder grows past 2 GB the least recently used files go, down to 1.5 GB.
These are pixels from the photos, in the same place and with the same care as the
thumbnails.

**The name** is a hash of everything a fill depends on, so nothing stale is found:
the photo's path, size and modification time, the removal chain's fingerprint, the
crate version, and when the chain uses a generative removal, the model's name, size
and modification time. A fill made without a model, which used the fallback, is thus
not used once there is one. The hash holds within a build only, which the crate
version covers: an upgrade starts the cache again, and the old files age out.

## Limits and next steps

- Canvases are capped at 1536 px; removals larger than that in the photo
  are refined at that resolution and scaled up. Tiled refinement at full
  resolution would remove the cap.
- Photonation does not verify model files itself yet; pinning known-good
  hashes needs a SHA-256 implementation (a new dependency).
- People are found on an 864 px image with masks of 108 cells across, so
  someone far away gets a coarse outline; widen it by ⇧-painting.
- For the same reason a mask's edge is soft and doesn't trace hair or fine
  detail; refine it with the brush (Erase, or ⌥-paint).
- Next: dust spot detection, and wire/fence detection.
- `fill_missing` runs removals one after another; independent removals
  (non-overlapping canvases) could run in parallel.
- Note for distributors: the PatchMatch algorithm has been the subject of
  US patents (Adobe). Several open-source projects ship implementations;
  check the current status before distributing binaries commercially.
