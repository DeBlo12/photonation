# Roadmap

## Done

- Raw decoding: Bayer, X-Trans, linear DNG (ProRAW) with EXIF orientation and DNG baseline exposure
- Band-parallel CPU pipeline with MHC / half-size / neighbour-average demosaic
- DNG-style colour model (dual-illuminant interpolation, temperature/tint, eyedropper)
- Light: exposure, contrast, highlights, shadows, whites, blacks
- Presence: texture, clarity, dehaze
- Colour: vibrance, saturation, 8-band colour mixer with a band picker, point colours, colour grading wheels
- Tone curve: parametric, RGB and per-channel point curves, with the photo's
  histogram behind them
- Sharpening; Lanczos resizing; JPEG/PNG/TIFF export with ICC profiles and
  EXIF/XMP metadata (camera and capture, creator and copyright, optional GPS)
- JSON sidecars, process versions, undo/redo
- Masks: linear and radial gradients, drawn on the photo, and a brush, combined
  per mask (add, subtract, intersect), with invert, amount and eight local
  adjustments; layer panel with live thumbnails, reordering and a GPU overlay;
  masks a row can't reach cost it nothing on the CPU, and local saturation shares
  the colour step's trip through Oklab
- Crop and straighten tool (aspect ratios, angle, drag to turn)
- Lightroom XMP import: global develop settings and crops open with the photo
- Object removal: painted removals filled by content-aware fill (PatchMatch)
  or a neural inpainting model (LaMa / MI-GAN via tract), in the background;
  fills are kept on disk, so reopening a photo doesn't compute them again
- Desktop editor (toolbar, viewer with zoom/compare, filmstrip, adjustment cards, export sheet)
  in the Photonation brand (Ink and Paper surfaces, Photon amber, Inter and Sora, the iris on the welcome screen)
- CLI: info, export, edit
- GPU previews (wgpu compute) for every global adjustment, masks, Texture
  and Clarity, checked against the CPU pipeline
- Culling: star ratings, pick/reject flags and colour labels in the
  sidecar (Lightroom's imported), auto-advance, filter and sort, a grid
  with multi-select, a survey view of 2–8 photos, rejects to the system
  bin; embedded-preview thumbnails with a disk cache; a filmstrip scroll bar
- Presets and copy/paste: settings parts copied on their own, presets that
  apply only their parts, ten built-in looks, Lightroom `.xmp` presets
  imported, hover previews, applied to every selected photo
- Highlight reconstruction (process 2): each channel keeps its own headroom
  and clipped channels are rebuilt from the others, on CPU and GPU
- Every preview on the GPU: straightened crops and removal fills too
- Straighten by drawing a level line, and Auto straighten
- Noise reduction: luminance (with detail) and colour, à trous wavelet
  shrinkage on CPU and GPU
- Lens corrections from the bundled Lensfun database (distortion,
  vignetting, chromatic aberration) with manual distortion and vignetting,
  on CPU and GPU; Lightroom's lens corrections import
- macOS menus in the system menu bar; a macOS disk image, with a workflow that
  checks it on clean machines
- Folder overview: a folder that holds only folders shows the ones with raw
  photos, each with a cover and a count
- Light and dark modes that follow the system or a choice in Settings; Liquid
  Glass on macOS 26 and later
- Full screen: the photo alone on the whole screen (`F`), and the window alone
  (`F11` or the system's own)
- Opening a single photo as well as a folder

## Next

In this order:

1. **More masks**: later AI subject and sky masks.
2. **Better removals**: one-click people selection (segmentation), dust
   spot and wire/fence detection, full-resolution tiled fills. See
   [RETOUCH.md](RETOUCH.md).

## Priority

After Next, in this order:

1. **Profiles**: a look stage after the camera matrix (Standard, Neutral,
   Vivid, Monochrome as 3D lookup tables on CPU and GPU), selectable in
   presets; a proper black & white treatment with a per-colour mix and
   toning; presets with an Amount slider.
2. **AVIF and JPEG XL export**; IPTC fields (title, caption, keywords) and
   the rating and label (`xmp:Rating`, `xmp:Label`) in export metadata.
3. **Better demosaicing**: RCD or AMaZE for Bayer, Markesteijn for X-Trans.

## Later

1. **Lightroom import, continued**: masks (linear, radial; brush once masks
   have one), and the edits listed as not imported once Photonation has them.
2. **Local Highlights/Shadows** (edge-aware, as a new process), so they behave
   like Lightroom's on high-contrast scenes. Highlight reconstruction could
   then use the neighbourhood too, taking a clipped area's colour from its
   edges instead of holding it to process 1's.
3. **Violet accent (idea, not decided)**: a possible addition to Photon amber,
   which stays the accent for now. The fill is `#8B6CFF` with Ink text. Text
   and icons in the accent colour need `#A08AFF` on dark surfaces and `#5B3FD9`
   on light ones. The fill stands out better on white than amber does (3.7:1
   against 1.8:1), but it sits near the purple colour label, which would need
   to move towards magenta.

## Platform

- **GPU path, continued**: draw previews straight from the GPU texture
  instead of reading them back, and reuse intermediate textures between
  renders; thumbnails and pickers on the GPU.
- **Web app**: build the egui UI and engine for `wasm32` (WebGL/WebGPU, files
  through the browser's file APIs, OPFS for sidecars and caches). The core
  crates are already free of platform code; threads need a Web Worker pool.
- **Library**: a catalog across folders, with keywords, collections and search
  (ratings, flags and labels are per folder today).
- **More raw formats**: contribute missing cameras upstream to rawler or add a
  second decoder behind `photonation-raw`.
- **Packaging**: published macOS builds, signed with a Developer ID and
  notarized (the installer script can do both; images are signed ad hoc until
  then); a code-signed Windows installer built in CI (an unsigned one builds locally
  today); Linux builds.
