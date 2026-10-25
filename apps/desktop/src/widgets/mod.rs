// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Custom controls. Each widget draws itself with the painter for full
//! control over the look, and reports changes through `egui::Response`.

mod curve_editor;
mod histogram;
mod section;
mod segmented;
mod slider;
mod toggle;
mod wheel;

pub use curve_editor::{CurveChannel, curve_editor};
pub use histogram::histogram;
pub use section::{SectionAction, section};
pub use segmented::segmented;
pub use slider::{SliderStyle, Track, adjustment_slider};
pub use toggle::toggle;
pub use wheel::color_wheel;

use eframe::egui::{Color32, Rgba};

/// HSV (all 0..1) to a display colour.
pub fn hsv(hue: f32, saturation: f32, value: f32) -> Color32 {
    let hsva = eframe::egui::ecolor::Hsva::new(hue.rem_euclid(1.0), saturation, value, 1.0);
    Color32::from(Rgba::from(hsva))
}
