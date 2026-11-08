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
    // x: 1 brightness, 2 saturation (y: amount); 3 grade, over three slots (exposure, contrast,
    // saturation | temperature, tint, highlights, shadows | fade, vignette); 6 lookup table over
    // three slots (amount, size | domain min | domain max)
    fx: array<vec4<f32>, 16>,
}

@group(0) @binding(0) var<uniform> L: Layer;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_2d<f32>;
@group(0) @binding(3) var lut: texture_3d<f32>;

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

fn luma3(c: vec3<f32>) -> f32 {
    return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
}

// `cpu::grade`, step for step
fn grade(p: vec4<f32>, a0: vec4<f32>, a1: vec4<f32>, a2: vec4<f32>, uv: vec2<f32>) -> vec4<f32> {
    let a = p.a;
    if a <= 0.0 {
        return p;
    }
    let e = exp2(clamp(a0.y, -5.0, 5.0));
    let wb = vec3(1.0 + 0.25 * a1.x + 0.1 * a1.y, 1.0 - 0.2 * a1.y, 1.0 - 0.25 * a1.x + 0.1 * a1.y);
    var c = max(p.rgb / a * e * wb, vec3(0.0));
    let q = min(sqrt(max(luma3(c), 0.0)), 1.0);
    let ws = clamp(1.0 - q / 0.6, 0.0, 1.0);
    let wh = clamp((q - 0.4) / 0.6, 0.0, 1.0);
    let k = max(1.0 + a1.w * ws * ws + 0.8 * a1.z * wh * wh, 0.0);
    let power = 1.0 + clamp(a0.z, -0.9, 1.0);
    c = 0.18 * pow(c * k / 0.18, vec3(power));
    let l = luma3(c);
    let s = 1.0 + clamp(a0.w, -1.0, 1.0);
    let lift = 0.04 * clamp(a2.x, 0.0, 1.0);
    let d = length(uv - 0.5) * 1.4142135;
    let v = 1.0 - clamp(a2.y, 0.0, 1.0) * smoothstep(0.3, 1.0, d);
    let x = max(l + (c - l) * s, vec3(0.0));
    return vec4((x * (1.0 - 3.0 * lift) + lift) * v * a, a);
}

fn to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

// `Lut::apply`: encoded, looked up between the points, decoded, mixed in
fn apply_lut(p: vec4<f32>, a0: vec4<f32>, lo: vec4<f32>, hi: vec4<f32>) -> vec4<f32> {
    let a = p.a;
    if a <= 0.0 {
        return p;
    }
    let st = clamp(p.rgb / a, vec3(0.0), vec3(1.0));
    let enc = vec3(to_srgb(st.r), to_srgb(st.g), to_srgb(st.b));
    let n = i32(a0.z);
    let u = clamp((enc - lo.xyz) / (hi.xyz - lo.xyz), vec3(0.0), vec3(1.0)) * f32(n - 1);
    let b = min(vec3<i32>(floor(u)), vec3(n - 2));
    let f = u - vec3<f32>(b);
    var looked = vec3(0.0);
    for (var j = 0; j < 8; j++) {
        let o = vec3(j & 1, (j >> 1u) & 1, (j >> 2u) & 1);
        let w = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1) * select(1.0 - f.z, f.z, o.z == 1);
        looked += w * textureLoad(lut, b + o, 0).rgb;
    }
    let out = clamp(looked, vec3(0.0), vec3(1.0));
    let lin = vec3(to_linear(out.r), to_linear(out.g), to_linear(out.b)) * a;
    return vec4(p.rgb + (lin - p.rgb) * clamp(a0.y, 0.0, 1.0), a);
}

fn effects(p0: vec4<f32>, uv: vec2<f32>) -> vec4<f32> {
    var p = p0;
    var i = 0u;
    while i < L.nfx {
        let fx = L.fx[i];
        if fx.x == 1.0 {
            p = vec4(max(p.rgb + fx.y * p.a, vec3(0.0)), p.a);
        } else if fx.x == 2.0 {
            let luma = 0.2126 * p.r + 0.7152 * p.g + 0.0722 * p.b;
            p = vec4(max(luma + (p.rgb - luma) * fx.y, vec3(0.0)), p.a);
        } else if fx.x == 3.0 {
            p = grade(p, fx, L.fx[i + 1u], L.fx[i + 2u], uv);
            i += 2u;
        } else if fx.x == 6.0 {
            p = apply_lut(p, fx, L.fx[i + 1u], L.fx[i + 2u]);
            i += 2u;
        }
        i += 1u;
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
    s = effects(s, vec2(sx / L.inv1.z, sy / L.inv1.w)) * L.opacity;
    if L.blend == 2u {
        let d = textureLoad(dst, vec2<i32>(pos.xy), 0);
        let rgb = s.rgb * d.rgb + s.rgb * (1.0 - d.a) + d.rgb * (1.0 - s.a);
        return vec4(rgb, s.a + d.a * (1.0 - s.a));
    }
    return s;
}
