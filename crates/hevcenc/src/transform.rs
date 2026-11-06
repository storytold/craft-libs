//! Forward transforms (encoder side, any integer approximation is allowed), quantisation, and the
//! normative scaling (8.6.3) and inverse transforms (8.6.4.2) the decoder applies. The inverse path
//! is exact integer arithmetic: the even/odd decomposition computes the same sums as the matrix
//! products of the specification.

use crate::tables::{DST_MATRIX, LEVEL_SCALE, TRANS_MATRIX};

#[inline]
fn m(n: usize, k: usize, i: usize) -> i32 {
    // k-th basis function of the n-point DCT, sample i
    TRANS_MATRIX[k * (32 / n)][i] as i32
}

/// 1-D forward DCT of `x` (length n) into `out`.
fn fdct1d(x: &[i32], out: &mut [i32], n: usize) {
    if n <= 4 {
        for k in 0..n {
            let mut s = 0;
            for i in 0..n {
                s += m(n, k, i) * x[i];
            }
            out[k] = s;
        }
        return;
    }
    let h = n / 2;
    let mut e = [0i32; 16];
    let mut o = [0i32; 16];
    for i in 0..h {
        e[i] = x[i] + x[n - 1 - i];
        o[i] = x[i] - x[n - 1 - i];
    }
    let mut ev = [0i32; 16];
    fdct1d(&e[..h], &mut ev[..h], h);
    for k in 0..h {
        out[2 * k] = ev[k];
        let kk = 2 * k + 1;
        let mut s = 0;
        for i in 0..h {
            s += m(n, kk, i) * o[i];
        }
        out[kk] = s;
    }
}

/// 1-D inverse DCT: x[i] = sum_k M[k][i] * c[k].
fn idct1d(c: &[i32], out: &mut [i32], n: usize) {
    if n <= 4 {
        for i in 0..n {
            let mut s = 0;
            for k in 0..n {
                s += m(n, k, i) * c[k];
            }
            out[i] = s;
        }
        return;
    }
    let h = n / 2;
    let mut ce = [0i32; 16];
    for k in 0..h {
        ce[k] = c[2 * k];
    }
    let mut e = [0i32; 16];
    idct1d(&ce[..h], &mut e[..h], h);
    for i in 0..h {
        let mut o = 0;
        let mut k = 1;
        while k < n {
            let v = c[k];
            if v != 0 {
                o += m(n, k, i) * v;
            }
            k += 2;
        }
        out[i] = e[i] + o;
        out[n - 1 - i] = e[i] - o;
    }
}

fn fdst1d(x: &[i32], out: &mut [i32]) {
    for k in 0..4 {
        out[k] = (0..4).map(|i| DST_MATRIX[k][i] as i32 * x[i]).sum();
    }
}

fn idst1d(c: &[i32], out: &mut [i32]) {
    for i in 0..4 {
        out[i] = (0..4).map(|k| DST_MATRIX[k][i] as i32 * c[k]).sum();
    }
}

/// Forward 2-D transform of the n x n residual `r` (row-major) into `c`.
pub fn forward(r: &[i32], c: &mut [i32], n: usize, dst: bool, bit_depth: u32) {
    let log2 = n.trailing_zeros();
    let shift1 = log2 + bit_depth - 9;
    let shift2 = log2 + 6;
    let (r1, r2) = (1i32 << (shift1 - 1), 1i32 << (shift2 - 1));
    let mut tmp = vec![0i32; n * n];
    let mut out = [0i32; 32];
    // rows (horizontal frequencies)
    for y in 0..n {
        let row = &r[y * n..y * n + n];
        if dst {
            fdst1d(row, &mut out);
        } else {
            fdct1d(row, &mut out, n);
        }
        for k in 0..n {
            tmp[k * n + y] = (out[k] + r1) >> shift1; // transposed: tmp[kx][y]
        }
    }
    // columns
    let mut col = [0i32; 32];
    for kx in 0..n {
        col[..n].copy_from_slice(&tmp[kx * n..kx * n + n]);
        if dst {
            fdst1d(&col, &mut out);
        } else {
            fdct1d(&col, &mut out, n);
        }
        for ky in 0..n {
            c[ky * n + kx] = (out[ky] + r2) >> shift2;
        }
    }
}

/// Inverse 2-D transform (8.6.4.2) of scaled coefficients `d` in place, producing residuals.
pub fn inverse(d: &mut [i32], n: usize, dst: bool, bit_depth: u32) {
    let bd_shift = 20 - bit_depth;
    let mut tmp = vec![0i32; n * n];
    let mut col = [0i32; 32];
    let mut out = [0i32; 32];
    // max non-zero row, to skip empty work
    let mut nz_cols = [false; 32];
    for y in 0..n {
        for x in 0..n {
            if d[y * n + x] != 0 {
                nz_cols[x] = true;
            }
        }
    }
    // 1. columns
    for x in 0..n {
        if !nz_cols[x] {
            continue; // tmp column stays zero
        }
        for j in 0..n {
            col[j] = d[j * n + x];
        }
        if dst {
            idst1d(&col, &mut out);
        } else {
            idct1d(&col, &mut out, n);
        }
        for y in 0..n {
            tmp[y * n + x] = ((out[y] + 64) >> 7).clamp(-32768, 32767);
        }
    }
    // 2. rows
    let round = 1 << (bd_shift - 1);
    for y in 0..n {
        let row = &tmp[y * n..y * n + n];
        if dst {
            idst1d(row, &mut out);
        } else {
            idct1d(row, &mut out, n);
        }
        for x in 0..n {
            d[y * n + x] = (out[x] + round) >> bd_shift;
        }
    }
}

const QUANT_SCALE: [i64; 6] = [26214, 23302, 20560, 18396, 16384, 14564];

/// Quantise transform coefficients `c` into levels `l` with a dead-zone rounding offset
/// (`intra`: 1/3, otherwise 1/6). `qp` includes QpBdOffset. Returns the number of non-zero levels.
pub fn quantize(c: &[i32], l: &mut [i32], n: usize, qp: i32, intra: bool, bit_depth: u32) -> usize {
    let log2 = n.trailing_zeros() as i32;
    let tshift = 15 - bit_depth as i32 - log2;
    let qbits = 14 + qp / 6 + tshift;
    let scale = QUANT_SCALE[(qp % 6) as usize];
    let offset = (if intra { 171i64 } else { 85 }) << (qbits - 9);
    let mut nz = 0;
    for i in 0..n * n {
        let v = c[i] as i64;
        let a = ((v.abs() * scale + offset) >> qbits).min(32767);
        if a != 0 {
            nz += 1;
        }
        l[i] = if v < 0 { -(a as i32) } else { a as i32 };
    }
    nz
}

/// Sign data hiding (7.3.8.11 signHidden): in every 4x4 sub-block whose first and last non-zero
/// coefficients (in scan order) are more than 3 positions apart, the sign of the first one is not
/// coded but inferred from the parity of the sum of absolute levels. Fix the parity where needed by
/// the +-1 level change that adds the least quantisation error, without moving the first / last
/// non-zero positions.
pub fn sign_hiding(c: &[i32], l: &mut [i32], log2: u32, scan_idx: usize, qp: i32, bit_depth: u32) {
    use crate::tables::{SCAN_4, scan_order};
    let n = 1usize << log2;
    let tshift = 15 - bit_depth as i32 - log2 as i32;
    let qbits = 14 + qp / 6 + tshift;
    let scale = QUANT_SCALE[(qp % 6) as usize];
    let sb_scan = scan_order(log2 - 2, scan_idx);
    let pos = &SCAN_4[scan_idx];
    for &(xs, ys) in sb_scan {
        let idx = |p: usize| (ys as usize * 4 + pos[p].1 as usize) * n + xs as usize * 4 + pos[p].0 as usize;
        let (Some(first), Some(last)) = ((0..16).find(|&p| l[idx(p)] != 0), (0..16).rev().find(|&p| l[idx(p)] != 0)) else { continue };
        if last - first <= 3 {
            continue;
        }
        let sum: i32 = (0..16).map(|p| l[idx(p)].abs()).sum();
        let negative = l[idx(first)] < 0;
        if negative == (sum & 1 == 1) {
            continue;
        }
        // cost of +-1 changes in 1/256 units of quantisation error
        let mut best: Option<(i64, usize, i32)> = None;
        for p in first..=last {
            let i = idx(p);
            let q = ((c[i] as i64).abs() * scale) >> (qbits - 8);
            let a = l[i].abs() as i64;
            let err = |lv: i64| (q - (lv << 8)).abs();
            let mut consider = |cost: i64, d: i32| {
                if best.is_none_or(|b| cost < b.0) {
                    best = Some((cost, i, d));
                }
            };
            if a < 32767 && (a > 0 || (p > first && p < last)) {
                consider(err(a + 1) - err(a), 1);
            }
            if a > 1 || (a == 1 && p > first && p < last) {
                consider(err(a - 1) - err(a), -1);
            }
        }
        if let Some((_, i, d)) = best {
            let a = l[i].abs() + d;
            l[i] = if c[i] < 0 { -a } else { a };
        }
    }
}

/// Scaling process (8.6.2 / 8.6.3, flat scaling matrix): levels to scaled coefficients.
pub fn dequantize(l: &[i32], d: &mut [i32], n: usize, qp: i32, bit_depth: u32) {
    let log2 = n.trailing_zeros();
    let bd_shift = bit_depth + log2 - 5;
    let round = 1i64 << (bd_shift - 1);
    let scale = (LEVEL_SCALE[(qp % 6) as usize] as i64) << (qp / 6);
    for i in 0..n * n {
        let v = l[i];
        d[i] = if v == 0 { 0 } else { ((v as i64 * 16 * scale + round) >> bd_shift).clamp(-32768, 32767) as i32 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Direct matrix form of the inverse transform, as written in the specification.
    fn inverse_ref(c: &mut [i32], n: usize, dst: bool, bd: u32) {
        let mut tmp = vec![0i32; n * n];
        let step = 32 / n;
        for x in 0..n {
            for y in 0..n {
                let mut s = 0i32;
                for j in 0..n {
                    let m = if dst { DST_MATRIX[j][y] as i32 } else { TRANS_MATRIX[j * step][y] as i32 };
                    s += m * c[j * n + x];
                }
                tmp[y * n + x] = ((s + 64) >> 7).clamp(-32768, 32767);
            }
        }
        let bd_shift = 20 - bd;
        for y in 0..n {
            for x in 0..n {
                let mut s = 0i32;
                for j in 0..n {
                    let m = if dst { DST_MATRIX[j][x] as i32 } else { TRANS_MATRIX[j * step][x] as i32 };
                    s += m * tmp[y * n + j];
                }
                c[y * n + x] = (s + (1 << (bd_shift - 1))) >> bd_shift;
            }
        }
    }

    #[test]
    fn inverse_matches_matrix_form() {
        let mut seed = 7u32;
        for &(n, dst) in &[(4usize, true), (4, false), (8, false), (16, false), (32, false)] {
            for _ in 0..20 {
                let mut c = vec![0i32; n * n];
                for v in c.iter_mut() {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    if (seed >> 24).is_multiple_of(3) {
                        *v = ((seed >> 8) % 2001) as i32 - 1000;
                    }
                }
                let mut a = c.clone();
                let mut b = c.clone();
                inverse(&mut a, n, dst, 8);
                inverse_ref(&mut b, n, dst, 8);
                assert_eq!(a, b, "n {n} dst {dst}");
            }
        }
    }

    #[test]
    fn round_trip_is_close() {
        for &(n, dst) in &[(4usize, true), (8, false), (32, false)] {
            for bd in [8u32, 10] {
                let r: Vec<i32> = (0..n * n).map(|i| ((i * 37 + i / n * 11) % 61) as i32 - 30).collect();
                let mut c = vec![0; n * n];
                forward(&r, &mut c, n, dst, bd);
                let mut l = vec![0; n * n];
                let qp = 4;
                quantize(&c, &mut l, n, qp, true, bd);
                let mut d = vec![0; n * n];
                dequantize(&l, &mut d, n, qp, bd);
                inverse(&mut d, n, dst, bd);
                let err: i32 = r.iter().zip(&d).map(|(a, b)| (a - b).abs()).max().unwrap();
                assert!(err <= 2, "n {n} bd {bd} err {err}");
            }
        }
    }
}
