# The edit model

Crate: `photonation-develop`.

## Settings

`Settings` holds every adjustment of one photo. Values use Lightroom's familiar
scales: −100..100 for most sliders, stops for exposure, kelvin and tint for white
balance.

| Group (`AdjustmentGroup`) | Fields |
|---|---|
| White Balance | `white_balance`: `AsShot` or `Custom { temperature, tint }` (presets in `WhiteBalance::PRESETS`) |
| Light | `exposure`, `contrast`, `highlights`, `shadows`, `whites`, `blacks` |
| Presence | `texture`, `clarity`, `dehaze` |
| Color | `vibrance`, `saturation` |
| Curves | `tone_curve`: `parametric` (highlights, lights, darks, shadows + three splits), point curves `master`, `red`, `green`, `blue` |
| Color Mixer | `color_mixer`: hue/saturation/luminance for red, orange, yellow, green, aqua, blue, purple, magenta; `point_colors` (see below) |
| Color Grading | `color_grading`: `shadows`, `midtones`, `highlights`, `global` wheels (hue, saturation, luminance), `blending`, `balance` |
| Detail | `sharpening`: amount, radius |
| — | `crop` and `crop_angle` (see below), `masks` and `mask_groups` (see below), `removals` (see [RETOUCH.md](RETOUCH.md)), `process` |

`Settings::sanitized()` clamps everything into range and repairs bad input; the
pipeline always renders sanitized settings.

## Crop and straighten

A crop is a frame on the photo: `crop` holds the normalized positions of its
top-left and bottom-right **corners** on the displayed (oriented) image, inside
the camera's default crop, and `crop_angle` turns the frame clockwise on the
photo by −45..45 degrees. The render shows what is inside the frame, upright, so
the photo appears turned the other way. Without an angle the corners are simply
the crop rectangle; `crop: None` is the whole photo, or, with an angle, the
largest frame of the photo's aspect that fits. This is how Lightroom stores a
crop, which makes imports exact.

`photonation_image::CropFrame` does the geometry. Rotation needs equal units on
both axes, so it works in photo pixels: from the corners and angle it derives
the frame's centre and its size along its own axes (the size of a full
resolution render), maps points between the frame and the photo, and keeps a
frame on the photo (`constrained`: shrink around the centre, then slide back).
`photonation_pipeline::crop_frame` turns settings into the frame every render,
the viewer and the crop tool use.

Sanitizing clamps the angle and the corner coordinates to 0..1. It reorders an
unrotated crop's edges, but not a turned frame's corners: the bottom-right
corner of a tall frame turned clockwise can lie left of its top-left one.

## Lightroom import (`lightroom.rs`)

A photo opens with `load_photo_settings`: its Photonation sidecar if there is
one, else the develop settings in Lightroom's XMP sidecar (`IMG_0001.xmp`), else
none. The XMP is only read, never written; once the photo is edited here, the
Photonation sidecar takes over.

| Lightroom (`crs:`) | Photonation |
|---|---|
| `WhiteBalance`, `Temperature`, `Tint` | `white_balance` (custom unless "As Shot") |
| `Exposure2012` … `Blacks2012`, `Texture`, `Clarity2012`, `Dehaze`, `Vibrance`, `Saturation` | the same sliders |
| `Parametric*`, `ToneCurvePV2012[Red/Green/Blue]` (points on 0..255) | `tone_curve` |
| `Hue/Saturation/LuminanceAdjustment<Band>` | `color_mixer` |
| `SplitToning{Shadow,Highlight}{Hue,Saturation}`, `ColorGrade*`, `SplitToningBalance` | `color_grading` |
| `Sharpness`, `SharpenRadius` | `sharpening` |
| `HasCrop`, `CropLeft/Top/Right/Bottom`, `CropAngle` | `crop`, `crop_angle` |

Lightroom stores the crop on the *stored* image; `CropFrame::oriented` turns it
to the displayed one using the XMP's `tiff:Orientation` (a mirroring orientation
reverses the angle), with `tiff:ImageWidth/ImageLength` for the aspect ratio.
Checked against Lightroom's own exports: the frame sizes match to the pixel.

Only the top-level properties of each `rdf:Description` are read, so a mask's
own values (also `crs:` names) never leak into the global settings. Edits
without an equivalent are listed in `not_imported` (masks, spot removal, noise
reduction, lens corrections, upright, effects, black & white) and the editor
names them when the photo opens. Values go through `Settings::sanitized`; DTDs
are refused (no entity expansion) and the file is capped at
`MAX_SIDECAR_BYTES`. Sliders share Lightroom's scales, but the rendering differs,
so imported photos look similar rather than identical.

## Point colours (`point_colors.rs`)

`settings.point_colors` holds colours picked in the photo, at most
`MAX_POINT_COLORS` (8), each edited on its own (Lightroom's Point Color). A
`PointColor`:

| Field | Meaning |
|---|---|
| `target` | The picked colour as `OklabLch`: `lightness` 0..1, `chroma` 0..`MAX_SAMPLED_CHROMA`, `hue` in degrees |
| `hue`, `saturation`, `luminance` | Shifts, −100..100 |
| `hue_range`, `saturation_range`, `luminance_range` | How far a colour may sit from the target and still be affected, 0..100 (default 50) |

The sliders are `POINT_COLOR_CONTROLS`. A point colour is *active* when any
shift is non-zero. Sanitizing caps the count, clamps the sliders and the target,
and repairs non-finite numbers. The Color Mixer group's reset also clears the
point colours. How colours are selected and shifted: docs/PIPELINE.md §5.

## Masks (`masks.rs`)

`settings.masks` is a list of mask layers, at most `MAX_MASKS` (32). Each `Mask`:

| Field | Meaning |
|---|---|
| `name` | Up to `MAX_MASK_NAME_CHARS` (64) characters |
| `enabled` | Off: kept but not rendered |
| `components` | 1 to `MAX_MASK_COMPONENTS` (8) shapes, each a `MaskComponent { shape, mode }` |
| `invert` | Apply the adjustments outside the selection instead |
| `amount` | 0..100 %, scales all of the mask's adjustments |
| `adjustments` | `LocalAdjustments`: temperature, tint, exposure, contrast, highlights, shadows, dehaze, saturation, on the global sliders' scales (`LOCAL_CONTROLS`) |
| `group` | Index into `settings.mask_groups`, or absent for a mask outside any group |

**Shapes** (`MaskShape`): a `Linear` gradient (full effect up to the line through
`start`, fading to none at the parallel line through `end`) or a `Radial` one
(an ellipse with `center`, `radius` per axis, clockwise `rotation` in degrees,
and `feather`, the share of the radius that fades out).

**Combining.** A mask's selection at a point starts as its first shape's
weight (0..1); each further shape then combines with it by its `CombineMode`:

| Mode | Selection after the shape (`s` so far, `w` the shape's weight) |
|---|---|
| Add | `max(s, w)` |
| Subtract | `min(s, 1 − w)` |
| Intersect | `min(s, w)` |

`invert` then turns `s` into `1 − s`. These are the standard fuzzy-set rules: a
shape combined with itself changes nothing, and hard-edged shapes behave like
plain set union, difference and intersection.

**Layers add up.** At every pixel, each active mask contributes
`selection × amount × adjustments`, and the contributions are summed, so the
order of the masks only matters for display. A mask is active when it is
enabled, has shapes and an amount above 0, and adjusts something.

**Groups** (`mask_groups.rs`). `settings.mask_groups` lists folders of masks,
at most `MAX_MASK_GROUPS` (16). Each `MaskGroup` has a `name` (up to 64
characters), an `enabled` switch and an `amount` (0..100 %) that apply to every
mask in it: a grouped mask renders only when both it and its group are enabled,
at its own amount times the group's (`Mask::rendered`, `Settings::rendered_masks`).
Everything that shows or renders masks goes through that: the pipeline's
`LocalAdjust`, the editor's overlay, thumbnails and pins. Rendering is otherwise
unchanged: grouped masks add up with all the others.

The masks stay one flat list, so a mask's index means the same with or without
groups. Two rules make that list read as a tree:

- A group's masks sit next to each other; the group is shown where they are.
- A group without masks is removed, so every group has a place in the list.

`tidy_groups` restores both rules (ungrouping masks that name a missing group,
moving each group's masks together where its first mask is, dropping empty
groups and renumbering); `remove_group` removes one group and keeps its masks.
The editor's edits keep the rules as they go (`panels/masks/groups.rs`).

**No new process version.** An ungrouped mask renders exactly as before groups
existed, and so does a grouped one in an enabled group at 100 % (its amount is
multiplied by exactly 1). Older sidecars have no groups, so they render as they
did; a grouped sidecar opened by an older build loses only its groups (it
ignores the unknown fields and renders every mask as if ungrouped).

**Mask space** (`MaskSpace`). Positions are normalized to the photo: 0..1 across
its width and height, in display orientation, on the camera's default crop but
*before* the user's crop, so a mask stays on the same content when the crop
changes. Radii are in units of the long edge, so circles stay round on any
aspect ratio.

**Sanitizing** caps the number of masks, shapes and groups, truncates names,
clamps every value (group amounts too) and repairs non-finite numbers, then
tidies the groups (`sanitize_groups`). The caps also bound the per-pixel cost a
hostile sidecar can cause.

## Removals (`removals.rs`)

`settings.removals` lists brushed areas to replace with a fill, applied in
order before any adjustment: at most `MAX_REMOVALS` (64), each with up to 32
strokes of up to 1024 points, in mask space. The fill itself is not stored;
it is recomputed from the raw file. Sanitizing caps the counts, drops
non-finite points and clamps radii and feather. Details in
[RETOUCH.md](RETOUCH.md).

## Controls

Every slider is described once by a `Control` (id, label, range, default, field
accessor) in `controls.rs`, grouped by panel (`BASIC_CONTROLS`, `MIXER_CONTROLS`,
…). The desktop sliders, the CLI overrides, clamping and the tests all use these
specs, so a range or default changes in one place.

`AdjustmentGroup` resets a group, reports whether it is modified, and computes
the settings with some groups switched off (the panel switches in the editor).

## Sidecars

`IMG_0001.CR3` → `IMG_0001.CR3.photonation.json`:

```json
{
  "format": "photonation.develop",
  "settings": { "process": 1, "exposure": 0.3, "tone_curve": { "master": [[0, 0], [0.25, 0.2], [1, 1]] } }
}
```

- Missing fields take their defaults, so older sidecars stay readable.
- Masks written before masks could combine shapes have a single `"shape"`
  instead of `"components"`. They load as one *Add* component and render
  unchanged (`StoredMask`).
- Sidecars written before mask groups have neither `mask_groups` nor a mask
  `group`; their masks load ungrouped and render unchanged. While a photo has
  no groups, its sidecar leaves both fields out and looks exactly as before.
- Writes are atomic (temporary file, then rename).
- The rating, flag and colour label live next to the settings, outside
  them, in a `"library"` block (`LibraryInfo`), left out while a photo has
  none: `"library": { "rating": 4, "flag": "pick", "label": "green" }`.
  `flag` is `pick` or `reject`; labels are `red`, `yellow`, `green`, `blue`,
  `purple`. They are not edits: they don't take part in rendering, history
  or the process version. Without a Photonation sidecar, `xmp:Rating` (−1
  is a reject) and `xmp:Label` come from the Lightroom sidecar.
- `save_photo` deletes the sidecar when the settings are defaults and the
  photo has no rating, flag or label, unless a Lightroom sidecar is next to
  it (its edits would come back). `Settings::save` (the CLI's `edit`) keeps
  the library block already there. Builds before 0.14 ignore the block and
  drop it when they save the photo.
- Sidecars written before straightening have no `crop_angle` and render as before.

## Process versions

`process` records which rendering process interprets the settings
(`ProcessVersion`). **A change that alters how existing settings render ships as
a new process version**; old versions stay frozen, so earlier edits keep their
look until the user updates them. Sidecars from a newer build are rejected with a
clear error instead of rendering wrongly. Pure bug fixes (crashes, NaNs, decoder
errors) may land in place.
