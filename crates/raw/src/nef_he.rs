//! Nikon High Efficiency NEF (HE / HE★) sensor data.
//!
//! The strip is one wavelet codestream in JPEG XS framing (ISO/IEC 21122-1 markers SOC, CAP, PIH, CDT, WGT, SLH, EOC)
//! with Nikon's own colour transform. Everything here was measured on CC0 files (raw.pixls.us Z f HE, DSC_0043,
//! against the same body's Lossless compressed DSC_0040) and on HE★ files from the same body: no other
//! implementation was read. What the files show:
//!
//! - PIH: four components of half the sensor size (one value per 2×2 cell), code groups of 4, significance groups
//!   of 8, 5 horizontal and 1 vertical 5/3 wavelet levels, no JPEG XS colour transform, uniform quantization, signs
//!   in their own subpacket. CDT gives each component its levels: the third component has none.
//! - Precincts of two component lines, each with a 12-byte header (24-bit length, Q, R, a 2-bit mode per band) and
//!   eight packets (line 0 of components 0 to 3, then line 1), then zero fill. A packet has a 7-byte header (raw
//!   flag, data, count and sign lengths of 20, 20 and 15 bits), significance flags, bitplane counts, data, signs.
//! - Bitplane counts are unary. A slice's first precinct codes them as is; later lines predict them from the line
//!   above (see [`residual`]). WGT lists one gain and priority per band in packet order; a band's truncation is
//!   `Q - gain - (priority < R)`.
//! - The components are the Star-Tetrix lifting transform (Richter et al., IEEE TIP 30, 2021) of
//!   `24.24 * sqrt(sample - black)` over the mosaic (see [`mosaic`]); the scale and the luma offset are measured on
//!   the CC0 Z 9 Lossless / HE★ pair.
//!
//! Open: the meaning of the 13 bytes PIH carries past its usual end, the exact luma offset, and other bodies.
//! Variants that differ from the measured layout are refused as [`HIGH_EFFICIENCY`](crate::nef) so their
//! embedded preview is used as before.

use crate::{Limits, RawError, Result};

const UNSUPPORTED: &str = crate::nef::HIGH_EFFICIENCY;
/// Scale and luma offset of the square-root domain, measured on the CC0 Z 9 Lossless / HE★ pair (same exposure;
/// patches registered one by one): within 1.5 % of the lossless levels from 25 above black to white.
const SCALE: f64 = 24.24;
const LUMA_OFFSET: f64 = -1032.7;
const COMPONENTS: usize = 4;
const BANDS: usize = 25;
/// Most bitplanes a code group may hold; more means the stream is not what we measured.
const MAX_PLANES: u32 = 30;
/// Largest coefficient magnitude: the measured streams use 18-bit coefficients (PIH `Bw`). Holding damaged
/// ones to this keeps the lifting sums far inside `i32`.
const MAX_COEFFICIENT: i64 = 1 << 20;
/// Largest truncation a band can have with 18-bit coefficients; a larger one comes from a damaged precinct header.
const MAX_TRUNCATION: u32 = 20;

/// A variant that differs from the measured one: the same reason as before this decoder existed, so the
/// embedded preview is used. `_what` names the difference for a reader of the code.
fn unsupported(_what: &str) -> RawError {
    RawError::Unsupported(UNSUPPORTED.into())
}
fn corrupt(what: &str) -> RawError {
    RawError::Corrupt(format!("Nikon High Efficiency NEF: {what}"))
}

struct Bits<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Bits<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    fn bit(&mut self) -> u32 {
        let v = self.b.get(self.p >> 3).map_or(0, |byte| (byte >> (7 - (self.p & 7))) & 1);
        self.p += 1;
        u32::from(v)
    }
    fn unary(&mut self) -> Result<u32> {
        let mut n = 0;
        while self.bit() == 1 {
            n += 1;
            if n > MAX_PLANES {
                return Err(corrupt("bitplane count out of range"));
            }
        }
        Ok(n)
    }
    fn bytes_used(&self) -> usize {
        self.p.div_ceil(8)
    }
}

fn be(b: &[u8]) -> u64 {
    b.iter().fold(0, |a, &x| (a << 8) | u64::from(x))
}

fn slice(src: &[u8], at: usize, len: usize) -> Result<&[u8]> {
    src.get(at..at.checked_add(len).ok_or_else(|| corrupt("length overflow"))?).ok_or_else(|| corrupt("truncated codestream"))
}

struct Header {
    width: usize,
    height: usize,
    /// (gain, priority) per band, in packet order.
    weights: Vec<(u8, u8)>,
    /// Offset of the first slice header.
    body: usize,
}

fn header(src: &[u8]) -> Result<Header> {
    if src.get(..4) != Some(&[0xff, 0x10, 0xff, 0x50][..]) {
        return Err(unsupported("no JPEG XS codestream"));
    }
    let (mut p, mut dims, mut levels_ok, mut weights) = (2usize, None, false, None);
    loop {
        let marker = be(slice(src, p, 2)?);
        if marker == 0xff20 {
            break;
        }
        if marker >> 8 != 0xff {
            return Err(unsupported("not a marker"));
        }
        let len = be(slice(src, p + 2, 2)?) as usize;
        let body = slice(src, p + 4, len.checked_sub(2).ok_or_else(|| corrupt("marker length"))?)?;
        match marker {
            0xff12 => dims = Some(picture_header(body)?),
            0xff13 => levels_ok = body == [0x0e, 0x11, 0x51, 0x0e, 0x11, 0x51, 0x0e, 0x11, 0x00, 0x0e, 0x11, 0x51],
            0xff14 if body.len() == 2 * BANDS => weights = Some(body.chunks(2).map(|c| (c[0], c[1])).collect()),
            _ => {}
        }
        p += 2 + len;
    }
    let (Some((width, height)), true, Some(weights)) = (dims, levels_ok, weights) else {
        return Err(unsupported("component layout differs from the measured one"));
    };
    Ok(Header { width, height, weights, body: p })
}

/// Picture header: the sizes, and the coding choices this decoder implements.
fn picture_header(b: &[u8]) -> Result<(usize, usize)> {
    let b = b.get(..24).ok_or_else(|| corrupt("short picture header"))?;
    // Nc, Ng, Ss, (Cpih, Nlx/Nly), (Qpih, Fs, Rm)
    let layout = (b[16], b[17], b[18], b[21] & 0x0f, b[22], b[23] & 0x3f);
    if layout != (4, 4, 8, 0, 0x51, 0x14) {
        return Err(unsupported("coding options differ from the measured ones"));
    }
    Ok((be(&b[8..10]) as usize, be(&b[10..12]) as usize))
}

/// Band widths of one component line: line 0 is L5 H5 H4 H3 H2 HL1, line 1 LH1 HH1.
fn band_widths(cw: usize) -> (Vec<usize>, Vec<usize>) {
    let (l1, h1) = (cw.div_ceil(2), cw / 2);
    let mut lo = l1;
    let mut highs = Vec::with_capacity(4);
    for _ in 0..4 {
        highs.push(lo / 2);
        lo = lo.div_ceil(2);
    }
    let mut line0 = vec![lo];
    line0.extend(highs.iter().rev());
    line0.push(h1);
    (line0, vec![l1, h1])
}

/// One packet's layout: the component and line it holds, its bands, and its first WGT entry.
struct Packet {
    comp: usize,
    line: usize,
    bands: Vec<usize>,
    weight: usize,
}

fn packets(cw: usize) -> Vec<Packet> {
    let (l0, l1) = band_widths(cw);
    let p = |comp, line, bands: &Vec<usize>, weight| Packet { comp, line, bands: bands.clone(), weight };
    let c2 = vec![cw];
    vec![p(0, 0, &l0, 0), p(1, 0, &l0, 6), p(2, 0, &c2, 12), p(3, 0, &l0, 13), p(0, 1, &l1, 19), p(1, 1, &l1, 21), p(2, 1, &c2, 12), p(3, 1, &l1, 23)]
}

/// Signed residual of a predicted bitplane count: 0, -1, +1, -2, +2 ... while both signs fit above the
/// truncation, then only upward.
fn residual(u: u32, theta: u32) -> i64 {
    let (u, theta) = (i64::from(u), i64::from(theta));
    if u > 2 * theta {
        u - theta
    } else if u % 2 == 1 {
        -(u + 1) / 2
    } else {
        u / 2
    }
}

/// Uniform dequantization: the coded magnitude's bit pattern repeated below the truncation. `truncation` is at
/// most [`MAX_TRUNCATION`] and `planes` at most [`MAX_PLANES`] (both checked where they are read).
fn dequantize(magnitude: u32, planes: u32, truncation: u32) -> i32 {
    let base = (u64::from(magnitude) << truncation.min(MAX_TRUNCATION)).min(MAX_COEFFICIENT as u64);
    let (mut acc, mut s) = (base, base);
    loop {
        s >>= (planes + 1).min(63);
        if s == 0 {
            break;
        }
        acc += s;
    }
    i32::try_from(acc.min(MAX_COEFFICIENT as u64)).unwrap_or(0)
}

struct Subpackets<'a> {
    sig: Bits<'a>,
    cnt: Bits<'a>,
    dat: Bits<'a>,
    sgn: Bits<'a>,
    lens: [usize; 3],
}

impl<'a> Subpackets<'a> {
    /// Splits the packet at `at`; returns it and the offset after it.
    fn read(src: &'a [u8], at: usize, sig_len: usize) -> Result<(Self, usize)> {
        let h = be(slice(src, at, 7)?);
        if h >> 55 != 0 {
            return Err(unsupported("raw-mode packet"));
        }
        let (dat, cnt, sgn) = (((h >> 35) & 0xf_ffff) as usize, ((h >> 15) & 0xf_ffff) as usize, (h & 0x7fff) as usize);
        let mut o = at + 7;
        let mut take = |n: usize| -> Result<&'a [u8]> {
            let s = slice(src, o, n)?;
            o += n;
            Ok(s)
        };
        let (s, c, d, g) = (take(sig_len)?, take(cnt)?, take(dat)?, take(sgn)?);
        Ok((Self { sig: Bits::new(s), cnt: Bits::new(c), dat: Bits::new(d), sgn: Bits::new(g), lens: [cnt, dat, sgn] }, o))
    }
    fn consumed_exactly(&self) -> bool {
        [self.cnt.bytes_used(), self.dat.bytes_used(), self.sgn.bytes_used()] == self.lens
    }
}

/// Entropy decoding state across precincts.
struct Coder {
    packets: Vec<Packet>,
    weights: Vec<(u8, u8)>,
    /// Absolute bitplane count of every code group of each packet's previous line.
    above: Vec<Vec<i32>>,
}

/// What a precinct header says.
struct Precinct {
    q: i32,
    r: i32,
    first_in_slice: bool,
}

impl Coder {
    fn truncation(&self, weight: usize, pr: &Precinct) -> Result<i32> {
        let &(gain, priority) = self.weights.get(weight).ok_or_else(|| corrupt("band weight"))?;
        let t = (pr.q - i32::from(gain) - i32::from(i32::from(priority) < pr.r)).max(0);
        if t > MAX_TRUNCATION as i32 {
            return Err(corrupt("truncation out of range"));
        }
        Ok(t)
    }

    /// Decodes packet `k` into `out` (one line of the packet's bands, concatenated).
    fn packet(&mut self, k: usize, sub: &mut Subpackets, pr: &Precinct, out: &mut [i32]) -> Result<()> {
        // a slice's first line has nothing above it, except the third component's second line, which has its first
        let predict = !pr.first_in_slice || k == 6;
        let src = match k {
            2 => 6,
            6 => 2,
            _ => k,
        };
        // packet 2 reads the previous precinct's packet 6, packet 6 this precinct's packet 2
        let above = self.above.get(src).cloned().unwrap_or_default();
        let mut current = Vec::with_capacity(above.len());
        let (bands, weight) = self.packets.get(k).map(|p| (p.bands.clone(), p.weight)).ok_or_else(|| corrupt("packet"))?;
        let mut x = 0;
        for (bi, &width) in bands.iter().enumerate() {
            let t = self.truncation(if k == 6 { weight } else { weight + bi }, pr)?;
            let groups = width.div_ceil(4);
            let flags: Vec<bool> = (0..groups.div_ceil(8)).map(|_| sub.sig.bit() == 1).collect();
            for j in 0..groups {
                let top = above.get(current.len()).copied().unwrap_or(0);
                let theta = u32::try_from((top - t).max(0)).unwrap_or(0);
                let skipped = flags.get(j / 8).copied().unwrap_or(false);
                let planes = match (predict, skipped) {
                    (true, true) => i64::from(theta),
                    (true, false) => i64::from(theta) + residual(sub.cnt.unary()?, theta),
                    (false, true) => 0,
                    (false, false) => i64::from(sub.cnt.unary()?),
                };
                let planes = u32::try_from(planes.clamp(0, i64::from(MAX_PLANES))).unwrap_or(0);
                current.push(t + planes as i32);
                let n = (width - j * 4).min(4);
                let dst = out.get_mut(x..x + n).ok_or_else(|| corrupt("band outside its line"))?;
                group(sub, planes, t as u32, dst);
                x += n;
            }
        }
        if !sub.consumed_exactly() {
            return Err(corrupt("packet lengths do not match its contents"));
        }
        if let Some(slot) = self.above.get_mut(k) {
            *slot = current;
        }
        Ok(())
    }
}

/// One code group: `planes` bitplanes of 4 magnitudes, then a sign for each one that is not zero.
fn group(sub: &mut Subpackets, planes: u32, truncation: u32, dst: &mut [i32]) {
    let mut mags = [0u32; 4];
    for _ in 0..planes {
        for m in &mut mags {
            *m = (*m << 1) | sub.dat.bit();
        }
    }
    for (d, &m) in dst.iter_mut().zip(&mags) {
        *d = if m == 0 {
            0
        } else {
            let v = dequantize(m, planes, truncation);
            if sub.sgn.bit() == 1 { -v } else { v }
        };
    }
}

/// Inverse reversible 5/3 lifting of `low` and `high` (symmetric extension) into `n` samples.
fn synthesize(low: &[i32], high: &[i32], n: usize) -> Vec<i32> {
    let (ne, no) = (n.div_ceil(2), n / 2);
    let h = |i: isize| -> i32 {
        let i = if i < 0 {
            -i - 1
        } else if i as usize >= no {
            2 * no as isize - 1 - i
        } else {
            i
        };
        usize::try_from(i).ok().and_then(|i| high.get(i)).copied().unwrap_or(0)
    };
    let even: Vec<i32> = (0..ne).map(|i| low.get(i).copied().unwrap_or(0) - ((h(i as isize - 1) + h(i as isize) + 2) >> 2)).collect();
    let mut x = vec![0i32; n];
    for (i, &e) in even.iter().enumerate() {
        if let Some(v) = x.get_mut(2 * i) {
            *v = e;
        }
    }
    for i in 0..no {
        let a = even.get(i).copied().unwrap_or(0);
        let b = even.get(i + 1).copied().unwrap_or(a);
        if let Some(v) = x.get_mut(2 * i + 1) {
            *v = high.get(i).copied().unwrap_or(0) + ((a + b) >> 1);
        }
    }
    x
}

/// Line 0 of a wavelet component: the four horizontal-only levels, then level 1.
fn synthesize_line0(row: &[i32], bands: &[usize], cw: usize) -> Vec<i32> {
    let mut start = bands.first().copied().unwrap_or(0);
    let mut low = row.get(..start).unwrap_or_default().to_vec();
    for &width in bands.iter().skip(1).take(4) {
        let high = row.get(start..start + width).unwrap_or_default();
        low = synthesize(&low, high, low.len() + width);
        start += width;
    }
    synthesize(&low, row.get(start..).unwrap_or_default(), cw)
}

/// Decodes `rows` rows of a `width` × `height` sensor; `black` is the maker note's black level.
pub(crate) fn decode(src: &[u8], width: usize, height: usize, rows: usize, black: f64, limits: &Limits) -> Result<Vec<u16>> {
    // a header unlike the measured one is the unsupported variant (its preview is used), not a damaged file
    let hd = header(src).map_err(|e| if matches!(e, RawError::Corrupt(_)) { unsupported("header") } else { e })?;
    if (hd.width, hd.height) != (width, height) || !width.is_multiple_of(4) || !height.is_multiple_of(4) || width == 0 || height == 0 {
        return Err(unsupported("codestream size differs from the image's"));
    }
    if width.saturating_mul(height) > limits.max_samples.min(crate::MAX_SAMPLES) {
        return Err(RawError::Limit("image too large"));
    }
    let (cw, ch) = (width / 2, height / 2);
    let precincts = ch / 2;
    // the vertical synthesis of a row reads one or two lines on either side
    let needed = (rows.min(height).div_ceil(4) + 2).min(precincts);
    let packets = packets(cw);
    let mut coder =
        Coder { above: packets.iter().map(|p| vec![0; p.bands.iter().map(|w| w.div_ceil(4)).sum()]).collect(), weights: hd.weights, packets };
    let sig_lens: Vec<usize> = coder.packets.iter().map(|p| p.bands.iter().map(|w| w.div_ceil(4).div_ceil(8)).sum::<usize>().div_ceil(8)).collect();
    // lines[packet][precinct * cw ..]: each packet's line in every precinct
    let mut lines: Vec<Vec<i32>> = (0..8).map(|_| vec![0i32; cw * needed]).collect();
    let (mut p, mut index, mut first_in_slice) = (hd.body, 0usize, false);
    while index < needed {
        match be(slice(src, p, 2)?) {
            0xff20 => {
                p += 6;
                first_in_slice = true;
                continue;
            }
            0xff11 => return Err(corrupt("codestream ends early")),
            _ => {}
        }
        let len = be(slice(src, p, 3)?) as usize;
        let head = slice(src, p, 12)?;
        if head.get(5..11).is_none_or(|m| m.iter().any(|&b| b != 0xff)) || head.get(11).is_none_or(|&b| b & 0xc0 != 0xc0) {
            return Err(unsupported("band coding modes differ from the measured ones"));
        }
        let pr = Precinct { q: i32::from(head[3]), r: i32::from(head[4]), first_in_slice };
        let mut o = p + 12;
        for (k, line) in lines.iter_mut().enumerate() {
            let (mut sub, next) = Subpackets::read(src, o, sig_lens.get(k).copied().unwrap_or(0))?;
            let out = line.get_mut(index * cw..(index + 1) * cw).ok_or_else(|| corrupt("line"))?;
            coder.packet(k, &mut sub, &pr, out)?;
            o = next;
        }
        if o > p + 12 + len {
            return Err(corrupt("packets overrun their precinct"));
        }
        first_in_slice = false;
        p += 12 + len;
        index += 1;
    }
    let planes = components(&coder.packets, &lines, cw, needed);
    Ok(mosaic(&planes, cw, rows.min(height), black))
}

/// The four component planes, `2 * precincts` lines each.
fn components(packets: &[Packet], lines: &[Vec<i32>], cw: usize, precincts: usize) -> Vec<Vec<i32>> {
    let ch = 2 * precincts;
    let mut planes = vec![vec![0i32; cw * ch]; COMPONENTS];
    let line_of = |comp: usize, line: usize| packets.iter().position(|p| p.comp == comp && p.line == line);
    for (comp, plane) in planes.iter_mut().enumerate() {
        let (Some(k0), Some(k1)) = (line_of(comp, 0), line_of(comp, 1)) else { continue };
        let (Some(l0), Some(l1)) = (lines.get(k0), lines.get(k1)) else { continue };
        if comp == 2 {
            for i in 0..precincts {
                for (l, src) in [l0, l1].into_iter().enumerate() {
                    if let (Some(d), Some(s)) = (plane.get_mut((2 * i + l) * cw..(2 * i + l + 1) * cw), src.get(i * cw..(i + 1) * cw)) {
                        d.copy_from_slice(s);
                    }
                }
            }
            continue;
        }
        let bands0 = packets.get(k0).map(|p| p.bands.clone()).unwrap_or_default();
        let half = cw.div_ceil(2);
        let (mut low, mut high) = (vec![0i32; cw * precincts], vec![0i32; cw * precincts]);
        for i in 0..precincts {
            let r0 = l0.get(i * cw..(i + 1) * cw).unwrap_or_default();
            let r1 = l1.get(i * cw..(i + 1) * cw).unwrap_or_default();
            let a = synthesize_line0(r0, &bands0, cw);
            let b = synthesize(r1.get(..half).unwrap_or_default(), r1.get(half..).unwrap_or_default(), cw);
            if let (Some(lo), Some(hi)) = (low.get_mut(i * cw..(i + 1) * cw), high.get_mut(i * cw..(i + 1) * cw)) {
                lo.copy_from_slice(&a);
                hi.copy_from_slice(&b);
            }
        }
        for x in 0..cw {
            let l: Vec<i32> = (0..precincts).map(|i| low.get(i * cw + x).copied().unwrap_or(0)).collect();
            let h: Vec<i32> = (0..precincts).map(|i| high.get(i * cw + x).copied().unwrap_or(0)).collect();
            for (y, v) in synthesize(&l, &h, ch).into_iter().enumerate() {
                if let Some(d) = plane.get_mut(y * cw + x) {
                    *d = v;
                }
            }
        }
    }
    planes
}

/// Components to RGGB samples: undo the colour transform, then the square-root coding.
///
/// The transform is the Star-Tetrix lifting scheme of Richter et al., "Bayer CFA Pattern Compression With JPEG XS",
/// IEEE TIP 30 (2021), section VI, on the mosaic (cell `(y, x)` holds R, G1 above-right, G2 below-left, B): R and B
/// less the mean of their four adjacent greens (second and fourth components); each green plus an eighth of its two
/// R and two B differences, a luma; the third component is the G2 luma less the mean of its four diagonal G1 lumas,
/// and the first the G1 luma plus an eighth of its four diagonal differences. The chroma exponents are 0 (an
/// eighth each): on the Z 9 pair, 1 or 2 move the levels by 8 % or more.
fn mosaic(planes: &[Vec<i32>], cw: usize, rows: usize, black: f64) -> Vec<u16> {
    let width = 2 * cw;
    let cells_h = planes.first().map_or(0, |p| p.len() / cw.max(1));
    let mut out = vec![0u16; width * rows];
    if cw == 0 || cells_h == 0 {
        return out;
    }
    // a plane's value at a cell, the edge repeated outside the plane
    let at = |c: usize, y: isize, x: isize| -> f64 {
        let y = y.clamp(0, cells_h as isize - 1) as usize;
        let x = x.clamp(0, cw as isize - 1) as usize;
        planes.get(c).and_then(|p| p.get(y * cw + x)).copied().unwrap_or(0) as f64
    };
    let (luma, cr, delta, cb) = (0, 1, 2, 3);
    let rows_c = rows.div_ceil(2).min(cells_h);
    // the G1 luma, G1 and G2 once for every cell the rows read (one row past them), then the samples
    let needed = (rows_c + 1).min(cells_h);
    let cell = |i: usize| ((i / cw) as isize, (i % cw) as isize);
    let plane_at = |plane: &[f32], y: isize, x: isize| -> f64 {
        let y = y.clamp(0, needed as isize - 1) as usize;
        let x = x.clamp(0, cw as isize - 1) as usize;
        f64::from(plane.get(y * cw + x).copied().unwrap_or(0.0))
    };
    let luma1: Vec<f32> = (0..needed * cw)
        .map(|i| {
            let (y, x) = cell(i);
            let d = at(delta, y, x) + at(delta, y, x + 1) + at(delta, y - 1, x) + at(delta, y - 1, x + 1);
            (at(luma, y, x) - LUMA_OFFSET - d / 8.0) as f32
        })
        .collect();
    let g1_plane: Vec<f32> = (0..needed * cw)
        .map(|i| {
            let (y, x) = cell(i);
            let c = at(cr, y, x) + at(cr, y, x + 1) + at(cb, y - 1, x) + at(cb, y, x);
            (plane_at(&luma1, y, x) - c / 8.0) as f32
        })
        .collect();
    let g2_plane: Vec<f32> = (0..needed * cw)
        .map(|i| {
            let (y, x) = cell(i);
            let l = plane_at(&luma1, y, x) + plane_at(&luma1, y, x - 1) + plane_at(&luma1, y + 1, x) + plane_at(&luma1, y + 1, x - 1);
            let c = at(cr, y, x) + at(cr, y + 1, x) + at(cb, y, x - 1) + at(cb, y, x);
            (at(delta, y, x) + l / 4.0 - c / 8.0) as f32
        })
        .collect();
    let g1 = |y: isize, x: isize| plane_at(&g1_plane, y, x);
    let g2 = |y: isize, x: isize| plane_at(&g2_plane, y, x);
    let linear = |e: f64| {
        let s = (e / SCALE).max(0.0);
        (s * s + black).round().clamp(0.0, 16383.0) as u16
    };
    for y in 0..rows_c {
        for x in 0..cw {
            let (yi, xi) = (y as isize, x as isize);
            let (a, b) = (g1(yi, xi), g2(yi, xi));
            let r = at(cr, yi, xi) + (a + g1(yi, xi - 1) + b + g2(yi - 1, xi)) / 4.0;
            let bl = at(cb, yi, xi) + (a + g1(yi + 1, xi) + b + g2(yi, xi + 1)) / 4.0;
            for (dy, pair) in [[r, a], [b, bl]].iter().enumerate() {
                for (dx, &e) in pair.iter().enumerate() {
                    if let Some(v) = out.get_mut((2 * y + dy) * width + 2 * x + dx) {
                        *v = linear(e);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measured headers (Z f HE) for a `width` × `height` sensor, then `body`.
    fn codestream(width: u16, height: u16, body: &[u8]) -> Vec<u8> {
        let mut s = vec![0xff, 0x10, 0xff, 0x50, 0x00, 0x22];
        s.extend(b"CONTACT_INTOPIX_");
        s.extend([0xef, 0xc0]);
        s.extend([0; 14]);
        s.extend([0xff, 0x12, 0x00, 0x27, 0x00, 0x8c, 0x2e, 0x00, 0, 0, 0, 0]);
        s.extend(width.to_be_bytes());
        s.extend(height.to_be_bytes());
        s.extend([0, 0, 0x00, 0x10, 4, 4, 8, 0x12, 0x44, 0x10, 0x51, 0x14]);
        s.extend([0x50, 0x88, 0x70, 0x83, 0xf0, 0x15, 0x23, 0xd1, 0x49, 0xcd, 0x3f, 0x7f, 0x07]);
        s.extend([0xff, 0x13, 0x00, 0x0e, 0x0e, 0x11, 0x51, 0x0e, 0x11, 0x51, 0x0e, 0x11, 0x00, 0x0e, 0x11, 0x51]);
        s.extend([0xff, 0x14, 0x00, 0x34]);
        s.extend((0..BANDS as u8).flat_map(|b| [b % 4, b]));
        s.extend([0xff, 0x20, 0x00, 0x04, 0x00, 0x00]);
        s.extend(body);
        s
    }

    #[test]
    fn hostile_codestreams_return_errors() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let limits = Limits::default();
        for _ in 0..200 {
            let body: Vec<u8> = (0..(next() % 4096)).map(|_| next() as u8).collect();
            let s = codestream(256, 32, &body);
            let _ = decode(&s, 256, 32, 32, 1008.0, &limits);
            let cut = (next() as usize) % s.len();
            let _ = decode(&s[..cut], 256, 32, 32, 1008.0, &limits);
        }
        // a precinct header that claims every band and then runs out of data
        let mut body = vec![0x00, 0x00, 0x10, 4, 11];
        body.extend([0xff; 6]);
        body.push(0xc0);
        assert!(decode(&codestream(256, 32, &body), 256, 32, 32, 1008.0, &limits).is_err());
        // a size the codestream does not state is refused as the unsupported variant
        assert!(matches!(decode(&codestream(256, 32, &[]), 512, 32, 32, 1008.0, &limits), Err(RawError::Unsupported(m)) if m == UNSUPPORTED));
    }

    /// A damaged precinct header (Q = 255) made the truncation shift overflow, and a negative intermediate never
    /// shifted down to zero.
    #[test]
    fn dequantization_stays_bounded() {
        assert_eq!(dequantize(u32::MAX >> 2, MAX_PLANES, 255), MAX_COEFFICIENT as i32);
        assert!(dequantize(3, 1, 4) > 3 << 4);
        let mut body = vec![0x00, 0x00, 0x40, 255, 11];
        body.extend([0xff; 6]);
        body.push(0xc0);
        body.extend([0x00, 0x00, 0x00, 0x20, 0x00, 0x08, 0x00]);
        body.resize(64 + 12, 0xff);
        assert!(decode(&codestream(256, 32, &body), 256, 32, 32, 1008.0, &Limits::default()).is_err());
    }

    #[test]
    fn synthesis_restores_a_constant_line() {
        // a flat signal has no high-pass energy: every level must give the flat signal back
        let low = vec![100; 8];
        assert_eq!(synthesize(&low, &[0; 8], 16), vec![100; 16]);
        assert_eq!(synthesize(&low, &[0; 7], 15), vec![100; 15]);
    }
}
