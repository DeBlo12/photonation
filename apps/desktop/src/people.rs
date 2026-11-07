// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The people found in the open photo, waiting to be clicked. The engine
//! looks for them (`RenderService::find_people`); a click turns one into
//! ordinary brush strokes (`Selection::to_strokes`). See docs/RETOUCH.md.

use photonation_develop::{BrushStroke, MaskSpace};
use photonation_engine::FoundPerson;
use std::path::{Path, PathBuf};

/// Most strokes an outline drawn on the photo keeps.
const OUTLINE_STROKES: usize = 64;

/// One person found, and the strokes that show them on the photo.
pub struct Person {
    pub found: FoundPerson,
    /// The selection exactly, for drawing.
    pub outline: Vec<BrushStroke>,
}

pub enum Search {
    Searching,
    Found(Vec<Person>),
    Failed(String),
}

/// The latest search, for one photo.
#[derive(Default)]
pub struct PeopleSearch {
    latest: Option<(PathBuf, Search)>,
}

impl PeopleSearch {
    /// The search for `path`, if there is one.
    pub fn of(&self, path: &Path) -> Option<&Search> {
        self.latest
            .as_ref()
            .filter(|(searched, _)| searched == path)
            .map(|(_, search)| search)
    }

    /// Marks a search for `path` as running (the caller asks the engine).
    pub fn start(&mut self, path: PathBuf) {
        self.latest = Some((path, Search::Searching));
    }

    /// The engine's answer, if it is for the photo searched last.
    pub fn arrived(
        &mut self,
        path: &Path,
        people: Result<Vec<FoundPerson>, String>,
        space: MaskSpace,
    ) {
        let Some((searched, search)) = &mut self.latest else {
            return;
        };
        if searched != path {
            return;
        }
        *search = match people {
            Ok(people) => Search::Found(
                people
                    .into_iter()
                    .map(|found| Person {
                        outline: found.selection.to_strokes(space, 0.0, OUTLINE_STROKES),
                        found,
                    })
                    .collect(),
            ),
            Err(message) => Search::Failed(message),
        };
    }

    /// Forgets a search that ended without an answer (the model went
    /// missing), so the next request searches again.
    pub fn cancel(&mut self) {
        if matches!(self.latest, Some((_, Search::Searching))) {
            self.latest = None;
        }
    }

    /// The people found in `path`, if the search is done.
    pub fn found(&self, path: &Path) -> &[Person] {
        match self.of(path) {
            Some(Search::Found(people)) => people,
            _ => &[],
        }
    }
}

/// The person under a mask position: the smallest whose selection covers
/// it, so one in front of another can be picked.
pub fn person_at(people: &[Person], position: [f64; 2]) -> Option<usize> {
    people
        .iter()
        .enumerate()
        .filter(|(_, person)| person.found.selection.contains(position))
        .min_by_key(|(_, person)| person.found.selection.area())
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonation_develop::Selection;

    fn found(cells: &[bool]) -> FoundPerson {
        FoundPerson {
            score: 0.9,
            selection: Selection {
                width: 2,
                height: 2,
                cells: cells.to_vec(),
            },
        }
    }

    #[test]
    fn the_smallest_person_under_the_pointer_is_picked() {
        let mut search = PeopleSearch::default();
        let photo = Path::new("a.cr3");
        search.start(photo.into());
        let everyone = vec![found(&[true; 4]), found(&[true, false, false, false])];
        search.arrived(photo, Ok(everyone), MaskSpace::new(3.0, 2.0));
        let people = search.found(photo);
        assert_eq!(person_at(people, [0.25, 0.25]), Some(1), "in front");
        assert_eq!(person_at(people, [0.75, 0.75]), Some(0));
        assert!(search.found(Path::new("b.cr3")).is_empty());
    }

    #[test]
    fn an_answer_for_another_photo_is_ignored() {
        let mut search = PeopleSearch::default();
        search.start("b.cr3".into());
        search.arrived(
            Path::new("a.cr3"),
            Ok(vec![found(&[true; 4])]),
            MaskSpace::new(1.0, 1.0),
        );
        assert!(matches!(
            search.of(Path::new("b.cr3")),
            Some(Search::Searching)
        ));
        search.cancel();
        assert!(search.of(Path::new("b.cr3")).is_none());
    }
}
