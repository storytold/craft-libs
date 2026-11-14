//! The 8×8 forward and inverse DCT of ISO/IEC 14496-2 Annex A, evaluated separably in `f64`.
//! The inverse rounds to the nearest integer and saturates to [-256, 255] (§7.4.4), well inside
//! the IEEE Std 1180-1990 accuracy the standard asks of a decoder's IDCT, so a conforming
//! decoder's reconstruction stays within a sample value of the encoder's.
//!
//! [`idct_xvid`] is a fixed-point inverse that gives the same samples as the IDCT of Xvid's
//! x86 builds, which encoded most Xvid streams. An encoder's reconstruction drifts from a
//! decoder that rounds differently, a little more with each P-VOP, so streams that name Xvid as
//! their encoder are decoded with it.

use std::sync::OnceLock;

/// `M[x][u] = C(u)/2 · cos((2x+1)uπ/16)`, `C(0) = 1/√2`, `C(u>0) = 1`.
fn basis() -> &'static [[f64; 8]; 8] {
    static B: OnceLock<[[f64; 8]; 8]> = OnceLock::new();
    B.get_or_init(|| {
        let mut m = [[0f64; 8]; 8];
        for (x, row) in m.iter_mut().enumerate() {
            for (u, v) in row.iter_mut().enumerate() {
                let c = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                *v = c / 2.0 * (((2 * x + 1) * u) as f64 * std::f64::consts::PI / 16.0).cos();
            }
        }
        m
    })
}

/// Forward DCT of 64 samples (raster order) into rounded coefficients.
#[cfg(any(feature = "mpeg4-encode", test))]
pub(super) fn fdct(px: &[i32; 64], out: &mut [i32; 64]) {
    let m = basis();
    // rows: tmp[y][u] = Σ_x M[x][u] f[y][x]
    let mut tmp = [[0f64; 8]; 8];
    for y in 0..8 {
        for u in 0..8 {
            tmp[y][u] = (0..8).map(|x| m[x][u] * f64::from(px[y * 8 + x])).sum();
        }
    }
    for v in 0..8 {
        for u in 0..8 {
            let s: f64 = (0..8).map(|y| m[y][v] * tmp[y][u]).sum();
            out[v * 8 + u] = s.round() as i32;
        }
    }
}

/// In-place inverse DCT of dequantised coefficients (raster order), saturated to [-256, 255].
pub(super) fn idct(blk: &mut [i32; 64]) {
    if blk[1..].iter().all(|&c| c == 0) {
        // F[0][0]/8 everywhere, rounded half away from zero.
        let d = blk[0];
        let v = if d >= 0 { (d + 4) >> 3 } else { -((-d + 4) >> 3) };
        blk.fill(v.clamp(-256, 255));
        return;
    }
    let m = basis();
    // Each sum adds its terms in the order of the plain `Σ`, skipping zero coefficients and rows:
    // a skipped term is ±0, so the result is the same, and most blocks are sparse.
    // rows: tmp[v][x] = Σ_u M[x][u] F[v][u]
    let mut tmp = [[0f64; 8]; 8];
    let mut rows = [0usize; 8];
    let mut nrows = 0;
    for (v, t) in tmp.iter_mut().enumerate() {
        let r = &blk[v * 8..v * 8 + 8];
        if r.iter().all(|&c| c == 0) {
            continue;
        }
        rows[nrows] = v;
        nrows += 1;
        for (u, &c) in r.iter().enumerate() {
            if c != 0 {
                let c = f64::from(c);
                for (x, t) in t.iter_mut().enumerate() {
                    *t += m[x][u] * c;
                }
            }
        }
    }
    for y in 0..8 {
        let mut acc = [0f64; 8];
        for &v in &rows[..nrows] {
            let k = m[y][v];
            for (a, &t) in acc.iter_mut().zip(&tmp[v]) {
                *a += k * t;
            }
        }
        for (o, a) in blk[y * 8..y * 8 + 8].iter_mut().zip(acc) {
            *o = (a.round() as i32).clamp(-256, 255);
        }
    }
}

#[cfg(feature = "mpeg4-decode")]
pub(super) use xvid::idct_xvid;

/// The IDCT of Xvid's x86 builds (see the module documentation).
#[cfg(feature = "mpeg4-decode")]
mod xvid {
    use std::sync::OnceLock;

    /// Row pre-scale of [`idct_xvid`]: row `k` of coefficients is multiplied by the factor the
    /// column pass leaves out of its input `k` (`cos(π/4)` for inputs 0 and 4, `cos(kπ/16)` for the
    /// others), so the column pass needs only tangents and one `cos(π/4)`.
    const XVID_ROW_SCALE: [u32; 8] = [4, 1, 2, 3, 4, 3, 2, 1];

    /// Per-row rounding constants of [`idct_xvid`]'s row pass, measured against Xvid's IDCT. Row
    /// 0's also holds the column pass's rounding (`32 << 11`): input 0 reaches every output of the
    /// column pass with weight one.
    const XVID_ROUNDER: [i64; 8] = [65536, 3597, 2260, 1203, 0, 120, 512, 512];

    /// `round(2^16 · tan(kπ/16))` for k = 1, 2, 3, and `round(2^15 · cos(π/4))`.
    const XVID_TAN1: i64 = 13036;
    const XVID_TAN2: i64 = 27146;
    const XVID_TAN3: i64 = 43790;
    const XVID_COS4_HALF: i64 = 23170;

    /// `T[s][n][u] = round(2^15 · cos(sπ/16) · C(u) · cos((2n+1)uπ/16))` for the four row scales
    /// (`s` = 4, 1, 2, 3 at index 0..3) and outputs `n` = 0..3; outputs 4..7 follow by symmetry.
    fn xvid_row_table() -> &'static [[[i64; 8]; 4]; 4] {
        static T: OnceLock<[[[i64; 8]; 4]; 4]> = OnceLock::new();
        T.get_or_init(|| {
            let mut t = [[[0i64; 8]; 4]; 4];
            for (si, scale) in [4u32, 1, 2, 3].into_iter().enumerate() {
                let pre = (f64::from(scale) * std::f64::consts::PI / 16.0).cos();
                for (n, out) in t[si].iter_mut().enumerate() {
                    for (u, e) in out.iter_mut().enumerate() {
                        let c = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                        let v = pre * c * (((2 * n + 1) * u) as f64 * std::f64::consts::PI / 16.0).cos();
                        *e = (v * 32768.0).round() as i64;
                    }
                }
            }
            t
        })
    }

    fn sat16(v: i64) -> i64 {
        v.clamp(-32768, 32767)
    }

    /// `(a · t) >> 16`: the high half of a 16×16-bit product.
    fn mulhi(a: i64, t: i64) -> i64 {
        (a * t) >> 16
    }

    /// In-place inverse DCT matching Xvid's x86 IDCT bit for bit; the output is in [-512, 511].
    ///
    /// A row pass multiplies each row by a table (pre-scaled per row, see [`XVID_ROW_SCALE`]) and
    /// keeps 16 bits after a shift by 11. A column pass factors the 8-point transform into
    /// rotations by `tan(π/16)`, `tan(π/8)` and `tan(3π/16)`, with the `cos(π/4)` products taken at
    /// half scale and doubled, saturating every sum to 16 bits, and shifts by 6.
    pub(in crate::mpeg4) fn idct_xvid(blk: &mut [i32; 64]) {
        let t = xvid_row_table();
        let scale_index = |k: usize| match XVID_ROW_SCALE[k] {
            4 => 0,
            s => s as usize,
        };
        let mut y = [0i64; 64];
        for (k, out) in y.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            let row: [i64; 8] = std::array::from_fn(|u| i64::from(blk[k * 8 + u].clamp(-32768, 32767)));
            let r = XVID_ROUNDER[k];
            if row.iter().all(|&c| c == 0) {
                out.fill(sat16(r >> 11));
                continue;
            }
            let tk = &t[scale_index(k)];
            for n in 0..4 {
                let even: i64 = [0, 2, 4, 6].iter().map(|&u| tk[n][u] * row[u]).sum();
                let odd: i64 = [1, 3, 5, 7].iter().map(|&u| tk[n][u] * row[u]).sum();
                out[n] = sat16((even + odd + r) >> 11);
                out[7 - n] = sat16((even - odd + r) >> 11);
            }
        }
        for c in 0..8 {
            let x: [i64; 8] = std::array::from_fn(|k| y[k * 8 + c]);
            let tp17 = sat16(x[1] + mulhi(x[7], XVID_TAN1));
            let tm17 = sat16(mulhi(x[1], XVID_TAN1) - x[7]);
            let tp35 = sat16(x[3] + mulhi(x[5], XVID_TAN3));
            let tm35 = sat16(mulhi(x[3], XVID_TAN3) - x[5]);
            let b0 = sat16(tp17 + tp35);
            let b3 = sat16(tm17 - tm35);
            let u = sat16(tp17 - tp35);
            let v = sat16(tm17 + tm35);
            let b1 = sat16(2 * mulhi(sat16(u + v), XVID_COS4_HALF));
            let b2 = sat16(2 * mulhi(sat16(u - v), XVID_COS4_HALF));
            let tp26 = sat16(x[2] + mulhi(x[6], XVID_TAN2));
            let tm26 = sat16(mulhi(x[2], XVID_TAN2) - x[6]);
            let tp04 = sat16(x[0] + x[4]);
            let tm04 = sat16(x[0] - x[4]);
            let a0 = sat16(tp04 + tp26);
            let a3 = sat16(tp04 - tp26);
            let a1 = sat16(tm04 + tm26);
            let a2 = sat16(tm04 - tm26);
            let o = [a0 + b0, a1 + b1, a2 + b2, a3 + b3, a3 - b3, a2 - b2, a1 - b1, a0 - b0];
            for (n, o) in o.into_iter().enumerate() {
                blk[n * 8 + c] = (sat16(o) >> 6) as i32;
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The column constants are the formulas they are documented as.
        #[test]
        fn xvid_constants_follow_from_their_formulas() {
            let pi = std::f64::consts::PI;
            assert_eq!(XVID_TAN1, ((pi / 16.0).tan() * 65536.0).round() as i64);
            assert_eq!(XVID_TAN2, ((pi / 8.0).tan() * 65536.0).round() as i64);
            assert_eq!(XVID_TAN3, ((3.0 * pi / 16.0).tan() * 65536.0).round() as i64);
            assert_eq!(XVID_COS4_HALF, ((pi / 4.0).cos() * 32768.0).round() as i64);
            // The first table entries of the four row scales.
            let t = xvid_row_table();
            assert_eq!([t[0][0][0], t[1][0][0], t[2][0][0], t[3][0][0]], [16384, 22725, 21407, 19266]);
        }

        /// Two blocks whose samples from Xvid's IDCT differ from the exactly rounded transform.
        #[test]
        fn xvid_idct_matches_xvid_where_exact_rounding_does_not() {
            type Case = (&'static [(usize, i32)], [i32; 64]);
            let cases: [Case; 2] = [
                (
                    &[(1, -7), (3, -46), (6, -67), (14, -39), (17, -87), (18, -71)],
                    [
                        -51, -3, -17, 24, 23, -10, 30, 4, -30, 9, -13, 17, 11, -19, 22, 2, 0, 26, -7, 6, -6, -30, 11, 0, 22, 36, -1, -2, -18, -37, 2,
                        -2, 23, 33, 3, -3, -20, -34, -2, 0, 4, 16, 3, 2, -10, -20, 1, 4, -24, -6, 2, 10, 5, -4, 7, 8, -44, -20, 0, 16, 16, 8, 12, 12,
                    ],
                ),
                (
                    &[(0, 27), (1, -61), (3, -8), (4, -7), (6, 9), (14, -3)],
                    [
                        -9, -5, 0, 1, 4, 10, 12, 15, -9, -5, 1, 1, 3, 10, 12, 15, -9, -6, 1, 1, 3, 10, 12, 15, -9, -6, 1, 1, 3, 10, 12, 15, -9, -6,
                        1, 1, 3, 10, 11, 15, -8, -6, 1, 0, 3, 11, 11, 15, -8, -6, 2, 0, 3, 11, 11, 15, -8, -7, 2, 0, 3, 11, 11, 15,
                    ],
                ),
            ];
            for (coeffs, expected) in cases {
                let mut blk = [0i32; 64];
                for &(i, c) in coeffs {
                    blk[i] = c;
                }
                let mut exact = blk;
                super::super::idct(&mut exact);
                assert_ne!(exact, expected);
                idct_xvid(&mut blk);
                assert_eq!(blk, expected);
            }
        }

        /// Extreme coefficients saturate instead of overflowing.
        #[test]
        fn xvid_idct_of_extreme_coefficients_stays_in_range() {
            for c in [i32::MIN, -2048, 2047, i32::MAX] {
                let mut blk = [c; 64];
                idct_xvid(&mut blk);
                assert!(blk.iter().all(|v| (-512..=511).contains(v)), "{c}: {blk:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_undoes_forward() {
        let mut x = 0x1234_5678u32;
        for _ in 0..200 {
            let px: [i32; 64] = std::array::from_fn(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x % 256) as i32
            });
            let mut c = [0; 64];
            fdct(&px, &mut c);
            idct(&mut c);
            for (a, b) in px.iter().zip(&c) {
                assert!((a - b).abs() <= 1, "{a} vs {b}");
            }
        }
    }

    #[test]
    fn flat_block_has_only_dc() {
        let mut c = [0; 64];
        fdct(&[100; 64], &mut c);
        assert_eq!(c[0], 800);
        assert!(c[1..].iter().all(|&v| v == 0));
    }
}
