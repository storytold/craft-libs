//! Helpers the maker decoders share: the Exif CFA layout, and black and white levels measured from the samples
//! for files that state none.

use crate::{BlackLevel, Cfa, Rect};
use craft_tiff::Tiff;
use std::ops::Range;

/// The colour-filter layout from the Exif `CFAPattern` tag (`0xa302`: two 16-bit repeat counts, found in either
/// byte order, then one byte per site: 0 = red, 1 = green, 2 = blue), when it describes a 2×2 Bayer cell.
pub(crate) fn cfa_from_exif(tiff: &Tiff) -> Option<Cfa> {
    let &[c0, c1, r0, r1, s0, s1, s2, s3] = tiff.exif()?.bytes(crate::EXIF_CFA_PATTERN)? else { return None };
    let two = |a: u8, b: u8| matches!((a, b), (2, 0) | (0, 2));
    let name = match [s0, s1, s2, s3] {
        [0, 1, 1, 2] => "RGGB",
        [2, 1, 1, 0] => "BGGR",
        [1, 0, 2, 1] => "GRBG",
        [1, 2, 0, 1] => "GBRG",
        _ => return None,
    };
    (two(c0, c1) && two(r0, r1)).then(|| Cfa::bayer_static(name))
}

/// Black level per 2×2 CFA position (anchored at the active area origin) from masked sensor columns `cols` over
/// rows `rows`. Falls back to 0 when the region is empty.
pub(crate) fn black_from_columns(data: &[u16], width: usize, cols: Range<usize>, rows: Range<usize>, active: Rect) -> BlackLevel {
    let height = data.len() / width.max(1);
    let (mut sum, mut n) = ([0f64; 4], [0u64; 4]);
    for y in rows.start..rows.end.min(height) {
        let py = (y as isize - active.y as isize).rem_euclid(2) as usize;
        for x in cols.start..cols.end.min(width) {
            let px = (x as isize - active.x as isize).rem_euclid(2) as usize;
            sum[py * 2 + px] += data[y * width + x] as f64;
            n[py * 2 + px] += 1;
        }
    }
    if n.contains(&0) {
        return BlackLevel::uniform(0.0);
    }
    BlackLevel { repeat_rows: 2, repeat_cols: 2, values: (0..4).map(|i| (sum[i] / n[i] as f64) as f32).collect(), delta_h: vec![], delta_v: vec![] }
}

/// Samples of up to 512 evenly spaced rows of the active area, for [`white_from_data`]: the borders can hold
/// their own saturated runs (0xffff junk on RW2, masked rows at 16383 on some Canon CR2s). The step is odd so both
/// row parities, and with them every CFA site, are read.
pub(crate) fn active_rows(data: &[u16], w: usize, a: Rect) -> Vec<u16> {
    let step = (a.height / 512).max(1) | 1;
    (a.y..a.y + a.height).step_by(step).filter_map(|y| data.get(y * w + a.x..y * w + a.x + a.width)).flatten().copied().collect()
}

/// White level estimate: the saturation plateau if a noticeable number of samples sit at the maximum value,
/// otherwise the full `bits` range. A few stray samples above the plateau (hot pixels, e.g. on a Canon EOS 70D
/// CR2) don't hide it.
pub(crate) fn white_from_data(data: &[u16], bits: u32) -> f32 {
    let full_level = (1u32 << bits.clamp(1, 16)) - 1;
    let full = full_level as f32;
    let step = (data.len() / 2_000_000).max(1);
    // samples above the nominal range are padding / invalid (e.g. 0xFFFF fill), not saturation
    let mut hist = vec![0usize; full_level as usize + 1];
    for &v in data.iter().step_by(step) {
        if let Some(h) = hist.get_mut(v as usize) {
            *h += 1;
        }
    }
    let min_plateau = (data.len() / 20_000).max(16);
    // the top of the data, skipping up to half a plateau's worth of strays so a real plateau can't be skipped
    let strays = min_plateau / 2;
    let mut above = 0usize;
    let Some(mx) = hist.iter().enumerate().rev().find_map(|(v, &n)| {
        above += n * step;
        (above > strays).then_some(v as u32)
    }) else {
        return full;
    };
    let band = (mx / 256).max(1);
    let near = mx.saturating_sub(band);
    let below = near.saturating_sub(band);
    let count = |from: u32, to: u32| hist.get(from as usize..to as usize).map_or(0, |h| h.iter().sum::<usize>());
    // the strays count as saturated too
    let top = count(near, full_level + 1);
    let under = count(below, near);
    // a saturation plateau is a spike: many more samples in the top band than in the band just below it
    if mx as f32 > full * 0.5 && top * step >= min_plateau && top >= 4 * under + 4 {
        // plateau: use its lower edge so everything at saturation maps to ≥ 1.0
        near as f32
    } else {
        full
    }
}

#[cfg(test)]
pub(crate) fn declare_strip_len(file: &mut [u8], len: u32, declared: u32) {
    let big = file.starts_with(b"MM");
    let entry = |v: u32| -> Vec<u8> {
        if big {
            [&279u16.to_be_bytes()[..], &4u16.to_be_bytes(), &1u32.to_be_bytes(), &v.to_be_bytes()].concat()
        } else {
            [&279u16.to_le_bytes()[..], &4u16.to_le_bytes(), &1u32.to_le_bytes(), &v.to_le_bytes()].concat()
        }
    };
    let (old, new) = (entry(len), entry(declared));
    let at: Vec<usize> = file.windows(old.len()).enumerate().filter(|(_, w)| *w == old.as_slice()).map(|(i, _)| i).collect();
    assert_eq!(at.len(), 1, "one StripByteCounts entry of {len} bytes");
    file[at[0]..at[0] + new.len()].copy_from_slice(&new);
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::tags as t;

    /// `file`, whose image IFD holds one strip, as two strips at the same offset whose StripByteCounts are LONG8 (which
    /// classic TIFF readers accept) `[u64::MAX, 1]`: their sum is past `u64::MAX`.
    pub(super) fn huge_strip_counts(mut file: Vec<u8>) -> Vec<u8> {
        let tiff = Tiff::parse(&file).unwrap();
        let o = tiff.order;
        let ifd = tiff.all_ifds().into_iter().find(|i| i.contains(t::STRIP_OFFSETS)).unwrap();
        let (base, rows, offset) = (ifd.offset as usize, ifd.u32(t::IMAGE_LENGTH).unwrap(), ifd.u32(t::STRIP_OFFSETS).unwrap());
        assert!(rows >= 2);
        let at = file.len() as u32;
        // appended: the two strip offsets (LONG), then the two byte counts (LONG8)
        let mut tail = Vec::new();
        o.put_u32(&mut tail, offset);
        o.put_u32(&mut tail, offset);
        o.put_u64(&mut tail, u64::MAX);
        o.put_u64(&mut tail, 1);
        for i in 0..usize::from(o.read_u16(&file, base).unwrap()) {
            let e = base + 2 + 12 * i;
            let (ty, count, value) = match o.read_u16(&file, e).unwrap() {
                t::STRIP_OFFSETS => (4u16, 2u32, at),
                t::ROWS_PER_STRIP => (4, 1, rows.div_ceil(2)),
                t::STRIP_BYTE_COUNTS => (16, 2, at + 8),
                _ => continue,
            };
            let mut entry = Vec::new();
            o.put_u16(&mut entry, ty);
            o.put_u32(&mut entry, count);
            o.put_u32(&mut entry, value);
            file[e + 2..e + 12].copy_from_slice(&entry);
        }
        file.extend_from_slice(&tail);
        file
    }

    #[test]
    fn black_columns_by_parity() {
        // 8 wide, masked columns 0..4, active starts at x=4 (even)
        let data: Vec<u16> = (0..8 * 6)
            .map(|i| {
                let (x, y) = (i % 8, i / 8);
                if x < 4 { [100, 101, 102, 103][(y % 2) * 2 + x % 2] } else { 5000 }
            })
            .collect();
        let b = black_from_columns(&data, 8, 0..4, 0..6, Rect::new(4, 0, 4, 6));
        assert_eq!(b.values, vec![100.0, 101.0, 102.0, 103.0]);
        let b = black_from_columns(&data, 8, 0..4, 0..6, Rect::new(3, 1, 4, 5));
        assert_eq!(b.at(0, 0, 0, 1), 103.0);
        assert_eq!(black_from_columns(&data, 8, 0..0, 0..6, Rect::new(0, 0, 8, 6)), BlackLevel::uniform(0.0));
    }

    #[test]
    fn white_plateau() {
        let mut d: Vec<u16> = (0..100_000).map(|i| (i % 10000) as u16).collect();
        assert_eq!(white_from_data(&d, 14), 16383.0);
        d.extend(std::iter::repeat_n(15000u16, 1000));
        let w = white_from_data(&d, 14);
        assert!((14900.0..=15000.0).contains(&w), "{w}");
        assert_eq!(white_from_data(&[], 12), 4095.0);
    }

    /// A Canon EOS 70D CR2 saturates at 13583 but has a handful of samples up to 14426 (out of ~20M): the plateau
    /// is still found instead of falling back to 16383 (≈0.3 EV too dark, highlights never seen as clipped).
    /// A plateau on the blue sites alone (odd rows) is still found: an even row step would read one row parity only.
    #[test]
    fn active_rows_reads_both_row_parities() {
        let n = 2048;
        let d: Vec<u16> = (0..n * n).map(|i| if (i / n) % 2 == 1 && i % 2 == 1 { 15000 } else { (i % 10000) as u16 }).collect();
        let w = white_from_data(&active_rows(&d, n, Rect::new(0, 0, n, n)), 14);
        assert!((14900.0..=15000.0).contains(&w), "{w}");
    }

    #[test]
    fn white_plateau_with_strays_above() {
        let mut d: Vec<u16> = (0..100_000).map(|i| (2048 + i % 10000) as u16).collect();
        d.extend(std::iter::repeat_n(13583u16, 5000));
        d.extend([13700, 13900, 14100, 14426]);
        let w = white_from_data(&d, 14);
        assert!((13500.0..=13583.0).contains(&w), "{w}");
    }
}
