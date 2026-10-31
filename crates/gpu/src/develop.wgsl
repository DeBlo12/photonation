// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

// The source and develop stages for one pixel of the demosaiced source:
// normalize, white balance and demosaic (pipeline `source.rs`), then the
// develop steps with the masks' local adjustments (`develop.rs`,
// `masks.rs`, `tone.rs`, `color.rs`, `point_color.rs`) and the output
// gamut. A transcription of the CPU
// pipeline, which is the reference: keep the two in step (the crate's
// tests compare them). Parameters come from `RenderPlan` (`params.rs`).

const MODE_MHC: u32 = 0u;
const MODE_HALF_SIZE: u32 = 1u;
const MODE_NEIGHBOUR_AVERAGE: u32 = 2u;
const MODE_LINEAR: u32 = 3u;

struct Sensor {
    // The sensor region read: x, y, width, height.
    region: vec4<u32>,
    // x: demosaic mode; y, z: CFA pattern width and height.
    mode: vec4<u32>,
    black: vec4<f32>,
    gain: vec4<f32>,
    // CFA colours (0 red, 1 green, 2 blue), row-major, four per entry.
    cfa: array<vec4<u32>, 9>,
}

struct Selection {
    // Target lightness, chroma and hue (degrees); w: hue relevance.
    target_lch: vec4<f32>,
    // Reach in hue, chroma and lightness; w: share fully selected.
    reach: vec4<f32>,
    // Hue shift (degrees), chroma gain, lightness shift; w: grey chroma.
    shift: vec4<f32>,
}

struct Develop {
    camera_to_working: array<vec4<f32>, 3>,
    working_to_output: array<vec4<f32>, 3>,
    working_luminance: vec4<f32>,
    output_luminance: vec4<f32>,
    // x: haze, y: highlights, z: shadows, w: mid-grey.
    light: vec4<f32>,
    // x: Highlights/Shadows shift at ±100 (EV); y, z: tone table range
    // (EV); w: entries per channel.
    tone: vec4<f32>,
    // x: colour step on, y: mixer on, z: grading on, w: point colours.
    color_flags: vec4<u32>,
    // x: saturation, y: vibrance, z: 1 to show the overlay's selection.
    color: vec4<f32>,
    // Rows of RGB → LMS, LMS′ → Lab, Lab → LMS′ and LMS → RGB.
    oklab: array<vec4<f32>, 12>,
    band_hues: array<vec4<f32>, 2>,
    // Per mixer band: hue shift (degrees), chroma gain, lightness shift.
    bands: array<vec4<f32>, 8>,
    // Shadows, midtones, highlights, global: a, b, lightness offsets.
    grading: array<vec4<f32>, 4>,
    // x: pivot, y: transition half-width.
    grading_range: vec4<f32>,
    overlay: Selection,
    // x: masks on, y: number of masks.
    local: vec4<u32>,
    // Metric mask point of pixel p:
    // positions[0].xy + p.x · positions[0].zw + p.y · positions[1].xy.
    positions: array<vec4<f32>, 2>,
    // x: haze at Dehaze 100, y: white balance stops at 100, z: contrast
    // exponent change at 100, w: saturation restored by Dehaze.
    local_scales: vec4<f32>,
    // x: cap on summed exposure (EV), y: on summed tone amounts.
    local_caps: vec4<f32>,
}

@group(0) @binding(0) var mosaic: texture_2d<u32>;
@group(0) @binding(1) var<uniform> sensor: Sensor;
@group(0) @binding(2) var<uniform> develop: Develop;
@group(0) @binding(3) var<storage, read> tone_tables: array<f32>;
@group(0) @binding(4) var<storage, read> point_colors: array<Selection>;
@group(0) @binding(5) var developed: texture_storage_2d<rgba32float, write>;
@group(0) @binding(6) var<storage, read> masks: array<MaskEntry>;
@group(0) @binding(7) var<storage, read> shapes: array<Shape>;

// Local adjustment amounts (pipeline `LocalAmounts`).
struct Amounts {
    // Exposure (EV), temperature, tint, contrast.
    light: vec4<f32>,
    // Highlights, shadows, dehaze, saturation.
    tone: vec4<f32>,
}

// One mask (pipeline `MaskPlan`).
struct MaskEntry {
    // x: first entry in `shapes`, y: shape count, z: 1 when inverted.
    shapes: vec4<u32>,
    // At full selection, amount and group applied.
    amounts: Amounts,
}

// One shape of a mask (pipeline `ShapeWeight`).
struct Shape {
    // Linear: start (xy), ramp (zw). Radial: centre (xy), fade start (z).
    a: vec4<f32>,
    // Radial: axis_x (xy), axis_y (zw), each divided by its radius.
    b: vec4<f32>,
    // x: 0 linear, 1 radial; y: how it combines with the shapes before
    // it (0 add, 1 subtract, 2 intersect).
    kind: vec4<u32>,
}

fn smooth_step(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Rust's `rem_euclid(360.0)`.
fn wrap_degrees(x: f32) -> f32 {
    let r = x % 360.0;
    return select(r, r + 360.0, r < 0.0);
}

fn rows3(m: array<vec4<f32>, 3>, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        m[0].x * v.x + m[0].y * v.y + m[0].z * v.z,
        m[1].x * v.x + m[1].y * v.y + m[1].z * v.z,
        m[2].x * v.x + m[2].y * v.y + m[2].z * v.z,
    );
}

fn weighted_sum(weights: vec4<f32>, v: vec3<f32>) -> f32 {
    return weights.x * v.x + weights.y * v.y + weights.z * v.z;
}

// ---- Source -------------------------------------------------------------

fn cfa_color(x: u32, y: u32) -> u32 {
    let index = (y % sensor.mode.z) * sensor.mode.y + x % sensor.mode.y;
    return sensor.cfa[index / 4u][index % 4u];
}

// Clipped at 1 so every channel saturates at the same value.
fn normalize(raw: u32, color: u32) -> f32 {
    return min((f32(raw) - sensor.black[color]) * sensor.gain[color], 1.0);
}

// Reflects an index into 0..len without repeating the edge sample.
fn mirror(index: i32, len: u32) -> u32 {
    let last = i32(len) - 1;
    var reflected = index;
    if index < 0 {
        reflected = -index;
    } else if index > last {
        reflected = 2 * last - index;
    }
    return u32(clamp(reflected, 0, last));
}

// The 5×5 neighbourhood of a region pixel, mirrored at the region's edges:
// normalized samples and their CFA colours, row-major.
struct Window {
    values: array<f32, 25>,
    colors: array<u32, 25>,
}

fn window(x: u32, y: u32) -> Window {
    var w: Window;
    for (var dy = -2; dy <= 2; dy++) {
        let sensor_y = sensor.region.y + mirror(i32(y) + dy, sensor.region.w);
        for (var dx = -2; dx <= 2; dx++) {
            let sensor_x = sensor.region.x + mirror(i32(x) + dx, sensor.region.z);
            let color = cfa_color(sensor_x, sensor_y);
            let raw = textureLoad(mosaic, vec2<u32>(sensor_x, sensor_y), 0).r;
            let index = (dy + 2) * 5 + dx + 2;
            w.values[index] = normalize(raw, color);
            w.colors[index] = color;
        }
    }
    return w;
}

fn at(w: ptr<function, Window>, dy: i32, dx: i32) -> f32 {
    return (*w).values[(dy + 2) * 5 + dx + 2];
}

fn color_at(w: ptr<function, Window>, dy: i32, dx: i32) -> u32 {
    return (*w).colors[(dy + 2) * 5 + dx + 2];
}

// Malvar–He–Cutler for one Bayer site (kernels ×8, from the paper).
fn mhc(x: u32, y: u32) -> vec3<f32> {
    var w = window(x, y);
    let center = at(&w, 0, 0);
    let cross = at(&w, -1, 0) + at(&w, 1, 0) + at(&w, 0, -1) + at(&w, 0, 1);
    let diagonals = at(&w, -1, -1) + at(&w, -1, 1) + at(&w, 1, -1) + at(&w, 1, 1);
    let two_away_horizontal = at(&w, 0, -2) + at(&w, 0, 2);
    let two_away_vertical = at(&w, -2, 0) + at(&w, 2, 0);
    var rgb = vec3<f32>(0.0);
    let own = color_at(&w, 0, 0);
    if own == 1u {
        // Which chroma channel sits left/right of this green site.
        let horizontal = color_at(&w, 0, 1);
        let horizontal_neighbours = at(&w, 0, -1) + at(&w, 0, 1);
        let vertical_neighbours = at(&w, -1, 0) + at(&w, 1, 0);
        rgb[1] = center;
        rgb[horizontal] = (5.0 * center + 4.0 * horizontal_neighbours - diagonals
            - two_away_horizontal + 0.5 * two_away_vertical) * 0.125;
        rgb[2u - horizontal] = (5.0 * center + 4.0 * vertical_neighbours - diagonals
            - two_away_vertical + 0.5 * two_away_horizontal) * 0.125;
    } else {
        let axial = two_away_horizontal + two_away_vertical;
        rgb[own] = center;
        rgb[1] = (4.0 * center + 2.0 * cross - axial) * 0.125;
        rgb[2u - own] = (6.0 * center + 2.0 * diagonals - 1.5 * axial) * 0.125;
    }
    return rgb;
}

// Mean of same-coloured samples within distance 1, widening to 2 if a
// colour is absent nearby.
fn neighbour_average(x: u32, y: u32) -> vec3<f32> {
    var w = window(x, y);
    let own = color_at(&w, 0, 0);
    var rgb = vec3<f32>(0.0);
    rgb[own] = at(&w, 0, 0);
    for (var channel = 0u; channel < 3u; channel++) {
        if channel == own {
            continue;
        }
        for (var reach = 1; reach <= 2; reach++) {
            var sum = 0.0;
            var count = 0;
            for (var dy = -reach; dy <= reach; dy++) {
                for (var dx = -reach; dx <= reach; dx++) {
                    if color_at(&w, dy, dx) == channel {
                        sum += at(&w, dy, dx);
                        count++;
                    }
                }
            }
            if count > 0 {
                rgb[channel] = sum / f32(count);
                break;
            }
        }
    }
    return rgb;
}

// One RGB pixel per 2×2 quad.
fn half_size(x: u32, y: u32) -> vec3<f32> {
    let sensor_x = sensor.region.x + x * 2u;
    let sensor_y = sensor.region.y + y * 2u;
    var rgb = vec3<f32>(0.0);
    for (var dy = 0u; dy < 2u; dy++) {
        for (var dx = 0u; dx < 2u; dx++) {
            let color = cfa_color(sensor_x + dx, sensor_y + dy);
            let raw = textureLoad(mosaic, vec2<u32>(sensor_x + dx, sensor_y + dy), 0).r;
            rgb[color] += normalize(raw, color);
        }
    }
    rgb[1] *= 0.5; // two green sites per quad
    return rgb;
}

fn linear(x: u32, y: u32) -> vec3<f32> {
    let raw = textureLoad(mosaic, vec2<u32>(sensor.region.x + x, sensor.region.y + y), 0);
    return vec3<f32>(normalize(raw.r, 0u), normalize(raw.g, 1u), normalize(raw.b, 2u));
}

// ---- Masks --------------------------------------------------------------

fn shape_weight(shape: Shape, point: vec2<f32>) -> f32 {
    if shape.kind.x == 0u {
        let t = (point.x - shape.a.x) * shape.a.z + (point.y - shape.a.y) * shape.a.w;
        return 1.0 - smooth_step(0.0, 1.0, t);
    }
    let offset = point - shape.a.xy;
    let x = offset.x * shape.b.x + offset.y * shape.b.y;
    let y = offset.x * shape.b.z + offset.y * shape.b.w;
    let distance = sqrt(x * x + y * y);
    let fade_start = shape.a.z;
    if fade_start >= 1.0 {
        // No feather: a hard edge.
        return select(0.0, 1.0, distance <= 1.0);
    }
    return 1.0 - smooth_step(fade_start, 1.0, distance);
}

// A mask's selection, 0..1: its shapes combined in order, maybe inverted.
fn mask_weight(mask: MaskEntry, point: vec2<f32>) -> f32 {
    var selected = 0.0;
    for (var index = 0u; index < mask.shapes.y; index++) {
        let shape = shapes[mask.shapes.x + index];
        let weight = shape_weight(shape, point);
        if index == 0u {
            selected = weight;
        } else if shape.kind.y == 0u {
            selected = max(selected, weight);
        } else if shape.kind.y == 1u {
            selected = min(selected, 1.0 - weight);
        } else {
            selected = min(selected, weight);
        }
    }
    if mask.shapes.z != 0u {
        selected = 1.0 - selected;
    }
    return selected;
}

// The masks' adjustments at a pixel, each weighted by its selection.
fn local_amounts(pixel: vec2<u32>) -> Amounts {
    let x = f32(pixel.x);
    let y = f32(pixel.y);
    let origin = develop.positions[0].xy;
    let step_x = develop.positions[0].zw;
    let step_y = develop.positions[1].xy;
    let point = vec2<f32>(
        origin.x + x * step_x.x + y * step_y.x,
        origin.y + x * step_x.y + y * step_y.y,
    );
    var sum = Amounts(vec4<f32>(0.0), vec4<f32>(0.0));
    for (var index = 0u; index < develop.local.y; index++) {
        let mask = masks[index];
        let weight = mask_weight(mask, point);
        if weight > 0.0 {
            sum.light += mask.amounts.light * weight;
            sum.tone += mask.amounts.tone * weight;
        }
    }
    return sum;
}

// ---- Develop ------------------------------------------------------------

fn dehaze(rgb: vec3<f32>, haze: f32) -> vec3<f32> {
    if haze > 0.0 {
        let v = max(rgb, vec3<f32>(0.0));
        return v * v / (v + haze) * (1.0 + haze);
    }
    let veil = -haze;
    return rgb * (1.0 - veil) + veil;
}

fn highlights_shadows(rgb: vec3<f32>, highlights: f32, shadows: f32) -> vec3<f32> {
    let luminance = max(weighted_sum(develop.working_luminance, rgb), 1e-6);
    let stops = log2(luminance / develop.light.w);
    let shadow_weight = 1.0 - smooth_step(-4.0, 0.0, stops);
    let highlight_weight = smooth_step(-0.5, 2.5, stops);
    let shift = develop.tone.x * (shadows * shadow_weight + highlights * highlight_weight);
    return rgb * exp2(shift);
}

// Local Temperature/Tint as channel gains (warmer: more red, less blue;
// magenta: less green) keeping a neutral's luminance, with local exposure.
fn local_light(rgb: vec3<f32>, light: vec4<f32>) -> vec3<f32> {
    let exposure = exp2(clamp(light.x, -develop.local_caps.x, develop.local_caps.x));
    if light.y == 0.0 && light.z == 0.0 {
        return rgb * exposure;
    }
    let warm = light.y * develop.local_scales.y / 2.0;
    let magenta = light.z * develop.local_scales.y;
    let balance = vec3<f32>(exp2(warm), exp2(-magenta), exp2(-warm));
    let scale = exposure / weighted_sum(develop.working_luminance, balance);
    return rgb * balance * scale;
}

// Local Contrast: steepens or flattens luminance around mid-grey in log space.
fn local_contrast(rgb: vec3<f32>, contrast: f32) -> vec3<f32> {
    let luminance = max(weighted_sum(develop.working_luminance, rgb), 1e-6);
    let exponent = clamp(1.0 + contrast * develop.local_scales.z, 0.25, 2.0);
    let grey = develop.light.w;
    return rgb * (grey * pow(luminance / grey, exponent) / luminance);
}

fn tone(channel: u32, scene: f32) -> f32 {
    let size = u32(develop.tone.w);
    let base = channel * size;
    let lut_min = exp2(develop.tone.y);
    if scene <= lut_min {
        return max(scene, 0.0) * (tone_tables[base] / lut_min);
    }
    let position = (log2(scene) - develop.tone.y)
        * (f32(size - 1u) / (develop.tone.z - develop.tone.y));
    if position >= f32(size - 1u) {
        return tone_tables[base + size - 1u];
    }
    let index = u32(position);
    let fraction = position - f32(index);
    let low = tone_tables[base + index];
    return low + (tone_tables[base + index + 1u] - low) * fraction;
}

fn cube_root(v: f32) -> f32 {
    if v == 0.0 {
        return 0.0;
    }
    return sign(v) * pow(abs(v), 1.0 / 3.0);
}

fn oklab_rows(first: u32) -> array<vec4<f32>, 3> {
    return array<vec4<f32>, 3>(
        develop.oklab[first],
        develop.oklab[first + 1u],
        develop.oklab[first + 2u],
    );
}

fn to_oklab(rgb: vec3<f32>) -> vec3<f32> {
    let lms = rows3(oklab_rows(0u), rgb);
    let root = vec3<f32>(cube_root(lms.x), cube_root(lms.y), cube_root(lms.z));
    return rows3(oklab_rows(3u), root);
}

fn from_oklab(lab: vec3<f32>) -> vec3<f32> {
    let root = rows3(oklab_rows(6u), lab);
    return rows3(oklab_rows(9u), root * root * root);
}

fn hue_of(lab: vec3<f32>) -> f32 {
    // Rust's atan2(0, 0) is 0; GPUs may differ.
    if lab.y == 0.0 && lab.z == 0.0 {
        return 0.0;
    }
    return wrap_degrees(degrees(atan2(lab.z, lab.y)));
}

fn band_hue(index: u32) -> f32 {
    return develop.band_hues[index / 4u][index % 4u];
}

fn apply_mixer(lab_in: vec3<f32>) -> vec3<f32> {
    var lab = lab_in;
    let chroma = sqrt(lab.y * lab.y + lab.z * lab.z);
    if chroma < 1e-5 {
        return lab;
    }
    let hue = hue_of(lab);
    var upper = 0u;
    for (var band = 0u; band < 8u; band++) {
        if band_hue(band) > hue {
            upper = band;
            break;
        }
    }
    let lower = (upper + 7u) % 8u;
    let span = wrap_degrees(band_hue(upper) - band_hue(lower));
    let t = smooth_step(0.0, 1.0, wrap_degrees(hue - band_hue(lower)) / span);
    let low = develop.bands[lower];
    let adjustment = low + (develop.bands[upper] - low) * t;
    let new_hue = radians(hue + adjustment.x);
    let new_chroma = chroma * max(adjustment.y, 0.0);
    // Lightness changes only make sense for coloured pixels.
    lab.x += adjustment.z * smooth_step(0.0, 0.1, chroma);
    lab.y = new_chroma * cos(new_hue);
    lab.z = new_chroma * sin(new_hue);
    return lab;
}

fn apply_grading(lab_in: vec3<f32>) -> vec3<f32> {
    var lab = lab_in;
    let pivot = develop.grading_range.x;
    let transition = develop.grading_range.y;
    let toward_highlights = smooth_step(pivot - transition, pivot + transition, lab.x);
    let shadow_weight = (1.0 - toward_highlights) * (1.0 - toward_highlights);
    let highlight_weight = toward_highlights * toward_highlights;
    let midtone_weight = 1.0 - shadow_weight - highlight_weight;
    let weights = array<f32, 4>(shadow_weight, midtone_weight, highlight_weight, 1.0);
    for (var wheel = 0u; wheel < 4u; wheel++) {
        let offset = develop.grading[wheel];
        lab.x += offset.z * weights[wheel];
        lab.y += offset.x * weights[wheel];
        lab.z += offset.y * weights[wheel];
    }
    return lab;
}

fn falloff(distance: f32, reach: f32, solid_share: f32) -> f32 {
    return 1.0 - smooth_step(reach * solid_share, reach, distance);
}

// How strongly a colour (lightness, chroma, hue) is selected, 0..1.
fn selection_weight(selection: Selection, lch: vec3<f32>) -> f32 {
    let goal = selection.target_lch;
    let solid = selection.reach.w;
    let lightness = falloff(abs(lch.x - goal.x), selection.reach.z, solid);
    let chroma = falloff(abs(lch.y - goal.y), selection.reach.y, solid);
    if lightness == 0.0 || chroma == 0.0 {
        return 0.0;
    }
    let difference = wrap_degrees(lch.z - goal.z);
    let hue_distance = min(difference, 360.0 - difference);
    // A grey pixel's hue is noise; grey targets ignore hue.
    let pixel_hue_relevance = smooth_step(0.0, selection.shift.w, lch.y);
    let hue = goal.w * pixel_hue_relevance * falloff(hue_distance, selection.reach.x, solid)
        + (1.0 - goal.w);
    return lightness * chroma * hue;
}

fn apply_point_colors(lab: vec3<f32>) -> vec3<f32> {
    let original = vec3<f32>(lab.x, sqrt(lab.y * lab.y + lab.z * lab.z), hue_of(lab));
    var lch = original;
    for (var index = 0u; index < develop.color_flags.w; index++) {
        let selection = point_colors[index];
        let weight = selection_weight(selection, original);
        if weight > 0.0 {
            lch.z += selection.shift.x * weight;
            lch.y *= max(1.0 + selection.shift.y * weight, 0.0);
            lch.x += selection.shift.z * weight;
        }
    }
    if develop.color.z != 0.0 {
        lch.y *= selection_weight(develop.overlay, original);
    }
    let angle = radians(lch.z);
    return vec3<f32>(lch.x, lch.y * cos(angle), lch.y * sin(angle));
}

fn apply_color(rgb: vec3<f32>) -> vec3<f32> {
    var lab = to_oklab(max(rgb, vec3<f32>(0.0)));
    if develop.color_flags.y != 0u {
        lab = apply_mixer(lab);
    }
    if develop.color_flags.z != 0u {
        lab = apply_grading(lab);
    }
    let chroma = sqrt(lab.y * lab.y + lab.z * lab.z);
    let gain = (1.0 + develop.color.x)
        * (1.0 + develop.color.y * (1.0 - smooth_step(0.0, 0.2, chroma)));
    lab.y *= max(gain, 0.0);
    lab.z *= max(gain, 0.0);
    if develop.color_flags.w != 0u || develop.color.z != 0.0 {
        lab = apply_point_colors(lab);
    }
    return from_oklab(lab);
}

// Local Saturation: scales Oklab chroma.
fn saturate(rgb: vec3<f32>, amount: f32) -> vec3<f32> {
    var lab = to_oklab(max(rgb, vec3<f32>(0.0)));
    let gain = max(1.0 + amount, 0.0);
    lab.y *= gain;
    lab.z *= gain;
    return from_oklab(lab);
}

fn pull_toward(out: vec3<f32>, luminance: f32, t: f32) -> vec3<f32> {
    return luminance + (out - luminance) * t;
}

// Out-of-gamut colours move toward their luminance instead of clipping
// channels independently (which would shift hue).
fn to_output_gamut(rgb: vec3<f32>) -> vec3<f32> {
    var out = rows3(develop.working_to_output, rgb);
    let luminance = clamp(weighted_sum(develop.output_luminance, out), 0.0, 1.0);
    let low = min(min(out.x, out.y), out.z);
    if low < 0.0 {
        out = pull_toward(out, luminance, select(0.0, luminance / (luminance - low), luminance > 0.0));
    }
    let high = max(max(out.x, out.y), out.z);
    if high > 1.0 {
        out = pull_toward(
            out,
            luminance,
            select(0.0, (1.0 - luminance) / (high - luminance), high > luminance),
        );
    }
    return clamp(out, vec3<f32>(0.0), vec3<f32>(1.0));
}

// The develop steps for one pixel; with masks, their summed adjustments
// join the global ones (`working_pixel::<true>`).
fn develop_pixel(camera_rgb: vec3<f32>, pixel: vec2<u32>) -> vec3<f32> {
    var rgb = rows3(develop.camera_to_working, camera_rgb);
    var haze = develop.light.x;
    var highlights = develop.light.y;
    var shadows = develop.light.z;
    let masked = develop.local.x != 0u;
    var local = Amounts(vec4<f32>(0.0), vec4<f32>(0.0));
    if masked {
        local = local_amounts(pixel);
        if local.light.x != 0.0 || local.light.y != 0.0 || local.light.z != 0.0 {
            rgb = local_light(rgb, local.light);
        }
        let max_tone = develop.local_caps.y;
        let max_haze = max_tone * develop.local_scales.x;
        haze = clamp(haze + local.tone.z * develop.local_scales.x, -max_haze, max_haze);
        highlights = clamp(highlights + local.tone.x, -max_tone, max_tone);
        shadows = clamp(shadows + local.tone.y, -max_tone, max_tone);
    }
    if haze != 0.0 {
        rgb = dehaze(rgb, haze);
    }
    if highlights != 0.0 || shadows != 0.0 {
        rgb = highlights_shadows(rgb, highlights, shadows);
    }
    if masked && local.light.w != 0.0 {
        rgb = local_contrast(rgb, local.light.w);
    }
    rgb = vec3<f32>(tone(0u, rgb.x), tone(1u, rgb.y), tone(2u, rgb.z));
    if develop.color_flags.x != 0u {
        rgb = apply_color(rgb);
    }
    if masked {
        // Like the global slider, local Dehaze restores washed-out colour.
        let saturation = local.tone.w + develop.local_scales.w * local.tone.z;
        if saturation != 0.0 {
            rgb = saturate(rgb, saturation);
        }
    }
    return to_output_gamut(rgb);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(developed);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    var camera: vec3<f32>;
    switch sensor.mode.x {
        case MODE_MHC: {
            camera = mhc(id.x, id.y);
        }
        case MODE_HALF_SIZE: {
            camera = half_size(id.x, id.y);
        }
        case MODE_NEIGHBOUR_AVERAGE: {
            camera = neighbour_average(id.x, id.y);
        }
        default: {
            camera = linear(id.x, id.y);
        }
    }
    let developed_rgb = develop_pixel(max(camera, vec3<f32>(0.0)), id.xy);
    textureStore(developed, id.xy, vec4<f32>(developed_rgb, 1.0));
}
