//! Real-time audio DSP for the Craft apps: dependency-free and `wasm32` clean.
//!
//! * [`loudness`]: ITU-R BS.1770-4 / EBU R128 loudness meter (momentary, short-term,
//!   integrated, loudness range, sample peak, true peak) and the loudness-normalization helper.
//! * [`effects`]: audio effects behind the common [`AudioEffect`] trait, with a registry
//!   ([`effects()`]) describing every effect's parameters (range, unit, default) so a host and
//!   its interface can build controls generically.
//! * [`biquad`]: RBJ-cookbook biquads (TDF-II) with analytic magnitude response.
//! * [`sync`]: offset between two recordings of one event (GCC-PHAT, sample-accurate).
//! * [`resample`]: streaming, block-invariant rate conversion for playback on a device at another
//!   rate. [`offline`] converts whole buffers with a polyphase windowed-sinc filter.
//! * [`stretch`]: WSOLA time stretch that keeps the pitch.
//! * [`channels`]: channel layouts, ITU-R BS.775 up/downmix, 5.1 mixdown types, the 5.1 panner.
//! * [`pan`]: stereo pan laws, a surround panner over any speaker layout, and first-order
//!   ambisonic encoding and decoding.
//! * [`generator`]: band-limited test tones and white and pink noise.
//!
//! Conventions: audio is **planar f32** (`&mut [&mut [f32]]`, one slice per channel, all the same
//! length). `process` never allocates; all buffers are sized at construction. Parameters may be
//! changed between blocks. Some are smoothed per sample when they change (gains, mix amounts and
//! equalizer bands among them); others, such as a compressor's threshold, ratio, knee and times or
//! a limiter's ceiling and release, take effect from the next sample. With the parameters held
//! still, the output does not depend on how the stream is cut into blocks. Recursive state is flushed to zero when it decays below ~1e-30 so
//! denormals never appear (no FTZ/DAZ CPU flags needed, which keeps the crate `unsafe`-free and
//! portable to wasm).
//!
//! **Stability.** The items exported here are the crate's API. A change that removes or changes
//! one is a breaking change and bumps the crate's major version (its minor version while it is
//! below 1.0). Effect ids and parameter ids are part of the API too: hosts save them in their
//! files.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
// Per-sample loops index several channel slices with one frame index.
#![allow(clippy::needless_range_loop)]

pub mod biquad;
pub mod channels;
pub mod design;
pub mod ducking;
pub mod effects;
pub(crate) mod fft;
pub mod generator;
pub mod loudness;
pub mod offline;
pub(crate) mod oversample;
pub mod pan;
pub mod resample;
mod smooth;
pub mod stretch;
pub mod sync;

pub use effects::{Category, EffectInfo, create_effect, effect_info, effects};
pub use loudness::{LoudnessMeter, normalize_gain_db};
pub(crate) use smooth::Smoothed;

/// Lowest sample rate the crate builds state for; a lower, zero or non-finite rate becomes
/// [`FALLBACK_SAMPLE_RATE`].
pub const MIN_SAMPLE_RATE: f32 = 1_000.0;
/// Highest sample rate the crate builds state for; a higher rate is clamped to it.
pub const MAX_SAMPLE_RATE: f32 = 1_536_000.0;
/// The rate used in place of an unusable one.
pub const FALLBACK_SAMPLE_RATE: f32 = 48_000.0;
/// Most channels an effect or meter builds state for. Channels past it pass through untouched.
pub const MAX_CHANNELS: usize = 64;

/// A sample rate the crate can build filters for: `rate` within [`MIN_SAMPLE_RATE`] ..=
/// [`MAX_SAMPLE_RATE`], or [`FALLBACK_SAMPLE_RATE`] when it is below the range, zero or not finite.
pub fn sane_sample_rate(rate: f32) -> f32 {
    if rate.is_finite() && rate >= MIN_SAMPLE_RATE { rate.min(MAX_SAMPLE_RATE) } else { FALLBACK_SAMPLE_RATE }
}

/// A channel count the crate builds state for: at least 1, at most [`MAX_CHANNELS`].
pub fn sane_channels(channels: usize) -> usize {
    channels.clamp(1, MAX_CHANNELS)
}

/// Unit of a parameter value (for display and control construction).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    /// Unitless scalar.
    None,
    Decibels,
    Hertz,
    Milliseconds,
    Seconds,
    Percent,
    /// Filter quality factor.
    Q,
    /// Compression ratio (`x:1`).
    Ratio,
    Semitones,
    Bpm,
    /// Pan / balance position, −100 (left) … +100 (right).
    Pan,
    /// Boolean toggle: 0 = off, 1 = on.
    Toggle,
    /// Index into [`ParamSpec::choices`].
    Choice,
}

/// Static description of one effect parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSpec {
    /// Stable id used by `set_param` / automation (e.g. `"threshold"`, `"b3.freq"`).
    pub id: &'static str,
    /// Human-readable label.
    pub name: &'static str,
    /// Lowest value [`ParamSpec::sanitize`] keeps.
    pub min: f32,
    /// Highest value [`ParamSpec::sanitize`] keeps.
    pub max: f32,
    /// Value a new effect starts with, and the value NaN is read as.
    pub default: f32,
    /// Unit the value is in.
    pub unit: Unit,
    /// Hint that the control should be logarithmic (frequencies, times).
    pub log_scale: bool,
    /// Option labels when `unit == Unit::Choice` (value is the index).
    pub choices: &'static [&'static str],
}

impl ParamSpec {
    /// A continuous (linear) parameter.
    pub const fn new(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, unit: Unit) -> Self {
        ParamSpec { id, name, min, max, default, unit, log_scale: false, choices: &[] }
    }
    /// A continuous parameter best shown on a logarithmic control.
    pub const fn log(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, unit: Unit) -> Self {
        ParamSpec { id, name, min, max, default, unit, log_scale: true, choices: &[] }
    }
    /// An on/off toggle.
    pub const fn toggle(id: &'static str, name: &'static str, default: bool) -> Self {
        ParamSpec { id, name, min: 0.0, max: 1.0, default: if default { 1.0 } else { 0.0 }, unit: Unit::Toggle, log_scale: false, choices: &[] }
    }
    /// A discrete choice; the value is the index into `choices`. A default past the last choice
    /// is read as 0, and an empty list has the single value 0.
    pub const fn choice(id: &'static str, name: &'static str, choices: &'static [&'static str], default: usize) -> Self {
        let default = if default < choices.len() { default } else { 0 };
        let max = choices.len().saturating_sub(1) as f32;
        ParamSpec { id, name, min: 0.0, max, default: default as f32, unit: Unit::Choice, log_scale: false, choices }
    }
    /// Clamp (and for choices/toggles, round) a value into this parameter's domain.
    /// NaN maps to the default. When `min` is above `max`, the value is `max`.
    pub fn sanitize(&self, v: f32) -> f32 {
        if v.is_nan() {
            return self.default;
        }
        let v = v.max(self.min).min(self.max);
        match self.unit {
            Unit::Choice | Unit::Toggle => v.round(),
            _ => v,
        }
    }
}

/// A real-time audio effect operating in place on planar channels.
///
/// Implementations are constructed (allocating) with a sample rate and channel count; after that
/// `process`, `process_keyed`, `set_param` and `reset` never allocate. If `process` receives more
/// channels than the effect was built for, the extra channels are passed through untouched
/// (channel-generic effects such as gain process all of them).
///
/// The methods after [`AudioEffect::transfer_db`] have defaults, so an effect implements only
/// what it supports. [`AudioEffect::events`], [`AudioEffect::editor`] and the state methods are
/// for instruments and hosted third-party plugins; the built-in effects return `None` and
/// `false`. The editor and state-reader handles are used on the interface thread while the
/// effect keeps processing on the audio thread, and may allocate.
pub trait AudioEffect: Send {
    /// Registry id (e.g. `"compressor"`).
    fn id(&self) -> &'static str;
    /// Parameter descriptions (same as the registry entry's).
    fn params(&self) -> &'static [ParamSpec];
    /// Set a parameter by id; the value is clamped to its range. Returns `false` for unknown ids.
    /// Takes effect from the next processed sample. Gains, mix amounts and equalizer bands move to
    /// the new value smoothly; the crate documentation lists which other parameters change at once.
    fn set_param(&mut self, id: &str, value: f32) -> bool;
    /// Current (target) value of a parameter.
    fn param(&self, id: &str) -> Option<f32>;
    /// Clear all internal state (delay lines, envelopes, filter memories); smoothed parameters
    /// jump to their targets.
    fn reset(&mut self);
    /// Process one block in place. All channel slices must have the same length (the shortest
    /// length is used otherwise). Zero-length blocks are allowed.
    fn process(&mut self, channels: &mut [&mut [f32]]);
    /// Processing latency in samples (look-ahead / STFT delay) for delay compensation.
    fn latency(&self) -> usize {
        0
    }
    /// Analytic magnitude response (dB) at `freq` Hz for the current target settings, for
    /// filter/EQ effects (drives the graphical EQ editors). `None` when not applicable.
    fn response_db(&self, _freq: f64) -> Option<f64> {
        None
    }
    /// Static input → output level curve (dB) of a dynamics effect for `band` (0 for
    /// single-band processors), at the current settings. `None` when not applicable.
    fn transfer_db(&self, _band: usize, _input_db: f32) -> Option<f32> {
        None
    }
    /// How long the output keeps sounding after the input stops, in samples (reverb and delay
    /// tails), so an offline render can run past the end of its input.
    fn tail_samples(&self) -> usize {
        0
    }
    /// Gain reduction applied to the last processed block, as a non-negative amount in dB, for
    /// metering. 0 for effects that do not reduce gain.
    fn gain_reduction_db(&self) -> f32 {
        0.0
    }
    /// Whether the effect renders offline (an export, faster or slower than real time) rather
    /// than live. Effects start live.
    fn set_offline(&mut self, _offline: bool) {}
    /// Channels of key (sidechain) input the effect reads in [`AudioEffect::process_keyed`]; 0
    /// when it has no key input.
    fn key_channels(&self) -> usize {
        0
    }
    /// [`AudioEffect::process`] with a key input: read-only planar channels that drive the
    /// effect's detector instead of its own input (a voice pushing music down, a kick drum
    /// opening a gate). A key whose channels are shorter than the block, or an empty key, means
    /// the effect detects on its own input. Effects without a key input ignore it.
    fn process_keyed(&mut self, channels: &mut [&mut [f32]], key: &[&[f32]]) {
        let _ = key;
        self.process(channels);
    }
    /// The effect's note and transport input (instruments, tempo-synced effects), or `None`.
    fn events(&mut self) -> Option<&mut dyn EventInput> {
        None
    }
    /// The effect's complete internal state as an opaque blob, as its
    /// [`AudioEffect::state_reader`] reads it. `None` when it has no state beyond its parameters.
    /// May allocate: never call it while processing.
    fn save_state(&mut self) -> Option<Vec<u8>> {
        self.state_reader()?.save_state()
    }
    /// Restores a blob from [`AudioEffect::save_state`]. `false` when unsupported or rejected
    /// (the effect keeps its previous state). The data is hostile input.
    fn load_state(&mut self, _data: &[u8]) -> bool {
        false
    }
    /// A handle that reads this effect's state from the interface thread while the effect
    /// processes on the audio thread, or `None`.
    fn state_reader(&mut self) -> Option<Box<dyn StateReader>> {
        None
    }
    /// A handle to the effect's own editor window, or `None`. The handle stays valid after the
    /// effect is dropped; its calls then do nothing.
    fn editor(&mut self) -> Option<Box<dyn Editor>> {
        None
    }
}

/// The host's transport at the start of a block, for tempo-synced effects and instruments.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transport {
    /// Whether the timeline is playing.
    pub playing: bool,
    /// Whether the host is recording.
    pub recording: bool,
    /// Quarter notes per minute.
    pub tempo: f64,
    /// Beats per bar, the upper number of the time signature.
    pub numerator: u32,
    /// Note value of one beat, the lower number of the time signature (4 for a quarter note).
    pub denominator: u32,
    /// Timeline position of the block's first frame, in samples.
    pub samples: i64,
    /// The same position in seconds.
    pub seconds: f64,
    /// The same position in quarter notes.
    pub quarters: f64,
    /// Start of the current bar, in quarter notes, and its number (0 for the first bar).
    pub bar_start: f64,
    pub bar: i64,
}

/// Note and transport input of an instrument or tempo-synced effect. Every call refers to the
/// next processed block; `offset` is a frame within it. Calls never allocate.
///
/// Callers keep `offset` below the frame count of the next block. The trait does not define what
/// an implementation does with a larger offset.
pub trait EventInput {
    /// The transport for the next block.
    fn set_transport(&mut self, transport: &Transport);
    /// Starts `note`, a MIDI 1.0 key number from 0 to 127 (60 is middle C), at `velocity` from 1
    /// to 127. Callers send a MIDI note-on with velocity 0 as [`EventInput::note_off`].
    fn note_on(&mut self, offset: usize, note: u8, velocity: u8);
    /// Releases `note`, a MIDI 1.0 key number from 0 to 127.
    fn note_off(&mut self, offset: usize, note: u8);
    /// Any other MIDI 1.0 channel message (control change, pitch bend, pressure, program change).
    fn midi(&mut self, offset: usize, message: [u8; 3]);
    /// Releases every sounding note at the start of the next block.
    fn all_notes_off(&mut self);
}

/// An effect's own editor window, driven from the interface thread while the effect may be
/// processing on the audio thread.
pub trait Editor: Send {
    /// Opens (or raises) the editor window. The error says why it cannot.
    fn open(&mut self) -> Result<(), String>;
    /// Closes the editor window.
    fn close(&mut self);
    fn is_open(&self) -> bool;
    /// Call every interface frame while open: services the editor, notices a window the user
    /// closed, and returns the parameter edits made in it as `(parameter id, value)`.
    fn idle(&mut self) -> Vec<(String, f32)>;
    /// `true` once after the effect's state changed in a way its parameters do not show, so the
    /// host saves the new state.
    fn state_changed(&mut self) -> bool {
        false
    }
    /// Tells the editor that a parameter changed elsewhere, so it shows the new value.
    fn set_param(&mut self, _id: &str, _value: f32) {}
    /// The effect's own text for a parameter value, or `None` when the host formats it.
    fn value_text(&mut self, _id: &str, _value: f32) -> Option<String> {
        None
    }
}

/// Reads an effect's state from the interface thread. Closures returning the blob are readers.
pub trait StateReader: Send {
    /// What [`AudioEffect::save_state`] would return; `None` once the effect is gone.
    fn save_state(&mut self) -> Option<Vec<u8>>;
}

impl<F: FnMut() -> Option<Vec<u8>> + Send> StateReader for F {
    fn save_state(&mut self) -> Option<Vec<u8>> {
        self()
    }
}

/// The key channels to detect on: `key` when it has channels and each covers the block's
/// `frames`, else `None` (the effect detects on its own input).
#[inline]
pub(crate) fn usable_key<'a>(key: &'a [&'a [f32]], frames: usize) -> Option<&'a [&'a [f32]]> {
    (!key.is_empty() && key.iter().all(|k| k.len() >= frames)).then_some(key)
}

/// Parameter value storage shared by the effect implementations.
#[derive(Clone, Debug)]
pub(crate) struct ParamValues {
    specs: &'static [ParamSpec],
    values: Vec<f32>,
}

impl ParamValues {
    pub(crate) fn new(specs: &'static [ParamSpec]) -> Self {
        ParamValues { specs, values: specs.iter().map(|s| s.default).collect() }
    }
    pub(crate) fn index_of(&self, id: &str) -> Option<usize> {
        self.specs.iter().position(|s| s.id == id)
    }
    /// Returns `true` if the id exists.
    pub(crate) fn set(&mut self, id: &str, v: f32) -> bool {
        match self.index_of(id) {
            Some(i) => {
                self.values[i] = self.specs[i].sanitize(v);
                true
            }
            None => false,
        }
    }
    pub(crate) fn get(&self, id: &str) -> Option<f32> {
        self.index_of(id).map(|i| self.values[i])
    }
    /// Value of a parameter that is known to exist (0 otherwise; a missing id is a
    /// programming error that debug builds catch).
    pub(crate) fn v(&self, id: &str) -> f32 {
        debug_assert!(self.get(id).is_some(), "unknown parameter {id}");
        self.get(id).unwrap_or(0.0)
    }
    pub(crate) fn on(&self, id: &str) -> bool {
        self.v(id) >= 0.5
    }
    pub(crate) fn idx(&self, id: &str) -> usize {
        self.v(id).max(0.0) as usize
    }
}

/// Generates the id/params/set_param/param methods of [`AudioEffect`] for a struct with a
/// `pv: ParamValues` field, a `PARAMS` const and an `apply_params(&mut self, snap: bool)` method.
macro_rules! param_plumbing {
    ($id:expr) => {
        fn id(&self) -> &'static str {
            $id
        }
        fn params(&self) -> &'static [$crate::ParamSpec] {
            Self::PARAMS
        }
        fn set_param(&mut self, id: &str, value: f32) -> bool {
            if self.pv.set(id, value) {
                self.apply_params(false);
                true
            } else {
                false
            }
        }
        fn param(&self, id: &str) -> Option<f32> {
            self.pv.get(id)
        }
    };
}
pub(crate) use param_plumbing;

/// Decibels → linear amplitude.
#[inline]
pub(crate) fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Linear amplitude → decibels (−inf for 0).
#[cfg(test)]
pub(crate) fn gain_to_db(g: f32) -> f32 {
    20.0 * g.abs().log10()
}

/// Flush tiny values to zero (denormal protection for recursive state). Non-finite values are
/// zeroed too, so one NaN or infinite input sample cannot poison the state until `reset`.
#[inline(always)]
pub(crate) fn flush(x: f64) -> f64 {
    if (1e-30..=f64::MAX).contains(&x.abs()) { x } else { 0.0 }
}

/// f32 variant of [`flush`] for f32 delay lines.
#[inline(always)]
pub(crate) fn flush32(x: f32) -> f32 {
    if (1e-25..=f32::MAX).contains(&x.abs()) { x } else { 0.0 }
}

/// Common block length of planar channels.
#[inline]
pub(crate) fn block_len(channels: &[&mut [f32]]) -> usize {
    channels.iter().map(|c| c.len()).min().unwrap_or(0)
}

#[cfg(test)]
pub(crate) mod testutil {
    /// Tiny deterministic PRNG (xorshift64*) for tests.
    pub struct Rng(u64);
    impl Rng {
        pub fn new(seed: u64) -> Self {
            Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
        }
        pub fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        /// Uniform in [-1, 1).
        pub fn uniform(&mut self) -> f32 {
            ((self.next_u64() >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        }
    }

    pub fn sine(freq: f64, amp: f64, sr: f64, n: usize, phase: f64) -> Vec<f32> {
        (0..n).map(|i| (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / sr + phase).sin()) as f32).collect()
    }

    pub fn rms(x: &[f32]) -> f64 {
        if x.is_empty() {
            return 0.0;
        }
        (x.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / x.len() as f64).sqrt()
    }

    pub fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    /// Amplitude of the `freq` component of `x` (single-bin DFT / Goertzel-style correlation).
    pub fn tone_amplitude(x: &[f32], freq: f64, sr: f64) -> f64 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            let w = 2.0 * std::f64::consts::PI * freq * i as f64 / sr;
            re += v as f64 * w.cos();
            im += v as f64 * w.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f64
    }
}
