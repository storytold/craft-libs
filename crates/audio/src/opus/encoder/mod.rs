//! Clean-room pure-Rust Opus encoder (RFC 6716 SILK, hybrid and CELT modes; RFC 7845 `OpusHead`).
//!
//! Produces 20 ms Opus packets from 48 kHz float input, mono or stereo. The coding mode and
//! audio bandwidth are chosen once per stream from the per-channel bitrate and the
//! [`Application`] (see the crate README for the table):
//!
//! - **SILK** (narrowband/mediumband/wideband): linear-predictive speech coding at an internal
//!   rate of 8/12/16 kHz; packet sizes vary around the target bitrate.
//! - **Hybrid** (super-wideband/fullband): SILK wideband codes 0–8 kHz and CELT codes the bands
//!   above in the same range-coded frame; constant packet size.
//! - **CELT** (fullband): MDCT transform coding; constant packet size.
//!
//! From EffectCraft's `effectcraft-opusenc` (storytold/effectcraft@c618429). Public items keep their
//! names, so EffectCraft can re-export this module.
//!
//! ```
//! # #[cfg(feature = "opus-encode")] {
//! use craft_audio::opus::encoder::{Application, Mode, Encoder};
//! let mut enc = Encoder::new(2, 128_000).unwrap();
//! let pcm = vec![0.0f32; Encoder::FRAME_SIZE * 2];
//! let packet = enc.encode_float(&pcm).unwrap();
//! assert_eq!(packet[0] >> 3, 31); // CELT-only, fullband, 20 ms
//! assert_eq!(enc.opus_head().len(), 19);
//! let voip = Encoder::with_application(1, 16_000, Application::Voip).unwrap();
//! assert_eq!(voip.mode(), Mode::Silk);
//! # }
//! ```

// The bit-allocation code mirrors the normative integer arithmetic; keep C-like shift expressions.
#![allow(clippy::precedence, clippy::int_plus_one)]

mod bands;
mod energy;
mod mdct;
mod range;
mod rate;
mod resample;
mod silk;
mod tables;

use std::sync::OnceLock;

use bands::{SPREAD_NORMAL, quant_all_bands};
use energy::{quant_coarse_energy, quant_energy_finalise, quant_fine_energy};
use mdct::Mdct;
use range::{BITRES, RangeEncoder};
use rate::{Mode as CeltMode, compute_allocation};
use resample::Downsampler;
use silk::SilkEncoder;
use tables::{E_MEANS, EBANDS, NB_EBANDS, OVERLAP, PREEMPH, SPREAD_ICDF, TF_SELECT_TABLE, TRIM_ICDF};

/// Samples per channel in one 20 ms frame at 48 kHz.
const N: usize = 960;
/// LM of a 20 ms frame (8 short blocks).
const LM: usize = 3;
/// Smallest CELT payload the encoder produces (bytes).
const MIN_FRAME_BYTES: usize = 16;
/// Largest payload allowed in one frame (RFC 6716 §3.4, R2).
const MAX_FRAME_BYTES: usize = 1275;
/// First CELT band coded in hybrid frames (8 kHz).
const HYBRID_START_BAND: usize = 17;

fn celt_mode() -> &'static CeltMode {
    static MODE: OnceLock<CeltMode> = OnceLock::new();
    MODE.get_or_init(CeltMode::new)
}

/// Why an encoder could not be created or a frame could not be encoded.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Opus family-0 streams have 1 or 2 channels.
    #[error("Opus family-0 streams have 1 or 2 channels, not {0}")]
    Channels(u8),
    /// The mode cannot code that bandwidth.
    #[error("unsupported mode/bandwidth combination {0:?}/{1:?}")]
    ModeBandwidth(Mode, Bandwidth),
    /// `encode_float` takes exactly `FRAME_SIZE * channels` samples.
    #[error("a frame is {expected} interleaved samples, not {got}")]
    FrameLength {
        /// Interleaved samples a frame takes (`FRAME_SIZE * channels`).
        expected: usize,
        /// Interleaved samples passed.
        got: usize,
    },
}

/// Intended use of the stream; selects the mode/bandwidth table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Application {
    /// General audio (music, mixed content): CELT unless the bitrate is low.
    #[default]
    Audio,
    /// Speech: SILK and hybrid over a wider bitrate range, narrower bandwidths at low rates.
    Voip,
}

/// Opus coding mode (RFC 6716 §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// MDCT transform coding only.
    Celt,
    /// Linear-prediction coding only.
    Silk,
    /// SILK below 8 kHz plus CELT above.
    Hybrid,
}

pub use crate::opus::Bandwidth;

/// Mode and bandwidth for a per-channel bitrate (bits per second per channel).
fn choose_mode(app: Application, per_channel: u32) -> (Mode, Bandwidth) {
    match app {
        Application::Voip => match per_channel {
            0..10_000 => (Mode::Silk, Bandwidth::Narrowband),
            10_000..12_000 => (Mode::Silk, Bandwidth::Mediumband),
            12_000..18_000 => (Mode::Silk, Bandwidth::Wideband),
            18_000..28_000 => (Mode::Hybrid, Bandwidth::SuperWideband),
            28_000..40_000 => (Mode::Hybrid, Bandwidth::Fullband),
            _ => (Mode::Celt, Bandwidth::Fullband),
        },
        Application::Audio => match per_channel {
            0..16_000 => (Mode::Silk, Bandwidth::Wideband),
            16_000..36_000 => (Mode::Hybrid, Bandwidth::Fullband),
            _ => (Mode::Celt, Bandwidth::Fullband),
        },
    }
}

/// SILK side of a SILK-only or hybrid stream.
struct SilkPath {
    enc: SilkEncoder,
    down: Vec<Downsampler>,
    /// Bits per frame aimed at (excluding the TOC byte in SILK-only mode).
    target_bits: f64,
    /// Bits saved (positive) or overspent by previous frames (SILK-only mode).
    reservoir: f64,
}

/// The name EffectCraft's `effectcraft-opusenc` used, kept so it can re-export this module.
#[doc(hidden)]
pub type OpusEncoder = Encoder;

/// Opus encoder for one mono or stereo stream at 48 kHz.
pub struct Encoder {
    channels: usize,
    application: Application,
    mode: Mode,
    bandwidth: Bandwidth,
    /// Requested bitrate (clamped), bits per second.
    bitrate_bps: u32,
    /// Payload bytes per packet in CELT and hybrid modes (the packet is one byte longer: the
    /// TOC).
    frame_bytes: usize,
    mdct: Mdct,
    /// Last input sample of each channel (pre-emphasis filter memory, ×32768).
    preemph_mem: [f32; 2],
    /// Last `OVERLAP` pre-emphasised samples of each channel.
    hist: Vec<f32>,
    /// Quantised band energies, exactly as the decoder holds them.
    old_band_e: [f32; 2 * NB_EBANDS],
    block: Vec<f32>,
    freq: Vec<f32>,
    silk: Option<SilkPath>,
}

impl Encoder {
    /// Samples per channel per packet (20 ms at 48 kHz).
    pub const FRAME_SIZE: usize = N;

    /// Creates a general-audio encoder ([`Application::Audio`]) for `channels` (1 or 2)
    /// channels of 48 kHz audio at `bitrate_bps` (total for all channels).
    pub fn new(channels: u8, bitrate_bps: u32) -> Result<Self, Error> {
        Self::with_application(channels, bitrate_bps, Application::Audio)
    }

    /// Creates an encoder for `channels` (1 or 2) channels of 48 kHz audio at `bitrate_bps`
    /// (total for all channels; clamped to 6 kb/s per channel – 510 kb/s), choosing the mode
    /// and bandwidth for `application`.
    pub fn with_application(channels: u8, bitrate_bps: u32, application: Application) -> Result<Self, Error> {
        let c = channels.max(1) as u32;
        let (mode, bandwidth) = choose_mode(application, bitrate_bps.clamp(6_000 * c, 510_000) / c);
        Self::build(channels, bitrate_bps, application, mode, bandwidth)
    }

    /// Creates an encoder with an explicit mode and bandwidth instead of the application's
    /// table. Valid combinations: SILK with narrowband, mediumband or wideband; hybrid with
    /// super-wideband or fullband; CELT with fullband.
    ///
    /// Any other combination is [`Error::ModeBandwidth`]; `channels` other than 1 or 2 is
    /// [`Error::Channels`].
    pub fn with_mode(channels: u8, bitrate_bps: u32, mode: Mode, bandwidth: Bandwidth) -> Result<Self, Error> {
        let ok = match mode {
            Mode::Silk => bandwidth <= Bandwidth::Wideband,
            Mode::Hybrid => bandwidth >= Bandwidth::SuperWideband,
            Mode::Celt => bandwidth == Bandwidth::Fullband,
        };
        if !ok {
            return Err(Error::ModeBandwidth(mode, bandwidth));
        }
        Self::build(channels, bitrate_bps, Application::Audio, mode, bandwidth)
    }

    fn build(channels: u8, bitrate_bps: u32, application: Application, mode: Mode, bandwidth: Bandwidth) -> Result<Self, Error> {
        if channels != 1 && channels != 2 {
            return Err(Error::Channels(channels));
        }
        let c = channels as usize;
        let bitrate_bps = bitrate_bps.clamp(6_000 * c as u32, 510_000);
        let packet_bytes = (bitrate_bps as usize).div_ceil(400);
        let frame_bytes = packet_bytes.saturating_sub(1).clamp(MIN_FRAME_BYTES, MAX_FRAME_BYTES);
        let silk = match mode {
            Mode::Celt => None,
            Mode::Silk | Mode::Hybrid => {
                let fs_khz = match bandwidth {
                    Bandwidth::Narrowband => 8,
                    Bandwidth::Mediumband => 12,
                    _ => 16,
                };
                let target_bits = if mode == Mode::Silk {
                    bitrate_bps as f64 / 50.0 - 8.0
                } else {
                    // SILK gets most of a hybrid packet; CELT codes 8 kHz and up with the rest.
                    let per = (bitrate_bps / c as u32) as f64;
                    let share = if bandwidth == Bandwidth::SuperWideband { 0.72 } else { 0.62 };
                    c as f64 * (per * share).min(per - 6_000.0).max(per * 0.5) / 50.0
                };
                Some(SilkPath {
                    enc: SilkEncoder::new(fs_khz, c),
                    down: (0..c).map(|_| Downsampler::new(fs_khz as u32 * 1000)).collect(),
                    target_bits,
                    reservoir: 0.0,
                })
            }
        };
        Ok(Encoder {
            channels: c,
            application,
            mode,
            bandwidth,
            bitrate_bps,
            frame_bytes,
            mdct: Mdct::new(N),
            preemph_mem: [0.0; 2],
            hist: vec![0.0; c * OVERLAP],
            old_band_e: [0.0; 2 * NB_EBANDS],
            block: vec![0.0; N + OVERLAP],
            freq: vec![0.0; c * N],
            silk,
        })
    }

    /// Number of channels.
    pub fn channels(&self) -> u8 {
        self.channels as u8
    }

    /// The application the encoder was created for.
    pub fn application(&self) -> Application {
        self.application
    }

    /// Coding mode of every packet of the stream.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Coded audio bandwidth of every packet of the stream.
    pub fn bandwidth(&self) -> Bandwidth {
        self.bandwidth
    }

    /// Bitrate in bits per second: the constant packet size × 50 packets per second in CELT
    /// and hybrid modes, the (average) target in SILK mode.
    pub fn bitrate(&self) -> u32 {
        match self.mode {
            Mode::Silk => self.bitrate_bps,
            _ => ((self.frame_bytes + 1) * 8 * 50) as u32,
        }
    }

    /// Samples at 48 kHz the decoder must discard from the start of the stream (the `OpusHead`
    /// pre-skip): the encoder's total delay. CELT: the 120-sample MDCT overlap. SILK/hybrid:
    /// the decimator (83 samples, 88 for narrowband) plus the decoder's SILK delay (one
    /// internal-rate sample and the resampler delay of RFC 6716 Table 54), which also rounds
    /// to 120 samples.
    pub fn pre_skip(&self) -> u16 {
        let (fs, table54_ms) = match (self.mode, self.bandwidth) {
            (Mode::Celt, _) => return OVERLAP as u16,
            (_, Bandwidth::Narrowband) => (8_000, 0.538),
            (_, Bandwidth::Mediumband) => (12_000, 0.692),
            _ => (16_000, 0.706),
        };
        (resample::delay(fs) as f64 + 48_000.0 / fs as f64 + table54_ms * 48.0).round() as u16
    }

    /// The 19-byte `OpusHead` identification header (RFC 7845 §5.1): version 1, channel count,
    /// pre-skip, input sample rate 48000, output gain 0, channel mapping family 0.
    pub fn opus_head(&self) -> Vec<u8> {
        let mut h = Vec::with_capacity(19);
        h.extend_from_slice(b"OpusHead");
        h.push(1);
        h.push(self.channels as u8);
        h.extend_from_slice(&self.pre_skip().to_le_bytes());
        h.extend_from_slice(&48_000u32.to_le_bytes());
        h.extend_from_slice(&0i16.to_le_bytes());
        h.push(0);
        h
    }

    /// TOC byte (RFC 6716 §3.1, Table 2) of a 20 ms single-frame packet.
    fn toc(&self) -> u8 {
        let config: u8 = match (self.mode, self.bandwidth) {
            (Mode::Silk, Bandwidth::Narrowband) => 1,
            (Mode::Silk, Bandwidth::Mediumband) => 5,
            (Mode::Silk, _) => 9,
            (Mode::Hybrid, Bandwidth::SuperWideband) => 13,
            (Mode::Hybrid, _) => 15,
            (Mode::Celt, _) => 31,
        };
        (config << 3) | if self.channels == 2 { 4 } else { 0 }
    }

    /// Encodes one 20 ms frame of interleaved f32 samples in [-1, 1] (exactly
    /// `FRAME_SIZE * channels` values) and returns one Opus packet, or
    /// [`Error::FrameLength`] for any other length.
    pub fn encode_float(&mut self, pcm: &[f32]) -> Result<Vec<u8>, Error> {
        let c = self.channels;
        if pcm.len() != N * c {
            return Err(Error::FrameLength { expected: N * c, got: pcm.len() });
        }
        let mut packet = vec![self.toc()];
        match self.mode {
            Mode::Celt => {
                let (log_e, silence) = self.celt_analysis(pcm);
                let len = self.frame_bytes;
                packet.extend_from_slice(&self.encode_celt(RangeEncoder::new(), len, 0, NB_EBANDS, &log_e, silence));
            }
            Mode::Silk => {
                let x = self.silk_input(pcm);
                // The SILK state exists whenever the mode uses SILK (set with the mode).
                if let Some(sp) = self.silk.as_mut() {
                    let target = sp.target_bits + 0.25 * sp.reservoir;
                    let mut enc = RangeEncoder::new();
                    sp.enc.encode(&x, target.max(24.0), (MAX_FRAME_BYTES * 8) as i32, &mut enc);
                    let bytes = (enc.tell() as usize).div_ceil(8).max(2);
                    let lim = 4.0 * sp.target_bits;
                    sp.reservoir = (sp.reservoir + sp.target_bits - (bytes * 8) as f64).clamp(-lim, lim);
                    packet.extend_from_slice(&enc.done(bytes));
                }
            }
            Mode::Hybrid => {
                let x = self.silk_input(pcm);
                let len = self.frame_bytes;
                let total = (len * 8) as i32;
                let celt_min = (total / 5).max(80);
                let mut enc = RangeEncoder::new();
                if let Some(sp) = self.silk.as_mut() {
                    sp.enc.encode(&x, sp.target_bits, total - celt_min, &mut enc);
                }
                // No redundancy (RFC 6716 §4.5.1); the flag is present when there is room.
                if enc.tell() + 17 + 20 <= total {
                    enc.bit_logp(false, 12);
                }
                let (log_e, _) = self.celt_analysis(pcm);
                let end = if self.bandwidth == Bandwidth::SuperWideband { 19 } else { NB_EBANDS };
                packet.extend_from_slice(&self.encode_celt(enc, len, HYBRID_START_BAND, end, &log_e, false));
            }
        }
        Ok(packet)
    }

    /// Decimates the input to the SILK internal rate (16-bit scale), one vector per channel.
    fn silk_input(&mut self, pcm: &[f32]) -> Vec<Vec<f32>> {
        let c = self.channels;
        let Some(sp) = self.silk.as_mut() else { return vec![vec![]; c] };
        let mut out = Vec::with_capacity(c);
        let mut tmp = vec![0f32; N];
        for ch in 0..c {
            for (j, t) in tmp.iter_mut().enumerate() {
                let s = pcm[j * c + ch];
                *t = if s.is_finite() { s.clamp(-1.0, 1.0) * 32768.0 } else { 0.0 };
            }
            let mut y = Vec::with_capacity(N / 3);
            sp.down[ch].process(&tmp, &mut y);
            out.push(y);
        }
        out
    }

    /// Pre-emphasis, the MDCT, band energies (log2, relative to `E_MEANS`) and normalised
    /// bands; returns the energies and whether the frame is digital silence.
    fn celt_analysis(&mut self, pcm: &[f32]) -> ([f32; 2 * NB_EBANDS], bool) {
        let c = self.channels;
        let mut log_e = [0f32; 2 * NB_EBANDS];
        let mut peak = 0f32;
        for ch in 0..c {
            self.block[..OVERLAP].copy_from_slice(&self.hist[ch * OVERLAP..(ch + 1) * OVERLAP]);
            let mut mem = self.preemph_mem[ch];
            for j in 0..N {
                let s = pcm[j * c + ch];
                let s = if s.is_finite() { s.clamp(-2.0, 2.0) * 32768.0 } else { 0.0 };
                let v = s - PREEMPH * mem;
                mem = s;
                self.block[OVERLAP + j] = v;
            }
            self.preemph_mem[ch] = mem;
            self.hist[ch * OVERLAP..(ch + 1) * OVERLAP].copy_from_slice(&self.block[N..]);
            peak = self.block.iter().fold(peak, |p, v| p.max(v.abs()));
            let freq = &mut self.freq[ch * N..(ch + 1) * N];
            self.mdct.forward(&self.block, freq);
            for i in 0..NB_EBANDS {
                let lo = (EBANDS[i] as usize) << LM;
                let hi = (EBANDS[i + 1] as usize) << LM;
                let e = (1e-27f32 + freq[lo..hi].iter().map(|v| v * v).sum::<f32>()).sqrt();
                log_e[ch * NB_EBANDS + i] = e.log2() - E_MEANS[i];
                let g = 1.0 / e;
                for v in freq[lo..hi].iter_mut() {
                    *v *= g;
                }
            }
        }
        (log_e, peak < 1e-4)
    }

    /// Codes one CELT frame (RFC 6716 §4.3, in bitstream order) of `len` bytes into `enc`
    /// (which already holds the SILK frame in hybrid mode), coding bands `start..end`, and
    /// finalises the frame.
    #[allow(clippy::too_many_arguments)]
    fn encode_celt(&mut self, mut enc: RangeEncoder, len: usize, start: usize, end: usize, log_e: &[f32; 2 * NB_EBANDS], silence: bool) -> Vec<u8> {
        let m = celt_mode();
        let c = self.channels;
        if c == 1 {
            for i in 0..NB_EBANDS {
                self.old_band_e[i] = self.old_band_e[i].max(self.old_band_e[NB_EBANDS + i]);
            }
        }
        if silence && start == 0 {
            // The silence flag; the decoder treats every remaining bit as consumed.
            enc.bit_logp(true, 15);
            self.old_band_e = [-28.0; 2 * NB_EBANDS];
            return enc.done(2);
        }
        let total_bits_raw = len as i32 * 8;
        if enc.tell() == 1 {
            enc.bit_logp(false, 15);
        }
        // No pitch post-filter (only signalled in CELT-only frames).
        if start == 0 && enc.tell() + 16 <= total_bits_raw {
            enc.bit_logp(false, 1);
        }
        // Long blocks only.
        if enc.tell() + 3 <= total_bits_raw {
            enc.bit_logp(false, 3);
        }
        // Coarse energy, choosing intra or inter prediction by trial (whichever is cheaper).
        if enc.tell() + 3 <= total_bits_raw {
            let mut enc_intra = enc.clone();
            let mut old_intra = self.old_band_e;
            enc_intra.bit_logp(true, 3);
            quant_coarse_energy(start, end, log_e, &mut old_intra, true, &mut enc_intra, c, LM, total_bits_raw);
            enc.bit_logp(false, 3);
            let mut old_inter = self.old_band_e;
            quant_coarse_energy(start, end, log_e, &mut old_inter, false, &mut enc, c, LM, total_bits_raw);
            if enc_intra.tell_frac() < enc.tell_frac() {
                enc = enc_intra;
                self.old_band_e = old_intra;
            } else {
                self.old_band_e = old_inter;
            }
        } else {
            quant_coarse_energy(start, end, log_e, &mut self.old_band_e, false, &mut enc, c, LM, total_bits_raw);
        }
        // TF resolution: no changes.
        {
            let mut budget = total_bits_raw;
            let mut tell = enc.tell();
            let mut logp = 4;
            let tf_select_rsv = LM > 0 && tell + logp + 1 <= budget;
            budget -= tf_select_rsv as i32;
            for _ in start..end {
                if tell + logp <= budget {
                    enc.bit_logp(false, logp as u32);
                    tell = enc.tell();
                }
                logp = 5;
            }
            if tf_select_rsv && TF_SELECT_TABLE[LM][0] != TF_SELECT_TABLE[LM][2] {
                enc.bit_logp(false, 1);
            }
        }
        if enc.tell() + 4 <= total_bits_raw {
            enc.icdf(SPREAD_NORMAL, &SPREAD_ICDF, 5);
        }
        // Dynamic allocation: no boosts.
        let cap = m.init_caps(LM, c);
        let offsets = [0i32; NB_EBANDS];
        let total_bits = total_bits_raw << BITRES;
        let mut tellf = enc.tell_frac();
        for &cp in cap.iter().take(end).skip(start) {
            if tellf + (6 << BITRES) < total_bits && 0 < cp {
                enc.bit_logp(false, 6);
                tellf = enc.tell_frac();
            }
        }
        let alloc_trim = 5;
        if tellf + (6 << BITRES) <= total_bits {
            enc.icdf(alloc_trim as usize, &TRIM_ICDF, 7);
        }
        let bits = (total_bits_raw << BITRES) - enc.tell_frac() - 1;
        let alloc = compute_allocation(m, start, end, &offsets, &cap, alloc_trim, bits, c, LM, &mut enc);
        quant_fine_energy(start, end, log_e, &mut self.old_band_e, &alloc.fine_quant, &mut enc, c);
        quant_all_bands(
            m,
            start,
            end,
            &self.freq,
            N,
            c,
            &alloc.pulses,
            SPREAD_NORMAL,
            alloc.dual_stereo,
            alloc.intensity,
            total_bits_raw << BITRES,
            alloc.balance,
            &mut enc,
            LM,
            alloc.coded_bands,
        );
        let bits_left = total_bits_raw - enc.tell();
        quant_energy_finalise(start, end, log_e, &mut self.old_band_e, &alloc.fine_quant, &alloc.fine_priority, bits_left, &mut enc, c);
        debug_assert!(enc.tell() <= total_bits_raw, "frame overflow: {} > {}", enc.tell(), total_bits_raw);
        if c == 1 {
            let (a, b) = self.old_band_e.split_at_mut(NB_EBANDS);
            b.copy_from_slice(a);
        }
        // The decoder forgets the energies of bands it did not code.
        for ch in 0..2 {
            for i in (0..start).chain(end..NB_EBANDS) {
                self.old_band_e[ch * NB_EBANDS + i] = 0.0;
            }
        }
        enc.done(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_head_layout() {
        let e = Encoder::new(2, 96_000).unwrap();
        let h = e.opus_head();
        assert_eq!(&h[..8], b"OpusHead");
        assert_eq!(h[8], 1);
        assert_eq!(h[9], 2);
        assert_eq!(u16::from_le_bytes([h[10], h[11]]), e.pre_skip());
        assert_eq!(u32::from_le_bytes([h[12], h[13], h[14], h[15]]), 48_000);
        assert_eq!(&h[16..], &[0, 0, 0]);
    }

    #[test]
    fn packet_sizes_follow_bitrate() {
        for (ch, rate) in [(1u8, 64_000u32), (2, 128_000), (2, 256_000)] {
            let mut e = Encoder::new(ch, rate).unwrap();
            let pcm: Vec<f32> = (0..N * ch as usize).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
            let p = e.encode_float(&pcm).unwrap();
            assert_eq!(p.len(), rate as usize / 400);
            assert_eq!(p[0], 0xF8 | if ch == 2 { 4 } else { 0 });
            assert_eq!(e.bitrate(), rate);
        }
    }

    #[test]
    fn silence_is_a_tiny_packet() {
        let mut e = Encoder::new(1, 64_000).unwrap();
        let p = e.encode_float(&vec![0.0; N]).unwrap();
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn wrong_frame_length_is_an_error() {
        let mut e = Encoder::new(2, 128_000).unwrap();
        assert_eq!(e.encode_float(&vec![0.0; N]), Err(Error::FrameLength { expected: 2 * N, got: N }));
        assert_eq!(e.encode_float(&[]), Err(Error::FrameLength { expected: 2 * N, got: 0 }));
        assert!(e.encode_float(&vec![0.0; 2 * N]).is_ok());
    }
}
