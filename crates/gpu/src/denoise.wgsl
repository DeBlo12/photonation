// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

// Noise reduction (pipeline `denoise.rs`): the developed image in square-
// root luminance and colour differences, split by an à trous B3 wavelet
// into detail per level and a coarse rest; each level's detail shrinks by
// its thresholds and the levels add back up. The CPU smooths band by band
// with exactly the halo the levels need, so its result equals this
// whole-image smoothing with edges clamped.

struct Level {
    // x: step between taps; y: 1 for the first level (no sum to read yet).
    step: vec4<u32>,
    // x: luminance threshold, y: colour threshold, z: share of the
    // luminance detail removed that is put back.
    amounts: vec4<f32>,
}

@group(0) @binding(0) var<uniform> level: Level;
// The developed image (`opponent`), or the level's approximation.
@group(0) @binding(1) var image_in: texture_2d<f32>;
@group(0) @binding(2) var image_out: texture_storage_2d<rgba32float, write>;
// The approximation smoothed along rows.
@group(0) @binding(3) var rows_in: texture_2d<f32>;
// The shrunk detail of the levels so far.
@group(0) @binding(4) var sum_in: texture_2d<f32>;
@group(0) @binding(5) var sum_out: texture_storage_2d<rgba32float, write>;

const KERNEL = array<f32, 5>(1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0);

fn to_opponent(rgb: vec3<f32>) -> vec3<f32> {
    let q = sqrt(max(rgb, vec3<f32>(0.0)));
    return vec3<f32>((q.x + 2.0 * q.y + q.z) * 0.25, q.z - q.y, q.x - q.y);
}

fn from_opponent(o: vec3<f32>) -> vec3<f32> {
    let g = o.x - 0.25 * (o.y + o.z);
    let rgb = max(vec3<f32>(o.z + g, g, o.y + g), vec3<f32>(0.0));
    return rgb * rgb;
}

fn shrink(detail: f32, threshold: f32) -> f32 {
    if threshold <= 0.0 {
        return detail;
    }
    let energy = detail * detail;
    return detail * energy / (energy + threshold * threshold);
}

@compute @workgroup_size(16, 16)
fn opponent(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(image_in);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let rgb = textureLoad(image_in, id.xy, 0).rgb;
    textureStore(image_out, id.xy, vec4<f32>(to_opponent(rgb), 1.0));
}

@compute @workgroup_size(16, 16)
fn smooth_rows(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(image_in);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let step = i32(level.step.x);
    var sum = vec3<f32>(0.0);
    for (var k = 0; k < 5; k++) {
        let x = clamp(i32(id.x) + (k - 2) * step, 0, i32(size.x) - 1);
        sum += KERNEL[k] * textureLoad(image_in, vec2<u32>(u32(x), id.y), 0).rgb;
    }
    textureStore(image_out, id.xy, vec4<f32>(sum, 1.0));
}

// Smooths along columns into the next approximation, and adds this level's
// shrunk detail to the sum.
@compute @workgroup_size(16, 16)
fn smooth_columns(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(image_in);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let step = i32(level.step.x);
    var coarse = vec3<f32>(0.0);
    for (var k = 0; k < 5; k++) {
        let y = clamp(i32(id.y) + (k - 2) * step, 0, i32(size.y) - 1);
        coarse += KERNEL[k] * textureLoad(rows_in, vec2<u32>(id.x, u32(y)), 0).rgb;
    }
    textureStore(image_out, id.xy, vec4<f32>(coarse, 1.0));

    let detail = textureLoad(image_in, id.xy, 0).rgb - coarse;
    var sum = vec3<f32>(0.0);
    if level.step.y == 0u {
        sum = textureLoad(sum_in, id.xy, 0).rgb;
    }
    let smoothed = shrink(detail.x, level.amounts.x);
    sum.x += smoothed + level.amounts.z * (detail.x - smoothed);
    sum.y += shrink(detail.y, level.amounts.y);
    sum.z += shrink(detail.z, level.amounts.y);
    textureStore(sum_out, id.xy, vec4<f32>(sum, 1.0));
}

// The shrunk detail plus the coarse rest, back to linear RGB.
@compute @workgroup_size(16, 16)
fn combine(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(image_in);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let opponent = textureLoad(sum_in, id.xy, 0).rgb + textureLoad(image_in, id.xy, 0).rgb;
    textureStore(image_out, id.xy, vec4<f32>(from_opponent(opponent), 1.0));
}
