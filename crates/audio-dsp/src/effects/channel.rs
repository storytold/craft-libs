//! Gain and channel utilities: amplify, channel volume, balance/pan, invert, swap, fill.

use crate::{AudioEffect, ParamSpec, ParamValues, Smoothed, Unit, block_len, db_to_gain, param_plumbing};
use std::f32::consts::FRAC_PI_2;

const RAMP_MS: f32 = 20.0;

/// Amplify: one smoothed gain for all channels.
pub struct Amplify {
    pv: ParamValues,
    gain: Smoothed,
}

impl Amplify {
    pub const PARAMS: &'static [ParamSpec] = &[ParamSpec::new("gain", "Gain", -96.0, 24.0, 0.0, Unit::Decibels)];

    pub fn new(sample_rate: f32, _channels: usize) -> Self {
        let sample_rate = crate::sane_sample_rate(sample_rate);
        let mut s = Amplify { pv: ParamValues::new(Self::PARAMS), gain: Smoothed::with_ms(1.0, sample_rate, RAMP_MS) };
        s.apply_params(true);
        s
    }
    fn apply_params(&mut self, snap: bool) {
        let db = self.pv.v("gain");
        self.gain.set(if db <= -96.0 { 0.0 } else { db_to_gain(db) });
        if snap {
            self.gain.snap();
        }
    }
}

impl AudioEffect for Amplify {
    param_plumbing!("amplify");
    fn reset(&mut self) {
        self.gain.snap();
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        for i in 0..n {
            let g = self.gain.tick();
            for ch in channels.iter_mut() {
                ch[i] *= g;
            }
        }
    }
}

/// Per-channel volume (L R C LFE Ls Rs order).
pub struct ChannelVolume {
    pv: ParamValues,
    gains: [Smoothed; 6],
}

const CHANNEL_IDS: [&str; 6] = ["left", "right", "center", "lfe", "left_surround", "right_surround"];

impl ChannelVolume {
    pub const PARAMS: &'static [ParamSpec] = &[
        ParamSpec::new("left", "Left", -96.0, 24.0, 0.0, Unit::Decibels),
        ParamSpec::new("right", "Right", -96.0, 24.0, 0.0, Unit::Decibels),
        ParamSpec::new("center", "Center", -96.0, 24.0, 0.0, Unit::Decibels),
        ParamSpec::new("lfe", "LFE", -96.0, 24.0, 0.0, Unit::Decibels),
        ParamSpec::new("left_surround", "Left Surround", -96.0, 24.0, 0.0, Unit::Decibels),
        ParamSpec::new("right_surround", "Right Surround", -96.0, 24.0, 0.0, Unit::Decibels),
    ];

    pub fn new(sample_rate: f32, _channels: usize) -> Self {
        let sample_rate = crate::sane_sample_rate(sample_rate);
        let mut s = ChannelVolume { pv: ParamValues::new(Self::PARAMS), gains: [Smoothed::with_ms(1.0, sample_rate, RAMP_MS); 6] };
        s.apply_params(true);
        s
    }
    fn apply_params(&mut self, snap: bool) {
        for (g, id) in self.gains.iter_mut().zip(CHANNEL_IDS) {
            let db = self.pv.v(id);
            g.set(if db <= -96.0 { 0.0 } else { db_to_gain(db) });
            if snap {
                g.snap();
            }
        }
    }
}

impl AudioEffect for ChannelVolume {
    param_plumbing!("channel_volume");
    fn reset(&mut self) {
        self.gains.iter_mut().for_each(Smoothed::snap);
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        for i in 0..n {
            for (c, g) in self.gains.iter_mut().enumerate() {
                let g = g.tick();
                if let Some(ch) = channels.get_mut(c) {
                    ch[i] *= g;
                }
            }
        }
    }
}

/// Constant-power gains `(left, right)` for a pan position in −1 … +1 (center = −3 dB each).
pub fn constant_power_pan(p: f32) -> (f32, f32) {
    let theta = (p.clamp(-1.0, 1.0) + 1.0) * 0.25 * std::f32::consts::PI;
    (theta.cos(), theta.sin())
}

/// Balance gains for a stereo source: the side opposite the direction is attenuated with a
/// cosine (constant-power) law, the other side stays at unity. Center = unity both sides.
pub fn balance_gains(p: f32) -> (f32, f32) {
    let p = p.clamp(-1.0, 1.0);
    ((p.max(0.0) * FRAC_PI_2).cos(), ((-p).max(0.0) * FRAC_PI_2).cos())
}

/// Stereo balance / pan. "Balance" attenuates one side of a stereo source; "Pan" sums to mono
/// and positions it with the constant-power law (normalized so the center is unity for a
/// correlated source).
pub struct Balance {
    pv: ParamValues,
    gl: Smoothed,
    gr: Smoothed,
    pan_mode: bool,
    mode_mix: Smoothed,
}

impl Balance {
    pub const PARAMS: &'static [ParamSpec] = &[
        ParamSpec::new("balance", "Balance", -100.0, 100.0, 0.0, Unit::Pan),
        ParamSpec::choice("mode", "Mode", &["Balance", "Pan (constant power)"], 0),
    ];

    pub fn new(sample_rate: f32, _channels: usize) -> Self {
        let sample_rate = crate::sane_sample_rate(sample_rate);
        let mut s = Balance {
            pv: ParamValues::new(Self::PARAMS),
            gl: Smoothed::with_ms(1.0, sample_rate, RAMP_MS),
            gr: Smoothed::with_ms(1.0, sample_rate, RAMP_MS),
            pan_mode: false,
            mode_mix: Smoothed::with_ms(0.0, sample_rate, RAMP_MS),
        };
        s.apply_params(true);
        s
    }
    fn apply_params(&mut self, snap: bool) {
        let p = self.pv.v("balance") / 100.0;
        self.pan_mode = self.pv.idx("mode") == 1;
        let (l, r) = if self.pan_mode {
            let (l, r) = constant_power_pan(p);
            (l * std::f32::consts::SQRT_2, r * std::f32::consts::SQRT_2)
        } else {
            balance_gains(p)
        };
        self.gl.set(l);
        self.gr.set(r);
        self.mode_mix.set(if self.pan_mode { 1.0 } else { 0.0 });
        if snap {
            self.gl.snap();
            self.gr.snap();
            self.mode_mix.snap();
        }
    }
}

impl AudioEffect for Balance {
    param_plumbing!("balance");
    fn reset(&mut self) {
        self.gl.snap();
        self.gr.snap();
        self.mode_mix.snap();
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        if channels.len() < 2 {
            return;
        }
        let (a, b) = channels.split_at_mut(1);
        let (l, r) = (&mut a[0], &mut b[0]);
        for i in 0..n {
            let gl = self.gl.tick();
            let gr = self.gr.tick();
            let mm = self.mode_mix.tick();
            let mono = 0.5 * (l[i] + r[i]);
            let il = l[i] + mm * (mono - l[i]);
            let ir = r[i] + mm * (mono - r[i]);
            l[i] = il * gl;
            r[i] = ir * gr;
        }
    }
}

/// Polarity inversion.
pub struct Invert {
    pv: ParamValues,
}

impl Invert {
    pub const PARAMS: &'static [ParamSpec] = &[ParamSpec::choice("channels", "Channels", &["All", "Left", "Right"], 0)];
    pub fn new(_sample_rate: f32, _channels: usize) -> Self {
        Invert { pv: ParamValues::new(Self::PARAMS) }
    }
    fn apply_params(&mut self, _snap: bool) {}
}

impl AudioEffect for Invert {
    param_plumbing!("invert");
    fn reset(&mut self) {}
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        let which = self.pv.idx("channels");
        for (c, ch) in channels.iter_mut().enumerate() {
            if which == 0 || which == c + 1 {
                ch[..n].iter_mut().for_each(|v| *v = -*v);
            }
        }
    }
}

/// Parameterless channel routing utilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Routing {
    /// Swap left and right.
    Swap,
    /// Fill left with right.
    FillLeft,
    /// Fill right with left.
    FillRight,
}

/// Swap / Fill Left with Right / Fill Right with Left.
pub struct Route {
    pv: ParamValues,
    kind: Routing,
}

impl Route {
    pub const PARAMS: &'static [ParamSpec] = &[];
    pub fn new(kind: Routing) -> Self {
        Route { pv: ParamValues::new(Self::PARAMS), kind }
    }
    fn apply_params(&mut self, _snap: bool) {}
}

impl AudioEffect for Route {
    fn id(&self) -> &'static str {
        match self.kind {
            Routing::Swap => "swap_channels",
            Routing::FillLeft => "fill_left",
            Routing::FillRight => "fill_right",
        }
    }
    fn params(&self) -> &'static [ParamSpec] {
        Self::PARAMS
    }
    fn set_param(&mut self, id: &str, value: f32) -> bool {
        let ok = self.pv.set(id, value);
        self.apply_params(false);
        ok
    }
    fn param(&self, id: &str) -> Option<f32> {
        self.pv.get(id)
    }
    fn reset(&mut self) {}
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        if channels.len() < 2 {
            return;
        }
        let (a, b) = channels.split_at_mut(1);
        let (l, r) = (&mut a[0][..n], &mut b[0][..n]);
        match self.kind {
            Routing::Swap => l.swap_with_slice(r),
            Routing::FillLeft => l.copy_from_slice(r),
            Routing::FillRight => r.copy_from_slice(l),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amplify_reaches_exact_gain() {
        let mut a = Amplify::new(48000.0, 2);
        a.set_param("gain", -6.0);
        let mut l = vec![1.0f32; 2000];
        let mut r = vec![0.5f32; 2000];
        a.process(&mut [&mut l, &mut r]);
        assert!((l[1999] - db_to_gain(-6.0)).abs() < 1e-6);
        assert!((r[1999] - 0.5 * db_to_gain(-6.0)).abs() < 1e-6);
        // Smoothed, not stepped.
        assert!(l[0] > 0.99);
        a.set_param("gain", -96.0);
        a.process(&mut [&mut l, &mut r]);
        assert_eq!(l[1999], 0.0);
    }

    #[test]
    fn channel_volume_per_channel() {
        let mut c = ChannelVolume::new(48000.0, 2);
        c.set_param("right", -20.0);
        c.reset();
        let mut l = vec![1.0f32; 10];
        let mut r = vec![1.0f32; 10];
        c.process(&mut [&mut l, &mut r]);
        assert_eq!(l[5], 1.0);
        assert!((r[5] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn pan_is_constant_power() {
        for i in -10..=10 {
            let (l, r) = constant_power_pan(i as f32 / 10.0);
            assert!((l * l + r * r - 1.0).abs() < 1e-6);
        }
        assert_eq!(balance_gains(0.0), (1.0, 1.0));
        let (l, r) = balance_gains(1.0);
        assert!(l.abs() < 1e-6 && r == 1.0);
        let mut b = Balance::new(48000.0, 2);
        b.set_param("mode", 1.0);
        b.set_param("balance", 100.0);
        b.reset();
        let mut l = vec![0.5f32; 4];
        let mut r = vec![0.5f32; 4];
        b.process(&mut [&mut l, &mut r]);
        assert!(l[3].abs() < 1e-6);
        assert!((r[3] - 0.5 * std::f32::consts::SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn invert_swap_fill() {
        let mut l = vec![1.0f32, 2.0];
        let mut r = vec![3.0f32, 4.0];
        Route::new(Routing::Swap).process(&mut [&mut l, &mut r]);
        assert_eq!((l.clone(), r.clone()), (vec![3.0, 4.0], vec![1.0, 2.0]));
        Route::new(Routing::FillLeft).process(&mut [&mut l, &mut r]);
        assert_eq!(l, vec![1.0, 2.0]);
        l[0] = 9.0;
        Route::new(Routing::FillRight).process(&mut [&mut l, &mut r]);
        assert_eq!(r, vec![9.0, 2.0]);
        let mut inv = Invert::new(48000.0, 2);
        inv.set_param("channels", 2.0);
        inv.process(&mut [&mut l, &mut r]);
        assert_eq!((l, r), (vec![9.0, 2.0], vec![-9.0, -2.0]));
    }
}
