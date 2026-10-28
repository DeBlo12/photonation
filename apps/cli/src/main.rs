// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use photonation_color::ColorSpace;
use photonation_develop::{Settings, WhiteBalance, load_photo_settings, sidecar_path};
use photonation_engine::{ExportOptions, MetadataOptions, MetadataPreset};
use photonation_export::Format;
use photonation_image::BitDepth;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "photonation",
    version,
    about = "Non-destructive raw photo developer"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show camera, exposure and colour information for raw files.
    Info { inputs: Vec<PathBuf> },
    /// Develop raw files and write JPEG, PNG or TIFF.
    Export(ExportArgs),
    /// Create or update the develop sidecar next to each raw file.
    Edit {
        inputs: Vec<PathBuf>,
        #[command(flatten)]
        adjust: Adjustments,
        /// Start from defaults instead of the existing sidecar.
        #[arg(long)]
        reset: bool,
    },
}

#[derive(Args)]
struct ExportArgs {
    /// Raw files or directories of raw files.
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
    /// Output directory (default: next to each input).
    #[arg(short, long)]
    output: Option<PathBuf>,
    #[arg(short, long, value_enum, default_value_t = FormatArg::Jpeg)]
    format: FormatArg,
    /// JPEG quality, 1–100.
    #[arg(short, long, default_value_t = 92, value_parser = clap::value_parser!(u8).range(1..=100))]
    quality: u8,
    /// Longest edge in pixels (default: full resolution).
    #[arg(short, long)]
    long_edge: Option<usize>,
    /// Output file name without extension: `{name}` is the original's
    /// name, `{n}` the photo's number (e.g. "Trip {n}").
    #[arg(short, long, default_value = photonation_engine::NAME_TOKEN)]
    name: String,
    #[arg(long, value_enum, default_value_t = SpaceArg::Srgb)]
    color_space: SpaceArg,
    /// Bits per channel (PNG and TIFF; JPEG is always 8).
    #[arg(long, value_parser = ["8", "16"])]
    depth: Option<String>,
    /// Ignore develop sidecars.
    #[arg(long)]
    no_sidecar: bool,
    /// Develop settings JSON (a sidecar or a bare settings object) applied to every input.
    #[arg(long)]
    settings: Option<PathBuf>,
    /// Replace existing output files.
    #[arg(long)]
    overwrite: bool,
    #[command(flatten)]
    metadata: MetadataArgs,
    #[command(flatten)]
    adjust: Adjustments,
}

/// What metadata the exported files carry, as EXIF.
#[derive(Args)]
struct MetadataArgs {
    /// Metadata to write: camera, capture and copyright (`all`), only
    /// artist and copyright (`copyright`), or nothing (`none`).
    #[arg(long, value_enum, default_value_t = MetadataArg::All)]
    metadata: MetadataArg,
    /// Also write the GPS position (left out by default for privacy).
    #[arg(long)]
    location: bool,
    /// Artist to write instead of the raw file's.
    #[arg(long)]
    creator: Option<String>,
    /// Copyright notice to write instead of the raw file's.
    #[arg(long)]
    copyright: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
enum MetadataArg {
    All,
    Copyright,
    None,
}

impl MetadataArgs {
    fn options(&self) -> MetadataOptions {
        let mut options = MetadataOptions {
            creator: self.creator.clone().unwrap_or_default(),
            copyright_notice: self.copyright.clone().unwrap_or_default(),
            ..MetadataOptions::default()
        };
        options.apply(match self.metadata {
            MetadataArg::All => MetadataPreset::All,
            MetadataArg::Copyright => MetadataPreset::CopyrightOnly,
            MetadataArg::None => MetadataPreset::None,
        });
        options.location = self.location;
        options
    }
}

/// Slider overrides; applied on top of the sidecar or settings file.
#[derive(Args, Default)]
struct Adjustments {
    /// Exposure in stops.
    #[arg(long, allow_hyphen_values = true)]
    exposure: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    contrast: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    highlights: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    shadows: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    whites: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    blacks: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    vibrance: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    saturation: Option<f32>,
    /// White balance temperature in kelvin (switches to custom white balance).
    #[arg(long)]
    temperature: Option<f64>,
    /// White balance tint, positive = magenta.
    #[arg(long, allow_hyphen_values = true)]
    tint: Option<f64>,
    /// Sharpening amount, 0–150.
    #[arg(long)]
    sharpen: Option<f32>,
}

#[derive(Clone, Copy, ValueEnum)]
enum FormatArg {
    Jpeg,
    Png,
    Tiff,
}

#[derive(Clone, Copy, ValueEnum)]
enum SpaceArg {
    Srgb,
    DisplayP3,
    AdobeRgb,
    Prophoto,
}

impl From<SpaceArg> for ColorSpace {
    fn from(s: SpaceArg) -> ColorSpace {
        match s {
            SpaceArg::Srgb => ColorSpace::Srgb,
            SpaceArg::DisplayP3 => ColorSpace::DisplayP3,
            SpaceArg::AdobeRgb => ColorSpace::AdobeRgb,
            SpaceArg::Prophoto => ColorSpace::ProphotoRgb,
        }
    }
}

impl Adjustments {
    fn apply(&self, s: &mut Settings, as_shot: impl FnOnce() -> (f64, f64)) {
        let sliders = [
            (self.exposure, &mut s.exposure),
            (self.contrast, &mut s.contrast),
            (self.highlights, &mut s.highlights),
            (self.shadows, &mut s.shadows),
            (self.whites, &mut s.whites),
            (self.blacks, &mut s.blacks),
            (self.vibrance, &mut s.vibrance),
            (self.saturation, &mut s.saturation),
        ];
        for (value, field) in sliders {
            if let Some(v) = value {
                *field = v;
            }
        }
        if let Some(v) = self.sharpen {
            s.sharpening.amount = v;
        }
        if self.temperature.is_some() || self.tint.is_some() {
            let (t0, tint0) = match s.white_balance {
                WhiteBalance::Custom { temperature, tint } => (temperature, tint),
                WhiteBalance::AsShot => as_shot(),
            };
            s.white_balance = WhiteBalance::Custom {
                temperature: self.temperature.unwrap_or(t0),
                tint: self.tint.unwrap_or(tint0),
            };
        }
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { inputs } => info(&collect_inputs(&inputs)?),
        Command::Export(args) => export(&args),
        Command::Edit {
            inputs,
            adjust,
            reset,
        } => edit(&collect_inputs(&inputs)?, &adjust, reset),
    }
}

/// Expands directories (non-recursively) into the raw files they contain.
fn collect_inputs(inputs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for input in inputs {
        if input.is_dir() {
            let mut found: Vec<PathBuf> = std::fs::read_dir(input)
                .with_context(|| format!("reading {}", input.display()))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file() && photonation_raw::is_supported(p))
                .collect();
            found.sort();
            files.extend(found);
        } else {
            files.push(input.clone());
        }
    }
    if files.is_empty() {
        bail!("no raw files found");
    }
    Ok(files)
}

fn info(files: &[PathBuf]) -> Result<()> {
    for path in files {
        let img = photonation_raw::decode_file(path)?;
        let i = &img.info;
        println!("{}", path.display());
        println!("  camera       {} {}", i.make, i.model);
        if let Some(lens) = &i.lens {
            println!("  lens         {lens}");
        }
        let mut exposure = Vec::new();
        if let Some(t) = i.exposure_time {
            exposure.push(if t < 1.0 {
                format!("1/{:.0} s", 1.0 / t)
            } else {
                format!("{t:.1} s")
            });
        }
        if let Some(f) = i.f_number {
            exposure.push(format!("f/{f:.1}"));
        }
        if let Some(iso) = i.iso {
            exposure.push(format!("ISO {iso}"));
        }
        if let Some(fl) = i.focal_length {
            exposure.push(format!("{fl:.0} mm"));
        }
        if let Some(bias) = i.exposure_bias.filter(|b| *b != 0.0) {
            exposure.push(format!("{bias:+.1} EV"));
        }
        if !exposure.is_empty() {
            println!("  exposure     {}", exposure.join("  "));
        }
        if let Some(d) = &i.date_time {
            println!(
                "  taken        {d} {}",
                i.time_offset.as_deref().unwrap_or("")
            );
        }
        if let Some(artist) = &i.artist {
            println!("  artist       {artist}");
        }
        if let Some(copyright) = &i.copyright {
            println!("  copyright    {copyright}");
        }
        if let Some(gps) = &i.gps {
            println!("  location     {:.5}, {:.5}", gps.latitude, gps.longitude);
        }
        let (w, h) = img.orientation.display_size(img.crop.w, img.crop.h);
        println!(
            "  size         {w} × {h} ({:.1} MP), sensor {} × {}",
            (w * h) as f64 / 1e6,
            img.width,
            img.height
        );
        println!("  orientation  {:?}", img.orientation);
        let cam = photonation_pipeline::CameraModel::new(&img, &WhiteBalance::AsShot);
        println!(
            "  as shot WB   {:.0} K, tint {:+.0}",
            cam.temperature, cam.tint
        );
        println!("  levels       black {:?}, white {}", img.black, img.white);
    }
    Ok(())
}

fn load_settings_file(path: &Path) -> Result<Settings> {
    if let Some(s) = Settings::load(path).ok().flatten() {
        return Ok(s);
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

fn export(args: &ExportArgs) -> Result<()> {
    let files = collect_inputs(&args.inputs)?;
    let format = match args.format {
        FormatArg::Jpeg => Format::Jpeg {
            quality: args.quality,
        },
        FormatArg::Png => Format::Png,
        FormatArg::Tiff => Format::Tiff,
    };
    let bit_depth = match (args.depth.as_deref(), args.format) {
        (Some("16"), FormatArg::Jpeg) => bail!("JPEG only supports 8 bits per channel"),
        (Some("16"), _) | (None, FormatArg::Tiff) => BitDepth::Sixteen,
        _ => BitDepth::Eight,
    };
    let base_settings = args
        .settings
        .as_deref()
        .map(load_settings_file)
        .transpose()?;
    if let Some(dir) = &args.output {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let options = ExportOptions {
        format,
        bit_depth,
        color_space: args.color_space.into(),
        long_edge: args.long_edge,
        file_name: args.name.clone(),
        metadata: args.metadata.options(),
    };

    let started = Instant::now();
    let mut failures = 0;
    for (n, path) in files.iter().enumerate() {
        let dir = args
            .output
            .clone()
            .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).to_path_buf());
        let target = photonation_engine::output_path(path, &dir, &options, n + 1, files.len());
        print!("[{}/{}] {} ", n + 1, files.len(), path.display());
        if target.exists() && !args.overwrite {
            println!(
                "skipped ({} exists; --overwrite to replace)",
                target.display()
            );
            continue;
        }
        let settings = match (&base_settings, args.no_sidecar) {
            (Some(s), _) => Some(s.clone()),
            (None, true) => Some(Settings::default()),
            (None, false) => None, // the engine reads the sidecar
        };
        let result = photonation_engine::export_photo(
            path,
            &target,
            settings.as_ref(),
            &options,
            |s, sensor| args.adjust.apply(s, || as_shot(sensor)),
        );
        match result {
            Ok(report) => println!(
                "→ {}  ({}×{}, decode {} ms, render {} ms, encode {} ms)",
                target.display(),
                report.size.0,
                report.size.1,
                report.decode.as_millis(),
                report.render.as_millis(),
                report.encode.as_millis()
            ),
            Err(e) => {
                failures += 1;
                println!("FAILED\n  {e:#}");
            }
        }
    }
    println!(
        "{} file(s) in {:.2} s",
        files.len(),
        started.elapsed().as_secs_f64()
    );
    if failures > 0 {
        bail!("{failures} file(s) failed");
    }
    Ok(())
}

fn as_shot(img: &photonation_image::SensorImage) -> (f64, f64) {
    let cam = photonation_pipeline::CameraModel::new(img, &WhiteBalance::AsShot);
    (cam.temperature, cam.tint)
}

fn edit(files: &[PathBuf], adjust: &Adjustments, reset: bool) -> Result<()> {
    for path in files {
        let sidecar = sidecar_path(path);
        let mut settings = if reset {
            Settings::default()
        } else {
            load_photo_settings(path)?
                .map(|loaded| loaded.settings)
                .unwrap_or_default()
        };
        adjust.apply(&mut settings, || {
            photonation_raw::decode_file(path)
                .map(|img| as_shot(&img))
                .unwrap_or((5500.0, 0.0))
        });
        settings.sanitized().save(&sidecar)?;
        println!("{}", sidecar.display());
    }
    Ok(())
}
