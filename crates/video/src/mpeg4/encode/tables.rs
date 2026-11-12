//! Variable-length code tables of ISO/IEC 14496-2 Annex B, as `(code, length)` pairs (the code
//! in the low `length` bits, most significant bit first).

use std::sync::OnceLock;

/// Table B-6, `mcbpc` of an I-VOP macroblock, indexed by `(mb_type - 3) * 4 + cbpc`.
pub(super) const MCBPC_INTRA: [(u16, u8); 8] = [(1, 1), (1, 3), (2, 3), (3, 3), (1, 4), (1, 6), (2, 6), (3, 6)];

/// Table B-7, `mcbpc` of a P-VOP macroblock, indexed by `mb_type * 4 + cbpc`
/// (`mb_type` 0 inter, 1 inter+q, 2 inter4v, 3 intra, 4 intra+q).
pub(super) const MCBPC_INTER: [(u16, u8); 20] = [
    (1, 1),
    (3, 4),
    (2, 4),
    (5, 6),
    (3, 3),
    (7, 7),
    (6, 7),
    (5, 9),
    (2, 3),
    (5, 7),
    (4, 7),
    (5, 8),
    (3, 5),
    (4, 8),
    (3, 8),
    (3, 7),
    (4, 6),
    (4, 9),
    (3, 9),
    (2, 9),
];

/// Table B-8, `cbpy` indexed by the coded block pattern of an intra macroblock (block 0 in the
/// most significant of the four bits). An inter macroblock sends the code of the inverted pattern.
pub(super) const CBPY: [(u16, u8); 16] =
    [(3, 4), (5, 5), (4, 5), (9, 4), (3, 5), (7, 4), (2, 6), (11, 4), (2, 5), (3, 6), (5, 4), (10, 4), (4, 4), (8, 4), (6, 4), (3, 2)];

/// Table B-12, `motion_code` by magnitude 0–32; a sign bit (1 = negative) follows every code but 0.
pub(super) const MVD: [(u16, u8); 33] = [
    (1, 1),
    (1, 2),
    (1, 3),
    (1, 4),
    (3, 6),
    (5, 7),
    (4, 7),
    (3, 7),
    (11, 9),
    (10, 9),
    (9, 9),
    (17, 10),
    (16, 10),
    (15, 10),
    (14, 10),
    (13, 10),
    (12, 10),
    (11, 10),
    (10, 10),
    (9, 10),
    (8, 10),
    (7, 10),
    (6, 10),
    (5, 10),
    (4, 10),
    (7, 11),
    (6, 11),
    (5, 11),
    (4, 11),
    (3, 11),
    (2, 11),
    (3, 12),
    (2, 12),
];

/// Table B-13, `dct_dc_size_luminance` 0–12.
pub(super) const DC_SIZE_LUMA: [(u16, u8); 13] =
    [(3, 3), (3, 2), (2, 2), (2, 3), (1, 3), (1, 4), (1, 5), (1, 6), (1, 7), (1, 8), (1, 9), (1, 10), (1, 11)];

/// Table B-14, `dct_dc_size_chrominance` 0–12.
pub(super) const DC_SIZE_CHROMA: [(u16, u8); 13] =
    [(3, 2), (2, 2), (1, 2), (1, 3), (1, 4), (1, 5), (1, 6), (1, 7), (1, 8), (1, 9), (1, 10), (1, 11), (1, 12)];

/// The TCOEF escape code (`0000 011`), shared by both coefficient tables.
pub(super) const ESCAPE: (u16, u8) = (3, 7);

/// Table B-16, intra TCOEF codes, ordered by `last`, then `run`, then `level` (as
/// [`INTRA_LMAX`] lays them out).
const INTRA_VLC: [(u16, u8); 102] = [
    // last 0, run 0, levels 1–27
    (0x2, 2),
    (0x6, 3),
    (0xF, 4),
    (0xD, 5),
    (0xC, 5),
    (0x15, 6),
    (0x13, 6),
    (0x12, 6),
    (0x17, 7),
    (0x1F, 8),
    (0x1E, 8),
    (0x1D, 8),
    (0x25, 9),
    (0x24, 9),
    (0x23, 9),
    (0x21, 9),
    (0x21, 10),
    (0x20, 10),
    (0xF, 10),
    (0xE, 10),
    (0x7, 11),
    (0x6, 11),
    (0x20, 11),
    (0x21, 11),
    (0x50, 12),
    (0x51, 12),
    (0x52, 12),
    // last 0, run 1, levels 1–10
    (0xE, 4),
    (0x14, 6),
    (0x16, 7),
    (0x1C, 8),
    (0x20, 9),
    (0x1F, 9),
    (0xD, 10),
    (0x22, 11),
    (0x53, 12),
    (0x55, 12),
    // last 0, run 2, levels 1–5
    (0xB, 5),
    (0x15, 7),
    (0x1E, 9),
    (0xC, 10),
    (0x56, 12),
    // last 0, run 3, levels 1–4
    (0x11, 6),
    (0x1B, 8),
    (0x1D, 9),
    (0xB, 10),
    // last 0, runs 4–7, levels 1–3
    (0x10, 6),
    (0x22, 9),
    (0xA, 10),
    (0xD, 6),
    (0x1C, 9),
    (0x8, 10),
    (0x12, 7),
    (0x1B, 9),
    (0x54, 12),
    (0x14, 7),
    (0x1A, 9),
    (0x57, 12),
    // last 0, runs 8–9, levels 1–2
    (0x19, 8),
    (0x9, 10),
    (0x18, 8),
    (0x23, 11),
    // last 0, runs 10–14, level 1
    (0x17, 8),
    (0x19, 9),
    (0x18, 9),
    (0x7, 10),
    (0x58, 12),
    // last 1, run 0, levels 1–8
    (0x7, 4),
    (0xC, 6),
    (0x16, 8),
    (0x17, 9),
    (0x6, 10),
    (0x5, 11),
    (0x4, 11),
    (0x59, 12),
    // last 1, run 1, levels 1–3
    (0xF, 6),
    (0x16, 9),
    (0x5, 10),
    // last 1, runs 2–6, levels 1–2
    (0xE, 6),
    (0x4, 10),
    (0x11, 7),
    (0x24, 11),
    (0x10, 7),
    (0x25, 11),
    (0x13, 7),
    (0x5A, 12),
    (0x15, 8),
    (0x5B, 12),
    // last 1, runs 7–20, level 1
    (0x14, 8),
    (0x13, 8),
    (0x1A, 8),
    (0x15, 9),
    (0x14, 9),
    (0x13, 9),
    (0x12, 9),
    (0x11, 9),
    (0x26, 11),
    (0x27, 11),
    (0x5C, 12),
    (0x5D, 12),
    (0x5E, 12),
    (0x5F, 12),
];

/// The largest level of each run in [`INTRA_VLC`]: `[last][run]`, 0 past the last run.
const INTRA_LMAX: [&[u8]; 2] = [&[27, 10, 5, 4, 3, 3, 3, 3, 2, 2, 1, 1, 1, 1, 1], &[8, 3, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]];

/// Table B-17, inter TCOEF codes, in the order of [`INTER_LMAX`].
const INTER_VLC: [(u16, u8); 102] = [
    // last 0, run 0, levels 1–12
    (0x2, 2),
    (0xF, 4),
    (0x15, 6),
    (0x17, 7),
    (0x1F, 8),
    (0x25, 9),
    (0x24, 9),
    (0x21, 10),
    (0x20, 10),
    (0x7, 11),
    (0x6, 11),
    (0x20, 11),
    // last 0, run 1, levels 1–6
    (0x6, 3),
    (0x14, 6),
    (0x1E, 8),
    (0xF, 10),
    (0x21, 11),
    (0x50, 12),
    // last 0, run 2, levels 1–4
    (0xE, 4),
    (0x1D, 8),
    (0xE, 10),
    (0x51, 12),
    // last 0, runs 3–6, levels 1–3
    (0xD, 5),
    (0x23, 9),
    (0xD, 10),
    (0xC, 5),
    (0x22, 9),
    (0x52, 12),
    (0xB, 5),
    (0xC, 10),
    (0x53, 12),
    (0x13, 6),
    (0xB, 10),
    (0x54, 12),
    // last 0, runs 7–10, levels 1–2
    (0x12, 6),
    (0xA, 10),
    (0x11, 6),
    (0x9, 10),
    (0x10, 6),
    (0x8, 10),
    (0x16, 7),
    (0x55, 12),
    // last 0, runs 11–26, level 1
    (0x15, 7),
    (0x14, 7),
    (0x1C, 8),
    (0x1B, 8),
    (0x21, 9),
    (0x20, 9),
    (0x1F, 9),
    (0x1E, 9),
    (0x1D, 9),
    (0x1C, 9),
    (0x1B, 9),
    (0x1A, 9),
    (0x22, 11),
    (0x23, 11),
    (0x56, 12),
    (0x57, 12),
    // last 1, run 0, levels 1–3
    (0x7, 4),
    (0x19, 9),
    (0x5, 11),
    // last 1, run 1, levels 1–2
    (0xF, 6),
    (0x4, 11),
    // last 1, runs 2–40, level 1
    (0xE, 6),
    (0xD, 6),
    (0xC, 6),
    (0x13, 7),
    (0x12, 7),
    (0x11, 7),
    (0x10, 7),
    (0x1A, 8),
    (0x19, 8),
    (0x18, 8),
    (0x17, 8),
    (0x16, 8),
    (0x15, 8),
    (0x14, 8),
    (0x13, 8),
    (0x18, 9),
    (0x17, 9),
    (0x16, 9),
    (0x15, 9),
    (0x14, 9),
    (0x13, 9),
    (0x12, 9),
    (0x11, 9),
    (0x7, 10),
    (0x6, 10),
    (0x5, 10),
    (0x4, 10),
    (0x24, 11),
    (0x25, 11),
    (0x26, 11),
    (0x27, 11),
    (0x58, 12),
    (0x59, 12),
    (0x5A, 12),
    (0x5B, 12),
    (0x5C, 12),
    (0x5D, 12),
    (0x5E, 12),
    (0x5F, 12),
];

/// The largest level of each run in [`INTER_VLC`]: `[last][run]`.
const INTER_LMAX: [&[u8]; 2] = [
    &[12, 6, 4, 3, 3, 3, 3, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    &[3, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
];

/// A TCOEF table indexed by `(last, run, level)`, with the `LMAX` and `RMAX` of the escape modes.
pub(super) struct Tcoef {
    /// `code << 8 | length`, 0 where the table has no code; `[last][run][level]`.
    codes: [[[u32; 28]; 64]; 2],
    /// The largest level of a run (`LMAX`, 0 = none): `[last][run]`.
    lmax: [[u8; 64]; 2],
    /// The largest run of a level (`RMAX`, -1 = none): `[last][level]`.
    rmax: [[i8; 28]; 2],
}

impl Tcoef {
    fn build(vlc: &[(u16, u8); 102], lmax: [&[u8]; 2]) -> Tcoef {
        let mut t = Tcoef { codes: [[[0; 28]; 64]; 2], lmax: [[0; 64]; 2], rmax: [[-1; 28]; 2] };
        let mut codes = vlc.iter();
        for (last, runs) in lmax.iter().enumerate() {
            for (run, &max) in runs.iter().enumerate() {
                t.lmax[last][run] = max;
                for level in 1..=usize::from(max) {
                    let (code, len) = codes.next().copied().unwrap_or((0, 0));
                    t.codes[last][run][level] = u32::from(code) << 8 | u32::from(len);
                    t.rmax[last][level] = t.rmax[last][level].max(run as i8);
                }
            }
        }
        t
    }

    /// The code of `(last, run, level)` (`level` ≥ 1), if the table has one.
    pub(super) fn code(&self, last: bool, run: usize, level: u32) -> Option<(u32, u32)> {
        let row = self.codes[usize::from(last)].get(run)?;
        let c = *row.get(level as usize)?;
        (c != 0).then_some((c >> 8, c & 0xFF))
    }

    /// `LMAX(last, run)`, 0 when the table has no such run.
    pub(super) fn lmax(&self, last: bool, run: usize) -> u32 {
        self.lmax[usize::from(last)].get(run).map_or(0, |&m| u32::from(m))
    }

    /// `RMAX(last, level)`, `None` when the table has no such level.
    pub(super) fn rmax(&self, last: bool, level: u32) -> Option<usize> {
        let r = *self.rmax[usize::from(last)].get(level as usize)?;
        usize::try_from(r).ok()
    }
}

/// The intra (B-16) table.
pub(super) fn intra() -> &'static Tcoef {
    static T: OnceLock<Tcoef> = OnceLock::new();
    T.get_or_init(|| Tcoef::build(&INTRA_VLC, INTRA_LMAX))
}

/// The inter (B-17) table.
pub(super) fn inter() -> &'static Tcoef {
    static T: OnceLock<Tcoef> = OnceLock::new();
    T.get_or_init(|| Tcoef::build(&INTER_VLC, INTER_LMAX))
}

/// The zigzag scan: `ZIGZAG[i]` is the raster position of the `i`-th coefficient.
pub(super) const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43,
    36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Fails if a code is a prefix of another (each read as a bit string); returns the Kraft sum
    /// of the lengths (1 for a complete code).
    fn prefix_free(name: &str, codes: &[(u16, u8)]) -> f64 {
        for (i, &(a, la)) in codes.iter().enumerate() {
            for &(b, lb) in &codes[i + 1..] {
                let n = la.min(lb);
                assert!(a >> (la - n) != b >> (lb - n), "{name}: {a:#b}/{la} and {b:#b}/{lb} share a prefix");
            }
        }
        codes.iter().map(|&(_, l)| 0.5f64.powi(i32::from(l))).sum()
    }

    /// Both TCOEF tables use the same codes (Table B-16 reassigns B-17's), they form a prefix
    /// code with the escape, and one 9-bit code is left unused.
    #[test]
    fn tcoef_tables_are_prefix_codes() {
        let mut a = INTRA_VLC.to_vec();
        let mut b = INTER_VLC.to_vec();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
        let mut codes = INTER_VLC.to_vec();
        codes.push(ESCAPE);
        let kraft = prefix_free("tcoef", &codes);
        assert!((kraft - (1.0 - 0.5f64.powi(9))).abs() < 1e-12, "Kraft sum {kraft}");
    }

    #[test]
    fn tcoef_layout_matches_the_code_count() {
        for (name, lmax) in [("intra", INTRA_LMAX), ("inter", INTER_LMAX)] {
            let n: usize = lmax.iter().flat_map(|r| r.iter()).map(|&m| usize::from(m)).sum();
            assert_eq!(n, 102, "{name}");
        }
        let t = intra();
        assert_eq!(t.code(false, 0, 1), Some((0x2, 2)));
        assert_eq!(t.code(true, 0, 1), Some((0x7, 4)));
        assert_eq!(t.code(true, 20, 1), Some((0x5F, 12)));
        assert_eq!(t.lmax(false, 0), 27);
        assert_eq!(t.rmax(true, 1), Some(20));
        assert_eq!(t.rmax(false, 27), Some(0));
        assert_eq!(t.rmax(false, 28), None);
        let t = inter();
        assert_eq!(t.code(false, 1, 1), Some((0x6, 3)));
        assert_eq!(t.code(true, 40, 1), Some((0x5F, 12)));
        assert_eq!(t.rmax(true, 1), Some(40));
        assert_eq!(t.code(false, 27, 1), None);
    }

    #[test]
    fn small_tables_are_prefix_free() {
        let stuffed = |t: &[(u16, u8)]| [t, &[(1, 9)]].concat();
        assert!(prefix_free("mcbpc intra", &stuffed(&MCBPC_INTRA)) <= 1.0);
        assert!(prefix_free("mcbpc inter", &stuffed(&MCBPC_INTER)) <= 1.0);
        assert!(prefix_free("cbpy", &CBPY) <= 1.0);
        assert!(prefix_free("dc luma", &DC_SIZE_LUMA) <= 1.0);
        assert!(prefix_free("dc chroma", &DC_SIZE_CHROMA) <= 1.0);
        assert!(prefix_free("mvd", &MVD) <= 1.0);
        let mut seen = [false; 64];
        for &z in &ZIGZAG {
            seen[z] = true;
        }
        assert!(seen.iter().all(|&s| s));
    }
}
