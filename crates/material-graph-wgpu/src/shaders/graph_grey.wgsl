struct Params {
    dimensions: vec4<u32>, // output width, band rows, input width, opcode (0 enlarges, 1 reduces)
    blend: vec4<u32>,      // opacity bits, whole-image input, has mask, resize input to 16-bit scale
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> front: array<u32>;
@group(0) @binding(2) var<storage, read> back: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<u32>;
@group(0) @binding(4) var<storage, read> mask: array<u32>;

fn even(word: u32) -> f32 {
    return decode(word & 0xffffu);
}

fn odd(word: u32) -> f32 {
    return decode(word >> 16u);
}

fn source(x: u32, y: u32) -> f32 {
    let word = front[(y * params.dimensions.z + x) / 2u];
    let value = (word >> ((x & 1u) * 16u)) & 0xffffu;
    return decode(value * params.blend.w);
}

fn doubled(x: u32, y: u32) -> vec2<f32> {
    let w = params.dimensions.z;
    let whole_image = params.blend.y != 0u;
    let main = y / 2u + select(1u, 0u, whole_image);
    let side_offset = select(w - 1u, 1u, (y & 1u) != 0u);
    var side = (main + side_offset) & (w - 1u);
    if !whole_image {
        side = select(main - 1u, main + 1u, (y & 1u) != 0u);
    }
    let left = (x + w - 1u) & (w - 1u);
    let right = (x + 1u) & (w - 1u);
    let main_sample = vec2(source(left, main), source(right, main));
    let side_sample = vec2(source(left, side), source(right, side));
    // Round products before the additions, as the CPU does while doubling columns and rows.
    let near = vec2f(0.75);
    let far = vec2f(0.25);
    let main_row = individual_product(vec2f(source(x, main)), near) + individual_product(main_sample, far);
    let side_row = individual_product(vec2f(source(x, side)), near) + individual_product(side_sample, far);
    return individual_product(main_row, near) + individual_product(side_row, far);
}

fn halved(x: u32, y: u32) -> vec2<f32> {
    let stride = params.dimensions.z / 2u;
    let i = 2u * (y * stride + x);
    let top = vec2(front[i], front[i + 1u]);
    let bottom = vec2(front[i + stride], front[i + stride + 1u]);
    // Accumulate in CPU row order. A packed integer average changes ties because the CPU sums decoded f32 values.
    let scale = params.blend.w;
    let tl = (top & vec2u(0xffffu)) * scale;
    let tr = (top >> vec2u(16u)) * scale;
    let bl = (bottom & vec2u(0xffffu)) * scale;
    let br = (bottom >> vec2u(16u)) * scale;
    var sum = vec2(decode(tl.x), decode(tl.y));
    sum += vec2(decode(tr.x), decode(tr.y));
    sum += vec2(decode(bl.x), decode(bl.y));
    sum += vec2(decode(br.x), decode(br.y));
    return sum * 0.25;
}

fn mask_weight(i: u32) -> vec2f {
    let word = mask[i];
    let v = vec2(even(word), odd(word));
    // Match the reference luminance of an RGB sample with three equal channels.
    return individual_product(v, vec2f(0.2126)) + individual_product(v, vec2f(0.7152)) + individual_product(v, vec2f(0.0722));
}

fn blended(i: u32) -> vec2<f32> {
    let a = front[i];
    let b = back[i];
    let f = vec2(even(a), odd(a));
    let bg = vec2(even(b), odd(b));
    var opacity = vec2f(bitcast<f32>(params.blend.x));
    if params.blend.z != 0u {
        opacity = individual_product(mask_weight(i), opacity);
    }
    let mode = params.dimensions.w - 3u; // Blend opcodes follow enlarge, reduce, and levels.
    return blend_channels(mode, f, bg, opacity);
}

fn lookup_level(sample: u32) -> u32 {
    return (back[sample / 2u] >> ((sample & 1u) * 16u)) & 0xffffu;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let width = params.dimensions.x / 2u;
    if gid.x >= width || gid.y >= params.dimensions.y {
        return;
    }
    let i = gid.y * width + gid.x;
    var words: vec2<u32>;
    if params.dimensions.w <= 1u {
        var value: vec2<f32>;
        if params.dimensions.w == 0u {
            value = doubled(gid.x, gid.y);
        } else {
            value = halved(gid.x, gid.y);
        }
        words = vec2(rounded_read(value.x), rounded_read(value.y));
    } else if params.dimensions.w == 2u {
        let packed = front[i];
        words = vec2(lookup_level(packed & 0xffffu), lookup_level(packed >> 16u));
    } else {
        words = compiled_16bit(blended(i));
    }
    output[i] = words.x | (words.y << 16u);
}
