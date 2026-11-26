//! Pulse cache and bit allocation (RFC 6716 §4.3.3, §4.3.4.1).

use super::tables::{EBANDS, LOG2_FRAC_TABLE, MAX_LM, NB_EBANDS, OVERLAP};
use crate::opus::decode::range::{BITRES, RangeDecoder};

pub use crate::opus::rate_common::get_pulses;
pub const MAX_FINE_BITS: i32 = 8;
const FINE_OFFSET: i32 = 21;
pub const QTHETA_OFFSET: i32 = 4;
pub const QTHETA_OFFSET_TWOPHASE: i32 = 16;
const ALLOC_STEPS: i32 = 6;

/// Decoder mode: shared allocation tables and decoder-specific inverse transforms.
pub struct Mode {
    core: crate::opus::rate_common::Mode,
    pub window: [f32; OVERLAP],
    pub mdct: [super::mdct::Imdct; MAX_LM + 1],
}
impl Mode {
    pub fn new() -> Self {
        let mut mode = Self {
            core: crate::opus::rate_common::Mode::new(),
            window: [0.0; OVERLAP],
            mdct: [super::mdct::Imdct::new(1920), super::mdct::Imdct::new(960), super::mdct::Imdct::new(480), super::mdct::Imdct::new(240)],
        };
        for i in 0..OVERLAP {
            let x = std::f64::consts::FRAC_PI_2 * (i as f64 + 0.5) / OVERLAP as f64;
            let s = x.sin();
            mode.window[i] = (std::f64::consts::FRAC_PI_2 * s * s).sin() as f32;
        }
        mode
    }
}
impl std::ops::Deref for Mode {
    type Target = crate::opus::rate_common::Mode;
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

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

/// `clt_compute_allocation` (decoder side).
pub struct AllocationInput<'a, 'd> {
    pub mode: &'a Mode,
    pub start: usize,
    pub end: usize,
    pub offsets: &'a [i32; NB_EBANDS],
    pub cap: &'a [i32; NB_EBANDS],
    pub trim: i32,
    pub total: i32,
    pub channels: usize,
    pub lm: usize,
    pub decoder: &'a mut RangeDecoder<'d>,
}

pub fn compute_allocation(input: AllocationInput<'_, '_>) -> Allocation {
    let AllocationInput { mode: m, start, end, offsets, cap, trim: alloc_trim, total, channels: c, lm, decoder: dec } = input;
    let crate::opus::rate_common::Bracket { skip_start, bits1, bits2, thresh, total, skip_rsv, intensity_rsv, dual_stereo_rsv } =
        crate::opus::rate_common::bracket(start, end, offsets, cap, alloc_trim, total, c, lm);
    interp_bits2pulses(InterpPulsesInput {
        mode: m,
        start,
        end,
        skip_start,
        bits1: &bits1,
        bits2: &bits2,
        threshold: &thresh,
        cap,
        total,
        skip_reservation: skip_rsv,
        intensity_reservation: intensity_rsv,
        stereo_reservation: dual_stereo_rsv,
        channels: c,
        lm,
        decoder: dec,
    })
}

struct InterpPulsesInput<'a, 'd> {
    mode: &'a Mode,
    start: usize,
    end: usize,
    skip_start: usize,
    bits1: &'a [i32; NB_EBANDS],
    bits2: &'a [i32; NB_EBANDS],
    threshold: &'a [i32; NB_EBANDS],
    cap: &'a [i32; NB_EBANDS],
    total: i32,
    skip_reservation: i32,
    intensity_reservation: i32,
    stereo_reservation: i32,
    channels: usize,
    lm: usize,
    decoder: &'a mut RangeDecoder<'d>,
}

fn interp_bits2pulses(input: InterpPulsesInput<'_, '_>) -> Allocation {
    let InterpPulsesInput {
        mode: m,
        start,
        end,
        skip_start,
        bits1,
        bits2,
        threshold: thresh,
        cap,
        mut total,
        skip_reservation: skip_rsv,
        intensity_reservation: mut intensity_rsv,
        stereo_reservation: mut dual_stereo_rsv,
        channels: c,
        lm,
        decoder: dec,
    } = input;
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
        let mut band_bits = bits[j] + percoeff * band_width + rem;
        if band_bits >= thresh[j].max(alloc_floor + (1 << BITRES)) {
            if dec.bit_logp(1) {
                break;
            }
            psum += 1 << BITRES;
            band_bits -= 1 << BITRES;
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
    let intensity = if intensity_rsv > 0 { s + dec.uint((coded_bands + 1 - s) as u32) as usize } else { 0 };
    if intensity <= s {
        total += dual_stereo_rsv;
        dual_stereo_rsv = 0;
    }
    let dual_stereo = if dual_stereo_rsv > 0 { dec.bit_logp(1) } else { false };
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
