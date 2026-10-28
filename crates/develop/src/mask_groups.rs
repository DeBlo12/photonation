// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Mask groups: folders of masks with a shared visibility switch and
//! amount. See docs/EDIT-MODEL.md.
//!
//! Masks stay one flat list (`Settings::masks`); a grouped mask names its
//! group by index into `Settings::mask_groups`. Every group's masks sit
//! next to each other in the list, so the list reads as a tree, and a group
//! without masks is dropped: a group is shown where its masks are.

use crate::masks::{MAX_MASK_NAME_CHARS, Mask, truncate_name};
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// Most groups one photo keeps.
pub const MAX_MASK_GROUPS: usize = 16;
/// Longest group name kept, in characters: the same as a mask's.
pub const MAX_MASK_GROUP_NAME_CHARS: usize = MAX_MASK_NAME_CHARS;

/// A folder of masks. Its switch and amount apply to every mask in it: a
/// mask renders only when both it and its group are enabled, at its own
/// amount times the group's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaskGroup {
    pub name: String,
    /// Disabled groups hide their masks without changing them.
    pub enabled: bool,
    /// Scales the amount of every mask in the group, 0..100 %.
    pub amount: f32,
}

impl Default for MaskGroup {
    fn default() -> Self {
        MaskGroup::new("")
    }
}

impl MaskGroup {
    /// An enabled group at full amount.
    pub fn new(name: impl Into<String>) -> MaskGroup {
        MaskGroup {
            name: name.into(),
            enabled: true,
            amount: 100.0,
        }
    }

    /// Clamps every value into range and repairs non-finite input.
    pub fn sanitized(&self) -> MaskGroup {
        let mut group = self.clone();
        truncate_name(&mut group.name);
        group.amount = if group.amount.is_finite() {
            group.amount.clamp(0.0, 100.0)
        } else {
            100.0
        };
        group
    }
}

impl Mask {
    /// This mask's group, if it is in one of `groups`.
    pub fn group_in<'a>(&self, groups: &'a [MaskGroup]) -> Option<&'a MaskGroup> {
        groups.get(self.group?)
    }

    /// This mask as it renders: hidden when its group is, and its amount
    /// scaled by the group's. Ungrouped masks come back unchanged.
    pub fn rendered(&self, groups: &[MaskGroup]) -> Mask {
        let mut mask = self.clone();
        if let Some(group) = self.group_in(groups) {
            mask.enabled &= group.enabled;
            mask.amount *= group.amount / 100.0;
        }
        mask
    }
}

/// Every mask as it renders ([`Mask::rendered`]), in order.
pub fn rendered_masks(masks: &[Mask], groups: &[MaskGroup]) -> Vec<Mask> {
    masks.iter().map(|mask| mask.rendered(groups)).collect()
}

/// The indices of group `group`'s masks: a run, as groups keep their masks
/// together (empty if it has none).
pub fn group_members(masks: &[Mask], group: usize) -> Range<usize> {
    let Some(first) = masks.iter().position(|mask| mask.group == Some(group)) else {
        return 0..0;
    };
    let count = masks[first..]
        .iter()
        .take_while(|mask| mask.group == Some(group))
        .count();
    first..first + count
}

/// Removes group `group`, keeping its masks ungrouped where they are, and
/// renumbers the groups after it.
pub fn remove_group(masks: &mut [Mask], groups: &mut Vec<MaskGroup>, group: usize) {
    if group >= groups.len() {
        return;
    }
    groups.remove(group);
    for mask in masks {
        mask.group = match mask.group {
            Some(g) if g == group => None,
            Some(g) if g > group => Some(g - 1),
            other => other,
        };
    }
}

/// Repairs the group structure: masks naming a missing group are
/// ungrouped, each group's masks are moved together (to where its first
/// mask is, otherwise keeping the order), and groups without masks are
/// removed. Masks add up, so moving them does not change the render.
pub fn tidy_groups(masks: &mut Vec<Mask>, groups: &mut Vec<MaskGroup>) {
    for mask in masks.iter_mut() {
        mask.group = mask.group.filter(|group| *group < groups.len());
    }
    // Sort by where each mask's group starts (or the mask itself, if
    // ungrouped); the sort is stable, so the order is kept otherwise.
    let first_member: Vec<Option<usize>> = (0..groups.len())
        .map(|group| masks.iter().position(|mask| mask.group == Some(group)))
        .collect();
    let mut keyed: Vec<(usize, Mask)> = std::mem::take(masks)
        .into_iter()
        .enumerate()
        .map(|(index, mask)| {
            let key = mask
                .group
                .and_then(|group| first_member[group])
                .unwrap_or(index);
            (key, mask)
        })
        .collect();
    keyed.sort_by_key(|(key, _)| *key);
    *masks = keyed.into_iter().map(|(_, mask)| mask).collect();
    for group in (0..groups.len()).rev() {
        if first_member[group].is_none() {
            remove_group(masks, groups, group);
        }
    }
}

/// Caps and sanitizes the groups, then repairs the structure
/// ([`tidy_groups`]). Masks are sanitized on their own.
pub fn sanitize_groups(masks: &mut Vec<Mask>, groups: &mut Vec<MaskGroup>) {
    groups.truncate(MAX_MASK_GROUPS);
    *groups = groups.iter().map(MaskGroup::sanitized).collect();
    tidy_groups(masks, groups);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MaskShape;

    fn mask(name: &str, group: Option<usize>) -> Mask {
        Mask {
            group,
            ..Mask::new(name, MaskShape::default())
        }
    }

    fn layout(masks: &[Mask]) -> Vec<(&str, Option<usize>)> {
        masks
            .iter()
            .map(|mask| (mask.name.as_str(), mask.group))
            .collect()
    }

    #[test]
    fn group_visibility_and_amount_apply_to_their_masks() {
        let mut groups = vec![MaskGroup::new("People")];
        groups[0].amount = 50.0;
        let mut grouped = mask("Face", Some(0));
        grouped.amount = 60.0;
        let rendered = grouped.rendered(&groups);
        assert_eq!((rendered.enabled, rendered.amount), (true, 30.0));
        groups[0].enabled = false;
        assert!(!grouped.rendered(&groups).enabled, "the group hides it");

        let ungrouped = mask("Sky", None);
        assert_eq!(ungrouped.rendered(&groups), ungrouped, "unchanged");
        // A full-amount group leaves the amount exactly as it was.
        groups[0] = MaskGroup::new("People");
        assert_eq!(grouped.rendered(&groups), grouped);
    }

    #[test]
    fn members_are_the_groups_run() {
        let masks = [
            mask("A", None),
            mask("B", Some(1)),
            mask("C", Some(1)),
            mask("D", Some(0)),
        ];
        assert_eq!(group_members(&masks, 1), 1..3);
        assert_eq!(group_members(&masks, 0), 3..4);
        assert_eq!(group_members(&masks, 2), 0..0);
    }

    #[test]
    fn removing_a_group_keeps_its_masks_and_renumbers_the_rest() {
        let mut masks = vec![mask("A", Some(0)), mask("B", Some(1)), mask("C", Some(2))];
        let mut groups = vec![
            MaskGroup::new("0"),
            MaskGroup::new("1"),
            MaskGroup::new("2"),
        ];
        remove_group(&mut masks, &mut groups, 1);
        assert_eq!(
            layout(&masks),
            [("A", Some(0)), ("B", None), ("C", Some(1))]
        );
        assert_eq!(groups[1].name, "2");
    }

    #[test]
    fn tidying_gathers_groups_and_drops_empty_or_missing_ones() {
        let mut masks = vec![
            mask("A", Some(1)),
            mask("B", None),
            mask("C", Some(1)),
            mask("D", Some(7)),
            mask("E", Some(2)),
        ];
        let mut groups = vec![
            MaskGroup::new("Empty"),
            MaskGroup::new("Split"),
            MaskGroup::new("Last"),
        ];
        tidy_groups(&mut masks, &mut groups);
        assert_eq!(
            layout(&masks),
            [
                ("A", Some(0)),
                ("C", Some(0)),
                ("B", None),
                ("D", None),
                ("E", Some(1)),
            ]
        );
        assert_eq!(
            groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            ["Split", "Last"]
        );
        let tidy = (masks.clone(), groups.clone());
        tidy_groups(&mut masks, &mut groups);
        assert_eq!((masks, groups), tidy, "tidying twice changes nothing");
    }

    #[test]
    fn sanitize_caps_groups_and_repairs_hostile_values() {
        let mut groups = vec![
            MaskGroup {
                name: "x".repeat(10_000),
                enabled: true,
                amount: f32::NAN,
            };
            MAX_MASK_GROUPS * 2
        ];
        groups[1].amount = 400.0;
        let mut masks: Vec<Mask> = (0..groups.len())
            .map(|group| mask("M", Some(group)))
            .collect();
        sanitize_groups(&mut masks, &mut groups);
        assert_eq!(groups.len(), MAX_MASK_GROUPS);
        assert_eq!(groups[0].name.chars().count(), MAX_MASK_GROUP_NAME_CHARS);
        assert_eq!((groups[0].amount, groups[1].amount), (100.0, 100.0));
        assert!(
            masks[MAX_MASK_GROUPS..].iter().all(|m| m.group.is_none()),
            "masks of dropped groups are ungrouped"
        );
    }
}
