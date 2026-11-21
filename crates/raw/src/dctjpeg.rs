//! DCT-based JPEG at 8 or 12 bits: ITU-T T.81 / ISO 10918-1 baseline (SOF0) and extended sequential (SOF1)
//! Huffman processes, as used by Blackmagic's lossy CinemaDNG (12-bit CFA tiles filed under DNG compression 7).
//!
//! - One scan, 1-4 components with 1x1 sampling (CFA tiles are a single component), 8 or 12 bit precision,
//!   8 or 16 bit quantisation tables, restart intervals.
//! - [`decode`] returns the frame as interleaved samples in raster order, the same [`Frame`] shape as
//!   [`crate::ljpeg::decode`], so callers re-tile it the same way.
//! - [`is_dct`] tells which frame type a stream carries, to pick between this and the lossless decoder.
//! - Dequantised coefficients and the separable inverse DCT are kept in `f64`, so every `i32 x u16` coefficient
//!   product is exact; the result is rounded and clamped to the sample range.

use crate::RawError;
use crate::ljpeg::{BitReader, Frame, Huffman};

fn err(s: &str) -> RawError {
    RawError::Corrupt(format!("DCT JPEG: {s}"))
}

/// Zig-zag scan position -> natural (row-major) index within a block.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43,
    36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// The frame type (SOF marker low nibble) of the first frame header in a JPEG stream, if any.
pub fn sof_kind(d: &[u8]) -> Option<u8> {
    if d.len() < 4 || d[0] != 0xff || d[1] != 0xd8 {
        return None;
    }
    let mut i = 2;
    while i + 4 <= d.len() {
        if d[i] != 0xff {
            i += 1;
            continue;
        }
        let m = d[i + 1];
        if m == 0xff {
            i += 1;
            continue;
        }
        if m == 0 || m == 0x01 || (0xd0..=0xd8).contains(&m) {
            i += 2;
            continue;
        }
        if m == 0xda || m == 0xd9 {
            return None;
        }
        if (0xc0..=0xcf).contains(&m) && !matches!(m, 0xc4 | 0xc8 | 0xcc) {
            return Some(m - 0xc0);
        }
        i += 2 + u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
    }
    None
}

/// Whether the stream is DCT-coded (SOF0 baseline or SOF1 extended sequential) rather than lossless.
pub fn is_dct(d: &[u8]) -> bool {
    matches!(sof_kind(d), Some(0 | 1))
}

/// Header-only check that [`decode`] accepts this stream's frame: (width, height, components, precision).
pub fn frame_info(d: &[u8]) -> Result<(usize, usize, usize, u8), RawError> {
    let mut i = 2;
    while i + 4 <= d.len() {
        if d[i] != 0xff || matches!(d[i + 1], 0 | 0xff) {
            i += 1;
            continue;
        }
        let m = d[i + 1];
        let len = u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
        if matches!(m, 0xc0 | 0xc1) {
            let seg = d.get(i + 4..i + 2 + len).ok_or_else(|| err("truncated SOF"))?;
            let f = parse_sof(seg)?;
            return Ok((f.width, f.height, f.comps.len(), f.precision));
        }
        i += 2 + len;
    }
    Err(err("no frame header"))
}

/// A frame header (SOF0/SOF1 segment body) both [`frame_info`] and [`decode`] accept.
struct Sof {
    precision: u8,
    width: usize,
    height: usize,
    /// (component id, quantisation table) per frame component, in frame order
    comps: Vec<(u8, usize)>,
}

fn parse_sof(seg: &[u8]) -> Result<Sof, RawError> {
    if seg.len() < 6 {
        return Err(err("truncated SOF"));
    }
    let precision = seg[0];
    if !matches!(precision, 8 | 12) {
        return Err(RawError::Unsupported(format!("{precision}-bit DCT JPEG")));
    }
    let height = u16::from_be_bytes([seg[1], seg[2]]) as usize;
    let width = u16::from_be_bytes([seg[3], seg[4]]) as usize;
    let n = seg[5] as usize;
    // a zero height would need a DNL segment (T.81 B.2.5), which DNG tiles don't use
    if n == 0 || n > 4 || seg.len() < 6 + 3 * n || width == 0 || height == 0 {
        return Err(err("bad frame header"));
    }
    if (0..n).any(|c| seg[7 + 3 * c] != 0x11) {
        return Err(RawError::Unsupported("subsampled DCT JPEG".into()));
    }
    let comps = (0..n).map(|c| (seg[6 + 3 * c], (seg[8 + 3 * c] & 3) as usize)).collect();
    Ok(Sof { precision, width, height, comps })
}

/// Where the entropy-coded data starting at `d[start..]` ends: the first marker that is not `FF 00` stuffing or
/// `RSTn`, after any `FF` fill bytes. The marker stream after it must reach the outer EOI, skipping
/// length-delimited segments (so an EOI inside, say, an APP segment's thumbnail does not count).
fn entropy_end(d: &[u8], start: usize) -> Result<usize, RawError> {
    let mut i = start;
    let end = loop {
        while i < d.len() && d[i] != 0xff {
            i += 1;
        }
        let mut j = i;
        while d.get(j + 1) == Some(&0xff) {
            j += 1;
        }
        match d.get(j + 1) {
            None => return Err(err("no EOI after the scan")),
            Some(0) | Some(0xd0..=0xd7) => i = j + 2,
            Some(_) => break i,
        }
    };
    let mut i = end;
    loop {
        while d.get(i) == Some(&0xff) && d.get(i + 1) == Some(&0xff) {
            i += 1;
        }
        let m = match (d.get(i), d.get(i + 1)) {
            (Some(0xff), Some(&m)) => m,
            _ => return Err(err("no EOI after the scan")),
        };
        match m {
            0xd9 => return Ok(end),
            0x01 => i += 2,
            0xda | 0xc0..=0xc3 | 0xc5..=0xcb | 0xcd..=0xcf | 0xd0..=0xd8 | 0x00 => return Err(err("unexpected marker after the scan")),
            _ => {
                let len = d.get(i + 2..i + 4).map(|s| u16::from_be_bytes([s[0], s[1]]) as usize).ok_or_else(|| err("no EOI after the scan"))?;
                if len < 2 {
                    return Err(err("bad segment after the scan"));
                }
                i += 2 + len;
            }
        }
    }
}

struct Comp {
    q: usize,
    dc: usize,
    ac: usize,
}

/// `t[x][u]` = C(u)/2 * cos((2x+1)u*pi/16): the 1-D inverse DCT basis.
fn basis() -> [[f64; 8]; 8] {
    let mut t = [[0f64; 8]; 8];
    for (x, row) in t.iter_mut().enumerate() {
        for (u, v) in row.iter_mut().enumerate() {
            let c = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
            *v = c / 2.0 * ((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI / 16.0).cos();
        }
    }
    t
}

fn idct(coef: &[f64; 64], t: &[[f64; 8]; 8]) -> [f64; 64] {
    let mut tmp = [0f64; 64];
    for v in 0..8 {
        for x in 0..8 {
            tmp[v * 8 + x] = (0..8).map(|u| t[x][u] * coef[v * 8 + u]).sum();
        }
    }
    let mut out = [0f64; 64];
    for x in 0..8 {
        for y in 0..8 {
            out[y * 8 + x] = (0..8).map(|v| t[y][v] * tmp[v * 8 + x]).sum();
        }
    }
    out
}

/// T.81 F.12 EXTEND: the signed value of an `s`-bit magnitude.
#[inline]
fn extend(br: &mut BitReader, s: u8) -> i32 {
    if s == 0 {
        return 0;
    }
    let v = br.get(s as u32) as i32;
    if v < (1 << (s - 1)) { v - (1 << s) + 1 } else { v }
}

/// Decode a DCT JPEG (8 or 12 bit) into interleaved samples; `max_samples` bounds the frame size.
pub fn decode(d: &[u8], max_samples: usize) -> Result<Frame, RawError> {
    if d.len() < 4 || d[0] != 0xff || d[1] != 0xd8 {
        return Err(err("missing SOI"));
    }
    let mut quant = [[0u16; 64]; 4];
    let mut quant_defined = [false; 4];
    let mut dc_tab: [Option<Huffman>; 4] = [None, None, None, None];
    let mut ac_tab: [Option<Huffman>; 4] = [None, None, None, None];
    let (mut precision, mut width, mut height) = (0u8, 0usize, 0usize);
    let mut comps: Vec<(u8, usize)> = vec![]; // (component id, quantisation table)
    let mut restart = 0usize;
    let mut i = 2;
    let scan: Vec<Comp>;
    let data_start;
    loop {
        while i < d.len() && d[i] != 0xff {
            i += 1;
        }
        while i < d.len() && d[i] == 0xff {
            i += 1;
        }
        let Some(&m) = d.get(i) else { return Err(err("no scan")) };
        i += 1;
        if m == 0xd8 || m == 0x01 || (0xd0..=0xd7).contains(&m) {
            continue;
        }
        if m == 0xd9 {
            return Err(err("EOI before scan"));
        }
        let len = d.get(i..i + 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as usize).ok_or_else(|| err("truncated header"))?;
        if len < 2 || i + len > d.len() {
            return Err(err("truncated segment"));
        }
        let seg = &d[i + 2..i + len];
        match m {
            0xc0 | 0xc1 => {
                if !comps.is_empty() {
                    return Err(err("bad SOF"));
                }
                let f = parse_sof(seg)?;
                (precision, width, height, comps) = (f.precision, f.width, f.height, f.comps);
            }
            0xc2 | 0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                return Err(RawError::Unsupported(format!("JPEG process SOF{:X} (not sequential Huffman DCT)", m - 0xc0)));
            }
            0xdb => {
                let mut j = 0;
                while j < seg.len() {
                    let (pq, tq) = (seg[j] >> 4, (seg[j] & 15) as usize);
                    j += 1;
                    let step = if pq == 0 { 1 } else { 2 };
                    let body = seg.get(j..j + 64 * step).ok_or_else(|| err("truncated DQT"))?;
                    if tq < 4 {
                        quant_defined[tq] = true;
                        for k in 0..64 {
                            quant[tq][ZIGZAG[k]] = if pq == 0 { body[k] as u16 } else { u16::from_be_bytes([body[2 * k], body[2 * k + 1]]) };
                        }
                    }
                    j += 64 * step;
                }
            }
            0xc4 => {
                let mut j = 0;
                while j + 17 <= seg.len() {
                    let (class, id) = (seg[j] >> 4, (seg[j] & 15) as usize);
                    let mut counts = [0u8; 16];
                    counts.copy_from_slice(&seg[j + 1..j + 17]);
                    let n: usize = counts.iter().map(|&c| c as usize).sum();
                    let vals = seg.get(j + 17..j + 17 + n).ok_or_else(|| err("truncated DHT"))?;
                    if id < 4 {
                        let t = Some(Huffman::new(&counts, vals)?);
                        if class == 0 {
                            dc_tab[id] = t;
                        } else {
                            ac_tab[id] = t;
                        }
                    }
                    j += 17 + n;
                }
            }
            0xdd => {
                restart = seg.get(..2).map(|s| u16::from_be_bytes([s[0], s[1]]) as usize).ok_or_else(|| err("short DRI"))?;
            }
            0xda => {
                if comps.is_empty() {
                    return Err(err("SOS before SOF"));
                }
                let ns = *seg.first().ok_or_else(|| err("short SOS"))? as usize;
                if ns != comps.len() {
                    return Err(RawError::Unsupported("multi-scan DCT JPEG".into()));
                }
                // T.81 B.2.3: the selectors, then Ss, Se and Ah/Al, which a sequential scan sets to 0, 63 and 0
                if seg.len() != 4 + 2 * ns {
                    return Err(err("bad scan header"));
                }
                if seg[1 + 2 * ns..] != [0, 63, 0] {
                    return Err(err("scan parameters are not sequential (0, 63, 0)"));
                }
                let mut v = vec![];
                for (k, &(id, q)) in comps.iter().enumerate() {
                    let (cid, tabs) = (seg[1 + 2 * k], seg[2 + 2 * k]);
                    // the one scan codes every frame component once, in frame order (T.81 B.2.3)
                    if cid != id {
                        return Err(err("scan components do not match the frame's"));
                    }
                    // T.81 B.2.2: the frame's table must be installed before its scan; a zero table would decode flat
                    if !quant_defined[q] {
                        return Err(err("undefined quantisation table"));
                    }
                    v.push(Comp { q, dc: (tabs >> 4) as usize & 3, ac: (tabs & 15) as usize & 3 });
                }
                scan = v;
                data_start = i + len;
                break;
            }
            _ => {}
        }
        i += len;
    }

    let nc = scan.len();
    let samples = width.checked_mul(height).and_then(|v| v.checked_mul(nc)).ok_or(RawError::Limit("JPEG too large"))?;
    if samples > max_samples {
        return Err(RawError::Limit("JPEG too large"));
    }
    let (bw, bh) = (width.div_ceil(8), height.div_ceil(8));
    let maxv = (1i32 << precision) - 1;
    let shift = f64::from(1i32 << (precision - 1));
    let t = basis();
    // a tile cut off inside its last blocks would otherwise decode from zero bits: the outer EOI must be there
    let data_end = entropy_end(d, data_start)?;
    for c in &scan {
        if dc_tab[c.dc].is_none() || ac_tab[c.ac].is_none() {
            return Err(err("missing Huffman table"));
        }
    }
    let mut out = zeroed(samples)?;
    let mut br = BitReader::new(&d[data_start..data_end]);
    let mut pred = vec![0i32; nc];
    let mut rst = 0u8;
    for mcu in 0..bw * bh {
        if restart > 0 && mcu > 0 && mcu % restart == 0 {
            // T.81 F.1.2.3: RST0 first, then modulo 8, right after the interval's padding
            if !br.restart_exact(rst) {
                return Err(err("missing restart marker"));
            }
            rst = (rst + 1) & 7;
            pred.fill(0);
        }
        let (bx, by) = (mcu % bw, mcu / bw);
        for (k, c) in scan.iter().enumerate() {
            let dct = dc_tab[c.dc].as_ref().ok_or_else(|| err("missing DC table"))?;
            let act = ac_tab[c.ac].as_ref().ok_or_else(|| err("missing AC table"))?;
            let q = &quant[c.q];
            let mut coef = [0f64; 64];
            let s = dct.decode(&mut br)?;
            if s > 15 {
                return Err(err("bad DC category"));
            }
            coef[0] = dc_coefficient(&mut pred[k], extend(&mut br, s), q[0])?;
            let mut z = 1;
            while z < 64 {
                let rs = act.decode(&mut br)?;
                let (r, s) = ((rs >> 4) as usize, rs & 15);
                if s == 0 {
                    if r == 15 {
                        z += 16;
                        continue;
                    }
                    break;
                }
                z += r;
                if z > 63 {
                    return Err(err("AC run past block end"));
                }
                let n = ZIGZAG[z];
                coef[n] = f64::from(extend(&mut br, s)) * f64::from(q[n]);
                z += 1;
            }
            // the reader feeds zeros past the data: a block that needed any of them is cut off
            if br.consumed_synthetic() {
                return Err(err("entropy data ends early"));
            }
            let px = idct(&coef, &t);
            for y in 0..8 {
                let yy = by * 8 + y;
                if yy >= height {
                    break;
                }
                for x in 0..8 {
                    let xx = bx * 8 + x;
                    if xx >= width {
                        break;
                    }
                    out[(yy * width + xx) * nc + k] = ((px[y * 8 + x] + shift).round() as i32).clamp(0, maxv) as u16;
                }
            }
        }
    }
    Ok(Frame { width, height, components: nc, precision, predictor: 0, point_transform: 0, data: out })
}

/// `samples` zeros, or a limit error: a frame within the sample budget may still be more bytes than this target can
/// allocate (a 32-bit one, say), and a failed infallible allocation aborts the process.
fn zeroed(samples: usize) -> Result<Vec<u16>, RawError> {
    let mut out = Vec::new();
    out.try_reserve_exact(samples).map_err(|_| RawError::Limit("JPEG too large"))?;
    out.resize(samples, 0u16);
    Ok(out)
}

/// Accumulate hostile DC differences without integer overflow, then dequantise exactly in `f64`.
fn dc_coefficient(predictor: &mut i32, difference: i32, quantiser: u16) -> Result<f64, RawError> {
    *predictor = predictor.checked_add(difference).ok_or_else(|| err("DC predictor overflow"))?;
    Ok(f64::from(*predictor) * f64::from(quantiser))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BitWriter {
        out: Vec<u8>,
        acc: u32,
        n: u32,
    }
    impl BitWriter {
        fn put(&mut self, v: u32, bits: u32) {
            for b in (0..bits).rev() {
                self.acc = (self.acc << 1) | ((v >> b) & 1);
                self.n += 1;
                if self.n == 8 {
                    self.out.push(self.acc as u8);
                    if self.acc as u8 == 0xff {
                        self.out.push(0);
                    }
                    self.acc = 0;
                    self.n = 0;
                }
            }
        }
        fn flush(&mut self) {
            while self.n != 0 {
                self.put(1, 1);
            }
        }
    }

    fn category(v: i32) -> u32 {
        32 - v.unsigned_abs().leading_zeros()
    }

    fn magnitude_bits(v: i32, s: u32) -> u32 {
        if v < 0 { (v - 1 + (1 << s)) as u32 & ((1 << s) - 1) } else { v as u32 }
    }

    /// The fixtures' DC code for category `s`: 4 bits for 0-14, `11110` for 15, never the reserved all-ones code
    /// (T.81 C.2). An 8-bit table lists categories 0-11 only, a 12-bit one 0-15 (T.81 F.1.2.1.1).
    fn put_dc_category(bw: &mut BitWriter, s: u32) {
        if s < 15 { bw.put(s, 4) } else { bw.put(0b11110, 5) }
    }

    /// A minimal conforming encoder: unit quantiser, the DC codes of [`put_dc_category`], 8-bit AC codes (255 symbols).
    fn encode(px: &[u16], w: usize, h: usize, precision: u8, sof: u8, restart: usize) -> Vec<u8> {
        let mut o = vec![0xff, 0xd8, 0xff, 0xdb, 0, 67, 0];
        o.extend(std::iter::repeat_n(1u8, 64));
        o.extend([0xff, 0xc0 + sof, 0, 11, precision, (h >> 8) as u8, h as u8, (w >> 8) as u8, w as u8, 1, 1, 0x11, 0]);
        let mut dc = [0u8; 16];
        let categories: u8 = if precision == 8 { 12 } else { 16 };
        dc[3] = categories.min(15);
        dc[4] = categories - dc[3];
        o.extend([0xff, 0xc4, 0, 2 + 17 + categories, 0x00]);
        o.extend(dc);
        o.extend(0..categories);
        let mut ac = [0u8; 16];
        ac[7] = 255;
        let l = 2 + 17 + 255;
        o.extend([0xff, 0xc4, (l >> 8) as u8, l as u8, 0x10]);
        o.extend(ac);
        o.extend(0..255u8);
        if restart > 0 {
            o.extend([0xff, 0xdd, 0, 4, (restart >> 8) as u8, restart as u8]);
        }
        o.extend([0xff, 0xda, 0, 8, 1, 1, 0x00, 0, 63, 0]);
        let (bw, bh) = (w.div_ceil(8), h.div_ceil(8));
        let mut bwr = BitWriter { out: vec![], acc: 0, n: 0 };
        let mut pred = 0i32;
        let mut rst = 0u8;
        for mcu in 0..bw * bh {
            if restart > 0 && mcu > 0 && mcu % restart == 0 {
                bwr.flush();
                bwr.out.extend([0xff, 0xd0 + rst]);
                rst = (rst + 1) & 7;
                pred = 0;
            }
            let (bx, by) = (mcu % bw, mcu / bw);
            let mut blk = [0f64; 64];
            for y in 0..8 {
                for x in 0..8 {
                    let (xx, yy) = ((bx * 8 + x).min(w - 1), (by * 8 + y).min(h - 1));
                    blk[y * 8 + x] = px[yy * w + xx] as f64 - (1u32 << (precision - 1)) as f64;
                }
            }
            let mut co = [0i32; 64];
            for v in 0..8 {
                for u in 0..8 {
                    let mut s = 0.0;
                    for y in 0..8 {
                        for x in 0..8 {
                            s += blk[y * 8 + x]
                                * ((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI / 16.0).cos()
                                * ((2 * y + 1) as f64 * v as f64 * std::f64::consts::PI / 16.0).cos();
                        }
                    }
                    let cu = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                    let cv = if v == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                    co[v * 8 + u] = (s * cu * cv / 4.0).round() as i32;
                }
            }
            let diff = co[0] - pred;
            pred = co[0];
            let s = category(diff);
            put_dc_category(&mut bwr, s);
            bwr.put(magnitude_bits(diff, s), s);
            let mut run = 0u32;
            let last = (1..64).rev().find(|&z| co[ZIGZAG[z]] != 0).unwrap_or(0);
            for z in 1..=last {
                let v = co[ZIGZAG[z]];
                if v == 0 {
                    run += 1;
                    continue;
                }
                while run > 15 {
                    bwr.put(0xf0, 8);
                    run -= 16;
                }
                let s = category(v);
                bwr.put((run << 4) | s, 8);
                bwr.put(magnitude_bits(v, s), s);
                run = 0;
            }
            if last < 63 {
                bwr.put(0, 8);
            }
        }
        bwr.flush();
        o.extend(bwr.out);
        o.extend([0xff, 0xd9]);
        o
    }

    fn image(w: usize, h: usize, max: u32) -> Vec<u16> {
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f64, (i / w) as f64);
                (max as f64 * (0.5 + 0.3 * (x / 9.0).sin() * (y / 7.0).cos() + 0.1 * (x / 40.0))).clamp(0.0, max as f64) as u16
            })
            .collect()
    }

    fn worst(a: &[u16], b: &[u16]) -> i32 {
        a.iter().zip(b).map(|(&a, &b)| (a as i32 - b as i32).abs()).max().unwrap()
    }

    #[test]
    fn twelve_bit_extended_sequential_round_trip() {
        let (w, h) = (37, 21);
        let px = image(w, h, 4095);
        let jpg = encode(&px, w, h, 12, 1, 0);
        assert_eq!(sof_kind(&jpg), Some(1));
        assert!(is_dct(&jpg));
        let f = decode(&jpg, 1 << 20).unwrap();
        assert_eq!((f.width, f.height, f.components, f.precision), (w, h, 1, 12));
        assert!(worst(&f.data, &px) <= 3, "worst error {}", worst(&f.data, &px));
    }

    #[test]
    fn eight_bit_baseline_with_restarts() {
        let (w, h) = (40, 24);
        let px = image(w, h, 255);
        let f = decode(&encode(&px, w, h, 8, 0, 4), 1 << 20).unwrap();
        assert!(worst(&f.data, &px) <= 2, "worst error {}", worst(&f.data, &px));
    }

    #[test]
    fn lossless_stream_is_not_dct() {
        let jpg = crate::ljpeg::encode(&[1, 2, 3, 4], 2, 2, 1, 12, 1, 0);
        assert_eq!(sof_kind(&jpg), Some(3));
        assert!(!is_dct(&jpg));
    }

    #[test]
    fn truncated_stream_does_not_panic() {
        let px = image(32, 32, 4095);
        let jpg = encode(&px, 32, 32, 12, 1, 0);
        let _ = decode(&jpg[..jpg.len() / 2], 1 << 20);
        assert!(decode(&jpg[..40], 1 << 20).is_err());
    }
    #[test]
    fn undefined_quantisation_table_is_corrupt() {
        let px = vec![2048; 8 * 8];
        let mut jpg = encode(&px, 8, 8, 12, 1, 0);
        // the encoder's only DQT installs table 0; install it as table 1 instead, leaving the frame's table 0 undefined
        let dqt = jpg.windows(2).position(|b| b == [0xff, 0xdb]).unwrap();
        jpg[dqt + 4] = 1;
        assert!(matches!(decode(&jpg, 64), Err(RawError::Corrupt(m)) if m.contains("quantisation")));
    }

    #[test]
    fn missing_eoi_is_corrupt() {
        let px = image(32, 32, 4095);
        let mut jpg = encode(&px, 32, 32, 12, 1, 0);
        assert!(decode(&jpg, 1 << 20).is_ok());
        assert_eq!(jpg[jpg.len() - 2..], [0xff, 0xd9]);
        jpg.truncate(jpg.len() - 2);
        assert!(matches!(decode(&jpg, 1 << 20), Err(RawError::Corrupt(m)) if m.contains("EOI")));
    }

    #[test]
    fn missing_required_restart_is_corrupt() {
        let px = vec![2048; 16 * 8];
        let mut jpg = encode(&px, 16, 8, 12, 1, 1);
        let restart = jpg.windows(2).position(|b| b == [0xff, 0xd0]).unwrap();
        jpg.truncate(restart);
        jpg.extend([0xff, 0xd9]);
        assert!(matches!(decode(&jpg, 128), Err(RawError::Corrupt(_))));
    }

    #[test]
    fn dc_arithmetic_is_checked_before_dequantising() {
        let mut predictor = 32767;
        let value = dc_coefficient(&mut predictor, 32767, u16::MAX).unwrap();
        assert_eq!(predictor, 65534);
        assert_eq!(value, 4_294_770_690.0);
        for (mut predictor, difference) in [(i32::MAX, 1), (i32::MIN, -1)] {
            assert!(matches!(dc_coefficient(&mut predictor, difference, u16::MAX), Err(RawError::Corrupt(_))));
        }
    }

    #[test]
    fn large_coefficients_reconstruct_without_rounding() {
        // 8 * (2^24 + 1) is past f32's exact-integer range; a DC-only block must come back as exactly 2^24 + 1.
        let mut coef = [0f64; 64];
        coef[0] = dc_coefficient(&mut 16_777_217, 0, 8).unwrap();
        assert!(idct(&coef, &basis()).iter().all(|v| (v - 16_777_217.0).abs() < 1e-6));
    }

    #[test]
    fn large_dc_coefficients_do_not_overflow_dequantisation() {
        let mut jpg = encode(&[2048; 128], 16, 8, 12, 1, 0);
        let mut dqt = vec![0xff, 0xdb, 0, 131, 0x10];
        dqt.extend(std::iter::repeat_n(0xff, 128));
        jpg.splice(2..71, dqt);
        let entropy = jpg.windows(2).position(|b| b == [0xff, 0xda]).unwrap() + 10;
        jpg.truncate(entropy);
        let mut bits = BitWriter { out: vec![], acc: 0, n: 0 };
        for _ in 0..2 {
            put_dc_category(&mut bits, 15);
            bits.put(32767, 15);
            bits.put(0, 8); // EOB
        }
        bits.flush();
        jpg.extend(bits.out);
        jpg.extend([0xff, 0xd9]);
        let frame = decode(&jpg, 128).unwrap();
        assert_eq!(frame.data, vec![4095; 128]);
    }
    #[test]
    fn accumulated_dc_prediction_overflow_is_corrupt() {
        let (w, h) = (65528u16, 72u16);
        let mut jpg = vec![0xff, 0xd8, 0xff, 0xdb, 0, 67, 0];
        jpg.extend([1; 64]);
        jpg.extend([0xff, 0xc1, 0, 11, 12]);
        jpg.extend(h.to_be_bytes());
        jpg.extend(w.to_be_bytes());
        jpg.extend([1, 1, 0x11, 0]);
        // One-bit Huffman codes: DC category 15 and AC end-of-block.
        for (id, symbol) in [(0, 15), (0x10, 0)] {
            jpg.extend([0xff, 0xc4, 0, 20, id, 1]);
            jpg.extend([0; 15]);
            jpg.push(symbol);
        }
        jpg.extend([0xff, 0xda, 0, 8, 1, 1, 0, 0, 63, 0]);
        let mut bits = BitWriter { out: vec![], acc: 0, n: 0 };
        for _ in 0..usize::from(w).div_ceil(8) * usize::from(h).div_ceil(8) {
            bits.put(0, 1);
            bits.put(32767, 15);
            bits.put(0, 1);
        }
        bits.flush();
        jpg.extend(bits.out);
        jpg.extend([0xff, 0xd9]);
        assert!(matches!(decode(&jpg, 10_000_000), Err(RawError::Corrupt(_))));
    }

    /// SOI, a unit DQT, a `w`x`h` SOF1 frame with `ids` (1x1, table 0), and one-bit DC (category 0) and AC (EOB)
    /// codes: every block is the single bit pair `0 0`.
    fn flat_header(w: u16, h: u16, ids: &[u8]) -> Vec<u8> {
        let mut jpg = vec![0xff, 0xd8, 0xff, 0xdb, 0, 67, 0];
        jpg.extend([1; 64]);
        jpg.extend([0xff, 0xc1, 0, 8 + 3 * ids.len() as u8, 12]);
        jpg.extend(h.to_be_bytes());
        jpg.extend(w.to_be_bytes());
        jpg.push(ids.len() as u8);
        for &id in ids {
            jpg.extend([id, 0x11, 0]);
        }
        for class in [0x00, 0x10] {
            jpg.extend([0xff, 0xc4, 0, 20, class, 1]);
            jpg.extend([0; 15]);
            jpg.push(0);
        }
        jpg
    }

    fn sos(ids: &[u8], params: [u8; 3]) -> Vec<u8> {
        let mut s = vec![0xff, 0xda, 0, 6 + 2 * ids.len() as u8, ids.len() as u8];
        for &id in ids {
            s.extend([id, 0x00]);
        }
        s.extend(params);
        s
    }

    #[test]
    fn zero_dimensions_fail_frame_inspection() {
        for (w, h) in [(8, 0), (0, 8)] {
            let mut jpg = flat_header(w, h, &[1]);
            jpg.extend(sos(&[1], [0, 63, 0]));
            jpg.extend([0x3f, 0xff, 0xd9]);
            assert!(matches!(frame_info(&jpg), Err(RawError::Corrupt(_))), "{w}x{h}");
            assert!(matches!(decode(&jpg, 1 << 20), Err(RawError::Corrupt(_))), "{w}x{h}");
        }
        let mut jpg = flat_header(8, 8, &[1]);
        jpg.extend(sos(&[1], [0, 63, 0]));
        jpg.extend([0x3f, 0xff, 0xd9]);
        assert_eq!(frame_info(&jpg), Ok((8, 8, 1, 12)));
        assert_eq!(decode(&jpg, 64).unwrap().data, vec![2048; 64]);
    }

    #[test]
    fn scan_header_must_be_complete_and_sequential() {
        // one block, `0 0` padded with ones
        let ok = |scan: &[u8]| {
            let mut jpg = flat_header(8, 8, &[1]);
            jpg.extend(scan);
            jpg.extend([0x3f, 0xff, 0xd9]);
            decode(&jpg, 64)
        };
        assert!(ok(&sos(&[1], [0, 63, 0])).is_ok());
        // the selectors without Ss, Se and Ah/Al
        assert!(matches!(ok(&[0xff, 0xda, 0, 5, 1, 1, 0]), Err(RawError::Corrupt(m)) if m.contains("scan header")));
        for params in [[1, 63, 0], [0, 5, 0], [0, 63, 1], [0, 63, 0x10]] {
            assert!(matches!(ok(&sos(&[1], params)), Err(RawError::Corrupt(m)) if m.contains("sequential")), "{params:?}");
        }
    }

    #[test]
    fn scan_must_code_each_frame_component_once_in_order() {
        let run = |scan_ids: &[u8]| {
            let mut jpg = flat_header(8, 8, &[1, 2]);
            jpg.extend(sos(scan_ids, [0, 63, 0]));
            jpg.extend([0x0f, 0xff, 0xd9]); // two blocks
            decode(&jpg, 128)
        };
        assert_eq!(run(&[1, 2]).unwrap().data, vec![2048; 128]);
        for ids in [[1, 1], [2, 1], [2, 2], [1, 3]] {
            assert!(matches!(run(&ids), Err(RawError::Corrupt(m)) if m.contains("components")), "{ids:?}");
        }
    }

    #[test]
    fn an_unallocatable_frame_is_a_limit_error() {
        // within any sample budget, past what `Vec` can hold: a limit error, not an allocation abort
        assert!(matches!(zeroed(usize::MAX / 2), Err(RawError::Limit(_))));
        assert_eq!(zeroed(3).unwrap(), vec![0; 3]);
        let mut jpg = flat_header(65535, 65535, &[1, 2, 3, 4]);
        jpg.extend(sos(&[1, 2, 3, 4], [0, 63, 0]));
        jpg.extend([0xff, 0xd9]);
        // 65535^2 x 4 samples is 34 GB of output: refused by the budget here, never allocated
        assert!(matches!(decode(&jpg, 1 << 20), Err(RawError::Limit(_))));
    }

    #[test]
    fn the_outer_eoi_is_required_after_the_scan() {
        let mut jpg = flat_header(8, 8, &[1]);
        jpg.extend(sos(&[1], [0, 63, 0]));
        jpg.push(0x3f);
        // an APP1 segment holding a thumbnail's EOI, then no outer EOI
        let app = [0xff, 0xe1, 0, 8, 0xff, 0xd8, 0, 0, 0xff, 0xd9];
        let mut cut = jpg.clone();
        cut.extend(app);
        assert!(matches!(decode(&cut, 64), Err(RawError::Corrupt(m)) if m.contains("EOI")));
        // the same segment followed by the outer EOI decodes
        cut.extend([0xff, 0xd9]);
        assert_eq!(decode(&cut, 64).unwrap().data, vec![2048; 64]);
        // a second scan is not this decoder's layout
        let mut two = jpg.clone();
        two.extend(sos(&[1], [0, 63, 0]));
        two.extend([0x3f, 0xff, 0xd9]);
        assert!(matches!(decode(&two, 64), Err(RawError::Corrupt(_))));
    }

    #[test]
    fn restart_markers_follow_the_interval_sequence() {
        // DRI 1 over three blocks: RST0 then RST1, each right after its interval's padded byte
        let build = |markers: [u8; 2], gap: &[u8]| {
            let mut jpg = flat_header(24, 8, &[1]);
            jpg.extend([0xff, 0xdd, 0, 4, 0, 1]);
            jpg.extend(sos(&[1], [0, 63, 0]));
            jpg.push(0x3f);
            jpg.extend(gap);
            jpg.extend([0xff, markers[0], 0x3f, 0xff, markers[1], 0x3f, 0xff, 0xd9]);
            decode(&jpg, 1 << 10)
        };
        assert_eq!(build([0xd0, 0xd1], &[]).unwrap().data, vec![2048; 192]);
        // FF fill bytes may precede a marker
        assert!(build([0xd0, 0xd1], &[0xff]).is_ok());
        for markers in [[0xd3, 0xd4], [0xd1, 0xd2], [0xd0, 0xd0], [0xd0, 0xd2]] {
            assert!(matches!(build(markers, &[]), Err(RawError::Corrupt(m)) if m.contains("restart")), "{markers:x?}");
        }
        // unrelated data between the interval and its marker is not skipped
        assert!(matches!(build([0xd0, 0xd1], &[0x12, 0x34]), Err(RawError::Corrupt(m)) if m.contains("restart")));
    }

    #[test]
    fn blocks_may_not_consume_bits_after_the_entropy_data() {
        let mut jpg = flat_header(8, 8, &[1]);
        jpg.extend(sos(&[1], [0, 63, 0]));
        // no entropy bytes: the block's `0 0` would come from the zeros fed past the data
        let mut empty = jpg.clone();
        empty.extend([0xff, 0xd9]);
        assert!(matches!(decode(&empty, 64), Err(RawError::Corrupt(m)) if m.contains("ends early")));
        // five blocks, one byte (four blocks' bits): the fifth runs past the data
        let mut short = flat_header(40, 8, &[1]);
        short.extend(sos(&[1], [0, 63, 0]));
        short.extend([0x00, 0xff, 0xd9]);
        assert!(matches!(decode(&short, 320), Err(RawError::Corrupt(m)) if m.contains("ends early")));
        // the block's real bits suffice even though the reader's lookahead reaches the EOI
        let mut exact = jpg;
        exact.extend([0x3f, 0xff, 0xd9]);
        assert_eq!(decode(&exact, 64).unwrap().data, vec![2048; 64]);
    }
}
