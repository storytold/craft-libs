//! The Layer II encoder; the module docs are in `mpa/mod.rs`.

// Index loops follow the specification's pseudo-code (FilmCraft allows this lint workspace-wide).
#![allow(clippy::needless_range_loop)]

use craft_media::bitstream::BitWriter;

use super::tables::{self, AllocTable, SUBBANDS};
use crate::mp3::filterbank::Polyphase;
use crate::mp3::tables::ath_db;

/// Settings an [`Encoder`] refuses.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EncodeError {
    /// A sample rate outside [`SAMPLE_RATES`].
    #[error("MP2: unsupported sample rate {0} Hz (32, 44.1, 48 kHz, or 16, 22.05, 24 kHz)")]
    SampleRate(u32),
    /// A bit rate that is not one of [`bitrates`] for the sample rate and channel count.
    #[error("MP2: unsupported bit rate {kbps} kbit/s at {rate} Hz with {channels} channel(s)")]
    Bitrate {
        /// The bit rate asked for, kbit/s.
        kbps: u32,
        /// The sample rate, Hz.
        rate: u32,
        /// The channel count.
        channels: usize,
    },
    /// More than two channels, or none.
    #[error("MP2: unsupported channel count {0} (1 or 2)")]
    Channels(usize),
}

/// Result with this module's [`EncodeError`].
pub type EncodeResult<T> = std::result::Result<T, EncodeError>;

/// Encoder settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// One of [`SAMPLE_RATES`].
    pub sample_rate: u32,
    /// 1 or 2. Two channels are coded as stereo (`mode` 00), each with its own allocation.
    pub channels: usize,
    /// One of [`bitrates`] for the sample rate and channel count, kbit/s.
    pub bitrate_kbps: u32,
}

impl Config {
    /// `channels` at `sample_rate`, constant `bitrate_kbps`.
    pub fn cbr(sample_rate: u32, channels: usize, bitrate_kbps: u32) -> Config {
        Config { sample_rate, channels, bitrate_kbps }
    }
}

/// The sample rates the encoder accepts.
pub const SAMPLE_RATES: [u32; 6] = [32_000, 44_100, 48_000, 16_000, 22_050, 24_000];

/// Samples by which a standard decoder's output lags the input: the polyphase analysis and
/// synthesis filters.
pub const DELAY: u32 = 481;

/// Samples per channel in a frame.
pub const FRAME_SAMPLES: usize = 1152;

/// Bit rates ISO/IEC 11172-3 Table 3-B.2 allows for a single channel at 32 / 44.1 / 48 kHz.
const MPEG1_MONO: [u32; 10] = [32, 48, 56, 64, 80, 96, 112, 128, 160, 192];
/// Bit rates it allows for two channels.
const MPEG1_STEREO: [u32; 10] = [64, 96, 112, 128, 160, 192, 224, 256, 320, 384];

/// Whether `rate` is one of the MPEG-2 lower sampling frequencies; `None` if it is unsupported.
fn is_lsf(rate: u32) -> Option<bool> {
    match rate {
        32_000 | 44_100 | 48_000 => Some(false),
        16_000 | 22_050 | 24_000 => Some(true),
        _ => None,
    }
}

/// The bit rates (kbit/s) a Layer II stream can use at `sample_rate` with `channels` channels:
/// at 32 / 44.1 / 48 kHz the combinations ISO/IEC 11172-3 allows (32-192 for one channel, 64-384
/// for two), at the lower sampling frequencies 8-160 for either. Empty for an unsupported rate.
pub fn bitrates(sample_rate: u32, channels: usize) -> &'static [u32] {
    match is_lsf(sample_rate) {
        Some(false) if channels == 1 => &MPEG1_MONO,
        Some(false) => &MPEG1_STEREO,
        Some(true) => &tables::BITRATES_LSF_L23[1..],
        None => &[],
    }
}

/// The highest bit rate at `sample_rate` and `channels` not above `kbps` (the lowest one when
/// `kbps` is below all of them); `None` for an unsupported rate.
pub fn nearest_bitrate(sample_rate: u32, channels: usize, kbps: u32) -> Option<u32> {
    let rates = bitrates(sample_rate, channels);
    rates.iter().rev().find(|&&r| r <= kbps).or(rates.first()).copied()
}

/// SNR-free noise model: the power of the error of rounding a value in ±1 to one of `steps`
/// levels spaced `2 / steps` apart, is `(2 / steps)² / 12`.
fn quantization_noise(steps: u16) -> f32 {
    let d = 2.0 / f32::from(steps);
    d * d / 12.0
}

/// Index of the scalefactor for a peak of `peak` (Table 3-B.1): the smallest scalefactor that is
/// at least `peak`, i.e. the largest index.
fn scalefactor_index(peak: f32) -> u32 {
    if peak.is_nan() || peak <= 0.0 {
        return 62;
    }
    let mut i = (3.0 * (1.0 - f64::from(peak).log2())).floor().clamp(0.0, 62.0) as u32;
    while i > 0 && tables::scalefactor(i) < peak {
        i -= 1;
    }
    while i < 62 && tables::scalefactor(i + 1) >= peak {
        i += 1;
    }
    i
}

/// Which scalefactors of a subband's three parts are sent (`scfsi`, §2.4.1.6) and what they are.
fn share(sf: [u32; 3]) -> (u32, [u32; 3]) {
    let near = |a: u32, b: u32| a.abs_diff(b) <= 1;
    let [a, b, c] = sf;
    if near(a, b) && near(b, c) && near(a, c) {
        let m = a.min(b).min(c);
        (2, [m, m, m])
    } else if near(a, b) {
        let m = a.min(b);
        (1, [m, m, c])
    } else if near(b, c) {
        let m = b.min(c);
        (3, [a, m, m])
    } else {
        (0, sf)
    }
}

/// Scalefactors sent for an `scfsi` class.
fn scalefactors_sent(scfsi: u32) -> u64 {
    match scfsi {
        0 => 3,
        2 => 1,
        _ => 2,
    }
}

/// Bits of the 12 granules of samples in a subband with `steps` quantization steps.
fn sample_bits(steps: u16) -> u64 {
    12 * match tables::grouping(steps) {
        Some(bits) => u64::from(bits),
        None => 3 * u64::from((u32::from(steps) + 1).trailing_zeros()),
    }
}

/// What the encoder decided for one channel of a frame.
#[derive(Clone)]
struct ChannelPlan {
    /// The allocation value (index into the table's row) per subband.
    alloc: [usize; SUBBANDS],
    /// `scfsi` per subband.
    scfsi: [u32; SUBBANDS],
    /// The scalefactor index of each part of each subband, after sharing.
    sf: [[u32; 3]; SUBBANDS],
    /// Mean square of each subband's samples.
    energy: [f32; SUBBANDS],
}

/// A Layer II encoder. Feed planar `f32` samples (full scale ±1) to [`Encoder::encode`]; it
/// returns whole frames as they are completed. [`Encoder::flush`] pads the end with silence and
/// returns the rest.
///
/// ```
/// use craft_audio::mpa::encoder::{Config, DELAY, Encoder};
///
/// let left: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
/// let right = left.clone();
/// let mut enc = Encoder::new(Config::cbr(48_000, 2, 192))?;
/// let mut mp2 = enc.encode(&[&left, &right]); // whole frames as they complete
/// mp2.extend(enc.flush()); // pads with silence to the last input sample
/// assert_eq!(mp2.len() % 576, 0); // 192 kbit/s at 48 kHz: 576 bytes per frame
/// assert_eq!(DELAY, 481); // a decoder's output lags the input by this many samples
/// # Ok::<(), craft_audio::mpa::encoder::EncodeError>(())
/// ```
pub struct Encoder {
    cfg: Config,
    lsf: bool,
    sr_index: u32,
    br_index: u32,
    table: AllocTable,
    sblimit: usize,
    /// The masking offset floor per subband (power), from the threshold in quiet.
    ath: [f32; SUBBANDS],
    poly: Vec<Polyphase>,
    /// Buffered input per channel, and how much of it is consumed.
    input: Vec<Vec<f32>>,
    consumed: usize,
    /// Remainder accumulator for the padding slot (44.1 / 22.05 kHz).
    pad_acc: u64,
    samples_in: u64,
    frames: u64,
}

impl Encoder {
    /// An encoder for `cfg`, or the setting it cannot carry.
    pub fn new(cfg: Config) -> EncodeResult<Encoder> {
        let lsf = is_lsf(cfg.sample_rate).ok_or(EncodeError::SampleRate(cfg.sample_rate))?;
        if !(1..=2).contains(&cfg.channels) {
            return Err(EncodeError::Channels(cfg.channels));
        }
        let bad = EncodeError::Bitrate { kbps: cfg.bitrate_kbps, rate: cfg.sample_rate, channels: cfg.channels };
        if !bitrates(cfg.sample_rate, cfg.channels).contains(&cfg.bitrate_kbps) {
            return Err(bad);
        }
        let (rates, bitrate_table): (&[u32; 3], &[u32; 15]) =
            if lsf { (&[22_050, 24_000, 16_000], &tables::BITRATES_LSF_L23) } else { (&tables::SAMPLE_RATES, &tables::BITRATES_MPEG1_L2) };
        let sr_index = rates.iter().position(|&r| r == cfg.sample_rate).ok_or(EncodeError::SampleRate(cfg.sample_rate))?;
        let br_index = bitrate_table.iter().position(|&b| b == cfg.bitrate_kbps).ok_or(bad)?;
        let per_channel = cfg.bitrate_kbps / cfg.channels as u32;
        let table = AllocTable::select(lsf, cfg.sample_rate, per_channel);
        let sblimit = table.sblimit().min(SUBBANDS);
        let mut ath = [0f32; SUBBANDS];
        for (sb, a) in ath.iter_mut().enumerate() {
            let centre = (sb as f64 + 0.5) * f64::from(cfg.sample_rate) / 64.0;
            // a full-scale sine has a mean square of 1/2 and plays back at about 96 dB SPL
            *a = (0.5 * 10f64.powf((ath_db(centre) - 96.0) / 10.0)) as f32;
        }
        Ok(Encoder {
            cfg,
            lsf,
            sr_index: sr_index as u32,
            br_index: br_index as u32,
            table,
            sblimit,
            ath,
            poly: vec![Polyphase::new(); cfg.channels],
            input: vec![Vec::new(); cfg.channels],
            consumed: 0,
            pad_acc: 0,
            samples_in: 0,
            frames: 0,
        })
    }

    /// The settings.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Samples per channel in a frame: 1152.
    pub fn frame_samples(&self) -> usize {
        FRAME_SAMPLES
    }

    /// Samples by which a standard decoder's output lags the input.
    pub fn delay(&self) -> u32 {
        DELAY
    }

    /// Frames written so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Encode planar samples (one slice per channel; a missing channel is silence, extra ones
    /// are ignored, every channel takes the first channel's length). Returns the completed frames.
    pub fn encode(&mut self, planar: &[&[f32]]) -> Vec<u8> {
        let n = planar.first().map_or(0, |p| p.len());
        for (c, buf) in self.input.iter_mut().enumerate() {
            let src = planar.get(c).copied().unwrap_or(&[]);
            buf.extend((0..n).map(|i| src.get(i).copied().filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-4.0, 4.0)));
        }
        self.samples_in = self.samples_in.saturating_add(n as u64);
        let mut out = Vec::new();
        self.drain(&mut out);
        out
    }

    /// Encode the buffered rest padded with silence, plus enough silence that a decoder outputs
    /// every input sample, and return the remaining frames.
    pub fn flush(&mut self) -> Vec<u8> {
        let fs = FRAME_SAMPLES as u64;
        let needed = if self.samples_in == 0 { self.frames } else { self.samples_in.saturating_add(u64::from(DELAY)).div_ceil(fs) };
        let have = self.input.first().map_or(0, |b| b.len().saturating_sub(self.consumed));
        let want = (needed.saturating_sub(self.frames) as usize).saturating_mul(FRAME_SAMPLES);
        if want > have {
            for buf in &mut self.input {
                buf.resize(self.consumed + want, 0.0);
            }
        }
        let mut out = Vec::new();
        self.drain(&mut out);
        self.input.iter_mut().for_each(Vec::clear);
        self.consumed = 0;
        out
    }

    /// Encode every whole frame of buffered input.
    fn drain(&mut self, out: &mut Vec<u8>) {
        while self.input.first().is_some_and(|b| b.len() - self.consumed >= FRAME_SAMPLES) {
            let mut sub = vec![[[0f32; SUBBANDS]; 36]; self.cfg.channels];
            for (ch, slots) in sub.iter_mut().enumerate() {
                let (Some(buf), Some(poly)) = (self.input.get(ch), self.poly.get_mut(ch)) else { continue };
                let frame = buf.get(self.consumed..self.consumed + FRAME_SAMPLES).unwrap_or(&[]);
                for (slot, chunk) in slots.iter_mut().zip(frame.as_chunks::<32>().0) {
                    poly.filter(chunk, slot);
                }
            }
            self.consumed += FRAME_SAMPLES;
            self.write_frame(&sub, out);
            self.frames += 1;
        }
        // keep the buffer small
        if self.consumed >= 1 << 16 {
            for buf in &mut self.input {
                buf.drain(..self.consumed);
            }
            self.consumed = 0;
        }
    }

    /// The scalefactors and energies of a channel's 36 time slots.
    fn analyse(&self, slots: &[[f32; SUBBANDS]; 36]) -> ChannelPlan {
        let mut plan = ChannelPlan { alloc: [0; SUBBANDS], scfsi: [0; SUBBANDS], sf: [[62; 3]; SUBBANDS], energy: [0.0; SUBBANDS] };
        for sb in 0..self.sblimit {
            let mut sf = [62u32; 3];
            for (part, s) in sf.iter_mut().enumerate() {
                let peak = slots[12 * part..12 * part + 12].iter().map(|slot| slot[sb].abs()).fold(0f32, f32::max);
                *s = scalefactor_index(peak);
            }
            (plan.scfsi[sb], plan.sf[sb]) = share(sf);
            plan.energy[sb] = slots.iter().map(|slot| slot[sb] * slot[sb]).sum::<f32>() / 36.0;
        }
        plan
    }

    /// Choose the allocation of every subband of every channel with the bits the frame has.
    fn allocate(&self, plans: &mut [ChannelPlan], mut bits_left: i64) {
        let nch = plans.len();
        // power the quantization noise of each subband may have before it is audible: a spread
        // of the band energies, never below the threshold in quiet
        let mut mask = vec![[0f32; SUBBANDS]; nch];
        for (m, plan) in mask.iter_mut().zip(plans.iter()) {
            for sb in 0..self.sblimit {
                let mut level = self.ath[sb];
                for j in 0..self.sblimit {
                    let atten_db = 14.0 + 20.0 * sb.abs_diff(j) as f32;
                    level = level.max(plan.energy[j] * 10f32.powf(-atten_db / 10.0));
                }
                m[sb] = level;
            }
        }
        // the fixed cost: every subband's allocation field
        for sb in 0..self.sblimit {
            bits_left -= (nch * self.table.row(sb).len().trailing_zeros() as usize) as i64;
        }
        // the noise a subband has with allocation `a`: dropped, or the mean over its parts
        let noise = |plan: &ChannelPlan, sb: usize, a: usize| -> f32 {
            let row = self.table.row(sb);
            let steps = row.get(a).copied().unwrap_or(0);
            if steps == 0 {
                return plan.energy[sb];
            }
            let mean_sf2 = plan.sf[sb].iter().map(|&i| tables::scalefactor(i).powi(2)).sum::<f32>() / 3.0;
            (quantization_noise(steps) * mean_sf2).min(plan.energy[sb])
        };
        let cost = |plan: &ChannelPlan, sb: usize, a: usize| -> i64 {
            let row = self.table.row(sb);
            let step = |a: usize| row.get(a).copied().unwrap_or(0);
            let (from, to) = (step(a), step(a + 1));
            let mut bits = sample_bits(to) as i64 - if from == 0 { 0 } else { sample_bits(from) as i64 };
            if from == 0 {
                bits += 2 + 6 * scalefactors_sent(plan.scfsi[sb]) as i64;
            }
            bits
        };
        let mut blocked = vec![[false; SUBBANDS]; nch];
        loop {
            // the subband whose noise is furthest above its mask
            let mut best: Option<(usize, usize, f32)> = None;
            for (ch, plan) in plans.iter().enumerate() {
                for sb in 0..self.sblimit {
                    let a = plan.alloc[sb];
                    if a + 1 >= self.table.row(sb).len() || blocked[ch][sb] || plan.energy[sb] <= 0.0 {
                        continue;
                    }
                    let ratio = noise(plan, sb, a) / mask[ch][sb].max(1e-20);
                    if ratio > 1e-3 && best.is_none_or(|(_, _, r)| ratio > r) {
                        best = Some((ch, sb, ratio));
                    }
                }
            }
            let Some((ch, sb, _)) = best else { break };
            let plan = &mut plans[ch];
            let c = cost(plan, sb, plan.alloc[sb]);
            if c <= bits_left {
                bits_left -= c;
                plan.alloc[sb] += 1;
            } else {
                blocked[ch][sb] = true;
            }
        }
    }

    fn write_frame(&mut self, sub: &[[[f32; SUBBANDS]; 36]], out: &mut Vec<u8>) {
        let nch = self.cfg.channels;
        // frame length: 144 bytes per kbit/s over the sampling rate, plus a slot when the
        // remainder has added up to one
        let rate = u64::from(self.cfg.sample_rate);
        let exact = 144 * u64::from(self.cfg.bitrate_kbps) * 1000;
        self.pad_acc += exact % rate;
        let pad = self.pad_acc >= rate;
        if pad {
            self.pad_acc -= rate;
        }
        let frame_bytes = (exact / rate) as usize + usize::from(pad);

        let mut plans: Vec<ChannelPlan> = sub.iter().map(|s| self.analyse(s)).collect();
        self.allocate(&mut plans, frame_bytes as i64 * 8 - 32);

        let mut w = BitWriter::new();
        w.write_bits(0xFFF, 12);
        w.write_bits(u32::from(!self.lsf), 1);
        w.write_bits(0b10, 2); // Layer II
        w.write_bits(1, 1); // no CRC
        w.write_bits(self.br_index, 4);
        w.write_bits(self.sr_index, 2);
        w.write_bits(u32::from(pad), 1);
        w.write_bits(0, 1); // private
        w.write_bits(if nch == 1 { 3 } else { 0 }, 2); // mono or stereo
        w.write_bits(0, 2); // mode extension
        w.write_bits(0, 1); // copyright
        w.write_bits(1, 1); // original
        w.write_bits(0, 2); // emphasis
        for sb in 0..self.sblimit {
            let nbal = self.table.row(sb).len().trailing_zeros();
            for plan in &plans {
                w.write_bits(plan.alloc[sb] as u32, nbal);
            }
        }
        for sb in 0..self.sblimit {
            for plan in &plans {
                if plan.alloc[sb] != 0 {
                    w.write_bits(plan.scfsi[sb], 2);
                }
            }
        }
        for sb in 0..self.sblimit {
            for plan in &plans {
                if plan.alloc[sb] == 0 {
                    continue;
                }
                let [a, b, c] = plan.sf[sb];
                match plan.scfsi[sb] {
                    0 => [a, b, c].iter().for_each(|&s| w.write_bits(s, 6)),
                    1 => [a, c].iter().for_each(|&s| w.write_bits(s, 6)),
                    2 => w.write_bits(a, 6),
                    _ => [a, b].iter().for_each(|&s| w.write_bits(s, 6)),
                }
            }
        }
        for gr in 0..12 {
            let part = gr / 4;
            for sb in 0..self.sblimit {
                for (ch, plan) in plans.iter().enumerate() {
                    let a = plan.alloc[sb];
                    let steps = self.table.row(sb).get(a).copied().unwrap_or(0);
                    if steps == 0 {
                        continue;
                    }
                    let scale = tables::scalefactor(plan.sf[sb][part]);
                    let n = u32::from(steps);
                    let code = |k: usize| -> u32 {
                        let x = (sub[ch][3 * gr + k][sb] / scale).clamp(-1.0, 1.0);
                        // the reconstruction points are (2v + 1 - n) / n
                        ((x * n as f32 + n as f32 - 1.0) / 2.0).round().clamp(0.0, (n - 1) as f32) as u32
                    };
                    let v = [code(0), code(1), code(2)];
                    match tables::grouping(steps) {
                        Some(bits) => w.write_bits(v[0] + n * v[1] + n * n * v[2], bits),
                        None => {
                            let nb = (n + 1).trailing_zeros();
                            v.iter().for_each(|&x| w.write_bits(x, nb));
                        }
                    }
                }
            }
        }
        let mut bytes = w.finish();
        bytes.resize(frame_bytes, 0);
        out.extend_from_slice(&bytes);
    }
}
