// Per-channel lookup tables on straight colour, re-premultiplied: the integer rule
// `cpu::lut_apply` follows, so both give the same bytes.

struct Params {
    width: u32,
    height: u32,
    radius: u32,
    extra: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
// 1024 entries: red, green, blue, then alpha, 256 each.
@group(0) @binding(1) var<storage, read> tables: array<u32>;
@group(0) @binding(2) var<storage, read> src: array<u32>;
@group(0) @binding(3) var<storage, read_write> dst: array<u32>;

@compute @workgroup_size(16, 16)
fn lut(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.width || id.y >= p.height) {
        return;
    }
    let i = id.y * p.width + id.x;
    let v = src[i];
    let a = v >> 24u;
    let a2 = tables[768u + a];
    var out = a2 << 24u;
    for (var ch: u32 = 0u; ch < 3u; ch = ch + 1u) {
        let c = min((v >> (8u * ch)) & 255u, a);
        var s = 0u;
        if (a > 0u) {
            s = (c * 255u + a / 2u) / a;
        }
        let o = (tables[ch * 256u + s] * a2 + 127u) / 255u;
        out = out | (o << (8u * ch));
    }
    dst[i] = out;
}
