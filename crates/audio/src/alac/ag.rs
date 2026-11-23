// Ported from Apple's ALAC reference implementation (aglib.h, ag_dec.c and ag_enc.c, macosforge/alac@c38887c,
// https://github.com/macosforge/alac). Copyright (c) 2011 Apple Inc.
// Licensed under the Apache License, Version 2.0 (LICENSE-APACHE at the repository root).
// Modified: translated to Rust and changed as the `alac` module docs describe.
// SPDX-License-Identifier: Apache-2.0

//! Adaptive Golomb-Rice coding of prediction residuals, from Apple's `aglib.h`, `ag_dec.c` and
//! `ag_enc.c`.
//!
//! Arithmetic is `u32`/`i32` with wrapping, as the C code has on the platforms it targets, so
//! hostile parameters give garbage samples rather than a panic. Peeks past the packet read zero
//! bits; [`decompress`] fails when the codes it consumed end past the packet.

use super::bits::{BitReader, BitSink};
use super::{Error, Result};

pub(crate) const QBSHIFT: u32 = 9;
pub(crate) const QB: u32 = 1 << QBSHIFT;
pub(crate) const PB0: u32 = 40;
pub(crate) const MB0: u32 = 10;
pub(crate) const KB0: u32 = 14;
pub(crate) const MAX_RUN_DEFAULT: u32 = 255;
const MMULSHIFT: u32 = 2;
const MDENSHIFT: u32 = QBSHIFT - MMULSHIFT - 1;
const MOFF: u32 = 1 << (MDENSHIFT - 2);
const BITOFF: u32 = 24;
/// Longest prefix of ones before an escaped value.
const MAX_PREFIX_16: u32 = 9;
const MAX_PREFIX_32: u32 = 9;
/// Width of an escaped zero-run length.
const MAX_DATATYPE_BITS_16: u32 = 16;
const N_MAX_MEAN_CLAMP: u32 = 0xffff;
const N_MEAN_CLAMP_VAL: u32 = 0xffff;

/// `AGParamRec`: the coder's tuning (`fw`/`sw`, the image-row widths, are always the sample
/// count for audio and not kept).
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgParams {
    mb0: u32,
    pb: u32,
    kb: u32,
    wb: u32,
}

impl AgParams {
    /// `set_ag_params` (`maxrun` is unused by the coder).
    pub(crate) fn new(mb: u32, pb: u32, kb: u32) -> Self {
        let wb = 1u32.checked_shl(kb).unwrap_or(0).wrapping_sub(1);
        Self { mb0: mb, pb, kb, wb }
    }

    /// `set_standard_ag_params`.
    pub(crate) fn standard() -> Self {
        Self::new(MB0, PB0, KB0)
    }
}

/// Leading zero bits of `m` (`lead`).
fn lead(m: u32) -> u32 {
    m.leading_zeros()
}

/// `lg3a`: floor(log2(x + 3)).
fn lg3a(x: u32) -> u32 {
    31u32.wrapping_sub(lead(x.wrapping_add(3)))
}

/// `(1 << k) - 1`, 0 for `k` of 32 or more.
fn mask(k: u32) -> u32 {
    1u32.checked_shl(k).unwrap_or(0).wrapping_sub(1)
}

/// The Rice parameter of the zero-run length that follows when the mean has dropped this low.
fn zero_run_k(mb: u32) -> u32 {
    lead(mb).wrapping_sub(BITOFF).wrapping_add(mb.wrapping_add(MOFF) >> MDENSHIFT)
}

/// The next mean, after a value of `n` (`zmode` is 1 right after a zero run).
fn next_mean(pb: u32, mb: u32, n: u32, zmode: u32) -> u32 {
    let mb = pb.wrapping_mul(n.wrapping_add(zmode)).wrapping_add(mb).wrapping_sub(pb.wrapping_mul(mb) >> QBSHIFT);
    if n > N_MAX_MEAN_CLAMP { N_MEAN_CLAMP_VAL } else { mb }
}

/// `dyn_get`: a zero-run length, coded with parameter `k` (escape: 16 raw bits).
fn get_run(r: &BitReader, pos: &mut usize, m: u32, k: u32) -> u32 {
    let pre = lead(!r.peek_at(*pos, 32));
    if pre >= MAX_PREFIX_16 {
        let v = r.peek_at(*pos + MAX_PREFIX_16 as usize, MAX_DATATYPE_BITS_16);
        *pos += (MAX_PREFIX_16 + MAX_DATATYPE_BITS_16) as usize;
        v
    } else {
        let v = r.peek_at(*pos + pre as usize + 1, k);
        *pos += (pre + 1 + k) as usize;
        let mut result = pre.wrapping_mul(m).wrapping_add(v).wrapping_sub(1);
        if v < 2 {
            result = result.wrapping_sub(v.wrapping_sub(1));
            *pos -= 1;
        }
        result
    }
}

/// `dyn_get_32bit`: one residual magnitude, coded with parameter `k` (escape: `maxbits` raw bits).
fn get_value(r: &BitReader, pos: &mut usize, m: u32, k: u32, maxbits: u32) -> u32 {
    let pre = lead(!r.peek_at(*pos, 32));
    if pre >= MAX_PREFIX_32 {
        let v = r.peek_at(*pos + MAX_PREFIX_32 as usize, maxbits);
        *pos += (MAX_PREFIX_32 + maxbits) as usize;
        return v;
    }
    *pos += pre as usize + 1;
    if k == 1 {
        return pre;
    }
    let v = r.peek_at(*pos, k);
    // k - 1 bits when the suffix is 0 or 1, else k (k is 0 only in a hostile stream, which
    // moves back by one; the caller's loop is bounded by the sample count).
    *pos = (*pos + k as usize).saturating_sub(1);
    let mut result = pre.wrapping_mul(m);
    if v >= 2 {
        result = result.wrapping_add(v - 1);
        *pos += 1;
    }
    result
}

/// `dyn_decomp`: `out.len()` residuals of up to `max_size` bits from `r`, which is advanced past
/// them.
pub(crate) fn decompress(params: AgParams, r: &mut BitReader, out: &mut [i32], max_size: u32) -> Result<()> {
    let num = out.len();
    let max_pos = r.len_bits();
    let mut pos = r.position();
    let mut mb = params.mb0;
    let mut zmode = 0u32;
    let mut c = 0usize;
    while c < num {
        if pos >= max_pos {
            return Err(Error::Eof);
        }
        let k = lg3a(mb >> QBSHIFT).min(params.kb);
        let m = mask(k);
        let n = get_value(r, &mut pos, m, k, max_size);
        // the least significant bit is the sign
        let nd = n.wrapping_add(zmode);
        let multiplier = (nd & 1).wrapping_neg() | 1;
        // ceil(nd / 2) without the reference's `nd + 1`, which wraps 0xffffffff (i32::MIN) to 0
        out[c] = ((nd >> 1) + (nd & 1)).wrapping_mul(multiplier) as i32;
        c += 1;
        mb = next_mean(params.pb, mb, n, zmode);
        zmode = 0;
        if (mb << MMULSHIFT) < QB && c < num {
            zmode = 1;
            let k = zero_run_k(mb);
            let mz = mask(k) & params.wb;
            let run = get_run(r, &mut pos, mz, k);
            let run_end = c.checked_add(run as usize).filter(|&e| e <= num).ok_or(Error::Bitstream("zero run past the end of the frame"))?;
            out[c..run_end].fill(0);
            c = run_end;
            if run >= 65535 {
                zmode = 0;
            }
            mb = 0;
        }
    }
    r.set_position(pos)
}

/// `dyn_code`: a zero-run length as (value, bits).
fn code_run(m: u32, k: u32, n: u32) -> (u32, u32) {
    let escape = || (((1u32 << MAX_PREFIX_16) - 1) << MAX_DATATYPE_BITS_16).wrapping_add(n);
    let div = n / m.max(1);
    if div >= MAX_PREFIX_16 {
        return (escape(), MAX_PREFIX_16 + MAX_DATATYPE_BITS_16);
    }
    let rem = n % m.max(1);
    let de = u32::from(rem == 0);
    let bits = div + k + 1 - de;
    if bits > MAX_PREFIX_16 + MAX_DATATYPE_BITS_16 {
        return (escape(), MAX_PREFIX_16 + MAX_DATATYPE_BITS_16);
    }
    let value = (mask(div) << (bits - div)).wrapping_add(rem).wrapping_add(1).wrapping_sub(de);
    (value, bits)
}

/// `dyn_code_32bit`: a residual magnitude as (value, bits), plus the raw `maxbits`-bit escape
/// that follows a prefix of nine ones.
fn code_value(maxbits: u32, m: u32, k: u32, n: u32) -> (u32, u32, Option<(u32, u32)>) {
    let div = n / m.max(1);
    if div < MAX_PREFIX_32 {
        let rem = n - m * div;
        let de = u32::from(rem == 0);
        let bits = div + k + 1 - de;
        if bits <= 25 {
            let value = (mask(div) << (bits - div)).wrapping_add(rem).wrapping_add(1).wrapping_sub(de);
            return (value, bits, None);
        }
    }
    (mask(MAX_PREFIX_32), MAX_PREFIX_32, Some((n, maxbits)))
}

/// `dyn_comp`: code the residuals `pc`, each of up to `bit_size` bits, into `out`.
pub(crate) fn compress(params: AgParams, pc: &[i32], out: &mut impl BitSink, bit_size: u32) -> Result<()> {
    if !(1..=32).contains(&bit_size) {
        return Err(Error::InvalidInput("residual width outside 1 to 32 bits"));
    }
    let num = pc.len();
    let mut mb = params.mb0;
    let mut zmode = 0u32;
    let mut c = 0usize;
    while c < num {
        let k = lg3a(mb >> QBSHIFT).min(params.kb);
        let m = mask(k);
        let del = pc[c];
        let n = (del.unsigned_abs() << 1).wrapping_sub(u32::from(del < 0)).wrapping_sub(zmode);
        let (value, bits, escape) = code_value(bit_size, m, k, n);
        out.put(value, bits);
        if let Some((raw, raw_bits)) = escape {
            out.put(raw, raw_bits);
        }
        c += 1;
        mb = next_mean(params.pb, mb, n, zmode);
        zmode = 0;
        if (mb << MMULSHIFT) < QB && c < num {
            zmode = 1;
            let mut nz = 0u32;
            while c < num && pc[c] == 0 {
                nz += 1;
                c += 1;
                if nz >= 65535 {
                    zmode = 0;
                    break;
                }
            }
            let k = zero_run_k(mb);
            let mz = mask(k) & params.wb;
            let (value, bits) = code_run(mz, k, nz);
            out.put(value, bits);
            mb = 0;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::bits::{BitReader, BitSink, BitWriter};
    use super::*;

    fn roundtrip(values: &[i32], bits: u32) {
        let mut w = BitWriter::new();
        compress(AgParams::standard(), values, &mut w, bits).unwrap();
        let len = w.bit_len();
        let bytes = w.into_bytes();
        let mut r = BitReader::new(&bytes);
        let mut out = vec![0; values.len()];
        decompress(AgParams::standard(), &mut r, &mut out, bits).unwrap();
        assert_eq!(out, values);
        assert_eq!(r.position(), len);
    }

    #[test]
    fn residuals_roundtrip() {
        roundtrip(&[0, 1, -1, 2, -2, 100, -100, 30000, -32768, 32767], 16);
        roundtrip(&[0; 300], 16);
        let mut v = vec![0; 70_000];
        v[0] = 5;
        v[69_999] = -3;
        roundtrip(&v, 16);
        let wide: Vec<i32> = (0..500).map(|i| ((i * 7919) % 65_536 - 32_768) << 8).collect();
        roundtrip(&wide, 25);
        roundtrip(&[i32::MAX >> 1, i32::MIN >> 1, 0, 0, 0, 1], 32);
    }

    /// The largest escaped magnitude, 0xffffffff, is i32::MIN (the reference wraps it to 0).
    #[test]
    fn the_full_width_escape_decodes_to_the_minimum() {
        let mut w = BitWriter::new();
        w.put(0x1FF, 9); // nine ones: escape
        w.put(u32::MAX, 32);
        let bytes = w.into_bytes();
        let mut out = [7];
        decompress(AgParams::standard(), &mut BitReader::new(&bytes), &mut out, 32).unwrap();
        assert_eq!(out, [i32::MIN]);
    }

    #[test]
    fn a_truncated_stream_is_an_error() {
        let mut w = BitWriter::new();
        let values: Vec<i32> = (0..64).map(|i| i * 1000 - 32_000).collect();
        compress(AgParams::standard(), &values, &mut w, 16).unwrap();
        let bytes = w.into_bytes();
        let mut r = BitReader::new(&bytes[..bytes.len() / 2]);
        let mut out = vec![0; values.len()];
        assert!(decompress(AgParams::standard(), &mut r, &mut out, 16).is_err());
    }
}
