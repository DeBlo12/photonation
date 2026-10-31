# Desktop editor

Crate: `apps/desktop` (binary `photonation-desktop`), built on eframe/egui. The layout
follows Photomator: dark and image-first, with the controls in quiet cards on
a floating inspector. The look is Photonation's brand (see *Look* below).

## Layout

```text
┌─────────────────────────────────────────────────────────────┐
│ ● ● ●  File  Edit  Library  View  Help                           │  menu bar
├─────────────────────────────────────────────────────────────┤
│ Open ▦  IMG_0001.CR3 ★★★☆☆  camera · exp  ↶ ↷ ⛶ ◎ ◐ Fit [Export] │  toolbar
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
tool (⛶ or `R`) likewise swaps in the crop card, and the Remove tool (Remove
or `Q`) the Remove panel, where dragging on the photo paints a removal. At
most one of masks, crop and Remove is open.

The grid (▦ or `G`) and the survey view (`N`) take the place of the viewer
and the sidebar, under the library bar (see *Library* below); opening an
editing tool leaves them. While a tool is open the grid and survey wait
(`open_grid`, `toggle_survey`): the toast names the tool and how to close
it, and the inspector's outline pulses amber (`ViewState::tool_warning`).

| File | Region |
|---|---|
| `panels/toolbar.rs` | Open, the grid button and filter chip, photo name, its stars, flag and label, exposure info, undo/redo, compare, zoom, export |
| `panels/menu_bar.rs` | File, Edit, Library, View and Help menus; each item does what its button or shortcut does |
| `panels/settings.rs` | Settings window (graphics processor, export folder, export defaults) and About |
| `panels/help.rs` | The Help window (? button, Help ▸ Photonation Help, F1): one page per feature, written from the pipeline, the panels and these documents |
| `panels/viewer.rs` | Canvas: fit/zoom/pan, before/after divider, eyedroppers, status toast |
| `panels/adjustments.rs` | Histogram and one card per `AdjustmentGroup` |
| `panels/point_colors.rs` | The Color Mixer's Point Color tab: picked colours, before/after swatch, shifts, ranges, range overlay |
| `panels/filmstrip.rs` | Thumbnails in shown order; edited photos carry an amber dot, open ones their stars, flag and label. Rests as a thin strip, with a scroll bar while it moves (see below) |
| `library.rs` | Culling state and actions: filter, sort, the shown photos, selection, rating changes, auto-advance, batches, the culling keys |
| `panels/library_bar.rs` | The bar above the grid and survey: views, count, filter, sort, auto-advance, thumbnail size; the move-rejects-to-bin dialog |
| `panels/grid.rs` | The grid of the shown photos (multi-select, follows the current photo) |
| `panels/survey.rs` | 2–8 photos side by side, each with its stars and flag |
| `widgets/rating.rs` | Stars, flag and label badges, and the toolbar's rating control |
| `panels/masks.rs` | Masks panel: layer list, the selected mask's shapes, settings and local adjustments |
| `panels/masks/groups.rs` | Mask groups in the layer list: rows, drop targets, group edits, the group row and card |
| `panels/mask_thumbnails.rs` | The layer list's pictures of what each mask selects |
| `panels/mask_handles.rs` | On-canvas handles and pins for mask editing |
| `panels/crop.rs` | Crop tool: the frame on the canvas and the crop card |
| `photo_frame.rs` | Screen ↔ photo positions for the displayed crop (handles, overlay, crop tool) |
| `panels/removals.rs` | Remove panel: method, brush size, model status, removal list, selected removal |
| `panels/removal_brush.rs` | Painting removals on the canvas and drawing brushed areas |
| `gpu/mask_overlay.rs`, `.wgsl` | The selected mask's tint over the photo, evaluated per pixel on the GPU |
| `gpu/mod.rs` | Graphics adapter choice and texture limits |
| `panels/export.rs` | Export sheet (format, quality, size, colour space, scope, file name with a preview of the result, metadata) |
| `preferences.rs` | Choices remembered between launches (`preferences.json` in the data directory): Settings and the export metadata |
| `widgets/*` | Custom controls: adjustment slider, section card, toggle, segmented control, curve editor, colour wheel, histogram; the brand's stroke icons (`icon.rs`, SVG path data drawn as vectors) and iris mark (`brand_mark.rs`) |
| `theme.rs` | Every colour, font and metric |

## Look (`theme.rs`)

Photomator's arrangement in the Photonation brand:

- **Surfaces** are near-neutral greys leaning slightly toward the brand's Ink
  (`#0E1322`), so the window reads as Photonation without tinting how a photo
  is judged. The toolbar, viewer and filmstrip share the window colour; the
  adjustments, masks, crop and Remove panels sit in a rounded inspector that
  floats over it.
- **Photon amber** (`#FFB224`) means "on": active tools, switches, slider
  fills from the default, the modified dot on a card, selections and the
  primary button. Text and icons on an amber fill are Ink (`ON_ACCENT`),
  because Paper on amber fails contrast. Text is Paper (`#F6F5F1`).
- **Type**: Inter for the interface (regular; semibold for card and sheet
  titles), Sora semibold for the wordmark. Both are variable fonts bundled
  from `assets/fonts` and instanced by weight; egui's own fonts stay behind
  them as fallbacks.
- **Tooltips** (`panels::hover_tip`): toolbar and panel buttons name
  themselves and, after a first line, say what they do. They show on
  disabled tools too, so the welcome screen explains tools that need a photo.
- **Welcome screen**: the iris from the app icon, drawn as vectors, over the
  wordmark and an amber Open Folder… button.

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
   Then drop a mask or mask group selection that undo or a deletion made
   invalid (`MaskSelection::validated`, `GroupView::validate`).
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

It scrolls sideways, resting or open: a two-finger swipe, or the mouse wheel
while the pointer is anywhere on it, margins included (the strip's area claims
its whole rect so egui routes the wheel to it, the scroll area spans the whole
strip, and its one scroll direction takes vertical wheel turns too).
`FilmstripScroll` moves it by itself in two cases:

- **The photo changes from elsewhere** (← →, opening a folder): the new photo
  is centred, as far as the ends allow. A photo clicked in the strip is in view
  already, so the strip stays put.
- **Thumbnails change size** (opening, closing, thumbnails loading): the point
  under the pointer, or the middle of the strip when the pointer is elsewhere,
  stays on the same photo, so opening the strip grows it around what you are
  pointing at and the photo just centred stays centred.

Only thumbnails in view are drawn, so long folders stay cheap.

**Scroll bar** (`FilmstripScrollBar`). A thin bar along the bottom of the
strip shows while the strip moves and for 1.2 s after, while the pointer is
over it, and while it is dragged; otherwise it stays out of the way.
Dragging the thumb crosses a long folder quickly, with a bubble above it
naming the photo under the middle of the strip and its capture date; a
click on the track jumps there. It draws only when the photos don't fit.

## Library (`library.rs`)

Each `Photo` carries its `LibraryInfo` (rating 0–5, pick/reject flag,
colour label), read with the folder (`load_photo`, so Lightroom ratings
come along) and written to its sidecar on every change (`save_library`).
Ratings are not edits: they are outside `Settings`, undo and the amber dot.

**Shown photos.** `LibraryView::filter` and `sort` turn the folder into
`shown`, rebuilt every frame by `arrange_photos`. The filmstrip, the grid,
← → (`step`) and batch export all go through `shown`. The current photo
always stays shown, so rating it out of the filter doesn't make it vanish
under the pointer; it drops out once you move on.

**Selection.** `current` plus `selection` (other indices). Plain click
selects one, Ctrl/⌘-click toggles, Shift-click selects the shown run from
the current photo (`click_photo`). Rating changes apply to all of them
(`rating_targets`), except in the survey view, which rates only the
outlined photo. With auto-advance on, rating or flagging a single photo
steps to the next shown one.

**Views.** The grid shows `shown` in rows sized by `grid_size`; the survey
view shows the current and selected photos (`survey_photos`, at most 8),
each requested from the engine at the size its cell needs
(`request_survey_image`, in 256 px steps). Both keep the editor's preview
idle.

**Batches** (`export_batch`): the selection when there is one, else the
shown photos, named on the export sheet's checkbox and the File menu.

**Rejects** (`trash_rejected`): Library › Move Rejected Photos to Recycle
Bin… moves the rejected files and their sidecars (a Lightroom sidecar only
if no remaining photo shares it) to the system bin with the `trash` crate,
after a confirmation, then reopens the folder on the nearest keeper.

Sort order, grid size and auto-advance are remembered between launches
(`LibraryPreferences`); the filter is not.

## Masks

**Panel** (`panels/masks.rs`), top to bottom:

1. **Layer list.** *+ Linear* / *+ Radial* add a mask with one shape and select
   it (inside the selected group, or the selected mask's group, if there is
   one). Each row shows the photo's thumbnail tinted where the mask applies, its
   name, a summary ("2 shapes · inverted · 60 %") and a visibility switch.
   Click a row to select it, drag it to reorder, right-click to invert,
   duplicate, move, group or delete it.
   Each row is a fixed height and keeps its text on one line: names and
   summaries that do not fit end in "…", and the row's tooltip shows them in
   full.
   *+ Group* puts the selected mask in a new group, selects the group and
   focuses its name. A group's header row shows a fold arrow, its name, a
   summary ("3 masks · 60 %") and a visibility switch; its masks are indented
   beneath it, joined by a faint line. Click the header to select the group,
   double-click to rename it, click the arrow to collapse or expand it,
   right-click to rename, fold, ungroup or delete it (with its masks).
2. **Group** (instead of 2–4, while a group is selected): name, amount,
   visibility, *Ungroup* and *Delete Group*.
2. **Shapes** of the selected mask: add shapes, pick the one the handles edit,
   choose how each later shape combines (Add, Subtract, Intersect), remove
   shapes (one always stays). Feather and rotation for radial shapes.
3. **Name, Amount, Invert** and the overlay switch.
4. **Adjustments**: the local sliders, Reset, Delete Mask.

**Selection.** `ViewState::selected_mask` is a `MaskSelection { mask, component }`:
the mask the panel edits and the shape whose handles show. `ViewState::mask_groups`
(`GroupView`) holds the selected group, never set together with a selected mask
(picking a mask on the photo deselects the group), and the collapsed groups.
Both are view state: folding a group is not an edit and is not saved.

**Groups in the list** (`panels/masks/groups.rs`). `list_rows` turns the flat
mask list into rows: a group's header where its masks start, then its masks
unless it is collapsed. Dragging a row resolves against every row's place once
the list is drawn (`drop_target`):

- A mask dropped on the middle half of a group's header joins the group at its
  end (the header is outlined).
- Between rows, it lands in that gap: inside a group when the row below is one
  of the group's masks (the line is indented), so the gap below a group's last
  mask is outside it.
- A dragged group header moves all its masks; dropped inside an open group, it
  goes above that group, since groups don't nest.

*Move Up* / *Move Down* in a mask's menu stay inside its group, and step an
ungrouped mask over a whole neighbouring group. *Move to Group* moves it to the
end of another group or into a new one; *Remove from Group* puts it just below
its group. Every edit (`Layers::apply`) keeps each group's masks together, and
removes a group once its last mask leaves or is deleted; the selection and the
collapsed set follow the renumbering.

**Canvas** (`panels/mask_handles.rs`). The selected shape shows its lines or
ellipse with handles (drag to move or resize; ⇧ keeps a circle round). Hollow
amber rings mark the selected mask's other shapes; solid pins mark the other
masks. Clicking either selects it. A drag that starts on a handle edits the
shape instead of panning.

**Overlay** (`gpu/mask_overlay.rs`). A fragment shader tints the photo by the
selected mask's combined weight, mirroring `MaskWeight::at`. The tint's strength
follows the mask as it renders (`tint_strength`): its amount times its group's
(never fainter than a quarter, so a mask at 0 % still shows what it selects),
and dimmed to 40 % while it or its group is hidden. Pins of hidden masks are
drawn dimmed too. The CPU packs the
mask (up to eight shapes) into one uniform. Tests check the packing against
the pipeline's weights through a transcription of the shader, and validate the
shader with naga. Because the GPU evaluates the overlay, the tint follows
handle drags at once, without waiting for the photo to re-render.

**Thumbnails** (`panels/mask_thumbnails.rs`) are computed on the CPU at twice
the row's point size and cached by the mask's shapes, inversion, crop and size.
They are drawn as strongly as the overlay (`tint_strength`), and over a dimmed
photo while the mask or its group is hidden.
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

## Remove

**Panel** (`panels/removals.rs`): the method for new removals (*Generative
AI* or *Content-aware*), with the model's status under it (ready, loading,
not built, or not installed: a link to download LaMa, its SHA-256, Open
Models Folder and Check Again; see RETOUCH.md), and the brush size (% of the
long edge). Then the list of removals (select, show/hide, ×), and for the
selected one its method, Feather, *Try Another Fill* (content-aware only: a
new seed) and Delete. A spinner by the title shows while fills are computed.

**Canvas** (`panels/removal_brush.rs`): a drag paints a stroke, a click dabs
once, ⇧ adds the stroke to the selected removal. The stroke lives in
`ViewState::painting` and joins the settings on release, so each stroke is
one undo step and one fill. The selected removal's area is drawn red, others
grey, and a ring shows the brush under the pointer.

**Fills** arrive in the background (docs/ENGINE.md): on
`Event::RemovalsFilled` the app requests the current generation again. At
start-up it asks the engine to load the generative model
(`request_model_status`). The `ai` cargo feature (on by default) builds the
neural inpainting; see [RETOUCH.md](RETOUCH.md).

## Export metadata

The export sheet's **Metadata** section chooses what the files carry (see
[ENGINE.md](ENGINE.md#export-exportrs-export_photo)): a segmented control
with the presets **All**, **Copyright Only** and **None**, and a checkbox per
group (Camera and capture, Copyright and creator, Location). The control
highlights the preset the checkboxes match and nothing for a custom choice.
With Copyright and creator on, two fields take a creator and a copyright
notice; left blank, the raw file's are written. Location is off until ticked.

The choice and the entered text are saved to `preferences.json` when an
export starts and loaded when the app opens (`preferences.rs`), so they are
remembered between exports and launches. A missing or unreadable file means
the defaults (All, location off, nothing entered).

## Settings (`panels/settings.rs`, `preferences.rs`)

File ▸ Settings… (Ctrl+, / ⌘,) opens a window with two tabs:

- **General.** The graphics processor (see *GPU selection*), with the
  adapter in use and a note when the choice waits for a restart. The export
  folder: the export's folder dialog starts there, and with *Export straight
  to this folder* the export sheet skips the dialog and says where files go.
  A folder that no longer exists means the dialog again.
- **Export.** The options every new export sheet starts with: format,
  quality or bit depth, size, colour space, file name and metadata. The
  sheet and this tab share their controls (`export::format_controls`,
  `file_name_field`, `metadata`). Changing them in the sheet affects only
  that session; the metadata choice is also remembered when an export starts,
  as before.

Everything is saved to `preferences.json` when the window closes, and the
current export options are reset to the new defaults. Missing fields take
their defaults, so older files load.

**Menu bar** (`panels/menu_bar.rs`). File (Open Folder, Export, Export All
Photos, Settings, Quit), Edit (Undo, Redo, Reset All Adjustments), View (Fit,
Actual Pixels, Compare, Masks, Crop and Straighten, Remove Objects, previous
and next photo) and Help (Photonation Help, Keyboard Shortcuts, About).
Items show their shortcuts and are disabled when they cannot apply. On macOS
the bar starts right of the window buttons.

## Help (`panels/help.rs`)

The ? button, Help ▸ Photonation Help and F1 open a window with a list of
pages on the left and the selected page on the right, one page per feature:
getting started, how a photo is developed (the pipeline's order), one per
adjustment card, masks and groups, crop, Remove, compare and zoom, export,
Settings, Lightroom import and the keyboard shortcuts. Help ▸ Keyboard
Shortcuts opens the last one.

- **Where it opens.** On the page of the tool in use (`help::tool_page`): the
  export sheet, Settings, masks, crop or Remove, in that order; otherwise on
  the page last shown. The page is view state (`ViewState::help`), kept while
  the app runs and not saved. Each page keeps its own scroll position.
- **Above the sheets.** The window is not modal, so the photo stays usable
  beside it. It sits in egui's foreground order and is raised whenever it is
  opened, so F1 from the export sheet or Settings shows it above their
  backdrop. Esc closes it; `shortcuts` takes the key before the crop tool and
  the sheets see it.
- **Look.** Ink surfaces (the window in the inspector's colour, the page
  list in the main window's), Paper text in Inter, the selected page in amber with Ink text, the
  toolbar's stroke icons beside the pages of its tools, pipeline steps in
  amber discs and shortcuts as key caps.
- **Text.** Written from the code and `docs/*.md`; menu paths use › because
  Inter has no ▸. Tests check every page is listed once, has text, and opens
  for its tool.

## GPU selection (`gpu/mod.rs`)

wgpu draws the UI, previews and the overlay. Previews are also rendered on
the GPU where possible (docs/ENGINE.md, *GPU previews*), on a separate
device of the fastest hardware adapter; `PHOTONATION_GPU_RENDER=off`
renders them on the CPU. Adapters are ranked discrete → integrated → virtual → software,
skipping those that can't draw to the window and preferring native backends
(Metal, Vulkan, DX12) over OpenGL. Settings ▸ Graphics processor can put
integrated graphics or software rendering first, or name one adapter (the
list is the adapters that could draw the window at start-up); a named adapter
that is gone falls back to the automatic order. The choice is read when the
window is created, so it applies from the next start.
`PHOTONATION_GPU=integrated` or `software` overrides it. The device requests the adapter's largest texture side (up
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
the masks panel with the first mask selected (`=group` selects the first mask
group instead); `PHOTONATION_SCREENSHOT_CROP=1` opens the crop tool;
`PHOTONATION_SCREENSHOT_REMOVE=1` the Remove panel;
`PHOTONATION_SCREENSHOT_SIDEBAR=280` starts the sidebar at that width
(280–440), to check narrow and wide layouts; `PHOTONATION_SCREENSHOT_EXPORT=1`
opens the export sheet; `PHOTONATION_SCREENSHOT_HELP=1` the Help window
(or a page's title, e.g. `=Crop and straighten`, for that page);
`PHOTONATION_SCREENSHOT_SETTINGS=general` (or `=export`) the Settings window;
`PHOTONATION_SCREENSHOT_LIBRARY=grid` the grid (`=survey` surveys the first
two photos). The screenshot window opens unfocused and lets the pointer
through, so a run never takes the keyboard from whoever is at the computer.
