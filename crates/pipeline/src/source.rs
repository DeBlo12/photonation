// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Sensor source: normalization, white balance and demosaicing.

use crate::band::{Band, Source};
use photonation_image::{CfaColor, CfaPattern, Rect, SensorImage, SensorLayout};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demosaic {
    /// Bayer: Malvar–He–Cutler gradient-corrected linear interpolation.
    Mhc,
    /// Bayer: one RGB pixel per 2×2 quad — a quarter of the pixels, no
    /// interpolation. Used whenever the output is at most half size.
    HalfSize,
    /// Any other CFA (e.g. X-Trans): each missing colour is the mean of the
    /// nearest same-coloured samples.
    NeighbourAverage,
    /// Already-demosaiced linear RGB: normalization only.
    Linear,
}

impl Demosaic {
    /// The best method for a layout at a given downscale.
    pub fn choose(layout: &SensorLayout, half_size_is_enough: bool) -> Demosaic {
        match layout {
            SensorLayout::Cfa(pattern) if pattern.bayer().is_some() => {
                if half_size_is_enough {
                    Demosaic::HalfSize
                } else {
                    Demosaic::Mhc
                }
            }
            SensorLayout::Cfa(_) => Demosaic::NeighbourAverage,
            SensorLayout::LinearRgb => Demosaic::Linear,
        }
    }

    /// Rows/columns of context read around each pixel.
    fn halo(self) -> usize {
        match self {
            Demosaic::Mhc | Demosaic::NeighbourAverage => 2,
            Demosaic::HalfSize | Demosaic::Linear => 0,
        }
    }
}

/// Produces white-balanced camera RGB for a region of the sensor.
pub struct SensorSource<'a> {
    sensor: &'a SensorImage,
    region: Rect,
    mode: Demosaic,
    levels: [ChannelLevels; 3],
}

/// Maps a raw sample of one colour channel to a white-balanced value with 1.0 = clip.
#[derive(Clone, Copy)]
struct ChannelLevels {
    black: f32,
    /// White-balance multiplier ÷ (white − black).
    gain: f32,
}

impl ChannelLevels {
    /// Clipped at 1.0 so every channel saturates at the same value (no
    /// coloured highlights).
    #[inline(always)]
    fn normalize(self, raw: u16) -> f32 {
        ((raw as f32 - self.black) * self.gain).min(1.0)
    }
}

impl<'a> SensorSource<'a> {
    /// `wb_multipliers` are normalized so the smallest is 1, which makes 1.0
    /// the common clip point of all channels.
    pub fn new(
        sensor: &'a SensorImage,
        region: Rect,
        wb_multipliers: [f32; 3],
        mode: Demosaic,
    ) -> SensorSource<'a> {
        let region = match mode {
            Demosaic::HalfSize => align_to_quads(region),
            _ => region,
        };
        let levels = std::array::from_fn(|channel| {
            let black = sensor.black[channel];
            ChannelLevels {
                black,
                gain: wb_multipliers[channel] / (sensor.white - black).max(1.0),
            }
        });
        SensorSource {
            sensor,
            region,
            mode,
            levels,
        }
    }

    /// The sensor area this source reads: the requested region, shrunk to
    /// whole 2×2 quads for half-size demosaicing.
    pub fn region(&self) -> Rect {
        self.region
    }

    /// Per channel: black level and gain (see [`crate::plan::RenderPlan`]).
    pub fn levels(&self) -> ([f32; 3], [f32; 3]) {
        (
            self.levels.map(|level| level.black),
            self.levels.map(|level| level.gain),
        )
    }

    fn pattern(&self) -> &CfaPattern {
        match &self.sensor.layout {
            SensorLayout::Cfa(pattern) => pattern,
            // Constructed only with a demosaic mode matching the layout.
            SensorLayout::LinearRgb => unreachable!("CFA demosaic on linear data"),
        }
    }

    /// Colour and normalized value of the CFA sample at `(x, y)`.
    #[inline(always)]
    fn cfa_sample(&self, pattern: &CfaPattern, x: usize, y: usize) -> (CfaColor, f32) {
        let color = pattern.color_at(y, x);
        (
            color,
            self.levels[color as usize].normalize(self.sensor.sample(x, y, 0)),
        )
    }

    fn render_linear(&self, rows: Range<usize>) -> Band {
        let region = self.region;
        let mut band = Band::new(rows.start, region.w, rows.len());
        for out_y in rows {
            let row = band.row_mut(out_y);
            for (out_x, pixel) in row.chunks_exact_mut(3).enumerate() {
                for (channel, value) in pixel.iter_mut().enumerate() {
                    let raw = self
                        .sensor
                        .sample(region.x + out_x, region.y + out_y, channel);
                    *value = self.levels[channel].normalize(raw).max(0.0);
                }
            }
        }
        band
    }

    fn render_half_size(&self, rows: Range<usize>) -> Band {
        let pattern = self.pattern();
        let (width, _) = self.size();
        let mut band = Band::new(rows.start, width, rows.len());
        for out_y in rows {
            let sensor_y = self.region.y + out_y * 2;
            let row = band.row_mut(out_y);
            for (out_x, pixel) in row.chunks_exact_mut(3).enumerate() {
                let sensor_x = self.region.x + out_x * 2;
                let mut rgb = [0.0f32; 3];
                for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    let (color, value) = self.cfa_sample(pattern, sensor_x + dx, sensor_y + dy);
                    rgb[color as usize] += value;
                }
                rgb[1] *= 0.5; // two green sites per quad
                pixel.copy_from_slice(&rgb.map(|v| v.max(0.0)));
            }
        }
        band
    }

    /// Normalized mosaic for `rows` with a mirrored halo on every side,
    /// plus the true colour of each padded sample (mirroring can break the
    /// pattern period for non-Bayer CFAs).
    fn padded_mosaic(&self, rows: &Range<usize>) -> PaddedMosaic {
        let pattern = self.pattern();
        let halo = self.mode.halo();
        let region = self.region;
        let stride = region.w + 2 * halo;
        let height = rows.len() + 2 * halo;
        let mut samples = vec![0.0f32; stride * height];
        let mut colors = vec![CfaColor::Green; stride * height];
        for pad_y in 0..height {
            let region_y = mirror(
                rows.start as isize + pad_y as isize - halo as isize,
                region.h,
            );
            for pad_x in 0..stride {
                let region_x = mirror(pad_x as isize - halo as isize, region.w);
                let (color, value) =
                    self.cfa_sample(pattern, region.x + region_x, region.y + region_y);
                samples[pad_y * stride + pad_x] = value;
                colors[pad_y * stride + pad_x] = color;
            }
        }
        PaddedMosaic {
            samples,
            colors,
            stride,
        }
    }

    /// Runs `demosaic_pixel` for every output pixel of `rows`.
    fn render_cfa(
        &self,
        rows: Range<usize>,
        demosaic_pixel: impl Fn(&Neighbourhood) -> [f32; 3],
    ) -> Band {
        let halo = self.mode.halo();
        let mosaic = self.padded_mosaic(&rows);
        let mut band = Band::new(rows.start, self.region.w, rows.len());
        for (band_y, out_y) in rows.enumerate() {
            let row_start = (band_y + halo) * mosaic.stride + halo;
            for (out_x, pixel) in band.row_mut(out_y).chunks_exact_mut(3).enumerate() {
                let rgb = demosaic_pixel(&Neighbourhood {
                    mosaic: &mosaic,
                    center: row_start + out_x,
                });
                for (out, value) in pixel.iter_mut().zip(rgb) {
                    *out = value.max(0.0);
                }
            }
        }
        band
    }
}

/// Malvar–He–Cutler for one Bayer site.
fn mhc_pixel(site: &Neighbourhood) -> [f32; 3] {
    let mut rgb = [0.0; 3];
    match site.color(0, 0) {
        CfaColor::Green => {
            // Which chroma channel sits left/right of this green site.
            let horizontal = site.color(0, 1) as usize;
            rgb[1] = site.at(0, 0);
            rgb[horizontal] = site.chroma_at_green_horizontal();
            rgb[2 - horizontal] = site.chroma_at_green_vertical();
        }
        chroma => {
            let chroma = chroma as usize;
            rgb[chroma] = site.at(0, 0);
            rgb[1] = site.green_at_chroma();
            rgb[2 - chroma] = site.opposite_chroma();
        }
    }
    rgb
}

/// Mean of same-coloured samples within distance 1, widening to 2 if a
/// colour is absent nearby.
fn neighbour_average_pixel(site: &Neighbourhood) -> [f32; 3] {
    let own = site.color(0, 0) as usize;
    let mut rgb = [0.0; 3];
    rgb[own] = site.at(0, 0);
    for channel in (0..3).filter(|&c| c != own) {
        rgb[channel] = [1isize, 2]
            .iter()
            .find_map(|&reach| {
                let (mut sum, mut count) = (0.0, 0);
                for dy in -reach..=reach {
                    for dx in -reach..=reach {
                        if site.color(dy, dx) as usize == channel {
                            sum += site.at(dy, dx);
                            count += 1;
                        }
                    }
                }
                (count > 0).then(|| sum / count as f32)
            })
            .unwrap_or(0.0);
    }
    rgb
}

struct PaddedMosaic {
    samples: Vec<f32>,
    colors: Vec<CfaColor>,
    stride: usize,
}

/// The 5×5 window around one mosaic site, with the Malvar–He–Cutler
/// kernels (coefficients ×8, from the paper).
struct Neighbourhood<'m> {
    mosaic: &'m PaddedMosaic,
    center: usize,
}

impl Neighbourhood<'_> {
    #[inline(always)]
    fn index(&self, dy: isize, dx: isize) -> usize {
        (self.center as isize + dy * self.mosaic.stride as isize + dx) as usize
    }

    #[inline(always)]
    fn at(&self, dy: isize, dx: isize) -> f32 {
        self.mosaic.samples[self.index(dy, dx)]
    }

    #[inline(always)]
    fn color(&self, dy: isize, dx: isize) -> CfaColor {
        self.mosaic.colors[self.index(dy, dx)]
    }

    #[inline(always)]
    fn cross(&self) -> f32 {
        self.at(-1, 0) + self.at(1, 0) + self.at(0, -1) + self.at(0, 1)
    }

    #[inline(always)]
    fn diagonals(&self) -> f32 {
        self.at(-1, -1) + self.at(-1, 1) + self.at(1, -1) + self.at(1, 1)
    }

    #[inline(always)]
    fn two_away_horizontal(&self) -> f32 {
        self.at(0, -2) + self.at(0, 2)
    }

    #[inline(always)]
    fn two_away_vertical(&self) -> f32 {
        self.at(-2, 0) + self.at(2, 0)
    }

    /// Green at a red or blue site.
    fn green_at_chroma(&self) -> f32 {
        let axial = self.two_away_horizontal() + self.two_away_vertical();
        (4.0 * self.at(0, 0) + 2.0 * self.cross() - axial) * 0.125
    }

    /// Blue at a red site, or red at a blue site.
    fn opposite_chroma(&self) -> f32 {
        let axial = self.two_away_horizontal() + self.two_away_vertical();
        (6.0 * self.at(0, 0) + 2.0 * self.diagonals() - 1.5 * axial) * 0.125
    }

    /// The chroma channel whose sites are this green site's left/right neighbours.
    fn chroma_at_green_horizontal(&self) -> f32 {
        let neighbours = self.at(0, -1) + self.at(0, 1);
        (5.0 * self.at(0, 0) + 4.0 * neighbours - self.diagonals() - self.two_away_horizontal()
            + 0.5 * self.two_away_vertical())
            * 0.125
    }

    /// The chroma channel whose sites are this green site's up/down neighbours.
    fn chroma_at_green_vertical(&self) -> f32 {
        let neighbours = self.at(-1, 0) + self.at(1, 0);
        (5.0 * self.at(0, 0) + 4.0 * neighbours - self.diagonals() - self.two_away_vertical()
            + 0.5 * self.two_away_horizontal())
            * 0.125
    }
}

/// Reflects an index into `0..len` without repeating the edge sample.
fn mirror(index: isize, len: usize) -> usize {
    let last = len as isize - 1;
    let reflected = if index < 0 {
        -index
    } else if index > last {
        2 * last - index
    } else {
        index
    };
    reflected.clamp(0, last) as usize
}

/// Shrinks a region to even coordinates so each 2×2 quad holds a
/// full CFA period.
fn align_to_quads(region: Rect) -> Rect {
    let (x, y) = (region.x & !1, region.y & !1);
    let width = ((region.x + region.w - x) & !1).max(2);
    let height = ((region.y + region.h - y) & !1).max(2);
    Rect::new(x, y, width, height)
}

impl Source for SensorSource<'_> {
    fn size(&self) -> (usize, usize) {
        match self.mode {
            Demosaic::HalfSize => (self.region.w / 2, self.region.h / 2),
            _ => (self.region.w, self.region.h),
        }
    }

    fn render(&self, rows: Range<usize>) -> Band {
        match self.mode {
            Demosaic::Mhc => self.render_cfa(rows, mhc_pixel),
            Demosaic::NeighbourAverage => self.render_cfa(rows, neighbour_average_pixel),
            Demosaic::HalfSize => self.render_half_size(rows),
            Demosaic::Linear => self.render_linear(rows),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_reflects_without_repeating_edges() {
        let reflected: Vec<usize> = (-2..7).map(|i| mirror(i, 5)).collect();
        assert_eq!(reflected, vec![2, 1, 0, 1, 2, 3, 4, 3, 2]);
    }

    #[test]
    fn quads_stay_inside_and_even() {
        let aligned = align_to_quads(Rect::new(3, 5, 11, 7));
        assert_eq!(
            (aligned.x % 2, aligned.y % 2, aligned.w % 2, aligned.h % 2),
            (0, 0, 0, 0)
        );
        assert!(aligned.x + aligned.w <= 14 && aligned.y + aligned.h <= 12);
    }
}
