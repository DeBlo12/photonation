# Desktop editor

Crate: `apps/desktop` (binary `photonation-desktop`), built on eframe/egui. The look
follows Photomator: dark and image-first, with the controls in quiet cards.

## Layout

```text
┌─────────────────────────────────────────────────────────────┐
│ ● ● ●  Open  IMG_0001.CR3  camera · exposure    ↶ ↷ ◐ Fit 1:1 [Export] │  toolbar
├───────────────────────────────────────────┬─────────────────┤
│                                           │ histogram       │
│                                           │ ▸ White Balance │
│                 viewer                    │ ▾ Light         │  adjustments
│        (fit / zoom / pan / compare)       │   sliders…      │  sidebar
│                                           │ ▸ Curves …      │
├───────────────────────────────────────────┴─────────────────┤
│ [thumb] [thumb] [thumb] …                                    │  filmstrip
└─────────────────────────────────────────────────────────────┘
```

| File | Region |
|---|---|
| `panels/toolbar.rs` | Open, photo name and exposure info, undo/redo, compare, zoom, export |
| `panels/viewer.rs` | Canvas: fit/zoom/pan, before/after divider, eyedropper, status toast |
| `panels/adjustments.rs` | Histogram and one card per `AdjustmentGroup` |
| `panels/filmstrip.rs` | Thumbnails; edited photos carry a blue dot |
| `panels/export.rs` | Export sheet (format, quality, size, colour space, scope) |
| `widgets/*` | Custom controls: adjustment slider, section card, toggle, segmented control, curve editor, colour wheel, histogram |
| `theme.rs` | Every colour and metric |

## Per-frame control flow (`app.rs: PhotonationApp::ui`)

1. **Drain engine events**: install previews and thumbnails as textures
   (ignoring stale generations), apply eyedropper results, record photo info,
   show errors as a toast.
2. **Keyboard shortcuts.**
3. **Snapshot** the settings and the disabled panels.
4. **Draw** the toolbar, filmstrip, sidebar, viewer and, if open, the export
   sheet. Widgets edit `app.settings` directly. The viewer asks for a preview
   at the resolution it needs (`request_previews`), which sends a request only
   when the edit generation or the size changed.
5. **Edited?** If the settings or switches differ from the snapshot, bump the
   generation (triggering a new preview) and schedule a sidecar save.
6. **Commit history** when no pointer button is down (one undo step per drag).
7. **Save the sidecar** 400 ms after the last edit, once the pointer is released.

## Preview resolution

- Fit: the viewer's size in physical pixels, rounded up to 256 px steps so
  small window resizes don't re-render.
- Zoomed in: as many pixels as the zoom needs, up to full resolution, except
  while a control is being dragged, when it stays at fit size for speed and
  refines on release.

## Panel switches

Turning a card off renders the photo as if that group were at its defaults
(`AdjustmentGroup::apply_disabled`) without losing the values. Switches are view
state and are not saved.

## Screenshot mode (development)

`PHOTONATION_SCREENSHOT=out.png photonation-desktop <folder or photo>` saves the window
once the preview and thumbnails are ready, then quits.
`PHOTONATION_SCREENSHOT_PANELS="Curves,Color Grading"` picks the open cards.
