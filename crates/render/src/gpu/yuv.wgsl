// A decoded video picture, 8-bit planar Y'CbCr, into the compositor's picture form: linear,
// premultiplied (opaque). Same sums as `Yuv::to_image` and the decoder's own display path:
// nearest chroma, R'G'B' rounded to 8 bits, then the sRGB curve undone.

struct Params {
    // Kr, Kb, luma scale, luma offset
    k: vec4<f32>,
    // chroma scale, chroma shift x, chroma shift y, unused
    c: vec4<f32>,
}

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var luma: texture_2d<f32>;
@group(0) @binding(2) var cb: texture_2d<f32>;
@group(0) @binding(3) var cr: texture_2d<f32>;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<u32>(pos.xy);
    let q = vec2(p.x >> u32(P.c.y), p.y >> u32(P.c.z));
    let kr = P.k.x;
    let kb = P.k.y;
    let kg = 1.0 - kr - kb;
    let cs = P.c.x;
    let yy = (round(textureLoad(luma, p, 0).r * 255.0) - P.k.w) * P.k.z;
    let u = round(textureLoad(cb, q, 0).r * 255.0) - 128.0;
    let v = round(textureLoad(cr, q, 0).r * 255.0) - 128.0;
    let crr = 2.0 * (1.0 - kr) * cs;
    let cbb = 2.0 * (1.0 - kb) * cs;
    let rgb = vec3(yy + crr * v, yy - crr * kr / kg * v - cbb * kb / kg * u, yy + cbb * u);
    let c = clamp(round(rgb), vec3(0.0), vec3(255.0)) / 255.0;
    return vec4(srgb_to_linear(c.r), srgb_to_linear(c.g), srgb_to_linear(c.b), 1.0);
}
