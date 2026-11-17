//! GSM 06.10 full-rate speech (RPE-LTP): a decoder (feature `gsm-decode`) and an encoder (feature
//! `gsm-encode`), 8 kHz mono, 13 kbit/s.
//!
//! Written from the public ETSI specification (GSM 06.10 / 3GPP TS 46.010, clause 5: the bit-exact
//! fixed-point description of the coder and decoder, and its tables). A frame is 160 samples
//! (20 ms) coded as 76 parameters in 260 bits: eight log-area ratios for the short-term filter,
//! then four sub-frames of 40 samples, each with a long-term lag and gain, an RPE grid position,
//! a block amplitude and thirteen 3-bit pulses.
//!
//! The arithmetic is the specification's own 16-bit saturating arithmetic, so the output is bit
//! exact. Samples are `i16`; the coder uses the upper 13 bits of its input and the decoder's
//! output has its three low bits clear, as the specification says.
//!
//! Two packings of the 260 bits are supported ([`Packing`]):
//!
//! - [`Packing::Raw`]: 33 bytes per frame, the signature nibble `0xD` and then the parameters
//!   most significant bit first (the packing of RFC 3551 and of `.gsm` files).
//! - [`Packing::Wav49`]: 65 bytes per pair of frames, the parameters least significant bit first
//!   (the packing of the WAV format tag `0x31`, "GSM 6.10").
//!
//! Any 260 bits are a decodable frame, so decoding never fails on content, only on length.
//! Each stream starts from the specification's initial state.

// Index loops follow the specification's pseudo-code, which numbers the arrays the same way.
#![allow(clippy::needless_range_loop)]
// The two halves share the arithmetic, tables and synthesis helpers; with only one feature on, some
// of them are unused.
#![cfg_attr(not(all(feature = "gsm-decode", feature = "gsm-encode")), allow(dead_code))]

use std::fmt;

/// Samples in a frame.
pub const FRAME_SAMPLES: usize = 160;
/// Samples in a sub-frame.
pub const SUBFRAME_SAMPLES: usize = 40;
/// Bytes of a frame in the [`Packing::Raw`] packing.
pub const RAW_FRAME_BYTES: usize = 33;
/// Bytes of a pair of frames in the [`Packing::Wav49`] packing.
pub const WAV49_BLOCK_BYTES: usize = 65;
/// The sampling rate, in Hz.
pub const SAMPLE_RATE: u32 = 8000;

/// How the 260 bits of a frame are laid out in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Packing {
    /// 33 bytes per frame: `0xD` then the parameters, most significant bit first.
    Raw,
    /// 65 bytes per two frames: the parameters least significant bit first, with no signature.
    Wav49,
}

impl Packing {
    /// Bytes of one block: a frame for [`Packing::Raw`], two frames for [`Packing::Wav49`].
    #[must_use]
    pub const fn block_bytes(self) -> usize {
        match self {
            Self::Raw => RAW_FRAME_BYTES,
            Self::Wav49 => WAV49_BLOCK_BYTES,
        }
    }

    /// Samples of one block.
    #[must_use]
    pub const fn block_samples(self) -> usize {
        match self {
            Self::Raw => FRAME_SAMPLES,
            Self::Wav49 => 2 * FRAME_SAMPLES,
        }
    }
}

/// What went wrong reading GSM data.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The data is not a whole number of blocks.
    Truncated {
        /// The bytes in each block.
        block: usize,
        /// The bytes given.
        len: usize,
    },
    /// A raw frame does not start with the signature nibble `0xD`.
    BadSignature {
        /// The byte offset of the frame.
        offset: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { block, len } => write!(f, "GSM data of {len} bytes is not a whole number of {block}-byte blocks"),
            Self::BadSignature { offset } => write!(f, "GSM frame at byte {offset} does not start with the signature 0xD"),
        }
    }
}

impl std::error::Error for Error {}

// ------------------------------------------------------------------ the specification's arithmetic

const MIN16: i32 = -32768;
const MAX16: i32 = 32767;

fn sat16(x: i32) -> i32 {
    x.clamp(MIN16, MAX16)
}
fn add(a: i32, b: i32) -> i32 {
    sat16(a + b)
}
fn sub(a: i32, b: i32) -> i32 {
    sat16(a - b)
}
fn mult(a: i32, b: i32) -> i32 {
    if a == MIN16 && b == MIN16 { MAX16 } else { (a * b) >> 15 }
}
fn mult_r(a: i32, b: i32) -> i32 {
    if a == MIN16 && b == MIN16 { MAX16 } else { (a * b + 16384) >> 15 }
}
fn abs16(a: i32) -> i32 {
    if a == MIN16 { MAX16 } else { a.abs() }
}
fn l_add(a: i32, b: i32) -> i32 {
    a.saturating_add(b)
}
fn l_sub(a: i32, b: i32) -> i32 {
    a.saturating_sub(b)
}
fn l_mult(a: i32, b: i32) -> i32 {
    let p = (i64::from(a) * i64::from(b)) << 1;
    p.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}
/// A 16-bit variable assigned from a wider value: the low 16 bits, signed.
fn w16(x: i32) -> i32 {
    i32::from(x as i16)
}
/// The specification's `<<`: a negative count shifts right.
fn asl(x: i32, n: i32) -> i32 {
    if n >= 0 { x.wrapping_shl(n.min(31) as u32) } else { asr(x, -n) }
}
/// The specification's `>>`: a negative count shifts left.
fn asr(x: i32, n: i32) -> i32 {
    if n >= 0 { x >> n.min(31) } else { asl(x, -n) }
}
/// Left shifts that normalize a 32-bit value into `[2^30, 2^31)` (or its negative counterpart).
fn norm(x: i32) -> i32 {
    if x == 0 {
        return 0;
    }
    let y = if x < 0 { !x } else { x };
    if y == 0 { 31 } else { y.leading_zeros() as i32 - 1 }
}
/// Fractional division of `num` by `denum`, `0 < num <= denum`, as the specification gives it.
fn div(num: i32, denum: i32) -> i32 {
    let (mut l_num, mut q) = (num, 0);
    for _ in 0..15 {
        q <<= 1;
        l_num = l_num.wrapping_shl(1);
        if l_num >= denum {
            l_num = l_num.wrapping_sub(denum);
            q += 1;
        }
    }
    q
}

// ------------------------------------------------------------------------------ the tables

/// Table 5.1: `A`, `B`, `MIC`, `MAC` of the log-area ratio quantizers.
const A: [i32; 8] = [20480, 20480, 20480, 20480, 13964, 15360, 8534, 9036];
const B: [i32; 8] = [0, 0, 2048, -2560, 94, -1792, -341, -1144];
const MIC: [i32; 8] = [-32, -32, -16, -16, -8, -8, -4, -4];
const MAC: [i32; 8] = [31, 31, 15, 15, 7, 7, 3, 3];
/// Table 5.2: `1 / A`.
const INVA: [i32; 8] = [13107, 13107, 13107, 13107, 19223, 17476, 31454, 29708];
/// Table 5.3a: decision levels of the long-term gain quantizer.
const DLB: [i32; 4] = [6554, 16384, 26214, 32767];
/// Table 5.3b: its quantization levels.
const QLB: [i32; 4] = [3277, 11469, 21299, 32767];
/// Table 5.4: the weighting filter.
const H: [i32; 11] = [-134, -374, 0, 2054, 5741, 8192, 5741, 2054, 0, -374, -134];
/// Table 5.5: normalized inverse mantissa.
const NRFAC: [i32; 8] = [29128, 26215, 23832, 21846, 20165, 18725, 17476, 16384];
/// Table 5.6: normalized direct mantissa.
const FAC: [i32; 8] = [18431, 20479, 22527, 24575, 26623, 28671, 30719, 32767];

// -------------------------------------------------------------------------- the parameters

/// The parameters of one 40-sample sub-frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubFrame {
    /// Long-term lag, 7 bits (40 to 120 from a coder).
    pub lag: u8,
    /// Long-term gain index, 2 bits.
    pub gain: u8,
    /// RPE grid position, 2 bits.
    pub grid: u8,
    /// Block amplitude, 6 bits.
    pub xmax: u8,
    /// The 13 RPE pulses, 3 bits each.
    pub pulses: [u8; 13],
}

/// The 76 parameters of one frame. A decoder uses only the low bits each field is allowed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Frame {
    /// The eight log-area ratios, 6, 6, 5, 5, 4, 4, 3 and 3 bits, offset to be non-negative.
    pub lar: [u8; 8],
    /// The four sub-frames.
    pub sub: [SubFrame; 4],
}

const LAR_BITS: [u32; 8] = [6, 6, 5, 5, 4, 4, 3, 3];
const SUB_FIELDS: usize = 4 + 13;
const PARAMS: usize = 8 + 4 * SUB_FIELDS;

fn field_bits(i: usize) -> u32 {
    if i < 8 {
        return LAR_BITS.get(i).copied().unwrap_or(0);
    }
    match (i - 8) % SUB_FIELDS {
        0 => 7,
        1 | 2 => 2,
        3 => 6,
        _ => 3,
    }
}

impl Frame {
    fn values(&self) -> [u8; PARAMS] {
        let mut v = [0u8; PARAMS];
        v[..8].copy_from_slice(&self.lar);
        for (j, s) in self.sub.iter().enumerate() {
            let at = 8 + j * SUB_FIELDS;
            v[at] = s.lag;
            v[at + 1] = s.gain;
            v[at + 2] = s.grid;
            v[at + 3] = s.xmax;
            v[at + 4..at + SUB_FIELDS].copy_from_slice(&s.pulses);
        }
        v
    }

    fn from_values(v: &[u8; PARAMS]) -> Self {
        let mut f = Self::default();
        f.lar.copy_from_slice(&v[..8]);
        for (j, s) in f.sub.iter_mut().enumerate() {
            let at = 8 + j * SUB_FIELDS;
            s.lag = v[at];
            s.gain = v[at + 1];
            s.grid = v[at + 2];
            s.xmax = v[at + 3];
            s.pulses.copy_from_slice(&v[at + 4..at + SUB_FIELDS]);
        }
        f
    }

    /// The frame in the [`Packing::Raw`] layout.
    #[must_use]
    pub fn to_raw(&self) -> [u8; RAW_FRAME_BYTES] {
        let mut out = [0u8; RAW_FRAME_BYTES];
        out[0] = 0xD0;
        let mut pos = 4usize;
        for (i, &v) in self.values().iter().enumerate() {
            let w = field_bits(i);
            for b in (0..w).rev() {
                if (u32::from(v) >> b) & 1 == 1 {
                    out[pos / 8] |= 0x80 >> (pos % 8);
                }
                pos += 1;
            }
        }
        out
    }

    /// A frame from the [`Packing::Raw`] layout; fails when the signature nibble is not `0xD`.
    pub fn from_raw(bytes: &[u8; RAW_FRAME_BYTES]) -> Result<Self, Error> {
        if bytes[0] >> 4 != 0xD {
            return Err(Error::BadSignature { offset: 0 });
        }
        let mut v = [0u8; PARAMS];
        let mut pos = 4usize;
        for (i, slot) in v.iter_mut().enumerate() {
            let mut x = 0u8;
            for _ in 0..field_bits(i) {
                x = (x << 1) | ((bytes[pos / 8] >> (7 - pos % 8)) & 1);
                pos += 1;
            }
            *slot = x;
        }
        Ok(Self::from_values(&v))
    }

    /// Two frames in the [`Packing::Wav49`] layout.
    #[must_use]
    pub fn pair_to_wav49(a: &Self, b: &Self) -> [u8; WAV49_BLOCK_BYTES] {
        let mut out = [0u8; WAV49_BLOCK_BYTES];
        let mut pos = 0usize;
        for f in [a, b] {
            for (i, &v) in f.values().iter().enumerate() {
                for bit in 0..field_bits(i) {
                    if (u32::from(v) >> bit) & 1 == 1 {
                        out[pos / 8] |= 1 << (pos % 8);
                    }
                    pos += 1;
                }
            }
        }
        out
    }

    /// Two frames from the [`Packing::Wav49`] layout.
    #[must_use]
    pub fn pair_from_wav49(bytes: &[u8; WAV49_BLOCK_BYTES]) -> [Self; 2] {
        let mut pos = 0usize;
        let mut frames = [Self::default(); 2];
        for f in &mut frames {
            let mut v = [0u8; PARAMS];
            for (i, slot) in v.iter_mut().enumerate() {
                let mut x = 0u8;
                for bit in 0..field_bits(i) {
                    x |= ((bytes[pos / 8] >> (pos % 8)) & 1) << bit;
                    pos += 1;
                }
                *slot = x;
            }
            *f = Self::from_values(&v);
        }
        frames
    }
}

// ------------------------------------------------------------------- shared synthesis pieces

/// Clause 5.2.8: the decoded log-area ratios `LARpp` of a frame's coded ones.
fn decode_lar(lar: &[u8; 8]) -> [i32; 8] {
    let mut out = [0i32; 8];
    for i in 0..8 {
        let c = i32::from(lar[i] & ((1u8 << LAR_BITS[i]) - 1));
        let t1 = w16(add(c, MIC[i]) << 10);
        let t2 = w16(B[i] << 1);
        let t1 = sub(t1, t2);
        let t1 = mult_r(INVA[i], t1);
        out[i] = add(t1, t1);
    }
    out
}

/// Clause 5.2.9.1: the interpolated log-area ratios of segment `seg` (0 to 3).
fn interpolate(prev: &[i32; 8], cur: &[i32; 8], seg: usize) -> [i32; 8] {
    let mut out = [0i32; 8];
    for i in 0..8 {
        out[i] = match seg {
            0 => add(add(prev[i] >> 2, cur[i] >> 2), prev[i] >> 1),
            1 => add(prev[i] >> 1, cur[i] >> 1),
            2 => add(add(prev[i] >> 2, cur[i] >> 2), cur[i] >> 1),
            _ => cur[i],
        };
    }
    out
}

/// Clause 5.2.9.2: reflection coefficients from log-area ratios.
fn reflection(larp: &[i32; 8]) -> [i32; 8] {
    let mut rp = [0i32; 8];
    for i in 0..8 {
        let a = abs16(larp[i]);
        let t = if a < 11059 {
            w16(a << 1)
        } else if a < 20070 {
            add(a, 11059)
        } else {
            add(a >> 2, 26112)
        };
        rp[i] = if larp[i] < 0 { sub(0, t) } else { t };
    }
    rp
}

/// The sample ranges the four sets of interpolated coefficients cover.
const SEGMENTS: [(usize, usize); 4] = [(0, 12), (13, 26), (27, 39), (40, 159)];

/// The exponent and mantissa of a block amplitude (clause 5.2.15).
fn exp_mant(xmaxc: i32) -> (i32, i32) {
    let mut exp = 0;
    if xmaxc > 15 {
        exp = (xmaxc >> 3) - 1;
    }
    let mut mant = xmaxc - (exp << 3);
    if mant == 0 {
        exp = -4;
        mant = 15;
    } else {
        let mut itest = false;
        for _ in 0..3 {
            if mant > 7 {
                itest = true;
            }
            if !itest {
                mant = (mant << 1) + 1;
                exp -= 1;
            }
        }
    }
    (exp, mant - 8)
}

/// Clauses 5.2.16 and 5.2.17: the 40 samples of the long-term residual from a sub-frame's RPE data.
fn rpe_decode(s: &SubFrame) -> [i32; SUBFRAME_SAMPLES] {
    let (exp, mant) = exp_mant(i32::from(s.xmax & 0x3F));
    let temp1 = FAC[mant as usize & 7];
    let temp2 = 6 - exp;
    let temp3 = asl(1, temp2 - 1);
    let mut ep = [0i32; SUBFRAME_SAMPLES];
    let grid = usize::from(s.grid & 3);
    for (i, &p) in s.pulses.iter().enumerate() {
        let t = sub(i32::from(p & 7) << 1, 7);
        let t = w16(t << 12);
        let t = mult_r(temp1, t);
        let t = add(t, temp3);
        let at = grid + 3 * i;
        if at < SUBFRAME_SAMPLES {
            ep[at] = asr(t, temp2);
        }
    }
    ep
}

/// The long-term lag, gain and reconstructed-residual history shared by coder and decoder.
#[derive(Clone)]
struct Synth {
    /// The reconstructed short-term residual: `dp[-120..-1]`, oldest first.
    dp: [i32; 120],
    nrp: i32,
    /// The short-term synthesis (or analysis) lattice state.
    v: [i32; 9],
    prev_lar: [i32; 8],
}

impl Synth {
    fn new() -> Self {
        Self { dp: [0; 120], nrp: 40, v: [0; 9], prev_lar: [0; 8] }
    }

    /// Clauses 5.3.1 and 5.3.2 for one sub-frame: the reconstructed residual, which is also pushed
    /// into the history.
    fn long_term(&mut self, s: &SubFrame) -> [i32; SUBFRAME_SAMPLES] {
        let erp = rpe_decode(s);
        let mut nr = i32::from(s.lag & 0x7F);
        if !(40..=120).contains(&nr) {
            nr = self.nrp;
        }
        self.nrp = nr;
        let brp = QLB[usize::from(s.gain & 3)];
        let mut drp = [0i32; SUBFRAME_SAMPLES];
        for k in 0..SUBFRAME_SAMPLES {
            // dp[k - nr] with k - nr in -120..=-1 is the history; later samples are this sub-frame's own.
            let idx = 120 + k as i32 - nr;
            let past = if idx < 120 { self.dp[idx as usize] } else { drp[(idx - 120) as usize] };
            drp[k] = add(erp[k], mult_r(brp, past));
        }
        self.dp.copy_within(40.., 0);
        self.dp[80..].copy_from_slice(&drp);
        drp
    }
}

// ------------------------------------------------------------------------------ the decoder

#[cfg(feature = "gsm-decode")]
/// A GSM 06.10 decoder. It keeps the filter and long-term state between frames.
#[derive(Clone)]
pub struct Decoder {
    synth: Synth,
    msr: i32,
}

#[cfg(feature = "gsm-decode")]
impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "gsm-decode")]
impl Decoder {
    /// A decoder in the specification's initial state.
    #[must_use]
    pub fn new() -> Self {
        Self { synth: Synth::new(), msr: 0 }
    }

    /// Decode one frame into 160 samples (clause 5.3).
    pub fn decode_frame(&mut self, frame: &Frame) -> [i16; FRAME_SAMPLES] {
        let cur = decode_lar(&frame.lar);
        let mut wt = [0i32; FRAME_SAMPLES];
        for (j, s) in frame.sub.iter().enumerate() {
            let drp = self.synth.long_term(s);
            wt[j * SUBFRAME_SAMPLES..(j + 1) * SUBFRAME_SAMPLES].copy_from_slice(&drp);
        }
        let mut sr = [0i32; FRAME_SAMPLES];
        for (seg, &(k0, k1)) in SEGMENTS.iter().enumerate() {
            let rrp = reflection(&interpolate(&self.synth.prev_lar, &cur, seg));
            for k in k0..=k1 {
                let mut sri = wt[k];
                for i in 1..=8 {
                    sri = sub(sri, mult_r(rrp[8 - i], self.synth.v[8 - i]));
                    self.synth.v[9 - i] = add(self.synth.v[8 - i], mult_r(rrp[8 - i], sri));
                }
                sr[k] = sri;
                self.synth.v[0] = sri;
            }
        }
        self.synth.prev_lar = cur;
        let mut out = [0i16; FRAME_SAMPLES];
        for k in 0..FRAME_SAMPLES {
            self.msr = add(sr[k], mult_r(self.msr, 28180));
            let up = add(self.msr, self.msr);
            out[k] = ((up >> 3) << 3) as i16;
        }
        out
    }
}

// ------------------------------------------------------------------------------ the encoder

#[cfg(feature = "gsm-encode")]
/// A GSM 06.10 encoder. It keeps the filter and long-term state between frames.
#[derive(Clone)]
pub struct Encoder {
    synth: Synth,
    z1: i32,
    l_z2: i32,
    mp: i32,
    /// The short-term analysis filter state.
    u: [i32; 8],
}

#[cfg(feature = "gsm-encode")]
impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "gsm-encode")]
impl Encoder {
    /// An encoder in the specification's initial state.
    #[must_use]
    pub fn new() -> Self {
        Self { synth: Synth::new(), z1: 0, l_z2: 0, mp: 0, u: [0; 8] }
    }

    /// Clauses 5.2.0 to 5.2.3: scaling, offset compensation and pre-emphasis.
    fn preprocess(&mut self, input: &[i16; FRAME_SAMPLES]) -> [i32; FRAME_SAMPLES] {
        let mut s = [0i32; FRAME_SAMPLES];
        for k in 0..FRAME_SAMPLES {
            let so = (i32::from(input[k]) >> 3) << 2;
            let s1 = sub(so, self.z1);
            self.z1 = so;
            let mut l_s2 = s1 << 15;
            let msp = self.l_z2 >> 15;
            let lsp = l_sub(self.l_z2, msp << 15);
            let temp = mult_r(lsp, 32735);
            l_s2 = l_add(l_s2, temp);
            self.l_z2 = l_add(l_mult(msp, 32735) >> 1, l_s2);
            let sof = w16(l_add(self.l_z2, 16384) >> 15);
            s[k] = add(sof, mult_r(self.mp, -28180));
            self.mp = sof;
        }
        s
    }

    /// Clauses 5.2.4 to 5.2.7: the coded log-area ratios of the frame.
    fn analyse(s: &[i32; FRAME_SAMPLES]) -> [u8; 8] {
        // Autocorrelation, with the signal scaled so the sums cannot overflow.
        let smax = s.iter().map(|&x| abs16(x)).max().unwrap_or(0);
        let scalauto = if smax == 0 { 0 } else { 4 - norm(smax << 16) };
        let mut sc = *s;
        if scalauto > 0 {
            let temp = 16384 >> (scalauto - 1);
            for x in &mut sc {
                *x = mult_r(*x, temp);
            }
        }
        let mut l_acf = [0i32; 9];
        for (k, acf) in l_acf.iter_mut().enumerate() {
            for i in k..FRAME_SAMPLES {
                *acf = l_add(*acf, l_mult(sc[i], sc[i - k]));
            }
        }
        // Schur recursion for the reflection coefficients.
        let mut r = [0i32; 9];
        if l_acf[0] != 0 {
            let t = norm(l_acf[0]);
            let mut acf = [0i32; 9];
            for k in 0..9 {
                acf[k] = w16(l_acf[k].wrapping_shl(t as u32) >> 16);
            }
            let mut kk = [0i32; 9];
            for i in 1..=7 {
                kk[9 - i] = acf[i];
            }
            let mut p = acf;
            for n in 1..=8 {
                if p[0] < abs16(p[1]) {
                    break;
                }
                r[n] = div(abs16(p[1]), p[0]);
                if p[1] > 0 {
                    r[n] = sub(0, r[n]);
                }
                if n == 8 {
                    break;
                }
                p[0] = add(p[0], mult_r(p[1], r[n]));
                for m in 1..=8 - n {
                    p[m] = add(p[m + 1], mult_r(kk[9 - m], r[n]));
                    kk[9 - m] = add(kk[9 - m], mult_r(p[m + 1], r[n]));
                }
            }
        }
        // Log-area ratios, quantized and offset to be non-negative.
        let mut lar = [0u8; 8];
        for i in 0..8 {
            let mut t = abs16(r[i + 1]);
            t = if t < 22118 {
                t >> 1
            } else if t < 31130 {
                sub(t, 11059)
            } else {
                w16(sub(t, 26112) << 2)
            };
            let l = if r[i + 1] < 0 { sub(0, t) } else { t };
            let mut c = add(add(mult(A[i], l), B[i]), 256) >> 9;
            c = c.clamp(MIC[i], MAC[i]);
            lar[i] = (c - MIC[i]) as u8;
        }
        lar
    }

    /// Encode 160 samples into one frame (clause 5.2).
    pub fn encode_frame(&mut self, input: &[i16; FRAME_SAMPLES]) -> Frame {
        let s = self.preprocess(input);
        let mut frame = Frame { lar: Self::analyse(&s), ..Frame::default() };

        // Short-term analysis filtering with the quantized coefficients.
        let cur = decode_lar(&frame.lar);
        let mut d = [0i32; FRAME_SAMPLES];
        for (seg, &(k0, k1)) in SEGMENTS.iter().enumerate() {
            let rp = reflection(&interpolate(&self.synth.prev_lar, &cur, seg));
            for k in k0..=k1 {
                let mut di = s[k];
                let mut sav = di;
                for i in 0..8 {
                    let temp = add(self.u[i], mult_r(rp[i], di));
                    di = add(di, mult_r(rp[i], self.u[i]));
                    self.u[i] = sav;
                    sav = temp;
                }
                d[k] = di;
            }
        }
        self.synth.prev_lar = cur;

        for (j, sub_frame) in frame.sub.iter_mut().enumerate() {
            let mut dj = [0i32; SUBFRAME_SAMPLES];
            dj.copy_from_slice(&d[j * SUBFRAME_SAMPLES..(j + 1) * SUBFRAME_SAMPLES]);
            *sub_frame = self.code_subframe(&dj);
        }
        frame
    }

    /// Clauses 5.2.11 to 5.2.18 for one sub-frame.
    fn code_subframe(&mut self, d: &[i32; SUBFRAME_SAMPLES]) -> SubFrame {
        let dp = self.synth.dp;
        let hist = |k: usize, lambda: i32| dp[(120 + k as i32 - lambda) as usize];

        // Long-term lag: the maximum cross-correlation with the reconstructed residual.
        let dmax = d.iter().map(|&x| abs16(x)).max().unwrap_or(0);
        let mut temp = 0;
        let mut scal = 0;
        if dmax != 0 {
            temp = norm(dmax << 16);
        }
        if temp <= 6 {
            scal = 6 - temp;
        }
        let mut wt = [0i32; SUBFRAME_SAMPLES];
        for k in 0..SUBFRAME_SAMPLES {
            wt[k] = d[k] >> scal;
        }
        let mut l_max = 0i32;
        let mut nc = 40i32;
        for lambda in 40..=120 {
            let mut l_result = 0i32;
            for k in 0..SUBFRAME_SAMPLES {
                l_result = l_add(l_result, l_mult(wt[k], hist(k, lambda)));
            }
            if l_result > l_max {
                nc = lambda;
                l_max = l_result;
            }
        }
        l_max >>= 6 - scal;

        // Long-term gain.
        for k in 0..SUBFRAME_SAMPLES {
            wt[k] = hist(k, nc) >> 3;
        }
        let mut l_power = 0i32;
        for k in 0..SUBFRAME_SAMPLES {
            l_power = l_add(l_power, l_mult(wt[k], wt[k]));
        }
        let bc = if l_max <= 0 {
            0
        } else if l_max >= l_power {
            3
        } else {
            let t = norm(l_power);
            let r = w16(l_max.wrapping_shl(t as u32) >> 16);
            let s = w16(l_power.wrapping_shl(t as u32) >> 16);
            (0..3).find(|&b| r <= mult(s, DLB[b])).unwrap_or(3)
        };

        // Long-term analysis filtering.
        let bp = QLB[bc];
        let mut dpp = [0i32; SUBFRAME_SAMPLES];
        let mut e = [0i32; SUBFRAME_SAMPLES];
        for k in 0..SUBFRAME_SAMPLES {
            dpp[k] = mult_r(bp, hist(k, nc));
            e[k] = sub(d[k], dpp[k]);
        }

        // Weighting filter, RPE grid selection.
        let mut w = [0i32; 50];
        w[5..45].copy_from_slice(&e);
        let mut x = [0i32; SUBFRAME_SAMPLES];
        for k in 0..SUBFRAME_SAMPLES {
            let mut l_result = 8192i32;
            for i in 0..11 {
                l_result = l_add(l_result, l_mult(w[k + i], H[i]));
            }
            l_result = l_add(l_result, l_result);
            l_result = l_add(l_result, l_result);
            x[k] = l_result >> 16;
        }
        let mut em = 0i32;
        let mut mc = 0usize;
        for m in 0..4 {
            let mut l_result = 0i32;
            for i in 0..13 {
                let t = x[m + 3 * i] >> 2;
                l_result = l_add(l_result, l_mult(t, t));
            }
            if l_result > em {
                mc = m;
                em = l_result;
            }
        }
        let mut xm = [0i32; 13];
        for i in 0..13 {
            xm[i] = x[mc + 3 * i];
        }

        // APCM quantization.
        let xmax = xm.iter().map(|&v| abs16(v)).max().unwrap_or(0);
        let mut exp = 0;
        let mut t = xmax >> 9;
        let mut itest = false;
        for _ in 0..=5 {
            if t <= 0 {
                itest = true;
            }
            t >>= 1;
            if !itest {
                exp += 1;
            }
        }
        let xmaxc = add(xmax >> (exp + 5), exp << 3);
        let (exp, mant) = exp_mant(xmaxc);
        let temp1 = 6 - exp;
        let temp2 = NRFAC[mant as usize & 7];
        let mut pulses = [0u8; 13];
        for i in 0..13 {
            let t = w16(asl(xm[i], temp1));
            let t = mult(t, temp2);
            pulses[i] = (add(t >> 12, 4)).clamp(0, 7) as u8;
        }
        let coded = SubFrame { lag: nc as u8, gain: bc as u8, grid: mc as u8, xmax: xmaxc.clamp(0, 63) as u8, pulses };

        // Reconstruct, exactly as the decoder will, to keep the history in step.
        let ep = rpe_decode(&coded);
        self.synth.dp.copy_within(40.., 0);
        for k in 0..SUBFRAME_SAMPLES {
            self.synth.dp[80 + k] = add(ep[k], dpp[k]);
        }
        coded
    }
}

// --------------------------------------------------------------------------------- streams

#[cfg(feature = "gsm-decode")]
/// Decode a stream of whole blocks into 8 kHz samples. A [`Packing::Raw`] stream is checked for the
/// signature nibble of every frame.
pub fn decode(data: &[u8], packing: Packing) -> Result<Vec<i16>, Error> {
    let block = packing.block_bytes();
    if !data.len().is_multiple_of(block) {
        return Err(Error::Truncated { block, len: data.len() });
    }
    let mut dec = Decoder::new();
    let mut out = Vec::with_capacity(data.len() / block * packing.block_samples());
    for (n, chunk) in data.chunks_exact(block).enumerate() {
        match packing {
            Packing::Raw => {
                let mut b = [0u8; RAW_FRAME_BYTES];
                b.copy_from_slice(chunk);
                let frame = Frame::from_raw(&b).map_err(|_| Error::BadSignature { offset: n * block })?;
                out.extend_from_slice(&dec.decode_frame(&frame));
            }
            Packing::Wav49 => {
                let mut b = [0u8; WAV49_BLOCK_BYTES];
                b.copy_from_slice(chunk);
                for frame in Frame::pair_from_wav49(&b) {
                    out.extend_from_slice(&dec.decode_frame(&frame));
                }
            }
        }
    }
    Ok(out)
}

#[cfg(feature = "gsm-encode")]
/// Encode 8 kHz samples. The last block is padded with silence to a whole block, so the output
/// decodes to `samples.len()` rounded up to a block, with the first `samples.len()` as the signal.
#[must_use]
pub fn encode(samples: &[i16], packing: Packing) -> Vec<u8> {
    let per_block = packing.block_samples();
    let mut enc = Encoder::new();
    let mut out = Vec::with_capacity(samples.len().div_ceil(per_block) * packing.block_bytes());
    for chunk in samples.chunks(per_block) {
        let mut padded = [0i16; 2 * FRAME_SAMPLES];
        padded[..chunk.len()].copy_from_slice(chunk);
        let mut frame = |at: usize| {
            let mut input = [0i16; FRAME_SAMPLES];
            input.copy_from_slice(&padded[at..at + FRAME_SAMPLES]);
            enc.encode_frame(&input)
        };
        match packing {
            Packing::Raw => out.extend_from_slice(&frame(0).to_raw()),
            Packing::Wav49 => {
                let a = frame(0);
                let b = frame(FRAME_SAMPLES);
                out.extend_from_slice(&Frame::pair_to_wav49(&a, &b));
            }
        }
    }
    out
}

#[cfg(all(test, feature = "gsm-decode", feature = "gsm-encode"))]
mod tests {
    use super::*;

    fn xorshift(seed: &mut u32) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        *seed
    }

    fn speechlike(n: usize) -> Vec<i16> {
        let mut seed = 0x1234_5678u32;
        (0..n)
            .map(|i| {
                let t = i as f64 / 8000.0;
                let f0 = 120.0 + 30.0 * (t * 3.0).sin();
                let v = (t * f0 * std::f64::consts::TAU).sin() * 6000.0
                    + (t * 3.0 * f0 * std::f64::consts::TAU).sin() * 2500.0
                    + f64::from(xorshift(&mut seed) % 400)
                    - 200.0;
                (v * (0.6 + 0.4 * (t * 5.0).sin())) as i16
            })
            .collect()
    }

    #[test]
    fn arithmetic_matches_the_specification() {
        assert_eq!(add(32767, 1), 32767);
        assert_eq!(sub(-32768, 1), -32768);
        assert_eq!(mult(-32768, -32768), 32767);
        assert_eq!(mult_r(-32768, -32768), 32767);
        assert_eq!(mult_r(16384, 16384), 8192);
        assert_eq!(abs16(-32768), 32767);
        assert_eq!(div(5, 5), 32767);
        assert_eq!(div(1, 2), 16384);
        assert_eq!(norm(1), 30);
        assert_eq!(norm(0x4000_0000), 0);
        assert_eq!(norm(-1), 31);
        assert_eq!(norm(i32::MIN), 0);
        assert_eq!(asl(1, -1), 0);
        assert_eq!(asr(-8, 1), -4);
        assert_eq!(l_mult(-32768, -32768), i32::MAX);
    }

    #[test]
    fn raw_packing_round_trips_every_field() {
        let mut seed = 99u32;
        for _ in 0..200 {
            let mut v = [0u8; PARAMS];
            for (i, x) in v.iter_mut().enumerate() {
                *x = (xorshift(&mut seed) & ((1 << field_bits(i)) - 1)) as u8;
            }
            let frame = Frame::from_values(&v);
            let raw = frame.to_raw();
            assert_eq!(raw[0] >> 4, 0xD);
            assert_eq!(Frame::from_raw(&raw), Ok(frame));
            let pair = Frame::pair_to_wav49(&frame, &Frame::default());
            assert_eq!(Frame::pair_from_wav49(&pair), [frame, Frame::default()]);
            let pair = Frame::pair_to_wav49(&Frame::default(), &frame);
            assert_eq!(Frame::pair_from_wav49(&pair)[1], frame);
        }
        assert_eq!((0..PARAMS).map(field_bits).sum::<u32>(), 260);
    }

    #[test]
    fn a_raw_frame_needs_its_signature() {
        let mut raw = Frame::default().to_raw();
        raw[0] = 0x30;
        assert_eq!(Frame::from_raw(&raw), Err(Error::BadSignature { offset: 0 }));
        assert_eq!(decode(&raw, Packing::Raw), Err(Error::BadSignature { offset: 0 }));
    }

    #[test]
    fn partial_blocks_are_rejected() {
        assert_eq!(decode(&[0xD0; 32], Packing::Raw), Err(Error::Truncated { block: 33, len: 32 }));
        assert_eq!(decode(&[0; 66], Packing::Wav49), Err(Error::Truncated { block: 65, len: 66 }));
        assert_eq!(decode(&[], Packing::Raw), Ok(vec![]));
    }

    #[test]
    fn silence_stays_near_silence() {
        let pcm = vec![0i16; 160 * 4];
        for packing in [Packing::Raw, Packing::Wav49] {
            let bytes = encode(&pcm, packing);
            let out = decode(&bytes, packing).expect("decode");
            assert_eq!(out.len(), pcm.len());
            // The quantizers have no exact zero, so silence comes back as a faint residue.
            assert!(out.iter().all(|&s| s.abs() <= 32), "{packing:?}: {:?}", out.iter().map(|s| s.abs()).max());
        }
    }

    #[test]
    fn output_has_three_clear_low_bits() {
        let out = decode(&encode(&speechlike(1600), Packing::Raw), Packing::Raw).expect("decode");
        assert!(out.iter().all(|&s| s & 7 == 0));
    }

    #[test]
    fn speech_survives_the_round_trip() {
        let pcm = speechlike(8000);
        for packing in [Packing::Raw, Packing::Wav49] {
            let out = decode(&encode(&pcm, packing), packing).expect("decode");
            assert_eq!(out.len(), pcm.len());
            let (mut sig, mut err) = (0f64, 0f64);
            for i in 400..pcm.len() {
                let (a, b) = (f64::from(pcm[i]), f64::from(out[i]));
                sig += a * a;
                err += (a - b) * (a - b);
            }
            let snr = 10.0 * (sig / err).log10();
            assert!(snr > 8.0, "{packing:?}: SNR {snr:.1} dB");
        }
    }

    #[test]
    fn encoding_is_deterministic_and_packings_agree() {
        let pcm = speechlike(1600);
        assert_eq!(encode(&pcm, Packing::Raw), encode(&pcm, Packing::Raw));
        let raw = decode(&encode(&pcm, Packing::Raw), Packing::Raw).expect("raw");
        let wav = decode(&encode(&pcm, Packing::Wav49), Packing::Wav49).expect("wav49");
        assert_eq!(raw, wav);
    }

    #[test]
    fn short_input_is_padded_to_a_block() {
        assert_eq!(encode(&[], Packing::Raw), Vec::<u8>::new());
        assert_eq!(encode(&[100; 1], Packing::Raw).len(), 33);
        assert_eq!(encode(&[100; 161], Packing::Raw).len(), 66);
        assert_eq!(encode(&[100; 1], Packing::Wav49).len(), 65);
        assert_eq!(encode(&[100; 321], Packing::Wav49).len(), 130);
    }

    #[test]
    fn any_bits_decode_without_panicking() {
        let mut seed = 7u32;
        for _ in 0..300 {
            let mut v = [0u8; PARAMS];
            for x in &mut v {
                // Fields wider than their width and lags outside 40..=120 must be tolerated.
                *x = xorshift(&mut seed) as u8;
            }
            let frame = Frame::from_values(&v);
            let mut dec = Decoder::new();
            for _ in 0..3 {
                let _ = dec.decode_frame(&frame);
            }
        }
    }

    #[test]
    fn extreme_input_encodes_without_panicking() {
        for pattern in [i16::MAX, i16::MIN] {
            let pcm = vec![pattern; 1600];
            let out = decode(&encode(&pcm, Packing::Raw), Packing::Raw).expect("decode");
            assert_eq!(out.len(), 1600);
        }
        let alternating: Vec<i16> = (0..3200).map(|i| if i % 2 == 0 { i16::MAX } else { i16::MIN }).collect();
        let _ = encode(&alternating, Packing::Wav49);
        let mut seed = 5u32;
        let noise: Vec<i16> = (0..3200).map(|_| xorshift(&mut seed) as i16).collect();
        let _ = encode(&noise, Packing::Raw);
    }

    #[test]
    fn a_decoder_keeps_state_between_frames() {
        let bytes = encode(&speechlike(1600), Packing::Raw);
        let whole = decode(&bytes, Packing::Raw).expect("whole");
        let mut dec = Decoder::new();
        let mut piecewise = Vec::new();
        for b in bytes.as_chunks::<33>().0 {
            piecewise.extend_from_slice(&dec.decode_frame(&Frame::from_raw(b).expect("frame")));
        }
        assert_eq!(whole, piecewise);
    }
}
