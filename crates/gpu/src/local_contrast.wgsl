// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

// Texture and Clarity (pipeline `local_contrast.rs`): lightness = √luma,
// blurred by box passes along rows and columns (≈ Gaussian), and the
// detail between the two added back at a large radius (Clarity,
// midtone-weighted) and a small one (Texture). The CPU blurs band by band
// with exactly the halo the passes need, so its result equals this
// whole-image blur with edges clamped.

// Box passes load a workgroup's span plus the radius on each side.
const GROUP: u32 = 256u;
// Keep in step with MAX_BOX_RADIUS in lib.rs.
const MAX_RADIUS: u32 = 192u;

struct Contrast {
    // x: Clarity, y: Texture (0 skips that blur).
    amounts: vec4<f32>,
    // xyz: luma weights.
    luma: vec4<f32>,
}

struct Box {
    // x: radius in pixels.
    radius: vec4<u32>,
}

@group(0) @binding(0) var<uniform> contrast: Contrast;
@group(0) @binding(1) var<uniform> box: Box;
@group(0) @binding(2) var rgb_in: texture_2d<f32>;
@group(0) @binding(3) var plane_in: texture_2d<f32>;
@group(0) @binding(4) var plane_out: texture_storage_2d<r32float, write>;
@group(0) @binding(5) var lightness_plane: texture_2d<f32>;
@group(0) @binding(6) var clarity_base: texture_2d<f32>;
@group(0) @binding(7) var texture_base: texture_2d<f32>;
@group(0) @binding(8) var rgb_out: texture_storage_2d<rgba32float, write>;

var<workgroup> span: array<f32, 640>; // GROUP + 2 · MAX_RADIUS

// √luma, a cheap perceptual lightness.
@compute @workgroup_size(16, 16)
fn lightness(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(rgb_in);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let p = textureLoad(rgb_in, id.xy, 0).rgb;
    let luma = p.x * contrast.luma.x + p.y * contrast.luma.y + p.z * contrast.luma.z;
    textureStore(plane_out, id.xy, vec4<f32>(sqrt(max(luma, 0.0)), 0.0, 0.0, 1.0));
}

// One box pass along a line of `len` samples: `along` is the position on
// the line, `line` which line, `rows` whether lines are rows.
fn box_pass(along: u32, local: u32, line: u32, len: u32, rows: bool) {
    let radius = i32(box.radius.x);
    let first = i32(along - local) - radius;
    let count = GROUP + 2u * box.radius.x;
    // A trip count every invocation shares keeps the barrier in uniform
    // control flow.
    for (var round = 0u; round < (count + GROUP - 1u) / GROUP; round++) {
        let i = local + round * GROUP;
        if i >= count {
            break;
        }
        let position = u32(clamp(first + i32(i), 0, i32(len) - 1));
        var at = vec2<u32>(position, line);
        if !rows {
            at = vec2<u32>(line, position);
        }
        span[i] = textureLoad(plane_in, at, 0).r;
    }
    workgroupBarrier();
    if along >= len {
        return;
    }
    var sum = 0.0;
    for (var k = 0u; k <= 2u * box.radius.x; k++) {
        sum += span[local + k];
    }
    var at = vec2<u32>(along, line);
    if !rows {
        at = vec2<u32>(line, along);
    }
    textureStore(plane_out, at, vec4<f32>(sum / f32(2u * box.radius.x + 1u), 0.0, 0.0, 1.0));
}

@compute @workgroup_size(256, 1)
fn box_rows(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    box_pass(id.x, local.x, id.y, textureDimensions(plane_in).x, true);
}

@compute @workgroup_size(1, 256)
fn box_columns(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    box_pass(id.y, local.y, id.x, textureDimensions(plane_in).y, false);
}

// Adds the detail back, scaling RGB by the lightness ratio so hue stays.
@compute @workgroup_size(16, 16)
fn combine(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(rgb_in);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let original = textureLoad(lightness_plane, id.xy, 0).r;
    var enhanced = original;
    if contrast.amounts.x != 0.0 {
        let midtone_weight = clamp(4.0 * original * (1.0 - original), 0.0, 1.0);
        let base = textureLoad(clarity_base, id.xy, 0).r;
        enhanced += contrast.amounts.x * (original - base) * midtone_weight;
    }
    if contrast.amounts.y != 0.0 {
        enhanced += contrast.amounts.y * (original - textureLoad(texture_base, id.xy, 0).r);
    }
    var gain = 1.0;
    if original > 1e-4 {
        let ratio = max(enhanced, 0.0) / original;
        gain = ratio * ratio;
    }
    let rgb = textureLoad(rgb_in, id.xy, 0).rgb;
    textureStore(rgb_out, id.xy, vec4<f32>(clamp(rgb * gain, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0));
}
