// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! How far a photo leans: the angle that levels its horizon or its
//! verticals, for the crop tool's Auto straighten.
//!
//! The strongest edges of a small luminance copy vote, as in a Hough
//! transform, for the lines they could lie on at each candidate tilt: a
//! point on a horizontal-ish edge votes for `−x·sin a + y·cos a`, one on a
//! vertical-ish edge for `x·cos a + y·sin a`. At the photo's true tilt the
//! points of each straight line fall into one bin, so the sum of the
//! squared bin counts peaks there.

/// Largest tilt Auto looks for, in degrees either way.
const MAX_TILT: f64 = 20.0;
/// Candidate tilts per degree.
const STEPS_PER_DEGREE: f64 = 10.0;
/// How far (degrees) an edge's own direction may lie from a candidate tilt
/// for it to vote there; gradient directions are only this precise.
const VOTE_SPREAD: f64 = 2.0;
/// Long side, in pixels, of the luminance copy the search works on.
const WORKING_SIZE: usize = 640;
/// Share of the pixels, the strongest edges, that vote.
const EDGE_SHARE: f64 = 0.1;
/// Weakest gradient (Sobel, on 0–255 luminance) that counts as an edge.
const MIN_GRADIENT: f32 = 40.0;
/// How many times the median tilt's score the best one needs for the photo
/// to have lines at all. Set on real photos: portraits and branches reach
/// about 10, a village of roofs 12, railings and window frames 25 and more.
const CONFIDENCE: f64 = 12.0;
/// Fewest voters at the best tilt, as a share of the working pixels.
const MIN_VOTERS: f64 = 0.002;

/// An edge pixel and its direction (degrees clockwise from the x axis,
/// −90..90).
struct Edge {
    x: f64,
    y: f64,
    direction: f64,
}

/// The angle (degrees, clockwise, as `Settings::crop_angle`) that turns the
/// crop frame along the photo's dominant horizontal or vertical lines, so
/// they come out level; `None` when it has no clear ones. `rgb` is 8-bit
/// interleaved RGB, `width` × `height`.
pub fn level_angle(rgb: &[u8], width: usize, height: usize) -> Option<f64> {
    let (luminance, [width, height]) = shrunk_luminance(rgb, width, height);
    if width < 8 || height < 8 {
        return None;
    }
    let edges = strong_edges(&luminance, width, height);
    let steps = (2.0 * MAX_TILT * STEPS_PER_DEGREE) as usize + 1;
    let tilt = |step: usize| step as f64 / STEPS_PER_DEGREE - MAX_TILT;
    let offset = width + height;
    let mut bins = vec![0u32; 2 * offset + 1];
    // Per candidate tilt: the sum of squared bin counts, and the voters.
    let votes: Vec<(f64, usize)> = (0..steps)
        .map(|step| {
            let (sin, cos) = tilt(step).to_radians().sin_cos();
            let mut score = 0.0;
            let mut voters = 0;
            // Horizontal-ish lines, then vertical-ish ones turned by 90°.
            for (turn, project) in [(0.0, (-sin, cos)), (90.0, (cos, sin))] {
                bins.fill(0);
                for edge in &edges {
                    let off_tilt = wrap(edge.direction - turn - tilt(step));
                    if off_tilt.abs() > VOTE_SPREAD {
                        continue;
                    }
                    let rho = edge.x * project.0 + edge.y * project.1;
                    bins[(rho.round() as isize + offset as isize) as usize] += 1;
                    voters += 1;
                }
                score += bins.iter().map(|&c| (c as f64) * (c as f64)).sum::<f64>();
            }
            (score, voters)
        })
        .collect();

    let (best, &(best_score, best_voters)) = votes
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.0.total_cmp(&b.1.0))?;
    // A peak at the end of the range is a slope beyond it, not a tilt.
    if best == 0 || best + 1 == steps {
        return None;
    }
    if (best_voters as f64) < MIN_VOTERS * (width * height) as f64 {
        return None;
    }
    let mut scores: Vec<f64> = votes.iter().map(|(score, _)| *score).collect();
    scores.sort_by(f64::total_cmp);
    if best_score < CONFIDENCE * scores[steps / 2] {
        return None;
    }
    // Between candidates: the top of a parabola through the best and its
    // neighbours.
    let (left, right) = (votes[best - 1].0, votes[best + 1].0);
    let curvature = left - 2.0 * best_score + right;
    let refined = if curvature < 0.0 {
        0.5 * (left - right) / curvature / STEPS_PER_DEGREE
    } else {
        0.0
    };
    Some(tilt(best) + refined)
}

/// `degrees` wrapped into −90..90 (a line's direction has no sign).
fn wrap(degrees: f64) -> f64 {
    (degrees + 90.0).rem_euclid(180.0) - 90.0
}

/// Luminance (0–255) box-averaged down to at most `WORKING_SIZE` on the
/// long side, with its size.
fn shrunk_luminance(rgb: &[u8], width: usize, height: usize) -> (Vec<f32>, [usize; 2]) {
    let factor = width.max(height).div_ceil(WORKING_SIZE).max(1);
    let (small_width, small_height) = (width / factor, height / factor);
    let mut luminance = vec![0.0f32; small_width * small_height];
    for y in 0..small_height * factor {
        let row = &rgb[y * width * 3..][..small_width * factor * 3];
        let out = &mut luminance[(y / factor) * small_width..][..small_width];
        for (x, pixel) in row.chunks_exact(3).enumerate() {
            out[x / factor] +=
                0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32;
        }
    }
    let area = (factor * factor) as f32;
    luminance.iter_mut().for_each(|value| *value /= area);
    (luminance, [small_width, small_height])
}

/// The pixels with the strongest Sobel gradients, and the direction of the
/// edge through each (across the gradient).
fn strong_edges(luminance: &[f32], width: usize, height: usize) -> Vec<Edge> {
    let at = |x: usize, y: usize| luminance[y * width + x];
    let mut gradients = Vec::with_capacity((width - 2) * (height - 2));
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let gx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1))
                - (at(x - 1, y - 1) + 2.0 * at(x - 1, y) + at(x - 1, y + 1));
            let gy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1))
                - (at(x - 1, y - 1) + 2.0 * at(x, y - 1) + at(x + 1, y - 1));
            gradients.push((x, y, gx, gy, gx.hypot(gy)));
        }
    }
    let mut magnitudes: Vec<f32> = gradients.iter().map(|g| g.4).collect();
    let rank = ((1.0 - EDGE_SHARE) * magnitudes.len() as f64) as usize;
    let rank = rank.min(magnitudes.len() - 1);
    let threshold = *magnitudes.select_nth_unstable_by(rank, f32::total_cmp).1;
    let threshold = threshold.max(MIN_GRADIENT);
    gradients
        .into_iter()
        .filter(|g| g.4 >= threshold)
        .map(|(x, y, gx, gy, _)| Edge {
            x: x as f64,
            y: y as f64,
            direction: wrap((gy as f64).atan2(gx as f64).to_degrees() + 90.0),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: [usize; 2] = [900, 600];

    /// An RGB image whose grey level is `shade(x, y)`.
    fn image(shade: impl Fn(f64, f64) -> f64) -> Vec<u8> {
        let mut rgb = Vec::with_capacity(SIZE[0] * SIZE[1] * 3);
        for y in 0..SIZE[1] {
            for x in 0..SIZE[0] {
                let value = shade(x as f64, y as f64).clamp(0.0, 255.0) as u8;
                rgb.extend([value; 3]);
            }
        }
        rgb
    }

    /// Small deterministic noise, −amount..amount.
    fn noise(x: f64, y: f64, amount: f64) -> f64 {
        let hash = ((x * 12.9898 + y * 78.233).sin() * 43758.545).fract();
        (hash * 2.0 - 1.0) * amount
    }

    fn level(rgb: &[u8]) -> Option<f64> {
        level_angle(rgb, SIZE[0], SIZE[1])
    }

    #[test]
    fn a_tilted_horizon_is_levelled() {
        for tilt in [-6.5, -1.2, 0.0, 3.0, 11.0] {
            let slope = f64::to_radians(tilt).tan();
            // Sky above, darker ground below, the line through the middle.
            let rgb = image(|x, y| {
                let horizon = 300.0 + (x - 450.0) * slope;
                let base = if y < horizon { 200.0 } else { 70.0 };
                base + noise(x, y, 12.0)
            });
            let angle = level(&rgb).expect("a horizon");
            assert!((angle - tilt).abs() < 0.2, "{tilt}: {angle}");
        }
    }

    #[test]
    fn leaning_verticals_are_straightened() {
        let tilt = 4.0f64;
        // Stripes whose edges run along (−sin, cos) of `tilt`: the y axis
        // of a frame turned by it.
        let shift = tilt.to_radians().tan();
        let rgb = image(|x, y| {
            let u = x + (y - 300.0) * shift;
            let stripe = (u / 90.0).floor() as i64 % 2 == 0;
            (if stripe { 180.0 } else { 90.0 }) + noise(x, y, 10.0)
        });
        let angle = level(&rgb).expect("verticals");
        assert!((angle - tilt).abs() < 0.2, "{angle}");
    }

    #[test]
    fn noise_has_no_level() {
        let rgb = image(|x, y| 128.0 + noise(x, y, 100.0));
        assert_eq!(level(&rgb), None);
        let flat = image(|_, _| 128.0);
        assert_eq!(level(&flat), None);
    }

    #[test]
    fn directions_wrap_to_a_half_turn() {
        assert_eq!(wrap(100.0), -80.0);
        assert_eq!(wrap(-95.0), 85.0);
        assert_eq!(wrap(30.0), 30.0);
    }
}
