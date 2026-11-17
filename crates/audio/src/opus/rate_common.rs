//! CELT pulse cache and allocation caps shared by encoder and decoder.

#[cfg(not(feature = "opus-encode"))]
use super::decode::celt::tables::{ALLOC, EBANDS, LOG2_FRAC_TABLE, MAX_LM, NB_EBANDS};
#[cfg(feature = "opus-encode")]
use super::encoder::tables::{ALLOC, EBANDS, LOG2_FRAC_TABLE, MAX_LM, NB_EBANDS};

const BITRES: u32 = 3;
const MAX_PSEUDO: i32 = 40;
const LOG_MAX_PSEUDO: i32 = 6;
const MAX_FINE_BITS: i32 = 8;
const FINE_OFFSET: i32 = 21;
const QTHETA_OFFSET: i32 = 4;
const QTHETA_OFFSET_TWOPHASE: i32 = 16;
fn ilog(x: u32) -> i32 {
    (32 - x.leading_zeros()) as i32
}

/// Number of pulses represented by pseudo-pulse index `i`.
pub fn get_pulses(i: i32) -> i32 {
    if i < 8 { i } else { (8 + (i & 7)) << ((i >> 3) - 1) }
}

/// Conservative (rounded up) log2 of `val` with `frac` fractional bits.
pub fn log2_frac(mut val: u32, mut frac: u32) -> i32 {
    let mut l = ilog(val);
    if val & val.wrapping_sub(1) != 0 {
        if l > 16 {
            val = ((val - 1) >> (l - 16)) + 1;
        } else {
            val <<= 16 - l;
        }
        let mut l = (l - 1) << frac;
        loop {
            let b = (val >> 16) as i32;
            l += b << frac;
            val = (val + b as u32) >> b;
            val = (val * val + 0x7FFF) >> 15;
            if frac == 0 {
                break;
            }
            frac -= 1;
        }
        l + (val > 0x8000) as i32
    } else {
        l -= 1;
        l << frac
    }
}

/// PVQ codebook size V(N, K) (saturating far above 2^32).
pub fn pvq_v(n: usize, k: usize) -> u128 {
    if k == 0 {
        return 1;
    }
    if n == 0 {
        return 0;
    }
    let mut row = vec![0u128; k + 1];
    row[0] = 1;
    for _ in 0..n {
        let mut prev_diag = row[0];
        for kk in 1..=k {
            let up = row[kk];
            let v = up.saturating_add(row[kk - 1]).saturating_add(prev_diag);
            prev_diag = up;
            row[kk] = v.min(u128::MAX >> 8);
        }
        row[0] = 1;
    }
    row[k]
}

fn fits_in32(n: usize, k: i32) -> bool {
    pvq_v(n, k as usize) <= u32::MAX as u128
}

/// Precomputed per-mode data.
pub struct Mode {
    /// log2 of band width in 1/8 bits.
    pub log_n: [i32; NB_EBANDS],
    pub(crate) cache_index: Vec<i16>,
    pub(crate) cache_bits: Vec<u8>,
    pub(crate) caps: Vec<u8>,
}

impl Mode {
    pub fn new() -> Mode {
        let mut log_n = [0i32; NB_EBANDS];
        for (i, l) in log_n.iter_mut().enumerate() {
            *l = log2_frac((EBANDS[i + 1] - EBANDS[i]) as u32, BITRES);
        }
        let (cache_index, cache_bits) = compute_pulse_cache(MAX_LM as i32);
        let mut m = Mode { log_n, cache_index, cache_bits, caps: Vec::new() };
        m.caps = compute_caps(&m);
        m
    }

    #[inline]
    pub fn cache(&self, lm: i32, band: usize) -> &[u8] {
        let idx = self.cache_index[((lm + 1) as usize) * NB_EBANDS + band];
        &self.cache_bits[idx.max(0) as usize..]
    }

    pub fn bits2pulses(&self, band: usize, lm: i32, bits: i32) -> i32 {
        let cache = self.cache(lm, band);
        let mut lo = 0i32;
        let mut hi = cache[0] as i32;
        let bits = bits - 1;
        for _ in 0..LOG_MAX_PSEUDO {
            let mid = (lo + hi + 1) >> 1;
            if cache[mid as usize] as i32 >= bits {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let lo_bits = if lo == 0 { -1 } else { cache[lo as usize] as i32 };
        if bits - lo_bits <= cache[hi as usize] as i32 - bits { lo } else { hi }
    }

    pub fn pulses2bits(&self, band: usize, lm: i32, pulses: i32) -> i32 {
        if pulses == 0 { 0 } else { self.cache(lm, band)[pulses as usize] as i32 + 1 }
    }

    /// Per-band allocation caps for this frame (`init_caps`).
    pub fn init_caps(&self, lm: usize, c: usize) -> [i32; NB_EBANDS] {
        let mut cap = [0i32; NB_EBANDS];
        for (i, cp) in cap.iter_mut().enumerate() {
            let n = (EBANDS[i + 1] - EBANDS[i]) << lm;
            *cp = ((self.caps[NB_EBANDS * (2 * lm + c - 1) + i] as i32 + 64) * c as i32 * n) >> 2;
        }
        cap
    }
}

fn compute_pulse_cache(lm_max: i32) -> (Vec<i16>, Vec<u8>) {
    let nb = NB_EBANDS;
    let rows = (lm_max + 2) as usize;
    let mut index = vec![-1i16; nb * rows];
    let mut entries: Vec<(usize, i32, usize)> = Vec::new();
    let mut curr = 0usize;
    for i in 0..rows {
        for j in 0..nb {
            let n = (((EBANDS[j + 1] - EBANDS[j]) << i) >> 1) as usize;
            index[i * nb + j] = -1;
            'search: for k in 0..=i {
                for nn in 0..nb {
                    if k == i && nn >= j {
                        break;
                    }
                    if n == (((EBANDS[nn + 1] - EBANDS[nn]) << k) >> 1) as usize {
                        index[i * nb + j] = index[k * nb + nn];
                        break 'search;
                    }
                }
            }
            if index[i * nb + j] == -1 && n != 0 {
                let mut k = 0;
                while k < MAX_PSEUDO && fits_in32(n, get_pulses(k + 1)) {
                    k += 1;
                }
                index[i * nb + j] = curr as i16;
                entries.push((n, k, curr));
                curr += (k + 1) as usize;
            }
        }
    }
    let mut bits = vec![0u8; curr];
    for &(n, k, off) in &entries {
        let maxk = get_pulses(k) as usize;
        let tmp = required_bits(n, maxk, BITRES);
        for j in 1..=k {
            bits[off + j as usize] = (tmp[get_pulses(j) as usize] - 1) as u8;
        }
        bits[off] = k as u8;
    }
    (index, bits)
}

fn required_bits(n: usize, maxk: usize, frac: u32) -> Vec<i32> {
    let mut out = vec![0i32; maxk + 1];
    if n == 1 {
        for o in out.iter_mut().skip(1) {
            *o = 1 << frac;
        }
    } else {
        for (k, o) in out.iter_mut().enumerate().skip(1) {
            *o = log2_frac(pvq_v(n, k) as u32, frac);
        }
    }
    out
}

fn compute_caps(m: &Mode) -> Vec<u8> {
    let nb = NB_EBANDS;
    let mut caps = vec![0u8; (MAX_LM + 1) * 2 * nb];
    for i in 0..=MAX_LM as i32 {
        for c in 1..=2i32 {
            for j in 0..nb {
                let mut n0 = EBANDS[j + 1] - EBANDS[j];

                let max_bits = if n0 << i == 1 {
                    (c * (1 + MAX_FINE_BITS)) << BITRES
                } else {
                    let mut lm0 = 0;
                    if n0 > 2 {
                        n0 >>= 1;
                        lm0 -= 1;
                    } else if n0 <= 1 {
                        lm0 = i.min(1);
                        n0 <<= lm0;
                    }
                    let pcache = m.cache(lm0, j);
                    let mut mb = pcache[pcache[0] as usize] as i32 + 1;
                    let mut n = n0;
                    for k in 0..(i - lm0) {
                        mb <<= 1;
                        let offset = ((m.log_n[j] + ((lm0 + k) << BITRES)) >> 1) - QTHETA_OFFSET;
                        let num = 459 * ((2 * n - 1) * offset + mb);
                        let den = ((2 * n - 1) << 9) - 459;
                        let qb = ((num + (den >> 1)) / den).min(57);
                        mb += qb;
                        n <<= 1;
                    }
                    if c == 2 {
                        mb <<= 1;
                        let offset = ((m.log_n[j] + (i << BITRES)) >> 1) - if n == 2 { QTHETA_OFFSET_TWOPHASE } else { QTHETA_OFFSET };
                        let ndof = 2 * n - 1 - (n == 2) as i32;
                        let p = if n == 2 { 512 } else { 487 };
                        let num = p * (mb + ndof * offset);
                        let den = (ndof << 9) - p;
                        let qb = ((num + (den >> 1)) / den).min(if n == 2 { 64 } else { 61 });
                        mb += qb;
                    }
                    let ndof = c * n + if c == 2 && n > 2 { 1 } else { 0 };
                    let mut offset = ((m.log_n[j] + (i << BITRES)) >> 1) - FINE_OFFSET;
                    if n == 2 {
                        offset += (1 << BITRES) >> 2;
                    }
                    let num = mb + ndof * offset;
                    let den = (ndof - 1) << BITRES;
                    let qb = ((num + (den >> 1)) / den).min(MAX_FINE_BITS);
                    mb += (c * qb) << BITRES;
                    mb
                };
                let w = c * ((EBANDS[j + 1] - EBANDS[j]) << i);
                let v = (4 * max_bits / w) - 64;
                caps[((i * 2 + c - 1) as usize) * nb + j] = v.clamp(0, 255) as u8;
            }
        }
    }
    caps
}

/// Shared allocation search before encoder/decoder signalling diverges.
pub struct Bracket {
    pub skip_start: usize,
    pub bits1: [i32; NB_EBANDS],
    pub bits2: [i32; NB_EBANDS],
    pub thresh: [i32; NB_EBANDS],
    pub total: i32,
    pub skip_rsv: i32,
    pub intensity_rsv: i32,
    pub dual_stereo_rsv: i32,
}

pub fn bracket(
    start: usize,
    end: usize,
    offsets: &[i32; NB_EBANDS],
    cap: &[i32; NB_EBANDS],
    alloc_trim: i32,
    total: i32,
    c: usize,
    lm: usize,
) -> Bracket {
    let ci = c as i32;
    let lmi = lm as i32;
    let mut total = total.max(0);
    let mut skip_start = start;
    let skip_rsv = if total >= 1 << BITRES { 1 << BITRES } else { 0 };
    total -= skip_rsv;
    let mut intensity_rsv = 0;
    let mut dual_stereo_rsv = 0;
    if c == 2 {
        intensity_rsv = LOG2_FRAC_TABLE[end - start];
        if intensity_rsv > total {
            intensity_rsv = 0;
        } else {
            total -= intensity_rsv;
            dual_stereo_rsv = if total >= 1 << BITRES { 1 << BITRES } else { 0 };
            total -= dual_stereo_rsv;
        }
    }
    let mut bits1 = [0i32; NB_EBANDS];
    let mut bits2 = [0i32; NB_EBANDS];
    let mut thresh = [0i32; NB_EBANDS];
    let mut trim_offset = [0i32; NB_EBANDS];
    for j in start..end {
        let w = EBANDS[j + 1] - EBANDS[j];
        thresh[j] = (ci << BITRES).max(((3 * w) << lmi << BITRES) >> 4);
        trim_offset[j] = (ci * w * (alloc_trim - 5 - lmi) * (end as i32 - j as i32 - 1) * (1 << (lmi + BITRES as i32))) >> 6;
        if w << lmi == 1 {
            trim_offset[j] -= ci << BITRES;
        }
    }
    let nb_alloc = ALLOC.len() as i32;
    let mut lo = 1;
    let mut hi = nb_alloc - 1;
    loop {
        let mut done = false;
        let mut psum = 0;
        let mid = (lo + hi) >> 1;
        for j in (start..end).rev() {
            let n = EBANDS[j + 1] - EBANDS[j];
            let mut bitsj = (ci * n * (ALLOC[mid as usize][j] as i32)) << lmi >> 2;
            if bitsj > 0 {
                bitsj = (bitsj + trim_offset[j]).max(0);
            }
            bitsj += offsets[j];
            if bitsj >= thresh[j] || done {
                done = true;
                psum += bitsj.min(cap[j]);
            } else if bitsj >= ci << BITRES {
                psum += ci << BITRES;
            }
        }
        if psum > total {
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
        if lo > hi {
            break;
        }
    }
    hi = lo;
    lo -= 1;
    for j in start..end {
        let n = EBANDS[j + 1] - EBANDS[j];
        let mut bits1j = (ci * n * (ALLOC[lo as usize][j] as i32)) << lmi >> 2;
        let mut bits2j = if hi >= nb_alloc { cap[j] } else { (ci * n * (ALLOC[hi as usize][j] as i32)) << lmi >> 2 };
        if bits1j > 0 {
            bits1j = (bits1j + trim_offset[j]).max(0);
        }
        if bits2j > 0 {
            bits2j = (bits2j + trim_offset[j]).max(0);
        }
        if lo > 0 {
            bits1j += offsets[j];
        }
        bits2j += offsets[j];
        if offsets[j] > 0 {
            skip_start = j;
        }
        bits2j = (bits2j - bits1j).max(0);
        bits1[j] = bits1j;
        bits2[j] = bits2j;
    }
    Bracket { skip_start, bits1, bits2, thresh, total, skip_rsv, intensity_rsv, dual_stereo_rsv }
}
