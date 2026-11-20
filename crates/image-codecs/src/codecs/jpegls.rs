//! JPEG-LS (ITU-T T.87 | ISO/IEC 14495-1, "LOCO-I"): lossless and near-lossless gray and RGB
//! images, 2 to 16 bits per sample.
//!
//! Read: baseline JPEG-LS with the default or `LSE`-preset coding parameters, any `NEAR`, and
//! scans that code one component each (`ILV` 0), which is also how this module writes. Line- and
//! sample-interleaved scans, colour transforms and mapping tables are reported as unsupported.
//! Write: lossless (`NEAR` 0), 8 or 16 bits, gray or RGB, one scan per component.
//!
//! Samples of 9 to 15 bits are scaled to 16 bits on read (the top bits are repeated into the low
//! ones), so a decoded image always spans the full range of its sample type.

use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::format::Format;
use crate::image::{ChannelLayout, Image, SampleType};
use crate::options::Limits;

const F: Format = Format::JpegLs;
const SOF55: u8 = 0xF7;
const LSE: u8 = 0xF8;
const RESET_DEFAULT: i32 = 64;
const MIN_C: i32 = -128;
const MAX_C: i32 = 127;
/// `J[RUNindex]`: the log2 of the run length a run-index step codes.
const J: [u32; 32] = [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// Context slots: the regular contexts (365 of the 405 slots the index below can reach), then the two
/// run-interruption contexts.
const RI_BASE: usize = 405;
const CONTEXTS: usize = RI_BASE + 2;

fn bad(reason: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, reason)
}

/// What the marker segments after a JPEG SOI lead to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JpegFrame {
    /// An SOF55 frame header: JPEG-LS.
    Ls,
    /// Another frame header, a scan header, an EOI, or a byte that is not a marker where one
    /// should be.
    Other,
    /// The bytes end before a frame or scan header. The next step reads the bytes up to this
    /// length.
    Short(usize),
}

/// Walks the marker segments of `b` from its SOI up to the first frame or scan header. `None`
/// when `b` does not start with an SOI. Each step moves forward at least one byte, so the walk
/// takes at most `b.len()` steps and reaches the frame header after any number of segments.
pub(crate) fn jpeg_frame(b: &[u8]) -> Option<JpegFrame> {
    jpeg_frame_from(b, &mut 0)
}

/// [`jpeg_frame`], started at `*pos`. A `*pos` of 0 starts at the SOI. Any other value is the
/// offset of the marker where an earlier walk of a shorter prefix of the same file stopped, and the
/// walk does not read the bytes before it. The walk leaves `*pos` at the marker it stopped at, so
/// walking longer and longer prefixes of one file this way reads each marker segment once.
pub(crate) fn jpeg_frame_from(b: &[u8], pos: &mut usize) -> Option<JpegFrame> {
    if *pos == 0 {
        if !b.starts_with(&[0xFF, 0xD8]) {
            return None;
        }
        *pos = 2;
    }
    loop {
        let (Some(&prefix), Some(&marker)) = (b.get(*pos), b.get(*pos + 1)) else {
            return Some(if b.get(*pos).is_none_or(|&p| p == 0xFF) { JpegFrame::Short(*pos + 2) } else { JpegFrame::Other });
        };
        if prefix != 0xFF {
            return Some(JpegFrame::Other);
        }
        match marker {
            SOF55 => return Some(JpegFrame::Ls),
            // `FF 00` is a stuffed zero, not a marker.
            0x00 | 0xDA | 0xD9 => return Some(JpegFrame::Other),
            // A DRI segment (`0xDD`) may precede an SOF55 frame header, so it takes the
            // length-prefixed path below.
            0xC0..=0xCF | 0xDB | 0xDC | 0xDE | 0xDF => return Some(JpegFrame::Other),
            0x01 | 0xD0..=0xD7 | 0xFF => *pos += if marker == 0xFF { 1 } else { 2 },
            _ => {
                let Some(len) = b.get(*pos + 2..*pos + 4).map(|l| usize::from(u16::from_be_bytes([l[0], l[1]]))) else {
                    return Some(JpegFrame::Short(*pos + 4));
                };
                *pos += 2 + len;
            }
        }
    }
}

/// Whether `b` is a JPEG-LS file: an SOI followed by marker segments up to an SOF55 frame header.
pub(crate) fn looks_like_jpegls(b: &[u8]) -> bool {
    jpeg_frame(b) == Some(JpegFrame::Ls)
}

/// The coding parameters of one scan, derived as T.87 section C.2 specifies.
#[derive(Clone, Copy)]
struct Params {
    maxval: i32,
    near: i32,
    range: i32,
    qbpp: u32,
    limit: u32,
    t1: i32,
    t2: i32,
    t3: i32,
    reset: i32,
}

fn ceil_log2(v: i32) -> u32 {
    let mut n = 0;
    while (1i64 << n) < i64::from(v) {
        n += 1;
    }
    n
}

impl Params {
    /// `preset` holds the `LSE` values (`maxval`, `t1`, `t2`, `t3`, `reset`), 0 where absent.
    fn new(precision: u32, near: i32, preset: [i32; 5]) -> Result<Params, CodecError> {
        let default_max = (1i32 << precision) - 1;
        let maxval = if preset[0] == 0 { default_max } else { preset[0] };
        if maxval < 1 || maxval > default_max || near < 0 || near > (maxval / 2).min(255) {
            return Err(bad("invalid MAXVAL or NEAR"));
        }
        let clamp = |v: i32, lo: i32, hi: i32| if v > hi || v < lo { lo } else { v };
        let (d1, d2, d3) = if maxval >= 128 {
            let factor = (maxval.min(4095) + 128) / 256;
            let t1 = clamp(factor + 2 + 3 * near, near + 1, maxval);
            let t2 = clamp(factor * 4 + 3 + 5 * near, t1, maxval);
            let t3 = clamp(factor * 17 + 4 + 7 * near, t2, maxval);
            (t1, t2, t3)
        } else {
            let factor = 256 / (maxval + 1);
            let t1 = clamp((3 / factor + 3 * near).max(2), near + 1, maxval);
            let t2 = clamp((7 / factor + 5 * near).max(3), t1, maxval);
            let t3 = clamp((21 / factor + 7 * near).max(4), t2, maxval);
            (t1, t2, t3)
        };
        let t1 = if preset[1] == 0 { d1 } else { preset[1] };
        let t2 = if preset[2] == 0 { d2 } else { preset[2] };
        let t3 = if preset[3] == 0 { d3 } else { preset[3] };
        let reset = if preset[4] == 0 { RESET_DEFAULT } else { preset[4] };
        if !(near < t1 && t1 <= t2 && t2 <= t3 && t3 <= maxval) || !(3..=255).contains(&reset) {
            return Err(bad("invalid threshold or RESET preset"));
        }
        let range = (maxval + 2 * near) / (2 * near + 1) + 1;
        let bpp = ceil_log2(maxval + 1).max(2);
        Ok(Params { maxval, near, range, qbpp: ceil_log2(range), limit: 2 * (bpp + bpp.max(8)), t1, t2, t3, reset })
    }
}

/// Adaptive state of one scan: contexts, bias and the run index.
struct State {
    a: [i32; CONTEXTS],
    b: [i32; CONTEXTS],
    c: [i32; CONTEXTS],
    n: [i32; CONTEXTS],
    nn: [i32; 2],
    run_index: usize,
}

impl State {
    fn new(p: &Params) -> State {
        let a0 = ((p.range + 32) / 64).max(2);
        State { a: [a0; CONTEXTS], b: [0; CONTEXTS], c: [0; CONTEXTS], n: [1; CONTEXTS], nn: [0; 2], run_index: 0 }
    }
}

fn quantise_gradient(d: i32, p: &Params) -> i32 {
    if d <= -p.t3 {
        -4
    } else if d <= -p.t2 {
        -3
    } else if d <= -p.t1 {
        -2
    } else if d < -p.near {
        -1
    } else if d <= p.near {
        0
    } else if d < p.t1 {
        1
    } else if d < p.t2 {
        2
    } else if d < p.t3 {
        3
    } else {
        4
    }
}

fn predict(ra: i32, rb: i32, rc: i32) -> i32 {
    if rc >= ra.max(rb) {
        ra.min(rb)
    } else if rc <= ra.min(rb) {
        ra.max(rb)
    } else {
        ra + rb - rc
    }
}

/// The error quantised for `NEAR` and reduced into the modulo range, as the encoder codes it.
fn reduce(mut e: i32, p: &Params) -> i32 {
    if p.near > 0 {
        e = if e > 0 { (e + p.near) / (2 * p.near + 1) } else { -(p.near - e) / (2 * p.near + 1) };
    }
    if e < 0 {
        e += p.range;
    }
    if e >= (p.range + 1) / 2 {
        e -= p.range;
    }
    e
}

/// The reconstructed sample for a decoded (sign-adjusted) error.
fn reconstruct(px: i32, sign: i32, e: i32, p: &Params) -> i32 {
    let mut r = px + sign * e * (2 * p.near + 1);
    if r < -p.near {
        r += p.range * (2 * p.near + 1);
    } else if r > p.maxval + p.near {
        r -= p.range * (2 * p.near + 1);
    }
    r.clamp(0, p.maxval)
}

fn golomb_k(n: i32, a: i32) -> u32 {
    let mut k = 0;
    while (i64::from(n) << k) < i64::from(a) {
        k += 1;
    }
    k
}

/// Bits of the entropy-coded segment, with T.87's stuffing: after an `0xFF` byte the next byte
/// carries only 7 bits.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    cur: u32,
    left: u32,
    prev_ff: bool,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0, cur: 0, left: 0, prev_ff: false }
    }

    fn bit(&mut self) -> Result<u32, CodecError> {
        if self.left == 0 {
            let byte = *self.data.get(self.pos).ok_or_else(|| bad("the entropy-coded data ends early"))?;
            self.pos += 1;
            if self.prev_ff {
                if byte & 0x80 != 0 {
                    return Err(bad("a marker inside entropy-coded data"));
                }
                self.cur = u32::from(byte);
                self.left = 7;
            } else {
                self.cur = u32::from(byte);
                self.left = 8;
            }
            self.prev_ff = byte == 0xFF;
        }
        self.left -= 1;
        Ok((self.cur >> self.left) & 1)
    }

    fn bits(&mut self, n: u32) -> Result<u32, CodecError> {
        let mut v = 0;
        for _ in 0..n {
            v = v << 1 | self.bit()?;
        }
        Ok(v)
    }

    /// A limited-length Golomb code: the mapped error.
    fn golomb(&mut self, k: u32, limit: u32, qbpp: u32) -> Result<i32, CodecError> {
        let max_zeros = limit.checked_sub(qbpp + 1).ok_or_else(|| bad("invalid LIMIT"))?;
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > max_zeros {
                return Err(bad("a Golomb code is longer than LIMIT"));
            }
        }
        let v = if zeros < max_zeros { (zeros << k) + self.bits(k)? } else { self.bits(qbpp)? + 1 };
        i32::try_from(v).map_err(|_| bad("mapped error out of range"))
    }
}

/// Decode one component scan of `w` x `h` samples into `out` (row-major).
fn decode_scan(data: &[u8], w: usize, h: usize, p: &Params, out: &mut [u16]) -> Result<(), CodecError> {
    let mut r = BitReader::new(data);
    let mut st = State::new(p);
    let mut prev = vec![0i32; w + 2];
    let mut cur = vec![0i32; w + 2];
    for y in 0..h {
        // Ra at the first sample is Rb; the slot at index 0 is Rc there, left over from the line above.
        cur[0] = prev[1];
        if y > 0 {
            prev[w + 1] = prev[w];
        }
        let mut x = 0usize;
        while x < w {
            let (ra, rb, rc, rd) = (cur[x], prev[x + 1], prev[x], if y == 0 { 0 } else { prev[x + 2] });
            let (q1, q2, q3) = (quantise_gradient(rd - rb, p), quantise_gradient(rb - rc, p), quantise_gradient(rc - ra, p));
            if q1 == 0 && q2 == 0 && q3 == 0 {
                x = decode_run(&mut r, &mut st, p, &prev, &mut cur, ra, x, w)?;
                continue;
            }
            let (sign, q1, q2, q3) = if q1 < 0 || (q1 == 0 && (q2 < 0 || (q2 == 0 && q3 < 0))) { (-1, -q1, -q2, -q3) } else { (1, q1, q2, q3) };
            let q = (q1 * 81 + (q2 + 4) * 9 + (q3 + 4)) as usize;
            let k = golomb_k(st.n[q], st.a[q]);
            let m = r.golomb(k, p.limit, p.qbpp)?;
            let swap = p.near == 0 && k == 0 && 2 * st.b[q] <= -st.n[q];
            let e = if swap {
                if m & 1 == 1 { (m - 1) >> 1 } else { -(m >> 1) - 1 }
            } else if m & 1 == 0 {
                m >> 1
            } else {
                -((m + 1) >> 1)
            };
            let px = (predict(ra, rb, rc) + sign * st.c[q]).clamp(0, p.maxval);
            let rx = reconstruct(px, sign, e, p);
            update_regular(&mut st, q, e, p);
            cur[x + 1] = rx;
            x += 1;
        }
        let row = &mut out[y * w..(y + 1) * w];
        for (o, &v) in row.iter_mut().zip(&cur[1..=w]) {
            *o = v as u16;
        }
        // The line just decoded becomes the line above; its slot 0 is the Rc of the next line's first sample.
        std::mem::swap(&mut prev, &mut cur);
    }
    Ok(())
}

fn update_regular(st: &mut State, q: usize, e: i32, p: &Params) {
    st.b[q] += e * (2 * p.near + 1);
    st.a[q] += e.abs();
    if st.n[q] == p.reset {
        st.a[q] >>= 1;
        st.b[q] = if st.b[q] >= 0 { st.b[q] >> 1 } else { -((1 - st.b[q]) >> 1) };
        st.n[q] >>= 1;
    }
    st.n[q] += 1;
    if st.b[q] <= -st.n[q] {
        if st.c[q] > MIN_C {
            st.c[q] -= 1;
        }
        st.b[q] += st.n[q];
        if st.b[q] <= -st.n[q] {
            st.b[q] = -st.n[q] + 1;
        }
    } else if st.b[q] > 0 {
        if st.c[q] < MAX_C {
            st.c[q] += 1;
        }
        st.b[q] -= st.n[q];
        if st.b[q] > 0 {
            st.b[q] = 0;
        }
    }
}

/// Run-interruption variables update shared by both directions.
fn update_run(st: &mut State, q: usize, e: i32, em: i32, ri_type: i32, p: &Params) {
    let i = q - RI_BASE;
    if e < 0 {
        st.nn[i] += 1;
    }
    st.a[q] += (em + 1 - ri_type) >> 1;
    if st.n[q] == p.reset {
        st.a[q] >>= 1;
        st.n[q] >>= 1;
        st.nn[i] >>= 1;
    }
    st.n[q] += 1;
}

/// The run-interruption Golomb parameter and the mapping flag of an error.
fn run_k(st: &State, q: usize, ri_type: i32) -> u32 {
    let temp = if ri_type == 1 { st.a[q] + (st.n[q] >> 1) } else { st.a[q] };
    golomb_k(st.n[q], temp)
}

#[allow(clippy::too_many_arguments)]
fn decode_run(
    r: &mut BitReader,
    st: &mut State,
    p: &Params,
    prev: &[i32],
    cur: &mut [i32],
    ra: i32,
    x: usize,
    w: usize,
) -> Result<usize, CodecError> {
    let remaining = w - x;
    let mut run = 0usize;
    while r.bit()? == 1 {
        let step = 1usize << J[st.run_index];
        let count = step.min(remaining - run);
        run += count;
        if count == step && st.run_index < 31 {
            st.run_index += 1;
        }
        if run == remaining {
            break;
        }
    }
    if run < remaining {
        let rest = r.bits(J[st.run_index])? as usize;
        run += rest;
        if run > remaining {
            return Err(bad("a run passes the end of the line"));
        }
    }
    for v in &mut cur[x + 1..x + 1 + run] {
        *v = ra;
    }
    if run == remaining {
        return Ok(w);
    }
    // The sample that ended the run: its own Rb, the sample above it.
    let rb = prev[x + run + 1];
    let ri_type = i32::from((ra - rb).abs() <= p.near);
    let q = RI_BASE + ri_type as usize;
    let k = run_k(st, q, ri_type);
    let limit = p.limit - J[st.run_index] - 1;
    let em = r.golomb(k, limit, p.qbpp)?;
    let tmp = em + ri_type;
    let cond = k == 0 && 2 * st.nn[ri_type as usize] < st.n[q];
    let map = tmp & 1;
    let mag = (tmp + map) / 2;
    let e = if (cond && map == 1) || (!cond && map == 0) { mag } else { -mag };
    let (px, sign) = if ri_type == 1 { (ra, 1) } else { (rb, if ra > rb { -1 } else { 1 }) };
    // The decoder holds the sign-adjusted error; the sign flips the reconstruction.
    let rx = reconstruct(px, sign, e, p);
    update_run(st, q, e, em, ri_type, p);
    if st.run_index > 0 {
        st.run_index -= 1;
    }
    cur[x + run + 1] = rx;
    Ok(x + run + 1)
}

// ---- encoder ----

struct BitWriter {
    out: Vec<u8>,
    cur: u32,
    used: u32,
    cap: u32,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter { out: Vec::new(), cur: 0, used: 0, cap: 8 }
    }

    fn put(&mut self, value: u32, n: u32) {
        for i in (0..n).rev() {
            // A run of more than 32 zero bits (a long escape code) has no bits of `value` to shift out.
            let bit = if i < 32 { (value >> i) & 1 } else { 0 };
            self.cur = self.cur << 1 | bit;
            self.used += 1;
            if self.used == self.cap {
                let byte = self.cur as u8;
                self.out.push(byte);
                self.cap = if byte == 0xFF { 7 } else { 8 };
                self.cur = 0;
                self.used = 0;
            }
        }
    }

    /// Pad the last byte with zero bits; a closing 0xFF is followed by a zero byte.
    fn finish(mut self) -> Vec<u8> {
        if self.used > 0 {
            let pad = self.cap - self.used;
            self.put(0, pad);
        }
        if self.out.last() == Some(&0xFF) {
            self.out.push(0);
        }
        self.out
    }

    fn golomb(&mut self, m: i32, k: u32, limit: u32, qbpp: u32) {
        let m = m as u32;
        let max_zeros = limit - qbpp - 1;
        let q = m >> k;
        if q < max_zeros {
            self.put(0, q);
            self.put(1, 1);
            self.put(m & ((1 << k) - 1), k);
        } else {
            self.put(0, max_zeros);
            self.put(1, 1);
            self.put(m - 1, qbpp);
        }
    }
}

fn encode_scan(plane: &[u16], w: usize, h: usize, p: &Params) -> Vec<u8> {
    let mut wr = BitWriter::new();
    let mut st = State::new(p);
    let mut prev = vec![0i32; w + 2];
    let mut cur = vec![0i32; w + 2];
    for y in 0..h {
        cur[0] = prev[1];
        if y > 0 {
            prev[w + 1] = prev[w];
        }
        let line = &plane[y * w..(y + 1) * w];
        let mut x = 0usize;
        while x < w {
            let (ra, rb, rc, rd) = (cur[x], prev[x + 1], prev[x], if y == 0 { 0 } else { prev[x + 2] });
            let ix = i32::from(line[x]);
            let (q1, q2, q3) = (quantise_gradient(rd - rb, p), quantise_gradient(rb - rc, p), quantise_gradient(rc - ra, p));
            if q1 == 0 && q2 == 0 && q3 == 0 {
                x = encode_run(&mut wr, &mut st, p, line, &prev, &mut cur, ra, x);
                continue;
            }
            let (sign, q1, q2, q3) = if q1 < 0 || (q1 == 0 && (q2 < 0 || (q2 == 0 && q3 < 0))) { (-1, -q1, -q2, -q3) } else { (1, q1, q2, q3) };
            let q = (q1 * 81 + (q2 + 4) * 9 + (q3 + 4)) as usize;
            let px = (predict(ra, rb, rc) + sign * st.c[q]).clamp(0, p.maxval);
            let e = reduce(sign * (ix - px), p);
            let k = golomb_k(st.n[q], st.a[q]);
            let swap = p.near == 0 && k == 0 && 2 * st.b[q] <= -st.n[q];
            let m = if swap {
                if e >= 0 { 2 * e + 1 } else { -2 * (e + 1) }
            } else if e >= 0 {
                2 * e
            } else {
                -2 * e - 1
            };
            wr.golomb(m, k, p.limit, p.qbpp);
            cur[x + 1] = reconstruct(px, sign, e, p);
            update_regular(&mut st, q, e, p);
            x += 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    wr.finish()
}

#[allow(clippy::too_many_arguments)]
fn encode_run(wr: &mut BitWriter, st: &mut State, p: &Params, line: &[u16], prev: &[i32], cur: &mut [i32], ra: i32, x: usize) -> usize {
    let w = line.len();
    let mut run = 0usize;
    while x + run < w && (i32::from(line[x + run]) - ra).abs() <= p.near {
        cur[x + run + 1] = ra;
        run += 1;
    }
    let mut count = run;
    while count >= 1usize << J[st.run_index] {
        wr.put(1, 1);
        count -= 1usize << J[st.run_index];
        if st.run_index < 31 {
            st.run_index += 1;
        }
    }
    if x + run == w {
        if count > 0 {
            wr.put(1, 1);
        }
        return w;
    }
    wr.put(0, 1);
    wr.put(count as u32, J[st.run_index]);
    let ix = i32::from(line[x + run]);
    let rb = prev[x + run + 1];
    let ri_type = i32::from((ra - rb).abs() <= p.near);
    let q = RI_BASE + ri_type as usize;
    let (px, sign) = if ri_type == 1 { (ra, 1) } else { (rb, if ra > rb { -1 } else { 1 }) };
    let e = reduce(sign * (ix - px), p);
    let k = run_k(st, q, ri_type);
    let n_neg = st.nn[ri_type as usize];
    let map = i32::from((k == 0 && e > 0 && 2 * n_neg < st.n[q]) || (e < 0 && 2 * n_neg >= st.n[q]) || (e < 0 && k != 0));
    let em = 2 * e.abs() - ri_type - map;
    wr.golomb(em, k, p.limit - J[st.run_index] - 1, p.qbpp);
    cur[x + run + 1] = reconstruct(px, sign, e, p);
    update_run(st, q, e, em, ri_type, p);
    if st.run_index > 0 {
        st.run_index -= 1;
    }
    x + run + 1
}

/// Writes a lossless JPEG-LS file: 8 or 16 bits, gray or RGB, one scan per component.
pub(crate) fn encode(src: &Image, plan: Plan) -> Result<Vec<u8>, CodecError> {
    let enc = |msg: String| CodecError::encode(F, msg);
    let img = src.converted(plan.layout, plan.sample);
    let nc = match img.layout() {
        ChannelLayout::Gray => 1,
        ChannelLayout::Rgb => 3,
        l => return Err(enc(format!("no JPEG-LS layout for {l:?}"))),
    };
    let precision: u32 = match img.sample_type() {
        SampleType::U8 => 8,
        SampleType::U16 => 16,
        s => return Err(enc(format!("JPEG-LS stores 8 or 16 bits per sample, not {s:?}"))),
    };
    let (w, h) = img.dimensions();
    let (Ok(w16), Ok(h16)) = (u16::try_from(w), u16::try_from(h)) else {
        return Err(enc(format!("dimensions exceed 65535x65535: {w}x{h}")));
    };
    let (wu, hu) = (w as usize, h as usize);
    let data = img.data();
    if data.len() != wu * hu * nc * (precision as usize / 8) {
        return Err(enc("the image has the wrong number of samples".into()));
    }
    let params = Params::new(precision, 0, [0; 5]).map_err(|_| enc("invalid coding parameters".into()))?;
    let mut out = vec![0xFF, 0xD8, 0xFF, SOF55];
    out.extend_from_slice(&((8 + 3 * nc) as u16).to_be_bytes());
    out.push(precision as u8);
    out.extend_from_slice(&h16.to_be_bytes());
    out.extend_from_slice(&w16.to_be_bytes());
    out.push(nc as u8);
    for c in 0..nc {
        out.extend_from_slice(&[c as u8 + 1, 0x11, 0]);
    }
    let mut plane = vec![0u16; wu * hu];
    for c in 0..nc {
        for (i, v) in plane.iter_mut().enumerate() {
            let at = i * nc + c;
            *v = if precision == 8 { u16::from(data[at]) } else { u16::from_ne_bytes([data[2 * at], data[2 * at + 1]]) };
        }
        out.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, c as u8 + 1, 0, 0, 0, 0]);
        out.extend_from_slice(&encode_scan(&plane, wu, hu, &params));
    }
    out.extend_from_slice(&[0xFF, 0xD9]);
    Ok(out)
}

struct Frame {
    precision: u32,
    width: usize,
    height: usize,
    components: Vec<u8>,
}

fn be16(b: &[u8], i: usize) -> Result<usize, CodecError> {
    b.get(i..i + 2).map(|s| usize::from(u16::from_be_bytes([s[0], s[1]]))).ok_or_else(|| bad("truncated header"))
}

pub(crate) fn decode(b: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    if !looks_like_jpegls(b) {
        return Err(bad("not a JPEG-LS file"));
    }
    let mut pos = 2;
    let mut frame: Option<Frame> = None;
    let mut preset = [0i32; 5];
    let mut planes: Vec<Vec<u16>> = Vec::new();
    let mut scans_done = 0;
    let mut restart_interval = 0usize;
    loop {
        // Find the next marker.
        while b.get(pos) != Some(&0xFF) {
            if pos >= b.len() {
                return finish(frame, planes, scans_done);
            }
            pos += 1;
        }
        while b.get(pos + 1) == Some(&0xFF) {
            pos += 1;
        }
        let Some(&marker) = b.get(pos + 1) else { return finish(frame, planes, scans_done) };
        if marker == 0xD9 {
            return finish(frame, planes, scans_done);
        }
        if marker == 0x00 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            pos += 2;
            continue;
        }
        let len = be16(b, pos + 2)?;
        let seg = b.get(pos + 4..pos + 2 + len).ok_or_else(|| bad("truncated marker segment"))?;
        match marker {
            SOF55 => {
                if frame.is_some() || seg.len() < 6 {
                    return Err(bad("bad or repeated frame header"));
                }
                let (precision, height, width, nf) = (u32::from(seg[0]), be16(seg, 1)?, be16(seg, 3)?, usize::from(seg[5]));
                if !(2..=16).contains(&precision) || width == 0 || height == 0 || !matches!(nf, 1 | 3) || seg.len() < 6 + 3 * nf {
                    return Err(bad("unsupported frame header"));
                }
                if height == 0 {
                    return Err(CodecError::unsupported(F, "a frame height set later by a DNL marker"));
                }
                let layout = if nf == 1 { ChannelLayout::Gray } else { ChannelLayout::Rgb };
                let sample = if precision <= 8 { SampleType::U8 } else { SampleType::U16 };
                limits.check(width as u32, height as u32, layout, sample)?;
                // Each scan retains u16 samples even when the final image is 8-bit.
                limits.check_bytes(width as u32, height as u32, 2)?;
                let components = (0..nf).map(|i| seg[6 + 3 * i]).collect();
                frame = Some(Frame { precision, width, height, components });
            }
            LSE => {
                if seg.first() == Some(&1) && seg.len() >= 11 {
                    for (i, p) in preset.iter_mut().enumerate() {
                        *p = be16(seg, 1 + 2 * i)? as i32;
                    }
                } else if seg.first() != Some(&1) {
                    return Err(CodecError::unsupported(F, "mapping tables and colour transforms (LSE)"));
                }
            }
            // DRI: a 2-, 3- or 4-byte interval in lines (T.87 C.2.5). An interval of 0 turns
            // restart markers off. The interval in effect at each scan header applies to that scan.
            0xDD => {
                if !(2..=4).contains(&seg.len()) {
                    return Err(bad("bad restart interval segment"));
                }
                restart_interval = seg.iter().fold(0, |ri, &v| (ri << 8) | usize::from(v));
            }
            0xDA => {
                let f = frame.as_ref().ok_or_else(|| bad("a scan before the frame header"))?;
                // A scan of one component has restart markers only when the interval is shorter
                // than the frame's height.
                if restart_interval != 0 && restart_interval < f.height {
                    return Err(CodecError::unsupported(F, "a restart interval"));
                }
                let ns = usize::from(*seg.first().ok_or_else(|| bad("empty scan header"))?);
                if seg.len() < 1 + 2 * ns + 3 || ns != 1 {
                    return Err(CodecError::unsupported(F, "interleaved scans (ILV 1 and 2)"));
                }
                let (cs, tm, near, ilv) = (seg[1], seg[2], i32::from(seg[3]), seg[4]);
                if tm != 0 {
                    return Err(CodecError::unsupported(F, "mapping tables"));
                }
                if ilv != 0 && f.components.len() > 1 {
                    return Err(CodecError::unsupported(F, "interleaved scans (ILV 1 and 2)"));
                }
                if seg[5] & 0x0f != 0 {
                    return Err(CodecError::unsupported(F, "a point transform"));
                }
                let idx = f.components.iter().position(|&c| c == cs).ok_or_else(|| bad("a scan names an unknown component"))?;
                if planes.len() <= idx {
                    planes.resize(f.components.len(), Vec::new());
                }
                if !planes[idx].is_empty() {
                    return Err(bad("a component is coded twice"));
                }
                let params = Params::new(f.precision, near, preset)?;
                let start = pos + 2 + len;
                // The scan ends at the next real marker: an 0xFF followed by a byte with its top bit set.
                let mut end = start;
                while end < b.len() {
                    if b[end] == 0xFF && b.get(end + 1).is_none_or(|&n| n & 0x80 != 0) {
                        break;
                    }
                    end += 1;
                }
                let mut samples = Vec::new();
                samples.try_reserve_exact(f.width * f.height).map_err(|_| bad("pixel buffer is too large"))?;
                samples.resize(f.width * f.height, 0u16);
                decode_scan(&b[start..end], f.width, f.height, &params, &mut samples)?;
                // A restart marker comes only between two intervals, so none follows a scan that
                // fits in one. Fill bytes (`0xFF`) may come before the marker.
                if restart_interval != 0 && b[end..].iter().find(|&&v| v != 0xFF).is_some_and(|m| (0xD0..=0xD7).contains(m)) {
                    return Err(bad("a restart marker after the last restart interval"));
                }
                planes[idx] = samples;
                scans_done += 1;
                pos = end;
                continue;
            }
            _ => {}
        }
        pos += 2 + len;
    }
}

fn finish(frame: Option<Frame>, planes: Vec<Vec<u16>>, scans: usize) -> Result<Image, CodecError> {
    let f = frame.ok_or_else(|| bad("no frame header"))?;
    if scans != f.components.len() || planes.iter().any(Vec::is_empty) || planes.len() != f.components.len() {
        return Err(bad("the file ends before every component is coded"));
    }
    let (w, h, nc) = (f.width, f.height, f.components.len());
    let layout = if nc == 1 { ChannelLayout::Gray } else { ChannelLayout::Rgb };
    let p = f.precision;
    let scale = |v: u16| -> u16 {
        if p == 16 {
            v
        } else {
            let v = u32::from(v);
            ((v << (16 - p)) | (v >> (2 * p).saturating_sub(16).min(15))) as u16
        }
    };
    if p <= 8 {
        let to8 = |v: u16| -> u8 { if p == 8 { v as u8 } else { (u32::from(v) * 255 / ((1u32 << p) - 1)) as u8 } };
        let mut data = Vec::with_capacity(w * h * nc);
        for i in 0..w * h {
            for plane in &planes {
                data.push(to8(plane[i]));
            }
        }
        Image::from_u8(w as u32, h as u32, layout, data)
    } else {
        let mut data = Vec::with_capacity(w * h * nc);
        for i in 0..w * h {
            for plane in &planes {
                data.push(scale(plane[i]));
            }
        }
        Image::from_u16(w as u32, h as u32, layout, &data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise_u16(n: usize, seed: &mut u32, mask: u32) -> Vec<u16> {
        (0..n)
            .map(|_| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 17;
                *seed ^= *seed << 5;
                (*seed & mask) as u16
            })
            .collect()
    }

    fn image(w: u32, h: u32, layout: ChannelLayout, wide: bool, kind: u8) -> Image {
        let n = w as usize * h as usize * layout.channels();
        let mut seed = 0x1234_5679u32;
        let samples: Vec<u16> = match kind {
            0 => noise_u16(n, &mut seed, if wide { 0xFFFF } else { 0xFF }),
            1 => vec![if wide { 40_000 } else { 100 }; n],
            _ => (0..n).map(|i| if wide { ((i / 3) * 37 % 65_536) as u16 } else { ((i / 3) % 251) as u16 }).collect(),
        };
        if wide {
            Image::from_u16(w, h, layout, &samples).unwrap()
        } else {
            Image::from_u8(w, h, layout, samples.iter().map(|&v| v as u8).collect()).unwrap()
        }
    }

    fn plan(img: &Image) -> Plan {
        Plan { layout: img.layout(), sample: img.sample_type() }
    }

    #[test]
    fn low_precision_samples_span_the_full_output_range() {
        for precision in 2..=7 {
            let max = (1u16 << precision) - 1;
            let f = Frame { precision, width: usize::from(max) + 1, height: 1, components: vec![1] };
            let image = finish(Some(f), vec![(0..=max).collect()], 1).unwrap();
            let expected: Vec<u8> = (0..=max).map(|v| (u32::from(v) * 255 / u32::from(max)) as u8).collect();
            assert_eq!(image.data(), expected);
        }
    }

    #[test]
    fn eight_bit_scan_intermediate_respects_allocation_limit() {
        let img = image(8, 8, ChannelLayout::Gray, false, 0);
        let bytes = encode(&img, plan(&img)).unwrap();
        assert!(matches!(decode(&bytes, &Limits::new(8, 8, 64, 127)), Err(CodecError::LimitExceeded(_))));
        assert!(decode(&bytes, &Limits::new(8, 8, 64, 128)).is_ok());
    }

    #[test]
    fn round_trips_noise_flat_and_smooth_images() {
        for layout in [ChannelLayout::Gray, ChannelLayout::Rgb] {
            for wide in [false, true] {
                for kind in 0..3 {
                    // Widths around the run-length steps, and single-pixel dimensions.
                    for (w, h) in [(1, 1), (1, 9), (2, 3), (7, 5), (33, 4), (300, 3), (1025, 2)] {
                        let img = image(w, h, layout, wide, kind);
                        let bytes = encode(&img, plan(&img)).unwrap();
                        assert!(looks_like_jpegls(&bytes));
                        let back = decode(&bytes, &Limits::default()).unwrap_or_else(|e| panic!("{layout:?} wide={wide} kind={kind} {w}x{h}: {e}"));
                        assert_eq!(back.data(), img.data(), "{layout:?} wide={wide} kind={kind} {w}x{h}");
                    }
                }
            }
        }
    }

    #[test]
    fn default_parameters_follow_the_standard() {
        let p = Params::new(8, 0, [0; 5]).unwrap();
        assert_eq!((p.maxval, p.range, p.qbpp, p.limit, p.t1, p.t2, p.t3, p.reset), (255, 256, 8, 32, 3, 7, 21, 64));
        let p = Params::new(16, 0, [0; 5]).unwrap();
        assert_eq!((p.maxval, p.range, p.qbpp, p.limit, p.t1, p.t2, p.t3), (65535, 65536, 16, 64, 18, 67, 276));
        let p = Params::new(8, 3, [0; 5]).unwrap();
        assert_eq!((p.range, p.t1), (38, 12)); // RANGE = floor((255 + 2 * 3) / 7) + 1
        assert!(Params::new(8, 200, [0; 5]).is_err());
    }

    #[test]
    fn detection_tells_jpeg_ls_from_jpeg() {
        let img = image(4, 4, ChannelLayout::Gray, false, 0);
        let ls = encode(&img, plan(&img)).unwrap();
        assert!(looks_like_jpegls(&ls));
        // A baseline JPEG starts SOI, APP0 / DQT, ... SOF0: not JPEG-LS.
        assert!(!looks_like_jpegls(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11]));
        assert!(!looks_like_jpegls(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 11]));
        // SOF55 after an application segment still is.
        assert!(looks_like_jpegls(&[0xFF, 0xD8, 0xFF, 0xE8, 0, 4, 0, 0, 0xFF, 0xF7, 0, 11]));
        assert!(!looks_like_jpegls(&[0xFF, 0xD8]));
    }

    #[test]
    fn a_resumed_walk_starts_at_the_marker_it_stopped_at() {
        // SOI, a comment with two bytes of text, then an SOF55 frame header.
        let file = [0xFF, 0xD8, 0xFF, 0xFE, 0, 4, 1, 2, 0xFF, 0xF7, 0, 11];
        let mut pos = 0;
        assert_eq!(jpeg_frame_from(&file[..5], &mut pos), Some(JpegFrame::Short(6)));
        assert_eq!(pos, 2);
        assert_eq!(jpeg_frame_from(&file[..9], &mut pos), Some(JpegFrame::Short(10)));
        assert_eq!(pos, 8);
        // The resumed walk does not read the bytes before `pos` again: with them cleared, it still
        // reaches the frame header, while a walk from the start finds no SOI.
        let mut cleared = file;
        cleared[..8].fill(0);
        assert_eq!(jpeg_frame_from(&cleared, &mut pos), Some(JpegFrame::Ls));
        assert_eq!(pos, 8);
        assert_eq!(jpeg_frame(&cleared), None);
        assert_eq!(jpeg_frame(&file), Some(JpegFrame::Ls));
    }

    #[test]
    fn a_walk_that_found_another_frame_header_stays_at_it() {
        // SOI, a comment with two bytes of text, then a baseline frame header.
        let file = [0xFF, 0xD8, 0xFF, 0xFE, 0, 4, 1, 2, 0xFF, 0xC0, 0, 11];
        let mut pos = 0;
        assert_eq!(jpeg_frame_from(&file, &mut pos), Some(JpegFrame::Other));
        assert_eq!(pos, 8);
        assert_eq!(jpeg_frame_from(&file, &mut pos), Some(JpegFrame::Other));
        assert_eq!(pos, 8);
    }

    #[test]
    fn interleaved_scans_and_tables_are_unsupported() {
        let img = image(8, 4, ChannelLayout::Rgb, false, 2);
        let good = encode(&img, plan(&img)).unwrap();
        // The scan header body is Ns, Cs, Tm, NEAR, ILV, point transform, after the marker and length.
        let sos = good.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap() + 4;
        let mut line = good.clone();
        line[sos + 4] = 1;
        assert!(matches!(decode(&line, &Limits::default()), Err(CodecError::Unsupported { .. })));
        let mut mapped = good.clone();
        mapped[sos + 2] = 1;
        assert!(matches!(decode(&mapped, &Limits::default()), Err(CodecError::Unsupported { .. })));
        let mut transform = good;
        transform[sos + 5] = 1;
        assert!(matches!(decode(&transform, &Limits::default()), Err(CodecError::Unsupported { .. })));
    }

    #[test]
    fn hostile_input_is_an_error_not_a_panic() {
        for (w, h, layout, wide) in [(23, 9, ChannelLayout::Gray, false), (11, 7, ChannelLayout::Rgb, true)] {
            let img = image(w, h, layout, wide, 0);
            let good = encode(&img, plan(&img)).unwrap();
            for len in 0..good.len() {
                let _ = decode(&good[..len], &Limits::default());
            }
            // Flip every bit of the entropy-coded data and of the headers, one at a time.
            for i in 0..good.len() {
                for bit in 0..8 {
                    let mut c = good.clone();
                    c[i] ^= 1 << bit;
                    let _ = decode(&c, &Limits::default());
                }
            }
        }
        // A huge frame is stopped by the limits or the length check before any allocation.
        let img = image(4, 4, ChannelLayout::Gray, false, 0);
        let mut huge = encode(&img, plan(&img)).unwrap();
        let sof = huge.windows(2).position(|w| w == [0xFF, 0xF7]).unwrap();
        huge[sof + 5..sof + 9].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(decode(&huge, &Limits::default()).is_err());
    }
}
