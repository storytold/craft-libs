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
    fn flush_pad(&mut self) {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
    }
    /// Pad with 1-bits to a byte boundary and return the coded bytes.
    fn finish(mut self) -> Vec<u8> {
        self.flush_pad();
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
    validate_encode(data, width, height, channels, app_segments)?;
    Ok(encode(data, width, height, channels, quality, sub, app_segments))
}

/// Encode the same optimized baseline JPEG as [`encode_baseline`], trading a second DCT pass
/// for working memory independent of image height. This path is sequential even when the
/// `parallel` feature is enabled. It retains only one MCU row of conversion planes and writes
/// directly into the returned encoded buffer: no image-wide tokens or duplicate coded buffer.
///
/// Scratch consists of at most `24 * width + 8` floats (odd-width 4:2:0), plus fixed-size coding tables.
/// The caller still owns the input pixels and the returned JPEG; this is not a cap on output
/// size or a replacement for the caller's image/admission budget. Geometry, quality, metadata,
/// Huffman tables and restart boundaries match [`encode_baseline`] byte for byte.
#[allow(clippy::too_many_arguments)]
pub fn encode_baseline_low_memory(
    data: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    quality: u8,
    sub: ChromaSubsampling,
    app_segments: &[(u8, Vec<u8>)],
) -> Result<Vec<u8>, EncodeError> {
    validate_encode(data, width, height, channels, app_segments)?;
    Ok(encode_impl(data, width, height, channels, quality, sub, app_segments, true, true))
}

fn validate_encode(data: &[u8], width: usize, height: usize, channels: usize, app_segments: &[(u8, Vec<u8>)]) -> Result<(), EncodeError> {
    let fits = width.checked_mul(height).and_then(|n| n.checked_mul(channels)).is_some_and(|n| n <= data.len());
    if !fits || width == 0 || height == 0 || width > 65_535 || height > 65_535 || !matches!(channels, 1 | 3 | 4) {
        return Err(EncodeError::InvalidInput(format!("{width}x{height}, {channels} channels, {} bytes", data.len())));
    }
    if app_segments.iter().any(|(marker, payload)| !(0xE0..=0xEF).contains(marker) || payload.len() > 65_533) {
        return Err(EncodeError::InvalidInput("APP markers must be 0xE0..=0xEF with payloads no larger than 65533 bytes".into()));
    }
    Ok(())
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
    encode_impl(data, width, height, channels, quality, sub, app_segments, optimize, false)
}

#[allow(clippy::too_many_arguments)]
fn encode_impl(
    data: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    quality: u8,
    sub: ChromaSubsampling,
    app_segments: &[(u8, Vec<u8>)],
    optimize: bool,
    low_memory: bool,
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
        #[cfg(test)]
        low_memory_tests::record_planes(planes.iter().map(|p| p.data.len()).sum());
        planes
    };

    let ql = scaled_quant(&Q_LUMA, quality);
    let qc = scaled_quant(&Q_CHROMA, quality);
    let (rl, rc) = (recip_table(&ql), recip_table(&qc));
    let (mcu_w, mcu_h) = (8 * hs, 8 * vs);
    let (mx, my) = (width.div_ceil(mcu_w), height.div_ceil(mcu_h));
    // restart interval: whole MCU rows per band, ≤ 65535 MCUs, enough bands to keep all cores busy
    let rows_per_band = (my / 64).clamp(1, (65_535 / mx.max(1)).max(1));
    let num_bands = my.div_ceil(rows_per_band);
    let bands: Vec<usize> = if low_memory { Vec::new() } else { (0..num_bands).collect() };
    // first pass: each band's quantized blocks as symbols, with their counts per table (DC
    // prediction restarts per band, as the restart markers say)
    let visit_band = |band: &usize, emit: &mut dyn FnMut(&[i16; 64], &mut i32, bool)| {
        let (mut blk, mut q) = ([0f32; 64], [0i16; 64]);
        let mut pred = [0i32; 3];
        let (m0, m1) = (band * rows_per_band, ((band + 1) * rows_per_band).min(my));
        for mcu_y in m0..m1 {
            // Convert only the current MCU row. DC predictors still span the original band,
            // so its symbols/restart decisions are identical to whole-band conversion.
            let planes = convert(mcu_y * mcu_h..((mcu_y + 1) * mcu_h).min(height), mcu_y * 8..((mcu_y + 1) * 8).min(ch));
            for mcu_x in 0..mx {
                for vy in 0..vs {
                    for hx in 0..hs {
                        planes[0].block(mcu_x * hs + hx, vy, &mut blk);
                        fdct_quant(&blk, &rl, &mut q);
                        emit(&q, &mut pred[0], false);
                    }
                }
                if !gray {
                    for (plane, pred) in planes.iter().zip(pred.iter_mut()).take(3).skip(1) {
                        plane.block(mcu_x, 0, &mut blk);
                        fdct_quant(&blk, &rc, &mut q);
                        emit(&q, pred, true);
                    }
                }
            }
        }
    };
    let tokenize_band = |band: &usize| -> (Vec<Token>, [Freq; 4]) {
        let (mut tokens, mut freq) = (Vec::new(), [[0u64; 256]; 4]);
        visit_band(band, &mut |q, pred, chroma| tokenize_block(q, pred, chroma, &mut tokens, &mut freq));
        #[cfg(test)]
        low_memory_tests::record_tokens(tokens.len());
        (tokens, freq)
    };
    #[cfg(feature = "parallel")]
    let tokenized: Vec<(Vec<Token>, [Freq; 4])> = if low_memory { Vec::new() } else { bands.par_iter().map(tokenize_band).collect() };
    #[cfg(not(feature = "parallel"))]
    let tokenized: Vec<(Vec<Token>, [Freq; 4])> = if low_memory { Vec::new() } else { bands.iter().map(tokenize_band).collect() };
    // the tables: DC luma, AC luma, DC chroma, AC chroma
    let tables: [([u8; 16], Vec<u8>); 4] = if optimize {
        let mut total = [[0u64; 256]; 4];
        if low_memory {
            for band in 0..num_bands {
                visit_band(&band, &mut |q, pred, chroma| {
                    let base = if chroma { 2 } else { 0 };
                    block_symbols(q, pred, |ac, symbol, _, _| total[base + ac as usize][symbol & 0xFF] += 1);
                });
            }
        }
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
    let coded: Vec<Vec<u8>> = if low_memory { Vec::new() } else { tokenized.into_par_iter().map(|(t, _)| write_tokens(&t, &huff)).collect() };
    #[cfg(not(feature = "parallel"))]
    let coded: Vec<Vec<u8>> = if low_memory { Vec::new() } else { tokenized.into_iter().map(|(t, _)| write_tokens(&t, &huff)).collect() };

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
    if num_bands > 1 {
        segment(&mut out, 0xDD, &interval.to_be_bytes());
    }
    if gray {
        segment(&mut out, 0xDA, &[1, 1, 0x00, 0, 63, 0]);
    } else {
        segment(&mut out, 0xDA, &[3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);
    }
    if low_memory {
        let last = num_bands.saturating_sub(1);
        for i in 0..num_bands {
            // Continue directly in the final buffer, resetting entropy bits at the same band
            // boundaries as the retained-token path. Only the output grows with image height.
            let mut bits = Bits { out, acc: 0, n: 0 };
            visit_band(&i, &mut |q, pred, chroma| {
                let base = if chroma { 2 } else { 0 };
                block_symbols(q, pred, |ac, symbol, extra, n| {
                    let (code, len) = huff[base + ac as usize].code[symbol & 0xFF];
                    bits.put(code as u32, len as u32);
                    bits.put(extra & 0xFFFF, n & 0xF);
                });
            });
            out = bits.finish();
            if i < last {
                out.extend_from_slice(&[0xFF, 0xD0 + (i % 8) as u8]);
            }
        }
    } else {
        let last = coded.len().saturating_sub(1);
        for (i, c) in coded.iter().enumerate() {
            out.extend_from_slice(c);
            if i < last {
                out.extend_from_slice(&[0xFF, 0xD0 + (i % 8) as u8]);
            }
        }
    }
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

#[derive(Clone, Copy)]
struct SofComp {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
}

struct SofInfo {
    width: usize,
    height: usize,
    comps: Vec<SofComp>,
}

#[derive(Clone, Copy)]
struct ScanComp {
    blocks: usize,
    td: usize,
    ta: usize,
}

struct OptimalTable {
    bits: [u8; 16],
    vals: Vec<u8>,
    huff: Huff,
}

struct BaselineHeaders {
    /// Byte offset where the rewritten optimal `DHT` segments are emitted: the start of the first
    /// source `DHT`, or the `SOS` marker when the stream carries no `DHT` at all.
    dht_insert_offset: usize,
    /// Total bytes of the segments copied verbatim into the output, so the writer can reserve
    /// space proportional to the retained payload instead of the raw input length.
    retained_meta_len: usize,
    dc_dec: [Option<super::Huffman>; 4],
    ac_dec: [Option<super::Huffman>; 4],
    restart: usize,
    total_mcus: usize,
    sos_range: (usize, usize),
    scan_comps: Vec<ScanComp>,
    dc_used: [bool; 4],
    ac_used: [bool; 4],
}

fn is_valid_baseline_symbol(tc: u8, sym: u8) -> bool {
    if tc == 0 {
        // ITU-T T.81 Table F.1: 8-bit Baseline DC categories are 0..=11.
        sym <= 11
    } else {
        // ITU-T T.81 Table F.2: 8-bit Baseline AC symbols are EOB (0x00), ZRL (0xF0),
        // or a zero-run with non-zero magnitude category SSSS in 1..=10.
        sym == 0x00 || sym == 0xF0 || (1..=10).contains(&(sym & 0x0F))
    }
}

type RawHuffTables<'a> = [Option<(&'a [u8], &'a [u8])>; 4];

/// Validates both prefix-tree Kraft balance at every code length `1..=16` and strict unused
/// 16-bit code space (`sum < 65536`), rejecting all-ones codewords forbidden by ITU-T T.81 §C.2
/// without allocating a decode LUT while still permitting single-symbol 1-bit tables (`32768`).
fn is_valid_huffman_counts(counts: &[u8]) -> bool {
    let mut code = 0u32;
    for (idx, &n) in counts.iter().enumerate() {
        let l = idx + 1;
        code += u32::from(n);
        if code > (1u32 << l) {
            return false;
        }
        code <<= 1;
    }
    (code >> 1) < 65_536
}

fn parse_dqt_tables(seg: &[u8], dqt_defined: &mut [bool; 4]) -> Option<()> {
    if seg.is_empty() {
        return None;
    }
    let mut at = 0usize;
    while at < seg.len() {
        let pq_tq = *seg.get(at)?;
        let pq = pq_tq >> 4;
        let tq = usize::from(pq_tq & 15);
        // ITU-T T.81 Table B.4 restricts Baseline SOF0 quantization tables to 8-bit precision
        // (Pq == 0) with strictly non-zero entries (1..=255).
        if pq != 0 || tq > 3 {
            return None;
        }
        let table_end = at.checked_add(1 + 64)?;
        let entries = seg.get(at + 1..table_end)?;
        if entries.contains(&0) {
            return None;
        }
        *dqt_defined.get_mut(tq)? = true;
        at = table_end;
    }
    Some(())
}

/// Validates every `DHT` definition against T.81 `SOF0` bounds while retaining only the latest
/// validated slice per slot so `Huffman::new` (512-entry LUT build) runs at most once per active
/// `SOS` slot rather than once per header definition.
fn parse_dht_tables<'a>(seg: &'a [u8], dc_raw: &mut RawHuffTables<'a>, ac_raw: &mut RawHuffTables<'a>) -> Option<()> {
    if seg.is_empty() {
        return None;
    }
    let mut at = 0usize;
    while at < seg.len() {
        let tc_th = *seg.get(at)?;
        let tc = tc_th >> 4;
        let th = usize::from(tc_th & 15);
        // ITU-T T.81 Table B.5 restricts Baseline SOF0 Huffman destinations to slots 0..=1.
        if tc > 1 || th > 1 {
            return None;
        }
        let counts = seg.get(at + 1..at + 17)?;
        let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
        if total == 0 || total > 256 || !is_valid_huffman_counts(counts) {
            return None;
        }
        let values = seg.get(at + 17..at + 17 + total)?;
        if !values.iter().all(|&v| is_valid_baseline_symbol(tc, v)) {
            return None;
        }
        // Why: libjpeg-turbo (`JERR_BAD_HUFF_TABLE`) and jpegli reject a table that assigns two
        // distinct codewords to the same `HUFFVAL`; `optimize_baseline_scan` counts frequencies per
        // symbol, so such a table would silently merge both codes into one.
        let mut seen = [false; 256];
        for &v in values {
            if std::mem::replace(&mut seen[usize::from(v)], true) {
                return None;
            }
        }
        if tc == 0 {
            *dc_raw.get_mut(th)? = Some((counts, values));
        } else {
            *ac_raw.get_mut(th)? = Some((counts, values));
        }
        at += 17 + total;
    }
    Some(())
}

fn parse_sof0(seg: &[u8]) -> Option<SofInfo> {
    if *seg.first()? != 8 {
        return None;
    }
    let height = usize::from(u16::from_be_bytes([*seg.get(1)?, *seg.get(2)?]));
    let width = usize::from(u16::from_be_bytes([*seg.get(3)?, *seg.get(4)?]));
    let nf = usize::from(*seg.get(5)?);
    if height == 0 || width == 0 || !matches!(nf, 1..=4) || seg.len() != 6 + 3 * nf {
        return None;
    }
    let mut comps = Vec::with_capacity(nf);
    for c in 0..nf {
        let id = *seg.get(6 + 3 * c)?;
        let hv = *seg.get(6 + 3 * c + 1)?;
        let tq = usize::from(*seg.get(6 + 3 * c + 2)?);
        let (h, v) = (usize::from(hv >> 4), usize::from(hv & 15));
        if !(1..=4).contains(&h) || !(1..=4).contains(&v) || tq > 3 || comps.iter().any(|existing: &SofComp| existing.id == id) {
            return None;
        }
        comps.push(SofComp { id, h, v, tq });
    }
    Some(SofInfo { width, height, comps })
}

fn parse_interleaved_sos(
    seg: &[u8],
    sof: &SofInfo,
    dc_raw: &RawHuffTables<'_>,
    ac_raw: &RawHuffTables<'_>,
    dqt_defined: &[bool; 4],
) -> Option<(Vec<ScanComp>, [bool; 4], [bool; 4])> {
    let ns = usize::from(*seg.first()?);
    if ns == 0 || ns != sof.comps.len() || seg.len() != 1 + 2 * ns + 3 {
        return None;
    }
    if seg.get(1 + 2 * ns..1 + 2 * ns + 3) != Some(&[0, 63, 0]) {
        return None;
    }
    let mut scan_comps = Vec::with_capacity(ns);
    let mut dc_used = [false; 4];
    let mut ac_used = [false; 4];
    let mut mcu_blocks = 0usize;
    for (s, comp) in sof.comps.iter().enumerate() {
        let cs = *seg.get(1 + 2 * s)?;
        // ITU-T T.81 §B.2.3 requires components in a full-frame interleaved scan to appear in the
        // exact order declared in SOF0; permitting permutations would emit invalid SOS headers.
        if cs != comp.id || !*dqt_defined.get(comp.tq)? {
            return None;
        }
        let tbl = *seg.get(2 + 2 * s)?;
        let (td, ta) = (usize::from(tbl >> 4), usize::from(tbl & 15));
        if td > 1 || ta > 1 || dc_raw.get(td)?.is_none() || ac_raw.get(ta)?.is_none() {
            return None;
        }
        let blocks = if ns == 1 { 1 } else { comp.h * comp.v };
        mcu_blocks = mcu_blocks.checked_add(blocks)?;
        scan_comps.push(ScanComp { blocks, td, ta });
        *dc_used.get_mut(td)? = true;
        *ac_used.get_mut(ta)? = true;
    }
    // ITU-T T.81 §A.2.3 limits interleaved MCUs to at most 10 data units (8x8 blocks).
    if ns > 1 && mcu_blocks > 10 {
        return None;
    }
    Some((scan_comps, dc_used, ac_used))
}

fn parse_baseline_headers(jpeg: &[u8], body_end: usize) -> Option<BaselineHeaders> {
    let mut pos = 2usize;
    let mut dht_insert_pos: Option<usize> = None;
    let mut retained_meta_len = 0usize;
    let mut dqt_defined = [false; 4];
    let mut dc_raw: RawHuffTables<'_> = [None, None, None, None];
    let mut ac_raw: RawHuffTables<'_> = [None, None, None, None];
    let mut sof_info: Option<SofInfo> = None;
    let mut restart = 0usize;
    let mut sos_range: Option<(usize, usize)> = None;
    let mut scan_params: Option<(Vec<ScanComp>, [bool; 4], [bool; 4])> = None;

    while pos < body_end {
        if *jpeg.get(pos)? != 0xFF {
            return None;
        }
        let marker = *jpeg.get(pos + 1)?;
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        if matches!(marker, 0x01 | 0xD0..=0xD9) {
            return None;
        }
        let len = usize::from(u16::from_be_bytes([*jpeg.get(pos + 2)?, *jpeg.get(pos + 3)?]));
        if len < 2 {
            return None;
        }
        let seg_end = pos.checked_add(2 + len)?;
        if seg_end > body_end {
            return None;
        }
        let seg = jpeg.get(pos + 4..seg_end)?;
        match marker {
            0xDA => {
                let sof = sof_info.as_ref()?;
                scan_params = Some(parse_interleaved_sos(seg, sof, &dc_raw, &ac_raw, &dqt_defined)?);
                sos_range = Some((pos, seg_end));
                break;
            }
            0xDB => parse_dqt_tables(seg, &mut dqt_defined)?,
            0xC0 => {
                if sof_info.is_some() {
                    return None;
                }
                sof_info = Some(parse_sof0(seg)?);
            }
            0xC4 => {
                if dht_insert_pos.is_none() {
                    dht_insert_pos = Some(pos);
                }
                parse_dht_tables(seg, &mut dc_raw, &mut ac_raw)?;
            }
            0xDD => {
                if seg.len() != 2 {
                    return None;
                }
                restart = usize::from(u16::from_be_bytes([seg[0], seg[1]]));
            }
            // Accept only pre-scan metadata (APPn, COM); reject DNL (0xDC) and all non-baseline
            // markers so non-conforming segments are never copied unchanged into the output. The
            // retained bytes are re-read from the validated prefix while writing, so no per-segment
            // range vector grows with the attacker-controlled segment count.
            0xE0..=0xEF | 0xFE => {}
            _ => return None,
        }
        if is_retained_metadata_marker(marker) {
            retained_meta_len = retained_meta_len.saturating_add(seg_end - pos);
        }
        pos = seg_end;
    }

    let sof = sof_info?;
    let sos_range = sos_range?;
    let (scan_comps, dc_used, ac_used) = scan_params?;
    let mut dc_dec: [Option<super::Huffman>; 4] = [None, None, None, None];
    let mut ac_dec: [Option<super::Huffman>; 4] = [None, None, None, None];
    for th in 0..2 {
        if *dc_used.get(th)? {
            let (counts, values) = (*dc_raw.get(th)?)?;
            *dc_dec.get_mut(th)? = Some(super::Huffman::new(counts, values).ok()?);
        }
        if *ac_used.get(th)? {
            let (counts, values) = (*ac_raw.get(th)?)?;
            *ac_dec.get_mut(th)? = Some(super::Huffman::new(counts, values).ok()?);
        }
    }

    let total_mcus = if scan_comps.len() == 1 {
        sof.width.div_ceil(8).checked_mul(sof.height.div_ceil(8))?
    } else {
        let hmax = sof.comps.iter().map(|c| c.h).max()?;
        let vmax = sof.comps.iter().map(|c| c.v).max()?;
        sof.width.div_ceil(8 * hmax).checked_mul(sof.height.div_ceil(8 * vmax))?
    };

    Some(BaselineHeaders {
        dht_insert_offset: dht_insert_pos.unwrap_or(sos_range.0),
        retained_meta_len,
        dc_dec,
        ac_dec,
        restart,
        total_mcus,
        sos_range,
        scan_comps,
        dc_used,
        ac_used,
    })
}

enum ScanEvent {
    Sym { is_ac: bool, tbl: usize, sym: u8, extra: u32, extra_len: u32 },
    Restart(usize),
}

fn decode_block(
    bits: &mut super::Bits<'_>,
    dc: &super::Huffman,
    ac: &super::Huffman,
    comp: ScanComp,
    dc_pred: &mut i32,
    on_event: &mut impl FnMut(ScanEvent),
) -> Option<()> {
    let s = dc.decode(bits).ok()?;
    if bits.exhausted || s > 11 {
        return None;
    }
    let s_bits = u32::from(s);
    let extra = bits.get(s_bits);
    if bits.exhausted {
        return None;
    }
    let dc_val = dc_pred.checked_add(super::extend(extra, s_bits))?;
    // ITU-T T.81 §F.1.1.5.1 / Table F.1 restricts reconstructed 8-bit SOF0 DC values to
    // [-1024, 1023]; checking each diff alone would admit out-of-range cumulative predictors.
    if !(-1024..=1023).contains(&dc_val) {
        return None;
    }
    *dc_pred = dc_val;
    on_event(ScanEvent::Sym { is_ac: false, tbl: comp.td, sym: s, extra, extra_len: s_bits });

    let mut k = 1usize;
    // Why: ITU-T T.81 §F.1.2.2.3 writes ZRL runs only when a nonzero coefficient follows, and EOB
    // already covers every remaining zero. Holding the ZRLs lets a sloppy `ZRL, ZRL, ZRL, EOB`
    // block be re-emitted as one EOB instead of copying the redundant run into the output.
    let mut pending_zrls = 0usize;
    while k < 64 {
        let rs = ac.decode(bits).ok()?;
        if bits.exhausted {
            return None;
        }
        let r = usize::from(rs >> 4);
        let ac_s = u32::from(rs & 15);
        if ac_s == 0 {
            if r != 15 {
                on_event(ScanEvent::Sym { is_ac: true, tbl: comp.ta, sym: rs, extra: 0, extra_len: 0 });
                break;
            }
            k += 16;
            if k > 64 {
                return None;
            }
            if k == 64 {
                // A ZRL landing on index 64 covers the last sixteen coefficients; EOB is the
                // canonical symbol for that, so any pending run collapses into it as well.
                on_event(ScanEvent::Sym { is_ac: true, tbl: comp.ta, sym: 0x00, extra: 0, extra_len: 0 });
                break;
            }
            pending_zrls += 1;
        } else {
            k += r;
            if k >= 64 {
                return None;
            }
            let ac_extra = bits.get(ac_s);
            if bits.exhausted {
                return None;
            }
            for _ in 0..pending_zrls {
                on_event(ScanEvent::Sym { is_ac: true, tbl: comp.ta, sym: 0xF0, extra: 0, extra_len: 0 });
            }
            pending_zrls = 0;
            on_event(ScanEvent::Sym { is_ac: true, tbl: comp.ta, sym: rs, extra: ac_extra, extra_len: ac_s });
            k += 1;
        }
    }
    Some(())
}

fn walk_interleaved_scan(scan_data: &[u8], headers: &BaselineHeaders, mut on_event: impl FnMut(ScanEvent)) -> Option<()> {
    let mut bits = super::Bits::new(scan_data);
    let mut rst = 0usize;
    let mut dc_pred = [0i32; 4];
    for m in 0..headers.total_mcus {
        if headers.restart > 0 && m > 0 && m % headers.restart == 0 {
            if !bits.has_valid_one_bit_padding() || bits.data_left() {
                return None;
            }
            let (next_pos, found) = super::skip_restart(scan_data, bits.pos);
            let expected = 0xD0u8.wrapping_add((rst % 8) as u8);
            if !found || next_pos == 0 || scan_data.get(next_pos - 1).copied() != Some(expected) {
                return None;
            }
            bits.restart();
            dc_pred = [0; 4];
            on_event(ScanEvent::Restart(rst));
            rst += 1;
        }
        for (comp_idx, &comp) in headers.scan_comps.iter().enumerate() {
            let dc = headers.dc_dec.get(comp.td)?.as_ref()?;
            let ac = headers.ac_dec.get(comp.ta)?.as_ref()?;
            let pred = dc_pred.get_mut(comp_idx)?;
            for _ in 0..comp.blocks {
                decode_block(&mut bits, dc, ac, comp, pred, &mut on_event)?;
            }
        }
    }
    if !bits.has_valid_one_bit_padding() || bits.data_left() || super::skip_restart(scan_data, bits.pos) != (scan_data.len(), false) {
        return None;
    }
    Some(())
}

fn build_optimal_tables(used: &[bool; 4], freqs: &[Freq; 4]) -> Option<[Option<OptimalTable>; 4]> {
    let mut tables: [Option<OptimalTable>; 4] = [None, None, None, None];
    for th in 0..4 {
        if *used.get(th)? {
            let (bits, vals) = optimal_table(freqs.get(th)?);
            let huff = Huff::new(&bits, &vals);
            *tables.get_mut(th)? = Some(OptimalTable { bits, vals, huff });
        }
    }
    Some(tables)
}

/// Returns the baseline entropy scan slice before trailing `0xFF` fill and `EOI` (`0xFF 0xD9`),
/// plus the number of bytes the writer will emit for that scan. Rejects `0xFF` fill runs before
/// `0x00` stuffed data bytes (ITU-T T.81 §B.1.1.2 permits `0xFF` fill bytes only immediately
/// before a marker) as well as unexpected or premature markers.
///
/// Why the byte count: fill bytes before `RSTn` are legal input but are never copied — the writer
/// emits the two-byte marker alone — so counting them would reserve capacity for discarded bytes.
fn strict_baseline_scan_slice(jpeg: &[u8], sos_end: usize, body_end: usize) -> Option<(&[u8], usize)> {
    let mut pos = sos_end;
    let mut retained_scan_len = 0usize;
    while pos <= body_end {
        let b = *jpeg.get(pos)?;
        if b != 0xFF {
            pos += 1;
            retained_scan_len += 1;
            continue;
        }
        let next = *jpeg.get(pos + 1)?;
        if next == 0x00 {
            // A stuffed data byte is two source bytes and two output bytes.
            pos += 2;
            retained_scan_len += 2;
            continue;
        }
        let mut marker_pos = pos + 1;
        while jpeg.get(marker_pos) == Some(&0xFF) {
            marker_pos += 1;
        }
        let marker = *jpeg.get(marker_pos)?;
        match marker {
            0xD0..=0xD7 if marker_pos < body_end => {
                pos = marker_pos + 1;
                retained_scan_len += 2;
            }
            0xD9 if marker_pos == body_end + 1 => return Some((jpeg.get(sos_end..pos)?, retained_scan_len)),
            _ => return None,
        }
    }
    None
}

/// Whether a validated pre-scan marker's segment bytes are copied verbatim into optimized output.
/// `DHT` is excluded because the tables are always rebuilt from the scan's symbol frequencies.
fn is_retained_metadata_marker(marker: u8) -> bool {
    matches!(marker, 0xDB | 0xC0 | 0xDD | 0xE0..=0xEF | 0xFE)
}

/// Writes the retained pre-scan metadata (`DQT`, `SOF0`, `DRI`, `APPn`, `COM`) by re-walking the
/// already validated header prefix, emitting `new_dhts` where the first source `DHT` used to sit.
/// Re-reading the bytes keeps auxiliary memory independent of the segment count an attacker
/// controls with thousands of empty `COM` segments.
fn copy_retained_metadata(jpeg: &[u8], header_end: usize, dht_offset: usize, new_dhts: &[u8], out: &mut Vec<u8>) -> Option<()> {
    let mut pos = 2usize;
    let mut dht_written = false;
    while pos < header_end {
        if !dht_written && pos >= dht_offset {
            out.extend_from_slice(new_dhts);
            dht_written = true;
        }
        if *jpeg.get(pos)? != 0xFF {
            return None;
        }
        let marker = *jpeg.get(pos + 1)?;
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([*jpeg.get(pos + 2)?, *jpeg.get(pos + 3)?]));
        let seg_end = pos.checked_add(2 + len)?;
        if len < 2 || seg_end > header_end {
            return None;
        }
        if is_retained_metadata_marker(marker) {
            out.extend_from_slice(jpeg.get(pos..seg_end)?);
        } else if marker != 0xC4 {
            return None;
        }
        pos = seg_end;
    }
    if !dht_written {
        out.extend_from_slice(new_dhts);
    }
    Some(())
}

/// Losslessly re-encode an 8-bit single-scan interleaved baseline JPEG (`SOF0`) with per-image
/// optimal Huffman tables (ITU-T T.81 Annex K.2), preserving the exact quantized coefficients,
/// metadata segments, and restart intervals.
///
/// Returns `None` if `jpeg` is not a complete single-scan interleaved `SOF0` stream. `DNL`
/// (`0xFF 0xDC`) streams (including `SOF0` height `0` and post-scan `DNL` markers) are
/// unsupported and return `None`.
pub fn optimize_baseline_scan(jpeg: &[u8]) -> Option<Vec<u8>> {
    if jpeg.len() < 4 || !jpeg.starts_with(&[0xFF, 0xD8]) || !jpeg.ends_with(&[0xFF, 0xD9]) {
        return None;
    }
    let body_end = jpeg.len() - 2;
    let headers = parse_baseline_headers(jpeg, body_end)?;
    let (sos_start, sos_end) = headers.sos_range;
    let (scan_data, retained_scan_len) = strict_baseline_scan_slice(jpeg, sos_end, body_end)?;
    let mut dc_freq: [Freq; 4] = [[0u64; 256]; 4];
    let mut ac_freq: [Freq; 4] = [[0u64; 256]; 4];
    walk_interleaved_scan(scan_data, &headers, |ev| {
        if let ScanEvent::Sym { is_ac, tbl, sym, .. } = ev {
            let row = if is_ac { ac_freq.get_mut(tbl) } else { dc_freq.get_mut(tbl) };
            if let Some(cell) = row.and_then(|r| r.get_mut(usize::from(sym))) {
                *cell += 1;
            }
        }
    })?;

    let dc_tables = build_optimal_tables(&headers.dc_used, &dc_freq)?;
    let ac_tables = build_optimal_tables(&headers.ac_used, &ac_freq)?;

    let mut new_dhts = Vec::with_capacity(4 * 273);
    for th in 0..4 {
        if let Some(t) = dc_tables.get(th)?.as_ref() {
            dht(&mut new_dhts, th as u8, &t.bits, &t.vals);
        }
        if let Some(t) = ac_tables.get(th)?.as_ref() {
            dht(&mut new_dhts, 0x10 | (th as u8), &t.bits, &t.vals);
        }
    }

    // Why: size the reservation from the bytes the writer is about to produce, not from the input
    // scan, whose tables may be far from optimal (a legal 16-bit-per-symbol input can shrink many
    // times over). Pass one already recorded every emitted symbol, so the exact code and amplitude
    // bits are known; padding, stuffing and restart markers get proportional slack, and the input
    // scan length stays as an upper clamp. `Vec` still grows if the estimate is short.
    let mut entropy_bits = 0u64;
    for (tables, freqs, is_ac) in [(&dc_tables, &dc_freq, false), (&ac_tables, &ac_freq, true)] {
        for (table, freq) in tables.iter().zip(freqs) {
            let Some(table) = table.as_ref() else { continue };
            for (sym, &count) in freq.iter().enumerate() {
                if count > 0 {
                    let (_, code_len) = table.huff.code[sym];
                    let extra = if is_ac { (sym & 0x0F) as u64 } else { sym as u64 };
                    entropy_bits = entropy_bits.saturating_add(count.saturating_mul(u64::from(code_len) + extra));
                }
            }
        }
    }
    let restarts = headers.total_mcus.saturating_sub(1).checked_div(headers.restart).unwrap_or(0);
    let entropy_bytes = usize::try_from(entropy_bits.div_ceil(8)).unwrap_or(usize::MAX);
    let scan_estimate =
        entropy_bytes.saturating_add(entropy_bytes / 16).saturating_add(restarts.saturating_mul(3)).saturating_add(64).min(retained_scan_len);
    let retained_len =
        headers.retained_meta_len.saturating_add(new_dhts.len()).saturating_add(scan_estimate).saturating_add(sos_end - sos_start).saturating_add(4);
    let mut out = Vec::with_capacity(retained_len);
    out.extend_from_slice(&[0xFF, 0xD8]);
    copy_retained_metadata(jpeg, sos_start, headers.dht_insert_offset, &new_dhts, &mut out)?;
    out.extend_from_slice(jpeg.get(sos_start..sos_end)?);

    let mut writer = Bits { out, acc: 0, n: 0 };
    walk_interleaved_scan(scan_data, &headers, |ev| match ev {
        ScanEvent::Restart(rst) => {
            writer.flush_pad();
            writer.out.extend_from_slice(&[0xFF, 0xD0 + (rst % 8) as u8]);
        }
        ScanEvent::Sym { is_ac, tbl, sym, extra, extra_len } => {
            let slot = if is_ac { ac_tables.get(tbl) } else { dc_tables.get(tbl) };
            if let Some(Some(t)) = slot {
                let (code, code_len) = t.huff.code[usize::from(sym)];
                writer.put(u32::from(code), u32::from(code_len));
                writer.put(extra, extra_len);
            }
        }
    })?;
    let mut out = writer.finish();
    out.extend_from_slice(&[0xFF, 0xD9]);
    Some(out)
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

    /// Locate the `nth` (0-based) header marker segment before the entropy scan.
    fn find_marker_segment(jpeg: &[u8], target_marker: u8, mut nth: usize) -> Option<(usize, usize)> {
        let mut pos = 2usize;
        while pos + 4 <= jpeg.len() {
            if jpeg[pos] != 0xFF {
                return None;
            }
            let marker = jpeg[pos + 1];
            if marker == 0xFF {
                pos += 1;
                continue;
            }
            if marker == 0xD9 {
                break;
            }
            let len = usize::from(u16::from_be_bytes([jpeg[pos + 2], jpeg[pos + 3]]));
            let seg_end = pos + 2 + len;
            if marker == target_marker {
                if nth == 0 {
                    return Some((pos, seg_end));
                }
                nth -= 1;
            }
            if marker == 0xDA {
                break;
            }
            pos = seg_end;
        }
        None
    }

    /// Re-optimizing Annex K baseline streams must produce bit-exact output to encoding directly
    /// with per-image optimal tables across 4:4:4, 4:2:2, 4:2:0, grayscale, DRI/RSTn restarts,
    /// and legal trailing 0xFF fill bytes before EOI (#2183).
    #[test]
    fn optimize_baseline_scan_matches_direct_optimal_encode() {
        for (w, h, sub) in [
            (37, 23, ChromaSubsampling::S444),
            (37, 23, ChromaSubsampling::S422),
            (37, 23, ChromaSubsampling::S420),
            (64, 1100, ChromaSubsampling::S420),
        ] {
            let src = test_rgb(w, h);
            let std_jpg = encode_with(&src, w, h, 3, 85, sub, &[], false);
            let direct_opt = encode(&src, w, h, 3, 85, sub, &[]);
            let reopt = optimize_baseline_scan(&std_jpg).expect("valid baseline scan");
            assert_eq!(reopt, direct_opt, "{w}x{h} {sub:?}");
        }

        // ITU-T T.81 §B.1.1.2 allows 0xFF fill bytes directly before EOI; trimming them keeps EOI
        // validation aligned with the end of the entropy segment without corrupting the last MCU.
        let mut padded = encode_with(&test_rgb(16, 16), 16, 16, 3, 85, ChromaSubsampling::S420, &[], false);
        let expected_padded = optimize_baseline_scan(&padded).expect("unpadded baseline optimizes");
        let eoi_pos = padded.len() - 2;
        padded.splice(eoi_pos..eoi_pos, [0xFF, 0xFF]);
        assert_eq!(optimize_baseline_scan(&padded).expect("0xFF fill before EOI optimizes"), expected_padded);

        let gray: Vec<u8> = test_rgb(37, 1100).as_chunks::<3>().0.iter().map(|p| p[1]).collect();
        let std_gray = encode_with(&gray, 37, 1100, 1, 90, ChromaSubsampling::S444, &[], false);
        let direct_gray = encode(&gray, 37, 1100, 1, 90, ChromaSubsampling::S444, &[]);
        assert_eq!(optimize_baseline_scan(&std_gray).expect("valid gray baseline"), direct_gray);
    }

    /// Reject non-baseline frames, non-interleaved scans, truncated entropy (with or without EOI),
    /// missing DQT definitions, out-of-range SOF0 Huffman table slots, and illegal DHT symbols
    /// rather than rewriting malformed or incomplete streams (#2183).
    #[test]
    fn optimize_baseline_scan_rejects_non_baseline_or_corrupt_streams() {
        assert!(optimize_baseline_scan(b"not a jpeg").is_none());
        let src = test_rgb(16, 16);
        let valid = encode_with(&src, 16, 16, 3, 80, ChromaSubsampling::S420, &[], false);

        // Truncated scan missing EOI.
        assert!(optimize_baseline_scan(&valid[..valid.len() - 4]).is_none());

        // Truncated entropy stream with EOI preserved: an incomplete MCU must still be rejected.
        let gray8 = encode_with(&[128u8; 64], 8, 8, 1, 80, ChromaSubsampling::S444, &[], false);
        assert!(optimize_baseline_scan(&gray8).is_some());
        let mut truncated_entropy = gray8[..gray8.len() - 3].to_vec();
        truncated_entropy.extend_from_slice(&[0xFF, 0xD9]);
        assert!(optimize_baseline_scan(&truncated_entropy).is_none());

        // Non-baseline frame (SOF2).
        let mut progressive = valid.clone();
        let (sof, _) = find_marker_segment(&progressive, 0xC0, 0).unwrap();
        progressive[sof + 1] = 0xC2;
        assert!(optimize_baseline_scan(&progressive).is_none());

        // Non-interleaved baseline output from jpeg-encoder (multiple SOS markers).
        let mut non_interleaved = Vec::new();
        let mut enc = jpeg_encoder::Encoder::new(&mut non_interleaved, 80);
        enc.set_optimized_huffman_tables(true);
        enc.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_4_4);
        enc.encode(&src, 16, 16, jpeg_encoder::ColorType::Rgb).unwrap();
        assert!(optimize_baseline_scan(&non_interleaved).is_none());

        // ITU-T T.81 Table B.5 limits SOF0 Baseline Huffman destinations to 0..=1.
        let mut bad_slot = valid.clone();
        let (dht0, _) = find_marker_segment(&bad_slot, 0xC4, 0).unwrap();
        assert_eq!(bad_slot[dht0 + 4], 0x00);
        bad_slot[dht0 + 4] = 0x02;
        let (sos, _) = find_marker_segment(&bad_slot, 0xDA, 0).unwrap();
        bad_slot[sos + 6] = 0x20;
        assert!(optimize_baseline_scan(&bad_slot).is_none());

        // Illegal baseline DC category (> 11) in DHT must be rejected before decoding.
        let mut bad_dc_sym = valid.clone();
        let (dht_dc, _) = find_marker_segment(&bad_dc_sym, 0xC4, 0).unwrap();
        assert_eq!(bad_dc_sym[dht_dc + 4], 0x00);
        bad_dc_sym[dht_dc + 4 + 17] = 12;
        assert!(optimize_baseline_scan(&bad_dc_sym).is_none());

        // Illegal baseline AC symbol (0x10: SSSS == 0 with non-EOB/ZRL run 1) must be rejected.
        let mut bad_ac_sym = valid.clone();
        let (dht_ac, _) = find_marker_segment(&bad_ac_sym, 0xC4, 1).unwrap();
        assert_eq!(bad_ac_sym[dht_ac + 4], 0x10);
        bad_ac_sym[dht_ac + 4 + 17] = 0x10;
        assert!(optimize_baseline_scan(&bad_ac_sym).is_none());

        // Missing quantization table referenced by SOF0 components before SOS.
        let mut no_chroma_dqt = valid;
        let (dqt1, dqt1_end) = find_marker_segment(&no_chroma_dqt, 0xDB, 1).unwrap();
        no_chroma_dqt.drain(dqt1..dqt1_end);
        assert!(optimize_baseline_scan(&no_chroma_dqt).is_none());

        // Why: ITU-T T.81 Table B.4 restricts SOF0 baseline frames to 8-bit quantization precision
        // (Pq == 0); copying a 16-bit (Pq == 1) DQT segment would emit a non-conforming SOF0 file.
        let mut dqt_16bit = gray8.clone();
        let (dqt0, dqt0_end) = find_marker_segment(&dqt_16bit, 0xDB, 0).unwrap();
        let mut dqt16_seg = vec![0xFF, 0xDB, 0x00, 131, 0x10];
        dqt16_seg.resize(133, 1);
        dqt_16bit.splice(dqt0..dqt0_end, dqt16_seg);
        assert!(optimize_baseline_scan(&dqt_16bit).is_none());

        // Why: Baseline quantization entries must lie in 1..=255 to avoid zero divisors on decode.
        let mut dqt_zero = gray8.clone();
        let (dqt0, _) = find_marker_segment(&dqt_zero, 0xDB, 0).unwrap();
        dqt_zero[dqt0 + 5] = 0;
        assert!(optimize_baseline_scan(&dqt_zero).is_none());

        // Why: ITU-T T.81 §C.2 forbids assigning an all-ones codeword (here two 1-bit codes `0` and
        // `1` that saturate the code space); single-symbol 1-bit code `0` remains legal and accepted.
        let build_one_bit_gray8 = |dc_counts_0: u8, dc_vals: &[u8]| -> Vec<u8> {
            let mut jpg = gray8[..find_marker_segment(&gray8, 0xC4, 0).unwrap().0].to_vec();
            dht(&mut jpg, 0x00, &[dc_counts_0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], dc_vals);
            dht(&mut jpg, 0x10, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0x00]);
            let (sos, sos_end) = find_marker_segment(&gray8, 0xDA, 0).unwrap();
            jpg.extend_from_slice(&gray8[sos..sos_end]);
            jpg.extend_from_slice(&[0x3F, 0xFF, 0xD9]);
            jpg
        };
        assert!(optimize_baseline_scan(&build_one_bit_gray8(1, &[0])).is_some());
        assert!(optimize_baseline_scan(&build_one_bit_gray8(2, &[0, 1])).is_none());

        // Why: ITU-T T.81 §B.2.3 requires SOS components in a single full-frame scan to match the
        // exact SOF0 component declaration order; permuting 1,2,3 -> 2,1,3 would preserve an
        // invalid SOS header even when pixel blocks happen to decode identically.
        let flat_rgb16 = encode_with(&vec![128u8; 16 * 16 * 3], 16, 16, 3, 80, ChromaSubsampling::S444, &[], false);
        assert!(optimize_baseline_scan(&flat_rgb16).is_some());
        let mut permuted_sos = flat_rgb16;
        let (sos, _) = find_marker_segment(&permuted_sos, 0xDA, 0).unwrap();
        permuted_sos.swap(sos + 5, sos + 7);
        permuted_sos.swap(sos + 6, sos + 8);
        assert!(optimize_baseline_scan(&permuted_sos).is_none());

        // Why: ITU-T T.81 §A.2.3 caps interleaved MCUs at 10 data units (8x8 blocks); 4x1 + 4x1 + 3x1
        // = 11 blocks/MCU passes individual component bounds (1..=4) and must be rejected.
        let mut over_10_mcu = Vec::new();
        over_10_mcu.extend_from_slice(&[0xFF, 0xD8]);
        segment(&mut over_10_mcu, 0xDB, &[0; 1].into_iter().chain([1u8; 64]).collect::<Vec<_>>());
        segment(&mut over_10_mcu, 0xDB, &[1; 1].into_iter().chain([1u8; 64]).collect::<Vec<_>>());
        segment(&mut over_10_mcu, 0xC0, &[8, 0, 8, 0, 8, 3, 1, 0x41, 0, 2, 0x41, 1, 3, 0x31, 1]);
        for tc_th in [0x00, 0x10, 0x01, 0x11] {
            dht(&mut over_10_mcu, tc_th, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0]);
        }
        segment(&mut over_10_mcu, 0xDA, &[3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);
        // 11 blocks * (1-bit DC `0` + 1-bit EOB `0`) = 22 zero bits + `11` padding -> 0x00 0x00 0x03.
        over_10_mcu.extend_from_slice(&[0x00, 0x00, 0x03, 0xFF, 0xD9]);
        assert!(optimize_baseline_scan(&over_10_mcu).is_none());

        // Why: ITU-T T.81 §B.2.5 permits DNL (0xDC) only immediately after the first scan, and this
        // API does not support DNL; rejecting pre-SOS DNL avoids copying illegal markers into output.
        let mut pre_sos_dnl = gray8.clone();
        let (sos, _) = find_marker_segment(&pre_sos_dnl, 0xDA, 0).unwrap();
        pre_sos_dnl.splice(sos..sos, [0xFF, 0xDC, 0x00, 0x04, 0x00, 0x08]);
        assert!(optimize_baseline_scan(&pre_sos_dnl).is_none());
    }

    /// Enforce ITU-T T.81 stream invariants during header resolution and entropy scanning:
    /// `1`-bit padding before `EOI`/`RSTn`, overwritten `DHT` redefinitions, `[-1024, 1023]`
    /// cumulative DC coefficients (and predictor reset at `RSTn`), and rejecting `0xFF` fill
    /// runs before stuffed `0x00` entropy data bytes (#2183).
    #[test]
    fn optimize_baseline_scan_enforces_entropy_and_redefinition_invariants() {
        let gray8 = encode_with(&[128u8; 64], 8, 8, 1, 80, ChromaSubsampling::S444, &[], false);

        // Why: ITU-T T.81 §F.1.2.3 mandates `1`-bit padding to the byte boundary before EOI and RSTn;
        // clearing those bits would otherwise pass byte-level checks and silently alter the stream.
        let mut zero_pad_eoi = gray8.clone();
        let last_scan_idx = zero_pad_eoi.len() - 3;
        zero_pad_eoi[last_scan_idx] &= 0xFC;
        assert!(optimize_baseline_scan(&zero_pad_eoi).is_none());

        let mut dri_jpg = gray8[..find_marker_segment(&gray8, 0xDA, 0).unwrap().0].to_vec();
        let (sof, _) = find_marker_segment(&dri_jpg, 0xC0, 0).unwrap();
        dri_jpg[sof + 7..sof + 9].copy_from_slice(&16u16.to_be_bytes());
        segment(&mut dri_jpg, 0xDD, &1u16.to_be_bytes());
        let (sos, sos_end) = find_marker_segment(&gray8, 0xDA, 0).unwrap();
        dri_jpg.extend_from_slice(&gray8[sos..sos_end]);
        let rst0_pre_idx = dri_jpg.len();
        dri_jpg.extend_from_slice(&[gray8[gray8.len() - 3], 0xFF, 0xD0, gray8[gray8.len() - 3], 0xFF, 0xD9]);
        assert!(optimize_baseline_scan(&dri_jpg).is_some());
        dri_jpg[rst0_pre_idx] &= 0xFC;
        assert!(optimize_baseline_scan(&dri_jpg).is_none());

        // Why: ITU-T T.81 allows DHT redefinitions before SOS, and `parse_dht_tables` defers
        // `Huffman::new` until SOS selection so duplicated header definitions still resolve to
        // the final active slot without building intermediate decode LUTs.
        let (dht_start, _) = find_marker_segment(&gray8, 0xC4, 0).unwrap();
        let mut redefined_dht = gray8[..dht_start].to_vec();
        dht(&mut redefined_dht, 0x00, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[11]);
        dht(&mut redefined_dht, 0x10, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0xF0]);
        redefined_dht.extend_from_slice(&gray8[dht_start..]);
        assert_eq!(optimize_baseline_scan(&redefined_dht), optimize_baseline_scan(&gray8));

        // Why: ITU-T T.81 §F.1.1.5.1 / Table F.1 restricts reconstructed 8-bit SOF0 DC values to
        // [-1024, 1023]; two category-10 diffs of +600 give 1200 without restart (`None`), whereas
        // separating them with `DRI = 1` + `RST0` resets the predictor to 0 (`Some`).
        let mut dc_range_hdr = gray8[..dht_start].to_vec();
        let (dc_sof, _) = find_marker_segment(&dc_range_hdr, 0xC0, 0).unwrap();
        dc_range_hdr[dc_sof + 7..dc_sof + 9].copy_from_slice(&16u16.to_be_bytes());
        dht(&mut dc_range_hdr, 0x00, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[10]);
        dht(&mut dc_range_hdr, 0x10, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0x00]);
        let mut dc_overflow = dc_range_hdr.clone();
        dc_overflow.extend_from_slice(&gray8[sos..sos_end]);
        // Two blocks of (1-bit DC `0`, 10-bit +600 `1001011000`, 1-bit EOB `0`) = 24 bits: 0x4B 0x04 0xB0.
        dc_overflow.extend_from_slice(&[0x4B, 0x04, 0xB0, 0xFF, 0xD9]);
        assert!(optimize_baseline_scan(&dc_overflow).is_none());

        let mut dc_reset_ok = dc_range_hdr;
        segment(&mut dc_reset_ok, 0xDD, &1u16.to_be_bytes());
        dc_reset_ok.extend_from_slice(&gray8[sos..sos_end]);
        dc_reset_ok.extend_from_slice(&[0x4B, 0x0F, 0xFF, 0xD0, 0x4B, 0x0F, 0xFF, 0xD9]);
        assert!(optimize_baseline_scan(&dc_reset_ok).is_some());

        // Why: ITU-T T.81 §B.1.1.2 allows `0xFF` fill bytes only directly before a marker; turning
        // stuffed `0xFF 0x00` entropy data into `0xFF 0xFF 0x00` is malformed and must return None.
        let mut stuffed_hdr = gray8[..dht_start].to_vec();
        dht(&mut stuffed_hdr, 0x00, &[0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0]);
        dht(&mut stuffed_hdr, 0x10, &[0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0x0A, 0x00]);
        stuffed_hdr.extend_from_slice(&gray8[sos..sos_end]);
        let mut stuffed_ok = stuffed_hdr.clone();
        // 6-bit DC `0` + 2-bit AC `0x0A` (`00`) + 10-bit +1023 (`1111111111`) -> `FF 00` byte stuffing.
        stuffed_ok.extend_from_slice(&[0x00, 0xFF, 0x00, 0xDF, 0xFF, 0xD9]);
        assert!(optimize_baseline_scan(&stuffed_ok).is_some());

        let mut fill_before_stuffing = stuffed_hdr;
        fill_before_stuffing.extend_from_slice(&[0x00, 0xFF, 0xFF, 0x00, 0xDF, 0xFF, 0xD9]);
        assert!(optimize_baseline_scan(&fill_before_stuffing).is_none());

        // Why: legal `0xFF` fill runs before `EOI` can dwarf the retained payload, so reserving
        // `jpeg.len()` would hand back a small image pinning the whole input size in the heap.
        let mut filled = gray8.clone();
        let eoi = filled.len() - 2;
        filled.splice(eoi..eoi, vec![0xFF; 1 << 20]);
        let opt_filled = optimize_baseline_scan(&filled).expect("0xFF fill before EOI must optimize");
        assert!(opt_filled.len() < 4096, "unexpected output size {}", opt_filled.len());
        assert!(opt_filled.capacity() < 64 * 1024, "capacity {} for {} output bytes", opt_filled.capacity(), opt_filled.len());

        // Why: libjpeg-turbo (`JERR_BAD_HUFF_TABLE`) and jpegli reject a `DHT` that assigns two
        // distinct codewords to the same `HUFFVAL`; frequency counting per symbol would silently
        // merge both codes, so duplicate symbols are rejected before decoding.
        let build_dc_table = |counts: &[u8; 16], vals: &[u8]| -> Vec<u8> {
            let mut jpg = gray8[..find_marker_segment(&gray8, 0xC4, 0).unwrap().0].to_vec();
            dht(&mut jpg, 0x00, counts, vals);
            dht(&mut jpg, 0x10, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0x00]);
            let (sos, sos_end) = find_marker_segment(&gray8, 0xDA, 0).unwrap();
            jpg.extend_from_slice(&gray8[sos..sos_end]);
            // One block: 1-bit DC `0`, 1-bit EOB `0`, then six `1` padding bits.
            jpg.extend_from_slice(&[0x3F, 0xFF, 0xD9]);
            jpg
        };
        let two_codes = [1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(optimize_baseline_scan(&build_dc_table(&two_codes, &[0, 0])).is_none());
        // The same code lengths with distinct symbols stay valid.
        assert!(optimize_baseline_scan(&build_dc_table(&two_codes, &[0, 1])).is_some());

        // Why: `APPn`/`COM` segments must survive in their exact original order around the
        // re-emitted `DHT` while the writer allocates nothing per segment, so a leading `COM` and an
        // `APP1` placed between `SOF0` and the tables both precede the rewritten `DHT`.
        let (meta_dht, _) = find_marker_segment(&gray8, 0xC4, 0).unwrap();
        let mut with_meta = vec![0xFF, 0xD8, 0xFF, 0xFE, 0x00, 0x05, b'a', b'b', b'c'];
        with_meta.extend_from_slice(&gray8[2..meta_dht]);
        segment(&mut with_meta, 0xE1, b"metadata");
        with_meta.extend_from_slice(&gray8[meta_dht..]);
        let opt_meta = optimize_baseline_scan(&with_meta).expect("APPn/COM metadata stream must optimize");
        let com = opt_meta.windows(7).position(|w| w == [0xFF, 0xFE, 0x00, 0x05, b'a', b'b', b'c']).expect("COM preserved");
        let app =
            opt_meta.windows(12).position(|w| w == [0xFF, 0xE1, 0x00, 0x0A, b'm', b'e', b't', b'a', b'd', b'a', b't', b'a']).expect("APP1 preserved");
        let dht_pos = opt_meta.windows(2).position(|w| w == [0xFF, 0xC4]).expect("optimized DHT emitted");
        assert_eq!(com, 2);
        assert!(app < dht_pos, "APP1 must stay ahead of the rewritten DHT");
    }

    /// Legal `0xFF` fill bytes before `RSTn` are dropped by the writer, which emits only the
    /// two-byte marker, so they must not size the output reservation either.
    #[test]
    fn optimize_baseline_scan_does_not_reserve_capacity_for_fill_before_restart() {
        let gray8 = encode_with(&[128u8; 64], 8, 8, 1, 80, ChromaSubsampling::S444, &[], false);
        let mut dri = gray8[..find_marker_segment(&gray8, 0xDA, 0).unwrap().0].to_vec();
        let (sof, _) = find_marker_segment(&dri, 0xC0, 0).unwrap();
        dri[sof + 7..sof + 9].copy_from_slice(&16u16.to_be_bytes());
        segment(&mut dri, 0xDD, &1u16.to_be_bytes());
        let (sos, sos_end) = find_marker_segment(&gray8, 0xDA, 0).unwrap();
        dri.extend_from_slice(&gray8[sos..sos_end]);
        // Two identical MCUs with `RST0` between them, so `DRI = 1` splits the single scan byte.
        let block = gray8[gray8.len() - 3];
        dri.extend_from_slice(&[block, 0xFF, 0xD0, block, 0xFF, 0xD9]);
        let expected = optimize_baseline_scan(&dri).expect("DRI = 1 stream must optimize");

        let rst0 = dri.windows(2).position(|w| w == [0xFF, 0xD0]).expect("RST0 marker");
        let mut filled = dri[..rst0].to_vec();
        filled.resize(filled.len() + (1 << 20), 0xFF);
        filled.extend_from_slice(&dri[rst0..]);

        let optimized = optimize_baseline_scan(&filled).expect("fill before RST0 must still optimize");
        assert_eq!(optimized, expected, "fill bytes must not change the output");
        assert!(optimized.len() < 4096, "unexpected output size {}", optimized.len());
        assert!(optimized.capacity() < 64 * 1024, "capacity {} for {} output bytes", optimized.capacity(), optimized.len());
    }

    /// Sloppy encoders may pad a block with ZRL runs before EOB. EOB already covers those zeros, so
    /// the optimizer must emit one EOB instead of copying the redundant run, while ZRLs that are
    /// followed by a nonzero coefficient still have to be written.
    #[test]
    fn repeated_zrl_before_eob_is_rewritten_as_a_single_eob() {
        let dc = crate::Huffman::new(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0]).unwrap();
        let ac = crate::Huffman::new(&[1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0xF0, 0x00]).unwrap();
        // DC `0`, then ZRL `0` three times, then EOB `10` and two `1` padding bits.
        let mut bits = crate::Bits::new(&[0x0B]);
        let mut pred = 0;
        let mut syms = Vec::new();
        decode_block(&mut bits, &dc, &ac, ScanComp { blocks: 1, td: 0, ta: 0 }, &mut pred, &mut |ev| {
            if let ScanEvent::Sym { is_ac: true, sym, .. } = ev {
                syms.push(sym);
            }
        })
        .expect("three ZRLs followed by EOB");
        assert_eq!(syms, [0x00]);

        // ZRL `0`, a coefficient `10` with amplitude `1`, then EOB `110`: the ZRL must survive.
        let ac = crate::Huffman::new(&[1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0xF0, 0x01, 0x00]).unwrap();
        let mut bits = crate::Bits::new(&[0x2E]);
        let mut pred = 0;
        let mut syms = Vec::new();
        decode_block(&mut bits, &dc, &ac, ScanComp { blocks: 1, td: 0, ta: 0 }, &mut pred, &mut |ev| {
            if let ScanEvent::Sym { is_ac: true, sym, .. } = ev {
                syms.push(sym);
            }
        })
        .expect("ZRL followed by a coefficient");
        assert_eq!(syms, [0xF0, 0x01, 0x00]);
    }

    /// A legal but wasteful input table can spend 16 bits on each of DC and EOB, so the input scan
    /// is many times the optimized one; the reservation must follow the optimized size.
    #[test]
    fn optimize_baseline_scan_reserves_capacity_from_the_optimized_scan() {
        let mut jpg = vec![0xFF, 0xD8];
        segment(&mut jpg, 0xDB, &[0u8].into_iter().chain([1u8; 64]).collect::<Vec<_>>());
        // 256x256 gray is 1024 blocks, so a 16-bit-per-symbol table costs 4 KiB of scan.
        segment(&mut jpg, 0xC0, &[8, 1, 0, 1, 0, 1, 1, 0x11, 0]);
        let one_long_code = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        dht(&mut jpg, 0x00, &one_long_code, &[0]);
        dht(&mut jpg, 0x10, &one_long_code, &[0x00]);
        segment(&mut jpg, 0xDA, &[1, 1, 0x00, 0, 63, 0]);
        jpg.extend_from_slice(&[0u8; 4 * 1024]);
        jpg.extend_from_slice(&[0xFF, 0xD9]);

        let optimized = optimize_baseline_scan(&jpg).expect("legal long-code stream must optimize");
        assert!(optimized.len() < 1024, "unexpected output size {}", optimized.len());
        assert!(optimized.capacity() < 1024, "capacity {} for {} output bytes", optimized.capacity(), optimized.len());
        let decoded = crate::decode(&optimized, &crate::Limits::default(), None).expect("optimized output decodes");
        assert_eq!(decoded.samples, crate::Samples::U8(vec![128u8; 256 * 256]));
    }

    /// A `ZRL` that covers exactly the last 16 coefficients drives `k` to `64` and ends the block
    /// without an `EOB`. libjpeg, zune-jpeg and `craft_jpeg::decode` all accept that input, while
    /// T.81 §F.1.2.2.3 requires the encoder to write `EOB` when the last coefficient is zero, so
    /// the optimizer accepts the block and emits the canonical `EOB` instead of the trailing `ZRL`.
    #[test]
    fn trailing_zrl_that_exactly_fills_the_block_is_accepted_and_rewritten_as_eob() {
        let mut jpg = vec![0xFF, 0xD8];
        segment(&mut jpg, 0xDB, &[0u8].into_iter().chain([1u8; 64]).collect::<Vec<_>>());
        segment(&mut jpg, 0xC0, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]);
        // DC `0` is a single 1-bit code; the AC table gives `0` to ZRL and `10` to `E1` (run 14, size 1).
        dht(&mut jpg, 0x00, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0]);
        dht(&mut jpg, 0x10, &[1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0xF0, 0xE1]);
        segment(&mut jpg, 0xDA, &[1, 1, 0x00, 0, 63, 0]);
        // DC `0`, ZRL (k 1..=16), ZRL (k 17..=32), `E1` + amplitude `1` (index 47), then ZRL over
        // indices 48..=63 and one `1` padding bit: the block ends with `k == 64` and no `EOB`.
        jpg.extend_from_slice(&[0x15, 0xFF, 0xD9]);

        let limits = crate::Limits::default();
        let original = crate::decode(&jpg, &limits, None).expect("craft-jpeg decodes a trailing ZRL");
        let optimized = optimize_baseline_scan(&jpg).expect("a trailing ZRL still optimizes");
        let round_trip = crate::decode(&optimized, &limits, None).expect("re-encoded output decodes");
        assert_eq!(original.samples, round_trip.samples);
        assert_eq!(jpeg_decoder::Decoder::new(&jpg[..]).decode().expect("libjpeg accepts a trailing ZRL").len(), 64);

        // The redundant trailing ZRL becomes EOB in the input event stream the writer re-encodes.
        let dc = crate::Huffman::new(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0]).unwrap();
        let ac = crate::Huffman::new(&[1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0xF0, 0xE1]).unwrap();
        let mut bits = crate::Bits::new(&[0x15]);
        let mut pred = 0;
        let mut ac_syms = Vec::new();
        decode_block(&mut bits, &dc, &ac, ScanComp { blocks: 1, td: 0, ta: 0 }, &mut pred, &mut |ev| {
            if let ScanEvent::Sym { is_ac: true, sym, .. } = ev {
                ac_syms.push(sym);
            }
        })
        .expect("single block with a trailing ZRL");
        assert_eq!(ac_syms, [0xF0, 0xF0, 0xE1, 0x00]);
    }
}

#[cfg(test)]
#[path = "encode_low_memory_tests.rs"]
mod low_memory_tests;
