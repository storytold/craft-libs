//! Test-only ARW6 synthetic encoder pieces: forward 5/3 transforms (exact inverse of `llvc::inverse_*`) and `quantize`.
//! Section names in italics refer to `docs/arw6-compression.md`.
#![allow(clippy::unwrap_used, clippy::type_complexity)] // a test-only encoder (the module is `cfg(test)`)

use crate::llvc::{Bands3, Plane, band_rows};
use craft_tiff::tags as t;
use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value};

/// Exact inverse of `dequant` on representable values: `sign(v) * (|v| >> qi)`.
pub(crate) fn quantize(v: i32, qi: u32) -> i32 {
    v.signum() * (v.abs() >> qi)
}

fn at(v: &[i32], i: isize) -> i32 {
    if v.is_empty() { 0 } else { v[i.clamp(0, v.len() as isize - 1) as usize] }
}

/// Forward 1-D 5/3 with whole-sample symmetric extension (port of the Phase 0 reference model, local and not in the repo).
fn forward_1d(x: &[i32], phase: u8) -> (Vec<i32>, Vec<i32>) {
    let n = x.len() as isize;
    let p = isize::from(phase);
    let xs = |i: isize| -> i32 {
        let r = if i < 0 {
            -i
        } else if i >= n {
            2 * (n - 1) - i
        } else {
            i
        };
        if (0..n).contains(&r) { x[r as usize] } else { 0 }
    };
    let (nl, nh) = band_rows(x.len(), phase);
    let h: Vec<i32> = (0..nh as isize)
        .map(|k| {
            let pos = 2 * k + 1 - p;
            xs(pos) - ((xs(pos - 1) + xs(pos + 1)) >> 1)
        })
        .collect();
    let l = (0..nl as isize).map(|j| xs(2 * j + p) + ((at(&h, j - 1 + p) + at(&h, j + p) + 2) >> 2)).collect();
    (l, h)
}

fn split(x: &Plane, phase: u8, vertical: bool) -> (Plane, Plane) {
    let (across, n) = if vertical { (x.width, x.height) } else { (x.height, x.width) };
    let (nl, nh) = band_rows(n, phase);
    let dims = |m| if vertical { (across, m) } else { (m, across) };
    let (mut lo, mut hi) = (Plane::zeros(dims(nl).0, dims(nl).1), Plane::zeros(dims(nh).0, dims(nh).1));
    for i in 0..across {
        let line: Vec<i32> = (0..n).map(|k| if vertical { x.data[k * x.width + i] } else { x.data[i * x.width + k] }).collect();
        let (l, h) = forward_1d(&line, phase);
        for (p, v) in [(&mut lo, l), (&mut hi, h)] {
            for (k, v) in v.into_iter().enumerate() {
                let idx = if vertical { k * p.width + i } else { i * p.width + k };
                p.data[idx] = v;
            }
        }
    }
    (lo, hi)
}

pub(crate) fn forward_53_1d_rows(x: &Plane, phase: u8) -> (Plane, Plane) {
    split(x, phase, true)
}

pub(crate) fn forward_53_1d_cols(x: &Plane, phase: u8) -> (Plane, Plane) {
    split(x, phase, false)
}

/// Horizontal first (phase 0), then vertical; returns `(ll, lh, hl, hh)`, the exact inverse of `inverse_53_2d`.
pub(crate) fn forward_53_2d(x: &Plane, vphase: u8) -> (Plane, Plane, Plane, Plane) {
    let (lo, hi) = forward_53_1d_cols(x, 0);
    let (ll, lh) = forward_53_1d_rows(&lo, vphase);
    let (hl, hh) = forward_53_1d_rows(&hi, vphase);
    (ll, lh, hl, hh)
}

/// 3-level forward, the exact inverse of `reconstruct3` with the same `phases`.
pub(crate) fn forward3(x: &Plane, phases: [u8; 3]) -> Bands3 {
    let (ll1, lh1, hl1, hh1) = forward_53_2d(x, phases[0]);
    let (ll2, lh2, hl2, hh2) = forward_53_2d(&ll1, phases[1]);
    let (ll3, lh3, hl3, hh3) = forward_53_2d(&ll2, phases[2]);
    Bands3 { ll3, hl3, lh3, hh3, hl2, lh2, hh2, hl1, lh1, hh1 }
}

// ---- RDD 34 line encoder (mirror of `llvc::vld_decode_line`; port of the Phase 0 reference model's encoder) ----

/// MSB-first bit sink; one `0`/`1` per element until `finish`.
#[derive(Default)]
pub(crate) struct BitWriter {
    bits: Vec<u8>,
}

impl BitWriter {
    pub(crate) fn put(&mut self, bit: bool) {
        self.bits.push(u8::from(bit));
    }
    pub(crate) fn put_n(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            self.put((v >> i) & 1 == 1);
        }
    }
    fn zeros(&mut self, n: u32) {
        for _ in 0..n {
            self.put(false);
        }
    }
    /// Zero-padded to a whole byte.
    pub(crate) fn finish(self) -> Vec<u8> {
        self.bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |a, (i, &b)| a | (b << (7 - i)))).collect()
    }
}

/// Encode one line (`width` coefficients, padded with zeros to whole sets of 4), with Sony's two deviations.
pub(crate) fn vld_encode_line(values: &[i32], width: usize, out: &mut BitWriter) {
    let sets_n = width.div_ceil(4);
    let mut vals = values[..width.min(values.len())].to_vec();
    vals.resize(4 * sets_n, 0);
    let sets: Vec<&[i32]> = vals.chunks(4).collect();
    let dpts: Vec<u32> = sets.iter().map(|s| 32 - s.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0).leading_zeros()).collect();
    let (mut dpt, mut dpts_state, mut cnt, mut zr) = (0u32, 0u8, sets_n, 0usize);
    let mut prev: Option<&[i32]> = None;
    let mut k = 0usize;
    loop {
        let dpt_v = cnt != 0;
        cnt = cnt.saturating_sub(1);
        zr = zr.saturating_sub(1);
        if dpt_v {
            let d = dpts[k];
            match dpts_state {
                0 if d == dpt => {
                    out.put(false);
                    if dpt == 0 {
                        dpts_state = 1;
                    }
                }
                0 if d > dpt => {
                    out.put(true);
                    out.put(false);
                    out.zeros(d - dpt - 1);
                    out.put(true);
                    dpt = d;
                }
                0 => {
                    out.put(true);
                    out.put(true);
                    if d >= 1 {
                        out.zeros(dpt - d - 1);
                        out.put(true);
                        dpt = d;
                    } else {
                        out.zeros(dpt - 1);
                        dpt = 0;
                        dpts_state = 1;
                    }
                }
                1 if d > 0 => {
                    out.put(true);
                    out.zeros(d - 1);
                    out.put(true);
                    dpt = d;
                    dpts_state = 0;
                }
                1 => {
                    let r = dpts[k..].iter().take_while(|&&x| x == 0).count();
                    let lim = (cnt as u64 + 2).next_power_of_two().trailing_zeros();
                    if k + r >= sets_n {
                        out.zeros(lim); // the rest of the line is zero
                        zr = cnt + 2;
                    } else {
                        let v = (r + 1) as u32;
                        let m = 31 - v.leading_zeros();
                        assert!(m < lim);
                        out.zeros(m);
                        out.put(true);
                        out.put_n(v - (1 << m), m);
                        zr = r + 1;
                    }
                    dpts_state = 2;
                }
                _ if zr == 1 => {
                    out.zeros(d - 1);
                    out.put(true);
                    dpt = d;
                    dpts_state = 0;
                }
                _ => assert_eq!(d, 0),
            }
        }
        if let Some(p) = prev {
            for &v in p {
                if v != 0 {
                    out.put(v < 0);
                }
            }
        }
        if k >= sets_n {
            return;
        }
        if dpt_v {
            for &v in sets[k] {
                out.put_n(v.unsigned_abs(), dpt);
            }
            prev = Some(sets[k]);
        }
        k += 1;
    }
}

// ---- tile / file serialiser (`tilewrite.py::TileModel.serialize`, `arw6dump.py`) ----

/// Per-band quantiser indices: `l1`/`l2`/`l3` per component, bands in HL, LH, HH order.
#[derive(Clone)]
pub(crate) struct Qis {
    pub l1: [[u32; 3]; 3],
    pub l2: [[u32; 3]; 3],
    pub l3: [[u32; 3]; 3],
    pub res: u32,
}

impl Qis {
    pub(crate) const ZERO: Qis = Qis { l1: [[0; 3]; 3], l2: [[0; 3]; 3], l3: [[0; 3]; 3], res: 0 };
    /// The values seen in every real file (*Container*).
    pub(crate) const REAL: Qis = Qis { l1: [[1, 1, 2], [2, 2, 3], [2, 2, 3]], l2: [[0, 0, 1]; 3], l3: [[0; 3]; 3], res: 2 };
}

/// One tile to encode: `s` is the anchor offset (*Frame geometry*), planes are W/2 x H/2.
pub(crate) struct TileSpec {
    pub s: u8,
    pub qi: Qis,
    pub m: Plane,
    pub c1: Plane,
    pub c2: Plane,
    pub res: Plane,
}

/// Band row indices of the plane rows `rows` (low: `t % 2 == p`, row `(t-p)/2`; high: row `(t-(1-p))/2`).
fn split_rows(rows: &[usize], p: usize) -> (Vec<usize>, Vec<usize>) {
    let (mut lo, mut hi) = (Vec::new(), Vec::new());
    for &t in rows {
        if t % 2 == p {
            lo.push((t - p) / 2);
        } else {
            hi.push((t - (1 - p)) / 2);
        }
    }
    (lo, hi)
}

fn quantised_rows(plane: &Plane, rows: &[usize], qi: u32) -> Vec<Vec<i32>> {
    rows.iter().map(|&r| plane.row(r).unwrap().iter().map(|&v| quantize(v, qi)).collect()).collect()
}

fn encode_lines(lines: &[Vec<i32>]) -> Vec<u8> {
    let mut w = BitWriter::default();
    for l in lines {
        vld_encode_line(l, l.len(), &mut w);
    }
    w.finish()
}

fn pad16(v: &mut Vec<u8>) {
    v.resize(v.len().next_multiple_of(16), 0);
}

fn u24(v: usize) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// One stream: header word, index table, data area. `halves[i]` = (bytes, qi nibbles of the TU).
fn build_stream(halves: &[(Vec<u8>, Vec<u32>)]) -> Vec<u8> {
    let (mut idx, mut data) = (Vec::new(), Vec::new());
    for pair in halves.chunks(2) {
        let mut nib: Vec<u32> = Vec::new();
        for (bytes, qi) in pair {
            nib.extend((0..4).rev().map(|i| (bytes.len() as u32 >> (4 * i)) & 15));
            nib.extend(qi);
            data.extend_from_slice(bytes);
        }
        idx.extend(nib.chunks(2).map(|c| (c[0] << 4 | c[1]) as u8));
    }
    pad16(&mut idx);
    pad16(&mut data);
    let mut out = (idx.len() as u16 / 16).to_be_bytes().to_vec();
    out.extend(u24(data.len() / 16));
    out.extend([0x40, 0x40, 0x01, 0x12, 0x10, 0, 0, 0, 0, 0, 0]);
    out.extend(idx);
    out.extend(data);
    out
}

/// Encode one tile: forward 3-level 5/3 per component with the phases of `s`, quantise, pack every half in natural
/// clipped order (*Frame geometry*), then add index tables, stream headers and the tile header.
pub(crate) fn encode_tile(t: &TileSpec) -> Vec<u8> {
    let (vs, s) = (t.m.height, i32::from(t.s));
    let ntu = crate::arw6::tu_count(vs, t.s);
    let p1 = s % 2;
    let p2 = ((2 + s - p1) / 2) % 2;
    let k3 = ((6 + s) % 8 - p1) / 2;
    let p3 = ((k3 - p2) / 2).rem_euclid(2);
    let (up1, up2, up3) = (p1 as usize, p2 as usize, p3 as usize);
    let comps = [&t.m, &t.c1, &t.c2];
    let bands: Vec<_> = comps.iter().map(|p| forward3(p, [p1 as u8, p2 as u8, p3 as u8])).collect();
    let qi = &t.qi;
    // streams[group][comp] -> halves
    let mut streams: Vec<Vec<Vec<(Vec<u8>, Vec<u32>)>>> =
        vec![vec![Vec::new(); 3], vec![Vec::new(); 3], vec![Vec::new(); 3], vec![Vec::new(); 3], vec![Vec::new()]];
    for n in 0..2 * ntu as i32 {
        let rows: Vec<usize> = (8 * n - 6 + s..8 * n + 2 + s).filter(|&r| r >= 0 && (r as usize) < vs).map(|r| r as usize).collect();
        let (low1, high1) = split_rows(&rows, up1);
        let (low2, high2) = split_rows(&low1, up2);
        let (low3, high3) = split_rows(&low2, up3);
        for (c, b) in bands.iter().enumerate() {
            let cat = |parts: [Vec<Vec<i32>>; 3]| parts.into_iter().flatten().collect::<Vec<_>>();
            let dpcm = {
                let mut ll = quantised_rows(&b.ll3, &low3, 0);
                for l in &mut ll {
                    let mut prev = if c == 0 { 2048 } else { 0 };
                    for v in l.iter_mut() {
                        (*v, prev) = (*v - prev, *v);
                    }
                }
                ll
            };
            let sets: [(usize, Vec<Vec<i32>>, Vec<u32>); 4] = [
                (0, dpcm, vec![0]),
                (
                    1,
                    cat([
                        quantised_rows(&b.hl3, &low3, qi.l3[c][0]),
                        quantised_rows(&b.lh3, &high3, qi.l3[c][1]),
                        quantised_rows(&b.hh3, &high3, qi.l3[c][2]),
                    ]),
                    qi.l3[c].to_vec(),
                ),
                (
                    2,
                    cat([
                        quantised_rows(&b.hl2, &low2, qi.l2[c][0]),
                        quantised_rows(&b.lh2, &high2, qi.l2[c][1]),
                        quantised_rows(&b.hh2, &high2, qi.l2[c][2]),
                    ]),
                    qi.l2[c].to_vec(),
                ),
                (
                    3,
                    cat([
                        quantised_rows(&b.hl1, &low1, qi.l1[c][0]),
                        quantised_rows(&b.lh1, &high1, qi.l1[c][1]),
                        quantised_rows(&b.hh1, &high1, qi.l1[c][2]),
                    ]),
                    qi.l1[c].to_vec(),
                ),
            ];
            for (g, lines, q) in sets {
                streams[g][c].push((encode_lines(&lines), q));
            }
        }
        streams[4][0].push((encode_lines(&quantised_rows(&t.res, &rows, qi.res)), vec![qi.res]));
    }
    let blobs: Vec<Vec<Vec<u8>>> = streams.iter().map(|g| g.iter().map(|s| build_stream(s)).collect()).collect();
    let mut head = vec![0u8; 8 * 16];
    head[..4].copy_from_slice(b"0000"); // bytes 4..8: tile index, 0
    let tw = 2 * t.m.width;
    let w0: u64 = (tw as u64) << 48 | (vs as u64) << 32 | 16 << 20 | 3 << 13 | 3 << 10 | 0b10_0000_0000;
    head[8..16].copy_from_slice(&w0.to_be_bytes());
    for (g, group) in blobs.iter().enumerate() {
        let words: Vec<usize> = group.iter().map(|b| b.len() / 16).collect();
        head[(3 + g) * 16] = words.len() as u8;
        for (i, w) in words.iter().enumerate() {
            head[(3 + g) * 16 + 1 + 3 * i..][..3].copy_from_slice(&u24(*w));
        }
        head[16 + 3 * g..][..3].copy_from_slice(&u24(words.iter().sum()));
    }
    let mut out = head;
    out.extend(blobs.into_iter().flatten().flatten());
    out.resize(out.len().next_multiple_of(4096), 0);
    out
}

/// A little-endian ARW6 TIFF: IFD0 (Sony, ILCE-7RM6) + a SubIFD with the strip = tile table, zero padding to 512,
/// then the tiles `(x, y, w, h, bytes)` back to back.
pub(crate) fn arw6_file(tiles: &[(usize, usize, usize, usize, Vec<u8>)], width: usize, height: usize) -> Vec<u8> {
    arw6_file_with(tiles, width, height, true, 14)
}

/// [`arw6_file`] with another BitsPerSample.
pub(crate) fn arw6_file_bits(tiles: &[(usize, usize, usize, usize, Vec<u8>)], width: usize, height: usize, bits: u16) -> Vec<u8> {
    arw6_file_with(tiles, width, height, true, bits)
}

/// [`arw6_file`] without the WhiteLevel tag.
pub(crate) fn arw6_file_no_white(tiles: &[(usize, usize, usize, usize, Vec<u8>)], width: usize, height: usize) -> Vec<u8> {
    arw6_file_with(tiles, width, height, false, 14)
}

fn arw6_file_with(tiles: &[(usize, usize, usize, usize, Vec<u8>)], width: usize, height: usize, white: bool, bits: u16) -> Vec<u8> {
    let mut strip = (tiles.len() as u32).to_le_bytes().to_vec();
    strip.extend([0; 4]);
    let mut offset = 512u64;
    for (x, y, w, h, bytes) in tiles {
        strip.extend(offset.to_le_bytes());
        for v in [x, y, w, h] {
            strip.extend((*v as u32).to_le_bytes());
        }
        offset += bytes.len() as u64;
    }
    strip.resize(512, 0);
    for (.., bytes) in tiles {
        strip.extend_from_slice(bytes);
    }
    let mut raw = IfdBuilder::new();
    raw.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
    raw.set(t::IMAGE_WIDTH, Value::Long(vec![width as u32]));
    raw.set(t::IMAGE_LENGTH, Value::Long(vec![height as u32]));
    raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![bits]));
    raw.set(t::COMPRESSION, Value::Short(vec![32766]));
    raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
    raw.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
    raw.set(t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 1, 2]));
    raw.set(0x7310, Value::Short(vec![512; 4]));
    if white {
        raw.set(t::WHITE_LEVEL, Value::Long(vec![15360]));
    }
    raw.set_image(ImageData::Strips { rows_per_strip: height as u32, strips: vec![strip] });
    let mut ifd0 = IfdBuilder::new();
    ifd0.set(t::MAKE, Value::Ascii("SONY".into()));
    ifd0.set(t::MODEL, Value::Ascii("ILCE-7RM6".into()));
    ifd0.set(t::ORIENTATION, Value::Short(vec![1]));
    ifd0.add_sub_ifd(raw);
    TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap()
}

/// A tile whose bands are random quantised integers `q` (level 1 wider), `v = dequant(q, qi)`, planes by
/// `reconstruct3` (so `quantize` in the encoder is exact on them); LL3 is random around 2048 (green mean) or 0 (chroma), the residual
/// `dequant(q, qi.res)`.
pub(crate) fn one_tile_quantised(w: usize, h: usize, s: u8, qi: Qis) -> (Vec<u8>, TileSpec) {
    use crate::llvc::{dequant, reconstruct3};
    let (w2, vs) = (w / 2, h / 2);
    let ph = crate::arw6::phases(s);
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let mut rnd = |span: i32, centre: i32, q: u32| -> i32 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let v = (x % (2 * span as u64 + 1)) as i32 - span;
        if centre == i32::MIN {
            v
        } else if centre == 0 {
            dequant(v, q)
        } else {
            centre + v
        }
    };
    let mut plane = |wd: usize, ht: usize, span: i32, centre: i32, q: u32| Plane {
        width: wd,
        height: ht,
        data: (0..wd * ht).map(|_| rnd(span, centre, q)).collect(),
    };
    let mut comps = Vec::new();
    for c in 0..3 {
        let (n1l, n1h) = band_rows(vs, ph[0]);
        let (n2l, n2h) = band_rows(n1l, ph[1]);
        let (n3l, n3h) = band_rows(n2l, ph[2]);
        let (w1, w2b, w3) = (w2 / 2, w2 / 4, w2 / 8);
        let (a, b, d) = (qi.l1[c], qi.l2[c], qi.l3[c]);
        let bands = Bands3 {
            ll3: plane(w3, n3l, 60, if c == 0 { 2048 } else { i32::MIN }, 0),
            hl3: plane(w3, n3l, 7, 0, d[0]),
            lh3: plane(w3, n3h, 7, 0, d[1]),
            hh3: plane(w3, n3h, 7, 0, d[2]),
            hl2: plane(w2b, n2l, 7, 0, b[0]),
            lh2: plane(w2b, n2h, 7, 0, b[1]),
            hh2: plane(w2b, n2h, 7, 0, b[2]),
            hl1: plane(w1, n1l, 15, 0, a[0]),
            lh1: plane(w1, n1h, 15, 0, a[1]),
            hh1: plane(w1, n1h, 15, 0, a[2]),
        };
        comps.push(reconstruct3(&bands, ph).unwrap());
    }
    let res = plane(w2, vs, 7, 0, qi.res);
    let (m, c1, c2) = (comps.remove(0), comps.remove(0), comps.remove(0));
    let t = TileSpec { s, qi, m, c1, c2, res };
    (encode_tile(&t), t)
}
