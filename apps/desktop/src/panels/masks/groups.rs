// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Mask groups in the layer list: which rows the list shows (a group's
//! header, then its masks unless it is collapsed), where a dragged row
//! lands, the edits that move masks into, out of and between groups, and
//! the group's header row and settings card. Masks stay one flat list in
//! which every group's masks sit together (`photonation_develop::
//! tidy_groups`); every edit here keeps it that way. See docs/DESKTOP.md.

use super::{add_mask, duplicate_mask, hint, label, move_mask, remove_mask, single_line};
use crate::app::MaskSelection;
use crate::theme;
use crate::widgets::{self, SliderStyle, adjustment_slider};
use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, pos2, vec2,
};
use photonation_develop::{
    MAX_MASK_GROUP_NAME_CHARS, MAX_MASK_GROUPS, Mask, MaskGroup, MaskShape, group_members,
    remove_group,
};
use std::collections::HashSet;

pub const GROUP_ROW_HEIGHT: f32 = 38.0;
/// How far a group's masks are indented under its header.
pub const GROUP_INDENT: f32 = 14.0;

/// The layer list's group state that is not part of the edit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupView {
    /// Index into `settings.mask_groups`; the panel shows the group's card
    /// instead of a mask's. Never set together with a mask selection.
    pub selected: Option<usize>,
    /// Groups whose masks the list hides.
    pub collapsed: HashSet<usize>,
    /// Put the keyboard in the selected group's name field once.
    pub focus_name: bool,
}

impl GroupView {
    /// Drops what undo or a deletion made invalid.
    pub fn validate(&mut self, groups: usize) {
        self.selected = self.selected.filter(|group| *group < groups);
        self.collapsed.retain(|group| *group < groups);
    }

    /// Follows the renumbering after group `removed` is removed.
    fn forget(&mut self, removed: usize) {
        let renumber = |group: usize| match group {
            g if g == removed => None,
            g if g > removed => Some(g - 1),
            g => Some(g),
        };
        self.selected = self.selected.and_then(renumber);
        self.collapsed = self.collapsed.iter().filter_map(|g| renumber(*g)).collect();
    }
}

/// Changes to the layer list, applied after it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerAction {
    /// A new mask of this shape, in the current group if there is one.
    Add(MaskShape),
    Select(usize),
    Duplicate(usize),
    Delete(usize),
    /// Move a mask to a slot `0..=len` (before the mask at that index) and
    /// into `group`. The slot must keep every group's masks together.
    Move {
        from: usize,
        to_slot: usize,
        group: Option<usize>,
    },
    /// Move a mask to the end of `group`, or (`None`) out of its group to
    /// just below it.
    MoveToGroup {
        mask: usize,
        group: Option<usize>,
    },
    /// Put a mask in a new group of its own, and select the group.
    NewGroup(usize),
    SelectGroup(usize),
    /// Select a group with its name field focused.
    RenameGroup(usize),
    ToggleCollapsed(usize),
    /// Move a group's masks to a slot outside any group.
    MoveGroup {
        group: usize,
        to_slot: usize,
    },
    /// Remove a group, keeping its masks.
    Ungroup(usize),
    /// Remove a group and its masks.
    DeleteGroup(usize),
}

/// A row of the layer list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListRow {
    /// A group's header.
    Group(usize),
    /// A mask (indented when it is in a group).
    Mask(usize),
}

/// The rows in list order: each group's header where its masks start,
/// followed by its masks unless the group is collapsed.
pub fn list_rows(masks: &[Mask], collapsed: &HashSet<usize>) -> Vec<ListRow> {
    let mut rows = Vec::with_capacity(masks.len());
    for (index, mask) in masks.iter().enumerate() {
        match mask.group {
            Some(group) => {
                if index == 0 || masks[index - 1].group != Some(group) {
                    rows.push(ListRow::Group(group));
                }
                if !collapsed.contains(&group) {
                    rows.push(ListRow::Mask(index));
                }
            }
            None => rows.push(ListRow::Mask(index)),
        }
    }
    rows
}

/// Where a dragged row would land.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropTarget {
    /// Slot in the mask list, 0..=len.
    pub slot: usize,
    /// The group a dropped mask joins.
    pub group: Option<usize>,
    pub marker: DropMarker,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropMarker {
    /// A line between rows at this height, indented inside a group.
    Line { y: f32, indented: bool },
    /// Onto a group's header: to the end of that group.
    Onto(Rect),
}

/// Where `dragged`, dropped at height `y`, goes among `rows` (as laid
/// out). A mask dropped on the middle of a group's header joins the group
/// at its end; between rows it lands in that gap, in the group when the
/// row below is one of the group's masks. Groups don't nest, so a group
/// dropped inside an open group goes above that group instead.
pub fn drop_target(
    masks: &[Mask],
    rows: &[(ListRow, Rect)],
    dragged: ListRow,
    y: f32,
) -> Option<DropTarget> {
    let last = rows.last()?.1;
    if let ListRow::Mask(_) = dragged {
        for (row, rect) in rows {
            let middle = rect.shrink2(vec2(0.0, rect.height() / 4.0));
            if let ListRow::Group(group) = *row
                && middle.y_range().contains(y)
            {
                return Some(DropTarget {
                    slot: group_members(masks, group).end,
                    group: Some(group),
                    marker: DropMarker::Onto(*rect),
                });
            }
        }
    }
    let mut below = rows.iter().filter(|(_, rect)| rect.center().y < y).count();
    if let ListRow::Group(_) = dragged
        && let Some((ListRow::Mask(index), _)) = rows.get(below)
        && let Some(group) = masks[*index].group
    {
        below = rows
            .iter()
            .position(|(row, _)| *row == ListRow::Group(group))
            .unwrap_or(below);
    }
    let (slot, group) = match rows.get(below) {
        Some((ListRow::Mask(index), _)) => (*index, masks[*index].group),
        Some((ListRow::Group(group), _)) => (group_members(masks, *group).start, None),
        None => (masks.len(), None),
    };
    let y = rows
        .get(below)
        .map_or(last.bottom() + 2.0, |(_, rect)| rect.top() - 2.0);
    Some(DropTarget {
        slot,
        group,
        marker: DropMarker::Line {
            y,
            indented: group.is_some(),
        },
    })
}

impl DropTarget {
    /// The edit dropping `dragged` here makes, if it changes anything.
    pub fn action(&self, masks: &[Mask], dragged: ListRow) -> Option<LayerAction> {
        match dragged {
            ListRow::Mask(from) => {
                let stays = self.slot == from || self.slot == from + 1;
                (!stays || masks.get(from)?.group != self.group).then_some(LayerAction::Move {
                    from,
                    to_slot: self.slot,
                    group: self.group,
                })
            }
            ListRow::Group(group) => {
                let members = group_members(masks, group);
                (!(members.start..=members.end).contains(&self.slot)).then_some(
                    LayerAction::MoveGroup {
                        group,
                        to_slot: self.slot,
                    },
                )
            }
        }
    }

    pub fn paint(&self, ui: &Ui, rows: &[(ListRow, Rect)]) {
        let Some(left) = rows.iter().map(|(_, rect)| rect.left()).reduce(f32::min) else {
            return;
        };
        let right = rows[0].1.right();
        let stroke = Stroke::new(2.0, theme::ACCENT);
        match self.marker {
            DropMarker::Line { y, indented } => {
                let left = if indented { left + GROUP_INDENT } else { left };
                ui.painter().hline(left..=right, y, stroke);
            }
            DropMarker::Onto(rect) => {
                let radius = CornerRadius::same(theme::CONTROL_RADIUS);
                ui.painter()
                    .rect_stroke(rect, radius, stroke, StrokeKind::Inside);
            }
        }
    }
}

/// The slot "Move Up" (or down) takes a mask to: past its neighbour in the
/// same group, or for an ungrouped mask, past a whole neighbouring group.
/// `None` at the top (bottom) of the list or of its group.
pub fn step_slot(masks: &[Mask], index: usize, up: bool) -> Option<usize> {
    let group = masks.get(index)?.group;
    let neighbour = if up { index.checked_sub(1)? } else { index + 1 };
    let other = masks.get(neighbour)?.group;
    match (group, other) {
        (Some(_), _) if other != group => None,
        (None, Some(other)) => {
            let members = group_members(masks, other);
            Some(if up { members.start } else { members.end })
        }
        _ => Some(if up { index - 1 } else { index + 2 }),
    }
}

/// Everything layer list edits change.
pub struct Layers<'a> {
    pub masks: &'a mut Vec<Mask>,
    pub groups: &'a mut Vec<MaskGroup>,
    pub selection: &'a mut Option<MaskSelection>,
    pub view: &'a mut GroupView,
}

impl Layers<'_> {
    pub fn apply(&mut self, action: LayerAction) {
        match action {
            LayerAction::Add(shape) => self.add(shape),
            LayerAction::Select(index) => {
                if self.selection.map(|s| s.mask) != Some(index) {
                    *self.selection = Some(MaskSelection::mask(index));
                }
                self.view.selected = None;
            }
            LayerAction::Duplicate(index) => duplicate_mask(self.masks, self.selection, index),
            LayerAction::Delete(index) => self.delete_mask(index),
            LayerAction::Move {
                from,
                to_slot,
                group,
            } => {
                self.move_mask(from, to_slot, group);
            }
            LayerAction::MoveToGroup { mask, group } => self.move_to_group(mask, group),
            LayerAction::NewGroup(index) => self.new_group(index),
            LayerAction::SelectGroup(group) => self.select_group(group),
            LayerAction::RenameGroup(group) => {
                self.select_group(group);
                self.view.focus_name = true;
            }
            LayerAction::ToggleCollapsed(group) => {
                if !self.view.collapsed.remove(&group) {
                    self.view.collapsed.insert(group);
                }
            }
            LayerAction::MoveGroup { group, to_slot } => self.move_group(group, to_slot),
            LayerAction::Ungroup(group) => self.remove_group(group),
            LayerAction::DeleteGroup(group) => {
                for index in group_members(self.masks, group).rev() {
                    remove_mask(self.masks, self.selection, index);
                }
                self.remove_group(group);
            }
        }
    }

    fn select_group(&mut self, group: usize) {
        if group < self.groups.len() {
            self.view.selected = Some(group);
            *self.selection = None;
        }
    }

    /// The group new masks join: the selected one, or the selected mask's.
    fn current_group(&self) -> Option<usize> {
        self.view.selected.or_else(|| {
            self.selection
                .and_then(|selection| self.masks.get(selection.mask)?.group)
        })
    }

    fn add(&mut self, shape: MaskShape) {
        let group = self.current_group();
        add_mask(self.masks, self.selection, shape);
        self.view.selected = None;
        if let Some(group) = group {
            let end = group_members(self.masks, group).end;
            self.move_mask(self.masks.len() - 1, end, Some(group));
        }
    }

    fn delete_mask(&mut self, index: usize) {
        let Some(group) = self.masks.get(index).map(|mask| mask.group) else {
            return;
        };
        remove_mask(self.masks, self.selection, index);
        self.drop_if_empty(group);
    }

    /// Moves a mask (see [`LayerAction::Move`]) and returns where it is now.
    fn move_mask(&mut self, from: usize, to_slot: usize, group: Option<usize>) -> Option<usize> {
        let old_group = self.masks.get(from)?.group;
        let to = move_mask(self.masks, self.selection, from, to_slot);
        self.masks[to].group = group;
        if old_group != group {
            // This may renumber `group`; the mask follows.
            self.drop_if_empty(old_group);
        }
        Some(to)
    }

    fn move_to_group(&mut self, mask: usize, group: Option<usize>) {
        let Some(current) = self.masks.get(mask).map(|mask| mask.group) else {
            return;
        };
        let Some(target) = group.or(current) else {
            return;
        };
        if current == group || target >= self.groups.len() {
            return;
        }
        let end = group_members(self.masks, target).end;
        self.move_mask(mask, end, group);
    }

    fn new_group(&mut self, mask: usize) {
        if self.groups.len() >= MAX_MASK_GROUPS || mask >= self.masks.len() {
            return;
        }
        self.groups
            .push(MaskGroup::new(format!("Group {}", self.groups.len() + 1)));
        let group = self.groups.len() - 1;
        // Out of its old group first, so that group's masks stay together.
        let slot = self.masks[mask]
            .group
            .map_or(mask, |old| group_members(self.masks, old).end);
        let Some(moved) = self.move_mask(mask, slot, Some(group)) else {
            return;
        };
        if let Some(group) = self.masks[moved].group {
            self.select_group(group);
            self.view.focus_name = true;
        }
    }

    fn move_group(&mut self, group: usize, to_slot: usize) {
        let members = group_members(self.masks, group);
        if members.is_empty() || (members.start..=members.end).contains(&to_slot) {
            return;
        }
        let rest: Vec<usize> = (0..self.masks.len())
            .filter(|index| !members.contains(index))
            .collect();
        let at = if to_slot > members.end {
            to_slot - members.len()
        } else {
            to_slot
        };
        let order: Vec<usize> = rest[..at]
            .iter()
            .copied()
            .chain(members)
            .chain(rest[at..].iter().copied())
            .collect();
        let old = std::mem::take(self.masks);
        let mut old: Vec<Option<Mask>> = old.into_iter().map(Some).collect();
        *self.masks = order
            .iter()
            .map(|index| old[*index].take().expect("a permutation"))
            .collect();
        if let Some(selection) = self.selection {
            selection.mask = order
                .iter()
                .position(|index| *index == selection.mask)
                .unwrap_or(selection.mask);
        }
    }

    fn drop_if_empty(&mut self, group: Option<usize>) {
        if let Some(group) = group
            && group_members(self.masks, group).is_empty()
        {
            self.remove_group(group);
        }
    }

    fn remove_group(&mut self, group: usize) {
        remove_group(self.masks, self.groups, group);
        self.view.forget(group);
    }
}

pub fn display_name(group: &MaskGroup, index: usize) -> String {
    if group.name.trim().is_empty() {
        format!("Group {}", index + 1)
    } else {
        group.name.clone()
    }
}

/// "3 masks", "1 mask · 60 %".
pub fn summary(group: &MaskGroup, members: usize) -> String {
    let mut text = match members {
        1 => "1 mask".to_owned(),
        count => format!("{count} masks"),
    };
    if group.amount < 100.0 {
        text += &format!(" · {:.0} %", group.amount);
    }
    text
}

/// A group's header: a fold arrow, its name and summary, and its
/// visibility switch. Returns the row's response and whether the arrow
/// was clicked.
pub fn group_row(
    ui: &mut Ui,
    group: &mut MaskGroup,
    index: usize,
    members: usize,
    selected: bool,
    collapsed: bool,
) -> (Response, bool) {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), GROUP_ROW_HEIGHT),
        Sense::click_and_drag(),
    );
    let radius = CornerRadius::same(theme::CONTROL_RADIUS);
    if selected {
        ui.painter().rect(
            rect,
            radius,
            theme::ACCENT_SOFT,
            Stroke::new(1.0, theme::ACCENT),
            StrokeKind::Inside,
        );
    } else if response.hovered() || response.dragged() {
        ui.painter().rect_filled(rect, radius, theme::CONTROL);
    } else {
        ui.painter()
            .rect_filled(rect, radius, theme::CONTROL.gamma_multiply(0.45));
    }

    let arrow_rect = Rect::from_min_size(
        pos2(rect.left() + 2.0, rect.top()),
        vec2(22.0, rect.height()),
    );
    let arrow = ui
        .interact(arrow_rect, response.id.with("fold"), Sense::click())
        .on_hover_text(if collapsed {
            "Show this group's masks"
        } else {
            "Hide this group's masks"
        });
    let arrow_color = if arrow.hovered() {
        theme::TEXT
    } else {
        theme::TEXT_SECONDARY
    };
    paint_fold_arrow(ui.painter(), arrow_rect.center(), collapsed, arrow_color);

    let toggle_size = vec2(30.0, 17.0);
    let text_left = arrow_rect.right() + 2.0;
    let text_width = rect.right() - toggle_size.x - 14.0 - text_left;
    let text_color = if group.enabled {
        theme::TEXT
    } else {
        theme::TEXT_TERTIARY
    };
    let name = single_line(
        ui.painter(),
        display_name(group, index),
        theme::body(13.0),
        text_color,
        text_width,
    );
    let details = single_line(
        ui.painter(),
        summary(group, members),
        theme::body(11.0),
        theme::TEXT_SECONDARY,
        text_width,
    );
    ui.painter().galley(
        pos2(text_left, rect.center().y - name.size().y + 1.0),
        name,
        text_color,
    );
    ui.painter().galley(
        pos2(text_left, rect.center().y + 1.0),
        details,
        theme::TEXT_SECONDARY,
    );

    // A child Ui, unlike `ui.put`, leaves the list's cursor below the row.
    let toggle_rect = Rect::from_center_size(
        pos2(rect.right() - 8.0 - toggle_size.x / 2.0, rect.center().y),
        toggle_size,
    );
    let mut toggle_ui = ui.new_child(egui::UiBuilder::new().max_rect(toggle_rect));
    widgets::toggle(&mut toggle_ui, &mut group.enabled)
        .on_hover_text("Show the effect of this group's masks");
    let tip = format!(
        "{}\n{}",
        display_name(group, index),
        summary(group, members)
    );
    (response.on_hover_text(tip), arrow.clicked())
}

/// A small triangle pointing right (collapsed) or down (open), painted so
/// it needs no font glyph.
fn paint_fold_arrow(painter: &egui::Painter, center: egui::Pos2, collapsed: bool, color: Color32) {
    let size = 4.0;
    let points = if collapsed {
        vec![
            center + vec2(-size * 0.6, -size),
            center + vec2(size * 0.9, 0.0),
            center + vec2(-size * 0.6, size),
        ]
    } else {
        vec![
            center + vec2(-size, -size * 0.6),
            center + vec2(size, -size * 0.6),
            center + vec2(0.0, size * 0.9),
        ]
    };
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

/// A group row's right-click menu.
pub fn group_menu(ui: &mut Ui, group: usize, collapsed: bool) -> Option<LayerAction> {
    if ui.button("Rename").clicked() {
        return Some(LayerAction::RenameGroup(group));
    }
    if ui
        .button(if collapsed { "Expand" } else { "Collapse" })
        .clicked()
    {
        return Some(LayerAction::ToggleCollapsed(group));
    }
    if ui.button("Ungroup").clicked() {
        return Some(LayerAction::Ungroup(group));
    }
    ui.separator();
    if ui
        .button("Delete Group")
        .on_hover_text("Delete the group and its masks")
        .clicked()
    {
        return Some(LayerAction::DeleteGroup(group));
    }
    None
}

/// The "Move to Group" part of a mask row's menu.
pub fn mask_group_menu(
    ui: &mut Ui,
    masks: &[Mask],
    groups: &[MaskGroup],
    mask: usize,
) -> Option<LayerAction> {
    let current = masks.get(mask)?.group;
    let mut action = None;
    ui.menu_button("Move to Group", |ui| {
        for (group, details) in groups.iter().enumerate() {
            let button = egui::Button::new(display_name(details, group));
            if ui.add_enabled(current != Some(group), button).clicked() {
                action = Some(LayerAction::MoveToGroup {
                    mask,
                    group: Some(group),
                });
            }
        }
        if !groups.is_empty() {
            ui.separator();
        }
        if ui
            .add_enabled(
                groups.len() < MAX_MASK_GROUPS,
                egui::Button::new("New Group"),
            )
            .clicked()
        {
            action = Some(LayerAction::NewGroup(mask));
        }
    });
    if current.is_some() && ui.button("Remove from Group").clicked() {
        action = Some(LayerAction::MoveToGroup { mask, group: None });
    }
    action
}

/// The selected group's name, amount and switch, Ungroup and Delete Group.
pub fn group_settings(
    group: &mut MaskGroup,
    index: usize,
    members: usize,
    focus_name: &mut bool,
    ui: &mut Ui,
) -> Option<LayerAction> {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Group").size(13.0).color(theme::TEXT));
        ui.label(
            RichText::new(summary(group, members))
                .size(11.0)
                .color(theme::TEXT_TERTIARY),
        );
    });
    ui.horizontal(|ui| {
        label(ui, "Name");
        let name = ui.add(
            egui::TextEdit::singleline(&mut group.name)
                .char_limit(MAX_MASK_GROUP_NAME_CHARS)
                .hint_text(display_name(&MaskGroup::default(), index))
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(focus_name) {
            name.request_focus();
        }
    });
    adjustment_slider(
        ui,
        "Amount",
        &mut group.amount,
        0.0..=100.0,
        100.0,
        &SliderStyle {
            format: |v| format!("{v:.0} %"),
            ..SliderStyle::default()
        },
    );
    ui.horizontal(|ui| {
        widgets::toggle(ui, &mut group.enabled)
            .on_hover_text("Show the effect of this group's masks");
        label(ui, "Visible");
    });
    hint(
        ui,
        "Each mask applies at its own amount times the group's, and only while the group is visible.",
    );
    ui.add_space(6.0);
    let mut action = None;
    ui.horizontal(|ui| {
        if ui
            .button("Ungroup")
            .on_hover_text("Remove the group and keep its masks")
            .clicked()
        {
            action = Some(LayerAction::Ungroup(index));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .button("Delete Group")
                .on_hover_text("Delete the group and its masks")
                .clicked()
            {
                action = Some(LayerAction::DeleteGroup(index));
            }
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonation_develop::tidy_groups;

    fn mask(name: &str, group: Option<usize>) -> Mask {
        Mask {
            group,
            ..Mask::new(name, MaskShape::default())
        }
    }

    /// A, [People: B, C], D, [Sky: E]
    fn sample() -> (Vec<Mask>, Vec<MaskGroup>) {
        (
            vec![
                mask("A", None),
                mask("B", Some(0)),
                mask("C", Some(0)),
                mask("D", None),
                mask("E", Some(1)),
            ],
            vec![MaskGroup::new("People"), MaskGroup::new("Sky")],
        )
    }

    struct State {
        masks: Vec<Mask>,
        groups: Vec<MaskGroup>,
        selection: Option<MaskSelection>,
        view: GroupView,
    }

    impl State {
        fn new() -> State {
            let (masks, groups) = sample();
            State {
                masks,
                groups,
                selection: None,
                view: GroupView::default(),
            }
        }

        fn apply(&mut self, action: LayerAction) {
            Layers {
                masks: &mut self.masks,
                groups: &mut self.groups,
                selection: &mut self.selection,
                view: &mut self.view,
            }
            .apply(action);
            let (mut masks, mut groups) = (self.masks.clone(), self.groups.clone());
            tidy_groups(&mut masks, &mut groups);
            assert_eq!(
                (&masks, &groups),
                (&self.masks, &self.groups),
                "{action:?} keeps groups together and none empty"
            );
        }

        /// "A", "People: B", …
        fn layout(&self) -> Vec<String> {
            self.masks
                .iter()
                .map(|mask| match mask.group {
                    Some(group) => format!("{}: {}", self.groups[group].name, mask.name),
                    None => mask.name.clone(),
                })
                .collect()
        }

        fn selected_name(&self) -> Option<&str> {
            self.selection
                .map(|selection| self.masks[selection.mask].name.as_str())
        }
    }

    #[test]
    fn rows_show_headers_where_groups_start_and_hide_collapsed_masks() {
        let (masks, _) = sample();
        use ListRow::{Group, Mask};
        assert_eq!(
            list_rows(&masks, &HashSet::new()),
            [
                Mask(0),
                Group(0),
                Mask(1),
                Mask(2),
                Mask(3),
                Group(1),
                Mask(4)
            ]
        );
        assert_eq!(
            list_rows(&masks, &HashSet::from([0])),
            [Mask(0), Group(0), Mask(3), Group(1), Mask(4)]
        );
    }

    /// Rows 40 points apart.
    fn laid_out(rows: Vec<ListRow>) -> Vec<(ListRow, Rect)> {
        rows.into_iter()
            .enumerate()
            .map(|(i, row)| {
                let rect = Rect::from_min_size(pos2(0.0, i as f32 * 40.0), vec2(100.0, 36.0));
                (row, rect)
            })
            .collect()
    }

    #[test]
    fn drops_join_groups_between_their_masks_or_onto_their_header() {
        let (masks, _) = sample();
        let rows = laid_out(list_rows(&masks, &HashSet::new()));
        let at = |y: f32| {
            let target = drop_target(&masks, &rows, ListRow::Mask(0), y).expect("rows");
            (target.slot, target.group)
        };
        // Rows: A 0, People 40, B 80, C 120, D 160, Sky 200, E 240.
        assert_eq!(at(58.0), (3, Some(0)), "onto People: to its end");
        assert_eq!(at(70.0), (1, Some(0)), "below the header, above B");
        assert_eq!(at(110.0), (2, Some(0)), "between B and C");
        assert_eq!(at(150.0), (3, None), "below People's last mask: out");
        assert_eq!(at(290.0), (5, None), "below everything");
        // A group dropped inside another group goes above it.
        let group = drop_target(&masks, &rows, ListRow::Group(1), 110.0).expect("rows");
        assert_eq!((group.slot, group.group), (1, None));
    }

    #[test]
    fn drop_actions_skip_drops_that_change_nothing() {
        let (masks, _) = sample();
        let target = |slot, group| DropTarget {
            slot,
            group,
            marker: DropMarker::Line {
                y: 0.0,
                indented: false,
            },
        };
        assert_eq!(target(1, Some(0)).action(&masks, ListRow::Mask(1)), None);
        assert!(target(1, None).action(&masks, ListRow::Mask(1)).is_some());
        assert_eq!(target(3, None).action(&masks, ListRow::Group(0)), None);
        assert!(target(0, None).action(&masks, ListRow::Group(0)).is_some());
    }

    #[test]
    fn dragging_masks_into_out_of_and_between_groups() {
        let mut state = State::new();
        state.selection = Some(MaskSelection::mask(0));
        state.apply(LayerAction::Move {
            from: 0,
            to_slot: 2,
            group: Some(0),
        });
        assert_eq!(
            state.layout(),
            ["People: B", "People: A", "People: C", "D", "Sky: E"]
        );
        assert_eq!(state.selected_name(), Some("A"), "the selection follows");
        // The last mask leaving Sky removes the group.
        state.apply(LayerAction::Move {
            from: 4,
            to_slot: 1,
            group: Some(0),
        });
        assert_eq!(
            state.layout(),
            ["People: B", "People: E", "People: A", "People: C", "D"]
        );
        assert_eq!(state.groups.len(), 1);
        state.apply(LayerAction::MoveToGroup {
            mask: 0,
            group: None,
        });
        assert_eq!(
            state.layout(),
            ["People: E", "People: A", "People: C", "B", "D"]
        );
        assert_eq!(state.selected_name(), Some("A"));
    }

    #[test]
    fn groups_move_as_a_whole() {
        let mut state = State::new();
        state.selection = Some(MaskSelection::mask(2));
        state.apply(LayerAction::MoveGroup {
            group: 0,
            to_slot: 5,
        });
        assert_eq!(
            state.layout(),
            ["A", "D", "Sky: E", "People: B", "People: C"]
        );
        assert_eq!(state.selected_name(), Some("C"));
        state.apply(LayerAction::MoveGroup {
            group: 0,
            to_slot: 0,
        });
        assert_eq!(
            state.layout(),
            ["People: B", "People: C", "A", "D", "Sky: E"]
        );
    }

    #[test]
    fn new_groups_take_the_mask_and_are_selected_for_naming() {
        let mut state = State::new();
        state.apply(LayerAction::NewGroup(2));
        assert_eq!(
            state.layout(),
            ["A", "People: B", "Group 3: C", "D", "Sky: E"]
        );
        assert_eq!(state.view.selected, Some(2));
        assert!(state.view.focus_name && state.selection.is_none());
        // B was People's last mask, so People goes.
        state.apply(LayerAction::NewGroup(1));
        assert_eq!(state.groups.len(), 3);
        assert_eq!(
            state.layout(),
            ["A", "Group 4: B", "Group 3: C", "D", "Sky: E"]
        );
    }

    #[test]
    fn new_masks_join_the_current_group() {
        let mut state = State::new();
        state.apply(LayerAction::Select(1));
        state.apply(LayerAction::Add(MaskShape::default()));
        assert_eq!(state.masks[3].group, Some(0), "after People's masks");
        assert_eq!(state.selection, Some(MaskSelection::mask(3)));
        state.apply(LayerAction::SelectGroup(1));
        state.apply(LayerAction::Add(MaskShape::default()));
        assert_eq!(state.masks.last().and_then(|m| m.group), Some(1));
        assert_eq!(state.view.selected, None, "the new mask is selected");
    }

    #[test]
    fn ungrouping_and_deleting_groups() {
        let mut state = State::new();
        state.view.collapsed.insert(1);
        state.selection = Some(MaskSelection::mask(4));
        state.apply(LayerAction::Ungroup(0));
        assert_eq!(state.layout(), ["A", "B", "C", "D", "Sky: E"]);
        assert_eq!(state.view.collapsed, HashSet::from([0]), "renumbered");
        state.apply(LayerAction::DeleteGroup(0));
        assert_eq!(state.layout(), ["A", "B", "C", "D"]);
        assert!(state.groups.is_empty() && state.view.collapsed.is_empty());
        assert_eq!(state.selection, None, "its mask was deleted");
    }

    #[test]
    fn deleting_a_groups_last_mask_removes_the_group() {
        let mut state = State::new();
        state.view.selected = Some(1);
        state.apply(LayerAction::Delete(4));
        assert_eq!(state.groups.len(), 1);
        assert_eq!(state.view.selected, None);
    }

    #[test]
    fn menu_moves_stay_within_a_group_or_step_over_one() {
        let (masks, _) = sample();
        assert_eq!(step_slot(&masks, 0, false), Some(3), "A below People");
        assert_eq!(step_slot(&masks, 3, true), Some(1), "D above People");
        assert_eq!(step_slot(&masks, 1, true), None, "top of People");
        assert_eq!(step_slot(&masks, 1, false), Some(3), "B below C");
        assert_eq!(step_slot(&masks, 2, false), None, "bottom of People");
        assert_eq!(step_slot(&masks, 4, false), None, "bottom of the list");
    }
}
