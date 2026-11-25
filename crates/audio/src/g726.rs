//! ITU-T Recommendation G.726 ADPCM speech codec (16, 24, 32, 40 kbit/s).
//!
//! Clean-room implementation based on ITU-T Recommendation G.726 (12/1990)
//! "40, 32, 24, 16 kbit/s Adaptive Differential Pulse Code Modulation (ADPCM)".
//!
//! Provides encoding from 8 kHz 16-bit linear PCM to G.726 bitstreams, and decoding
//! from G.726 bitstreams to 8 kHz 16-bit linear PCM.
//!
//! Supports both standard big-endian bit packing (network order / RFC 3551) and
//! little-endian bit packing (RIFF WAV / AVID format tag 0x0045 / 0x0014).

const MAX_CHANNELS: usize = 8;
const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_SAMPLES: usize = 64 * 1024 * 1024;

/// Bit rate for G.726 ADPCM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitRate {
    /// 16 kbit/s (2 bits per sample).
    Rate16k,
    /// 24 kbit/s (3 bits per sample).
    Rate24k,
    /// 32 kbit/s (4 bits per sample).
    Rate32k,
    /// 40 kbit/s (5 bits per sample).
    Rate40k,
}

impl BitRate {
    /// Number of bits per code (sample).
    #[inline]
    #[must_use]
    pub const fn code_size(self) -> usize {
        match self {
            Self::Rate16k => 2,
            Self::Rate24k => 3,
            Self::Rate32k => 4,
            Self::Rate40k => 5,
        }
    }

    /// Bit rate in bits per second.
    #[inline]
    #[must_use]
    pub const fn bit_rate_bps(self) -> u32 {
        match self {
            Self::Rate16k => 16_000,
            Self::Rate24k => 24_000,
            Self::Rate32k => 32_000,
            Self::Rate40k => 40_000,
        }
    }
}

/// Bit packing order within bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BitPacking {
    /// Big-endian packing: MSB to LSB (standard RFC 3551 / network order).
    #[default]
    BigEndian,
    /// Little-endian packing: LSB to MSB (WAV / RIFF order).
    LittleEndian,
}

/// Errors returned by G.726 operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Channel count must be between 1 and 8.
    #[error("unsupported channel count: {0} (must be 1..=8)")]
    InvalidChannels(usize),
    /// Input exceeds safe maximum memory bound.
    #[error("input exceeds safe maximum processing limit")]
    InputTooLarge,
    /// Output buffer allocation failed.
    #[error("memory allocation failed")]
    AllocationFailed,
}

// ---------------------------------------------------------------------------
// G.726 Tables
// ---------------------------------------------------------------------------

// Table 11..14 (I -> DQS, DQLN)
static OUT_16: [(u8, u16); 4] = [(0, 116), (0, 365), (1, 365), (1, 116)];

static OUT_24: [(u8, u16); 8] = [(0, 2048), (0, 135), (0, 273), (0, 373), (1, 373), (1, 273), (1, 135), (1, 2048)];

static OUT_32: [(u8, u16); 16] = [
    (0, 2048),
    (0, 4),
    (0, 135),
    (0, 213),
    (0, 273),
    (0, 323),
    (0, 373),
    (0, 425),
    (1, 425),
    (1, 373),
    (1, 323),
    (1, 273),
    (1, 213),
    (1, 135),
    (1, 4),
    (1, 2048),
];

static OUT_40: [(u8, u16); 32] = [
    (0, 2048),
    (0, 4030),
    (0, 28),
    (0, 104),
    (0, 169),
    (0, 224),
    (0, 274),
    (0, 318),
    (0, 358),
    (0, 395),
    (0, 429),
    (0, 459),
    (0, 488),
    (0, 514),
    (0, 539),
    (0, 566),
    (1, 566),
    (1, 539),
    (1, 514),
    (1, 488),
    (1, 459),
    (1, 429),
    (1, 395),
    (1, 358),
    (1, 318),
    (1, 274),
    (1, 224),
    (1, 169),
    (1, 104),
    (1, 28),
    (1, 4030),
    (1, 2048),
];

// FUNCTW: scale factor multipliers WI
static WI_16: [u16; 2] = [4074, 439];
static WI_24: [u16; 4] = [4092, 30, 137, 582];
static WI_32: [u16; 8] = [4084, 18, 41, 64, 112, 198, 355, 1122];
static WI_40: [u16; 16] = [14, 14, 24, 39, 40, 41, 58, 100, 141, 179, 219, 280, 358, 440, 529, 696];

#[inline]
fn get_wi(rate: BitRate, code: u8) -> u16 {
    match rate {
        BitRate::Rate16k => {
            let im = if (code >> 1) == 0 { (code & 1) as usize } else { ((3 - code) & 1) as usize };
            WI_16[im]
        }
        BitRate::Rate24k => {
            let im = if (code >> 2) == 0 { (code & 3) as usize } else { ((7 - code) & 3) as usize };
            WI_24[im]
        }
        BitRate::Rate32k => {
            let im = if (code >> 3) == 0 { (code & 7) as usize } else { ((15 - code) & 7) as usize };
            WI_32[im]
        }
        BitRate::Rate40k => {
            let im = if (code >> 4) == 0 { (code & 15) as usize } else { ((31 - code) & 15) as usize };
            WI_40[im]
        }
    }
}

#[inline]
fn get_fi(rate: BitRate, code: u8) -> u8 {
    match rate {
        BitRate::Rate16k => {
            let im = if (code >> 1) == 0 { code & 1 } else { (3 - code) & 1 };
            if im == 1 { 7 } else { 0 }
        }
        BitRate::Rate24k => {
            let im = if (code >> 2) == 0 { code & 3 } else { (7 - code) & 3 };
            match im {
                0 => 0,
                1 => 1,
                2 => 2,
                _ => 7,
            }
        }
        BitRate::Rate32k => {
            let im = if (code >> 3) == 0 { code & 7 } else { (15 - code) & 7 };
            if im <= 2 {
                0
            } else if im <= 5 {
                1
            } else if im == 6 {
                3
            } else {
                7
            }
        }
        BitRate::Rate40k => {
            let im = if (code >> 4) == 0 { code & 15 } else { (31 - code) & 15 };
            if im <= 4 {
                0
            } else if im <= 9 {
                1
            } else {
                match im {
                    10 => 2,
                    11 => 3,
                    12 => 4,
                    13 => 5,
                    _ => 6,
                }
            }
        }
    }
}

// Quantizer decision intervals (lo, hi, positive_code, negative_code)
static THRESH_16: [(i32, i32, u8, u8); 1] = [(261, 2047, 1, 2)];

static THRESH_24: [(i32, i32, u8, u8); 3] = [(331, 2047, 3, 4), (218, 330, 2, 5), (8, 217, 1, 6)];

static THRESH_32: [(i32, i32, u8, u8); 8] = [
    (400, 2047, 7, 8),
    (349, 399, 6, 9),
    (300, 348, 5, 10),
    (246, 299, 4, 11),
    (178, 245, 3, 12),
    (80, 177, 2, 13),
    (0, 79, 1, 14),
    (3972, 4095, 1, 14),
];

static THRESH_40: [(i32, i32, u8, u8); 16] = [
    (553, 2047, 15, 16),
    (528, 552, 14, 17),
    (502, 527, 13, 18),
    (475, 501, 12, 19),
    (445, 474, 11, 20),
    (413, 444, 10, 21),
    (378, 412, 9, 22),
    (339, 377, 8, 23),
    (298, 338, 7, 24),
    (250, 297, 6, 25),
    (198, 249, 5, 26),
    (139, 197, 4, 27),
    (68, 138, 3, 28),
    (0, 67, 2, 29),
    (4080, 4095, 2, 29),
    (3974, 4079, 1, 30),
];

#[inline]
fn quantize_16(dln: i32, ds: u8) -> u8 {
    for &(lo, hi, pos, neg) in &THRESH_16 {
        if (lo..=hi).contains(&dln) {
            return if ds == 0 { pos } else { neg };
        }
    }
    if ds == 0 { 0 } else { 3 }
}

#[inline]
fn quantize_24(dln: i32, ds: u8) -> u8 {
    for &(lo, hi, pos, neg) in &THRESH_24 {
        if (lo..=hi).contains(&dln) {
            return if ds == 0 { pos } else { neg };
        }
    }
    7
}

#[inline]
fn quantize_32(dln: i32, ds: u8) -> u8 {
    for &(lo, hi, pos, neg) in &THRESH_32 {
        if (lo..=hi).contains(&dln) {
            return if ds == 0 { pos } else { neg };
        }
    }
    15
}

#[inline]
fn quantize_40(dln: i32, ds: u8) -> u8 {
    for &(lo, hi, pos, neg) in &THRESH_40 {
        if (lo..=hi).contains(&dln) {
            return if ds == 0 { pos } else { neg };
        }
    }
    31
}

// ---------------------------------------------------------------------------
// G.726 Core State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Core {
    rate: BitRate,
    is_40k: bool,
    a: [u16; 2],
    b: [u16; 6],
    dms: i16,
    dml: i16,
    ap: i16,
    yu: i16,
    yl: i32,
    sr: [u16; 2],
    dq: [u16; 6],
    pk: [u8; 2],
    td: i16,
}

impl Core {
    fn new(rate: BitRate) -> Self {
        let is_40k = matches!(rate, BitRate::Rate40k);
        let mut core =
            Self { rate, is_40k, a: [0; 2], b: [0; 6], dms: 0, dml: 0, ap: 0, yu: 544, yl: 34816, sr: [32, 32], dq: [32; 6], pk: [0; 2], td: 0 };
        core.reset();
        core
    }

    fn reset(&mut self) {
        self.a = [0; 2];
        self.b = [0; 6];
        self.dms = 0;
        self.dml = 0;
        self.ap = 0;
        self.yu = 544;
        self.yl = 34816;
        self.sr = [32, 32];
        self.dq = [32; 6];
        self.pk = [0; 2];
        self.td = 0;
    }

    #[inline]
    fn fmult(an: u16, srn: u16) -> u16 {
        let an_s = (an >> 15) & 1;
        let an_mag = if an_s == 0 { (an >> 2) & 0x1FFF } else { (16384_u32.wrapping_sub((an >> 2) as u32) & 0x1FFF) as u16 };
        let an_exp = if an_mag > 0 { (16 - an_mag.leading_zeros()).min(13) as u16 } else { 0 };
        let an_mant = if an_mag == 0 { 1 << 5 } else { (((an_mag as u32) << 6) >> an_exp) as u16 & 0x3F };

        let srn_s = (srn >> 10) & 1;
        let srn_exp = (srn >> 6) & 15;
        let srn_mant = srn & 63;

        let wan_s = srn_s ^ an_s;
        let wan_exp = srn_exp + an_exp;
        let wan_mant = (((srn_mant as u32 * an_mant as u32) + 48) >> 4) as u16;

        let wan_mag =
            if wan_exp <= 26 { ((wan_mant as u32) << 7) >> (26 - wan_exp) } else { (((wan_mant as u32) << 7) << (wan_exp - 26)) & 0x7FFF } as u16;

        if wan_s == 0 { wan_mag } else { (65536_u32.wrapping_sub(wan_mag as u32) & 0xFFFF) as u16 }
    }

    #[inline]
    fn quantize(&self, dln: i32, ds: u8) -> u8 {
        match self.rate {
            BitRate::Rate16k => quantize_16(dln, ds),
            BitRate::Rate24k => quantize_24(dln, ds),
            BitRate::Rate32k => quantize_32(dln, ds),
            BitRate::Rate40k => quantize_40(dln, ds),
        }
    }

    #[inline]
    fn table_lookup(&self, code: u8) -> (u8, u16) {
        match self.rate {
            BitRate::Rate16k => OUT_16[(code & 3) as usize],
            BitRate::Rate24k => OUT_24[(code & 7) as usize],
            BitRate::Rate32k => OUT_32[(code & 15) as usize],
            BitRate::Rate40k => OUT_40[(code & 31) as usize],
        }
    }

    #[inline]
    fn compute_accum(&self) -> (i16, i16) {
        let wa0 = Self::fmult(self.a[0], self.sr[0]);
        let wa1 = Self::fmult(self.a[1], self.sr[1]);
        let mut sezi: u32 = 0;
        for i in 0..6 {
            sezi = (sezi + Self::fmult(self.b[i], self.dq[i]) as u32) & 0xFFFF;
        }
        let sei = (sezi + wa1 as u32 + wa0 as u32) & 0xFFFF;
        let sez = (sezi as i16) >> 1;
        let se = (sei as i16) >> 1;
        (sez, se)
    }

    #[inline]
    fn compute_mix(&self) -> i32 {
        let al = if self.ap >= 256 { 64 } else { self.ap >> 2 } as i32;
        let dif = (self.yu as i32 + 16384 - (self.yl >> 6)) & 16383;
        let difs = (dif >> 13) & 1;
        let difm = if difs == 0 { dif } else { (16384 - dif) & 8191 };
        let prodm = (difm * al) >> 6;
        let prod = if difs == 0 { prodm } else { (16384 - prodm) & 8191 };
        ((self.yl >> 6) + prod) & 8191
    }

    #[inline]
    fn quantize_sample(&self, sample: i16, se: i16, y: i32) -> u8 {
        let sl = (sample >> 2) as i32;
        let sl_ext = if sl >= 0 { sl } else { 65536 + sl };
        let se_ext = if se >= 0 { se as i32 } else { 65536 + se as i32 };
        let d = ((sl_ext + 65536 - se_ext) & 0xFFFF) as u16;

        let ds = ((d >> 15) & 1) as u8;
        let dqm = if ds == 0 { d } else { (65536_u32.wrapping_sub(d as u32) & 0x7FFF) as u16 };
        let exp = if dqm > 1 { (15 - dqm.leading_zeros()).min(14) } else { 0 };
        let mant = (((dqm as u32) << 7) >> exp) & 127;
        let dl = ((exp << 7) + mant) as u16;
        let dln = (dl as i32 + 4096 - (y >> 2)) & 4095;
        self.quantize(dln, ds)
    }

    #[inline]
    fn step(&mut self, sample: i16, is_encoder: bool, code_in: u8) -> (u8, i16) {
        let (sez, se) = self.compute_accum();
        let y = self.compute_mix();

        let code = if is_encoder { self.quantize_sample(sample, se, y) } else { code_in };

        // 4. RECONST & ADDA & ANTILOG
        let (dqs, dqln) = self.table_lookup(code);
        let dql = (dqln as i32 + (y >> 2)) & 4095;
        let dex = (dql >> 7) & 15;
        let dmn = dql & 127;
        let dqt = 128 + dmn;
        let dqmag = if (dql >> 11) == 0 { ((dqt << 7) >> (14 - dex)) as u16 } else { 0 };
        let dq = if self.is_40k { ((dqs as u16) << 15) | (dqmag & 0x7FFF) } else { ((dqs as u16) << 14) | (dqmag & 0x3FFF) };

        // 5. ADDB & ADDC
        let dqi = if dqs == 0 {
            dq as u32
        } else if self.is_40k {
            65536_u32.wrapping_sub((dq & 0x7FFF) as u32) & 0xFFFF
        } else {
            65536_u32.wrapping_sub((dq & 0x3FFF) as u32) & 0xFFFF
        };
        let sei_ext = if se >= 0 { se as u32 } else { (65536 + se as i32) as u32 };
        let sr = ((dqi + sei_ext) & 0xFFFF) as u16;

        let sezi_ext = if sez >= 0 { sez as u32 } else { (65536 + sez as i32) as u32 };
        let dqsez = ((dqi + sezi_ext) & 0xFFFF) as u16;
        let pk0 = ((dqsez >> 15) & 1) as u8;
        let sigpk = if dqsez == 0 { 1 } else { 0 };

        // 6. FLOATA & FLOATB
        let dq_mag = if self.is_40k { dq & 0x7FFF } else { dq & 0x3FFF };
        let (dq_exp, dq_mant) = if dq_mag == 0 {
            (0, 32)
        } else {
            let max_exp = if self.is_40k { 15 } else { 14 };
            let exp = (16 - dq_mag.leading_zeros()).min(max_exp);
            let mant = (((dq_mag as u32) << 6) >> exp) as u16;
            (exp as u16, mant)
        };
        let dq0 = ((dqs as u16) << 10) | (dq_exp << 6) | (dq_mant & 63);

        let sr_s = ((sr >> 15) & 1) as u8;
        let sr_mag = if sr_s == 0 { sr } else { (65536_u32.wrapping_sub(sr as u32) & 0x7FFF) as u16 };
        let (sr_exp, sr_mant) = if sr_mag == 0 {
            (0, 32)
        } else {
            let exp = (16 - sr_mag.leading_zeros()).min(15);
            let mant = (((sr_mag as u32) << 6) >> exp) as u16;
            (exp as u16, mant)
        };
        let sr0 = ((sr_s as u16) << 10) | (sr_exp << 6) | (sr_mant & 63);

        self.update_adaptation_and_predictors(code, y, pk0, sigpk, dqs, dq_mag);

        self.sr = [sr0, self.sr[0]];
        self.dq = [dq0, self.dq[0], self.dq[1], self.dq[2], self.dq[3], self.dq[4]];
        self.pk = [pk0, self.pk[0]];

        let sr_signed = if sr < 32768 { sr as i32 } else { sr as i32 - 65536 };
        let out_pcm = (sr_signed << 2).clamp(-32768, 32767) as i16;
        (code, out_pcm)
    }

    #[inline]
    fn adapt_scale(&mut self, code: u8, y: i32) {
        let wi = get_wi(self.rate, code);
        let dif = (((wi as i32) << 5) + 131072 - y) & 131071;
        let difs = (dif >> 16) & 1;
        let difsx = if difs == 0 { dif >> 5 } else { (dif >> 5) + 4096 };
        let yut = (y + difsx) & 8191;
        let yup = yut.clamp(544, 5120);

        let dif = (yup + ((1048576 - self.yl) >> 6)) & 16383;
        let difs = (dif >> 13) & 1;
        let difsx = if difs == 0 { dif } else { dif + 507904 };
        self.yu = yup as i16;
        self.yl = (self.yl + difsx) & 524287;
    }

    #[inline]
    fn adapt_speed(&mut self, code: u8, y: i32, dq_mag: u16) -> (i16, i16, u8) {
        let fi = get_fi(self.rate, code);
        let dif = (((fi as i32) << 9) + 8192 - self.dms as i32) & 8191;
        let difs = (dif >> 12) & 1;
        let difsx = if difs == 0 { dif >> 5 } else { (dif >> 5) + 3840 };
        let dmsp = ((difsx + self.dms as i32) & 4095) as i16;

        let dif = (((fi as i32) << 11) + 32768 - self.dml as i32) & 32767;
        let difs = (dif >> 14) & 1;
        let difsx = if difs == 0 { dif >> 7 } else { (dif >> 7) + 16128 };
        let dmlp = ((difsx + self.dml as i32) & 16383) as i16;

        let dif = (((dmsp as i32) << 2) + 32768 - dmlp as i32) & 32767;
        let difs = (dif >> 14) & 1;
        let difm = if difs == 0 { dif } else { (32768 - dif) & 16383 };
        let dthr = (dmlp as i32) >> 3;

        let tdp = if (32768..53760).contains(&self.a[1]) { 1 } else { 0 };
        let ax = if y >= 1536 && tdp == 0 && difm < dthr { 0 } else { 1 };

        let dif = ((ax << 9) + 2048 - self.ap as i32) & 2047;
        let difs = (dif >> 10) & 1;
        let difsx = if difs == 0 { dif >> 4 } else { (dif >> 4) + 1920 };
        let app = ((difsx + self.ap as i32) & 1023) as i16;

        let ylint = (self.yl >> 15) as u32;
        let ylfrac = ((self.yl >> 10) & 31) as u32;
        let thr1 = (32 + ylfrac) << ylint;
        let thr2 = if self.is_40k && ylint > 9 {
            31 << 10
        } else if !self.is_40k && ylint > 8 {
            31 << 9
        } else {
            thr1
        };
        let dqthr = (thr2 + (thr2 >> 1)) >> 1;
        let tr = if (dq_mag as u32) > dqthr && self.td == 1 { 1 } else { 0 };
        let apr = if tr == 1 { 256 } else { app };

        self.dms = dmsp;
        self.dml = dmlp;
        self.ap = apr;
        self.td = if tr == 1 { 0 } else { tdp };

        (apr, tdp, tr)
    }

    #[inline]
    fn update_predictors(&mut self, pk0: u8, sigpk: u8, dqs: u8, dq_mag: u16, tr: u8) {
        let pks = pk0 ^ self.pk[0];
        let uga1 = if sigpk == 1 {
            0
        } else if pks == 0 {
            192
        } else {
            65344
        };
        let a1s = (self.a[0] >> 15) & 1;
        let ula1 = if a1s == 0 {
            (65536_u32.wrapping_sub((self.a[0] >> 8) as u32) & 0xFFFF) as u16
        } else {
            (65536_u32.wrapping_sub(((self.a[0] >> 8) + 65280) as u32) & 0xFFFF) as u16
        };
        let ua1 = (uga1 as u32 + ula1 as u32) & 0xFFFF;
        let a1t = ((self.a[0] as u32 + ua1) & 0xFFFF) as u16;

        let pks1 = pk0 ^ self.pk[0];
        let pks2 = pk0 ^ self.pk[1];
        let uga2a = if pks2 == 0 { 16384_u32 } else { 114688_u32 };
        let a1_u = self.a[0];
        let fa1 = if a1s == 0 {
            if a1_u <= 8191 { (a1_u as u32) << 2 } else { 8191 << 2 }
        } else if a1_u >= 57345 {
            ((a1_u as u32) << 2) & 131071
        } else {
            24577 << 2
        };
        let fa = if pks1 == 1 { fa1 } else { (131072_u32.wrapping_sub(fa1)) & 131071 };
        let uga2b = (uga2a + fa) & 131071;
        let uga2s = (uga2b >> 16) & 1;
        let uga2 = if sigpk == 1 {
            0
        } else if uga2s == 0 {
            uga2b >> 7
        } else {
            (uga2b >> 7) + 64512
        } as u16;
        let a2s = (self.a[1] >> 15) & 1;
        let ula2 = if a2s == 0 {
            (65536_u32.wrapping_sub((self.a[1] >> 7) as u32) & 0xFFFF) as u16
        } else {
            (65536_u32.wrapping_sub(((self.a[1] >> 7) + 65024) as u32) & 0xFFFF) as u16
        };
        let ua2 = (uga2 as u32 + ula2 as u32) & 0xFFFF;
        let a2t = ((self.a[1] as u32 + ua2) & 0xFFFF) as u16;

        let a2p = if (32768..=53248).contains(&a2t) {
            53248
        } else if (12288..=32767).contains(&a2t) {
            12288
        } else {
            a2t
        };

        let ome = 15360_u32;
        let a1ul = ((ome + 65536 - a2p as u32) & 0xFFFF) as u16;
        let a1ll = ((a2p as u32 + 65536 - ome) & 0xFFFF) as u16;
        let a1p = if a1t >= 32768 && a1t <= a1ll {
            a1ll
        } else if a1t >= a1ul && a1t <= 32767 {
            a1ul
        } else {
            a1t
        };

        let a1r = if tr == 1 { 0 } else { a1p };
        let a2r = if tr == 1 { 0 } else { a2p };

        let mut new_b = [0u16; 6];
        for (i, slot) in new_b.iter_mut().enumerate() {
            let un = (dqs ^ ((self.dq[i] >> 10) as u8)) & 1;
            let ugbn = if dq_mag == 0 {
                0
            } else if un == 0 {
                128
            } else {
                65408
            };
            let bns = (self.b[i] >> 15) & 1;
            let ulbn = if self.is_40k {
                if bns == 0 {
                    (65536_u32.wrapping_sub((self.b[i] >> 9) as u32) & 0xFFFF) as u16
                } else {
                    (65536_u32.wrapping_sub(((self.b[i] >> 9) + 65408) as u32) & 0xFFFF) as u16
                }
            } else if bns == 0 {
                (65536_u32.wrapping_sub((self.b[i] >> 8) as u32) & 0xFFFF) as u16
            } else {
                (65536_u32.wrapping_sub(((self.b[i] >> 8) + 65280) as u32) & 0xFFFF) as u16
            };
            let ubn = (ugbn as u32 + ulbn as u32) & 0xFFFF;
            let bnp = ((self.b[i] as u32 + ubn) & 0xFFFF) as u16;
            *slot = if tr == 1 { 0 } else { bnp };
        }

        self.a[0] = a1r;
        self.a[1] = a2r;
        self.b = new_b;
    }

    #[inline]
    fn update_adaptation_and_predictors(&mut self, code: u8, y: i32, pk0: u8, sigpk: u8, dqs: u8, dq_mag: u16) {
        self.adapt_scale(code, y);
        let (_, _, tr) = self.adapt_speed(code, y, dq_mag);
        self.update_predictors(pk0, sigpk, dqs, dq_mag, tr);
    }
}

// ---------------------------------------------------------------------------
// Decoder & Encoder structs
// ---------------------------------------------------------------------------

/// G.726 ADPCM decoder.
#[derive(Debug, Clone)]
pub struct Decoder {
    core: Core,
    packing: BitPacking,
    bit_buf: u32,
    num_bits: usize,
}

impl Decoder {
    /// Create a new G.726 decoder at the specified bit rate and bit packing.
    #[must_use]
    pub fn new(rate: BitRate, packing: BitPacking) -> Self {
        Self { core: Core::new(rate), packing, bit_buf: 0, num_bits: 0 }
    }

    /// Reset decoder internal predictor state.
    pub fn reset(&mut self) {
        self.core.reset();
        self.bit_buf = 0;
        self.num_bits = 0;
    }

    /// Decode a single quantized ADPCM code into an 8 kHz 16-bit linear PCM sample.
    /// Bits above the configured code width are ignored, as in packed-stream decoding.
    #[inline]
    pub fn decode_sample(&mut self, code: u8) -> i16 {
        // Adaptation must see the same bounded codeword as the reconstruction table;
        // otherwise unused high bits can underflow its unsigned inverse-code arithmetic.
        let code = code & ((1u8 << self.core.rate.code_size()) - 1);
        self.core.step(0, false, code).1
    }

    /// Decode G.726 bitstream bytes into 8 kHz 16-bit linear PCM samples, appending to `dst`.
    pub fn decode(&mut self, src: &[u8], dst: &mut Vec<i16>) -> Result<(), Error> {
        if src.len() > MAX_INPUT_BYTES {
            return Err(Error::InputTooLarge);
        }
        let cs = self.core.rate.code_size();
        let est_samples = (src.len() * 8) / cs;
        dst.try_reserve(est_samples).map_err(|_| Error::AllocationFailed)?;

        match self.packing {
            BitPacking::BigEndian => {
                for &b in src {
                    self.bit_buf = ((self.bit_buf & ((1 << self.num_bits) - 1)) << 8) | (b as u32);
                    self.num_bits += 8;
                    while self.num_bits >= cs {
                        let code = ((self.bit_buf >> (self.num_bits - cs)) & ((1 << cs) - 1)) as u8;
                        self.num_bits -= cs;
                        let s = self.decode_sample(code);
                        dst.push(s);
                    }
                    self.bit_buf &= (1 << self.num_bits) - 1;
                }
            }
            BitPacking::LittleEndian => {
                for &b in src {
                    self.bit_buf |= (b as u32) << self.num_bits;
                    self.num_bits += 8;
                    while self.num_bits >= cs {
                        let code = (self.bit_buf & ((1 << cs) - 1)) as u8;
                        self.bit_buf >>= cs;
                        self.num_bits -= cs;
                        let s = self.decode_sample(code);
                        dst.push(s);
                    }
                }
            }
        }
        Ok(())
    }
}

/// G.726 ADPCM encoder.
#[derive(Debug, Clone)]
pub struct Encoder {
    core: Core,
    packing: BitPacking,
    bit_buf: u32,
    num_bits: usize,
}

impl Encoder {
    /// Create a new G.726 encoder at the specified bit rate and bit packing.
    #[must_use]
    pub fn new(rate: BitRate, packing: BitPacking) -> Self {
        Self { core: Core::new(rate), packing, bit_buf: 0, num_bits: 0 }
    }

    /// Reset encoder internal predictor state.
    pub fn reset(&mut self) {
        self.core.reset();
        self.bit_buf = 0;
        self.num_bits = 0;
    }

    /// Encode a single 8 kHz 16-bit linear PCM sample into an ADPCM code word.
    #[inline]
    pub fn encode_sample(&mut self, sample: i16) -> u8 {
        self.core.step(sample, true, 0).0
    }

    /// Encode 8 kHz 16-bit linear PCM samples into G.726 bitstream bytes, appending to `dst`.
    pub fn encode(&mut self, src: &[i16], dst: &mut Vec<u8>) -> Result<(), Error> {
        if src.len() > MAX_SAMPLES {
            return Err(Error::InputTooLarge);
        }
        let cs = self.core.rate.code_size();
        let est_bytes = (src.len() * cs).div_ceil(8);
        dst.try_reserve(est_bytes).map_err(|_| Error::AllocationFailed)?;

        match self.packing {
            BitPacking::BigEndian => {
                for &s in src {
                    let code = self.encode_sample(s);
                    self.bit_buf = ((self.bit_buf & ((1 << self.num_bits) - 1)) << cs) | ((code as u32) & ((1 << cs) - 1));
                    self.num_bits += cs;
                    while self.num_bits >= 8 {
                        dst.push(((self.bit_buf >> (self.num_bits - 8)) & 0xFF) as u8);
                        self.num_bits -= 8;
                    }
                    self.bit_buf &= (1 << self.num_bits) - 1;
                }
            }
            BitPacking::LittleEndian => {
                for &s in src {
                    let code = self.encode_sample(s);
                    self.bit_buf |= (code as u32) << self.num_bits;
                    self.num_bits += cs;
                    while self.num_bits >= 8 {
                        dst.push((self.bit_buf & 0xFF) as u8);
                        self.bit_buf >>= 8;
                        self.num_bits -= 8;
                    }
                }
            }
        }
        Ok(())
    }

    /// Flush any remaining buffered bits into `dst`, padding with zeroes.
    pub fn flush(&mut self, dst: &mut Vec<u8>) -> Result<(), Error> {
        if self.num_bits > 0 {
            dst.try_reserve(1).map_err(|_| Error::AllocationFailed)?;
            match self.packing {
                BitPacking::BigEndian => {
                    let b = ((self.bit_buf << (8 - self.num_bits)) & 0xFF) as u8;
                    dst.push(b);
                }
                BitPacking::LittleEndian => {
                    let b = (self.bit_buf & 0xFF) as u8;
                    dst.push(b);
                }
            }
            self.bit_buf = 0;
            self.num_bits = 0;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public Top-Level Functions
// ---------------------------------------------------------------------------

/// Decode a mono G.726 bitstream into 8 kHz 16-bit linear PCM samples.
pub fn decode(bytes: &[u8], rate: BitRate, packing: BitPacking) -> Result<Vec<i16>, Error> {
    let mut decoder = Decoder::new(rate, packing);
    let mut out = Vec::new();
    decoder.decode(bytes, &mut out)?;
    Ok(out)
}

/// Encode mono 8 kHz 16-bit linear PCM samples into a G.726 bitstream.
pub fn encode(samples: &[i16], rate: BitRate, packing: BitPacking) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder::new(rate, packing);
    let mut out = Vec::new();
    encoder.encode(samples, &mut out)?;
    encoder.flush(&mut out)?;
    Ok(out)
}

/// Decode an interleaved multi-channel G.726 stream into interleaved 8 kHz 16-bit PCM samples.
pub fn decode_interleaved(bytes: &[u8], channels: usize, rate: BitRate, packing: BitPacking) -> Result<Vec<i16>, Error> {
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(Error::InvalidChannels(channels));
    }
    if channels == 1 {
        return decode(bytes, rate, packing);
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(Error::InputTooLarge);
    }

    let mut decoders: Vec<Decoder> = Vec::with_capacity(channels);
    for _ in 0..channels {
        decoders.push(Decoder::new(rate, packing));
    }

    let cs = rate.code_size();
    let est_samples = (bytes.len() * 8) / cs;
    let mut out = Vec::new();
    out.try_reserve(est_samples).map_err(|_| Error::AllocationFailed)?;

    let mut ch_outs: Vec<Vec<i16>> = Vec::with_capacity(channels);
    for _ in 0..channels {
        ch_outs.push(Vec::new());
    }

    for (i, &b) in bytes.iter().enumerate() {
        let ch = i % channels;
        decoders[ch].decode(&[b], &mut ch_outs[ch])?;
    }

    let min_len = ch_outs.iter().map(Vec::len).min().unwrap_or(0);
    for i in 0..min_len {
        for ch_out in ch_outs.iter().take(channels) {
            out.push(ch_out[i]);
        }
    }

    Ok(out)
}

/// Encode interleaved 8 kHz 16-bit PCM samples into an interleaved multi-channel G.726 stream.
pub fn encode_interleaved(samples: &[i16], channels: usize, rate: BitRate, packing: BitPacking) -> Result<Vec<u8>, Error> {
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(Error::InvalidChannels(channels));
    }
    if channels == 1 {
        return encode(samples, rate, packing);
    }
    if samples.len() > MAX_SAMPLES {
        return Err(Error::InputTooLarge);
    }

    let mut encoders: Vec<Encoder> = Vec::with_capacity(channels);
    for _ in 0..channels {
        encoders.push(Encoder::new(rate, packing));
    }

    let cs = rate.code_size();
    let est_bytes = (samples.len() * cs).div_ceil(8);
    let mut out = Vec::new();
    out.try_reserve(est_bytes).map_err(|_| Error::AllocationFailed)?;

    let mut ch_samples: Vec<Vec<i16>> = Vec::with_capacity(channels);
    for _ in 0..channels {
        ch_samples.push(Vec::new());
    }

    for (i, &s) in samples.iter().enumerate() {
        let ch = i % channels;
        ch_samples[ch].push(s);
    }

    let mut ch_bytes: Vec<Vec<u8>> = Vec::with_capacity(channels);
    for ch in 0..channels {
        let mut b = Vec::new();
        encoders[ch].encode(&ch_samples[ch], &mut b)?;
        encoders[ch].flush(&mut b)?;
        ch_bytes.push(b);
    }

    let min_bytes = ch_bytes.iter().map(Vec::len).min().unwrap_or(0);
    for i in 0..min_bytes {
        for ch_b in ch_bytes.iter().take(channels) {
            out.push(ch_b[i]);
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_api_ignores_unused_codeword_bits() {
        for rate in [BitRate::Rate16k, BitRate::Rate24k, BitRate::Rate32k, BitRate::Rate40k] {
            let mask = (1u8 << rate.code_size()) - 1;
            let mut actual = Decoder::new(rate, BitPacking::BigEndian);
            let mut expected = Decoder::new(rate, BitPacking::BigEndian);
            for code in 0..=u8::MAX {
                assert_eq!(actual.decode_sample(code), expected.decode_sample(code & mask));
            }
            // High bits must not leave a different predictor state for later valid codes.
            for code in 0..=mask {
                assert_eq!(actual.decode_sample(code), expected.decode_sample(code));
            }
        }
    }

    #[test]
    fn reject_invalid_channels() {
        assert!(decode_interleaved(&[], 0, BitRate::Rate32k, BitPacking::BigEndian).is_err());
        assert!(decode_interleaved(&[], 9, BitRate::Rate32k, BitPacking::BigEndian).is_err());
        assert!(encode_interleaved(&[], 0, BitRate::Rate32k, BitPacking::BigEndian).is_err());
        assert!(encode_interleaved(&[], 9, BitRate::Rate32k, BitPacking::BigEndian).is_err());
    }

    #[test]
    fn roundtrip_silence_all_rates() {
        let pcm = vec![0i16; 64];
        for rate in [BitRate::Rate16k, BitRate::Rate24k, BitRate::Rate32k, BitRate::Rate40k] {
            for packing in [BitPacking::BigEndian, BitPacking::LittleEndian] {
                let encoded = encode(&pcm, rate, packing).expect("encode silence");
                let decoded = decode(&encoded, rate, packing).expect("decode silence");
                assert_eq!(decoded.len(), pcm.len());
                for &s in &decoded {
                    assert!(s.abs() <= 16, "silence sample too large: {s}");
                }
            }
        }
    }

    #[test]
    fn hostile_input_does_not_panic() {
        let mut decoder = Decoder::new(BitRate::Rate32k, BitPacking::BigEndian);
        let mut out = Vec::new();
        let hostile = vec![0xFF; 1024];
        let _ = decoder.decode(&hostile, &mut out);

        let mut decoder = Decoder::new(BitRate::Rate16k, BitPacking::LittleEndian);
        let mut out = Vec::new();
        let _ = decoder.decode(&[0x00, 0xAA, 0x55, 0x12, 0x34], &mut out);
    }
}
