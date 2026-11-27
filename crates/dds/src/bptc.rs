//! BPTC blocks: BC7 (8-bit RGBA) and BC6H (half-float RGB), written from the Khronos Data Format
//! Specification's "BPTC Compressed Texture Image Formats" section. Each 16-byte block holds 4×4
//! texels, row by row. Tables are the specification's, numbered as it numbers them.

/// A block's bits, read from the lowest up.
struct Bits {
    value: u128,
    pos: u32,
}

impl Bits {
    fn new(block: &[u8; 16]) -> Self {
        Self { value: u128::from_le_bytes(*block), pos: 0 }
    }

    /// The next `n` bits (at most 16), as a number.
    fn take(&mut self, n: u8) -> u32 {
        let n = u32::from(n);
        let mask = !u128::MAX.checked_shl(n).unwrap_or(0);
        let out = self.value.checked_shr(self.pos).unwrap_or(0) & mask;
        self.pos = self.pos.saturating_add(n);
        out as u32
    }
}

/// Interpolation weights by index bits (Table 119).
const WEIGHTS_2: [i32; 4] = [0, 21, 43, 64];
const WEIGHTS_3: [i32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const WEIGHTS_4: [i32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

fn weight(index_bits: u8, index: u32) -> i32 {
    let table: &[i32] = match index_bits {
        2 => &WEIGHTS_2,
        3 => &WEIGHTS_3,
        _ => &WEIGHTS_4,
    };
    table.get(index as usize).copied().unwrap_or(0)
}

/// Equation 2: a value between two endpoints.
fn interpolate(e0: i32, e1: i32, weight: i32) -> i32 {
    ((64 - weight) * e0 + weight * e1 + 32) >> 6
}

/// Which subset texel `i` is in, for a block with `subsets` subsets and partition `partition`.
fn subset(subsets: u8, partition: usize, i: usize) -> usize {
    let part = match subsets {
        2 => u32::from(PARTITIONS_2.get(partition).copied().unwrap_or(0)) >> i & 1,
        3 => PARTITIONS_3.get(partition).copied().unwrap_or(0) >> (2 * i) & 3,
        _ => 0,
    };
    part as usize
}

/// Whether texel `i` is an anchor (its index is stored with one bit fewer).
fn anchor(subsets: u8, partition: usize, i: usize) -> bool {
    let at = |table: &[u8; 64]| table.get(partition).is_some_and(|a| usize::from(*a) == i);
    i == 0
        || match subsets {
            2 => at(&ANCHOR_2),
            3 => at(&ANCHOR_3_SECOND) || at(&ANCHOR_3_THIRD),
            _ => false,
        }
}

/// The 16 indices of a block, each `bits` wide (anchors one fewer).
fn indices(r: &mut Bits, bits: u8, subsets: u8, partition: usize) -> [u32; 16] {
    let mut out = [0; 16];
    for (i, x) in out.iter_mut().enumerate() {
        *x = r.take(bits - u8::from(anchor(subsets, partition, i)));
    }
    out
}

/// BC7's modes (Table 109): subsets, partition bits, rotation bits, index selection bits, colour
/// bits, alpha bits, per-endpoint P-bits, shared P-bits, index bits, secondary index bits.
const BC7_MODES: [[u8; 10]; 8] = [
    [3, 4, 0, 0, 4, 0, 1, 0, 3, 0],
    [2, 6, 0, 0, 6, 0, 0, 1, 3, 0],
    [3, 6, 0, 0, 5, 0, 0, 0, 2, 0],
    [2, 6, 0, 0, 7, 0, 1, 0, 2, 0],
    [1, 0, 2, 1, 5, 6, 0, 0, 2, 3],
    [1, 0, 2, 0, 7, 8, 0, 0, 2, 2],
    [1, 0, 0, 0, 7, 7, 1, 0, 4, 0],
    [2, 6, 0, 0, 5, 5, 1, 0, 2, 0],
];

/// The 16 RGBA texels of a BC7 block.
pub fn bc7(block: &[u8; 16]) -> [[u8; 4]; 16] {
    let mode = block[0].trailing_zeros();
    // A low byte of 0 is reserved: every channel of every texel is 0.
    let Some(&[subsets, partition_bits, rotation_bits, selection_bits, colour_bits, alpha_bits, endpoint_p, shared_p, index_bits, index2_bits]) =
        BC7_MODES.get(mode as usize)
    else {
        return [[0; 4]; 16];
    };
    let mut r = Bits::new(block);
    r.pos = mode + 1;
    let partition = r.take(partition_bits) as usize;
    let rotation = r.take(rotation_bits);
    let swapped = r.take(selection_bits) == 1;
    let ends = usize::from(subsets) * 2;
    // Endpoints: every endpoint's red, then green, blue and alpha; then the P-bits.
    let mut raw = [[0u32; 4]; 6];
    for channel in 0..4 {
        let bits = if channel < 3 { colour_bits } else { alpha_bits };
        for e in raw.iter_mut().take(ends) {
            e[channel] = r.take(bits);
        }
    }
    let mut p = [0u32; 6];
    if endpoint_p == 1 {
        for x in p.iter_mut().take(ends) {
            *x = r.take(1);
        }
    }
    if shared_p == 1 {
        for pair in p.chunks_mut(2).take(usize::from(subsets)) {
            pair.fill(r.take(1));
        }
    }
    let has_p = endpoint_p + shared_p > 0;
    let endpoints: [[i32; 4]; 6] = std::array::from_fn(|e| {
        std::array::from_fn(|channel| {
            let bits = u32::from(if channel < 3 { colour_bits } else { alpha_bits });
            let (v, bits) = match (raw[e][channel], bits) {
                (_, 0) => return 255,
                (v, bits) if has_p => (v << 1 | p[e], bits + 1),
                other => other,
            };
            // The stored bits are a byte's top bits; its top bits repeat into the rest.
            (v << (8 - bits) | v >> (2 * bits - 8)) as i32
        })
    });
    let primary = indices(&mut r, index_bits, subsets, partition);
    let secondary = if index2_bits > 0 { indices(&mut r, index2_bits, 1, 0) } else { [0; 16] };
    let (colour, colour_index_bits) = if swapped { (secondary, index2_bits) } else { (primary, index_bits) };
    let (alpha, alpha_index_bits) = if index2_bits > 0 && !swapped { (secondary, index2_bits) } else { (primary, index_bits) };
    std::array::from_fn(|i| {
        let s = subset(subsets, partition, i);
        let (e0, e1) = (endpoints.get(2 * s).copied().unwrap_or_default(), endpoints.get(2 * s + 1).copied().unwrap_or_default());
        let (wc, wa) = (weight(colour_index_bits, colour[i]), weight(alpha_index_bits, alpha[i]));
        let mut px: [u8; 4] = std::array::from_fn(|c| interpolate(e0[c], e1[c], if c < 3 { wc } else { wa }) as u8);
        if (1..=3).contains(&rotation) {
            px.swap(rotation as usize - 1, 3);
        }
        px
    })
}

// BC6H's fields: endpoint `e`'s red, green and blue are fields `3e`, `3e + 1` and `3e + 2`.
const R0: u8 = 0;
const G0: u8 = 1;
const B0: u8 = 2;
const R1: u8 = 3;
const G1: u8 = 4;
const B1: u8 = 5;
const R2: u8 = 6;
const G2: u8 = 7;
const B2: u8 = 8;
const R3: u8 = 9;
const G3: u8 = 10;
const B3: u8 = 11;
/// The partition number.
const PB: u8 = 12;

/// A BC6H mode (Tables 121 and 122).
struct Bc6hMode {
    id: u8,
    /// Endpoints after the first are stored as differences from it.
    transformed: bool,
    subsets: u8,
    endpoint_bits: u8,
    /// Red, green and blue bits of the endpoints after the first.
    delta_bits: [u8; 3],
    /// After the mode bits, in block order: `(field, a, b)` is the spec's `field a..b`, so the
    /// next `|a - b| + 1` bits of the block go to the field's bits `b`, `b ± 1`, …, `a`.
    fields: &'static [(u8, u8, u8)],
}

fn sign_extend(v: i32, bits: u8) -> i32 {
    let shift = 32 - u32::from(bits);
    (v << shift) >> shift
}

/// An endpoint value stretched to 16 bits (the unquantization pseudocode).
fn unquantize(x: i32, bits: u8, signed: bool) -> i32 {
    let n = i32::from(bits);
    if !signed {
        return match x {
            _ if n >= 15 => x,
            0 => 0,
            _ if x == (1 << n) - 1 => 0xffff,
            _ => ((x << 15) + 0x4000) >> (n - 1),
        };
    }
    if n >= 16 {
        return x;
    }
    let magnitude = match x.abs() {
        0 => 0,
        a if a >= (1 << (n - 1)) - 1 => 0x7fff,
        a => ((a << 15) + 0x4000) >> (n - 1),
    };
    if x < 0 { -magnitude } else { magnitude }
}

/// An interpolated value as a half float's bits.
fn finish(i: i32, signed: bool) -> u16 {
    match (signed, i < 0) {
        (false, _) => ((i * 31) >> 6) as u16,
        (true, true) => (((-i * 31) >> 5) | 0x8000) as u16,
        (true, false) => ((i * 31) >> 5) as u16,
    }
}

/// The 16 RGB texels of a BC6H block as half floats' bits; a reserved mode gives black.
fn bc6h_halves(block: &[u8; 16], signed: bool) -> [[u16; 3]; 16] {
    let mut r = Bits::new(block);
    let id = match r.take(2) {
        low @ 0..=1 => low,
        low => low | r.take(3) << 2,
    };
    let Some(mode) = BC6H_MODES.iter().find(|m| u32::from(m.id) == id) else {
        return [[0; 3]; 16];
    };
    let mut fields = [0i32; 13];
    for &(field, a, b) in mode.fields {
        for k in 0..=a.abs_diff(b) {
            let bit = if a >= b { b + k } else { b - k };
            if let Some(f) = fields.get_mut(usize::from(field)) {
                *f |= (r.take(1) as i32) << bit;
            }
        }
    }
    let bits = mode.endpoint_bits;
    let ends = usize::from(mode.subsets) * 2;
    let mut endpoints = [[0i32; 3]; 4];
    for (e, endpoint) in endpoints.iter_mut().enumerate().take(ends) {
        for (c, v) in endpoint.iter_mut().enumerate() {
            let (base, stored) = (fields[c], fields[3 * e + c]);
            let x = match e {
                0 => stored,
                _ if mode.transformed => (base + sign_extend(stored, mode.delta_bits[c])) & ((1 << bits) - 1),
                _ => stored,
            };
            *v = unquantize(if signed { sign_extend(x, bits) } else { x }, bits, signed);
        }
    }
    let partition = fields[usize::from(PB)] as usize;
    let index_bits = if mode.subsets == 2 { 3 } else { 4 };
    let index = indices(&mut r, index_bits, mode.subsets, partition);
    std::array::from_fn(|i| {
        let s = subset(mode.subsets, partition, i);
        let (e0, e1) = (endpoints.get(2 * s).copied().unwrap_or_default(), endpoints.get(2 * s + 1).copied().unwrap_or_default());
        let w = weight(index_bits, index[i]);
        std::array::from_fn(|c| finish(interpolate(e0[c], e1[c], w), signed))
    })
}

/// A half float's value.
pub fn half(h: u16) -> f32 {
    let sign = if h & 0x8000 == 0 { 1.0 } else { -1.0 };
    let (exponent, mantissa) = (i32::from(h >> 10 & 31), f32::from(h & 1023));
    sign * match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 if mantissa != 0.0 => f32::NAN,
        31 => f32::INFINITY,
        e => (1024.0 + mantissa) * 2f32.powi(e - 25),
    }
}

/// A linear light value as an 8-bit sRGB value; 0..1 is kept, brighter and negative values clip.
pub fn srgb8(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// The 16 texels of a BC6H block, tone-mapped to 8-bit sRGB (opaque).
pub fn bc6h(block: &[u8; 16], signed: bool) -> [[u8; 4]; 16] {
    bc6h_halves(block, signed).map(|[r, g, b]| [srgb8(half(r)), srgb8(half(g)), srgb8(half(b)), 255])
}

/// Two-subset partitions (Table 114): bit `i` is texel `i`'s subset.
const PARTITIONS_2: [u16; 64] = [
    0xcccc, 0x8888, 0xeeee, 0xecc8, 0xc880, 0xfeec, 0xfec8, 0xec80, 0xc800, 0xffec, 0xfe80, 0xe800, 0xffe8, 0xff00, 0xfff0, 0xf000, 0xf710, 0x008e,
    0x7100, 0x08ce, 0x008c, 0x7310, 0x3100, 0x8cce, 0x088c, 0x3110, 0x6666, 0x366c, 0x17e8, 0x0ff0, 0x718e, 0x399c, 0xaaaa, 0xf0f0, 0x5a5a, 0x33cc,
    0x3c3c, 0x55aa, 0x9696, 0xa55a, 0x73ce, 0x13c8, 0x324c, 0x3bdc, 0x6996, 0xc33c, 0x9966, 0x0660, 0x0272, 0x04e4, 0x4e40, 0x2720, 0xc936, 0x936c,
    0x39c6, 0x639c, 0x9336, 0x9cc6, 0x817e, 0xe718, 0xccf0, 0x0fcc, 0x7744, 0xee22,
];

/// Three-subset partitions (Table 115): bits `2i` and `2i + 1` are texel `i`'s subset.
const PARTITIONS_3: [u32; 64] = [
    0xaa685050, 0x6a5a5040, 0x5a5a4200, 0x5450a0a8, 0xa5a50000, 0xa0a05050, 0x5555a0a0, 0x5a5a5050, 0xaa550000, 0xaa555500, 0xaaaa5500, 0x90909090,
    0x94949494, 0xa4a4a4a4, 0xa9a59450, 0x2a0a4250, 0xa5945040, 0x0a425054, 0xa5a5a500, 0x55a0a0a0, 0xa8a85454, 0x6a6a4040, 0xa4a45000, 0x1a1a0500,
    0x0050a4a4, 0xaaa59090, 0x14696914, 0x69691400, 0xa08585a0, 0xaa821414, 0x50a4a450, 0x6a5a0200, 0xa9a58000, 0x5090a0a8, 0xa8a09050, 0x24242424,
    0x00aa5500, 0x24924924, 0x24499224, 0x50a50a50, 0x500aa550, 0xaaaa4444, 0x66660000, 0xa5a0a5a0, 0x50a050a0, 0x69286928, 0x44aaaa44, 0x66666600,
    0xaa444444, 0x54a854a8, 0x95809580, 0x96969600, 0xa85454a8, 0x80959580, 0xaa141414, 0x96960000, 0xaaaa1414, 0xa05050a0, 0xa0a5a5a0, 0x96000000,
    0x40804080, 0xa9a8a9a8, 0xaaaaaa44, 0x2a4a5254,
];

/// The anchor texel of the second subset of two-subset partitions (Table 118).
const ANCHOR_2: [u8; 64] = [
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 2, 8, 2, 2, 8, 8, 15, 2, 8, 2, 2, 8, 8, 2, 2, 15, 15, 6, 8, 2, 8, 15, 15, 2,
    8, 2, 2, 2, 15, 15, 6, 6, 2, 6, 8, 15, 15, 2, 2, 15, 15, 15, 15, 15, 2, 2, 15,
];

/// The anchor texels of the second and third subsets of three-subset partitions (Tables 116, 117).
const ANCHOR_3_SECOND: [u8; 64] = [
    3, 3, 15, 15, 8, 3, 15, 15, 8, 8, 6, 6, 6, 5, 3, 3, 3, 3, 8, 15, 3, 3, 6, 10, 5, 8, 8, 6, 8, 5, 15, 15, 8, 15, 3, 5, 6, 10, 8, 15, 15, 3, 15, 5,
    15, 15, 15, 15, 3, 15, 5, 5, 5, 8, 5, 10, 5, 10, 8, 13, 15, 12, 3, 3,
];

const ANCHOR_3_THIRD: [u8; 64] = [
    15, 8, 8, 3, 15, 15, 3, 8, 15, 15, 15, 15, 15, 15, 15, 8, 15, 8, 15, 3, 15, 8, 15, 8, 3, 15, 6, 10, 15, 15, 10, 8, 15, 3, 15, 10, 10, 8, 9, 10,
    6, 15, 8, 15, 3, 6, 6, 8, 15, 3, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 3, 15, 15, 8,
];

/// BC6H's modes, in the order of Table 121.
const BC6H_MODES: [Bc6hMode; 14] = [
    Bc6hMode {
        id: 0,
        transformed: true,
        subsets: 2,
        endpoint_bits: 10,
        delta_bits: [5, 5, 5],
        fields: &[
            (G2, 4, 4),
            (B2, 4, 4),
            (B3, 4, 4),
            (R0, 9, 0),
            (G0, 9, 0),
            (B0, 9, 0),
            (R1, 4, 0),
            (G3, 4, 4),
            (G2, 3, 0),
            (G1, 4, 0),
            (B3, 0, 0),
            (G3, 3, 0),
            (B1, 4, 0),
            (B3, 1, 1),
            (B2, 3, 0),
            (R2, 4, 0),
            (B3, 2, 2),
            (R3, 4, 0),
            (B3, 3, 3),
            (PB, 4, 0),
        ],
    },
    Bc6hMode {
        id: 1,
        transformed: true,
        subsets: 2,
        endpoint_bits: 7,
        delta_bits: [6, 6, 6],
        fields: &[
            (G2, 5, 5),
            (G3, 4, 4),
            (G3, 5, 5),
            (R0, 6, 0),
            (B3, 0, 0),
            (B3, 1, 1),
            (B2, 4, 4),
            (G0, 6, 0),
            (B2, 5, 5),
            (B3, 2, 2),
            (G2, 4, 4),
            (B0, 6, 0),
            (B3, 3, 3),
            (B3, 5, 5),
            (B3, 4, 4),
            (R1, 5, 0),
            (G2, 3, 0),
            (G1, 5, 0),
            (G3, 3, 0),
            (B1, 5, 0),
            (B2, 3, 0),
            (R2, 5, 0),
            (R3, 5, 0),
            (PB, 4, 0),
        ],
    },
    Bc6hMode {
        id: 2,
        transformed: true,
        subsets: 2,
        endpoint_bits: 11,
        delta_bits: [5, 4, 4],
        fields: &[
            (R0, 9, 0),
            (G0, 9, 0),
            (B0, 9, 0),
            (R1, 4, 0),
            (R0, 10, 10),
            (G2, 3, 0),
            (G1, 3, 0),
            (G0, 10, 10),
            (B3, 0, 0),
            (G3, 3, 0),
            (B1, 3, 0),
            (B0, 10, 10),
            (B3, 1, 1),
            (B2, 3, 0),
            (R2, 4, 0),
            (B3, 2, 2),
            (R3, 4, 0),
            (B3, 3, 3),
            (PB, 4, 0),
        ],
    },
    Bc6hMode {
        id: 6,
        transformed: true,
        subsets: 2,
        endpoint_bits: 11,
        delta_bits: [4, 5, 4],
        fields: &[
            (R0, 9, 0),
            (G0, 9, 0),
            (B0, 9, 0),
            (R1, 3, 0),
            (R0, 10, 10),
            (G3, 4, 4),
            (G2, 3, 0),
            (G1, 4, 0),
            (G0, 10, 10),
            (G3, 3, 0),
            (B1, 3, 0),
            (B0, 10, 10),
            (B3, 1, 1),
            (B2, 3, 0),
            (R2, 3, 0),
            (B3, 0, 0),
            (B3, 2, 2),
            (R3, 3, 0),
            (G2, 4, 4),
            (B3, 3, 3),
            (PB, 4, 0),
        ],
    },
    Bc6hMode {
        id: 10,
        transformed: true,
        subsets: 2,
        endpoint_bits: 11,
        delta_bits: [4, 4, 5],
        fields: &[
            (R0, 9, 0),
            (G0, 9, 0),
            (B0, 9, 0),
            (R1, 3, 0),
            (R0, 10, 10),
            (B2, 4, 4),
            (G2, 3, 0),
            (G1, 3, 0),
            (G0, 10, 10),
            (B3, 0, 0),
            (G3, 3, 0),
            (B1, 4, 0),
            (B0, 10, 10),
            (B2, 3, 0),
            (R2, 3, 0),
            (B3, 1, 1),
            (B3, 2, 2),
            (R3, 3, 0),
            (B3, 4, 4),
            (B3, 3, 3),
            (PB, 4, 0),
        ],
    },
    Bc6hMode {
        id: 14,
        transformed: true,
        subsets: 2,
        endpoint_bits: 9,
        delta_bits: [5, 5, 5],
        fields: &[
            (R0, 8, 0),
            (B2, 4, 4),
            (G0, 8, 0),
            (G2, 4, 4),
            (B0, 8, 0),
            (B3, 4, 4),
            (R1, 4, 0),
            (G3, 4, 4),
            (G2, 3, 0),
            (G1, 4, 0),
            (B3, 0, 0),
            (G3, 3, 0),
            (B1, 4, 0),
            (B3, 1, 1),
            (B2, 3, 0),
            (R2, 4, 0),
            (B3, 2, 2),
            (R3, 4, 0),
            (B3, 3, 3),
            (PB, 4, 0),
        ],
    },
    Bc6hMode {
        id: 18,
        transformed: true,
        subsets: 2,
        endpoint_bits: 8,
        delta_bits: [6, 5, 5],
        fields: &[
            (R0, 7, 0),
            (G3, 4, 4),
            (B2, 4, 4),
            (G0, 7, 0),
            (B3, 2, 2),
            (G2, 4, 4),
            (B0, 7, 0),
            (B3, 3, 3),
            (B3, 4, 4),
            (R1, 5, 0),
            (G2, 3, 0),
            (G1, 4, 0),
            (B3, 0, 0),
            (G3, 3, 0),
            (B1, 4, 0),
            (B3, 1, 1),
            (B2, 3, 0),
            (R2, 5, 0),
            (R3, 5, 0),
            (PB, 4, 0),
        ],
    },
    Bc6hMode {
        id: 22,
        transformed: true,
        subsets: 2,
        endpoint_bits: 8,
        delta_bits: [5, 6, 5],
        fields: &[
            (R0, 7, 0),
            (B3, 0, 0),
            (B2, 4, 4),
            (G0, 7, 0),
            (G2, 5, 5),
            (G2, 4, 4),
            (B0, 7, 0),
            (G3, 5, 5),
            (B3, 4, 4),
            (R1, 4, 0),
            (G3, 4, 4),
            (G2, 3, 0),
            (G1, 5, 0),
            (G3, 3, 0),
            (B1, 4, 0),
            (B3, 1, 1),
            (B2, 3, 0),
            (R2, 4, 0),
            (B3, 2, 2),
            (R3, 4, 0),
            (B3, 3, 3),
            (PB, 4, 0),
        ],
    },
    Bc6hMode {
        id: 26,
        transformed: true,
        subsets: 2,
        endpoint_bits: 8,
        delta_bits: [5, 5, 6],
        fields: &[
            (R0, 7, 0),
            (B3, 1, 1),
            (B2, 4, 4),
            (G0, 7, 0),
            (B2, 5, 5),
            (G2, 4, 4),
            (B0, 7, 0),
            (B3, 5, 5),
            (B3, 4, 4),
            (R1, 4, 0),
            (G3, 4, 4),
            (G2, 3, 0),
            (G1, 4, 0),
            (B3, 0, 0),
            (G3, 3, 0),
            (B1, 5, 0),
            (B2, 3, 0),
            (R2, 4, 0),
            (B3, 2, 2),
            (R3, 4, 0),
            (B3, 3, 3),
            (PB, 4, 0),
        ],
    },
    Bc6hMode {
        id: 30,
        transformed: false,
        subsets: 2,
        endpoint_bits: 6,
        delta_bits: [6, 6, 6],
        fields: &[
            (R0, 5, 0),
            (G3, 4, 4),
            (B3, 0, 0),
            (B3, 1, 1),
            (B2, 4, 4),
            (G0, 5, 0),
            (G2, 5, 5),
            (B2, 5, 5),
            (B3, 2, 2),
            (G2, 4, 4),
            (B0, 5, 0),
            (G3, 5, 5),
            (B3, 3, 3),
            (B3, 5, 5),
            (B3, 4, 4),
            (R1, 5, 0),
            (G2, 3, 0),
            (G1, 5, 0),
            (G3, 3, 0),
            (B1, 5, 0),
            (B2, 3, 0),
            (R2, 5, 0),
            (R3, 5, 0),
            (PB, 4, 0),
        ],
    },
    Bc6hMode {
        id: 3,
        transformed: false,
        subsets: 1,
        endpoint_bits: 10,
        delta_bits: [10, 10, 10],
        fields: &[(R0, 9, 0), (G0, 9, 0), (B0, 9, 0), (R1, 9, 0), (G1, 9, 0), (B1, 9, 0)],
    },
    Bc6hMode {
        id: 7,
        transformed: true,
        subsets: 1,
        endpoint_bits: 11,
        delta_bits: [9, 9, 9],
        fields: &[(R0, 9, 0), (G0, 9, 0), (B0, 9, 0), (R1, 8, 0), (R0, 10, 10), (G1, 8, 0), (G0, 10, 10), (B1, 8, 0), (B0, 10, 10)],
    },
    Bc6hMode {
        id: 11,
        transformed: true,
        subsets: 1,
        endpoint_bits: 12,
        delta_bits: [8, 8, 8],
        fields: &[(R0, 9, 0), (G0, 9, 0), (B0, 9, 0), (R1, 7, 0), (R0, 10, 11), (G1, 7, 0), (G0, 10, 11), (B1, 7, 0), (B0, 10, 11)],
    },
    Bc6hMode {
        id: 15,
        transformed: true,
        subsets: 1,
        endpoint_bits: 16,
        delta_bits: [4, 4, 4],
        fields: &[(R0, 9, 0), (G0, 9, 0), (B0, 9, 0), (R1, 3, 0), (R0, 10, 15), (G1, 3, 0), (G0, 10, 15), (B1, 3, 0), (B0, 10, 15)],
    },
];

#[cfg(test)]
mod tests {
    use super::{B0, B1, B2, B3, BC6H_MODES, G0, G1, G2, G3, PB, R0, R1, R2, R3, bc6h, bc6h_halves, bc7, half, srgb8};

    /// Pack `(value, bits)` fields into a block from the lowest bit up.
    fn pack(fields: &[(u32, u32)]) -> [u8; 16] {
        let (mut v, mut pos) = (0u128, 0);
        for &(x, n) in fields {
            assert!(u128::from(x) < 1 << n, "{x} doesn't fit in {n} bits");
            v |= u128::from(x) << pos;
            pos += n;
        }
        assert_eq!(pos, 128, "a block is 128 bits");
        v.to_le_bytes()
    }

    /// The mode bits of BC7 mode `m`, then `fields`.
    fn bc7_block(m: u32, fields: &[(u32, u32)]) -> [u8; 16] {
        pack(&[&[(1 << m, m + 1)], fields].concat())
    }

    /// One `bits`-wide field per value.
    fn each(values: &[u32], bits: u32) -> Vec<(u32, u32)> {
        values.iter().map(|v| (*v, bits)).collect()
    }

    /// 16 indices, `bits` wide, one bit fewer at the `anchors`.
    fn texel_indices(values: [u32; 16], bits: u32, anchors: &[usize]) -> Vec<(u32, u32)> {
        values.iter().enumerate().map(|(i, v)| (*v, bits - u32::from(anchors.contains(&i)))).collect()
    }

    fn index_set(set: &[(usize, u32)]) -> [u32; 16] {
        let mut out = [0; 16];
        for (i, v) in set {
            out[*i] = *v;
        }
        out
    }

    /// Grey endpoints: the same value in red, green and blue.
    fn grey(values: &[u32], bits: u32) -> Vec<(u32, u32)> {
        [each(values, bits), each(values, bits), each(values, bits)].concat()
    }

    // Each case's colours are worked out from the specification by hand: endpoint bits (and the
    // P-bit) are a byte's top bits with its top bits repeated below, and a texel is
    // ((64 - w) × e0 + w × e1 + 32) >> 6 with the weight w of its index.

    #[test]
    fn bc7_mode_0_three_subsets_with_endpoint_p_bits() {
        // Partition 0 of three subsets: rows 0011 / 0011 / 0221 / 2222; anchors 0, 3 and 15.
        // Endpoints (4 bits + P-bit): 15+1 → 255, 0 → 0, 8+0 → 132, 8+1 → 140, 2+1 → 41, 4+0 → 66.
        let idx = index_set(&[(1, 7), (2, 4), (3, 3), (9, 5), (15, 3)]);
        let b = bc7_block(
            0,
            &[&[(0, 4)], &grey(&[15, 0, 8, 8, 2, 4], 4)[..], &each(&[1, 0, 0, 1, 1, 0], 1), &texel_indices(idx, 3, &[0, 3, 15])].concat(),
        );
        let px = bc7(&b);
        let g = |v: u8| [v, v, v, 255];
        assert_eq!(px[0], g(255));
        assert_eq!(px[1], g(0), "index 7: the second endpoint");
        assert_eq!(px[2], g(137), "subset 1, weight 37: (27×132 + 37×140 + 32) >> 6");
        assert_eq!(px[3], g(135), "subset 1's anchor, weight 27");
        assert_eq!(px[9], g(59), "subset 2, weight 46: (18×41 + 46×66 + 32) >> 6");
        assert_eq!(px[15], g(52), "subset 2's anchor, weight 27");
        assert_eq!((px[8], px[10], px[11]), (g(255), g(41), g(132)));
    }

    #[test]
    fn bc7_mode_1_shared_p_bits() {
        // Partition 13: texels 0-7 subset 0, 8-15 subset 1 (anchor 15). Endpoints (6 bits +
        // shared P-bit): 63+1 → 255, 0+1 → 2; 32+0 → 129, 16+0 → 64.
        let idx = index_set(&[(1, 7), (8, 2), (15, 3)]);
        let b = bc7_block(1, &[&[(13, 6)], &grey(&[63, 0, 32, 16], 6)[..], &each(&[1, 0], 1), &texel_indices(idx, 3, &[0, 15])].concat());
        let px = bc7(&b);
        let g = |v: u8| [v, v, v, 255];
        assert_eq!((px[0], px[1], px[2]), (g(255), g(2), g(255)));
        assert_eq!(px[8], g(111), "weight 18: (46×129 + 18×64 + 32) >> 6");
        assert_eq!(px[15], g(102), "anchor, weight 27");
        assert_eq!(px[9], g(129));
    }

    #[test]
    fn bc7_mode_2_matches_the_specifications_example() {
        // Partition 6 of three subsets: rows 0022 / 0022 / 1111 / 1111; anchors 0, 15 and 3 (the
        // specification's worked example: texel 9 is subset 1, its index at bits 115-116).
        // Endpoints (5 bits): 31 → 255, 0, 10 → 82, 20 → 165, 1 → 8, 30 → 247.
        let idx = index_set(&[(0, 1), (2, 2), (3, 1), (9, 3), (15, 1)]);
        let b = bc7_block(2, &[&[(6, 6)], &grey(&[31, 0, 10, 20, 1, 30], 5)[..], &texel_indices(idx, 2, &[0, 15, 3])].concat());
        assert_eq!(u128::from_le_bytes(b) >> 115 & 3, 3, "texel 9's index where the specification says");
        let px = bc7(&b);
        let g = |v: u8| [v, v, v, 255];
        assert_eq!(px[0], g(171), "weight 21: (43×255 + 32) >> 6");
        assert_eq!(px[2], g(169), "subset 2, weight 43: (21×8 + 43×247 + 32) >> 6");
        assert_eq!(px[3], g(86), "subset 2's anchor, weight 21");
        assert_eq!(px[9], g(165));
        assert_eq!(px[15], g(109), "subset 1's anchor, weight 21: (43×82 + 21×165 + 32) >> 6");
        assert_eq!((px[1], px[8], px[6]), (g(255), g(82), g(8)));
    }

    #[test]
    fn bc7_mode_3_seven_bit_endpoints() {
        // Partition 0: columns 2-3 are subset 1 (anchor 15). Endpoints (7 bits + P-bit):
        // 100+1 → 201, 10+0 → 20, 127+1 → 255, 0+0 → 0.
        let idx = index_set(&[(0, 1), (1, 2), (2, 3), (15, 1)]);
        let b = bc7_block(3, &[&[(0, 6)], &grey(&[100, 10, 127, 0], 7)[..], &each(&[1, 0, 1, 0], 1), &texel_indices(idx, 2, &[0, 15])].concat());
        let px = bc7(&b);
        let g = |v: u8| [v, v, v, 255];
        assert_eq!(px[0], g(142), "weight 21: (43×201 + 21×20 + 32) >> 6");
        assert_eq!(px[1], g(79), "weight 43");
        assert_eq!(px[2], g(0));
        assert_eq!(px[15], g(171));
        assert_eq!((px[3], px[4]), (g(255), g(201)));
    }

    #[test]
    fn bc7_mode_4_rotation_and_index_selection() {
        // Rotation 1 swaps alpha and red; the selection bit takes colour from the 3-bit indices
        // and alpha from the 2-bit ones. Red 31 → 255 to 0, green 0 to 255, blue 16 → 132,
        // alpha (6 bits) 63 → 255 to 0.
        let colour = index_set(&[(0, 3), (1, 7)]);
        let alpha = index_set(&[(0, 1)]);
        let ends = [each(&[31, 0, 0, 31, 16, 16], 5), each(&[63, 0], 6)].concat();
        let b = bc7_block(4, &[&[(1, 2), (1, 1)], &ends[..], &texel_indices(alpha, 2, &[0]), &texel_indices(colour, 3, &[0])].concat());
        let px = bc7(&b);
        // Before the swap: red (37×255 + 32) >> 6 = 147, green 108, blue 132, alpha 171.
        assert_eq!(px[0], [171, 108, 132, 147]);
        assert_eq!(px[1], [255, 255, 132, 0]);
        assert_eq!(px[2], [255, 0, 132, 255]);
    }

    #[test]
    fn bc7_mode_5_separate_alpha_indices() {
        // Red 127 → 255 to 0, green 64 → 129, blue 0 to 255; alpha (8 bits) 200 to 100.
        let colour = index_set(&[(0, 1), (5, 2)]);
        let alpha = index_set(&[(0, 1), (5, 3)]);
        let ends = [each(&[127, 0, 64, 64, 0, 127], 7), each(&[200, 100], 8)].concat();
        let b = bc7_block(5, &[&[(0, 2)], &ends[..], &texel_indices(colour, 2, &[0]), &texel_indices(alpha, 2, &[0])].concat());
        let px = bc7(&b);
        assert_eq!(px[0], [171, 129, 84, 167], "weight 21 for colour and alpha: (43×200 + 21×100 + 32) >> 6");
        assert_eq!(px[5], [84, 129, 171, 100]);
        assert_eq!(px[1], [255, 129, 0, 200]);
    }

    #[test]
    fn bc7_mode_6_four_bit_indices() {
        // Endpoints (7 bits + P-bit) in every channel: [255, 1, 129, 255] and [0, 254, 64, 100].
        let idx = index_set(&[(0, 7), (1, 15), (2, 8)]);
        let ends = each(&[127, 0, 0, 127, 64, 32, 127, 50], 7);
        let b = bc7_block(6, &[&ends[..], &each(&[1, 0], 1), &texel_indices(idx, 4, &[0])].concat());
        let px = bc7(&b);
        assert_eq!(px[0], [135, 120, 99, 182], "the anchor's largest index, 7: weight 30");
        assert_eq!(px[1], [0, 254, 64, 100]);
        assert_eq!(px[2], [120, 135, 94, 173], "weight 34: red (30×255 + 32) >> 6");
        assert_eq!(px[3], [255, 1, 129, 255]);
    }

    #[test]
    fn bc7_mode_7_two_subsets_with_alpha() {
        // Partition 1: column 3 is subset 1 (anchor 15). Endpoints (5 bits + P-bit):
        // [255; 4], [0; 4], [130, 0, 0, 251], [4, 134, 69, 4].
        let idx = index_set(&[(0, 1), (3, 3), (15, 1)]);
        let ends = [each(&[31, 0, 16, 0], 5), each(&[31, 0, 0, 16], 5), each(&[31, 0, 0, 8], 5), each(&[31, 0, 31, 0], 5)].concat();
        let b = bc7_block(7, &[&[(1, 6)], &ends[..], &each(&[1, 0, 0, 1], 1), &texel_indices(idx, 2, &[0, 15])].concat());
        let px = bc7(&b);
        assert_eq!(px[0], [171; 4]);
        assert_eq!(px[3], [4, 134, 69, 4]);
        assert_eq!(px[15], [89, 44, 23, 170], "weight 21: red (43×130 + 21×4 + 32) >> 6");
        assert_eq!((px[7], px[1]), ([130, 0, 0, 251], [255; 4]));
    }

    #[test]
    fn bc7_reserved_block_is_all_zero() {
        assert_eq!(bc7(&[0; 16]), [[0; 4]; 16]);
    }

    /// A BC6H block of mode `id`: `values` by field (missing ones are 0), then the indices.
    fn bc6h_block(id: u8, values: &[(u8, i32)], index: [u32; 16]) -> [u8; 16] {
        let mode = BC6H_MODES.iter().find(|m| m.id == id).unwrap();
        let value = |f: u8| values.iter().find(|(g, _)| *g == f).map_or(0, |(_, v)| *v);
        let mut fields = vec![(u32::from(id), if id < 2 { 2 } else { 5 })];
        for &(f, a, b) in mode.fields {
            for k in 0..=a.abs_diff(b) {
                let bit = if a >= b { b + k } else { b - k };
                fields.push(((value(f) >> bit) as u32 & 1, 1));
            }
        }
        let bits = if mode.subsets == 2 { 3 } else { 4 };
        let partition = value(PB) as usize;
        for (i, x) in index.iter().enumerate() {
            fields.push((*x, bits - u32::from(super::anchor(mode.subsets, partition, i))));
        }
        pack(&fields)
    }

    /// The same endpoints for every mode: the first is (largest, 0, middle); the others are small
    /// differences from it (or, without transformed endpoints, values near it). Two-subset modes
    /// use partition 13 (texels 8-15 are subset 1); indices pick each endpoint and two blends.
    fn bc6h_case(id: u8) -> ([u8; 16], Vec<usize>) {
        let mode = BC6H_MODES.iter().find(|m| m.id == id).unwrap();
        let (max, mid, d) = ((1 << mode.endpoint_bits) - 1, 1 << (mode.endpoint_bits - 1), mode.delta_bits);
        let mut v = vec![(R0, max), (G0, 0), (B0, mid)];
        if mode.transformed {
            v.extend([(R1, -1), (G1, 1), (B1, 0)]);
        } else {
            v.extend([(R1, max - 1), (G1, 1), (B1, mid)]);
        }
        match (mode.transformed, mode.subsets) {
            (true, 2) => v.extend([(R2, 0), (G2, 2), (B2, -2), (R3, -(1 << (d[0] - 1))), (G3, 0), (B3, 1), (PB, 13)]),
            (false, 2) => v.extend([(R2, max), (G2, 2), (B2, mid - 2), (R3, 0), (G3, max), (B3, mid + 1), (PB, 13)]),
            _ => {}
        }
        if mode.subsets == 2 {
            (bc6h_block(id, &v, index_set(&[(1, 7), (8, 7), (2, 4), (9, 3)])), vec![0, 1, 2, 8, 9, 15])
        } else {
            (bc6h_block(id, &v, index_set(&[(1, 15), (2, 8)])), vec![0, 1, 2])
        }
    }

    // Half-float results for `bc6h_case`, from a separate decoder written from the specification's
    // pseudocode. By hand, for mode 3's first texel: red is the largest 10-bit value → 0xffff →
    // (0xffff × 31) >> 6 = 0x7bff (65504); blue 512 → ((512 << 15) + 0x4000) >> 9 = 32800 →
    // (32800 × 31) >> 6 = 0x3e0f. Signed, red is -1 → -96 → 0x805d and blue -512 → -0x7fff → 0xfbff.

    const UNSIGNED: [(u8, &[[u16; 3]]); 14] = [
        (
            0,
            &[
                [0x7bff, 0x0000, 0x3e0f],
                [0x7bd1, 0x002e, 0x3e0f],
                [0x7be4, 0x001b, 0x3e0f],
                [0x7a00, 0x0000, 0x3e2e],
                [0x7b27, 0x002d, 0x3df8],
                [0x7bff, 0x004d, 0x3dd1],
            ],
        ),
        (
            1,
            &[
                [0x7bff, 0x0000, 0x3e7c],
                [0x7a8c, 0x0174, 0x3e7c],
                [0x7b28, 0x00d7, 0x3e7c],
                [0x5c84, 0x0000, 0x3f74],
                [0x6eb7, 0x0166, 0x3dc5],
                [0x7bff, 0x026c, 0x3c8c],
            ],
        ),
        (
            2,
            &[
                [0x7bff, 0x0000, 0x3e07],
                [0x7be8, 0x0017, 0x3e07],
                [0x7bf2, 0x000d, 0x3e07],
                [0x7b00, 0x0000, 0x3e17],
                [0x7b93, 0x0016, 0x3dfc],
                [0x7bff, 0x0026, 0x3de8],
            ],
        ),
        (
            6,
            &[
                [0x7bff, 0x0000, 0x3e07],
                [0x7be8, 0x0017, 0x3e07],
                [0x7bf2, 0x000d, 0x3e07],
                [0x7b7c, 0x0000, 0x3e17],
                [0x7bc8, 0x0016, 0x3dfc],
                [0x7bff, 0x0026, 0x3de8],
            ],
        ),
        (
            10,
            &[
                [0x7bff, 0x0000, 0x3e07],
                [0x7be8, 0x0017, 0x3e07],
                [0x7bf2, 0x000d, 0x3e07],
                [0x7b7c, 0x0000, 0x3e17],
                [0x7bc8, 0x0016, 0x3dfc],
                [0x7bff, 0x0026, 0x3de8],
            ],
        ),
        (
            14,
            &[
                [0x7bff, 0x0000, 0x3e1f],
                [0x7ba3, 0x005d, 0x3e1f],
                [0x7bca, 0x0035, 0x3e1f],
                [0x7801, 0x0000, 0x3e5d],
                [0x7a4f, 0x0059, 0x3df1],
                [0x7bff, 0x009b, 0x3da3],
            ],
        ),
        (
            18,
            &[
                [0x7bff, 0x0000, 0x3e3e],
                [0x7b46, 0x00ba, 0x3e3e],
                [0x7b94, 0x006b, 0x3e3e],
                [0x6c42, 0x0000, 0x3eba],
                [0x755b, 0x00b3, 0x3de2],
                [0x7bff, 0x0136, 0x3d46],
            ],
        ),
        (
            22,
            &[
                [0x7bff, 0x0000, 0x3e3e],
                [0x7b46, 0x00ba, 0x3e3e],
                [0x7b94, 0x006b, 0x3e3e],
                [0x7402, 0x0000, 0x3eba],
                [0x78a0, 0x00b3, 0x3de2],
                [0x7bff, 0x0136, 0x3d46],
            ],
        ),
        (
            26,
            &[
                [0x7bff, 0x0000, 0x3e3e],
                [0x7b46, 0x00ba, 0x3e3e],
                [0x7b94, 0x006b, 0x3e3e],
                [0x7402, 0x0000, 0x3eba],
                [0x78a0, 0x00b3, 0x3de2],
                [0x7bff, 0x0136, 0x3d46],
            ],
        ),
        (
            30,
            &[
                [0x7bff, 0x0000, 0x3ef8],
                [0x7918, 0x02e8, 0x3ef8],
                [0x7a51, 0x01ae, 0x3ef8],
                [0x0000, 0x7bff, 0x40e8],
                [0x47af, 0x371c, 0x3d8b],
                [0x7bff, 0x04d8, 0x3b18],
            ],
        ),
        (3, &[[0x7bff, 0x0000, 0x3e0f], [0x7bd1, 0x002e, 0x3e0f], [0x7be7, 0x0018, 0x3e0f]]),
        (7, &[[0x7bff, 0x0000, 0x3e07], [0x7be8, 0x0017, 0x3e07], [0x7bf3, 0x000c, 0x3e07]]),
        (11, &[[0x7bff, 0x0000, 0x3e03], [0x7bf4, 0x000b, 0x3e03], [0x7bf9, 0x0006, 0x3e03]]),
        (15, &[[0x7bff, 0x0000, 0x3e00], [0x7bff, 0x0000, 0x3e00], [0x7bff, 0x0000, 0x3e00]]),
    ];
    const SIGNED: [(u8, &[[u16; 3]]); 14] = [
        (
            0,
            &[
                [0x805d, 0x0000, 0xfbff],
                [0x809b, 0x005d, 0xfbff],
                [0x8080, 0x0036, 0xfbff],
                [0x843d, 0x0000, 0xfbff],
                [0x81ff, 0x005a, 0x132a],
                [0x805d, 0x009b, 0x7ba3],
            ],
        ),
        (
            1,
            &[
                [0x82e8, 0x0000, 0xfbff],
                [0x84d8, 0x02e8, 0xfbff],
                [0x8406, 0x01ae, 0xfbff],
                [0xc0e8, 0x0000, 0xfbff],
                [0x9d10, 0x02cc, 0x11b1],
                [0x82e8, 0x04d8, 0x7918],
            ],
        ),
        (
            2,
            &[
                [0x802e, 0x0000, 0xfbff],
                [0x804d, 0x002e, 0xfbff],
                [0x803f, 0x001b, 0xfbff],
                [0x821e, 0x0000, 0xfbff],
                [0x80ff, 0x002c, 0x1345],
                [0x802e, 0x004d, 0x7bd1],
            ],
        ),
        (
            6,
            &[
                [0x802e, 0x0000, 0xfbff],
                [0x804d, 0x002e, 0xfbff],
                [0x803f, 0x001b, 0xfbff],
                [0x8126, 0x0000, 0xfbff],
                [0x8097, 0x002c, 0x1345],
                [0x802e, 0x004d, 0x7bd1],
            ],
        ),
        (
            10,
            &[
                [0x802e, 0x0000, 0xfbff],
                [0x804d, 0x002e, 0xfbff],
                [0x803f, 0x001b, 0xfbff],
                [0x8126, 0x0000, 0xfbff],
                [0x8097, 0x002c, 0x1345],
                [0x802e, 0x004d, 0x7bd1],
            ],
        ),
        (
            14,
            &[
                [0x80ba, 0x0000, 0xfbff],
                [0x8136, 0x00ba, 0xfbff],
                [0x8101, 0x006b, 0xfbff],
                [0x887a, 0x0000, 0xfbff],
                [0x83ff, 0x00b3, 0x12f4],
                [0x80ba, 0x0136, 0x7b46],
            ],
        ),
        (
            18,
            &[
                [0x8174, 0x0000, 0xfbff],
                [0x826c, 0x0174, 0xfbff],
                [0x8203, 0x00d7, 0xfbff],
                [0xa074, 0x0000, 0xfbff],
                [0x8e88, 0x0166, 0x1288],
                [0x8174, 0x026c, 0x7a8c],
            ],
        ),
        (
            22,
            &[
                [0x8174, 0x0000, 0xfbff],
                [0x826c, 0x0174, 0xfbff],
                [0x8203, 0x00d7, 0xfbff],
                [0x90f4, 0x0000, 0xfbff],
                [0x87fe, 0x0166, 0x1288],
                [0x8174, 0x026c, 0x7a8c],
            ],
        ),
        (
            26,
            &[
                [0x8174, 0x0000, 0xfbff],
                [0x826c, 0x0174, 0xfbff],
                [0x8203, 0x00d7, 0xfbff],
                [0x90f4, 0x0000, 0xfbff],
                [0x87fe, 0x0166, 0x1288],
                [0x8174, 0x026c, 0x7a8c],
            ],
        ),
        (
            30,
            &[
                [0x85d0, 0x0000, 0xfbff],
                [0x89b0, 0x05d0, 0xfbff],
                [0x880d, 0x035c, 0xfbff],
                [0x0000, 0x85d0, 0xfbff],
                [0x835c, 0x0326, 0x1003],
                [0x85d0, 0x09b0, 0x7630],
            ],
        ),
        (3, &[[0x805d, 0x0000, 0xfbff], [0x809b, 0x005d, 0xfbff], [0x807d, 0x0031, 0xfbff]]),
        (7, &[[0x802e, 0x0000, 0xfbff], [0x804d, 0x002e, 0xfbff], [0x803e, 0x0019, 0xfbff]]),
        (11, &[[0x8017, 0x0000, 0xfbff], [0x8026, 0x0017, 0xfbff], [0x801f, 0x000c, 0xfbff]]),
        (15, &[[0x8000, 0x0000, 0xfc00], [0x8001, 0x0000, 0xfc00], [0x8001, 0x0000, 0xfc00]]),
    ];

    #[test]
    fn bc6h_every_mode_unsigned_and_signed() {
        for (signed, expected) in [(false, &UNSIGNED), (true, &SIGNED)] {
            for (id, want) in expected {
                let (block, texels) = bc6h_case(*id);
                let got = bc6h_halves(&block, signed);
                let got: Vec<[u16; 3]> = texels.iter().map(|t| got[*t]).collect();
                assert_eq!(&got[..], *want, "mode {id}, signed {signed}");
            }
        }
        // Reserved modes (19, 23, 27, 31) are black.
        for id in [19u32, 23, 27, 31] {
            assert_eq!(bc6h_halves(&pack(&[(id, 5), (0, 123)]), false), [[0; 3]; 16]);
        }
    }

    #[test]
    fn bc6h_tone_maps_to_srgb() {
        assert_eq!([0x0000, 0x3800, 0x3c00, 0x7bff, 0xfc00, 0xbc00, 0x0001].map(|h| srgb8(half(h))), [0, 188, 255, 255, 0, 0, 0]);
        assert_eq!(half(0x3555), 0.333_251_95, "a normal half: (1024 + 0x155) × 2^(13 - 25)");
        assert_eq!(half(0x0200), 2f32.powi(-15), "a subnormal half");
        // Mode 3's first texel: red 65504 and blue 1.5 clip to white, green is 0.
        assert_eq!(bc6h(&bc6h_case(3).0, false)[0], [255, 0, 255, 255]);
    }

    #[test]
    fn any_block_decodes_without_panicking() {
        let mut x = 0x9e37_79b9_7f4a_7c15u128 << 64 | 0x2545_f491_4f6c_dd1d;
        for _ in 0..20_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let b = x.to_le_bytes();
            let _ = (bc7(&b), bc6h(&b, false), bc6h(&b, true));
        }
    }
}
