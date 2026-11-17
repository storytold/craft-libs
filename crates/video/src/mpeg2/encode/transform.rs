//! Forward DCT, quantisation and the decoder-exact reconstruction of one block.

use crate::mpeg2::idct::{basis, idct};
use crate::mpeg2::slice::dequant_mpeg2;

/// Forward 8×8 DCT of `p` (raster order), the transpose of the decoder's IDCT basis:
/// `F[v][u] = Σ_y Σ_x M[x][u] M[y][v] p[y][x]`, so a flat block of value `a` has `F[0][0] = 8a`.
pub(crate) fn fdct(p: &[i32; 64]) -> [f32; 64] {
    let m = basis();
    let mut t = [0f32; 64];
    for y in 0..8 {
        let row: [f32; 8] = std::array::from_fn(|x| p[y * 8 + x] as f32);
        for u in 0..8 {
            let mut s = 0f32;
            for x in 0..8 {
                s += m[x][u] * row[x];
            }
            t[y * 8 + u] = s;
        }
    }
    let mut out = [0f32; 64];
    for v in 0..8 {
        for u in 0..8 {
            let mut s = 0f32;
            for y in 0..8 {
                s += m[y][v] * t[y * 8 + u];
            }
            out[v * 8 + u] = s;
        }
    }
    out
}

/// Rounding offset of intra AC quantisation (in units of the quantiser step): below one half,
/// a small dead zone.
const INTRA_ROUND: f32 = 0.375;

/// Quantise intra AC coefficient `f` with weight `w` and quantiser_scale `qs` (inverse of
/// `F = 2·QF·w·qs / 32`).
#[inline]
pub(crate) fn quant_intra(f: f32, w: u8, qs: i32) -> i32 {
    let y = 16.0 * f.abs() / (f32::from(w) * qs as f32);
    let q = ((y + INTRA_ROUND).floor() as i32).min(2047);
    if f < 0.0 { -q } else { q }
}

/// Quantise non-intra coefficient `f` (inverse of `F = (2·QF + sign)·w·qs / 32`): truncation,
/// which leaves the usual wide dead zone around zero.
#[inline]
pub(crate) fn quant_inter(f: f32, w: u8, qs: i32) -> i32 {
    let y = 16.0 * f.abs() / (f32::from(w) * qs as f32);
    let q = (y.floor() as i32).min(2047);
    if f < 0.0 { -q } else { q }
}

/// Reconstruct a block as the decoder does: dequantise `levels` (raster order; for intra blocks
/// `levels[0]` is the quantised DC), mismatch control, IDCT. Returns the residual (inter) or
/// the samples before clamping (intra).
pub(crate) fn reconstruct(levels: &[i32; 64], matrix: &[u8; 64], qs: i32, intra: bool, dc_mult: i32) -> [i32; 64] {
    let mut blk = [0i32; 64];
    let mut sum = 0i32;
    for (i, &l) in levels.iter().enumerate() {
        if l == 0 {
            continue;
        }
        let v = if intra && i == 0 { l * dc_mult } else { dequant_mpeg2(l, qs, i32::from(matrix[i]), intra) };
        blk[i] = v;
        sum += v;
    }
    if sum & 1 == 0 {
        blk[63] ^= 1;
    }
    let dc_only = blk[1..].iter().all(|&v| v == 0);
    idct(&mut blk, dc_only);
    blk
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_block_has_only_dc() {
        let f = fdct(&[100; 64]);
        assert!((f[0] - 800.0).abs() < 0.01);
        assert!(f[1..].iter().all(|v| v.abs() < 0.01));
    }

    #[test]
    fn forward_then_inverse_is_close() {
        let p: [i32; 64] = std::array::from_fn(|i| ((i * 37 + i / 8 * 11) % 200) as i32 - 60);
        let f = fdct(&p);
        let mut b: [i32; 64] = std::array::from_fn(|i| f[i].round() as i32);
        idct(&mut b, false);
        for i in 0..64 {
            assert!((b[i] - p[i]).abs() <= 1, "{i}: {} vs {}", b[i], p[i]);
        }
    }

    #[test]
    fn quantisers_invert_the_dequantiser() {
        for qs in [1, 2, 8, 31, 62, 112] {
            for w in [8u8, 16, 40, 83] {
                // only where the dequantiser's integer division is exact
                if (i32::from(w) * qs) % 32 != 0 {
                    continue;
                }
                for level in [-40, -3, -1, 1, 2, 17] {
                    let f = dequant_mpeg2(level, qs, i32::from(w), true) as f32;
                    // without the clamp the intra quantiser returns the level it came from
                    if f.abs() < 2047.0 {
                        assert_eq!(quant_intra(f, w, qs), level, "intra qs {qs} w {w}");
                    }
                    let f = dequant_mpeg2(level, qs, i32::from(w), false) as f32;
                    if f.abs() < 2047.0 {
                        assert_eq!(quant_inter(f, w, qs), level, "inter qs {qs} w {w}");
                    }
                }
            }
        }
    }
}
