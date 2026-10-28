// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The masks panel. While masking it takes the place of the adjustment
//! cards in the sidebar: a layer list (what each mask selects over the
//! photo, its name, shapes and amount, and a visibility switch; masks can
//! be gathered in collapsible groups, `groups.rs`; drag a row to reorder,
//! right-click for more), then the selected mask's shapes, settings and
//! local adjustments, or the selected group's settings. Shapes are edited
//! by dragging their handles in the viewer (`mask_handles.rs`). See
//! docs/DESKTOP.md.

pub mod groups;

use crate::app::{MaskSelection, PhotonationApp, ViewState};
use crate::panels::adjustments::{control_slider, style_for};
use crate::panels::mask_thumbnails::{HIDDEN_OPACITY, tint_strength};
use crate::panels::tool_button;
use crate::panels::viewer::photo_frame;
use crate::theme;
use crate::widgets::{self, SliderStyle, adjustment_slider};
use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke, StrokeKind,
    TextureId, Ui, pos2, vec2,
};
use groups::{GROUP_INDENT, LayerAction, Layers, ListRow};
use photonation_develop::{
    CombineMode, LOCAL_CONTROLS, LinearGradient, LocalAdjustments, MAX_MASK_COMPONENTS,
    MAX_MASK_GROUPS, MAX_MASK_NAME_CHARS, MAX_MASKS, Mask, MaskComponent, MaskGroup, MaskShape,
    RadialGradient, group_members,
};

const ROW_HEIGHT: f32 = 46.0;
const THUMB_HEIGHT: f32 = 34.0;
/// Thumbnail width limits, so panoramas and portraits keep a usable size.
const THUMB_WIDTH: std::ops::RangeInclusive<f32> = 26.0..=60.0;
/// Rendered mask tints are this many times the thumbnail's point size.
const THUMB_PIXELS_PER_POINT: f32 = 2.0;

pub fn show(app: &mut PhotonationApp, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 8.0;
    // A mask picked on the photo replaces a selected group.
    if app.view.selected_mask.is_some() {
        app.view.mask_groups.selected = None;
    }
    let mut action = None;
    card(ui, |ui| {
        action = header(&app.settings, &mut app.view, ui);
        if let Some(chosen) = layer_list(app, ui) {
            action = Some(chosen);
        }
    });
    if let Some(group) = app.view.mask_groups.selected
        && group < app.settings.mask_groups.len()
    {
        let members = group_members(&app.settings.masks, group).len();
        card(ui, |ui| {
            if let Some(chosen) = groups::group_settings(
                &mut app.settings.mask_groups[group],
                group,
                members,
                &mut app.view.mask_groups.focus_name,
                ui,
            ) {
                action = Some(chosen);
            }
        });
    }
    if let Some(action) = action {
        layers(app).apply(action);
    }
    let Some(selection) = app.view.selected_mask else {
        return;
    };
    let mask = &mut app.settings.masks[selection.mask];
    let mut component = selection.component;
    card(ui, |ui| shapes(mask, &mut component, ui));
    app.view.selected_mask = Some(MaskSelection {
        component,
        ..selection
    });
    card(ui, |ui| {
        layer_settings(mask, &mut app.view.mask_overlay, ui)
    });
    let mut delete = false;
    card(ui, |ui| delete = adjustments(mask, ui));
    if delete {
        layers(app).apply(LayerAction::Delete(selection.mask));
    }
}

fn layers(app: &mut PhotonationApp) -> Layers<'_> {
    Layers {
        masks: &mut app.settings.masks,
        groups: &mut app.settings.mask_groups,
        selection: &mut app.view.selected_mask,
        view: &mut app.view.mask_groups,
    }
}

pub fn card(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        content(ui);
    });
}

/// The title, Done, and the buttons that add masks and groups.
fn header(
    settings: &photonation_develop::Settings,
    view: &mut ViewState,
    ui: &mut Ui,
) -> Option<LayerAction> {
    let masks = &settings.masks;
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Masks").size(14.0).color(theme::TEXT));
        ui.label(
            RichText::new(format!("{} / {MAX_MASKS}", masks.len()))
                .size(11.0)
                .color(theme::TEXT_TERTIARY),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if tool_button(ui, "Done", false, "Back to the adjustments (M)").clicked() {
                view.show_masks = false;
            }
        });
    });
    ui.horizontal(|ui| {
        ui.add_enabled_ui(masks.len() < MAX_MASKS, |ui| {
            for shape in new_shapes() {
                if tool_button(
                    ui,
                    &format!("+ {}", shape_name(&shape)),
                    false,
                    shape_tooltip(&shape),
                )
                .clicked()
                {
                    action = Some(LayerAction::Add(shape));
                    // Show what the new mask selects; its adjustments start neutral.
                    view.mask_overlay = true;
                }
            }
        });
        let selected = view.selected_mask.map(|selection| selection.mask);
        ui.add_enabled_ui(
            selected.is_some() && settings.mask_groups.len() < MAX_MASK_GROUPS,
            |ui| {
                if tool_button(ui, "+ Group", false, "Put the selected mask in a new group")
                    .clicked()
                    && let Some(mask) = selected
                {
                    action = Some(LayerAction::NewGroup(mask));
                }
            },
        );
    });
    if masks.is_empty() {
        hint(ui, "Add a gradient, then drag its handles in the photo.");
    }
    action
}

/// One of each shape, as new masks and new shapes start.
fn new_shapes() -> [MaskShape; 2] {
    [
        MaskShape::Linear(LinearGradient::default()),
        MaskShape::Radial(RadialGradient::default()),
    ]
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

fn shape_tooltip(shape: &MaskShape) -> &'static str {
    match shape {
        MaskShape::Linear(_) => {
            "Linear gradient: full effect up to the first line, fading out by the last"
        }
        MaskShape::Radial(_) => {
            "Radial gradient: full effect inside the ellipse, fading out at its edge"
        }
    }
}

fn display_name(mask: &Mask, index: usize) -> String {
    if mask.name.trim().is_empty() {
        format!("Mask {}", index + 1)
    } else {
        mask.name.clone()
    }
}

/// "Radial", "3 shapes · inverted · 60 %", "Linear · no adjustments".
fn summary(mask: &Mask) -> String {
    let mut parts = vec![match mask.components.as_slice() {
        [only] => shape_name(&only.shape).to_owned(),
        components => format!("{} shapes", components.len()),
    }];
    if mask.invert {
        parts.push("inverted".into());
    }
    if mask.amount < 100.0 {
        parts.push(format!("{:.0} %", mask.amount));
    }
    if mask.adjustments.is_neutral() {
        parts.push("no adjustments".into());
    }
    parts.join(" · ")
}

/// The rows: masks, and group headers with their masks beneath. Returns
/// the edit the user asked for.
fn layer_list(app: &mut PhotonationApp, ui: &mut Ui) -> Option<LayerAction> {
    // What the thumbnails need: the photo's thumbnail and where it sits.
    let photo = app
        .current
        .and_then(|index| app.photos.get(index))
        .and_then(|photo| photo.thumbnail.as_ref())
        .map(|texture| texture.id());
    let frame = photo_frame(app, Rect::NOTHING);
    let aspect = frame.map_or(1.5, |frame| {
        let [width, height] = frame.crop.size();
        (width / height.max(1e-6)) as f32
    });
    let thumb_size = vec2(
        (THUMB_HEIGHT * aspect).clamp(*THUMB_WIDTH.start(), *THUMB_WIDTH.end()),
        THUMB_HEIGHT,
    );
    let tint_pixels = (thumb_size * THUMB_PIXELS_PER_POINT).round();
    let tint_size = [tint_pixels.x as usize, tint_pixels.y as usize];

    let masks = &mut app.settings.masks;
    let groups = &mut app.settings.mask_groups;
    let view = &mut app.view;
    let selected = view.selected_mask.map(|selection| selection.mask);
    let rows = groups::list_rows(masks, &view.mask_groups.collapsed);
    let mut placed = Vec::with_capacity(rows.len());
    let mut action = None;
    // (row, pointer height) while a row is dragged, and when it is dropped.
    let mut dragging = None;
    let mut dropped = None;
    for row in rows {
        let response = match row {
            ListRow::Group(group) => {
                let members = group_members(masks, group).len();
                let collapsed = view.mask_groups.collapsed.contains(&group);
                let (response, fold) = groups::group_row(
                    ui,
                    &mut groups[group],
                    group,
                    members,
                    view.mask_groups.selected == Some(group),
                    collapsed,
                );
                if fold {
                    action = Some(LayerAction::ToggleCollapsed(group));
                } else if response.double_clicked() {
                    action = Some(LayerAction::RenameGroup(group));
                } else if response.clicked() {
                    action = Some(LayerAction::SelectGroup(group));
                }
                response.context_menu(|ui| {
                    if let Some(chosen) = groups::group_menu(ui, group, collapsed) {
                        action = Some(chosen);
                        ui.close();
                    }
                });
                response
            }
            ListRow::Mask(index) => {
                let steps = [true, false].map(|up| groups::step_slot(masks, index, up));
                let rendered = masks[index].rendered(groups);
                let mask = &mut masks[index];
                let tint =
                    frame.map(|frame| view.mask_thumbnails.tint(ui.ctx(), mask, frame, tint_size));
                let picture = Picture {
                    photo,
                    tint,
                    size: thumb_size,
                    shown: rendered.enabled,
                    tint_strength: tint_strength(&rendered),
                };
                let grouped = mask.group.is_some();
                let draw = |ui: &mut Ui| {
                    layer_row(ui, &mut *mask, index, selected == Some(index), picture)
                };
                let response = if grouped {
                    ui.horizontal(|ui| {
                        ui.add_space(GROUP_INDENT - ui.spacing().item_spacing.x);
                        draw(ui)
                    })
                    .inner
                } else {
                    draw(ui)
                };
                if response.clicked() {
                    action = Some(LayerAction::Select(index));
                }
                response.context_menu(|ui| {
                    if let Some(chosen) = row_menu(ui, masks, groups, index, steps) {
                        action = Some(chosen);
                        ui.close();
                    }
                });
                response
            }
        };
        let pointer_y = response
            .interact_pointer_pos()
            .or_else(|| ui.input(|i| i.pointer.latest_pos()))
            .map(|p| p.y);
        if response.dragged() {
            dragging = pointer_y.map(|y| (row, y));
        }
        if response.drag_stopped() {
            dropped = pointer_y.map(|y| (row, y));
        }
        placed.push((row, response.rect));
    }
    view.mask_thumbnails.end_frame();
    paint_group_guides(ui, masks, &placed);
    // Targets need every row's place, so drags resolve after the list.
    if let Some((row, y)) = dropped
        && let Some(target) = groups::drop_target(masks, &placed, row, y)
        && let Some(chosen) = target.action(masks, row)
    {
        action = Some(chosen);
    }
    if let Some((row, y)) = dragging
        && let Some(target) = groups::drop_target(masks, &placed, row, y)
    {
        target.paint(ui, &placed);
        if target.action(masks, row).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    }
    if !masks.is_empty() {
        ui.add_space(2.0);
        hint(
            ui,
            "Drag a row to reorder it, or onto a group to add it; right-click for more.",
        );
    }
    action
}

/// A faint line left of each group's masks, joining them to the header.
fn paint_group_guides(ui: &Ui, masks: &[Mask], rows: &[(ListRow, Rect)]) {
    let gap = ui.spacing().item_spacing.y;
    let grouped =
        |row: &ListRow| matches!(row, ListRow::Mask(index) if masks[*index].group.is_some());
    for (_, rect) in rows.iter().filter(|(row, _)| grouped(row)) {
        let x = rect.left() - GROUP_INDENT / 2.0;
        ui.painter().vline(
            x,
            rect.top() - gap..=rect.bottom(),
            Stroke::new(1.0, theme::SEPARATOR),
        );
    }
}

/// A mask row's right-click menu. `steps` are the slots Move Up and Move
/// Down go to ([`groups::step_slot`]).
fn row_menu(
    ui: &mut Ui,
    masks: &mut [Mask],
    groups: &[MaskGroup],
    index: usize,
    steps: [Option<usize>; 2],
) -> Option<LayerAction> {
    if ui.button("Select").clicked() {
        return Some(LayerAction::Select(index));
    }
    if ui.checkbox(&mut masks[index].invert, "Inverted").changed() {
        ui.close();
    }
    if ui.button("Duplicate").clicked() {
        return Some(LayerAction::Duplicate(index));
    }
    let group = masks[index].group;
    for (step, label) in steps.into_iter().zip(["Move Up", "Move Down"]) {
        if ui
            .add_enabled(step.is_some(), egui::Button::new(label))
            .clicked()
            && let Some(to_slot) = step
        {
            return Some(LayerAction::Move {
                from: index,
                to_slot,
                group,
            });
        }
    }
    if let Some(action) = groups::mask_group_menu(ui, masks, groups, index) {
        return Some(action);
    }
    ui.separator();
    if ui.button("Delete").clicked() {
        return Some(LayerAction::Delete(index));
    }
    None
}

/// What a row's thumbnail shows.
struct Picture {
    photo: Option<TextureId>,
    tint: Option<TextureId>,
    size: egui::Vec2,
    /// Whether the mask renders (it and its group are switched on).
    shown: bool,
    /// The tint's opacity ([`tint_strength`]).
    tint_strength: f32,
}

/// One layer: thumbnail, name and summary, and a visibility switch.
fn layer_row(
    ui: &mut Ui,
    mask: &mut Mask,
    index: usize,
    selected: bool,
    picture: Picture,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), ROW_HEIGHT),
        Sense::click_and_drag(),
    );
    let painter = ui.painter();
    let radius = CornerRadius::same(theme::CONTROL_RADIUS);
    if selected {
        painter.rect(
            rect,
            radius,
            theme::ACCENT_SOFT,
            Stroke::new(1.0, theme::ACCENT),
            StrokeKind::Inside,
        );
    } else if response.hovered() || response.dragged() {
        painter.rect_filled(rect, radius, theme::CONTROL);
    }

    let thumb = Rect::from_min_size(
        pos2(rect.left() + 6.0, rect.center().y - picture.size.y / 2.0),
        picture.size,
    );
    let visible = if picture.shown { 1.0 } else { HIDDEN_OPACITY };
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    match picture.photo {
        Some(photo) => {
            painter.image(photo, thumb, uv, Color32::WHITE.gamma_multiply(visible));
        }
        None => {
            painter.rect_filled(thumb, 3.0, theme::CONTROL);
        }
    }
    if let Some(tint) = picture.tint {
        let strength = picture.tint_strength;
        painter.image(tint, thumb, uv, Color32::WHITE.gamma_multiply(strength));
    }
    painter.rect_stroke(
        thumb,
        3.0,
        Stroke::new(1.0, theme::SEPARATOR),
        StrokeKind::Outside,
    );

    let text_left = thumb.right() + 10.0;
    let text_width = rect.right() - widgets::TOGGLE_SIZE.x - 14.0 - text_left;
    let text_color = if picture.shown {
        theme::TEXT
    } else {
        theme::TEXT_TERTIARY
    };
    let name = single_line(
        painter,
        display_name(mask, index),
        theme::body(13.0),
        text_color,
        text_width,
    );
    let details = single_line(
        painter,
        summary(mask),
        theme::body(11.0),
        theme::TEXT_SECONDARY,
        text_width,
    );
    painter.galley(
        pos2(text_left, rect.center().y - name.size().y),
        name,
        text_color,
    );
    painter.galley(
        pos2(text_left, rect.center().y + 1.0),
        details,
        theme::TEXT_SECONDARY,
    );

    // A child Ui, unlike `ui.put`, leaves the list's cursor below the row.
    let toggle_rect = Rect::from_center_size(
        pos2(
            rect.right() - 8.0 - widgets::TOGGLE_SIZE.x / 2.0,
            rect.center().y,
        ),
        widgets::TOGGLE_SIZE,
    );
    let mut toggle_ui = ui.new_child(egui::UiBuilder::new().max_rect(toggle_rect));
    widgets::toggle(&mut toggle_ui, &mut mask.enabled).on_hover_text("Show this mask's effect");
    // Text is painted, so the row gets a hover tip with the full name.
    let tip = format!("{}\n{}", display_name(mask, index), summary(mask));
    response.on_hover_text(tip)
}

/// Row text on one line, ending in "…" where it does not fit, so it stays
/// inside the row at any sidebar width.
fn single_line(
    painter: &egui::Painter,
    text: String,
    font: egui::FontId,
    color: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    painter.layout_job(job)
}

/// The selected mask's shapes and how they combine, then the selected
/// shape's options.
fn shapes(mask: &mut Mask, selected: &mut usize, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Shapes").size(13.0).color(theme::TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_enabled_ui(mask.components.len() < MAX_MASK_COMPONENTS, |ui| {
                for shape in new_shapes().into_iter().rev() {
                    let tooltip = format!(
                        "Add a {} gradient to this mask",
                        shape_name(&shape).to_lowercase()
                    );
                    if tool_button(ui, &format!("+ {}", shape_name(&shape)), false, &tooltip)
                        .clicked()
                    {
                        add_component(mask, selected, shape);
                    }
                }
            });
        });
    });
    let mut delete = None;
    let removable = mask.components.len() > 1;
    for (index, component) in mask.components.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let label = format!(
                "{}  {} {}",
                shape_icon(&component.shape),
                shape_name(&component.shape),
                index + 1
            );
            if ui.selectable_label(*selected == index, label).clicked() {
                *selected = index;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(removable, |ui| {
                    if tool_button(ui, "×", false, "Remove this shape").clicked() {
                        delete = Some(index);
                    }
                });
            });
        });
        if index > 0 {
            let labels = CombineMode::ALL.map(CombineMode::label);
            let mut mode = CombineMode::ALL
                .iter()
                .position(|mode| *mode == component.mode)
                .unwrap_or(0);
            if widgets::segmented(ui, &labels, &mut mode).changed() {
                component.mode = CombineMode::ALL[mode];
            }
        }
    }
    if let Some(index) = delete {
        remove_component(mask, selected, index);
    }
    if mask.components.len() > 1 {
        hint(
            ui,
            "Each shape adds to, subtracts from or intersects with the shapes above it.",
        );
    }
    if let Some(component) = mask.components.get_mut(*selected) {
        ui.add_space(4.0);
        shape_options(&mut component.shape, ui);
    }
}

fn shape_options(shape: &mut MaskShape, ui: &mut Ui) {
    match shape {
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
}

fn layer_settings(mask: &mut Mask, show_overlay: &mut bool, ui: &mut Ui) {
    ui.horizontal(|ui| {
        label(ui, "Name");
        ui.add(
            egui::TextEdit::singleline(&mut mask.name)
                .char_limit(MAX_MASK_NAME_CHARS)
                .desired_width(f32::INFINITY),
        );
    });
    adjustment_slider(
        ui,
        "Amount",
        &mut mask.amount,
        0.0..=100.0,
        100.0,
        &SliderStyle {
            format: |v| format!("{v:.0} %"),
            ..SliderStyle::default()
        },
    );
    ui.horizontal(|ui| {
        widgets::toggle(ui, &mut mask.invert)
            .on_hover_text("Apply the adjustments outside the selection instead");
        label(ui, "Invert");
        ui.add_space(16.0);
        widgets::toggle(ui, show_overlay).on_hover_text("Tint the area this mask affects");
        label(ui, "Show overlay");
    });
}

/// The local sliders, Reset and Delete Mask. Returns whether to delete.
fn adjustments(mask: &mut Mask, ui: &mut Ui) -> bool {
    ui.label(RichText::new("Adjustments").size(13.0).color(theme::TEXT));
    for control in LOCAL_CONTROLS {
        control_slider(ui, &mut mask.adjustments, control, &style_for(control.id));
    }
    ui.add_space(6.0);
    let mut delete = false;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!mask.adjustments.is_neutral(), egui::Button::new("Reset"))
            .clicked()
        {
            mask.adjustments = LocalAdjustments::default();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            delete = ui.button("Delete Mask").clicked();
        });
    });
    delete
}

fn add_mask(masks: &mut Vec<Mask>, selection: &mut Option<MaskSelection>, shape: MaskShape) {
    let name = format!("{} {}", shape_name(&shape), masks.len() + 1);
    masks.push(Mask::new(name, shape));
    *selection = Some(MaskSelection::mask(masks.len() - 1));
}

/// Removes a mask; the selection stays on the mask it was on, if it remains.
fn remove_mask(masks: &mut Vec<Mask>, selection: &mut Option<MaskSelection>, index: usize) {
    if index >= masks.len() {
        return;
    }
    masks.remove(index);
    *selection = match *selection {
        Some(current) if current.mask == index => None,
        Some(current) if current.mask > index => Some(MaskSelection {
            mask: current.mask - 1,
            ..current
        }),
        other => other,
    };
}

/// Inserts a copy after the original and selects it.
fn duplicate_mask(masks: &mut Vec<Mask>, selection: &mut Option<MaskSelection>, index: usize) {
    if masks.len() >= MAX_MASKS {
        return;
    }
    let Some(original) = masks.get(index) else {
        return;
    };
    let mut copy = original.clone();
    copy.name = format!("{} copy", display_name(original, index));
    if let Some((cut, _)) = copy.name.char_indices().nth(MAX_MASK_NAME_CHARS) {
        copy.name.truncate(cut);
    }
    masks.insert(index + 1, copy);
    *selection = Some(MaskSelection::mask(index + 1));
}

/// Moves mask `from` to `to_slot` (0..=len: before the mask at that index,
/// or last), keeping the selection on the same mask. Returns the mask's
/// new index. `from` must be a mask's index.
fn move_mask(
    masks: &mut Vec<Mask>,
    selection: &mut Option<MaskSelection>,
    from: usize,
    to_slot: usize,
) -> usize {
    let to = if to_slot > from { to_slot - 1 } else { to_slot }.min(masks.len() - 1);
    if to == from {
        return to;
    }
    let mask = masks.remove(from);
    masks.insert(to, mask);
    if let Some(current) = selection {
        current.mask = match current.mask {
            moved if moved == from => to,
            other if from < other && other <= to => other - 1,
            other if to <= other && other < from => other + 1,
            other => other,
        };
    }
    to
}

/// Adds a shape that adds to the selection, and selects it.
fn add_component(mask: &mut Mask, selected: &mut usize, shape: MaskShape) {
    if mask.components.len() >= MAX_MASK_COMPONENTS {
        return;
    }
    mask.components
        .push(MaskComponent::new(shape, CombineMode::Add));
    *selected = mask.components.len() - 1;
}

/// Removes a shape, keeping at least one; the selection stays on its shape
/// or moves to the one before.
fn remove_component(mask: &mut Mask, selected: &mut usize, index: usize) {
    if mask.components.len() <= 1 || index >= mask.components.len() {
        return;
    }
    mask.components.remove(index);
    if *selected > index || *selected >= mask.components.len() {
        *selected = selected.saturating_sub(1);
    }
}

fn label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(theme::TEXT_SECONDARY));
}

pub(crate) fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(theme::TEXT_TERTIARY));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(names: &[&str]) -> Vec<Mask> {
        names
            .iter()
            .map(|name| Mask::new(*name, MaskShape::default()))
            .collect()
    }

    fn names(masks: &[Mask]) -> Vec<&str> {
        masks.iter().map(|mask| mask.name.as_str()).collect()
    }

    #[test]
    fn adding_selects_and_removing_keeps_the_selection_on_its_mask() {
        let mut masks = Vec::new();
        let mut selection = None;
        for shape in [new_shapes()[0], new_shapes()[1], new_shapes()[1]] {
            add_mask(&mut masks, &mut selection, shape);
        }
        assert_eq!(selection, Some(MaskSelection::mask(2)));
        assert_eq!(masks[1].name, "Radial 2");

        remove_mask(&mut masks, &mut selection, 0);
        assert_eq!(
            selection,
            Some(MaskSelection::mask(1)),
            "still the third mask"
        );
        remove_mask(&mut masks, &mut selection, 1);
        assert_eq!(selection, None, "the selected mask is gone");
    }

    #[test]
    fn moving_reorders_and_the_selection_follows_its_mask() {
        let mut masks = named(&["A", "B", "C", "D"]);
        let mut selection = Some(MaskSelection {
            mask: 1,
            component: 0,
        });
        move_mask(&mut masks, &mut selection, 0, 3);
        assert_eq!(names(&masks), ["B", "C", "A", "D"]);
        assert_eq!(selection.map(|s| s.mask), Some(0), "B");
        move_mask(&mut masks, &mut selection, 3, 0);
        assert_eq!(names(&masks), ["D", "B", "C", "A"]);
        assert_eq!(selection.map(|s| s.mask), Some(1), "B");
        move_mask(&mut masks, &mut selection, 1, 4);
        assert_eq!(names(&masks), ["D", "C", "A", "B"]);
        assert_eq!(selection.map(|s| s.mask), Some(3), "B");
        // Dropping a row just above or below itself changes nothing.
        move_mask(&mut masks, &mut selection, 2, 2);
        move_mask(&mut masks, &mut selection, 2, 3);
        assert_eq!(names(&masks), ["D", "C", "A", "B"]);
    }

    #[test]
    fn rows_leave_room_for_each_other_and_the_hint_below() {
        let ctx = egui::Context::default();
        let mut masks = named(&[
            "Sky",
            "A name far too long to fit on one line of even the widest sidebar",
            "Face",
        ]);
        for width in [280.0, 440.0] {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 600.0))),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let mut previous_bottom = f32::NEG_INFINITY;
                for (index, mask) in masks.iter_mut().enumerate() {
                    let picture = Picture {
                        photo: None,
                        tint: None,
                        size: vec2(50.0, THUMB_HEIGHT),
                        shown: true,
                        tint_strength: 1.0,
                    };
                    let row = layer_row(ui, mask, index, index == 1, picture).rect;
                    assert_eq!(row.height(), ROW_HEIGHT);
                    assert!(
                        row.top() >= previous_bottom,
                        "row {index} overlaps the one above"
                    );
                    assert!(
                        ui.cursor().top() >= row.bottom(),
                        "what follows row {index} would start inside it"
                    );
                    previous_bottom = row.bottom();
                }
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn duplicates_go_after_the_original_and_are_selected() {
        let mut masks = named(&["Sky", "Face"]);
        let mut selection = None;
        duplicate_mask(&mut masks, &mut selection, 0);
        assert_eq!(names(&masks), ["Sky", "Sky copy", "Face"]);
        assert_eq!(selection, Some(MaskSelection::mask(1)));
    }

    #[test]
    fn shapes_are_added_selected_and_removed_down_to_one() {
        let mut mask = Mask::new("Sky", MaskShape::default());
        let mut selected = 0;
        add_component(&mut mask, &mut selected, new_shapes()[0]);
        add_component(&mut mask, &mut selected, new_shapes()[1]);
        assert_eq!((mask.components.len(), selected), (3, 2));
        remove_component(&mut mask, &mut selected, 0);
        assert_eq!(selected, 1, "still the last shape");
        remove_component(&mut mask, &mut selected, 1);
        assert_eq!(selected, 0);
        remove_component(&mut mask, &mut selected, 0);
        assert_eq!(mask.components.len(), 1, "a mask keeps one shape");
    }

    #[test]
    fn summaries_name_what_matters() {
        let mut mask = Mask::new("Sky", MaskShape::default());
        assert_eq!(summary(&mask), "Radial · no adjustments");
        mask.adjustments.exposure = 1.0;
        mask.invert = true;
        mask.amount = 60.0;
        mask.components.push(mask.components[0]);
        assert_eq!(summary(&mask), "2 shapes · inverted · 60 %");
    }
}
