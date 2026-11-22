@group(0) @binding(0) var<storage, read> parameters: array<vec4f>;

fn tc_wrap(i: vec2i, size: vec2i) -> vec2i {
    let remainder = i % size;
    return select(remainder, remainder + size, remainder < vec2i(0));
}

fn tc_sample_position(uv: vec2f) -> vec2f {
    let finite = (bitcast<vec2u>(uv) & vec2u(0x7f800000u)) != vec2u(0x7f800000u);
    return select(vec2f(0.0), uv, finite);
}

fn tc_nearest(position: vec2f, n: vec2i, mode: u32) -> vec2i {
    let size = vec2f(n);
    if ((mode & 2u) != 0u) {
        return clamp(vec2i(floor(clamp(position, vec2f(0.0), vec2f(1.0)) * size)), vec2i(0), n - 1);
    }
    return tc_wrap(vec2i(floor(position * size)), n);
}

struct SampleFootprint {
    a: vec2i,
    b: vec2i,
    t: vec2f,
}

fn tc_bilinear(position: vec2f, n: vec2i, mode: u32) -> SampleFootprint {
    let size = vec2f(n);
    var uv = position;
    if ((mode & 2u) != 0u) {
        let edge = 0.5 / size;
        uv = clamp(uv, edge, 1.0 - edge);
    }
    let texel = uv * size - 0.5;
    let base = floor(texel);
    let a = tc_wrap(vec2i(base), n);
    let b = select(a + 1, vec2i(0), a + 1 == n);
    return SampleFootprint(a, b, texel - base);
}

fn tc_sample(input: texture_2d<f32>, uv: vec2f, mode: u32) -> vec4f {
    let n = vec2i(textureDimensions(input));
    let position = tc_sample_position(uv);
    if ((mode & 1u) == 0u) {
        return textureLoad(input, tc_nearest(position, n, mode), 0);
    }
    let sample = tc_bilinear(position, n, mode);
    let p00 = textureLoad(input, sample.a, 0);
    let p10 = textureLoad(input, vec2i(sample.b.x, sample.a.y), 0);
    let p01 = textureLoad(input, vec2i(sample.a.x, sample.b.y), 0);
    let p11 = textureLoad(input, sample.b, 0);
    let top = p00 + (p10 - p00) * sample.t.x;
    let bottom = p01 + (p11 - p01) * sample.t.x;
    return top + (bottom - top) * sample.t.y;
}

// Read packed grayscale values in the same normalized units as an uploaded float image.
fn tc_sample_steps(input: texture_2d<u32>, uv: vec2f, mode: u32, div: f32) -> f32 {
    let n = vec2i(textureDimensions(input));
    let position = tc_sample_position(uv);
    if ((mode & 1u) == 0u) {
        return f32(textureLoad(input, tc_nearest(position, n, mode), 0).x) / div;
    }
    let sample = tc_bilinear(position, n, mode);
    let p00 = f32(textureLoad(input, sample.a, 0).x) / div;
    let p10 = f32(textureLoad(input, vec2i(sample.b.x, sample.a.y), 0).x) / div;
    let p01 = f32(textureLoad(input, vec2i(sample.a.x, sample.b.y), 0).x) / div;
    let p11 = f32(textureLoad(input, sample.b, 0).x) / div;
    let top = p00 + (p10 - p00) * sample.t.x;
    let bottom = p01 + (p11 - p01) * sample.t.x;
    return top + (bottom - top) * sample.t.y;
}

fn tc_lum(p: vec4f) -> f32 {
    return 0.2126 * p.x + 0.7152 * p.y + 0.0722 * p.z;
}

fn tc_div_large(p: f32, q: f32) -> f32 {
    if (q != 0.0) { return p / q; }
    return sign(p) * 1e30;
}

fn tc_turn(x: f32) -> f32 {
    let inv_tau = 0.15915494;
    let near = x * inv_tau;
    let residual = fma(x, inv_tau, -near);
    let rounded_away = (near > 0.0 && residual < 0.0) || (near < 0.0 && residual > 0.0);
    let t = select(near, bitcast<f32>(bitcast<u32>(near) - 1u), rounded_away);
    return (t - floor(t)) * 6.2831855;
}

fn tc_sin_turns(x: f32) -> f32 { return sin(tc_turn(x)); }
fn tc_cos_turns(x: f32) -> f32 { return cos(tc_turn(x)); }

fn tc_sin_poly(x: f32) -> f32 {
    let turns = x / 6.2831855;
    let u = x - sign(turns) * floor(abs(turns) + 0.5) * 6.2831855;
    let v = select(abs(u), 3.1415927 - abs(u), abs(u) > 1.5707964);
    let q = v * v;
    return sign(u) * v * (1.0 + q * (-1.0 / 6.0 + q * (1.0 / 120.0 + q * -0.000191765)));
}

fn tc_cos_poly(x: f32) -> f32 { return tc_sin_poly(1.5707964 - x); }

fn tc_atan2(y: f32, x: f32) -> f32 {
    let ax = abs(x);
    let ay = abs(y);
    if (max(ax, ay) == 0.0) { return 0.0; }
    let t = min(ax, ay) / max(ax, ay);
    let p = t * (1.0024898 + t * (-0.0123815 + t * (-0.34731436 + t * 0.14260462)));
    let r = select(p, 1.5707964 - p, ay > ax);
    let angle = select(r, 3.1415927 - r, x < 0.0);
    return select(angle, -angle, y < 0.0);
}

fn tc_smooth(lo: f32, hi: f32, x: f32) -> f32 {
    let t = select(saturate((x - lo) / (hi - lo)), step(lo, x), abs(hi - lo) < 1e-12);
    return t * t * (3.0 - 2.0 * t);
}

fn tc_pow(p: f32, q: f32) -> f32 {
    if (p >= 0.0) { return pow(p, q); }
    if (fract(q) != 0.0) { return 0.0; }
    return select(pow(-p, q), -pow(-p, q), q % 2.0 != 0.0);
}
