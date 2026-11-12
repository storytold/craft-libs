//! The Layer III decoder: ISO/IEC 11172-3 §2.4.1.7 (side information, main data), §2.4.2.7
//! (semantics) and §2.4.3.4 (decoding: Huffman, requantization, reordering, stereo, alias
//! reduction, IMDCT, frequency inversion, synthesis), with ISO/IEC 13818-3 §2.4.3.2 for the
//! scalefactors and intensity stereo of the lower sampling frequencies.

use std::f64::consts::PI;
use std::sync::OnceLock;

use craft_media::bitstream::BitReader;

use super::huffman_tables::{LINBITS, QUAD_A_CODES, QUAD_A_LENS, pair_table};
use super::tables;
use crate::mpa::{self, Error, Frame, Header, Mode, Result, Version};

/// Frequency lines of a granule.
const LINES: usize = 576;
/// The furthest back `main_data_begin` can reach: 9 bits (MPEG-1).
const MAX_BACK_MPEG1: usize = 511;
/// 8 bits for the lower sampling frequencies.
const MAX_BACK_LSF: usize = 255;
/// The scalefactor band boost of `preflag` (Table 3-B.6).
const PRETAB: [u8; 22] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 3, 2, 0];

/// Short-block scalefactor band widths (Table 3-B.8 of 11172-3, Table B.2 of 13818-3) at 44.1,
/// 48, 32, 22.05, 24, 16 (also 11.025 and 12) and 8 kHz.
const SHORT_WIDTHS: [[u16; 13]; 7] = [
    [4, 4, 4, 4, 6, 8, 10, 12, 14, 18, 22, 30, 56],
    [4, 4, 4, 4, 6, 6, 10, 12, 14, 16, 20, 26, 66],
    [4, 4, 4, 4, 6, 8, 12, 16, 20, 26, 34, 42, 12],
    [4, 4, 4, 6, 6, 8, 10, 14, 18, 26, 32, 42, 18],
    [4, 4, 4, 6, 8, 10, 12, 14, 18, 24, 32, 44, 12],
    [4, 4, 4, 6, 8, 10, 12, 14, 18, 24, 30, 40, 18],
    [8, 8, 8, 12, 16, 20, 24, 28, 36, 2, 2, 2, 26],
];
/// Long-block band boundaries at 8 kHz (MPEG-2.5); 11.025 and 12 kHz use those of 16 kHz.
const LONG_8000: [u16; 23] = [0, 12, 24, 36, 48, 60, 72, 88, 108, 132, 160, 192, 232, 280, 336, 400, 476, 566, 568, 570, 572, 574, 576];

/// Scalefactor bands per partition for the lower sampling frequencies (13818-3 Table B.4,
/// `nr_of_sfb_block`), by table (0-2 without intensity stereo on the channel, 3-5 with) and block
/// kind (long, short, mixed). Short-block counts are of band × window entries.
const NR_OF_SFB: [[[usize; 4]; 3]; 6] = [
    [[6, 5, 5, 5], [9, 9, 9, 9], [6, 9, 9, 9]],
    [[6, 5, 7, 3], [9, 9, 12, 6], [6, 9, 12, 6]],
    [[11, 10, 0, 0], [18, 18, 0, 0], [15, 18, 0, 0]],
    [[7, 7, 7, 0], [12, 12, 12, 0], [6, 15, 12, 0]],
    [[6, 6, 6, 3], [12, 9, 9, 6], [6, 12, 9, 6]],
    [[8, 8, 5, 0], [15, 12, 9, 0], [6, 18, 9, 0]],
];

/// The scalefactor bands of a sampling frequency, as line boundaries.
struct Bands {
    long: [usize; 23],
    /// Short bands, in lines of one window (`0..=192`).
    short: [usize; 14],
}

impl Bands {
    fn of(sample_rate: u32) -> Bands {
        let (long, short): (&[u16; 23], &[u16; 13]) = match sample_rate {
            44_100 => (&tables::SFB_MPEG1[0], &SHORT_WIDTHS[0]),
            48_000 => (&tables::SFB_MPEG1[1], &SHORT_WIDTHS[1]),
            32_000 => (&tables::SFB_MPEG1[2], &SHORT_WIDTHS[2]),
            22_050 => (&tables::SFB_LSF[0], &SHORT_WIDTHS[3]),
            24_000 => (&tables::SFB_LSF[1], &SHORT_WIDTHS[4]),
            8_000 => (&LONG_8000, &SHORT_WIDTHS[6]),
            // 16, 12 and 11.025 kHz
            _ => (&tables::SFB_LSF[2], &SHORT_WIDTHS[5]),
        };
        let mut b = Bands { long: [0; 23], short: [0; 14] };
        for (o, &l) in b.long.iter_mut().zip(long) {
            *o = usize::from(l).min(LINES);
        }
        let mut at = 0;
        for (o, &w) in b.short.iter_mut().skip(1).zip(short) {
            at += usize::from(w);
            *o = at.min(LINES / 3);
        }
        b
    }

    /// Lines `start..end` of long band `sfb` (empty past the last band).
    fn long_band(&self, sfb: usize) -> (usize, usize) {
        (self.long.get(sfb).copied().unwrap_or(LINES), self.long.get(sfb + 1).copied().unwrap_or(LINES))
    }

    /// Line of the first line of short band `sfb` (counting all three windows), and the band's
    /// width in one window.
    fn short_band(&self, sfb: usize) -> (usize, usize) {
        let a = self.short.get(sfb).copied().unwrap_or(LINES / 3);
        let b = self.short.get(sfb + 1).copied().unwrap_or(LINES / 3);
        (3 * a, b.saturating_sub(a))
    }
}

/// Side information of one granule of one channel (§2.4.1.7).
#[derive(Clone, Copy, Default)]
struct Granule {
    part2_3_length: usize,
    big_values: usize,
    global_gain: i32,
    scalefac_compress: u32,
    window_switching: bool,
    /// 0 normal, 1 start, 2 short, 3 stop.
    block_type: u8,
    /// Mixed block: the lowest two subbands are long (only with block type 2).
    mixed: bool,
    table_select: [usize; 3],
    subblock_gain: [i32; 3],
    region0_count: usize,
    region1_count: usize,
    preflag: bool,
    scalefac_scale: bool,
    count1_table_b: bool,
}

impl Granule {
    fn short(&self) -> bool {
        self.block_type == 2
    }
}

/// A frame's side information.
#[derive(Default)]
struct SideInfo {
    main_data_begin: usize,
    scfsi: [[bool; 4]; 2],
    /// `[granule][channel]`.
    granules: [[Granule; 2]; 2],
}

/// Scalefactors of one granule of one channel, and the illegal intensity position of each band.
#[derive(Clone, Copy)]
struct Scalefactors {
    long: [u8; 22],
    short: [[u8; 3]; 13],
    long_illegal: [u8; 22],
    short_illegal: [[u8; 3]; 13],
}

impl Scalefactors {
    fn new(illegal: u8) -> Scalefactors {
        Scalefactors { long: [0; 22], short: [[0; 3]; 13], long_illegal: [illegal; 22], short_illegal: [[illegal; 3]; 13] }
    }

    /// Store the `k`-th scalefactor read for a granule: long bands first (up to `long_end`; all of
    /// them for long blocks), then short bands from `short_start` window by window.
    fn set(&mut self, g: &Granule, long_end: usize, k: usize, value: u8, illegal: u8) {
        let (long, short_start) = match (g.short(), g.mixed) {
            (false, _) => (usize::MAX, 0),
            (true, true) => (long_end, 3),
            (true, false) => (0, 0),
        };
        if k < long {
            if let (Some(v), Some(i)) = (self.long.get_mut(k), self.long_illegal.get_mut(k)) {
                (*v, *i) = (value, illegal);
            }
        } else {
            let s = k - long;
            let (sfb, w) = (short_start + s / 3, s % 3);
            if let (Some(v), Some(i)) =
                (self.short.get_mut(sfb).and_then(|b| b.get_mut(w)), self.short_illegal.get_mut(sfb).and_then(|b| b.get_mut(w)))
            {
                (*v, *i) = (value, illegal);
            }
        }
    }
}

/// A Huffman code as a binary tree: `nodes[i][bit]` is a child node, or a symbol when it has
/// [`LEAF`] set; 0 is no child (the root is never a child).
struct Tree(Vec<[u16; 2]>);

const LEAF: u16 = 0x8000;

impl Tree {
    /// The tree of the code `(code, length)` of each symbol in turn; `None` when two codes
    /// collide (the tables are prefix-free: a unit test checks every one builds).
    fn build(codes: impl Iterator<Item = (u32, u8)>) -> Option<Tree> {
        let mut nodes = vec![[0u16; 2]];
        for (sym, (code, len)) in codes.enumerate() {
            if len == 0 || len > 24 || sym >= usize::from(LEAF) {
                return None;
            }
            let mut n = 0usize;
            for i in (0..len).rev() {
                let bit = ((code >> i) & 1) as usize;
                let child = *nodes.get(n)?.get(bit)?;
                if i == 0 {
                    if child != 0 {
                        return None;
                    }
                    *nodes.get_mut(n)?.get_mut(bit)? = LEAF | sym as u16;
                } else if child == 0 {
                    nodes.push([0, 0]);
                    let new = u16::try_from(nodes.len() - 1).ok().filter(|&x| x < LEAF)?;
                    *nodes.get_mut(n)?.get_mut(bit)? = new;
                    n = usize::from(new);
                } else if child & LEAF != 0 {
                    return None;
                } else {
                    n = usize::from(child);
                }
            }
        }
        Some(Tree(nodes))
    }

    fn decode(&self, r: &mut BitReader) -> Result<usize> {
        let mut n = 0usize;
        // no code is longer than 19 bits
        for _ in 0..24 {
            let bit = usize::from(r.read_bit().map_err(|_| overrun())?);
            let child = self.0.get(n).and_then(|c| c.get(bit)).copied().unwrap_or(0);
            if child & LEAF != 0 {
                return Ok(usize::from(child & !LEAF));
            }
            if child == 0 {
                return Err(Error::Invalid("invalid Huffman code"));
            }
            n = usize::from(child);
        }
        Err(Error::Invalid("invalid Huffman code"))
    }
}

/// The Huffman trees: the pair tables by `table_select` (16-23 share one tree, as do 24-31) and
/// count1 table A.
struct Trees {
    pairs: Vec<Option<(Tree, usize)>>,
    quad: Option<Tree>,
}

fn trees() -> &'static Trees {
    static T: OnceLock<Trees> = OnceLock::new();
    T.get_or_init(|| {
        let pair = |select: usize| {
            let t = pair_table(select)?;
            Some((Tree::build(t.codes.iter().zip(t.lens).map(|(&c, &l)| (u32::from(c), l)))?, t.size))
        };
        let mut pairs: Vec<Option<(Tree, usize)>> = (0..16).map(pair).collect();
        pairs.push(pair(16));
        pairs.push(pair(24));
        let quad = Tree::build(QUAD_A_CODES.iter().zip(QUAD_A_LENS).map(|(&c, l)| (u32::from(c), l)));
        Trees { pairs, quad }
    })
}

impl Trees {
    fn pair(&self, select: usize) -> Option<&(Tree, usize)> {
        let i = match select {
            16..=23 => 16,
            24..=31 => 17,
            s => s,
        };
        self.pairs.get(i)?.as_ref()
    }
}

fn overrun() -> Error {
    Error::Invalid("Layer III data runs past the frame")
}

fn bits(r: &mut BitReader, n: u32) -> Result<u32> {
    if n == 0 { Ok(0) } else { r.read_bits(n).map_err(|_| overrun()) }
}

fn flag(r: &mut BitReader) -> Result<bool> {
    r.read_bit().map_err(|_| overrun())
}

/// Granules per frame: two in MPEG-1, one at the lower sampling frequencies.
fn granules(h: &Header) -> usize {
    if h.version == Version::Mpeg1 { 2 } else { 1 }
}

/// Bytes of side information (§2.4.1.7; 13818-3 §2.4.1.7).
fn side_info_len(h: &Header) -> usize {
    match (h.version == Version::Mpeg1, h.channels()) {
        (true, 1) => 17,
        (true, _) => 32,
        (false, 1) => 9,
        (false, _) => 17,
    }
}

fn side_info(r: &mut BitReader, h: &Header) -> Result<SideInfo> {
    let lsf = h.version != Version::Mpeg1;
    let nch = h.channels();
    let mut si = SideInfo { main_data_begin: bits(r, if lsf { 8 } else { 9 })? as usize, ..SideInfo::default() };
    // private bits
    bits(
        r,
        match (lsf, nch) {
            (false, 1) => 5,
            (false, _) => 3,
            (true, 1) => 1,
            (true, _) => 2,
        },
    )?;
    if !lsf {
        for ch in si.scfsi.iter_mut().take(nch) {
            for band in ch.iter_mut() {
                *band = flag(r)?;
            }
        }
    }
    for gr in si.granules.iter_mut().take(granules(h)) {
        for g in gr.iter_mut().take(nch) {
            g.part2_3_length = bits(r, 12)? as usize;
            g.big_values = bits(r, 9)? as usize;
            if g.big_values > LINES / 2 {
                return Err(Error::Invalid("big_values above 288"));
            }
            g.global_gain = bits(r, 8)? as i32;
            g.scalefac_compress = bits(r, if lsf { 9 } else { 4 })?;
            g.window_switching = flag(r)?;
            if g.window_switching {
                g.block_type = bits(r, 2)? as u8;
                if g.block_type == 0 {
                    return Err(Error::Invalid("window switching with the normal block type"));
                }
                g.mixed = flag(r)? && g.block_type == 2;
                for t in g.table_select.iter_mut().take(2) {
                    *t = bits(r, 5)? as usize;
                }
                for s in g.subblock_gain.iter_mut() {
                    *s = bits(r, 3)? as i32;
                }
            } else {
                for t in g.table_select.iter_mut() {
                    *t = bits(r, 5)? as usize;
                }
                g.region0_count = bits(r, 4)? as usize;
                g.region1_count = bits(r, 3)? as usize;
            }
            if !lsf {
                g.preflag = flag(r)?;
            }
            g.scalefac_scale = flag(r)?;
            g.count1_table_b = flag(r)?;
        }
    }
    Ok(si)
}

/// MPEG-1 scalefactors (§2.4.1.7 `part2`, §2.4.2.7): `slen1` for the low bands, `slen2` for the
/// high ones; in the second granule a long block reuses the first granule's scalefactors of the
/// band groups `scfsi` marks.
fn scalefactors_mpeg1(r: &mut BitReader, g: &Granule, scfsi: &[bool; 4], second: bool, prev: &Scalefactors) -> Result<Scalefactors> {
    let (slen1, slen2) = tables::SLEN.get(g.scalefac_compress as usize).copied().unwrap_or((0, 0));
    let mut sf = Scalefactors::new(7);
    if g.short() {
        // mixed: 8 long bands and short bands 3-11; short: bands 0-11. Bands below short band 6
        // (and the long ones) take slen1.
        let (n, first) = if g.mixed { (8 + 27, 17) } else { (36, 18) };
        for k in 0..n {
            let v = bits(r, if k < first { slen1 } else { slen2 })? as u8;
            sf.set(g, 8, k, v, 7);
        }
    } else {
        const GROUPS: [(usize, usize); 4] = [(0, 6), (6, 11), (11, 16), (16, 21)];
        for (i, &(a, b)) in GROUPS.iter().enumerate() {
            let len = if i < 2 { slen1 } else { slen2 };
            let reuse = second && scfsi.get(i).copied().unwrap_or(false);
            for band in a..b {
                let v = if reuse { prev.long.get(band).copied().unwrap_or(0) } else { bits(r, len)? as u8 };
                if let Some(s) = sf.long.get_mut(band) {
                    *s = v;
                }
            }
        }
    }
    Ok(sf)
}

/// Lower-sampling-frequency scalefactors (13818-3 §2.4.3.2): `scalefac_compress` gives four
/// partitions' lengths; on the right channel of intensity stereo it is read differently, and
/// the largest value of each band marks an illegal intensity position. Returns the scalefactors
/// and `preflag`.
fn scalefactors_lsf(r: &mut BitReader, g: &Granule, intensity_right: bool) -> Result<(Scalefactors, bool)> {
    let sfc = g.scalefac_compress;
    let (slen, table, preflag) = if intensity_right {
        let i = sfc >> 1;
        if i < 180 {
            ([i / 36, (i % 36) / 6, (i % 36) % 6, 0], 3, false)
        } else if i < 244 {
            let i = i - 180;
            ([(i & 63) >> 4, (i & 15) >> 2, i & 3, 0], 4, false)
        } else {
            let i = i - 244;
            ([i / 3, i % 3, 0, 0], 5, false)
        }
    } else if sfc < 400 {
        ([(sfc >> 4) / 5, (sfc >> 4) % 5, (sfc & 15) >> 2, sfc & 3], 0, false)
    } else if sfc < 500 {
        let s = sfc - 400;
        ([(s >> 2) / 5, (s >> 2) % 5, s & 3, 0], 1, false)
    } else {
        let s = sfc - 500;
        ([s / 3, s % 3, 0, 0], 2, true)
    };
    let kind = match (g.short(), g.mixed) {
        (false, _) => 0,
        (true, false) => 1,
        (true, true) => 2,
    };
    let counts = NR_OF_SFB.get(table).and_then(|t| t.get(kind)).copied().unwrap_or_default();
    let mut sf = Scalefactors::new(u8::MAX);
    let mut k = 0;
    for (n, len) in counts.into_iter().zip(slen) {
        // slen is at most 4 bits for every scalefac_compress
        let len = len.min(4);
        let illegal = ((1u32 << len) - 1) as u8;
        for _ in 0..n {
            let v = bits(r, len)? as u8;
            sf.set(g, 6, k, v, illegal);
            k += 1;
        }
    }
    Ok((sf, preflag))
}

/// The Huffman-coded lines of a granule (§2.4.2.7 `huffmancodebits`) into `out`, reading up to
/// bit `end` of the main data. Bits past `end` belong to the next granule: a damaged granule
/// whose big values would run past it stops there, and a count1 quadruple that crosses it is
/// dropped. Lines not coded stay 0.
fn huffman(r: &mut BitReader, end: usize, g: &Granule, bands: &Bands, out: &mut [i32; LINES]) -> Result<()> {
    let big = (g.big_values * 2).min(LINES);
    // region boundaries: with window switching, region 1 starts at short band 3 (3 × 12 = 36
    // lines, but 72 at 8 kHz) for short blocks, else at long band 8; region 2 is empty
    let (r1, r2) = if g.window_switching {
        (if g.short() { bands.short_band(3).0 } else { bands.long[8] }, LINES)
    } else {
        let a = g.region0_count + 1;
        let b = a + g.region1_count + 1;
        (bands.long.get(a).copied().unwrap_or(LINES), bands.long.get(b).copied().unwrap_or(LINES))
    };
    let r1 = r1.min(big);
    let r2 = r2.clamp(r1, big);
    let trees = trees();
    let mut i = 0;
    for (region_end, &select) in [r1, r2, big].into_iter().zip(&g.table_select) {
        if select == 0 {
            i = region_end.max(i);
            continue;
        }
        let (tree, size) = trees.pair(select).ok_or(Error::Invalid("Huffman table 4 or 14"))?;
        let linbits = LINBITS.get(select).copied().unwrap_or(0);
        while i + 2 <= region_end {
            if r.position() >= end {
                return Ok(());
            }
            let sym = tree.decode(r)?;
            let mut pair = [(sym / size) as i32, (sym % size) as i32];
            for v in pair.iter_mut() {
                if linbits > 0 && *v == 15 {
                    *v += bits(r, linbits)? as i32;
                }
                if *v != 0 && flag(r)? {
                    *v = -*v;
                }
            }
            if let Some(o) = out.get_mut(i..i + 2) {
                o.copy_from_slice(&pair);
            }
            i += 2;
        }
    }
    let quad = trees.quad.as_ref().ok_or(Error::Invalid("count1 table"))?;
    while i + 4 <= LINES && r.position() < end {
        let q = if g.count1_table_b { 15 - bits(r, 4)? as usize } else { quad.decode(r)? };
        let mut v = [(q >> 3) & 1, (q >> 2) & 1, (q >> 1) & 1, q & 1].map(|b| b as i32);
        for x in v.iter_mut() {
            if *x != 0 && flag(r)? {
                *x = -1;
            }
        }
        if r.position() > end {
            break;
        }
        if let Some(o) = out.get_mut(i..i + 4) {
            o.copy_from_slice(&v);
        }
        i += 4;
    }
    Ok(())
}

/// Requantize (§2.4.3.4.7) into `xr`, in coded order: short-block bands window by window.
fn requantize(ints: &[i32; LINES], g: &Granule, sf: &Scalefactors, preflag: bool, lsf: bool, bands: &Bands, xr: &mut [f32; LINES]) {
    let p43 = tables::pow43();
    let shift = if g.scalefac_scale { 1.0 } else { 0.5 };
    let gain = 0.25 * (g.global_gain - 210) as f32;
    let value = |v: i32, exp: f32| {
        let m = p43.get(v.unsigned_abs() as usize).copied().unwrap_or(0.0) * exp.exp2();
        if v < 0 { -m } else { m }
    };
    *xr = [0.0; LINES];
    let (long_end, short_start) = match (g.short(), g.mixed) {
        (false, _) => (22, 13),
        (true, true) => (if lsf { 6 } else { 8 }, 3),
        (true, false) => (0, 0),
    };
    for sfb in 0..long_end {
        let (a, b) = bands.long_band(sfb);
        let boost = if preflag { PRETAB.get(sfb).copied().unwrap_or(0) } else { 0 };
        let exp = gain - shift * f32::from(sf.long.get(sfb).copied().unwrap_or(0) + boost);
        for (x, &q) in xr.get_mut(a..b).unwrap_or_default().iter_mut().zip(ints.get(a..b).unwrap_or_default()) {
            *x = value(q, exp);
        }
    }
    if !g.short() {
        return;
    }
    for sfb in short_start..13 {
        let (start, width) = bands.short_band(sfb);
        for w in 0..3 {
            let scf = sf.short.get(sfb).and_then(|s| s.get(w)).copied().unwrap_or(0);
            let exp = gain - 2.0 * g.subblock_gain[w] as f32 - shift * f32::from(scf);
            let at = start + w * width;
            for (x, &q) in xr.get_mut(at..at + width).unwrap_or_default().iter_mut().zip(ints.get(at..at + width).unwrap_or_default()) {
                *x = value(q, exp);
            }
        }
    }
}

/// Reorder a short block's lines (§2.4.3.4.8) from window by window within each band to the
/// IMDCT's order: each subband's 18 lines become 6 frequencies × 3 windows. Mixed blocks keep
/// their long bands as they are.
fn reorder(xr: &mut [f32; LINES], g: &Granule, bands: &Bands) {
    if !g.short() {
        return;
    }
    let coded = *xr;
    for sfb in if g.mixed { 3 } else { 0 }..13 {
        let (start, width) = bands.short_band(sfb);
        for w in 0..3 {
            for i in 0..width {
                if let (Some(&v), Some(x)) = (coded.get(start + w * width + i), xr.get_mut(start + 3 * i + w)) {
                    *x = v;
                }
            }
        }
    }
}
/// Intensity-stereo ratios `(left, right)` for MPEG-1 positions 0-6: `tan(pos·π/12)` split as
/// `(r / (1 + r), 1 / (1 + r))` (§2.4.3.4.9.3).
fn is_ratio_mpeg1(pos: u8) -> (f32, f32) {
    let a = f64::from(pos) * PI / 12.0;
    let (s, c) = a.sin_cos();
    ((s / (s + c)) as f32, (c / (s + c)) as f32)
}

/// Lower-sampling-frequency intensity ratios (13818-3 §2.4.3.2).
fn is_ratio_lsf(pos: u8, intensity_scale: bool) -> (f32, f32) {
    let io: f32 = if intensity_scale { 0.5f32.sqrt() } else { 0.5f32.powf(0.25) };
    match pos {
        0 => (1.0, 1.0),
        p if p % 2 == 1 => (io.powi(i32::from(p).div_euclid(2) + 1), 1.0),
        p => (1.0, io.powi(i32::from(p) / 2)),
    }
}

/// Joint stereo (§2.4.3.4.9): intensity stereo above the right channel's last non-zero band
/// (per window for short blocks), mid/side elsewhere. The right channel's block layout decides
/// the bands, as both channels must share it. The lines are in coded order (before
/// [`reorder`]), as ffmpeg and libmad process them: where the channels' layouts differ, against
/// the specification, each channel is then reordered by its own.
fn stereo(xr: &mut [[f32; LINES]; 2], mode_extension: u8, g: &Granule, sf: &Scalefactors, lsf: bool, bands: &Bands) {
    let ms = mode_extension & 2 != 0;
    let intensity = mode_extension & 1 != 0;
    // per line: the intensity ratios, when the line is in an intensity band with a legal position
    let mut ratios: Vec<Option<(f32, f32)>> = vec![None; LINES];
    if intensity {
        let ratio = |pos: u8, illegal: u8| -> Option<(f32, f32)> {
            if pos >= illegal {
                None
            } else if lsf {
                Some(is_ratio_lsf(pos, g.scalefac_compress & 1 == 1))
            } else {
                Some(is_ratio_mpeg1(pos))
            }
        };
        let right = &xr[1];
        let mut set = |line: usize, r: Option<(f32, f32)>| {
            if let Some(x) = ratios.get_mut(line) {
                *x = r;
            }
        };
        let long_end = match (g.short(), g.mixed) {
            (false, _) => 22,
            (true, true) => {
                if lsf {
                    6
                } else {
                    8
                }
            }
            (true, false) => 0,
        };
        let mut nonzero_short = false;
        if g.short() {
            let short_start = if g.mixed { 3 } else { 0 };
            for w in 0..3 {
                for sfb in (short_start..13).rev() {
                    let (start, width) = bands.short_band(sfb);
                    let lines = (0..width).map(|i| start + w * width + i);
                    if lines.clone().any(|l| right.get(l).is_some_and(|&v| v != 0.0)) {
                        nonzero_short = true;
                        break;
                    }
                    let k = sfb.min(11);
                    let pos = sf.short.get(k).and_then(|s| s.get(w)).copied().unwrap_or(0);
                    let illegal = sf.short_illegal.get(k).and_then(|s| s.get(w)).copied().unwrap_or(0);
                    let r = ratio(pos, illegal);
                    lines.for_each(|l| set(l, r));
                }
            }
        }
        if !nonzero_short {
            for sfb in (0..long_end).rev() {
                let (a, b) = bands.long_band(sfb);
                if right.get(a..b).unwrap_or_default().iter().any(|&v| v != 0.0) {
                    break;
                }
                let k = sfb.min(20);
                let r = ratio(sf.long.get(k).copied().unwrap_or(0), sf.long_illegal.get(k).copied().unwrap_or(0));
                (a..b).for_each(|l| set(l, r));
            }
        }
    }
    let [left, right] = xr;
    let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
    for ((l, r), ratio) in left.iter_mut().zip(right.iter_mut()).zip(&ratios) {
        match ratio {
            Some((kl, kr)) => {
                let v = *l;
                *l = v * kl;
                *r = v * kr;
            }
            None if ms => {
                let (m, s) = (*l, *r);
                *l = (m + s) * inv_sqrt2;
                *r = (m - s) * inv_sqrt2;
            }
            None => {}
        }
    }
}

/// Alias reduction butterflies (§2.4.3.4.10.1) between the long-block subbands.
fn antialias(xr: &mut [f32; LINES], g: &Granule) {
    let boundaries = match (g.short(), g.mixed) {
        (false, _) => 31,
        (true, true) => 1,
        (true, false) => 0,
    };
    let coefficients = tables::alias_coefficients();
    for sb in 1..=boundaries {
        for (i, &(cs, ca)) in coefficients.iter().enumerate() {
            let (up, dn) = (18 * sb - 1 - i, 18 * sb + i);
            if let (Some(&a), Some(&b)) = (xr.get(up), xr.get(dn)) {
                xr[up] = a * cs - b * ca;
                xr[dn] = b * cs + a * ca;
            }
        }
    }
}

/// IMDCT kernels and windows (§2.4.3.4.10.2).
struct Imdct {
    /// `cos(π/72 · (2i + 1 + 18)(2k + 1))`, `[i][k]`.
    long: [[f32; 18]; 36],
    /// The windows of block types 0, 1 and 3 (index 2 unused).
    windows: [[f32; 36]; 4],
    /// `cos(π/24 · (2i + 1 + 6)(2k + 1))`, `[i][k]`.
    short: [[f32; 6]; 12],
    short_window: [f32; 12],
}

fn imdct() -> &'static Imdct {
    static I: OnceLock<Imdct> = OnceLock::new();
    I.get_or_init(|| {
        let mut t = Imdct { long: [[0.0; 18]; 36], windows: [[0.0; 36]; 4], short: [[0.0; 6]; 12], short_window: [0.0; 12] };
        for (i, row) in t.long.iter_mut().enumerate() {
            for (k, c) in row.iter_mut().enumerate() {
                *c = (PI / 72.0 * (2 * i + 1 + 18) as f64 * (2 * k + 1) as f64).cos() as f32;
            }
        }
        for (i, row) in t.short.iter_mut().enumerate() {
            for (k, c) in row.iter_mut().enumerate() {
                *c = (PI / 24.0 * (2 * i + 1 + 6) as f64 * (2 * k + 1) as f64).cos() as f32;
            }
        }
        let sin36 = |i: usize| (PI / 36.0 * (i as f64 + 0.5)).sin() as f32;
        let sin12 = |i: usize| (PI / 12.0 * (i as f64 + 0.5)).sin() as f32;
        for i in 0..36 {
            t.windows[0][i] = sin36(i);
            t.windows[1][i] = match i {
                0..18 => sin36(i),
                18..24 => 1.0,
                24..30 => sin12(i - 18),
                _ => 0.0,
            };
            t.windows[3][i] = match i {
                0..6 => 0.0,
                6..12 => sin12(i - 6),
                12..18 => 1.0,
                _ => sin36(i),
            };
        }
        for (i, w) in t.short_window.iter_mut().enumerate() {
            *w = sin12(i);
        }
        t
    })
}

/// The 36 windowed IMDCT outputs of one subband's 18 lines.
fn imdct_subband(x: &[f32; 18], short: bool, block_type: u8, out: &mut [f32; 36]) {
    let t = imdct();
    *out = [0.0; 36];
    if short {
        for w in 0..3 {
            for (i, (row, win)) in t.short.iter().zip(&t.short_window).enumerate() {
                let s: f32 = row.iter().enumerate().map(|(k, c)| x[3 * k + w] * c).sum();
                out[6 + 6 * w + i] += s * win;
            }
        }
    } else {
        let win = &t.windows[usize::from(block_type & 3)];
        for ((o, row), w) in out.iter_mut().zip(&t.long).zip(win) {
            let s: f32 = row.iter().zip(x).map(|(c, v)| c * v).sum();
            *o = s * w;
        }
    }
}

/// What a Xing / Info or VBRI frame at the start of a stream says. That frame carries no
/// audio: a decoder skips it.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InfoTag {
    /// Audio frames in the stream, the tag's frame not counted, when the tag says.
    pub frames: Option<u32>,
    /// The LAME tag's encoder delay and padding, in samples, when there is one: the stream's
    /// audio starts `delay + 529` samples into the decoded output (529 being the decoder's own
    /// delay) and the last `padding - 529` decoded samples are not audio.
    pub delay_padding: Option<(u32, u32)>,
}

impl InfoTag {
    /// The tag in the Layer III frame at the start of `frame`, if it is a Xing / Info or VBRI
    /// frame.
    pub fn parse(frame: &[u8]) -> Option<InfoTag> {
        let h = mpa::parse_header(frame).ok().filter(|h| h.layer == 3)?;
        let frame = frame.get(..h.frame_bytes.max(4)).unwrap_or(frame);
        let be32 = |at: usize| frame.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes);
        if frame.get(36..40) == Some(b"VBRI") {
            // VBRI: version (2), delay (2), quality (2), bytes (4), frames (4)
            return Some(InfoTag { frames: be32(36 + 14), delay_padding: None });
        }
        let at = 4 + side_info_len(&h);
        let id = frame.get(at..at + 4)?;
        if id != b"Xing" && id != b"Info" {
            return None;
        }
        let flags = be32(at + 4)?;
        let mut p = at + 8;
        let mut tag = InfoTag::default();
        if flags & 1 != 0 {
            tag.frames = be32(p);
            p += 4;
        }
        if flags & 2 != 0 {
            p += 4;
        }
        if flags & 4 != 0 {
            p += 100;
        }
        if flags & 8 != 0 {
            p += 4;
        }
        // the LAME extension: a 9-byte encoder string, then delay and padding 12 bits each at
        // byte 21
        let encoder = frame.get(p..p + 4);
        if matches!(encoder, Some(b"LAME" | b"Lavf" | b"Lavc"))
            && let Some(&[a, b, c]) = frame.get(p + 21..p + 24)
        {
            let delay = (u32::from(a) << 4) | (u32::from(b) >> 4);
            let padding = (u32::from(b & 15) << 8) | u32::from(c);
            tag.delay_padding = Some((delay, padding));
        }
        Some(tag)
    }
}

/// A Layer III (MP3) decoder: MPEG-1, MPEG-2 lower sampling frequencies and MPEG-2.5; mono,
/// stereo, dual channel and joint stereo (mid/side and intensity); long, short and mixed blocks.
/// Output is planar f32 at full scale ±1, 1152 samples per channel per MPEG-1 frame and 576 per
/// MPEG-2 / 2.5 frame.
///
/// Frames share the bit reservoir and the filter banks' state, so feed them in order and call
/// [`Decoder::reset`] before decoding from another position. A frame whose main data starts in
/// frames this decoder has not seen (the first frames after a seek) decodes as silence. Errors
/// are [`mpa::Error`](crate::mpa::Error), as for the other MPEG audio layers; a damaged frame
/// still updates the reservoir, so the frames after it decode.
///
/// Not decoded: the free format (bit rate index 0) and the CRC (read, not checked). A Xing / Info
/// frame decodes like any other (to silence); [`InfoTag::parse`] tells it apart.
///
/// ```
/// use craft_audio::mp3::Decoder;
///
/// // three silent frames: MPEG-1 Layer III, 128 kbit/s, 44.1 kHz, stereo
/// let mut frame = vec![0u8; 417];
/// frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
/// let stream = frame.repeat(3);
///
/// let mut dec = Decoder::new();
/// let mut left = Vec::new();
/// let mut at = 0;
/// while at < stream.len() {
///     let frame = dec.decode(&stream[at..])?;
///     at += frame.header.frame_bytes;
///     left.extend_from_slice(&frame.channels[0]);
/// }
/// assert_eq!(left.len(), 3 * 1152);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Decoder {
    /// Main data of the frames before, for `main_data_begin` (at most 511 bytes).
    reservoir: Vec<u8>,
    /// The second half of each subband's last IMDCT output per channel, `[ch][sb * 18 + i]`.
    overlap: Box<[[f32; LINES]; 2]>,
    /// The synthesis filter's `V` ring per channel and where it starts (see `mpa`).
    v: Box<[[f32; 1024]; 2]>,
    pos: [usize; 2],
    /// The last frame's header, for [`Decoder::preroll`].
    last: Option<Header>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// A decoder with an empty reservoir and filter state.
    pub fn new() -> Self {
        Decoder { reservoir: Vec::new(), overlap: Box::new([[0.0; LINES]; 2]), v: Box::new([[0.0; 1024]; 2]), pos: [0; 2], last: None }
    }

    /// Forget the reservoir and the filter state.
    pub fn reset(&mut self) {
        self.reservoir.clear();
        *self.overlap = [[0.0; LINES]; 2];
        *self.v = [[0.0; 1024]; 2];
        self.pos = [0; 2];
    }

    /// Samples per channel to decode before a seek target, so that its frame's main data and
    /// the filter state are complete: enough of the smallest frames the stream can hold to span
    /// the furthest reach of `main_data_begin`, and two more for the IMDCT overlap and synthesis
    /// filter. A VBR stream's frames before the target may be of the lowest bit rate whatever the
    /// last frame was; the version, sampling frequency, channel mode and CRC do not change within
    /// a stream. `None` before the first frame.
    pub fn preroll(&self) -> Option<usize> {
        let h = self.last?;
        // the lowest bit rate: 32 kbit/s in MPEG-1, 8 kbit/s at the lower sampling frequencies
        let (slots, kbps, back) = if h.version == Version::Mpeg1 { (144, 32, MAX_BACK_MPEG1) } else { (72, 8, MAX_BACK_LSF) };
        let smallest = slots * kbps * 1000 / (h.sample_rate as usize).max(1);
        let main = smallest.saturating_sub(4 + if h.crc { 2 } else { 0 } + side_info_len(&h)).max(1);
        Some((back.div_ceil(main) + 2) * h.samples())
    }

    /// Take the stream's format from `header` (its first frame) before any frame is decoded, so
    /// that [`Decoder::preroll`] knows it.
    pub(crate) fn prime(&mut self, header: Header) {
        self.last.get_or_insert(header);
    }

    /// Decode the frame at the start of `frame` (which may run on into the next frames; only
    /// [`Header::frame_bytes`](crate::mpa::Header::frame_bytes) are read).
    pub fn decode(&mut self, frame: &[u8]) -> Result<Frame> {
        let header = mpa::parse_header(frame)?;
        if header.layer != 3 {
            return Err(Error::Unsupported("not Layer III"));
        }
        if header.frame_bytes == 0 {
            return Err(Error::Unsupported("free-format bit rate"));
        }
        let data = frame.get(..header.frame_bytes).ok_or(Error::Invalid("truncated frame"))?;
        let si_start = 4 + if header.crc { 2 } else { 0 };
        let si_end = si_start + side_info_len(&header);
        let si_bytes = data.get(si_start..si_end).ok_or(Error::Invalid("frame shorter than its side information"))?;
        let main = data.get(si_end..).unwrap_or_default();
        let mut r = BitReader::new(si_bytes);
        let si = side_info(&mut r, &header);
        // The main data runs from main_data_begin bytes before this frame's into it; the bytes
        // of this frame join the reservoir whether or not it decodes.
        let begin = si.as_ref().map_or(0, |si| si.main_data_begin);
        let have = begin <= self.reservoir.len();
        let mut buf = Vec::with_capacity(begin + main.len());
        if have {
            buf.extend_from_slice(self.reservoir.get(self.reservoir.len() - begin..).unwrap_or_default());
            buf.extend_from_slice(main);
        }
        self.reservoir.extend_from_slice(main);
        let excess = self.reservoir.len().saturating_sub(MAX_BACK_MPEG1);
        self.reservoir.drain(..excess);
        let si = si?;
        self.last = Some(header);

        let nch = header.channels();
        let lsf = header.version != Version::Mpeg1;
        let bands = Bands::of(header.sample_rate);
        let joint = header.mode == Mode::JointStereo && nch == 2;
        let mut out = vec![vec![0f32; header.samples()]; nch];
        let mut r = BitReader::new(&buf);
        let mut at = 0usize;
        let mut prev = [Scalefactors::new(7); 2];
        let mut damaged = false;
        for gr in 0..granules(&header) {
            let mut xr = [[0f32; LINES]; 2];
            let mut sfs = [Scalefactors::new(7); 2];
            for ch in 0..nch {
                let g = &si.granules[gr][ch];
                let start = at;
                at += g.part2_3_length;
                if !have {
                    continue;
                }
                let end = at;
                r.seek_bits(start.min(buf.len() * 8));
                let mut ints = [0i32; LINES];
                let decoded = (|| -> Result<(Scalefactors, bool)> {
                    if end > buf.len() * 8 {
                        return Err(overrun());
                    }
                    let (sf, preflag) = if lsf {
                        scalefactors_lsf(&mut r, g, joint && ch == 1 && header.mode_extension & 1 != 0)?
                    } else {
                        (scalefactors_mpeg1(&mut r, g, &si.scfsi[ch], gr == 1, &prev[ch])?, g.preflag)
                    };
                    huffman(&mut r, end, g, &bands, &mut ints)?;
                    Ok((sf, preflag))
                })();
                match decoded {
                    Ok((sf, preflag)) => {
                        requantize(&ints, g, &sf, preflag, lsf, &bands, &mut xr[ch]);
                        sfs[ch] = sf;
                    }
                    Err(_) => damaged = true,
                }
            }
            if joint {
                stereo(&mut xr, header.mode_extension, &si.granules[gr][1], &sfs[1], lsf, &bands);
            }
            prev = sfs;
            for ch in 0..nch {
                let g = &si.granules[gr][ch];
                reorder(&mut xr[ch], g, &bands);
                antialias(&mut xr[ch], g);
                let pcm = out.get_mut(ch).and_then(|o| o.get_mut(gr * LINES..(gr + 1) * LINES));
                if let Some(pcm) = pcm {
                    self.hybrid_synthesis(ch, &xr[ch], g, pcm);
                }
            }
        }
        if damaged {
            return Err(Error::Invalid("damaged Layer III main data"));
        }
        Ok(Frame { header, channels: out })
    }

    /// IMDCT, overlap-add, frequency inversion and polyphase synthesis of one granule of one
    /// channel into its 576 samples.
    fn hybrid_synthesis(&mut self, ch: usize, xr: &[f32; LINES], g: &Granule, pcm: &mut [f32]) {
        let mut sub = [[0f32; 32]; 18];
        let overlap = &mut self.overlap[ch & 1];
        let mut y = [0f32; 36];
        for (sb, (x, prev)) in xr.as_chunks::<18>().0.iter().zip(overlap.as_chunks_mut::<18>().0).enumerate() {
            let short = g.short() && !(g.mixed && sb < 2);
            let block_type = if g.short() { 0 } else { g.block_type };
            imdct_subband(x, short, block_type, &mut y);
            for (t, (p, (&a, &b))) in prev.iter_mut().zip(y.iter().zip(&y[18..])).enumerate() {
                let v = a + *p;
                *p = b;
                sub[t][sb] = if sb % 2 == 1 && t % 2 == 1 { -v } else { v };
            }
        }
        let window = mpa::tables::window();
        let matrix = mpa::tables::matrix_t();
        let (v, pos) = (&mut self.v[ch & 1], &mut self.pos[ch & 1]);
        for (slot, o) in sub.iter().zip(pcm.as_chunks_mut::<32>().0) {
            mpa::synthesize(v, pos, matrix, window, slot, o);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A damaged granule whose big values run past its `part2_3_length` stops at its end: the
    /// bits after it are the next granule's.
    #[test]
    fn big_values_stop_at_the_granule_end() {
        // table 1 decodes each run of zero bits as (1, 1) with two positive signs: 5 bits a pair
        let data = [0u8; 200];
        let mut r = BitReader::new(&data);
        let g = Granule { big_values: 288, table_select: [1, 1, 1], region0_count: 15, region1_count: 7, ..Granule::default() };
        let mut out = [0i32; LINES];
        huffman(&mut r, 10, &g, &Bands::of(44_100), &mut out).unwrap();
        assert_eq!(out[..4], [1, 1, 1, 1]);
        assert!(out[4..].iter().all(|&v| v == 0), "lines past the granule's end were decoded");
    }

    /// The seek pre-roll covers the smallest frames the stream may hold, not the last frame's
    /// size: a 320 kbit/s frame at 44.1 kHz holds 1008 bytes of main data, a 32 kbit/s one 68,
    /// so the 511 bytes of reservoir span 8 small frames.
    #[test]
    fn preroll_assumes_the_smallest_frames() {
        let mut frame = vec![0u8; 1044];
        frame[..4].copy_from_slice(&[0xFF, 0xFB, 0xE0, 0x00]); // MPEG-1 Layer III, 320 kbit/s, 44.1 kHz, stereo
        let mut dec = Decoder::new();
        assert_eq!(dec.preroll(), None);
        dec.decode(&frame).unwrap();
        assert_eq!(dec.preroll(), Some((8 + 2) * 1152));
        // 8 kbit/s at 24 kHz, stereo: 24-byte frames with 3 bytes of main data each
        let mut dec = Decoder::new();
        dec.prime(mpa::parse_header(&[0xFF, 0xF3, 0x14, 0x00]).unwrap());
        assert_eq!(dec.preroll(), Some((85 + 2) * 576));
    }

    #[test]
    fn every_huffman_table_is_a_complete_prefix_code() {
        let t = trees();
        for select in (1..32).filter(|s| *s != 4 && *s != 14) {
            let Some(p) = pair_table(select) else { panic!("table {select}") };
            assert!(t.pair(select).is_some(), "table {select} does not build");
            // Kraft sum of exactly 1: no code is missing
            let kraft: f64 = p.lens.iter().map(|&l| 0.5f64.powi(i32::from(l))).sum();
            assert!((kraft - 1.0).abs() < 1e-12, "table {select}: {kraft}");
        }
        assert!(t.pair(4).is_none() && t.pair(14).is_none());
        assert!(t.quad.is_some());
    }

    #[test]
    fn band_tables_cover_the_granule() {
        for rate in [44_100, 48_000, 32_000, 22_050, 24_000, 16_000, 12_000, 11_025, 8_000] {
            let b = Bands::of(rate);
            assert_eq!(b.long[22], LINES, "{rate}");
            assert_eq!(b.short[13] * 3, LINES, "{rate}");
            assert!(b.long.windows(2).all(|w| w[0] < w[1]), "{rate}");
        }
    }

    #[test]
    fn intensity_ratios() {
        assert_eq!(is_ratio_mpeg1(0), (0.0, 1.0));
        let (l, r) = is_ratio_mpeg1(3);
        assert!((l - 0.5).abs() < 1e-6 && (r - 0.5).abs() < 1e-6);
        assert_eq!(is_ratio_lsf(0, false), (1.0, 1.0));
        let (l, r) = is_ratio_lsf(1, true);
        assert!((l - 0.5f32.sqrt()).abs() < 1e-6 && r == 1.0);
        let (l, r) = is_ratio_lsf(4, true);
        assert!(l == 1.0 && (r - 0.5).abs() < 1e-6);
    }

    /// Hostile frames: every header combination with random side information and main data
    /// decodes or fails without panicking.
    #[test]
    fn random_frames_do_not_panic() {
        let mut seed = 0x1234_5678_u32;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut dec = Decoder::new();
        for i in 0..20_000 {
            let mut f: Vec<u8> = (0..1500).map(|_| rnd() as u8).collect();
            f[0] = 0xFF;
            f[1] = 0xE2 | ((i as u8 & 3) << 3) | (rnd() as u8 & 1);
            let _ = dec.decode(&f);
            if i % 1000 == 0 {
                dec.reset();
            }
        }
    }
}
