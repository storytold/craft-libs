//! Variable-length code tables of ITU-T H.262 Annex B (B.1-B.4, B.9-B.15), which include those
//! of ISO/IEC 11172-2 Annex B. Codes are written as in the standard (spaces ignored) and turned
//! into two-level lookup tables once.

use std::sync::OnceLock;

use crate::mpeg2::bits::Bits;

#[derive(Clone, Copy, Default)]
struct Entry {
    /// Decoded value, or the subtable offset when `sub > 0`.
    val: i16,
    /// Code length (0: invalid code).
    len: u8,
    /// Index bits of the subtable this entry points to.
    sub: u8,
}

/// A two-level lookup table.
pub(crate) struct Vlc {
    root: u32,
    table: Vec<Entry>,
}

fn parse_code(s: &str) -> (u32, u32) {
    let mut v = 0u32;
    let mut n = 0u32;
    for c in s.chars() {
        match c {
            '0' | '1' => {
                v = (v << 1) | (c as u32 - '0' as u32);
                n += 1;
            }
            ' ' => {}
            // Codes are literals in this crate; the table tests cover them.
            _ => debug_assert!(false, "bad code {s}"),
        }
    }
    (v, n)
}

impl Vlc {
    pub fn new(root: u32, codes: &[(&str, i16)]) -> Vlc {
        let codes: Vec<(u32, u32, i16)> = codes
            .iter()
            .map(|(s, v)| {
                let (c, n) = parse_code(s);
                (c, n, *v)
            })
            .collect();
        let mut table = vec![Entry::default(); 1 << root];
        // subtable sizes per root prefix
        let mut sub_bits = vec![0u32; 1 << root];
        for &(c, n, _) in &codes {
            if n > root {
                let p = (c >> (n - root)) as usize;
                sub_bits[p] = sub_bits[p].max(n - root);
            }
        }
        for (p, &b) in sub_bits.iter().enumerate() {
            if b > 0 {
                let off = table.len();
                table.extend(std::iter::repeat_n(Entry::default(), 1 << b));
                table[p] = Entry { val: off as i16, len: 0, sub: b as u8 };
            }
        }
        for &(c, n, v) in &codes {
            if n <= root {
                let shift = root - n;
                for k in 0..(1u32 << shift) {
                    let e = &mut table[((c << shift) | k) as usize];
                    assert!(e.len == 0 && e.sub == 0, "VLC prefix clash");
                    *e = Entry { val: v, len: n as u8, sub: 0 };
                }
            } else {
                let p = (c >> (n - root)) as usize;
                let Entry { val: off, sub: b, .. } = table[p];
                let b = b as u32;
                let rest = c & ((1 << (n - root)) - 1);
                let shift = b - (n - root);
                for k in 0..(1u32 << shift) {
                    let e = &mut table[off as usize + ((rest << shift) | k) as usize];
                    assert!(e.len == 0, "VLC prefix clash");
                    *e = Entry { val: v, len: (n - root) as u8, sub: 0 };
                }
            }
        }
        Vlc { root, table }
    }

    /// Decode one code; `None` for an invalid code.
    #[inline(always)]
    pub fn decode(&self, b: &mut Bits) -> Option<i16> {
        let w = b.peek32();
        let e = self.table[(w >> (32 - self.root)) as usize];
        if e.sub == 0 {
            if e.len == 0 {
                return None;
            }
            b.skip(e.len as u32);
            return Some(e.val);
        }
        let idx = ((w << self.root) >> (32 - e.sub as u32)) as usize;
        let e2 = self.table[e.val as usize + idx];
        if e2.len == 0 {
            return None;
        }
        b.skip(self.root + e2.len as u32);
        Some(e2.val)
    }
}

/// macroblock_address_increment (B.1): 1..=33, [`MBA_ESCAPE`], [`MBA_STUFFING`].
pub(crate) const MBA_ESCAPE: i16 = 34;
pub(crate) const MBA_STUFFING: i16 = 35;
pub(crate) const MBA: &[(&str, i16)] = &[
    ("1", 1),
    ("011", 2),
    ("010", 3),
    ("0011", 4),
    ("0010", 5),
    ("0001 1", 6),
    ("0001 0", 7),
    ("0000 111", 8),
    ("0000 110", 9),
    ("0000 1011", 10),
    ("0000 1010", 11),
    ("0000 1001", 12),
    ("0000 1000", 13),
    ("0000 0111", 14),
    ("0000 0110", 15),
    ("0000 0101 11", 16),
    ("0000 0101 10", 17),
    ("0000 0101 01", 18),
    ("0000 0101 00", 19),
    ("0000 0100 11", 20),
    ("0000 0100 10", 21),
    ("0000 0100 011", 22),
    ("0000 0100 010", 23),
    ("0000 0100 001", 24),
    ("0000 0100 000", 25),
    ("0000 0011 111", 26),
    ("0000 0011 110", 27),
    ("0000 0011 101", 28),
    ("0000 0011 100", 29),
    ("0000 0011 011", 30),
    ("0000 0011 010", 31),
    ("0000 0011 001", 32),
    ("0000 0011 000", 33),
    ("0000 0001 000", MBA_ESCAPE),
    ("0000 0001 111", MBA_STUFFING),
];

/// macroblock_type flags.
pub(crate) const MB_QUANT: i16 = 1;
pub(crate) const MB_FWD: i16 = 2;
pub(crate) const MB_BWD: i16 = 4;
pub(crate) const MB_PATTERN: i16 = 8;
pub(crate) const MB_INTRA: i16 = 16;

/// B.2: I pictures.
pub(crate) const MBTYPE_I: &[(&str, i16)] = &[("1", MB_INTRA), ("01", MB_INTRA | MB_QUANT)];
/// B.3: P pictures.
pub(crate) const MBTYPE_P: &[(&str, i16)] = &[
    ("1", MB_FWD | MB_PATTERN),
    ("01", MB_PATTERN),
    ("001", MB_FWD),
    ("0001 1", MB_INTRA),
    ("0001 0", MB_QUANT | MB_FWD | MB_PATTERN),
    ("0000 1", MB_QUANT | MB_PATTERN),
    ("0000 01", MB_QUANT | MB_INTRA),
];
/// B.4: B pictures.
pub(crate) const MBTYPE_B: &[(&str, i16)] = &[
    ("10", MB_FWD | MB_BWD),
    ("11", MB_FWD | MB_BWD | MB_PATTERN),
    ("010", MB_BWD),
    ("011", MB_BWD | MB_PATTERN),
    ("0010", MB_FWD),
    ("0011", MB_FWD | MB_PATTERN),
    ("0001 1", MB_INTRA),
    ("0001 0", MB_QUANT | MB_FWD | MB_BWD | MB_PATTERN),
    ("0000 11", MB_QUANT | MB_FWD | MB_PATTERN),
    ("0000 10", MB_QUANT | MB_BWD | MB_PATTERN),
    ("0000 01", MB_QUANT | MB_INTRA),
];
/// ISO/IEC 11172-2 D pictures (DC intra-coded).
pub(crate) const MBTYPE_D: &[(&str, i16)] = &[("1", MB_INTRA)];

/// B.9: coded_block_pattern (0 only in ISO/IEC 13818-2).
pub(crate) const CBP: &[(&str, i16)] = &[
    ("111", 60),
    ("1101", 4),
    ("1100", 8),
    ("1011", 16),
    ("1010", 32),
    ("1001 1", 12),
    ("1001 0", 48),
    ("1000 1", 20),
    ("1000 0", 40),
    ("0111 1", 28),
    ("0111 0", 44),
    ("0110 1", 52),
    ("0110 0", 56),
    ("0101 1", 1),
    ("0101 0", 61),
    ("0100 1", 2),
    ("0100 0", 62),
    ("0011 11", 24),
    ("0011 10", 36),
    ("0011 01", 3),
    ("0011 00", 63),
    ("0010 111", 5),
    ("0010 110", 9),
    ("0010 101", 17),
    ("0010 100", 33),
    ("0010 011", 6),
    ("0010 010", 10),
    ("0010 001", 18),
    ("0010 000", 34),
    ("0001 1111", 7),
    ("0001 1110", 11),
    ("0001 1101", 19),
    ("0001 1100", 35),
    ("0001 1011", 13),
    ("0001 1010", 49),
    ("0001 1001", 21),
    ("0001 1000", 41),
    ("0001 0111", 14),
    ("0001 0110", 50),
    ("0001 0101", 22),
    ("0001 0100", 42),
    ("0001 0011", 15),
    ("0001 0010", 51),
    ("0001 0001", 23),
    ("0001 0000", 43),
    ("0000 1111", 25),
    ("0000 1110", 37),
    ("0000 1101", 26),
    ("0000 1100", 38),
    ("0000 1011", 29),
    ("0000 1010", 45),
    ("0000 1001", 53),
    ("0000 1000", 57),
    ("0000 0111", 30),
    ("0000 0110", 46),
    ("0000 0101", 54),
    ("0000 0100", 58),
    ("0000 0011 1", 31),
    ("0000 0011 0", 47),
    ("0000 0010 1", 55),
    ("0000 0010 0", 59),
    ("0000 0001 1", 27),
    ("0000 0001 0", 39),
    ("0000 0000 1", 0),
];

/// B.10: motion_code (-16..=16).
pub(crate) const MOTION: &[(&str, i16)] = &[
    ("0000 0011 001", -16),
    ("0000 0011 011", -15),
    ("0000 0011 101", -14),
    ("0000 0011 111", -13),
    ("0000 0100 001", -12),
    ("0000 0100 011", -11),
    ("0000 0100 11", -10),
    ("0000 0101 01", -9),
    ("0000 0101 11", -8),
    ("0000 0111", -7),
    ("0000 1001", -6),
    ("0000 1011", -5),
    ("0000 111", -4),
    ("0001 1", -3),
    ("0011", -2),
    ("011", -1),
    ("1", 0),
    ("010", 1),
    ("0010", 2),
    ("0001 0", 3),
    ("0000 110", 4),
    ("0000 1010", 5),
    ("0000 1000", 6),
    ("0000 0110", 7),
    ("0000 0101 10", 8),
    ("0000 0101 00", 9),
    ("0000 0100 10", 10),
    ("0000 0100 010", 11),
    ("0000 0100 000", 12),
    ("0000 0011 110", 13),
    ("0000 0011 100", 14),
    ("0000 0011 010", 15),
    ("0000 0011 000", 16),
];

/// B.11: dmvector.
pub(crate) const DMV: &[(&str, i16)] = &[("11", -1), ("0", 0), ("10", 1)];

/// B.12: dct_dc_size_luminance.
pub(crate) const DC_LUMA: &[(&str, i16)] = &[
    ("100", 0),
    ("00", 1),
    ("01", 2),
    ("101", 3),
    ("110", 4),
    ("1110", 5),
    ("1111 0", 6),
    ("1111 10", 7),
    ("1111 110", 8),
    ("1111 1110", 9),
    ("1111 1111 0", 10),
    ("1111 1111 1", 11),
];

/// B.13: dct_dc_size_chrominance.
pub(crate) const DC_CHROMA: &[(&str, i16)] = &[
    ("00", 0),
    ("01", 1),
    ("10", 2),
    ("110", 3),
    ("1110", 4),
    ("1111 0", 5),
    ("1111 10", 6),
    ("1111 110", 7),
    ("1111 1110", 8),
    ("1111 1111 0", 9),
    ("1111 1111 10", 10),
    ("1111 1111 11", 11),
];

/// DCT coefficient table values: `run | level << 6`, or these.
pub(crate) const DCT_EOB: i16 = -1;
pub(crate) const DCT_ESCAPE: i16 = -2;

pub(crate) const fn rl(run: i16, level: i16) -> i16 {
    run | (level << 6)
}

/// Table zero's codes for levels 12-15 at run 0 (table one has shorter ones).
pub(crate) const DCT_ZERO_LONG: &[(&str, i16)] =
    &[("0000 0000 1101 0", rl(0, 12)), ("0000 0000 1100 1", rl(0, 13)), ("0000 0000 1100 0", rl(0, 14)), ("0000 0000 1011 1", rl(0, 15))];

/// Codes from "0000 0000 1" on, common to tables zero and one (sign bit excluded).
pub(crate) const DCT_LONG: &[(&str, i16)] = &[
    ("0000 0000 1011 0", rl(1, 6)),
    ("0000 0000 1010 1", rl(1, 7)),
    ("0000 0000 1010 0", rl(2, 5)),
    ("0000 0000 1001 1", rl(3, 4)),
    ("0000 0000 1001 0", rl(5, 3)),
    ("0000 0000 1000 1", rl(9, 2)),
    ("0000 0000 1000 0", rl(10, 2)),
    ("0000 0000 1111 1", rl(22, 1)),
    ("0000 0000 1111 0", rl(23, 1)),
    ("0000 0000 1110 1", rl(24, 1)),
    ("0000 0000 1110 0", rl(25, 1)),
    ("0000 0000 1101 1", rl(26, 1)),
    ("0000 0000 0111 11", rl(0, 16)),
    ("0000 0000 0111 10", rl(0, 17)),
    ("0000 0000 0111 01", rl(0, 18)),
    ("0000 0000 0111 00", rl(0, 19)),
    ("0000 0000 0110 11", rl(0, 20)),
    ("0000 0000 0110 10", rl(0, 21)),
    ("0000 0000 0110 01", rl(0, 22)),
    ("0000 0000 0110 00", rl(0, 23)),
    ("0000 0000 0101 11", rl(0, 24)),
    ("0000 0000 0101 10", rl(0, 25)),
    ("0000 0000 0101 01", rl(0, 26)),
    ("0000 0000 0101 00", rl(0, 27)),
    ("0000 0000 0100 11", rl(0, 28)),
    ("0000 0000 0100 10", rl(0, 29)),
    ("0000 0000 0100 01", rl(0, 30)),
    ("0000 0000 0100 00", rl(0, 31)),
    ("0000 0000 0011 000", rl(0, 32)),
    ("0000 0000 0010 111", rl(0, 33)),
    ("0000 0000 0010 110", rl(0, 34)),
    ("0000 0000 0010 101", rl(0, 35)),
    ("0000 0000 0010 100", rl(0, 36)),
    ("0000 0000 0010 011", rl(0, 37)),
    ("0000 0000 0010 010", rl(0, 38)),
    ("0000 0000 0010 001", rl(0, 39)),
    ("0000 0000 0010 000", rl(0, 40)),
    ("0000 0000 0011 111", rl(1, 8)),
    ("0000 0000 0011 110", rl(1, 9)),
    ("0000 0000 0011 101", rl(1, 10)),
    ("0000 0000 0011 100", rl(1, 11)),
    ("0000 0000 0011 011", rl(1, 12)),
    ("0000 0000 0011 010", rl(1, 13)),
    ("0000 0000 0011 001", rl(1, 14)),
    ("0000 0000 0001 0011", rl(1, 15)),
    ("0000 0000 0001 0010", rl(1, 16)),
    ("0000 0000 0001 0001", rl(1, 17)),
    ("0000 0000 0001 0000", rl(1, 18)),
    ("0000 0000 0001 0100", rl(6, 3)),
    ("0000 0000 0001 1010", rl(11, 2)),
    ("0000 0000 0001 1001", rl(12, 2)),
    ("0000 0000 0001 1000", rl(13, 2)),
    ("0000 0000 0001 0111", rl(14, 2)),
    ("0000 0000 0001 0110", rl(15, 2)),
    ("0000 0000 0001 0101", rl(16, 2)),
    ("0000 0000 0001 1111", rl(27, 1)),
    ("0000 0000 0001 1110", rl(28, 1)),
    ("0000 0000 0001 1101", rl(29, 1)),
    ("0000 0000 0001 1100", rl(30, 1)),
    ("0000 0000 0001 1011", rl(31, 1)),
];

/// "0000 0001 xxxx" codes of both tables (table one replaces the six marked `*` with shorter ones).
pub(crate) const DCT_MID_SHARED: &[(&str, i16)] = &[
    ("0000 0001 1100", rl(3, 3)),
    ("0000 0001 0010", rl(4, 3)),
    ("0000 0001 1110", rl(6, 2)),
    ("0000 0001 0101", rl(7, 2)),
    ("0000 0001 0001", rl(8, 2)),
    ("0000 0001 1111", rl(17, 1)),
    ("0000 0001 1010", rl(18, 1)),
    ("0000 0001 1001", rl(19, 1)),
    ("0000 0001 0111", rl(20, 1)),
    ("0000 0001 0110", rl(21, 1)),
];

/// B.14: DCT coefficients table zero (the "1s" first-coefficient code is handled by the caller).
pub(crate) const DCT_ZERO: &[(&str, i16)] = &[
    ("10", DCT_EOB),
    ("11", rl(0, 1)),
    ("011", rl(1, 1)),
    ("0100", rl(0, 2)),
    ("0101", rl(2, 1)),
    ("0010 1", rl(0, 3)),
    ("0011 1", rl(3, 1)),
    ("0011 0", rl(4, 1)),
    ("0001 10", rl(1, 2)),
    ("0001 11", rl(5, 1)),
    ("0001 01", rl(6, 1)),
    ("0001 00", rl(7, 1)),
    ("0000 110", rl(0, 4)),
    ("0000 100", rl(2, 2)),
    ("0000 111", rl(8, 1)),
    ("0000 101", rl(9, 1)),
    ("0000 01", DCT_ESCAPE),
    ("0010 0110", rl(0, 5)),
    ("0010 0001", rl(0, 6)),
    ("0010 0101", rl(1, 3)),
    ("0010 0100", rl(3, 2)),
    ("0010 0111", rl(10, 1)),
    ("0010 0011", rl(11, 1)),
    ("0010 0010", rl(12, 1)),
    ("0010 0000", rl(13, 1)),
    ("0000 0010 10", rl(0, 7)),
    ("0000 0011 00", rl(1, 4)),
    ("0000 0010 11", rl(2, 3)),
    ("0000 0011 11", rl(4, 2)),
    ("0000 0010 01", rl(5, 2)),
    ("0000 0011 10", rl(14, 1)),
    ("0000 0011 01", rl(15, 1)),
    ("0000 0010 00", rl(16, 1)),
    ("0000 0001 1101", rl(0, 8)),
    ("0000 0001 1000", rl(0, 9)),
    ("0000 0001 0011", rl(0, 10)),
    ("0000 0001 0000", rl(0, 11)),
    ("0000 0001 1011", rl(1, 5)),
    ("0000 0001 0100", rl(2, 4)),
];

/// B.15: DCT coefficients table one (intra blocks with intra_vlc_format = 1).
pub(crate) const DCT_ONE: &[(&str, i16)] = &[
    ("0110", DCT_EOB),
    ("10", rl(0, 1)),
    ("010", rl(1, 1)),
    ("110", rl(0, 2)),
    ("0010 1", rl(2, 1)),
    ("0111", rl(0, 3)),
    ("0011 1", rl(3, 1)),
    ("0001 10", rl(4, 1)),
    ("0011 0", rl(1, 2)),
    ("0001 11", rl(5, 1)),
    ("0000 110", rl(6, 1)),
    ("0000 100", rl(7, 1)),
    ("1110 0", rl(0, 4)),
    ("0000 111", rl(2, 2)),
    ("0000 101", rl(8, 1)),
    ("1111 000", rl(9, 1)),
    ("0000 01", DCT_ESCAPE),
    ("1110 1", rl(0, 5)),
    ("0001 01", rl(0, 6)),
    ("1111 001", rl(1, 3)),
    ("0010 0110", rl(3, 2)),
    ("1111 010", rl(10, 1)),
    ("0010 0001", rl(11, 1)),
    ("0010 0101", rl(12, 1)),
    ("0010 0100", rl(13, 1)),
    ("0001 00", rl(0, 7)),
    ("0010 0111", rl(1, 4)),
    ("1111 1100", rl(2, 3)),
    ("1111 1101", rl(4, 2)),
    ("0000 0010 0", rl(5, 2)),
    ("0000 0010 1", rl(14, 1)),
    ("0000 0011 1", rl(15, 1)),
    ("0000 0011 01", rl(16, 1)),
    ("1111 011", rl(0, 8)),
    ("1111 100", rl(0, 9)),
    ("0010 0011", rl(0, 10)),
    ("0010 0010", rl(0, 11)),
    ("0010 0000", rl(1, 5)),
    ("0000 0011 00", rl(2, 4)),
    ("1111 1010", rl(0, 12)),
    ("1111 1011", rl(0, 13)),
    ("1111 1110", rl(0, 14)),
    ("1111 1111", rl(0, 15)),
];

pub(crate) struct Tables {
    pub mba: Vlc,
    pub mbtype: [Vlc; 4],
    pub cbp: Vlc,
    pub motion: Vlc,
    pub dmv: Vlc,
    pub dc_luma: Vlc,
    pub dc_chroma: Vlc,
    pub dct_zero: Vlc,
    pub dct_one: Vlc,
}

/// A complete DCT coefficient table: its own short codes plus the shared long ones.
pub(crate) fn dct_table(short: &'static [(&'static str, i16)], extra: &'static [(&'static str, i16)]) -> Vec<(&'static str, i16)> {
    let mut v = short.to_vec();
    v.extend_from_slice(extra);
    v.extend_from_slice(DCT_MID_SHARED);
    v.extend_from_slice(DCT_LONG);
    v
}

pub(crate) fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| Tables {
        mba: Vlc::new(8, MBA),
        mbtype: [Vlc::new(2, MBTYPE_I), Vlc::new(6, MBTYPE_P), Vlc::new(6, MBTYPE_B), Vlc::new(1, MBTYPE_D)],
        cbp: Vlc::new(9, CBP),
        motion: Vlc::new(8, MOTION),
        dmv: Vlc::new(2, DMV),
        dc_luma: Vlc::new(9, DC_LUMA),
        dc_chroma: Vlc::new(10, DC_CHROMA),
        dct_zero: Vlc::new(9, &dct_table(DCT_ZERO, DCT_ZERO_LONG)),
        dct_one: Vlc::new(9, &dct_table(DCT_ONE, &[])),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kraft sum of a code list (2^-len summed).
    fn kraft(codes: &[(&str, i16)]) -> f64 {
        codes.iter().map(|(c, _)| 0.5f64.powi(parse_code(c).1 as i32)).sum()
    }

    #[test]
    fn tables_build_without_prefix_clashes() {
        let _ = tables();
    }

    #[test]
    fn kraft_sums_match_the_reserved_codes() {
        // B.1: "0000 0000", "0000 0010" and "0000 0001 001/010/011/100/101/110" are unused
        let unused = 2.0 * 0.5f64.powi(8) + 6.0 * 0.5f64.powi(11);
        assert!((kraft(MBA) + unused - 1.0).abs() < 1e-12);
        assert!((kraft(MBTYPE_I) - 0.75).abs() < 1e-12, "01 is the last I code: 00 is forbidden");
        assert!((kraft(MBTYPE_P) + 0.5f64.powi(6) - 1.0).abs() < 1e-12);
        assert!((kraft(MBTYPE_B) + 0.5f64.powi(6) - 1.0).abs() < 1e-12);
        // B.9: only "0000 0000 0" is forbidden
        assert!((kraft(CBP) + 0.5f64.powi(9) - 1.0).abs() < 1e-12);
        // B.10: "0000 0000", "0000 0001" and "0000 0010" are unused
        assert!((kraft(MOTION) + 3.0 * 0.5f64.powi(8) - 1.0).abs() < 1e-12);
        assert!((kraft(DMV) - 1.0).abs() < 1e-12);
        assert!((kraft(DC_LUMA) - 1.0).abs() < 1e-12);
        assert!((kraft(DC_CHROMA) - 1.0).abs() < 1e-12);
        // table zero is complete except "0000 0000 0000 xxxx"
        let z = kraft(&dct_table(DCT_ZERO, DCT_ZERO_LONG));
        assert!((z + 0.5f64.powi(12) - 1.0).abs() < 1e-12, "{z}");
        // table one leaves six 12-bit and four 13-bit codes of table zero unused
        let o = kraft(&dct_table(DCT_ONE, &[]));
        assert!((o + 0.5f64.powi(12) + 6.0 * 0.5f64.powi(12) + 4.0 * 0.5f64.powi(13) - 1.0).abs() < 1e-12, "{o}");
    }

    #[test]
    fn every_value_appears_once() {
        let mut seen = [false; 64];
        for &(_, v) in CBP {
            assert!(!seen[v as usize]);
            seen[v as usize] = true;
        }
        assert!(seen.iter().all(|&s| s));
        let mut m: Vec<i16> = MOTION.iter().map(|x| x.1).collect();
        m.sort();
        assert_eq!(m, (-16..=16).collect::<Vec<_>>());
        // motion codes are the address increment codes with a trailing sign bit
        for &(c, v) in MOTION {
            if v == 0 {
                continue;
            }
            let (code, n) = parse_code(c);
            let mba = MBA.iter().find(|(m, _)| parse_code(m) == (code, n)).map(|m| m.1).unwrap();
            assert_eq!(mba / 2, v.abs());
            assert_eq!(mba % 2 == 0, v < 0, "{c}");
        }
        // run/level pairs are distinct within each table
        for t in [dct_table(DCT_ZERO, DCT_ZERO_LONG), dct_table(DCT_ONE, &[])] {
            let mut vals: Vec<i16> = t.iter().map(|x| x.1).filter(|&v| v >= 0).collect();
            let n = vals.len();
            vals.sort();
            vals.dedup();
            assert_eq!(vals.len(), n);
            assert_eq!(n, 111, "111 run/level pairs besides EOB and escape");
        }
    }

    #[test]
    fn decodes_codes() {
        let t = tables();
        // 0000 0011 000 (33) then 1 (1) then 0000 0001 000 (escape)
        let d = [0b0000_0011, 0b0001_0000, 0b0001_0000, 0];
        let mut b = Bits::new(&d);
        assert_eq!(t.mba.decode(&mut b), Some(33));
        assert_eq!(t.mba.decode(&mut b), Some(1));
        assert_eq!(t.mba.decode(&mut b), Some(MBA_ESCAPE));
        let d = [0b0000_0000, 0b0001_1111, 0];
        let mut b = Bits::new(&d);
        assert_eq!(t.dct_zero.decode(&mut b), Some(rl(27, 1)));
    }
}
