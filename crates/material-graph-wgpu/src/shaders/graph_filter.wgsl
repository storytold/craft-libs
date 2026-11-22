struct Params {
    source: vec4<u32>,  // width, height, grayscale divisor
    output: vec4<u32>,
    map: vec4<u32>,
    control: vec4<u32>, // filter, major axis, wrap x, wrap y
    values: vec4<f32>,
    direction: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> front: array<u32>;
@group(0) @binding(2) var<storage, read> auxiliary: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<u32>;

fn unpack_grey(word: u32, i: u32) -> u32 {
    return (word >> ((i & 1u) * 16u)) & 0xffffu;
}

fn decode_grey(value: u32, div: u32) -> f32 {
    if div == 255u {
        return decode(value * 257u);
    }
    return decode(value);
}

fn front_at(p: vec2<i32>) -> f32 {
    let i = address(p, params.source.xy, params.control.zw != vec2<u32>(0u));
    return decode_grey(unpack_grey(front[i / 2u], i), params.source.z);
}

fn front_units(p: vec2<i32>) -> u32 {
    let i = address(p, params.source.xy, params.control.zw != vec2<u32>(0u));
    return unpack_grey(front[i / 2u], i) * select(1u, 257u, params.source.z == 255u);
}

fn map_at(p: vec2<i32>, wraps: vec2<bool>) -> f32 {
    let i = address(p, params.map.xy, wraps);
    return decode_grey(unpack_grey(auxiliary[i / 2u], i), params.map.z);
}

fn lerp(lo: f32, hi: f32, weight: f32) -> f32 {
    return lo + (hi - lo) * weight;
}

fn affine_lerp(lo: f32, hi: f32, weight: f32) -> f32 {
    // Keep the reference's separate product rounding at the 16-bit read boundaries.
    let product = bitcast<u32>((hi - lo) * weight);
    return lo + bitcast<f32>(select(product, 0u, product == 0x80000000u));
}

fn affine_uv(uv: vec2<f32>) -> vec2<f32> {
    let center = uv - 0.5;
    // Retain the reference arithmetic's intermediate rounding at nearest and bilinear sampling boundaries.
    let horizontal = bitcast<vec2<u32>>(params.direction.xy * center.x);
    let vertical = bitcast<vec2<u32>>(params.direction.zw * center.y);
    let shifted = bitcast<vec2<f32>>(select(horizontal, vec2<u32>(0u), horizontal == vec2<u32>(0x80000000u)))
        + bitcast<vec2<f32>>(select(vertical, vec2<u32>(0u), vertical == vec2<u32>(0x80000000u)));
    return shifted + 0.5 + params.values.xy;
}

fn luminance(v: f32) -> f32 {
    return (v * 0.2126 + v * 0.7152) + v * 0.0722;
}

fn map_luminance(uv: vec2<f32>) -> f32 {
    // A compiled warp samples a differently sized map with clamped edges.
    // At the output pixel centers, this is equivalent to resampling it first.
    let size = vec2<f32>(params.map.xy);
    let half_texel = 0.5 / size;
    let xy = clamp(uv, half_texel, 1.0 - half_texel) * size - 0.5;
    let base = vec2<i32>(floor(xy));
    let weight = fract(xy);
    let wraps = vec2<bool>(true);
    let top = lerp(map_at(base, wraps), map_at(base + vec2<i32>(1, 0), wraps), weight.x);
    let bottom = lerp(map_at(base + vec2<i32>(0, 1), wraps), map_at(base + vec2<i32>(1, 1), wraps), weight.x);
    return luminance(lerp(top, bottom, weight.y));
}

fn map_slope(p: vec2<i32>) -> vec2<f32> {
    let wraps = params.control.zw != vec2<u32>(0u);
    let center = luminance(map_at(p, wraps));
    let left = luminance(map_at(p - vec2<i32>(1, 0), wraps));
    let above = luminance(map_at(p - vec2<i32>(0, 1), wraps));
    return vec2<f32>(center - left, center - above) * vec2<f32>(params.map.xy);
}

fn lerp_slope(lo: vec2<f32>, hi: vec2<f32>, weight: f32) -> vec2<f32> {
    return lo + (hi - lo) * weight;
}

fn sampled_slope(uv: vec2<f32>) -> vec2<f32> {
    let xy = uv * vec2<f32>(params.map.xy) - vec2<f32>(0.5);
    let mask = vec2<i32>(params.map.xy) - vec2<i32>(1);
    let base = vec2<i32>(floor(xy)) & mask;
    let weight = fract(xy);
    if all(weight == vec2<f32>(0.0)) {
        return map_slope(base);
    }
    // The slope field's bilinear read wraps even when computing its differences clamps an edge
    let top = lerp_slope(map_slope(base), map_slope((base + vec2<i32>(1, 0)) & mask), weight.x);
    let bottom = lerp_slope(map_slope((base + vec2<i32>(0, 1)) & mask), map_slope((base + vec2<i32>(1, 1)) & mask), weight.x);
    return lerp_slope(top, bottom, weight.y);
}

fn stepped_front(uv: vec2<f32>) -> f32 {
    let taps = stepped_taps(uv, params.source.xy, params.control.zw != vec2u(0u));
    let base = taps.base;
    let weights = taps.weights;
    let a = front_at(base);
    let b = front_at(base + vec2<i32>(1, 0));
    let c = front_at(base + vec2<i32>(0, 1));
    let d = front_at(base + vec2<i32>(1, 1));
    if params.control.x == 6u {
        return affine_lerp(affine_lerp(a, b, weights.x), affine_lerp(c, d, weights.x), weights.y);
    }
    return lerp(lerp(a, b, weights.x), lerp(c, d, weights.x), weights.y);
}

fn affine_front(uv: vec2<f32>) -> f32 {
    let wraps = params.control.zw != vec2u(0u);
    if params.control.y != 0u {
        let value = stepped_front(uv);
        if all(wraps) {
            return value;
        }
        let fade = clamp(min(uv, 1.0 - uv) * params.values.zw + 0.5, vec2f(0.0), vec2f(1.0));
        let coverage = select(fade, vec2f(1.0), wraps);
        return value * (coverage.x * coverage.y);
    }
    let p = vec2i(floor(uv * vec2f(params.source.xy)));
    let outside = (p < vec2i(0)) | (p >= vec2i(params.source.xy));
    if any(outside & !wraps) {
        return 0.0;
    }
    return front_at(p);
}

fn box_edge(p: vec2<i32>, axis: vec2<i32>, inner: i32, outer: i32) -> f32 {
    let far = u32(params.values.y);
    return f32((((256u - far) * front_units(p + inner * axis) + far * front_units(p + outer * axis)) + 128u) >> 8u);
}

fn box_pair(a: u32, b: u32) -> u32 {
    let sum = a + b;
    if params.output.z == 255u {
        return compiled_8bit_sample(f32(sum) / 131070.0) * 257u;
    }
    return (sum + 1u) / 2u;
}

fn box_small_block(p: vec2<i32>, axis: vec2<i32>, start: i32, width: u32) -> u32 {
    var values: array<u32, 16>;
    for (var i = 0u; i < width; i++) {
        values[i] = front_units(p + (start + i32(i)) * axis);
    }
    for (var count = width >> 1u; count > 0u; count >>= 1u) {
        for (var i = 0u; i < count; i++) {
            values[i] = box_pair(values[2u * i], values[2u * i + 1u]);
        }
    }
    return values[0];
}

fn box_pyramid_value(p: vec2<i32>, axis: vec2<i32>, start: i32, width: u32) -> u32 {
    if width <= 16u {
        return width * box_small_block(p, axis, start, width);
    }
    let left = box_pair(box_small_block(p, axis, start, 16u), box_small_block(p, axis, start + 16, 16u));
    if width == 32u {
        return width * left;
    }
    let right = box_pair(box_small_block(p, axis, start + 32, 16u), box_small_block(p, axis, start + 48, 16u));
    return width * box_pair(left, right);
}

fn box_pyramid_sum(p: vec2<i32>, axis: vec2<i32>, start: i32, end: i32, side: i32) -> f32 {
    // Cover the interior with the largest aligned blocks available at each position.
    let origin = p.x * axis.x + p.y * axis.y;
    let top = 1u << min(6u, firstLeadingBit(u32(2 * side - 1)));
    var sum = 0u;
    var offset = start;
    while offset < end {
        let remaining = u32(end - offset);
        var width = min(top, 1u << firstLeadingBit(remaining));
        while ((origin + offset) & i32(width - 1u)) != 0 {
            width >>= 1u;
        }
        sum += box_pyramid_value(p, axis, offset, width);
        offset += i32(width);
    }
    return f32(sum);
}

fn box_blur(p: vec2<i32>) -> f32 {
    let radius = params.values.x;
    if radius <= 0.5 {
        return front_at(p);
    }
    let axis = select(vec2<i32>(1, 0), vec2<i32>(0, 1), params.control.y != 0u);
    let side = i32(ceil(radius - 0.5));
    let edge = radius - (f32(side) - 0.5);
    let wrap = select(params.control.z, params.control.w, params.control.y != 0u);
    let row_size = select(params.source.x, params.source.y, params.control.y != 0u);
    let denominator = 2.0 * radius * 65535.0;
    let outside = f32(front_units(p - side * axis) + front_units(p + side * axis));
    if wrap == 0u || u32(2 * side + 1) >= row_size || side == 1 {
        var inner = 0u;
        for (var step = 1 - side; step < side; step++) {
            inner += front_units(p + step * axis);
        }
        return (f32(inner) + edge * outside) / denominator;
    }
    if side == 2 {
        let ends = box_edge(p, axis, -1, -2) + box_edge(p, axis, 1, 2);
        return (f32(front_units(p)) + (1.0 + edge) * ends) / denominator;
    }
    let start_is_odd = ((p.x * axis.x + p.y * axis.y - side) & 1) == 1;
    let lone = select(side, -side, start_is_odd);
    let mixed = -lone;
    let neighbor = mixed - sign(mixed);
    let interior = box_pyramid_sum(p, axis, -side + select(2, 1, start_is_odd), side - select(0, 1, start_is_odd), side);
    return (edge * f32(front_units(p + lone * axis)) + (1.0 + edge) * box_edge(p, axis, neighbor, mixed) + interior) / denominator;
}

fn minor_read(p: vec2<i32>, minor: vec2<i32>, offset: f32) -> f32 {
    let base = floor(offset);
    let fractional = offset - base;
    let weight = floor(fractional * 256.0 + 0.5) / 256.0;
    let at = p + i32(base) * minor;
    return lerp(front_at(at), front_at(at + minor), weight);
}

fn short_line(p: vec2<i32>) -> f32 {
    let major = select(vec2<i32>(1, 0), vec2<i32>(0, 1), params.control.y != 0u);
    let minor = major.yx;
    let radius = params.values.x;
    let slope = params.values.y;
    let a = minor_read(p - major, minor, -slope);
    let b = minor_read(p + major, minor, slope);
    return (front_at(p) + (radius - 0.5) * (a + b)) / (2.0 * radius);
}

fn shift_word(column: i32, reverse: bool) -> i32 {
    let index = u32(column + i32(params.values.y)) * 2u + select(0u, 1u, reverse);
    return bitcast<i32>(auxiliary[index]);
}

fn shift_weight(word: i32) -> f32 {
    return f32(word & 511) / 256.0;
}

fn sheared_read(column: i32, row: i32, major: vec2<i32>) -> f32 {
    let width = i32(select(params.source.x, params.source.y, major.y != 0));
    let wrap = select(params.control.z, params.control.w, major.y != 0) != 0u;
    // Clamping the sheared image repeats the shift of its edge column. Wrapping continues that shift across the seam.
    let x = select(clamp(column, 0, width - 1), column, wrap);
    let shift = shift_word(x, false);
    let minor = major.yx;
    let p = x * major + (row + (shift >> 9u)) * minor;
    return lerp(front_at(p), front_at(p + minor), shift_weight(shift));
}

fn sheared_box(column: i32, row: i32, major: vec2<i32>) -> f32 {
    let radius = params.values.x;
    let side = i32(params.values.y);
    let edge = radius - (f32(side) - 0.5);
    var sum = 0.0;
    for (var step = 1 - side; step < side; step++) {
        sum += sheared_read(column + step, row, major);
    }
    let outer = sheared_read(column - side, row, major) + sheared_read(column + side, row, major);
    return (sum + edge * outer) / (2.0 * radius);
}

fn sheared_line(p: vec2<i32>) -> f32 {
    let major = select(vec2<i32>(1, 0), vec2<i32>(0, 1), params.control.y != 0u);
    let column = select(p.x, p.y, major.y != 0);
    let shift = shift_word(column, true);
    let row = select(p.y, p.x, major.y != 0) + (shift >> 9u);
    let a = sheared_box(column, row, major);
    let weight = shift_weight(shift);
    if weight == 0.0 {
        return a;
    }
    return lerp(a, sheared_box(column, row + 1, major), weight);
}

fn sharpen(p: vec2<i32>) -> f32 {
    let c = front_at(p);
    let l = front_at(p - vec2<i32>(1, 0));
    let r = front_at(p + vec2<i32>(1, 0));
    let t = front_at(p - vec2<i32>(0, 1));
    let b = front_at(p + vec2<i32>(0, 1));
    return clamp((c + params.values.x * ((2.0 * c - l) - r)) + params.values.y * ((2.0 * c - t) - b), 0.0, 1.0);
}

fn value_at(p: vec2<i32>) -> f32 {
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(params.output.xy);
    switch params.control.x {
        case 0u: {
            let distance = params.values.x * (map_luminance(uv) - params.values.y);
            return stepped_front(uv + (distance * params.direction.xy) * params.values.zw);
        }
        case 1u: { return stepped_front(uv + params.values.xy * sampled_slope(uv)); }
        case 2u: { return box_blur(p); }
        case 3u: { return short_line(p); }
        case 5u: { return sheared_line(p); }
        case 6u: { return affine_front(affine_uv(uv)); }
        default: { return sharpen(p); }
    }
}

fn compiled_8bit_sample(value: f32) -> u32 {
    return (255u * (rounded_read(value) >> 4u) + 2047u) >> 12u;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let width = params.output.x / 2u;
    if gid.x >= width || gid.y >= params.output.y {
        return;
    }
    let p = vec2<i32>(i32(gid.x * 2u), i32(gid.y));
    let values = vec2<f32>(value_at(p), value_at(p + vec2<i32>(1, 0)));
    var words: vec2<u32>;
    if params.output.z == 255u {
        words = vec2<u32>(compiled_8bit_sample(values.x), compiled_8bit_sample(values.y));
    } else if params.control.x < 2u || params.control.x == 6u {
        // Warps and translations round reads to 16 bits before applying the node's compiled format.
        words = vec2<u32>(rounded_read(values.x), rounded_read(values.y));
    } else {
        words = compiled_16bit(values);
    }
    output[gid.y * width + gid.x] = words.x | (words.y << 16u);
}
