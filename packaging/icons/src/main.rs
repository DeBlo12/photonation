// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Renders the Photonation app icon to the files in `assets/icon`: SVG masters
//! of the four variants, PNGs, a Windows `.ico` and a macOS `.icns`.
//!
//! The design is the "Photonation Icon System" canvas (mark construction and
//! app icon boards): a 100 × 100 u tile with a 22.4 u corner, an iris of six
//! blades (Ø 68 u, rotated 15°) around a hexagonal opening and the photon,
//! an amber Ø 10 u dot. Seams between blades thicken at small sizes.
//!
//! ```bash
//! cargo run --release --manifest-path packaging/icons/Cargo.toml
//! ```

use std::path::{Path, PathBuf};

/// The blades from the top-left light to the far side, as in the design.
const BLADES: [&str; 6] = [
    "M62.07 53.24 L34.86 80.45 A34 34 0 0 0 68.80 78.33 Z",
    "M53.24 62.07 L16.07 52.11 A34 34 0 0 0 34.86 80.45 Z",
    "M41.16 58.84 L31.20 21.67 A34 34 0 0 0 16.07 52.11 Z",
    "M37.93 46.76 L65.14 19.55 A34 34 0 0 0 31.20 21.67 Z",
    "M46.76 37.93 L83.93 47.89 A34 34 0 0 0 65.14 19.55 Z",
    "M58.84 41.16 L68.80 78.33 A34 34 0 0 0 83.93 47.89 Z",
];
const SEAMS: &str = "M62.07 53.24 L68.80 78.33 M53.24 62.07 L34.86 80.45 \
    M41.16 58.84 L16.07 52.11 M37.93 46.76 L31.20 21.67 \
    M46.76 37.93 L65.14 19.55 M58.84 41.16 L83.93 47.89";
const OPENING: &str =
    "M62.07 53.24 L53.24 62.07 L41.16 58.84 L37.93 46.76 L46.76 37.93 L58.84 41.16 Z";

struct Variant {
    name: &'static str,
    tile: &'static str,
    blades: [&'static str; 6],
    dot: &'static str,
    ring: &'static str,
}

const LIGHT_BLADES: [&str; 6] = [
    "#D6CFC1", "#EAE5DA", "#FFFDF8", "#F6F2EA", "#E2DCD0", "#CDC5B6",
];
const DARK_BLADES: [&str; 6] = [
    "#171B2A", "#262C3E", "#3A4258", "#2F364A", "#1E2334", "#11141F",
];

/// Ink is the app icon everywhere; the others are for light docks,
/// marketing and system tinting.
const INK: Variant = Variant {
    name: "ink",
    tile: "#0E1322",
    blades: LIGHT_BLADES,
    dot: "#FFB224",
    ring: "rgba(255,255,255,0.10)",
};
const VARIANTS: [Variant; 4] = [
    INK,
    Variant {
        name: "paper",
        tile: "#F6F5F1",
        blades: DARK_BLADES,
        dot: "#FFB224",
        ring: "rgba(14,19,34,0.10)",
    },
    Variant {
        name: "photon",
        tile: "#FFB224",
        blades: DARK_BLADES,
        dot: "#FFFDF8",
        ring: "rgba(14,19,34,0.10)",
    },
    Variant {
        name: "mono",
        tile: "#161616",
        blades: [
            "rgba(255,255,255,0.62)",
            "rgba(255,255,255,0.80)",
            "#FFFFFF",
            "rgba(255,255,255,0.92)",
            "rgba(255,255,255,0.72)",
            "rgba(255,255,255,0.55)",
        ],
        dot: "#FFFFFF",
        ring: "rgba(255,255,255,0.10)",
    },
];

/// Seam width in tile units for a tile shown `size` px (or points) wide.
fn seam(size: u32) -> f32 {
    match size {
        0..=32 => 1.6,
        33..=64 => 1.2,
        _ => 0.9,
    }
}

/// How much of the canvas the tile fills.
#[derive(Clone, Copy)]
enum Layout {
    /// Edge to edge (Windows, Linux, the web).
    FullBleed,
    /// Apple's macOS grid: an 824 px tile centred on a 1024 px canvas.
    MacOs,
}

fn svg(variant: &Variant, seam: f32, layout: Layout) -> String {
    let transform = match layout {
        Layout::FullBleed => String::new(),
        Layout::MacOs => {
            let scale = 824.0 / 1024.0;
            let offset = 100.0 * (1.0 - scale) / 2.0;
            format!(r#" transform="translate({offset} {offset}) scale({scale})""#)
        }
    };
    let mut blades = String::new();
    for (path, fill) in BLADES.iter().zip(variant.blades) {
        blades.push_str(&format!("    <path d=\"{path}\" fill=\"{fill}\"/>\n"));
    }
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="1024" height="1024">
  <title>Photonation</title>
  <g{transform}>
    <rect width="100" height="100" rx="22.4" fill="{tile}"/>
{blades}    <path d="{SEAMS}" fill="none" stroke="{tile}" stroke-width="{seam}" stroke-linecap="round"/>
    <path d="{OPENING}" fill="{tile}"/>
    <circle cx="50" cy="50" r="5" fill="{dot}"/>
    <rect x="0.25" y="0.25" width="99.5" height="99.5" rx="22.15" fill="none" stroke="{ring}" stroke-width="0.5"/>
  </g>
</svg>
"##,
        tile = variant.tile,
        dot = variant.dot,
        ring = variant.ring,
    )
}

/// The ink icon as a `size` px PNG, seams set for `display` px or points.
fn png(layout: Layout, size: u32, display: u32) -> Vec<u8> {
    render(layout, size, display)
        .encode_png()
        .expect("PNG encodes")
}

/// The ink icon as a 32-bit BMP icon image: a bottom-up BGRA DIB at double
/// height (colour, then an all-clear AND mask; the alpha channel masks).
fn ico_bitmap(size: u32) -> Vec<u8> {
    let pixmap = render(Layout::FullBleed, size, size);
    let side = size as usize;
    let mask_row = side.div_ceil(32) * 4;
    let image_size = side * side * 4 + mask_row * side;
    let mut out = Vec::with_capacity(40 + image_size);
    out.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER size
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&(2 * size as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // uncompressed
    out.extend_from_slice(&u32::try_from(image_size).expect("small").to_le_bytes());
    out.extend_from_slice(&[0; 16]); // resolution and palette: unused
    for row in pixmap.pixels().chunks(side).rev() {
        for pixel in row {
            let color = pixel.demultiply();
            out.extend_from_slice(&[color.blue(), color.green(), color.red(), color.alpha()]);
        }
    }
    out.resize(out.len() + mask_row * side, 0);
    out
}

fn render(layout: Layout, size: u32, display: u32) -> resvg::tiny_skia::Pixmap {
    let tile = match layout {
        Layout::FullBleed => display,
        Layout::MacOs => display * 824 / 1024,
    };
    let source = svg(&INK, seam(tile), layout);
    let tree = resvg::usvg::Tree::from_str(&source, &resvg::usvg::Options::default())
        .expect("the icon SVG parses");
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).expect("non-zero size");
    let scale = size as f32 / tree.size().width();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
}

/// A Windows icon: 32-bit bitmaps below 256 px, which every Windows API
/// reads, and PNG at 256 px, as Windows' own icons do.
fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&size| {
            if size >= 256 {
                png(Layout::FullBleed, size, size)
            } else {
                ico_bitmap(size)
            }
        })
        .collect();
    let count = u16::try_from(sizes.len()).expect("few sizes");
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * sizes.len();
    for (&size, image) in sizes.iter().zip(&images) {
        // 256 is stored as 0.
        let side = u8::try_from(size).unwrap_or(0);
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&u32::try_from(image.len()).expect("small").to_le_bytes());
        out.extend_from_slice(&u32::try_from(offset).expect("small").to_le_bytes());
        offset += image.len();
    }
    for image in images {
        out.extend_from_slice(&image);
    }
    out
}

/// A macOS icon: PNG entries by type code, pixel size and point size.
fn icns() -> Vec<u8> {
    const ENTRIES: [(&[u8; 4], u32, u32); 10] = [
        (b"icp4", 16, 16),
        (b"ic11", 32, 16),
        (b"icp5", 32, 32),
        (b"ic12", 64, 32),
        (b"ic07", 128, 128),
        (b"ic13", 256, 128),
        (b"ic08", 256, 256),
        (b"ic14", 512, 256),
        (b"ic09", 512, 512),
        (b"ic10", 1024, 512),
    ];
    let mut body = Vec::new();
    for (code, pixels, points) in ENTRIES {
        let image = png(Layout::MacOs, pixels, points);
        body.extend_from_slice(code);
        body.extend_from_slice(&u32::try_from(image.len() + 8).expect("small").to_be_bytes());
        body.extend_from_slice(&image);
    }
    let mut out = b"icns".to_vec();
    out.extend_from_slice(&u32::try_from(body.len() + 8).expect("small").to_be_bytes());
    out.extend_from_slice(&body);
    out
}

fn write(directory: &Path, name: &str, bytes: impl AsRef<[u8]>) {
    let path = directory.join(name);
    std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    println!("wrote {}", path.display());
}

fn main() {
    let directory: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icon");
    std::fs::create_dir_all(&directory).expect("create assets/icon");

    for variant in &VARIANTS {
        let name = match variant.name {
            "ink" => "photonation.svg".to_owned(),
            other => format!("photonation-{other}.svg"),
        };
        write(
            &directory,
            &name,
            svg(variant, seam(1024), Layout::FullBleed),
        );
    }
    write(
        &directory,
        "photonation-macos.svg",
        svg(&INK, seam(1024), Layout::MacOs),
    );
    // The window icon (Windows, Linux) and a large one for documentation.
    write(
        &directory,
        "photonation-256.png",
        png(Layout::FullBleed, 256, 256),
    );
    write(
        &directory,
        "photonation-1024.png",
        png(Layout::FullBleed, 1024, 1024),
    );
    // The Dock icon while the app runs unbundled on macOS.
    write(
        &directory,
        "photonation-macos-512.png",
        png(Layout::MacOs, 512, 512),
    );
    write(
        &directory,
        "photonation.ico",
        ico(&[16, 20, 24, 32, 40, 48, 64, 96, 128, 256]),
    );
    write(&directory, "photonation.icns", icns());
}
