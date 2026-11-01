// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Finding a shot's lens in the database, after Lensfun's
//! `lfDatabase::FindLenses`: every lens scores by crop factor, focal range
//! and aperture guessed from the names, mount and a fuzzy comparison of
//! the names' words; the best name wins.

use crate::database::{Camera, Database, Lens};

/// Lower case with runs of white space as one space, trimmed: how Lensfun
/// compares camera names.
pub fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Focal range and widest aperture read from a lens name such as
/// "EF-S 18-55mm f/3.5-5.6" or "1:2.8 70-200mm"; 0 where it says nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Guessed {
    pub focal: [f64; 2],
    pub aperture: f64,
}

pub fn guessed_parameters(name: &str) -> Guessed {
    let lower = name.to_lowercase();
    let mut guessed = Guessed::default();
    let Some(mm) = lower.find("mm") else {
        return guessed;
    };
    // The focal length, "18-55" or "50", right before "mm".
    let before = &lower[..mm];
    let start = before
        .char_indices()
        .rfind(|(_, c)| !(c.is_ascii_digit() || *c == '.' || *c == '-'))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let mut focals = before[start..]
        .split('-')
        .filter_map(|f| f.parse::<f64>().ok());
    if let Some(min) = focals.next() {
        guessed.focal = [min, focals.next().unwrap_or(0.0)];
    }
    // The aperture: "f/3.5", "f3.5", "1:3.5" after the focal length, or
    // "1:2.8" before it.
    let after = lower[mm + 2..].trim_start();
    let after = ["f/", "f", "1:", "1/"]
        .iter()
        .find_map(|prefix| after.strip_prefix(prefix))
        .unwrap_or(after);
    guessed.aperture = leading_number(after)
        .or_else(|| {
            before
                .rfind("1:")
                .and_then(|i| leading_number(&before[i + 2..]))
        })
        .unwrap_or(0.0);
    guessed
}

fn leading_number(text: &str) -> Option<f64> {
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    text[..end].trim_end_matches('.').parse().ok()
}

/// A name's words, as Lensfun splits them: runs of digits (with points,
/// "50.0" as "50"), of punctuation and of letters, without single
/// punctuation marks (but "+" and "*") or a lone "f"; lower case, sorted.
fn words(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let punctuation = |c: char| c.is_ascii_punctuation();
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let first = chars[i];
        if first.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        let mut end_trim = 0;
        if first.is_ascii_digit() {
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i - start >= 2 && chars[i - 2] == '.' && chars[i - 1] == '0' {
                end_trim = 2;
            }
        } else if punctuation(first) {
            while i < chars.len() && punctuation(chars[i]) {
                i += 1;
            }
        } else {
            while i < chars.len()
                && !chars[i].is_whitespace()
                && !chars[i].is_ascii_digit()
                && !punctuation(chars[i])
            {
                i += 1;
            }
        }
        let lone = i - start == 1
            && (punctuation(first) || first.eq_ignore_ascii_case(&'f'))
            && first != '*'
            && first != '+';
        if !lone {
            words.push(
                chars[start..i - end_trim]
                    .iter()
                    .collect::<String>()
                    .to_lowercase(),
            );
        }
    }
    words.sort();
    words
}

/// Lensfun's strict fuzzy comparison: 0 unless every word of `pattern` is
/// in `name`, else 200 × the shared words over all words.
fn name_score(pattern: &[String], name: &str) -> u32 {
    let name = words(name);
    if pattern.is_empty() || name.is_empty() {
        return 0;
    }
    let mut unused = name.clone();
    for word in pattern {
        match unused.iter().position(|w| w == word) {
            Some(at) => {
                unused.remove(at);
            }
            None => return 0,
        }
    }
    (pattern.len() * 200 / (pattern.len() + name.len())) as u32
}

/// Lensfun's comparison of two numbers that may be unknown (0): `Some(true)`
/// within 1 %, `Some(false)` otherwise.
fn same(a: f64, b: f64) -> Option<bool> {
    if a == 0.0 || b == 0.0 {
        return None;
    }
    let ratio = a / b;
    Some(ratio > 0.99 && ratio < 1.01)
}

fn score(
    lens: &Lens,
    camera: &Camera,
    compatible: &[String],
    pattern: Option<(&[String], Guessed)>,
) -> u32 {
    let crop = camera.crop;
    let ratio = crop / lens.crop;
    let mut score = match ratio {
        r if r < 0.96 => return 0,
        r if r >= 1.41 => 2,
        r if r >= 1.31 => 4,
        r if r >= 1.21 => 6,
        r if r >= 1.11 => 8,
        r if r >= 1.01 => 10,
        r if r >= 1.0 => 5,
        _ => 3,
    };
    if !lens.mounts.is_empty() {
        if lens.mounts.contains(&camera.mount) {
            score += 10;
        } else if lens.mounts.iter().any(|m| compatible.contains(m)) {
            score += 9;
        } else {
            return 0;
        }
    }
    let Some((pattern, guessed)) = pattern else {
        // No lens name: only a fixed lens of the camera's own mount.
        return if lens.mounts.contains(&camera.mount) {
            score
        } else {
            0
        };
    };
    let numbers = [
        (guessed.focal[0], lens.focal[0]),
        (guessed.focal[1], lens.focal[1]),
        (guessed.aperture, lens.aperture[0]),
    ];
    for (a, b) in numbers {
        match same(a, b) {
            Some(false) => return 0,
            Some(true) => score += 10,
            None => {}
        }
    }
    let name = lens
        .models
        .iter()
        .map(|model| name_score(pattern, model))
        .max()
        .unwrap_or(0);
    if name == 0 {
        return 0;
    }
    score + (name * 4 / 10).max(1)
}

/// The entries of the best matching lens for `camera`, which may hold
/// calibrations for several sensor sizes; empty when none matches.
pub fn lenses<'a>(database: &'a Database, camera: &Camera, name: Option<&str>) -> Vec<&'a Lens> {
    let name = name.map(str::trim).filter(|name| !name.is_empty());
    let pattern = name.map(|name| (words(name), guessed_parameters(name)));
    let pattern = pattern.as_ref().map(|(w, g)| (w.as_slice(), *g));
    let compatible = database.compatible(&camera.mount);
    let scored: Vec<(&Lens, u32)> = database
        .lenses
        .iter()
        .map(|lens| (lens, score(lens, camera, compatible, pattern)))
        .filter(|(_, score)| *score > 0)
        .collect();
    let Some(&(best, _)) = scored.iter().max_by_key(|(_, score)| *score) else {
        return Vec::new();
    };
    let best_name = normalized(&best.name);
    let mut entries: Vec<&Lens> = scored
        .iter()
        .map(|(lens, _)| *lens)
        .filter(|lens| normalized(&lens.name) == best_name)
        .collect();
    // The best scoring entry first.
    entries.sort_by_key(|lens| std::cmp::Reverse(score(lens, camera, compatible, pattern)));
    if name.is_none() && entries.len() < scored.len() {
        // Several fixed lenses fit an unnamed one: don't guess.
        return Vec::new();
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_like_lensfun() {
        assert_eq!(
            words("Canon EF-S18-55mm f/3.5-5.6 III"),
            ["18", "3.5", "5.6", "55", "canon", "ef", "iii", "mm", "s"].map(String::from)
        );
        assert_eq!(words("50.0mm F1.8"), ["1.8", "50", "mm"].map(String::from));
    }

    #[test]
    fn parameters_come_from_the_name() {
        let zoom = guessed_parameters("Canon EF-S 18-55mm f/3.5-5.6 III");
        assert_eq!(
            zoom,
            Guessed {
                focal: [18.0, 55.0],
                aperture: 3.5
            }
        );
        let prime = guessed_parameters("Canon EF 50mm f/1.8 STM");
        assert_eq!(
            prime,
            Guessed {
                focal: [50.0, 0.0],
                aperture: 1.8
            }
        );
        let minolta = guessed_parameters("Minolta AF 1:2.8 70-210mm");
        assert_eq!(
            minolta,
            Guessed {
                focal: [70.0, 210.0],
                aperture: 2.8
            }
        );
        assert_eq!(guessed_parameters("fixed lens"), Guessed::default());
    }

    #[test]
    fn every_pattern_word_must_be_in_the_name() {
        let pattern = words("Canon EF-S 18-55mm f/3.5-5.6 III");
        assert!(name_score(&pattern, "Canon EF-S 18-55mm f/3.5-5.6 III") >= 100);
        assert_eq!(
            name_score(&pattern, "Canon EF-S 18-55mm f/3.5-5.6 IS STM"),
            0
        );
        // The camera's spelling without spaces finds the database's.
        let tight = words("Canon EF-S18-55mm f/3.5-5.6 III");
        assert!(name_score(&tight, "Canon EF-S 18-55mm f/3.5-5.6 III") >= 100);
    }
}
