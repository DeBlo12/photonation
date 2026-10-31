# Engine: background work

Crate: `photonation-engine`. Front ends talk to `RenderService` and receive
`Event`s; the service owns all worker threads.

## Workers

| Worker | Threads | Input | Output events |
|---|---|---|---|
| Preview | 1 | `PreviewJob::Render(PreviewRequest)`, `PreviewJob::Pick` (white balance or colour) | `PhotoLoaded`, `Preview`, `WhiteBalancePicked`, `ColorPicked`, `Error` |
| Thumbnails | 2 | `ThumbnailJob` | `Thumbnail`, `Error` |
| Export | 1 | `ExportJob` (a batch) | `ExportProgress`, `ExportFinished` |
| Retouch | 1 | `RetouchJob::Fill`, `RetouchJob::Status` | `RemovalsFilled`, `ModelStatus`, `Error` |

Every event send also calls the `notify` callback the front end passed in (the
desktop app passes `ctx.request_repaint`), so the UI wakes up only when there is
something to show.

## Preview control flow (`service.rs: preview_worker`)

```text
loop:
  block for the next job
  drain everything else queued            ← a slider drag queues many requests
  keep the newest Render per kind (Edited, Before) and every picker click
  for each pick:   load photo (cached) → white_balance_at → WhiteBalancePicked
                                       or color_at        → ColorPicked
  for each render: load photo (cached) → render 8-bit sRGB + histogram → Preview
```

- **GPU previews** (`preview.rs`, feature `gpu`, on in the desktop app):
  `RenderService::new` gives the preview worker a `PreviewRenderer` that
  renders on the GPU (docs/PIPELINE.md §8) and on the CPU whatever the GPU
  path doesn't cover yet. A GPU error moves all later previews to the CPU.
  `PHOTONATION_GPU_RENDER=off` keeps previews on the CPU. Services made with
  `with_decoder` or `with_parts` (tests) render on the CPU. Thumbnails,
  pickers and exports always use the CPU.

- **Latest wins**: intermediate slider positions are skipped, never queued, so
  the preview keeps up with the pointer.
- **Decoded photo cache**: the current photo's `SensorImage` stays in memory, so
  edits only re-run the pipeline (no decoding). Changing photos frees the old
  mosaic before decoding the new one.
- **Generations**: every request carries the editor's edit generation; the UI
  drops results older than one it has already shown.
- **Overlays**: `PreviewRequest::point_color_overlay` renders one point
  colour's range instead of the edit (see docs/PIPELINE.md §5).

## Removal fills (`retouch.rs`)

One `Retoucher` is shared by all workers. It caches fills per photo (the 8
most recent photos) by removal chain fingerprint, chooses the inpainter per
removal (the generative model when loaded, else content-aware fill), and
fills missing removals in order.

- **Preview** renders with the cached fills and, if any are missing, sends
  `RetouchJob::Fill` to the retouch worker instead of waiting.
- **Retouch worker** keeps only the newest request of a burst, fills what is
  missing and sends `RemovalsFilled` after each fill; the desktop then asks
  for the same generation again. `RetouchJob::Status`
  (`request_model_status`) loads the model if needed and answers with
  `ModelStatus`.
- **Thumbnails and exports** fill missing removals synchronously.

See [RETOUCH.md](RETOUCH.md).

## Export (`export.rs: export_photo`)

`decode → settings (explicit, or the sidecar, or Lightroom's) → optional adjust hook → fill
removals → render → encode → atomic write`. The CLI and the desktop export sheet both use it; the
CLI's slider overrides arrive through the adjust hook.

**File names** (`export_file_stem`, `output_path`). `ExportOptions::file_name`
is a template: `{name}` becomes the original's file name without extension,
`{n}` the photo's number in the batch (from 1, zero-padded to the batch
size's width, so `Trip {n}` gives `Trip 01` … `Trip 12`). A batch whose
template has neither token gets `-{n}` appended so the files stay distinct.
The result is always one plain file name in the chosen folder: `/`, `\`, `:`
and control characters become `-`, leading dots and surrounding spaces are
dropped, it is cut at 200 characters, and an empty name falls back to the
original's. Exports never replace a file (`unused_path` adds `-2`, `-3`, …).
The CLI takes the template as `--name`.

**Metadata** (`metadata.rs: MetadataOptions`). The raw decoder fills
`CameraInfo` from the file's EXIF: make, model, lens, exposure (shutter,
aperture, ISO, compensation, focal length), capture time with its UTC
offset, artist, copyright and GPS position. `ExportOptions::metadata`
chooses what of it the export writes, in three groups:

| Group | Fields | Default |
|---|---|---|
| Camera and capture (`camera`) | make, model, lens, exposure, capture date and time | on |
| Copyright and creator (`copyright`) | artist and copyright: the ones entered (`creator`, `copyright_notice`), else the file's | on |
| Location (`location`) | GPS latitude, longitude, altitude | off (privacy) |

Presets (`MetadataPreset`, Lightroom's "Include" menu) set the groups:
**All** turns on camera and copyright and leaves location as chosen, like
Lightroom's separate "Remove Location Info"; **Copyright Only** and **None**
also turn location off. `MetadataOptions::select` runs after rendering and
gives the encoder a `CameraInfo` with only the chosen fields;
`photonation_export::write` writes what is not empty and nothing at all for
an empty one (preset None):

- **EXIF** (`crates/export/src/exif.rs`, our own encoder: the format is a
  few TIFF directories). IFD0 has make, model, artist, copyright,
  `Software` ("Photonation <version>") and `Orientation` = 1, because
  exports are rendered upright. The Exif IFD has exposure, capture times
  (`DateTimeOriginal`/`Digitized` with `OffsetTime…`), lens model,
  `PixelX/YDimension` and `ColorSpace` (1 for sRGB, else 0xFFFF
  "uncalibrated" so readers use the ICC profile). Decimal values become the
  simplest fraction (1/160 s, −2/3 EV stay exact). Text is UTF-8 (readers
  accept it in ASCII fields), NUL-free and cut at 2000 bytes so a JPEG
  segment always fits. JPEG carries the TIFF stream in APP1 (`Exif\0\0`),
  PNG in `eXIf`; TIFF writes the Exif and GPS directories first and links
  them from the image's own IFD0.
- **XMP** (`xmp.rs`): creator (`dc:creator`), copyright (`dc:rights`), lens
  (`aux:Lens`, `exifEX:LensModel`), capture time and creating tool. macOS
  Spotlight and Finder read the creator, copyright and lens only from XMP.
  JPEG: APP1 `http://ns.adobe.com/xap/1.0/`; PNG: `iTXt`
  `XML:com.adobe.xmp`; TIFF: tag 700.

The ICC profile is written as before (JPEG APP2 after the APP1 segments,
PNG `iCCP`, TIFF tag 34675). The CLI takes `--metadata all|copyright|none`,
`--location`, `--creator` and `--copyright`.

## History (`history.rs`)

Undo/redo of whole `Settings` values. Front ends call `commit(&settings)` when an
interaction ends (the desktop app: whenever no mouse button is down), so one
slider drag is one undo step.
