# Roadmap

## Done

- Raw decoding: Bayer, X-Trans, linear DNG (ProRAW) with EXIF orientation and DNG baseline exposure
- Band-parallel CPU pipeline with MHC / half-size / neighbour-average demosaic
- DNG-style colour model (dual-illuminant interpolation, temperature/tint, eyedropper)
- Light: exposure, contrast, highlights, shadows, whites, blacks
- Presence: texture, clarity, dehaze
- Colour: vibrance, saturation, 8-band colour mixer, colour grading wheels
- Tone curve: parametric, RGB and per-channel point curves
- Sharpening; Lanczos resizing; JPEG/PNG/TIFF export with ICC profiles
- JSON sidecars, process versions, undo/redo
- Masks: linear and radial gradients combined per mask (add, subtract,
  intersect), with invert, amount and eight local adjustments; layer panel with
  live thumbnails, reordering and a GPU overlay
- Desktop editor (toolbar, viewer with zoom/compare, filmstrip, adjustment cards, export sheet)
- CLI: info, export, edit

## Next

1. **Lightroom XMP import**: read `crs:` develop settings from existing `.xmp`
   sidecars so photos already edited in Lightroom open with their edits.
2. **Crop and straighten** tool in the viewer (the model already stores a crop).
3. **Highlight reconstruction** (recover detail where only some channels clip).
4. **Local Highlights/Shadows** (edge-aware, as process V2), so they behave like
   Lightroom's on high-contrast scenes.
5. **Noise reduction** (luminance and colour) and **lens corrections**
   (distortion, vignetting, chromatic aberration).
6. **More masks**: a brush, creating gradients by dragging on the photo, masks
   in compare mode, and later AI subject and sky masks. Skip masks whose
   bounds miss a band, and fold local saturation into the colour step's Oklab
   round trip, to speed up masked renders.
7. **Better demosaicing**: RCD or AMaZE for Bayer, Markesteijn for X-Trans.
8. **Presets** and copy/paste of settings across photos.
9. **EXIF/XMP metadata in exports**; AVIF and JPEG XL export.

## Platform

- **GPU path** (wgpu compute) for real-time previews at any zoom, validated
  against the CPU pipeline.
- **Web app**: build the egui UI and engine for `wasm32` (WebGL/WebGPU, files
  through the browser's file APIs, OPFS for sidecars and caches). The core
  crates are already free of platform code; threads need a Web Worker pool.
- **Library**: catalog with ratings, flags, keywords and search across folders.
- **More raw formats**: contribute missing cameras upstream to rawler or add a
  second decoder behind `photonation-raw`.
- **Packaging**: signed macOS app bundle, then Windows and Linux builds.
