//! Voice pitch shifter for live use: two read taps sweep through a short delay line and crossfade
//! (sin² and cos² gains), so the output moves by the pitch ratio with a latency of 10 ms.
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

use crate::biquad::{Biquad, Coeffs, FilterType};
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

/// Pitch shifter for a live voice, 10 ms of latency (±24 semitones and ±100 cents).
pub struct VoicePitch {
    pv: ParamValues,
    latency: usize,
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
    ];

    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let sample_rate = crate::sane_sample_rate(sample_rate);
        let ch = crate::sane_channels(channels);
        let latency = ((sample_rate * LATENCY_MS / 1000.0).round() as usize).max(4);
        let len = (2 * latency + 8).next_power_of_two();
        let mut s = VoicePitch {
            pv: ParamValues::new(Self::PARAMS),
            latency,
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
        if snap {
            self.wet = self.target_wet();
        }
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
        self.latency
    }
    fn tail_samples(&self) -> usize {
        // A tap reads up to `latency + max_spacing` samples back and the cubic read two samples
        // more, which is twice the latency in all.
        self.latency
    }
    fn reset(&mut self) {
        self.lines.iter_mut().for_each(|l| l.iter_mut().for_each(|v| *v = 0.0));
        self.pos = 0;
        self.tracker.reset();
        self.phase = 0.5;
        self.spacing = self.max_spacing;
        self.wet = self.target_wet();
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let len = block_len(channels);
        let ch = channels.len().min(self.lines.len());
        let dry_at = self.mask + 1 - self.latency;
        for i in 0..len {
            let mut sum = 0.0f32;
            for (line, io) in self.lines.iter_mut().zip(channels.iter()) {
                // A non-finite sample is stored as silence, so it never reaches the output.
                let x = flush32(io[i]);
                line[self.pos] = x;
                sum += x;
            }
            let first = self.lines[0][self.pos];
            self.tracker.push(sum / ch as f32, first);
            let t = self.advance();
            for (line, io) in self.lines.iter().zip(channels.iter_mut()) {
                let dry = line[(self.pos + dry_at) & self.mask] as f64;
                let y = if t.wet == 0.0 {
                    dry
                } else {
                    let shifted =
                        t.gain_a * read(line, self.mask, self.pos, t.delay_a) + (1.0 - t.gain_a) * read(line, self.mask, self.pos, t.delay_b);
                    t.wet * shifted + (1.0 - t.wet) * dry
                };
                io[i] = y.clamp(-f32::MAX as f64, f32::MAX as f64) as f32;
            }
            self.pos = (self.pos + 1) & self.mask;
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
