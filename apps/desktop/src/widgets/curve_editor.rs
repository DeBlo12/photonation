// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Tone curve editor: click to add a point, drag to move, double-click a
//! point to remove it. Endpoints can move vertically only.

use crate::theme;
use eframe::egui::{Color32, CornerRadius, Pos2, Response, Sense, Stroke, Ui, pos2, vec2};
use photonation_develop::PointCurve;
use photonation_pipeline::curve::Spline;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveChannel {
    Rgb,
    Red,
    Green,
    Blue,
}

impl CurveChannel {
    pub const ALL: [CurveChannel; 4] = [
        CurveChannel::Rgb,
        CurveChannel::Red,
        CurveChannel::Green,
        CurveChannel::Blue,
    ];

    pub fn color(self) -> Color32 {
        match self {
            CurveChannel::Rgb => theme::TEXT,
            CurveChannel::Red => Color32::from_rgb(255, 90, 90),
            CurveChannel::Green => Color32::from_rgb(90, 220, 110),
            CurveChannel::Blue => Color32::from_rgb(90, 150, 255),
        }
    }
}

const HANDLE_RADIUS: f32 = 5.0;
const GRAB_DISTANCE: f32 = 10.0;

/// Edits `curve`; `preview` (if given) is drawn as a read-only curve, used
/// for the parametric mode.
pub fn curve_editor(
    ui: &mut Ui,
    curve: Option<&mut PointCurve>,
    preview: Option<&dyn Fn(f32) -> f32>,
    color: Color32,
) -> Response {
    let side = ui.available_width();
    let (rect, mut response) =
        ui.allocate_exact_size(vec2(side, side * 0.85), Sense::click_and_drag());
    let area = rect.shrink(6.0);
    let to_screen = |p: [f32; 2]| {
        pos2(
            area.left() + p[0] * area.width(),
            area.bottom() - p[1] * area.height(),
        )
    };
    let from_screen = |p: Pos2| {
        [
            ((p.x - area.left()) / area.width()).clamp(0.0, 1.0),
            ((area.bottom() - p.y) / area.height()).clamp(0.0, 1.0),
        ]
    };

    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        CornerRadius::same(theme::CONTROL_RADIUS),
        theme::WINDOW,
    );
    for i in 1..4 {
        let t = i as f32 / 4.0;
        let grid = Stroke::new(1.0, theme::SEPARATOR);
        painter.line_segment(
            [
                pos2(area.left() + t * area.width(), area.top()),
                pos2(area.left() + t * area.width(), area.bottom()),
            ],
            grid,
        );
        painter.line_segment(
            [
                pos2(area.left(), area.top() + t * area.height()),
                pos2(area.right(), area.top() + t * area.height()),
            ],
            grid,
        );
    }
    painter.line_segment(
        [to_screen([0.0, 0.0]), to_screen([1.0, 1.0])],
        Stroke::new(1.0, theme::TEXT_TERTIARY),
    );

    let draw_function = |f: &dyn Fn(f32) -> f32| {
        let points: Vec<Pos2> = (0..=128)
            .map(|i| i as f32 / 128.0)
            .map(|x| to_screen([x, f(x)]))
            .collect();
        painter.line(points, Stroke::new(1.8, color));
    };

    if let Some(curve) = curve {
        let dragging_id = response.id.with("dragging");
        let mut dragging: Option<usize> = ui.data(|d| d.get_temp(dragging_id));
        let pointer = response.interact_pointer_pos();
        let nearest = |points: &[[f32; 2]], at: Pos2| {
            points
                .iter()
                .enumerate()
                .map(|(i, p)| (i, to_screen(*p).distance(at)))
                .filter(|(_, d)| *d < GRAB_DISTANCE)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        };
        let before = curve.clone();
        if response.double_clicked()
            && let Some(at) = pointer
            && let Some(index) = nearest(&curve.0, at)
            && index != 0
            && index != curve.0.len() - 1
        {
            curve.0.remove(index);
        } else if response.drag_started()
            && let Some(at) = pointer
        {
            dragging = nearest(&curve.0, at).or_else(|| {
                let p = from_screen(at);
                let index = curve.0.partition_point(|q| q[0] < p[0]);
                curve.0.insert(index, p);
                Some(index)
            });
        } else if response.clicked()
            && let Some(at) = pointer
            && nearest(&curve.0, at).is_none()
        {
            let p = from_screen(at);
            let index = curve.0.partition_point(|q| q[0] < p[0]);
            curve.0.insert(index, p);
        }
        if response.dragged()
            && let (Some(index), Some(at)) = (dragging, pointer)
        {
            let mut p = from_screen(at);
            let last = curve.0.len() - 1;
            // Keep points ordered and endpoints on the edges.
            p[0] = match index {
                0 => 0.0,
                i if i == last => 1.0,
                i => p[0].clamp(curve.0[i - 1][0] + 0.01, curve.0[i + 1][0] - 0.01),
            };
            curve.0[index] = p;
        }
        if response.drag_stopped() {
            dragging = None;
        }
        ui.data_mut(|d| d.insert_temp(dragging_id, dragging));
        if *curve != before {
            response.mark_changed();
        }

        let spline = Spline::new(curve);
        draw_function(&|x| spline.eval(x));
        for (index, point) in curve.0.iter().enumerate() {
            let active = dragging == Some(index);
            let center = to_screen(*point);
            painter.circle_filled(
                center,
                if active {
                    HANDLE_RADIUS + 1.0
                } else {
                    HANDLE_RADIUS
                },
                theme::WINDOW,
            );
            painter.circle_stroke(center, HANDLE_RADIUS, Stroke::new(1.6, color));
        }
    } else if let Some(f) = preview {
        draw_function(f);
    }
    response
}
