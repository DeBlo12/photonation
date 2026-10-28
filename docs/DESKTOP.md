# Desktop editor

Crate: `apps/desktop` (binary `photonation-desktop`), built on eframe/egui. The look
follows Photomator: dark and image-first, with the controls in quiet cards.

## Layout

```text
┌─────────────────────────────────────────────────────────────┐
│ ● ● ●  Open  IMG_0001.CR3  camera · exposure  ↶ ↷ ⛶ ◎ ◐ Fit 1:1 [Export] │  toolbar
├───────────────────────────────────────────┬─────────────────┤
│                                           │ histogram       │
│                                           │ ▸ White Balance │
│                 viewer                    │ ▾ Light         │  adjustments
│        (fit / zoom / pan / compare)       │   sliders…      │  sidebar
│                                           │ ▸ Curves …      │
├───────────────────────────────────────────┴─────────────────┤
│ ▫ ▫ ▫ …                                                      │  filmstrip (resting)
└─────────────────────────────────────────────────────────────┘
```

While masking (◎ or `M`) the sidebar shows the masks panel instead of the
adjustment cards, and the photo shows the selected shape's handles. The crop
tool (⛶ or `R`) likewise swaps in the crop card; masking and cropping close
each other.

| File | Region |
|---|---|
| `panels/toolbar.rs` | Open, photo name and exposure info, undo/redo, compare, zoom, export |
| `panels/viewer.rs` | Canvas: fit/zoom/pan, before/after divider, eyedroppers, status toast |
| `panels/adjustments.rs` | Histogram and one card per `AdjustmentGroup` |
| `panels/point_colors.rs` | The Color Mixer's Point Color tab: picked colours, before/after swatch, shifts, ranges, range overlay |
| `panels/filmstrip.rs` | Thumbnails; edited photos carry a blue dot. Rests as a thin strip (see below) |
| `panels/masks.rs` | Masks panel: layer list, the selected mask's shapes, settings and local adjustments |
| `panels/mask_thumbnails.rs` | The layer list's pictures of what each mask selects |
| `panels/mask_handles.rs` | On-canvas handles and pins for mask editing |
| `panels/crop.rs` | Crop tool: the frame on the canvas and the crop card |
| `photo_frame.rs` | Screen ↔ photo positions for the displayed crop (handles, overlay, crop tool) |
| `gpu/mask_overlay.rs`, `.wgsl` | The selected mask's tint over the photo, evaluated per pixel on the GPU |
| `gpu/mod.rs` | Graphics adapter choice and texture limits |
| `panels/export.rs` | Export sheet (format, quality, size, colour space, scope, file name with a preview of the result) |
| `widgets/*` | Custom controls: adjustment slider, section card, toggle, segmented control, curve editor, colour wheel, histogram |
| `theme.rs` | Every colour and metric |

## Per-frame control flow (`app.rs: PhotonationApp::ui`)

1. **Drain engine events**: install previews and thumbnails as textures
   (ignoring stale generations), apply eyedropper results (see below), record photo info,
   show errors as a toast.
2. **Keyboard shortcuts.**
3. **Snapshot** the settings and the disabled panels.
4. **Draw** the toolbar, filmstrip, sidebar (adjustment cards or the masks
   panel), viewer and, if open, the export sheet. Widgets edit `app.settings` directly. The viewer asks for a preview
   at the resolution it needs (`request_previews`), which sends a request only
   when the edit generation or the size changed.
   Then drop a mask selection that undo or a deletion made invalid
   (`MaskSelection::validated`).
5. **Edited?** If the settings or switches differ from the snapshot, bump the
   generation (triggering a new preview) and schedule a sidecar save. While
   the crop tool is open, a change to the crop alone only schedules the save:
   the tool shows the whole photo, so there is nothing new to render.
6. **Commit history** when no pointer button is down (one undo step per drag).
7. **Save the sidecar** 400 ms after the last edit, once the pointer is released.

## Eyedroppers

`view.eyedropper` holds the active picker (`Eyedropper`); the next click on the
photo goes to `PhotonationApp::pick_color` and turns the picker off (Esc
cancels it).

| Picker | Button | Engine call | Result |
|---|---|---|---|
| `WhiteBalance` | White Balance card | `pick_white_balance` | Custom white balance |
| `PointColor` | Color Mixer, Point Color tab | `pick_color` | A new point colour, selected |
| `MixerBand` | Color Mixer, Mixer tab | `pick_color` | Selects the band nearest the colour's hue (`nearest_mixer_band`) |

Colour picks remember their purpose in `pending_color_pick` until the
`ColorPicked` event arrives.

**Range overlay.** With "Visualize range" on, previews render the selected
point colour's range (`ViewState::point_color_overlay`) while its tab is
visible and the Color Mixer is switched on. The overlay is part of the preview
request's key, and switching it bumps the generation, so the old look is
never shown as current.

## Preview resolution

- Fit: the viewer's size in physical pixels, rounded up to 256 px steps so
  small window resizes don't re-render.
- Zoomed in: as many pixels as the zoom needs, up to full resolution, except
  while a control is being dragged, when it stays at fit size for speed and
  refines on release.

## Filmstrip

The filmstrip rests as a 26 pt row of tiny thumbnails, so the photo gets the
room. It grows to full size (104 pt) while the pointer is on it, stays open for
half a second after the pointer leaves, and opens for 1.2 s whenever the photo
changes, to show where in the folder the new one is (`FilmstripReveal`). Only
the resting strip is reserved in the layout. The open strip floats over the
bottom of the viewer, so the photo neither moves nor re-renders as the strip
opens and closes.

## Masks

**Panel** (`panels/masks.rs`), top to bottom:

1. **Layer list.** *+ Linear* / *+ Radial* add a mask with one shape and select
   it. Each row shows the photo's thumbnail tinted where the mask applies, its
   name, a summary ("2 shapes · inverted · 60 %") and a visibility switch.
   Click a row to select it, drag it to reorder, right-click to invert,
   duplicate, move or delete it.
2. **Shapes** of the selected mask: add shapes, pick the one the handles edit,
   choose how each later shape combines (Add, Subtract, Intersect), remove
   shapes (one always stays). Feather and rotation for radial shapes.
3. **Name, Amount, Invert** and the overlay switch.
4. **Adjustments**: the local sliders, Reset, Delete Mask.

**Selection.** `ViewState::selected_mask` is a `MaskSelection { mask, component }`:
the mask the panel edits and the shape whose handles show.

**Canvas** (`panels/mask_handles.rs`). The selected shape shows its lines or
ellipse with handles (drag to move or resize; ⇧ keeps a circle round). Hollow
blue rings mark the selected mask's other shapes; solid pins mark the other
masks. Clicking either selects it. A drag that starts on a handle edits the
shape instead of panning.

**Overlay** (`gpu/mask_overlay.rs`). A fragment shader tints the photo by the
selected mask's combined weight, mirroring `MaskWeight::at`. The CPU packs the
mask (up to eight shapes) into one uniform. Tests check the packing against
the pipeline's weights through a transcription of the shader, and validate the
shader with naga. Because the GPU evaluates the overlay, the tint follows
handle drags at once, without waiting for the photo to re-render.

**Thumbnails** (`panels/mask_thumbnails.rs`) are computed on the CPU at twice
the row's point size and cached by the mask's shapes, inversion, crop and size.
Textures not drawn in a frame are dropped.

**Straightened photos.** `PhotoFrame` maps through the crop frame, so handles,
pins and the overlay follow a turned crop; the overlay's uniform carries the
screen → mask mapping as an origin and one step per screen axis.

## Crop and straighten (`panels/crop.rs`)

The crop tool (⛶ or `R`; `Return` or `Esc` closes it) previews the whole photo,
unturned, and draws the crop frame over it, turned by the angle, with a
rule-of-thirds grid and the outside dimmed:

- **Drag inside** the frame to move it; it slides along the photo's edges.
- **Drag a corner or an edge** to resize it; the opposite side stays put and the
  frame stops at the photo's edge (a binary search between the frame at the
  start of the drag, which fits, and the one under the pointer).
- **Drag outside** to turn it around its centre (straighten); it shrinks to stay
  on the photo.
- **Aspect** (sidebar): Original (default), Free, 1:1, 4:5, 5:7, 2:3, 9:16, in
  the frame's orientation; ⇔ swaps portrait and landscape. A corner drag follows
  the larger change, an edge drag resizes the other side around its middle.
- **Angle** slider: turns the frame from where it was when the drag began, so
  turning back gives the frame its size back.
- **Reset** removes the crop and the angle.

The whole photo, unturned, is stored as no crop (`settings.crop = None`), so a
reset photo needs no sidecar. Compare (◐) frames the original like the edit.

## Lightroom edits

A photo with a Lightroom `.xmp` sidecar and no Photonation sidecar opens with
its Lightroom edits (see EDIT-MODEL.md), and a toast names any edits that could
not come along; it goes away when another photo is shown. Thumbnails and the
filmstrip's edited dot use the same settings. Resetting such a photo keeps a
default sidecar, so the Lightroom edits do not return.

## GPU selection (`gpu/mod.rs`)

wgpu draws the UI, previews and the overlay; the develop pipeline itself runs
on the CPU. Adapters are ranked discrete → integrated → virtual → software,
skipping those that can't draw to the window and preferring native backends
(Metal, Vulkan, DX12) over OpenGL. `PHOTONATION_GPU=integrated` or `software`
changes the order. The device requests the adapter's largest texture side (up
to 16384). Previews never exceed it, so 1:1 views of high-resolution cameras
don't fail.

## Panel switches

Turning a card off renders the photo as if that group were at its defaults
(`AdjustmentGroup::apply_disabled`) without losing the values. Switches are view
state and are not saved.

## Screenshot mode (development)

`PHOTONATION_SCREENSHOT=out.png photonation-desktop <folder or photo>` saves the window
once the preview and thumbnails are ready, then quits.
`PHOTONATION_SCREENSHOT_PANELS="Curves,Color Grading"` picks the open cards;
`PHOTONATION_SCREENSHOT_POINT_COLOR=1` opens the Point Color tab (`=range` also
shows the first point colour's range); `PHOTONATION_SCREENSHOT_MASKS=1` opens
the masks panel with the first mask selected; `PHOTONATION_SCREENSHOT_CROP=1`
opens the crop tool.
