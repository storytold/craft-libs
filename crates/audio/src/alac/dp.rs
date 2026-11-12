// Ported from Apple's ALAC reference implementation (dplib.h, dp_dec.c and dp_enc.c, macosforge/alac@c38887c,
// https://github.com/macosforge/alac). Copyright (c) 2011 Apple Inc.
// Licensed under the Apache License, Version 2.0 (LICENSE-APACHE at the repository root).
// Modified: translated to Rust and changed as the `alac` module docs describe.
// SPDX-License-Identifier: Apache-2.0

//! The adaptive FIR predictor, from Apple's `dplib.h`, `dp_dec.c` and `dp_enc.c`.
//!
//! The reference has unrolled copies for 4 and 8 taps; they compute the same values as its
//! general loop, which is the one ported. Arithmetic wraps as the C code does on its targets.
//! As in the reference, the warm-up (the first `numactive` samples, first differences) runs to
//! `numactive` even in a shorter block, so the encoder's trial runs leave the buffers as the
//! reference leaves them; here it stops at the end of the buffers.

pub(crate) const DENSHIFT_DEFAULT: u32 = 9;
const AINIT: i32 = 38;
const BINIT: i32 = -29;
const CINIT: i32 = -2;
/// Coefficients of the first-difference shortcut (`numactive == 31`).
pub(crate) const FIRST_ORDER: usize = 31;

/// `init_coefs`.
pub(crate) fn init_coefs(coefs: &mut [i16], denshift: u32) {
    let den = 1i32 << denshift;
    coefs.fill(0);
    for (c, init) in coefs.iter_mut().zip([AINIT, BINIT, CINIT]) {
        *c = ((init * den) >> 4) as i16;
    }
}

/// `sign_of_int`: -1, 0 or 1.
fn sign(i: i32) -> i32 {
    i.signum()
}

/// `(v << shift) >> shift`: `v` wrapped to `chanbits` bits, sign-extended.
fn wrap(v: i32, chanshift: u32) -> i32 {
    (v << chanshift) >> chanshift
}

/// 1 << (denshift - 1), 0 when `denshift` is 0 (a shift the reference leaves undefined).
fn half(denshift: u32) -> i32 {
    if denshift == 0 { 0 } else { 1i32 << (denshift - 1).min(30) }
}

/// The sign-LMS coefficient update after a residual of `del` (`top`, `prev`: the base sample and
/// the samples before the current one, nearest first).
fn adapt(coefs: &mut [i16], del: i32, top: i32, prev: impl Fn(usize) -> i32, denshift: u32) {
    let numactive = coefs.len();
    let mut del0 = del;
    let shift = denshift.min(31);
    if del > 0 {
        for k in (0..numactive).rev() {
            let dd = top.wrapping_sub(prev(k));
            let sgn = sign(dd);
            coefs[k] = coefs[k].wrapping_sub(sgn as i16);
            del0 = del0.wrapping_sub(((numactive - k) as i32).wrapping_mul(sgn.wrapping_mul(dd) >> shift));
            if del0 <= 0 {
                break;
            }
        }
    } else if del < 0 {
        for k in (0..numactive).rev() {
            let dd = top.wrapping_sub(prev(k));
            let sgn = sign(dd);
            coefs[k] = coefs[k].wrapping_add(sgn as i16);
            del0 = del0.wrapping_sub(((numactive - k) as i32).wrapping_mul(sgn.wrapping_neg().wrapping_mul(dd) >> shift));
            if del0 >= 0 {
                break;
            }
        }
    }
}

/// `pc_block`: residuals `pc` of `input` (both at least `num` long) with `numactive`
/// coefficients ([`FIRST_ORDER`]: first differences, 0: a copy).
pub(crate) fn pc_block(input: &[i32], pc: &mut [i32], num: usize, coefs: &mut [i16], numactive: usize, chanbits: u32, denshift: u32) {
    let cap = input.len().min(pc.len());
    if cap == 0 {
        return;
    }
    let num = num.min(cap);
    let chanshift = 32u32.saturating_sub(chanbits).min(31);
    pc[0] = input[0];
    if numactive == 0 {
        if num > 1 {
            pc[1..num].copy_from_slice(&input[1..num]);
        }
        return;
    }
    let warm = if numactive == FIRST_ORDER { num } else { (numactive + 1).min(cap) };
    for j in 1..warm {
        pc[j] = wrap(input[j].wrapping_sub(input[j - 1]), chanshift);
    }
    if numactive == FIRST_ORDER {
        return;
    }
    let active = numactive.min(coefs.len());
    let coefs = &mut coefs[..active];
    let numactive = coefs.len();
    let lim = numactive + 1;
    let denhalf = half(denshift);
    let shift = denshift.min(31);
    for j in lim..num {
        let top = input[j - lim];
        let prev = |k: usize| input[j - 1 - k];
        let mut sum: i32 = 0;
        for (k, &c) in coefs.iter().enumerate() {
            sum = sum.wrapping_sub(i32::from(c).wrapping_mul(top.wrapping_sub(prev(k))));
        }
        let del = wrap(input[j].wrapping_sub(top).wrapping_sub(sum.wrapping_add(denhalf) >> shift), chanshift);
        pc[j] = del;
        adapt(coefs, del, top, prev, denshift);
    }
}

/// `unpc_block`: samples `out` from residuals `pc` (both at least `num` long), the inverse of
/// [`pc_block`] with the same coefficients.
pub(crate) fn unpc_block(pc: &[i32], out: &mut [i32], num: usize, coefs: &mut [i16], numactive: usize, chanbits: u32, denshift: u32) {
    let cap = pc.len().min(out.len());
    if cap == 0 {
        return;
    }
    let num = num.min(cap);
    let chanshift = 32u32.saturating_sub(chanbits).min(31);
    out[0] = pc[0];
    if numactive == 0 {
        if num > 1 {
            out[1..num].copy_from_slice(&pc[1..num]);
        }
        return;
    }
    let warm = if numactive == FIRST_ORDER { num } else { (numactive + 1).min(cap) };
    for j in 1..warm {
        out[j] = wrap(pc[j].wrapping_add(out[j - 1]), chanshift);
    }
    if numactive == FIRST_ORDER {
        return;
    }
    let active = numactive.min(coefs.len());
    let coefs = &mut coefs[..active];
    let numactive = coefs.len();
    let lim = numactive + 1;
    let denhalf = half(denshift);
    let shift = denshift.min(31);
    for j in lim..num {
        let top = out[j - lim];
        let mut sum: i32 = 0;
        for (k, &c) in coefs.iter().enumerate() {
            sum = sum.wrapping_add(i32::from(c).wrapping_mul(out[j - 1 - k].wrapping_sub(top)));
        }
        let del = pc[j];
        out[j] = wrap(del.wrapping_add(top).wrapping_add(sum.wrapping_add(denhalf) >> shift), chanshift);
        let (done, _) = out.split_at(j);
        adapt(coefs, del, top, |k| done[j - 1 - k], denshift);
    }
}

/// [`unpc_block`] in place for the first-difference shortcut (mode 1's first stage).
pub(crate) fn unpc_first_order_in_place(buf: &mut [i32], num: usize, chanbits: u32) {
    let num = num.min(buf.len());
    let chanshift = 32u32.saturating_sub(chanbits).min(31);
    for j in 1..num {
        buf[j] = wrap(buf[j].wrapping_add(buf[j - 1]), chanshift);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_predictor_inverts() {
        let input: Vec<i32> = (0..2000).map(|i: i32| ((i as f64 * 0.05).sin() * 20000.0) as i32 + (i * 31 % 17) - 8).collect();
        for numactive in [0, 1, 4, 8, 12, FIRST_ORDER] {
            for num in [0, 1, 3, 9, 2000] {
                let mut ce = [0i16; 32];
                init_coefs(&mut ce, DENSHIFT_DEFAULT);
                let mut cd = ce;
                let mut pc = vec![0; 2000];
                let mut out = vec![0; 2000];
                pc_block(&input, &mut pc, num, &mut ce, numactive, 16, DENSHIFT_DEFAULT);
                unpc_block(&pc, &mut out, num, &mut cd, numactive, 16, DENSHIFT_DEFAULT);
                assert_eq!(out[..num], input[..num], "numactive {numactive}, num {num}");
                assert_eq!(ce, cd);
            }
        }
    }

    #[test]
    fn coefficient_start_values() {
        let mut c = [7i16; 16];
        init_coefs(&mut c, DENSHIFT_DEFAULT);
        assert_eq!(&c[..4], &[1216, -928, -64, 0]);
    }
}
