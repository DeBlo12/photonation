// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The image canvas: fit/zoom/pan, before/after split, eyedropper, mask
//! editing (handles, the brush and the GPU overlay) while the masks panel is open, the
//! crop frame while the crop tool is, and painting removals while the
//! Remove panel is.

use crate::app::{PhotonationApp, Preview, ViewState, Zoom};
use crate::gpu::mask_overlay::MaskOverlay;
use crate::menus;
use crate::panels::mask_thumbnails::tint_strength;
use crate::panels::{
    crop, mask_brush, mask_handles, masks, people_picker, removal_brush, removals,
};
use crate::photo_frame::PhotoFrame;
use crate::theme;
use crate::widgets::{self, Icon};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2,
};
use photonation_develop::{AdjustmentGroup, MaskSpace};
use photonation_image::CropFrame;
use photonation_pipeline::LensMap;
use photonation_pipeline::masks::MaskWeight;

const MARGIN: f32 = 28.0;
/// How close (points) the pointer must be to grab the compare divider.
const DIVIDER_GRAB: f32 = 16.0;
const MIN_SCALE: f32 = 0.05;
const MAX_SCALE: f32 = 8.0;

/// How long a status message stays on screen.
const TOAST_DURATION: std::time::Duration = std::time::Duration::from_secs(6);

pub fn show(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    egui::CentralPanel::no_frame()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let canvas = ui.max_rect();
            let photo_only = app.view.full_screen.is_some();
            toast(app, ui, canvas);
            // Nothing is drawn under the open filmstrip, which may be glass:
            // the photo's background stays solid and stops at the strip.
            let visible = match app.view.filmstrip_rect {
                Some(strip) if theme::glass() => {
                    canvas.with_max_y(canvas.bottom().min(strip.top() - theme::GLASS_PANE_MARGIN))
                }
                Some(strip) => canvas.with_max_y(canvas.bottom().min(strip.top())),
                None => canvas,
            };
            ui.set_clip_rect(visible);
            ui.painter().rect_filled(
                visible,
                0.0,
                match (app.current.is_some(), theme::glass()) {
                    _ if photo_only => Color32::BLACK,
                    (false, _) => theme::window_fill(),
                    // On glass only the photo's own border is solid.
                    (true, true) => Color32::TRANSPARENT,
                    (true, false) => theme::palette().viewer,
                },
            );
            if app.current.is_none() {
                empty_state(app, ui, canvas);
                return;
            }
            let pixels_per_point = ui.ctx().pixels_per_point();
            let available = canvas.shrink(if photo_only { 0.0 } else { MARGIN });
            let interacting = ui.ctx().input(|i| i.pointer.any_down());
            let viewer_long_edge =
                (available.width().max(available.height()) * pixels_per_point) as usize;
            let max_texture_side = ui.ctx().input(|i| i.max_texture_side);
            app.request_previews(viewer_long_edge, interacting, max_texture_side);

            let Some(image_size) = full_resolution_size(app) else {
                loading(ui, canvas);
                return;
            };

            let response = ui.interact(
                canvas,
                ui.scope_id().with("canvas"),
                Sense::click_and_drag(),
            );
            let fit_scale =
                (available.width() / image_size.x).min(available.height() / image_size.y);
            let points_per_image_pixel = match app.view.zoom {
                Zoom::Fit => fit_scale,
                Zoom::Scale(scale) => scale / pixels_per_point,
            };
            handle_zoom(
                app,
                ui,
                &response,
                canvas,
                fit_scale,
                points_per_image_pixel,
                pixels_per_point,
            );
            let points_per_image_pixel = match app.view.zoom {
                Zoom::Fit => fit_scale,
                Zoom::Scale(scale) => scale / pixels_per_point,
            };
            let displayed = image_size * points_per_image_pixel;
            let editing_masks = app.view.show_masks && app.view.eyedropper.is_none();
            let cropping = app.view.cropping && app.view.eyedropper.is_none();
            let painting_removals = app.view.show_removals && app.view.eyedropper.is_none();
            // While picking people a click picks one: no handles, no brush.
            let picking_people = app.view.selecting_people && (editing_masks || painting_removals);
            let brushing_mask = editing_masks && !picking_people && mask_brush::in_hand(app);
            // The panel closed or the shape went mid-stroke (a shortcut): drop
            // the stroke, or the next render would still show it.
            if !brushing_mask && app.view.mask_stroke.take().is_some() {
                app.refresh();
            }
            // A drag that grabs a mask handle or the crop frame, paints a
            // removal or a mask, or moves the compare divider, edits it instead
            // of panning. The divider goes first: one drag moves one thing.
            let tool_on_photo = editing_masks || cropping || painting_removals;
            let moving_divider = compare_divider(
                app,
                ui,
                &response,
                Rect::from_center_size(canvas.center() + app.view.pan, displayed),
                tool_on_photo,
            );
            let mut dragging_mask = moving_divider;
            if !moving_divider
                && cropping
                && let Some(frame) = photo_frame(
                    app,
                    Rect::from_center_size(canvas.center() + app.view.pan, displayed),
                )
            {
                dragging_mask = crop::interact(app, ui, &response, &frame);
            }
            if !moving_divider
                && editing_masks
                && let Some(frame) = photo_frame(
                    app,
                    Rect::from_center_size(canvas.center() + app.view.pan, displayed),
                )
            {
                dragging_mask = if picking_people {
                    let path = app.current_path().cloned().unwrap_or_default();
                    let picked =
                        people_picker::clicked(app.people.found(&path), ui, &response, &frame);
                    if let Some((index, add)) = picked {
                        masks::mask_person(app, index, add);
                    }
                    false
                } else if brushing_mask {
                    let before = app.view.mask_stroke.as_ref().map(|s| s.path.points.len());
                    let painting = mask_brush::interact(app, ui, &response, &frame);
                    let after = app.view.mask_stroke.as_ref().map(|s| s.path.points.len());
                    if before != after && after.is_some() {
                        // Show the effect as the stroke grows, also if the
                        // pointer then rests.
                        app.refresh();
                        ui.ctx().request_repaint();
                    }
                    painting
                } else {
                    mask_handles::interact(
                        &mut app.settings.masks,
                        &mut app.view.selected_mask,
                        &mut app.view.shape_to_draw,
                        ui,
                        &response,
                        &frame,
                    )
                };
            }
            if !moving_divider
                && painting_removals
                && let Some(frame) = photo_frame(
                    app,
                    Rect::from_center_size(canvas.center() + app.view.pan, displayed),
                )
            {
                if picking_people {
                    let path = app.current_path().cloned().unwrap_or_default();
                    let picked =
                        people_picker::clicked(app.people.found(&path), ui, &response, &frame);
                    if let Some((index, add)) = picked {
                        removals::remove_person(app, index, add);
                    }
                } else {
                    // A stroke owns the pointer like a mask handle drag does.
                    dragging_mask |= removal_brush::interact(
                        &mut app.settings.removals,
                        &mut app.view,
                        ui,
                        &response,
                        &frame,
                    );
                }
            }
            if response.dragged()
                && app.view.zoom != Zoom::Fit
                && app.view.eyedropper.is_none()
                && !dragging_mask
            {
                app.view.pan += response.drag_delta();
            }
            app.view.pan = clamp_pan(app.view.pan, displayed, canvas.size());
            let image_rect = Rect::from_center_size(canvas.center() + app.view.pan, displayed);

            let painter = ui.painter_at(canvas);
            // The solid background: a border on glass, else the viewer itself.
            let backing = if theme::glass() {
                image_rect.expand(theme::PHOTO_BORDER)
            } else {
                image_rect
            };
            // Nothing but the photo in full screen.
            if !photo_only {
                painter.rect_filled(
                    backing.translate(vec2(0.0, 6.0)).expand(4.0),
                    4.0,
                    Color32::from_black_alpha(60),
                );
                if theme::glass() {
                    painter.rect_filled(backing, 2.0, theme::palette().viewer);
                }
            }
            if let Some(preview) = &app.preview {
                paint(&painter, preview, image_rect, image_rect);
            }
            if app.view.compare
                && let Some(before) = &app.before
            {
                compare_overlay(&app.view, &painter, before, image_rect);
                // After the tools, which set their own cursors.
                let split = app.view.compare_split;
                if response
                    .hover_pos()
                    .is_some_and(|pointer| on_divider(pointer, split, image_rect, tool_on_photo))
                {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                }
            }
            if editing_masks && let Some(frame) = photo_frame(app, image_rect) {
                let edited = edited_part(app, image_rect);
                let tint = painter.with_clip_rect(edited.intersect(painter.clip_rect()));
                selected_mask_overlay(app, &tint, &frame);
                // As rendered, so pins of hidden masks look hidden.
                mask_handles::paint(
                    &app.settings.rendered_masks(),
                    app.view.selected_mask,
                    &painter,
                    &frame,
                );
                if brushing_mask {
                    mask_brush::paint(&app.view, ui, &response, &painter, &frame, edited);
                }
            }
            if picking_people
                && let Some(frame) = photo_frame(app, image_rect)
                && let Some(path) = app.current_path()
            {
                people_picker::paint(
                    app.people.found(path),
                    &painter,
                    &frame,
                    response.hover_pos(),
                );
            }
            if cropping && let Some(frame) = photo_frame(app, image_rect) {
                crop::paint(app, &painter, &frame);
            }
            if painting_removals && let Some(frame) = photo_frame(app, image_rect) {
                removal_brush::paint(
                    &app.settings.removals,
                    &app.view,
                    &painter,
                    &frame,
                    // No brush outline while a click picks a person.
                    response.hover_pos().filter(|_| !picking_people),
                );
            }
            if app.is_rendering() || app.filling_removals {
                let spinner = Rect::from_center_size(
                    pos2(canvas.right() - 22.0, canvas.top() + 22.0),
                    vec2(16.0, 16.0),
                );
                ui.put(
                    spinner,
                    egui::Spinner::new()
                        .size(16.0)
                        .color(theme::palette().on_viewer),
                );
            }
            eyedropper(app, ui, &response, image_rect);

            // A second dab with the mask brush isn't a request to zoom.
            if response.double_clicked()
                && app.view.eyedropper.is_none()
                && !dragging_mask
                && !brushing_mask
            {
                app.view.zoom = if app.view.zoom == Zoom::Fit {
                    Zoom::Scale(1.0)
                } else {
                    Zoom::Fit
                };
                app.view.pan = Vec2::ZERO;
            }
        });
}

/// Size of the rendered image at 100%: the preview's aspect ratio (which
/// reflects any crop) at the photo's full-resolution long edge.
/// Status message as a pill at the bottom of the canvas, fading out.
pub fn toast(app: &mut PhotonationApp, ui: &egui::Ui, canvas: Rect) {
    let Some((message, posted)) = &app.status else {
        return;
    };
    let age = posted.elapsed();
    if age > TOAST_DURATION {
        app.status = None;
        return;
    }
    let opacity =
        (1.0 - (age.as_secs_f32() - (TOAST_DURATION.as_secs_f32() - 1.0)).max(0.0)).clamp(0.0, 1.0);
    let painter = ui.painter_at(canvas).with_layer_id(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::unique("toast"),
    ));
    let max_width = (canvas.width() - 80.0).max(100.0);
    let galley = painter.layout(
        message.clone(),
        theme::body(12.5),
        theme::palette().text.gamma_multiply(opacity),
        max_width,
    );
    let rect = Rect::from_center_size(
        pos2(
            canvas.center().x,
            canvas.bottom() - 30.0 - galley.size().y / 2.0,
        ),
        galley.size() + vec2(24.0, 12.0),
    );
    painter.rect_filled(
        rect,
        rect.height().min(24.0) / 2.0,
        theme::palette().card.gamma_multiply(0.95 * opacity),
    );
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        theme::palette().text,
    );
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(100));
}

fn full_resolution_size(app: &PhotonationApp) -> Option<Vec2> {
    let preview = app.preview.as_ref()?;
    let preview_size = vec2(preview.size[0] as f32, preview.size[1] as f32);
    let full_long_edge = app
        .info
        .as_ref()
        .map_or(preview_size.max_elem(), |i| i.size.0.max(i.size.1) as f32);
    Some(preview_size * (full_long_edge / preview_size.max_elem()))
}

/// Where the photo is on screen, for mask editing (once its size is known).
pub fn photo_frame(app: &PhotonationApp, image_rect: Rect) -> Option<PhotoFrame> {
    let info = app.info.as_ref()?;
    let photo = [info.size.0 as f64, info.size.1 as f64];
    // The crop tool shows the whole photo with the frame drawn over it.
    let crop = if app.view.cropping {
        CropFrame::full(photo)
    } else {
        photonation_pipeline::crop_frame(photo, app.settings.crop, app.settings.crop_angle)
    };
    // Masks and removals stay on the photo as shot, the crop tool works on
    // the corrected one.
    let lens = &app.settings.lens;
    let corrected = !app.view.cropping && !app.view.disabled.contains(&AdjustmentGroup::Lens);
    let profile = (corrected && (lens.profile || lens.chromatic_aberration))
        .then(|| photonation_pipeline::lens_profile(&info.camera))
        .flatten();
    Some(PhotoFrame {
        image_rect,
        crop,
        space: MaskSpace::new(photo[0], photo[1]),
        lens: corrected
            .then(|| LensMap::new(photo, lens, profile.as_ref()))
            .flatten(),
    })
}

/// Tints where the selected mask applies, computed per pixel on the GPU.
fn selected_mask_overlay(app: &PhotonationApp, painter: &egui::Painter, frame: &PhotoFrame) {
    if !(app.mask_overlay_ready && app.view.mask_overlay) {
        return;
    }
    let Some(mask) = app
        .view
        .selected_mask
        .and_then(|selection| app.settings.masks.get(selection.mask))
    else {
        return;
    };
    // The render sanitizes masks, so the overlay shows the sanitized shape.
    let mask = mask.rendered(&app.settings.mask_groups).sanitized();
    MaskOverlay {
        frame: *frame,
        weight: MaskWeight::new(&mask, frame.space),
        color: theme::MASK_OVERLAY.gamma_multiply(tint_strength(&mask)),
    }
    .paint(painter);
}

fn paint(painter: &egui::Painter, preview: &Preview, rect: Rect, clip: Rect) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter
        .with_clip_rect(clip.intersect(painter.clip_rect()))
        .image(preview.texture.id(), rect, uv, Color32::WHITE);
}

/// The welcome screen: the brand's iris and wordmark over the open button.
fn empty_state(app: &mut PhotonationApp, ui: &mut egui::Ui, canvas: Rect) {
    let painter = ui.painter();
    widgets::paint_brand_mark(
        painter,
        canvas.center() - vec2(0.0, 76.0),
        88.0,
        theme::palette().window,
    );
    painter.text(
        canvas.center() + vec2(0.0, 6.0),
        Align2::CENTER_CENTER,
        "photonation",
        theme::display(30.0),
        theme::palette().text,
    );
    painter.text(
        canvas.center() + vec2(0.0, 40.0),
        Align2::CENTER_CENTER,
        "Open a folder of raw photos, or one photo, to start editing",
        theme::body(14.0),
        theme::palette().text_secondary,
    );
    let button_at = |offset: f32| {
        Rect::from_center_size(canvas.center() + vec2(offset, 84.0), vec2(150.0, 34.0))
    };
    if ui
        .put(button_at(-80.0), theme::primary_button(menus::OPEN_FOLDER))
        .clicked()
    {
        app.pick_folder();
    }
    if ui
        .put(button_at(80.0), egui::Button::new(menus::OPEN_PHOTO))
        .clicked()
    {
        app.pick_photo();
    }
}

/// A spinner on the viewer: its own background differs on glass.
fn spinner_color() -> Color32 {
    if theme::glass() {
        theme::palette().text
    } else {
        theme::palette().on_viewer
    }
}

fn loading(ui: &mut egui::Ui, canvas: Rect) {
    ui.put(
        Rect::from_center_size(canvas.center(), vec2(28.0, 28.0)),
        egui::Spinner::new().size(28.0).color(spinner_color()),
    );
}

/// Pinch or ⌘-scroll zooms around the pointer; plain scroll pans when zoomed.
fn handle_zoom(
    app: &mut PhotonationApp,
    ui: &egui::Ui,
    response: &egui::Response,
    canvas: Rect,
    fit_scale: f32,
    points_per_image_pixel: f32,
    pixels_per_point: f32,
) {
    if !response.hovered() {
        return;
    }
    let (zoom_delta, scroll) = ui
        .ctx()
        .input(|i| (i.zoom_delta(), i.smooth_scroll_delta()));
    if (zoom_delta - 1.0).abs() > 1e-4 {
        let new_points =
            (points_per_image_pixel * zoom_delta).clamp(fit_scale.min(MIN_SCALE), MAX_SCALE);
        if new_points <= fit_scale {
            app.view.zoom = Zoom::Fit;
            app.view.pan = Vec2::ZERO;
        } else {
            // Keep the image point under the cursor fixed.
            if let Some(pointer) = response.hover_pos() {
                let from_center = pointer - canvas.center() - app.view.pan;
                app.view.pan -= from_center * (new_points / points_per_image_pixel - 1.0);
            }
            app.view.zoom = Zoom::Scale(new_points * pixels_per_point);
        }
    } else if app.view.zoom != Zoom::Fit && scroll != Vec2::ZERO {
        app.view.pan += scroll;
    }
}

fn clamp_pan(pan: Vec2, displayed: Vec2, canvas: Vec2) -> Vec2 {
    let limit = ((displayed - canvas) / 2.0).max(Vec2::ZERO);
    pan.clamp(-limit, limit)
}

/// Before (left of the divider) over after; drag the divider handle.
/// The part of the photo showing the edit: all of it, or while comparing, the
/// part right of the divider. The original has no masks, so a mask's tint
/// stays out of its side.
fn edited_part(app: &PhotonationApp, image_rect: Rect) -> Rect {
    if app.view.compare && app.before.is_some() {
        image_rect.with_min_x(divider_x(app.view.compare_split, image_rect))
    } else {
        image_rect
    }
}

fn divider_x(split: f32, image_rect: Rect) -> f32 {
    image_rect.left() + image_rect.width() * split
}

/// Whether a drag from `pointer` moves the compare divider. Any point along
/// it does, unless a tool also works on the photo: then only its knob, so
/// the tool can still work right next to the divider.
fn on_divider(pointer: Pos2, split: f32, image_rect: Rect, tool_on_photo: bool) -> bool {
    let knob = pos2(divider_x(split, image_rect), image_rect.center().y);
    if tool_on_photo {
        pointer.distance(knob) < DIVIDER_GRAB
    } else {
        (pointer.x - knob.x).abs() < DIVIDER_GRAB
    }
}

/// Handles dragging the compare divider. Returns whether it owns the pointer.
fn compare_divider(
    app: &mut PhotonationApp,
    ui: &egui::Ui,
    response: &egui::Response,
    image_rect: Rect,
    tool_on_photo: bool,
) -> bool {
    if !(app.view.compare && app.before.is_some()) {
        return false;
    }
    let split = app.view.compare_split;
    let on = |pointer| on_divider(pointer, split, image_rect, tool_on_photo);
    let dragging_id = response.id.with("dragging-divider");
    let mut dragging = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(dragging_id))
        .unwrap_or(false);
    if response.drag_started() && ui.input(|i| i.pointer.press_origin()).is_some_and(on) {
        dragging = true;
    }
    if !response.dragged() {
        dragging = false;
    }
    if dragging && let Some(pointer) = response.interact_pointer_pos() {
        app.view.compare_split =
            ((pointer.x - image_rect.left()) / image_rect.width()).clamp(0.0, 1.0);
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    ui.ctx().data_mut(|d| d.insert_temp(dragging_id, dragging));
    dragging
}

fn compare_overlay(view: &ViewState, painter: &egui::Painter, before: &Preview, image_rect: Rect) {
    let split_x = divider_x(view.compare_split, image_rect);
    let left = Rect::from_min_max(image_rect.min, pos2(split_x, image_rect.bottom()));
    paint(painter, before, image_rect, left);
    painter.line_segment(
        [
            pos2(split_x, image_rect.top()),
            pos2(split_x, image_rect.bottom()),
        ],
        Stroke::new(2.0, theme::KNOB),
    );
    let handle = pos2(split_x, image_rect.center().y);
    painter.circle_filled(handle, 14.0, theme::KNOB);
    widgets::paint_icon(
        painter,
        Rect::from_center_size(handle, Vec2::splat(16.0)),
        Icon::SplitHandle,
        theme::INK,
    );
    painter.text(
        image_rect.left_top() + vec2(10.0, 10.0),
        Align2::LEFT_TOP,
        "Original",
        theme::body(12.0),
        theme::ON_PHOTO,
    );
    painter.text(
        image_rect.right_top() + vec2(-10.0, 10.0),
        Align2::RIGHT_TOP,
        "Edited",
        theme::body(12.0),
        theme::ON_PHOTO,
    );
}

fn eyedropper(
    app: &mut PhotonationApp,
    ui: &egui::Ui,
    response: &egui::Response,
    image_rect: Rect,
) {
    let Some(purpose) = app.view.eyedropper else {
        return;
    };
    if response.hover_pos().is_some_and(|p| image_rect.contains(p)) {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    }
    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
        && image_rect.contains(pointer)
    {
        let point = [
            ((pointer.x - image_rect.left()) / image_rect.width()) as f64,
            ((pointer.y - image_rect.top()) / image_rect.height()) as f64,
        ];
        app.pick_color(point, purpose);
        app.view.eyedropper = None;
    }
    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        app.view.eyedropper = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_a_tool_open_only_the_knob_grabs_the_divider() {
        let photo = Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0));
        let knob = pos2(400.0, 300.0);
        let up_the_line = pos2(400.0, 150.0);
        assert!(on_divider(knob, 0.5, photo, true));
        assert!(!on_divider(up_the_line, 0.5, photo, true));
        assert!(on_divider(knob, 0.5, photo, false));
        assert!(on_divider(up_the_line, 0.5, photo, false));
        assert!(!on_divider(pos2(450.0, 300.0), 0.5, photo, false));
        assert!(
            on_divider(pos2(250.0, 300.0), 0.25, photo, true),
            "follows the split"
        );
    }
}
