// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

// Mask overlay: tints each screen pixel by the selected mask's weight.
// `weight` mirrors photonation_pipeline::masks::ShapeWeight::at (the
// reference); the CPU prepares the same parameters (mask_overlay.rs).

struct Overlay {
    // Metric mask point of framebuffer pixel p: origin + p * step.
    origin: vec2<f32>,
    step: vec2<f32>,
    // 0: linear gradient, 1: radial gradient.
    kind: u32,
    invert: u32,
    // Radial: normalized radius where the fade starts (1 - feather).
    fade_start: f32,
    _padding: f32,
    // Linear: start (xy), ramp (zw). Radial: centre (xy).
    a: vec4<f32>,
    // Radial: axis_x (xy), axis_y (zw), each divided by its radius.
    b: vec4<f32>,
    // Premultiplied colour at full weight.
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> overlay: Overlay;

// One triangle covering the viewport, which egui sets to the image rect.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

// Same as the pipeline's smoothstep (WGSL's is undefined for edge0 >= edge1).
fn ramp(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn weight(point: vec2<f32>) -> f32 {
    var w: f32;
    if overlay.kind == 0u {
        w = 1.0 - ramp(0.0, 1.0, dot(point - overlay.a.xy, overlay.a.zw));
    } else {
        let offset = point - overlay.a.xy;
        let distance = length(vec2<f32>(dot(offset, overlay.b.xy), dot(offset, overlay.b.zw)));
        if overlay.fade_start >= 1.0 {
            w = select(0.0, 1.0, distance <= 1.0);
        } else {
            w = 1.0 - ramp(overlay.fade_start, 1.0, distance);
        }
    }
    if overlay.invert != 0u {
        w = 1.0 - w;
    }
    return w;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return overlay.color * weight(overlay.origin + position.xy * overlay.step);
}
