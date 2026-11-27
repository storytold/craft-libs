// Ported from Apple's ALAC reference implementation (ALACEncoder.cpp, macosforge/alac@c38887c,
// https://github.com/macosforge/alac). Copyright (c) 2011 Apple Inc.
// Licensed under the Apache License, Version 2.0 (LICENSE-APACHE at the repository root).
// Modified: translated to Rust and changed as the `alac` module docs describe.
// SPDX-License-Identifier: Apache-2.0

//! The packet encoder, from Apple's `ALACEncoder.cpp`.
//!
//! The search (stereo matrixing, 4 or 8 predictor coefficients) and the state kept across
//! packets (coefficients, the last matrixing, and the buffers the trial runs read stale parts of)
//! follow the reference, so for the same input this writes the same packets as it, with one
//! exception: the shifted-off low bytes. The reference passes 1 (24-bit) or 2 (32-bit) low bytes
//! of every sample through uncompressed, even for silence. Here an element shifts off fewer bytes
//! in a packet whose samples fit well within the width the reference predicts (see
//! [`packet_shift`]), so silence and quiet passages are not padded with a byte or two per sample.

use super::ag::{self, AgParams};
use super::bits::{BitCounter, BitSink, BitWriter};
use super::dp::{DENSHIFT_DEFAULT, init_coefs, pc_block};
use super::matrix::{Mix, mix, shift_mono};
use super::{Error, ID_CPE, ID_END, ID_LFE, MAX_CHANNELS, MAX_FRAME_LENGTH, Result, SpecificConfig, element_map};

const MAX_COEFS: usize = 16;
const MAX_SEARCHES: usize = 16;
const DEFAULT_MIX_BITS: u32 = 2;
const MAX_RES: i32 = 4;
const DEFAULT_NUM_UV: usize = 8;
const MIN_UV: usize = 4;
const MAX_UV: usize = 8;
/// The `pbFactor` every element is coded with (pb = 40 · 4 / 4).
const PB_FACTOR: u32 = 4;

/// Encoder settings.
///
/// `#[non_exhaustive]`: build it with [`EncoderConfig::new`] and the `with_*` methods.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    /// Sample rate in Hz, recorded in the cookie.
    pub sample_rate: u32,
    /// 1 to 8 channels.
    pub channels: u8,
    /// 16, 20, 24 or 32.
    pub bit_depth: u8,
    /// Frames per packet (default [`DEFAULT_FRAME_LENGTH`](super::DEFAULT_FRAME_LENGTH)); only the
    /// last packet may be shorter.
    pub frame_length: u32,
    /// The reference's fast mode: a stereo stream skips the matrixing and coefficient search
    /// (streams of other channel counts are encoded as without it, as in the reference).
    pub fast: bool,
}

impl EncoderConfig {
    /// `channels` channels of `bit_depth`-bit samples at `sample_rate`, 4096 frames per packet.
    pub fn new(sample_rate: u32, channels: u8, bit_depth: u8) -> Self {
        Self { sample_rate, channels, bit_depth, frame_length: super::DEFAULT_FRAME_LENGTH, fast: false }
    }

    /// Sets `frame_length`.
    pub fn with_frame_length(mut self, frame_length: u32) -> Self {
        self.frame_length = frame_length;
        self
    }

    /// Sets `fast`.
    pub fn with_fast(mut self, fast: bool) -> Self {
        self.fast = fast;
        self
    }
}

/// Per-channel predictor coefficients, one set per search width.
type SearchCoefs = [[i16; MAX_COEFS]; MAX_SEARCHES];

/// Encodes planar `i32` (see the [module docs](super)) into ALAC packets.
pub struct Encoder {
    cfg: EncoderConfig,
    bit_depth: u32,
    last_mix_res: [i32; MAX_CHANNELS as usize],
    mix_u: Vec<i32>,
    mix_v: Vec<i32>,
    pred_u: Vec<i32>,
    pred_v: Vec<i32>,
    shift_uv: Vec<u16>,
    coefs_u: Vec<SearchCoefs>,
    coefs_v: Vec<SearchCoefs>,
    total_bytes: u64,
    total_frames: u64,
    max_frame_bytes: u32,
}

impl std::fmt::Debug for Encoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encoder").field("cfg", &self.cfg).finish_non_exhaustive()
    }
}

/// The bytes of each sample the reference passes through uncompressed below the predicted part.
fn bytes_shifted(bit_depth: u32) -> u32 {
    match bit_depth {
        32 => 2,
        24.. => 1,
        _ => 0,
    }
}

/// The bytes an element shifts off in one packet: the fewest, down from the reference's
/// [`bytes_shifted`], that leave every sample at least one bit narrower than the width the
/// reference predicts (`bit_depth − 8 · bytes_shifted`). The predictor then works on values no
/// wider than in the reference, so its arithmetic stays in the same range; only fewer low bytes go
/// out uncompressed. The spare bit is there because at the full width, coding the low byte through
/// the predictor costs about as much as storing it, and for 24-bit mono measured 2% more.
/// A 32-bit pair keeps at least 1: with none, its prediction width (33 bits) exceeds the 32 the
/// format allows.
fn packet_shift(bit_depth: u32, channels: &[&[i32]]) -> u32 {
    let reference = bytes_shifted(bit_depth);
    let width = bit_depth - reference * 8 - 1;
    let floor = u32::from(bit_depth == 32 && channels.len() == 2);
    let fits = |bytes: u32| {
        let (lo, hi) = (-(1i64 << (width - 1)), (1i64 << (width - 1)) - 1);
        channels.iter().flat_map(|c| c.iter()).all(|&x| (lo..=hi).contains(&i64::from(x >> (bytes * 8))))
    };
    (floor..reference).find(|&b| fits(b)).unwrap_or(reference)
}

impl Encoder {
    /// An encoder for `cfg`.
    pub fn new(cfg: EncoderConfig) -> Result<Self> {
        if ![16, 20, 24, 32].contains(&cfg.bit_depth) {
            return Err(Error::InvalidInput("bit depth other than 16, 20, 24 or 32"));
        }
        if cfg.channels == 0 || cfg.channels > MAX_CHANNELS {
            return Err(Error::InvalidInput("channel count outside 1 to 8"));
        }
        if cfg.frame_length == 0 || cfg.frame_length > MAX_FRAME_LENGTH {
            return Err(Error::InvalidInput("frame length outside 1 to 2^20"));
        }
        let n = cfg.frame_length as usize;
        let mut coefs: SearchCoefs = [[0; MAX_COEFS]; MAX_SEARCHES];
        for c in &mut coefs {
            init_coefs(c, DENSHIFT_DEFAULT);
        }
        let channels = usize::from(cfg.channels);
        Ok(Self {
            cfg,
            bit_depth: u32::from(cfg.bit_depth),
            last_mix_res: [0; MAX_CHANNELS as usize],
            mix_u: vec![0; n],
            mix_v: vec![0; n],
            pred_u: vec![0; n],
            pred_v: vec![0; n],
            shift_uv: vec![0; 2 * n],
            coefs_u: vec![coefs; channels],
            coefs_v: vec![coefs; channels],
            total_bytes: 0,
            total_frames: 0,
            max_frame_bytes: 0,
        })
    }

    /// The settings.
    pub fn encoder_config(&self) -> &EncoderConfig {
        &self.cfg
    }

    /// The stream configuration, with the largest packet and the average bit rate of the packets
    /// encoded so far.
    pub fn config(&self) -> SpecificConfig {
        let mut c = SpecificConfig::new(self.cfg.sample_rate, self.cfg.channels, self.cfg.bit_depth, self.cfg.frame_length);
        c.max_frame_bytes = self.max_frame_bytes;
        if self.total_frames > 0 {
            let rate = u128::from(self.total_bytes) * 8 * u128::from(self.cfg.sample_rate) / u128::from(self.total_frames);
            c.avg_bit_rate = u32::try_from(rate).unwrap_or(u32::MAX);
        }
        c
    }

    /// The configuration as MP4's `alac` box and Matroska's CodecPrivate hold it:
    /// [`Self::config`] as [`SpecificConfig::to_bytes`] writes it (for CAF, use
    /// [`SpecificConfig::to_cookie`]). Write it after the last packet for the statistics to cover
    /// the whole stream.
    pub fn cookie(&self) -> Vec<u8> {
        self.config().to_bytes()
    }

    /// Encode one packet from one slice per channel, in ALAC channel order (for WAV-order audio,
    /// pass WAV channel [`alac_order`](super::alac_order)`(n)[j]` as channel `j`), each holding the same
    /// number of samples: 1 to the frame length (fewer only for the last packet). Samples must fit
    /// the bit depth.
    pub fn encode(&mut self, channels: &[&[i32]]) -> Result<Vec<u8>> {
        if channels.len() != usize::from(self.cfg.channels) {
            return Err(Error::InvalidInput("channel count differs from the configuration"));
        }
        let n = channels.first().map_or(0, |c| c.len());
        if n == 0 || n > self.cfg.frame_length as usize {
            return Err(Error::InvalidInput("packet length outside 1 to the frame length"));
        }
        if channels.iter().any(|c| c.len() != n) {
            return Err(Error::InvalidInput("channels differ in length"));
        }
        if self.bit_depth < 32 {
            let max = (1i32 << (self.bit_depth - 1)) - 1;
            if channels.iter().flat_map(|c| c.iter()).any(|&s| s > max || s < -max - 1) {
                return Err(Error::InvalidInput("a sample outside the bit depth"));
            }
        }
        let mut w = BitWriter::new();
        let mut index = 0;
        let (mut sce_tag, mut cpe_tag, mut lfe_tag) = (0u32, 0u32, 0u32);
        for &element in element_map(channels.len()) {
            w.put(element, 3);
            match element {
                ID_CPE => {
                    w.put(cpe_tag, 4);
                    cpe_tag += 1;
                    let (l, r) = (channels[index], channels[index + 1]);
                    let bs = packet_shift(self.bit_depth, &[l, r]);
                    // as in the reference, fast mode applies to stereo streams only
                    if self.cfg.fast && channels.len() == 2 {
                        self.encode_stereo_fast(&mut w, index, l, r, bs)?;
                    } else {
                        self.encode_stereo(&mut w, index, l, r, bs)?;
                    }
                    index += 2;
                }
                _ => {
                    let tag = if element == ID_LFE { &mut lfe_tag } else { &mut sce_tag };
                    w.put(*tag, 4);
                    *tag += 1;
                    let bs = packet_shift(self.bit_depth, &[channels[index]]);
                    self.encode_mono(&mut w, index, channels[index], bs)?;
                    index += 1;
                }
            }
        }
        w.put(ID_END, 3);
        let bytes = w.into_bytes();
        self.total_bytes += bytes.len() as u64;
        self.total_frames += n as u64;
        self.max_frame_bytes = self.max_frame_bytes.max(u32::try_from(bytes.len()).unwrap_or(u32::MAX));
        Ok(bytes)
    }

    fn partial(&self, n: usize) -> bool {
        n != self.cfg.frame_length as usize
    }

    /// The element header: 12 zero bits, the flags, the sample count of a partial frame.
    fn header(&self, w: &mut BitWriter, n: usize, flags: u32) {
        let partial = self.partial(n);
        w.put(0, 12);
        w.put((u32::from(partial) << 3) | flags, 4);
        if partial {
            w.put(n as u32, 32);
        }
    }

    /// `EncodeStereo`: a channel pair with the matrixing and coefficient search.
    /// `bs`: the bytes shifted off, from [`packet_shift`].
    fn encode_stereo(&mut self, w: &mut BitWriter, ch: usize, l: &[i32], r: &[i32], bs: u32) -> Result<()> {
        let start = w.bit_len();
        let n = l.len();
        let chan_bits = self.bit_depth - bs * 8 + 1;
        let partial = self.partial(n);
        let params = AgParams::new(ag::MB0, PB_FACTOR * ag::PB0 / 4, ag::KB0);

        // matrixing search over an eighth of the samples
        let mut dilate = 8;
        let mut min_bits1 = 1u64 << 31;
        let mut best_res = self.last_mix_res[ch];
        for mix_res in 0..=MAX_RES {
            let nd = n / dilate;
            let m = Mix { bits: DEFAULT_MIX_BITS, res: mix_res };
            mix((&l[..nd], &r[..nd]), (&mut self.mix_u, &mut self.mix_v), m, &mut self.shift_uv, bs);
            let cu = &mut self.coefs_u[ch][DEFAULT_NUM_UV - 1];
            pc_block(&self.mix_u, &mut self.pred_u, nd, cu, DEFAULT_NUM_UV, chan_bits, DENSHIFT_DEFAULT);
            let cv = &mut self.coefs_v[ch][DEFAULT_NUM_UV - 1];
            pc_block(&self.mix_v, &mut self.pred_v, nd, cv, DEFAULT_NUM_UV, chan_bits, DENSHIFT_DEFAULT);
            let bits = count(params, &self.pred_u[..nd], chan_bits)? + count(params, &self.pred_v[..nd], chan_bits)?;
            if bits < min_bits1 {
                min_bits1 = bits;
                best_res = mix_res;
            }
        }
        self.last_mix_res[ch] = best_res;
        let m = Mix { bits: DEFAULT_MIX_BITS, res: best_res };
        mix((l, r), (&mut self.mix_u, &mut self.mix_v), m, &mut self.shift_uv, bs);

        // predictor search: 4 or 8 coefficients, converged on a 32nd of the samples
        let (mut num_u, mut num_v) = (MIN_UV, MIN_UV);
        let (mut min_bits1, mut min_bits2) = (1u64 << 31, 1u64 << 31);
        for num_uv in (MIN_UV..=MAX_UV).step_by(4) {
            dilate = 32;
            for _ in 0..8 {
                let cu = &mut self.coefs_u[ch][num_uv - 1];
                pc_block(&self.mix_u, &mut self.pred_u, n / dilate, cu, num_uv, chan_bits, DENSHIFT_DEFAULT);
                let cv = &mut self.coefs_v[ch][num_uv - 1];
                pc_block(&self.mix_v, &mut self.pred_v, n / dilate, cv, num_uv, chan_bits, DENSHIFT_DEFAULT);
            }
            dilate = 8;
            let header = 16 * num_uv as u64;
            let bits1 = count(params, &self.pred_u[..n / dilate], chan_bits)? * dilate as u64 + header;
            if bits1 < min_bits1 {
                min_bits1 = bits1;
                num_u = num_uv;
            }
            let bits2 = count(params, &self.pred_v[..n / dilate], chan_bits)? * dilate as u64 + header;
            if bits2 < min_bits2 {
                min_bits2 = bits2;
                num_v = num_uv;
            }
        }

        // escape when the estimate is no smaller than the samples themselves
        let partial_bits = if partial { 32 } else { 0 };
        let mut min_bits = min_bits1 + min_bits2 + 64 + partial_bits;
        if bs != 0 {
            min_bits += n as u64 * u64::from(bs * 8) * 2;
        }
        let escape_bits = n as u64 * u64::from(self.bit_depth) * 2 + partial_bits + 16;
        let mut escape = min_bits >= escape_bits;
        if !escape {
            self.header(w, n, bs << 1);
            w.put(DEFAULT_MIX_BITS, 8);
            w.put(best_res as u32, 8);
            for (coefs, num) in [(&self.coefs_u[ch][num_u - 1], num_u), (&self.coefs_v[ch][num_v - 1], num_v)] {
                w.put(DENSHIFT_DEFAULT, 8); // mode 0
                w.put((PB_FACTOR << 5) | num as u32, 8);
                for &c in &coefs[..num] {
                    w.put(u32::from(c as u16), 16);
                }
            }
            if bs != 0 {
                let shift = bs * 8;
                for pair in self.shift_uv[..2 * n].as_chunks::<2>().0 {
                    w.put((u32::from(pair[0]) << shift) | u32::from(pair[1]), shift * 2);
                }
            }
            let cu = &mut self.coefs_u[ch][num_u - 1];
            pc_block(&self.mix_u, &mut self.pred_u, n, cu, num_u, chan_bits, DENSHIFT_DEFAULT);
            ag::compress(params, &self.pred_u[..n], w, chan_bits)?;
            let cv = &mut self.coefs_v[ch][num_v - 1];
            pc_block(&self.mix_v, &mut self.pred_v, n, cv, num_v, chan_bits, DENSHIFT_DEFAULT);
            ag::compress(params, &self.pred_v[..n], w, chan_bits)?;
            if (w.bit_len() - start) as u64 >= escape_bits {
                w.truncate(start);
                escape = true;
            }
        }
        if escape {
            self.encode_stereo_escape(w, l, r);
        }
        Ok(())
    }

    /// `EncodeStereoFast`: a channel pair without matrixing or search.
    fn encode_stereo_fast(&mut self, w: &mut BitWriter, ch: usize, l: &[i32], r: &[i32], bs: u32) -> Result<()> {
        let start = w.bit_len();
        let n = l.len();
        let chan_bits = self.bit_depth - bs * 8 + 1;
        let partial = self.partial(n);
        let params = AgParams::new(ag::MB0, PB_FACTOR * ag::PB0 / 4, ag::KB0);
        let num = DEFAULT_NUM_UV;
        let m = Mix { bits: DEFAULT_MIX_BITS, res: 0 };
        mix((l, r), (&mut self.mix_u, &mut self.mix_v), m, &mut self.shift_uv, bs);
        // written speculatively, rewound for an escape
        self.header(w, n, bs << 1);
        w.put(DEFAULT_MIX_BITS, 8);
        w.put(0, 8);
        for coefs in [&self.coefs_u[ch][num - 1], &self.coefs_v[ch][num - 1]] {
            w.put(DENSHIFT_DEFAULT, 8);
            w.put((PB_FACTOR << 5) | num as u32, 8);
            for &c in &coefs[..num] {
                w.put(u32::from(c as u16), 16);
            }
        }
        if bs != 0 {
            let shift = bs * 8;
            for pair in self.shift_uv[..2 * n].as_chunks::<2>().0 {
                w.put((u32::from(pair[0]) << shift) | u32::from(pair[1]), shift * 2);
            }
        }
        let cu = &mut self.coefs_u[ch][num - 1];
        pc_block(&self.mix_u, &mut self.pred_u, n, cu, num, chan_bits, DENSHIFT_DEFAULT);
        let before = w.bit_len();
        ag::compress(params, &self.pred_u[..n], w, chan_bits)?;
        let bits1 = (w.bit_len() - before) as u64;
        let cv = &mut self.coefs_v[ch][num - 1];
        pc_block(&self.mix_v, &mut self.pred_v, n, cv, num, chan_bits, DENSHIFT_DEFAULT);
        let before = w.bit_len();
        ag::compress(params, &self.pred_v[..n], w, chan_bits)?;
        let bits2 = (w.bit_len() - before) as u64;

        let partial_bits = if partial { 32 } else { 0 };
        let mut min_bits = bits1 + bits2 + 2 * 16 * num as u64 + 64 + partial_bits;
        if bs != 0 {
            min_bits += n as u64 * u64::from(bs * 8) * 2;
        }
        let escape_bits = n as u64 * u64::from(self.bit_depth) * 2 + partial_bits + 16;
        if min_bits >= escape_bits || (w.bit_len() - start) as u64 >= escape_bits {
            w.truncate(start);
            self.encode_stereo_escape(w, l, r);
        }
        Ok(())
    }

    /// `EncodeStereoEscape`: a channel pair uncompressed, L R interleaved.
    fn encode_stereo_escape(&mut self, w: &mut BitWriter, l: &[i32], r: &[i32]) {
        let n = l.len();
        self.header(w, n, 1);
        for (&a, &b) in l.iter().zip(r) {
            w.put(a as u32, self.bit_depth);
            w.put(b as u32, self.bit_depth);
        }
        // the reference de-interleaves 20- and 24-bit input through the mix buffers
        if matches!(self.bit_depth, 20 | 24) {
            self.mix_u[..n].copy_from_slice(l);
            self.mix_v[..n].copy_from_slice(r);
        }
    }

    /// `EncodeMono`: one channel (SCE or LFE).
    fn encode_mono(&mut self, w: &mut BitWriter, ch: usize, input: &[i32], bs: u32) -> Result<()> {
        let start = w.bit_len();
        let n = input.len();
        let shift = bs * 8;
        let chan_bits = self.bit_depth - shift;
        let partial = self.partial(n);
        let params = AgParams::new(ag::MB0, PB_FACTOR * ag::PB0 / 4, ag::KB0);
        shift_mono(input, &mut self.mix_u, &mut self.shift_uv, n, bs);

        let mut min_bits = 1u64 << 31;
        let mut best_u = MIN_UV;
        for num_u in (MIN_UV..=MAX_UV).step_by(4) {
            let coefs = &mut self.coefs_u[ch][num_u - 1];
            for _ in 0..7 {
                pc_block(&self.mix_u, &mut self.pred_u, n / 32, coefs, num_u, chan_bits, DENSHIFT_DEFAULT);
            }
            pc_block(&self.mix_u, &mut self.pred_u, n / 8, coefs, num_u, chan_bits, DENSHIFT_DEFAULT);
            let bits = 8 * count(params, &self.pred_u[..n / 8], chan_bits)? + 16 * num_u as u64;
            if bits < min_bits {
                best_u = num_u;
                min_bits = bits;
            }
        }
        let partial_bits = if partial { 32 } else { 0 };
        min_bits += 32 + partial_bits;
        if bs != 0 {
            min_bits += n as u64 * u64::from(shift);
        }
        let escape_bits = n as u64 * u64::from(self.bit_depth) + partial_bits + 16;
        let mut escape = min_bits >= escape_bits;
        if !escape {
            self.header(w, n, bs << 1);
            w.put(0, 16); // mixBits = mixRes = 0
            w.put(DENSHIFT_DEFAULT, 8); // mode 0
            w.put((PB_FACTOR << 5) | best_u as u32, 8);
            for &c in &self.coefs_u[ch][best_u - 1][..best_u] {
                w.put(u32::from(c as u16), 16);
            }
            if bs != 0 {
                for &s in &self.shift_uv[..n] {
                    w.put(u32::from(s), shift);
                }
            }
            let coefs = &mut self.coefs_u[ch][best_u - 1];
            pc_block(&self.mix_u, &mut self.pred_u, n, coefs, best_u, chan_bits, DENSHIFT_DEFAULT);
            ag::compress(AgParams::standard(), &self.pred_u[..n], w, chan_bits)?;
            if (w.bit_len() - start) as u64 >= escape_bits {
                w.truncate(start);
                escape = true;
            }
        }
        if escape {
            self.header(w, n, 1);
            for &s in input {
                w.put(s as u32, self.bit_depth);
            }
            // the reference converts 20- and 24-bit input through the mix buffer
            if matches!(self.bit_depth, 20 | 24) {
                self.mix_u[..n].copy_from_slice(input);
            }
        }
        Ok(())
    }
}

/// Bits the residuals `pc` code to.
fn count(params: AgParams, pc: &[i32], bits: u32) -> Result<u64> {
    let mut c = BitCounter::default();
    ag::compress(params, pc, &mut c, bits)?;
    Ok(c.bit_len() as u64)
}

// The round trips need the decoder.
#[cfg(test)]
#[cfg(feature = "alac-decode")]
mod tests {
    use super::super::Decoder;
    use super::*;

    fn signal(n: usize, ch: usize, bits: u32) -> Vec<Vec<i32>> {
        let amp = ((1i64 << (bits - 1)) - 1) as f64 * 0.8;
        (0..ch)
            .map(|c| {
                (0..n)
                    .map(|i| {
                        let t = i as f64 / 44_100.0;
                        let x =
                            (t * (220.0 + 110.0 * c as f64) * std::f64::consts::TAU).sin() * 0.7 + ((i * 7919 + c * 31) % 101) as f64 / 400.0 - 0.125;
                        (x * amp) as i32
                    })
                    .collect()
            })
            .collect()
    }

    fn roundtrip(cfg: EncoderConfig, frames: usize) -> usize {
        let input = signal(frames, usize::from(cfg.channels), u32::from(cfg.bit_depth));
        let mut enc = Encoder::new(cfg).unwrap();
        let mut packets = Vec::new();
        let fl = cfg.frame_length as usize;
        for start in (0..frames).step_by(fl) {
            let end = (start + fl).min(frames);
            let chunk: Vec<&[i32]> = input.iter().map(|c| &c[start..end]).collect();
            packets.push(enc.encode(&chunk).unwrap());
        }
        let mut dec = Decoder::new(&enc.cookie()).unwrap();
        let mut out = vec![Vec::new(); input.len()];
        for p in &packets {
            for (o, c) in out.iter_mut().zip(dec.decode(p).unwrap()) {
                o.extend(c);
            }
        }
        assert_eq!(out, input, "{cfg:?}");
        assert_eq!(enc.config().max_frame_bytes as usize, packets.iter().map(Vec::len).max().unwrap());
        packets.iter().map(Vec::len).sum()
    }

    #[test]
    fn every_depth_and_channel_count_roundtrips() {
        for bits in [16, 20, 24, 32] {
            for ch in 1..=8 {
                for fast in [false, true] {
                    let cfg = EncoderConfig::new(44_100, ch, bits).with_fast(fast).with_frame_length(1024);
                    roundtrip(cfg, 3000);
                }
            }
        }
    }

    #[test]
    fn short_and_tiny_packets_roundtrip() {
        for n in [1, 2, 5, 9, 31, 33, 100] {
            roundtrip(EncoderConfig::new(48_000, 2, 16).with_frame_length(4096), n);
            roundtrip(EncoderConfig::new(48_000, 1, 24).with_frame_length(4096), n);
        }
        roundtrip(EncoderConfig::new(48_000, 2, 24).with_frame_length(3), 50);
    }

    #[test]
    fn compresses_tonal_audio_and_escapes_noise() {
        // two tones, one per channel, at half scale: well under half the PCM size
        let tones: Vec<Vec<i32>> = [440.0, 660.0]
            .iter()
            .map(|f| (0..44_100).map(|i| ((i as f64 * f / 44_100.0 * std::f64::consts::TAU).sin() * 16_000.0) as i32).collect())
            .collect();
        let mut enc = Encoder::new(EncoderConfig::new(44_100, 2, 16)).unwrap();
        let mut bytes = 0;
        for start in (0..44_100).step_by(4096) {
            let end = (start + 4096).min(44_100);
            bytes += enc.encode(&[&tones[0][start..end], &tones[1][start..end]]).unwrap().len();
        }
        assert!(bytes < 44_100 * 4 / 2, "{bytes} bytes");
        // white noise at full scale does not compress: escaped frames, a few bytes over raw
        let mut seed = 1u32;
        let noise: Vec<i32> = (0..8192)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 16) as i16 as i32
            })
            .collect();
        let mut enc = Encoder::new(EncoderConfig::new(44_100, 1, 16)).unwrap();
        let p = enc.encode(&[&noise[..4096]]).unwrap();
        assert!(p.len() <= 4096 * 2 + 4, "{}", p.len());
        let mut dec = Decoder::new(&enc.cookie()).unwrap();
        assert_eq!(dec.decode(&p).unwrap(), vec![noise[..4096].to_vec()]);
    }

    /// One packet of `frames` frames per element, encoded and decoded back; its size in bytes.
    fn packet_bytes(cfg: EncoderConfig, input: &[Vec<i32>]) -> usize {
        let mut enc = Encoder::new(cfg).unwrap();
        let chunk: Vec<&[i32]> = input.iter().map(Vec::as_slice).collect();
        let p = enc.encode(&chunk).unwrap();
        let mut dec = Decoder::new(&enc.cookie()).unwrap();
        assert_eq!(dec.decode(&p).unwrap(), input, "{cfg:?}");
        p.len()
    }

    /// The reference shifts 1 (24-bit) or 2 (32-bit) low bytes of every sample off and stores them
    /// uncompressed, so a 4096-frame stereo packet of 24-bit silence was 8224 bytes (#1368).
    #[test]
    fn silence_is_a_few_bytes_at_every_depth() {
        for bits in [16, 20, 24, 32] {
            for ch in [1, 2, 6] {
                for fast in [false, true] {
                    let cfg = EncoderConfig::new(48_000, ch, bits).with_fast(fast);
                    let bytes = packet_bytes(cfg, &vec![vec![0; 4096]; usize::from(ch)]);
                    // a 32-bit pair keeps one shifted byte per sample: the format allows no less
                    let floor = if bits == 32 { 4096 * 2 * (usize::from(ch) / 2) } else { 0 };
                    assert!(bytes <= floor + 40 * usize::from(ch), "{bits} bits, {ch} ch, fast {fast}: {bytes} bytes");
                }
            }
        }
    }

    /// Quiet audio (within 12 bits) at 24 and 32 bits shifts nothing off and comes within 10% of
    /// the same audio at 16 bits (the wider prediction width makes the coder's escapes longer);
    /// the reference stored a byte or two more per sample. Samples at the edges of each width and
    /// loud audio decode exactly.
    #[test]
    fn quiet_packets_compress_like_16_bit_audio_and_loud_ones_round_trip() {
        let quiet = signal(4096, 2, 12);
        let at16 = packet_bytes(EncoderConfig::new(48_000, 2, 16), &quiet);
        let at24 = packet_bytes(EncoderConfig::new(48_000, 2, 24), &quiet);
        assert!(at24 * 10 <= at16 * 11, "24-bit {at24} bytes, 16-bit {at16}");
        let mono = vec![quiet[0].clone()];
        let at16 = packet_bytes(EncoderConfig::new(48_000, 1, 16), &mono);
        let at32 = packet_bytes(EncoderConfig::new(48_000, 1, 32), &mono);
        assert!(at32 * 10 <= at16 * 11, "32-bit mono {at32} bytes, 16-bit {at16}");
        // the samples just past each narrower width, and loud audio, decode exactly
        for bits in [24u32, 32] {
            for ch in [1u8, 2] {
                let edge: Vec<Vec<i32>> = (0..ch)
                    .map(|c| {
                        (0..4096)
                            .map(|i| match (i + usize::from(c)) % 4 {
                                0 => (16_384 << (bits - 24)) - 1,
                                1 => -(16_384 << (bits - 24)),
                                2 => 16_384 << (bits - 24),
                                _ => -(16_384 << (bits - 24)) - 1,
                            })
                            .collect()
                    })
                    .collect();
                packet_bytes(EncoderConfig::new(48_000, ch, bits as u8), &edge);
                packet_bytes(EncoderConfig::new(48_000, ch, bits as u8), &signal(4096, usize::from(ch), bits));
            }
        }
    }

    #[test]
    fn the_shift_is_the_fewest_bytes_that_keep_the_reference_width() {
        let z = [0i32; 4];
        assert_eq!(packet_shift(16, &[&z]), 0);
        assert_eq!(packet_shift(24, &[&z, &z]), 0);
        assert_eq!(packet_shift(24, &[&[16_383, -16_384], &z]), 0);
        assert_eq!(packet_shift(24, &[&[16_384], &z]), 1);
        assert_eq!(packet_shift(24, &[&[-16_385]]), 1);
        assert_eq!(packet_shift(24, &[&[32_767]]), 1);
        assert_eq!(packet_shift(32, &[&z]), 0);
        assert_eq!(packet_shift(32, &[&[16_383]]), 0);
        assert_eq!(packet_shift(32, &[&z, &z]), 1);
        assert_eq!(packet_shift(32, &[&[1 << 21]]), 1);
        assert_eq!(packet_shift(32, &[&[1 << 22]]), 2);
        assert_eq!(packet_shift(32, &[&[i32::MIN]]), 2);
        assert_eq!(packet_shift(32, &[&[i32::MAX], &z]), 2);
    }
}
