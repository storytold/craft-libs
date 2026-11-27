// Separable Gaussian blur of premultiplied RGBA8, the rule `cpu::gaussian_blur` follows.
// `blur_h` reads the RGBA8 source and writes 16-bit channels to `mid`; `blur_v` reads `mid` and
// writes RGBA8 to `dst`. Pixels outside the image are transparent black.

struct Params {
    width: u32,
    height: u32,
    radius: u32,
    extra: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> weights: array<f32>;
@group(0) @binding(2) var<storage, read> src: array<u32>;
@group(0) @binding(3) var<storage, read_write> mid: array<u32>;
@group(0) @binding(4) var<storage, read_write> dst: array<u32>;

// A source pixel in [0, 1], each colour channel held to its alpha.
fn load_src(i: u32) -> vec4<f32> {
    let v = src[i];
    let a = v >> 24u;
    let c = min(vec3<u32>(v & 255u, (v >> 8u) & 255u, (v >> 16u) & 255u), vec3<u32>(a));
    return vec4<f32>(vec4<u32>(c, a)) / 255.0;
}

fn load_mid(i: u32) -> vec4<f32> {
    return vec4<f32>(unpack2x16unorm(mid[2u * i]), unpack2x16unorm(mid[2u * i + 1u]));
}

@compute @workgroup_size(16, 16)
fn blur_h(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.width || id.y >= p.height) {
        return;
    }
    let row = id.y * p.width;
    var acc = weights[0] * load_src(row + id.x);
    for (var i: u32 = 1u; i <= p.radius; i = i + 1u) {
        var pair = vec4<f32>(0.0);
        if (id.x >= i) {
            pair = pair + load_src(row + id.x - i);
        }
        if (id.x + i < p.width) {
            pair = pair + load_src(row + id.x + i);
        }
        acc = acc + weights[i] * pair;
    }
    let q = clamp(acc, vec4<f32>(0.0), vec4<f32>(1.0));
    let o = row + id.x;
    mid[2u * o] = pack2x16unorm(q.xy);
    mid[2u * o + 1u] = pack2x16unorm(q.zw);
}

@compute @workgroup_size(16, 16)
fn blur_v(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.width || id.y >= p.height) {
        return;
    }
    let w = p.width;
    var acc = weights[0] * load_mid(id.y * w + id.x);
    for (var i: u32 = 1u; i <= p.radius; i = i + 1u) {
        var pair = vec4<f32>(0.0);
        if (id.y >= i) {
            pair = pair + load_mid((id.y - i) * w + id.x);
        }
        if (id.y + i < p.height) {
            pair = pair + load_mid((id.y + i) * w + id.x);
        }
        acc = acc + weights[i] * pair;
    }
    // Round to 8 bits, then hold each colour channel to the alpha.
    let q = vec4<u32>(floor(vec4<f32>(0.5) + 255.0 * clamp(acc, vec4<f32>(0.0), vec4<f32>(1.0))));
    let c = min(q.xyz, vec3<u32>(q.w));
    dst[id.y * w + id.x] = c.x | (c.y << 8u) | (c.z << 16u) | (q.w << 24u);
}
