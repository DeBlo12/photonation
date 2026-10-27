// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

// Mask overlay: tints each screen pixel by the selected mask's weight.
// `weight` mirrors photonation_pipeline::masks::MaskWeight::at (the
// reference); the CPU prepares the same parameters (mask_overlay.rs).

// Keep in step with MAX_MASK_COMPONENTS (photonation-develop).
const MAX_COMPONENTS: u32 = 8u;

// One shape of the mask: photonation_pipeline::masks::ShapeWeight.
struct Component {
    // Linear: start (xy), ramp (zw). Radial: centre (xy).
    a: vec4<f32>,
    // Radial: axis_x (xy), axis_y (zw), each divided by its radius.
    b: vec4<f32>,
    // 0: linear gradient, 1: radial gradient.
    kind: u32,
    // How it combines with the shapes before it: 0 add, 1 subtract,
    // 2 intersect (photonation_develop::CombineMode).
    mode: u32,
    // Radial: normalized radius where the fade starts (1 - feather).
    fade_start: f32,
    _padding: f32,
};

struct Overlay {
    // Metric mask point of framebuffer pixel p:
    // origin + p.x * step_x + p.y * step_y (a straightened crop turns it).
    origin: vec2<f32>,
    step_x: vec2<f32>,
    step_y: vec2<f32>,
    // How many entries of `components` are in use.
    count: u32,
    invert: u32,
    // Premultiplied colour at full weight.
    color: vec4<f32>,
    components: array<Component, MAX_COMPONENTS>,
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

fn shape_weight(component: Component, point: vec2<f32>) -> f32 {
    if component.kind == 0u {
        return 1.0 - ramp(0.0, 1.0, dot(point - component.a.xy, component.a.zw));
    }
    let offset = point - component.a.xy;
    let distance = length(vec2<f32>(dot(offset, component.b.xy), dot(offset, component.b.zw)));
    if component.fade_start >= 1.0 {
        return select(0.0, 1.0, distance <= 1.0);
    }
    return 1.0 - ramp(component.fade_start, 1.0, distance);
}

fn weight(point: vec2<f32>) -> f32 {
    var selected = 0.0;
    for (var index = 0u; index < min(overlay.count, MAX_COMPONENTS); index++) {
        let component = overlay.components[index];
        let w = shape_weight(component, point);
        if index == 0u {
            selected = w;
        } else if component.mode == 0u {
            selected = max(selected, w);
        } else if component.mode == 1u {
            selected = min(selected, 1.0 - w);
        } else {
            selected = min(selected, w);
        }
    }
    if overlay.invert != 0u {
        selected = 1.0 - selected;
    }
    return selected;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let point = overlay.origin + position.x * overlay.step_x + position.y * overlay.step_y;
    return overlay.color * weight(point);
}
