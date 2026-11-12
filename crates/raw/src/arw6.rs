//! Sony ARW6 ("Compressed RAW 2" / HQ, TIFF Compression 32766): container parsing, geometry, colour and decode.
//!
//! Clean-room. The layout comes from SMPTE RDD 34 (Picture_info in word 0) and from Phase 0 black-box
//! inspection of real ILCE-7RM6 files (tile table, group size words, stream headers and index entries), each rule
//! checked by re-serialising parsed tiles byte-for-byte. The format is described in `docs/arw6-compression.md`; section
//! names in italics below refer to it. No other raw decoder's source was consulted.

use crate::arw6_curve::curve;
use crate::huffman::Bits;
use crate::llvc::{Bands3, Plane, band_rows, dequant, reconstruct3, vld_decode_line};
use crate::{RawError, Result};
use rayon::prelude::*;
use std::ops::Range;

fn corrupt(why: &str) -> RawError {
    RawError::Corrupt(format!("ARW6: {why}"))
}

fn unsupported(why: &str) -> RawError {
    RawError::Unsupported(format!("ARW6 {why}"))
}

fn u16_be(b: &[u8], at: usize) -> Option<usize> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(usize::from(u16::from_be_bytes([*s.first()?, *s.get(1)?])))
}

fn u24_be(b: &[u8], at: usize) -> Option<usize> {
    let s = b.get(at..at.checked_add(3)?)?;
    Some(s.iter().fold(0usize, |a, &v| (a << 8) | usize::from(v)))
}

fn u32_le(b: &[u8], at: usize) -> Option<usize> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(s.try_into().ok()?) as usize)
}

/// One tile of the strip: its bytes run from `offset` to the next tile's offset (the last: the strip end).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TileEntry {
    pub offset: usize,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// Most tiles a strip may hold (real files have 1 or 4).
const MAX_TILES: usize = 16;

/// Parse the tile table at the start of the strip (file structure, *Container*): `u32` count, `u32` 0, then
/// 24-byte entries `u64` offset (relative to the strip), `u32` x, y, width, height, all little-endian.
pub(crate) fn parse_tile_table(strip: &[u8], width: usize, height: usize) -> Result<Vec<TileEntry>> {
    let count = u32_le(strip, 0).ok_or_else(|| corrupt("truncated tile table"))?;
    if count == 0 || count > MAX_TILES {
        return Err(unsupported("tile count outside 1..=16"));
    }
    let mut tiles: Vec<TileEntry> = Vec::with_capacity(count);
    let mut area = 0usize;
    for i in 0..count {
        let at = 8 + 24 * i;
        let e = strip.get(at..at + 24).ok_or_else(|| corrupt("truncated tile table"))?;
        let offset = e.get(..8).and_then(|b| <[u8; 8]>::try_from(b).ok()).map(u64::from_le_bytes).and_then(|o| usize::try_from(o).ok());
        let offset = offset.filter(|&o| o < strip.len()).ok_or_else(|| corrupt("tile offset outside the strip"))?;
        if tiles.last().is_some_and(|p| offset <= p.offset) {
            return Err(corrupt("tile offsets do not increase"));
        }
        let (x, y, w, h) = (u32_le(e, 8), u32_le(e, 12), u32_le(e, 16), u32_le(e, 20));
        let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) else { return Err(corrupt("truncated tile table")) };
        // Even geometry; the level-3 band width w / 16 must be whole (w/2 a multiple of 8).
        if w == 0 || h == 0 || x % 2 != 0 || y % 2 != 0 || w % 2 != 0 || h % 2 != 0 || (w / 2) % 8 != 0 {
            return Err(unsupported("tile geometry"));
        }
        let fits = x.checked_add(w).is_some_and(|r| r <= width) && y.checked_add(h).is_some_and(|b| b <= height);
        if !fits {
            return Err(corrupt("tile outside the image"));
        }
        let a = w.checked_mul(h).filter(|&a| a <= crate::MAX_SAMPLES).ok_or(RawError::Limit("ARW6 tile too large"))?;
        area = area.checked_add(a).ok_or(RawError::Limit("ARW6 tiles too large"))?;
        tiles.push(TileEntry { offset, x, y, width: w, height: h });
    }
    for (i, a) in tiles.iter().enumerate() {
        for b in tiles.iter().skip(i + 1) {
            if a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height {
                return Err(corrupt("tiles overlap"));
            }
        }
    }
    // In-image, disjoint tiles cover the image exactly when their areas sum to it.
    if width.checked_mul(height) != Some(area) {
        return Err(corrupt("tiles do not cover the image"));
    }
    Ok(tiles)
}

/// Tile `i`'s bytes: from its offset to the next tile's offset (the last: the strip end).
pub(crate) fn tile_bytes<'a>(strip: &'a [u8], tiles: &[TileEntry], i: usize) -> Option<&'a [u8]> {
    let start = tiles.get(i)?.offset;
    let end = match tiles.get(i.checked_add(1)?) {
        Some(next) => next.offset,
        None => strip.len(),
    };
    strip.get(start..end)
}

/// One of a tile's 13 streams (positions in 16-byte words from the tile start).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StreamRef {
    pub group: u8,
    pub comp: u8,
    pub word: usize,
    pub size_words: usize,
    pub index_words: usize,
    pub data_words: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TileHeader {
    pub hs: usize,
    pub vs: usize,
    /// File order: g0 c0..2, g1 c0..2, g2 c0..2, g3 c0..2, g4 c0.
    pub streams: Vec<StreamRef>,
}

/// `n` bits at bit position `pos` (MSB-first) of a tile's word 0.
fn field(w: &[u8], pos: usize, n: usize) -> Option<u32> {
    let mut v = 0u32;
    for i in pos..pos.checked_add(n)? {
        let byte = *w.get(i / 8)?;
        v = (v << 1) | u32::from((byte >> (7 - i % 8)) & 1);
    }
    Some(v)
}

/// Streams per group: g0..g3 one per colour component, g4 the green residual.
const GROUP_STREAMS: [usize; 5] = [3, 3, 3, 3, 1];

/// Parse word 0 (RDD 34 `Picture_info`; every real file has HS = tile width, VS = tile height / 2, BBD 16, NC 3, NW 3,
/// NP 1000000000b), the group size words 3..=7 and each stream's header word (*Container*, Phase 0).
pub(crate) fn parse_tile_header(tile: &[u8], tw: usize, th: usize) -> Result<TileHeader> {
    let w0 = tile.get(..16).ok_or_else(|| corrupt("tile shorter than its picture header"))?;
    let f = |pos, n| field(w0, pos, n).map(|v| v as usize);
    let (hs, vs, bbd, nc, nw, np) = (f(64, 16), f(80, 16), f(102, 6), f(112, 3), f(115, 3), f(118, 10));
    let (Some(hs), Some(vs), Some(bbd), Some(nc), Some(nw), Some(np)) = (hs, vs, bbd, nc, nw, np) else {
        return Err(corrupt("picture header"));
    };
    if hs != tw || vs * 2 != th || bbd != 16 || nc != 3 || nw != 3 || np != 0b10_0000_0000 {
        return Err(unsupported(&format!("picture header variant (HS {hs}, VS {vs}, BBD {bbd}, NC {nc}, NW {nw}, NP {np:#b})")));
    }
    let mut streams = Vec::with_capacity(13);
    let mut word = 8usize;
    for (group, &n) in GROUP_STREAMS.iter().enumerate() {
        let table = tile.get((3 + group) * 16..(4 + group) * 16).ok_or_else(|| corrupt("truncated group table"))?;
        if usize::from(table.first().copied().unwrap_or(0)) != n {
            return Err(unsupported("group stream count"));
        }
        for comp in 0..n {
            let size_words = u24_be(table, 1 + 3 * comp).ok_or_else(|| corrupt("group table"))?;
            let end = word.checked_add(size_words).filter(|e| e.checked_mul(16).is_some_and(|b| b <= tile.len()));
            let end = end.ok_or_else(|| corrupt("stream outside the tile"))?;
            let hdr = tile.get(word * 16..word * 16 + 16).ok_or_else(|| corrupt("stream outside the tile"))?;
            let (index_words, data_words) =
                (u16_be(hdr, 0).ok_or_else(|| corrupt("stream header"))?, u24_be(hdr, 2).ok_or_else(|| corrupt("stream header"))?);
            if index_words.checked_add(data_words).and_then(|s| s.checked_add(1)).is_none_or(|s| s > size_words) {
                return Err(corrupt("stream header sizes exceed the stream"));
            }
            streams.push(StreamRef { group: group as u8, comp: comp as u8, word, size_words, index_words, data_words });
            word = end;
        }
    }
    Ok(TileHeader { hs, vs, streams })
}

/// One half-TU of a stream: `len` bytes at byte `start` of the tile, quantiser index per band
/// (g0/g4: the single nibble repeated).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Half {
    pub start: usize,
    pub len: usize,
    pub qi: [u32; 3],
}

fn nibbles(b: &[u8], from: usize, n: usize) -> Option<u32> {
    (from..from + n).try_fold(0u32, |a, i| Some((a << 4) | u32::from((*b.get(i / 2)? >> (4 * (1 - i % 2))) & 15)))
}

/// The `2 * ntu` halves of a stream. Index entry per TU (file structure): `A:16 X B:16 X` with A / B the byte
/// lengths of half 0 / 1 and X the QI nibble (g0, g4: 1 nibble) or three nibbles HL, LH, HH (g1..g3); the data area
/// holds each TU's A bytes then B bytes, consecutive TUs contiguous.
pub(crate) fn stream_halves(tile: &[u8], s: &StreamRef, ntu: usize) -> Result<Vec<Half>> {
    let nx = if s.group == 0 || s.group == 4 { 1 } else { 3 };
    let esz = 4 + nx; // bytes: (16 + 4nx + 16 + 4nx) / 8
    let idx_start = s.word.checked_add(1).and_then(|w| w.checked_mul(16)).ok_or_else(|| corrupt("stream position"))?;
    let idx_len = s.index_words.checked_mul(16).ok_or_else(|| corrupt("stream position"))?;
    if ntu.checked_mul(esz).is_none_or(|n| n > idx_len) {
        return Err(corrupt("index table too small for the tile's TUs"));
    }
    let data_start = idx_start.checked_add(idx_len).ok_or_else(|| corrupt("stream position"))?;
    let index = tile.get(idx_start..data_start).ok_or_else(|| corrupt("index table outside the tile"))?;
    let data_end = s.data_words.checked_mul(16).and_then(|d| d.checked_add(data_start)).ok_or_else(|| corrupt("stream position"))?;
    if data_end > tile.len() {
        return Err(corrupt("stream data outside the tile"));
    }
    let mut halves = Vec::with_capacity(2 * ntu);
    let mut pos = data_start;
    for k in 0..ntu {
        let e = index.get(k * esz..(k + 1) * esz).ok_or_else(|| corrupt("index table"))?;
        let bad = || corrupt("index entry");
        let (a, x1) = (nibbles(e, 0, 4).ok_or_else(bad)? as usize, nibbles(e, 4, nx).ok_or_else(bad)?);
        let (b, x2) = (nibbles(e, 4 + nx, 4).ok_or_else(bad)? as usize, nibbles(e, 8 + nx, nx).ok_or_else(bad)?);
        for (len, x) in [(a, x1), (b, x2)] {
            let end = pos.checked_add(len).filter(|&e| e <= data_end).ok_or_else(|| corrupt("half outside its stream's data"))?;
            let qi = if nx == 1 { [x; 3] } else { [(x >> 8) & 15, (x >> 4) & 15, x & 15] };
            halves.push(Half { start: pos, len, qi });
            pos = end;
        }
    }
    Ok(halves)
}

// ---- sensor-anchored frame geometry and tile reconstruction (*Frame geometry*) ----

/// Halves of `vs` plane rows are anchored to the sensor: half `n` covers plane rows `8n - 6 + s .. 8n + 1 + s`, so
/// `ceil((vs - 2 - s) / 8) + 1` halves are needed, two per TU (*Container*).
pub(crate) fn tu_count(vs: usize, s: u8) -> usize {
    let halves = vs.saturating_sub(2usize.saturating_add(usize::from(s))).div_ceil(8).saturating_add(1);
    halves.div_ceil(2)
}

/// Lifting phases of levels 1..=3 for anchor offset `s` (the Phase 0 reference model, local and not in the repo; see `docs/arw6-compression.md`):
/// FF (s = 0) gives [0, 1, 1], the APS-C crop (s = 3) [1, 0, 0].
pub(crate) fn phases(s: u8) -> [u8; 3] {
    let s = i32::from(s);
    let p1 = s % 2;
    let p2 = ((2 + s - p1) / 2) % 2;
    let k3 = ((6 + s) % 8 - p1).div_euclid(2);
    let p3 = (k3 - p2).div_euclid(2).rem_euclid(2);
    [p1 as u8, p2 as u8, p3 as u8]
}

/// Plane rows of half `n`: `8n - 6 + s .. 8n + 2 + s`, clipped to `0..rows` (empty when disjoint).
pub(crate) fn half_rows(n: usize, s: u8, rows: usize) -> Range<usize> {
    let base = n.saturating_mul(8).saturating_add(usize::from(s));
    base.saturating_sub(6).min(rows)..base.saturating_add(2).min(rows)
}

/// First half's frame rows `lines - 2`, read before the TU count is known from g4's first index entry alone
/// (index table at word `g4.word + 1`, entry 0's `A` = big-endian u16; its bytes start after the index table).
/// Residual lines are decoded while at least 8 bits remain and fewer than 8 lines were read (*Frame geometry*).
pub(crate) fn first_half_shift(tile: &[u8], g4: &StreamRef, width2: usize) -> Result<u8> {
    let idx = g4.word.checked_add(1).and_then(|w| w.checked_mul(16)).ok_or_else(|| corrupt("stream position"))?;
    let data = g4.index_words.checked_mul(16).and_then(|l| l.checked_add(idx)).ok_or_else(|| corrupt("stream position"))?;
    let a = u16_be(tile, idx).ok_or_else(|| corrupt("index table outside the tile"))?;
    let bytes = tile.get(data..data.checked_add(a).ok_or_else(|| corrupt("stream position"))?).ok_or_else(|| corrupt("half outside the tile"))?;
    let total = bytes.len() * 8;
    let mut bits = Bits::new(bytes);
    let mut lines = 0usize;
    while lines < 8 && total.saturating_sub(bits.consumed_bits()) >= 8 {
        vld_decode_line(&mut bits, total, width2)?;
        lines += 1;
    }
    // The stop rule is sound because a residual line costs at least 8 bits: an all-zero line of width w costs
    // 1 + ceil(log2(ceil(w/4) + 1)) bits, >= 8 for w >= 253, and real plane widths are >= 2496.
    match lines.checked_sub(2) {
        Some(s @ 0..=5) => Ok(s as u8),
        _ => Err(unsupported("first-half row count outside 2..=7")),
    }
}

/// Band rows of one half: plane rows `t` and the rows they land on at each level (`split`).
struct HalfRows {
    t: Range<usize>,
    l1: Vec<usize>,
    h1: Vec<usize>,
    l2: Vec<usize>,
    h2: Vec<usize>,
    l3: Vec<usize>,
    h3: Vec<usize>,
}

/// Rows with `t % 2 == p` go to the low band at `(t - p) / 2`, the others to the high band at `(t - (1 - p)) / 2`.
fn split(rows: &[usize], p: u8) -> (Vec<usize>, Vec<usize>) {
    let p = usize::from(p);
    let (mut lo, mut hi) = (Vec::new(), Vec::new());
    for &t in rows {
        if t % 2 == p {
            lo.push(t.saturating_sub(p) / 2);
        } else {
            hi.push(t.saturating_sub(1 - p) / 2);
        }
    }
    (lo, hi)
}

fn half_layout(n: usize, s: u8, vs: usize, ph: [u8; 3]) -> HalfRows {
    let t = half_rows(n, s, vs);
    let rows: Vec<usize> = t.clone().collect();
    let (l1, h1) = split(&rows, ph[0]);
    let (l2, h2) = split(&l1, ph[1]);
    let (l3, h3) = split(&l2, ph[2]);
    HalfRows { t, l1, h1, l2, h2, l3, h3 }
}

/// Lines a half holds in group `g`: g0 LL3 rows; g1 HL3 + LH3 + HH3; g2 HL2 + LH2 + HH2; g3 HL1 + LH1 + HH1; g4 residual.
fn line_count(g: u8, r: &HalfRows) -> usize {
    match g {
        0 => r.l3.len(),
        1 => r.l3.len() + 2 * r.h3.len(),
        2 => r.l2.len() + 2 * r.h2.len(),
        3 => r.l1.len() + 2 * r.h1.len(),
        _ => r.t.len(),
    }
}

/// Line width of group `g`: level-`l` bands are `(tw / 2) >> l` wide (g0, g1: level 3; g2: 2; g3: 1; g4: the plane).
fn line_width(g: u8, w2: usize) -> usize {
    match g {
        0 | 1 => w2 >> 3,
        2 => w2 >> 2,
        3 => w2 >> 1,
        _ => w2,
    }
}

/// A half with its decoded (still quantised) lines.
type HalfLines = (Half, Vec<Vec<i32>>);

/// Index the stream's halves and VLD-decode exactly the lines each holds; a line running past its half is `Corrupt`.
fn decode_stream(tile: &[u8], st: &StreamRef, ntu: usize, w2: usize, layouts: &[HalfRows]) -> Result<Vec<HalfLines>> {
    let halves = stream_halves(tile, st, ntu)?;
    let width = line_width(st.group, w2);
    // halves are independent bit streams: decode them in parallel
    halves
        .into_par_iter()
        .zip(layouts.par_iter())
        .map(|(half, rows)| {
            let end = half.start.checked_add(half.len).ok_or_else(|| corrupt("half position"))?;
            let bytes = tile.get(half.start..end).ok_or_else(|| corrupt("half outside the tile"))?;
            let total = bytes.len() * 8;
            let mut bits = Bits::new(bytes);
            let mut lines = Vec::with_capacity(line_count(st.group, rows));
            let n = line_count(st.group, rows);
            for _ in 0..n {
                lines.push(vld_decode_line(&mut bits, total, width)?);
                if bits.consumed_bits() > total {
                    return Err(corrupt("line runs past its half"));
                }
            }
            // Phase 0 measurement: in all 3562 halves of the 8 real files the lines end within the half's last
            // byte (`len*8 - 8 < consumed <= len*8`); garbage that decodes to fewer bits is corruption.
            if n > 0 && bits.consumed_bits() <= total.saturating_sub(8) {
                return Err(corrupt("half ends early"));
            }
            Ok((half, lines))
        })
        .collect()
}

/// Dequantise `line` into band row `row`.
fn put(plane: &mut Plane, row: usize, line: &[i32], mut f: impl FnMut(i32) -> i32) -> Result<()> {
    let start = row.checked_mul(plane.width).ok_or_else(|| corrupt("band row"))?;
    let dst = plane.data.get_mut(start..start.saturating_add(plane.width)).ok_or_else(|| corrupt("band row outside its band"))?;
    for (d, &v) in dst.iter_mut().zip(line) {
        *d = f(v);
    }
    Ok(())
}

/// Lay a half's lines into (band, rows, QI) in order: the natural clipped assignment of *Frame geometry*.
fn fill(set: [(&mut Plane, &[usize], u32); 3], lines: &[Vec<i32>]) -> Result<()> {
    let mut it = lines.iter();
    for (plane, rows, qi) in set {
        for &r in rows {
            put(plane, r, it.next().ok_or_else(|| corrupt("half holds fewer lines than rows"))?, |q| dequant(q, qi))?;
        }
    }
    Ok(())
}

/// One colour component: its ten bands from streams g0..g3 (component `c`), then the 3-level inverse.
fn component(c: usize, streams: &[Vec<HalfLines>], layouts: &[HalfRows], vs: usize, w2: usize, ph: [u8; 3]) -> Result<Plane> {
    let (n1l, n1h) = band_rows(vs, ph[0]);
    let (n2l, n2h) = band_rows(n1l, ph[1]);
    let (n3l, n3h) = band_rows(n2l, ph[2]);
    let (w1, w2b, w3) = (w2 >> 1, w2 >> 2, w2 >> 3);
    let z = Plane::zeros;
    let mut b = Bands3 {
        ll3: z(w3, n3l),
        hl3: z(w3, n3l),
        lh3: z(w3, n3h),
        hh3: z(w3, n3h),
        hl2: z(w2b, n2l),
        lh2: z(w2b, n2h),
        hh2: z(w2b, n2h),
        hl1: z(w1, n1l),
        lh1: z(w1, n1h),
        hh1: z(w1, n1h),
    };
    let grp = |g: usize, n: usize| streams.get(3 * g + c).and_then(|s| s.get(n)).ok_or_else(|| corrupt("missing half"));
    for (n, r) in layouts.iter().enumerate() {
        let (h0, l0) = grp(0, n)?;
        // LL3: the nibble must be 0 (Phase 0: never seen otherwise); lines are DPCM from 2048 for the green mean (c = 0) and
        // from 0 for the base-free chroma planes (*Entropy coding and dequantisation*).
        if h0.qi[0] != 0 {
            return Err(unsupported("LL3 quantiser index"));
        }
        for (&row, line) in r.l3.iter().zip(l0) {
            let mut acc = if c == 0 { 2048i32 } else { 0 };
            put(&mut b.ll3, row, line, |d| {
                acc = acc.saturating_add(d);
                acc
            })?;
        }
        let (h1, l1) = grp(1, n)?;
        fill([(&mut b.hl3, &r.l3, h1.qi[0]), (&mut b.lh3, &r.h3, h1.qi[1]), (&mut b.hh3, &r.h3, h1.qi[2])], l1)?;
        let (h2, l2) = grp(2, n)?;
        fill([(&mut b.hl2, &r.l2, h2.qi[0]), (&mut b.lh2, &r.h2, h2.qi[1]), (&mut b.hh2, &r.h2, h2.qi[2])], l2)?;
        let (h3, l3) = grp(3, n)?;
        fill([(&mut b.hl1, &r.l1, h3.qi[0]), (&mut b.lh1, &r.h1, h3.qi[1]), (&mut b.hh1, &r.h1, h3.qi[2])], l3)?;
    }
    reconstruct3(&b, ph)
}

/// A decoded tile: the three wavelet planes (green mean, two chroma) and the green residual, each `tw/2 x th/2`.
pub(crate) struct TilePlanes {
    pub m: Plane,
    pub c1: Plane,
    pub c2: Plane,
    pub res: Plane,
}

/// Decode a tile to its planes: header, `s` from g4's first half, TU count, all 13 streams (in parallel), band
/// assembly and the 3-level inverse per component (in parallel).
pub(crate) fn decode_tile_planes(tile: &[u8], tw: usize, th: usize) -> Result<TilePlanes> {
    let h = parse_tile_header(tile, tw, th)?;
    let (w2, vs) = (tw / 2, h.vs);
    if vs == 0 || !tw.is_multiple_of(2) || !w2.is_multiple_of(8) || w2.checked_mul(vs).is_none_or(|a| a > crate::MAX_SAMPLES) {
        return Err(unsupported("tile geometry"));
    }
    let g4 = h.streams.get(12).ok_or_else(|| corrupt("missing residual stream"))?;
    let s = first_half_shift(tile, g4, w2)?;
    let (ntu, ph) = (tu_count(vs, s), phases(s));
    let layouts: Vec<HalfRows> = (0..2 * ntu).map(|n| half_layout(n, s, vs, ph)).collect();
    let streams =
        h.streams.par_iter().map(|st| decode_stream(tile, st, ntu, w2, &layouts)).collect::<Vec<_>>().into_iter().collect::<Result<Vec<_>>>()?;
    let comps =
        (0..3).into_par_iter().map(|c| component(c, &streams, &layouts, vs, w2, ph)).collect::<Vec<_>>().into_iter().collect::<Result<Vec<_>>>()?;
    // Residual: g4's lines are the half's plane rows, dequantised with the half's nibble.
    let mut res = Plane::zeros(w2, vs);
    for ((half, lines), r) in streams.get(12).ok_or_else(|| corrupt("missing residual stream"))?.iter().zip(&layouts) {
        for (row, line) in r.t.clone().zip(lines) {
            put(&mut res, row, line, |q| dequant(q, half.qi[0]))?;
        }
    }
    let mut it = comps.into_iter();
    let (Some(m), Some(c1), Some(c2)) = (it.next(), it.next(), it.next()) else { return Err(corrupt("missing component")) };
    Ok(TilePlanes { m, c1, c2, res })
}

/// Colour reconstruction (*Colour reconstruction*, checked against Phase 0 oracle output): the planes give a
/// `(2 W2) x (2 H2)` mosaic with R at (0,0), G1 (0,1), G2 (1,0), B (1,1) of every 2x2 cell, each value passed through
/// [`curve`]. G1 comes from the green mean minus a residual average, G2 is predicted from the (unclipped) G1 plus the
/// residual, and R/B are chroma plus the mean of the greens clamped to 0..=4095.
pub(crate) fn colour(p: &TilePlanes) -> Result<Vec<u16>> {
    let (w2, h2) = (p.m.width, p.m.height);
    let len = w2.checked_mul(h2).ok_or_else(|| corrupt("tile size"))?;
    if [&p.c1, &p.c2, &p.res].iter().any(|q| q.width != w2 || q.height != h2 || q.data.len() != len) || p.m.data.len() != len {
        return Err(corrupt("plane size mismatch"));
    }
    let mw = w2.checked_mul(2).ok_or_else(|| corrupt("tile size"))?;
    let total = len.checked_mul(4).ok_or_else(|| corrupt("tile size"))?;
    // (rows as slices: `at` is 0 past a row's end, as never happens once the sizes above match)
    let at = |r: &[i32], x: usize| i64::from(r.get(x).copied().unwrap_or(0));
    let mut g1 = vec![0i64; len];
    g1.par_chunks_mut(w2.max(1)).enumerate().for_each(|(j, row)| {
        let (Some(m), Some(rp), Some(rj)) = (p.m.row(j), p.res.row(j.saturating_sub(1)), p.res.row(j)) else { return };
        for (x, g) in row.iter_mut().enumerate() {
            let xn = (x + 1).min(w2 - 1);
            let r = at(rp, x) + at(rp, xn) + at(rj, x) + at(rj, xn);
            *g = at(m, x) - ((r + 4) >> 3);
        }
    });
    let g1_row = |j: usize| g1.get(j * w2..(j + 1) * w2).unwrap_or(&[]);
    let g = |r: &[i64], x: usize| r.get(x).copied().unwrap_or(0);
    let mut out = vec![0u16; total];
    // each plane row gives two mosaic rows
    out.par_chunks_mut((2 * mw).max(1)).enumerate().for_each(|(j, rows)| {
        let (gj, gn) = (g1_row(j), g1_row((j + 1).min(h2 - 1)));
        let (Some(res), Some(c1), Some(c2), Some((top, bottom))) = (p.res.row(j), p.c1.row(j), p.c2.row(j), rows.split_at_mut_checked(mw)) else {
            return;
        };
        for (x, (t, b)) in top.as_chunks_mut::<2>().0.iter_mut().zip(bottom.as_chunks_mut::<2>().0.iter_mut()).enumerate() {
            let xp = x.saturating_sub(1);
            let g2 = ((g(gj, xp) + g(gj, x) + g(gn, xp) + g(gn, x)) >> 2) + at(res, x);
            let mean = (g(gj, x).clamp(0, 4095) + g2.clamp(0, 4095)) >> 1;
            let cells = [2 * at(c1, x) + mean, g(gj, x), g2, 2 * at(c2, x) + mean];
            for (o, v) in t.iter_mut().chain(b.iter_mut()).zip(cells) {
                *o = curve(v.clamp(0, 4095) as i32);
            }
        }
    });
    Ok(out)
}

/// One tile to its `tw x th` mosaic in output units.
pub(crate) fn decode_tile(tile: &[u8], tw: usize, th: usize) -> Result<Vec<u16>> {
    colour(&decode_tile_planes(tile, tw, th)?)
}

/// Decode a whole ARW6 strip: [`Mode::Header`](crate::Mode) validates the tile table, tile headers and stream
/// tables and returns no samples; `Full` decodes the tiles in parallel into a `width x height` mosaic.
pub(crate) fn decode(strip: &[u8], width: usize, height: usize, mode: crate::Mode) -> Result<Vec<u16>> {
    let tiles = parse_tile_table(strip, width, height)?;
    let bytes = |i: usize| tile_bytes(strip, &tiles, i).ok_or_else(|| corrupt("tile outside strip"));
    if mode == crate::Mode::Header {
        for (i, t) in tiles.iter().enumerate() {
            let tile = bytes(i)?;
            let h = parse_tile_header(tile, t.width, t.height)?;
            let g4 = h.streams.get(12).ok_or_else(|| corrupt("missing residual stream"))?;
            let s = first_half_shift(tile, g4, t.width / 2)?;
            let ntu = tu_count(h.vs, s);
            for st in &h.streams {
                stream_halves(tile, st, ntu)?;
            }
        }
        return Ok(Vec::new());
    }
    // a tile that fails is reported before the mosaic is allocated (storytold/lightcraft#1512)
    let decoded: Vec<Vec<u16>> = tiles.par_iter().enumerate().map(|(i, t)| decode_tile(bytes(i)?, t.width, t.height)).collect::<Result<_>>()?;
    let mut out = vec![0u16; width.checked_mul(height).ok_or_else(|| corrupt("image size"))?];
    for (t, d) in tiles.iter().zip(decoded) {
        for (r, row) in d.chunks_exact(t.width.max(1)).enumerate() {
            let start = (t.y + r).checked_mul(width).and_then(|a| a.checked_add(t.x)).ok_or_else(|| corrupt("tile placement"))?;
            let dst = start.checked_add(row.len()).and_then(|end| out.get_mut(start..end)).ok_or_else(|| corrupt("tile outside image"))?;
            dst.copy_from_slice(row);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arw6_testenc::*;
    use crate::huffman::Bits;
    use craft_tiff::Tiff;
    use craft_tiff::image::chunk_bytes;
    use craft_tiff::tags as t;

    fn planes(w2: usize, h2: usize, seed: u64) -> (Plane, Plane, Plane, Plane) {
        let mut x = seed.max(1);
        let mut next = |span: i32| -> i32 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % (2 * span as u64 + 1)) as i32 - span
        };
        let mut mk = |centre: i32, span: i32| Plane { width: w2, height: h2, data: (0..w2 * h2).map(|_| centre + next(span)).collect() };
        (mk(2048, 200), mk(0, 60), mk(0, 60), mk(0, 24))
    }

    fn one_tile(w: usize, h: usize, s: u8, qi: Qis) -> (Vec<u8>, TileSpec) {
        seeded_tile(w, h, s, qi, 7)
    }

    fn seeded_tile(w: usize, h: usize, s: u8, qi: Qis, seed: u64) -> (Vec<u8>, TileSpec) {
        let (m, c1, c2, res) = planes(w / 2, h / 2, seed);
        let t = TileSpec { s, qi, m, c1, c2, res };
        (encode_tile(&t), t)
    }

    /// The raw strip bytes of the compression-32766 IFD, as `arw.rs` locates them.
    fn strip_of(file: &[u8]) -> &[u8] {
        let tiff = Tiff::parse(file).unwrap();
        let raw = tiff.all_ifds().into_iter().find(|i| i.u16(t::COMPRESSION) == Some(32766)).unwrap();
        let info = raw.image().unwrap();
        let chunks = info.chunks(file.len() as u64);
        chunk_bytes(file, &chunks[0]).unwrap()
    }

    #[test]
    fn tile_table_round_trips() {
        let (tile, _) = one_tile(64, 48, 0, Qis::ZERO);
        let file = arw6_file(&[(0, 0, 64, 48, tile.clone())], 64, 48);
        let strip = strip_of(&file);
        let t = parse_tile_table(strip, 64, 48).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].x, t[0].y, t[0].width, t[0].height, t[0].offset), (0, 0, 64, 48, 512));
    }

    #[test]
    fn tile_table_rejects_bad_layouts() {
        let (tile, _) = one_tile(64, 48, 0, Qis::ZERO);
        let strip = strip_of(&arw6_file(&[(0, 0, 64, 48, tile.clone())], 64, 48)).to_vec();
        let mut zero = strip.clone();
        zero[0] = 0;
        assert!(parse_tile_table(&zero, 64, 48).is_err());
        let mut far = strip.clone();
        far[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(parse_tile_table(&far, 64, 48).is_err());
        let mut odd = strip.clone();
        odd[20..24].copy_from_slice(&63u32.to_le_bytes()); // the first tile's y: odd, and no longer covering
        assert!(parse_tile_table(&odd, 64, 48).is_err());
        assert!(parse_tile_table(&strip, 64, 64).is_err()); // does not cover the image
        let two = arw6_file(&[(0, 0, 64, 48, tile.clone()), (0, 0, 64, 48, tile.clone())], 128, 48);
        assert!(parse_tile_table(strip_of(&two), 128, 48).is_err()); // overlapping
        assert!(parse_tile_table(&strip[..7], 64, 48).is_err()); // truncated table
    }

    #[test]
    fn tile_header_and_streams() {
        let (tile, _) = one_tile(64, 48, 3, Qis::REAL);
        let h = parse_tile_header(&tile, 64, 48).unwrap();
        assert_eq!((h.hs, h.vs, h.streams.len()), (64, 24, 13));
        let ntu = tu_count(24, 3);
        assert_eq!(ntu, 2);
        assert_eq!(
            h.streams.iter().map(|s| (s.group, s.comp)).collect::<Vec<_>>(),
            [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (2, 0), (2, 1), (2, 2), (3, 0), (3, 1), (3, 2), (4, 0)]
        );
        let g3 = stream_halves(&tile, &h.streams[9], ntu).unwrap();
        assert_eq!(g3.len(), 4);
        assert_eq!(g3[0].qi, [1, 1, 2]);
        let g4 = stream_halves(&tile, &h.streams[12], ntu).unwrap();
        assert_eq!(g4[1].qi, [2, 2, 2]);
        assert!(g4[3].start + g4[3].len <= tile.len());
        let mut bad = tile.clone();
        bad[13] = 0xff; // corrupts word 0's BBD bits: any picture-header deviation is Unsupported
        assert!(matches!(parse_tile_header(&bad, 64, 48), Err(RawError::Unsupported(_))));
        assert!(parse_tile_header(&tile, 64, 50).is_err()); // VS != th / 2
    }

    fn put32(b: &mut [u8], at: usize, v: u32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn tile_table_validation_cases() {
        let (tile, _) = one_tile(64, 48, 0, Qis::ZERO);
        let many = |n: usize| {
            let tiles: Vec<_> = (0..n).map(|i| (64 * i, 0, 64, 48, tile.clone())).collect();
            strip_of(&arw6_file(&tiles, 64 * n, 48)).to_vec()
        };
        let sixteen = many(16);
        assert_eq!(parse_tile_table(&sixteen, 1024, 48).unwrap().len(), 16);
        let mut seventeen = sixteen.clone();
        seventeen[0] = 17;
        assert!(parse_tile_table(&seventeen, 1088, 48).is_err());

        let two = strip_of(&arw6_file(&[(0, 0, 64, 48, tile.clone()), (64, 0, 64, 48, tile.clone())], 128, 48)).to_vec();
        assert_eq!(parse_tile_table(&two, 128, 48).unwrap().len(), 2);
        let mut swapped = two.clone();
        let (a, b) = (two[8..16].to_vec(), two[32..40].to_vec());
        swapped[8..16].copy_from_slice(&b);
        swapped[32..40].copy_from_slice(&a);
        assert!(parse_tile_table(&swapped, 128, 48).is_err());
        let mut at_end = two.clone();
        at_end[32..40].copy_from_slice(&(two.len() as u64).to_le_bytes());
        assert!(parse_tile_table(&at_end, 128, 48).is_err());

        let one = many(1);
        let mut wide = one.clone();
        put32(&mut wide, 24, 72); // w / 2 = 36 is not a multiple of 8
        assert!(parse_tile_table(&wide, 72, 48).is_err());
        let mut huge = one.clone();
        put32(&mut huge, 24, 1 << 20);
        put32(&mut huge, 28, 1 << 20);
        assert!(matches!(parse_tile_table(&huge, 1 << 20, 1 << 20), Err(RawError::Limit(_))));
    }

    #[test]
    fn tile_bytes_span_to_the_next_offset() {
        let (tile, _) = one_tile(64, 48, 0, Qis::ZERO);
        let file = arw6_file(&[(0, 0, 64, 48, tile.clone()), (64, 0, 64, 48, tile.clone())], 128, 48);
        let strip = strip_of(&file);
        let tiles = parse_tile_table(strip, 128, 48).unwrap();
        let (t0, t1) = (tile_bytes(strip, &tiles, 0).unwrap(), tile_bytes(strip, &tiles, 1).unwrap());
        assert_eq!(t0.len(), tiles[1].offset - tiles[0].offset);
        assert_eq!(t0, &tile[..]);
        assert_eq!(t1.as_ptr() as usize + t1.len(), strip.as_ptr() as usize + strip.len());
        assert!(tile_bytes(strip, &tiles, 2).is_none());
    }

    #[test]
    fn tile_header_and_stream_validation_cases() {
        let (tile, _) = one_tile(64, 48, 0, Qis::REAL);
        let h = parse_tile_header(&tile, 64, 48).unwrap();
        let mut two_streams = tile.clone();
        two_streams[3 * 16] = 2;
        assert!(parse_tile_header(&two_streams, 64, 48).is_err());
        let mut big = tile.clone();
        big[3 * 16 + 1..3 * 16 + 4].copy_from_slice(&[0xff; 3]); // stream 0 larger than the tile
        assert!(parse_tile_header(&big, 64, 48).is_err());
        let mut idx = tile.clone();
        idx[8 * 16..8 * 16 + 2].copy_from_slice(&[0xff, 0xff]); // index words beyond the stream
        assert!(parse_tile_header(&idx, 64, 48).is_err());

        let s = &h.streams[9];
        let mut bad_half = tile.clone();
        let idx_start = (s.word + 1) * 16;
        bad_half[idx_start..idx_start + 2].copy_from_slice(&[0xff, 0xff]); // A past the data area
        assert!(stream_halves(&bad_half, s, tu_count(24, 0)).is_err());
        let outside = StreamRef { index_words: 0xffff, ..s.clone() };
        assert!(stream_halves(&tile, &outside, tu_count(24, 0)).is_err());
    }

    #[test]
    fn every_stream_of_an_encoded_tile_parses() {
        for s in [0u8, 3] {
            let (tile, spec) = one_tile(64, 48, s, Qis::REAL);
            assert_eq!(tile.len() % 4096, 0);
            let h = parse_tile_header(&tile, 64, 48).unwrap();
            let ntu = tu_count(24, s);
            for st in &h.streams {
                assert_eq!(stream_halves(&tile, st, ntu).unwrap().len(), 2 * ntu);
            }
            // the residual stream's halves decode to the clipped, quantised rows 8n-6+s ..= 8n+1+s
            let g4 = stream_halves(&tile, &h.streams[12], ntu).unwrap();
            for (n, half) in g4.iter().enumerate() {
                let bytes = &tile[half.start..half.start + half.len];
                let mut bits = Bits::new(bytes);
                for r in (8 * n as i32 - 6 + s as i32..8 * n as i32 + 2 + s as i32).filter(|&r| (0..24).contains(&r)) {
                    let line = crate::llvc::vld_decode_line(&mut bits, bytes.len() * 8, 32).unwrap();
                    let want: Vec<i32> = spec.res.row(r as usize).unwrap().iter().map(|&v| quantize(v, 2)).collect();
                    assert_eq!(line, want, "s {s} half {n} row {r}");
                }
            }
            // a stream that claims more halves than its index holds is rejected
            assert!(stream_halves(&tile, &h.streams[0], 1000).is_err());
        }
    }

    #[test]
    fn encoder_decodes_its_own_lines() {
        let vals: Vec<i32> = (0..100).map(|i| (i * 37) % 23 - 11).collect();
        let mut w = BitWriter::default();
        vld_encode_line(&vals, 100, &mut w);
        let bytes = w.finish();
        let mut b = Bits::new(&bytes);
        assert_eq!(crate::llvc::vld_decode_line(&mut b, bytes.len() * 8, 100).unwrap(), vals);
    }

    #[test]
    fn phases_and_half_rows_match_the_spec() {
        assert_eq!(phases(0), [0, 1, 1]);
        assert_eq!(phases(3), [1, 0, 0]);
        assert_eq!(half_rows(0, 0, 1668), 0..2);
        assert_eq!(half_rows(209, 0, 1668), 1666..1668);
        assert_eq!(half_rows(1, 0, 1668), 2..10);
        assert_eq!(half_rows(0, 3, 2186), 0..5);
        assert_eq!(half_rows(273, 3, 2186), 2181..2186);
        assert_eq!(half_rows(300, 3, 2186), 2186..2186);
    }

    #[test]
    fn tu_count_covers_every_row() {
        assert_eq!((tu_count(1668, 0), tu_count(2186, 3), tu_count(24, 3), tu_count(48, 0)), (105, 137, 2, 4));
        for s in 0u8..=5 {
            for vs in 1usize..200 {
                let last = half_rows(2 * tu_count(vs, s) - 1, s, vs);
                assert!(last.end == vs || (2 * tu_count(vs, s) - 1) * 8 + usize::from(s) + 2 >= vs, "s {s} vs {vs}");
                assert_eq!((0..2 * tu_count(vs, s)).map(|n| half_rows(n, s, vs).len()).sum::<usize>(), vs, "s {s} vs {vs}");
            }
        }
    }

    #[test]
    fn first_half_shift_is_read_from_the_stream() {
        for s in [0u8, 3, 5] {
            let (tile, _) = one_tile(64, 48, s, Qis::REAL);
            let h = parse_tile_header(&tile, 64, 48).unwrap();
            assert_eq!(first_half_shift(&tile, &h.streams[12], 32).unwrap(), s);
        }
    }

    #[test]
    fn round_trip_every_shift() {
        for s in 0u8..=5 {
            for (w, h) in [(64usize, 48usize), (80, 96), (64, 36)] {
                let (tile, spec) = one_tile(w, h, s, Qis::ZERO); // QI 0: lossless, planes must come back exactly
                let p = decode_tile_planes(&tile, w, h).unwrap_or_else(|e| panic!("s={s} {w}x{h}: {e}"));
                assert_eq!(p.m, spec.m, "s={s} {w}x{h} m");
                assert_eq!(p.c1, spec.c1);
                assert_eq!(p.c2, spec.c2);
                assert_eq!(p.res, spec.res);
            }
        }
    }

    #[test]
    fn round_trip_with_real_quantisers() {
        // bands drawn as quantised integers q, planes = reconstruct3(dequant(q)); the encoder must reproduce them bit for bit
        for s in [0u8, 3] {
            let (tile, spec) = one_tile_quantised(96, 64, s, Qis::REAL);
            let p = decode_tile_planes(&tile, 96, 64).unwrap();
            assert_eq!(p.m, spec.m);
            assert_eq!(p.c1, spec.c1);
            assert_eq!(p.c2, spec.c2);
            assert_eq!(p.res, spec.res);
        }
    }

    #[test]
    fn half_too_short_is_corrupt() {
        let (mut tile, _) = one_tile(64, 48, 0, Qis::ZERO);
        let h = parse_tile_header(&tile, 64, 48).unwrap();
        let w = h.streams[9].word * 16 + 16; // first index entry of g3 c0: shrink A by 1 byte
        let a = u16::from_be_bytes([tile[w], tile[w + 1]]) - 1;
        tile[w..w + 2].copy_from_slice(&a.to_be_bytes());
        assert!(matches!(decode_tile_planes(&tile, 64, 48), Err(RawError::Corrupt(_))));
    }

    proptest::proptest! {
        #[test]
        fn mutated_tiles_never_panic(flips in proptest::collection::vec((proptest::num::usize::ANY, proptest::num::u8::ANY), 1..12), cut in proptest::num::usize::ANY) {
            let (mut tile, _) = one_tile(64, 48, 3, Qis::REAL);
            for (i, b) in flips { let i = i % tile.len(); tile[i] ^= b; }
            let n = cut % (tile.len() + 1);
            let _ = decode_tile(&tile[..n], 64, 48);
            let _ = decode_tile(&tile, 64, 48);
        }
    }

    fn plane(w: usize, v: &[i32]) -> Plane {
        Plane { width: w, height: v.len() / w, data: v.to_vec() }
    }

    fn example() -> TilePlanes {
        TilePlanes {
            m: plane(3, &[2048, 2052, 2060, 2050, 2049, 2070]),
            res: plane(3, &[4, -8, 2, 0, 2, -6]),
            c1: plane(3, &[10, -10, 0, 0, 3, -2]),
            c2: plane(3, &[-5, 7, 1, 2, 0, 4]),
        }
    }

    #[test]
    fn colour_matches_the_phase0_example() {
        let out = colour(&example()).unwrap(); // 6 wide x 4 rows, curve units
        let code = |r: usize, c: usize| out[r * 6 + c] as i32;
        assert_eq!(
            [code(0, 1), code(0, 3), code(0, 5), code(2, 1), code(2, 3), code(2, 5)],
            [2049, 2053, 2059, 2050, 2050, 2071].map(|c| curve(c) as i32)
        ); // G1
        assert_eq!(
            [code(1, 0), code(1, 2), code(1, 4), code(3, 0), code(3, 2), code(3, 4)],
            [2053, 2042, 2060, 2050, 2052, 2054].map(|c| curve(c) as i32)
        ); // G2
        assert_eq!([code(0, 0), code(0, 2), code(0, 4)], [2071, 2027, 2059].map(|c| curve(c) as i32)); // R
        assert_eq!([code(1, 1), code(1, 3), code(1, 5)], [2041, 2061, 2061].map(|c| curve(c) as i32)); // B
    }

    #[test]
    fn colour_clips_like_the_oracle() {
        let mut p = example();
        p.m.data[1] = 4100;
        let out = colour(&p).unwrap();
        assert_eq!(out[3], curve(4095)); // G1 = 4101 -> clipped output
        assert_eq!(out[6 + 2], curve(2554)); // G2 predicted from the unclipped G1
        assert_eq!(out[2], curve(3304)); // R = 2*(-10) + ((4095 + 2554) >> 1): chroma uses the clipped greens
    }

    #[test]
    fn colour_clamps_negative_greens() {
        let p = TilePlanes {
            m: plane(2, &[10, 2048, 2048, 2048]),
            res: plane(2, &[100, 100, 0, 0]),
            c1: plane(2, &[7, 0, 0, 0]),
            c2: plane(2, &[-3, 0, 0, 0]),
        };
        let g1_00 = 10 - ((400 + 4) >> 3); // -40
        let g1_10 = 2048 - ((200 + 4) >> 3);
        let g2_00 = ((2 * g1_00 + 2 * g1_10) >> 2) + 100;
        assert!(g1_00 < 0);
        let mean = g2_00.clamp(0, 4095) >> 1; // G1 contributes 0
        let out = colour(&p).unwrap();
        assert_eq!(out[0], curve(14 + mean));
        assert_eq!(out[1], curve(0)); // G1 itself clamps
        assert_eq!(out[2 * 2 + 1], curve(-6 + mean)); // B at (0,0): row 1, col 1 of a 4-wide mosaic
    }

    #[test]
    fn decode_header_and_full_and_truncations() {
        let (tile, _) = one_tile(64, 48, 3, Qis::REAL);
        let file = arw6_file(&[(0, 0, 64, 48, tile)], 64, 48);
        let strip = strip_of(&file);
        assert!(decode(strip, 64, 48, crate::Mode::Header).unwrap().is_empty());
        assert_eq!(decode(strip, 64, 48, crate::Mode::Full).unwrap().len(), 64 * 48);
        for n in 0..strip.len() {
            let _ = decode(&strip[..n], 64, 48, crate::Mode::Full);
            let _ = decode(&strip[..n], 64, 48, crate::Mode::Header);
        }
    }

    #[test]
    fn garbage_streams_import_then_fail() {
        let (mut tile, _) = one_tile(64, 48, 0, Qis::REAL);
        let h = parse_tile_header(&tile, 64, 48).unwrap();
        let st = &h.streams[9]; // g3 c0
        let start = (st.word + 1 + st.index_words) * 16;
        for (i, b) in tile[start..start + st.data_words * 16].iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(97) ^ 0xa5;
        }
        let strip = strip_of(&arw6_file(&[(0, 0, 64, 48, tile)], 64, 48)).to_vec();
        assert!(decode(&strip, 64, 48, crate::Mode::Header).is_ok());
        assert!(matches!(decode(&strip, 64, 48, crate::Mode::Full), Err(RawError::Corrupt(_))));
    }

    #[test]
    fn chroma_planes_are_base_free() {
        let (w2, h2) = (32, 24);
        let flat = |v: i32| Plane { width: w2, height: h2, data: vec![v; w2 * h2] };
        let (m, _, _, res) = planes(w2, h2, 7);
        for (c1, c2) in [(0, 0), (5, -3)] {
            let t = TileSpec { s: 0, qi: Qis::ZERO, m: m.clone(), c1: flat(c1), c2: flat(c2), res: res.clone() };
            let tile = encode_tile(&t);
            let p = decode_tile_planes(&tile, 64, 48).unwrap();
            assert!(p.c1.data.iter().all(|&v| v == c1) && p.c2.data.iter().all(|&v| v == c2));
            if c1 == 0 {
                // zero chroma: R = B = (G1 + G2) >> 1 at every cell (no 2048 base, no saturation)
                let out = decode_tile(&tile, 64, 48).unwrap();
                for j in 0..h2 {
                    for x in 0..w2 {
                        let at = |dy: usize, dx: usize| out[(2 * j + dy) * 64 + 2 * x + dx];
                        assert_eq!(at(0, 0), at(1, 1), "R == B at ({x},{j})");
                        assert!(at(0, 0) < curve(4095));
                    }
                }
            }
        }
    }

    #[test]
    fn arw6_white_without_tag_is_32766() {
        let (tile, _) = one_tile(64, 48, 0, Qis::ZERO);
        let file = arw6_file_no_white(&[(0, 0, 64, 48, tile)], 64, 48);
        let img = crate::decode(&file).unwrap();
        assert_eq!(img.white, [32766.0]);
    }

    #[test]
    fn twelve_bit_mode_is_unsupported() {
        let (tile, _) = one_tile(64, 48, 0, Qis::ZERO);
        let file = arw6_file_bits(&[(0, 0, 64, 48, tile)], 64, 48, 12);
        assert!(matches!(crate::decode(&file), Err(RawError::Unsupported(_))));
    }

    #[test]
    fn shift_out_of_range_is_unsupported() {
        let (w2, h2) = (32, 24);
        let (m, c1, c2, res) = planes(w2, h2, 7);
        let tile = encode_tile(&TileSpec { s: 6, qi: Qis::ZERO, m, c1, c2, res });
        assert!(matches!(decode_tile_planes(&tile, 64, 48), Err(RawError::Unsupported(_))));
    }

    #[test]
    fn corrupt_streams_import_then_fail() {
        // Garbage VLD data is self-delimiting and decodes to *something*; what is detectable is a half whose index
        // length is too short for the lines it must hold.
        let (mut tile, _) = one_tile(64, 48, 0, Qis::REAL);
        let h = parse_tile_header(&tile, 64, 48).unwrap();
        let at = (h.streams[9].word + 1) * 16; // g3 c0 index table, first entry: half 0's byte length (u16 BE)
        tile[at] = 0;
        tile[at + 1] = 1;
        let strip = strip_of(&arw6_file(&[(0, 0, 64, 48, tile)], 64, 48)).to_vec();
        assert!(decode(&strip, 64, 48, crate::Mode::Header).is_ok());
        assert!(matches!(decode(&strip, 64, 48, crate::Mode::Full), Err(RawError::Corrupt(_))));
    }

    #[test]
    fn four_tiles_place_correctly_and_arw_reports_doubled_levels() {
        let tiles: Vec<_> = [(0, 0), (64, 0), (0, 48), (64, 48)]
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| {
                let (t, _) = seeded_tile(64, 48, 0, Qis::ZERO, 11 + i as u64 * 17); // a different image per tile
                (x, y, 64, 48, t)
            })
            .collect();
        let file = arw6_file(&tiles, 128, 96);
        let img = crate::decode(&file).unwrap();
        assert_eq!((img.width, img.height, img.bits), (128, 96, 16));
        assert_eq!(img.black.values, vec![1024.0; 4]);
        assert_eq!(img.white, vec![30720.0]);
        assert_eq!(img.cfa.as_ref().map(|c| c.pattern.clone()), Some(vec![0, 1, 1, 2]));
        let crate::RawData::U16(d) = img.data else { panic!() };
        for (x, y, w, h, bytes) in &tiles {
            let want = decode_tile(bytes, *w, *h).unwrap();
            for r in 0..*h {
                assert_eq!(&d[(y + r) * 128 + x..(y + r) * 128 + x + w], &want[r * w..(r + 1) * w], "tile at ({x},{y}) row {r}");
            }
        }
        assert_eq!(crate::probe_info(&file).unwrap().width, 128); // header mode: tile tables and headers only
    }
}
