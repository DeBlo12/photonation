// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Content-aware fill: multi-scale PatchMatch inpainting.
//!
//! ```text
//! build a pyramid, halving until every hole pixel is within a patch
//!   radius of known pixels
//!   (but no smaller than 64 px)
//! coarsest level: fill the hole from its rim inwards (onion peel), and
//!   weigh that smooth guess less than known pixels when matching (it
//!   would otherwise attract flat areas)
//! for each level, coarse to fine:
//!   start from the coarser fill (upsampled) and nearest-neighbour field
//!   repeat a few times (expectation–maximization):
//!     PatchMatch: for every patch touching the hole, find a similar
//!                 patch entirely outside it (propagation + random search)
//!     vote:       each hole pixel becomes the weighted mean of what the
//!                 matched patches covering it say it should be
//! ```
//!
//!
//! **Refining a guess** ([`ContentAwareFill::refine`], [`Refined`]): given
//! a fill from elsewhere (a neural network working at low resolution), the
//! pyramid is only three levels deep and its coarsest level starts from
//! that fill at full weight, so the guess supplies the structure and
//! PatchMatch replaces its soft pixels with real texture from the photo.
//!
//! References: Barnes et al., "PatchMatch" (SIGGRAPH 2009); Wexler et al.,
//! "Space-Time Completion of Video" (PAMI 2007).

use crate::{Canvas, InpaintError, Inpainter};

/// How much the coarsest level's guessed hole pixels count in patch
/// distances, relative to known pixels.
const COARSE_GUESS_WEIGHT: f32 = 0.2;
/// The pyramid stops before a level's shorter side drops below this, so
/// the coarsest level still has plenty of source patches.
const MIN_LEVEL_SIDE: usize = 64;
/// Pyramid levels when refining a guess: full, half and quarter size, so
/// the guess's structure is kept and only finer detail is resynthesized.
const REFINE_LEVELS: usize = 3;

/// Multi-scale PatchMatch fill. See the module docs.
#[derive(Clone, Copy, Debug)]
pub struct ContentAwareFill {
    /// Patches are `(2 · radius + 1)²` pixels.
    pub patch_radius: usize,
    /// Expectation–maximization rounds at the coarsest level; finer levels
    /// start from a good guess and use half as many.
    pub coarse_rounds: usize,
}

impl Default for ContentAwareFill {
    fn default() -> Self {
        ContentAwareFill {
            patch_radius: 3,
            coarse_rounds: 8,
        }
    }
}

impl Inpainter for ContentAwareFill {
    fn name(&self) -> &str {
        "Content-aware"
    }

    fn inpaint(&self, canvas: &Canvas, seed: u32) -> Result<Vec<f32>, InpaintError> {
        self.fill(canvas, seed, None)
    }
}

impl ContentAwareFill {
    /// Resynthesizes the hole of `canvas` starting from `guess` (a filled
    /// version of the canvas, e.g. a neural network's): the guess sets the
    /// structure, PatchMatch the texture. See the module docs.
    pub fn refine(
        &self,
        canvas: &Canvas,
        guess: &[f32],
        seed: u32,
    ) -> Result<Vec<f32>, InpaintError> {
        if guess.len() != canvas.rgb.len() {
            return Err(InpaintError::InvalidCanvas);
        }
        self.fill(canvas, seed, Some(guess))
    }

    fn fill(
        &self,
        canvas: &Canvas,
        seed: u32,
        guess: Option<&[f32]>,
    ) -> Result<Vec<f32>, InpaintError> {
        if !canvas.is_valid() {
            return Err(InpaintError::InvalidCanvas);
        }
        if canvas.hole_pixels() == 0 {
            return Ok(canvas.rgb.clone());
        }
        let radius = self.patch_radius.max(1);
        let max_levels = if guess.is_some() {
            REFINE_LEVELS
        } else {
            usize::MAX
        };
        let pyramid = build_pyramid(canvas, radius, max_levels);
        let mut random = Random::new(seed);
        let coarsest = pyramid.len() - 1;

        let mut image = pyramid[coarsest].rgb.clone();
        match guess {
            Some(guess) => {
                // The guess at the coarsest level's size, in the hole only.
                let mut scaled = (guess.to_vec(), canvas.width, canvas.height);
                for _ in 0..coarsest {
                    scaled = halved_image(&scaled.0, scaled.1, scaled.2);
                }
                let level = &pyramid[coarsest];
                for pixel in (0..level.width * level.height).filter(|p| level.hole[*p]) {
                    image[pixel * 3..pixel * 3 + 3]
                        .copy_from_slice(&scaled.0[pixel * 3..pixel * 3 + 3]);
                }
            }
            None => onion_peel_fill(&pyramid[coarsest], &mut image),
        }
        let mut field: Option<NearestField> = None;
        for depth in (0..pyramid.len()).rev() {
            let level = &pyramid[depth];
            if depth != coarsest {
                image = upsampled_guess(level, &pyramid[depth + 1], &image);
            }
            if level.sources.is_empty() {
                // Nothing to copy from (the hole covers the canvas): keep
                // the smooth fill.
                field = None;
                continue;
            }
            let mut current = match field.take() {
                Some(coarser) => {
                    NearestField::upscaled(&coarser, &pyramid[depth + 1], level, &mut random)
                }
                None => NearestField::random(level, &mut random),
            };
            let rounds = if depth == coarsest {
                self.coarse_rounds
            } else {
                (self.coarse_rounds / 2).max(2)
            };
            // The coarsest level starts from a smooth guess, which would
            // match flat areas best and stay flat; there, the guessed pixels
            // count for less than the known ones around the hole. (Not
            // zero: deep in a large hole the guess is all there is.)
            // A refined guess is trusted everywhere.
            let guess_weight = if depth == coarsest && guess.is_none() {
                COARSE_GUESS_WEIGHT
            } else {
                1.0
            };
            for round in 0..rounds {
                current.improve(level, &image, radius, round, guess_weight, &mut random);
                let last_round = round + 1 == rounds;
                vote(level, &current, &mut image, radius, !last_round);
            }
            field = Some(current);
        }

        // Only hole pixels change; the rest is returned exactly.
        let mut filled = canvas.rgb.clone();
        for index in 0..canvas.width * canvas.height {
            if canvas.is_hole(index) {
                filled[index * 3..index * 3 + 3].copy_from_slice(&image[index * 3..index * 3 + 3]);
            }
        }
        Ok(filled)
    }
}

/// An inpainter whose fill is then refined by content-aware fill at the
/// canvas's full resolution: structure from `inner`, texture from the
/// photo.
pub struct Refined<I> {
    pub inner: I,
    pub detail: ContentAwareFill,
}

impl<I: Inpainter> Inpainter for Refined<I> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn inpaint(&self, canvas: &Canvas, seed: u32) -> Result<Vec<f32>, InpaintError> {
        let guess = self.inner.inpaint(canvas, seed)?;
        self.detail.refine(canvas, &guess, seed)
    }
}

/// An interleaved RGB image at half size (rounded up), each pixel the
/// mean of its children: the same grid as [`Level::halved`].
fn halved_image(rgb: &[f32], width: usize, height: usize) -> (Vec<f32>, usize, usize) {
    let (half_width, half_height) = (width.div_ceil(2), height.div_ceil(2));
    let mut half = vec![0.0; half_width * half_height * 3];
    for y in 0..half_height {
        for x in 0..half_width {
            let children: Vec<usize> = ((2 * y)..(2 * y + 2).min(height))
                .flat_map(|child_y| {
                    ((2 * x)..(2 * x + 2).min(width)).map(move |child_x| child_y * width + child_x)
                })
                .collect();
            for channel in 0..3 {
                let sum: f32 = children.iter().map(|child| rgb[child * 3 + channel]).sum();
                half[(y * half_width + x) * 3 + channel] = sum / children.len() as f32;
            }
        }
    }
    (half, half_width, half_height)
}

/// One pyramid level: the image with its hole, and which patches may serve
/// as sources (entirely known) or need a match (touch the hole).
struct Level {
    width: usize,
    height: usize,
    rgb: Vec<f32>,
    hole: Vec<bool>,
    /// Patch centred here lies inside the image and outside the hole.
    is_source: Vec<bool>,
    /// Every source centre, for random picks.
    sources: Vec<u32>,
    /// Patch centred here overlaps the hole (raster order).
    targets: Vec<u32>,
    is_target: Vec<bool>,
}

impl Level {
    fn new(width: usize, height: usize, rgb: Vec<f32>, hole: Vec<bool>, radius: usize) -> Level {
        let hole_count = SummedArea::new(width, height, &hole);
        let mut is_source = vec![false; width * height];
        let mut is_target = vec![false; width * height];
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let inside =
                    x >= radius && y >= radius && x + radius < width && y + radius < height;
                let covered = hole_count.window(x, y, radius);
                is_source[index] = inside && covered == 0;
                is_target[index] = covered > 0;
            }
        }
        let indices = |flags: &[bool]| {
            flags
                .iter()
                .enumerate()
                .filter(|(_, flag)| **flag)
                .map(|(index, _)| index as u32)
                .collect::<Vec<u32>>()
        };
        Level {
            sources: indices(&is_source),
            targets: indices(&is_target),
            width,
            height,
            rgb,
            hole,
            is_source,
            is_target,
        }
    }

    /// How far the hole reaches from known pixels: the largest
    /// chessboard distance from a hole pixel to the nearest known one
    /// (two-pass distance transform). A thin stroke is shallow however
    /// long it is.
    fn hole_depth(&self) -> usize {
        let (width, height) = (self.width, self.height);
        let unreachable = width + height;
        let mut distance: Vec<usize> = self
            .hole
            .iter()
            .map(|hole| if *hole { unreachable } else { 0 })
            .collect();
        let neighbours_before = [(-1isize, -1isize), (0, -1), (1, -1), (-1, 0)];
        let passes: [(&[(isize, isize)], bool); 2] =
            [(&neighbours_before, true), (&neighbours_before, false)];
        for (offsets, forward) in passes {
            let sign = if forward { 1 } else { -1 };
            let indices: Box<dyn Iterator<Item = usize>> = if forward {
                Box::new(0..width * height)
            } else {
                Box::new((0..width * height).rev())
            };
            for index in indices {
                if distance[index] == 0 {
                    continue;
                }
                let (x, y) = ((index % width) as isize, (index / width) as isize);
                for (dx, dy) in offsets {
                    let (neighbour_x, neighbour_y) = (x + dx * sign, y + dy * sign);
                    if (0..width as isize).contains(&neighbour_x)
                        && (0..height as isize).contains(&neighbour_y)
                    {
                        let neighbour = neighbour_y as usize * width + neighbour_x as usize;
                        distance[index] = distance[index].min(distance[neighbour] + 1);
                    }
                }
            }
        }
        distance.into_iter().max().unwrap_or(0)
    }

    /// Half the size: known pixels average their known children, and a
    /// pixel is a hole if any child is.
    fn halved(&self, radius: usize) -> Level {
        let width = self.width.div_ceil(2);
        let height = self.height.div_ceil(2);
        let mut rgb = vec![0.0; width * height * 3];
        let mut hole = vec![false; width * height];
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0.0f32; 3];
                let mut known = 0;
                for child_y in (2 * y)..(2 * y + 2).min(self.height) {
                    for child_x in (2 * x)..(2 * x + 2).min(self.width) {
                        let child = child_y * self.width + child_x;
                        if self.hole[child] {
                            hole[y * width + x] = true;
                        } else {
                            for (total, value) in sum.iter_mut().zip(&self.rgb[child * 3..]) {
                                *total += value;
                            }
                            known += 1;
                        }
                    }
                }
                if known > 0 {
                    let pixel = &mut rgb[(y * width + x) * 3..(y * width + x) * 3 + 3];
                    for (value, total) in pixel.iter_mut().zip(sum) {
                        *value = total / known as f32;
                    }
                }
            }
        }
        Level::new(width, height, rgb, hole, radius)
    }
}

/// Levels from full size (index 0) to coarsest.
fn build_pyramid(canvas: &Canvas, radius: usize, max_levels: usize) -> Vec<Level> {
    let hole = canvas.hole.iter().map(|weight| *weight > 0.0).collect();
    let mut pyramid = vec![Level::new(
        canvas.width,
        canvas.height,
        canvas.rgb.clone(),
        hole,
        radius,
    )];
    let patch = 2 * radius + 1;
    loop {
        let finest_so_far = pyramid.last().expect("at least one level");
        // Halve until every patch touching the hole also covers known
        // pixels, which the coarsest level matches on.
        let small_enough = finest_so_far.hole_depth() <= radius;
        let too_small_to_halve =
            finest_so_far.width.min(finest_so_far.height) / 2 < MIN_LEVEL_SIDE.max(4 * patch);
        if small_enough || too_small_to_halve || pyramid.len() >= max_levels {
            break;
        }
        let next = finest_so_far.halved(radius);
        if next.sources.is_empty() {
            break;
        }
        pyramid.push(next);
    }
    pyramid
}

/// Fills hole pixels from the rim inwards with the mean of their known
/// neighbours: a smooth first guess for the coarsest level.
fn onion_peel_fill(level: &Level, image: &mut [f32]) {
    let (width, height) = (level.width, level.height);
    let mut known: Vec<bool> = level.hole.iter().map(|hole| !hole).collect();
    loop {
        let mut ring = Vec::new();
        for y in 0..height {
            for x in 0..width {
                if known[y * width + x] {
                    continue;
                }
                let mut sum = [0.0f32; 3];
                let mut count = 0;
                for neighbour_y in y.saturating_sub(1)..(y + 2).min(height) {
                    for neighbour_x in x.saturating_sub(1)..(x + 2).min(width) {
                        let neighbour = neighbour_y * width + neighbour_x;
                        if known[neighbour] {
                            for (total, value) in sum.iter_mut().zip(&image[neighbour * 3..]) {
                                *total += value;
                            }
                            count += 1;
                        }
                    }
                }
                if count > 0 {
                    ring.push((y * width + x, sum.map(|total| total / count as f32)));
                }
            }
        }
        if ring.is_empty() {
            return;
        }
        for (index, color) in ring {
            image[index * 3..index * 3 + 3].copy_from_slice(&color);
            known[index] = true;
        }
    }
}

/// The finer level's known pixels, with its hole filled from the coarser
/// level's result (nearest parent).
fn upsampled_guess(level: &Level, coarser: &Level, coarse_image: &[f32]) -> Vec<f32> {
    let mut image = level.rgb.clone();
    for y in 0..level.height {
        for x in 0..level.width {
            let index = y * level.width + x;
            if level.hole[index] {
                let parent = (y / 2).min(coarser.height - 1) * coarser.width
                    + (x / 2).min(coarser.width - 1);
                image[index * 3..index * 3 + 3]
                    .copy_from_slice(&coarse_image[parent * 3..parent * 3 + 3]);
            }
        }
    }
    image
}

/// For every target patch centre, the source centre that matches it best
/// so far, and the squared distance between them.
struct NearestField {
    source: Vec<u32>,
    cost: Vec<f32>,
}

impl NearestField {
    fn random(level: &Level, random: &mut Random) -> NearestField {
        let mut source = vec![0u32; level.width * level.height];
        for &target in &level.targets {
            source[target as usize] = level.sources[random.below(level.sources.len())];
        }
        NearestField {
            source,
            cost: vec![f32::INFINITY; level.width * level.height],
        }
    }

    /// The coarser field mapped to this level: each target inherits its
    /// parent's match, scaled up, where that lands on a valid source.
    fn upscaled(
        coarser_field: &NearestField,
        coarser: &Level,
        level: &Level,
        random: &mut Random,
    ) -> NearestField {
        let mut field = NearestField::random(level, random);
        for &target in &level.targets {
            let (x, y) = (target as usize % level.width, target as usize / level.width);
            let parent =
                (y / 2).min(coarser.height - 1) * coarser.width + (x / 2).min(coarser.width - 1);
            if !coarser.is_target[parent] {
                continue;
            }
            let parent_source = coarser_field.source[parent] as usize;
            let source_x = (parent_source % coarser.width) * 2 + x % 2;
            let source_y = (parent_source / coarser.width) * 2 + y % 2;
            if source_x < level.width && source_y < level.height {
                let candidate = source_y * level.width + source_x;
                if level.is_source[candidate] {
                    field.source[target as usize] = candidate as u32;
                }
            }
        }
        field
    }

    /// One PatchMatch round: recompute costs against the current fill, then
    /// propagate good matches from neighbours (alternating scan direction)
    /// and try random candidates at shrinking distances.
    fn improve(
        &mut self,
        level: &Level,
        image: &[f32],
        radius: usize,
        round: usize,
        guess_weight: f32,
        random: &mut Random,
    ) {
        let width = level.width as isize;
        for &target in &level.targets {
            let target = target as usize;
            self.cost[target] = patch_distance(
                level,
                image,
                target,
                self.source[target] as usize,
                radius,
                guess_weight,
                f32::INFINITY,
            );
        }
        let forward = round.is_multiple_of(2);
        let step: isize = if forward { 1 } else { -1 };
        let search_start = level.width.max(level.height) as isize;
        let order: Box<dyn Iterator<Item = &u32>> = if forward {
            Box::new(level.targets.iter())
        } else {
            Box::new(level.targets.iter().rev())
        };
        for &target in order {
            let target = target as usize;
            let (x, y) = (
                (target % level.width) as isize,
                (target / level.width) as isize,
            );
            let try_candidate = |field: &mut NearestField, candidate: usize| {
                if candidate != field.source[target] as usize && level.is_source[candidate] {
                    let cost = patch_distance(
                        level,
                        image,
                        target,
                        candidate,
                        radius,
                        guess_weight,
                        field.cost[target],
                    );
                    if cost < field.cost[target] {
                        field.cost[target] = cost;
                        field.source[target] = candidate as u32;
                    }
                }
            };
            // Propagation: the neighbour we came from, shifted by one.
            for (neighbour_x, neighbour_y) in [(x - step, y), (x, y - step)] {
                if neighbour_x < 0
                    || neighbour_y < 0
                    || neighbour_x >= width
                    || neighbour_y >= level.height as isize
                {
                    continue;
                }
                let neighbour = (neighbour_y * width + neighbour_x) as usize;
                if !level.is_target[neighbour] {
                    continue;
                }
                let neighbour_source = self.source[neighbour] as isize;
                let shifted_x = neighbour_source % width + (x - neighbour_x);
                let shifted_y = neighbour_source / width + (y - neighbour_y);
                if (0..width).contains(&shifted_x)
                    && (0..level.height as isize).contains(&shifted_y)
                {
                    try_candidate(self, (shifted_y * width + shifted_x) as usize);
                }
            }
            // Random search around the current best, halving the window.
            let mut search = search_start;
            while search >= 1 {
                let best = self.source[target] as isize;
                let candidate_x = (best % width + random.within(search))
                    .clamp(radius as isize, width - 1 - radius as isize);
                let candidate_y = (best / width + random.within(search))
                    .clamp(radius as isize, level.height as isize - 1 - radius as isize);
                try_candidate(self, (candidate_y * width + candidate_x) as usize);
                search /= 2;
            }
        }
    }
}

/// Sum of squared RGB differences between the patches centred at `target`
/// and `source`, over the target pixels inside the image; hole pixels count
/// `guess_weight` times. Stops early once above `limit`.
fn patch_distance(
    level: &Level,
    image: &[f32],
    target: usize,
    source: usize,
    radius: usize,
    guess_weight: f32,
    limit: f32,
) -> f32 {
    let width = level.width as isize;
    let radius = radius as isize;
    let (target_x, target_y) = (
        (target % level.width) as isize,
        (target / level.width) as isize,
    );
    let (source_x, source_y) = (
        (source % level.width) as isize,
        (source / level.width) as isize,
    );
    let mut total = 0.0;
    for dy in -radius..=radius {
        let row = target_y + dy;
        if row < 0 || row >= level.height as isize {
            continue;
        }
        for dx in -radius..=radius {
            let column = target_x + dx;
            if column < 0 || column >= width {
                continue;
            }
            let pixel = (row * width + column) as usize;
            let weight = if level.hole[pixel] { guess_weight } else { 1.0 };
            let a = pixel * 3;
            // Sources lie at least `radius` from every edge.
            let b = (((source_y + dy) * width + source_x + dx) * 3) as usize;
            for channel in 0..3 {
                let difference = image[a + channel] - image[b + channel];
                total += weight * difference * difference;
            }
        }
        if total > limit {
            return total;
        }
    }
    total
}

/// Replaces each hole pixel with what its covering patches' matches put
/// there: their weighted mean (`average`), or the colour from the single
/// best-matching covering patch. Averaging converges smoothly but blurs
/// stochastic texture such as grass; each level's last round therefore
/// takes the best patch, which keeps the grain.
///
/// Weighted mean: Closer matches weigh more
/// (Gaussian in the patch distance, scaled by the lower-quartile distance).
fn vote(level: &Level, field: &NearestField, image: &mut [f32], radius: usize, average: bool) {
    let mut costs: Vec<f32> = level
        .targets
        .iter()
        .map(|&target| field.cost[target as usize])
        .filter(|cost| cost.is_finite())
        .collect();
    // Scale by the lower quartile: good matches dominate, so the vote
    // keeps texture instead of averaging it away.
    let quartile = if costs.is_empty() {
        1.0
    } else {
        let rank = costs.len() / 4;
        *costs.select_nth_unstable_by(rank, |a, b| a.total_cmp(b)).1
    };
    let spread = (2.0 * quartile).max(1e-6);

    let width = level.width as isize;
    let radius = radius as isize;
    let mut accumulated = vec![0.0f32; level.width * level.height * 4];
    // Best patch: the lowest cost seen per pixel (in `accumulated[3]` as
    // its negation, so "larger is better" like the weights).
    for &target in &level.targets {
        let target = target as usize;
        let source = field.source[target] as isize;
        let weight = (-field.cost[target] / spread).exp().max(1e-8);
        let (target_x, target_y) = (
            (target % level.width) as isize,
            (target / level.width) as isize,
        );
        let (source_x, source_y) = (source % width, source / width);
        for dy in -radius..=radius {
            let row = target_y + dy;
            if row < 0 || row >= level.height as isize {
                continue;
            }
            for dx in -radius..=radius {
                let column = target_x + dx;
                if column < 0 || column >= width {
                    continue;
                }
                let pixel = (row * width + column) as usize;
                if !level.hole[pixel] {
                    continue;
                }
                let from = (((source_y + dy) * width + source_x + dx) * 3) as usize;
                let sums = &mut accumulated[pixel * 4..pixel * 4 + 4];
                if average {
                    for channel in 0..3 {
                        sums[channel] += weight * image[from + channel];
                    }
                    sums[3] += weight;
                } else if sums[3] == 0.0 || -field.cost[target] > sums[3] - 1.0 {
                    sums[..3].copy_from_slice(&image[from..from + 3]);
                    // Offset by one so a zero cost still marks the pixel.
                    sums[3] = 1.0 - field.cost[target];
                }
            }
        }
    }
    for (pixel, sums) in accumulated.chunks_exact(4).enumerate() {
        if !level.hole[pixel] || sums[3] == 0.0 {
            continue;
        }
        for channel in 0..3 {
            image[pixel * 3 + channel] = if average {
                sums[channel] / sums[3]
            } else {
                sums[channel]
            };
        }
    }
}

/// Counts of set flags over rectangles in O(1) (a summed-area table).
struct SummedArea {
    width: usize,
    height: usize,
    /// `(width + 1) × (height + 1)`, with a zero first row and column.
    sums: Vec<u32>,
}

impl SummedArea {
    fn new(width: usize, height: usize, flags: &[bool]) -> SummedArea {
        let stride = width + 1;
        let mut sums = vec![0u32; stride * (height + 1)];
        for y in 0..height {
            let mut row_sum = 0;
            for x in 0..width {
                row_sum += u32::from(flags[y * width + x]);
                sums[(y + 1) * stride + x + 1] = sums[y * stride + x + 1] + row_sum;
            }
        }
        SummedArea {
            width,
            height,
            sums,
        }
    }

    /// Set flags in the square of `radius` around `(x, y)`, clipped to the
    /// image.
    fn window(&self, x: usize, y: usize, radius: usize) -> u32 {
        let stride = self.width + 1;
        let (left, top) = (x.saturating_sub(radius), y.saturating_sub(radius));
        let right = (x + radius + 1).min(self.width);
        let bottom = (y + radius + 1).min(self.height);
        self.sums[bottom * stride + right] + self.sums[top * stride + left]
            - self.sums[top * stride + right]
            - self.sums[bottom * stride + left]
    }
}

/// xorshift64*: small, fast and reproducible across platforms.
struct Random(u64);

impl Random {
    fn new(seed: u32) -> Random {
        Random(0x9E37_79B9_7F4A_7C15 ^ u64::from(seed).wrapping_mul(0xD1B5_4A32_D192_ED03) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..len` (`len > 0`).
    fn below(&mut self, len: usize) -> usize {
        (self.next() % len as u64) as usize
    }

    /// Uniform in `-reach..=reach`.
    fn within(&mut self, reach: isize) -> isize {
        self.below(2 * reach as usize + 1) as isize - reach
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vertical stripes, period 6, with a square hole in the middle.
    fn striped_canvas(size: usize, hole: (usize, usize)) -> Canvas {
        let mut rgb = Vec::with_capacity(size * size * 3);
        let mut mask = vec![0.0; size * size];
        for y in 0..size {
            for x in 0..size {
                let value = if (x / 3).is_multiple_of(2) { 0.2 } else { 0.8 };
                rgb.extend_from_slice(&[value, value * 0.5, 1.0 - value]);
                if (hole.0..hole.1).contains(&x) && (hole.0..hole.1).contains(&y) {
                    mask[y * size + x] = 1.0;
                    // Something to remove: a bright object.
                    rgb[(y * size + x) * 3..(y * size + x) * 3 + 3]
                        .copy_from_slice(&[1.0, 1.0, 0.0]);
                }
            }
        }
        Canvas {
            width: size,
            height: size,
            rgb,
            hole: mask,
        }
    }

    fn expected_stripe(x: usize) -> f32 {
        if (x / 3).is_multiple_of(2) { 0.2 } else { 0.8 }
    }

    #[test]
    fn fills_a_hole_by_continuing_the_texture() {
        let canvas = striped_canvas(72, (28, 44));
        let filled = ContentAwareFill::default()
            .inpaint(&canvas, 7)
            .expect("fill");
        let mut error = 0.0;
        let mut count = 0;
        for y in 28..44 {
            for x in 28..44 {
                let red = filled[(y * 72 + x) * 3];
                error += (red - expected_stripe(x)).abs();
                count += 1;
                assert!(
                    filled[(y * 72 + x) * 3 + 2] < 0.95 || expected_stripe(x) < 0.1 + 0.2,
                    "no trace of the removed object"
                );
            }
        }
        let mean_error = error / count as f32;
        assert!(mean_error < 0.15, "mean error {mean_error}");
    }

    #[test]
    fn pixels_outside_the_hole_are_untouched_and_seed_is_reproducible() {
        let canvas = striped_canvas(48, (20, 28));
        let fill = ContentAwareFill::default();
        let first = fill.inpaint(&canvas, 1).expect("fill");
        let again = fill.inpaint(&canvas, 1).expect("fill");
        assert_eq!(first, again, "same seed, same fill");
        for index in 0..48 * 48 {
            if !canvas.is_hole(index) {
                assert_eq!(
                    first[index * 3..index * 3 + 3],
                    canvas.rgb[index * 3..index * 3 + 3]
                );
            }
        }
    }

    #[test]
    fn a_hole_covering_everything_still_returns_finite_pixels() {
        let canvas = Canvas {
            width: 16,
            height: 16,
            rgb: vec![0.3; 16 * 16 * 3],
            hole: vec![1.0; 16 * 16],
        };
        let filled = ContentAwareFill::default()
            .inpaint(&canvas, 0)
            .expect("fill");
        assert!(filled.iter().all(|v| v.is_finite()));
        let invalid = Canvas {
            rgb: Vec::new(),
            ..canvas
        };
        assert!(matches!(
            ContentAwareFill::default().inpaint(&invalid, 0),
            Err(InpaintError::InvalidCanvas)
        ));
    }

    #[test]
    fn refining_keeps_the_guess_structure_and_adds_texture() {
        let canvas = striped_canvas(72, (28, 44));
        // A blurry guess with the right layout: the stripes' mean colour.
        let mut guess = canvas.rgb.clone();
        for index in (0..72 * 72).filter(|i| canvas.is_hole(*i)) {
            guess[index * 3..index * 3 + 3].copy_from_slice(&[0.5, 0.25, 0.5]);
        }
        let refined = ContentAwareFill::default()
            .refine(&canvas, &guess, 3)
            .expect("refine");
        let mut error = 0.0;
        for y in 28..44 {
            for x in 28..44 {
                error += (refined[(y * 72 + x) * 3] - expected_stripe(x)).abs();
            }
        }
        let mean_error = error / 256.0;
        assert!(
            mean_error < 0.15,
            "texture restored, mean error {mean_error}"
        );
        assert!(matches!(
            ContentAwareFill::default().refine(&canvas, &guess[..3], 0),
            Err(InpaintError::InvalidCanvas)
        ));
    }

    #[test]
    fn summed_area_counts_windows_clipped_to_the_image() {
        let flags: Vec<bool> = (0..25).map(|i| i % 2 == 0).collect();
        let table = SummedArea::new(5, 5, &flags);
        assert_eq!(table.window(2, 2, 10), 13);
        assert_eq!(table.window(0, 0, 0), 1);
        assert_eq!(table.window(1, 0, 0), 0);
    }
}
