// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use photonation_color::ColorSpace;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BitDepth {
    #[default]
    Eight,
    Sixteen,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pixels {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

/// Display-referred, encoded, interleaved RGB.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputImage {
    pub width: usize,
    pub height: usize,
    pub color_space: ColorSpace,
    pub pixels: Pixels,
}

impl OutputImage {
    pub fn bit_depth(&self) -> BitDepth {
        match self.pixels {
            Pixels::U8(_) => BitDepth::Eight,
            Pixels::U16(_) => BitDepth::Sixteen,
        }
    }
}
