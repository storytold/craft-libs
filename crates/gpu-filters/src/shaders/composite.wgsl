// Premultiplied source-over of `layer` onto `backdrop`, the layer scaled by the opacity weight
// `p.extra` (0 to 255): the integer rule `cpu::composite` follows, so both give the same bytes.

struct Params {
    width: u32,
    height: u32,
    radius: u32,
    extra: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> backdrop: array<u32>;
@group(0) @binding(2) var<storage, read> layer: array<u32>;
@group(0) @binding(3) var<storage, read_write> dst: array<u32>;

// A pixel's channels, each colour channel held to its alpha.
fn channels(v: u32) -> vec4<u32> {
    let a = v >> 24u;
    let c = min(vec3<u32>(v & 255u, (v >> 8u) & 255u, (v >> 16u) & 255u), vec3<u32>(a));
    return vec4<u32>(c, a);
}

@compute @workgroup_size(16, 16)
fn over(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.width || id.y >= p.height) {
        return;
    }
    let i = id.y * p.width + id.x;
    let s = (channels(layer[i]) * p.extra + vec4<u32>(127u)) / 255u;
    let d = channels(backdrop[i]);
    let o = s + (d * (255u - s.w) + vec4<u32>(127u)) / 255u;
    dst[i] = o.x | (o.y << 8u) | (o.z << 16u) | (o.w << 24u);
}
