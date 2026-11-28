//! Noise suppression for a live voice (`noise_suppression`), with 256 samples (5.3 ms) of latency
//! at 48 kHz.
//!
//! The effect runs a short STFT: 256 points at up to 50 kHz (twice that for each doubling of the
//! rate above it), a hop of a quarter of the size, and square-root Hann windows. Each bin's noise
//! power is the smallest value of its smoothed power over the last 1.5 s (minimum statistics,
//! R. Martin, 2001), so the estimate follows a noise floor that changes and needs no "learn noise"
//! step. Each bin's gain is a Wiener gain on the decision-directed a priori SNR (Ephraim and Malah,
//! 1984), held at or above the floor that `reduction` sets, and averaged with its neighbors. At a
//! reduction of 0 every gain is 1 and the effect is a plain delay.
//!
//! A frame enters the noise estimate only after the next four frames have arrived. A frame of
//! digital silence (a mute or a gate) drops the frames still waiting, and the four frames after
//! it do not enter either, so no frame that shares a sample with the silence lowers the estimate
//! and lets the noise pass when the sound resumes, wherever between two hops the silence ends.
//! A mute too short to fill a frame keeps every frame that holds a sample of it out too, down to
//! a run of zeros one hop long (64 samples at 48 kHz); a shorter run of zeros counts as sound.
//! Nor does a frame more than 15 dB below the estimate, until such frames have lasted a whole
//! search window without digital silence between them (the floor has really fallen).
//! The noise power the gains are computed from falls at once and rises by at most 10 dB a
//! second, so a vowel or a note held for longer than the search window keeps its level for a few
//! seconds more, and a noise floor that rises by 12 dB is suppressed again within 3 s.
//! After a reset the gain floor moves from 0 dB toward the attenuation set by `reduction` over
//! the first search window of sound. The first noise estimate after a reset starts from the
//! median bin's power. Each bin's value is clamped between 30 dB below its own estimate and its
//! own estimate. The noise power then rises by at most 10 dB a second. A vowel held from the
//! first sample keeps its level for a few seconds, like one held after a stretch of noise, while
//! steady noise of an even spectrum is suppressed as soon as the floor allows; noise whose
//! spectrum is uneven takes up to 3 s more in the bins well above the median. Continuous voiced
//! or tonal audio present from startup can eventually enter the noise estimate and be attenuated.

use super::stft::{Pairing, Plan, Spectral, StftChannel, stream};
use crate::{AudioEffect, MAX_CHANNELS, ParamSpec, ParamValues, Unit, block_len, db_to_gain, flush32, param_plumbing};

/// Length of the minimum-statistics search, and the number of sub-windows it rotates through.
const SEARCH_S: f32 = 1.5;
const SUBWINDOWS: usize = 8;
/// Time constant of the smoothing of each bin's power.
const POWER_MS: f32 = 10.0;
/// Time constant of the decision-directed a priori SNR.
const PRIORI_MS: f32 = 80.0;
/// Ratio of the mean noise power to the minimum of its smoothed power with these windows (see
/// `the_noise_estimate_follows_white_noise`).
const BIAS: f32 = 3.25;
/// Smallest noise power a bin is divided by, so digital silence keeps its gain at the floor.
const MIN_NOISE: f32 = 1.0e-30;
/// A frame whose power is below the estimated noise power by this ratio (15 dB) is quiet.
const QUIET: f32 = 1.0 / 32.0;
/// Frames that share a sample with any one frame: the hop is a quarter of the STFT size.
const OVERLAP: usize = 4;
/// Lowest the first noise power of a bin starts against the bin's own estimate (30 dB).
const SEED_SPAN: f32 = 1.0e-3;
/// Fastest rise of the noise power a bin's gain is computed from, in dB per second.
const RISE_DB_S: f32 = 10.0;

/// STFT size: 256 at up to 50 kHz (5.3 ms at 48 kHz), doubled for each doubling of the rate above
/// it, up to 8192.
fn suppress_size(sample_rate: f32) -> usize {
    let (mut n, mut top) = (256, 50_000.0);
    while sample_rate > top && n < 8192 {
        n *= 2;
        top *= 2.0;
    }
    n
}

/// Smoothing and floor shared by every frame, from the parameters and the frame rate.
#[derive(Clone, Copy, Debug)]
struct Coefs {
    /// Lowest gain a bin gets.
    floor: f32,
    /// Weight of the previous smoothed power.
    smooth: f32,
    /// Weight of the previous frame's clean power in the a priori SNR.
    priori: f32,
    /// Frames in one minimum-statistics sub-window.
    sub_frames: usize,
    /// Frames in the whole search window.
    window: usize,
    /// Largest ratio of one frame's noise power to the previous frame's (`RISE_DB_S`).
    rise: f32,
}

/// One channel's noise estimate and gains. Every buffer is sized at creation.
#[derive(Clone, Debug)]
struct Bins {
    /// Smoothed power of each bin.
    power: Vec<f32>,
    /// Smallest smoothed power in the current sub-window.
    min: Vec<f32>,
    /// Smallest smoothed power of each of the last `SUBWINDOWS` sub-windows.
    ring: Vec<f32>,
    slot: usize,
    frames: usize,
    /// Power the previous frame kept (gain² × power).
    clean: Vec<f32>,
    gains: Vec<f32>,
    /// Smallest smoothed power of the search window for each bin, for the frame being processed.
    est: Vec<f32>,
    /// Noise power of each bin that the gains are computed from: `est` times the bias, rising by
    /// at most `RISE_DB_S`.
    noise: Vec<f32>,
    /// Power of each bin of the frames waiting to enter the estimate, `OVERLAP` frames of
    /// `bins` values, the oldest at `head`.
    queue: Vec<f32>,
    /// Total power of each waiting frame, and whether it may enter the estimate.
    queue_total: [f32; OVERLAP],
    queue_ok: [bool; OVERLAP],
    /// Whether a mute began between a waiting frame and the one before it.
    queue_cut: [bool; OVERLAP],
    /// Whether a mute began since the last frame was queued.
    cut_pending: bool,
    head: usize,
    queued: usize,
    /// Whether a frame has entered the estimate since the last reset.
    started: bool,
    /// Frames that entered the estimate since the last reset, up to a search window.
    heard: usize,
    /// Frames with sound since the last frame of digital silence, up to `OVERLAP + 1`. It starts
    /// at 1: the zeros the STFT starts with end on a hop boundary, so `OVERLAP` frames clear them.
    since_zero: usize,
    /// Quiet frames in a row since the last frame of digital silence, up to a search window.
    quiet_run: usize,
    /// Whether `noise` has been set from a first estimate since the last reset.
    seeded: bool,
    /// Room to find the median bin of the first estimate.
    scratch: Vec<f32>,
    /// Zero samples in a row up to the last sample, up to a hop.
    zero_run: usize,
    /// Samples since the last sample of a mute (a run of at least a hop of zeros), or `usize::MAX`
    /// when there has been none since the last reset.
    since_mute: usize,
    /// Samples since the last frame.
    since_frame: usize,
}

impl Bins {
    fn new(bins: usize) -> Self {
        Bins {
            power: vec![0.0; bins],
            min: vec![f32::INFINITY; bins],
            ring: vec![f32::INFINITY; bins * SUBWINDOWS],
            slot: 0,
            frames: 0,
            clean: vec![0.0; bins],
            gains: vec![1.0; bins],
            est: vec![f32::INFINITY; bins],
            noise: vec![f32::INFINITY; bins],
            queue: vec![0.0; bins * OVERLAP],
            queue_total: [0.0; OVERLAP],
            queue_ok: [false; OVERLAP],
            queue_cut: [false; OVERLAP],
            cut_pending: false,
            head: 0,
            queued: 0,
            started: false,
            heard: 0,
            since_zero: 1,
            quiet_run: 0,
            seeded: false,
            scratch: vec![0.0; bins],
            zero_run: 0,
            since_mute: usize::MAX,
            since_frame: 0,
        }
    }

    fn reset(&mut self) {
        self.power.fill(0.0);
        self.min.fill(f32::INFINITY);
        self.ring.fill(f32::INFINITY);
        self.clean.fill(0.0);
        self.gains.fill(1.0);
        self.est.fill(f32::INFINITY);
        self.noise.fill(f32::INFINITY);
        self.slot = 0;
        self.frames = 0;
        self.head = 0;
        self.queued = 0;
        self.cut_pending = false;
        self.started = false;
        self.heard = 0;
        self.since_zero = 1;
        self.quiet_run = 0;
        self.seeded = false;
        self.zero_run = 0;
        self.since_mute = usize::MAX;
        self.since_frame = 0;
    }

    /// Takes one input sample before the STFT does, to find mutes too short to give a frame of
    /// digital silence. Once `hop` zeros run in a row, the frame still waiting that holds the first
    /// of them leaves the queue, and so does every later frame that holds a sample of the run.
    fn sample(&mut self, x: f32, hop: usize) {
        self.since_frame += 1;
        self.zero_run = if x == 0.0 { (self.zero_run + 1).min(hop) } else { 0 };
        if self.zero_run < hop {
            self.since_mute = self.since_mute.saturating_add(1);
            return;
        }
        if self.since_mute != 0 {
            // The run has just reached a hop. It started `hop - 1` samples ago, so of the frames
            // waiting only the newest can hold it: when it ended less than a hop ago.
            if self.queued > 0 && self.since_frame < hop {
                self.queue_ok[(self.head + self.queued - 1) % OVERLAP] = false;
            }
            self.cut_pending = true;
        }
        self.since_mute = 0;
    }

    /// Noise power of bin `k` for the frame last processed: the smallest smoothed power of the
    /// search window times the bias, rising by at most `RISE_DB_S`.
    fn noise(&self, k: usize) -> f32 {
        self.noise[k].max(MIN_NOISE)
    }

    /// Moves the oldest waiting frame into the noise estimate, unless it shares a sample with
    /// digital silence, or it is quiet and quiet frames have not yet lasted a search window.
    fn commit(&mut self, c: &Coefs) {
        let bins = self.power.len();
        let at = self.head;
        self.head = (at + 1) % OVERLAP;
        self.queued -= 1;
        if self.queue_cut[at] {
            self.quiet_run = 0;
        }
        if !self.queue_ok[at] {
            return;
        }
        let noise_total = self.est.iter().map(|e| e * BIAS).sum::<f32>();
        let quiet = noise_total.is_finite() && self.queue_total[at] < noise_total * QUIET;
        self.quiet_run = if quiet { (self.quiet_run + 1).min(c.window) } else { 0 };
        if quiet && self.quiet_run < c.window {
            return;
        }
        let a = if self.started { c.smooth } else { 0.0 };
        for k in 0..bins {
            let p = self.queue[at * bins + k];
            self.power[k] = flush32(a * self.power[k] + (1.0 - a) * p);
            self.min[k] = self.min[k].min(self.power[k]);
            self.est[k] = self.est[k].min(self.power[k]);
        }
        self.frames += 1;
        if self.frames == c.sub_frames {
            for k in 0..bins {
                self.ring[self.slot * bins + k] = self.min[k];
                self.min[k] = f32::INFINITY;
            }
            self.slot = (self.slot + 1) % SUBWINDOWS;
            self.frames = 0;
        }
        self.started = true;
        self.heard = (self.heard + 1).min(c.window);
    }

    /// Takes one frame's half spectrum, updates the noise estimate and the gains, and writes the
    /// gains averaged with their neighbors to `out`. A frame with a non-finite bin leaves the
    /// state as it is and repeats the last gains.
    fn frame(&mut self, re: &[f32], im: &[f32], c: &Coefs, out: &mut [f32]) {
        let bins = self.power.len();
        self.since_frame = 0;
        let finite = re.iter().zip(im).fold(true, |ok, (a, b)| ok & (a * a + b * b).is_finite());
        if finite {
            for k in 0..bins {
                self.est[k] = (0..SUBWINDOWS).fold(self.min[k], |m, u| m.min(self.ring[u * bins + k]));
            }
            let total = (0..bins).map(|k| re[k] * re[k] + im[k] * im[k]).sum::<f32>();
            if total == 0.0 {
                // Digital silence: the frames still waiting share a sample with it, and so do the
                // next `OVERLAP - 1` frames, or the next `OVERLAP` when the silence ends between
                // two hops. A quiet run ends here too.
                self.queued = 0;
                self.cut_pending = false;
                self.since_zero = 0;
                self.quiet_run = 0;
            } else {
                self.since_zero = (self.since_zero + 1).min(OVERLAP + 1);
                if self.queued == OVERLAP {
                    self.commit(c);
                }
                let at = (self.head + self.queued) % OVERLAP;
                for k in 0..bins {
                    self.queue[at * bins + k] = re[k] * re[k] + im[k] * im[k];
                }
                self.queue_total[at] = total;
                self.queue_cut[at] = std::mem::take(&mut self.cut_pending);
                // A frame holds the last STFT size of samples, `2 * (bins - 1)`.
                self.queue_ok[at] = self.since_zero > OVERLAP && self.since_mute >= 2 * (bins - 1);
                self.queued += 1;
            }
            if self.started && !self.seeded {
                // The first estimate: a bin well above the median bin, such as a harmonic of a
                // vowel held from the start, starts from the median and rises from there.
                self.seeded = true;
                self.scratch.copy_from_slice(&self.est);
                let median = *self.scratch.select_nth_unstable_by(bins / 2, f32::total_cmp).1 * BIAS;
                for k in 0..bins {
                    let own = self.est[k] * BIAS;
                    self.noise[k] = own.min(median.max(own * SEED_SPAN));
                }
            }
            // The floor moves from 1 to `c.floor` (in dB) over the first search window of sound.
            let floor = c.floor.powf(self.heard as f32 / c.window as f32);
            for k in 0..bins {
                let p = re[k] * re[k] + im[k] * im[k];
                // The noise power falls at once and rises by at most `c.rise` a frame, so a sound
                // held longer than the search window is not taken for the noise at once.
                self.noise[k] = (self.est[k] * BIAS).min(self.noise[k].max(MIN_NOISE) * c.rise);
                let noise = self.noise(k);
                let excess = (p / noise - 1.0).max(0.0);
                let xi = c.priori * self.clean[k] / noise + (1.0 - c.priori) * excess;
                // `1 - 1 / (1 + xi)` is `xi / (1 + xi)`, and stays 1 for an infinite `xi`.
                let g = (1.0 - 1.0 / (1.0 + xi)).max(floor);
                self.gains[k] = g;
                self.clean[k] = flush32(g * g * p);
            }
        }
        for k in 0..bins {
            let lo = self.gains[k.saturating_sub(1)];
            let hi = self.gains[(k + 1).min(bins - 1)];
            out[k] = 0.25 * lo + 0.5 * self.gains[k] + 0.25 * hi;
        }
    }
}

/// Noise suppression for a live voice: lowers steady background noise (fans, hiss, room tone) by
/// up to `reduction` dB, with no step that learns the noise first. Its latency is the STFT size,
/// 256 samples (5.3 ms) at 48 kHz. Continuous voiced or tonal audio present from startup can
/// enter the noise estimate and be attenuated after the initial search window.
pub struct NoiseSuppression {
    pv: ParamValues,
    n: usize,
    hop: usize,
    spec: Spectral,
    stft: Vec<StftChannel>,
    bins: Vec<Bins>,
    /// Gains of the frame being processed, averaged with their neighbors.
    smoothed: Vec<f32>,
    coefs: Coefs,
    sample_rate: f32,
    /// Samples since the last frame, the same in every channel.
    pos: usize,
}

impl NoiseSuppression {
    pub const PARAMS: &'static [ParamSpec] = &[ParamSpec::new("reduction", "Reduction", 0.0, 40.0, 24.0, Unit::Decibels)];

    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let sample_rate = crate::sane_sample_rate(sample_rate);
        let n = suppress_size(sample_rate);
        let bins = n / 2 + 1;
        let ch = crate::sane_channels(channels);
        let mut s = NoiseSuppression {
            pv: ParamValues::new(Self::PARAMS),
            n,
            hop: n / OVERLAP,
            spec: Spectral::new(n, true),
            stft: vec![StftChannel::new(n); ch],
            bins: vec![Bins::new(bins); ch],
            smoothed: vec![1.0; bins],
            coefs: Coefs { floor: 1.0, smooth: 0.0, priori: 0.0, sub_frames: 1, window: SUBWINDOWS, rise: 1.0 },
            sample_rate,
            pos: 0,
        };
        s.apply_params(true);
        s
    }

    fn apply_params(&mut self, _snap: bool) {
        let frame_s = self.hop as f32 / self.sample_rate;
        let sub_frames = ((SEARCH_S / frame_s / SUBWINDOWS as f32).round() as usize).max(1);
        self.coefs = Coefs {
            floor: db_to_gain(-self.pv.v("reduction")),
            smooth: (-frame_s * 1000.0 / POWER_MS).exp(),
            priori: (-frame_s * 1000.0 / PRIORI_MS).exp(),
            sub_frames,
            window: sub_frames * SUBWINDOWS,
            // A power ratio: twice the dB of an amplitude ratio.
            rise: db_to_gain(2.0 * RISE_DB_S * frame_s),
        };
    }
}

impl AudioEffect for NoiseSuppression {
    param_plumbing!("noise_suppression");
    fn latency(&self) -> usize {
        self.n
    }
    fn tail_samples(&self) -> usize {
        self.n
    }
    fn reset(&mut self) {
        self.stft.iter_mut().for_each(StftChannel::reset);
        self.bins.iter_mut().for_each(Bins::reset);
        self.pos = 0;
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let len = block_len(channels);
        let nch = channels.len().min(self.stft.len());
        // A non-finite sample is taken as silence, so it never reaches the output.
        for c in channels.iter_mut().take(nch) {
            c[..len].iter_mut().for_each(|v| *v = flush32(*v));
        }
        let (n, hop, coefs) = (self.n, self.hop, self.coefs);
        let scale = 1.0 / (n as f32 * 2.0);
        let plan = Plan { hop, scale, pairing: Pairing::SynthesisOnly };
        // The block runs in pieces that end at the next frame, so each channel's noise estimate
        // takes every sample of a frame before the STFT frames it.
        let mut start = 0;
        while start < len {
            let end = len.min(start + hop - self.pos);
            for (b, c) in self.bins.iter_mut().zip(channels.iter()).take(nch) {
                c[start..end].iter().for_each(|&x| b.sample(x, hop));
            }
            let mut piece: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| <&mut [f32]>::default());
            for (p, c) in piece.iter_mut().zip(channels.iter_mut()).take(nch) {
                *p = &mut c[start..end];
            }
            let (bins, smoothed) = (&mut self.bins, &mut self.smoothed);
            stream(&mut self.stft, &mut self.spec, &mut piece[..nch], end - start, plan, &mut |ch, _, re, im| {
                bins[ch].frame(re, im, &coefs, smoothed);
                for ((r, i), g) in re.iter_mut().zip(im.iter_mut()).zip(smoothed.iter()) {
                    *r *= g;
                    *i *= g;
                }
            });
            self.pos = (self.pos + end - start) % hop;
            start = end;
        }
        // A channel the block leaves out is fed silence, so it holds none of its earlier audio
        // when it comes back.
        let half = n / 2 + 1;
        let (spec, smoothed) = (&mut self.spec, &mut self.smoothed);
        for (st, b) in self.stft.iter_mut().zip(self.bins.iter_mut()).skip(nch) {
            for _ in 0..len {
                b.sample(0.0, hop);
                st.tick(0.0, hop, &mut |input, out| {
                    spec.analyze(input);
                    b.frame(&spec.re[..half], &spec.im[..half], &coefs, smoothed);
                    for k in 0..half {
                        spec.re[k] *= smoothed[k];
                        spec.im[k] *= smoothed[k];
                    }
                    spec.synthesize_add(out, scale);
                });
            }
        }
        // A huge finite input can overflow the inverse FFT.
        for c in channels.iter_mut().take(nch) {
            c[..len].iter_mut().for_each(|v| *v = flush32(*v));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    const SR: f32 = 48000.0;

    fn suppressor(reduction: f32) -> NoiseSuppression {
        let mut s = NoiseSuppression::new(SR, 1);
        assert!(s.set_param("reduction", reduction));
        s.reset();
        s
    }

    fn white(seed: u64, n: usize, amp: f32) -> Vec<f32> {
        let mut r = Rng::new(seed);
        (0..n).map(|_| r.uniform() * amp).collect()
    }

    /// Voiced syllables (seven harmonics of a gliding pitch from 110 to 230 Hz under a sine
    /// envelope) with pauses between them, at a peak level of about 0.5.
    fn voiced(sr: f64, secs: f64, seed: u64) -> Vec<f32> {
        let n = (sr * secs) as usize;
        let mut rng = Rng::new(seed);
        let mut out = vec![0.0f32; n];
        let mut i = 0usize;
        while i < n {
            let len = ((0.12 + 0.2 * (rng.uniform() * 0.5 + 0.5) as f64) * sr) as usize;
            let f0 = 110.0 + 120.0 * (rng.uniform() as f64 * 0.5 + 0.5);
            let glide = 1.0 + 0.3 * rng.uniform() as f64;
            let amp = 0.1 + 0.1 * (rng.uniform() * 0.5 + 0.5);
            for k in 0..len.min(n - i) {
                let t = k as f64 / sr;
                let env = (std::f64::consts::PI * k as f64 / len as f64).sin() as f32;
                let f = f0 * (1.0 + (glide - 1.0) * k as f64 / len as f64);
                let v: f32 = (1..8).map(|h| ((2.0 * std::f64::consts::PI * f * h as f64 * t).sin() / h as f64) as f32).sum();
                out[i + k] = v * env * amp;
            }
            i += len + ((rng.uniform() * 0.5 + 0.5) * 0.25 * sr as f32) as usize;
        }
        out
    }

    /// Runs the suppressor's gains, computed from `speech + noise`, on the speech and the noise
    /// apart, so the SNR before and after can be measured. Returns the processed mix, speech and
    /// noise.
    fn shadow(speech: &[f32], noise: &[f32], reduction: f32) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let s = suppressor(reduction);
        let (n, hop, coefs) = (s.n, s.hop, s.coefs);
        let mut spec = Spectral::new(n, true);
        let half = n / 2 + 1;
        let scale = 1.0 / (n as f32 * 2.0);
        let mut bins = Bins::new(half);
        let mut gains = vec![1.0f32; half];
        let mut lanes = [StftChannel::new(n), StftChannel::new(n), StftChannel::new(n)];
        let mut out = [vec![0.0f32; speech.len()], vec![0.0f32; speech.len()], vec![0.0f32; speech.len()]];
        for i in 0..speech.len() {
            let x = [speech[i] + noise[i], speech[i], noise[i]];
            bins.sample(x[0], hop);
            for (lane, (st, o)) in lanes.iter_mut().zip(out.iter_mut()).enumerate() {
                o[i] = st.tick(x[lane], hop, &mut |input, frame_out| {
                    spec.analyze(input);
                    if lane == 0 {
                        bins.frame(&spec.re[..half], &spec.im[..half], &coefs, &mut gains);
                    }
                    for k in 0..half {
                        spec.re[k] *= gains[k];
                        spec.im[k] *= gains[k];
                    }
                    spec.synthesize_add(frame_out, scale);
                });
            }
        }
        let [mix, speech, noise] = out;
        (mix, speech, noise)
    }

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|&v| v as f64 * v as f64).sum()
    }

    /// SNR before and after the suppressor at a reduction of 24 dB, and the change of the voice's
    /// own level, in dB, measured from 3 s on (the noise estimate has settled by then).
    fn snr_gain(speech: &[f32], noise_amp: f32) -> (f64, f64, f64) {
        let len = speech.len();
        let noise = white(17, len, noise_amp);
        let (mix, ys, yv) = shadow(speech, &noise, 24.0);
        let mut real: Vec<f32> = speech.iter().zip(&noise).map(|(a, b)| a + b).collect();
        suppressor(24.0).process(&mut [&mut real]);
        assert!(real.iter().zip(&mix).all(|(a, b)| (a - b).abs() < 1e-5), "the shadow run differs from the effect");
        let lat = 256;
        let (from, to) = (SR as usize * 3, len - lat);
        let after = from + lat..to + lat;
        let snr_in = db((energy(&speech[from..to]) / energy(&noise[from..to])).sqrt());
        let snr_out = db((energy(&ys[after.clone()]) / energy(&yv[after.clone()])).sqrt());
        let kept = db((energy(&ys[after]) / energy(&speech[from..to])).sqrt());
        (snr_in, snr_out - snr_in, kept)
    }

    /// Voiced syllables in white noise come out with an SNR at least 15 dB higher at an input SNR
    /// of 7 dB (13 dB higher at 17 dB), and the voice's level changes by less than 3 dB.
    #[test]
    fn the_voice_gains_15_db_of_snr_and_keeps_its_level() {
        let speech = voiced(SR as f64, 12.0, 4);
        for (noise_amp, want) in [(0.06f32, 15.0), (0.02, 13.0)] {
            let (snr_in, gain, kept) = snr_gain(&speech, noise_amp);
            assert!(gain >= want, "input SNR {snr_in:.1} dB: SNR gain {gain:.1} dB");
            assert!(kept.abs() < 3.0, "input SNR {snr_in:.1} dB: voice level changed by {kept:.2} dB");
        }
    }

    /// Speech with consonants (bursts of broadband noise at the level of the vowels) keeps its
    /// level within 3 dB and still gains SNR. Bins where a consonant covers the noise keep both.
    #[test]
    fn speech_with_consonants_keeps_its_level() {
        let speech = speechy(SR as f64, 12.0, 4);
        for noise_amp in [0.06f32, 0.02] {
            let (snr_in, gain, kept) = snr_gain(&speech, noise_amp);
            assert!(gain >= 4.0, "input SNR {snr_in:.1} dB: SNR gain {gain:.1} dB");
            assert!(kept.abs() < 3.0, "input SNR {snr_in:.1} dB: speech level changed by {kept:.2} dB");
        }
    }

    /// Steady noise falls by close to the reduction setting, at 44.1, 48, 96 and 192 kHz, and the
    /// latency is 256 samples up to 50 kHz and doubles with each doubling of the rate above it.
    #[test]
    fn steady_noise_falls_by_the_reduction() {
        for (sr, lat) in [(44_100.0f32, 256), (48_000.0, 256), (96_000.0, 512), (192_000.0, 1024)] {
            for reduction in [6.0f32, 12.0, 24.0, 40.0] {
                let mut s = NoiseSuppression::new(sr, 1);
                assert!(s.set_param("reduction", reduction));
                assert_eq!(s.latency(), lat);
                let n = sr as usize * 4;
                let x = white(5, n, 0.05);
                let mut y = x.clone();
                s.process(&mut [&mut y]);
                let tail = n - sr as usize..n;
                let got = -db(rms(&y[tail.clone()]) / rms(&x[tail]));
                // At 40 dB the Wiener gains of noise-only bins sit above the floor.
                let limit = if reduction == 40.0 { 28.0 } else { reduction as f64 - 2.0 };
                assert!(got >= limit && got <= reduction as f64 + 1.0, "{sr} Hz, reduction {reduction}: noise fell by {got:.1} dB");
            }
        }
    }

    /// At a reduction of 0 the output is the input delayed by the latency.
    #[test]
    fn zero_reduction_is_a_delay() {
        let mut s = suppressor(0.0);
        let lat = s.latency();
        let x = speechy(SR as f64, 2.0, 8);
        let mut y = x.clone();
        s.process(&mut [&mut y]);
        for i in lat..x.len() {
            assert!((y[i] - x[i - lat]).abs() < 1e-5, "{i}: {} vs {}", y[i], x[i - lat]);
        }
    }

    /// The noise estimate follows a noise floor that rises by 12 dB within 3 s, and one that falls
    /// by 12 dB at once.
    #[test]
    fn the_estimate_follows_a_changing_noise_floor() {
        let level = |amps: [f32; 2], at: f32| -> f64 {
            let mut s = suppressor(24.0);
            let n = SR as usize * 8;
            let mut rng = Rng::new(31);
            let x: Vec<f32> = (0..n).map(|i| rng.uniform() * if i < n / 2 { amps[0] } else { amps[1] }).collect();
            let mut y = x.clone();
            s.process(&mut [&mut y]);
            let from = n / 2 + (at * SR) as usize;
            let range = from..from + SR as usize / 4;
            -db(rms(&y[range.clone()]) / rms(&x[range]))
        };
        let up = level([0.02, 0.08], 3.0);
        assert!(up >= 20.0, "rise: noise fell by {up:.1} dB 3 s later");
        let down = level([0.08, 0.02], 0.25);
        assert!(down >= 20.0, "fall: noise fell by {down:.1} dB 0.25 s later");
    }

    /// Noise that starts after digital silence is suppressed again within the search window.
    #[test]
    fn noise_after_silence_is_suppressed_again() {
        let mut s = suppressor(24.0);
        let n = SR as usize * 6;
        let mut x = white(41, n, 0.05);
        x[..SR as usize * 2].fill(0.0);
        let mut y = x.clone();
        s.process(&mut [&mut y]);
        let tail = n - SR as usize / 2..n;
        let got = -db(rms(&y[tail.clone()]) / rms(&x[tail]));
        assert!(got >= 20.0, "noise fell by {got:.1} dB");
    }

    /// Noise that resumes after a pause of digital silence, or of noise 40 dB lower, is suppressed
    /// from the start: the pause does not enter the noise estimate.
    #[test]
    fn noise_after_a_pause_is_suppressed_at_once() {
        for pause_amp in [0.0f32, 0.0005] {
            let mut s = suppressor(24.0);
            let lat = s.latency();
            let n = SR as usize * 5;
            let (pause, resume) = (SR as usize * 3, SR as usize * 4);
            let mut x = white(43, n, 0.05);
            x[pause..resume].iter_mut().for_each(|v| *v *= pause_amp / 0.05);
            let mut y = x.clone();
            s.process(&mut [&mut y]);
            let range = resume + lat + SR as usize / 50..resume + lat + SR as usize / 2;
            let got = -db(rms(&y[range.clone()]) / rms(&x[range.start - lat..range.end - lat]));
            assert!(got >= 20.0, "pause level {pause_amp}: noise fell by {got:.1} dB after the pause");
        }
    }

    /// No frame that shares a sample with digital silence enters the noise estimate: not the
    /// frames that run into a mute, nor those that run out of it, whether or not the mute starts
    /// or ends on a frame boundary. At most two frames per mute that hold no silence are dropped
    /// with them.
    #[test]
    fn frames_that_touch_a_mute_stay_out_of_the_estimate() {
        let s = suppressor(24.0);
        let (n, hop, coefs) = (s.n, s.hop, s.coefs);
        let half = n / 2 + 1;
        for offset in [0usize, 17, 40, 63] {
            let mut spec = Spectral::new(n, true);
            let mut bins = Bins::new(half);
            let mut gains = vec![1.0f32; half];
            let mut lane = StftChannel::new(n);
            let len = SR as usize;
            let mut x = white(29, len, 0.05);
            let mutes = [(SR as usize * 3 / 10 + offset, SR as usize * 4 / 10 + offset), (SR as usize * 7 / 10 + offset, SR as usize * 8 / 10)];
            for &(a, b) in &mutes {
                x[a..b].fill(0.0);
            }
            assert!(x.iter().enumerate().all(|(i, v)| *v != 0.0 || mutes.iter().any(|&(a, b)| (a..b).contains(&i))));
            let mut frames = 0usize;
            // A frame enters the estimate when the frame `OVERLAP` after it arrives, which is when
            // `heard` grows.
            let mut entered = Vec::new();
            for &v in &x {
                lane.tick(v, hop, &mut |input, _| {
                    spec.analyze(input);
                    let heard = bins.heard;
                    bins.frame(&spec.re[..half], &spec.im[..half], &coefs, &mut gains);
                    frames += 1;
                    if bins.heard > heard {
                        entered.push(frames - OVERLAP);
                    }
                });
            }
            // Frame `j` (from 1) holds samples `j * hop - n .. j * hop`; samples before 0 are
            // the zeros the STFT starts with.
            let holds_no_silence = |j: usize| j * hop >= n && x[j * hop - n..j * hop].iter().all(|v| *v != 0.0);
            let clean = (1..=frames).filter(|&j| holds_no_silence(j)).count();
            assert!(frames < coefs.window, "the test needs `heard` below its cap");
            assert_eq!(entered.len(), bins.heard);
            for &j in &entered {
                assert!(holds_no_silence(j), "offset {offset}: frame {j} holds silence and entered the estimate");
            }
            assert!(bins.heard + 2 * mutes.len() + OVERLAP >= clean, "offset {offset}: {} of {clean} frames entered", bins.heard);
        }
    }

    /// A mute too short to fill a frame, from one hop long up, keeps the frames that hold a sample
    /// of it out of the estimate too, wherever it starts and ends between two hops.
    #[test]
    fn frames_that_touch_a_short_mute_stay_out_of_the_estimate() {
        for offset in [0usize, 17, 40, 63] {
            let mut s = suppressor(24.0);
            let (n, hop) = (s.n, s.hop);
            let mut x = white(31, SR as usize / 2, 0.05);
            // The last mute is one STFT size long and holds no whole frame unless it starts on a hop.
            let mutes = [
                (192 + offset, 256 + offset),
                (6000 + offset, 6100 + offset),
                (12_000 + offset, 12_200 + offset),
                (17_984 + offset, 17_984 + offset + n),
            ];
            for &(a, b) in &mutes {
                x[a..b].fill(0.0);
            }
            assert!(x.iter().enumerate().all(|(i, v)| *v != 0.0 || mutes.iter().any(|&(a, b)| (a..b).contains(&i))));
            // One hop per block, so each block ends with a frame. A frame enters the estimate when
            // the frame `OVERLAP` after it arrives, which is when `heard` grows.
            let mut entered = Vec::new();
            for (k, chunk) in x.chunks(hop).enumerate() {
                let heard = s.bins[0].heard;
                let mut y = chunk.to_vec();
                s.process(&mut [&mut y]);
                if s.bins[0].heard > heard {
                    entered.push(k + 1 - OVERLAP);
                }
            }
            let frames = x.len() / hop;
            let holds_no_silence = |j: usize| j * hop >= n && x[j * hop - n..j * hop].iter().all(|v| *v != 0.0);
            let clean = (1..=frames).filter(|&j| holds_no_silence(j)).count();
            assert!(frames < s.coefs.window, "the test needs `heard` below its cap");
            assert_eq!(entered.len(), s.bins[0].heard);
            for &j in &entered {
                assert!(holds_no_silence(j), "offset {offset}: frame {j} holds silence and entered the estimate");
            }
            assert!(s.bins[0].heard + 2 * mutes.len() + OVERLAP >= clean, "offset {offset}: {} of {clean} frames entered", s.bins[0].heard);
        }
    }

    /// Quiet stretches that a mute separates do not add up to a search window: the estimate stays
    /// at the noise before them, and falls only after a whole search window of quiet in a row.
    #[test]
    fn a_mute_ends_a_quiet_run() {
        let s = suppressor(24.0);
        let (n, hop, coefs) = (s.n, s.hop, s.coefs);
        let half = n / 2 + 1;
        let mut spec = Spectral::new(n, true);
        let mut bins = Bins::new(half);
        let mut gains = vec![1.0f32; half];
        let mut lane = StftChannel::new(n);
        let window = coefs.window * hop;
        let amp = 0.05f32;
        let (loud, quiet, mute) = (SR as usize * 2, window * 6 / 10, SR as usize / 5);
        let mut x = white(47, loud + 2 * quiet + mute + window + quiet, amp);
        x[loud..].iter_mut().for_each(|v| *v *= 0.01);
        x[loud + quiet..loud + quiet + mute].fill(0.0);
        let want = amp * amp / 3.0 * n as f32 / 2.0;
        let mut feed = |part: &[f32]| -> f64 {
            for &v in part {
                lane.tick(v, hop, &mut |input, _| {
                    spec.analyze(input);
                    bins.frame(&spec.re[..half], &spec.im[..half], &coefs, &mut gains);
                });
            }
            (1..half - 1).map(|k| db((bins.est[k] as f64 * BIAS as f64 / want as f64).sqrt())).sum::<f64>() / (half - 2) as f64
        };
        let split = loud + 2 * quiet + mute;
        let kept = feed(&x[..split]);
        assert!(kept > -6.0, "after two quiet stretches split by a mute the estimate fell by {:.1} dB", -kept);
        let fell = feed(&x[split..]);
        assert!(fell < -20.0, "after a search window of quiet the estimate fell by only {:.1} dB", -fell);
    }

    /// A mute of one hop, which leaves no frame of digital silence, ends the quiet run where it
    /// falls in the stream: the quiet frames waiting before it still count, and only the frames
    /// after the mute that may enter the estimate start the next run.
    #[test]
    fn a_short_mute_ends_the_quiet_run_in_queue_order() {
        let s = suppressor(24.0);
        let (n, hop, coefs) = (s.n, s.hop, s.coefs);
        let half = n / 2 + 1;
        let mut bins = Bins::new(half);
        let mut gains = vec![1.0f32; half];
        let im = vec![0.0f32; half];
        let (loud, quiet) = (vec![1.0f32; half], vec![0.001f32; half]);
        let mut step = |bins: &mut Bins, sample: f32, spectrum: &[f32]| {
            for _ in 0..hop {
                bins.sample(sample, hop);
            }
            bins.frame(spectrum, &im, &coefs, &mut gains);
        };
        for _ in 0..40 {
            step(&mut bins, 0.1, &loud);
        }
        for _ in 0..OVERLAP + 3 {
            step(&mut bins, 0.001, &quiet);
        }
        assert!(bins.quiet_run > 0, "the quiet frames before the mute did not count");
        step(&mut bins, 0.0, &quiet);
        let mut eligible = 0;
        for _ in 0..2 * n / hop {
            step(&mut bins, 0.001, &quiet);
            let newest = (bins.head + bins.queued - 1) % OVERLAP;
            eligible += usize::from(bins.queue_ok[newest]);
        }
        let waiting = (0..bins.queued).filter(|j| bins.queue_ok[(bins.head + j) % OVERLAP]).count();
        assert_eq!(waiting, bins.queued, "a frame waiting after the mute may not enter the estimate");
        assert!(eligible + 3 < coefs.window, "the test needs the run below its cap");
        assert_eq!(bins.quiet_run, eligible - waiting, "frames from before the mute counted in the new run");
    }

    #[test]
    fn a_short_mute_ends_the_queued_quiet_run() {
        for offset in [0, 17] {
            let mut s = suppressor(24.0);
            let (n, hop) = (s.n, s.hop);
            let mut loud = white(47, SR as usize * 2, 0.05);
            s.process(&mut [&mut loud]);
            let mut quiet = white(51, 20 * hop + offset, 0.0005);
            s.process(&mut [&mut quiet]);
            assert!(s.bins[0].quiet_run > 0);
            let mute_end = loud.len() + quiet.len() + hop;
            let first_clean_frame = (mute_end + n).div_ceil(hop);
            let commit_end = (first_clean_frame + OVERLAP) * hop;
            let mut after = white(53, commit_end - loud.len() - quiet.len(), 0.0005);
            after[..hop].fill(0.0);
            let before_mute = s.bins[0].quiet_run;
            s.process(&mut [&mut after[..hop]]);
            assert_eq!(s.bins[0].quiet_run, before_mute + 1, "offset {offset}: queued pre-mute frames lost their quiet run");
            s.process(&mut [&mut after[hop..]]);
            assert_eq!(s.bins[0].quiet_run, 1, "offset {offset}: pre-mute quiet frames contributed to the new run");
        }
    }

    /// A voice that starts with the stream keeps its level within 3 dB over its first 0.25 s and
    /// its first 1.5 s, while the suppressor has no noise estimate yet. One of the voices starts on
    /// its first sample with a vowel held for 0.25 s.
    #[test]
    fn a_voice_at_the_start_keeps_its_level() {
        let mut abrupt = voiced(SR as f64, 2.0, 7);
        let vowel = SR as usize / 4;
        let len = abrupt.len();
        abrupt.copy_within(..len - vowel, vowel);
        for (i, v) in abrupt[..vowel].iter_mut().enumerate() {
            let t = std::f64::consts::TAU * 160.0 * i as f64 / SR as f64;
            *v = (1..8).map(|h| (0.15 * (h as f64 * t).sin() / h as f64) as f32).sum();
        }
        for speech in [abrupt, voiced(SR as f64, 2.0, 6), speechy(SR as f64, 2.0, 6)] {
            let noise = white(19, speech.len(), 0.02);
            let (_, ys, _) = shadow(&speech, &noise, 24.0);
            let lat = 256;
            for to in [SR as usize / 4, SR as usize * 3 / 2] {
                let kept = db((energy(&ys[lat..to + lat]) / energy(&speech[..to])).sqrt());
                assert!(kept.abs() < 3.0, "voice level changed by {kept:.2} dB in the first {to} samples");
            }
        }
    }

    /// A steady vowel held for 3 s in noise, longer than the search window, keeps its level within
    /// 3 dB over its last second.
    #[test]
    fn a_held_vowel_keeps_its_level() {
        let (lead, held) = (SR as usize * 2, SR as usize * 3);
        let mut speech = vec![0.0f32; lead + held + SR as usize / 2];
        for (i, v) in speech[lead..lead + held].iter_mut().enumerate() {
            let t = std::f64::consts::TAU * 160.0 * i as f64 / SR as f64;
            let fade = (i as f32 / 480.0).min(1.0);
            *v = fade * (1..8).map(|h| (0.15 * (h as f64 * t).sin() / h as f64) as f32).sum::<f32>();
        }
        let noise = white(37, speech.len(), 0.02);
        let (_, ys, _) = shadow(&speech, &noise, 24.0);
        let lat = 256;
        let last = lead + held - SR as usize..lead + held;
        let kept = db((energy(&ys[last.start + lat..last.end + lat]) / energy(&speech[last])).sqrt());
        assert!(kept.abs() < 3.0, "the held vowel's level changed by {kept:.2} dB in its last second");
    }

    /// A steady vowel held from the first sample, with no noise before it, keeps its level within
    /// 3 dB over its first 2 s, after the floor has reached the reduction.
    #[test]
    fn a_vowel_held_from_the_start_keeps_its_level() {
        let held = SR as usize * 3;
        let mut speech = vec![0.0f32; held + SR as usize / 2];
        for (i, v) in speech[..held].iter_mut().enumerate() {
            let t = std::f64::consts::TAU * 160.0 * i as f64 / SR as f64;
            *v = (1..8).map(|h| (0.15 * (h as f64 * t).sin() / h as f64) as f32).sum();
        }
        let noise = white(53, speech.len(), 0.02);
        let (_, ys, _) = shadow(&speech, &noise, 24.0);
        let lat = 256;
        for range in [0..SR as usize * 2, SR as usize * 3 / 2..SR as usize * 2] {
            let kept = db((energy(&ys[range.start + lat..range.end + lat]) / energy(&speech[range.clone()])).sqrt());
            assert!(kept.abs() < 3.0, "the vowel's level changed by {kept:.2} dB over {range:?}");
        }
    }

    #[test]
    fn a_hop_periodic_startup_vowel_keeps_its_level_at_16_khz() {
        let sr = 16_000.0;
        let held = sr as usize * 3;
        let mut speech = vec![0.0f32; held + sr as usize / 2];
        for (i, v) in speech[..held].iter_mut().enumerate() {
            let t = std::f64::consts::TAU * 250.0 * i as f64 / sr as f64;
            *v = (1..8).map(|h| (0.15 * (h as f64 * t).sin() / h as f64) as f32).sum();
        }
        let mut s = NoiseSuppression::new(sr, 1);
        assert!(s.set_param("reduction", 24.0));
        s.reset();
        assert_eq!(s.hop, sr as usize / 250);
        let lat = s.latency();
        let mut y = speech.clone();
        s.process(&mut [&mut y]);
        for range in [0..sr as usize * 2, sr as usize * 3 / 2..sr as usize * 2] {
            let kept = db((energy(&y[range.start + lat..range.end + lat]) / energy(&speech[range.clone()])).sqrt());
            assert!(kept.abs() < 3.0, "the 250 Hz startup vowel at 16 kHz changed by {kept:.2} dB over {range:?}");
        }
    }

    /// Steady noise whose spectrum falls with frequency (white noise through a one-pole low-pass
    /// at 500 Hz) is suppressed by at least 20 dB from 4 s on.
    #[test]
    fn steady_low_passed_noise_falls_by_the_reduction() {
        let mut s = suppressor(24.0);
        let n = SR as usize * 5;
        let a = (-std::f32::consts::TAU * 500.0 / SR).exp();
        let mut state = 0.0f32;
        let x: Vec<f32> = white(59, n, 0.2)
            .into_iter()
            .map(|v| {
                state = a * state + (1.0 - a) * v;
                state
            })
            .collect();
        let mut y = x.clone();
        s.process(&mut [&mut y]);
        let lat = s.latency();
        let from = SR as usize * 4;
        let got = -db(rms(&y[from + lat..n]) / rms(&x[from..n - lat]));
        assert!(got >= 20.0, "noise fell by {got:.1} dB");
    }

    /// NaN, infinite and the largest finite samples give finite output, and the noise is still
    /// suppressed afterwards.
    #[test]
    fn non_finite_and_huge_samples_give_finite_output() {
        let mut s = suppressor(24.0);
        let n = SR as usize * 4;
        let x = white(13, n, 0.05);
        let mut y = x.clone();
        for (i, bad) in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX, -f32::MAX].into_iter().enumerate() {
            y[3000 + i * 1500] = bad;
            y[3001 + i * 1500] = bad;
        }
        s.process(&mut [&mut y]);
        assert!(y.iter().all(|v| v.is_finite()));
        let tail = n - SR as usize / 2..n;
        let got = -db(rms(&y[tail.clone()]) / rms(&x[tail]));
        assert!(got >= 20.0, "noise fell by {got:.1} dB");
    }

    /// A block that leaves channels out feeds them silence, so when they come back without a
    /// reset none of their earlier audio sounds.
    #[test]
    fn channels_left_out_of_a_block_come_back_silent() {
        let noise = white(23, 4_000, 0.5);
        let mut s = NoiseSuppression::new(SR, 3);
        let (mut a, mut b, mut c) = (noise.clone(), noise.clone(), noise.clone());
        s.process(&mut [&mut a, &mut b, &mut c]);
        // The first channel is silent too, so the paired inverse FFT carries none of its audio
        // into the second channel.
        let mut a = vec![0.0f32; 4_000];
        s.process(&mut [&mut a]);
        let (mut a, mut b, mut c) = (vec![0.0f32; 4_000], vec![0.0f32; 4_000], vec![0.0f32; 4_000]);
        s.process(&mut [&mut a, &mut b, &mut c]);
        assert!(b.iter().chain(&c).all(|v| *v == 0.0), "earlier audio came back");
    }

    /// On steady white noise, the noise estimate, averaged over the bins, lands within 1 dB of the
    /// noise's mean power once the search window has filled.
    #[test]
    fn the_noise_estimate_follows_white_noise() {
        let s = suppressor(24.0);
        let (n, hop, coefs) = (s.n, s.hop, s.coefs);
        let half = n / 2 + 1;
        let mut spec = Spectral::new(n, true);
        let mut bins = Bins::new(half);
        let mut gains = vec![1.0f32; half];
        let mut lane = StftChannel::new(n);
        let amp = 0.1f32;
        let x = white(3, SR as usize * 4, amp);
        for &v in &x {
            lane.tick(v, hop, &mut |input, _| {
                spec.analyze(input);
                bins.frame(&spec.re[..half], &spec.im[..half], &coefs, &mut gains);
            });
        }
        // uniform noise of amplitude `amp` has a variance of amp²/3; each bin of the
        // square-root-Hann windowed FFT gets n/2 of that
        let want = amp * amp / 3.0 * n as f32 / 2.0;
        let mean = (1..half - 1).map(|k| db((bins.noise(k) as f64 / want as f64).sqrt())).sum::<f64>() / (half - 2) as f64;
        assert!(mean.abs() < 1.0, "the noise estimate is {mean:.2} dB from the mean power");
    }
}
