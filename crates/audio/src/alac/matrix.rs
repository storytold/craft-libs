// Ported from Apple's ALAC reference implementation (matrixlib.h, matrix_dec.c and matrix_enc.c, macosforge/alac@c38887c,
// https://github.com/macosforge/alac). Copyright (c) 2011 Apple Inc.
// Licensed under the Apache License, Version 2.0 (LICENSE-APACHE at the repository root).
// Modified: translated to Rust and changed as the `alac` module docs describe.
// SPDX-License-Identifier: Apache-2.0

//! Stereo matrixing and the shifted-off low bytes, from Apple's `matrixlib.h`, `matrix_dec.c` and
//! `matrix_enc.c`.
//!
//! The reference has one routine per sample width (16, 20, 24, 32 bits) over interleaved buffers;
//! on right-aligned planar `i32` samples they reduce to the same arithmetic, ported once here.
//!
//! A generalised mid/side transform, lossless for any `mixres` and `mixbits` (m = 2^mixbits):
//! u = floor((r·L + (m − r)·R) / m), v = L − R; inverse L = u + v − floor(r·v / m), R = L − v.

/// `x >> n` for any `n` (31 and above give the sign).
fn shr(x: i32, n: u32) -> i32 {
    x >> n.min(31)
}

/// Split off the low `shift` bits of `x` into the shift buffer value and return the rest.
fn split(x: i32, shift: u32) -> (i32, u16) {
    if shift == 0 { (x, 0) } else { (x >> shift, (x as u32 & ((1u32 << shift) - 1)) as u16) }
}

/// A channel pair's matrixing: `mixbits` (m = 2^mixbits) and `mixres` (r; 0 for none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mix {
    pub(crate) bits: u32,
    pub(crate) res: i32,
}

/// `mix16/20/24/32`: `u`, `v` from the samples of `l` and `r` (as many as the shortest of the
/// four holds), the low `bytes_shifted` bytes of each going to `shift_uv` (interleaved L, R).
pub(crate) fn mix((l, r): (&[i32], &[i32]), (u, v): (&mut [i32], &mut [i32]), m: Mix, shift_uv: &mut [u16], bytes_shifted: u32) {
    let (mixbits, mixres) = (m.bits, m.res);
    let shift = bytes_shifted * 8;
    let m2 = (1i32 << mixbits.min(30)).wrapping_sub(mixres);
    for j in 0..l.len().min(r.len()).min(u.len()).min(v.len()) {
        let (lj, ls) = split(l[j], shift);
        let (rj, rs) = split(r[j], shift);
        if shift != 0
            && let Some(s) = shift_uv.get_mut(2 * j..2 * j + 2)
        {
            s.copy_from_slice(&[ls, rs]);
        }
        if mixres != 0 {
            u[j] = shr(mixres.wrapping_mul(lj).wrapping_add(m2.wrapping_mul(rj)), mixbits);
            v[j] = lj.wrapping_sub(rj);
        } else {
            u[j] = lj;
            v[j] = rj;
        }
    }
}

/// `unmix16/20/24/32`: the samples of `l` and `r` (as many as the shortest of the four holds) from
/// `u`, `v` and the shifted-off bytes, wrapped to `bit_depth` bits as the reference's stores to
/// 16/24/32-bit words wrap them.
pub(crate) fn unmix((u, v): (&[i32], &[i32]), (l, r): (&mut [i32], &mut [i32]), m: Mix, shift_uv: &[u16], bytes_shifted: u32, bit_depth: u32) {
    let (mixbits, mixres) = (m.bits, m.res);
    let shift = bytes_shifted * 8;
    for j in 0..u.len().min(v.len()).min(l.len()).min(r.len()) {
        let (mut lj, mut rj) = if mixres != 0 {
            let lj = u[j].wrapping_add(v[j]).wrapping_sub(shr(mixres.wrapping_mul(v[j]), mixbits));
            (lj, lj.wrapping_sub(v[j]))
        } else {
            (u[j], v[j])
        };
        if shift != 0 {
            let ls = shift_uv.get(2 * j).copied().unwrap_or(0);
            let rs = shift_uv.get(2 * j + 1).copied().unwrap_or(0);
            lj = (lj << shift) | i32::from(ls);
            rj = (rj << shift) | i32::from(rs);
        }
        l[j] = super::bits::sign_extend(lj as u32, bit_depth);
        r[j] = super::bits::sign_extend(rj as u32, bit_depth);
    }
}

/// `copyPredictorTo24Shift` and friends for one channel: the samples from the predictor output
/// and the shifted-off bytes, wrapped to `bit_depth`.
pub(crate) fn unshift(pred: &[i32], shift_buf: &[u16], out: &mut [i32], num: usize, bytes_shifted: u32, bit_depth: u32) {
    let shift = bytes_shifted * 8;
    for j in 0..num.min(pred.len()).min(out.len()) {
        let x = if shift != 0 { (pred[j] << shift) | i32::from(shift_buf.get(j).copied().unwrap_or(0)) } else { pred[j] };
        out[j] = super::bits::sign_extend(x as u32, bit_depth);
    }
}

/// The encoder's mono split (`EncodeMono`): predictor input and shifted-off bytes of `input`.
pub(crate) fn shift_mono(input: &[i32], pred_in: &mut [i32], shift_buf: &mut [u16], num: usize, bytes_shifted: u32) {
    let shift = bytes_shifted * 8;
    for j in 0..num.min(input.len()).min(pred_in.len()) {
        let (x, s) = split(input[j], shift);
        pred_in[j] = x;
        if let Some(b) = shift_buf.get_mut(j) {
            *b = s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrixing_inverts() {
        let l: Vec<i32> = (0..100).map(|i| (i * 977 % 16_000_000) - 8_000_000).collect();
        let r: Vec<i32> = (0..100).map(|i| (i * 7919 % 16_000_000) - 8_000_000).collect();
        for (mixres, bytes) in [(0, 0), (1, 0), (2, 1), (4, 1), (3, 0)] {
            let (mut u, mut v, mut s) = (vec![0; 100], vec![0; 100], vec![0u16; 200]);
            let m = Mix { bits: 2, res: mixres };
            mix((&l, &r), (&mut u, &mut v), m, &mut s, bytes);
            let (mut l2, mut r2) = (vec![0; 100], vec![0; 100]);
            unmix((&u, &v), (&mut l2, &mut r2), m, &s, bytes, 24);
            assert_eq!((l2, r2), (l.clone(), r.clone()), "mixres {mixres}");
        }
    }
}
