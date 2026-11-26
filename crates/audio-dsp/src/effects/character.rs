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

use super::util::{DelayLine, Envelope, Lfo, Prng, coef, delay_line_len, echo_tail};
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
                        line.read(FM_BASE + swing * (1.0 + carrier))
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
}
