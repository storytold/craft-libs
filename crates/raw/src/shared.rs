//! Helpers the maker decoders share: the Exif CFA layout, and black and white levels measured from the samples
//! for files that state none.

use crate::{BlackLevel, Cfa, LevelSource, Rect};
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
    black_from_columns_sourced(data, width, cols, rows, active).0
}

/// [`black_from_columns`], saying whether it measured the masked samples or fell back to 0.
pub(crate) fn black_from_columns_sourced(
    data: &[u16],
    width: usize,
    cols: Range<usize>,
    rows: Range<usize>,
    active: Rect,
) -> (BlackLevel, LevelSource) {
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
        return (BlackLevel::uniform(0.0), LevelSource::Fallback);
    }
    let black = BlackLevel {
        repeat_rows: 2,
        repeat_cols: 2,
        values: (0..4).map(|i| (sum[i] / n[i] as f64) as f32).collect(),
        delta_h: vec![],
        delta_v: vec![],
    };
    (black, LevelSource::MaskedPixels)
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
    white_from_data_sourced(data, bits).0
}

/// [`white_from_data`], saying whether it found a plateau ([`LevelSource::Inferred`]) or returned the full range.
pub(crate) fn white_from_data_sourced(data: &[u16], bits: u32) -> (f32, LevelSource) {
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
        return (full, LevelSource::Fallback);
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
        (near as f32, LevelSource::Inferred)
    } else {
        (full, LevelSource::Fallback)
    }
}

#[cfg(any(feature = "rw2", feature = "arw"))]
/// Output radii sampled to fit a radial warp.
const DISTORTION_SAMPLES: usize = 64;

#[cfg(any(feature = "rw2", feature = "arw"))]
/// Least-squares radial warp from a camera's forward distortion map.
///
/// `corrected` maps a source radius to the output radius it lands at; it must rise monotonically up to `t_max`, the
/// output radius of the image corners. Radii use the caller's unit; pass `WarpRectilinear`'s normalisation (distance
/// from the centre to the **farthest** corner = 1) so the result is directly a `WarpRectilinear` plane. The map is
/// scanned at 4096 points up to a source radius of `3 * t_max`: it must reach `t_max` within that range and rise at
/// every scan point (monotonicity is checked only there), otherwise `None`; a NaN at a scan point is rejected the
/// same way, other non-finite values by the fit. The map is inverted by bisection and
/// `source = u·(k0 + k1u² + k2u⁴ + k3u⁶)` is fitted over output radii `u·t_max`, `u` in [0, 1] ([`fit_radial_warp`]):
/// the returned `[k0, k1, k2, k3]` give the source radius in the caller's unit as a function of `u`. `None` too when
/// the worst fit error exceeds `max_fit_error`.
pub(crate) fn warp_from_forward_map(corrected: &dyn Fn(f64) -> f64, t_max: f64, max_fit_error: f64) -> Option<[f64; 4]> {
    if !(t_max.is_finite() && t_max > 0.0) {
        return None;
    }
    let mut top = None;
    let mut prev = 0.0;
    for i in 1..=4096 {
        let r = 3.0 * t_max * i as f64 / 4096.0;
        let v = corrected(r);
        if v.is_nan() || v <= prev {
            return None;
        }
        prev = v;
        if v >= t_max {
            top = Some(r);
            break;
        }
    }
    let top = top?;
    let source = |t: f64| {
        let (mut lo, mut hi) = (0.0, top);
        for _ in 0..60 {
            let m = 0.5 * (lo + hi);
            if corrected(m) < t { lo = m } else { hi = m }
        }
        0.5 * (lo + hi)
    };
    fit_radial_warp(&|u| source(u * t_max), max_fit_error, 1.0)
}

#[cfg(any(feature = "rw2", feature = "arw"))]
/// Least-squares `source = u·(k0 + k1u² + k2u⁴ + k3u⁶)` through `source_of_u` sampled at `u = i/64`, i = 0..=64
/// (`u` = output radius, `WarpRectilinear` units, so the result is directly a plane's `[k0, k1, k2, k3]`). `None` when
/// a sample is not finite, the normal equations are singular, or the worst fit error at the samples up to the first
/// one at or beyond `gate_until` (the radii the image shows; 1 = all) exceeds `max_fit_error`.
pub(crate) fn fit_radial_warp(source_of_u: &dyn Fn(f64) -> f64, max_fit_error: f64, gate_until: f64) -> Option<[f64; 4]> {
    let samples: Vec<(f64, f64)> = (0..=DISTORTION_SAMPLES)
        .map(|i| {
            let u = i as f64 / DISTORTION_SAMPLES as f64;
            (u, source_of_u(u))
        })
        .collect();
    if samples.iter().any(|&(_, v)| !v.is_finite()) {
        return None;
    }
    let basis = |u: f64| {
        let u2 = u * u;
        [u, u * u2, u * u2 * u2, u * u2 * u2 * u2]
    };
    let mut a = [[0.0; 4]; 4];
    let mut rhs = [0.0; 4];
    for &(u, v) in &samples {
        let f = basis(u);
        for (i, fi) in f.iter().enumerate() {
            for (j, fj) in f.iter().enumerate() {
                a[i][j] += fi * fj;
            }
            rhs[i] += fi * v;
        }
    }
    let k = solve4(a, rhs)?;
    let fit = |u: f64| basis(u).iter().zip(&k).map(|(f, c)| f * c).sum::<f64>();
    // rounded up to a sample, so the band between the last sample inside and `gate_until` itself is checked too;
    // capped at 1 (`min` also turns a NaN into 1: every sample is gated)
    let n = DISTORTION_SAMPLES as f64;
    let gate_until = (gate_until * n).ceil().min(n) / n;
    let worst = samples.iter().filter(|&&(u, _)| u <= gate_until).map(|&(u, v)| (fit(u) - v).abs()).fold(0.0, f64::max);
    (worst <= max_fit_error).then_some(k)
}

#[cfg(any(feature = "rw2", feature = "arw"))]
/// Solve the 4 × 4 system `a·x = b` (Gaussian elimination with partial pivoting); `None` when singular or not finite.
fn solve4(mut a: [[f64; 4]; 4], mut b: [f64; 4]) -> Option<[f64; 4]> {
    for c in 0..4 {
        let p = (c..4).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].is_nan() || a[p][c].abs() <= 1e-300 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..4 {
            let f = a[r][c] / a[c][c];
            let (top, bottom) = a.split_at_mut(r);
            for (x, p) in bottom[0][c..].iter_mut().zip(&top[c][c..]) {
                *x -= f * p;
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = [0.0; 4];
    for c in (0..4).rev() {
        let s: f64 = (c + 1..4).map(|k| a[c][k] * x[k]).sum();
        x[c] = (b[c] - s) / a[c][c];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
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
pub(crate) mod tests {
    use super::*;
    use craft_tiff::tags as t;

    /// `file`, whose image IFD holds one strip, as two strips at the same offset whose StripByteCounts are LONG8 (which
    /// classic TIFF readers accept) `[u64::MAX, 1]`: their sum is past `u64::MAX`.
    pub(crate) fn huge_strip_counts(mut file: Vec<u8>) -> Vec<u8> {
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

    /// The sourced variants report how a level was found and return exactly what the plain ones do.
    #[test]
    fn level_sources_say_how_black_and_white_were_found() {
        let data: Vec<u16> = (0..8 * 6).map(|i| if i % 8 < 4 { 100 } else { 5000 }).collect();
        let active = Rect::new(4, 0, 4, 6);
        let (b, src) = black_from_columns_sourced(&data, 8, 0..4, 0..6, active);
        assert_eq!((b, src), (black_from_columns(&data, 8, 0..4, 0..6, active), LevelSource::MaskedPixels));
        let (b, src) = black_from_columns_sourced(&data, 8, 0..0, 0..6, active);
        assert_eq!((b, src), (BlackLevel::uniform(0.0), LevelSource::Fallback));

        let mut d: Vec<u16> = (0..100_000).map(|i| (i % 10000) as u16).collect();
        assert_eq!(white_from_data_sourced(&d, 14), (16383.0, LevelSource::Fallback));
        assert_eq!(white_from_data_sourced(&[], 12), (4095.0, LevelSource::Fallback));
        d.extend(std::iter::repeat_n(15000u16, 1000));
        let (w, src) = white_from_data_sourced(&d, 14);
        assert_eq!((w, src), (white_from_data(&d, 14), LevelSource::Inferred));
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

    #[cfg(any(feature = "rw2", feature = "arw"))]
    #[test]
    fn warp_from_forward_map_recovers_identity_and_rejects_folds() {
        let k = warp_from_forward_map(&|r| r, 1.0, 1e-9).unwrap();
        assert!((k[0] - 1.0).abs() < 1e-9 && k[1].abs() < 1e-9 && k[2].abs() < 1e-9 && k[3].abs() < 1e-9);
        let barrel = |r: f64| r * (1.0 + 0.05 * r * r); // mild barrel: source < output radius
        let t_max = 0.9;
        let k = warp_from_forward_map(&barrel, t_max, 1e-3).unwrap();
        assert!(k[0] < t_max && k[0] > 0.8 && k[1] < 0.0); // source radius at u = 1 is below t_max
        for i in 0..=20 {
            let u = i as f64 / 20.0;
            let s = u * (k[0] + u * u * (k[1] + u * u * (k[2] + u * u * k[3])));
            assert!((barrel(s) - u * t_max).abs() < 1e-5, "u {u}");
        }
        assert!(warp_from_forward_map(&|r| if r < 0.5 { r } else { 0.5 - (r - 0.5) }, 1.0, 1e-3).is_none()); // fold, never reaches t_max
        // a tiny dip (two scan points wide, 1e-4 deep) that still reaches t_max and fits well: only the monotonicity check rejects it
        let dip = |r: f64| {
            if r < 0.5 {
                r
            } else if r < 0.5015 {
                0.5 - 0.05 * (r - 0.5)
            } else {
                r - 0.001575
            }
        };
        assert!(warp_from_forward_map(&dip, 1.0, 1e-2).is_none());
        assert!(warp_from_forward_map(&|_| f64::NAN, 1.0, 1e-3).is_none());
    }

    #[cfg(any(feature = "rw2", feature = "arw"))]
    #[test]
    fn fit_radial_warp_identity_and_gate() {
        let k = fit_radial_warp(&|u| u, 1e-9, 1.0).unwrap();
        assert!((k[0] - 1.0).abs() < 1e-9 && k[1].abs() < 1e-9 && k[2].abs() < 1e-9 && k[3].abs() < 1e-9, "{k:?}");
        assert!(fit_radial_warp(&|u| if u > 0.5 { f64::NAN } else { u }, 1.0, 1.0).is_none(), "a NaN sample");
        assert!(fit_radial_warp(&|u| if u > 0.5 { f64::INFINITY } else { u }, 1.0, 1.0).is_none(), "an infinite sample");
        assert!(fit_radial_warp(&|u: f64| u.sqrt(), 1.0, 1.0).is_some());
        assert!(fit_radial_warp(&|u: f64| u.sqrt(), 0.0, 1.0).is_none(), "not a polynomial: the gate rejects it");
        // a step at u = 1 alone: worst fit error 0.057 over all samples, 0.006 over u <= 0.9 (the fit is the same)
        let step = |u: f64| if u > 0.99 { 1.1 } else { u };
        assert!(fit_radial_warp(&step, 0.01, 1.0).is_none());
        assert_eq!(fit_radial_warp(&step, 0.01, 0.9), fit_radial_warp(&step, 1.0, 1.0), "gated only where shown");
        // the first sample at or beyond `gate_until` is gated too (the band up to it is shown): 0.023 up to
        // u = 62/64, 0.032 at 63/64
        assert!(fit_radial_warp(&step, 0.03, 62.0 / 64.0).is_some(), "a sample exactly at the bound");
        assert!(fit_radial_warp(&step, 0.03, 0.98).is_none(), "63/64 is the next sample beyond 0.98");
        // capped at 1, and a NaN gates every sample (fails closed)
        assert_eq!(fit_radial_warp(&step, 1.0, f64::INFINITY), fit_radial_warp(&step, 1.0, 1.0));
        assert!(fit_radial_warp(&step, 0.01, f64::NAN).is_none(), "a NaN gate");
    }
}
