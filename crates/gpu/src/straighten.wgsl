// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

// Straightening (pipeline `straighten.rs`): resamples the developed image
// along a turned crop frame with a Catmull–Rom kernel, so the frame's
// content comes out upright. Parameters come from `StraightenPlan`.

struct Frame {
    // xy: input position of output position (0, 0); zw: input step per
    // output pixel along x.
    origin_step_x: vec4<f32>,
    // xy: input step per output pixel along y.
    step_y: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var input: texture_2d<f32>;
@group(0) @binding(2) var output: texture_storage_2d<rgba32float, write>;

// Weights of the four pixels around a sample at fraction `t` past the second.
fn catmull_rom(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(
        (-t3 + 2.0 * t2 - t) / 2.0,
        (3.0 * t3 - 5.0 * t2 + 2.0) / 2.0,
        (-3.0 * t3 + 4.0 * t2 + t) / 2.0,
        (t3 - t2) / 2.0,
    );
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let position = vec2<f32>(id.xy) + vec2<f32>(0.5);
    let input_position = frame.origin_step_x.xy + position.x * frame.origin_step_x.zw
        + position.y * frame.step_y.xy;
    let sample_position = input_position - vec2<f32>(0.5);
    let base = floor(sample_position);
    let weights_x = catmull_rom(sample_position.x - base.x);
    let weights_y = catmull_rom(sample_position.y - base.y);
    let last = vec2<i32>(textureDimensions(input)) - vec2<i32>(1);
    var sum = vec3<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        let row = clamp(i32(base.y) - 1 + j, 0, last.y);
        for (var i = 0; i < 4; i++) {
            let column = clamp(i32(base.x) - 1 + i, 0, last.x);
            let weight = weights_x[i] * weights_y[j];
            sum += textureLoad(input, vec2<i32>(column, row), 0).rgb * weight;
        }
    }
    // Bicubic lobes overshoot at edges; keep values displayable.
    textureStore(output, id.xy, vec4<f32>(clamp(sum, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0));
}
