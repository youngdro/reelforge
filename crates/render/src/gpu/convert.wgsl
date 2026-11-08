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
