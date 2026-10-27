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
| Color Mixer | `color_mixer`: hue/saturation/luminance for red, orange, yellow, green, aqua, blue, purple, magenta |
| Color Grading | `color_grading`: `shadows`, `midtones`, `highlights`, `global` wheels (hue, saturation, luminance), `blending`, `balance` |
| Detail | `sharpening`: amount, radius |
| — | `crop` and `crop_angle` (see below), `masks` (see below), `process` |

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
crop too.

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

**Mask space** (`MaskSpace`). Positions are normalized to the photo: 0..1 across
its width and height, in display orientation, on the camera's default crop but
*before* the user's crop, so a mask stays on the same content when the crop
changes. Radii are in units of the long edge, so circles stay round on any
aspect ratio.

**Sanitizing** caps the number of masks and shapes, truncates names, clamps
every value and repairs non-finite numbers. The caps also bound the per-pixel
cost a hostile sidecar can cause.

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
- Writes are atomic (temporary file, then rename).
- The editor deletes the sidecar when a photo is reset to defaults.
- Sidecars written before straightening have no `crop_angle` and render as before.

## Process versions

`process` records which rendering process interprets the settings
(`ProcessVersion`). **A change that alters how existing settings render ships as
a new process version**; old versions stay frozen, so earlier edits keep their
look until the user updates them. Sidecars from a newer build are rejected with a
clear error instead of rendering wrongly. Pure bug fixes (crashes, NaNs, decoder
errors) may land in place.
