//! DNG `ProfileGainTableMap` (tag 52525, DNG 1.6) and `ProfileGainTableMap2` (tag 52544, DNG 1.7):
//! a coarse grid of 1D gain tables carrying the maker's local tone mapping. Apple ProRAW has one.
//!
//! This module reads and writes the tag (a DNG export keeps it); rendering it is the develop pipeline's.
//!
//! What the fields mean (DNG 1.7.1, `ProfileGainTableMap` → "Description"):
//!
//! - the four tables around a position are interpolated bilinearly; outside the grid the edge
//!   tables are replicated. Positions are relative to the active area at pixel centres;
//! - the table input `clamp((R, G, B, min, max) · weights, 0, 1) ^ gamma` is computed in linear
//!   RIMM (ProPhoto) after the baseline exposure;
//! - the gain is the table looked up linearly at `input × MapPointsN` ("multiply the table input
//!   value by MapPointsN to compute the floating-point table index"), clamped to the last point,
//!   and multiplies R, G and B. Results may exceed 1 and aren't clipped.
//!
//! - The grid has `MapPointsV × MapPointsH` tables of `MapPointsN` gains. Its origin and spacing
//!   are relative to the active area (1.0 = the active area's height or width).
//! - The table input is the dot product of (R, G, B, min, max) with the tag's five weights;
//!   version 2 then raises it to the tag's `Gamma`.
//! - Version 2 stores gains as u8 or u16 (mapped linearly onto `GainMin..=GainMax`), f16 or f32.
//!   It takes precedence over version 1 when both are present.
//!
//! Malformed tags are ignored: a wrong size, zero or huge dimensions, non-finite or negative gains,
//! non-positive spacing, or gamma outside 0.25..=4.
//!
//! Byte order: the tag's fields are TIFF types (LONG, DOUBLE, FLOAT, …), and neither DNG 1.6 nor
//! 1.7.1 gives this tag a byte order of its own, as they do for opcode lists and
//! `OriginalRawFileData` (always big-endian). What applies is the general rule of the DNG 1.6
//! specification ("DNG Format Overview" → "Byte Order", worded the same in 1.7.1):
//!
//! > DNG readers are required to support either byte order, even for files from a particular
//! > camera model.
//!
//! (<https://helpx.adobe.com/camera-raw/digital-negative.html>, "DNG Specification".) So we read
//! the fields in the file's byte order, and fall back to big-endian for writers that followed the
//! opcode-list convention. A read in the wrong order can't pass for a valid map: one of
//! `MapPointsV` / `MapPointsH` is below 256 (their product is at most `MAX_TABLES`, 4096), so swapped
//! it is at least 2^24 and the size checks reject it. We write in the file's byte order.

use craft_tiff::ByteOrder;
use serde::{Deserialize, Serialize};

/// Upper bound on `MapPointsV × MapPointsH` (Apple uses 6 × 8).
const MAX_TABLES: usize = 1 << 12;
/// Upper bound on the total number of gains (Apple: 6 × 8 × 257 = 12 336).
const MAX_GAINS: usize = 1 << 22;

/// A parsed `ProfileGainTableMap` / `ProfileGainTableMap2`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GainTableMap {
    /// Tables vertically (`MapPointsV`) and horizontally (`MapPointsH`), and points per table (`MapPointsN`).
    pub points_v: usize,
    pub points_h: usize,
    pub points_n: usize,
    /// Grid spacing and origin, relative to the active area's height (`_v`) and width (`_h`).
    pub spacing_v: f64,
    pub spacing_h: f64,
    pub origin_v: f64,
    pub origin_h: f64,
    /// Weights of (R, G, B, min, max) for the table input.
    pub weights: [f32; 5],
    /// Exponent for the table input (1.0 for the version-1 tag).
    pub gamma: f32,
    /// `points_v × points_h × points_n` gains, row-major with the table points innermost. Integer
    /// storage has already been mapped onto `GainMin..=GainMax`. Mapping before interpolating is
    /// the same as mapping after, because the mapping is linear.
    pub gains: Vec<f32>,
}

impl GainTableMap {
    /// Parse the tag's bytes from a file in byte order `order` (`version2`: `ProfileGainTableMap2`):
    /// in that order, else big-endian (see the module docs). `None` when malformed.
    pub fn parse(b: &[u8], order: ByteOrder, version2: bool) -> Option<GainTableMap> {
        Self::parse_in(b, order, version2).or_else(|| if order == ByteOrder::Big { None } else { Self::parse_in(b, ByteOrder::Big, version2) })
    }

    /// Parse the tag's bytes with every multi-byte field in `order`.
    fn parse_in(b: &[u8], order: ByteOrder, version2: bool) -> Option<GainTableMap> {
        let take = |o: usize, n: usize| b.get(o..o.checked_add(n)?);
        let u32_at = |o: usize| take(o, 4).and_then(|s| s.try_into().ok()).map(|a: [u8; 4]| order.u32(a));
        let f32_at = |o: usize| u32_at(o).map(f32::from_bits);
        let f64_at = |o: usize| take(o, 8).and_then(|s| s.try_into().ok()).map(|a: [u8; 8]| f64::from_bits(order.u64(a)));
        let (points_v, points_h) = (u32_at(0)? as usize, u32_at(4)? as usize);
        let (spacing_v, spacing_h, origin_v, origin_h) = (f64_at(8)?, f64_at(16)?, f64_at(24)?, f64_at(32)?);
        let points_n = u32_at(40)? as usize;
        let mut weights = [0f32; 5];
        for (i, w) in weights.iter_mut().enumerate() {
            *w = f32_at(44 + 4 * i)?;
        }
        let tables = points_v.checked_mul(points_h).filter(|&t| t > 0 && t <= MAX_TABLES)?;
        let count = tables.checked_mul(points_n).filter(|&c| c > 0 && c <= MAX_GAINS)?;
        let spacing_ok = |s: f64, points: usize| s.is_finite() && (s > 0.0 || points == 1);
        if !spacing_ok(spacing_v, points_v)
            || !spacing_ok(spacing_h, points_h)
            || !origin_v.is_finite()
            || !origin_h.is_finite()
            || !weights.iter().all(|w| w.is_finite())
        {
            return None;
        }
        let (header, data_type, gamma, gain_min, gain_max) =
            if version2 { (80, u32_at(64)?, f32_at(68)?, f32_at(72)?, f32_at(76)?) } else { (64, 3, 1.0, 0.0, 1.0) };
        if !(0.25..=4.0).contains(&gamma) {
            return None;
        }
        let width = match data_type {
            0 => 1,
            1 | 2 => 2,
            3 => 4,
            _ => return None,
        };
        if b.len() != count.checked_mul(width)?.checked_add(header)? {
            return None;
        }
        let data = b.get(header..)?;
        let scaled = |q: f32, max: f32| gain_min + q / max * (gain_max - gain_min);
        let gains: Vec<f32> = match data_type {
            0 => data.iter().map(|&q| scaled(q as f32, 255.0)).collect(),
            1 => data.as_chunks::<2>().0.iter().map(|c| scaled(order.u16(*c) as f32, 65535.0)).collect(),
            2 => data.as_chunks::<2>().0.iter().map(|c| crate::unpack::f16_to_f32(order.u16(*c))).collect(),
            _ => data.as_chunks::<4>().0.iter().map(|c| f32::from_bits(order.u32(*c))).collect(),
        };
        if gains.len() != count || !gains.iter().all(|g| g.is_finite() && *g >= 0.0) {
            return None;
        }
        Some(GainTableMap { points_v, points_h, points_n, spacing_v, spacing_h, origin_v, origin_h, weights, gamma, gains })
    }

    /// The tag's bytes in `order`: version 1 (DNG 1.6, f32 gains) when the gamma is 1, else
    /// version 2 with f32 gains. Returns `(version2, bytes)`.
    pub fn to_bytes(&self, order: ByteOrder) -> (bool, Vec<u8>) {
        let version2 = self.gamma != 1.0;
        let mut out = Vec::with_capacity(80 + 4 * self.gains.len());
        let u32b = |v: u32| match order {
            ByteOrder::Big => v.to_be_bytes(),
            ByteOrder::Little => v.to_le_bytes(),
        };
        let f64b = |v: f64| match order {
            ByteOrder::Big => v.to_be_bytes(),
            ByteOrder::Little => v.to_le_bytes(),
        };
        let dim = |v: usize| u32b(u32::try_from(v).unwrap_or(u32::MAX));
        out.extend_from_slice(&dim(self.points_v));
        out.extend_from_slice(&dim(self.points_h));
        for v in [self.spacing_v, self.spacing_h, self.origin_v, self.origin_h] {
            out.extend_from_slice(&f64b(v));
        }
        out.extend_from_slice(&dim(self.points_n));
        for w in self.weights {
            out.extend_from_slice(&u32b(w.to_bits()));
        }
        if version2 {
            // DataType 3 (f32); GainMin/GainMax are ignored for float data
            for v in [3, self.gamma.to_bits(), 0f32.to_bits(), 1f32.to_bits()] {
                out.extend_from_slice(&u32b(v));
            }
        }
        for g in &self.gains {
            out.extend_from_slice(&u32b(g.to_bits()));
        }
        (version2, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tag bytes: the header fields (`v2`: data type, gamma, gain min, gain max), then `data`.
    fn tag(
        order: ByteOrder,
        dims: [u32; 3],
        spacing: [f64; 2],
        origin: [f64; 2],
        weights: [f32; 5],
        v2: Option<(u32, f32, f32, f32)>,
        data: &[u8],
    ) -> Vec<u8> {
        let be = order == ByteOrder::Big;
        let u = |v: u32| if be { v.to_be_bytes() } else { v.to_le_bytes() };
        let d = |v: f64| if be { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut b = Vec::new();
        b.extend_from_slice(&u(dims[0]));
        b.extend_from_slice(&u(dims[1]));
        for v in [spacing[0], spacing[1], origin[0], origin[1]] {
            b.extend_from_slice(&d(v));
        }
        b.extend_from_slice(&u(dims[2]));
        for w in weights {
            b.extend_from_slice(&u(w.to_bits()));
        }
        if let Some((dt, gamma, min, max)) = v2 {
            for v in [dt, gamma.to_bits(), min.to_bits(), max.to_bits()] {
                b.extend_from_slice(&u(v));
            }
        }
        b.extend_from_slice(data);
        b
    }

    fn floats(order: ByteOrder, g: &[f32]) -> Vec<u8> {
        g.iter().flat_map(|v| if order == ByteOrder::Big { v.to_be_bytes() } else { v.to_le_bytes() }).collect()
    }

    const GREEN: [f32; 5] = [0.0, 1.0, 0.0, 0.0, 0.0];

    /// 2 × 2 tables of N = 4 points: table (r, c) = base · [1, 2, 3, 4] with base 1, 2, 3, 4.
    fn grid() -> Vec<f32> {
        (0..4).flat_map(|t| (1..=4).map(move |i| (t + 1) as f32 * i as f32)).collect()
    }

    #[test]
    fn version_2_storage_types_and_gamma() {
        let order = ByteOrder::Little;
        let v2 = |dt: u32, gamma: f32, data: &[u8]| {
            GainTableMap::parse(&tag(order, [1, 1, 4], [1.0, 1.0], [0.0, 0.0], GREEN, Some((dt, gamma, 0.5, 2.5)), data), order, true)
        };
        // u8: 0 → GainMin, 255 → GainMax
        let m = v2(0, 1.0, &[0, 51, 255, 255]).unwrap();
        let want = [0.5, 0.9, 2.5, 2.5];
        assert!(m.gains.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-6), "{:?}", m.gains);
        // u16
        let m = v2(1, 1.0, &[0, 0, 0xff, 0xff, 0, 0, 0xff, 0xff]).unwrap();
        assert_eq!(m.gains, vec![0.5, 2.5, 0.5, 2.5]);
        // f16 ignores GainMin/GainMax: 1.0, 2.0, 0.5, 0
        let m = v2(2, 1.0, &[0x00, 0x3c, 0x00, 0x40, 0x00, 0x38, 0, 0]).unwrap();
        assert_eq!(m.gains, vec![1.0, 2.0, 0.5, 0.0]);
        // f32 with gamma 2
        let m = v2(3, 2.0, &floats(order, &[1.0, 2.0, 3.0, 4.0])).unwrap();
        assert_eq!((m.gains.clone(), m.gamma), (vec![1.0, 2.0, 3.0, 4.0], 2.0));
        // unknown data type, gamma out of range, size for another type: ignored
        assert!(v2(4, 1.0, &[0; 4]).is_none());
        assert!(v2(0, 0.1, &[0; 4]).is_none());
        assert!(v2(0, 5.0, &[0; 4]).is_none());
        assert!(v2(1, 1.0, &[0; 4]).is_none());
        // negative integer range
        let neg = tag(order, [1, 1, 4], [1.0, 1.0], [0.0, 0.0], GREEN, Some((0, 1.0, -1.0, 1.0)), &[0; 4]);
        assert!(GainTableMap::parse(&neg, order, true).is_none());
    }

    #[test]
    fn malformed_tags_are_rejected() {
        let order = ByteOrder::Big;
        let good = tag(order, [2, 2, 4], [0.5, 0.5], [0.25, 0.25], GREEN, None, &floats(order, &grid()));
        assert!(GainTableMap::parse(&good, order, false).is_some());
        // every truncation (and an extra byte)
        for n in 0..good.len() {
            assert!(GainTableMap::parse(&good[..n], order, false).is_none(), "{n} bytes");
        }
        let mut long = good.clone();
        long.push(0);
        assert!(GainTableMap::parse(&long, order, false).is_none());
        // the wrong byte order reads absurd dimensions
        assert!(GainTableMap::parse_in(&good, ByteOrder::Little, false).is_none());
        // read as version 2 the header is too short for the data
        assert!(GainTableMap::parse(&good, order, true).is_none());
        let bad = |dims: [u32; 3], spacing: [f64; 2], origin: [f64; 2], gains: &[f32]| {
            GainTableMap::parse(&tag(order, dims, spacing, origin, GREEN, None, &floats(order, gains)), order, false)
        };
        let mut g = grid();
        g[5] = f32::NAN;
        assert!(bad([2, 2, 4], [0.5, 0.5], [0.0, 0.0], &g).is_none(), "NaN gain");
        g[5] = -1.0;
        assert!(bad([2, 2, 4], [0.5, 0.5], [0.0, 0.0], &g).is_none(), "negative gain");
        g[5] = f32::INFINITY;
        assert!(bad([2, 2, 4], [0.5, 0.5], [0.0, 0.0], &g).is_none(), "infinite gain");
        assert!(bad([2, 2, 4], [0.0, 0.5], [0.0, 0.0], &grid()).is_none(), "zero spacing");
        assert!(bad([2, 2, 4], [0.5, f64::NAN], [0.0, 0.0], &grid()).is_none(), "NaN spacing");
        assert!(bad([2, 2, 4], [0.5, 0.5], [f64::INFINITY, 0.0], &grid()).is_none(), "infinite origin");
        assert!(bad([0, 2, 4], [0.5, 0.5], [0.0, 0.0], &[]).is_none(), "no rows");
        assert!(bad([1, 1, 0], [0.5, 0.5], [0.0, 0.0], &[]).is_none(), "no points");
        // huge dimensions (whose product overflows or exceeds the cap) are rejected before allocating
        assert!(bad([u32::MAX, u32::MAX, u32::MAX], [0.5, 0.5], [0.0, 0.0], &grid()).is_none());
        assert!(bad([4096, 4096, 1], [0.5, 0.5], [0.0, 0.0], &grid()).is_none());
        assert!(bad([1, 1, u32::MAX], [0.5, 0.5], [0.0, 0.0], &grid()).is_none());
        // a single table needs no spacing
        assert!(bad([1, 1, 2], [0.0, 0.0], [0.0, 0.0], &[1.0, 2.0]).is_some());
        let mut w = GREEN;
        w[3] = f32::NAN;
        assert!(GainTableMap::parse(&tag(order, [1, 1, 1], [1.0, 1.0], [0.0, 0.0], w, None, &floats(order, &[1.0])), order, false).is_none());
    }

    /// The payload is read in the file's byte order, else big-endian; a little-endian payload in
    /// a big-endian file is malformed.
    #[test]
    fn byte_order_is_the_files_with_a_big_endian_fallback() {
        use ByteOrder::{Big, Little};
        let v1 = |o: ByteOrder| tag(o, [2, 2, 4], [0.5, 0.5], [0.25, 0.25], GREEN, None, &floats(o, &grid()));
        let v2 = |o: ByteOrder| {
            let mut gains = Vec::new();
            for q in [0u16, 0x1234, 0xffff, 0x8000] {
                gains.extend_from_slice(&if o == Big { q.to_be_bytes() } else { q.to_le_bytes() });
            }
            tag(o, [1, 1, 4], [1.0, 1.0], [0.0, 0.0], GREEN, Some((1, 2.0, 0.5, 2.5)), &gains)
        };
        let (want1, want2) = (GainTableMap::parse_in(&v1(Big), Big, false).unwrap(), GainTableMap::parse_in(&v2(Big), Big, true).unwrap());
        assert_eq!(want1.gains, grid());
        assert_eq!((want2.gains[0], want2.gains[2], want2.gamma), (0.5, 2.5, 2.0));
        // in the file's order
        assert_eq!(GainTableMap::parse(&v1(Little), Little, false).as_ref(), Some(&want1));
        assert_eq!(GainTableMap::parse(&v2(Little), Little, true).as_ref(), Some(&want2));
        assert_eq!(GainTableMap::parse(&v1(Big), Big, false).as_ref(), Some(&want1));
        // a big-endian payload in a little-endian file: the fallback
        assert_eq!(GainTableMap::parse(&v1(Big), Little, false).as_ref(), Some(&want1));
        assert_eq!(GainTableMap::parse(&v2(Big), Little, true).as_ref(), Some(&want2));
        // no little-endian fallback in a big-endian file
        assert!(GainTableMap::parse(&v1(Little), Big, false).is_none());
        assert!(GainTableMap::parse(&v2(Little), Big, true).is_none());
        // whatever the grid, a read in the wrong order fails the size checks
        for dims in [[1, 1, 1], [6, 8, 257], [48, 64, 2], [255, 16, 1], [256, 16, 1], [1, 4096, 1]] {
            for (o, wrong) in [(Big, Little), (Little, Big)] {
                let g = vec![1.0; dims.iter().product::<u32>() as usize];
                let b = tag(o, dims, [0.5, 0.5], [0.0, 0.0], GREEN, None, &floats(o, &g));
                assert!(GainTableMap::parse_in(&b, o, false).is_some(), "{dims:?}");
                assert!(GainTableMap::parse_in(&b, wrong, false).is_none(), "{dims:?} read {wrong:?}");
            }
        }
    }

    #[test]
    fn writes_what_it_reads() {
        for order in [ByteOrder::Big, ByteOrder::Little] {
            let m =
                GainTableMap::parse(&tag(order, [2, 2, 4], [0.5, 0.5], [0.25, 0.25], GREEN, None, &floats(order, &grid())), order, false).unwrap();
            let (v2, b) = m.to_bytes(order);
            assert!(!v2);
            assert_eq!(GainTableMap::parse(&b, order, false).unwrap(), m);
            let g = GainTableMap { gamma: 2.0, ..m };
            let (v2, b) = g.to_bytes(order);
            assert!(v2);
            assert_eq!(GainTableMap::parse(&b, order, true).unwrap(), g);
        }
    }
}
