//! Fujifilm's striped predictive RAW compression.
//!
//! Clean-room implementation: Fabian's prose description of lossless RAF compression
//! (https://capnfabs.net/posts/fuji-raf-compression-algorithm/) supplied the broad structure;
//! headers, colour-vector placement, predictors, contexts and adaptation were recovered
//! from sample bytes and black-box comparisons of decoded sensor arrays. No other RAW
//! decoder's source was consulted. See docs/raf-compression.md for verification details.

use crate::{Limits, Mode, RawError, Result};
use rayon::prelude::*;

const BLOCK: usize = 768;
const XTRANS: [[usize; 6]; 6] =
    [[1, 1, 0, 1, 1, 2], [1, 1, 2, 1, 1, 0], [2, 0, 1, 0, 2, 1], [1, 1, 2, 1, 1, 0], [1, 1, 0, 1, 1, 2], [0, 2, 1, 2, 0, 1]];
const MASKS: [[u8; 2]; 6] = [[10, 15], [15, 10], [14, 10], [15, 11], [11, 15], [10, 14]];

fn corrupt(why: &str) -> RawError {
    RawError::Corrupt(format!("Fujifilm compressed RAF: {why}"))
}

fn be16(src: &[u8], at: usize) -> Result<usize> {
    let s = src.get(at..at.saturating_add(2)).ok_or_else(|| corrupt("truncated header"))?;
    let [a, b] = s else { return Err(corrupt("truncated header")) };
    Ok(u16::from_be_bytes([*a, *b]) as usize)
}

struct Stripe<'a> {
    bytes: &'a [u8],
    quantizers: Option<&'a [u8]>,
    width: usize,
}

pub(crate) fn decode(src: &[u8], width: usize, height: usize, bits: u32, xtrans: bool, mode: Mode, limits: &Limits) -> Result<Vec<u16>> {
    let h = src.get(..16).ok_or_else(|| corrupt("truncated header"))?;
    if !h.starts_with(b"IS") {
        return Err(corrupt("missing compression signature"));
    }
    let flag = h.get(2).copied().ok_or_else(|| corrupt("truncated header"))?;
    if flag > 1 {
        return Err(corrupt("invalid compression flag"));
    }
    let sensor = h.get(3).copied().ok_or_else(|| corrupt("truncated header"))?;
    if sensor != if xtrans { 16 } else { 0 } {
        return Err(corrupt("compression sensor type disagrees with CFA"));
    }
    if !matches!(bits, 12 | 14 | 16) {
        return Err(RawError::Unsupported(format!("Fujifilm compressed RAF at {bits} bits")));
    }
    let count = h.get(13).copied().ok_or_else(|| corrupt("truncated header"))? as usize;
    if h.get(4).copied() != Some(bits as u8)
        || be16(h, 5)? != height
        || be16(h, 9)? != width
        || count == 0
        || count != width.div_ceil(BLOCK)
        || be16(h, 7)? != count * BLOCK
        || be16(h, 11)? != BLOCK
        || height == 0
        || !height.is_multiple_of(6)
        || be16(h, 14)? != height / 6
    {
        return Err(corrupt("inconsistent compression dimensions"));
    }
    let n = width.checked_mul(height).filter(|&n| n <= limits.samples()).ok_or(RawError::Limit("image too large"))?;
    let table_end = 16 + count * 4;
    let mut offset = table_end.div_ceil(16) * 16;
    let quantizer_start = offset;
    let quantizer_stride = (height / 6).div_ceil(16) * 16;
    if flag == 0 {
        offset += quantizer_stride * count;
        if src.get(quantizer_start..offset).is_none() {
            return Err(corrupt("truncated quantizer tables"));
        }
    }
    let mut stripes = Vec::with_capacity(count);
    for i in 0..count {
        let entry = src.get(16 + i * 4..20 + i * 4).ok_or_else(|| corrupt("truncated stripe size table"))?;
        let [a, b, c, d] = entry else { return Err(corrupt("truncated stripe size table")) };
        let size = u32::from_be_bytes([*a, *b, *c, *d]) as usize;
        let end = offset.checked_add(size).ok_or_else(|| corrupt("stripe size overflow"))?;
        let bytes = src.get(offset..end).filter(|s| !s.is_empty()).ok_or_else(|| corrupt("stripe outside file"))?;
        let quantizers = if flag == 0 {
            Some(
                src.get(quantizer_start + i * quantizer_stride..quantizer_start + i * quantizer_stride + height / 6)
                    .ok_or_else(|| corrupt("truncated quantizer table"))?,
            )
        } else {
            None
        };
        stripes.push(Stripe { bytes, quantizers, width: (width - i * BLOCK).min(BLOCK) });
        offset = end;
    }
    if mode == Mode::Header {
        return Ok(Vec::new());
    }
    let decoded: Vec<Vec<u16>> = stripes.par_iter().map(|s| decode_stripe(s, height, bits, xtrans)).collect::<Result<_>>()?;
    let mut output = vec![0; n];
    for (i, (stripe, data)) in stripes.iter().zip(&decoded).enumerate() {
        for (row, samples) in output.chunks_exact_mut(width).zip(data.chunks_exact(stripe.width)) {
            row.get_mut(i * BLOCK..i * BLOCK + stripe.width).ok_or_else(|| corrupt("invalid stripe placement"))?.copy_from_slice(samples);
        }
    }
    Ok(output)
}

struct Bits<'a> {
    src: &'a [u8],
    pos: usize,
    acc: u64,
    available: u32,
}

impl<'a> Bits<'a> {
    fn new(src: &'a [u8]) -> Self {
        Self { src, pos: 0, acc: 0, available: 0 }
    }

    /// Tops the accumulator up to at least 57 bits, or to every remaining byte near the end of the stream.
    #[inline]
    fn refill(&mut self) {
        while self.available <= 56 {
            let Some(&b) = self.src.get(self.pos) else { return };
            self.pos += 1;
            self.acc = (self.acc << 8) | u64::from(b);
            self.available += 8;
        }
    }

    #[inline]
    fn get(&mut self, n: u32) -> Result<u32> {
        if self.available < n {
            self.refill();
            if self.available < n {
                return Err(corrupt("truncated entropy stream"));
            }
        }
        self.available -= n;
        Ok(((self.acc >> self.available) & ((1u64 << n) - 1)) as u32)
    }

    /// Reads the unary prefix: the zeros before the next one bit, which is consumed. Errors exactly where reading
    /// bit by bit would: more than `limit` zeros, or a stream that ends first.
    #[inline]
    fn zeros(&mut self, limit: u32) -> Result<u32> {
        let mut zeros = 0;
        loop {
            if self.available == 0 {
                self.refill();
                if self.available == 0 {
                    return Err(corrupt("truncated entropy stream"));
                }
            }
            // the unread bits, left-aligned; the zeros shifted in below them are cut off by `min`
            let run = (self.acc << (64 - self.available)).leading_zeros().min(self.available);
            if zeros + run > limit {
                return Err(corrupt("invalid residual escape"));
            }
            zeros += run;
            if run < self.available {
                self.available -= run + 1;
                return Ok(zeros);
            }
            self.available = 0;
        }
    }

    fn residual(&mut self, k: u32, bits: u32, range: u32, limit: u32) -> Result<i32> {
        let zeros = self.zeros(limit)?;
        let code = if zeros == limit { self.get(bits)? + 1 } else { (zeros << k) | self.get(k)? };
        if code > range {
            return Err(corrupt(&format!(
                "residual {code} outside sample range {range} (k={k}, prefix={zeros}, bits={bits}, position={})",
                self.pos * 8 - self.available as usize
            )));
        }
        Ok(if code & 1 == 0 { (code / 2) as i32 } else { -((code / 2) as i32) - 1 })
    }
}

#[derive(Clone, Copy)]
struct State {
    sum: u32,
    count: u32,
}

impl State {
    fn k(self) -> u32 {
        let mut k = 0;
        while self.count << k < self.sum {
            k += 1;
        }
        k
    }

    fn update(&mut self, delta: i32) {
        self.sum += delta.unsigned_abs();
        if self.count == 64 {
            self.sum >>= 1;
            self.count >>= 1;
        }
        self.count += 1;
    }
}

#[inline]
fn quant(v: i32, near: u32) -> i32 {
    let t = [18 + 3 * near, 67 + 5 * near, 276 + 7 * near];
    let a = v.unsigned_abs();
    let bucket = if a <= near {
        0
    } else if a < t[0] {
        1
    } else if a < t[1] {
        2
    } else if a < t[2] {
        3
    } else {
        4
    };
    if v < 0 { -bucket } else { bucket }
}

#[inline]
fn even_prediction(nw: i32, n: i32, ne: i32, nn: i32) -> i32 {
    let (a, b, c) = ((nw - n).abs(), (ne - n).abs(), (nn - n).abs());
    let pair = if a > b && a > c {
        ne + nn
    } else if b > a && b > c {
        nw + nn
    } else {
        nw + ne
    };
    (pair + 2 * n) >> 2
}

#[inline]
fn sample(rows: &[Vec<i32>], y: usize, x: usize) -> Result<i32> {
    rows.get(y).and_then(|r| r.get(x)).copied().ok_or_else(|| corrupt("invalid predictor position"))
}

fn decode_stripe(stripe: &Stripe<'_>, height: usize, bits: u32, xtrans: bool) -> Result<Vec<u16>> {
    let size = if xtrans { BLOCK * 2 / 3 } else { BLOCK / 2 };
    let mut previous = vec![vec![vec![0; size + 2]; 2]; 3];
    let max = (1u32 << bits) - 1;
    let range = |near: u32| (max + 2 * near) / (2 * near + 1) + 1;
    let seed = |near: u32| State { sum: ((range(near) + 32) / 64).max(2), count: 1 };
    let mut states = [[[seed(0); 41]; 2]; 3];
    // Smooth regions retain finer precision and independent state across block quantizer changes.
    let mut fine = [[[[seed(0); 5]; 2]; 3], [[[seed(1); 5]; 2]; 3], [[[seed(2); 5]; 2]; 3]];
    let mut last_near = None;
    let mut reader = Bits::new(stripe.bytes);
    let n = height.checked_mul(stripe.width).filter(|&n| n <= crate::MAX_SAMPLES).ok_or(RawError::Limit("image too large"))?;
    // a capacity hint only: the stripe's declared height is the file's word, so the reservation is capped at a sample
    // per bit of the stripe's real bytes (all stripes reserve in parallel; a one-byte stripe must not take 2 GiB)
    let mut output = Vec::with_capacity(n.min(stripe.bytes.len().saturating_mul(8)));
    for group in 0..height / 6 {
        let near = match stripe.quantizers {
            Some(q) => u32::from(*q.get(group).ok_or_else(|| corrupt("truncated quantizer table"))?),
            None => 0,
        };
        if last_near != Some(near) {
            states = [[[seed(near); 41]; 2]; 3];
            last_near = Some(near);
        }
        let mut rows = previous;
        for pair in 0..6 {
            let colours = if pair % 2 == 0 { [0, 1] } else { [1, 2] };
            for &colour in &colours {
                let lines = rows.get_mut(colour).ok_or_else(|| corrupt("invalid colour vector"))?;
                let mut row = vec![0; size + 2];
                let prev = lines.len() - 1;
                let left = sample(lines, prev, 1)?;
                let right = sample(lines, prev, size)?;
                if let Some(v) = row.first_mut() {
                    *v = left;
                }
                if let Some(v) = row.last_mut() {
                    *v = right;
                }
                lines.push(row);
            }
            for even in (0..size + 8).step_by(2) {
                for (odd, position) in [(0, Some(even)), (1, even.checked_sub(7))] {
                    let Some(pos) = position.filter(|&x| x < size) else { continue };
                    let x = pos + 1;
                    for (which, &colour) in colours.iter().enumerate() {
                        let lines = rows.get_mut(colour).ok_or_else(|| corrupt("invalid colour vector"))?;
                        let y = lines.len() - 1;
                        let (nw, north, ne, nn) =
                            (sample(lines, y - 1, x - 1)?, sample(lines, y - 1, x)?, sample(lines, y - 1, x + 1)?, sample(lines, y - 2, x)?);
                        let west = sample(lines, y, x - 1)?;
                        let east = sample(lines, y, x + 1)?;
                        let (predict, d1, d2) = if odd == 0 {
                            (even_prediction(nw, north, ne, nn), north - nn, nw - north)
                        } else {
                            let p = if (nw.min(ne)..=nw.max(ne)).contains(&north) { (west + east) >> 1 } else { (west + east + 2 * north) >> 2 };
                            (p, north - nw, nw - west)
                        };
                        let real = !xtrans || MASKS.get(pair).and_then(|m| m.get(which)).is_some_and(|m| (m >> (pos % 4)) & 1 != 0);
                        let value = if real {
                            let contrast = d1.unsigned_abs() + d2.unsigned_abs();
                            let precision = match contrast {
                                0..=5 => 0,
                                6 => near.min(1),
                                7 => near.min(2),
                                _ => near,
                            };
                            let (context, state) = if precision < near {
                                let sign = |v: i32| if v.unsigned_abs() <= precision { 0 } else { v.signum() };
                                let context = 3 * sign(d1) + sign(d2);
                                let state = fine
                                    .get_mut(precision as usize)
                                    .and_then(|f| f.get_mut(pair % 3))
                                    .and_then(|s| s.get_mut(odd))
                                    .and_then(|s| s.get_mut(context.unsigned_abs() as usize))
                                    .ok_or_else(|| corrupt("invalid fine coding context"))?;
                                (context, state)
                            } else {
                                let context = 9 * quant(d1, near) + quant(d2, near);
                                let state = states
                                    .get_mut(pair % 3)
                                    .and_then(|s| s.get_mut(odd))
                                    .and_then(|s| s.get_mut(context.unsigned_abs() as usize))
                                    .ok_or_else(|| corrupt("invalid coding context"))?;
                                (context, state)
                            };
                            let coded_range = range(precision);
                            let coded_bits = 32 - (coded_range - 1).leading_zeros();
                            let mut delta = reader.residual(state.k(), coded_bits, coded_range, 4 * bits - coded_bits - 1)?;
                            state.update(delta);
                            if context < 0 {
                                delta = -delta;
                            }
                            let step = (2 * precision + 1) as i32;
                            let mut value = predict + delta * step;
                            if value < -(precision as i32) {
                                value += coded_range as i32 * step;
                            } else if value > (max + precision) as i32 {
                                value -= coded_range as i32 * step;
                            }
                            value.clamp(0, max as i32)
                        } else {
                            predict
                        };
                        *lines.get_mut(y).and_then(|r| r.get_mut(x)).ok_or_else(|| corrupt("invalid predictor position"))? = value;
                    }
                }
            }
        }
        for y in 0..6 {
            for x in 0..stripe.width {
                let colour = if xtrans {
                    XTRANS.get(y).and_then(|r| r.get(x % 6)).copied().ok_or_else(|| corrupt("invalid CFA position"))?
                } else {
                    match (y % 2, x % 2) {
                        (0, 0) => 0,
                        (1, 1) => 2,
                        _ => 1,
                    }
                };
                let row = 2 + if colour == 1 { y } else { y / 2 };
                let col = 1 + if xtrans { 2 * (x / 3) + (x % 3).min(1) } else { x / 2 };
                output.push(sample(rows.get(colour).ok_or_else(|| corrupt("invalid colour vector"))?, row, col)? as u16);
            }
        }
        previous = rows.into_iter().map(|mut r| r.split_off(r.len() - 2)).collect();
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cfa;
    use proptest::prelude::*;

    /// A one-byte stripe declaring the tallest height the header can carry fails as corrupt (its capacity hint is
    /// capped at the stripe's bits, not the declared height × 768 samples).
    #[test]
    fn a_tiny_stripe_with_a_huge_height_is_corrupt() {
        let stripe = Stripe { bytes: &[0], quantizers: None, width: BLOCK };
        for xtrans in [false, true] {
            let got = decode_stripe(&stripe, 65532, 14, xtrans);
            assert!(matches!(got, Err(RawError::Corrupt(_))), "xtrans {xtrans}: {got:?}");
        }
    }

    /// Procedural zero residuals: no camera media or encoder dependency.
    fn zero_stream(bits: u32, xtrans: bool, quantizers: Option<&[u8]>) -> Vec<u8> {
        let groups = quantizers.map_or(3, <[u8]>::len);
        let mut main = [[(1u32 << (bits - 6), 1u32); 2]; 3];
        let mut fine = main;
        let mut previous = None;
        let mut stream = Vec::new();
        for group in 0..groups {
            let q = quantizers.map_or(0, |q| q[group]);
            if previous != Some(q) {
                main = [[(1u32 << (bits - 6), 1u32); 2]; 3];
                previous = Some(q);
            }
            for (pair, masks) in MASKS.iter().enumerate() {
                let width = if xtrans { 512 } else { 384 };
                for even in (0usize..width + 8).step_by(2) {
                    for (parity, column) in [(0, Some(even)), (1, even.checked_sub(7))] {
                        let Some(column) = column.filter(|&x| x < width) else { continue };
                        for mask in masks {
                            if xtrans && (mask >> (column % 4)) & 1 == 0 {
                                continue;
                            }
                            let (sum, count) = &mut if q == 0 { &mut main } else { &mut fine }[pair % 3][parity];
                            let k = 32 - (sum.saturating_sub(1) / *count).leading_zeros();
                            stream.push(true);
                            stream.extend(std::iter::repeat_n(false, k as usize));
                            if *count == 64 {
                                *sum /= 2;
                                *count /= 2;
                            }
                            *count += 1;
                        }
                    }
                }
            }
        }
        stream.chunks(8).map(|chunk| chunk.iter().enumerate().fold(0, |b, (i, bit)| b | (u8::from(*bit) << (7 - i)))).collect()
    }

    fn compressed(bits: u32, xtrans: bool, quantizers: Option<&[u8]>) -> Vec<u8> {
        let groups = quantizers.map_or(3, <[u8]>::len);
        let stream = zero_stream(bits, xtrans, quantizers);
        // Two stripes; the second is mostly padding, which must never enter the output.
        let mut out = b"IS".to_vec();
        out.extend_from_slice(&[u8::from(quantizers.is_none()), if xtrans { 16 } else { 0 }, bits as u8]);
        for v in [groups as u16 * 6, 1536, 780, 768] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.push(2);
        out.extend_from_slice(&(groups as u16).to_be_bytes());
        for _ in 0..2 {
            out.extend_from_slice(&(stream.len() as u32).to_be_bytes());
        }
        out.resize(32, 0);
        if let Some(q) = quantizers {
            for _ in 0..2 {
                out.extend_from_slice(q);
                out.resize(out.len() + q.len().div_ceil(16) * 16 - q.len(), 0);
            }
        }
        out.extend_from_slice(&stream);
        out.extend_from_slice(&stream);
        out
    }

    #[test]
    fn lossless_and_lossy_bayer_and_xtrans_stripes() {
        for bits in [12, 14, 16] {
            for xtrans in [false, true] {
                for q in [None, Some([2, 0, 5].as_slice())] {
                    let src = compressed(bits, xtrans, q);
                    let out = decode(&src, 780, 18, bits, xtrans, Mode::Full, &Limits::default()).unwrap();
                    assert_eq!(out, vec![0; 780 * 18]);
                    assert!(decode(&src, 780, 18, bits, xtrans, Mode::Header, &Limits::default()).unwrap().is_empty());
                    let layout = xtrans.then(|| {
                        let mut l = [0; 36];
                        for (to, from) in l.iter_mut().zip(XTRANS.iter().flatten().rev()) {
                            *to = *from as u8;
                        }
                        l
                    });
                    let raf = crate::raf::tests::raf(780, 18, bits, src, layout, &[]);
                    let image = crate::decode(&raf).unwrap();
                    assert_eq!(crate::probe_info(&raf).unwrap(), image.info());
                    assert_eq!(image.data, crate::RawData::U16(out));
                }
            }
        }
    }

    #[test]
    fn malformed_headers_and_truncated_entropy_are_errors() {
        let valid = compressed(14, true, Some(&[1, 2, 0]));
        for len in (0..96).chain([valid.len() / 2, valid.len() - 1]) {
            assert!(decode(&valid[..len], 780, 18, 14, true, Mode::Full, &Limits::default()).is_err(), "length {len}");
        }
        for at in [2, 3, 4, 5, 7, 9, 11, 13, 14, 16] {
            let mut src = valid.clone();
            src[at] ^= 0xff;
            assert!(decode(&src, 780, 18, 14, true, Mode::Full, &Limits::default()).is_err(), "header byte {at}");
        }
        assert!(decode(&valid, usize::MAX, usize::MAX, 14, true, Mode::Full, &Limits::default()).is_err());
        let mut reader = Bits::new(&[0; 64]);
        assert!(reader.residual(0, 14, 16384, 41).is_err());
    }

    #[test]
    fn unsupported_container_modes_and_cfa_are_not_silently_decoded() {
        let layout = std::array::from_fn(|i| Cfa::xtrans().pattern[35 - i]);
        let src = compressed(14, true, None);
        let raf = crate::raf::tests::raf(780, 18, 14, src, Some(layout), &[]);
        let mut mismatched = raf.clone();
        mismatched[108..112].copy_from_slice(&3u32.to_be_bytes());
        assert!(crate::decode(&mismatched).is_err(), "declared lossy mode disagrees with lossless stream");
        let bad_layout = layout.map(|c| {
            if c == 0 {
                2
            } else if c == 2 {
                0
            } else {
                c
            }
        });
        let raf = crate::raf::tests::raf(780, 18, 14, compressed(14, true, None), Some(bad_layout), &[]);
        assert!(matches!(crate::decode(&raf), Err(RawError::Unsupported(_))), "compression uses a fixed X-Trans arrangement");
        let mut raf = crate::raf::tests::raf(780, 18, 16, vec![0; 780 * 18 * 2], None, &[]);
        for mode in [1u32, 4, u32::MAX] {
            raf[108..112].copy_from_slice(&mode.to_be_bytes());
            assert!(matches!(crate::decode(&raf), Err(RawError::Unsupported(_))), "unknown mode {mode}");
            assert!(matches!(crate::probe_info(&raf), Err(RawError::Unsupported(_))), "header-only mode {mode}");
        }
    }

    #[test]
    fn residual_sign_wrap_and_escape() {
        // Ordinary codes 0, 1, 2, 3 with k=2; then the escape for -8192.
        let mut stream = vec![true, false, false, true, false, true, true, true, false, true, true, true];
        stream.extend(std::iter::repeat_n(false, 41));
        stream.push(true);
        let escaped = 16382u32;
        stream.extend((0..14).rev().map(|i| (escaped >> i) & 1 != 0));
        let bytes: Vec<u8> = stream.chunks(8).map(|chunk| chunk.iter().enumerate().fold(0, |b, (i, bit)| b | (u8::from(*bit) << (7 - i)))).collect();
        let mut reader = Bits::new(&bytes);
        for delta in [0, -1, 1, -2, -8192] {
            assert_eq!(reader.residual(2, 14, 16384, 41).unwrap(), delta);
        }
        assert_eq!(even_prediction(1112, 858, 1104, 1152), 983);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        #[test]
        fn malformed_streams_never_panic(data in prop::collection::vec(any::<u8>(), 0..2048), quantizer in any::<u8>(), bits in prop::sample::select(vec![12,14,16]), xtrans in any::<bool>()) {
            let mut src = compressed(bits, xtrans, Some(&[quantizer]));
            let start = 64;
            src.truncate(start);
            for i in [16, 20] { src[i..i+4].copy_from_slice(&(data.len() as u32).to_be_bytes()); }
            src.extend_from_slice(&data);
            src.extend_from_slice(&data);
            let _ = decode(&src, 780, 6, bits, xtrans, Mode::Full, &Limits::default());
        }

        /// The word-at-a-time reader returns what reading bit by bit returns, and fails where it fails: long zero
        /// runs, the escape limit and the end of the stream included.
        #[test]
        fn bit_reader_matches_reading_bit_by_bit(
            data in prop::collection::vec(prop_oneof![3 => Just(0u8), 1 => any::<u8>()], 0..64),
            reads in prop::collection::vec((any::<bool>(), 0u32..=24, 0u32..=63), 1..200),
        ) {
            let bit = |i: usize| data.get(i / 8).map(|b| (b >> (7 - i % 8)) & 1 == 1);
            let mut at = 0usize;
            let mut reader = Bits::new(&data);
            for (prefix, n, limit) in reads {
                let expected = if prefix {
                    let mut zeros = 0;
                    loop {
                        let Some(b) = bit(at) else { break Err("truncated") };
                        at += 1;
                        if b {
                            break Ok(zeros);
                        }
                        zeros += 1;
                        if zeros > limit {
                            break Err("escape");
                        }
                    }
                } else if at + n as usize > data.len() * 8 {
                    Err("truncated")
                } else {
                    let value = (0..n as usize).fold(0, |v, i| (v << 1) | u32::from(bit(at + i) == Some(true)));
                    at += n as usize;
                    Ok(value)
                };
                let actual = if prefix { reader.zeros(limit) } else { reader.get(n) };
                match (expected, actual) {
                    (Ok(want), Ok(got)) => prop_assert_eq!(want, got),
                    (Err(want), Err(got)) => {
                        prop_assert!(got.to_string().contains(want), "{got}");
                        break;
                    }
                    (want, got) => prop_assert!(false, "expected {want:?}, got {got:?}"),
                }
            }
        }
    }
}
