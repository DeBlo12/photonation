// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

use photonation_develop::Settings;

/// Undo/redo for develop settings.
///
/// Edits are committed when an interaction finishes (e.g. a slider is
/// released), so one drag is one undo step however many frames it spans.
#[derive(Debug, Default)]
pub struct History {
    committed: Settings,
    undo: Vec<Settings>,
    redo: Vec<Settings>,
}

const MAX_STEPS: usize = 200;

impl History {
    pub fn new(initial: Settings) -> History {
        History {
            committed: initial,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Records `current` as a step if it differs from the last commit.
    /// Returns whether a step was recorded.
    pub fn commit(&mut self, current: &Settings) -> bool {
        if *current == self.committed {
            return false;
        }
        self.undo
            .push(std::mem::replace(&mut self.committed, current.clone()));
        if self.undo.len() > MAX_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
        true
    }

    pub fn undo(&mut self) -> Option<Settings> {
        let previous = self.undo.pop()?;
        self.redo
            .push(std::mem::replace(&mut self.committed, previous.clone()));
        Some(previous)
    }

    pub fn redo(&mut self) -> Option<Settings> {
        let next = self.redo.pop()?;
        self.undo
            .push(std::mem::replace(&mut self.committed, next.clone()));
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exposure(v: f32) -> Settings {
        Settings {
            exposure: v,
            ..Settings::default()
        }
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut h = History::new(exposure(0.0));
        assert!(!h.commit(&exposure(0.0)));
        assert!(h.commit(&exposure(1.0)));
        assert!(h.commit(&exposure(2.0)));
        assert_eq!(h.undo(), Some(exposure(1.0)));
        assert_eq!(h.undo(), Some(exposure(0.0)));
        assert_eq!(h.undo(), None);
        assert_eq!(h.redo(), Some(exposure(1.0)));
        assert!(h.commit(&exposure(5.0)));
        assert!(!h.can_redo());
    }
}
