struct Params {
    image: vec4<u32>, // width, height, first output row, first bin row
    mode: vec4<u32>,  // rows in this band, color and grayscale flags, custom frame, background image
    region: vec4<f32>,
    period: vec4<f32>,
    clear: vec4<f32>,
}

struct Stamp {
    bounds: vec4<i32>,
    repeats: vec4<i32>,
    placement: vec4<f32>, // center and signed half-size
    pattern: vec4<u32>,   // sine, cosine, variation, flags
    brightness: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> stamps: array<Stamp>;
@group(0) @binding(2) var<storage, read> bins: array<u32>;
@group(0) @binding(3) var<storage, read> images: array<u32>;
@group(0) @binding(4) var<storage, read_write> output: array<f32>;

fn smooth_shape(value: f32) -> f32 {
    let t = clamp(value, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn falloff(value: f32, span: f32) -> f32 {
    if span <= 0.0 {
        return 0.0;
    }
    return 1.0 - smooth_shape(value / span);
}

fn squared(v: f32) -> f32 {
    return v * v;
}

fn shape(kind: u32, pos: vec2<f32>, variation: f32) -> f32 {
    let v = clamp(variation, 0.0, 1.0);
    let r2 = pos.x * pos.x + pos.y * pos.y;
    let r = sqrt(r2);
    let inside = select(0.0, 1.0, r <= 1.0);
    let paraboloid = max(1.0 - r2, 0.0);
    let bell = squared(squared(paraboloid));
    switch kind {
        case 0u: { return 1.0; }
        case 2u: { return paraboloid; }
        case 3u: { return bell; }
        case 4u: { return max(exp(-11.1 * r2) - exp(-11.1), 0.0) * inside; }
        case 5u: { return squared(squared(max(1.0 - r, 0.0))); }
        case 6u: { return 1.0 - max(abs(pos.x), abs(pos.y)); }
        case 7u: {
            if v <= 0.0 {
                return 1.0;
            }
            return smooth_shape((1.0 - abs(pos.x)) / v) * smooth_shape((1.0 - abs(pos.y)) / v);
        }
        case 8u: { return (pos.x + 1.0) * 0.5 * (1.0 - v); }
        case 9u: { return paraboloid * squared(cos((1.5 + 6.0 * variation) * 3.141592653589793 * pos.x)); }
        case 10u: { return bell * select(1.0, falloff(pos.x, 1.0 - v), pos.x > 0.0); }
        case 11u: { return bell * squared(1.0 - abs(pos.x)); }
        case 12u: { return 6.75 * r2 * paraboloid * paraboloid * falloff(pos.x + 1.0, 2.0 * (1.0 - v)); }
        case 13u: {
            let d = max(abs(pos.x) - v * 0.7698, 0.0);
            return max(1.0 - (d * d + pos.y * pos.y) / squared(0.2236068), 0.0);
        }
        case 14u: { return max(1.0 - r, 0.0); }
        case 15u: { return min((pos.x + 1.0) * 0.5 + v, 1.0); }
        case 16u: { return min(sqrt(paraboloid) + v, 1.0) * inside; }
        default: { return inside; }
    }
}

fn pos_mod(value: i32, size: i32) -> i32 {
    return (value % size + size) % size;
}

fn image_desc(index: u32) -> vec4<u32> {
    let i = index * 4u;
    return vec4(images[i], images[i + 1u], images[i + 2u], images[i + 3u]);
}

fn image_pixel(desc: vec4<u32>, pos: vec2<i32>, wrap: bool) -> vec4<f32> {
    let size = vec2<i32>(desc.yz);
    var xy = pos;
    if wrap {
        xy = vec2(pos_mod(pos.x, size.x), pos_mod(pos.y, size.y));
    } else if any(pos < vec2(0)) || any(pos >= size) {
        return vec4(0.0);
    }
    let i = desc.x + (u32(xy.y) * desc.y + u32(xy.x)) * desc.w;
    let red = bitcast<f32>(images[i]);
    if desc.w == 1u {
        return vec4(red, red, red, 1.0);
    }
    return vec4(red, bitcast<f32>(images[i + 1u]), bitcast<f32>(images[i + 2u]), bitcast<f32>(images[i + 3u]));
}

fn sample_stepped(desc: vec4<u32>, uv: vec2<f32>, wrap: bool) -> vec4<f32> {
    let coord = uv * vec2<f32>(desc.yz) - 0.5;
    let lower = floor(coord);
    let top_left = vec2<i32>(lower);
    let frac = floor((coord - lower) * 256.0 + 0.5) / 256.0;
    let a = image_pixel(desc, top_left, wrap);
    let b = image_pixel(desc, top_left + vec2(1, 0), wrap);
    let c = image_pixel(desc, top_left + vec2(0, 1), wrap);
    let d = image_pixel(desc, top_left + vec2(1, 1), wrap);
    let top = a + (b - a) * frac.x;
    let bottom = c + (d - c) * frac.x;
    return top + (bottom - top) * frac.y;
}

fn sample_stamp(p: vec4<f32>, stamp: Stamp, uv: vec2<f32>) -> vec4<f32> {
    let delta = uv - stamp.placement.xy;
    let sn = bitcast<f32>(stamp.pattern.x);
    let cs = bitcast<f32>(stamp.pattern.y);
    let local = vec2(delta.x * cs - delta.y * sn, delta.x * sn + delta.y * cs) / stamp.placement.zw;
    if any(local <= vec2(-1.0)) || any(local > vec2(1.0)) {
        return p;
    }
    let flags = stamp.pattern.w;
    let image = (flags >> 8u) & 0x003fffffu;
    let color = (params.mode.y & 1u) != 0u;
    let keep_larger = (flags & 0x80000000u) != 0u;
    var value: vec4<f32>;
    if image != 0u {
        let desc = image_desc(image);
        let read_uv = (local + 1.0) * 0.5;
        if (flags & 0x40000000u) != 0u {
            let xy = min(vec2<u32>(read_uv * vec2<f32>(desc.yz)), desc.yz - 1u);
            value = image_pixel(desc, vec2<i32>(xy), false);
        } else {
            value = sample_stepped(desc, read_uv, false);
        }
    } else {
        let a = shape(flags & 0xffu, local, bitcast<f32>(stamp.pattern.z));
        value = select(vec4(a), vec4(1.0, 1.0, 1.0, a), color);
    }
    let brightness = stamp.brightness;
    if color {
        let rgb = value.rgb * brightness.rgb;
        let alpha = clamp(value.a * brightness.a, 0.0, 1.0);
        if keep_larger {
            return vec4(p.rgb + (rgb - p.rgb) * alpha, p.a + (1.0 - p.a) * alpha);
        }
        return clamp(p + vec4(rgb * alpha, alpha), vec4(0.0), vec4(1.0));
    }
    let grey = value.r * brightness.r;
    return vec4(select(p.rgb + grey, max(p.rgb, vec3(grey)), keep_larger), p.a);
}

fn stamp_at(p: vec4<f32>, stamp: Stamp, pixel: vec2<i32>, size: vec2<i32>) -> vec4<f32> {
    var result = p;
    if params.mode.z == 0u {
        let start = stamp.bounds.xy + vec2(pos_mod(pixel.x - stamp.bounds.x, size.x), pos_mod(pixel.y - stamp.bounds.y, size.y));
        for (var y = start.y; y <= stamp.bounds.w; y += size.y) {
            for (var x = start.x; x <= stamp.bounds.z; x += size.x) {
                let uv = (vec2<f32>(f32(x), f32(y)) + 0.5) / vec2<f32>(size);
                result = sample_stamp(result, stamp, uv);
            }
        }
    } else if all(pixel >= stamp.bounds.xy) && all(pixel <= stamp.bounds.zw) {
        let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(size);
        let span = params.region.yw - params.region.xz;
        let frame_uv = select(uv, params.region.xz + uv * span, span > vec2(0.0));
        for (var y = stamp.repeats.y; y <= stamp.repeats.w; y++) {
            for (var x = stamp.repeats.x; x <= stamp.repeats.z; x++) {
                result = sample_stamp(result, stamp, frame_uv + vec2<f32>(f32(x), f32(y)) * params.period.xy);
            }
        }
    }
    return result;
}

@compute @workgroup_size(8, 8)
fn raster(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = params.image.xy;
    if gid.x >= size.x || gid.y >= params.mode.x {
        return;
    }
    let pixel = vec2(gid.x, gid.y + params.image.z);
    var p = params.clear;
    if params.mode.w != 0u {
        let bg = image_desc(0u);
        if all(bg.yz == size) {
            p = image_pixel(bg, vec2<i32>(pixel), false);
        } else {
            p = sample_stepped(bg, (vec2<f32>(pixel) + 0.5) / vec2<f32>(size), true);
        }
    }
    let columns = (size.x + 7u) / 8u;
    let bin = (pixel.y / 8u - params.image.w) * columns + pixel.x / 8u;
    for (var i = bins[bin]; i < bins[bin + 1u]; i++) {
        p = stamp_at(p, stamps[bins[i]], vec2<i32>(pixel), vec2<i32>(size));
    }
    let i = gid.y * size.x + gid.x;
    if (params.mode.y & 2u) != 0u {
        output[i] = clamp(p.r, 0.0, 1.0);
    } else {
        output[i * 4u] = clamp(p.r, 0.0, 1.0);
        output[i * 4u + 1u] = clamp(p.g, 0.0, 1.0);
        output[i * 4u + 2u] = clamp(p.b, 0.0, 1.0);
        output[i * 4u + 3u] = p.a;
    }
}
