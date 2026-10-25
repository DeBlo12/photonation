// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The image canvas: fit/zoom/pan, before/after split, eyedropper.

use crate::app::{PhotonationApp, Preview, ViewState, Zoom};
use crate::theme;
use eframe::egui::{self, Align2, Color32, CursorIcon, Rect, Sense, Stroke, Vec2, pos2, vec2};

const MARGIN: f32 = 28.0;
const MIN_SCALE: f32 = 0.05;
const MAX_SCALE: f32 = 8.0;

/// How long a status message stays on screen.
const TOAST_DURATION: std::time::Duration = std::time::Duration::from_secs(6);

pub fn show(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    egui::CentralPanel::no_frame()
        .frame(egui::Frame::NONE.fill(theme::WINDOW))
        .show(ui, |ui| {
            let canvas = ui.max_rect();
            toast(app, ui, canvas);
            if app.current.is_none() {
                empty_state(app, ui, canvas);
                return;
            }
            let pixels_per_point = ui.ctx().pixels_per_point();
            let available = canvas.shrink(MARGIN);
            let interacting = ui.ctx().input(|i| i.pointer.any_down());
            let viewer_long_edge =
                (available.width().max(available.height()) * pixels_per_point) as usize;
            app.request_previews(viewer_long_edge, interacting);

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
            if response.dragged() && app.view.zoom != Zoom::Fit && !app.view.eyedropper {
                app.view.pan += response.drag_delta();
            }
            app.view.pan = clamp_pan(app.view.pan, displayed, canvas.size());
            let image_rect = Rect::from_center_size(canvas.center() + app.view.pan, displayed);

            let painter = ui.painter_at(canvas);
            painter.rect_filled(
                image_rect.translate(vec2(0.0, 6.0)).expand(4.0),
                4.0,
                Color32::from_black_alpha(60),
            );
            if let Some(preview) = &app.preview {
                paint(&painter, preview, image_rect, image_rect);
            }
            if app.view.compare
                && let Some(before) = &app.before
            {
                compare_overlay(&mut app.view, ui, &response, &painter, before, image_rect);
            }
            if app.is_rendering() {
                let spinner = Rect::from_center_size(
                    pos2(canvas.right() - 22.0, canvas.top() + 22.0),
                    vec2(16.0, 16.0),
                );
                ui.put(
                    spinner,
                    egui::Spinner::new().size(16.0).color(theme::TEXT_SECONDARY),
                );
            }
            eyedropper(app, ui, &response, image_rect);

            if response.double_clicked() && !app.view.eyedropper {
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
fn toast(app: &mut PhotonationApp, ui: &egui::Ui, canvas: Rect) {
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
        theme::TEXT.gamma_multiply(opacity),
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
        theme::CARD.gamma_multiply(0.95 * opacity),
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, theme::TEXT);
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

fn paint(painter: &egui::Painter, preview: &Preview, rect: Rect, clip: Rect) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter
        .with_clip_rect(clip.intersect(painter.clip_rect()))
        .image(preview.texture.id(), rect, uv, Color32::WHITE);
}

fn empty_state(app: &mut PhotonationApp, ui: &mut egui::Ui, canvas: Rect) {
    let painter = ui.painter();
    painter.text(
        canvas.center() - vec2(0.0, 40.0),
        Align2::CENTER_CENTER,
        "Photonation",
        theme::body(28.0),
        theme::TEXT,
    );
    painter.text(
        canvas.center() - vec2(0.0, 8.0),
        Align2::CENTER_CENTER,
        "Open a folder of raw photos to start editing",
        theme::body(14.0),
        theme::TEXT_SECONDARY,
    );
    let button_rect = Rect::from_center_size(canvas.center() + vec2(0.0, 36.0), vec2(150.0, 34.0));
    let button = egui::Button::new(
        egui::RichText::new("Open Folder…")
            .size(14.0)
            .color(theme::TEXT),
    )
    .fill(theme::ACCENT)
    .corner_radius(8);
    if ui.put(button_rect, button).clicked() {
        app.pick_folder();
    }
}

fn loading(ui: &mut egui::Ui, canvas: Rect) {
    ui.put(
        Rect::from_center_size(canvas.center(), vec2(28.0, 28.0)),
        egui::Spinner::new().size(28.0),
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
fn compare_overlay(
    view: &mut ViewState,
    ui: &egui::Ui,
    response: &egui::Response,
    painter: &egui::Painter,
    before: &Preview,
    image_rect: Rect,
) {
    let split_x = image_rect.left() + image_rect.width() * view.compare_split;
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
    painter.text(
        handle,
        Align2::CENTER_CENTER,
        "⇔",
        theme::body(13.0),
        theme::WINDOW,
    );
    painter.text(
        image_rect.left_top() + vec2(10.0, 10.0),
        Align2::LEFT_TOP,
        "Original",
        theme::body(12.0),
        theme::TEXT,
    );
    painter.text(
        image_rect.right_top() + vec2(-10.0, 10.0),
        Align2::RIGHT_TOP,
        "Edited",
        theme::body(12.0),
        theme::TEXT,
    );

    let near_handle = response
        .hover_pos()
        .is_some_and(|p| (p.x - split_x).abs() < 16.0);
    if near_handle {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    let dragging_id = response.id.with("dragging-divider");
    let mut dragging = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(dragging_id))
        .unwrap_or(false);
    if response.drag_started() && near_handle {
        dragging = true;
    }
    if !response.dragged() {
        dragging = false;
    }
    if dragging && let Some(pointer) = response.interact_pointer_pos() {
        view.compare_split = ((pointer.x - image_rect.left()) / image_rect.width()).clamp(0.0, 1.0);
    }
    ui.ctx().data_mut(|d| d.insert_temp(dragging_id, dragging));
}

fn eyedropper(
    app: &mut PhotonationApp,
    ui: &egui::Ui,
    response: &egui::Response,
    image_rect: Rect,
) {
    if !app.view.eyedropper {
        return;
    }
    if response.hover_pos().is_some_and(|p| image_rect.contains(p)) {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    }
    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
        && image_rect.contains(pointer)
        && let Some(path) = app.current_path().cloned()
    {
        let point = [
            ((pointer.x - image_rect.left()) / image_rect.width()) as f64,
            ((pointer.y - image_rect.top()) / image_rect.height()) as f64,
        ];
        app.engine
            .pick_white_balance(path, app.settings.clone(), point);
        app.view.eyedropper = false;
    }
    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        app.view.eyedropper = false;
    }
}
