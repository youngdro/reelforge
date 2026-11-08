// One layer of a frame plan, drawn over the target. Mirrors `cpu.rs` step for step: the same
// inverse map, the same bilinear sample with transparent outside the picture, the same effects,
// opacity and blend, all in linear light with premultiplied alpha.

struct Layer {
    // inverse map target px -> content px: sx = a x + b y + c, sy = d x + e y + f
    inv0: vec4<f32>, // a, b, c, d
    inv1: vec4<f32>, // e, f, content width, content height
    bbox: vec4<f32>, // target px box to cover: x0, y0, x1, y1
    color: vec4<f32>, // flat colour, premultiplied linear
    frame_size: vec2<f32>,
    opacity: f32,
    // 0: the bound picture, bilinear; 1: flat colour; 2: a picture filled with `color`, bilinear
    mode: u32,
    // 0 normal, 1 add, 2 multiply, 3 screen (only multiply is blended here; the rest are fixed
    // function)
    blend: u32,
    nfx: u32,
    _pad0: u32,
    _pad1: u32,
    // x: 1 brightness, 2 saturation; y: amount
    fx: array<vec4<f32>, 8>,
}

@group(0) @binding(0) var<uniform> L: Layer;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corner = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let c = corner[i];
    let p = mix(L.bbox.xy, L.bbox.zw, c);
    return vec4(p.x / L.frame_size.x * 2.0 - 1.0, 1.0 - p.y / L.frame_size.y * 2.0, 0.0, 1.0);
}

fn texel(x: i32, y: i32) -> vec4<f32> {
    let size = vec2<i32>(L.inv1.zw);
    if x < 0 || y < 0 || x >= size.x || y >= size.y {
        return vec4(0.0);
    }
    if L.mode == 2u {
        return L.color;
    }
    return textureLoad(src, vec2(x, y), 0);
}

// `Image::sample`: pixel centres at n + 0.5, transparent outside.
fn bilinear(x: f32, y: f32) -> vec4<f32> {
    let f = vec2(x - 0.5, y - 0.5);
    let o = floor(f);
    let t = f - o;
    let i = vec2<i32>(o);
    let a = texel(i.x, i.y);
    let b = texel(i.x + 1, i.y);
    let c = texel(i.x, i.y + 1);
    let d = texel(i.x + 1, i.y + 1);
    let top = a + (b - a) * t.x;
    let bottom = c + (d - c) * t.x;
    return top + (bottom - top) * t.y;
}

fn effects(p0: vec4<f32>) -> vec4<f32> {
    var p = p0;
    for (var i = 0u; i < L.nfx; i++) {
        let fx = L.fx[i];
        if fx.x == 1.0 {
            p = vec4(max(p.rgb + fx.y * p.a, vec3(0.0)), p.a);
        } else if fx.x == 2.0 {
            let luma = 0.2126 * p.r + 0.7152 * p.g + 0.0722 * p.b;
            p = vec4(max(luma + (p.rgb - luma) * fx.y, vec3(0.0)), p.a);
        }
    }
    return p;
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let sx = L.inv0.x * pos.x + L.inv0.y * pos.y + L.inv0.z;
    let sy = L.inv0.w * pos.x + L.inv1.x * pos.y + L.inv1.y;
    if sx < 0.0 || sy < 0.0 || sx >= L.inv1.z || sy >= L.inv1.w {
        discard;
    }
    var s: vec4<f32>;
    if L.mode == 1u {
        s = L.color;
    } else {
        s = bilinear(sx, sy);
    }
    s = effects(s) * L.opacity;
    if L.blend == 2u {
        let d = textureLoad(dst, vec2<i32>(pos.xy), 0);
        let rgb = s.rgb * d.rgb + s.rgb * (1.0 - d.a) + d.rgb * (1.0 - s.a);
        return vec4(rgb, s.a + d.a * (1.0 - s.a));
    }
    return s;
}
