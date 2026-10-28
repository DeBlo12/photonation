// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The filmstrip: thumbnails of the folder's photos along the bottom.
//! It rests as a thin strip of tiny thumbnails so the photo gets the room,
//! and grows over the bottom of the viewer while the pointer is on it or
//! just after switching photos (see docs/DESKTOP.md).

use crate::app::{Photo, PhotonationApp};
use crate::theme;
use eframe::egui::{self, CornerRadius, Rangef, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

/// Thumbnail height when the strip is open.
const THUMB_HEIGHT: f32 = 72.0;
/// Thumbnail height when it rests.
const RESTING_THUMB_HEIGHT: f32 = 16.0;
const PLACEHOLDER_ASPECT: f32 = 1.5;
/// Space before the first thumbnail and after the last.
const END_PADDING: f32 = 14.0;
/// Gap plus stroke of the ring around the current photo.
const SELECTION_RING: f32 = 4.0;
/// Stay open this long after the pointer leaves, so brushing past the
/// edge doesn't make it flicker.
const LEAVE_DELAY: f64 = 0.5;
/// Open this long after the photo changes (arrow keys, opening a folder),
/// to show where in the folder the new photo is.
const NAVIGATION_REVEAL: f64 = 1.2;
/// Seconds the strip takes to open or close.
const ANIMATION_TIME: f32 = 0.18;

/// When the filmstrip is open. Time is egui's input time in seconds.
#[derive(Default)]
pub struct FilmstripReveal {
    open: bool,
    open_until: f64,
    shown_photo: Option<usize>,
}

impl FilmstripReveal {
    /// Whether the strip is open this frame. `pointer_over` is whether the
    /// pointer is on the strip as it was drawn last frame.
    pub fn update(&mut self, now: f64, pointer_over: bool, current_photo: Option<usize>) -> bool {
        if pointer_over {
            self.open_until = self.open_until.max(now + LEAVE_DELAY);
        }
        if current_photo != self.shown_photo {
            self.shown_photo = current_photo;
            self.open_until = self.open_until.max(now + NAVIGATION_REVEAL);
        }
        self.open = now < self.open_until;
        self.open
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Seconds until the strip closes by itself, if it is open.
    pub fn closes_in(&self, now: f64) -> Option<f64> {
        self.open.then(|| (self.open_until - now).max(0.0))
    }
}

pub fn show(app: &mut PhotonationApp, ui: &mut egui::Ui) {
    // Reserve only the resting strip; the open strip floats over the viewer
    // so the photo never changes size (and never re-renders) as it opens.
    let resting = egui::Panel::bottom("filmstrip")
        .exact_size(theme::FILMSTRIP_RESTING_HEIGHT)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(theme::WINDOW))
        .show(ui, |_| {})
        .response
        .rect;
    let ctx = ui.ctx().clone();
    let open_rect = Rect::from_min_max(
        pos2(resting.left(), resting.bottom() - theme::FILMSTRIP_HEIGHT),
        resting.right_bottom(),
    );

    let (now, pointer) = ctx.input(|i| (i.time, i.pointer.hover_pos()));
    let hover_zone = if app.view.filmstrip.is_open() {
        open_rect
    } else {
        resting
    };
    let open = app.view.filmstrip.update(
        now,
        pointer.is_some_and(|p| hover_zone.contains(p)),
        app.current,
    );
    if let Some(seconds) = app.view.filmstrip.closes_in(now) {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(seconds));
    }
    let openness =
        ctx.animate_bool_with_time(egui::Id::unique("filmstrip-open"), open, ANIMATION_TIME);
    let height = egui::lerp(
        theme::FILMSTRIP_RESTING_HEIGHT..=theme::FILMSTRIP_HEIGHT,
        openness,
    );
    let thumb_height = egui::lerp(RESTING_THUMB_HEIGHT..=THUMB_HEIGHT, openness);
    let strip = Rect::from_min_max(pos2(resting.left(), resting.bottom() - height), resting.max);

    egui::Area::new(egui::Id::unique("filmstrip-strip"))
        .order(egui::Order::Foreground)
        .fixed_pos(strip.min)
        .show(&ctx, |ui| {
            // Claim the whole strip: egui sends the wheel only to an area
            // whose rect holds the pointer, and the child below claims nothing.
            ui.allocate_rect(strip, Sense::hover());
            ui.painter().rect_filled(strip, 0.0, theme::WINDOW);
            ui.set_clip_rect(strip);
            // The scroll area spans the whole strip, so the wheel works
            // anywhere on it, not just on the thumbnails.
            let mut content_ui = ui.new_child(egui::UiBuilder::new().max_rect(strip));
            let pointer_x = pointer.filter(|p| strip.contains(*p)).map(|p| p.x);
            thumbnails(app, &mut content_ui, thumb_height, openness, pointer_x);
        });
}

/// The scrolling row of thumbnails. `pointer_x` is where the pointer is, if
/// it is on the strip.
fn thumbnails(
    app: &mut PhotonationApp,
    ui: &mut egui::Ui,
    thumb_height: f32,
    openness: f32,
    pointer_x: Option<f32>,
) {
    let layout = StripLayout::new(
        app.photos.iter().map(thumbnail_aspect),
        thumb_height,
        egui::lerp(3.0..=8.0, openness),
    );
    let visible = ui.max_rect().x_range();
    let anchor = pointer_x.map_or(visible.span() / 2.0, |x| x - visible.min);
    let forced_offset = app.view.filmstrip_scroll.forced_offset(
        layout.clone(),
        visible.span(),
        anchor,
        app.current,
    );

    // A mouse wheel scrolls vertically; this strip only goes sideways.
    ui.style_mut().always_scroll_the_only_direction = true;
    let mut scroll_area = egui::ScrollArea::horizontal()
        .auto_shrink([false; 2])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
    if let Some(offset) = forced_offset {
        scroll_area = scroll_area.horizontal_scroll_offset(offset);
    }
    let mut clicked = None;
    let output = scroll_area.show_viewport(ui, |ui, viewport| {
        let (content, _) =
            ui.allocate_exact_size(vec2(layout.width, ui.max_rect().height()), Sense::hover());
        let y_range = Rangef::point(content.center().y).expand(thumb_height / 2.0);
        // Draw only what is in view (plus the ring), for folders of thousands.
        let in_view = viewport.x_range().expand(SELECTION_RING);
        for (index, (photo, span)) in app.photos.iter().zip(&layout.thumbnails).enumerate() {
            if !span.intersects(in_view) {
                continue;
            }
            let x_range = Rangef::new(content.left() + span.min, content.left() + span.max);
            let rect = Rect::from_x_y_ranges(x_range, y_range);
            let selected = app.current == Some(index);
            let response = thumbnail(ui, index, rect, photo, selected, openness);
            if response.on_hover_text(&photo.name).clicked() {
                clicked = Some(index);
            }
        }
    });
    app.view
        .filmstrip_scroll
        .scrolled_to(output.state.clamped_offset().x);
    if let Some(index) = clicked {
        // It is in view already: keep the strip where it is.
        app.view.filmstrip_scroll.clicked(index);
        app.select(index);
    }
}

fn thumbnail_aspect(photo: &Photo) -> f32 {
    photo
        .thumbnail
        .as_ref()
        .map_or(PLACEHOLDER_ASPECT, |t| t.aspect_ratio())
}

/// Where the thumbnails sit along the strip, from the start of its content.
#[derive(Clone, Debug, Default, PartialEq)]
struct StripLayout {
    thumbnails: Vec<Rangef>,
    width: f32,
}

impl StripLayout {
    fn new(aspects: impl Iterator<Item = f32>, thumb_height: f32, spacing: f32) -> Self {
        // Room for the selection ring too, which the strip clips.
        let mut x = END_PADDING + SELECTION_RING;
        let thumbnails: Vec<Rangef> = aspects
            .map(|aspect| {
                let span = Rangef::new(x, x + thumb_height * aspect);
                x = span.max + spacing;
                span
            })
            .collect();
        let width = thumbnails.last().map_or(0.0, |last| last.max) + SELECTION_RING + END_PADDING;
        Self { thumbnails, width }
    }

    /// Where photo `index`'s slot (its thumbnail and the gap after it)
    /// starts and how wide it is.
    fn slot(&self, index: usize) -> (f32, f32) {
        let start = self.thumbnails[index].min;
        let end = self
            .thumbnails
            .get(index + 1)
            .map_or(self.width, |next| next.min);
        (start, end - start)
    }

    /// `x` as a photo index plus how far across that photo's slot it is,
    /// so a point can be found again after the thumbnails change size.
    fn position_at(&self, x: f32) -> f32 {
        if self.thumbnails.is_empty() {
            return 0.0;
        }
        let index = self
            .thumbnails
            .partition_point(|span| span.min <= x)
            .saturating_sub(1);
        let (start, width) = self.slot(index);
        index as f32 + (x - start) / width
    }

    /// Inverse of [`Self::position_at`].
    fn x_at(&self, position: f32) -> f32 {
        if self.thumbnails.is_empty() {
            return 0.0;
        }
        let index = (position.floor().max(0.0) as usize).min(self.thumbnails.len() - 1);
        let (start, width) = self.slot(index);
        start + (position - index as f32) * width
    }
}

/// Where the strip is scrolled to, where egui's scroll area alone would get
/// it wrong: it centres the current photo when that changes from elsewhere
/// (arrow keys, opening a folder), and holds the point under the pointer
/// (or the middle of the strip) still while thumbnails grow, shrink or load.
#[derive(Default)]
pub struct FilmstripScroll {
    previous_layout: StripLayout,
    previous_offset: f32,
    centred_photo: Option<usize>,
}

impl FilmstripScroll {
    /// The scroll offset to set this frame, if the strip should move by
    /// itself. `anchor` is the point to hold still, from the left of the
    /// `visible_width` wide strip.
    fn forced_offset(
        &mut self,
        layout: StripLayout,
        visible_width: f32,
        anchor: f32,
        current_photo: Option<usize>,
    ) -> Option<f32> {
        let new_folder = layout.thumbnails.len() != self.previous_layout.thumbnails.len();
        let offset = if current_photo != self.centred_photo || new_folder {
            self.centred_photo = current_photo;
            current_photo
                .and_then(|index| layout.thumbnails.get(index))
                .map(|span| span.center() - visible_width / 2.0)
        } else if layout != self.previous_layout {
            let position = self
                .previous_layout
                .position_at(self.previous_offset + anchor);
            Some(layout.x_at(position) - anchor)
        } else {
            None
        };
        let max_offset = (layout.width - visible_width).max(0.0);
        self.previous_layout = layout;
        offset.map(|offset| offset.clamp(0.0, max_offset))
    }

    /// Where the scroll area ended up this frame.
    fn scrolled_to(&mut self, offset: f32) {
        self.previous_offset = offset;
    }

    /// Photo `index` was clicked, so it is in view: don't centre it.
    fn clicked(&mut self, index: usize) {
        self.centred_photo = Some(index);
    }
}

/// One thumbnail; `openness` (0 resting, 1 open) fades in the details
/// that don't fit a tiny thumbnail.
fn thumbnail(
    ui: &egui::Ui,
    index: usize,
    rect: Rect,
    photo: &Photo,
    selected: bool,
    openness: f32,
) -> egui::Response {
    let id = egui::Id::unique(("filmstrip-thumbnail", index));
    let response = ui.interact(rect, id, Sense::click());
    let painter = ui.painter();
    let corner = egui::lerp(2.0..=f32::from(theme::CONTROL_RADIUS), openness);
    let radius = CornerRadius::same(corner.round() as u8);
    match &photo.thumbnail {
        Some(texture) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.add(
                egui::epaint::RectShape::filled(rect, radius, egui::Color32::WHITE)
                    .with_texture(texture.id(), uv),
            );
        }
        None => {
            painter.rect_filled(rect, radius, theme::CARD);
            if photo.unsupported && openness > 0.5 {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Unsupported",
                    theme::body(10.5),
                    theme::TEXT_TERTIARY,
                );
            }
        }
    }
    if selected {
        painter.rect_stroke(
            rect.expand(SELECTION_RING / 2.0),
            CornerRadius::same(radius.nw + 2),
            Stroke::new(SELECTION_RING / 2.0, theme::ACCENT),
            StrokeKind::Outside,
        );
    } else {
        let dim = if response.hovered() { 20 } else { 70 };
        painter.rect_filled(rect, radius, egui::Color32::from_black_alpha(dim));
    }
    if photo.has_edits && openness > 0.5 {
        painter.circle_filled(
            pos2(rect.right() - 8.0, rect.top() + 8.0),
            3.5,
            theme::ACCENT,
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_while_the_pointer_is_on_it_and_shortly_after() {
        let mut reveal = FilmstripReveal::default();
        reveal.update(0.0, false, Some(0));
        assert!(!reveal.update(10.0, false, Some(0)), "rests by default");
        assert!(reveal.update(11.0, true, Some(0)));
        assert!(reveal.update(11.0 + LEAVE_DELAY - 0.1, false, Some(0)));
        assert!(!reveal.update(11.0 + LEAVE_DELAY + 0.1, false, Some(0)));
    }

    #[test]
    fn opens_briefly_when_the_photo_changes() {
        let mut reveal = FilmstripReveal::default();
        reveal.update(0.0, false, Some(0));
        assert!(!reveal.update(10.0, false, Some(0)));
        assert!(reveal.update(20.0, false, Some(1)));
        let closes_in = reveal.closes_in(20.0).expect("open");
        assert!((closes_in - NAVIGATION_REVEAL).abs() < 1e-9);
        assert!(!reveal.update(20.0 + NAVIGATION_REVEAL + 0.1, false, Some(1)));
        assert_eq!(reveal.closes_in(30.0), None);
    }

    /// `count` square thumbnails, a fifth of their height apart.
    fn strip(count: usize, thumb_height: f32) -> StripLayout {
        let spacing = thumb_height / 5.0;
        StripLayout::new(std::iter::repeat_n(1.0, count), thumb_height, spacing)
    }

    #[test]
    fn lays_thumbnails_out_with_room_for_the_ring_at_both_ends() {
        let layout = StripLayout::new([1.5, 0.5].into_iter(), 10.0, 2.0);
        let start = END_PADDING + SELECTION_RING;
        assert_eq!(layout.thumbnails[0], Rangef::new(start, start + 15.0));
        assert_eq!(
            layout.thumbnails[1],
            Rangef::new(start + 17.0, start + 22.0)
        );
        assert_eq!(layout.width, start + 22.0 + start);
    }

    #[test]
    fn finds_a_point_again_in_a_resized_strip() {
        let small = strip(5, 10.0);
        let large = strip(5, 50.0);
        // The middle of the third thumbnail stays the middle of it.
        let middle = small.thumbnails[2].center();
        let position = small.position_at(middle);
        assert!((large.x_at(position) - large.thumbnails[2].center()).abs() < 1e-3);
        for x in [-3.0, 0.0, 27.0, small.width + 5.0] {
            assert!((small.x_at(small.position_at(x)) - x).abs() < 1e-3, "{x}");
        }
    }

    #[test]
    fn centres_the_current_photo_when_it_changes() {
        let layout = strip(100, 10.0);
        let mut scroll = FilmstripScroll::default();
        let offset = scroll.forced_offset(layout.clone(), 120.0, 60.0, Some(50));
        let offset = offset.expect("centres it");
        assert_eq!(offset, layout.thumbnails[50].center() - 60.0);
        scroll.scrolled_to(offset);
        assert_eq!(
            scroll.forced_offset(layout.clone(), 120.0, 60.0, Some(50)),
            None,
            "leaves the strip to the user while nothing changes"
        );
        assert_eq!(
            scroll.forced_offset(layout.clone(), 120.0, 60.0, Some(0)),
            Some(0.0),
            "stops at the start"
        );
        assert_eq!(
            scroll.forced_offset(layout.clone(), 120.0, 60.0, Some(99)),
            Some(layout.width - 120.0),
            "stops at the end"
        );
    }

    #[test]
    fn does_not_move_a_clicked_photo() {
        let layout = strip(100, 10.0);
        let mut scroll = FilmstripScroll::default();
        scroll.forced_offset(layout.clone(), 120.0, 60.0, Some(0));
        scroll.clicked(7);
        assert_eq!(scroll.forced_offset(layout, 120.0, 60.0, Some(7)), None);
    }

    #[test]
    fn holds_the_thumbnail_under_the_pointer_while_the_strip_opens() {
        let mut scroll = FilmstripScroll::default();
        let resting = strip(100, 10.0);
        scroll.forced_offset(resting.clone(), 120.0, 60.0, Some(0));
        scroll.scrolled_to(300.0);
        let pointer = 40.0;
        let under_pointer = resting.position_at(300.0 + pointer);
        let open = strip(100, 50.0);
        let offset = scroll.forced_offset(open.clone(), 120.0, pointer, Some(0));
        let offset = offset.expect("moves with the thumbnails");
        assert!((open.position_at(offset + pointer) - under_pointer).abs() < 1e-4);
    }
}

/// The strip drawn headless, fed the events a window would send.
#[cfg(test)]
mod scrolling {
    use super::*;
    use eframe::egui::{Event, MouseWheelSource, MouseWheelUnit, RawInput, TouchPhase};

    const SCREEN: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1200.0, 800.0));
    const ON_THE_VIEWER: egui::Pos2 = pos2(600.0, 100.0);
    /// On the resting strip, below its thumbnails.
    const ON_THE_STRIP: egui::Pos2 = pos2(600.0, 797.0);

    struct Window {
        ctx: egui::Context,
        app: PhotonationApp,
        time: f64,
    }

    impl Window {
        /// A folder of `count` photos, the first one open.
        fn with_photos(count: usize) -> Self {
            let ctx = egui::Context::default();
            let mut app = PhotonationApp::empty(&ctx);
            app.photos = (0..count)
                .map(|i| Photo {
                    path: format!("{i}.CR3").into(),
                    name: format!("{i}.CR3"),
                    thumbnail: None,
                    has_edits: false,
                    unsupported: false,
                })
                .collect();
            app.current = Some(0);
            let mut window = Window {
                ctx,
                app,
                time: 0.0,
            };
            window.frame(vec![Event::PointerMoved(ON_THE_VIEWER)]);
            window.frames(10);
            window
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
            let app = &mut self.app;
            let mut output = self.ctx.run_ui(input, |ui| show(app, ui));
            output.textures_delta.clear();
        }

        fn frames(&mut self, count: usize) {
            for _ in 0..count {
                self.frame(Vec::new());
            }
        }

        fn offset(&self) -> f32 {
            self.app.view.filmstrip_scroll.previous_offset
        }
    }

    fn scroll(delta: egui::Vec2, unit: MouseWheelUnit, source: MouseWheelSource) -> Event {
        Event::MouseWheel {
            unit,
            delta,
            phase: TouchPhase::Move,
            source,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn a_mouse_wheel_scrolls_it_sideways_from_anywhere_on_the_strip() {
        let mut window = Window::with_photos(96);
        assert_eq!(window.offset(), 0.0);
        window.frame(vec![Event::PointerMoved(ON_THE_STRIP)]);
        let down = scroll(
            vec2(0.0, -5.0),
            MouseWheelUnit::Line,
            MouseWheelSource::Wheel,
        );
        window.frame(vec![down]);
        window.frames(30);
        assert!(window.offset() > 100.0, "{}", window.offset());
    }

    #[test]
    fn a_trackpad_swipe_scrolls_it() {
        let mut window = Window::with_photos(96);
        window.frame(vec![Event::PointerMoved(ON_THE_STRIP)]);
        window.frames(30);
        let swipe = scroll(
            vec2(-40.0, 0.0),
            MouseWheelUnit::Point,
            MouseWheelSource::Trackpad,
        );
        for _ in 0..10 {
            window.frame(vec![swipe.clone()]);
        }
        window.frames(30);
        assert!(window.offset() > 300.0, "{}", window.offset());
    }

    #[test]
    fn keeps_a_photo_chosen_elsewhere_centred_as_the_strip_opens_and_closes() {
        let mut window = Window::with_photos(96);
        window.app.current = Some(50);
        let centre_offset = |window: &Window| {
            let scroll = &window.app.view.filmstrip_scroll;
            let centre = scroll.previous_layout.thumbnails[50].center() - scroll.previous_offset;
            centre - SCREEN.width() / 2.0
        };
        window.frames(10);
        assert!(window.app.view.filmstrip.is_open());
        assert!(centre_offset(&window).abs() < 2.0);
        window.frames(150);
        assert!(!window.app.view.filmstrip.is_open());
        assert!(centre_offset(&window).abs() < 2.0);
    }
}
