// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The mask editor: a floating window listing the photo's masks, with the
//! selected mask's shape options, invert switch and local adjustments.
//! Shapes are edited by dragging their handles in the viewer
//! (`mask_handles.rs`).

use crate::app::{PhotonationApp, ViewState};
use crate::panels::adjustments::{control_slider, style_for};
use crate::panels::tool_button;
use crate::theme;
use crate::widgets::{self, SliderStyle, adjustment_slider};
use eframe::egui::{self, Align, Layout, RichText, Ui};
use photonation_develop::{
    LOCAL_CONTROLS, LinearGradient, LocalAdjustments, MAX_MASK_NAME_CHARS, MAX_MASKS, Mask,
    MaskShape, RadialGradient,
};

const WIDTH: f32 = 280.0;

pub fn show(app: &mut PhotonationApp, ctx: &egui::Context) {
    let mut open = app.view.show_masks;
    egui::Window::new("Masks")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(WIDTH)
        .default_pos(egui::pos2(24.0, theme::TOOLBAR_HEIGHT + 24.0))
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            contents(&mut app.settings.masks, &mut app.view, ui);
        });
    app.view.show_masks = open;
}

fn contents(masks: &mut Vec<Mask>, view: &mut ViewState, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let room = masks.len() < MAX_MASKS;
        ui.add_enabled_ui(room, |ui| {
            if tool_button(
                ui,
                "+ Linear",
                false,
                "Linear gradient: full effect up to the first line, fading out by the last",
            )
            .clicked()
            {
                add(masks, view, MaskShape::Linear(LinearGradient::default()));
            }
            if tool_button(
                ui,
                "+ Radial",
                false,
                "Radial gradient: full effect inside the ellipse, fading out at its edge",
            )
            .clicked()
            {
                add(masks, view, MaskShape::Radial(RadialGradient::default()));
            }
        });
    });
    if masks.is_empty() {
        hint(ui, "Add a gradient, then drag its handles in the photo.");
        return;
    }
    ui.add_space(4.0);
    mask_list(masks, view, ui);

    let Some(index) = view.selected_mask.filter(|&index| index < masks.len()) else {
        hint(ui, "Select a mask to edit it.");
        return;
    };
    ui.separator();
    if selected_mask(&mut masks[index], view, ui) == MaskAction::Delete {
        remove(masks, view, index);
    }
}

fn add(masks: &mut Vec<Mask>, view: &mut ViewState, shape: MaskShape) {
    let kind = shape_name(&shape);
    masks.push(Mask::new(format!("{kind} {}", masks.len() + 1), shape));
    view.selected_mask = Some(masks.len() - 1);
    // Show what the new mask selects; its adjustments start neutral.
    view.mask_overlay = true;
}

fn remove(masks: &mut Vec<Mask>, view: &mut ViewState, index: usize) {
    masks.remove(index);
    view.selected_mask = match view.selected_mask {
        Some(selected) if selected == index => None,
        Some(selected) if selected > index => Some(selected - 1),
        other => other,
    };
}

fn shape_name(shape: &MaskShape) -> &'static str {
    match shape {
        MaskShape::Linear(_) => "Linear",
        MaskShape::Radial(_) => "Radial",
    }
}

fn shape_icon(shape: &MaskShape) -> &'static str {
    match shape {
        MaskShape::Linear(_) => "▤",
        MaskShape::Radial(_) => "◯",
    }
}

fn mask_list(masks: &mut Vec<Mask>, view: &mut ViewState, ui: &mut Ui) {
    let mut delete = None;
    for (index, mask) in masks.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            widgets::toggle(ui, &mut mask.enabled).on_hover_text("Show this mask's effect");
            let mut label = format!("{}  {}", shape_icon(&mask.shape), mask.name);
            if mask.invert {
                label.push_str("  (inverted)");
            }
            let selected = view.selected_mask == Some(index);
            if ui.selectable_label(selected, label).clicked() {
                view.selected_mask = (!selected).then_some(index);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if tool_button(ui, "×", false, "Delete mask").clicked() {
                    delete = Some(index);
                }
            });
        });
    }
    if let Some(index) = delete {
        remove(masks, view, index);
    }
}

#[derive(PartialEq)]
enum MaskAction {
    None,
    Delete,
}

fn selected_mask(mask: &mut Mask, view: &mut ViewState, ui: &mut Ui) -> MaskAction {
    ui.horizontal(|ui| {
        label(ui, "Name");
        ui.add(
            egui::TextEdit::singleline(&mut mask.name)
                .char_limit(MAX_MASK_NAME_CHARS)
                .desired_width(f32::INFINITY),
        );
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        widgets::toggle(ui, &mut mask.invert)
            .on_hover_text("Apply the adjustments outside the shape instead");
        label(ui, "Invert");
        ui.add_space(16.0);
        widgets::toggle(ui, &mut view.mask_overlay)
            .on_hover_text("Tint the area this mask affects");
        label(ui, "Show overlay");
    });
    ui.add_space(4.0);
    match &mut mask.shape {
        MaskShape::Linear(_) => hint(
            ui,
            "Drag the outer lines to set where the effect starts and ends, the middle one to move it.",
        ),
        MaskShape::Radial(radial) => {
            let defaults = RadialGradient::default();
            let whole_number = SliderStyle {
                format: |v| format!("{v:.0}"),
                ..SliderStyle::default()
            };
            adjustment_slider(
                ui,
                "Feather",
                &mut radial.feather,
                0.0..=100.0,
                defaults.feather,
                &whole_number,
            );
            // An ellipse looks the same turned by 180°; show the angle in
            // that range, but only write it back when the user changes it.
            let mut rotation = radial.rotation.rem_euclid(180.0);
            let turned = adjustment_slider(
                ui,
                "Rotation",
                &mut rotation,
                0.0..=180.0,
                defaults.rotation,
                &SliderStyle {
                    format: |v| format!("{v:.0}°"),
                    ..SliderStyle::default()
                },
            );
            if turned.changed() {
                radial.rotation = rotation;
            }
            hint(
                ui,
                "Drag the centre to move, the edge handles to resize (⇧ keeps it round).",
            );
        }
    }

    ui.add_space(6.0);
    label(ui, "Adjustments");
    for control in LOCAL_CONTROLS {
        control_slider(ui, &mut mask.adjustments, control, &style_for(control.id));
    }
    ui.add_space(6.0);
    let mut action = MaskAction::None;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!mask.adjustments.is_neutral(), egui::Button::new("Reset"))
            .clicked()
        {
            mask.adjustments = LocalAdjustments::default();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Delete Mask").clicked() {
                action = MaskAction::Delete;
            }
        });
    });
    action
}

fn label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(theme::TEXT_SECONDARY));
}

fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(theme::TEXT_TERTIARY));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_selects_and_removing_keeps_the_selection_on_its_mask() {
        let mut masks = Vec::new();
        let mut view = ViewState::default();
        add(
            &mut masks,
            &mut view,
            MaskShape::Linear(LinearGradient::default()),
        );
        add(
            &mut masks,
            &mut view,
            MaskShape::Radial(RadialGradient::default()),
        );
        add(
            &mut masks,
            &mut view,
            MaskShape::Radial(RadialGradient::default()),
        );
        assert_eq!(view.selected_mask, Some(2));
        assert_eq!(masks[1].name, "Radial 2");

        remove(&mut masks, &mut view, 0);
        assert_eq!(view.selected_mask, Some(1), "still the third mask");
        remove(&mut masks, &mut view, 1);
        assert_eq!(view.selected_mask, None, "the selected mask is gone");
    }
}
