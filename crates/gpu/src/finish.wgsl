// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

// The output end of the pipeline: display encoding (`encode.rs`), the
// unsharp mask on encoded luma (`sharpen.rs`, one pass along rows and one
// along columns) and packing into 8-bit RGB in display orientation (the
// pipeline's `render_samples`).

struct Finish {
    // x: sharpen kernel radius.
    sharpen_radius: vec4<u32>,
    // xyz: luma weights; w: sharpen amount.
    sharpen: vec4<f32>,
    // Stored pixel of display pixel d: (dot(xy, d) + z) per row.
    to_stored: array<vec4<i32>, 2>,
    // x: display width; y: output bytes.
    display: vec4<u32>,
}

@group(0) @binding(0) var<uniform> finish: Finish;
// encode
@group(0) @binding(1) var linear_rgb: texture_2d<f32>;
@group(0) @binding(2) var<storage, read> transfer: array<f32>;
@group(0) @binding(3) var encoded_out: texture_storage_2d<rgba32float, write>;
// sharpen
@group(0) @binding(4) var encoded: texture_2d<f32>;
@group(0) @binding(5) var<storage, read> kernel: array<f32>;
@group(0) @binding(6) var blurred_rows_out: texture_storage_2d<r32float, write>;
@group(0) @binding(7) var blurred_rows: texture_2d<f32>;
@group(0) @binding(8) var sharpened_out: texture_storage_2d<rgba32float, write>;
// pack
@group(0) @binding(9) var finished: texture_2d<f32>;
@group(0) @binding(10) var<storage, read_write> packed: array<u32>;

@compute @workgroup_size(16, 16)
fn encode(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(encoded_out);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let last = arrayLength(&transfer) - 1u;
    let rgb = textureLoad(linear_rgb, id.xy, 0).rgb;
    var out = vec3<f32>(0.0);
    for (var channel = 0u; channel < 3u; channel++) {
        let position = clamp(rgb[channel], 0.0, 1.0) * f32(last);
        let index = min(u32(position), last - 1u);
        let fraction = position - f32(index);
        out[channel] = transfer[index] + (transfer[index + 1u] - transfer[index]) * fraction;
    }
    textureStore(encoded_out, id.xy, vec4<f32>(out, 1.0));
}

fn luma(rgb: vec3<f32>) -> f32 {
    return rgb.x * finish.sharpen.x + rgb.y * finish.sharpen.y + rgb.z * finish.sharpen.z;
}

@compute @workgroup_size(16, 16)
fn sharpen_rows(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(encoded);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let radius = i32(finish.sharpen_radius.x);
    var sum = 0.0;
    for (var k = 0; k <= 2 * radius; k++) {
        let x = clamp(i32(id.x) + k - radius, 0, i32(size.x) - 1);
        sum += luma(textureLoad(encoded, vec2<i32>(x, i32(id.y)), 0).rgb) * kernel[k];
    }
    textureStore(blurred_rows_out, id.xy, vec4<f32>(sum, 0.0, 0.0, 1.0));
}

@compute @workgroup_size(16, 16)
fn sharpen_columns(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(encoded);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let radius = i32(finish.sharpen_radius.x);
    var blurred = 0.0;
    for (var k = 0; k <= 2 * radius; k++) {
        let y = clamp(i32(id.y) + k - radius, 0, i32(size.y) - 1);
        blurred += textureLoad(blurred_rows, vec2<i32>(i32(id.x), y), 0).r * kernel[k];
    }
    let rgb = textureLoad(encoded, id.xy, 0).rgb;
    let detail = finish.sharpen.w * (luma(rgb) - blurred);
    textureStore(sharpened_out, id.xy, vec4<f32>(clamp(rgb + detail, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0));
}

// One word (four bytes) of the interleaved RGB output per invocation.
@compute @workgroup_size(256)
fn pack(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
) {
    let word = (group.y * groups.x + group.x) * 256u + local;
    let total = finish.display.y;
    if word * 4u >= total {
        return;
    }
    var bits = 0u;
    for (var i = 0u; i < 4u; i++) {
        let byte = word * 4u + i;
        if byte >= total {
            break;
        }
        let pixel = byte / 3u;
        let display = vec2<i32>(i32(pixel % finish.display.x), i32(pixel / finish.display.x));
        let stored = vec2<i32>(
            dot(finish.to_stored[0].xy, display) + finish.to_stored[0].z,
            dot(finish.to_stored[1].xy, display) + finish.to_stored[1].z,
        );
        let value = textureLoad(finished, stored, 0)[byte % 3u];
        bits |= u32(clamp(value, 0.0, 1.0) * 255.0 + 0.5) << (8u * i);
    }
    packed[word] = bits;
}
