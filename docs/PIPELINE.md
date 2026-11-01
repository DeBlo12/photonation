# The develop pipeline

`photonation_pipeline::render(sensor, settings, options)` turns a decoded raw into a
display-ready image. This document follows that call from start to finish.

## 1. Setup (`lib.rs: render`, `build_graph`)

1. **Sanitize** the settings (clamp every control to its range).
2. **Framing** (`Framing`): the part of the sensor to read. Without an angle,
   the camera's recommended crop narrowed by the user's crop, which is given on
   the *displayed* (oriented) image and mapped back to sensor coordinates
   (`Orientation::display_rect_to_stored`); the output is that region. With an
   angle, the bounding box of the turned frame (`crop_frame`, see EDIT-MODEL.md)
   plus a 3-pixel margin; the output is the frame's size. With lens geometry
   corrected (§6b), the frame lies on the corrected photo: its edges, mapped
   through the lens map for every channel, give the region to read.
3. **Target size**: the output scaled so its long edge fits `long_edge`.
4. **Demosaic method** (`Demosaic::choose`):
   - Bayer, output ≤ half size → `HalfSize` (one pixel per 2×2 quad: 4× fewer pixels, no interpolation).
   - Bayer, larger output → `Mhc` (Malvar–He–Cutler).
   - Other CFA such as X-Trans → `NeighbourAverage`.
   - Linear DNG → `Linear` (normalization only).
5. **Camera model** (`camera.rs`): white-balance multipliers and the matrix from
   white-balanced camera RGB to the working space (see §4).
6. **Stage chain**, in order:

| Stage | Kind | Halo (rows) | Present when |
|---|---|---|---|
| `SensorSource` | source | 2 (MHC, neighbour average), 0 otherwise | always |
| `Retouch` | point | 0 | a removal has a fill (`render_with_fills`) |
| `Develop` | point | 0 | always |
| `Denoise` (à trous wavelet) | spatial | 2 + 4 + … + 2^levels | noise reduction on |
| `LocalContrast` | spatial | 3 × box radius | Texture or Clarity ≠ 0 |
| `Straighten` (bicubic) | spatial | the frame's tilt (and lens map) across the band, + 2 | crop angle ≠ 0, or lens geometry corrected |
| `Resize` (Lanczos-3) | spatial | filter support | output smaller than source |
| `Encode` (OETF) | point | 0 | always |
| `Sharpen` (unsharp mask) | spatial | 3σ | sharpening > 0 |

## 2. Band execution (`band.rs`)

`Graph::run` splits the output into bands of output rows and renders them in
parallel with rayon. For a band, the last stage reports which input rows it
needs (`Stage::input_rows`), the stage before it is asked for those rows, and so
on back to the source. Each stage returns a new `Band`, except point stages,
which modify their input in place.

Because every band computes exactly the rows it needs from the same inputs, the
result is bit-identical for any band height (`tests.rs:
band_size_does_not_change_output` checks this for every stage).

The band height targets ~256 source rows so halos stay cheap relative to the
band. Finished bands are quantized straight into the output buffer at their
display position (orientation applied while writing), so no rotation pass is
needed.

## 3. Source: normalize, white balance, demosaic (`source.rs`)

For each sample: `(raw − black[channel]) × multiplier[channel] / (white − black)`.
The multipliers are normalized so the smallest is 1. Process V1 clips every
channel at 1.0, the common clip point, so blown highlights stay neutral
instead of turning magenta, but whatever red and blue recorded above green's
clip is lost. From process V2 each channel is clipped at its own level, its
multiplier, and the develop stage reconstructs the highlights (§3c).

Mosaic-based methods first build a **padded mosaic** for the band: normalized
samples plus a mirrored halo, together with each padded sample's true colour.
Demosaicing then reads a 5×5 neighbourhood per pixel.

## 3b. Removal fills (`retouch.rs`)

`render_with_fills` takes the removals' precomputed fills (`RetouchPatch`:
linear camera RGB before white balance, on a sensor rectangle). The
`Retouch` point stage samples each patch bilinearly, applies the white
balance multipliers and clip exactly as the source does, and blends it in by
the removal's brush weight. Because this happens before the develop stage,
every adjustment applies to the fill as to the rest of the photo. Computing
fills is non-local and happens outside the render; see
[RETOUCH.md](RETOUCH.md). `render` is `render_with_fills` without fills.

## 3c. Highlight reconstruction (`highlights.rs`, process V2)

The first develop step, per pixel, in white-balanced camera RGB, where each
channel `c` clips at its multiplier `clip[c]`:

1. **How clipped** each channel is: `smoothstep(0.9 · clip, clip, value)`. Below
   90 % of its clip a channel is trusted; the soft zone keeps the result
   continuous and absorbs demosaicing next to clipped samples. A pixel
   with no channel in it is left as it is: colours that exceed 1 without
   clipping keep the colour V1 clipped away.
2. **Raise** each clipped channel (by how clipped it is) to the larger of
   - the trusted channels' mean, weighted by trust (with every channel
     clipped, the brightest value), and
   - the highest value among the clipped channels: they are lower bounds
     that can't be told apart, so green clipped at 1 next to blue clipped
     at 1.5 must not read as less green than blue (that turns skies pink).

   A neutral gradient comes out exact up to the highest clip, about one
   stop above V1's.
3. **Hold the colour.** The raised pixel's colour is a guess, and a sky
   that clips one channel after another turns the guesses into bands. So
   its distance from its grey (the channel mean) is scaled down, by how
   clipped its most clipped channel is, to at most the distance V1's
   common clip at 1 leaves. Brightness and hue come from the
   reconstruction, saturation no higher than V1's.

V2 renders a photo with no channel above 90 % of its clip exactly as V1
does. Removal fills (§3b) are clipped the same way as the source, so a
fill in a blown area is reconstructed with it.

## 4. Colour model (`camera.rs`)

DNG-style, using the file's one or two calibration matrices (XYZ → camera):

1. **White point.** *As shot*: the camera's recorded neutral is converted to a
   temperature and tint by iterating "interpolate the matrix at T → neutral to
   XYZ → chromaticity → T" until it settles. *Custom*: temperature and tint give
   a chromaticity (Kim et al. Planckian locus, tint ⟂ locus in CIE 1960 uv), and
   the interpolated matrix gives the camera neutral.
2. **Matrix interpolation** is linear in mired (1/T) between the calibrations.
3. **Camera → working**: undo the multipliers, camera → XYZ, Bradford-adapt the
   shot white to D65, XYZ → linear Rec.2020. Then scale so that a neutral at clip
   maps to (1, 1, 1).

## 5. Develop: the per-pixel stage (`develop.rs`, `tone.rs`, `color.rs`, `masks.rs`)

**Local adjustments.** When any mask is active, the stage first sums the
masks' adjustments at the pixel (`LocalAdjust::at`): the pixel maps to a point
in mask space through an affine map that covers orientation and the
half-size path (`PixelToMask`). At that point each mask's shapes are combined
(`MaskWeight::at`, see docs/EDIT-MODEL.md), and the resulting weight times the
mask's amount scales its adjustments. The sums join the global steps below,
marked *local*. Without active masks the stage runs a separately compiled copy
with every local step removed, so unmasked renders cost what they did before
masks existed.

Each pixel, in this order:

0. **Highlight reconstruction** (process V2, §3c), then the **vignetting**
   gain (§6b) on the camera RGB.
1. **Camera → working** matrix, with exposure folded in:
   `2^(exposure + BASELINE_EXPOSURE + DNG BaselineExposure)`.
   *Local* exposure, temperature and tint then scale the channels (warmer:
   more red and less blue; magenta: less green; a neutral keeps its luminance).
2. **Dehaze**: removes a neutral veil smoothly (`x² / (x + h)`), or adds one
   for negative values. *Local* dehaze adds to the global amount.
3. **Highlights / Shadows**: an exposure shift weighted by how many stops the
   pixel's luminance sits above or below mid-grey. *Local* amounts add to the
   global ones. *Local* contrast then steepens or flattens luminance around
   mid-grey in log space.
4. **Tone**: one lookup table per channel (sampled over log2 of scene value)
   holding the composition of
   - the base curve: black point (Blacks) → extended Reinhard shoulder that
     reaches white at the Whites-adjusted white point → contrast S-curve
     around mid-grey in a gamma-2.2 domain;
   - the parametric tone curve (four regions with movable splits);
   - the RGB point curve and the per-channel point curves (monotone cubic splines).
5. **Colour**, in Oklab (only when any colour control is set):
   - colour mixer: hue → blend of the two nearest of eight bands; hue shift,
     chroma scale and lightness change (lightness only for coloured pixels);
   - colour grading: shadows/midtones/highlights weights from Oklab L, with the
     pivot moved by Balance and the transition width set by Blending, plus global;
   - vibrance (boosts muted colours more) and saturation;
   - point colours (`point_color.rs`): each active one selects colours by
     their distance to its target in lightness, chroma and hue (hue as the
     shorter way round the circle). Each distance has a reach set by its range
     slider (`HUE_REACH`, `CHROMA_REACH`, `LIGHTNESS_REACH`); the selection is
     full up to 35 % of the reach and fades smoothly to none at it, and the
     three are multiplied. Below chroma 0.02 a hue is noise: grey pixels drop
     out of colour targets, and a grey target ignores hue. Selections are
     measured on the colour before any point colour applies, and the shifts
     (hue up to ±60°, chroma × (1 + saturation), lightness up to ±0.3) add
     up, so their order doesn't matter.
     With `RenderOptions::point_color_overlay` the stage instead multiplies
     chroma by one point colour's selection: what it selects keeps its
     colour, the rest turns grey (the editor's "Visualize range").

   *Local* saturation (plus a little for local dehaze, like the global slider)
   then scales Oklab chroma.
6. **Output primaries**: Rec.2020 → output space (with Bradford when the output
   white differs, e.g. ProPhoto's D50). Out-of-gamut colours are pulled toward
   their luminance instead of clipping channels independently, which preserves
   hue.

## 5a. Colour picker (`lib.rs: color_at`)

The Point Color eyedropper measures what the point colour step sees: the
settings without their point colours, a ≈8 × 8 sensor-pixel patch around the
click (`pick_patch`, shared with the white balance eyedropper) run through the
source and the develop stage including masks (`Develop::working_at`), averaged
in the working space and converted to Oklab LCh. Texture, Clarity and
sharpening don't take part.

## 6. Spatial stages

- **Noise reduction** (`denoise.rs`): each pixel goes to a square-root
  domain, where noise is roughly even from shadows to highlights, as
  luminance `(r + 2g + b) / 4` and the differences `b − g`, `r − g`. An à
  trous wavelet (the B3 spline `[1 4 6 4 1] / 16` with `2^level − 1` holes
  between taps, rows then columns, edges clamped) splits each into detail
  at five levels and a coarse rest. Each level's detail shrinks by
  `d · d² / (d² + t²)`: barely below the threshold `t`, hardly above it.
  Luminance thresholds follow the grain expected at each level
  (`LUMINANCE_LEVELS`) times Luminance; Detail puts back up to 60 % of what
  smoothing removed. Colour thresholds stay strong at coarse levels
  (`COLOR_NOISE`), since demosaicing leaves colour noise blotchy. After
  half-size demosaicing a pixel averages four, so the finest level is skipped
  and luminance thresholds halve: previews remove what exports remove. Each
  pixel's sums run in a fixed order, so the result doesn't depend on the band
  height.
- **Local contrast** (`local_contrast.rs`): detail = √luma − blur(√luma), added
  back at a large radius (Clarity, midtone-weighted) and a small one (Texture).
  The blur is three running-sum box passes (≈ Gaussian, O(1) per pixel). The
  sums run in fixed point (2⁻³⁰ steps), where adding and removing samples is
  exact; a floating-point running sum would round differently depending on
  where a band starts, which breaks band-size invariance. Radii are fractions
  of the image's long edge, so previews and exports match.
- **Straighten** (`straighten.rs`): resamples along the turned frame with a
  Catmull–Rom kernel, at the source's scale (half size after half-size
  demosaicing). Output pixel → output displayed → photo displayed → photo stored
  → sensor → input is affine, so it is precomputed as an origin and two steps;
  the lens map (§6b) then moves that position, per channel when chromatic
  aberration is corrected (three bicubic samples instead of one). A band reads
  the input rows its corners reach (with the lens map, its edge pixels: the
  map is radial and monotonic), which overlap the next band's by the tilt;
  resampled renders therefore use one band per thread. Masks and local
  contrast run before it, on the upright sensor region, so they need no change.

## 6b. Lens corrections (`lens.rs`, `photonation-lens`)

Profiles come from the Lensfun database (CC BY-SA 3.0), embedded unchanged in
`crates/lens/lensfun/db` and parsed on first use (~40 ms). `photonation-lens`
finds the camera by name and the lens as Lensfun's `FindLenses` does: every
lens scores by crop factor, mount, focal range and aperture read from the
names, and a fuzzy comparison of the names' words (every word of the camera's
lens name must be in the database's). A lens may have entries calibrated on
several sensor sizes; each correction uses the closest one no smaller than
the camera's. Distortion and chromatic aberration are interpolated over the
focal length (Lensfun's Hermite spline, terms scaled by the focal length
where they follow 1/f), vignetting by inverse distance weighting over focal
length, 1/aperture and 1/distance (distance unknown: far away). Hugin's
models (ptlens, poly3, poly5; linear and poly3 TCA; pa vignetting) are then
rewritten as polynomials in one radius, 1 at the photo's corners; distortion
keeps the centre's scale, as Lensfun does.

The settings scale the profile (Distortion and Vignetting, 0..200 %) and add
manual corrections: Distortion multiplies the radius factor by
`1 − 0.15 · amount · ρ²`, Vignetting brightens by `2 · amount · r³` stops.
The pipeline turns that into:

- **`LensMap`**: a corrected point at radius ρ (after a zoom) lies at
  `ρ · F(ρ)` on the photo as shot, red and blue further scaled by their TCA
  factors. The zoom is the largest that keeps every channel of the corrected
  photo's edges on the photo (checked from the edges' middles to the
  corners), so the corrected photo fills its frame. The crop is on the
  corrected photo; masks and removals stay on the photo as shot, so the
  desktop maps them through the lens map (`from_shot` inverts it by
  bisection).
- **`Vignette`**: the gain `2^(manual · r³) / (1 + k₁r² + k₂r⁴ + k₃r⁶)^amount`
  at the source pixel's radius on the photo as shot, in `Develop` after
  highlight reconstruction.
- **Resize** (`resize.rs`): separable Lanczos-3 in linear light, widened when
  shrinking to avoid aliasing. Weights are precomputed per output row and column.
- **Encode** (`encode.rs`): the output space's transfer curve from a 65 536-entry table.
- **Sharpen** (`sharpen.rs`): unsharp mask on the luma of encoded values, applied
  equally to R, G and B (no colour fringes), at output resolution.

## 7. Performance (Apple M1 Pro, 24 MP Canon CR3)

| Operation | Time |
|---|---|
| Decode (rawler) | ~130 ms |
| Full-size render (MHC) | ~170 ms |
| Full-size render, straightened 3° (5205 × 3470) | ~250 ms |
| 1600 px render (half-size path) | ~35 ms |
| Full-size JPEG encode | ~330 ms |

Masks cost time at every pixel they are evaluated for: two masks covering the
photo roughly double the full-size render (≈350 ms; ≈100 ms at 1600 px), about
a quarter of it the Oklab round trip of local saturation. Renders without
active masks run at the speed above.

The colour step's Oklab round trip runs only when a colour control is set; it
takes a full-size render to ≈600 ms. Point colours ride on the same round
trip: one point colour on its own costs about what one mixer slider does.

Peak memory for a full-size 8-bit export ≈ mosaic (50 MB) + output (72 MB) + a
few MB of band scratch per thread.

## 8. GPU path (`photonation-gpu`)

`GpuRenderer::render` produces the same 8-bit image as `render_with_fills`
with wgpu compute shaders. It doesn't decide anything itself:
`render_plan` (`plan.rs`) runs the setup of §1 with the same code as the
band graph and returns every parameter and table as plain data (framing,
demosaic method, levels, the removals' brush strokes and fills with the
pixel → mask and pixel → sensor maps, the develop matrices and amounts,
tone tables, colour step, the masks' prepared shapes and amounts with the
pixel → mask space map, Texture and Clarity's amounts and radii, the
straightened frame's affine map and lens map, the vignetting gain's
terms, Lanczos taps, transfer table, sharpening
kernel). The shaders
transcribe only the per-pixel maths.

| Pass | Shader | Present when |
|---|---|---|
| Source + removal fills + highlight reconstruction + vignetting + develop (with masks) + output gamut | `develop.wgsl` | always |
| Noise reduction: to opponent, per level rows then columns + shrunk detail, combine | `denoise.wgsl` | noise reduction on |
| Texture and Clarity: √luma, box passes, combine | `local_contrast.wgsl` | Texture or Clarity ≠ 0 |
| Straighten (Catmull–Rom along the frame, through the lens map) | `straighten.wgsl` | crop angle ≠ 0, or lens geometry corrected |
| Resize, rows then columns | `resize.wgsl` | output smaller than source |
| Encode | `finish.wgsl: encode` | always |
| Sharpen, rows then columns | `finish.wgsl: sharpen_*` | sharpening > 0 |
| Pack to RGB8, display orientation | `finish.wgsl: pack` | always |

The raw mosaic is uploaded once per photo as a 16-bit texture and kept while
the photo is edited; intermediates are `rgba32float` textures, so values
match the CPU's `f32` bands. Only the packed output is read back.

Masks are evaluated per pixel in the develop pass exactly as on the CPU
(`LocalAdjust::at`, then the masked steps of §5); they cost no measurable
time there.

Removal fills are blended in the same pass, after demosaicing, as the
`Retouch` stage does (§3b): each removal's brush weight from its strokes'
distances, its fill sampled bilinearly, white balanced and clipped. The
fills are uploaded once, as one storage buffer, and kept while the same
fills are rendered (a slider drag uploads none); a new fill replaces them.

Straightening resamples the whole developed image at once: each output
pixel maps to an input position through the frame's affine map (origin
and two steps, from `Straighten`) and the lens map, and the 4 × 4
Catmull–Rom taps around it are read with edges clamped, as the CPU's bands
are.

Texture and Clarity blur the whole image at once: each box pass loads a
workgroup's 256 samples plus the radius on each side into shared memory
and sums the window, with edges clamped as on the CPU (whose bands carry
exactly the halo the passes need, so its result is the same). Radii up to
192 pixels fit, Clarity on a 24 000-pixel long edge; larger ones render
on the CPU.

**Not on the GPU**: 16-bit output (exports, which stay on the CPU), images
larger than the device's largest texture, and fills larger than its
largest storage buffer. The renderer returns `Unsupported` for those, and
callers render them on the CPU.

**Validation.** The CPU pipeline is the reference. The crate's tests render
the same synthetic photos on both paths (Bayer at full size and half size,
X-Trans, linear DNG, clipped highlights under processes 1 and 2,
straightened crops at several angles with masks, Texture and Clarity,
overlapping removals, every orientation, crops, custom white balance, every
global adjustment, masks of every shape, combination and local adjustment,
groups and piled-up masks, Texture and Clarity in both directions, noise
reduction at full and half size, other
output spaces, the point colour overlay)
and allow
at most 2 levels of difference per 8-bit sample and 0.01 on average (in
practice the outputs are bit-identical but for a few samples off by one).
Without a graphics adapter the renders are skipped; the shaders are still
validated with naga.

24 MP Bayer mosaic with every global adjustment set, without and with three
overlapping masks, and with Texture and Clarity on top (RTX 5080, Windows):

| Render | CPU | GPU (incl. readback) |
|---|---|---|
| Full size | ~1470 ms | ~53 ms |
| 1600 px | ~330 ms | ~10 ms |
| Full size, 3 masks | ~1730 ms | ~53 ms |
| 1600 px, 3 masks | ~390 ms | ~10 ms |
| Full size, 3 masks, Texture, Clarity | ~3300 ms | ~81 ms |
| 1600 px, 3 masks, Texture, Clarity | ~620 ms | ~16 ms |
| Full size, straightened 3° | ~1900 ms | ~52 ms |
| 1600 px, straightened 3° | ~470 ms | ~10 ms |
| Full size, 2 removals | ~1280 ms | ~55 ms |
| 1600 px, 2 removals | ~330 ms | ~11 ms |
| Full size, noise reduction | ~1760 ms | ~108 ms |
| 1600 px, noise reduction | ~370 ms | ~24 ms |
| Full size, lens corrections | ~1150 ms | ~59 ms |
| 1600 px, lens corrections | ~300 ms | ~15 ms |

`cargo test --release -p photonation-gpu -- --ignored --nocapture` prints
these timings.
