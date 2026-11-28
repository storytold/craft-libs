//! Dynamics: compressor, expander/gate, look-ahead true-peak limiter and the maximizer.
//!
//! Detection is stereo-linked (the loudest channel drives one common gain) so the image does
//! not shift. Gain computers work in dB; attack/release are one-pole smoothers on the gain.
//! The compressor and the gate take a key input ([`AudioEffect::process_keyed`]): when one is
//! given, the loudest key channel drives the detector instead of the processed channels.

use super::util::{coef, lin_db};
use crate::oversample::{Interpolator, PolyphaseBank, true_peak_factor};
use crate::{AudioEffect, MAX_CHANNELS, ParamSpec, ParamValues, Smoothed, Unit, block_len, db_to_gain, param_plumbing, sane_sample_rate, usable_key};
use std::collections::VecDeque;

/// Channels of key input the compressor and the gate read.
const KEY_CHANNELS: usize = 2;

/// The detector's input for frame `i`: the loudest absolute value and the largest square over
/// the key channels when a key is given, else over the processed channels.
#[inline]
fn detect(channels: &[&mut [f32]], key: Option<&[&[f32]]>, i: usize) -> (f32, f32) {
    let mut peak = 0.0f32;
    let mut sq = 0.0f32;
    let mut take = |x: f32| {
        if x.is_finite() {
            peak = peak.max(x.abs());
            sq = sq.max(x * x);
        }
    };
    match key {
        Some(key) => key.iter().take(KEY_CHANNELS).for_each(|k| take(k[i])),
        None => channels.iter().for_each(|c| take(c[i])),
    }
    (peak, sq)
}

/// Static compressor curve with a quadratic soft knee (output level in dB for input level `x`).
pub fn compressor_curve(x: f32, threshold: f32, ratio: f32, knee: f32) -> f32 {
    let d = x - threshold;
    if knee > 0.0 && 2.0 * d.abs() <= knee {
        x + (1.0 / ratio - 1.0) * (d + knee / 2.0).powi(2) / (2.0 * knee)
    } else if d > 0.0 {
        threshold + d / ratio
    } else {
        x
    }
}

/// Static downward-expander curve (output level in dB), attenuation limited to `range` dB.
pub fn expander_curve(x: f32, threshold: f32, ratio: f32, range: f32) -> f32 {
    if x >= threshold { x } else { x + ((x - threshold) * (ratio - 1.0)).max(-range) }
}

// ---------------------------------------------------------------------------------------------

/// Feed-forward compressor (peak or RMS detector, soft knee, attack/release, make-up gain).
pub struct Compressor {
    pv: ParamValues,
    sr: f32,
    threshold: f32,
    ratio: f32,
    knee: f32,
    rms: bool,
    att: f32,
    rel: f32,
    rms_coef: f32,
    makeup: Smoothed,
    /// Detector mean square (RMS mode).
    ms: f32,
    /// Smoothed gain reduction (dB, ≤ 0).
    env: f32,
    gr_meter: f32,
}

impl Compressor {
    pub const PARAMS: &'static [ParamSpec] = &[
        ParamSpec::new("threshold", "Threshold", -60.0, 0.0, -20.0, Unit::Decibels),
        ParamSpec::new("ratio", "Ratio", 1.0, 30.0, 4.0, Unit::Ratio),
        ParamSpec::new("knee", "Knee", 0.0, 24.0, 6.0, Unit::Decibels),
        ParamSpec::log("attack", "Attack", 0.01, 300.0, 10.0, Unit::Milliseconds),
        ParamSpec::log("release", "Release", 1.0, 3000.0, 100.0, Unit::Milliseconds),
        ParamSpec::new("makeup", "Make-up Gain", -40.0, 40.0, 0.0, Unit::Decibels),
        ParamSpec::choice("detector", "Detector", &["Peak", "RMS"], 0),
    ];

    pub fn new(sample_rate: f32, _channels: usize) -> Self {
        let sample_rate = sane_sample_rate(sample_rate);
        let mut s = Compressor {
            pv: ParamValues::new(Self::PARAMS),
            sr: sample_rate,
            threshold: 0.0,
            ratio: 1.0,
            knee: 0.0,
            rms: false,
            att: 0.0,
            rel: 0.0,
            rms_coef: coef(10.0, sample_rate),
            makeup: Smoothed::with_ms(1.0, sample_rate, 20.0),
            ms: 0.0,
            env: 0.0,
            gr_meter: 0.0,
        };
        s.apply_params(true);
        s
    }

    fn apply_params(&mut self, snap: bool) {
        self.threshold = self.pv.v("threshold");
        self.ratio = self.pv.v("ratio");
        self.knee = self.pv.v("knee");
        self.att = coef(self.pv.v("attack"), self.sr);
        self.rel = coef(self.pv.v("release"), self.sr);
        self.rms = self.pv.idx("detector") == 1;
        self.makeup.set(db_to_gain(self.pv.v("makeup")));
        if snap {
            self.makeup.snap();
        }
    }

    /// Static curve at the current settings.
    pub fn curve(&self, x_db: f32) -> f32 {
        compressor_curve(x_db, self.threshold, self.ratio, self.knee)
    }

    fn run(&mut self, channels: &mut [&mut [f32]], key: &[&[f32]]) {
        let n = block_len(channels);
        let key = usable_key(key, n);
        let mut deepest = 0.0f32;
        for i in 0..n {
            let (peak, sq) = detect(channels, key, i);
            let level_db = if self.rms {
                self.ms = self.rms_coef * self.ms + (1.0 - self.rms_coef) * sq;
                if !(1e-30..=f32::MAX).contains(&self.ms) {
                    self.ms = 0.0;
                }
                10.0 * self.ms.max(1e-20).log10()
            } else {
                lin_db(peak)
            };
            let target = self.curve(level_db) - level_db;
            let c = if target < self.env { self.att } else { self.rel };
            self.env = c * self.env + (1.0 - c) * target;
            if !(1e-9..=f32::MAX).contains(&self.env.abs()) {
                self.env = 0.0;
            }
            deepest = deepest.min(self.env);
            let g = db_to_gain(self.env) * self.makeup.tick();
            for ch in channels.iter_mut() {
                ch[i] *= g;
            }
        }
        if n > 0 {
            self.gr_meter = -deepest;
        }
    }
}

impl AudioEffect for Compressor {
    param_plumbing!("compressor");
    fn transfer_db(&self, band: usize, input_db: f32) -> Option<f32> {
        (band == 0).then(|| self.curve(input_db) + self.pv.v("makeup"))
    }
    fn reset(&mut self) {
        self.makeup.snap();
        self.ms = 0.0;
        self.env = 0.0;
        self.gr_meter = 0.0;
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        self.run(channels, &[]);
    }
    fn key_channels(&self) -> usize {
        KEY_CHANNELS
    }
    fn process_keyed(&mut self, channels: &mut [&mut [f32]], key: &[&[f32]]) {
        self.run(channels, key);
    }
    /// The deepest gain reduction during the last block, in dB (≥ 0).
    fn gain_reduction_db(&self) -> f32 {
        self.gr_meter
    }
}

// ---------------------------------------------------------------------------------------------

/// Downward expander / noise gate with hold (ratio ≥ 10 behaves as a gate).
pub struct Gate {
    pv: ParamValues,
    sr: f32,
    threshold: f32,
    ratio: f32,
    range: f32,
    att: f32,
    rel: f32,
    hold: u32,
    hold_left: u32,
    env: f32,
    gr_meter: f32,
}

impl Gate {
    pub const PARAMS: &'static [ParamSpec] = &[
        ParamSpec::new("threshold", "Threshold", -90.0, 0.0, -40.0, Unit::Decibels),
        ParamSpec::log("ratio", "Ratio", 1.0, 100.0, 10.0, Unit::Ratio),
        ParamSpec::new("range", "Range", 0.0, 90.0, 60.0, Unit::Decibels),
        ParamSpec::log("attack", "Attack", 0.01, 100.0, 1.0, Unit::Milliseconds),
        ParamSpec::new("hold", "Hold", 0.0, 2000.0, 50.0, Unit::Milliseconds),
        ParamSpec::log("release", "Release", 1.0, 3000.0, 150.0, Unit::Milliseconds),
    ];

    pub fn new(sample_rate: f32, _channels: usize) -> Self {
        let sample_rate = sane_sample_rate(sample_rate);
        let mut s = Gate {
            pv: ParamValues::new(Self::PARAMS),
            sr: sample_rate,
            threshold: 0.0,
            ratio: 1.0,
            range: 0.0,
            att: 0.0,
            rel: 0.0,
            hold: 0,
            hold_left: 0,
            env: 0.0,
            gr_meter: 0.0,
        };
        s.apply_params(true);
        s
    }

    fn apply_params(&mut self, _snap: bool) {
        self.threshold = self.pv.v("threshold");
        self.ratio = self.pv.v("ratio");
        self.range = self.pv.v("range");
        self.att = coef(self.pv.v("attack"), self.sr);
        self.rel = coef(self.pv.v("release"), self.sr);
        self.hold = (self.pv.v("hold") * 0.001 * self.sr) as u32;
    }

    pub fn curve(&self, x_db: f32) -> f32 {
        expander_curve(x_db, self.threshold, self.ratio, self.range)
    }
}

impl AudioEffect for Gate {
    param_plumbing!("gate");
    fn transfer_db(&self, band: usize, input_db: f32) -> Option<f32> {
        (band == 0).then(|| self.curve(input_db))
    }
    fn reset(&mut self) {
        self.env = 0.0;
        self.hold_left = 0;
        self.gr_meter = 0.0;
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        self.run(channels, &[]);
    }
    fn key_channels(&self) -> usize {
        KEY_CHANNELS
    }
    fn process_keyed(&mut self, channels: &mut [&mut [f32]], key: &[&[f32]]) {
        self.run(channels, key);
    }
    /// The deepest attenuation during the last block, in dB (≥ 0).
    fn gain_reduction_db(&self) -> f32 {
        self.gr_meter
    }
}

impl Gate {
    fn run(&mut self, channels: &mut [&mut [f32]], key: &[&[f32]]) {
        let n = block_len(channels);
        let key = usable_key(key, n);
        let mut deepest = 0.0f32;
        for i in 0..n {
            let (peak, _) = detect(channels, key, i);
            let level = lin_db(peak);
            let target = self.curve(level) - level;
            if target >= self.env {
                // Opening (or staying open): attack.
                self.env = self.att * self.env + (1.0 - self.att) * target;
                if target >= 0.0 {
                    self.hold_left = self.hold;
                }
            } else if self.hold_left > 0 {
                self.hold_left -= 1;
            } else {
                self.env = self.rel * self.env + (1.0 - self.rel) * target;
                if !self.env.is_finite() {
                    self.env = 0.0;
                }
            }
            deepest = deepest.min(self.env);
            let g = db_to_gain(self.env);
            for ch in channels.iter_mut() {
                ch[i] *= g;
            }
        }
        if n > 0 {
            self.gr_meter = -deepest;
        }
    }
}

// ---------------------------------------------------------------------------------------------

/// Look-ahead brick-wall limiter.
///
/// The required gain for each (optionally true-peak) detected sample is held for the look-ahead
/// window with a sliding minimum, released with a one-pole, and then smoothed with a moving
/// average exactly as long as the look-ahead, so the gain has reached its target when the peak
/// leaves the delay line. A final clip at the ceiling guarantees the sample peak never exceeds it.
pub struct Limiter {
    pv: ParamValues,
    sr: f32,
    in_gain: Smoothed,
    ceiling: f32,
    rel: f32,
    true_peak: bool,
    /// Look-ahead (samples) and detector alignment delay.
    look: usize,
    det_delay: usize,
    bank: PolyphaseBank,
    interp: Vec<Interpolator>,
    /// Per-channel delay line (ring), length `look + det_delay + 1`.
    delay: Vec<Vec<f32>>,
    wpos: usize,
    /// Monotonic deque for the sliding minimum: (sample index, gain).
    minq: VecDeque<(u64, f32)>,
    /// Box filter ring and running sum.
    box_ring: Vec<f32>,
    box_pos: usize,
    box_sum: f64,
    rel_state: f32,
    t: u64,
    gr_meter: f32,
}

impl Limiter {
    pub const PARAMS: &'static [ParamSpec] = &[
        ParamSpec::new("input_gain", "Input Boost", 0.0, 24.0, 0.0, Unit::Decibels),
        ParamSpec::new("ceiling", "Ceiling", -24.0, 0.0, -1.0, Unit::Decibels),
        ParamSpec::log("release", "Release", 1.0, 1000.0, 60.0, Unit::Milliseconds),
        ParamSpec::toggle("true_peak", "True Peak", true),
    ];
    /// Fixed look-ahead time.
    pub const LOOKAHEAD_MS: f32 = 5.0;

    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let sample_rate = sane_sample_rate(sample_rate);
        let channels = channels.clamp(1, MAX_CHANNELS);
        let bank = PolyphaseBank::new(true_peak_factor(sample_rate as f64).max(2));
        let look = ((Self::LOOKAHEAD_MS * 0.001 * sample_rate).round() as usize).max(1);
        let det_delay = bank.delay();
        let dlen = look + det_delay + 1;
        let mut s = Limiter {
            pv: ParamValues::new(Self::PARAMS),
            sr: sample_rate,
            in_gain: Smoothed::with_ms(1.0, sample_rate, 20.0),
            ceiling: 1.0,
            rel: 0.0,
            true_peak: true,
            look,
            det_delay,
            bank,
            interp: vec![Interpolator::default(); channels],
            delay: vec![vec![0.0; dlen]; channels],
            wpos: 0,
            minq: VecDeque::with_capacity(look + 2),
            box_ring: vec![1.0; look + 1],
            box_pos: 0,
            box_sum: (look + 1) as f64,
            rel_state: 1.0,
            t: 0,
            gr_meter: 0.0,
        };
        s.apply_params(true);
        s
    }

    fn apply_params(&mut self, snap: bool) {
        self.in_gain.set(db_to_gain(self.pv.v("input_gain")));
        self.ceiling = db_to_gain(self.pv.v("ceiling"));
        self.rel = coef(self.pv.v("release"), self.sr);
        self.true_peak = self.pv.on("true_peak");
        if snap {
            self.in_gain.snap();
        }
    }
}

impl AudioEffect for Limiter {
    param_plumbing!("limiter");
    fn latency(&self) -> usize {
        self.look + self.det_delay
    }
    /// The deepest gain reduction during the last block, in dB (≥ 0).
    fn gain_reduction_db(&self) -> f32 {
        self.gr_meter
    }
    fn reset(&mut self) {
        self.in_gain.snap();
        self.interp.iter_mut().for_each(Interpolator::reset);
        self.delay.iter_mut().for_each(|d| d.iter_mut().for_each(|v| *v = 0.0));
        self.wpos = 0;
        self.minq.clear();
        self.box_ring.iter_mut().for_each(|v| *v = 1.0);
        self.box_pos = 0;
        self.box_sum = self.box_ring.len() as f64;
        self.rel_state = 1.0;
        self.t = 0;
        self.gr_meter = 0.0;
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        let nch = channels.len().min(self.delay.len());
        let dlen = self.delay[0].len();
        let window = (self.look + 1) as u64;
        let mut min_g = None::<f32>;
        for i in 0..n {
            let g_in = self.in_gain.tick();
            let det_pos = (self.wpos + dlen - self.det_delay) % dlen;
            let out_pos = (self.wpos + 1) % dlen; // oldest = look + det_delay samples ago
            let mut det = 0.0f32;
            for ch in 0..nch {
                // A non-finite sample would pass the final clip as NaN; it becomes silence.
                let x = channels[ch][i];
                let x = if x.is_finite() { x * g_in } else { 0.0 };
                self.delay[ch][self.wpos] = x;
                let aligned = self.delay[ch][det_pos].abs();
                let tp = if self.true_peak { self.interp[ch].push_peak(&self.bank, x) } else { 0.0 };
                det = det.max(aligned.max(tp));
            }
            let greq = if det > self.ceiling { self.ceiling / det } else { 1.0 };
            // Sliding minimum over the last `window` values.
            while let Some(&(_, v)) = self.minq.back() {
                if v >= greq {
                    self.minq.pop_back();
                } else {
                    break;
                }
            }
            self.minq.push_back((self.t, greq));
            while let Some(&(idx, _)) = self.minq.front() {
                if idx + window <= self.t {
                    self.minq.pop_front();
                } else {
                    break;
                }
            }
            let m = self.minq.front().map_or(1.0, |&(_, v)| v);
            // Release.
            self.rel_state = if m < self.rel_state { m } else { self.rel_state + (m - self.rel_state) * (1.0 - self.rel) };
            // Moving average over look+1 samples.
            self.box_sum += self.rel_state as f64 - self.box_ring[self.box_pos] as f64;
            self.box_ring[self.box_pos] = self.rel_state;
            self.box_pos = (self.box_pos + 1) % self.box_ring.len();
            let g = (self.box_sum / self.box_ring.len() as f64) as f32;
            for ch in 0..nch {
                let y = self.delay[ch][out_pos] * g;
                channels[ch][i] = y.clamp(-self.ceiling, self.ceiling);
            }
            self.wpos = (self.wpos + 1) % dlen;
            self.t += 1;
            // Periodically re-sum to cancel floating-point drift.
            if self.t.is_multiple_of(65536) {
                self.box_sum = self.box_ring.iter().map(|&v| v as f64).sum();
            }
            min_g = Some(min_g.map_or(g, |m| m.min(g)));
        }
        if let Some(g) = min_g {
            self.gr_meter = -lin_db(g).min(0.0);
        }
    }
}

// ---------------------------------------------------------------------------------------------

/// Look-ahead of the maximizer.
const MAXIMIZER_LOOKAHEAD_MS: f32 = 1.5;

/// Loudness maximizer: a look-ahead brick-wall limiter driven by `-threshold` dB, so lowering
/// the threshold raises the level into the ceiling. The gain is the minimum required gain over
/// the look-ahead window, released with a one-pole and smoothed by a moving average as long as
/// the window, so it has ramped down when each peak leaves the delay line. The output never
/// exceeds the ceiling; the ceiling changes at once, the drive glides over 20 ms.
pub struct Maximizer {
    pv: ParamValues,
    sr: f32,
    drive: Smoothed,
    ceiling: f32,
    rel: f32,
    /// Look-ahead in samples (at least 1).
    look: usize,
    /// Per-channel delay rings of `look` samples, which delay by `look - 1`.
    delay: Vec<Vec<f32>>,
    pos: usize,
    /// Monotonic ring of (sample index, required gain) for the sliding minimum.
    win: Vec<(u64, f32)>,
    win_head: usize,
    win_len: usize,
    clock: u64,
    /// Moving-average ring and its running sum.
    smooth: Vec<f32>,
    sum: f64,
    held: f32,
    gr_meter: f32,
}

impl Maximizer {
    pub const PARAMS: &'static [ParamSpec] = &[
        ParamSpec::new("threshold", "Threshold", -30.0, 0.0, 0.0, Unit::Decibels),
        ParamSpec::new("ceiling", "Ceiling", -30.0, 0.0, -0.3, Unit::Decibels),
        ParamSpec::log("release", "Release", 1.0, 1000.0, 50.0, Unit::Milliseconds),
    ];

    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let sample_rate = sane_sample_rate(sample_rate);
        let channels = channels.clamp(1, MAX_CHANNELS);
        let look = ((MAXIMIZER_LOOKAHEAD_MS * 0.001 * sample_rate).round() as usize).max(1);
        let mut s = Maximizer {
            pv: ParamValues::new(Self::PARAMS),
            sr: sample_rate,
            drive: Smoothed::with_ms(1.0, sample_rate, 20.0),
            ceiling: 1.0,
            rel: 0.0,
            look,
            delay: vec![vec![0.0; look]; channels],
            pos: 0,
            win: vec![(0, 1.0); look],
            win_head: 0,
            win_len: 0,
            clock: 0,
            smooth: vec![1.0; look],
            sum: look as f64,
            held: 1.0,
            gr_meter: 0.0,
        };
        s.apply_params(true);
        s
    }

    fn apply_params(&mut self, snap: bool) {
        self.drive.set(db_to_gain(-self.pv.v("threshold")));
        self.ceiling = db_to_gain(self.pv.v("ceiling"));
        self.rel = coef(self.pv.v("release"), self.sr);
        if snap {
            self.drive.snap();
        }
    }

    /// The minimum of the requests of the last `look` samples, `req` included.
    fn window_min(&mut self, req: f32) -> f32 {
        let cap = self.win.len();
        let t = self.clock;
        self.clock = t.saturating_add(1);
        let la = self.look as u64;
        while self.win_len > 0 && self.win.get(self.win_head).is_some_and(|&(at, _)| at.saturating_add(la) <= t) {
            self.win_head = (self.win_head + 1) % cap;
            self.win_len -= 1;
        }
        while self.win_len > 0 && self.win.get((self.win_head + self.win_len - 1) % cap).is_some_and(|&(_, v)| v >= req) {
            self.win_len -= 1;
        }
        if self.win_len == cap {
            self.win_head = (self.win_head + 1) % cap;
            self.win_len -= 1;
        }
        if let Some(slot) = self.win.get_mut((self.win_head + self.win_len) % cap) {
            *slot = (t, req);
            self.win_len += 1;
        }
        self.win.get(self.win_head).map_or(1.0, |&(_, v)| v.min(1.0))
    }
}

impl AudioEffect for Maximizer {
    param_plumbing!("maximizer");
    fn latency(&self) -> usize {
        self.look - 1
    }
    /// The deepest gain reduction during the last block, in dB (≥ 0).
    fn gain_reduction_db(&self) -> f32 {
        self.gr_meter
    }
    fn reset(&mut self) {
        self.drive.snap();
        self.delay.iter_mut().for_each(|d| d.iter_mut().for_each(|v| *v = 0.0));
        self.pos = 0;
        self.win_head = 0;
        self.win_len = 0;
        self.clock = 0;
        self.smooth.iter_mut().for_each(|v| *v = 1.0);
        self.sum = self.look as f64;
        self.held = 1.0;
        self.gr_meter = 0.0;
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        let nch = channels.len().min(self.delay.len());
        let la = self.look;
        let mut min_gain = 1.0f32;
        for i in 0..n {
            let drive = self.drive.tick();
            let ceil = self.ceiling;
            let p = self.pos;
            let mut peak = 0.0f32;
            for ch in 0..nch {
                let x = channels[ch][i];
                let x = if x.is_finite() { x * drive } else { 0.0 };
                peak = peak.max(x.abs());
                self.delay[ch][p] = x;
            }
            let req = if peak > ceil { ceil / peak } else { 1.0 };
            let hold = self.window_min(req);
            self.held = if hold < self.held { hold } else { hold + (self.held - hold) * self.rel };
            self.sum += f64::from(self.held) - f64::from(self.smooth[p]);
            self.smooth[p] = self.held;
            self.pos = (p + 1) % la;
            if self.pos == 0 {
                // Re-sum once per window to cancel floating-point drift.
                self.sum = self.smooth.iter().map(|&v| f64::from(v)).sum();
            }
            let g = ((self.sum / la as f64) as f32).clamp(0.0, 1.0);
            min_gain = min_gain.min(g);
            // The next slot holds the sample written `la - 1` samples ago: by now the moving
            // average has fully taken in the gain that sample required.
            for ch in 0..nch {
                channels[ch][i] = (self.delay[ch][self.pos] * g).clamp(-ceil, ceil);
            }
        }
        if n > 0 {
            self.gr_meter = -lin_db(min_gain).min(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loudness::LoudnessMeter;
    use crate::testutil::*;

    const SR: f32 = 48000.0;

    #[test]
    fn compressor_static_curve() {
        let c = |x| compressor_curve(x, -20.0, 4.0, 0.0);
        assert_eq!(c(-30.0), -30.0);
        assert_eq!(c(-20.0), -20.0);
        assert_eq!(c(0.0), -15.0);
        // Soft knee: continuous at both knee edges, midpoint below the hard curve.
        let k = |x| compressor_curve(x, -20.0, 4.0, 10.0);
        assert!((k(-25.0) + 25.0).abs() < 1e-5);
        assert!((k(-15.0) - c(-15.0)).abs() < 1e-5);
        assert!(k(-20.0) < -20.0);
        assert!((k(-20.0) - (-20.0 + (0.25 - 1.0) * 25.0 / 20.0)).abs() < 1e-5);
        // Monotonic.
        let mut prev = f32::MIN;
        for i in 0..800 {
            let y = k(-80.0 + i as f32 * 0.1);
            assert!(y >= prev);
            prev = y;
        }
    }

    #[test]
    fn compressor_steady_state_matches_curve() {
        for detector in [0.0, 1.0] {
            let mut c = Compressor::new(SR, 2);
            c.set_param("threshold", -20.0);
            c.set_param("ratio", 4.0);
            c.set_param("knee", 0.0);
            c.set_param("makeup", 3.0);
            c.set_param("detector", detector);
            // Constant (DC) level: peak and RMS detectors agree.
            for level_db in [-40.0f32, -10.0, -2.0] {
                c.reset();
                let v = db_to_gain(level_db);
                let mut l = vec![v; 48000];
                let mut r = vec![-v; 48000];
                c.process(&mut [&mut l, &mut r]);
                let out_db = lin_db(l[47999]);
                let expect = compressor_curve(level_db, -20.0, 4.0, 0.0) + 3.0;
                assert!((out_db - expect).abs() < 0.05, "det {detector} in {level_db}: {out_db} vs {expect}");
                assert_eq!(l[47999], -r[47999]);
            }
        }
    }

    #[test]
    fn compressor_attack_time() {
        let mut c = Compressor::new(SR, 1);
        c.set_param("threshold", -20.0);
        c.set_param("ratio", 20.0);
        c.set_param("knee", 0.0);
        c.set_param("attack", 10.0);
        let mut x = vec![1.0f32; 4800];
        c.process(&mut [&mut x]);
        // After one time constant ~63% of the final reduction (in dB) is applied.
        let final_db = compressor_curve(0.0, -20.0, 20.0, 0.0);
        let at_tau = lin_db(x[480]);
        assert!((at_tau / final_db - 0.632).abs() < 0.02, "{at_tau} / {final_db}");
    }

    #[test]
    fn gate_attenuates_below_threshold_and_holds() {
        let mut g = Gate::new(SR, 1);
        g.set_param("threshold", -30.0);
        g.set_param("ratio", 100.0);
        g.set_param("range", 40.0);
        g.set_param("hold", 100.0);
        g.set_param("release", 10.0);
        let loud = db_to_gain(-10.0);
        let quiet = db_to_gain(-50.0);
        let mut x: Vec<f32> = (0..48000).map(|i| if i < 24000 { loud } else { quiet }).collect();
        g.process(&mut [&mut x]);
        assert!((lin_db(x[23999]) + 10.0).abs() < 0.01);
        // During hold (100 ms) the gate stays open.
        assert!((lin_db(x[24000 + 2400]) + 50.0).abs() < 0.01);
        // Well after hold + release: attenuated by the full range.
        assert!((lin_db(x[47999]) + 90.0).abs() < 0.1, "{}", lin_db(x[47999]));
        assert!((expander_curve(-40.0, -30.0, 2.0, 60.0) + 50.0).abs() < 1e-6);
    }

    #[test]
    fn limiter_never_exceeds_ceiling() {
        let mut rng = Rng::new(7);
        for tp in [0.0, 1.0] {
            let mut lim = Limiter::new(SR, 2);
            lim.set_param("ceiling", -1.0);
            lim.set_param("input_gain", 12.0);
            lim.set_param("true_peak", tp);
            let ceiling = db_to_gain(-1.0);
            let mut all_l = Vec::new();
            let mut all_r = Vec::new();
            for block in 0..200 {
                let len = 1 + (rng.next_u64() % 700) as usize;
                let mut l: Vec<f32> = (0..len).map(|_| rng.uniform() * 0.8).collect();
                let mut r: Vec<f32> = (0..len).map(|_| rng.uniform() * 0.3).collect();
                if block % 7 == 0 {
                    l[len / 2] = 4.0; // impulse
                    r[0] = -3.0;
                }
                lim.process(&mut [&mut l, &mut r]);
                assert!(l.iter().chain(&r).all(|v| v.abs() <= ceiling + 1e-7));
                all_l.extend_from_slice(&l);
                all_r.extend_from_slice(&r);
            }
            if tp == 1.0 {
                // With true-peak detection the reconstructed peak stays close to the ceiling.
                let mut m = LoudnessMeter::new(SR as f64, 2);
                m.process(&[&all_l, &all_r]);
                assert!(m.true_peak_dbtp() < -1.0 + 0.6, "TP {}", m.true_peak_dbtp());
            }
        }
    }

    #[test]
    fn limiter_is_transparent_below_ceiling_with_reported_latency() {
        let mut lim = Limiter::new(SR, 1);
        let lat = lim.latency();
        let x = sine(1000.0, 0.3, SR as f64, 4800, 0.0);
        let mut y = x.clone();
        lim.process(&mut [&mut y]);
        for i in lat..4800 {
            assert!((y[i] - x[i - lat]).abs() < 1e-6);
        }
    }

    #[test]
    fn limiter_reduces_loud_sine_to_ceiling_smoothly() {
        let mut lim = Limiter::new(SR, 1);
        lim.set_param("ceiling", -6.0);
        let mut x = sine(200.0, 1.0, SR as f64, 48000, 0.0);
        lim.process(&mut [&mut x]);
        let tail = &x[24000..];
        let peak = tail.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!((lin_db(peak) + 6.0).abs() < 0.3, "{}", lin_db(peak));
        assert!((lim.gain_reduction_db() - 6.0).abs() < 0.5, "{}", lim.gain_reduction_db());
    }

    #[test]
    fn limiter_turns_non_finite_input_into_silence() {
        let mut lim = Limiter::new(SR, 2);
        let mut l = vec![0.1f32; 2048];
        let mut r = vec![0.1f32; 2048];
        l[10] = f32::NAN;
        r[20] = f32::INFINITY;
        lim.process(&mut [&mut l, &mut r]);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() <= db_to_gain(-1.0)));
    }

    #[test]
    fn a_key_drives_the_compressor() {
        let quiet = db_to_gain(-40.0);
        let loud = vec![db_to_gain(-6.0); 48_000];
        let mut c = Compressor::new(SR, 2);
        assert_eq!(c.key_channels(), 2);
        c.set_param("threshold", -30.0);
        c.set_param("ratio", 10.0);
        c.set_param("knee", 0.0);
        let (mut l, mut r) = (vec![quiet; 48_000], vec![quiet; 48_000]);
        c.process_keyed(&mut [&mut l, &mut r], &[&loud, &loud]);
        // The key sits 24 dB over the threshold: 24 * (1 - 1/10) = 21.6 dB of reduction.
        assert!((lin_db(l[47_999]) + 40.0 + 21.6).abs() < 0.1, "{}", lin_db(l[47_999]));
        assert!((c.gain_reduction_db() - 21.6).abs() < 0.1);
        // Without a key the quiet input is under the threshold and passes.
        c.reset();
        let mut x = vec![quiet; 48_000];
        c.process(&mut [&mut x]);
        assert!((lin_db(x[47_999]) + 40.0).abs() < 0.01);
        assert_eq!(c.gain_reduction_db(), 0.0);
        // A key shorter than the block is ignored.
        c.reset();
        let mut x = vec![quiet; 1024];
        c.process_keyed(&mut [&mut x], &[&loud[..10]]);
        assert!((lin_db(x[1023]) + 40.0).abs() < 0.01);
    }

    #[test]
    fn a_key_opens_the_gate() {
        let mut g = Gate::new(SR, 1);
        g.set_param("threshold", -30.0);
        g.set_param("range", 60.0);
        let quiet = db_to_gain(-50.0);
        let open = vec![db_to_gain(-10.0); 24_000];
        let closed = vec![0.0f32; 24_000];
        let mut x = vec![quiet; 24_000];
        g.process_keyed(&mut [&mut x], &[&open]);
        assert!((lin_db(x[23_999]) + 50.0).abs() < 0.01, "open: {}", lin_db(x[23_999]));
        let mut x = vec![quiet; 24_000];
        g.process_keyed(&mut [&mut x], &[&closed]);
        assert!(lin_db(x[23_999]) < -105.0, "closed: {}", lin_db(x[23_999]));
        assert!(g.gain_reduction_db() > 55.0);
    }

    #[test]
    fn maximizer_holds_the_ceiling_and_reports_its_latency() {
        let mut m = Maximizer::new(SR, 2);
        m.set_param("threshold", -12.0);
        m.set_param("ceiling", -1.0);
        m.reset();
        let ceiling = db_to_gain(-1.0);
        let mut rng = Rng::new(11);
        for block in 0..100 {
            let len = (rng.next_u64() % 900) as usize;
            let mut l: Vec<f32> = (0..len).map(|_| rng.uniform() * 0.9).collect();
            let mut r: Vec<f32> = (0..len).map(|_| rng.uniform() * 0.9).collect();
            if block % 5 == 0 && len > 0 {
                l[len / 3] = 1.0;
            }
            m.process(&mut [&mut l, &mut r]);
            assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() <= ceiling + 1e-7));
        }
        assert!(m.gain_reduction_db() > 0.0);
        // Below the ceiling it is a pure delay of `latency()` samples.
        let mut m = Maximizer::new(SR, 1);
        m.set_param("ceiling", 0.0);
        let lat = m.latency();
        assert_eq!(lat, 71, "1.5 ms at 48 kHz is 72 samples of look-ahead");
        let x = sine(1000.0, 0.5, SR as f64, 4800, 0.0);
        let mut y = x.clone();
        m.process(&mut [&mut y]);
        for i in lat..4800 {
            assert!((y[i] - x[i - lat]).abs() < 1e-6, "sample {i}");
        }
    }

    #[test]
    fn maximizer_ramps_down_before_a_peak_instead_of_clipping_it() {
        // One loud sample after silence: the output peak lands on the ceiling, and the gain
        // has already fallen when it arrives, so samples just before it are attenuated too.
        let mut m = Maximizer::new(SR, 1);
        m.set_param("ceiling", -6.0);
        let lat = m.latency();
        let mut x = vec![0.25f32; 2000];
        x[1000] = 1.0;
        m.process(&mut [&mut x]);
        let at = 1000 + lat;
        assert!((x[at] - db_to_gain(-6.0)).abs() < 1e-4, "{}", x[at]);
        assert!(x[at - 10] < 0.25 * 0.99, "the gain ramps down ahead of the peak");
    }

    /// The sliding minimum against a scan of the last `look` requests.
    #[test]
    fn maximizer_window_min_matches_a_scan() {
        let mut rng = Rng::new(5);
        for sr in [1_000.0f32, 2_000.0, 48_000.0] {
            let mut m = Maximizer::new(sr, 1);
            let la = m.look;
            let mut ring = vec![1.0f32; la];
            for i in 0..6_000 {
                if i == 3_000 {
                    m.reset();
                    ring.iter_mut().for_each(|v| *v = 1.0);
                }
                let req = (rng.uniform() * 0.5 + 0.5).min(1.0);
                ring[i % la] = req;
                let scan = ring.iter().fold(1.0f32, |a, &v| a.min(v));
                // Before the window is full, the scan sees the initial 1.0 values the window
                // never held, which are larger and do not change the minimum.
                assert_eq!(m.window_min(req), scan, "sr {sr} step {i}");
            }
        }
    }
}
