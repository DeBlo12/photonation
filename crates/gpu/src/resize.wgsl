// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

// Separable Lanczos-3 resampling with the CPU's precomputed weights
// (pipeline `resize.rs`): one pass along rows, then one along columns.

@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;
// Per output coordinate: first input, offset into `weights`, tap count.
@group(0) @binding(2) var<storage, read> taps: array<vec4<u32>>;
@group(0) @binding(3) var<storage, read> weights: array<f32>;

@compute @workgroup_size(16, 16)
fn horizontal(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let tap = taps[id.x];
    var sum = vec3<f32>(0.0);
    for (var k = 0u; k < tap.z; k++) {
        sum += textureLoad(input, vec2<u32>(tap.x + k, id.y), 0).rgb * weights[tap.y + k];
    }
    textureStore(output, id.xy, vec4<f32>(sum, 1.0));
}

@compute @workgroup_size(16, 16)
fn vertical(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let tap = taps[id.y];
    var sum = vec3<f32>(0.0);
    for (var k = 0u; k < tap.z; k++) {
        sum += textureLoad(input, vec2<u32>(id.x, tap.x + k), 0).rgb * weights[tap.y + k];
    }
    // Lanczos lobes overshoot at edges; keep values displayable.
    textureStore(output, id.xy, vec4<f32>(clamp(sum, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0));
}
