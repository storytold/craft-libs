//! Nikon Huffman-compressed NEF image data (TIFF compression 34713): "lossless compressed" and "lossy compressed"
//! (type 1 and type 2) at 12 and 14 bits.
//!
//! **Clean-room.** No raw-decoder source code (dcraw, LibRaw, rawspeed, rawloader/rawler, darktable, RawTherapee,
//! libopenraw, ExifTool's Perl code, …) was read or consulted. What we used:
//!
//! - Prose format descriptions: Laurent Clévy, "Nikon Electronic File (NEF) format" (<http://lclevy.free.fr/nef/>),
//!   the maker-note tag table and the "0x96 (linearization table) tag format" table (version bytes, four `u16`
//!   predictor seeds at offset 2, curve size at 10, curve at 12, split value at 562 for lossy type 2, "lossy type
//!   2 reads an incomplete table and interpolation is required"); Bill Claff, "NEF Compression"
//!   (<https://www.photonstophotos.net/NikonInfo/NEF_Compression.htm>): a lossy encoding curve followed by a
//!   non-adaptive (fixed-table) Huffman stage, and the number of encoded values per camera (683, 769, 1025, 2753,
//!   3073, 4097 …), which confirmed our curve interpolation; the libopenraw format notes
//!   (<https://libopenraw.freedesktop.org/formats/nef/>, prose only); ExifTool's Nikon tag-name documentation
//!   (`0x0093` NEFCompression, `0x0096` NEFLinearizationTable); ITU-T T.81 Annex F/H for the "difference category
//!   + additional bits" coding and canonical (BITS/HUFFVAL) Huffman codes.
//! - Our own black-box analysis of CC0 samples from raw.pixls.us. Nikon's Huffman tables are not stored in the
//!   files and no prose source lists them, so they were recovered from the data: several cameras (D5100, D300,
//!   D700) were shot with the same scene in compressed and uncompressed modes; a beam search assigned code words
//!   to difference categories so that the decoded compressed image matched the uncompressed one (likelihood of
//!   the decoded values given the reference, minus the bits consumed, plus a description-length penalty per new
//!   code word). Each table was then verified on ~45 files from ~35 bodies (D40 … D850, Df, Z 6, Z 50, 1 J1): with
//!   the right table the whole strip decodes and ends within the last byte of the data, with every value inside
//!   the encoded range. The predictor and difference coding are the same as Pentax PEF (see `pef.rs`).
//!
//! Format, as established:
//! - One strip, MSB-first bit stream, no byte stuffing, continuous across rows. Per pixel a canonical Huffman code
//!   gives the category `n`, followed by `n` additional bits (T.81: a leading 0 means a negative difference).
//! - Each pixel is predicted from the same-colour pixel two to the left; the first two pixels of a row from the
//!   first two pixels of the previous row of the same parity, starting at the seeds of maker note `0x0096`.
//! - Lossless (`0x0096` version `0x46`): the decoded values are the samples. Lossy (version `0x44`): they index a
//!   curve. Type 1 (`0x44 0x10`) stores the full curve; type 2 (`0x44 0x20`, `0x44 0x40`) stores 257 points,
//!   one every `2^bits / 256` codes (`0x20`) or `2^bits / 1024` codes (`0x40`), linearly interpolated. The span
//!   was established from the files: in every `0x20` file the first interval of the 257 points rises by
//!   `2^bits / 256` (16 at 12 bit, 64 at 14 bit) and in every `0x40` file by `2^bits / 1024` (4 and 16), i.e.
//!   the curve is the identity in the dark part, one code per sample. Read with a point every `2^bits / 256`
//!   codes instead, the 12 `0x40` files that are not split decoded to values that never exceeded 255 (12 bit) or
//!   1023 (14 bit) and whose darkest 0.01 % sat at a quarter of the black level of maker note `0x003d` (63 against
//!   252 in 12-bit units); with the first-interval step they sit at that black level.
//! - Values in the maker note are in the maker note's byte order (newer bodies write little-endian notes).
//!
//! "Lossy after split" (non-zero split row at `0x0096` offset 562, type 2 tables only, found in 11 of the CC0
//! samples): derived by black-box analysis of those files. Rows below the split row use the lossy Huffman path
//! above unchanged. Row `split` starts at the very next bit (no alignment); from there every pixel is a word of
//! `bits - 4` bits (8 at 12 bit, 10 at 14 bit), MSB first, with no row padding. A word is a complete prefix code
//! for a difference category `n` followed by the top `W - prefix length` bits of the T.81 additional bits (the low
//! bits are dropped); the difference is the centre of the magnitude bin that was kept. The predictor is the same
//! as above and the running value is not clamped (only the sample index is, before the curve lookup). The camera
//! ends the strip with 0 to 7 one-bits after the last full row, which the decoder checks.
//!
//! Not supported (returned as [`RawError::Unsupported`], the embedded preview is used instead):
//! - "Lossy after split" files whose fixed-rate section does not end within the last byte of the strip (see
//!   below): refused rather than decoded wrongly.
//! - Other `0x0096` versions, bit depths other than 12/14.
//! - Code words that never occurred in any sample (the lossy 12-bit 8-bit code word `11111110`, and code words
//!   longer than 9 bits in the lossy tables) are rejected as corrupt; they cannot code a difference within the
//!   lossy value range anyway.

use crate::huffman::{Bits, Huffman, diff};
use crate::{MAX_SAMPLES, RawError, Result};
use craft_tiff::ByteOrder;

/// A table code word whose symbol was never observed (see the module docs).
const UNSEEN: u8 = u8::MAX;

/// Canonical Huffman table: number of codes of length 1..=16 and the symbols (difference categories) in code order.
struct Table {
    counts: &'static [u8],
    symbols: &'static [u8],
}

const LOSSLESS_12: Table = Table { counts: &[0, 1, 4, 2, 3, 1, 2], symbols: &[5, 4, 6, 3, 7, 2, 8, 1, 9, 0, 10, 11, 12] };
const LOSSLESS_14: Table = Table { counts: &[0, 1, 4, 2, 2, 3, 1, 2], symbols: &[7, 6, 8, 5, 9, 4, 10, 3, 11, 12, 2, 0, 1, 13, 14] };
const LOSSY_12: Table = Table { counts: &[0, 1, 5, 1, 1, 1, 1, 1, 1], symbols: &[5, 4, 3, 6, 2, 7, 1, 0, 8, 9, UNSEEN, 10] };
const LOSSY_14: Table = Table { counts: &[0, 1, 4, 3, 1, 1, 1, 1, 1], symbols: &[5, 6, 4, 7, 8, 3, 9, 2, 1, 0, 10, 11, 12] };

impl Table {
    /// `(left-aligned 12-bit code, length)` per symbol, for [`Huffman::new`].
    fn codes(&self) -> Vec<(u16, u8)> {
        let mut out = vec![(0u16, 0u8); 17];
        let mut code = 0u32;
        let mut symbols = self.symbols.iter();
        for (i, &n) in self.counts.iter().enumerate() {
            let len = i as u32 + 1;
            for _ in 0..n {
                if let Some(&s) = symbols.next()
                    && let Some(slot) = out.get_mut(s as usize)
                {
                    *slot = ((code << (12 - len)) as u16, len as u8);
                }
                code += 1;
            }
            code <<= 1;
        }
        out
    }
}

/// How the decoded values map to samples.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Encoding {
    /// The decoded values are the samples (`0..2^bits`).
    Lossless,
    /// The decoded values index this curve.
    Lossy(Vec<u16>),
}

/// Maker note `0x0096` (NEF linearization table), parsed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DecodeTable {
    pub encoding: Encoding,
    /// Predictor seeds `[row parity][column]`.
    pub seeds: [[i32; 2]; 2],
    /// First row of the fixed-rate section ("lossy after split"), 0 when the whole strip is Huffman coded.
    pub split: usize,
}

/// Parse maker note `0x0096` for a `bits`-bit image. `order` is the maker note's byte order.
pub(crate) fn parse_table(t: &[u8], order: ByteOrder, bits: u32) -> Result<DecodeTable> {
    let u16_at = |o: usize| t.get(o..o + 2).and_then(|s| <[u8; 2]>::try_from(s).ok()).map(|b| order.u16(b));
    let short = || RawError::Corrupt(format!("NEF: linearization table too short ({} bytes)", t.len()));
    if bits != 12 && bits != 14 {
        return Err(RawError::Unsupported(format!("Nikon compressed NEF with {bits}-bit samples")));
    }
    let (v0, v1) = (t.first().copied().ok_or_else(short)?, t.get(1).copied().ok_or_else(short)?);
    let mut seeds = [[0i32; 2]; 2];
    for (i, s) in seeds.iter_mut().flatten().enumerate() {
        *s = u16_at(2 + 2 * i).ok_or_else(short)? as i32;
    }
    let mut split = 0;
    let encoding = match (v0, v1) {
        (0x46, _) => Encoding::Lossless,
        (0x44, 0x10 | 0x20 | 0x40) => {
            let n = u16_at(10).ok_or_else(short)? as usize;
            let points: Vec<u16> = (0..n).map(|i| u16_at(12 + 2 * i)).collect::<Option<_>>().ok_or_else(short)?;
            if n < 2 {
                return Err(RawError::Corrupt(format!("NEF: linearization curve of {n} points")));
            }
            if v1 == 0x10 {
                Encoding::Lossy(points)
            } else {
                split = u16_at(562).unwrap_or(0) as usize;
                // type 2: `n` points covering `range` codes, one every `range / (n - 1)`; the curve's first interval
                // rises by exactly that step (identity in the dark part) and the table version sets the span:
                // `0x20` covers all 2^bits codes, `0x40` the first quarter (see the module docs)
                let range = if v1 == 0x40 { 1usize << (bits - 2) } else { 1usize << bits };
                if n - 1 > range || !range.is_multiple_of(n - 1) {
                    return Err(RawError::Unsupported(format!("NEF: {n}-point lossy curve for {bits}-bit data")));
                }
                let step = range / (n - 1);
                let mut curve = Vec::with_capacity(range + 1);
                for pair in points.windows(2) {
                    let &[a, b] = pair else { continue };
                    let (a, b) = (a as u32, b as u32);
                    for k in 0..step as u32 {
                        let s = step as u32;
                        curve.push(((a * (s - k) + b * k) / s) as u16);
                    }
                }
                curve.extend(points.last().copied());
                Encoding::Lossy(curve)
            }
        }
        _ => return Err(RawError::Unsupported(format!("Nikon compressed NEF version {v0:#04x} {v1:#04x}"))),
    };
    Ok(DecodeTable { encoding, seeds, split })
}

/// Difference-category prefix codes of the fixed-rate section: `(prefix bits, prefix length, category n)`.
/// A word of `W` bits is a prefix followed by `W - length` bits of the T.81 additional bits of category `n`.
/// (The 14-bit `010` = category 12 never occurs in the samples; its category is deduced.)
const FIXED_12: &[(u32, u32, u32)] = &[
    (0b00, 2, 9),
    (0b010, 3, 10),
    (0b011, 3, 8),
    (0b100, 3, 7),
    (0b101, 3, 6),
    (0b110, 3, 5),
    (0b1110, 4, 4),
    (0b11110, 5, 3),
    (0b111110, 6, 2),
    (0b1111110, 7, 1),
    (0b11111110, 8, 0),
];
const FIXED_14: &[(u32, u32, u32)] = &[
    (0b00, 2, 8),
    (0b010, 3, 12),
    (0b011, 3, 11),
    (0b100, 3, 10),
    (0b101, 3, 9),
    (0b110, 3, 7),
    (0b1110, 4, 6),
    (0b11110, 5, 5),
    (0b111110, 6, 4),
    (0b1111110, 7, 3),
    (0b11111110, 8, 2),
    (0b111111110, 9, 1),
    (0b1111111110, 10, 0),
];

/// Word (`bits - 4` bits) to dequantised difference for the fixed-rate section; `None` marks the unused all-ones word.
fn fixed_word_table(bits: u32) -> Vec<Option<i32>> {
    let (w, codes) = if bits == 12 { (8, FIXED_12) } else { (10, FIXED_14) };
    let mut lut = vec![None; 1usize << w];
    for &(prefix, len, n) in codes {
        let kept = w - len;
        for m in 0..1u32 << kept {
            let e = n.saturating_sub(kept);
            let ex = (m << e) as i32;
            let half = (1i32 << e) >> 1;
            let d = if n == 0 {
                0
            } else if ex >= 1 << (n - 1) {
                ex + half
            } else if e > 0 {
                ex + half - (1 << n)
            } else {
                ex - ((1 << n) - 1)
            };
            if let Some(slot) = lut.get_mut(((prefix << kept) | m) as usize) {
                *slot = Some(d);
            }
        }
    }
    lut
}

/// Decode a whole `w × h` Nikon Huffman-compressed strip `src` with `bits`-bit samples.
#[cfg(test)]
pub(crate) fn decode(src: &[u8], w: usize, h: usize, bits: u32, table: &DecodeTable) -> Result<Vec<u16>> {
    decode_rows(src, w, h, h, bits, table)
}

/// The first `rows` rows (`w × rows` samples) of a `w × h` strip. The stream is sequential, so a header-only probe
/// decodes just the rows its masked-column test looks at (storytold/lightcraft#708) instead of the whole image. The size and
/// truncation checks still cover the whole image; the "lossy after split" end-of-strip rule (and corruption
/// further down) can only be seen when the whole strip is decoded.
pub(crate) fn decode_rows(src: &[u8], w: usize, h: usize, rows: usize, bits: u32, table: &DecodeTable) -> Result<Vec<u16>> {
    if bits != 12 && bits != 14 {
        return Err(RawError::Unsupported(format!("Nikon compressed NEF with {bits}-bit samples")));
    }
    let whole = w.checked_mul(h).filter(|&n| n > 0 && n <= MAX_SAMPLES).ok_or(RawError::Limit("NEF image size"))?;
    // every code word is at least 2 bits long: a strip that can't hold the image is truncated (also bounds the
    // allocation by the file size)
    if whole / 4 > src.len() {
        return Err(RawError::Corrupt(format!("NEF: {} bytes of compressed data for {w}x{h} pixels", src.len())));
    }
    let rows = rows.min(h);
    let n = w.saturating_mul(rows);
    let huff = Huffman::new(
        &match (&table.encoding, bits) {
            (Encoding::Lossless, 12) => LOSSLESS_12,
            (Encoding::Lossless, _) => LOSSLESS_14,
            (Encoding::Lossy(_), 12) => LOSSY_12,
            (Encoding::Lossy(_), _) => LOSSY_14,
        }
        .codes(),
    )?;
    let (lut, max): (&[u16], i32) = match &table.encoding {
        Encoding::Lossless => (&[], (1i32 << bits) - 1),
        Encoding::Lossy(curve) => (curve, curve.len() as i32 - 1),
    };
    if table.split != 0 && table.split >= h {
        return Err(RawError::Corrupt(format!("NEF: split row {} outside the {h} rows", table.split)));
    }
    let fixed = if table.split != 0 { fixed_word_table(bits) } else { Vec::new() };
    let word = bits - 4;
    let mut out = vec![0u16; n];
    let mut stream = Bits::new(src);
    let mut vpred = table.seeds;
    let mut clipped = 0usize;
    for (y, row) in out.chunks_exact_mut(w).enumerate() {
        let seeds = vpred.get_mut(y & 1).ok_or(RawError::Limit("NEF row"))?;
        let mut hpred = [0i32; 2];
        for (x, o) in row.iter_mut().enumerate() {
            let d = if table.split != 0 && y >= table.split {
                let code = stream.get(word) as usize;
                fixed.get(code).copied().flatten().ok_or_else(|| RawError::Corrupt(format!("NEF: invalid fixed-rate word at row {y}")))?
            } else {
                diff(&mut stream, &huff).ok_or_else(|| RawError::Corrupt(format!("NEF: invalid Huffman code at row {y}")))?
            };
            // a run of large differences in a corrupt stream can drive a predictor past i32 (storytold/lightcraft#1515)
            let overflow = || RawError::Corrupt(format!("NEF: predictor out of range at row {y}"));
            let p = if x < 2 {
                let s = seeds.get_mut(x).ok_or(RawError::Limit("NEF column"))?;
                *s = s.checked_add(d).ok_or_else(overflow)?;
                hpred[x & 1] = *s;
                *s
            } else {
                let p = hpred[x & 1].checked_add(d).ok_or_else(overflow)?;
                hpred[x & 1] = p;
                p
            };
            let v = p.clamp(0, max);
            clipped += (v != p && (table.split == 0 || y < table.split)) as usize;
            *o = if lut.is_empty() { v as u16 } else { lut.get(v as usize).copied().unwrap_or(0) };
        }
        // the camera pads the strip; reading more than a few bytes past its end means truncated or corrupt data
        if stream.consumed_bits() > src.len() * 8 + 64 {
            return Err(RawError::Corrupt(format!("NEF: compressed data ends at row {y} of {h}")));
        }
    }
    if table.split != 0 && rows == h {
        // the fixed-rate section must fill the rest of the strip: whole rows of w words and 0..7 bits of padding.
        // A strip may hold a few more rows than the image (one body stores 4022 rows for 4020); anything else means
        // the file does not follow the rule, so refuse it instead of decoding it wrongly.
        let left = (src.len() * 8).checked_sub(stream.consumed_bits());
        let row_bits = w * word as usize;
        if !left.is_some_and(|l| l % row_bits < 8 && l / row_bits <= 4) {
            return Err(RawError::Unsupported(format!(
                "Nikon \"lossy after split\" NEF whose fixed-rate section (from row {}) does not end within the last byte",
                table.split
            )));
        }
    }
    // a wrong table / corrupt stream drifts out of range quickly; real files stay inside (a handful of edge pixels)
    if clipped > n / 1000 + 16 {
        return Err(RawError::Corrupt(format!("NEF: {clipped} decoded values out of range")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MSB-first bit writer for the test encoder.
    #[derive(Default)]
    struct Writer {
        out: Vec<u8>,
        acc: u64,
        n: u32,
    }

    impl Writer {
        fn put(&mut self, v: u32, len: u32) {
            for i in (0..len).rev() {
                self.acc = (self.acc << 1) | ((v >> i) & 1) as u64;
                self.n += 1;
                if self.n == 8 {
                    self.out.push(self.acc as u8);
                    self.acc = 0;
                    self.n = 0;
                }
            }
        }
        fn finish(mut self) -> Vec<u8> {
            if self.n > 0 {
                self.out.push((self.acc << (8 - self.n)) as u8);
            }
            self.out.extend_from_slice(&[0; 4]);
            self.out
        }
    }

    fn category(d: i32) -> u32 {
        32 - d.unsigned_abs().leading_zeros()
    }

    /// Encode `values` (decoded domain: samples or curve indices) the way the camera does.
    fn encode(values: &[i32], w: usize, table: &Table, seeds: [[i32; 2]; 2]) -> Vec<u8> {
        let codes = table.codes();
        let mut wr = Writer::default();
        let mut vpred = seeds;
        for (y, row) in values.chunks(w).enumerate() {
            for (x, &v) in row.iter().enumerate() {
                let pred = if x < 2 { vpred[y & 1][x] } else { row[x - 2] };
                if x < 2 {
                    vpred[y & 1][x] = v;
                }
                let d = v - pred;
                let k = category(d);
                let (code, len) = codes[k as usize];
                assert!(len > 0, "no code for category {k}");
                wr.put(code as u32 >> (12 - len), len as u32);
                let extra = if d < 0 { d + (1 << k) - 1 } else { d };
                wr.put(extra as u32, k);
            }
        }
        wr.finish()
    }

    /// Deterministic test image: smooth gradient + texture + hard edges, within `0..=max`.
    fn image(w: usize, h: usize, max: i32, seed: u32) -> Vec<i32> {
        let mut s = seed;
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (s >> 24) as i32 % 9 - 4;
                let base = (x * 37 + y * 11) % (max / 2) + if (x / 7 + y / 5) % 3 == 0 { max / 3 } else { 0 };
                (base + noise).clamp(0, max)
            })
            .collect()
    }

    fn lossless(bits: u32) -> DecodeTable {
        let s = 1 << (bits - 3);
        DecodeTable { encoding: Encoding::Lossless, seeds: [[s, s], [s, s]], split: 0 }
    }

    #[test]
    fn tables_are_complete_prefix_codes() {
        for t in [LOSSLESS_12, LOSSLESS_14, LOSSY_12, LOSSY_14] {
            assert_eq!(t.counts.iter().map(|&c| c as usize).sum::<usize>(), t.symbols.len());
            let kraft: f64 = t.counts.iter().enumerate().map(|(i, &c)| c as f64 / (1u64 << (i + 1)) as f64).sum();
            assert!(kraft <= 1.0);
            assert!(Huffman::new(&t.codes()).is_ok());
        }
        assert_eq!(LOSSLESS_14.codes()[7], (0b00 << 10, 2));
        assert_eq!(LOSSLESS_14.codes()[0], (0b111110 << 6, 6));
        assert_eq!(LOSSY_12.codes()[10], (0b111111110 << 3, 9));
    }

    #[test]
    fn lossless_roundtrip_12_and_14() {
        for (bits, table) in [(12, LOSSLESS_12), (14, LOSSLESS_14)] {
            let (w, h) = (64, 9);
            let max = (1 << bits) - 1;
            let mut img = image(w, h, max, bits);
            // extreme jumps exercise the longest categories
            img[5] = 0;
            img[7] = max;
            img[w + 9] = max;
            let t = lossless(bits);
            let src = encode(&img, w, &table, t.seeds);
            let out = decode(&src, w, h, bits, &t).unwrap();
            assert_eq!(out, img.iter().map(|&v| v as u16).collect::<Vec<_>>(), "{bits}-bit");
        }
    }

    fn lossy_table(bits: u32, order: ByteOrder) -> Vec<u8> {
        // version 0x44 0x20, seeds, 257 points of a concave curve, padding up to the split value (0)
        let mut t = vec![0x44, 0x20];
        let put = |t: &mut Vec<u8>, v: u16| t.extend_from_slice(&if order == ByteOrder::Big { v.to_be_bytes() } else { v.to_le_bytes() });
        for _ in 0..4 {
            put(&mut t, 300);
        }
        put(&mut t, 257);
        let max = ((1u32 << bits) - 1) as f64;
        for i in 0..257u32 {
            put(&mut t, (max * (i as f64 / 64.0).min(1.0).sqrt()) as u16);
        }
        t.resize(624, 0);
        t
    }

    #[test]
    fn lossy_type2_curve_and_roundtrip() {
        for (bits, table) in [(12u32, LOSSY_12), (14, LOSSY_14)] {
            for order in [ByteOrder::Big, ByteOrder::Little] {
                let t = parse_table(&lossy_table(bits, order), order, bits).unwrap();
                assert_eq!(t.seeds, [[300, 300], [300, 300]]);
                let Encoding::Lossy(curve) = &t.encoding else { panic!() };
                let step = (1usize << bits) / 256;
                assert_eq!(curve.len(), (1 << bits) + 1);
                assert_eq!(curve[64 * step], (1 << bits) - 1);
                assert!(curve.windows(2).all(|p| p[0] <= p[1]));
                // the encoded range is the curve's rising part
                let (w, h) = (48, 7);
                let img = image(w, h, 64 * step as i32 - 1, 7);
                let src = encode(&img, w, &table, t.seeds);
                let out = decode(&src, w, h, bits, &t).unwrap();
                assert_eq!(out, img.iter().map(|&v| curve[v as usize]).collect::<Vec<_>>());
            }
        }
    }

    /// Maker note `0x0096` as bytes, written out by hand: version `0x44 <v1>`, four seeds, 257 points (little-endian
    /// as newer bodies write it) of a curve whose first 256 points rise by `step` and whose last is the white
    /// level, and a zero split value at offset 562.
    fn hand_table(v1: u8, seed: u16, step: u16, white: u16) -> Vec<u8> {
        let mut t = vec![0x44, v1];
        for _ in 0..4 {
            t.extend_from_slice(&seed.to_le_bytes());
        }
        t.extend_from_slice(&257u16.to_le_bytes());
        for k in 0..256u16 {
            t.extend_from_slice(&(k * step).to_le_bytes());
        }
        t.extend_from_slice(&white.to_le_bytes());
        t.resize(624, 0);
        t
    }

    /// Known answers for the `0x44 0x40` curve, from bytes and bit strings written by hand (nothing here uses the
    /// decoder's span rule or the test encoder): the first interval of the points rises by `2^bits / 1024`, so
    /// the curve is the identity for one code per sample and spans `2^bits / 4` codes. The `0x20` curve of the
    /// same sizes keeps its span of all `2^bits` codes.
    #[test]
    fn lossy_type2_version_0x40_spans_a_quarter_of_the_range() {
        // 12 bit: points 0, 4, 8 … 1020, 4095
        let t = parse_table(&hand_table(0x40, 252, 4, 4095), ByteOrder::Little, 12).unwrap();
        let Encoding::Lossy(curve) = &t.encoding else { panic!() };
        assert_eq!(curve.len(), 1025);
        assert_eq!(curve[0..6], [0, 1, 2, 3, 4, 5]);
        assert_eq!((curve[252], curve[255], curve[1020]), (252, 255, 1020));
        assert_eq!((curve[1021], curve[1022], curve[1023], curve[1024]), (1788, 2557, 3326, 4095));
        // the pixels p0 = 252 + 4, p1 = 252, p2 = p0 - 1, p3 = p1, coded as 011 100 | 11110 | 1110 0 | 11110
        // (LOSSY_12: category 3 = 011, 0 = 11110, 1 = 1110; extra bits 100 = +4, 0 = -1)
        let out = decode(&[0x73, 0xdc, 0xf0], 4, 1, 12, &t).unwrap();
        assert_eq!(out, [256, 252, 255, 252]);
        // 14 bit: points 0, 16, 32 … 4080, 16383
        let t = parse_table(&hand_table(0x40, 1008, 16, 16383), ByteOrder::Little, 14).unwrap();
        let Encoding::Lossy(curve) = &t.encoding else { panic!() };
        assert_eq!(curve.len(), 4097);
        assert_eq!(curve[0..6], [0, 1, 2, 3, 4, 5]);
        assert_eq!((curve[1008], curve[4080], curve[4096]), (1008, 4080, 16383));
        // the same pixels coded as 1100 100 | 111110 | 11110 0 | 111110
        // (LOSSY_14: category 3 = 1100, 0 = 111110, 1 = 11110)
        let out = decode(&[0xc9, 0xf7, 0x9f, 0x00], 4, 1, 14, &t).unwrap();
        assert_eq!(out, [1012, 1008, 1011, 1008]);
        // version 0x20 with the same points is a curve over all 2^bits codes: the points are 16 codes apart
        let t = parse_table(&hand_table(0x20, 252, 16, 4095), ByteOrder::Little, 12).unwrap();
        let Encoding::Lossy(curve) = &t.encoding else { panic!() };
        assert_eq!((curve.len(), curve[16], curve[17], curve[4095]), (4097, 16, 17, 4094));
        // 200 points can't be spaced evenly over the 1024 codes of a 12-bit 0x40 table
        let mut odd = hand_table(0x40, 252, 4, 4095);
        odd[10..12].copy_from_slice(&200u16.to_le_bytes());
        assert!(parse_table(&odd, ByteOrder::Little, 12).is_err());
    }

    /// Pack a string of `0`/`1` (spaces ignored) MSB first, padding the last byte with one-bits like the camera.
    fn pack(bits: &str) -> Vec<u8> {
        let b: Vec<bool> = bits.chars().filter(|c| !c.is_whitespace()).map(|c| c == '1').collect();
        b.chunks(8).map(|c| (0..8).fold(0u8, |a, i| (a << 1) | c.get(i).copied().unwrap_or(true) as u8)).collect()
    }

    fn split_table(v1: u8, seed: u16, step: u16, white: u16, split: u16) -> DecodeTable {
        let mut t = hand_table(v1, seed, step, white);
        t[562..564].copy_from_slice(&split.to_le_bytes());
        parse_table(&t, ByteOrder::Little, if step == 4 { 12 } else { 14 }).unwrap()
    }

    /// 12-bit "lossy after split": row 0 Huffman coded, rows 1 and 2 as 8-bit words that follow the last Huffman
    /// bit directly. Bit strings written by hand from the rule (prefix, then the kept top bits of the extra bits).
    const SPLIT_12: &str = "011 100 11110 1110 0 11110         11111110 11110100 11100111 01110000         00000001 11111110 01111111 11111110";

    #[test]
    fn lossy_after_split_known_answers_12_bit() {
        let t = split_table(0x40, 252, 4, 4095, 1);
        assert_eq!(t.split, 1);
        let out = decode(&pack(SPLIT_12), 4, 3, 12, &t).unwrap();
        // row 1: d = 0 (n0), +4 (n3, exact), -8 (n4, exact), +132 (n8 keeps 5 bits: 16 << 3 plus half a bin, 4);
        // row 2: -500 (n9 keeps 6 bits: bin 1 of the negative side, centre -508 + 8), then +252 on the running
        // value -244 which is not clamped (8, not 252)
        assert_eq!(out, [256, 252, 255, 252, 252, 256, 244, 388, 0, 252, 8, 252]);
    }

    #[test]
    fn lossy_after_split_known_answers_14_bit() {
        let t = split_table(0x40, 1008, 16, 16383, 1);
        // row 0 as in the 0x40 test above; row 1 in 10-bit words: 0 (n0), +32 (n6, exact), -254 (n8, exact),
        // +510 (n9 keeps 7 bits: 127 << 2 plus 2)
        let bits = "1100 100 111110 11110 0 111110 1111111110 1110100000 0000000001 1011111111";
        let out = decode(&pack(bits), 4, 2, 14, &t);
        // word 3 is `00` + 8 bits: 00 00000001 -> n8 extra 00000001 = -254
        assert_eq!(out.unwrap(), [1012, 1008, 1011, 1008, 1008, 1040, 754, 1550]);
    }

    #[test]
    fn lossy_after_split_dequantisation_is_symmetric() {
        let lut = fixed_word_table(12);
        assert_eq!((lut[0b00_000000], lut[0b00_111111]), (Some(-508), Some(508)));
        assert_eq!((lut[0b011_00000], lut[0b011_11111]), (Some(-252), Some(252)));
        assert_eq!(lut[0b11111110], Some(0));
        assert_eq!(lut[0xff], None);
        let lut = fixed_word_table(14);
        assert_eq!((lut[0b00_00000000], lut[0b00_11111111]), (Some(-255), Some(255)));
        assert_eq!(lut[0b101_0000000], Some(-510));
        assert_eq!(lut[0b1111111110], Some(0));
        assert_eq!(lut[0x3ff], None);
        for bits in [12, 14] {
            let lut = fixed_word_table(bits);
            assert_eq!(lut.iter().filter(|d| d.is_none()).count(), 1, "{bits}-bit: only the all-ones word is unused");
        }
    }

    #[test]
    fn lossy_after_split_refuses_strips_that_do_not_follow_the_rule() {
        let t = split_table(0x40, 252, 4, 4095, 1);
        let mut longer = pack(SPLIT_12);
        longer.extend_from_slice(&[0xff; 3]);
        assert!(matches!(decode(&longer, 4, 3, 12, &t), Err(RawError::Unsupported(_))));
        let shorter = &pack(SPLIT_12)[..10];
        assert!(decode(shorter, 4, 3, 12, &t).is_err());
        // the unused all-ones word
        let bad = pack("011 100 11110 1110 0 11110 11111111 11111110 11111110 11111110");
        assert!(matches!(decode(&bad, 4, 2, 12, &t), Err(RawError::Corrupt(_))));
        // a split row beyond the image
        assert!(matches!(decode(&pack(SPLIT_12), 4, 1, 12, &t), Err(RawError::Corrupt(_))));
        // through the container
        let mut table = hand_table(0x40, 252, 4, 4095);
        table[562..564].copy_from_slice(&1u16.to_le_bytes());
        let ok = crate::decode(&nef_file(pack(SPLIT_12), 4, 3, 12, table.clone(), ByteOrder::Little)).unwrap();
        assert_eq!(ok.data, crate::RawData::U16(vec![256, 252, 255, 252, 252, 256, 244, 388, 0, 252, 8, 252]));
        let e = crate::decode(&nef_file(longer, 4, 3, 12, table, ByteOrder::Little));
        assert!(matches!(e, Err(RawError::Unsupported(_))), "{e:?}");
    }

    #[test]
    fn lossy_type1_full_curve() {
        let mut t = vec![0x44, 0x10, 0, 10, 0, 10, 0, 10, 0, 10, 0, 5];
        for v in [0u16, 100, 400, 900, 1600] {
            t.extend_from_slice(&v.to_be_bytes());
        }
        let parsed = parse_table(&t, ByteOrder::Big, 12).unwrap();
        assert_eq!(parsed.encoding, Encoding::Lossy(vec![0, 100, 400, 900, 1600]));
        let img = vec![0, 4, 1, 3, 2, 2, 4, 0];
        let src = encode(&img, 4, &LOSSY_12, parsed.seeds);
        assert_eq!(decode(&src, 4, 2, 12, &parsed).unwrap(), vec![0, 1600, 100, 900, 400, 400, 1600, 0]);
    }

    #[test]
    fn rejects_unsupported_and_malformed_tables() {
        let unsupported = |r: Result<DecodeTable>| matches!(r, Err(RawError::Unsupported(_)));
        let corrupt = |r: Result<DecodeTable>| matches!(r, Err(RawError::Corrupt(_)));
        let mut split = lossy_table(12, ByteOrder::Big);
        split[562..564].copy_from_slice(&345u16.to_be_bytes());
        assert_eq!(parse_table(&split, ByteOrder::Big, 12).unwrap().split, 345);
        assert!(unsupported(parse_table(&[0x49, 0x30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], ByteOrder::Big, 12)));
        assert!(unsupported(parse_table(&lossy_table(12, ByteOrder::Big), ByteOrder::Big, 16)));
        assert!(corrupt(parse_table(&[], ByteOrder::Big, 12)));
        assert!(corrupt(parse_table(&[0x46, 0x30, 8], ByteOrder::Big, 14)));
        assert!(corrupt(parse_table(&lossy_table(12, ByteOrder::Big)[..100], ByteOrder::Big, 12)));
        // 300 points don't divide the 12-bit range
        let mut odd = lossy_table(12, ByteOrder::Big);
        odd[10..12].copy_from_slice(&300u16.to_be_bytes());
        assert!(parse_table(&odd, ByteOrder::Big, 12).is_err());
        assert!(parse_table(&[0x44, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0], ByteOrder::Big, 12).is_err());
    }

    /// A minimal NEF: CFA SubIFD with one compressed strip, Exif maker note (`Nikon\0` v2 header + embedded TIFF in
    /// `mn_order`) holding the linearization table.
    fn nef_file(strip: Vec<u8>, w: u32, h: u32, bits: u16, table: Vec<u8>, mn_order: ByteOrder) -> Vec<u8> {
        use craft_tiff::tags::{self as t, photometric};
        use craft_tiff::{IfdBuilder, ImageData, TiffWriter, Value};
        let mut mn = IfdBuilder::new();
        mn.set(0x0096, Value::Undefined(table));
        let mut note = b"Nikon\0\x02\x10\0\0".to_vec();
        note.extend(TiffWriter::new(mn_order, false).write(&[mn]).unwrap());
        let mut exif = IfdBuilder::new();
        exif.set(t::MAKER_NOTE, Value::Undefined(note));
        let mut raw = IfdBuilder::new();
        raw.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
        raw.set(t::IMAGE_WIDTH, Value::Long(vec![w]));
        raw.set(t::IMAGE_LENGTH, Value::Long(vec![h]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![bits]));
        raw.set(t::COMPRESSION, Value::Short(vec![t::compression::NIKON]));
        raw.set(t::PHOTOMETRIC, Value::Short(vec![photometric::CFA]));
        raw.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
        raw.set(t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 1, 2]));
        raw.set_image(ImageData::Strips { rows_per_strip: h, strips: vec![strip] });
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::MAKE, Value::Ascii("NIKON CORPORATION".into()));
        ifd0.set(t::MODEL, Value::Ascii("NIKON TEST".into()));
        ifd0.set_child(t::EXIF_IFD, exif);
        ifd0.add_sub_ifd(raw);
        TiffWriter::new(ByteOrder::Big, false).write(&[ifd0]).unwrap()
    }

    #[test]
    fn decodes_through_the_nef_container() {
        // lossless 14-bit, big-endian maker note
        let (w, h) = (40usize, 6usize);
        let img = image(w, h, 16383, 11);
        let mut table = vec![0x46, 0x30];
        for _ in 0..4 {
            table.extend_from_slice(&2048u16.to_be_bytes());
        }
        table.resize(46, 0);
        let src = encode(&img, w, &LOSSLESS_14, [[2048; 2]; 2]);
        let bytes = nef_file(src, w as u32, h as u32, 14, table, ByteOrder::Big);
        let r = crate::decode(&bytes).unwrap();
        assert_eq!(r.data, crate::RawData::U16(img.iter().map(|&v| v as u16).collect()));
        assert_eq!(r.cfa.as_ref().unwrap().name(), "RGGB");
        assert_eq!(crate::probe_info(&bytes).unwrap(), r.info());
        // lossy 12-bit type 2, little-endian maker note (newer bodies)
        let table = lossy_table(12, ByteOrder::Little);
        let t = parse_table(&table, ByteOrder::Little, 12).unwrap();
        let Encoding::Lossy(curve) = &t.encoding else { panic!() };
        let img = image(w, h, 1023, 5);
        let src = encode(&img, w, &LOSSY_12, t.seeds);
        let r = crate::decode(&nef_file(src.clone(), w as u32, h as u32, 12, table.clone(), ByteOrder::Little)).unwrap();
        assert_eq!(r.data, crate::RawData::U16(img.iter().map(|&v| curve[v as usize]).collect()));
        // a split file whose strip does not follow the fixed-rate rule is refused, never decoded wrongly
        let mut split = table;
        split[562..564].copy_from_slice(&3u16.to_le_bytes());
        let e = crate::decode(&nef_file(src, w as u32, h as u32, 12, split, ByteOrder::Little));
        assert!(e.is_err(), "{e:?}");
    }

    /// storytold/lightcraft#708: the header probe of a compressed NEF decodes only the first [`crate::nef::MASK_ROWS`]
    /// rows (the ones the masked-column test reads) instead of the whole Huffman stream, and still finds the masked
    /// columns the full decode finds. A strip cut off after those rows proves it: the probe succeeds with the same
    /// description while the full decode reports the truncation.
    #[test]
    fn header_probe_stops_after_the_masked_column_rows() {
        use crate::nef::MASK_ROWS;
        let (w, h) = (160usize, MASK_ROWS * 3);
        let mut img = image(w, h, 16383, 7);
        // eight optically masked columns at 0, as the D5100 / D7000 write them
        for row in img.chunks_mut(w) {
            row[w - 8..].fill(0);
        }
        let t = lossless(14);
        let table = {
            let mut v = vec![0x46, 0x30];
            for s in t.seeds.iter().flatten() {
                v.extend_from_slice(&(*s as u16).to_be_bytes());
            }
            v.resize(46, 0);
            v
        };
        let src = encode(&img, w, &LOSSLESS_14, t.seeds);
        let whole = nef_file(src.clone(), w as u32, h as u32, 14, table.clone(), ByteOrder::Big);
        let full = crate::decode(&whole).unwrap();
        assert_eq!(full.active_area, crate::Rect::new(0, 0, w - 8, h), "the masked columns are found in a compressed file");
        assert_eq!(crate::probe_info(&whole).unwrap(), full.info());
        // the same stream cut off after two thirds of the rows (the first rows' codes are a prefix of the whole)
        let cut = encode(&img[..w * (h * 2 / 3)], w, &LOSSLESS_14, t.seeds).len();
        assert!(cut > w * h / 4, "enough bytes to pass the size check");
        let truncated = nef_file(src[..cut].to_vec(), w as u32, h as u32, 14, table, ByteOrder::Big);
        assert!(matches!(crate::decode(&truncated), Err(RawError::Corrupt(_))), "the full decode sees the truncation");
        assert_eq!(crate::probe_info(&truncated).unwrap(), full.info(), "the header probe never reads that far");
        // decode_rows itself: a prefix of the whole decode, and the whole image's size check still applies
        let top = decode_rows(&src, w, h, MASK_ROWS, 14, &t).unwrap();
        assert_eq!(top.len(), w * MASK_ROWS);
        assert_eq!(top[..], decode(&src, w, h, 14, &t).unwrap()[..w * MASK_ROWS]);
        assert_eq!(decode_rows(&src, w, h, h + 5, 14, &t).unwrap().len(), w * h, "rows are clamped to the height");
        assert!(decode_rows(&src[..w * h / 4 - 1], w, h, 1, 14, &t).is_err(), "a strip too short for the image is corrupt in any mode");
    }

    /// A long run of the largest difference drove a predictor past i32: a panic in builds with overflow checks,
    /// wrapped values otherwise (storytold/lightcraft#1515). Both the row and the column predictors now stop with an error.
    #[test]
    fn predictor_overflow_is_an_error_not_a_panic() {
        let codes = LOSSLESS_14.codes();
        let k = (0..codes.len()).rev().find(|&k| codes[k].1 > 0).unwrap();
        let d = (1u32 << k) - 1;
        let stream = |n: usize| {
            let mut wr = Writer::default();
            let (code, len) = codes[k];
            for _ in 0..n {
                wr.put(code as u32 >> (12 - len), len as u32);
                wr.put(d, k as u32);
            }
            wr.finish()
        };
        // differences one predictor needs to pass i32::MAX
        let steps = (i32::MAX as u32 / d) as usize + 2;
        let table = DecodeTable { encoding: Encoding::Lossless, seeds: [[0; 2]; 2], split: 0 };
        // along a row: every other sample adds to the same predictor
        let w = 2 * steps + 2;
        let err = decode(&stream(w), w, 1, 14, &table).unwrap_err();
        assert!(matches!(err, RawError::Corrupt(ref m) if m.contains("predictor")), "{err}");
        // down two columns: each row adds to the seed of its parity
        let h = 2 * steps + 2;
        let err = decode(&stream(2 * h), 2, h, 14, &table).unwrap_err();
        assert!(matches!(err, RawError::Corrupt(ref m) if m.contains("predictor")), "{err}");
    }

    #[test]
    fn truncated_and_corrupted_streams_error_never_panic() {
        let (w, h) = (64, 16);
        let t = lossless(14);
        let img = image(w, h, 16383, 3);
        let src = encode(&img, w, &LOSSLESS_14, t.seeds);
        // truncation: too short for the image, or ends early
        for cut in [0, 1, 10, src.len() / 2, src.len() - 40] {
            assert!(decode(&src[..cut], w, h, 14, &t).is_err(), "cut {cut}");
        }
        // garbage and bit flips: an error or (rarely) some image, never a panic
        let mut s = 12345u32;
        for i in 0..200 {
            let mut bad = src.clone();
            for _ in 0..1 + i % 8 {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let at = (s >> 8) as usize % bad.len();
                bad[at] ^= 1 << (s % 8);
            }
            let _ = decode(&bad, w, h, 14, &t);
            let noise: Vec<u8> = (0..src.len()).map(|k| ((k as u32).wrapping_mul(2_654_435_761) >> (i % 24)) as u8).collect();
            let _ = decode(&noise, w, h, 14, &t);
            let _ = decode(&noise, w, h, 12, &lossless(12));
        }
        // all-ones never forms a valid lossy 12-bit code
        assert!(decode(&[0xff; 64], 8, 8, 12, &parse_table(&lossy_table(12, ByteOrder::Big), ByteOrder::Big, 12).unwrap()).is_err());
        // absurd dimensions are limited before allocating
        assert!(decode(&src, usize::MAX, 2, 14, &t).is_err());
        assert!(decode(&src, 1 << 20, 1 << 20, 14, &t).is_err());
        assert!(decode(&src, 0, 2, 14, &t).is_err());
    }
}
