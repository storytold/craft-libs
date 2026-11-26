//! ITU-T Recommendation G.722 wideband audio coding (64 kbit/s).
//!
//! Pure safe Rust clean-room implementation of the G.722 sub-band adaptive differential
//! pulse code modulation (SB-ADPCM) speech and audio codec.
//!
//! Input audio is 16 kHz 16-bit linear PCM. An analysis Quadrature Mirror Filter (QMF) splits
//! the 16 kHz audio into a lower sub-band (0–4 kHz) and a higher sub-band (4–8 kHz), each
//! sampled at 8 kHz. The lower sub-band is encoded at 48 kbit/s (6 bits/sample) and the higher
//! sub-band at 16 kbit/s (2 bits/sample), multiplexed into an 8-bit octet per 8 kHz cycle
//! (64 kbit/s total stream rate).
//!
//! On decoding, inverse adaptive quantizers reconstruct each sub-band signal, and a synthesis
//! QMF interpolates and combines them back to 16 kHz 16-bit linear PCM.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// Maximum allowed channels for multi-channel processing.
pub const MAX_CHANNELS: usize = 8;

/// Maximum input byte buffer bound to prevent memory exhaustion on hostile inputs (64 MiB).
pub const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;

/// Maximum input sample count bound (128 million samples).
pub const MAX_SAMPLES: usize = 128 * 1024 * 1024;

/// Error conditions encountered during G.722 encoding or decoding.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// The channel count is invalid.
    #[error("invalid channel count: {0} (must be 1..={MAX_CHANNELS})")]
    InvalidChannels(usize),

    /// The input size exceeds safety bounds.
    #[error("input exceeds maximum size bound")]
    InputTooLarge,

    /// Encoding requires an even number of samples per channel (16 kHz sample pairs).
    #[error("odd number of samples: {0} (G.722 encodes pairs of 16 kHz samples)")]
    OddSampleCount(usize),

    /// System allocation failed when reserving buffer memory.
    #[error("allocation failed")]
    AllocationFailed,
}

// ---------------------------------------------------------------------------
// Specification Tables from ITU-T Recommendation G.722
// ---------------------------------------------------------------------------

/// 24-tap QMF filter coefficients scaled by 2^13 (Table 11/G.722).
const H: [i16; 24] = [3, -11, -11, 53, 12, -156, 32, 362, -210, -805, 951, 3876, 3876, 951, -805, -210, 362, 32, -156, 12, 53, -11, -11, 3];

/// Lower sub-band 60-level inverse quantizer reconstruction table QQ6 (Table 14/G.722).
const QQ6: [i16; 31] = [
    0, 17, 54, 91, 130, 170, 211, 254, 300, 347, 396, 447, 501, 558, 618, 682, 750, 822, 899, 982, 1072, 1170, 1279, 1399, 1535, 1689, 1873, 2088,
    2376, 2738, 3101,
];

/// Lower sub-band quantizer decision levels Q6 (Table 14/G.722).
const Q6: [i16; 30] = [
    0, 35, 72, 110, 150, 190, 233, 276, 323, 370, 422, 473, 530, 587, 650, 714, 786, 858, 940, 1023, 1121, 1219, 1339, 1458, 1612, 1765, 1980, 2195,
    2557, 2919,
];

/// Lower sub-band 15-level inverse quantizer reconstruction table QQ4 (Table 14/G.722).
const QQ4: [i16; 8] = [0, 150, 323, 530, 786, 1121, 1612, 2557];

/// Lower sub-band logarithmic scale factor multipliers WL (Table 14/G.722).
const WL: [i16; 8] = [-60, -30, 58, 172, 334, 538, 1198, 3042];

/// Higher sub-band 4-level inverse quantizer reconstruction table QQ2 (Table 14/G.722).
const QQ2: [i16; 3] = [0, 202, 926];

/// Higher sub-band logarithmic scale factor multipliers WH (Table 14/G.722).
const WH: [i16; 3] = [0, -214, 798];

/// 32-entry inverse logarithmic scale table ILB (Table 15/G.722).
const ILB: [i32; 32] = [
    2048, 2093, 2139, 2186, 2233, 2282, 2332, 2383, 2435, 2489, 2543, 2599, 2656, 2714, 2774, 2834, 2896, 2960, 3025, 3091, 3158, 3228, 3298, 3371,
    3444, 3520, 3597, 3676, 3756, 3838, 3922, 4008,
];

/// Table 17/G.722: 4-bit codeword (RIL = IL >> 2) to (SIL, IL4).
const IL4_MAP: [(i16, usize); 16] =
    [(0, 0), (-1, 7), (-1, 6), (-1, 5), (-1, 4), (-1, 3), (-1, 2), (-1, 1), (0, 7), (0, 6), (0, 5), (0, 4), (0, 3), (0, 2), (0, 1), (0, 0)];

/// Table 21/G.722: 2-bit codeword IH to (SIH, IH2).
const IH2_MAP: [(i16, usize); 4] = [(-1, 2), (-1, 1), (0, 2), (0, 1)];

#[inline]
fn mult(a: i16, b: i16) -> i16 {
    (((a as i32) * (b as i32)) >> 15) as i16
}

#[inline]
fn scalel(nbpl: i32) -> i32 {
    let wd1 = ((nbpl >> 6) & 31) as usize;
    let wd2 = nbpl >> 11;
    let shift = 8 - wd2;
    let wd3 = if shift >= 0 { ILB[wd1] >> shift } else { ILB[wd1] << (-shift) };
    wd3 << 2
}

#[inline]
fn scaleh(nbph: i32) -> i32 {
    let wd1 = ((nbph >> 6) & 31) as usize;
    let wd2 = nbph >> 11;
    let shift = 10 - wd2;
    let wd3 = if shift >= 0 { ILB[wd1] >> shift } else { ILB[wd1] << (-shift) };
    wd3 << 2
}

// ---------------------------------------------------------------------------
// Sub-band ADPCM State (Pole/Zero Predictors & Adaptations)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct BandState {
    s: i16,
    sp: i16,
    sz: i16,
    d: [i16; 7],
    r: [i16; 3],
    p: [i16; 3],
    a: [i16; 3],
    b: [i16; 7],
    nb: i32,
    det: i32,
}

impl BandState {
    fn new(is_lower: bool) -> Self {
        Self { s: 0, sp: 0, sz: 0, d: [0; 7], r: [0; 3], p: [0; 3], a: [0; 3], b: [0; 7], nb: 0, det: if is_lower { 32 } else { 8 } }
    }

    fn reset(&mut self, is_lower: bool) {
        self.s = 0;
        self.sp = 0;
        self.sz = 0;
        self.d = [0; 7];
        self.r = [0; 3];
        self.p = [0; 3];
        self.a = [0; 3];
        self.b = [0; 7];
        self.nb = 0;
        self.det = if is_lower { 32 } else { 8 };
    }

    fn update_predictor(&mut self, d: i16, p_sig: i16, r_sig: i16) {
        // UPZERO: update 6th order zero predictor coefficients
        let wd1: i16 = if d != 0 { 128 } else { 0 };
        let sg0: i16 = if d < 0 { -1 } else { 0 };
        for i in 1..=6 {
            let sgi = if self.d[i] < 0 { -1 } else { 0 };
            let wd2 = if sg0 == sgi { wd1 } else { wd1.saturating_neg() };
            let wd3 = mult(self.b[i], 32640);
            self.b[i] = wd2.saturating_add(wd3);
        }

        // UPPOL2: update 2nd pole coefficient
        let sg0 = if p_sig < 0 { -1 } else { 0 };
        let sg1 = if self.p[1] < 0 { -1 } else { 0 };
        let sg2 = if self.p[2] < 0 { -1 } else { 0 };
        let wd1 = self.a[1].saturating_add(self.a[1]).saturating_add(self.a[1].saturating_add(self.a[1]));
        let wd2 = if sg0 == sg1 { wd1.saturating_neg() } else { wd1 } >> 7;
        let wd3: i16 = if sg0 == sg2 { 128 } else { -128 };
        let wd4 = wd2.saturating_add(wd3);
        let wd5 = mult(self.a[2], 32512);
        let apl2 = wd4.saturating_add(wd5).clamp(-12288, 12288);

        // UPPOL1: update 1st pole coefficient
        let wd1: i16 = if sg0 == sg1 { 192 } else { -192 };
        let wd2 = mult(self.a[1], 32640);
        let apl1 = wd1.saturating_add(wd2);
        let wd3 = 15360 - apl2;
        let apl1 = apl1.clamp(-wd3, wd3);

        self.a[1] = apl1;
        self.a[2] = apl2;

        // Shift history delay lines
        for i in (2..=6).rev() {
            self.d[i] = self.d[i - 1];
        }
        self.d[1] = d;

        self.p[2] = self.p[1];
        self.p[1] = p_sig;

        self.r[2] = self.r[1];
        self.r[1] = r_sig;

        // FILTEZ: compute zero section output
        let mut sz: i16 = 0;
        for i in (1..=6).rev() {
            let part = mult(self.b[i], self.d[i].saturating_add(self.d[i]));
            sz = sz.saturating_add(part);
        }
        self.sz = sz;

        // FILTEP: compute pole section output
        let sp1 = mult(self.a[1], self.r[1].saturating_add(self.r[1]));
        let sp2 = mult(self.a[2], self.r[2].saturating_add(self.r[2]));
        self.sp = sp1.saturating_add(sp2);

        // PREDIC: compute full predictor output value
        self.s = self.sp.saturating_add(self.sz);
    }
}

// ---------------------------------------------------------------------------
// Quadrature Mirror Filters (QMF)
// ---------------------------------------------------------------------------

/// Analysis QMF (transmit filter bank): splits pairs of 16 kHz samples into lower and higher sub-bands.
#[derive(Debug, Clone, Default)]
pub struct QmfTx {
    x: [i16; 24],
}

impl QmfTx {
    /// Create a new analysis QMF filter.
    pub fn new() -> Self {
        Self { x: [0; 24] }
    }

    /// Reset filter history.
    pub fn reset(&mut self) {
        self.x = [0; 24];
    }

    /// Process a pair of 16 kHz input samples `(s0, s1)` into `(xl, xh)`.
    pub fn filter(&mut self, s0: i16, s1: i16) -> (i16, i16) {
        for i in (2..24).rev() {
            self.x[i] = self.x[i - 2];
        }
        self.x[0] = s1 >> 1;
        self.x[1] = s0 >> 1;

        let mut xa: i32 = 0;
        let mut xb: i32 = 0;
        for i in 0..12 {
            xa = xa.saturating_add((self.x[2 * i] as i32) * (H[2 * i] as i32));
            xb = xb.saturating_add((self.x[2 * i + 1] as i32) * (H[2 * i + 1] as i32));
        }

        let xl = ((xa.saturating_add(xb)) >> 13).clamp(-16384, 16383) as i16;
        let xh = ((xa.saturating_sub(xb)) >> 13).clamp(-16384, 16383) as i16;
        (xl, xh)
    }
}

/// Synthesis QMF (receive filter bank): synthesizes lower and higher sub-band signals into 16 kHz output.
#[derive(Debug, Clone, Default)]
pub struct QmfRx {
    xd: [i16; 12],
    xs: [i16; 12],
}

impl QmfRx {
    /// Create a new synthesis QMF filter.
    pub fn new() -> Self {
        Self { xd: [0; 12], xs: [0; 12] }
    }

    /// Reset filter history.
    pub fn reset(&mut self) {
        self.xd = [0; 12];
        self.xs = [0; 12];
    }

    /// Reconstruct a pair of 16 kHz output samples from sub-band samples `(rl, rh)`.
    pub fn filter(&mut self, rl: i16, rh: i16) -> (i16, i16) {
        let xd_curr = rl.saturating_sub(rh);
        let xs_curr = rl.saturating_add(rh);

        for i in (1..12).rev() {
            self.xd[i] = self.xd[i - 1];
            self.xs[i] = self.xs[i - 1];
        }
        self.xd[0] = xd_curr;
        self.xs[0] = xs_curr;

        let mut wd_c: i32 = 0;
        let mut wd_d: i32 = 0;
        for i in 0..12 {
            wd_c = wd_c.saturating_add((self.xd[i] as i32) * (H[2 * i] as i32));
            wd_d = wd_d.saturating_add((self.xs[i] as i32) * (H[2 * i + 1] as i32));
        }

        let xout1 = (wd_c >> 12).clamp(-16384, 16383);
        let xout2 = (wd_d >> 12).clamp(-16384, 16383);

        let s0 = (xout1 << 1).clamp(-32768, 32767) as i16;
        let s1 = (xout2 << 1).clamp(-32768, 32767) as i16;
        (s0, s1)
    }
}

// ---------------------------------------------------------------------------
// G.722 Decoder & Encoder
// ---------------------------------------------------------------------------

/// ITU-T G.722 64 kbit/s single-channel decoder.
#[derive(Debug, Clone)]
pub struct Decoder {
    band_l: BandState,
    band_h: BandState,
    qmf: QmfRx,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// Create a new G.722 decoder initialized to standard reset state.
    pub fn new() -> Self {
        Self { band_l: BandState::new(true), band_h: BandState::new(false), qmf: QmfRx::new() }
    }

    /// Reset decoder internal state.
    pub fn reset(&mut self) {
        self.band_l.reset(true);
        self.band_h.reset(false);
        self.qmf.reset();
    }

    /// Decode a single 8-bit octet into two 16 kHz 16-bit PCM samples.
    pub fn decode_octet(&mut self, octet: u8) -> (i16, i16) {
        let ih = ((octet >> 6) & 3) as usize;
        let il = (octet & 0x3F) as usize;

        // INVQBL: 6-bit lower sub-band inverse quantization
        let (sil6, il6) = table18_lookup(il);
        let wd1 = (QQ6[il6] as i32) << 3;
        let wd2 = if sil6 == 0 { wd1 } else { -wd1 };
        let dl = mult(self.band_l.det as i16, wd2 as i16);
        let yl = self.band_l.s.saturating_add(dl);
        let rl = yl.clamp(-16384, 16383);

        // INVQAL: lower sub-band predictor update (4-bit feedback path)
        let ril = il >> 2;
        let (sil4, il4) = IL4_MAP[ril & 0x0F];
        let wd1 = (QQ4[il4] as i32) << 3;
        let wd2 = if sil4 == 0 { wd1 } else { -wd1 };
        let dlt = mult(self.band_l.det as i16, wd2 as i16);
        let plt = dlt.saturating_add(self.band_l.sz);
        let rlt = self.band_l.s.saturating_add(dlt);

        // Lower sub-band scale factor adaptation (LOGSCL)
        let wd_scale = mult(self.band_l.nb as i16, 32512);
        let nbpl = (wd_scale as i32 + WL[il4] as i32).clamp(0, 18432);
        self.band_l.nb = nbpl;
        self.band_l.det = scalel(nbpl);
        self.band_l.update_predictor(dlt, plt, rlt);

        // INVQAH: higher sub-band inverse quantization
        let (sih_dec, ih2) = IH2_MAP[ih & 0x03];
        let wd1 = (QQ2[ih2] as i32) << 3;
        let wd2 = if sih_dec == 0 { wd1 } else { -wd1 };
        let dh = mult(self.band_h.det as i16, wd2 as i16);
        let yh = self.band_h.s.saturating_add(dh);
        let rh = yh.clamp(-16384, 16383);

        // Higher sub-band scale factor adaptation (LOGSCH)
        let wd_scale = mult(self.band_h.nb as i16, 32512);
        let nbph = (wd_scale as i32 + WH[ih2] as i32).clamp(0, 22528);
        self.band_h.nb = nbph;
        self.band_h.det = scaleh(nbph);
        let ph = dh.saturating_add(self.band_h.sz);
        self.band_h.update_predictor(dh, ph, yh);

        // Synthesis QMF
        self.qmf.filter(rl, rh)
    }

    /// Decode G.722 octets into 16 kHz 16-bit linear PCM samples, appending to `dst`.
    pub fn decode(&mut self, src: &[u8], dst: &mut Vec<i16>) -> Result<(), Error> {
        if src.len() > MAX_INPUT_BYTES {
            return Err(Error::InputTooLarge);
        }
        let needed_samples = src.len().saturating_mul(2);
        if needed_samples > MAX_SAMPLES {
            return Err(Error::InputTooLarge);
        }
        dst.try_reserve(needed_samples).map_err(|_| Error::AllocationFailed)?;
        for &octet in src {
            let (s0, s1) = self.decode_octet(octet);
            dst.push(s0);
            dst.push(s1);
        }
        Ok(())
    }
}

/// ITU-T G.722 64 kbit/s single-channel encoder.
#[derive(Debug, Clone)]
pub struct Encoder {
    qmf: QmfTx,
    band_l: BandState,
    band_h: BandState,
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder {
    /// Create a new G.722 encoder initialized to standard reset state.
    pub fn new() -> Self {
        Self { qmf: QmfTx::new(), band_l: BandState::new(true), band_h: BandState::new(false) }
    }

    /// Reset encoder internal state.
    pub fn reset(&mut self) {
        self.qmf.reset();
        self.band_l.reset(true);
        self.band_h.reset(false);
    }

    /// Encode a pair of 16 kHz 16-bit PCM samples into one 8-bit G.722 octet.
    pub fn encode_sample_pair(&mut self, s0: i16, s1: i16) -> u8 {
        // Analysis QMF
        let (xl, xh) = self.qmf.filter(s0, s1);

        // Lower sub-band quantizer (QUANTL)
        let el = xl.saturating_sub(self.band_l.s);
        let sil = if el < 0 { -1 } else { 0 };
        let wd = if sil == 0 { el } else { 32767 - (el & 32767) };
        let mut mil: usize = 30;
        for (k, &thresh_q) in Q6.iter().enumerate().skip(1) {
            let thresh = (((thresh_q as i32) << 3) * self.band_l.det) >> 15;
            if (wd as i32) < thresh {
                mil = k;
                break;
            }
        }

        // Table 16 conversion to IL
        let il: u8 = if sil == 0 {
            (62 - mil) as u8
        } else if mil == 1 {
            63
        } else if mil == 2 {
            62
        } else {
            (34 - mil) as u8
        };

        // Lower sub-band feedback and predictor update (INVQAL)
        let ril = (il >> 2) as usize;
        let (sil4, il4) = IL4_MAP[ril & 0x0F];
        let wd1 = (QQ4[il4] as i32) << 3;
        let wd2 = if sil4 == 0 { wd1 } else { -wd1 };
        let dlt = mult(self.band_l.det as i16, wd2 as i16);
        let plt = dlt.saturating_add(self.band_l.sz);
        let rlt = self.band_l.s.saturating_add(dlt);

        // Lower sub-band scale factor adaptation (LOGSCL)
        let wd_scale = mult(self.band_l.nb as i16, 32512);
        let nbpl = (wd_scale as i32 + WL[il4] as i32).clamp(0, 18432);
        self.band_l.nb = nbpl;
        self.band_l.det = scalel(nbpl);
        self.band_l.update_predictor(dlt, plt, rlt);

        // Higher sub-band quantizer (QUANTH)
        let eh = xh.saturating_sub(self.band_h.s);
        let sih = if eh < 0 { -1 } else { 0 };
        let wd = if sih == 0 { eh } else { 32767 - (eh & 32767) };
        let thresh = ((564 << 3) * self.band_h.det) >> 15;
        let mih = if (wd as i32) < thresh { 1 } else { 2 };

        // Table 20 conversion to IH
        let ih: u8 = if sih == -1 {
            if mih == 2 { 0 } else { 1 }
        } else if mih == 1 {
            3
        } else {
            2
        };

        // Higher sub-band feedback and predictor update (INVQAH)
        let (sih_dec, ih2) = IH2_MAP[ih as usize];
        let wd1 = (QQ2[ih2] as i32) << 3;
        let wd2 = if sih_dec == 0 { wd1 } else { -wd1 };
        let dh = mult(self.band_h.det as i16, wd2 as i16);
        let yh = self.band_h.s.saturating_add(dh);

        // Higher sub-band scale factor adaptation (LOGSCH)
        let wd_scale = mult(self.band_h.nb as i16, 32512);
        let nbph = (wd_scale as i32 + WH[ih2] as i32).clamp(0, 22528);
        self.band_h.nb = nbph;
        self.band_h.det = scaleh(nbph);
        let ph = dh.saturating_add(self.band_h.sz);
        self.band_h.update_predictor(dh, ph, yh);

        // Multiplexer (MUX)
        (ih << 6) | (il & 0x3F)
    }

    /// Encode 16 kHz 16-bit linear PCM samples into G.722 octets, appending to `dst`.
    pub fn encode(&mut self, src: &[i16], dst: &mut Vec<u8>) -> Result<(), Error> {
        if !src.len().is_multiple_of(2) {
            return Err(Error::OddSampleCount(src.len()));
        }
        if src.len() > MAX_SAMPLES {
            return Err(Error::InputTooLarge);
        }
        let needed_octets = src.len() / 2;
        dst.try_reserve(needed_octets).map_err(|_| Error::AllocationFailed)?;
        for &[s0, s1] in src.as_chunks::<2>().0 {
            let octet = self.encode_sample_pair(s0, s1);
            dst.push(octet);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn table18_lookup(il: usize) -> (i16, usize) {
    let il = il & 0x3F;
    if (4..=31).contains(&il) {
        (-1, 34 - il)
    } else if il == 62 {
        (-1, 2)
    } else if il == 63 {
        (-1, 1)
    } else if (32..=61).contains(&il) {
        (0, 62 - il)
    } else {
        // Transmission error fallbacks (0..=3)
        (-1, 1)
    }
}

// ---------------------------------------------------------------------------
// Public Top-Level Functions
// ---------------------------------------------------------------------------

/// Decode a mono 64 kbit/s G.722 stream into 16 kHz 16-bit linear PCM samples.
pub fn decode(bytes: &[u8]) -> Result<Vec<i16>, Error> {
    let mut decoder = Decoder::new();
    let mut out = Vec::new();
    decoder.decode(bytes, &mut out)?;
    Ok(out)
}

/// Encode mono 16 kHz 16-bit linear PCM samples into a 64 kbit/s G.722 stream.
pub fn encode(samples: &[i16]) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder::new();
    let mut out = Vec::new();
    encoder.encode(samples, &mut out)?;
    Ok(out)
}

/// Decode an interleaved multi-channel G.722 stream into interleaved 16 kHz 16-bit PCM samples.
pub fn decode_interleaved(bytes: &[u8], channels: usize) -> Result<Vec<i16>, Error> {
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(Error::InvalidChannels(channels));
    }
    if channels == 1 {
        return decode(bytes);
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(Error::InputTooLarge);
    }
    let frames = bytes.len() / channels;
    let mut decoders: Vec<Decoder> = Vec::with_capacity(channels);
    for _ in 0..channels {
        decoders.push(Decoder::new());
    }
    let total_samples = frames.saturating_mul(channels).saturating_mul(2);
    let mut out = Vec::new();
    out.try_reserve(total_samples).map_err(|_| Error::AllocationFailed)?;

    // Each frame has `channels` bytes. Decode each byte with its channel's decoder,
    // producing sample 0 for each channel, then sample 1 for each channel.
    for chunk in bytes.chunks_exact(channels) {
        let mut pairs = [(0i16, 0i16); MAX_CHANNELS];
        for (ch, &b) in chunk.iter().enumerate() {
            pairs[ch] = decoders[ch].decode_octet(b);
        }
        for pair in pairs.iter().take(channels) {
            out.push(pair.0);
        }
        for pair in pairs.iter().take(channels) {
            out.push(pair.1);
        }
    }
    Ok(out)
}

/// Encode interleaved 16 kHz 16-bit PCM samples into an interleaved multi-channel G.722 stream.
pub fn encode_interleaved(samples: &[i16], channels: usize) -> Result<Vec<u8>, Error> {
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(Error::InvalidChannels(channels));
    }
    if channels == 1 {
        return encode(samples);
    }
    if samples.len() > MAX_SAMPLES {
        return Err(Error::InputTooLarge);
    }
    let frame_len = channels.saturating_mul(2);
    if !samples.len().is_multiple_of(frame_len) {
        return Err(Error::OddSampleCount(samples.len()));
    }
    let frames = samples.len() / frame_len;
    let mut encoders: Vec<Encoder> = Vec::with_capacity(channels);
    for _ in 0..channels {
        encoders.push(Encoder::new());
    }
    let total_bytes = frames.saturating_mul(channels);
    let mut out = Vec::new();
    out.try_reserve(total_bytes).map_err(|_| Error::AllocationFailed)?;

    for chunk in samples.chunks_exact(frame_len) {
        for ch in 0..channels {
            let s0 = chunk[ch];
            let s1 = chunk[ch + channels];
            out.push(encoders[ch].encode_sample_pair(s0, s1));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_invalid_channels() {
        assert_eq!(decode_interleaved(&[0; 4], 0), Err(Error::InvalidChannels(0)));
        assert_eq!(decode_interleaved(&[0; 4], 9), Err(Error::InvalidChannels(9)));
        assert_eq!(encode_interleaved(&[0; 4], 0), Err(Error::InvalidChannels(0)));
        assert_eq!(encode_interleaved(&[0; 4], 9), Err(Error::InvalidChannels(9)));
    }

    #[test]
    fn reject_odd_sample_count() {
        assert_eq!(encode(&[0; 3]), Err(Error::OddSampleCount(3)));
        assert_eq!(encode_interleaved(&[0; 5], 2), Err(Error::OddSampleCount(5)));
    }

    #[test]
    fn roundtrip_silence() {
        let input = vec![0i16; 320];
        let encoded = encode(&input).expect("encode silence");
        assert_eq!(encoded.len(), 160);
        let decoded = decode(&encoded).expect("decode silence");
        assert_eq!(decoded.len(), 320);
        for &s in &decoded[40..] {
            assert!(s.abs() <= 6, "silence sample too large: {s}");
        }
    }

    #[test]
    fn multichannel_stereo_roundtrip() {
        let mut stereo_pcm = Vec::new();
        for i in 0..400 {
            stereo_pcm.push((i * 10) as i16);
            stereo_pcm.push((-i * 10) as i16);
        }
        let enc = encode_interleaved(&stereo_pcm, 2).expect("encode stereo");
        assert_eq!(enc.len(), 400);
        let dec = decode_interleaved(&enc, 2).expect("decode stereo");
        assert_eq!(dec.len(), 800);
    }

    #[test]
    fn hostile_input_does_not_panic() {
        let garbage = [0xFF, 0x00, 0x55, 0xAA, 0x12, 0x34, 0x7F, 0x80];
        let dec = decode(&garbage).expect("decode garbage");
        assert_eq!(dec.len(), 16);
    }

    #[test]
    fn uppol2_overflow_does_not_panic() {
        let mut state = BandState::new(true);
        state.a[1] = -8192;
        state.p = [0, 0, 0];
        state.update_predictor(0, 0, 0);
    }
}
