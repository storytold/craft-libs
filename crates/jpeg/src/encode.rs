//! Parallel baseline JPEG encoder (ITU-T T.81 / JFIF).
//!
//! The image is cut into bands of MCU rows separated by restart markers (`DRI` + `RSTn`): DC
//! prediction restarts at every marker, so bands are entropy-coded independently on all cores and
//! concatenated. Quantization uses the example tables of T.81 Annex K scaled by the usual 1–100
//! quality convention. Huffman tables are made for each image from its own symbol counts (T.81
//! Annex K.2): a first pass over the bands counts the symbols, a second codes them. The file stays
//! interleaved baseline with the tables in its `DHT` segments, readable by every decoder, and is
//! smaller than with the Annex K example tables at the same pixels. Supports grayscale and YCbCr
//! 4:4:4, 4:2:2, and 4:2:0.

/// Chroma sampling for baseline JPEG encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChromaSubsampling {
    /// Full resolution chroma.
    #[default]
    S444,
    /// Half horizontal resolution chroma.
    S422,
    /// Half horizontal and vertical resolution chroma.
    S420,
}

/// Why a baseline JPEG could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    /// Invalid dimensions, channel count, or source buffer length.
    #[error("invalid JPEG input: {0}")]
    InvalidInput(String),
}

/// T.81 Annex K.1 / K.2 example quantization tables (natural order).
const Q_LUMA: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56,
    68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];
const Q_CHROMA: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99, 47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

/// Zig-zag position k → natural index.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43,
    36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// T.81 Annex K.3 example Huffman tables: (code counts per length 1..16, symbols).
const DC_LUMA_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_CHROMA_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_LUMA_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const AC_LUMA_VALS: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08,
    0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
    0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6,
    0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
];
const AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const AC_CHROMA_VALS: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91,
    0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0, 0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58,
    0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4,
    0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
];

/// Huffman code table: symbol → (code, length).
struct Huff {
    code: [(u16, u8); 256],
}

impl Huff {
    /// Canonical codes from counts (T.81 Annex C).
    fn new(bits: &[u8; 16], vals: &[u8]) -> Huff {
        let mut code = [(0u16, 0u8); 256];
        let (mut c, mut k) = (0u16, 0usize);
        for (len, &n) in bits.iter().enumerate() {
            for _ in 0..n {
                if let Some(&v) = vals.get(k) {
                    code[v as usize] = (c, len as u8 + 1);
                }
                c = c.wrapping_add(1);
                k += 1;
            }
            c = c.wrapping_shl(1);
        }
        Huff { code }
    }
}

/// Symbol counts of one Huffman table: index = symbol (DC size category, or AC run/size byte).
type Freq = [u64; 256];

/// The code lengths (`BITS`, counts per length 1..16) and symbols (`HUFFVAL`) of an optimal
/// Huffman table for `freq`, limited to 16-bit codes and never assigning the all-ones code
/// (T.81 Annex K.2, figures K.1–K.4). Symbols with a zero count get no code. At least one symbol
/// is always present (a table nothing used gets symbol 0), so the result is a valid `DHT`.
fn optimal_table(freq: &Freq) -> ([u8; 16], Vec<u8>) {
    // 256 is the reserved pseudo-symbol: counted once, it takes the longest code, which is
    // then dropped so no real code is all ones (K.2)
    let mut f = [0u64; 257];
    f[..256].copy_from_slice(freq);
    if f[..256].iter().all(|&c| c == 0) {
        f[0] = 1;
    }
    f[256] = 1;
    let mut size = [0usize; 257];
    let mut next: [Option<usize>; 257] = [None; 257];
    loop {
        // the two smallest non-zero counts; on equal counts the larger symbol goes first (K.1)
        let mut c1: Option<usize> = None;
        for i in 0..257 {
            if f[i] > 0 && c1.is_none_or(|c| f[i] <= f[c]) {
                c1 = Some(i);
            }
        }
        let Some(c1) = c1 else { break };
        let mut c2: Option<usize> = None;
        for i in 0..257 {
            if i != c1 && f[i] > 0 && c2.is_none_or(|c| f[i] <= f[c]) {
                c2 = Some(i);
            }
        }
        let Some(c2) = c2 else { break };
        f[c1] = f[c1].saturating_add(f[c2]);
        f[c2] = 0;
        // every symbol in both trees goes one level deeper; c2's chain is appended to c1's
        let mut k = c1;
        size[k] += 1;
        while let Some(n) = next[k] {
            k = n;
            size[k] += 1;
        }
        next[k] = Some(c2);
        let mut k = c2;
        size[k] += 1;
        while let Some(n) = next[k] {
            k = n;
            size[k] += 1;
        }
    }
    // counts per code length; 257 symbols need at most 256 levels
    let mut bits = [0usize; 257];
    for &s in &size {
        if s > 0 {
            bits[s.min(256)] += 1;
        }
    }
    // limit to 16 bits (K.3): move pairs of the longest codes up, one level at a time (the
    // deepest level of a full tree always holds an even number of codes)
    for i in (17..257).rev() {
        while bits[i] >= 2 {
            let Some(j) = (1..i - 1).rev().find(|&j| bits[j] > 0) else { break };
            bits[i] -= 2;
            bits[i - 1] += 1;
            bits[j + 1] += 2;
            bits[j] -= 1;
        }
    }
    // drop the reserved symbol's code: it is one of the longest
    if let Some(i) = (1..=16).rev().find(|&i| bits[i] > 0) {
        bits[i] -= 1;
    }
    // symbols by code length, then by value (K.4); the reserved symbol is not listed
    let mut vals = Vec::new();
    for len in 1..257 {
        vals.extend((0..256).filter(|&s| size[s] == len).map(|s| s as u8));
    }
    let mut out = [0u8; 16];
    for (o, &b) in out.iter_mut().zip(&bits[1..=16]) {
        *o = b.min(255) as u8;
    }
    (out, vals)
}

fn scaled_quant(base: &[u16; 64], quality: u8) -> [u16; 64] {
    let q = quality.clamp(1, 100) as u32;
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    base.map(|b| ((b as u32 * scale + 50) / 100).clamp(1, 255) as u16)
}

/// Entropy-coded segment writer with 0xFF byte stuffing.
struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    fn new() -> Bits {
        Bits { out: Vec::new(), acc: 0, n: 0 }
    }
    #[inline]
    fn put(&mut self, code: u32, len: u32) {
        if len == 0 {
            return;
        }
        self.acc = (self.acc << len) | (code & ((1 << len) - 1));
        self.n += len;
        while self.n >= 8 {
            let b = (self.acc >> (self.n - 8)) as u8;
            self.out.push(b);
            if b == 0xFF {
                self.out.push(0);
            }
            self.n -= 8;
        }
        self.acc &= (1 << self.n) - 1;
    }
    /// Pad with 1-bits to a byte boundary.
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
        self.out
    }
}

/// AAN output scale factors: `s[0] = 1`, `s[k] = cos(kπ/16)·√2`.
fn aan_scale() -> [f32; 8] {
    std::array::from_fn(|k| if k == 0 { 1.0 } else { ((k as f64 * std::f64::consts::PI / 16.0).cos() * std::f64::consts::SQRT_2) as f32 })
}

/// Quantization reciprocals with the AAN scaling folded in: coefficient = out / (8·s[u]·s[v]·q).
fn recip_table(q: &[u16; 64]) -> [f32; 64] {
    let s = aan_scale();
    std::array::from_fn(|i| 1.0 / (q[i] as f32 * s[i / 8] * s[i % 8] * 8.0))
}

/// One 8-point forward DCT (Arai–Agui–Nakajima flowgraph, 5 multiplies) on `d[o + k·stride]`.
#[inline(always)]
fn aan_1d(d: &mut [f32; 64], o: usize, stride: usize) {
    let g = |k: usize| o + k * stride;
    let (t0, t7) = (d[g(0)] + d[g(7)], d[g(0)] - d[g(7)]);
    let (t1, t6) = (d[g(1)] + d[g(6)], d[g(1)] - d[g(6)]);
    let (t2, t5) = (d[g(2)] + d[g(5)], d[g(2)] - d[g(5)]);
    let (t3, t4) = (d[g(3)] + d[g(4)], d[g(3)] - d[g(4)]);
    // even part
    let (t10, t13, t11, t12) = (t0 + t3, t0 - t3, t1 + t2, t1 - t2);
    d[g(0)] = t10 + t11;
    d[g(4)] = t10 - t11;
    let z1 = (t12 + t13) * 0.707_106_77;
    d[g(2)] = t13 + z1;
    d[g(6)] = t13 - z1;
    // odd part
    let (t10, t11, t12) = (t4 + t5, t5 + t6, t6 + t7);
    let z5 = (t10 - t12) * 0.382_683_43;
    let z2 = 0.541_196_1 * t10 + z5;
    let z4 = 1.306_563 * t12 + z5;
    let z3 = t11 * 0.707_106_77;
    let (z11, z13) = (t7 + z3, t7 - z3);
    d[g(5)] = z13 + z2;
    d[g(3)] = z13 - z2;
    d[g(1)] = z11 + z4;
    d[g(7)] = z11 - z4;
}

/// Forward DCT of a level-shifted 8×8 block and quantization.
#[inline]
fn fdct_quant(block: &[f32; 64], recip: &[f32; 64], out: &mut [i16; 64]) {
    let mut d = *block;
    for r in 0..8 {
        aan_1d(&mut d, r * 8, 1);
    }
    for c in 0..8 {
        aan_1d(&mut d, c, 8);
    }
    for i in 0..64 {
        out[i] = round_i16(d[i] * recip[i]);
    }
}
/// `x.round() as i16` (half away from zero, saturating, NaN to 0) for every `x` (checked for all of them), in plain
/// arithmetic: without SSE4.1 `f32::round` is a libm call, one per coefficient, that keeps the loop from vectorising.
#[inline(always)]
fn round_i16(x: f32) -> i16 {
    // (below 2^24 the truncation is exact and so is the remainder)
    let c = x.clamp(-65536.0, 65536.0);
    let t = c as i32;
    let f = c - t as f32;
    let r = t + i32::from(f >= 0.5) - i32::from(f <= -0.5);
    r.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

#[inline]
fn magnitude(v: i32) -> (u32, u32) {
    let a = v.unsigned_abs();
    let size = 32 - a.leading_zeros();
    let bits = if v < 0 { (v - 1) as u32 } else { v as u32 };
    (size, bits & ((1u32 << size) - 1))
}

/// The Huffman symbols of one quantized block in coding order: `sym(ac, symbol, bits, n)` for the
/// DC size category (`ac` false) and each AC run/size, ZRL and EOB (`ac` true), each followed by
/// its `n` extra bits `bits`.
#[inline(always)]
fn block_symbols(q: &[i16; 64], pred: &mut i32, mut sym: impl FnMut(bool, usize, u32, u32)) {
    let d = q[0] as i32 - *pred;
    *pred = q[0] as i32;
    let (s, b) = magnitude(d);
    sym(false, s as usize, b, s);
    let mut run = 0u32;
    for &zi in &ZIGZAG[1..] {
        let v = q[zi] as i32;
        if v == 0 {
            run += 1;
            continue;
        }
        while run >= 16 {
            sym(true, 0xF0, 0, 0);
            run -= 16;
        }
        let (s, b) = magnitude(v);
        sym(true, ((run << 4) | s) as usize, b, s);
        run = 0;
    }
    if run > 0 {
        sym(true, 0x00, 0, 0);
    }
}

/// One coded symbol of the entropy-coded data: bits 0–7 the symbol, 8–9 its table (0 DC luma,
/// 1 AC luma, 2 DC chroma, 3 AC chroma), 10–13 the number of extra bits, 14–29 those bits (a
/// baseline coefficient needs at most 11).
type Token = u32;

/// Append one block's symbols to `out` and count them in `freq` (by table, as in [`Token`]).
fn tokenize_block(q: &[i16; 64], pred: &mut i32, chroma: bool, out: &mut Vec<Token>, freq: &mut [Freq; 4]) {
    let base = if chroma { 2 } else { 0 };
    block_symbols(q, pred, |is_ac, s, bits, n| {
        let table = base + is_ac as usize;
        freq[table][s & 0xFF] += 1;
        out.push((s as u32 & 0xFF) | (table as u32) << 8 | (n & 0xF) << 10 | (bits & 0xFFFF) << 14);
    });
}

/// Code a band's symbols with `tables` (indexed as in [`Token`]).
fn write_tokens(tokens: &[Token], tables: &[Huff; 4]) -> Vec<u8> {
    let mut w = Bits::new();
    for &t in tokens {
        let (s, table, n, bits) = (t & 0xFF, (t >> 8) & 3, (t >> 10) & 0xF, t >> 14);
        let (c, l) = tables[table as usize].code[s as usize];
        w.put(c as u32, l as u32);
        w.put(bits, n);
    }
    w.finish()
}

/// Planar component samples (level-shifted to −128..127) with edge replication on read.
struct Plane {
    w: usize,
    h: usize,
    data: Vec<f32>,
}

impl Plane {
    #[inline]
    fn block(&self, bx: usize, by: usize, out: &mut [f32; 64]) {
        let (x0, y0) = (bx * 8, by * 8);
        if x0 + 8 <= self.w && y0 + 8 <= self.h {
            for y in 0..8 {
                let i = (y0 + y) * self.w + x0;
                out[y * 8..y * 8 + 8].copy_from_slice(&self.data[i..i + 8]);
            }
            return;
        }
        for y in 0..8 {
            let sy = (by * 8 + y).min(self.h - 1);
            let row = &self.data[sy * self.w..(sy + 1) * self.w];
            for x in 0..8 {
                out[y * 8 + x] = row[(bx * 8 + x).min(self.w - 1)];
            }
        }
    }
}

fn segment(out: &mut Vec<u8>, marker: u8, payload: &[u8]) {
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(payload);
}

fn dht(out: &mut Vec<u8>, class_id: u8, bits: &[u8; 16], vals: &[u8]) {
    let mut p = vec![class_id];
    p.extend_from_slice(bits);
    p.extend_from_slice(vals);
    segment(out, 0xC4, &p);
}

/// Encode interleaved 8-bit samples (`channels` 1, 3 or 4; alpha is dropped), with Huffman
/// tables made for the image.
pub fn encode(
    data: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    quality: u8,
    sub: ChromaSubsampling,
    app_segments: &[(u8, Vec<u8>)],
) -> Vec<u8> {
    encode_with(data, width, height, channels, quality, sub, app_segments, true)
}

/// Encode an 8-bit grayscale, RGB, or RGBA image as an interleaved baseline JPEG with per-image
/// optimized Huffman tables. Alpha is ignored. Metadata APP segments are supplied as `(marker,
/// payload)` pairs.
pub fn encode_baseline(
    data: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    quality: u8,
    sub: ChromaSubsampling,
    app_segments: &[(u8, Vec<u8>)],
) -> Result<Vec<u8>, EncodeError> {
    let fits = width.checked_mul(height).and_then(|n| n.checked_mul(channels)).is_some_and(|n| n <= data.len());
    if !fits || width == 0 || height == 0 || width > 65_535 || height > 65_535 || !matches!(channels, 1 | 3 | 4) {
        return Err(EncodeError::InvalidInput(format!("{width}x{height}, {channels} channels, {} bytes", data.len())));
    }
    if app_segments.iter().any(|(marker, payload)| !(0xE0..=0xEF).contains(marker) || payload.len() > 65_533) {
        return Err(EncodeError::InvalidInput("APP markers must be 0xE0..=0xEF with payloads no larger than 65533 bytes".into()));
    }
    Ok(encode(data, width, height, channels, quality, sub, app_segments))
}

/// [`encode`], with optimized Huffman tables or (`optimize` false) the Annex K example tables.
#[allow(clippy::too_many_arguments)]
fn encode_with(
    data: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    quality: u8,
    sub: ChromaSubsampling,
    app_segments: &[(u8, Vec<u8>)],
    optimize: bool,
) -> Vec<u8> {
    // The public entry takes caller-sized dimensions: refuse (an empty output) what would size an allocation from
    // them or index past `data`, rather than panic or allocate for them.
    let fits = width.checked_mul(height).and_then(|n| n.checked_mul(channels)).is_some_and(|n| n <= data.len());
    if !fits || width == 0 || height == 0 || width > 65_535 || height > 65_535 || !matches!(channels, 1 | 3 | 4) {
        return Vec::new();
    }
    let gray = channels == 1;
    let (hs, vs) = match (gray, sub) {
        (true, _) | (false, ChromaSubsampling::S444) => (1usize, 1usize),
        (false, ChromaSubsampling::S422) => (2, 1),
        (false, ChromaSubsampling::S420) => (2, 2),
    };
    #[cfg(feature = "parallel")]
    use rayon::prelude::*;
    // colour conversion (JFIF YCbCr, full range, level-shifted) of the pixel rows `rows`, one band's worth at a
    // time (in the band's task: the planes of the whole image would be ~290 MB at 24 MP, written and read back
    // once): 4:4:4 Y, Cb, Cr at full resolution in one pass; 4:2:0 Y, and chroma (rows `crows` of the
    // half-size planes) straight from 2×2 RGB means (conversion is linear: average first, then convert once)
    let ncomp = if gray || hs == 2 { 1 } else { 3 };
    let (cw, ch) = (width.div_ceil(hs), height.div_ceil(vs));
    let convert = |rows: std::ops::Range<usize>, crows: std::ops::Range<usize>| -> Vec<Plane> {
        let mut full: Vec<Vec<f32>> = (0..ncomp).map(|_| vec![0f32; rows.len() * width]).collect();
        for (k, y) in rows.clone().enumerate() {
            let src = &data[y * width * channels..(y + 1) * width * channels];
            let mut out: Vec<&mut [f32]> = full.iter_mut().map(|p| &mut p[k * width..(k + 1) * width]).collect();
            if gray {
                for (o, p) in out[0].iter_mut().zip(src.chunks_exact(channels)) {
                    *o = p[0] as f32 - 128.0;
                }
                continue;
            }
            if out.len() == 1 {
                for (o, p) in out[0].iter_mut().zip(src.chunks_exact(channels)) {
                    *o = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32 - 128.0;
                }
                continue;
            }
            for (x, p) in src.chunks_exact(channels).enumerate() {
                let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
                out[0][x] = 0.299 * r + 0.587 * g + 0.114 * b - 128.0;
                out[1][x] = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
                out[2][x] = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
            }
        }
        let mut planes: Vec<Plane> = full.into_iter().map(|data| Plane { w: width, h: rows.len(), data }).collect();
        if !gray && hs == 2 {
            let (mut cb, mut cr) = (vec![0f32; cw * crows.len()], vec![0f32; cw * crows.len()]);
            for (cy, (ob, or)) in crows.clone().zip(cb.chunks_mut(cw).zip(cr.chunks_mut(cw))) {
                for cx in 0..cw {
                    let s = |k: usize| {
                        let mut sum = 0.0;
                        for dy in 0..vs {
                            for dx in 0..hs {
                                let y = (cy * vs + dy).min(height - 1);
                                let x = (cx * hs + dx).min(width - 1);
                                sum += data[(y * width + x) * channels + k] as f32;
                            }
                        }
                        sum / (hs * vs) as f32
                    };
                    let (r, g, b) = (s(0), s(1), s(2));
                    ob[cx] = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
                    or[cx] = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
                }
            }
            planes.push(Plane { w: cw, h: crows.len(), data: cb });
            planes.push(Plane { w: cw, h: crows.len(), data: cr });
        }
        planes
    };

    let ql = scaled_quant(&Q_LUMA, quality);
    let qc = scaled_quant(&Q_CHROMA, quality);
    let (rl, rc) = (recip_table(&ql), recip_table(&qc));
    let (mcu_w, mcu_h) = (8 * hs, 8 * vs);
    let (mx, my) = (width.div_ceil(mcu_w), height.div_ceil(mcu_h));
    // restart interval: whole MCU rows per band, ≤ 65535 MCUs, enough bands to keep all cores busy
    let rows_per_band = (my / 64).clamp(1, (65_535 / mx.max(1)).max(1));
    let bands: Vec<usize> = (0..my.div_ceil(rows_per_band)).collect();
    // first pass: each band's quantized blocks as symbols, with their counts per table (DC
    // prediction restarts per band, as the restart markers say)
    let tokenize_band = |band: &usize| -> (Vec<Token>, [Freq; 4]) {
        let (mut blk, mut q) = ([0f32; 64], [0i16; 64]);
        let (mut tokens, mut freq) = (Vec::new(), [[0u64; 256]; 4]);
        let mut pred = [0i32; 3];
        let (m0, m1) = (band * rows_per_band, ((band + 1) * rows_per_band).min(my));
        // the band's planes, its MCU row `m0` at their top (blocks past the image's bottom repeat its last row,
        // which is the band's last row then)
        let planes = convert(m0 * mcu_h..(m1 * mcu_h).min(height), m0 * 8..(m1 * 8).min(ch));
        for mcu_y in m0..m1 {
            let mcu_y = mcu_y - m0;
            for mcu_x in 0..mx {
                for vy in 0..vs {
                    for hx in 0..hs {
                        planes[0].block(mcu_x * hs + hx, mcu_y * vs + vy, &mut blk);
                        fdct_quant(&blk, &rl, &mut q);
                        tokenize_block(&q, &mut pred[0], false, &mut tokens, &mut freq);
                    }
                }
                if !gray {
                    for (plane, pred) in planes.iter().zip(pred.iter_mut()).take(3).skip(1) {
                        plane.block(mcu_x, mcu_y, &mut blk);
                        fdct_quant(&blk, &rc, &mut q);
                        tokenize_block(&q, pred, true, &mut tokens, &mut freq);
                    }
                }
            }
        }
        (tokens, freq)
    };
    #[cfg(feature = "parallel")]
    let tokenized: Vec<(Vec<Token>, [Freq; 4])> = bands.par_iter().map(tokenize_band).collect();
    #[cfg(not(feature = "parallel"))]
    let tokenized: Vec<(Vec<Token>, [Freq; 4])> = bands.iter().map(tokenize_band).collect();
    // the tables: DC luma, AC luma, DC chroma, AC chroma
    let tables: [([u8; 16], Vec<u8>); 4] = if optimize {
        let mut total = [[0u64; 256]; 4];
        for (_, f) in &tokenized {
            for (t, f) in total.iter_mut().zip(f) {
                for (t, f) in t.iter_mut().zip(f) {
                    *t = t.saturating_add(*f);
                }
            }
        }
        total.each_ref().map(optimal_table)
    } else {
        [
            (DC_LUMA_BITS, DC_VALS.to_vec()),
            (AC_LUMA_BITS, AC_LUMA_VALS.to_vec()),
            (DC_CHROMA_BITS, DC_VALS.to_vec()),
            (AC_CHROMA_BITS, AC_CHROMA_VALS.to_vec()),
        ]
    };
    let huff = tables.each_ref().map(|(bits, vals)| Huff::new(bits, vals));
    // second pass: code the symbols
    #[cfg(feature = "parallel")]
    let coded: Vec<Vec<u8>> = tokenized.into_par_iter().map(|(t, _)| write_tokens(&t, &huff)).collect();
    #[cfg(not(feature = "parallel"))]
    let coded: Vec<Vec<u8>> = tokenized.into_iter().map(|(t, _)| write_tokens(&t, &huff)).collect();

    // headers
    let mut out = Vec::with_capacity(coded.iter().map(Vec::len).sum::<usize>() + 4096);
    out.extend_from_slice(&[0xFF, 0xD8]);
    // JFIF APP0 first (callers may pass their own, e.g. with a print density)
    if !app_segments.iter().any(|(m, _)| *m == 0xE0) {
        segment(&mut out, 0xE0, &[b'J', b'F', b'I', b'F', 0, 1, 2, 0, 0, 1, 0, 1, 0, 0]);
    }
    for (marker, payload) in app_segments {
        segment(&mut out, *marker, payload);
    }
    let dqt = |id: u8, q: &[u16; 64]| -> Vec<u8> {
        let mut p = vec![id];
        p.extend(ZIGZAG.iter().map(|&i| q[i] as u8));
        p
    };
    segment(&mut out, 0xDB, &dqt(0, &ql));
    if !gray {
        segment(&mut out, 0xDB, &dqt(1, &qc));
    }
    let mut sof = vec![8];
    sof.extend_from_slice(&(height as u16).to_be_bytes());
    sof.extend_from_slice(&(width as u16).to_be_bytes());
    if gray {
        sof.extend_from_slice(&[1, 1, 0x11, 0]);
    } else {
        sof.extend_from_slice(&[3, 1, ((hs as u8) << 4) | vs as u8, 0, 2, 0x11, 1, 3, 0x11, 1]);
    }
    segment(&mut out, 0xC0, &sof);
    let [(dlb, dlv), (alb, alv), (dcb, dcv), (acb, acv)] = &tables;
    dht(&mut out, 0x00, dlb, dlv);
    dht(&mut out, 0x10, alb, alv);
    if !gray {
        dht(&mut out, 0x01, dcb, dcv);
        dht(&mut out, 0x11, acb, acv);
    }
    let interval = (rows_per_band * mx) as u16;
    if coded.len() > 1 {
        segment(&mut out, 0xDD, &interval.to_be_bytes());
    }
    if gray {
        segment(&mut out, 0xDA, &[1, 1, 0x00, 0, 63, 0]);
    } else {
        segment(&mut out, 0xDA, &[3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);
    }
    let last = coded.len().saturating_sub(1);
    for (i, c) in coded.iter().enumerate() {
        out.extend_from_slice(c);
        if i < last {
            out.extend_from_slice(&[0xFF, 0xD0 + (i % 8) as u8]);
        }
    }
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn hostile_dimensions_give_no_output_instead_of_a_panic_or_allocation() {
        for (w, h, c) in [(1usize << 40, 1usize << 40, 3usize), (0, 8, 3), (70_000, 2, 3), (8, 8, 7), (8, 8, 3)] {
            assert!(encode(&[0u8; 16], w, h, c, 90, ChromaSubsampling::S444, &[]).is_empty(), "{w} x {h} x {c}");
        }
    }

    #[test]
    fn checked_entry_point_reports_invalid_input() {
        assert!(matches!(encode_baseline(&[0; 16], 8, 8, 3, 90, ChromaSubsampling::S444, &[]), Err(EncodeError::InvalidInput(_))));
        assert!(matches!(
            encode_baseline(&[0; 192], 8, 8, 3, 90, ChromaSubsampling::S422, &[]),
            Ok(bytes) if bytes.starts_with(&[0xFF, 0xD8])
        ));
        let too_large_app = [(0xE1, vec![0; 65_534])];
        assert!(matches!(encode_baseline(&[0; 192], 8, 8, 3, 90, ChromaSubsampling::S444, &too_large_app), Err(EncodeError::InvalidInput(_))));
    }

    #[test]
    fn requested_chroma_subsampling_is_written_in_the_frame_header() {
        let src = test_rgb(19, 9);
        for (sub, expected) in [(ChromaSubsampling::S444, 0x11), (ChromaSubsampling::S422, 0x21), (ChromaSubsampling::S420, 0x22)] {
            let jpeg = encode(&src, 19, 9, 3, 90, sub, &[]);
            let sof = jpeg.windows(2).position(|w| w == [0xFF, 0xC0]).expect("SOF0 marker");
            assert_eq!(jpeg.get(sof + 11), Some(&expected), "{sub:?}");
        }
    }

    fn test_rgb(w: usize, h: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let t = ((x as f32 / 13.0).sin() * 0.5 + 0.5) * 255.0;
                v.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, t as u8]);
            }
        }
        v
    }

    fn psnr(a: &[u8], b: &[u8]) -> f64 {
        let mse = a.iter().zip(b).map(|(x, y)| (*x as f64 - *y as f64).powi(2)).sum::<f64>() / a.len() as f64;
        10.0 * (255.0f64.powi(2) / mse.max(1e-9)).log10()
    }

    /// The AAN path matches the direct DCT-II definition.
    #[test]
    fn aan_matches_reference_dct() {
        let block: [f32; 64] = std::array::from_fn(|i| ((i * 37 % 255) as f32) - 128.0);
        let q = [1u16; 64];
        let mut out = [0i16; 64];
        fdct_quant(&block, &recip_table(&q), &mut out);
        for v in 0..8 {
            for u in 0..8 {
                let c = |k: usize| if k == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                let mut s = 0.0;
                for y in 0..8 {
                    for x in 0..8 {
                        s += block[y * 8 + x] as f64
                            * (((2 * x + 1) * u) as f64 * std::f64::consts::PI / 16.0).cos()
                            * (((2 * y + 1) * v) as f64 * std::f64::consts::PI / 16.0).cos();
                    }
                }
                let f = 0.25 * c(u) * c(v) * s;
                assert!((out[v * 8 + u] as f64 - f).abs() <= 0.51, "({u},{v}): {} vs {f}", out[v * 8 + u]);
            }
        }
    }

    #[test]
    fn huffman_tables_are_consistent() {
        for (bits, n) in [(&AC_LUMA_BITS, AC_LUMA_VALS.len()), (&AC_CHROMA_BITS, AC_CHROMA_VALS.len()), (&DC_LUMA_BITS, 12), (&DC_CHROMA_BITS, 12)] {
            assert_eq!(bits.iter().map(|&b| b as usize).sum::<usize>(), n);
        }
    }

    #[test]
    fn decodes_with_independent_decoders_at_odd_sizes() {
        for (w, h, sub) in [
            (1, 1, ChromaSubsampling::S444),
            (37, 23, ChromaSubsampling::S420),
            (37, 23, ChromaSubsampling::S422),
            (640, 1031, ChromaSubsampling::S420),
            (803, 600, ChromaSubsampling::S444),
        ] {
            let src = test_rgb(w, h);
            let jpg = encode(&src, w, h, 3, 92, sub, &[]);
            let mut d = jpeg_decoder::Decoder::new(&jpg[..]);
            let px = d.decode().expect("jpeg-decoder");
            let info = d.info().unwrap();
            assert_eq!((info.width as usize, info.height as usize), (w, h));
            if w * h > 64 {
                assert!(psnr(&src, &px) > 30.0, "{w}x{h} {sub:?}: psnr {}", psnr(&src, &px));
            }
            let z = zune_jpeg::JpegDecoder::new(zune_jpeg::zune_core::bytestream::ZCursor::new(&jpg[..])).decode().expect("zune-jpeg");
            assert_eq!(z.len(), px.len());
        }
    }

    /// The code lengths of `(bits, vals)`, checked: every listed symbol once, at most 16 bits,
    /// and room left in the code space (the all-ones code is never assigned, Kraft sum < 1).
    fn check_table(bits: &[u8; 16], vals: &[u8]) {
        assert_eq!(bits.iter().map(|&b| b as usize).sum::<usize>(), vals.len());
        let mut seen = [false; 256];
        for &v in vals {
            assert!(!seen[v as usize], "symbol {v} listed twice");
            seen[v as usize] = true;
        }
        let kraft: f64 = bits.iter().enumerate().map(|(i, &n)| n as f64 / 2f64.powi(i as i32 + 1)).sum();
        assert!(kraft < 1.0, "code space full: {kraft}");
        let h = Huff::new(bits, vals);
        for &v in vals {
            let (c, l) = h.code[v as usize];
            assert!((1..=16).contains(&l));
            assert_ne!(c as u32, (1u32 << l) - 1, "symbol {v} has the all-ones code");
        }
    }

    #[test]
    fn optimal_tables_are_valid_and_bounded() {
        // one symbol, two, a typical spread and every symbol
        let mut one = [0u64; 256];
        one[7] = 5;
        let mut two = [0u64; 256];
        two[0] = 1;
        two[0xF0] = 1_000_000;
        let typical: Freq = std::array::from_fn(|s| if s % 3 == 0 { (s as u64 * 7919) % 1000 + 1 } else { 0 });
        let all: Freq = std::array::from_fn(|s| s as u64 + 1);
        // Fibonacci counts make an unbounded Huffman tree ~40 levels deep: the 16-bit limit applies
        let mut fib = [0u64; 256];
        let (mut a, mut b) = (1u64, 1u64);
        for f in fib.iter_mut().take(40) {
            *f = a;
            (a, b) = (b, a + b);
        }
        for (name, f) in [("one", one), ("two", two), ("typical", typical), ("all", all), ("fib", fib), ("none", [0; 256])] {
            let (bits, vals) = optimal_table(&f);
            check_table(&bits, &vals);
            for (s, &count) in f.iter().enumerate() {
                assert_eq!(vals.contains(&(s as u8)), count > 0 || (name == "none" && s == 0), "{name}: symbol {s}");
            }
        }
        // more frequent symbols never get longer codes
        let (bits, vals) = optimal_table(&typical);
        let h = Huff::new(&bits, &vals);
        for &x in &vals {
            for &y in &vals {
                if typical[x as usize] > typical[y as usize] {
                    assert!(h.code[x as usize].1 <= h.code[y as usize].1, "{x} vs {y}");
                }
            }
        }
        // the Annex K example tables pass the same checks
        check_table(&AC_LUMA_BITS, &AC_LUMA_VALS);
        check_table(&DC_CHROMA_BITS, &DC_VALS);
    }

    /// Issue #1373: tables made for the image give a smaller file with exactly the same pixels as
    /// the Annex K example tables (the quantized coefficients are the same), at every quality and
    /// subsampling, and both decoders read it.
    #[test]
    fn optimized_tables_shrink_files_without_changing_pixels() {
        let (w, h) = (517, 333);
        // gradients plus deterministic noise, like a photo's grain
        let mut seed = 0x2545_f491_u32;
        let src: Vec<u8> = test_rgb(w, h)
            .into_iter()
            .map(|v| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                v.saturating_add((seed % 23) as u8)
            })
            .collect();
        for (q, sub) in [
            (100, ChromaSubsampling::S444),
            (92, ChromaSubsampling::S444),
            (75, ChromaSubsampling::S420),
            (30, ChromaSubsampling::S420),
            (85, ChromaSubsampling::S422),
        ] {
            let std = encode_with(&src, w, h, 3, q, sub, &[], false);
            let opt = encode(&src, w, h, 3, q, sub, &[]);
            assert!(opt.len() < std.len(), "q{q} {sub:?}: {} ≥ {}", opt.len(), std.len());
            let a = jpeg_decoder::Decoder::new(&std[..]).decode().expect("standard tables");
            let b = jpeg_decoder::Decoder::new(&opt[..]).decode().expect("optimized tables");
            assert_eq!(a, b, "q{q} {sub:?}: pixels differ");
            let z = zune_jpeg::JpegDecoder::new(zune_jpeg::zune_core::bytestream::ZCursor::new(&opt[..])).decode().expect("zune-jpeg");
            assert_eq!(z.len(), b.len());
        }
        // grayscale too
        let gray: Vec<u8> = src.iter().step_by(3).copied().collect();
        let std = encode_with(&gray, w, h, 1, 90, ChromaSubsampling::S444, &[], false);
        let opt = encode(&gray, w, h, 1, 90, ChromaSubsampling::S444, &[]);
        assert!(opt.len() < std.len());
        assert_eq!(jpeg_decoder::Decoder::new(&std[..]).decode().unwrap(), jpeg_decoder::Decoder::new(&opt[..]).decode().unwrap());
    }

    /// The bytes written for 4:4:4, 4:2:0, grayscale and RGBA input at a size cut into many bands, with partial MCUs
    /// at the right and bottom edges, pinned: each band converts its own rows to YCbCr, which must not change a bit.
    #[test]
    fn encoded_bytes_are_pinned() {
        let fnv = |b: &[u8]| b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3));
        let (w, h) = (1203, 1037);
        let rgb = test_rgb(w, h);
        let gray: Vec<u8> = rgb.as_chunks::<3>().0.iter().map(|p| p[1]).collect();
        let rgba: Vec<u8> = rgb.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        let got = [
            fnv(&encode(&rgb, w, h, 3, 90, ChromaSubsampling::S444, &[])),
            fnv(&encode(&rgb, w, h, 3, 75, ChromaSubsampling::S420, &[])),
            fnv(&encode(&gray, w, h, 1, 90, ChromaSubsampling::S444, &[])),
            fnv(&encode(&rgba, w, h, 4, 85, ChromaSubsampling::S444, &[])),
        ];
        assert_eq!(got, PINNED, "{got:#x?}");
    }
    /// [`round_i16`] against `f32::round` at the halves, the ends of `i16`, NaN and infinities, and a spread of every
    /// magnitude (all 2^32 inputs were compared once, offline).
    #[test]
    fn round_i16_is_round_as_i16() {
        let special = [0.5f32, -0.5, 1.5, -1.5, 2.5, -2.5, 0.499_999_97, -0.499_999_97, 32767.5, -32768.5, 32766.5, 1e9, -1e9];
        let all = special.into_iter().chain([f32::NAN, f32::INFINITY, f32::NEG_INFINITY]).chain((0..=u32::MAX).step_by(9973).map(f32::from_bits));
        for x in all {
            assert_eq!(round_i16(x), x.round() as i16, "{x:e}");
        }
    }
    const PINNED: [u64; 4] = [0x52ba_772a_589c_1430, 0x013f_8364_821f_1dde, 0xb2a3_b5e5_aef3_7402, 0x08df_d6dc_aa2f_1c78];
    #[test]
    fn grayscale_and_quality_monotone() {
        let (w, h) = (300, 200);
        let src: Vec<u8> = (0..w * h).map(|i| ((i % w) * 255 / w) as u8 ^ ((i / w) as u8 & 0x10)).collect();
        let lo = encode(&src, w, h, 1, 30, ChromaSubsampling::S444, &[]);
        let hi = encode(&src, w, h, 1, 95, ChromaSubsampling::S444, &[]);
        assert!(lo.len() < hi.len());
        let px = jpeg_decoder::Decoder::new(&hi[..]).decode().unwrap();
        assert!(psnr(&src, &px) > 35.0);
    }
}
