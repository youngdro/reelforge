// From the compositor's working picture (linear, premultiplied) to what leaves the GPU: straight
// sRGB for display and PNGs (`Image::to_srgba8`), and BT.709 limited-range 4:2:0 for the encoder
// (`to_yuv420` in reelforge-media). Same formulas as the CPU code.

@group(0) @binding(0) var frame: texture_2d<f32>;

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

// ---- display: a full-frame triangle into an Rgba8Unorm target ----

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn straight(pos: vec4<f32>) -> vec4<f32> {
    let p = textureLoad(frame, vec2<i32>(pos.xy), 0);
    let a = clamp(p.a, 0.0, 1.0);
    if a <= 0.0 {
        return vec4(0.0);
    }
    let c = clamp(p.rgb / a, vec3(0.0), vec3(1.0));
    return vec4(linear_to_srgb(c.r), linear_to_srgb(c.g), linear_to_srgb(c.b), a);
}

@fragment
fn fs_srgb(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return straight(pos);
}

// egui's textures are premultiplied in sRGB
@fragment
fn fs_srgb_premultiplied(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = straight(pos);
    return vec4(c.rgb * c.a, c.a);
}

// ---- glow: the picture far out of focus, a few texels a side ----

// The glow's side in texels (`GLOW` in mod.rs).
const GLOW_SIDE: f32 = 16.0;
const GLOW_TAPS: i32 = 16;
// One deviation of the blur, as a part of the picture's side.
const GLOW_SPREAD: f32 = 0.15;

// A texel of the glow is the light of a wide part of the picture: taps on a grid out to three
// deviations (nearer, the blur's end shows as an edge in the dark), weighed by a Gaussian. Light is added as light (linear, premultiplied: what is
// transparent gives none) and leaves as opaque sRGB. Past the picture's edge, the edge goes on.
@fragment
fn fs_glow(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let last = vec2<f32>(textureDimensions(frame)) - 1.0;
    let at = pos.xy / GLOW_SIDE;
    var sum = vec3(0.0);
    var weight = 0.0;
    for (var j = 0; j < GLOW_TAPS; j++) {
        for (var i = 0; i < GLOW_TAPS; i++) {
            let d = (vec2(f32(i), f32(j)) + 0.5) / f32(GLOW_TAPS) * 6.0 - 3.0;
            let w = exp(-0.5 * dot(d, d));
            let p = clamp(at + d * GLOW_SPREAD, vec2(0.0), vec2(1.0)) * last;
            sum += w * clamp(textureLoad(frame, vec2<i32>(round(p)), 0).rgb, vec3(0.0), vec3(1.0));
            weight += w;
        }
    }
    let c = sum / weight;
    return vec4(linear_to_srgb(c.r), linear_to_srgb(c.g), linear_to_srgb(c.b), 1.0);
}

// ---- frost: the picture as frosted glass over it shows it ----

// The frost's side in texels (`FROST` in mod.rs).
const FROST_SIDE: f32 = 96.0;
const FROST_TAPS: i32 = 16;
// One deviation of the blur, as a part of the picture's longer side.
const FROST_SPREAD: f32 = 0.025;
// The frost is the picture and this much of each of its sides around it (`FROST_ROOM` in mod.rs),
// where its light ends softly.
const FROST_ROOM: f32 = 0.125;

// A texel of the frost is the picture near it, blurred: taps on a grid out to two and a half
// deviations, weighed by a Gaussian, where there is no picture nothing. It leaves as sRGB with
// premultiplied alpha (what egui draws), so it lies over whatever is around the picture.
@fragment
fn fs_frost(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(frame));
    let at = (pos.xy / FROST_SIDE * (1.0 + 2.0 * FROST_ROOM) - FROST_ROOM) * size;
    let spread = FROST_SPREAD * max(size.x, size.y);
    var sum = vec4(0.0);
    var weight = 0.0;
    for (var j = 0; j < FROST_TAPS; j++) {
        for (var i = 0; i < FROST_TAPS; i++) {
            let d = (vec2(f32(i), f32(j)) + 0.5) / f32(FROST_TAPS) * 5.0 - 2.5;
            let w = exp(-0.5 * dot(d, d));
            let p = at + d * spread;
            if p.x >= 0.0 && p.y >= 0.0 && p.x < size.x && p.y < size.y {
                sum += w * clamp(textureLoad(frame, vec2<i32>(p), 0), vec4(0.0), vec4(1.0));
            }
            weight += w;
        }
    }
    // the outermost texels are empty: drawn past its edge, a texture repeats them
    let edge = min(min(pos.x, pos.y), min(FROST_SIDE - pos.x, FROST_SIDE - pos.y));
    if sum.a <= 0.0 || edge < 1.0 {
        return vec4(0.0);
    }
    let c = clamp(sum.rgb / sum.a, vec3(0.0), vec3(1.0));
    let a = sum.a / weight;
    return vec4(vec3(linear_to_srgb(c.r), linear_to_srgb(c.g), linear_to_srgb(c.b)) * a, a);
}

// ---- encoder input: Y', Cb, Cr planes, four 8-bit samples per word ----

struct Dims {
    width: u32,
    height: u32,
    // words per row of the luma plane and of each chroma plane
    luma_stride: u32,
    chroma_stride: u32,
}

@group(0) @binding(1) var<uniform> D: Dims;
@group(0) @binding(2) var<storage, read_write> luma_out: array<u32>;
@group(0) @binding(3) var<storage, read_write> cb_out: array<u32>;
@group(0) @binding(4) var<storage, read_write> cr_out: array<u32>;

fn gamma_rgb(x: u32, y: u32) -> vec3<f32> {
    let p = clamp(textureLoad(frame, vec2(i32(x), i32(y)), 0).rgb, vec3(0.0), vec3(1.0));
    return vec3(linear_to_srgb(p.r), linear_to_srgb(p.g), linear_to_srgb(p.b));
}

fn luma_of(c: vec3<f32>) -> f32 {
    return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
}

@compute @workgroup_size(16, 4)
fn luma(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= D.luma_stride || id.y >= D.height {
        return;
    }
    var word = 0u;
    for (var k = 0u; k < 4u; k++) {
        let x = id.x * 4u + k;
        if x < D.width {
            let v = u32(round(16.0 + 219.0 * luma_of(gamma_rgb(x, id.y))));
            word |= (v & 255u) << (8u * k);
        }
    }
    luma_out[id.y * D.luma_stride + id.x] = word;
}

@compute @workgroup_size(16, 4)
fn chroma(@builtin(global_invocation_id) id: vec3<u32>) {
    let cw = (D.width + 1u) / 2u;
    let ch = (D.height + 1u) / 2u;
    if id.x >= D.chroma_stride || id.y >= ch {
        return;
    }
    var cbw = 0u;
    var crw = 0u;
    for (var k = 0u; k < 4u; k++) {
        let cx = id.x * 4u + k;
        if cx >= cw {
            break;
        }
        var cb = 0.0;
        var cr = 0.0;
        var n = 0.0;
        for (var row = id.y * 2u; row < min(id.y * 2u + 2u, D.height); row++) {
            for (var x = cx * 2u; x < min(cx * 2u + 2u, D.width); x++) {
                let c = gamma_rgb(x, row);
                let l = luma_of(c);
                cb += (c.b - l) / 1.8556;
                cr += (c.r - l) / 1.5748;
                n += 1.0;
            }
        }
        let u = u32(clamp(round(128.0 + 224.0 * cb / n), 16.0, 240.0));
        let v = u32(clamp(round(128.0 + 224.0 * cr / n), 16.0, 240.0));
        cbw |= u << (8u * k);
        crw |= v << (8u * k);
    }
    cb_out[id.y * D.chroma_stride + id.x] = cbw;
    cr_out[id.y * D.chroma_stride + id.x] = crw;
}
