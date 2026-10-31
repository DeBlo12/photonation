// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The filmstrip: thumbnails of the shown photos along the bottom, in the
//! library's filter and sort order. It rests as a thin strip of tiny
//! thumbnails so the photo gets the room, and grows over the bottom of the
//! viewer while the pointer is on it or just after switching photos. A
//! scroll bar along its bottom edge shows while it scrolls or the pointer
//! is on the bar, for crossing long folders quickly (see docs/DESKTOP.md).

use crate::app::{Photo, PhotonationApp};
use crate::theme;
use crate::widgets::paint_badges;
use eframe::egui::{self, CornerRadius, Rangef, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

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
/// The scroll bar's thickness at rest and under the pointer.
const BAR_THICKNESS: f32 = 4.0;
const BAR_HOVER_THICKNESS: f32 = 8.0;
/// Gap between the bar and the strip's bottom and ends.
const BAR_INSET: f32 = 3.0;
const BAR_SIDE_INSET: f32 = 12.0;
const BAR_MIN_THUMB: f32 = 32.0;
/// How long the bar stays after the strip last moved.
const BAR_LINGER: f64 = 1.2;

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
            let (layout, offset) =
                thumbnails(app, &mut content_ui, thumb_height, openness, pointer_x);
            scroll_bar(app, ui, strip, &layout, offset, openness);
        });
}

/// The scrolling row of thumbnails. `pointer_x` is where the pointer is, if
/// it is on the strip. Returns the layout and where the strip is scrolled to.
fn thumbnails(
    app: &mut PhotonationApp,
    ui: &mut egui::Ui,
    thumb_height: f32,
    openness: f32,
    pointer_x: Option<f32>,
) -> (StripLayout, f32) {
    let shown = app.view.library.shown.clone();
    let layout = StripLayout::new(
        shown
            .iter()
            .map(|&index| thumbnail_aspect(&app.photos[index])),
        thumb_height,
        egui::lerp(3.0..=8.0, openness),
    );
    let visible = ui.max_rect().x_range();
    let anchor = pointer_x.map_or(visible.span() / 2.0, |x| x - visible.min);
    let current_position = app
        .current
        .and_then(|current| shown.iter().position(|&index| index == current));
    let forced_offset = app.view.filmstrip_scroll.forced_offset(
        layout.clone(),
        visible.span(),
        anchor,
        current_position,
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
        for (position, (&index, span)) in shown.iter().zip(&layout.thumbnails).enumerate() {
            if !span.intersects(in_view) {
                continue;
            }
            let x_range = Rangef::new(content.left() + span.min, content.left() + span.max);
            let rect = Rect::from_x_y_ranges(x_range, y_range);
            let photo = &app.photos[index];
            let selection = if app.current == Some(index) {
                Selection::Current
            } else if app.view.library.selection.contains(&index) {
                Selection::Selected
            } else {
                Selection::None
            };
            let response = thumbnail(ui, index, rect, photo, selection, openness);
            if response.on_hover_text(&photo.name).clicked() {
                clicked = Some((position, index, ui.input(|i| i.modifiers)));
            }
        }
    });
    let offset = output.state.clamped_offset().x;
    app.view.filmstrip_scroll.scrolled_to(offset);
    if let Some((position, index, modifiers)) = clicked {
        // It is in view already: keep the strip where it is.
        app.view.filmstrip_scroll.clicked(position);
        app.click_photo(index, modifiers);
    }
    (layout, offset)
}

/// The bar along the bottom of the strip: it shows while the strip
/// scrolls, while the pointer is on it and while it is dragged. Dragging
/// its thumb crosses the folder quickly, with the name and date of the
/// photo in the middle of the strip above it; clicking the track jumps
/// there.
fn scroll_bar(
    app: &mut PhotonationApp,
    ui: &mut egui::Ui,
    strip: Rect,
    layout: &StripLayout,
    offset: f32,
    openness: f32,
) {
    let visible_width = strip.width();
    let scrollable = layout.width - visible_width;
    if scrollable < 1.0 {
        return;
    }
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    let id = egui::Id::unique("filmstrip-scroll-bar");
    // The hit area stays in the margin under the thumbnails.
    let hit_height = egui::lerp(5.0..=14.0, openness);
    let hit = Rect::from_min_max(
        pos2(strip.left() + BAR_SIDE_INSET, strip.bottom() - hit_height),
        pos2(strip.right() - BAR_SIDE_INSET, strip.bottom()),
    );
    let response = ui.interact(hit, id, Sense::click_and_drag());
    let bar = &mut app.view.filmstrip_bar;
    let geometry = BarGeometry::new(hit.x_range(), visible_width, layout.width);
    let mut jump = None;
    if let Some(pointer) = response.interact_pointer_pos() {
        let thumb = geometry.thumb(offset);
        if response.drag_started() || (response.clicked() && bar.grab.is_none()) {
            // On the thumb, hold it where it was grabbed; elsewhere,
            // centre it on the pointer.
            bar.grab = Some(if thumb.contains(pointer.x) {
                pointer.x - thumb.min
            } else {
                thumb.span() / 2.0
            });
        }
        if let Some(grab) = bar.grab {
            jump = Some(geometry.offset_for(pointer.x - grab));
        }
    }
    if !response.dragged() && !response.is_pointer_button_down_on() {
        bar.grab = None;
    }
    if (offset - bar.last_offset).abs() > 0.5 {
        bar.active_until = now + BAR_LINGER;
    }
    bar.last_offset = offset;
    let dragging = bar.grab.is_some();
    let shown = dragging || response.hovered() || now < bar.active_until;
    if now < bar.active_until {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(bar.active_until - now));
    }
    let opacity = ctx.animate_bool_with_time(id.with("shown"), shown, 0.15);
    let emphasis =
        ctx.animate_bool_with_time(id.with("hover"), dragging || response.hovered(), 0.1);
    if let Some(offset) = jump {
        app.view.filmstrip_scroll.jump_to(offset);
        ctx.request_repaint();
    }
    if opacity <= 0.0 {
        return;
    }
    let thickness = egui::lerp(BAR_THICKNESS..=BAR_HOVER_THICKNESS, emphasis)
        * egui::lerp(0.75..=1.0, openness);
    let bottom = strip.bottom() - BAR_INSET * egui::lerp(0.5..=1.0, openness);
    let track = Rect::from_min_max(
        pos2(hit.left(), bottom - thickness),
        pos2(hit.right(), bottom),
    );
    let thumb = geometry.thumb(jump.unwrap_or(offset));
    let thumb = Rect::from_x_y_ranges(thumb, track.y_range());
    let painter = ui.painter();
    let radius = CornerRadius::same((thickness / 2.0).round() as u8);
    painter.rect_filled(
        track,
        radius,
        theme::CONTROL.gamma_multiply(0.85 * opacity * emphasis),
    );
    let thumb_color = if dragging {
        theme::ACCENT
    } else {
        theme::TEXT_SECONDARY
    };
    painter.rect_filled(thumb, radius, thumb_color.gamma_multiply(opacity));

    if dragging && openness > 0.5 {
        // The photo in the middle of the strip, to see where the drag is.
        let middle = layout.position_at(jump.unwrap_or(offset) + visible_width / 2.0);
        let shown = &app.view.library.shown;
        let Some(&index) = shown.get((middle.floor().max(0.0) as usize).min(shown.len() - 1))
        else {
            return;
        };
        let photo = &app.photos[index];
        let text = match photo
            .captured
            .as_deref()
            .and_then(crate::library::capture_time_label)
        {
            Some(date) => format!("{} · {date}", photo.name),
            None => photo.name.clone(),
        };
        let galley = painter.layout_no_wrap(text, theme::body(11.5), theme::TEXT);
        let size = galley.size() + vec2(14.0, 6.0);
        let x = thumb
            .center()
            .x
            .clamp(strip.left() + size.x / 2.0, strip.right() - size.x / 2.0);
        let bubble = Rect::from_center_size(pos2(x, track.top() - 4.0 - size.y / 2.0), size);
        painter.rect_filled(bubble, size.y / 2.0, theme::CARD.gamma_multiply(0.96));
        painter.galley(bubble.center() - galley.size() / 2.0, galley, theme::TEXT);
    }
}

/// The scroll bar's dragging state.
#[derive(Default)]
pub struct FilmstripScrollBar {
    /// Where the pointer holds the thumb, from its left end, while dragging.
    grab: Option<f32>,
    last_offset: f32,
    /// Shown until then (egui time), after the strip last moved.
    active_until: f64,
}

/// Where the thumb sits on the track for a scroll offset, and back.
struct BarGeometry {
    track: Rangef,
    thumb_width: f32,
    scrollable: f32,
}

impl BarGeometry {
    fn new(track: Rangef, visible_width: f32, content_width: f32) -> Self {
        let thumb_width = (track.span() * visible_width / content_width)
            .clamp(BAR_MIN_THUMB.min(track.span()), track.span());
        BarGeometry {
            track,
            thumb_width,
            scrollable: (content_width - visible_width).max(0.0),
        }
    }

    fn travel(&self) -> f32 {
        (self.track.span() - self.thumb_width).max(1e-3)
    }

    fn thumb(&self, offset: f32) -> Rangef {
        let fraction = if self.scrollable > 0.0 {
            (offset / self.scrollable).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let start = self.track.min + fraction * self.travel();
        Rangef::new(start, start + self.thumb_width)
    }

    /// The offset that puts the thumb's left end at `thumb_left`.
    fn offset_for(&self, thumb_left: f32) -> f32 {
        ((thumb_left - self.track.min) / self.travel()).clamp(0.0, 1.0) * self.scrollable
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
    /// Asked for by the scroll bar, for the next frame.
    jump: Option<f32>,
}

impl FilmstripScroll {
    /// The scroll offset to set this frame, if the strip should move by
    /// itself. `anchor` is the point to hold still, from the left of the
    /// `visible_width` wide strip. `current_photo` is the current photo's
    /// place among the thumbnails.
    fn forced_offset(
        &mut self,
        layout: StripLayout,
        visible_width: f32,
        anchor: f32,
        current_photo: Option<usize>,
    ) -> Option<f32> {
        let new_folder = layout.thumbnails.len() != self.previous_layout.thumbnails.len();
        let offset = if let Some(jump) = self.jump.take() {
            Some(jump)
        } else if current_photo != self.centred_photo || new_folder {
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

    /// The thumbnail at `position` was clicked, so it is in view: don't
    /// centre it.
    fn clicked(&mut self, position: usize) {
        self.centred_photo = Some(position);
    }

    /// Scroll to `offset` next frame (the scroll bar was dragged).
    fn jump_to(&mut self, offset: f32) {
        self.jump = Some(offset);
    }
}

/// How a thumbnail is marked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    None,
    /// Selected besides the current photo.
    Selected,
    Current,
}

/// One thumbnail; `openness` (0 resting, 1 open) fades in the details
/// that don't fit a tiny thumbnail.
fn thumbnail(
    ui: &egui::Ui,
    index: usize,
    rect: Rect,
    photo: &Photo,
    selection: Selection,
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
    let rejected = photo.library.flag == photonation_develop::Flag::Reject;
    match selection {
        Selection::Current => {
            painter.rect_stroke(
                rect.expand(SELECTION_RING / 2.0),
                CornerRadius::same(radius.nw + 2),
                Stroke::new(SELECTION_RING / 2.0, theme::ACCENT),
                StrokeKind::Outside,
            );
        }
        Selection::Selected => {
            painter.rect_stroke(
                rect.expand(SELECTION_RING / 2.0),
                CornerRadius::same(radius.nw + 2),
                Stroke::new(1.5, theme::TEXT_SECONDARY),
                StrokeKind::Outside,
            );
        }
        Selection::None => {
            let dim = match (rejected, response.hovered()) {
                (true, _) => 150,
                (false, true) => 20,
                (false, false) => 70,
            };
            painter.rect_filled(rect, radius, egui::Color32::from_black_alpha(dim));
        }
    }
    if openness > 0.5 {
        paint_badges(painter, rect, photo.library, 0.8);
    } else if let Some(label) = photo.library.label {
        // A resting strip shows the label only, as a line under the photo.
        let line = Rect::from_min_size(rect.left_bottom() + Vec2::Y, vec2(rect.width(), 2.0));
        painter.rect_filled(line, 0.0, crate::widgets::label_color(label));
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
                .map(|i| Photo::new(format!("{i}.CR3").into()))
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
            let mut output = self.ctx.run_ui(input, |ui| {
                app.arrange_photos();
                show(app, ui)
            });
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
