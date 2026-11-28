// Identity transform of a power-of-two, opaque grayscale image. The input reader
// is supplied by the pipeline for either a packed image or a float texture.
@group(0) @binding(0) var<uniform> dims: vec4u;
@group(0) @binding(2) var output: texture_storage_2d<r32float, write>;

fn truncate_half(v: f32) -> f32 {
    if v < 0.00006103515625 {
        return trunc(v * 16777216.0) * 0.000000059604644775390625;
    }
    return bitcast<f32>(bitcast<u32>(v) & ~0x1fffu);
}

fn reduce(position: vec2u) -> f32 {
    let p = position * 2u;
    var value = read_pixel(p);
    value += read_pixel(p + vec2u(1u, 0u));
    value += read_pixel(p + vec2u(0u, 1u));
    value += read_pixel(p + vec2u(1u, 1u));
    return decode(rounded_read(value * 0.25));
}

// The reference rounds each f32 product before adding. Observing the product's
// bits to canonicalize zero prevents a driver from contracting the operations.
fn product(a: f32, b: f32) -> f32 {
    let bits = bitcast<u32>(a * b);
    return bitcast<f32>(select(bits, 0u, bits == 0x80000000u));
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    return a + product(b - a, t);
}

fn enlarge(position: vec2u) -> f32 {
    let p = position / 2u;
    let dir = select(vec2u(dims.x - 1u), vec2u(1u), (position & vec2u(1u)) != vec2u(0u));
    let other = (p + dir) & vec2u(dims.x - 1u);
    let row0 = product(0.75, read_pixel(p)) + product(0.25, read_pixel(vec2u(other.x, p.y)));
    let row1 = product(0.75, read_pixel(vec2u(p.x, other.y))) + product(0.25, read_pixel(other));
    return product(0.75, row0) + product(0.25, row1);
}

fn sample_stepped(position: vec2u) -> f32 {
    if dims.x == dims.y {
        return read_pixel(position);
    }
    let s = (vec2f(position) + 0.5) * (f32(dims.x) / f32(dims.y)) - 0.5;
    let base = floor(s);
    let f = floor((s - base) * 256.0 + 0.5) / 256.0;
    let mask = vec2u(dims.x - 1u);
    let p = bitcast<vec2u>(vec2i(base));
    let a = read_pixel(p & mask);
    let b = read_pixel((p + vec2u(1u, 0u)) & mask);
    let c = read_pixel((p + vec2u(0u, 1u)) & mask);
    let d = read_pixel((p + vec2u(1u, 1u)) & mask);
    return lerp(lerp(a, b, f.x), lerp(c, d, f.x), f.y);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    if gid.x >= dims.y || gid.y >= dims.y {
        return;
    }
    var value: f32;
    switch dims.z {
        case 0u: { value = reduce(gid.xy); }
        case 1u: { value = enlarge(gid.xy); }
        case 2u: { value = truncate_half(enlarge(gid.xy)); }
        default: { value = truncate_half(sample_stepped(gid.xy)); }
    }
    textureStore(output, vec2i(gid.xy), vec4f(value, 0.0, 0.0, 0.0));
}
