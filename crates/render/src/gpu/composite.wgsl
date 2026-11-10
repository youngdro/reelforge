// One layer of a frame plan, drawn over the target. Mirrors `cpu.rs` step for step: the same
// inverse map, the same bilinear sample with transparent outside the picture, the same effects,
// opacity and blend, all in linear light with premultiplied alpha.

struct Layer {
    // inverse map target px -> content px: sx = (a x + b y + c) / q, sy = (d x + e y + f) / q
    inv0: vec4<f32>, // a, b, c, d
    inv1: vec4<f32>, // e, f, content width, content height
    bbox: vec4<f32>, // target px box to cover: x0, y0, x1, y1
    color: vec4<f32>, // flat colour, premultiplied linear
    frame_size: vec2<f32>,
    opacity: f32,
    // 0: the bound picture, bilinear; 1: flat colour; 2: a picture filled with `color`, bilinear
    mode: u32,
    // 0 normal, 1 add, 2 multiply, 3 screen, 4 overlay, 5 darken, 6 lighten, 7 soft light,
    // 8 hard light, 9 difference, 10 colour dodge, 11 colour burn (normal, add and screen are
    // fixed function; the rest are blended here)
    blend: u32,
    nfx: u32,
    _pad0: u32,
    _pad1: u32,
    // the whole picture's size; the size a mask is shaped on
    pic: vec4<f32>,
    // crop offset in picture pixels; mirrored x, y (1 or 0)
    geo: vec4<f32>,
    // mask: kind (0 none, 1 linear, 2 mirror, 3 circle, 4 rectangle), centre x, y, feather
    mask0: vec4<f32>,
    // mask: width, height, rotation (degrees), inverted (1 or 0)
    mask1: vec4<f32>,
    // distortions: wave amount, waves, phase, mosaic size
    warp0: vec4<f32>,
    // distortions: glitch amount, seed, split x, y
    warp1: vec4<f32>,
    // the inverse map's third row: sx and sy are divided by g x + h y + i (0, 0, 1 for a layer
    // that is not turned in space)
    persp: vec4<f32>,
    // x: 1 brightness, 2 saturation (y: amount); 3 grade, over three slots (exposure, contrast,
    // saturation | temperature, tint, highlights, shadows | fade, vignette); 6 lookup table over
    // three slots (amount, size | domain min | domain max); 7 chroma key over two slots
    // (tolerance, softness, spill | key r, g, b)
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
    let size = vec2<i32>(L.pic.xy);
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

// `distort::hash`
fn hash2(a: u32, b: u32) -> u32 {
    var x = (a * 0x9E3779B1u) ^ (b * 0x85EBCA77u + 0xC2B2AE3Du);
    x ^= x >> 15u;
    x *= 0x2C1B3C6Du;
    x ^= x >> 12u;
    x *= 0x297A2D39u;
    return x ^ (x >> 15u);
}

// the picture at a point of the visible part, in its pixels: mirrored, then moved past the crop
fn read(sx: f32, sy: f32) -> vec4<f32> {
    var px = sx;
    var py = sy;
    if L.geo.z > 0.5 {
        px = L.inv1.z - sx;
    }
    if L.geo.w > 0.5 {
        py = L.inv1.w - sy;
    }
    return bilinear(px + L.geo.x, py + L.geo.y);
}

// `Warp::sample`: a point held half a pixel inside the visible part
fn held(uv: vec2<f32>) -> vec4<f32> {
    let size = L.inv1.zw;
    return read(min(max(uv.x * size.x, 0.5), size.x - 0.5), min(max(uv.y * size.y, 0.5), size.y - 0.5));
}

// `Warp::taps` and `Warp::sample`
fn warped(uv: vec2<f32>) -> vec4<f32> {
    let size = L.inv1.zw;
    var u = uv.x;
    var v = uv.y;
    if L.warp0.x != 0.0 {
        u += L.warp0.x * sin(6.2831855 * (v * L.warp0.y + L.warp0.z));
    }
    if L.warp1.x != 0.0 {
        let step = u32(floor(max(L.warp1.y, 0.0)));
        let bands = 6u + hash2(step, 0u) % 18u;
        let band = min(u32(max(floor(v * f32(bands)), 0.0)), bands - 1u);
        let h = hash2(step, band + 1u);
        if (h & 0xffu) < 96u {
            let slide = (f32((h >> 8u) & 0xffffu) / 65535.0 * 2.0 - 1.0) * L.warp1.x;
            let moved = u + slide;
            u = moved - floor(moved);
        }
    }
    if L.warp0.w > 0.0 {
        let side = L.warp0.w * min(size.x, size.y);
        let bx = side / size.x;
        let by = side / size.y;
        u = (floor(u / bx) + 0.5) * bx;
        v = (floor(v / by) + 0.5) * by;
    }
    let green = held(vec2(u, v));
    let split = L.warp1.zw;
    if split.x == 0.0 && split.y == 0.0 {
        return green;
    }
    let red = held(vec2(u, v) + split);
    let blue = held(vec2(u, v) - split);
    return vec4(red.r, green.g, blue.b, max(max(red.a, green.a), blue.a));
}

// `cpu::chroma_key`, step for step
fn chroma_key(p: vec4<f32>, how: vec4<f32>, key: vec4<f32>) -> vec4<f32> {
    let a = p.a;
    if a <= 0.0 {
        return p;
    }
    let tolerance = clamp(how.y, 0.0, 1.0);
    let softness = clamp(how.z, 0.0, 1.0);
    let spill = clamp(how.w, 0.0, 1.0);
    let c = p.rgb / a;
    let st = clamp(c, vec3(0.0), vec3(1.0));
    let e = vec3(to_srgb(st.r), to_srgb(st.g), to_srgb(st.b));
    let cb = vec3(-0.168736, -0.331264, 0.5);
    let cr = vec3(0.5, -0.418688, -0.081312);
    let q = vec2(dot(cb, e), dot(cr, e));
    let k = vec2(dot(cb, key.xyz), dot(cr, key.xyz));
    let d = length(q - k);
    var keep: f32;
    if softness > 0.0 {
        let t = clamp((d - tolerance) / softness, 0.0, 1.0);
        keep = t * t * (3.0 - 2.0 * t);
    } else if d > tolerance {
        keep = 1.0;
    } else {
        keep = 0.0;
    }
    let near = 1.0 - clamp((d - tolerance) / (softness + 0.3), 0.0, 1.0);
    let len = length(k);
    var towards = 0.0;
    if len > 1e-4 {
        towards = dot(q, k) / len;
    }
    var o = c;
    if spill * near > 0.0 && towards > 0.0 {
        let off = k * (-towards * spill * near / len);
        let m = clamp(
            e + vec3(1.402 * off.y, -0.344136 * off.x - 0.714136 * off.y, 1.772 * off.x),
            vec3(0.0),
            vec3(1.0),
        );
        o = vec3(to_linear(m.r), to_linear(m.g), to_linear(m.b));
    }
    return vec4(o * a * keep, a * keep);
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
        } else if fx.x == 7.0 {
            p = chroma_key(p, fx, L.fx[i + 1u]);
            i += 1u;
        }
        i += 1u;
    }
    return p;
}

// `ShapeMask::coverage`
fn coverage(uv: vec2<f32>) -> f32 {
    let kind = L.mask0.x;
    if kind == 0.0 {
        return 1.0;
    }
    let w = L.pic.z;
    let h = L.pic.w;
    let unit = max(min(w, h), 1e-6);
    let dx = (uv.x - L.mask0.y) * w / unit;
    let dy = (uv.y - L.mask0.z) * h / unit;
    let a = -radians(L.mask1.z);
    let sn = sin(a);
    let cs = cos(a);
    let x = dx * cs - dy * sn;
    let y = dx * sn + dy * cs;
    let hw = max(L.mask1.x, 0.0) / 2.0;
    let hh = max(L.mask1.y, 0.0) / 2.0;
    var d: f32;
    if kind == 1.0 {
        d = y;
    } else if kind == 2.0 {
        d = abs(y) - hh;
    } else if kind == 3.0 {
        let rx = max(hw, 1e-6);
        let ry = max(hh, 1e-6);
        d = (sqrt((x / rx) * (x / rx) + (y / ry) * (y / ry)) - 1.0) * min(rx, ry);
    } else {
        let qx = abs(x) - hw;
        let qy = abs(y) - hh;
        d = sqrt(max(qx, 0.0) * max(qx, 0.0) + max(qy, 0.0) * max(qy, 0.0)) + min(max(qx, qy), 0.0);
    }
    var m: f32;
    let feather = L.mask0.w;
    if feather > 0.0 {
        m = clamp(0.5 - d / feather, 0.0, 1.0);
    } else if d <= 0.0 {
        m = 1.0;
    } else {
        m = 0.0;
    }
    if L.mask1.w > 0.5 {
        return 1.0 - m;
    }
    return m;
}

fn hard_light(s: f32, d: f32) -> f32 {
    if s <= 0.5 {
        return 2.0 * s * d;
    }
    let t = 2.0 * s - 1.0;
    return d + t - d * t;
}

// `Blend::mix`: the mode's function of straight colours
fn mix1(s: f32, d: f32) -> f32 {
    switch L.blend {
        case 4u: { return hard_light(d, s); }
        case 5u: { return min(s, d); }
        case 6u: { return max(s, d); }
        case 7u: {
            if s <= 0.5 {
                return d - (1.0 - 2.0 * s) * d * (1.0 - d);
            }
            var g: f32;
            if d <= 0.25 {
                g = ((16.0 * d - 12.0) * d + 4.0) * d;
            } else {
                g = sqrt(d);
            }
            return d + (2.0 * s - 1.0) * (g - d);
        }
        case 8u: { return hard_light(s, d); }
        case 9u: { return abs(s - d); }
        case 10u: {
            if d <= 0.0 {
                return 0.0;
            }
            if s >= 1.0 {
                return 1.0;
            }
            return min(d / (1.0 - s), 1.0);
        }
        case 11u: {
            if d >= 1.0 {
                return 1.0;
            }
            if s <= 0.0 {
                return 0.0;
            }
            return 1.0 - min((1.0 - d) / s, 1.0);
        }
        default: { return s; }
    }
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // how far from the eye: 1 unless the layer is turned in space, 0 or less where it is not seen
    let q = L.persp.x * pos.x + L.persp.y * pos.y + L.persp.z;
    if q <= 1e-6 {
        discard;
    }
    let sx = (L.inv0.x * pos.x + L.inv0.y * pos.y + L.inv0.z) / q;
    let sy = (L.inv0.w * pos.x + L.inv1.x * pos.y + L.inv1.y) / q;
    if sx < 0.0 || sy < 0.0 || sx >= L.inv1.z || sy >= L.inv1.w {
        discard;
    }
    var s: vec4<f32>;
    if L.mode == 1u {
        s = L.color;
    } else if L.warp0.x != 0.0 || L.warp0.w > 0.0 || L.warp1.x != 0.0 || L.warp1.z != 0.0 || L.warp1.w != 0.0 {
        s = warped(vec2(sx / L.inv1.z, sy / L.inv1.w));
    } else {
        s = read(sx, sy);
    }
    let uv = vec2(sx / L.inv1.z, sy / L.inv1.w);
    s = effects(s, uv) * (L.opacity * coverage(uv));
    if L.blend == 2u {
        let d = textureLoad(dst, vec2<i32>(pos.xy), 0);
        let rgb = s.rgb * d.rgb + s.rgb * (1.0 - d.a) + d.rgb * (1.0 - s.a);
        return vec4(rgb, s.a + d.a * (1.0 - s.a));
    }
    if L.blend >= 4u {
        let d = textureLoad(dst, vec2<i32>(pos.xy), 0);
        var cs = vec3(0.0);
        var cd = vec3(0.0);
        if s.a > 0.0 {
            cs = s.rgb / s.a;
        }
        if d.a > 0.0 {
            cd = d.rgb / d.a;
        }
        let b = vec3(mix1(cs.r, cd.r), mix1(cs.g, cd.g), mix1(cs.b, cd.b));
        let rgb = s.rgb * (1.0 - d.a) + d.rgb * (1.0 - s.a) + s.a * d.a * b;
        return vec4(rgb, s.a + d.a * (1.0 - s.a));
    }
    return s;
}
