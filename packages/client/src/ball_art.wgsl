#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct Ball {
    orientation: vec4<f32>,
    number: vec4<f32>,
};
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> ball: Ball;

fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
}

fn palette(number: u32) -> vec3<f32> {
    switch (number - 1u) % 8u {
        case 0u: { return vec3(0.96, 0.75, 0.08); }
        case 1u: { return vec3(0.10, 0.30, 0.85); }
        case 2u: { return vec3(0.88, 0.12, 0.10); }
        case 3u: { return vec3(0.40, 0.12, 0.58); }
        case 4u: { return vec3(0.95, 0.38, 0.04); }
        case 5u: { return vec3(0.08, 0.47, 0.20); }
        case 6u: { return vec3(0.50, 0.12, 0.08); }
        default: { return vec3(0.04); }
    }
}

// Original 3-by-5 digits, projected with the badge onto the sphere.
fn digit_mask(digit: u32) -> u32 {
    switch digit {
        case 0u: { return 31599u; }
        case 1u: { return 29842u; }
        case 2u: { return 29671u; }
        case 3u: { return 31207u; }
        case 4u: { return 18925u; }
        case 5u: { return 31183u; }
        case 6u: { return 31695u; }
        case 7u: { return 18727u; }
        case 8u: { return 31727u; }
        default: { return 31215u; }
    }
}

fn digit_ink(p: vec2<f32>, digit: u32) -> bool {
    if any(p < vec2(0.0)) || any(p >= vec2(3.0, 5.0)) { return false; }
    let cell = vec2<u32>(floor(p));
    return (digit_mask(digit) & (1u << (cell.y * 3u + cell.x))) != 0u;
}

fn surface(local: vec3<f32>, number: u32) -> vec3<f32> {
    let ivory = vec3(0.97, 0.96, 0.91);
    if number == 0u { return ivory; }
    if abs(local.z) > 0.903 {
        let width = select(3.0, 7.0, number >= 10u);
        // Opposite badges stay attached to the surface, including on the back.
        let x = local.x * select(-1.0, 1.0, local.z > 0.0);
        let p = vec2(x, -local.y) / 0.092 + vec2(width * 0.5, 2.5);
        var ink = digit_ink(p, number);
        if number >= 10u {
            ink = digit_ink(p, number / 10u) || digit_ink(p - vec2(4.0, 0.0), number % 10u);
        }
        return select(ivory, vec3(0.035, 0.045, 0.055), ink);
    }
    if number > 8u && abs(local.y) > 0.48 { return ivory; }
    return palette(number);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = (in.uv * 2.0 - 1.0) * 1.25;
    let radius = length(p);
    if radius >= 1.0 {
        let shadow = length(p - vec2(0.07, 0.12));
        return vec4(0.0, 0.0, 0.0, clamp((1.17 - shadow) * 2.0, 0.0, 0.28));
    }
    let normal = vec3(p.x, -p.y, sqrt(1.0 - radius * radius));
    let local = rotate(vec4(-ball.orientation.xyz, ball.orientation.w), normal);
    let light = max(dot(normal, vec3(-0.35, 0.45, 0.82)), 0.0);
    let highlight = exp(-dot(p + vec2(0.30, 0.38), p + vec2(0.30, 0.38)) / 0.025) * 0.50;
    let color = min(surface(local, u32(ball.number.x)) * (0.42 + 0.58 * light) + highlight, vec3(1.0));
    // Palette is authored in sRGB, whereas material output is linear.
    let linear = select(color / 12.92, pow((color + 0.055) / 1.055, vec3(2.4)), color > vec3(0.04045));
    return vec4(linear, clamp((1.0 - radius) / max(fwidth(radius), 0.001), 0.0, 1.0));
}
