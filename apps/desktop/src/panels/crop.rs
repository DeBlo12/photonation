// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The crop tool. While it is open the viewer shows the whole photo with
//! the crop frame over it: drag inside the frame to move it, its corners
//! and edges to resize it, and outside it to turn it (straighten). The
//! sidebar's crop card picks an aspect ratio and sets the angle. The frame
//! always stays on the photo. See docs/DESKTOP.md.

use crate::app::PhotonationApp;
use crate::panels::masks::card;
use crate::panels::tool_button;
use crate::photo_frame::PhotoFrame;
use crate::theme;
use crate::widgets::{SliderStyle, adjustment_slider};
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, Layout, Mesh, Painter, Pos2, RichText, Shape, Stroke,
    Ui, Vec2, pos2,
};
use photonation_develop::{CROP_CONTROLS, Settings};
use photonation_image::CropFrame;

/// Screen distance within which a corner or edge grip is grabbed.
const GRAB_DISTANCE: f32 = 12.0;
/// Length of the corner and edge grips along the frame, in points.
const GRIP_LENGTH: f32 = 16.0;
/// Smallest frame side, in photo pixels.
const MIN_SIDE: f64 = 32.0;
/// Frame positions between the start and the target a drag stops at, as a
/// binary search: 2⁻²⁰ of the way is far below a pixel.
const FIT_STEPS: usize = 20;

/// Aspect-ratio choices, width : height in the frame's orientation.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Aspect {
    Free,
    /// The photo's own.
    #[default]
    Original,
    Ratio(u32, u32),
}

const ASPECTS: [(&str, Aspect); 7] = [
    ("Original", Aspect::Original),
    ("Free", Aspect::Free),
    ("1:1", Aspect::Ratio(1, 1)),
    ("4:5", Aspect::Ratio(4, 5)),
    ("5:7", Aspect::Ratio(5, 7)),
    ("2:3", Aspect::Ratio(2, 3)),
    ("9:16", Aspect::Ratio(9, 16)),
];

impl Aspect {
    /// Width ÷ height for a frame that is `portrait` or not; `None` when free.
    fn ratio(self, photo: [f64; 2], portrait: bool) -> Option<f64> {
        let (long, short) = match self {
            Aspect::Free => return None,
            Aspect::Original => (photo[0].max(photo[1]), photo[0].min(photo[1])),
            Aspect::Ratio(a, b) => (a.max(b) as f64, a.min(b) as f64),
        };
        Some(if portrait { short / long } else { long / short })
    }
}

/// What a drag on the canvas does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grip {
    Move,
    /// Moves the frame's left (−1) or right (+1) side and its top (−1) or
    /// bottom (+1) side; 0 leaves that axis alone.
    Resize([i8; 2]),
    Turn,
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    grip: Grip,
    /// The frame and pointer (photo pixels) when the drag started.
    frame: CropFrame,
    pointer: [f64; 2],
}

/// The crop tool's transient state (not saved with the photo).
#[derive(Default)]
pub struct CropTool {
    pub aspect: Aspect,
    drag: Option<Drag>,
    /// The frame when the angle slider started moving: turning it back
    /// gives the frame its size back.
    angle_base: Option<CropFrame>,
}

/// The frame the settings describe on a `photo`-sized photo.
fn current_frame(settings: &Settings, photo: [f64; 2]) -> CropFrame {
    photonation_pipeline::crop_frame(photo, settings.crop, settings.crop_angle)
}

/// Stores `frame` in the settings; the whole photo, unturned, is no crop.
fn store(settings: &mut Settings, frame: CropFrame) {
    let corners = frame.corners();
    let whole = (corners.left.abs() + corners.top.abs())
        + ((1.0 - corners.right).abs() + (1.0 - corners.bottom).abs())
        < 1e-9;
    if whole && frame.angle() == 0.0 {
        settings.crop = None;
    } else {
        settings.crop = Some(corners);
    }
    settings.crop_angle = frame.angle() as f32;
}

/// The frame of the photo's size, `size` pixels large, around `center`.
fn sized(frame: CropFrame, center: [f64; 2], size: [f64; 2]) -> CropFrame {
    CropFrame::from_center(frame.photo_size(), center, size, frame.angle())
}

/// `vector` turned `degrees` clockwise.
fn rotate(vector: [f64; 2], degrees: f64) -> [f64; 2] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [
        vector[0] * cos - vector[1] * sin,
        vector[0] * sin + vector[1] * cos,
    ]
}

/// The furthest frame on the way from `start` (which fits) to `target`
/// that still lies on the photo, so a drag stops at the photo's edge.
fn fitted(start: CropFrame, target: CropFrame) -> CropFrame {
    if target.fits() {
        return target;
    }
    let between = |t: f64| {
        let lerp = |a: [f64; 2], b: [f64; 2]| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        sized(
            start,
            lerp(start.center(), target.center()),
            lerp(start.size(), target.size()),
        )
    };
    let (mut fits, mut overshoots) = (0.0, 1.0);
    for _ in 0..FIT_STEPS {
        let middle = (fits + overshoots) / 2.0;
        if between(middle).fits() {
            fits = middle;
        } else {
            overshoots = middle;
        }
    }
    between(fits)
}

/// The frame after dragging `grip` from `drag.pointer` to `pointer` (photo
/// pixels), keeping `ratio` (width ÷ height) when set.
fn dragged(drag: &Drag, pointer: [f64; 2], ratio: Option<f64>) -> CropFrame {
    let start = drag.frame;
    let angle = start.angle();
    match drag.grip {
        Grip::Move => {
            let moved = [
                start.center()[0] + pointer[0] - drag.pointer[0],
                start.center()[1] + pointer[1] - drag.pointer[1],
            ];
            sized(start, moved, start.size()).constrained()
        }
        Grip::Turn => {
            let center = start.center();
            let direction = |p: [f64; 2]| (p[1] - center[1]).atan2(p[0] - center[0]);
            let turned = (direction(pointer) - direction(drag.pointer)).to_degrees();
            let turned = (turned + 180.0).rem_euclid(360.0) - 180.0;
            let angle = (angle + turned).clamp(-45.0, 45.0);
            start.with_angle(angle).constrained()
        }
        Grip::Resize(sides) => {
            let center = start.center();
            let [width, height] = start.size();
            // Work along the frame's own axes, from its centre.
            let local = rotate([pointer[0] - center[0], pointer[1] - center[1]], -angle);
            let half = [width / 2.0, height / 2.0];
            let mut low = [-half[0], -half[1]];
            let mut high = half;
            for axis in 0..2 {
                match sides[axis] {
                    -1 => low[axis] = local[axis].min(high[axis] - MIN_SIDE),
                    1 => high[axis] = local[axis].max(low[axis] + MIN_SIDE),
                    _ => {}
                }
            }
            if let Some(ratio) = ratio {
                lock_ratio(&mut low, &mut high, sides, ratio);
            }
            let local_center = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0];
            let offset = rotate(local_center, angle);
            let target = sized(
                start,
                [center[0] + offset[0], center[1] + offset[1]],
                [high[0] - low[0], high[1] - low[1]],
            );
            fitted(start, target)
        }
    }
}

/// Adjusts the dragged extent (`low..high` per axis) to `ratio`: a corner
/// follows the larger change, an edge drag resizes the other axis around
/// its middle.
fn lock_ratio(low: &mut [f64; 2], high: &mut [f64; 2], sides: [i8; 2], ratio: f64) {
    let width = high[0] - low[0];
    let height = high[1] - low[1];
    let (width, height) = match sides {
        [0, _] => (height * ratio, height),
        [_, 0] => (width, width / ratio),
        _ if width / height > ratio => (width, width / ratio),
        _ => (height * ratio, height),
    };
    for (axis, length) in [(0, width), (1, height)] {
        match sides[axis] {
            -1 => low[axis] = high[axis] - length,
            1 => high[axis] = low[axis] + length,
            _ => {
                let middle = (low[axis] + high[axis]) / 2.0;
                low[axis] = middle - length / 2.0;
                high[axis] = middle + length / 2.0;
            }
        }
    }
}

/// The largest frame of `ratio` inside `frame`, around its centre.
fn with_ratio(frame: CropFrame, ratio: f64) -> CropFrame {
    let [width, height] = frame.size();
    let size = if width / height > ratio {
        [height * ratio, height]
    } else {
        [width, width / ratio]
    };
    sized(frame, frame.center(), size).constrained()
}

/// The crop card in the sidebar, in place of the adjustments.
pub fn sidebar(app: &mut PhotonationApp, ui: &mut Ui) {
    let Some(info) = &app.info else {
        return;
    };
    let photo = [info.size.0 as f64, info.size.1 as f64];
    let settings = &mut app.settings;
    let tool = &mut app.view.crop_tool;
    let frame = current_frame(settings, photo);
    let mut done = false;
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Crop").size(14.0).color(theme::TEXT));
            let [width, height] = frame.size();
            ui.label(
                RichText::new(format!("{width:.0} × {height:.0}"))
                    .size(11.0)
                    .color(theme::TEXT_TERTIARY),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                done = tool_button(ui, "Done", false, "Back to the adjustments (R)").clicked();
            });
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new("Aspect")
                .size(12.0)
                .color(theme::TEXT_SECONDARY),
        );
        ui.horizontal_wrapped(|ui| {
            for (label, aspect) in ASPECTS {
                if tool_button(ui, label, tool.aspect == aspect, "").clicked() {
                    tool.aspect = aspect;
                    let portrait = frame.size()[1] > frame.size()[0];
                    if let Some(ratio) = aspect.ratio(photo, portrait) {
                        store(settings, with_ratio(frame, ratio));
                    }
                }
            }
            let swap = tool_button(ui, "⇔", false, "Swap portrait and landscape");
            if swap.clicked() {
                let [width, height] = frame.size();
                store(
                    settings,
                    sized(frame, frame.center(), [height, width]).constrained(),
                );
            }
        });
        ui.add_space(4.0);
        let control = &CROP_CONTROLS[0];
        let mut angle = settings.crop_angle;
        let style = SliderStyle {
            format: |v| format!("{v:+.1}°").replace("+0.0", "0.0"),
            ..SliderStyle::default()
        };
        let response = adjustment_slider(
            ui,
            control.label,
            &mut angle,
            control.min..=control.max,
            control.default,
            &style,
        );
        if response.changed() {
            let base = *tool.angle_base.get_or_insert(frame);
            store(settings, base.with_angle(angle as f64).constrained());
        }
        if !response.dragged() {
            tool.angle_base = None;
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Reset").clicked() {
                settings.crop = None;
                settings.crop_angle = 0.0;
            }
        });
        ui.label(
            RichText::new("Drag outside the frame to straighten.")
                .size(11.0)
                .color(theme::TEXT_TERTIARY),
        );
    });
    if done {
        app.view.cropping = false;
    }
}

/// Screen positions of the frame's corners, clockwise from its top-left.
fn screen_corners(frame: &CropFrame, view: &PhotoFrame) -> [Pos2; 4] {
    frame.corner_points().map(|corner| view.to_screen(corner))
}

/// Screen position of a frame point given normalized to the frame.
fn screen_point(frame: &CropFrame, view: &PhotoFrame, point: [f64; 2]) -> Pos2 {
    view.to_screen(frame.to_photo(point))
}

/// Photo pixels under a screen point.
fn photo_pixels(view: &PhotoFrame, screen: Pos2) -> [f64; 2] {
    let [x, y] = view.to_position(screen);
    let photo = view.crop.photo_size();
    [x * photo[0], y * photo[1]]
}

/// What a drag starting at `pointer` would grab.
fn grip_at(frame: &CropFrame, view: &PhotoFrame, pointer: Pos2) -> Grip {
    let nearest = (-1..=1i8)
        .flat_map(|x| (-1..=1i8).map(move |y| [x, y]))
        .filter(|sides| *sides != [0, 0])
        .map(|sides| {
            let point = sides.map(|side| (side as f64 + 1.0) / 2.0);
            (sides, screen_point(frame, view, point).distance(pointer))
        })
        .filter(|(_, distance)| *distance <= GRAB_DISTANCE)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((sides, _)) = nearest {
        return Grip::Resize(sides);
    }
    let [u, v] = frame.to_frame(view.to_position(pointer));
    if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) {
        Grip::Move
    } else {
        Grip::Turn
    }
}

fn cursor(grip: Grip, dragging: bool) -> CursorIcon {
    match grip {
        Grip::Move if dragging => CursorIcon::Grabbing,
        Grip::Move => CursorIcon::Grab,
        Grip::Turn => CursorIcon::Alias,
        Grip::Resize([0, _]) => CursorIcon::ResizeVertical,
        Grip::Resize([_, 0]) => CursorIcon::ResizeHorizontal,
        Grip::Resize([x, y]) if x == y => CursorIcon::ResizeNwSe,
        Grip::Resize(_) => CursorIcon::ResizeNeSw,
    }
}

/// Handles pointer input on the canvas. Returns whether a crop drag is
/// under way (so the canvas does not pan).
pub fn interact(
    app: &mut PhotonationApp,
    ui: &Ui,
    response: &egui::Response,
    view: &PhotoFrame,
) -> bool {
    let photo = view.crop.photo_size();
    let frame = current_frame(&app.settings, photo);
    let tool = &mut app.view.crop_tool;
    if response.drag_started()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        tool.drag = Some(Drag {
            grip: grip_at(&frame, view, pointer),
            frame,
            pointer: photo_pixels(view, pointer),
        });
    }
    if !response.dragged() {
        tool.drag = None;
    }
    let portrait = frame.size()[1] > frame.size()[0];
    let ratio = tool.aspect.ratio(photo, portrait);
    if let (Some(drag), Some(pointer)) = (tool.drag, response.interact_pointer_pos()) {
        ui.ctx().set_cursor_icon(cursor(drag.grip, true));
        store(
            &mut app.settings,
            dragged(&drag, photo_pixels(view, pointer), ratio),
        );
        return true;
    }
    if let Some(pointer) = response.hover_pos() {
        ui.ctx()
            .set_cursor_icon(cursor(grip_at(&frame, view, pointer), false));
    }
    false
}

/// Dims the photo outside the frame and draws the frame, a rule-of-thirds
/// grid and the grips.
pub fn paint(app: &PhotonationApp, painter: &Painter, view: &PhotoFrame) {
    let frame = current_frame(&app.settings, view.crop.photo_size());
    let corners = screen_corners(&frame, view);
    let image = view.image_rect;
    let outer = [
        image.left_top(),
        image.right_top(),
        image.right_bottom(),
        image.left_bottom(),
    ];
    painter.add(Shape::mesh(dimmed_surround(outer, corners)));

    let dragging = app.view.crop_tool.drag;
    let grid = Stroke::new(
        1.0,
        Color32::from_white_alpha(if dragging.is_some() { 120 } else { 50 }),
    );
    for third in [1.0 / 3.0, 2.0 / 3.0] {
        let line = |from: [f64; 2], to: [f64; 2]| {
            [
                screen_point(&frame, view, from),
                screen_point(&frame, view, to),
            ]
        };
        painter.line_segment(line([third, 0.0], [third, 1.0]), grid);
        painter.line_segment(line([0.0, third], [1.0, third]), grid);
    }
    let mut outline = corners.to_vec();
    outline.push(corners[0]);
    painter.add(Shape::line(outline, Stroke::new(1.0, theme::KNOB)));

    let grip_stroke = Stroke::new(3.0, theme::KNOB);
    let along = |from: Pos2, to: Pos2| (to - from).normalized() * GRIP_LENGTH;
    for (index, corner) in corners.iter().enumerate() {
        let next = corners[(index + 1) % 4];
        let previous = corners[(index + 3) % 4];
        painter.line_segment([*corner, *corner + along(*corner, next)], grip_stroke);
        painter.line_segment([*corner, *corner + along(*corner, previous)], grip_stroke);
        let middle = corner.lerp(next, 0.5);
        let half: Vec2 = along(*corner, next) / 2.0;
        painter.line_segment([middle - half, middle + half], grip_stroke);
    }

    if matches!(
        dragging,
        Some(Drag {
            grip: Grip::Turn,
            ..
        })
    ) {
        let top = corners[0].lerp(corners[1], 0.5);
        painter.text(
            pos2(top.x, top.y - 14.0),
            Align2::CENTER_BOTTOM,
            format!("{:+.1}°", frame.angle()),
            theme::body(13.0),
            theme::TEXT,
        );
    }
}

/// A mesh covering `outer` (a rectangle's corners, clockwise) except the
/// convex quad `inner` inside it: one quad per side between the two.
fn dimmed_surround(outer: [Pos2; 4], inner: [Pos2; 4]) -> Mesh {
    let shade = Color32::from_black_alpha(175);
    let mut mesh = Mesh::default();
    for point in outer.iter().chain(&inner) {
        mesh.colored_vertex(*point, shade);
    }
    for side in 0..4u32 {
        let next = (side + 1) % 4;
        mesh.add_triangle(side, next, 4 + next);
        mesh.add_triangle(side, 4 + next, 4 + side);
    }
    mesh
}

/// Whether two settings differ only in their crop (the crop tool's edits,
/// which do not change what it shows).
pub fn only_crop_differs(a: &Settings, b: &Settings) -> bool {
    uncropped(a.clone()) == uncropped(b.clone())
}

/// Settings for what the crop tool shows: the whole photo, unturned.
pub fn uncropped(settings: Settings) -> Settings {
    Settings {
        crop: None,
        crop_angle: 0.0,
        ..settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Rect;
    use photonation_image::NormRect;

    const PHOTO: [f64; 2] = [6000.0, 4000.0];

    fn drag(
        grip: Grip,
        frame: CropFrame,
        from: [f64; 2],
        to: [f64; 2],
        ratio: Option<f64>,
    ) -> CropFrame {
        dragged(
            &Drag {
                grip,
                frame,
                pointer: from,
            },
            to,
            ratio,
        )
    }

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
    }

    #[test]
    fn dragging_a_corner_keeps_the_opposite_one() {
        let frame = CropFrame::from_center(PHOTO, [3000.0, 2000.0], [2000.0, 1000.0], 0.0);
        let resized = drag(
            Grip::Resize([1, 1]),
            frame,
            [4000.0, 2500.0],
            [4500.0, 2900.0],
            None,
        );
        let corners = resized.corners();
        assert!(close(
            [corners.left, corners.top],
            [2000.0 / 6000.0, 1500.0 / 4000.0]
        ));
        assert!(close(
            [corners.right, corners.bottom],
            [4500.0 / 6000.0, 2900.0 / 4000.0]
        ));
    }

    #[test]
    fn locked_ratios_hold_while_resizing() {
        let frame = CropFrame::from_center(PHOTO, [3000.0, 2000.0], [1500.0, 1000.0], 7.0);
        for grip in [
            Grip::Resize([1, 1]),
            Grip::Resize([-1, 0]),
            Grip::Resize([0, 1]),
        ] {
            let resized = drag(grip, frame, [3700.0, 2400.0], [3900.0, 2800.0], Some(1.5));
            let [width, height] = resized.size();
            assert!((width / height - 1.5).abs() < 1e-9, "{grip:?}");
            assert!(resized.fits());
        }
    }

    #[test]
    fn resizing_stops_at_the_photo_edge() {
        let frame = CropFrame::from_center(PHOTO, [3000.0, 2000.0], [2000.0, 1000.0], 0.0);
        let resized = drag(
            Grip::Resize([1, 0]),
            frame,
            [4000.0, 2000.0],
            [9000.0, 2000.0],
            None,
        );
        assert!(resized.fits());
        assert!((resized.corners().right - 1.0).abs() < 1e-4);
        // The left edge did not move.
        assert!((resized.corners().left - 2000.0 / 6000.0).abs() < 1e-9);
    }

    #[test]
    fn moving_slides_along_the_photo_edge() {
        let frame = CropFrame::from_center(PHOTO, [3000.0, 2000.0], [2000.0, 1000.0], 0.0);
        let moved = drag(Grip::Move, frame, [3000.0, 2000.0], [9000.0, 2300.0], None);
        assert!(close(moved.center(), [5000.0, 2300.0]));
    }

    #[test]
    fn turning_follows_the_pointer_around_the_centre() {
        let frame = CropFrame::largest(PHOTO, 1.5, 0.0);
        // From right of the centre to slightly below it: clockwise.
        let turned = drag(
            Grip::Turn,
            frame,
            [5000.0, 2000.0],
            [5000.0, 2000.0 + 2000.0 * 0.0875],
            None,
        );
        assert!((turned.angle() - 5.0).abs() < 0.01, "{}", turned.angle());
        assert!(turned.fits());
    }

    #[test]
    fn ratios_follow_the_frame_orientation() {
        assert_eq!(Aspect::Ratio(4, 5).ratio(PHOTO, false), Some(1.25));
        assert_eq!(Aspect::Ratio(4, 5).ratio(PHOTO, true), Some(0.8));
        assert_eq!(Aspect::Original.ratio(PHOTO, true), Some(4000.0 / 6000.0));
        assert_eq!(Aspect::Free.ratio(PHOTO, false), None);
        let square = with_ratio(CropFrame::full(PHOTO), 1.0);
        assert!(close(square.size(), [4000.0, 4000.0]));
    }

    #[test]
    fn the_whole_photo_is_stored_as_no_crop() {
        let mut settings = Settings::default();
        store(&mut settings, CropFrame::full(PHOTO));
        assert_eq!(settings, Settings::default());
        store(&mut settings, CropFrame::largest(PHOTO, 1.5, 3.0));
        assert!(settings.crop.is_some());
        assert_eq!(settings.crop_angle, 3.0);
        // What is stored renders as the frame it came from.
        let frame = current_frame(&settings, PHOTO);
        assert!(close(
            frame.size(),
            CropFrame::largest(PHOTO, 1.5, 3.0).size()
        ));
    }

    #[test]
    fn the_surround_mesh_covers_the_outside_only() {
        let outer = [
            pos2(0.0, 0.0),
            pos2(100.0, 0.0),
            pos2(100.0, 80.0),
            pos2(0.0, 80.0),
        ];
        let inner = [
            pos2(20.0, 10.0),
            pos2(80.0, 15.0),
            pos2(75.0, 70.0),
            pos2(15.0, 65.0),
        ];
        let mesh = dimmed_surround(outer, inner);
        let area =
            |a: Pos2, b: Pos2, c: Pos2| ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs() / 2.0;
        let covered: f32 = mesh
            .indices
            .chunks_exact(3)
            .map(|t| {
                let v = |i: u32| mesh.vertices[i as usize].pos;
                area(v(t[0]), v(t[1]), v(t[2]))
            })
            .sum();
        let inner_area = area(inner[0], inner[1], inner[2]) + area(inner[0], inner[2], inner[3]);
        assert!((covered - (8000.0 - inner_area)).abs() < 1e-2);
    }

    #[test]
    fn only_crop_edits_are_recognized() {
        let a = Settings::default();
        let b = Settings {
            crop: Some(NormRect::FULL),
            crop_angle: 4.0,
            ..Settings::default()
        };
        assert!(only_crop_differs(&a, &b));
        let c = Settings {
            exposure: 1.0,
            ..b.clone()
        };
        assert!(!only_crop_differs(&b, &c));
    }

    #[test]
    fn grips_are_found_on_the_frame() {
        let view = PhotoFrame {
            image_rect: Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(600.0, 400.0)),
            crop: CropFrame::full(PHOTO),
            space: photonation_develop::MaskSpace::new(PHOTO[0], PHOTO[1]),
        };
        let frame = CropFrame::from_center(PHOTO, [3000.0, 2000.0], [3000.0, 2000.0], 0.0);
        assert_eq!(
            grip_at(&frame, &view, pos2(151.0, 101.0)),
            Grip::Resize([-1, -1])
        );
        assert_eq!(
            grip_at(&frame, &view, pos2(450.0, 200.0)),
            Grip::Resize([1, 0])
        );
        assert_eq!(grip_at(&frame, &view, pos2(300.0, 200.0)), Grip::Move);
        assert_eq!(grip_at(&frame, &view, pos2(20.0, 20.0)), Grip::Turn);
    }
}
