//! Effects for a live voice: a pitch shifter with 10 ms of latency and a formant shifter with
//! 10.7 ms of latency (at 48 kHz).
//!
//! The pitch shifter (`voice_pitch`) has two read taps that sweep through a short delay line and
//! crossfade (sin² and cos² gains), so the output moves by the pitch ratio with a latency of 10 ms.
//!
//! The taps sit half a sweep apart and are centered on the latency, so the delay of each one
//! stays within `latency ± spacing`. A pitch tracker (YIN, on a copy of the input low-passed and
//! decimated to about 12 kHz, over the last 42.7 ms of history) sets the spacing to a whole number
//! of periods whenever one tap passes the center, so the two taps read the same point of a
//! period and a voiced sound crossfades without beating. A period longer than the room inside the
//! latency (below about 100 Hz), and unvoiced input, use the widest spacing that fits; below
//! 100 Hz each crossfade then steps the phase of the voice, which can move its pitch by a few
//! hertz (5 Hz for 85 Hz shifted up 5 semitones). At a shift
//! of 0 the effect crossfades over 20 ms to the input delayed by the latency, so a neutral setting
//! is a plain delay.
//!
//! The formant shifter (`voice_formant`) moves the spectral envelope of the voice up or down while
//! its harmonics stay where they are. It runs a short STFT (512 points at up to 50 kHz, hop 128).
//! For each frame it fits a linear predictor (one coefficient per kHz plus 2, at most 96) to the
//! last two frame lengths of input (1,024 samples at 48 kHz), which end with the frame and add no
//! latency, and takes the
//! envelope from the predictor. It then multiplies every bin by the envelope read at
//! `bin / ratio` over the envelope at the bin, with at most 24 dB of boost. Its latency is the
//! STFT size. `voice_pitch` uses the same stage for Keep Formants: the stage moves the shifted
//! voice's envelope back by the inverse of the pitch ratio, so the pitch changes and the formants
//! stay, and the latency grows by the STFT size.

use super::stft::{Spectral, StftChannel, fft_size};
use crate::biquad::{Biquad, Coeffs, FilterType};
use crate::fft::Fft;
use crate::{AudioEffect, ParamSpec, ParamValues, Unit, block_len, flush32, param_plumbing};
use std::f64::consts::PI;

/// Latency, and the most a tap may read either side of it.
const LATENCY_MS: f32 = 10.0;
/// Lowest and highest fundamental the tracker reports.
const MIN_HZ: f32 = 70.0;
const MAX_HZ: f32 = 1000.0;
/// Rate the tracker analyzes at, before rounding to a whole decimation factor.
const TRACK_RATE: f32 = 12_000.0;
/// YIN's threshold on the cumulative mean normalized difference.
const THRESHOLD: f32 = 0.15;
/// Length of the crossfade to and from the plain delay at a shift of 0.
const BYPASS_MS: f32 = 20.0;
/// Samples kept between a tap and the newest sample, for the cubic interpolation.
const MARGIN: f64 = 2.0;
/// Largest step of the sweep phase per sample; one step never crosses two tap boundaries.
const MAX_STEP: f64 = 0.45;
/// Analysis input is clipped to this level, so a huge sample cannot overflow the tracker.
const TRACK_CLIP: f64 = 1.0e4;
/// Fewest and most predictor coefficients of the formant envelope (one per kHz plus 2 between).
const MIN_ORDER: usize = 8;
const MAX_ORDER: usize = 96;
/// Width of the Gaussian lag window, which widens each envelope peak to about this many hertz.
const LAG_HZ: f64 = 40.0;
/// White noise added to the autocorrelation (60 dB down), so a pure tone still gives a stable
/// predictor.
const WHITE: f64 = 1.0e-6;
/// Largest boost the envelope warp gives one bin: ln 16, about 24 dB.
const MAX_BOOST: f64 = 2.772_588_722_239_781;

/// STFT size of the formant stage: 512 at up to 50 kHz (10.7 ms at 48 kHz), 1024 up to 100 kHz,
/// 2048 above.
fn formant_size(sample_rate: f32) -> usize {
    fft_size(sample_rate) / 4
}

/// Fundamental period of the recent input, from YIN (de Cheveigné and Kawahara, 2002) run on a
/// low-passed, decimated copy. Every buffer is sized at creation.
///
/// It keeps two copies: the mean of the channels, and the first channel alone. A frame whose
/// mean holds less than a quarter of the first channel's energy (channels of opposite polarity
/// cancel in the mean) is analyzed on the first channel.
#[derive(Clone, Debug)]
struct Tracker {
    /// Input samples per analysis sample.
    factor: usize,
    count: usize,
    coeffs: [Coeffs; 2],
    feeds: [Feed; 2],
    at: usize,
    hop: usize,
    since: usize,
    frame: Vec<f32>,
    diff: Vec<f32>,
    min_tau: usize,
    max_tau: usize,
    /// Period in input samples, or `None` when the last frame was unvoiced.
    period: Option<f64>,
}

impl Tracker {
    fn new(sample_rate: f32) -> Self {
        let factor = (sample_rate / TRACK_RATE).round().max(1.0) as usize;
        let rate = sample_rate / factor as f32;
        let max_hz = MAX_HZ.min(rate / 4.0);
        let min_tau = ((rate / max_hz).floor() as usize).max(2);
        let max_tau = (rate / MIN_HZ).ceil() as usize + 1;
        let len = (2 * max_tau + 2).next_power_of_two();
        // Fourth-order Butterworth low-pass at a quarter of the analysis rate.
        let (sr, fc) = (sample_rate as f64, rate as f64 / 4.0);
        let coeffs = [0.541_196_1, 1.306_563].map(|q| Coeffs::design(FilterType::LowPass, sr, fc, q, 0.0));
        Tracker {
            factor,
            count: 0,
            coeffs,
            feeds: [Feed::new(len), Feed::new(len)],
            at: 0,
            hop: len / 4,
            since: 0,
            frame: vec![0.0; len],
            diff: vec![0.0; max_tau + 2],
            min_tau,
            max_tau,
            period: None,
        }
    }

    fn reset(&mut self) {
        self.count = 0;
        self.feeds.iter_mut().for_each(Feed::reset);
        self.at = 0;
        self.since = 0;
        self.period = None;
    }

    /// Takes one input sample, as the mean of the channels and the first channel; runs an
    /// analysis every `hop` analysis samples.
    fn push(&mut self, mean: f32, first: f32) {
        let clipped = [mean, first].map(|x| (x as f64).clamp(-TRACK_CLIP, TRACK_CLIP));
        for (feed, x) in self.feeds.iter_mut().zip(clipped) {
            feed.filter(&self.coeffs, x);
        }
        self.count += 1;
        if self.count < self.factor {
            return;
        }
        self.count = 0;
        let mask = self.frame.len() - 1;
        for feed in &mut self.feeds {
            feed.ring[self.at] = feed.last as f32;
        }
        self.at = (self.at + 1) & mask;
        self.since += 1;
        if self.since >= self.hop {
            self.since = 0;
            self.analyze();
        }
    }

    fn analyze(&mut self) {
        let [mean, first] = [0, 1].map(|f| self.feeds[f].ring.iter().map(|v| v * v).sum::<f32>());
        let feed = if mean < 0.25 * first { &self.feeds[1] } else { &self.feeds[0] };
        let mask = self.frame.len() - 1;
        for (i, v) in self.frame.iter_mut().enumerate() {
            *v = feed.ring[(self.at + i) & mask];
        }
        self.period = yin(&self.frame, &mut self.diff, self.min_tau, self.max_tau).map(|t| t * self.factor as f64);
    }

    /// Tap spacing for the current period: the most whole periods that fit in `max`, or `max`
    /// itself for unvoiced input and for a period longer than `max`.
    fn spacing(&self, max: f64) -> f64 {
        match self.period {
            Some(t) if t <= max => (max / t).floor() * t,
            _ => max,
        }
    }
}

/// One low-passed, decimated copy of the input that the tracker can analyze.
#[derive(Clone, Debug)]
struct Feed {
    lowpass: [Biquad; 2],
    /// The newest low-passed sample, kept at every input sample.
    last: f64,
    ring: Vec<f32>,
}

impl Feed {
    fn new(len: usize) -> Self {
        Feed { lowpass: [Biquad::new(), Biquad::new()], last: 0.0, ring: vec![0.0; len] }
    }

    fn reset(&mut self) {
        self.lowpass.iter_mut().for_each(Biquad::reset);
        self.last = 0.0;
        self.ring.iter_mut().for_each(|v| *v = 0.0);
    }

    fn filter(&mut self, coeffs: &[Coeffs; 2], x: f64) {
        let y = self.lowpass[0].tick(&coeffs[0], x);
        self.last = self.lowpass[1].tick(&coeffs[1], y);
    }
}

/// YIN on `frame`: the lag in samples, with parabolic interpolation, or `None` when no lag in
/// `min_tau..max_tau` falls below the threshold. `diff` holds at least `max_tau + 1` values.
fn yin(frame: &[f32], diff: &mut [f32], min_tau: usize, max_tau: usize) -> Option<f64> {
    let w = frame.len().checked_sub(max_tau)?;
    if max_tau <= min_tau + 2 || diff.len() <= max_tau || w == 0 {
        return None;
    }
    difference(frame, &mut diff[..=max_tau], w);
    normalize(&mut diff[..=max_tau]);
    pick(&diff[..=max_tau], min_tau)
}

/// Squared difference of the frame with itself at every lag.
fn difference(frame: &[f32], diff: &mut [f32], w: usize) {
    diff[0] = 0.0;
    for (tau, d) in diff.iter_mut().enumerate().skip(1) {
        *d = frame[..w].iter().zip(&frame[tau..tau + w]).map(|(a, b)| (a - b) * (a - b)).sum();
    }
}

/// Cumulative mean normalized difference, in place.
fn normalize(diff: &mut [f32]) {
    diff[0] = 1.0;
    let mut running = 0.0f32;
    for (tau, d) in diff.iter_mut().enumerate().skip(1) {
        running += *d;
        *d = if running > 0.0 { *d * tau as f32 / running } else { 1.0 };
    }
}

/// The first dip below the threshold, walked to its local minimum and interpolated.
fn pick(cmnd: &[f32], min_tau: usize) -> Option<f64> {
    let last = cmnd.len() - 1;
    let mut tau = (min_tau..last).find(|&t| cmnd[t] < THRESHOLD)?;
    while tau + 1 < last && cmnd[tau + 1] < cmnd[tau] {
        tau += 1;
    }
    let (a, b, c) = (cmnd[tau - 1] as f64, cmnd[tau] as f64, cmnd[tau + 1] as f64);
    let denom = a - 2.0 * b + c;
    let shift = if denom.abs() > 1e-12 { (0.5 * (a - c) / denom).clamp(-1.0, 1.0) } else { 0.0 };
    Some(tau as f64 + shift)
}

/// Four-point cubic Hermite read of `line` at a fractional `delay` behind `pos`. `delay` is at
/// least 2 and leaves two samples in the line behind it.
#[inline]
fn read(line: &[f32], mask: usize, pos: usize, delay: f64) -> f64 {
    let i = delay as usize;
    let f = delay - i as f64;
    let at = |k: usize| line[(pos + line.len() - k) & mask] as f64;
    let (ym1, y0, y1, y2) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    let c1 = 0.5 * (y1 - ym1);
    let c2 = ym1 - 2.5 * y0 + 2.0 * y1 - 0.5 * y2;
    let c3 = 0.5 * (y2 - ym1) + 1.5 * (y0 - y1);
    ((c3 * f + c2) * f + c1) * f + y0
}

/// Read positions and gains of both taps for one sample.
#[derive(Clone, Copy, Debug)]
struct Taps {
    wet: f64,
    delay_a: f64,
    gain_a: f64,
    delay_b: f64,
}

/// Pitch shifter for a live voice, 10 ms of latency (±24 semitones and ±100 cents). Keep Formants
/// (`formant_follow`) adds the formant stage's latency, 10.7 ms at 48 kHz, and `latency` reports
/// it while the switch is on.
pub struct VoicePitch {
    pv: ParamValues,
    latency: usize,
    /// Formant stage of Keep Formants, and the share of its output against the shifted voice.
    follow: Stage,
    follow_mix: f64,
    follow_on: bool,
    /// Widest tap spacing that keeps both taps inside the line, in samples.
    max_spacing: f64,
    lines: Vec<Vec<f32>>,
    mask: usize,
    pos: usize,
    tracker: Tracker,
    ratio: f64,
    /// Sweep phase in `[0, 1)`: tap A is at the latency when it is 0.5, tap B when it is 0.
    phase: f64,
    spacing: f64,
    /// Share of the shifted signal against the plain delay.
    wet: f64,
    wet_step: f64,
}

impl VoicePitch {
    pub const PARAMS: &'static [ParamSpec] = &[
        ParamSpec::new("semitones", "Pitch", -24.0, 24.0, 0.0, Unit::Semitones),
        ParamSpec::new("cents", "Fine Tune (cents)", -100.0, 100.0, 0.0, Unit::None),
        ParamSpec::new("formant_follow", "Keep Formants", 0.0, 1.0, 0.0, Unit::Toggle),
    ];

    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let sample_rate = crate::sane_sample_rate(sample_rate);
        let ch = crate::sane_channels(channels);
        let latency = ((sample_rate * LATENCY_MS / 1000.0).round() as usize).max(4);
        let len = (2 * latency + 8).next_power_of_two();
        let mut s = VoicePitch {
            pv: ParamValues::new(Self::PARAMS),
            latency,
            follow: Stage::new(sample_rate, ch),
            follow_mix: 0.0,
            follow_on: false,
            max_spacing: latency as f64 - MARGIN,
            lines: vec![vec![0.0; len]; ch],
            mask: len - 1,
            pos: 0,
            tracker: Tracker::new(sample_rate),
            ratio: 1.0,
            phase: 0.5,
            spacing: latency as f64 - MARGIN,
            wet: 0.0,
            wet_step: 1.0 / (sample_rate * BYPASS_MS / 1000.0).max(1.0) as f64,
        };
        s.apply_params(true);
        s
    }

    fn apply_params(&mut self, snap: bool) {
        let shift = self.pv.v("semitones") as f64 + self.pv.v("cents") as f64 / 100.0;
        self.ratio = 2f64.powf(shift / 12.0);
        self.follow_on = self.pv.v("formant_follow") >= 0.5;
        if snap {
            self.wet = self.target_wet();
            self.follow_mix = self.follow_target();
        }
    }

    fn follow_target(&self) -> f64 {
        if self.follow_on { 1.0 } else { 0.0 }
    }

    fn target_wet(&self) -> f64 {
        if self.ratio == 1.0 { 0.0 } else { 1.0 }
    }

    /// Moves the crossfade and the sweep by one sample and returns where the taps read.
    fn advance(&mut self) -> Taps {
        let target = self.target_wet();
        self.wet = if self.wet < target { (self.wet + self.wet_step).min(target) } else { (self.wet - self.wet_step).max(target) };
        if self.wet == 0.0 {
            // Fully on the plain delay: the next shift starts with tap A at the latency.
            self.phase = 0.5;
            self.spacing = self.max_spacing;
        } else {
            self.sweep();
        }
        let pb = if self.phase >= 0.5 { self.phase - 0.5 } else { self.phase + 0.5 };
        let (l, s) = (self.latency as f64, self.spacing);
        Taps { wet: self.wet, delay_a: l + (self.phase - 0.5) * 2.0 * s, gain_a: (PI * self.phase).sin().powi(2), delay_b: l + (pb - 0.5) * 2.0 * s }
    }

    /// Advances the sweep phase. A tap whose delay changes by `1 - ratio` per sample reads the
    /// input `ratio` times as fast.
    fn sweep(&mut self) {
        let step = ((1.0 - self.ratio) / (2.0 * self.spacing)).clamp(-MAX_STEP, MAX_STEP);
        let raw = self.phase + step;
        let wrapped = !(0.0..1.0).contains(&raw);
        let p = raw - raw.floor();
        if wrapped || (p >= 0.5) != (self.phase >= 0.5) {
            self.regrain(p, wrapped);
        } else {
            self.phase = p;
        }
    }

    /// One tap has just passed the latency and the other is silent at the end of its sweep:
    /// take the tracker's spacing, and scale the centered tap's small offset so its delay does
    /// not jump.
    fn regrain(&mut self, p: f64, wrapped: bool) {
        let new = self.tracker.spacing(self.max_spacing);
        let k = self.spacing / new;
        self.spacing = new;
        let phase = if wrapped {
            // Tap B is centered; its offset from the center is `p` (just past 0) or `p - 1`.
            let u = (if p < 0.5 { p } else { p - 1.0 }) * k;
            let u = u.clamp(-MAX_STEP, MAX_STEP);
            if u < 0.0 { u + 1.0 } else { u }
        } else {
            0.5 + ((p - 0.5) * k).clamp(-MAX_STEP, MAX_STEP)
        };
        self.phase = if phase < 1.0 { phase } else { 0.0 };
    }
}

impl AudioEffect for VoicePitch {
    param_plumbing!("voice_pitch");
    fn latency(&self) -> usize {
        self.latency + if self.follow_on { self.follow.n } else { 0 }
    }
    fn tail_samples(&self) -> usize {
        // A tap reads up to `latency + max_spacing` samples back and the cubic read two samples
        // more, which is twice the latency in all. Keep Formants spreads a frame over up to the
        // formant stage's size more. A render drains `latency() + tail_samples()` samples.
        self.latency + if self.follow_on { self.follow.n } else { 0 }
    }
    fn reset(&mut self) {
        self.lines.iter_mut().for_each(|l| l.iter_mut().for_each(|v| *v = 0.0));
        self.pos = 0;
        self.tracker.reset();
        self.phase = 0.5;
        self.spacing = self.max_spacing;
        self.wet = self.target_wet();
        self.follow.reset();
        self.follow_mix = self.follow_target();
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let len = block_len(channels);
        let ch = channels.len().min(self.lines.len());
        let dry_at = self.mask + 1 - self.latency;
        let follow_target = self.follow_target();
        // The formant stage moves the envelope by the inverse of the pitch ratio.
        let back = (1.0 / self.ratio) as f32;
        for i in 0..len {
            let mut sum = 0.0f32;
            for (c, line) in self.lines.iter_mut().enumerate() {
                // A non-finite sample is stored as silence, so it never reaches the output. A
                // channel the block leaves out is stored as silence too, so its line holds none
                // of its earlier audio when it comes back.
                let x = channels.get(c).map_or(0.0, |io| flush32(io[i]));
                line[self.pos] = x;
                sum += x;
            }
            let first = self.lines[0][self.pos];
            self.tracker.push(sum / ch as f32, first);
            let t = self.advance();
            let mix = if self.follow_mix < follow_target {
                (self.follow_mix + self.wet_step).min(follow_target)
            } else {
                (self.follow_mix - self.wet_step).max(follow_target)
            };
            self.follow_mix = mix;
            // While Keep Formants is off and faded out, the stage's frames copy the shifted voice,
            // so switching it on crossfades from a delayed copy instead of from silence.
            let warp = if mix > 0.0 { back } else { 1.0 };
            // A channel the block leaves out still reads its line into the formant stage, so the
            // audio its line holds reaches the stage on time; only its output is dropped.
            for (c, line) in self.lines.iter().enumerate() {
                let dry = line[(self.pos + dry_at) & self.mask] as f64;
                let y = if t.wet == 0.0 {
                    dry
                } else {
                    let shifted =
                        t.gain_a * read(line, self.mask, self.pos, t.delay_a) + (1.0 - t.gain_a) * read(line, self.mask, self.pos, t.delay_b);
                    t.wet * shifted + (1.0 - t.wet) * dry
                };
                let y = y.clamp(-f32::MAX as f64, f32::MAX as f64);
                let z = self.follow.tick(c, y as f32, warp);
                // The stage's output is `n` samples later than `y`, so a switch fades one out
                // before the other comes in instead of mixing them.
                let out = if mix < 0.5 { (1.0 - 2.0 * mix) * y } else { (2.0 * mix - 1.0) * z as f64 };
                if let Some(io) = channels.get_mut(c) {
                    io[i] = out.clamp(-f32::MAX as f64, f32::MAX as f64) as f32;
                }
            }
            self.pos = (self.pos + 1) & self.mask;
        }
    }
}

/// Spectral envelope of the recent input from linear prediction, and the warp that moves it.
/// Every buffer is sized at creation.
#[derive(Clone, Debug)]
struct Envelope {
    fft: Fft,
    /// Hann window over the analyzed history.
    window: Vec<f64>,
    /// Gaussian lag window, one value per autocorrelation lag (`order + 1`).
    lags: Vec<f64>,
    frame: Vec<f64>,
    r: Vec<f64>,
    /// Predictor coefficients, `a[0] = 1`.
    a: Vec<f64>,
    prev: Vec<f64>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Natural log of the envelope's magnitude, bins 0..=n/2 of the STFT.
    log: Vec<f32>,
}

impl Envelope {
    /// An envelope for STFT frames of `n` points, read from the last `2 n` samples.
    fn new(n: usize, sample_rate: f32) -> Self {
        let order = ((sample_rate / 1000.0).round() as usize + 2).clamp(MIN_ORDER, MAX_ORDER);
        let w = 2 * n;
        let window = (0..w).map(|i| 0.5 - 0.5 * (2.0 * PI * (i as f64 + 0.5) / w as f64).cos()).collect();
        let lags = (0..=order)
            .map(|i| {
                let x = 2.0 * PI * LAG_HZ * i as f64 / sample_rate as f64;
                (-0.5 * x * x).exp()
            })
            .collect();
        Envelope {
            fft: Fft::new(n),
            window,
            lags,
            frame: vec![0.0; w],
            r: vec![0.0; order + 1],
            a: vec![0.0; order + 1],
            prev: vec![0.0; order + 1],
            re: vec![0.0; n],
            im: vec![0.0; n],
            log: vec![0.0; n / 2 + 1],
        }
    }

    /// Reads the envelope of the newest `2 n` samples of `history`. Returns `false` for silence,
    /// which has no envelope.
    fn measure(&mut self, history: &History) -> bool {
        let mask = history.ring.len() - 1;
        for (i, (v, w)) in self.frame.iter_mut().zip(&self.window).enumerate() {
            *v = history.ring[(history.pos + i) & mask] as f64 * w;
        }
        let len = self.frame.len();
        for (lag, (r, w)) in self.r.iter_mut().zip(&self.lags).enumerate() {
            *r = dot(&self.frame[..len - lag], &self.frame[lag..]) * w;
        }
        if !(self.r[0] > 0.0 && self.r[0].is_finite()) {
            return false;
        }
        self.r[0] *= 1.0 + WHITE;
        levinson(&self.r, &mut self.a, &mut self.prev);
        // The envelope is 1 / |A| on the bins of the STFT.
        self.re.fill(0.0);
        self.im.fill(0.0);
        for (v, a) in self.re.iter_mut().zip(&self.a) {
            *v = *a as f32;
        }
        self.fft.forward(&mut self.re, &mut self.im);
        for (l, (re, im)) in self.log.iter_mut().zip(self.re.iter().zip(&self.im)) {
            *l = (-0.5 * (*re as f64 * *re as f64 + *im as f64 * *im as f64 + 1e-30).ln()) as f32;
        }
        true
    }

    /// Multiplies the half spectrum `re`/`im` (bins 0..=n/2) by the envelope read at `bin /
    /// ratio` over the envelope at the bin, so the envelope moves up by `ratio` and the
    /// harmonics stay. A frame with a non-finite bin is replaced with silence.
    fn warp(&self, re: &mut [f32], im: &mut [f32], ratio: f32) {
        let half = self.log.len();
        let (re, im) = (&mut re[..half], &mut im[..half]);
        if !re.iter().chain(im.iter()).fold(true, |ok, v| ok & v.is_finite()) {
            re.fill(0.0);
            im.fill(0.0);
            return;
        }
        let last = (half - 1) as f32;
        for k in 0..half {
            let src = (k as f32 / ratio).min(last);
            let i = src as usize;
            let e = match self.log.get(i + 1) {
                Some(next) => self.log[i] + (src - i as f32) * (next - self.log[i]),
                None => self.log[i],
            };
            let g = ((e - self.log[k]) as f64).min(MAX_BOOST).exp() as f32;
            re[k] *= g;
            im[k] *= g;
        }
    }
}

/// Dot product of two slices of the same length, in four running sums so it vectorizes.
fn dot(a: &[f64], b: &[f64]) -> f64 {
    let mut sums = [0.0f64; 4];
    let ((ca, ra), (cb, rb)) = (a.as_chunks::<4>(), b.as_chunks::<4>());
    for (x, y) in ca.iter().zip(cb) {
        for ((s, x), y) in sums.iter_mut().zip(x).zip(y) {
            *s += x * y;
        }
    }
    let rest: f64 = ra.iter().zip(rb).map(|(x, y)| x * y).sum();
    (sums[0] + sums[1]) + (sums[2] + sums[3]) + rest
}

/// Levinson-Durbin recursion: the predictor `a` (`a[0] = 1`) of the autocorrelation `r`. It
/// stops at the first reflection coefficient that would make the predictor unstable.
fn levinson(r: &[f64], a: &mut [f64], prev: &mut [f64]) {
    a.fill(0.0);
    a[0] = 1.0;
    let mut err = r[0];
    for i in 1..a.len() {
        let acc = r[i] + (1..i).map(|j| a[j] * r[i - j]).sum::<f64>();
        let k = -acc / err;
        // NaN (from a zero error) also stops the recursion.
        if k.is_nan() || k.abs() >= 1.0 {
            return;
        }
        prev[..i].copy_from_slice(&a[..i]);
        for j in 1..i {
            a[j] = prev[j] + k * prev[i - j];
        }
        a[i] = k;
        err *= 1.0 - k * k;
    }
}

/// The newest samples of one channel, for the envelope.
#[derive(Clone, Debug)]
struct History {
    ring: Vec<f32>,
    /// Where the next sample goes, which is also the oldest sample.
    pos: usize,
}

impl History {
    fn push(&mut self, x: f32) {
        self.ring[self.pos] = x;
        self.pos = (self.pos + 1) & (self.ring.len() - 1);
    }

    fn reset(&mut self) {
        self.ring.fill(0.0);
        self.pos = 0;
    }
}

/// STFT stage that moves each channel's envelope, fed one sample at a time: the formant
/// shifter, and the formant stage of `voice_pitch`'s Keep Formants.
#[derive(Clone, Debug)]
struct Stage {
    n: usize,
    hop: usize,
    scale: f32,
    spec: Spectral,
    stft: Vec<StftChannel>,
    history: Vec<History>,
    env: Envelope,
}

impl Stage {
    fn new(sample_rate: f32, channels: usize) -> Self {
        let n = formant_size(sample_rate);
        Stage {
            n,
            hop: n / 4,
            // √Hann on both sides is Hann, which sums to 2 at a hop of n/4; the inverse FFT is
            // unscaled.
            scale: 1.0 / (2 * n) as f32,
            spec: Spectral::new(n, true),
            stft: vec![StftChannel::new(n); channels],
            history: vec![History { ring: vec![0.0; 2 * n], pos: 0 }; channels],
            env: Envelope::new(n, sample_rate),
        }
    }

    fn reset(&mut self) {
        self.stft.iter_mut().for_each(StftChannel::reset);
        self.history.iter_mut().for_each(History::reset);
    }

    /// Pushes one sample of a channel and returns the stage's output for it, `n` samples later.
    /// A frame moves the envelope by `ratio`, which can spread a sound over the frame, so the
    /// output can go on for up to `n` samples more (the tail). At a ratio of 1, and for silence, a frame only
    /// windows and adds its input, which needs no FFT.
    fn tick(&mut self, channel: usize, x: f32, ratio: f32) -> f32 {
        let Stage { n, hop, scale, spec, stft, history, env } = self;
        let half = *n / 2 + 1;
        history[channel].push(x);
        let history = &history[channel];
        let z = stft[channel].tick(x, *hop, &mut |input, out| {
            if ratio != 1.0 && env.measure(history) {
                spec.analyze(input);
                env.warp(&mut spec.re[..half], &mut spec.im[..half], ratio);
                spec.synthesize_add(out, *scale);
            } else {
                for ((o, x), w) in out.iter_mut().zip(input).zip(&spec.window) {
                    *o += x * w * w * 0.5;
                }
            }
        });
        // A huge finite input can overflow the inverse FFT.
        flush32(z)
    }
}

/// Formant shifter for a live voice: moves the spectral envelope by -12 to +12 semitones (a ratio
/// of 0.5 to 2) and keeps the harmonics. Its latency is the STFT size, 512 samples (10.7 ms) at
/// 48 kHz.
pub struct VoiceFormant {
    pv: ParamValues,
    stage: Stage,
    ratio: f32,
}

impl VoiceFormant {
    pub const PARAMS: &'static [ParamSpec] = &[ParamSpec::new("semitones", "Formant Shift", -12.0, 12.0, 0.0, Unit::Semitones)];

    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let sample_rate = crate::sane_sample_rate(sample_rate);
        let mut s = VoiceFormant { pv: ParamValues::new(Self::PARAMS), stage: Stage::new(sample_rate, crate::sane_channels(channels)), ratio: 1.0 };
        s.apply_params(true);
        s
    }

    fn apply_params(&mut self, _snap: bool) {
        self.ratio = 2f32.powf(self.pv.v("semitones") / 12.0);
    }
}

impl AudioEffect for VoiceFormant {
    param_plumbing!("voice_formant");
    fn latency(&self) -> usize {
        self.stage.n
    }
    fn tail_samples(&self) -> usize {
        self.stage.n
    }
    fn reset(&mut self) {
        self.stage.reset();
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let len = block_len(channels);
        for i in 0..len {
            for c in 0..self.stage.stft.len() {
                // A non-finite sample is taken as silence, so it never reaches the output. A
                // channel the block leaves out is fed silence, so it holds none of its earlier
                // audio when it comes back.
                match channels.get_mut(c) {
                    Some(io) => io[i] = self.stage.tick(c, flush32(io[i]), self.ratio),
                    None => {
                        self.stage.tick(c, 0.0, self.ratio);
                    }
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

    fn shifter(semitones: f32, cents: f32) -> VoicePitch {
        let mut p = VoicePitch::new(SR, 2);
        assert!(p.set_param("semitones", semitones) && p.set_param("cents", cents));
        p.reset();
        p
    }

    fn run(p: &mut VoicePitch, x: &[f32]) -> Vec<f32> {
        let (mut l, mut r) = (x.to_vec(), x.to_vec());
        p.process(&mut [&mut l, &mut r]);
        assert_eq!(l, r, "both channels get the same shift");
        l
    }

    /// One second of output after the first half second.
    fn settled(y: &[f32]) -> &[f32] {
        &y[SR as usize / 2..SR as usize * 3 / 2]
    }

    /// Frequency of the strongest component between `lo` and `hi` Hz, to 0.01 Hz.
    fn peak_hz(x: &[f32], lo: f64, hi: f64) -> f64 {
        let best = |from: f64, to: f64, step: f64| {
            let n = ((to - from) / step) as usize;
            (0..=n)
                .map(|i| from + i as f64 * step)
                .max_by(|a, b| tone_amplitude(x, *a, SR as f64).total_cmp(&tone_amplitude(x, *b, SR as f64)))
                .unwrap_or(from)
        };
        let coarse = best(lo, hi, 0.25);
        best(coarse - 0.25, coarse + 0.25, 0.01)
    }

    fn cents(f: f64, want: f64) -> f64 {
        1200.0 * (f / want).log2()
    }

    /// A voiced tone shifted by an octave either way, or by a fifth with a fine offset, lands on
    /// the shifted frequency within 2 cents.
    #[test]
    fn a_shifted_tone_reads_the_shifted_frequency() {
        for (f_in, semis, fine) in [(220.0, 12.0, 0.0), (220.0, -12.0, 0.0), (180.0, 7.0, 30.0), (110.0, -24.0, 0.0)] {
            let mut p = shifter(semis, fine);
            let y = run(&mut p, &sine(f_in, 0.5, SR as f64, SR as usize * 2, 0.0));
            let want = f_in * 2f64.powf((semis as f64 + fine as f64 / 100.0) / 12.0);
            let got = peak_hz(settled(&y), want * 0.9, want * 1.1);
            assert!(cents(got, want).abs() < 2.0, "{f_in} Hz {semis:+} st {fine:+} ct: {got} Hz, want {want}");
        }
    }

    /// On a voiced tone the taps are spaced by whole periods, so the crossfades do not beat:
    /// almost all the output is the shifted tone.
    #[test]
    fn a_voiced_tone_crossfades_without_beating() {
        for (f_in, semis) in [(220.0, 7.0), (150.0, -5.0), (300.0, 12.0)] {
            let mut p = shifter(semis, 0.0);
            let y = run(&mut p, &sine(f_in, 0.5, SR as f64, SR as usize * 2, 0.0));
            let rest = beside_tone_db(&y, f_in * 2f64.powf(semis as f64 / 12.0));
            assert!(rest < -40.0, "{f_in} Hz {semis:+} st: {rest:.1} dB beside the tone");
        }
    }

    /// A voice whose two channels have opposite polarity, which cancel in their mean, is still
    /// tracked and crossfades without beating.
    #[test]
    fn opposite_polarity_channels_are_still_tracked() {
        let mut p = shifter(7.0, 0.0);
        let mut l = sine(220.0, 0.5, SR as f64, SR as usize * 2, 0.0);
        let mut r: Vec<f32> = l.iter().map(|v| -v).collect();
        p.process(&mut [&mut l, &mut r]);
        assert!(l.iter().zip(&r).all(|(a, b)| *a == -*b));
        let rest = beside_tone_db(&l, 220.0 * 2f64.powf(7.0 / 12.0));
        assert!(rest < -40.0, "{rest:.1} dB beside the tone");
    }

    /// Level of everything but the strongest component near `want`, against that component, in
    /// the settled output.
    fn beside_tone_db(y: &[f32], want: f64) -> f64 {
        let tail = settled(y);
        let tone = tone_amplitude(tail, peak_hz(tail, want * 0.98, want * 1.02), SR as f64) / std::f64::consts::SQRT_2;
        db((rms(tail).powi(2) - tone * tone).max(0.0).sqrt() / tone)
    }

    /// The output of a burst, shifted or not, comes from taps within the spacing of the latency:
    /// nothing sounds earlier than `latency - tail` or later than `latency + tail` around it.
    #[test]
    fn a_shifted_burst_stays_within_the_spacing_of_the_latency() {
        for semis in [0.0, -24.0, -3.0, 5.0, 24.0] {
            let mut p = shifter(semis, 0.0);
            let (n, at, len) = (8000, 1777, 1000);
            let mut rng = Rng::new(7);
            let mut x = vec![0.0f32; n];
            x[at..at + len].iter_mut().for_each(|v| *v = 0.5 * rng.uniform());
            let y = run(&mut p, &x);
            let reach = p.tail_samples();
            let (lo, hi) = (at + p.latency() - reach, at + len + p.latency() + reach);
            assert!(y.iter().enumerate().all(|(i, v)| *v == 0.0 || (lo..hi).contains(&i)), "{semis}");
            assert!(y.iter().any(|v| v.abs() > 0.05), "{semis}: the burst is lost");
        }
    }

    /// Over whole sweeps, the oldest sample either tap reads, interpolation included, lies within
    /// the latency plus the tail, so a render that compensates the latency and runs the tail past
    /// the end of the input keeps every sample the taps still hold.
    #[test]
    fn the_tail_covers_the_oldest_sample_a_tap_reads() {
        for semis in [-24.0, -3.0, 5.0, 24.0] {
            let mut p = shifter(semis, 0.0);
            let oldest = (0..SR as usize).map(|_| p.advance()).map(|t| t.delay_a.max(t.delay_b) as usize + 2).max().unwrap_or(0);
            assert!(oldest > p.latency(), "{semis}: the taps never sweep past the latency");
            assert!(oldest <= p.latency() + p.tail_samples(), "{semis}: a tap reads {oldest} samples back");
        }
    }

    /// A gliding voiced tone with a shift change part way gives the same output however the
    /// stream is cut into blocks, spacing changes included.
    #[test]
    fn output_does_not_depend_on_how_the_stream_is_cut() {
        let n = 40_000;
        let x: Vec<f32> = (0..n)
            .map(|i| (0.4 * (2.0 * std::f64::consts::PI * (120.0 * i as f64 / 48_000.0 + 0.6 * (i as f64 / 48_000.0).powi(2) * 200.0)).sin()) as f32)
            .collect();
        let change = 17_321;
        let run_cut = |sizes: &mut dyn FnMut() -> usize| {
            let mut p = shifter(4.0, -20.0);
            let mut y = x.clone();
            let mut pos = 0;
            while pos < n {
                if pos == change {
                    p.set_param("semitones", -9.0);
                }
                let mut len = sizes().min(n - pos);
                if pos < change {
                    len = len.min(change - pos);
                }
                p.process(&mut [&mut y[pos..pos + len]]);
                pos += len;
            }
            y
        };
        let whole = run_cut(&mut || usize::MAX);
        let mut rng = Rng::new(11);
        let cut = run_cut(&mut || (rng.next_u64() % 700) as usize);
        assert!(whole.iter().zip(&cut).all(|(a, b)| a.to_bits() == b.to_bits()));
    }

    /// NaN, infinite and the largest finite samples in a shifted voice give finite output, and
    /// the voice is still shifted afterwards.
    #[test]
    fn non_finite_and_huge_samples_give_finite_output() {
        let mut p = shifter(-7.0, 0.0);
        let mut x = sine(200.0, 0.5, SR as f64, SR as usize * 2, 0.0);
        for (i, bad) in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX, -f32::MAX].into_iter().enumerate() {
            x[3000 + i * 1500] = bad;
            x[3001 + i * 1500] = bad;
        }
        let y = run(&mut p, &x);
        assert!(y.iter().all(|v| v.is_finite()));
        let want = 200.0 * 2f64.powf(-7.0 / 12.0);
        let tail = settled(&y);
        assert!(cents(peak_hz(tail, want * 0.9, want * 1.1), want).abs() < 2.0);
        assert!(db(rms(tail) / (0.5 / std::f64::consts::SQRT_2)).abs() < 1.0, "level {}", rms(tail));
    }

    /// Two-pole resonance magnitude at `f`, 1 at `center`.
    fn resonance(f: f64, center: f64, bandwidth: f64) -> f64 {
        center * bandwidth / ((center * center - f * f).powi(2) + (f * bandwidth).powi(2)).sqrt()
    }

    /// A vowel: harmonics of `f0` up to 6 kHz shaped by two formants (`f1` with an 80 Hz
    /// bandwidth, `f2` with 120 Hz and half the level) over a floor 34 dB down.
    fn vowel(f0: f64, f1: f64, f2: f64, sr: f64, n: usize) -> Vec<f32> {
        let harmonics: Vec<(f64, f64)> = (1..)
            .map(|k| k as f64 * f0)
            .take_while(|f| *f < 6000.0)
            .map(|f| (f, 0.02 + resonance(f, f1, 80.0) + 0.5 * resonance(f, f2, 120.0)))
            .collect();
        (0..n)
            .map(|i| {
                let t = i as f64 / sr;
                (0.08 * harmonics.iter().map(|(f, a)| a * (2.0 * std::f64::consts::PI * f * t).sin()).sum::<f64>()) as f32
            })
            .collect()
    }

    /// The envelope peak of `y` between `lo` and `hi` Hz, read from the amplitudes of the
    /// harmonics of `f0` with a parabola through the strongest one and its neighbors (in dB).
    fn envelope_peak(y: &[f32], f0: f64, lo: f64, hi: f64, sr: f64) -> f64 {
        let amp = |k: usize| tone_amplitude(y, k as f64 * f0, sr).max(1e-12).ln();
        let ks = (lo / f0).ceil() as usize..=(hi / f0).floor() as usize;
        let k = ks.max_by(|a, b| amp(*a).total_cmp(&amp(*b))).unwrap_or(1);
        let (a, b, c) = (amp(k - 1), amp(k), amp(k + 1));
        let denom = a - 2.0 * b + c;
        let shift = if denom.abs() > 1e-12 { (0.5 * (a - c) / denom).clamp(-1.0, 1.0) } else { 0.0 };
        (k as f64 + shift) * f0
    }

    /// The second of `y` after its first half second, at the rate `sr`.
    fn settled_at(y: &[f32], sr: f64) -> &[f32] {
        &y[sr as usize / 2..sr as usize * 3 / 2]
    }

    /// The two formants of the vowel test signal (600 and 2000 Hz) in `y`, each looked for within
    /// a window moved by `ratio`, with `y`'s harmonics at multiples of `f0`.
    fn formants(y: &[f32], f0: f64, ratio: f64, sr: f64) -> (f64, f64) {
        let tail = settled_at(y, sr);
        (envelope_peak(tail, f0, 400.0 * ratio, 1000.0 * ratio, sr), envelope_peak(tail, f0, 1400.0 * ratio, 3000.0 * ratio, sr))
    }

    /// Level of everything off the harmonics of `f0` in `y`'s settled second, against the whole,
    /// in dB.
    fn off_harmonics_db(y: &[f32], f0: f64, sr: f64) -> f64 {
        let tail = settled_at(y, sr);
        let total = rms(tail).powi(2);
        let on: f64 = (1..).map(|k| k as f64 * f0).take_while(|f| *f < sr / 2.0 - f0).map(|f| tone_amplitude(tail, f, sr).powi(2) / 2.0).sum();
        10.0 * ((total - on).max(1e-30) / total).log10()
    }

    /// A formant shift moves both formants of a vowel by the ratio, within a tenth of the move for
    /// the first and 5% for the second, while every partial stays on the harmonics of the
    /// unchanged fundamental. Checked at three voice pitches and three sample rates.
    #[test]
    fn a_formant_shift_moves_the_envelope_and_keeps_the_harmonics() {
        for sr in [44_100.0, 48_000.0, 96_000.0] {
            for f0 in [110.0, 150.0, 220.0] {
                let x = vowel(f0, 600.0, 2000.0, sr, sr as usize * 2);
                let (p1, p2) = formants(&x, f0, 1.0, sr);
                for semis in [-4.0f32, 4.0] {
                    let mut f = VoiceFormant::new(sr as f32, 1);
                    assert!(f.set_param("semitones", semis));
                    let mut y = x.clone();
                    f.process(&mut [&mut y]);
                    let r = 2f64.powf(semis as f64 / 12.0);
                    let (q1, q2) = formants(&y, f0, r, sr);
                    let at = format!("{sr} Hz, f0 {f0}, {semis:+} st: formants {p1:.0} and {p2:.0} moved to {q1:.0} and {q2:.0}");
                    assert!((q1 / (p1 * r) - 1.0).abs() < 0.12, "{at}");
                    assert!((q2 / (p2 * r) - 1.0).abs() < 0.05, "{at}");
                    let off = off_harmonics_db(&y, f0, sr);
                    assert!(off < -30.0, "{at}: {off:.1} dB off the harmonics");
                }
            }
        }
    }

    /// Keep Formants: the pitch moves up a fourth, and the formants of the vowel stay where they
    /// were (where without it they move with the pitch).
    #[test]
    fn keep_formants_holds_the_envelope_while_the_pitch_moves() {
        let (sr, f0) = (SR as f64, 150.0);
        let x = vowel(f0, 600.0, 2000.0, sr, SR as usize * 2);
        let (p1, p2) = formants(&x, f0, 1.0, sr);
        let r = 2f64.powf(5.0 / 12.0);
        for keep in [false, true] {
            let mut p = shifter(5.0, 0.0);
            assert!(p.set_param("formant_follow", if keep { 1.0 } else { 0.0 }));
            p.reset();
            let y = run(&mut p, &x);
            let want = if keep { 1.0 } else { r };
            let (q1, q2) = formants(&y, f0 * r, want, sr);
            let at = format!("keep {keep}: formants {p1:.0} and {p2:.0} read {q1:.0} and {q2:.0}");
            assert!((q1 / (p1 * want) - 1.0).abs() < 0.12, "{at}");
            assert!((q2 / (p2 * want) - 1.0).abs() < 0.05, "{at}");
            assert!(cents(peak_hz(settled(&y), f0 * r * 0.98, f0 * r * 1.02), f0 * r).abs() < 2.0, "keep {keep}: pitch");
        }
    }

    /// Keep Formants adds the formant shifter's latency to the pitch shifter's while it is on, at
    /// each rate.
    #[test]
    fn the_latency_reports_the_formant_stage() {
        for sr in [44_100.0, 48_000.0, 96_000.0] {
            let n = VoiceFormant::new(sr, 2).latency();
            assert!(n > 0, "{sr}");
            let mut p = VoicePitch::new(sr, 2);
            let base = p.latency();
            assert!(p.set_param("formant_follow", 1.0));
            assert_eq!(p.latency(), base + n, "{sr}");
            assert!(p.set_param("formant_follow", 0.0));
            assert_eq!(p.latency(), base, "{sr}");
        }
    }

    /// After a burst of noise ends, the formant shifter and the pitch shifter with Keep Formants
    /// fall silent within `latency + tail_samples`, so an offline render that drains that long
    /// keeps all of the output.
    #[test]
    fn formant_output_ends_within_the_reported_tail() {
        let (n, at, len) = (12_000, 1_777, 1_000);
        let mut rng = Rng::new(17);
        let mut x = vec![0.0f32; n];
        x[at..at + len].iter_mut().for_each(|v| *v = 0.5 * rng.uniform());
        let mut f = VoiceFormant::new(SR, 1);
        assert!(f.set_param("semitones", 7.0));
        let mut p = shifter(-5.0, 0.0);
        assert!(p.set_param("formant_follow", 1.0));
        p.reset();
        let fxs: [&mut dyn AudioEffect; 2] = [&mut f, &mut p];
        for fx in fxs {
            let mut y = x.clone();
            fx.process(&mut [&mut y]);
            let end = at + len + fx.latency() + fx.tail_samples();
            assert!(y[end..].iter().all(|v| *v == 0.0), "{}: output after {end}", fx.id());
        }
    }

    /// Switching Keep Formants on fades the shifter's own output out over 10 ms and then fades the
    /// formant stage's output, 512 samples later, in over 10 ms: the two never sound together,
    /// so their different delays cannot comb-filter the voice. Checked at a shift of 0, where the
    /// stage passes the voice unchanged.
    #[test]
    fn switching_keep_formants_fades_through_silence() {
        let x = vowel(170.0, 650.0, 1900.0, SR as f64, 12_000);
        let at = 4_000;
        let mut plain = shifter(0.0, 0.0);
        let y = run(&mut plain, &x);
        let mut p = shifter(0.0, 0.0);
        let mut out = x.clone();
        p.process(&mut [&mut out[..at]]);
        assert!(p.set_param("formant_follow", 1.0));
        p.process(&mut [&mut out[at..]]);
        let n = p.follow.n;
        let step = 1.0 / (SR as f64 * BYPASS_MS as f64 / 1000.0);
        for i in at..x.len() {
            let m = ((i - at + 1) as f64 * step).min(1.0);
            let want = if m < 0.5 { (1.0 - 2.0 * m) * y[i] as f64 } else { (2.0 * m - 1.0) * y[i - n] as f64 };
            assert!((out[i] as f64 - want).abs() < 1e-4, "sample {i}: {} against {want}", out[i]);
        }
    }

    /// The formant shifter with a shift change part way, and the pitch shifter with Keep Formants
    /// switched on and off part way, give the same output however the stream is cut into blocks.
    #[test]
    fn formant_output_does_not_depend_on_how_the_stream_is_cut() {
        let n = 40_000;
        let x = vowel(130.0, 700.0, 1800.0, SR as f64, n);
        let changes: [(usize, &str, f32); 3] = [(9_001, "semitones", 3.0), (17_321, "formant_follow", 1.0), (29_999, "formant_follow", 0.0)];
        let run_cut = |make: &dyn Fn() -> Box<dyn AudioEffect>, sizes: &mut dyn FnMut() -> usize| {
            let mut fx = make();
            let mut y = x.clone();
            let mut pos = 0;
            while pos < n {
                for (at, id, v) in changes {
                    if pos == at {
                        fx.set_param(id, v);
                    }
                }
                let next = changes.iter().map(|c| c.0).find(|at| *at > pos).unwrap_or(n);
                let len = sizes().min(next - pos);
                fx.process(&mut [&mut y[pos..pos + len]]);
                pos += len;
            }
            y
        };
        let makers: [&dyn Fn() -> Box<dyn AudioEffect>; 2] = [
            &|| {
                let mut f = VoiceFormant::new(SR, 1);
                f.set_param("semitones", -5.0);
                Box::new(f)
            },
            &|| Box::new(shifter(-3.0, 0.0)),
        ];
        for make in makers {
            let whole = run_cut(make, &mut || usize::MAX);
            let mut rng = Rng::new(13);
            let cut = run_cut(make, &mut || (rng.next_u64() % 700) as usize);
            assert!(whole.iter().zip(&cut).all(|(a, b)| a.to_bits() == b.to_bits()));
            assert!(whole.iter().any(|v| v.abs() > 0.05));
        }
    }

    /// NaN, infinite and the largest finite samples in a vowel give finite output from the formant
    /// shifter and from Keep Formants, and the formants are still moved afterwards.
    #[test]
    fn formant_stage_output_stays_finite() {
        let (sr, f0) = (SR as f64, 150.0);
        let mut x = vowel(f0, 600.0, 2000.0, sr, SR as usize * 2);
        let (p1, _) = formants(&x, f0, 1.0, sr);
        for (i, bad) in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX, -f32::MAX].into_iter().enumerate() {
            x[3000 + i * 1500] = bad;
            x[3001 + i * 1500] = bad;
        }
        let mut f = VoiceFormant::new(SR, 2);
        assert!(f.set_param("semitones", 4.0));
        let (mut l, mut r) = (x.clone(), x.clone());
        f.process(&mut [&mut l, &mut r]);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        let ratio = 2f64.powf(4.0 / 12.0);
        let (q1, _) = formants(&l, f0, ratio, sr);
        assert!((q1 / (p1 * ratio) - 1.0).abs() < 0.12, "{q1}");
        let mut p = shifter(-7.0, 0.0);
        assert!(p.set_param("formant_follow", 1.0));
        let y = run(&mut p, &x);
        assert!(y.iter().all(|v| v.is_finite()));
        assert!(rms(settled(&y)) > 0.01);
    }

    /// A block that leaves channels out feeds them silence, so when they come back without a
    /// reset none of their earlier audio sounds. Checked for the formant shifter and for the
    /// pitch shifter with and without Keep Formants.
    #[test]
    fn channels_left_out_of_a_block_come_back_silent() {
        let mut rng = Rng::new(19);
        let noise: Vec<f32> = (0..4_000).map(|_| 0.5 * rng.uniform()).collect();
        let mut f = VoiceFormant::new(SR, 3);
        assert!(f.set_param("semitones", 4.0));
        let mut plain = VoicePitch::new(SR, 3);
        assert!(plain.set_param("semitones", 5.0));
        plain.reset();
        let mut keep = VoicePitch::new(SR, 3);
        assert!(keep.set_param("semitones", 5.0) && keep.set_param("formant_follow", 1.0));
        keep.reset();
        let fxs: [&mut dyn AudioEffect; 3] = [&mut f, &mut plain, &mut keep];
        for fx in fxs {
            let (mut a, mut b, mut c) = (noise.clone(), noise.clone(), noise.clone());
            fx.process(&mut [&mut a, &mut b, &mut c]);
            let mut a = noise.clone();
            fx.process(&mut [&mut a]);
            let (mut a, mut b, mut c) = (vec![0.0f32; 4_000], vec![0.0f32; 4_000], vec![0.0f32; 4_000]);
            fx.process(&mut [&mut a, &mut b, &mut c]);
            assert!(b.iter().chain(&c).all(|v| *v == 0.0), "{}: earlier audio came back", fx.id());
        }
    }

    /// With Keep Formants on, a channel left out of a short block while its line still holds an
    /// impulse passes that impulse to the formant stage, so after it comes back its output matches
    /// a channel that was never left out.
    #[test]
    fn a_short_gap_keeps_the_formant_stage_fed() {
        for semitones in [0.0, 5.0] {
            let mut p = VoicePitch::new(SR, 2);
            assert!(p.set_param("semitones", semitones) && p.set_param("formant_follow", 1.0));
            p.reset();
            let (mut a, mut b) = (vec![0.0f32; 480], vec![0.0f32; 480]);
            a[0] = 1.0;
            b[0] = 1.0;
            p.process(&mut [&mut a, &mut b]);
            // The impulse leaves the delay line during samples 480 to 735, while `b` is left out.
            let mut a = vec![0.0f32; 256];
            p.process(&mut [&mut a]);
            let (mut a, mut b) = (vec![0.0f32; 2_000], vec![0.0f32; 2_000]);
            p.process(&mut [&mut a, &mut b]);
            assert!(a.iter().any(|v| v.abs() > 0.01), "{semitones}: the impulse never came out");
            assert_eq!(a, b, "{semitones}: the left-out channel lost the impulse");
        }
    }

    /// The tracker finds a voice's period down to 70 Hz and reports nothing for silence.
    #[test]
    fn the_tracker_reaches_70_hz() {
        for f in [72.0, 110.0, 440.0, 950.0] {
            let mut t = Tracker::new(SR);
            for v in sine(f, 0.3, SR as f64, SR as usize / 4, 0.0) {
                t.push(v, v);
            }
            let period = t.period.unwrap_or(0.0);
            assert!((SR as f64 / period / f - 1.0).abs() < 0.005, "{f} Hz read as {} Hz", SR as f64 / period);
        }
        let mut t = Tracker::new(SR);
        (0..SR as usize / 4).for_each(|_| t.push(0.0, 0.0));
        assert!(t.period.is_none());
    }
}
