//! Pulse cache and bit allocation (RFC 6716 §4.3.3, §4.3.4.1).
//!
//! The allocation must be bit-identical to the decoder's, so this mirrors FilmCraft's
//! spec-derived `filmcraft-opus` decoder (first-party, MIT OR Apache-2.0) step by step; the only
//! difference is that the encoder *writes* the band-skip, intensity and dual-stereo symbols.

use crate::opus::encoder::CeltFrame;
use crate::opus::encoder::range::{BITRES, RangeEncoder};
use crate::opus::encoder::tables::{EBANDS, LOG2_FRAC_TABLE, NB_EBANDS};

#[cfg(test)]
pub use crate::opus::rate_common::pvq_v;
pub use crate::opus::rate_common::{Mode, get_pulses};
pub const MAX_FINE_BITS: i32 = 8;
const FINE_OFFSET: i32 = 21;
pub const QTHETA_OFFSET: i32 = 4;
const ALLOC_STEPS: i32 = 6;

/// Result of the allocation process.
pub struct Allocation {
    pub coded_bands: usize,
    pub intensity: usize,
    pub dual_stereo: bool,
    pub balance: i32,
    pub pulses: [i32; NB_EBANDS],
    pub fine_quant: [i32; NB_EBANDS],
    pub fine_priority: [i32; NB_EBANDS],
}

/// Per-band limits, boosts and available budget for the allocation search.
pub struct AllocationInput<'a> {
    pub offsets: &'a [i32; NB_EBANDS],
    pub cap: &'a [i32; NB_EBANDS],
    pub trim: i32,
    pub bits: i32,
}

/// `clt_compute_allocation`, encoder side. Stereo frames are always coded as dual (L/R) stereo
/// with no intensity bands, and no band is skipped voluntarily.
pub fn compute_allocation(m: &Mode, frame: CeltFrame, input: AllocationInput<'_>, enc: &mut RangeEncoder) -> Allocation {
    let CeltFrame { start, end, channels: c, lm } = frame;
    let AllocationInput { offsets, cap, trim: alloc_trim, bits: total } = input;
    let crate::opus::rate_common::Bracket { skip_start, bits1, bits2, thresh, total, skip_rsv, intensity_rsv, dual_stereo_rsv } =
        crate::opus::rate_common::bracket(start, end, offsets, cap, alloc_trim, total, c, lm);
    let interpolation =
        AllocationInterpolation { skip_start, bits1: &bits1, bits2: &bits2, thresh: &thresh, cap, total, skip_rsv, intensity_rsv, dual_stereo_rsv };
    interp_bits2pulses(m, frame, interpolation, enc)
}

/// Bracketing allocation vectors and signalling reserves for the interpolation stage.
struct AllocationInterpolation<'a> {
    skip_start: usize,
    bits1: &'a [i32; NB_EBANDS],
    bits2: &'a [i32; NB_EBANDS],
    thresh: &'a [i32; NB_EBANDS],
    cap: &'a [i32; NB_EBANDS],
    total: i32,
    skip_rsv: i32,
    intensity_rsv: i32,
    dual_stereo_rsv: i32,
}

fn interp_bits2pulses(m: &Mode, frame: CeltFrame, interpolation: AllocationInterpolation<'_>, enc: &mut RangeEncoder) -> Allocation {
    let CeltFrame { start, end, channels: c, lm } = frame;
    let AllocationInterpolation { skip_start, bits1, bits2, thresh, cap, mut total, skip_rsv, mut intensity_rsv, mut dual_stereo_rsv } =
        interpolation;
    let ci = c as i32;
    let alloc_floor = ci << BITRES;
    let stereo = (c > 1) as i32;
    let log_m = (lm as i32) << BITRES;
    let mut lo = 0;
    let mut hi = 1 << ALLOC_STEPS;
    for _ in 0..ALLOC_STEPS {
        let mid = (lo + hi) >> 1;
        let mut psum = 0;
        let mut done = false;
        for j in (start..end).rev() {
            let tmp = bits1[j] + ((mid * bits2[j]) >> ALLOC_STEPS);
            if tmp >= thresh[j] || done {
                done = true;
                psum += tmp.min(cap[j]);
            } else if tmp >= alloc_floor {
                psum += alloc_floor;
            }
        }
        if psum > total {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let mut bits = [0i32; NB_EBANDS];
    let mut psum = 0;
    let mut done = false;
    for j in (start..end).rev() {
        let mut tmp = bits1[j] + ((lo * bits2[j]) >> ALLOC_STEPS);
        if tmp < thresh[j] && !done {
            tmp = if tmp >= alloc_floor { alloc_floor } else { 0 };
        } else {
            done = true;
        }
        tmp = tmp.min(cap[j]);
        bits[j] = tmp;
        psum += tmp;
    }
    let s = start;
    let mut coded_bands = end;
    loop {
        let j = coded_bands - 1;
        if j <= skip_start {
            total += skip_rsv;
            break;
        }
        let mut left = total - psum;
        let span = EBANDS[coded_bands] - EBANDS[s];
        let percoeff = left / span;
        left -= span * percoeff;
        let rem = (left - (EBANDS[j] - EBANDS[s])).max(0);
        let band_width = EBANDS[coded_bands] - EBANDS[j];
        let band_bits = bits[j] + percoeff * band_width + rem;
        if band_bits >= thresh[j].max(alloc_floor + (1 << BITRES)) {
            // Keep this band (and every band below it).
            enc.bit_logp(true, 1);
            break;
        }
        psum -= bits[j] + intensity_rsv;
        if intensity_rsv > 0 {
            intensity_rsv = LOG2_FRAC_TABLE[j - s];
        }
        psum += intensity_rsv;
        if band_bits >= alloc_floor {
            psum += alloc_floor;
            bits[j] = alloc_floor;
        } else {
            bits[j] = 0;
        }
        coded_bands -= 1;
    }
    let intensity = if intensity_rsv > 0 {
        // No intensity stereo: every coded band is coded in full.
        enc.uint((coded_bands - s) as u32, (coded_bands + 1 - s) as u32);
        coded_bands
    } else {
        0
    };
    if intensity <= s {
        total += dual_stereo_rsv;
        dual_stereo_rsv = 0;
    }
    let dual_stereo = if dual_stereo_rsv > 0 {
        enc.bit_logp(true, 1);
        true
    } else {
        false
    };
    let mut left = total - psum;
    let span = EBANDS[coded_bands] - EBANDS[s];
    let percoeff = left / span;
    left -= span * percoeff;
    for j in s..coded_bands {
        bits[j] += percoeff * (EBANDS[j + 1] - EBANDS[j]);
    }
    for j in s..coded_bands {
        let tmp = left.min(EBANDS[j + 1] - EBANDS[j]);
        bits[j] += tmp;
        left -= tmp;
    }
    let mut ebits = [0i32; NB_EBANDS];
    let mut fine_priority = [0i32; NB_EBANDS];
    let mut balance = 0;
    for j in s..coded_bands {
        let n0 = EBANDS[j + 1] - EBANDS[j];
        let n = n0 << lm;
        let bit = bits[j] + balance;
        let mut excess;
        if n > 1 {
            excess = (bit - cap[j]).max(0);
            bits[j] = bit - excess;
            let den = ci * n + if c == 2 && n > 2 && !dual_stereo && j < intensity { 1 } else { 0 };
            let nclogn = den * (m.log_n[j] + log_m);
            let mut offset = (nclogn >> 1) - den * FINE_OFFSET;
            if n == 2 {
                offset += (den << BITRES) >> 2;
            }
            if bits[j] + offset < (den * 2) << BITRES {
                offset += nclogn >> 2;
            } else if bits[j] + offset < (den * 3) << BITRES {
                offset += nclogn >> 3;
            }
            let mut eb = (bits[j] + offset + (den << (BITRES - 1))).max(0);
            eb = (eb / den) >> BITRES;
            if ci * eb > (bits[j] >> BITRES) {
                eb = bits[j] >> stereo >> BITRES;
            }
            eb = eb.min(MAX_FINE_BITS);
            ebits[j] = eb;
            fine_priority[j] = (eb * (den << BITRES) >= bits[j] + offset) as i32;
            bits[j] -= (ci * eb) << BITRES;
        } else {
            excess = (bit - (ci << BITRES)).max(0);
            bits[j] = bit - excess;
            ebits[j] = 0;
            fine_priority[j] = 1;
        }
        if excess > 0 {
            let extra_fine = (excess >> (stereo + BITRES as i32)).min(MAX_FINE_BITS - ebits[j]);
            ebits[j] += extra_fine;
            let extra_bits = (extra_fine * ci) << BITRES;
            fine_priority[j] = (extra_bits >= excess - balance) as i32;
            excess -= extra_bits;
        }
        balance = excess;
    }
    for j in coded_bands..end {
        ebits[j] = bits[j] >> stereo >> BITRES;
        bits[j] = 0;
        fine_priority[j] = (ebits[j] < 1) as i32;
    }
    Allocation { coded_bands, intensity, dual_stereo, balance, pulses: bits, fine_quant: ebits, fine_priority }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_match_decoder_reference() {
        let m = Mode::new();
        let first_rows: [u8; 42] = [
            224, 224, 224, 224, 224, 224, 224, 224, 160, 160, 160, 160, 185, 185, 185, 178, 178, 168, 134, 61, 37, 224, 224, 224, 224, 224, 224, 224,
            224, 240, 240, 240, 240, 207, 207, 207, 198, 198, 183, 144, 66, 40,
        ];
        assert_eq!(&m.caps[..42], &first_rows[..]);
    }

    #[test]
    fn pvq_counts() {
        assert_eq!(pvq_v(5, 1), 10);
        assert_eq!(pvq_v(1, 7), 2);
        assert_eq!(pvq_v(2, 9), 36);
        assert_eq!(pvq_v(3, 2), 18);
    }
}
