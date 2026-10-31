# Roadmap

## Done

- Raw decoding: Bayer, X-Trans, linear DNG (ProRAW) with EXIF orientation and DNG baseline exposure
- Band-parallel CPU pipeline with MHC / half-size / neighbour-average demosaic
- DNG-style colour model (dual-illuminant interpolation, temperature/tint, eyedropper)
- Light: exposure, contrast, highlights, shadows, whites, blacks
- Presence: texture, clarity, dehaze
- Colour: vibrance, saturation, 8-band colour mixer with a band picker, point colours, colour grading wheels
- Tone curve: parametric, RGB and per-channel point curves
- Sharpening; Lanczos resizing; JPEG/PNG/TIFF export with ICC profiles and
  EXIF/XMP metadata (camera and capture, creator and copyright, optional GPS)
- JSON sidecars, process versions, undo/redo
- Masks: linear and radial gradients combined per mask (add, subtract,
  intersect), with invert, amount and eight local adjustments; layer panel with
  live thumbnails, reordering and a GPU overlay
- Crop and straighten tool (aspect ratios, angle, drag to turn)
- Lightroom XMP import: global develop settings and crops open with the photo
- Object removal: painted removals filled by content-aware fill (PatchMatch)
  or a neural inpainting model (LaMa / MI-GAN via tract), in the background
- Desktop editor (toolbar, viewer with zoom/compare, filmstrip, adjustment cards, export sheet)
  in the Photonation brand (Ink surfaces, Photon amber, Inter and Sora, the iris on the welcome screen)
- CLI: info, export, edit
- GPU previews (wgpu compute) for every global adjustment, checked against
  the CPU pipeline

## Next

1. **Lightroom import, continued**: masks (linear, radial; brush once masks
   have one), and the edits listed as not imported once Photonation has them.
2. **Straighten by drawing a line** along the horizon, and auto-straighten.
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
9. **AVIF and JPEG XL export**; IPTC fields (title, caption, keywords) in
   export metadata.

10. **Better removals**: one-click people selection (segmentation), dust
    spot and wire/fence detection, full-resolution tiled fills, a disk cache
    for fills. See [RETOUCH.md](RETOUCH.md).

## Platform

- **GPU path, continued**: masks, Texture and Clarity, straightened crops and
  removal fills on the GPU, so every preview renders there; then draw
  previews straight from the GPU texture instead of reading them back, and
  reuse intermediate textures between renders.
- **Web app**: build the egui UI and engine for `wasm32` (WebGL/WebGPU, files
  through the browser's file APIs, OPFS for sidecars and caches). The core
  crates are already free of platform code; threads need a Web Worker pool.
- **Library**: catalog with ratings, flags, keywords and search across folders.
- **More raw formats**: contribute missing cameras upstream to rawler or add a
  second decoder behind `photonation-raw`.
- **Packaging**: signed macOS app bundle (with `assets/icon/photonation.icns`);
  a code-signed Windows installer built in CI (an unsigned one builds locally
  today); Linux builds.
