//! Spectral band replication (HE-AAC, ISO/IEC 14496-3 §4.6.18): rebuilds the band above the
//! AAC-LC core from the core's own low band and the envelope data of the stream's SBR fill
//! elements, at twice the core rate.
//!
//! Each channel's core output goes through a 32-band QMF analysis bank. The HF generator copies
//! (patches) low band channels into the SBR range, inverse filtered; the HF adjuster scales them
//! to the transmitted envelopes and adds noise and sinusoids; a 64-band QMF synthesis bank merges
//! the delayed low band and the new high band. A frame without usable SBR data (none sent yet, or
//! damaged) still passes through the banks, so the output keeps its rate and delay.
//!
//! Written from the SBR description and tables of ETSI ES 201 980 (DRM, Annex I has the tables it
//! shares with ISO/IEC 14496-3) and the decoder process of ISO/IEC 14496-3. Parametric stereo
//! (HE-AAC v2) is not decoded: its extension is skipped and the mono core is extended alone.

mod bitstream;
mod freq;
mod hf;
mod qmf;
mod tables;

use std::ops::{Add, Mul};

use craft_media::bitstream::BitReader;

use self::bitstream::{ChannelData, Frame};
use self::freq::FreqTables;
use self::hf::{HighBand, History, Levels, LowBand};
use self::qmf::{Analysis, Synthesis};

/// QMF slots per frame: 1024 core samples in slots of 32.
pub(crate) const SLOTS: usize = 32;
/// The most envelopes in a frame.
pub(crate) const MAX_ENV: usize = 5;
/// Slots of the last frame kept ahead of this one's low band (`t_HFGen`).
const T_HF_GEN: usize = 8;
/// The SBR frame lags the core frame by `T_HF_GEN − T_HF_ADJ` slots.
pub(crate) const T_HF_ADJ: usize = 2;
/// Slots in the low and high band buffers: the frame and `T_HF_GEN` before it.
pub(crate) const X_SLOTS: usize = SLOTS + T_HF_GEN;
/// The 16-bit full scale the specification's energies are in.
const PCM_SCALE: f32 = 32768.0;
/// `NOISE_FLOOR_OFFSET`.
const NOISE_OFFSET: i32 = 6;

/// Output samples by which the SBR decoder delays the core: the filter banks, and the SBR frame
/// lagging the core frame by `T_HF_GEN − T_HF_ADJ` slots of 64.
pub const DELAY: usize = qmf::DELAY + 64 * (T_HF_GEN - T_HF_ADJ);

/// A complex subband sample.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(crate) struct Cx {
    pub(crate) re: f32,
    pub(crate) im: f32,
}

impl Cx {
    fn scale(self, g: f32) -> Cx {
        Cx { re: self.re * g, im: self.im * g }
    }
    fn norm_sqr(self) -> f32 {
        self.re * self.re + self.im * self.im
    }
}

impl Add for Cx {
    type Output = Cx;
    fn add(self, o: Cx) -> Cx {
        Cx { re: self.re + o.re, im: self.im + o.im }
    }
}

impl Mul for Cx {
    type Output = Cx;
    fn mul(self, o: Cx) -> Cx {
        Cx { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }
}

/// `sbr_header()` (§4.5.2.8.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub(crate) amp_res: bool,
    pub(crate) start_freq: u8,
    pub(crate) stop_freq: u8,
    pub(crate) xover_band: u8,
    pub(crate) freq_scale: u8,
    pub(crate) alter_scale: bool,
    pub(crate) noise_bands: u8,
    pub(crate) limiter_bands: u8,
    pub(crate) limiter_gains: u8,
    pub(crate) interpol_freq: bool,
    pub(crate) smoothing_mode: bool,
}

impl Header {
    /// Whether `o` keeps the frequency band tables (a change of any of these fields is an SBR
    /// reset).
    fn same_tables(&self, o: &Header) -> bool {
        (self.start_freq, self.stop_freq, self.freq_scale, self.alter_scale, self.xover_band, self.noise_bands)
            == (o.start_freq, o.stop_freq, o.freq_scale, o.alter_scale, o.xover_band, o.noise_bands)
    }
}

/// Where an element's SBR data is: the access unit, and the bit range of the fill element's
/// payload after its extension type.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Payload<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// `EXT_SBR_DATA_CRC`: a 10-bit CRC leads the data.
    pub(crate) crc: bool,
}

/// Quantised envelope values per envelope and band, and noise floor values per floor and band.
type Quantised = ([[i32; 64]; MAX_ENV], [[i32; 5]; 2]);

/// One channel's state across frames.
#[derive(Clone)]
struct Channel {
    analysis: Analysis,
    synthesis: Synthesis,
    /// Low band of the frame, with the last `T_HF_GEN` slots of the frame before ahead of it.
    x_low: Box<LowBand>,
    /// Adjusted high band of the last frame (its envelopes may reach into this one).
    y_prev: Box<HighBand>,
    /// Where `y_prev` holds data: QMF channels, and the end of its slots.
    y_prev_range: (usize, usize),
    y_prev_end: usize,
    /// Quantised envelope values of the last envelope, and its resolution; noise floor values of
    /// the last noise floor (time-direction delta coding).
    env: [i32; 64],
    env_high_res: bool,
    noise: [i32; 5],
    history: History,
}

impl Channel {
    fn new() -> Channel {
        Channel {
            analysis: Analysis::new(),
            synthesis: Synthesis::new(),
            x_low: Box::new([[Cx::default(); 32]; X_SLOTS]),
            y_prev: Box::new([[Cx::default(); 64]; X_SLOTS]),
            y_prev_range: (0, 0),
            y_prev_end: 0,
            env: [0; 64],
            env_high_res: false,
            noise: [0; 5],
            history: History::new(),
        }
    }

    /// Decode the delta-coded envelope and noise floor values (§4.6.18.3.4); `scale` is 2 for a
    /// balance channel.
    fn absolute(&mut self, d: &ChannelData, t: &FreqTables, scale: i32) -> Quantised {
        let mut env = [[0i32; 64]; MAX_ENV];
        for l in 0..d.grid.num_env {
            let res = d.grid.freq_res[l];
            let n = t.bands(res).len() - 1;
            if d.df_env[l] {
                for k in 0..n {
                    // the value this band refers to in the last envelope, mapped across resolutions
                    let reference = if res == self.env_high_res {
                        k
                    } else if res {
                        hf_band_of(t.bands(false), t.high[k])
                    } else {
                        t.high.iter().position(|&b| b == t.low[k]).unwrap_or(0)
                    };
                    env[l][k] = self.env.get(reference).copied().unwrap_or(0) + scale * d.env[l][k];
                }
            } else {
                env[l][0] = scale * d.env[l][0];
                for k in 1..n {
                    env[l][k] = env[l][k - 1] + scale * d.env[l][k];
                }
            }
            (self.env, self.env_high_res) = (env[l], res);
        }
        let mut noise = [[0i32; 5]; 2];
        for l in 0..d.grid.num_noise {
            for k in 0..t.n_q() {
                let delta = scale * d.noise[l][k];
                noise[l][k] = if d.df_noise[l] {
                    self.noise[k] + delta
                } else if k == 0 {
                    delta
                } else {
                    noise[l][k - 1] + delta
                };
            }
            self.noise = noise[l];
        }
        (env, noise)
    }
}

/// The index of the band of `bands` holding channel `k`.
fn hf_band_of(bands: &[usize], k: usize) -> usize {
    bands.partition_point(|&b| b <= k).saturating_sub(1)
}

/// `2^x`, kept to a range whose energies stay finite.
fn exp2(x: f32) -> f32 {
    x.clamp(-64.0, 64.0).exp2()
}

/// Dequantise one channel (§4.6.18.3.5), or a coupled pair from its level and balance channels.
fn dequantise(frame: &Frame, values: &[Quantised], t: &FreqTables) -> Vec<Levels> {
    let g = &frame.channels[0].grid;
    let amp = if g.amp_res { 1.0 } else { 0.5 };
    let mut out = vec![Levels { energy: [[0.0; 64]; MAX_ENV], noise: [[0.0; 5]; 2] }; values.len()];
    if frame.coupling && values.len() == 2 {
        let pan = if g.amp_res { 12.0 } else { 24.0 };
        let ((e0, q0), (e1, q1)) = (&values[0], &values[1]);
        for l in 0..g.num_env {
            for k in 0..t.bands(g.freq_res[l]).len() - 1 {
                let level = 64.0 * exp2(amp * e0[l][k] as f32 + 1.0);
                let balance = amp * (pan - e1[l][k] as f32);
                out[0].energy[l][k] = level / (1.0 + exp2(balance));
                out[1].energy[l][k] = level / (1.0 + exp2(-balance));
            }
        }
        for l in 0..g.num_noise {
            for k in 0..t.n_q() {
                let level = exp2((NOISE_OFFSET - q0[l][k] + 1) as f32);
                let balance = (12 - q1[l][k]) as f32;
                out[0].noise[l][k] = level / (1.0 + exp2(balance));
                out[1].noise[l][k] = level / (1.0 + exp2(-balance));
            }
        }
        return out;
    }
    for ((lv, (e, q)), d) in out.iter_mut().zip(values).zip(&frame.channels) {
        let g = &d.grid;
        let amp = if g.amp_res { 1.0 } else { 0.5 };
        for l in 0..g.num_env {
            for k in 0..t.bands(g.freq_res[l]).len() - 1 {
                lv.energy[l][k] = 64.0 * exp2(amp * e[l][k] as f32);
            }
        }
        for l in 0..g.num_noise {
            for k in 0..t.n_q() {
                lv.noise[l][k] = exp2((NOISE_OFFSET - q[l][k]) as f32);
            }
        }
    }
    out
}

/// The SBR decoder of one channel element: a single channel (SCE, or an LFE, which has no SBR
/// data) or a channel pair (CPE).
#[derive(Clone)]
pub(crate) struct Element {
    /// Output sampling rate.
    rate: u32,
    header: Option<Header>,
    tables: Option<FreqTables>,
    channels: Vec<Channel>,
}

impl Element {
    pub(crate) fn new(channels: usize, rate: u32) -> Element {
        Element { rate, header: None, tables: None, channels: (0..channels).map(|_| Channel::new()).collect() }
    }

    pub(crate) fn channels(&self) -> usize {
        self.channels.len()
    }

    /// Forget the signal (a seek). The header is kept: it holds until the stream sends another.
    pub(crate) fn reset(&mut self) {
        for c in &mut self.channels {
            *c = Channel::new();
        }
    }

    /// Read an element's SBR data: `None` without a header to read it by.
    fn read(&mut self, p: Payload) -> Option<Frame> {
        let mut br = BitReader::new(p.data);
        br.seek_bits(p.start);
        if p.crc {
            br.skip(10).ok()?;
        }
        if br.read_flag().ok()? {
            let h = Header::read(&mut br).ok()?;
            // a header that runs past its fill element is damaged: keep the one in use
            (br.position() <= p.end).then_some(())?;
            if self.header != Some(h) {
                if self.header.is_none_or(|o| !o.same_tables(&h)) {
                    for c in &mut self.channels {
                        c.history.reset_tables();
                    }
                }
                self.tables = FreqTables::new(&h, self.rate);
                self.header = Some(h);
            }
        }
        let (h, t) = (self.header?, self.tables.as_ref()?);
        let frame = bitstream::read_data(&mut br, t, h.amp_res, self.channels.len()).ok()?;
        (br.position() <= p.end).then_some(frame)
    }

    /// Decode one frame: 1024 core samples per channel in, 2048 out.
    pub(crate) fn process(&mut self, core: &[Vec<f32>], payload: Option<Payload>) -> Vec<Vec<f32>> {
        for (c, pcm) in self.channels.iter_mut().zip(core) {
            c.x_low.copy_within(SLOTS..X_SLOTS, 0);
            let scaled: Vec<f32> = pcm.iter().map(|s| s * PCM_SCALE).collect();
            c.analysis.process(&scaled, &mut c.x_low[T_HF_GEN..]);
        }
        let frame = payload.and_then(|p| self.read(p));
        let mut y: Vec<Box<HighBand>> = self.channels.iter().map(|_| Box::new([[Cx::default(); 64]; X_SLOTS])).collect();
        // per channel: kx, m, and the first and end slot of `y` this frame fills
        let mut current: Vec<Option<(usize, usize, usize, usize)>> = vec![None; self.channels.len()];
        if let (Some(frame), Some(h), Some(t)) = (&frame, self.header, self.tables.as_ref()) {
            let values: Vec<_> = self
                .channels
                .iter_mut()
                .zip(&frame.channels)
                .enumerate()
                .map(|(i, (c, d))| c.absolute(d, t, if frame.coupling && i == 1 { 2 } else { 1 }))
                .collect();
            let levels = dequantise(frame, &values, t);
            for (((c, d), (lv, y)), cur) in self.channels.iter_mut().zip(&frame.channels).zip(levels.iter().zip(&mut y)).zip(&mut current) {
                let g = &d.grid;
                let slots = (2 * g.t_env[0], 2 * g.t_env[g.num_env]);
                let mut x_high: Box<HighBand> = Box::new([[Cx::default(); 64]; X_SLOTS]);
                hf::generate(&c.x_low, t, &d.invf, &mut c.history, slots, &mut x_high);
                let f = hf::Frame { x: &x_high, t, hdr: &h, g, lv, harmonic: d.harmonic.as_ref() };
                hf::adjust(&f, &mut c.history, y);
                *cur = Some((t.kx, t.m, slots.0 + T_HF_ADJ, slots.1 + T_HF_ADJ));
            }
        }
        let mut out = vec![vec![0f32; 2 * 1024]; self.channels.len()];
        for (((c, y), out), current) in self.channels.iter_mut().zip(y).zip(&mut out).zip(current) {
            for (l, block) in out.as_chunks_mut::<64>().0.iter_mut().enumerate() {
                let i = l + T_HF_ADJ;
                let mut x = [Cx::default(); 64];
                // the high band of this slot: this frame's envelopes, the last frame's reaching
                // into this one, or none
                let high = match current {
                    Some((kx, m, from, to)) if (from..to).contains(&i) => Some((&y[i], kx, m)),
                    _ if i + SLOTS < c.y_prev_end => Some((&c.y_prev[i + SLOTS], c.y_prev_range.0, c.y_prev_range.1)),
                    _ => None,
                };
                let low = high.map_or(current.map_or(32, |cur| cur.0), |h| h.1);
                x[..low].copy_from_slice(&c.x_low[i][..low]);
                if let Some((row, kx, m)) = high {
                    x[kx..kx + m].copy_from_slice(&row[kx..kx + m]);
                }
                c.synthesis.process(&x, 64, block);
            }
            for s in out.iter_mut() {
                *s /= PCM_SCALE;
            }
            c.y_prev = y;
            (c.y_prev_range, c.y_prev_end) = current.map_or(((0, 0), 0), |cur| ((cur.0, cur.1), cur.3));
        }
        out
    }
}

/// The header of [`test_payload`].
#[cfg(test)]
pub(crate) const TEST_HEADER: Header = Header {
    amp_res: true,
    start_freq: 5,
    stop_freq: 9,
    xover_band: 0,
    freq_scale: 2,
    alter_scale: true,
    noise_bands: 2,
    limiter_bands: 2,
    limiter_gains: 2,
    interpol_freq: true,
    smoothing_mode: true,
};

/// An SBR single channel element for tests at output rate `rate`: [`TEST_HEADER`] (3 dB steps,
/// start 5, stop 9, the default extra fields), then one FIXFIX envelope at high resolution with
/// band `k` at `env(k)` (1.5 dB steps, as a single FIXFIX envelope has) and noise band `k` at
/// `noise(k)`.
#[cfg(test)]
pub(crate) fn test_payload(rate: u32, env: impl Fn(usize) -> i32, noise: impl Fn(usize) -> i32) -> Option<Vec<u8>> {
    use craft_media::bitstream::BitWriter;
    let t = FreqTables::new(&TEST_HEADER, rate)?;
    let (env, noise): (Vec<i32>, Vec<i32>) = ((0..t.n_high()).map(env).collect(), (0..t.n_q()).map(noise).collect());
    let mut bw = BitWriter::new();
    bw.write_bits(1, 1); // bs_header_flag
    for (v, n) in [(1, 1), (5, 4), (9, 4), (0, 3), (0, 2), (0, 1), (0, 1)] {
        bw.write_bits(v, n); // amp_res, start, stop, xover, reserved, no extra fields
    }
    bw.write_bits(0, 1); // bs_data_extra
    bw.write_bits(u32::from(bitstream::FIXFIX), 2);
    bw.write_bits(0, 2); // one envelope
    bw.write_bits(1, 1); // high resolution
    bw.write_bits(0, 2); // frequency-direction envelope and noise floor
    for _ in &noise {
        bw.write_bits(0, 2); // no inverse filtering
    }
    bw.write_bits(env[0] as u32, 7);
    for w in env.windows(2) {
        let (code, len) = tables::F_ENV_1_5DB[(w[1] - w[0] + 60) as usize];
        bw.write_bits(code, u32::from(len));
    }
    bw.write_bits(noise[0] as u32, 5);
    for w in noise.windows(2) {
        let (code, len) = tables::F_ENV_3_0DB[(w[1] - w[0] + 31) as usize];
        bw.write_bits(code, u32::from(len));
    }
    bw.write_bits(0, 2); // no sinusoids, no extended data
    Some(bw.finish())
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;

    const RATE: u32 = 44_100;

    fn tables() -> FreqTables {
        FreqTables::new(&TEST_HEADER, RATE).unwrap()
    }

    /// Mean energy per DFT bin of `x` between `lo` and `hi` QMF channels (64 per half rate).
    fn energy(x: &[f32], lo: usize, hi: usize) -> f64 {
        let n = x.len();
        let (b0, b1) = (lo * n / 128, hi * n / 128);
        let mut e = 0f64;
        for b in b0 + 2..b1 - 2 {
            let (mut re, mut im) = (0f64, 0f64);
            for (i, &s) in x.iter().enumerate() {
                let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
                let ph = 2.0 * PI * (b * i) as f64 / n as f64;
                re += f64::from(s) * w * ph.cos();
                im += f64::from(s) * w * ph.sin();
            }
            e += re * re + im * im;
        }
        e / (b1 - b0 - 4) as f64
    }

    fn run(e: &mut Element, data: &[u8], frames: usize) -> Vec<f32> {
        let core = vec![vec![0f32; 1024]];
        let mut out = Vec::new();
        for _ in 0..frames {
            let p = Payload { data, start: 0, end: 8 * data.len(), crc: false };
            let block = e.process(&core, Some(p));
            assert_eq!((block.len(), block[0].len()), (1, 2048));
            assert!(block[0].iter().all(|s| s.is_finite()));
            out.extend_from_slice(&block[0]);
        }
        out
    }

    /// Over a silent core the high band is the noise the envelope asks for: the upper bands,
    /// 12 steps of 1.5 dB above the lower ones, come out 18 dB louder, and nothing below `kx`.
    #[test]
    fn the_high_band_follows_the_envelope() {
        let t = tables();
        let mid = t.n_high() / 2;
        let data = test_payload(RATE, |k| if k < mid { 30 } else { 42 }, |_| 0).unwrap();
        let mut e = Element::new(1, RATE);
        let out = run(&mut e, &data, 6);
        let tail = &out[out.len() - 4096..];
        let (lower, upper) = (energy(tail, t.high[0], t.high[mid]), energy(tail, t.high[mid], t.high[t.n_high()]));
        let step = 10.0 * (upper / lower).log10();
        assert!((step - 18.06).abs() < 1.5, "{step:.2} dB");
        let below = energy(tail, 1, t.kx - 1);
        assert!(10.0 * (lower / below.max(1e-30)).log10() > 60.0);
        // the same envelope without its header is not read: an element that has seen none
        // passes the core through alone
        let mut fresh = Element::new(1, RATE);
        let mut no_header = data.clone();
        no_header[0] &= 0x7F;
        assert!(run(&mut fresh, &no_header, 3).iter().all(|&s| s == 0.0));
    }

    /// Damaged SBR data (flipped bits, cut short, or noise) never panics and keeps the output
    /// finite and of full length.
    #[test]
    fn damaged_sbr_data_is_survived() {
        let good = test_payload(RATE, |k| 20 + k as i32, |_| 3).unwrap();
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut e = Element::new(1, RATE);
        let mut pair = Element::new(2, RATE);
        for i in 0..600 {
            let mut data = good.clone();
            for _ in 0..1 + next() % 6 {
                let bit = next() as usize % (8 * data.len());
                data[bit / 8] ^= 0x80 >> (bit % 8);
            }
            if i % 3 == 0 {
                data.truncate(1 + next() as usize % data.len());
            }
            if i % 5 == 0 {
                data = (0..1 + next() % 40).map(|_| next() as u8).collect();
                data[0] |= 0x80;
            }
            run(&mut e, &data, 1);
            let p = Payload { data: &data, start: 0, end: 8 * data.len(), crc: i % 2 == 0 };
            let out = pair.process(&[vec![0.1; 1024], vec![-0.1; 1024]], Some(p));
            assert!(out.iter().all(|c| c.len() == 2048 && c.iter().all(|s| s.is_finite())));
        }
    }

    /// The largest envelope (127 steps of 1.5 dB) with the largest noise floor (each band 31
    /// steps below the last) keeps the output finite: the gains are formed so that no
    /// intermediate product of two clamped levels overflows.
    #[test]
    fn extreme_levels_stay_finite() {
        let rate =
            [22_050, 24_000, 32_000, 44_100, 48_000].into_iter().find(|&r| FreqTables::new(&TEST_HEADER, r).is_some_and(|t| t.n_q() >= 3)).unwrap();
        let data = test_payload(rate, |_| 127, |k| -31 * k as i32).unwrap();
        let mut e = Element::new(1, rate);
        run(&mut e, &data, 3);
    }

    /// A header that runs past the end of its fill element is not taken: the one in use stays,
    /// and the frame is dropped.
    #[test]
    fn a_header_cut_short_is_not_taken() {
        let data = test_payload(RATE, |_| 30, |_| 0).unwrap();
        let mut e = Element::new(1, RATE);
        run(&mut e, &data, 1);
        assert_eq!(e.header, Some(TEST_HEADER));
        // another start frequency (bits 2..6), in an element that ends 6 bits in
        let mut cut = data.clone();
        cut[0] ^= 0x04;
        assert!(e.read(Payload { data: &cut, start: 0, end: 6, crc: false }).is_none());
        assert_eq!(e.header, Some(TEST_HEADER));
        assert!(e.read(Payload { data: &cut, start: 0, end: 8 * cut.len(), crc: false }).is_some());
        assert_ne!(e.header, Some(TEST_HEADER));
    }
}
