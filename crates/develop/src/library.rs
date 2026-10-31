// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! What a photo is worth to its owner, apart from how it looks: a star
//! rating, a pick or reject flag and a colour label. Stored in the sidecar
//! beside the settings but not part of them, so rating a photo is not an
//! edit: it has no process version, no undo step, and leaves the photo
//! "unedited". Documented in docs/EDIT-MODEL.md.

use serde::{Deserialize, Serialize};

/// Highest star rating.
pub const MAX_RATING: u8 = 5;

/// Whether a photo is a keeper, a reject, or not decided yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Flag {
    #[default]
    None,
    Pick,
    Reject,
}

/// Lightroom's five colour labels, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorLabel {
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl ColorLabel {
    pub const ALL: [ColorLabel; 5] = [
        ColorLabel::Red,
        ColorLabel::Yellow,
        ColorLabel::Green,
        ColorLabel::Blue,
        ColorLabel::Purple,
    ];

    /// The name shown to the user and written as `xmp:Label`, as Lightroom
    /// spells it.
    pub fn name(self) -> &'static str {
        match self {
            ColorLabel::Red => "Red",
            ColorLabel::Yellow => "Yellow",
            ColorLabel::Green => "Green",
            ColorLabel::Blue => "Blue",
            ColorLabel::Purple => "Purple",
        }
    }

    /// The label an `xmp:Label` value names, ignoring case.
    pub fn from_name(name: &str) -> Option<ColorLabel> {
        ColorLabel::ALL
            .into_iter()
            .find(|label| label.name().eq_ignore_ascii_case(name.trim()))
    }
}

/// A photo's rating, flag and label. Each part is left out of the sidecar
/// while unset, so unrated photos' sidecars look as they did before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryInfo {
    /// Stars, 0 (unrated) to [`MAX_RATING`].
    #[serde(skip_serializing_if = "is_zero")]
    pub rating: u8,
    #[serde(skip_serializing_if = "is_unflagged")]
    pub flag: Flag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<ColorLabel>,
}

fn is_zero(rating: &u8) -> bool {
    *rating == 0
}

fn is_unflagged(flag: &Flag) -> bool {
    *flag == Flag::None
}

impl LibraryInfo {
    /// Nothing set: no reason to keep a sidecar for it.
    pub fn is_empty(&self) -> bool {
        *self == LibraryInfo::default()
    }

    /// Ratings above [`MAX_RATING`] (a hand-edited or corrupt sidecar)
    /// become the maximum.
    pub fn sanitized(self) -> LibraryInfo {
        LibraryInfo {
            rating: self.rating.min(MAX_RATING),
            ..self
        }
    }

    /// `xmp:Rating` as Lightroom writes it: -1 for a reject, otherwise the
    /// stars. Lightroom keeps picks in its catalog only, so a pick is
    /// written as its stars.
    pub fn xmp_rating(&self) -> i8 {
        if self.flag == Flag::Reject {
            -1
        } else {
            self.rating as i8
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_parts_are_left_out_of_the_json() {
        let text = serde_json::to_string(&LibraryInfo::default()).expect("json");
        assert_eq!(text, "{}");
        let info = LibraryInfo {
            rating: 4,
            flag: Flag::Pick,
            label: Some(ColorLabel::Green),
        };
        let text = serde_json::to_string(&info).expect("json");
        assert_eq!(text, r#"{"rating":4,"flag":"pick","label":"green"}"#);
        assert_eq!(
            serde_json::from_str::<LibraryInfo>(&text).expect("parse"),
            info
        );
    }

    #[test]
    fn ratings_are_capped() {
        let info: LibraryInfo = serde_json::from_str(r#"{"rating":9}"#).expect("parse");
        assert_eq!(info.sanitized().rating, MAX_RATING);
    }

    #[test]
    fn labels_are_named_as_lightroom_writes_them() {
        assert_eq!(ColorLabel::from_name("purple"), Some(ColorLabel::Purple));
        assert_eq!(ColorLabel::from_name(" Red "), Some(ColorLabel::Red));
        assert_eq!(ColorLabel::from_name("Orange"), None);
    }

    #[test]
    fn rejects_are_rated_minus_one_in_xmp() {
        let reject = LibraryInfo {
            rating: 3,
            flag: Flag::Reject,
            label: None,
        };
        assert_eq!(reject.xmp_rating(), -1);
        let pick = LibraryInfo {
            flag: Flag::Pick,
            ..reject
        };
        assert_eq!(pick.xmp_rating(), 3);
    }
}
