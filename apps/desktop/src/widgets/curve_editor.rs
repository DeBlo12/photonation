// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Tone curve editor: click to add a point, drag to move, double-click a
//! point to remove it. Endpoints can move vertically only. The photo's
//! histogram, of the curve's channel, is drawn behind the curve.

use crate::theme;
use eframe::egui::{Color32, CornerRadius, Pos2, Response, Sense, Stroke, Ui, pos2, vec2};
use photonation_develop::PointCurve;
use photonation_engine::Histogram;
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

    /// The bins of `histogram` this channel's curve shapes: luminance for
    /// the RGB curve.
    pub fn bins(self, histogram: &Histogram) -> &[u32] {
        match self {
            CurveChannel::Rgb => &histogram.luminance,
            CurveChannel::Red => &histogram.red,
            CurveChannel::Green => &histogram.green,
            CurveChannel::Blue => &histogram.blue,
        }
    }

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
/// Opacity of the histogram behind the curve.
const HISTOGRAM_ALPHA: u8 = 46;
const GRAB_DISTANCE: f32 = 10.0;
/// Smallest horizontal distance a dragged point keeps from its neighbours.
const MIN_GAP: f32 = 0.01;

/// Edits `curve`; `preview` (if given) is drawn as a read-only curve, used
/// for the parametric mode. `histogram` (256 bins) is drawn behind both.
pub fn curve_editor(
    ui: &mut Ui,
    curve: Option<&mut PointCurve>,
    preview: Option<&dyn Fn(f32) -> f32>,
    histogram: Option<&[u32]>,
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
    if let Some(bins) = histogram {
        // Scaled to this channel's own peak, without the clipped end bins
        // (as the sidebar histogram), so one channel fills the height.
        let peak = bins[1..bins.len() - 1]
            .iter()
            .copied()
            .max()
            .unwrap_or(0)
            .max(1);
        let tint =
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), HISTOGRAM_ALPHA);
        painter.add(super::histogram::area_mesh(area, bins, peak as f32, tint));
    }
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
        // A drag starts once the pointer has moved past egui's threshold;
        // the point to grab is the one where the button went down.
        let pressed_at = ui.input(|i| i.pointer.press_origin()).or(pointer);
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
            && let Some(at) = pressed_at
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
            curve.0[index] = constrained(&curve.0, index, from_screen(at));
        }
        if response.drag_stopped() {
            dragging = None;
        }
        // Stored as the index itself: egui keys temp data by type, so it
        // must be read back as the same type.
        ui.data_mut(|d| match dragging {
            Some(index) => {
                d.insert_temp(dragging_id, index);
            }
            None => d.remove::<usize>(dragging_id),
        });
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

/// Where point `index` of `points` goes when dragged to `p`: endpoints stay
/// on the left and right edges, and inner points stay between their
/// neighbours.
fn constrained(points: &[[f32; 2]], index: usize, mut p: [f32; 2]) -> [f32; 2] {
    let last = points.len() - 1;
    p[0] = match index {
        0 => 0.0,
        i if i == last => 1.0,
        i => {
            let (left, right) = (points[i - 1][0], points[i + 1][0]);
            if right - left > 2.0 * MIN_GAP {
                p[0].clamp(left + MIN_GAP, right - MIN_GAP)
            } else {
                // Neighbours placed closer than the gap: stay between them.
                (left + right) / 2.0
            }
        }
    };
    p
}

/// The editor drawn headless, fed the events a window would send.
#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, Event, PointerButton, RawInput, Rect};

    const SCREEN: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(300.0, 300.0));

    struct Editor {
        ctx: egui::Context,
        curve: PointCurve,
        area: Rect,
        time: f64,
    }

    impl Editor {
        fn with(points: &[[f32; 2]]) -> Self {
            let mut editor = Editor {
                ctx: egui::Context::default(),
                curve: PointCurve(points.to_vec()),
                area: Rect::NOTHING,
                time: 0.0,
            };
            editor.frame(Vec::new());
            editor
        }

        fn frame(&mut self, events: Vec<Event>) {
            const DT: f32 = 1.0 / 60.0;
            self.time += f64::from(DT);
            let input = RawInput {
                screen_rect: Some(SCREEN),
                time: Some(self.time),
                predicted_dt: DT,
                events,
                ..Default::default()
            };
            let (curve, mut rect) = (&mut self.curve, Rect::NOTHING);
            let mut output = self.ctx.run_ui(input, |ui| {
                rect = curve_editor(ui, Some(curve), None, None, theme::TEXT).rect;
            });
            output.textures_delta.clear();
            self.area = rect.shrink(6.0);
        }

        fn screen(&self, p: [f32; 2]) -> Pos2 {
            pos2(
                self.area.left() + p[0] * self.area.width(),
                self.area.bottom() - p[1] * self.area.height(),
            )
        }

        fn button(&mut self, at: Pos2, pressed: bool) {
            self.frame(vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }

        fn click(&mut self, p: [f32; 2]) {
            let at = self.screen(p);
            self.frame(vec![Event::PointerMoved(at)]);
            self.button(at, true);
            self.button(at, false);
            self.frame(Vec::new());
        }

        /// Presses at `from`, moves to `to` in `steps` frames and releases.
        fn drag(&mut self, from: [f32; 2], to: [f32; 2], steps: usize) {
            let (start, end) = (self.screen(from), self.screen(to));
            self.frame(vec![Event::PointerMoved(start)]);
            self.button(start, true);
            for step in 1..=steps {
                let at = start.lerp(end, step as f32 / steps as f32);
                self.frame(vec![Event::PointerMoved(at)]);
            }
            self.button(end, false);
            self.frame(Vec::new());
        }
    }

    const DIAGONAL: [[f32; 2]; 3] = [[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]];

    fn assert_near(actual: [f32; 2], expected: [f32; 2]) {
        let off = (actual[0] - expected[0])
            .abs()
            .max((actual[1] - expected[1]).abs());
        assert!(off < 0.01, "{actual:?} is not at {expected:?}");
    }

    #[test]
    fn a_dragged_point_follows_the_pointer_to_the_end() {
        for steps in [40, 8] {
            let mut editor = Editor::with(&DIAGONAL);
            editor.drag([0.5, 0.5], [0.6, 0.8], steps);
            assert_eq!(editor.curve.0.len(), 3, "{:?}", editor.curve.0);
            assert_near(editor.curve.0[1], [0.6, 0.8]);
        }
    }

    #[test]
    fn a_quick_drag_moves_the_point_instead_of_adding_one() {
        // Two frames: past the grab distance before egui reports the drag.
        let mut editor = Editor::with(&DIAGONAL);
        editor.drag([0.5, 0.5], [0.5, 0.2], 2);
        assert_eq!(editor.curve.0.len(), 3, "{:?}", editor.curve.0);
        assert_near(editor.curve.0[1], [0.5, 0.2]);
    }

    #[test]
    fn dragging_empty_space_adds_a_point_and_moves_it() {
        let mut editor = Editor::with(&DIAGONAL);
        editor.drag([0.25, 0.4], [0.3, 0.1], 10);
        assert_eq!(editor.curve.0.len(), 4, "{:?}", editor.curve.0);
        assert_near(editor.curve.0[1], [0.3, 0.1]);
        assert_near(editor.curve.0[2], [0.5, 0.5]);
    }

    #[test]
    fn endpoints_move_only_vertically() {
        let mut editor = Editor::with(&DIAGONAL);
        editor.drag([0.0, 0.0], [0.2, 0.15], 10);
        editor.drag([1.0, 1.0], [0.7, 0.9], 10);
        assert_near(editor.curve.0[0], [0.0, 0.15]);
        assert_near(editor.curve.0[2], [1.0, 0.9]);
    }

    #[test]
    fn points_cannot_pass_their_neighbours() {
        let mut editor = Editor::with(&[[0.0, 0.0], [0.3, 0.3], [0.6, 0.6], [1.0, 1.0]]);
        editor.drag([0.3, 0.3], [0.9, 0.3], 20);
        assert_near(editor.curve.0[1], [0.6 - MIN_GAP, 0.3]);
    }

    #[test]
    fn clicks_add_points_and_double_clicks_remove_them() {
        let mut editor = Editor::with(&DIAGONAL);
        editor.click([0.75, 0.9]);
        assert_eq!(editor.curve.0.len(), 4, "{:?}", editor.curve.0);
        assert_near(editor.curve.0[2], [0.75, 0.9]);
        // Later, so the next two clicks don't pair with the first.
        editor.time += 1.0;
        editor.click([0.75, 0.9]);
        editor.click([0.75, 0.9]);
        assert_eq!(editor.curve.0.len(), 3, "{:?}", editor.curve.0);
    }

    #[test]
    fn points_placed_closer_than_the_gap_stay_between_their_neighbours() {
        let points = [[0.0, 0.0], [0.5, 0.2], [0.505, 0.9], [1.0, 1.0]];
        let p = constrained(&points, 1, [0.9, 0.4]);
        assert!(p[0] > 0.0 && p[0] < 0.505, "{p:?}");
        assert_eq!(p[1], 0.4);
    }
}
