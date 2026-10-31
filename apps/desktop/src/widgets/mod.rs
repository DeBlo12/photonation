// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Custom controls. Each widget draws itself with the painter for full
//! control over the look, and reports changes through `egui::Response`.

mod brand_mark;
mod curve_editor;
mod histogram;
mod icon;
mod section;
mod segmented;
mod slider;
mod toggle;
mod wheel;

pub use brand_mark::paint_brand_mark;
pub use curve_editor::{CurveChannel, curve_editor};
pub use histogram::histogram;
pub use icon::{Icon, paint_icon};
pub use section::{SectionAction, section};
pub use segmented::segmented;
pub use slider::{SliderStyle, Track, adjustment_slider};
pub use toggle::{TOGGLE_SIZE, toggle};
pub use wheel::color_wheel;

use eframe::egui::{Color32, Rgba};

/// HSV (all 0..1) to a display colour.
pub fn hsv(hue: f32, saturation: f32, value: f32) -> Color32 {
    let hsva = eframe::egui::ecolor::Hsva::new(hue.rem_euclid(1.0), saturation, value, 1.0);
    Color32::from(Rgba::from(hsva))
}
