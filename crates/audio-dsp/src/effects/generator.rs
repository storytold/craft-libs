//! Signal generator: replaces its input with a test tone or noise (line-up tones, sync checks).

use crate::generator::{Oscillator, Waveform};
use crate::{AudioEffect, ParamSpec, ParamValues, Smoothed, Unit, block_len, db_to_gain, param_plumbing};

/// Level at and below which the generator is silent.
const OFF_DB: f32 = -96.0;

/// Test-tone and noise generator. Every channel gets the same signal; the level and frequency
/// glide over 20 ms when changed.
pub struct SignalGenerator {
    pv: ParamValues,
    sr: f32,
    osc: Oscillator,
    wave: Waveform,
    level: Smoothed,
    freq: Smoothed,
}

impl SignalGenerator {
    pub const PARAMS: &'static [ParamSpec] = &[
        ParamSpec::choice("waveform", "Waveform", &["Sine", "Square", "Saw", "Triangle", "White Noise", "Pink Noise"], 0),
        ParamSpec::log("freq", "Frequency", 20.0, 20_000.0, 1_000.0, Unit::Hertz),
        ParamSpec::new("level", "Level", OFF_DB, 0.0, -20.0, Unit::Decibels),
    ];

    pub fn new(sample_rate: f32, _channels: usize) -> Self {
        let sample_rate = crate::sane_sample_rate(sample_rate);
        let mut s = SignalGenerator {
            pv: ParamValues::new(Self::PARAMS),
            sr: sample_rate,
            osc: Oscillator::new(0x00C0_FFEE),
            wave: Waveform::Sine,
            level: Smoothed::with_ms(0.0, sample_rate, 20.0),
            freq: Smoothed::with_ms(1_000.0, sample_rate, 20.0),
        };
        s.apply_params(true);
        s
    }

    fn apply_params(&mut self, snap: bool) {
        self.wave = Waveform::from_index(self.pv.idx("waveform"));
        let db = self.pv.v("level");
        self.level.set(if db <= OFF_DB { 0.0 } else { db_to_gain(db) });
        self.freq.set(self.pv.v("freq"));
        if snap {
            self.level.snap();
            self.freq.snap();
        }
    }
}

impl AudioEffect for SignalGenerator {
    param_plumbing!("signal_generator");
    fn reset(&mut self) {
        self.osc.reset();
        self.level.snap();
        self.freq.snap();
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        for i in 0..n {
            let g = self.level.tick();
            let f = self.freq.tick();
            let y = self.osc.next(self.wave, f / self.sr) * g;
            for ch in channels.iter_mut() {
                ch[i] = y;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{db, tone_amplitude};

    #[test]
    fn the_tone_replaces_the_input_at_the_set_level() {
        let mut g = SignalGenerator::new(48_000.0, 2);
        assert!(g.set_param("level", -12.0));
        g.reset();
        let (mut l, mut r) = (vec![0.7f32; 48_000], vec![-0.7f32; 48_000]);
        g.process(&mut [&mut l, &mut r]);
        assert_eq!(l, r);
        assert!((db(tone_amplitude(&l, 1_000.0, 48_000.0)) + 12.0).abs() < 0.05);
    }

    #[test]
    fn the_lowest_level_is_silence() {
        let mut g = SignalGenerator::new(48_000.0, 1);
        assert!(g.set_param("level", -200.0));
        g.reset();
        let mut x = vec![0.5f32; 1024];
        g.process(&mut [&mut x]);
        assert!(x.iter().all(|v| *v == 0.0));
    }
}
