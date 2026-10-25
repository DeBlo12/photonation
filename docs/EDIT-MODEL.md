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
| — | `crop` (normalized, on the displayed image), `process` |

`Settings::sanitized()` clamps everything into range and repairs bad input; the
pipeline always renders sanitized settings.

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
- Writes are atomic (temporary file, then rename).
- The editor deletes the sidecar when a photo is reset to defaults.

## Process versions

`process` records which rendering process interprets the settings
(`ProcessVersion`). **A change that alters how existing settings render ships as
a new process version**; old versions stay frozen, so earlier edits keep their
look until the user updates them. Sidecars from a newer build are rejected with a
clear error instead of rendering wrongly. Pure bug fixes (crashes, NaNs, decoder
errors) may land in place.
