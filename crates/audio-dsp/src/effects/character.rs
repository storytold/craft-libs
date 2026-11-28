//! Character effects for a live voice. None of them adds latency.
//!
//! The ring modulator (`ring_mod`) has four effects:
//!
//! - Ring multiplies the input by a sine carrier at the frequency, which moves each partial of a
//!   voice to the sum and the difference of its frequency and the carrier's.
//! - Frequency Mod reads the input through a delay that the carrier swings between 2 samples and
//!   2 samples plus 2 ms.
//! - Resonator + and Resonator - feed the output back after one period of the frequency, with a
//!   positive or a negative sign, so the input rings at the frequency and its multiples (Resonator
//!   +) or at its odd half multiples (Resonator -). The output is scaled by `1 - resonance`, so the
//!   peak gain at resonance is unity.
//!
//! An LFO, an envelope follower (5 ms attack, 100 ms release) or a sample-and-hold source moves the
//! frequency by up to two octaves either way at 100 % depth. The envelope follower moves it up
//! only.
//!
//! The bitcrusher (`bitcrusher`) takes one input sample every `rate_divide` samples, holds it until
//! the next one, and rounds it to a word length of 1 to 16 bits.
//!
//! The tremolo (`tremolo`) lowers the gain of every channel by up to `depth` once per cycle of a
//! sine, triangle or square LFO at `rate`.
//!
//! The vibrato (`vibrato`) reads the input through a delay that grows from 0 and shrinks back once
//! per cycle of a sine or triangle LFO at `rate`, so the pitch swings by up to `depth` cents. While
//! no glide to new settings is in progress, the delay is 0 at the start of each cycle.

use super::util::{DelayLine, Envelope, Lfo, Prng, coef, delay_line_len, echo_tail, uni_sine, uni_tri};
use crate::{AudioEffect, ParamSpec, ParamValues, Smoothed, Unit, block_len, flush32, param_plumbing};
use std::f64::consts::TAU;

fn blend(dry: f64, wet: f64, mix: f64) -> f64 {
    (1.0 - mix) * dry + mix * wet
}

/// Smoothing time of the frequency, resonance, depth and mix, in ms.
const SM: f32 = 20.0;
const RING_EFFECTS: &[&str] = &["Ring", "Frequency Mod", "Resonator +", "Resonator -"];
const RING_SOURCES: &[&str] = &["LFO", "Envelope", "Sample & Hold"];
/// Lowest frequency the control reaches.
const MIN_HZ: f32 = 20.0;
/// Octaves the frequency moves at full modulation depth.
const MOD_OCTAVES: f32 = 2.0;
/// Peak delay swing of Frequency Mod, in ms.
const FM_SWING_MS: f32 = 1.0;
/// Shortest delay of Frequency Mod, in samples (room for the interpolated read).
const FM_BASE: f32 = 2.0;
/// Sample-and-hold seed (the same sequence after every reset).
const SEED: u64 = 0x5EED_1234;
/// Highest frequency the modulation reaches, as a fraction of the sample rate.
const MAX_FRACTION: f32 = 0.45;
const TREMOLO_SHAPES: &[&str] = &["Sine", "Triangle", "Square"];
/// Length of each edge of the tremolo's square, in ms.
const EDGE_MS: f64 = 2.0;
const VIBRATO_SHAPES: &[&str] = &["Sine", "Triangle"];
/// Lowest rate of the vibrato, in Hz.
const VIBRATO_MIN_RATE: f32 = 1.0;
/// Greatest depth of the vibrato, in cents.
const VIBRATO_MAX_CENTS: f32 = 100.0;
/// Most the vibrato's glide moves the delay per sample on top of the curve.
const VIBRATO_GLIDE_SLOPE: f32 = 1.0 / 32.0;

/// Ring modulation, frequency modulation and positive or negative comb resonators, with the
/// frequency moved by an LFO, an envelope follower or a sample-and-hold source.
pub struct RingMod {
    pv: ParamValues,
    sr: f32,
    lines: Vec<DelayLine>,
    carrier: Lfo,
    lfo: Lfo,
    rng: Prng,
    env: Envelope,
    env_attack: f32,
    env_release: f32,
    held: f32,
    hold_left: usize,
    /// Effect the lines hold state for.
    effect: usize,
    freq: Smoothed,
    res: Smoothed,
    depth: Smoothed,
    mix: Smoothed,
}

impl RingMod {
    pub const PARAMS: &'static [ParamSpec] = &[
        ParamSpec::choice("effect", "Effect", RING_EFFECTS, 0),
        ParamSpec::log("frequency", "Frequency", MIN_HZ, 5000.0, 440.0, Unit::Hertz),
        ParamSpec::new("resonance", "Resonance", 0.0, 95.0, 50.0, Unit::Percent),
        ParamSpec::choice("mod_source", "Mod Source", RING_SOURCES, 0),
        ParamSpec::log("mod_rate", "Mod Rate", 0.05, 20.0, 1.0, Unit::Hertz),
        ParamSpec::new("mod_depth", "Mod Depth", 0.0, 100.0, 0.0, Unit::Percent),
        ParamSpec::new("mix", "Mix", 0.0, 100.0, 100.0, Unit::Percent),
    ];

    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let sample_rate = crate::sane_sample_rate(sample_rate);
        let ch = crate::sane_channels(channels);
        // One period of the lowest frequency the modulation reaches, or the widest Frequency Mod
        // delay, whichever is longer.
        let longest_ms = (1000.0 / (MIN_HZ * (-MOD_OCTAVES).exp2())).max(2.0 * FM_SWING_MS) + 1.0;
        let len = delay_line_len(longest_ms, sample_rate, ch);
        let sm = |v| Smoothed::with_ms(v, sample_rate, SM);
        let mut s = RingMod {
            pv: ParamValues::new(Self::PARAMS),
            sr: sample_rate,
            lines: vec![DelayLine::new(len); ch],
            carrier: Lfo::default(),
            lfo: Lfo::default(),
            rng: Prng::new(SEED),
            env: Envelope::default(),
            env_attack: coef(5.0, sample_rate),
            env_release: coef(100.0, sample_rate),
            held: 0.0,
            hold_left: 0,
            effect: 0,
            freq: sm(440.0),
            res: sm(0.0),
            depth: sm(0.0),
            mix: sm(1.0),
        };
        s.apply_params(true);
        s.effect = s.pv.idx("effect");
        s
    }

    fn apply_params(&mut self, snap: bool) {
        self.freq.set(self.pv.v("frequency"));
        self.res.set(self.pv.v("resonance") / 100.0);
        self.depth.set(self.pv.v("mod_depth") / 100.0);
        self.mix.set(self.pv.v("mix") / 100.0);
        if snap {
            for s in [&mut self.freq, &mut self.res, &mut self.depth, &mut self.mix] {
                s.snap();
            }
        }
    }

    /// One period of the lowest frequency the modulation reaches, in ms, from the settings and from
    /// the values still ramping toward them. The envelope follower moves the frequency up only, so
    /// with it the lowest is the frequency. The frequency is held within the same range as in
    /// [`AudioEffect::process`].
    fn longest_period_ms(&self) -> f32 {
        let down = if self.pv.idx("mod_source") == 1 { 0.0 } else { self.depth.value().max(self.depth.target()) };
        let lowest = self.freq.value().min(self.freq.target()) * (-MOD_OCTAVES * down).exp2();
        1000.0 / lowest.clamp(1.0, self.sr * MAX_FRACTION)
    }
}

impl AudioEffect for RingMod {
    param_plumbing!("ring_mod");
    /// Ring has no tail. Frequency Mod holds up to its widest delay. The resonators ring until
    /// the feedback has fallen by 60 dB at the longest period the modulation reaches, at the higher
    /// of the resonance setting and the value still ramping toward it.
    fn tail_samples(&self) -> usize {
        match self.pv.idx("effect") {
            1 => (FM_BASE + 2.0 * FM_SWING_MS * 0.001 * self.sr).ceil() as usize + 1,
            2 | 3 => echo_tail(self.longest_period_ms(), self.res.value().max(self.res.target()), self.sr),
            _ => 0,
        }
    }
    fn reset(&mut self) {
        self.lines.iter_mut().for_each(DelayLine::reset);
        self.carrier.reset();
        self.lfo.reset();
        self.rng = Prng::new(SEED);
        self.env.env = 0.0;
        self.held = 0.0;
        self.hold_left = 0;
        self.effect = self.pv.idx("effect");
        for s in [&mut self.freq, &mut self.res, &mut self.depth, &mut self.mix] {
            s.snap();
        }
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        if n == 0 {
            return;
        }
        let nch = channels.len().min(self.lines.len());
        let effect = self.pv.idx("effect");
        if effect != self.effect {
            // The lines hold the previous effect's state.
            self.lines.iter_mut().for_each(DelayLine::reset);
            self.effect = effect;
        }
        let source = self.pv.idx("mod_source");
        let rate = self.pv.v("mod_rate");
        let hold = ((self.sr / rate.max(0.05)) as usize).max(1);
        // A faster rate cuts the value being held short, instead of holding it for the old length.
        self.hold_left = self.hold_left.min(hold);
        let highest = self.sr * MAX_FRACTION;
        let swing = FM_SWING_MS * 0.001 * self.sr;
        let sign = if effect == 3 { -1.0 } else { 1.0 };
        for i in 0..n {
            let level = channels[..nch].iter().map(|c| c[i]).filter(|x| x.is_finite()).fold(0.0f32, |m, x| m.max(x.abs()));
            let env = self.env.tick(self.env_attack, self.env_release, level);
            let m = match source {
                1 => env.min(1.0),
                2 => {
                    if self.hold_left == 0 {
                        self.held = self.rng.uniform();
                        self.hold_left = hold;
                    }
                    self.hold_left -= 1;
                    self.held
                }
                _ => (TAU * self.lfo.phase(0.0)).sin() as f32,
            };
            self.lfo.advance(rate, self.sr);
            let f = (self.freq.tick() * (MOD_OCTAVES * self.depth.tick() * m).exp2()).clamp(1.0, highest);
            let fb = self.res.tick();
            let mix = self.mix.tick();
            let carrier = (TAU * self.carrier.phase(0.0)).sin() as f32;
            self.carrier.advance(f, self.sr);
            // The read happens before this sample's push, so one period back is `period - 1`.
            let back = self.sr / f - 1.0;
            for (ch, line) in self.lines.iter_mut().enumerate() {
                // A non-finite sample is taken as silence. A channel the block leaves out is fed
                // silence, so it holds none of its earlier audio when it comes back.
                let dry = match channels.get(ch).filter(|_| ch < nch) {
                    Some(c) if c[i].is_finite() => c[i],
                    _ => 0.0,
                };
                let wet = match effect {
                    1 => {
                        line.push(dry);
                        // The read interpolates in f64, as Vibrato's does, so neighboring samples
                        // near the f32 limit with opposite signs do not overflow.
                        let d = (FM_BASE + swing * (1.0 + carrier)).clamp(0.0, line.max_delay());
                        let (di, frac) = (d.floor() as usize, f64::from(d - d.floor()));
                        let (a, b) = (f64::from(line.tap(di)), f64::from(line.tap(di + 1)));
                        (a + (b - a) * frac) as f32
                    }
                    2 | 3 => {
                        let y = flush32(dry + sign * fb * line.read(back));
                        line.push(y);
                        y * (1.0 - fb)
                    }
                    _ => dry * carrier,
                };
                if ch < nch {
                    let (dry, wet) = (dry as f64, wet as f64);
                    channels[ch][i] = blend(dry, wet, f64::from(mix)) as f32;
                }
            }
        }
    }
}

/// Bit-depth and sample-rate reduction. The input is sampled once every `rate_divide` samples and
/// held in between; a fractional value alternates between the two nearest whole hold lengths, so
/// the holds average it. Each held sample is rounded to the nearest multiple of the step
/// `2^(1 - bits)`, so silence stays silence and the range -1 ..= 1 takes `2^bits + 1` levels;
/// fractional bits give steps between the whole-bit ones. Bits and Rate Divide take effect from
/// the next sample: a new Bits rounds the sample being held again, and a shorter hold cuts the
/// one in progress short. Mix moves to a new value over 20 ms.
pub struct Bitcrusher {
    pv: ParamValues,
    /// The input sample each channel holds, before rounding.
    held: Vec<f32>,
    /// Samples since the last capture, counted so that a capture happens when it reaches the hold
    /// length. Infinite after a reset, so the next sample is captured.
    since: f64,
    mix: Smoothed,
}

impl Bitcrusher {
    pub const PARAMS: &'static [ParamSpec] = &[
        ParamSpec::new("bits", "Bits", 1.0, 16.0, 8.0, Unit::None),
        ParamSpec::new("rate_divide", "Rate Divide", 1.0, 50.0, 4.0, Unit::None),
        ParamSpec::new("mix", "Mix", 0.0, 100.0, 100.0, Unit::Percent),
    ];

    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let sample_rate = crate::sane_sample_rate(sample_rate);
        let ch = crate::sane_channels(channels);
        let mut s = Bitcrusher {
            pv: ParamValues::new(Self::PARAMS),
            held: vec![0.0; ch],
            since: f64::INFINITY,
            mix: Smoothed::with_ms(1.0, sample_rate, SM),
        };
        s.apply_params(true);
        s
    }

    fn apply_params(&mut self, snap: bool) {
        self.mix.set(self.pv.v("mix") / 100.0);
        if snap {
            self.mix.snap();
        }
    }
}

impl AudioEffect for Bitcrusher {
    param_plumbing!("bitcrusher");
    /// A held sample lasts at most the hold length rounded up, so it sounds for up to that many
    /// samples less one after the input ends.
    fn tail_samples(&self) -> usize {
        (self.pv.v("rate_divide").ceil() as usize).saturating_sub(1)
    }
    fn reset(&mut self) {
        self.held.fill(0.0);
        self.since = f64::INFINITY;
        self.mix.snap();
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        let nch = channels.len().min(self.held.len());
        let hold = f64::from(self.pv.v("rate_divide"));
        // The rounding and the mix run in f64, so a finite input near the f32 limit stays finite.
        let step = (1.0 - f64::from(self.pv.v("bits"))).exp2();
        let limit = f64::from(f32::MAX);
        for i in 0..n {
            let capture = self.since >= hold;
            if capture {
                // On schedule, less than one sample is left over. More is left after a reset or
                // after the hold got shorter; the new holds then count from this capture.
                let left = self.since - hold;
                self.since = if left < 1.0 { left } else { 0.0 };
            }
            self.since += 1.0;
            let mix = f64::from(self.mix.tick());
            for (ch, held) in self.held.iter_mut().enumerate() {
                // A non-finite sample is taken as silence. A channel the block leaves out holds
                // silence from its first left-out sample on, so it holds none of its earlier audio
                // when it comes back.
                let dry = match channels.get(ch).filter(|_| ch < nch) {
                    Some(c) if c[i].is_finite() => f64::from(c[i]),
                    _ => 0.0,
                };
                if capture || ch >= nch {
                    *held = dry as f32;
                }
                if ch < nch {
                    // The held input is rounded at the current step, so a change of Bits also
                    // applies to the hold in progress.
                    let crushed = ((f64::from(*held) / step).round() * step).clamp(-limit, limit);
                    channels[ch][i] = blend(dry, crushed, mix) as f32;
                }
            }
        }
    }
}

/// Amplitude modulation. The gain is `1 - depth * m`, where the LFO value `m` runs from 0 at the
/// start of each cycle to 1 at its middle and back, as a sine, a triangle or a square. The square
/// holds 0 and 1 for half a cycle each and moves between them along a raised-cosine edge 2 ms long,
/// centered on the quarter and three-quarter points, so it does not click. Rate takes effect from
/// the next sample with the phase kept. A change of Shape, or of Rate while the shape is the
/// square, glides from the last LFO value to the new curve over 20 ms, with the value kept within
/// 0 ..= 1. Depth moves to a new value over 20 ms.
pub struct Tremolo {
    pv: ParamValues,
    sr: f32,
    channels: usize,
    lfo: Lfo,
    /// Shape the LFO value follows.
    shape: usize,
    /// Length of the square's edges, in cycles, for the rate of the last block.
    edge: f64,
    /// LFO value of the last sample.
    last: f32,
    /// Difference between the last LFO value and the new curve at a change of Shape, or of the
    /// square's edges. It fades to zero over `glide_len` samples.
    glide: f32,
    glide_left: usize,
    glide_len: usize,
    depth: Smoothed,
}

impl Tremolo {
    pub const PARAMS: &'static [ParamSpec] = &[
        ParamSpec::log("rate", "Rate", 0.1, 20.0, 5.0, Unit::Hertz),
        ParamSpec::new("depth", "Depth", 0.0, 100.0, 50.0, Unit::Percent),
        ParamSpec::choice("shape", "Shape", TREMOLO_SHAPES, 0),
    ];

    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let sample_rate = crate::sane_sample_rate(sample_rate);
        let mut s = Tremolo {
            pv: ParamValues::new(Self::PARAMS),
            sr: sample_rate,
            channels: crate::sane_channels(channels),
            lfo: Lfo::default(),
            shape: 0,
            edge: 0.0,
            last: 0.0,
            glide: 0.0,
            glide_left: 0,
            glide_len: ((SM * 0.001 * sample_rate) as usize).max(1),
            depth: Smoothed::with_ms(0.5, sample_rate, SM),
        };
        s.apply_params(true);
        s.shape = s.pv.idx("shape");
        s.edge = s.edge_len();
        s
    }

    /// Length of the square's edges at the current rate, in cycles.
    fn edge_len(&self) -> f64 {
        (EDGE_MS * 0.001 * f64::from(self.pv.v("rate"))).clamp(1e-9, 0.25)
    }

    fn apply_params(&mut self, snap: bool) {
        self.depth.set(self.pv.v("depth") / 100.0);
        if snap {
            self.depth.snap();
        }
    }

    /// LFO value of `shape` at phase `p` (cycles), with square edges `edge` cycles long.
    fn shape_value(shape: usize, p: f64, edge: f64) -> f32 {
        match shape {
            1 => uni_tri(p),
            2 => {
                // The triangle crosses 1/2 at the edges' centers with a slope of 2 per cycle, so an
                // edge `edge` cycles long spans `2 * edge` of it.
                let tri = if p < 0.5 { 2.0 * p } else { 2.0 - 2.0 * p };
                let u = ((tri - 0.5) / (2.0 * edge) + 0.5).clamp(0.0, 1.0);
                (0.5 - 0.5 * (std::f64::consts::PI * u).cos()) as f32
            }
            _ => uni_sine(p),
        }
    }
}

impl AudioEffect for Tremolo {
    param_plumbing!("tremolo");
    fn reset(&mut self) {
        self.lfo.reset();
        self.shape = self.pv.idx("shape");
        self.edge = self.edge_len();
        self.last = 0.0;
        self.glide = 0.0;
        self.glide_left = 0;
        self.depth.snap();
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        if n == 0 {
            // No sample used the current settings, so a glide in progress goes on unchanged.
            return;
        }
        let nch = channels.len().min(self.channels);
        let rate = self.pv.v("rate");
        let edge = self.edge_len();
        let shape = self.pv.idx("shape");
        if shape != self.shape || (shape == 2 && edge != self.edge) {
            self.glide = self.last - Self::shape_value(shape, self.lfo.phase(0.0), edge);
            self.glide_left = self.glide_len;
            self.shape = shape;
        }
        self.edge = edge;
        for i in 0..n {
            let mut m = Self::shape_value(shape, self.lfo.phase(0.0), edge);
            if self.glide_left > 0 {
                m = (m + self.glide * self.glide_left as f32 / self.glide_len as f32).clamp(0.0, 1.0);
                self.glide_left -= 1;
            }
            self.last = m;
            self.lfo.advance(rate, self.sr);
            let gain = 1.0 - self.depth.tick() * m;
            for c in channels[..nch].iter_mut() {
                // A non-finite sample is taken as silence.
                c[i] = if c[i].is_finite() { c[i] * gain } else { 0.0 };
            }
        }
    }
}

/// Pitch modulation. The input is read through a delay of `swing * m` samples, where the LFO value
/// `m` runs from 0 at the start of each cycle to 1 at its middle and back, as a sine or a triangle.
/// The swing is set so that the pitch falls by `depth` cents while the delay grows, and rises by a
/// little less while it shrinks: the read moves at `1 -/+ d` samples per sample, with
/// `d = 1 - 2^(-depth / 1200)`. A sine reaches `d` at its steepest point, and a triangle holds it
/// for each half of the cycle. While no glide is in progress, the delay is 0 at the start of each
/// cycle. Rate takes effect from the next sample with the phase kept. A change of Rate, Depth or
/// Shape glides the delay from its last value to the new curve, moving it by at most 1/32 sample
/// per sample on top of the curve, so the output neither clicks nor jumps in pitch. A glide can run
/// past the start of a cycle, and the delay there stays above 0 until the glide ends.
pub struct Vibrato {
    pv: ParamValues,
    sr: f32,
    lines: Vec<DelayLine>,
    lfo: Lfo,
    /// Shape the delay follows.
    shape: usize,
    /// Peak delay of the curve, in samples, for the settings of the last block.
    swing: f32,
    /// Delay of the last sample, in samples.
    last: f32,
    /// Difference between the last delay and the new curve at a change of the settings. It fades
    /// to zero over `glide_len` samples.
    glide: f32,
    glide_left: usize,
    glide_len: usize,
}

impl Vibrato {
    pub const PARAMS: &'static [ParamSpec] = &[
        ParamSpec::log("rate", "Rate", VIBRATO_MIN_RATE, 14.0, 6.0, Unit::Hertz),
        ParamSpec::new("depth", "Depth (cents)", 0.0, VIBRATO_MAX_CENTS, 40.0, Unit::None),
        ParamSpec::choice("shape", "Shape", VIBRATO_SHAPES, 0),
    ];

    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let sample_rate = crate::sane_sample_rate(sample_rate);
        let ch = crate::sane_channels(channels);
        // The widest swing is the triangle's at the lowest rate and the greatest depth. A glide
        // can add up to one more swing on top of the curve.
        let widest_ms = (1000.0 * Self::slope(VIBRATO_MAX_CENTS) / (2.0 * f64::from(VIBRATO_MIN_RATE))) as f32;
        let len = delay_line_len(2.0 * widest_ms + 1.0, sample_rate, ch);
        let mut s = Vibrato {
            pv: ParamValues::new(Self::PARAMS),
            sr: sample_rate,
            lines: vec![DelayLine::new(len); ch],
            lfo: Lfo::default(),
            shape: 0,
            swing: 0.0,
            last: 0.0,
            glide: 0.0,
            glide_left: 0,
            glide_len: 1,
        };
        s.shape = s.pv.idx("shape");
        s.swing = s.swing_len();
        s
    }

    fn apply_params(&mut self, _snap: bool) {}

    /// Samples the read moves per sample, away from 1, at a depth of `cents`.
    fn slope(cents: f32) -> f64 {
        1.0 - (-f64::from(cents) / 1200.0).exp2()
    }

    /// Peak delay of the curve for the current settings, in samples.
    fn swing_len(&self) -> f32 {
        let (d, rate, sr) = (Self::slope(self.pv.v("depth")), f64::from(self.pv.v("rate")), f64::from(self.sr));
        // The sine's delay `swing * (1 - cos(2 pi p)) / 2` moves by at most `swing * pi * rate / sr`
        // per sample, and the triangle's moves by `swing * 2 * rate / sr`.
        let swing = if self.pv.idx("shape") == 1 { d * sr / (2.0 * rate) } else { d * sr / (std::f64::consts::PI * rate) };
        (swing as f32).min(self.max_delay())
    }

    fn max_delay(&self) -> f32 {
        self.lines.first().map_or(0.0, DelayLine::max_delay)
    }

    /// LFO value of `shape` at phase `p` (cycles).
    fn shape_value(shape: usize, p: f64) -> f32 {
        if shape == 1 { uni_tri(p) } else { uni_sine(p) }
    }
}

impl AudioEffect for Vibrato {
    param_plumbing!("vibrato");
    /// The input sounds until the delay has passed it. The delay reaches at most the new curve's
    /// swing plus the last delay, so the tail covers both.
    fn tail_samples(&self) -> usize {
        (self.swing_len() + self.last).min(self.max_delay()).ceil() as usize + 1
    }
    fn reset(&mut self) {
        self.lines.iter_mut().for_each(DelayLine::reset);
        self.lfo.reset();
        self.shape = self.pv.idx("shape");
        self.swing = self.swing_len();
        self.last = 0.0;
        self.glide = 0.0;
        self.glide_left = 0;
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        if n == 0 {
            // No sample uses the new settings, so no glide starts.
            return;
        }
        let nch = channels.len().min(self.lines.len());
        let rate = self.pv.v("rate");
        let shape = self.pv.idx("shape");
        let swing = self.swing_len();
        if shape != self.shape || swing != self.swing {
            self.glide = self.last - swing * Self::shape_value(shape, self.lfo.phase(0.0));
            self.glide_len = ((self.glide.abs() / VIBRATO_GLIDE_SLOPE).ceil() as usize).max(1);
            self.glide_left = if self.glide == 0.0 { 0 } else { self.glide_len };
            self.shape = shape;
            self.swing = swing;
        }
        let max = self.max_delay();
        for i in 0..n {
            let mut d = swing * Self::shape_value(shape, self.lfo.phase(0.0));
            if self.glide_left > 0 {
                d = (d + self.glide * self.glide_left as f32 / self.glide_len as f32).clamp(0.0, max);
                self.glide_left -= 1;
            }
            self.last = d;
            self.lfo.advance(rate, self.sr);
            // The read interpolates in f64, so neighboring samples near the f32 limit with opposite
            // signs do not overflow.
            let d = if d.is_nan() { 0.0 } else { d.clamp(0.0, max) };
            let (di, frac) = (d.floor() as usize, f64::from(d - d.floor()));
            for (ch, line) in self.lines.iter_mut().enumerate() {
                // A non-finite sample is taken as silence. A channel the block leaves out is fed
                // silence, so it holds none of its earlier audio when it comes back.
                let dry = match channels.get(ch).filter(|_| ch < nch) {
                    Some(c) if c[i].is_finite() => c[i],
                    _ => 0.0,
                };
                line.push(dry);
                if ch < nch {
                    let (a, b) = (f64::from(line.tap(di)), f64::from(line.tap(di + 1)));
                    channels[ch][i] = (a + (b - a) * frac) as f32;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    const SR: f32 = 48000.0;

    fn ring(params: &[(&str, f32)], channels: usize) -> RingMod {
        let mut r = RingMod::new(SR, channels);
        for (id, v) in params {
            assert!(r.set_param(id, *v), "{id}");
        }
        r.reset();
        r
    }

    /// Ring moves a tone to the sum and the difference of its frequency and the carrier's, each
    /// at half the tone's amplitude, and leaves nothing at the tone's own frequency.
    #[test]
    fn ring_moves_a_tone_to_the_sum_and_difference_frequencies() {
        let mut r = ring(&[("frequency", 300.0)], 1);
        let mut x = sine(1000.0, 0.5, SR as f64, SR as usize, 0.0);
        r.process(&mut [&mut x]);
        for (f, want) in [(700.0, 0.25), (1300.0, 0.25), (1000.0, 0.0), (300.0, 0.0)] {
            let a = tone_amplitude(&x, f, SR as f64);
            assert!((a - want).abs() < 0.005, "{f} Hz: {a}, want {want}");
        }
    }

    /// Resonator + repeats an impulse once per period of its frequency, each repeat lower by the
    /// resonance, and Resonator - flips the sign of each repeat.
    #[test]
    fn resonators_repeat_an_impulse_once_per_period() {
        for (effect, sign) in [(2.0, 1.0f32), (3.0, -1.0)] {
            let mut r = ring(&[("effect", effect), ("frequency", 500.0), ("resonance", 80.0)], 1);
            let mut x = vec![0.0f32; 1000];
            x[10] = 1.0;
            r.process(&mut [&mut x]);
            // 96 samples per period at 500 Hz.
            for k in 0..8 {
                let i = 10 + 96 * k;
                let want = 0.2 * (sign * 0.8).powi(k as i32);
                assert!((x[i] - want).abs() < 1e-5, "effect {effect}, repeat {k} at {i}: {}, want {want}", x[i]);
            }
            let others = x.iter().enumerate().filter(|(i, _)| *i < 10 || (*i - 10) % 96 != 0).fold(0.0f32, |m, (_, v)| m.max(v.abs()));
            assert!(others < 1e-5, "effect {effect}: {others} between the repeats");
        }
    }

    /// Frequency Mod delays the input by 2 samples to 2 samples plus 2 ms, swinging across the
    /// whole range at the carrier's rate.
    #[test]
    fn frequency_mod_swings_the_delay_by_two_ms() {
        let mut r = ring(&[("effect", 1.0), ("frequency", 440.0)], 1);
        let slope = 1e-4f32;
        let mut x: Vec<f32> = (0..4800).map(|i| i as f32 * slope).collect();
        r.process(&mut [&mut x]);
        let delays: Vec<f32> = x.iter().enumerate().skip(200).map(|(i, y)| (i as f32 * slope - y) / slope).collect();
        let (lo, hi) = delays.iter().fold((f32::MAX, f32::MIN), |(lo, hi), d| (lo.min(*d), hi.max(*d)));
        assert!((1.99..2.1).contains(&lo), "shortest delay {lo}");
        assert!((97.9..=98.01).contains(&hi), "longest delay {hi}");
    }

    /// With the envelope as the source, a steady input at full level moves the frequency up by
    /// the depth: two octaves at 100 %.
    #[test]
    fn envelope_moves_the_frequency_up_with_the_level() {
        let mut r = ring(&[("frequency", 100.0), ("mod_source", 1.0), ("mod_depth", 100.0)], 1);
        let n = SR as usize;
        let mut x = vec![1.0f32; n];
        r.process(&mut [&mut x]);
        let tail = &x[n / 2..];
        assert!(tone_amplitude(tail, 400.0, SR as f64) > 0.99, "400 Hz: {}", tone_amplitude(tail, 400.0, SR as f64));
        assert!(tone_amplitude(tail, 100.0, SR as f64) < 0.01, "100 Hz: {}", tone_amplitude(tail, 100.0, SR as f64));
    }

    /// A resonator's tail covers the longest period the modulation reaches: four times as long at
    /// full depth with the LFO or sample and hold, which move the frequency down two octaves, and
    /// unchanged with the envelope, which moves it up only.
    #[test]
    fn resonator_tail_follows_the_lowest_frequency_the_modulation_reaches() {
        let tail = |source: f32, depth: f32| {
            ring(&[("effect", 2.0), ("frequency", 200.0), ("resonance", 90.0), ("mod_source", source), ("mod_depth", depth)], 1).tail_samples() as f32
        };
        let still = tail(0.0, 0.0);
        for source in [0.0, 2.0] {
            let ratio = tail(source, 100.0) / still;
            assert!((ratio - 4.0).abs() < 0.01, "source {source}: {ratio}");
        }
        assert_eq!(tail(1.0, 100.0), still);
    }

    /// While the resonance ramps down to a new setting, the tail still covers the higher value,
    /// and it shrinks to the new setting's tail once the ramp has ended.
    #[test]
    fn resonator_tail_covers_a_resonance_still_ramping_down() {
        let mut r = ring(&[("effect", 2.0), ("frequency", 200.0), ("resonance", 95.0)], 1);
        let high = r.tail_samples();
        assert!(r.set_param("resonance", 0.0));
        assert_eq!(r.tail_samples(), high);
        let mut x = vec![0.0f32; 4800];
        r.process(&mut [&mut x]);
        let low = ring(&[("effect", 2.0), ("frequency", 200.0), ("resonance", 0.0)], 1).tail_samples();
        assert_eq!(r.tail_samples(), low);
        assert!(low * 10 < high, "{low} vs {high}");
    }

    /// At a low sample rate the resonators ring at the highest frequency processing reaches, not
    /// at the setting above it, and the tail covers that ring. A setting below that frequency
    /// keeps the tail of its own period.
    #[test]
    fn resonator_tail_covers_a_frequency_held_at_the_rate_limit() {
        let sr = 1000.0f32;
        let mut r = RingMod::new(sr, 1);
        for (id, v) in [("effect", 2.0), ("frequency", 5000.0), ("resonance", 95.0)] {
            assert!(r.set_param(id, v), "{id}");
        }
        r.reset();
        let tail = r.tail_samples();
        let mut x = vec![0.0f32; tail + 200];
        x[0] = 1.0;
        r.process(&mut [&mut x]);
        let after = x[tail + 1..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(after <= 1e-3 * x[0].abs(), "tail {tail}: {after} after it, against {} at the impulse", x[0]);
        let mut below = RingMod::new(sr, 1);
        for (id, v) in [("effect", 2.0), ("frequency", 400.0), ("resonance", 95.0)] {
            assert!(below.set_param(id, v), "{id}");
        }
        below.reset();
        assert_eq!(below.tail_samples(), echo_tail(1000.0 / 400.0, 0.95, sr));
    }

    /// Frequency Mod reads between two of the largest samples of opposite sign without
    /// overflowing.
    #[test]
    fn frequency_mod_reads_between_huge_samples_of_opposite_sign() {
        let mut r = ring(&[("effect", 1.0), ("frequency", 500.0)], 1);
        let mut x = vec![0.0f32; 200];
        x[6] = -f32::MAX;
        x[7] = f32::MAX;
        r.process(&mut [&mut x]);
        assert!(x.iter().all(|v| v.is_finite()), "{:?}", x.iter().position(|v| !v.is_finite()));
    }

    #[test]
    fn frequency_mod_full_mix_keeps_a_small_delayed_sample() {
        let mut r = ring(&[("effect", 1.0), ("frequency", 500.0), ("mix", 100.0)], 1);
        let mut x = vec![0.5f32; 600];
        x[500] = f32::MAX;
        r.process(&mut [&mut x]);
        assert_eq!(x[500], 0.5, "the delayed sample was lost at full mix");
    }

    /// Mixing a dry and a wet sample of opposite sign at the largest level stays finite: a mix of
    /// 0 gives the input, and every mix gives the result at a level of 1 scaled up.
    #[test]
    fn mixing_huge_samples_of_opposite_sign_stays_finite() {
        for mix in [0.0f32, 50.0, 100.0] {
            let run = |level: f32| {
                let mut r = ring(&[("frequency", 5000.0), ("mix", mix)], 1);
                let mut x = vec![level; 480];
                r.process(&mut [&mut x]);
                x
            };
            let (huge, unit) = (run(f32::MAX), run(1.0));
            assert!(huge.iter().all(|v| v.is_finite()), "mix {mix}");
            for (i, (h, u)) in huge.iter().zip(&unit).enumerate() {
                let want = *u as f64 * f32::MAX as f64;
                assert!((*h as f64 - want).abs() <= 1e-5 * f32::MAX as f64, "mix {mix}, sample {i}: {h} against {want}");
            }
            if mix == 0.0 {
                assert!(huge.iter().all(|v| *v == f32::MAX));
            }
        }
    }

    /// At a mix of 100% the output is the wet sample even when the dry sample is far larger.
    #[test]
    fn full_mix_keeps_a_small_wet_sample_beside_a_huge_dry_one() {
        let mut c = crush(&[("mix", 100.0), ("bits", 16.0), ("rate_divide", 4.0)], 1);
        let mut y = vec![0.5, f32::MAX, f32::MAX, f32::MAX, 0.25];
        c.process(&mut [&mut y]);
        assert_eq!(y[1], 0.5, "the held sample was lost at full mix: {y:?}");
    }

    /// Raising the sample-and-hold rate takes effect within one new hold, instead of after the
    /// value already held at the slow rate runs out.
    #[test]
    fn sample_and_hold_follows_a_faster_rate_at_once() {
        let mut r = ring(&[("mod_source", 2.0), ("mod_rate", 0.05), ("mod_depth", 100.0)], 1);
        let mut x = vec![0.0f32; 480];
        r.process(&mut [&mut x]);
        let first = r.held;
        assert!(r.set_param("mod_rate", 20.0));
        // A hold lasts 2,400 samples at 20 Hz and 960,000 at 0.05 Hz.
        let mut x = vec![0.0f32; 4800];
        r.process(&mut [&mut x]);
        assert_ne!(r.held, first, "the slow hold kept its value");
    }

    /// At a mix of 0 every effect passes the input through unchanged.
    #[test]
    fn mix_zero_passes_the_input_through() {
        for effect in 0..4 {
            let mut r = ring(&[("effect", effect as f32), ("mix", 0.0), ("mod_depth", 50.0)], 2);
            let x = sine(220.0, 0.5, SR as f64, 4800, 0.0);
            let (mut l, mut rr) = (x.clone(), x.clone());
            r.process(&mut [&mut l, &mut rr]);
            assert!(l == x && rr == x, "effect {effect}");
        }
    }

    /// A block that leaves a channel out feeds it silence, so a resonator's ring on that channel
    /// keeps decaying instead of waiting for the channel to come back.
    #[test]
    fn channels_a_block_leaves_out_are_fed_silence() {
        let p = [("effect", 2.0), ("frequency", 100.0), ("resonance", 90.0)];
        let (mut left_out, mut silent) = (ring(&p, 2), ring(&p, 2));
        let x = sine(220.0, 0.5, SR as f64, 4800, 0.0);
        for r in [&mut left_out, &mut silent] {
            let (mut l, mut rr) = (x.clone(), x.clone());
            r.process(&mut [&mut l, &mut rr]);
        }
        for _ in 0..10 {
            let mut l = vec![0.0f32; 480];
            left_out.process(&mut [&mut l]);
            let (mut l, mut rr) = (vec![0.0f32; 480], vec![0.0f32; 480]);
            silent.process(&mut [&mut l, &mut rr]);
        }
        let (mut la, mut ra) = (vec![0.0f32; 4800], vec![0.0f32; 4800]);
        left_out.process(&mut [&mut la, &mut ra]);
        let (mut lb, mut rb) = (vec![0.0f32; 4800], vec![0.0f32; 4800]);
        silent.process(&mut [&mut lb, &mut rb]);
        assert!(rms(&rb) > 1e-3, "the ring has already died out: {}", rms(&rb));
        let worst = ra.iter().zip(&rb).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst < 1e-6, "the left-out channel differs by {worst}");
        assert_eq!(la, lb);
    }

    /// A non-finite input sample comes out as silence and leaves no NaN in the resonator.
    #[test]
    fn a_non_finite_sample_leaves_the_state_finite() {
        for effect in 0..4 {
            let mut r = ring(&[("effect", effect as f32), ("resonance", 95.0)], 1);
            let mut x = sine(220.0, 0.5, SR as f64, 4800, 0.0);
            x[100] = f32::NAN;
            x[200] = f32::INFINITY;
            r.process(&mut [&mut x]);
            assert!(x.iter().all(|v| v.is_finite()), "effect {effect}");
        }
    }

    /// An empty block does not take up a change of effect, so a resonator switched away from and
    /// back before the next sample keeps its ring.
    #[test]
    fn an_empty_block_keeps_the_resonator_ring() {
        let p = [("effect", 2.0), ("frequency", 500.0), ("resonance", 80.0)];
        let (mut r, mut still) = (ring(&p, 1), ring(&p, 1));
        for f in [&mut r, &mut still] {
            let mut x = vec![0.0f32; 50];
            x[10] = 1.0;
            f.process(&mut [&mut x]);
        }
        assert!(r.set_param("effect", 0.0));
        let mut empty: [f32; 0] = [];
        r.process(&mut [&mut empty[..]]);
        r.process(&mut []);
        assert!(r.set_param("effect", 2.0));
        let (mut a, mut b) = (vec![0.0f32; 500], vec![0.0f32; 500]);
        r.process(&mut [&mut a]);
        still.process(&mut [&mut b]);
        assert!(rms(&b) > 1e-3, "the ring has already died out");
        assert_eq!(a, b);
    }

    /// Neighboring samples at the f32 limit with opposite signs come out finite from every effect
    /// at every mix, through Frequency Mod's fractional and whole-sample reads, and a mix of 0
    /// passes them through unchanged.
    #[test]
    fn samples_at_the_f32_limit_stay_finite() {
        let edge: Vec<f32> = (0..4800).map(|i| if i % 2 == 0 { f32::MAX } else { -f32::MAX }).collect();
        for effect in 0..4 {
            for mix in [0.0, 50.0, 100.0] {
                let mut r = ring(&[("effect", effect as f32), ("frequency", 5000.0), ("mix", mix)], 1);
                let mut y = edge.clone();
                r.process(&mut [&mut y]);
                assert!(y.iter().all(|v| v.is_finite()), "effect {effect}, mix {mix}");
                if mix == 0.0 {
                    assert_eq!(y, edge, "effect {effect}");
                }
            }
        }
        // At 48 kHz Frequency Mod's first read is 50 samples back, a whole number, and the next
        // ones fall between samples.
        let mut r = ring(&[("effect", 1.0)], 1);
        let mut y = edge.clone();
        r.process(&mut [&mut y]);
        assert!(y[60..].iter().any(|v| v.abs() > 1e30 && v.abs() < f32::MAX), "no fractional read near the limit");
    }

    fn crush(params: &[(&str, f32)], channels: usize) -> Bitcrusher {
        let mut c = Bitcrusher::new(SR, channels);
        for (id, v) in params {
            assert!(c.set_param(id, *v), "{id}");
        }
        c.reset();
        c
    }

    /// Indices of the samples where the output changes, with the first sample counted.
    fn changes(x: &[f32]) -> Vec<usize> {
        (0..x.len()).filter(|&i| i == 0 || x[i] != x[i - 1]).collect()
    }

    /// Every output sample is the multiple of `2^(1 - bits)` nearest the input, and silence stays
    /// silence.
    #[test]
    fn bitcrusher_rounds_to_the_word_length() {
        for (bits, step) in [(1.0, 1.0f32), (3.0, 0.25), (5.5, 2f32.powf(-4.5))] {
            let mut c = crush(&[("bits", bits), ("rate_divide", 1.0)], 1);
            let x: Vec<f32> = (0..2000).map(|i| -1.0 + i as f32 / 1000.0).collect();
            let mut y = x.clone();
            c.process(&mut [&mut y]);
            for (a, b) in x.iter().zip(&y) {
                let k = b / step;
                assert!((k - k.round()).abs() < 1e-3, "bits {bits}: {b} is not a multiple of {step}");
                assert!((a - b).abs() <= step / 2.0 + 1e-6, "bits {bits}: {a} became {b}");
            }
            let mut z = vec![0.0f32; 480];
            c.process(&mut [&mut z]);
            assert!(z.iter().all(|v| *v == 0.0), "bits {bits}");
        }
    }

    /// The input is taken once every `rate_divide` samples and held: a whole value holds each
    /// sample for exactly that many samples, and 2.5 alternates holds of 3 and 2.
    #[test]
    fn bitcrusher_holds_each_sample_for_the_rate_divide() {
        let x: Vec<f32> = (0..200).map(|i| (i + 1) as f32 / 1024.0).collect();
        for (divide, want) in [(1.0, vec![1usize]), (5.0, vec![5]), (2.5, vec![3, 2])] {
            let mut c = crush(&[("bits", 16.0), ("rate_divide", divide)], 1);
            let mut y = x.clone();
            c.process(&mut [&mut y]);
            let mut expected = vec![0usize];
            while let Some(&last) = expected.last() {
                let next = last + want[(expected.len() - 1) % want.len()];
                if next >= x.len() {
                    break;
                }
                expected.push(next);
            }
            assert_eq!(changes(&y), expected, "divide {divide}");
            assert!(expected.iter().all(|&i| y[i] == x[i]), "divide {divide}: a hold does not start with the input");
        }
    }

    /// A shorter rate divide cuts the hold in progress short, and the holds after it have the new
    /// length from the first one on.
    #[test]
    fn bitcrusher_follows_a_shorter_hold_at_once() {
        let mut c = crush(&[("bits", 16.0), ("rate_divide", 50.0)], 1);
        let mut a: Vec<f32> = (0..10).map(|i| (i + 1) as f32 / 1024.0).collect();
        c.process(&mut [&mut a]);
        assert!(c.set_param("rate_divide", 2.0));
        let input: Vec<f32> = (10..40).map(|i| (i + 1) as f32 / 1024.0).collect();
        let mut b = input.clone();
        c.process(&mut [&mut b]);
        // `changes` counts the first sample whatever it holds, so the capture is checked apart.
        assert_eq!(b[0], input[0], "the hold in progress was not cut short");
        assert_eq!(changes(&b), (0..30).step_by(2).collect::<Vec<_>>());
        assert!((0..30).step_by(2).all(|i| b[i] == input[i]));
    }

    /// A new Bits rounds the sample being held again from the next sample on, from the input it
    /// took rather than from its rounded value, and the hold keeps its length.
    #[test]
    fn bitcrusher_rounds_the_hold_in_progress_at_new_bits() {
        let mut c = crush(&[("bits", 16.0), ("rate_divide", 4.0)], 1);
        let mut a = [0.3f32];
        c.process(&mut [&mut a]);
        assert!((a[0] - 0.3).abs() < 1e-4);
        assert!(c.set_param("bits", 1.0));
        let mut b = [0.9f32];
        c.process(&mut [&mut b]);
        assert_eq!(b[0], 0.0, "1 bit did not round the held 0.3 to 0");
        assert!(c.set_param("bits", 16.0));
        let mut d = [0.9f32, 0.9, 0.7];
        c.process(&mut [&mut d]);
        assert!((d[0] - 0.3).abs() < 1e-4 && d[0] == d[1], "the held input was lost: {d:?}");
        assert_eq!(changes(&[a[0], b[0], d[0], d[1], d[2]]), vec![0, 1, 2, 4], "the hold did not keep its length");
    }

    /// A channel left out of one block of a single sample, between two captures, holds silence
    /// when it comes back instead of the sample it held before.
    #[test]
    fn bitcrusher_clears_a_channel_left_out_for_one_sample() {
        let mut c = crush(&[("bits", 8.0), ("rate_divide", 4.0)], 2);
        let (mut l, mut r) = ([1.0f32], [1.0f32]);
        c.process(&mut [&mut l, &mut r]);
        assert_eq!(r[0], 1.0);
        let mut l = [0.0f32];
        c.process(&mut [&mut l]);
        let (mut l, mut r) = ([0.0f32; 2], [0.0f32; 2]);
        c.process(&mut [&mut l, &mut r]);
        assert_eq!(r, [0.0, 0.0], "the channel came back with its earlier hold");
        assert_eq!(l, [1.0, 1.0], "the channel kept in the block lost its hold");
    }

    /// A held sample sounds for no longer than the tail reports after the input ends.
    #[test]
    fn bitcrusher_tail_covers_the_hold() {
        for divide in [1.0, 4.0, 7.5, 50.0] {
            let mut c = crush(&[("bits", 4.0), ("rate_divide", divide)], 1);
            let tail = c.tail_samples();
            let mut x = vec![0.0f32; 200];
            x[0] = 0.5;
            c.process(&mut [&mut x]);
            let last = x.iter().rposition(|v| *v != 0.0).unwrap_or(0);
            assert_eq!(last, tail, "divide {divide}");
        }
    }

    /// A block that leaves a channel out feeds it silence, so the channel holds none of its earlier
    /// audio when it comes back.
    #[test]
    fn bitcrusher_feeds_channels_a_block_leaves_out_silence() {
        let p = [("bits", 6.0), ("rate_divide", 7.0)];
        let (mut left_out, mut silent) = (crush(&p, 2), crush(&p, 2));
        let x = sine(220.0, 0.5, SR as f64, 4801, 0.0);
        for c in [&mut left_out, &mut silent] {
            let (mut l, mut r) = (x.clone(), x.clone());
            c.process(&mut [&mut l, &mut r]);
        }
        let mut l = x[..10].to_vec();
        left_out.process(&mut [&mut l]);
        let (mut l, mut r) = (x[..10].to_vec(), vec![0.0f32; 10]);
        silent.process(&mut [&mut l, &mut r]);
        let (mut la, mut ra) = (vec![0.0f32; 480], vec![0.0f32; 480]);
        left_out.process(&mut [&mut la, &mut ra]);
        let (mut lb, mut rb) = (vec![0.0f32; 480], vec![0.0f32; 480]);
        silent.process(&mut [&mut lb, &mut rb]);
        assert_eq!(ra, rb);
        assert_eq!(la, lb);
    }

    /// At a mix of 0 the input passes through unchanged, and a non-finite sample comes out finite.
    #[test]
    fn bitcrusher_mix_zero_and_non_finite_input() {
        let x = sine(220.0, 0.5, SR as f64, 4800, 0.0);
        let mut c = crush(&[("mix", 0.0), ("bits", 2.0), ("rate_divide", 9.0)], 2);
        let (mut l, mut r) = (x.clone(), x.clone());
        c.process(&mut [&mut l, &mut r]);
        assert!(l == x && r == x);
        let mut c = crush(&[("bits", 2.0), ("rate_divide", 3.0)], 1);
        let mut y = x.clone();
        y[99] = f32::NAN;
        y[200] = f32::INFINITY;
        c.process(&mut [&mut y]);
        assert!(y.iter().all(|v| v.is_finite()));
    }

    /// Finite samples at the f32 limit, held while the input swings to the other limit, come out
    /// finite at every mix.
    #[test]
    fn bitcrusher_keeps_samples_at_the_f32_limit_finite() {
        for mix in [0.0, 50.0, 100.0] {
            for bits in [1.0, 7.3, 16.0] {
                let mut c = crush(&[("mix", mix), ("bits", bits), ("rate_divide", 3.0)], 1);
                let mut y: Vec<f32> = (0..30).map(|i| if i % 2 == 0 { f32::MAX } else { -f32::MAX }).collect();
                c.process(&mut [&mut y]);
                assert!(y.iter().all(|v| v.is_finite()), "mix {mix}, bits {bits}: {y:?}");
            }
        }
    }

    fn tremolo(params: &[(&str, f32)], channels: usize) -> Tremolo {
        let mut t = Tremolo::new(SR, channels);
        for (id, v) in params {
            assert!(t.set_param(id, *v), "{id}");
        }
        t.reset();
        t
    }

    /// The gain of a tremolo over `n` samples, read from a constant input of 1.
    fn gains(t: &mut Tremolo, n: usize) -> Vec<f32> {
        let mut x = vec![1.0f32; n];
        t.process(&mut [&mut x]);
        x
    }

    /// At full depth the gain is 1 at the start of each cycle and 0 at its middle. The sine and the
    /// triangle follow their curves, the square holds each level for most of half a cycle, and
    /// half depth swings the gain between 1 and 1/2.
    #[test]
    fn tremolo_gain_follows_the_shape() {
        let period = 9600;
        for shape in 0..3 {
            let mut t = tremolo(&[("rate", 5.0), ("depth", 100.0), ("shape", shape as f32)], 1);
            let g = gains(&mut t, 2 * period);
            for (i, v) in g.iter().enumerate() {
                let p = (i % period) as f64 / period as f64;
                let want = match shape {
                    0 => 0.5 + 0.5 * (TAU * p).cos(),
                    1 => 1.0 - 2.0 * p.min(1.0 - p),
                    _ if !(0.24..=0.26).contains(&p) && !(0.74..=0.76).contains(&p) => {
                        if (0.25..0.75).contains(&p) {
                            0.0
                        } else {
                            1.0
                        }
                    }
                    _ => continue,
                };
                assert!((f64::from(*v) - want).abs() < 1e-4, "shape {shape} at {i}: {v}, want {want}");
            }
            let mut t = tremolo(&[("rate", 5.0), ("depth", 50.0), ("shape", shape as f32)], 1);
            let g = gains(&mut t, period);
            let (lo, hi) = g.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            assert!((lo - 0.5).abs() < 1e-4 && (hi - 1.0).abs() < 1e-6, "shape {shape}: {lo} ..= {hi}");
        }
    }

    /// The square's edges are 2 ms raised-cosine ramps at every rate, centered on the quarter and
    /// three-quarter points of the cycle.
    #[test]
    fn tremolo_square_edges_do_not_click() {
        let edge = (0.002 * SR) as usize;
        for rate in [0.1f32, 1.0, 20.0] {
            let mut t = tremolo(&[("rate", rate), ("depth", 100.0), ("shape", 2.0)], 1);
            let g = gains(&mut t, (SR / rate) as usize);
            let worst = g.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
            // A raised cosine over `edge` samples moves at most pi / 2 / edge per sample.
            assert!(worst <= std::f32::consts::FRAC_PI_2 / edge as f32 * 1.01, "rate {rate}: step {worst}");
            let moving = g.windows(2).filter(|w| w[1] != w[0]).count();
            assert!(moving <= 2 * edge + 2, "rate {rate}: the gain moves on {moving} samples");
            let quarter = (SR / rate / 4.0) as usize;
            assert!((g[quarter] - 0.5).abs() < 0.02, "rate {rate}: {} at the quarter point", g[quarter]);
        }
    }

    /// A new rate takes effect from the next sample, with the phase kept.
    #[test]
    fn tremolo_rate_change_keeps_the_phase() {
        let mut t = tremolo(&[("rate", 5.0), ("depth", 100.0)], 1);
        let a = gains(&mut t, 2400);
        assert!(t.set_param("rate", 10.0));
        let b = gains(&mut t, 2400);
        // A quarter of a cycle at 5 Hz, then a quarter at 10 Hz ends at the middle of the cycle.
        assert!((a[2399] - 0.5).abs() < 1e-3 && (b[0] - a[2399]).abs() < 1e-3, "{} then {}", a[2399], b[0]);
        assert!(b[1199] < 1e-3 && b[1201] < 1e-3, "{} {}", b[1199], b[1201]);
    }

    /// A change of shape glides from where the old shape was to the new one over 20 ms, so the
    /// gain does not jump, and the new shape is followed exactly afterward.
    #[test]
    fn tremolo_shape_change_glides() {
        let glide = (0.02 * SR) as usize;
        let mut t = tremolo(&[("rate", 5.0), ("depth", 100.0), ("shape", 0.0)], 1);
        let mut s = tremolo(&[("rate", 5.0), ("depth", 100.0), ("shape", 2.0)], 1);
        // 2,000 samples in, the sine has the gain at 0.63 and the square at 1.
        let a = gains(&mut t, 2000);
        gains(&mut s, 2000);
        assert!(t.set_param("shape", 2.0));
        let b = gains(&mut t, 2 * glide);
        let want = gains(&mut s, 2 * glide);
        // The square's own edge moves the gain by up to pi / 2 / 96 (0.0164) per sample.
        let step = (b[0] - a[1999]).abs();
        assert!(step < 0.02, "the gain jumped by {step}");
        assert!(b.windows(2).all(|w| (w[1] - w[0]).abs() < 0.02), "the glide jumped");
        assert_eq!(b[glide..], want[glide..]);
        assert!((b[0] - want[0]).abs() > 0.2, "{} {}", b[0], want[0]);
    }

    /// A shape that only an empty block saw does not restart a glide in progress: the glide goes
    /// on as if the shape had not changed.
    #[test]
    fn tremolo_empty_block_keeps_the_glide() {
        let glide = (0.02 * SR) as usize;
        let mut t = tremolo(&[("rate", 5.0), ("depth", 100.0), ("shape", 0.0)], 1);
        let mut s = tremolo(&[("rate", 5.0), ("depth", 100.0), ("shape", 0.0)], 1);
        for fx in [&mut t, &mut s] {
            gains(fx, 2000);
            assert!(fx.set_param("shape", 2.0));
            gains(fx, 100);
        }
        assert!(t.set_param("shape", 1.0));
        let mut empty: [f32; 0] = [];
        t.process(&mut [&mut empty[..]]);
        assert!(t.set_param("shape", 2.0));
        assert_eq!(gains(&mut t, 2 * glide), gains(&mut s, 2 * glide));
    }

    /// A new rate moves the square's edges to a new share of the cycle. Changed just before an
    /// edge, from 0.1 Hz (where the edge is still ahead) to 20 Hz (where the phase is inside it),
    /// the gain glides instead of jumping.
    #[test]
    fn tremolo_square_rate_change_glides() {
        let mut t = tremolo(&[("rate", 0.1), ("depth", 100.0), ("shape", 2.0)], 1);
        // Phase 0.249 at 0.1 Hz.
        let a = gains(&mut t, 119_520);
        assert_eq!(a[119_519], 1.0);
        assert!(t.set_param("rate", 20.0));
        let b = gains(&mut t, 2000);
        let step = (b[0] - a[119_519]).abs();
        assert!(step < 0.02, "the gain jumped by {step}");
        assert!(b.windows(2).all(|w| (w[1] - w[0]).abs() < 0.02), "the glide jumped");
    }

    /// At zero depth the input passes unchanged, a non-finite sample comes out as silence, and a
    /// channel past the configured count is left alone.
    #[test]
    fn tremolo_depth_zero_non_finite_and_extra_channels() {
        let x = sine(220.0, 0.5, SR as f64, 4800, 0.0);
        let mut t = tremolo(&[("depth", 0.0), ("shape", 2.0)], 2);
        let (mut l, mut r) = (x.clone(), x.clone());
        t.process(&mut [&mut l, &mut r]);
        assert!(l == x && r == x);
        let mut t = tremolo(&[("depth", 100.0)], 1);
        let (mut l, mut r) = (x.clone(), x.clone());
        l[99] = f32::NAN;
        l[200] = f32::INFINITY;
        t.process(&mut [&mut l, &mut r]);
        assert!(l[99] == 0.0 && l[200] == 0.0 && l.iter().all(|v| v.is_finite()));
        assert_eq!(r, x);
    }

    /// A glide keeps the LFO value within 0 ..= 1, so the gain stays between 1 - depth and 1 and
    /// never inverts the input, even when the new curve moves away from the glide's offset.
    #[test]
    fn tremolo_glide_keeps_the_gain_in_range() {
        for (from, to) in [(1.0, 2.0), (2.0, 1.0), (0.0, 2.0), (2.0, 0.0)] {
            for start in [240, 480, 1680, 1900] {
                let mut t = tremolo(&[("rate", 20.0), ("depth", 100.0), ("shape", from)], 1);
                gains(&mut t, start);
                assert!(t.set_param("shape", to));
                let g = gains(&mut t, 2400);
                assert!(g.iter().all(|v| (0.0..=1.0).contains(v)), "{from} to {to} at {start}: {g:?}");
            }
        }
    }

    /// An empty block does not take up a change of shape, so a shape changed and restored before
    /// the next sample starts no glide.
    #[test]
    fn tremolo_empty_block_starts_no_glide() {
        let p = [("rate", 5.0), ("depth", 100.0), ("shape", 0.0)];
        let (mut t, mut still) = (tremolo(&p, 1), tremolo(&p, 1));
        gains(&mut t, 1000);
        gains(&mut still, 1000);
        assert!(t.set_param("shape", 1.0));
        let mut empty: [f32; 0] = [];
        t.process(&mut [&mut empty[..]]);
        t.process(&mut []);
        assert!(t.set_param("shape", 0.0));
        assert_eq!(gains(&mut t, 2000), gains(&mut still, 2000));
    }

    fn vibrato(params: &[(&str, f32)], channels: usize) -> Vibrato {
        let mut v = Vibrato::new(SR, channels);
        for (id, x) in params {
            assert!(v.set_param(id, *x), "{id}");
        }
        v.reset();
        v
    }

    /// Slope of the ramp the delay tests feed in, per sample.
    const RAMP: f32 = 1e-3;
    /// Most the delay moves per sample during a glide at a depth of 100 cents: the curve's
    /// steepest slope plus the glide's, with room for the rounding of the ramp.
    const GLIDE_STEP: f32 = 0.0562 + 1.0 / 32.0 + 0.002;

    /// The delay of a vibrato over `n` samples, read from a ramp that starts at `start` samples.
    fn delays(v: &mut Vibrato, start: usize, n: usize) -> Vec<f32> {
        let x: Vec<f32> = (start..start + n).map(|i| i as f32 * RAMP).collect();
        let mut y = x.clone();
        v.process(&mut [&mut y]);
        x.iter().zip(&y).map(|(a, b)| (a - b) / RAMP).collect()
    }

    /// Largest change of the delay between neighboring samples, averaged over 2 * `k` samples to
    /// smooth out the rounding of the ramp.
    fn steepest(d: &[f32], k: usize) -> f32 {
        (k..d.len() - k).map(|i| (d[i + k] - d[i - k]).abs() / (2 * k) as f32).fold(0.0, f32::max)
    }

    /// The delay is 0 at the start of each cycle and follows the sine or the triangle, and its
    /// steepest slope lowers the pitch by the depth: 100 cents at a depth of 100 for both shapes.
    #[test]
    fn vibrato_delay_follows_the_shape_and_the_depth() {
        let d_max = 1.0 - 2f32.powf(-100.0 / 1200.0);
        for (shape, rate) in [(0, 6.0f32), (1, 6.0), (0, 1.0), (1, 14.0)] {
            let mut v = vibrato(&[("rate", rate), ("depth", 100.0), ("shape", shape as f32)], 1);
            let period = (SR / rate) as usize;
            let swing = v.swing;
            let d = delays(&mut v, 0, 2 * period);
            for (i, got) in d.iter().enumerate() {
                let p = (i as f64 * f64::from(rate) / f64::from(SR)).fract();
                let want = swing * if shape == 1 { uni_tri(p) } else { uni_sine(p) };
                assert!((got - want).abs() < 0.05, "shape {shape} at {rate} Hz, sample {i}: {got}, want {want}");
            }
            assert!(d[0].abs() < 0.05 && d[period].abs() < 0.05, "shape {shape}: {} {}", d[0], d[period]);
            let slope = steepest(&d, 40);
            let cents = 1200.0 * (1.0 - slope).log2();
            assert!((cents + 100.0).abs() < 2.0, "shape {shape} at {rate} Hz: the pitch falls by {cents} cents");
            assert!((slope - d_max).abs() < 0.002, "shape {shape} at {rate} Hz: slope {slope}");
        }
    }

    /// A lower depth set at the middle of the cycle, where the delay is longest, glides the delay
    /// down to the new curve without jumping, and afterward the output matches a vibrato that had
    /// the new depth from the start.
    #[test]
    fn vibrato_depth_change_glides_to_the_new_curve() {
        let half = 4000;
        let mut v = vibrato(&[("rate", 6.0), ("depth", 100.0)], 1);
        let mut w = vibrato(&[("rate", 6.0), ("depth", 10.0)], 1);
        let a = delays(&mut v, 0, half);
        delays(&mut w, 0, half);
        assert!(v.set_param("depth", 10.0));
        let b = delays(&mut v, half, 16_000);
        let want = delays(&mut w, half, 16_000);
        // The new curve moves by up to 0.0058 per sample, and the glide by 1/32 more.
        let step = (b[0] - a[half - 1]).abs();
        assert!(step < 0.04, "the delay jumped by {step}");
        assert!(b.windows(2).all(|p| (p[1] - p[0]).abs() < 0.04), "the glide moved too fast");
        assert!(b[100] > want[100] + 100.0, "{} {}", b[100], want[100]);
        let glide = v.glide_len;
        assert!(glide < b.len() / 2, "the glide has not ended: {glide}");
        assert_eq!(b[glide..], want[glide..]);
    }

    /// A new rate takes effect from the next sample with the phase kept, the delay glides to the
    /// new curve without jumping, and afterward follows it.
    #[test]
    fn vibrato_rate_change_keeps_the_phase_and_glides() {
        let mut v = vibrato(&[("rate", 6.0), ("depth", 100.0)], 1);
        // A quarter of a cycle at 6 Hz.
        let a = delays(&mut v, 0, 2000);
        assert!(v.set_param("rate", 3.0));
        let swing = v.swing_len();
        let b = delays(&mut v, 2000, 12000);
        let step = (b[0] - a[1999]).abs();
        assert!(step < GLIDE_STEP, "the delay jumped by {step}");
        assert!(b.windows(2).all(|p| (p[1] - p[0]).abs() < GLIDE_STEP), "the glide moved too fast");
        let glide = v.glide_len;
        assert!(glide < b.len() / 2, "the glide has not ended: {glide}");
        for (k, got) in b.iter().enumerate().skip(glide) {
            let p = (0.25 + k as f64 * 3.0 / f64::from(SR)).fract();
            let want = swing * uni_sine(p);
            assert!((got - want).abs() < 0.05, "sample {k}: {got}, want {want}");
        }
    }

    /// A change of shape glides from the last delay to the new curve without jumping.
    #[test]
    fn vibrato_shape_change_glides() {
        for (from, to) in [(0.0, 1.0), (1.0, 0.0)] {
            for start in [600, 2000, 4000, 7000] {
                let mut v = vibrato(&[("rate", 6.0), ("depth", 100.0), ("shape", from)], 1);
                let a = delays(&mut v, 0, start);
                assert!(v.set_param("shape", to));
                let b = delays(&mut v, start, 8000);
                assert!((b[0] - a[start - 1]).abs() < GLIDE_STEP, "{from} to {to} at {start}: jumped");
                assert!(b.windows(2).all(|p| (p[1] - p[0]).abs() < GLIDE_STEP), "{from} to {to} at {start}: the glide moved too fast");
                assert!(b.iter().all(|d| *d > -0.05), "{from} to {to} at {start}: the delay went below 0");
            }
        }
    }

    /// An empty block does not take up a change of depth, so a depth changed and restored before
    /// the next sample starts no glide.
    #[test]
    fn vibrato_empty_block_starts_no_glide() {
        let p = [("rate", 6.0), ("depth", 100.0)];
        let (mut v, mut still) = (vibrato(&p, 1), vibrato(&p, 1));
        delays(&mut v, 0, 1000);
        delays(&mut still, 0, 1000);
        assert!(v.set_param("depth", 10.0));
        let mut empty: [f32; 0] = [];
        v.process(&mut [&mut empty[..]]);
        v.process(&mut []);
        assert!(v.set_param("depth", 100.0));
        assert_eq!(delays(&mut v, 1000, 4000), delays(&mut still, 1000, 4000));
    }

    /// After the input ends, the output falls silent within the reported tail, for every shape and
    /// for a depth lowered just before the end, while the delay is still gliding down.
    #[test]
    fn vibrato_tail_covers_the_delay() {
        for (rate, depth, shape, lower) in
            [(1.0f32, 100.0f32, 1.0f32, false), (14.0, 50.0, 0.0, false), (1.0, 100.0, 1.0, true), (2.0, 100.0, 0.0, true)]
        {
            for len in [5000usize, 12000, 24000] {
                let mut v = vibrato(&[("rate", rate), ("depth", depth), ("shape", shape)], 1);
                let mut x = vec![0.5f32; len];
                v.process(&mut [&mut x]);
                if lower {
                    assert!(v.set_param("depth", 0.0));
                }
                let tail = v.tail_samples();
                let mut z = vec![0.0f32; 96_000];
                v.process(&mut [&mut z]);
                let last = z.iter().rposition(|s| *s != 0.0).map_or(0, |i| i + 1);
                assert!(last <= tail, "{rate} Hz {depth} ct shape {shape} lower {lower} len {len}: sound until {last} after a tail of {tail}");
            }
        }
    }

    /// At zero depth the input passes unchanged, a non-finite sample comes out as silence, and a
    /// channel past the configured count is left alone.
    #[test]
    fn vibrato_depth_zero_non_finite_and_extra_channels() {
        let x = sine(220.0, 0.5, SR as f64, 4800, 0.0);
        let mut v = vibrato(&[("depth", 0.0), ("shape", 1.0)], 2);
        let (mut l, mut r) = (x.clone(), x.clone());
        v.process(&mut [&mut l, &mut r]);
        assert!(l == x && r == x);
        let mut v = vibrato(&[("depth", 0.0)], 1);
        let (mut l, mut r) = (x.clone(), x.clone());
        l[99] = f32::NAN;
        l[200] = f32::INFINITY;
        v.process(&mut [&mut l, &mut r]);
        assert!(l[99] == 0.0 && l[200] == 0.0 && l.iter().all(|s| s.is_finite()));
        assert_eq!(r, x);
        let mut v = vibrato(&[("depth", 100.0)], 1);
        let mut l = x.clone();
        l[99] = f32::NAN;
        v.process(&mut [&mut l]);
        assert!(l.iter().all(|s| s.is_finite()));
        // Neighboring samples at the f32 limit with opposite signs stay finite.
        let edge: Vec<f32> = (0..4800).map(|i| if i % 2 == 0 { f32::MAX } else { -f32::MAX }).collect();
        for depth in [0.0, 100.0] {
            let mut v = vibrato(&[("depth", depth)], 1);
            let mut l = edge.clone();
            v.process(&mut [&mut l]);
            assert!(l.iter().all(|s| s.is_finite()), "depth {depth}");
            if depth == 0.0 {
                assert_eq!(l, edge);
            }
        }
    }

    /// A block that leaves a channel out feeds it silence, so the channel holds none of its earlier
    /// audio when it comes back.
    #[test]
    fn vibrato_feeds_channels_a_block_leaves_out_silence() {
        let p = [("rate", 1.0), ("depth", 100.0), ("shape", 1.0)];
        let (mut left_out, mut silent) = (vibrato(&p, 2), vibrato(&p, 2));
        let x = sine(220.0, 0.5, SR as f64, 12_000, 0.0);
        for v in [&mut left_out, &mut silent] {
            let (mut l, mut r) = (x.clone(), x.clone());
            v.process(&mut [&mut l, &mut r]);
        }
        let mut l = x[..2000].to_vec();
        left_out.process(&mut [&mut l]);
        let (mut l, mut r) = (x[..2000].to_vec(), vec![0.0f32; 2000]);
        silent.process(&mut [&mut l, &mut r]);
        let (mut la, mut ra) = (x[..4800].to_vec(), x[..4800].to_vec());
        left_out.process(&mut [&mut la, &mut ra]);
        let (mut lb, mut rb) = (x[..4800].to_vec(), x[..4800].to_vec());
        silent.process(&mut [&mut lb, &mut rb]);
        assert_eq!(ra, rb);
        assert_eq!(la, lb);
    }
}
