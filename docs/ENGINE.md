# Engine: background work

Crate: `photonation-engine`. Front ends talk to `RenderService` and receive
`Event`s; the service owns all worker threads.

## Workers

| Worker | Threads | Input | Output events |
|---|---|---|---|
| Preview | 1 | `PreviewJob::Render(PreviewRequest)`, `PreviewJob::Pick` (white balance or colour) | `PhotoLoaded`, `Preview`, `WhiteBalancePicked`, `ColorPicked`, `Error` |
| Thumbnails | 2 | `ThumbnailJob` | `Thumbnail`, `Error` |
| Export | 1 | `ExportJob` (a batch) | `ExportProgress`, `ExportFinished` |

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

- **Latest wins**: intermediate slider positions are skipped, never queued, so
  the preview keeps up with the pointer.
- **Decoded photo cache**: the current photo's `SensorImage` stays in memory, so
  edits only re-run the pipeline (no decoding). Changing photos frees the old
  mosaic before decoding the new one.
- **Generations**: every request carries the editor's edit generation; the UI
  drops results older than one it has already shown.
- **Overlays**: `PreviewRequest::point_color_overlay` renders one point
  colour's range instead of the edit (see docs/PIPELINE.md §5).

## Export (`export.rs: export_photo`)

`decode → settings (explicit, or the sidecar) → optional adjust hook → render →
encode → atomic write`. The CLI and the desktop export sheet both use it; the
CLI's slider overrides arrive through the adjust hook.

## History (`history.rs`)

Undo/redo of whole `Settings` values. Front ends call `commit(&settings)` when an
interaction ends (the desktop app: whenever no mouse button is down), so one
slider drag is one undo step.
