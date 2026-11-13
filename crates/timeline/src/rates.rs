//! Frame rates, sample rates and pixel aspect ratios, with the presets the settings dialogs and
//! menus offer (Premiere's sequence settings, After Effects' composition settings, Pro Tools'
//! session rates).

use crate::time::Tick;

pub use craft_time::FrameRate;

/// A frame rate's menu label without the unit: `24`, `23.976`, `29.97` (up to three decimals, in
/// the locale's digits and decimal mark). The unit ("fps") is the menu's word;
/// [`FrameRate::label`] is the ASCII form documents write.
// filmcraft: crates/time/src/lib.rs:282.
#[cfg(feature = "ui")]
pub fn frame_rate_label(rate: FrameRate, numbers: &craft_ui::runtime::env::Units) -> String {
    if rate.den == 1 {
        return numbers.number(rate.num as f64, 0..=0);
    }
    numbers.number(rate.as_f64(), 0..=3)
}

/// An audio sample rate in hertz.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SampleRate(u32);

impl Default for SampleRate {
    fn default() -> Self {
        Self::HZ_48000
    }
}

impl SampleRate {
    /// A rate of `hz` samples a second.
    pub const fn new(hz: u32) -> Self {
        SampleRate(hz)
    }

    #[allow(missing_docs)]
    pub const HZ_44100: SampleRate = SampleRate(44_100);
    #[allow(missing_docs)]
    pub const HZ_48000: SampleRate = SampleRate(48_000);

    /// The rates a session's sample-rate menu offers (SoundCraft `crates/time/src/lib.rs:46`).
    /// A video app offers [`VIDEO`](Self::VIDEO).
    pub const COMMON: [SampleRate; 8] = [
        SampleRate(44_100),
        SampleRate(48_000),
        SampleRate(88_200),
        SampleRate(96_000),
        SampleRate(176_400),
        SampleRate(192_000),
        SampleRate(352_800),
        SampleRate(384_000),
    ];

    /// The sequence sample rates FilmCraft offers (`panels/sequence_settings.rs:59`).
    pub const VIDEO: [SampleRate; 5] = [SampleRate(32_000), SampleRate(44_100), SampleRate(48_000), SampleRate(88_200), SampleRate(96_000)];

    /// The rate in samples a second.
    pub fn hz(self) -> u32 {
        self.0
    }

    /// The time of sample `n` (exact for every rate in [`COMMON`](Self::COMMON)).
    pub fn tick_of(self, n: i64) -> Tick {
        Tick::from_units(n, i64::from(self.0))
    }

    /// The sample holding `t` (floor).
    pub fn sample_at(self, t: Tick) -> i64 {
        t.to_units_floor(i64::from(self.0))
    }
}

/// A pixel aspect ratio, `num / den` (1/1 is square pixels).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct PixelAspect {
    /// The numerator of the width-to-height ratio of one pixel.
    pub num: u32,
    /// The denominator of the width-to-height ratio of one pixel.
    pub den: u32,
}

impl Default for PixelAspect {
    fn default() -> Self {
        Self::SQUARE
    }
}

impl PixelAspect {
    /// The ratio `num / den` of one pixel's width to its height.
    pub const fn new(num: u32, den: u32) -> Self {
        PixelAspect { num, den }
    }

    #[allow(missing_docs)]
    pub const SQUARE: PixelAspect = PixelAspect::new(1, 1);

    /// The ratio, 1.0 for a zero denominator.
    pub fn as_f64(self) -> f64 {
        if self.den == 0 { 1.0 } else { f64::from(self.num) / f64::from(self.den) }
    }

    /// The ratio as a menu shows it after its name, in the locale's digits and decimal mark:
    /// `1.0` for square pixels, else four decimals (`0.9091`). FilmCraft's labels are "Square
    /// Pixels (1.0)" and "Custom (0.9091)"; the words are the menu's.
    // filmcraft: crates/ui-egui/src/panels/sequence_settings.rs:204 `par_label`.
    #[cfg(feature = "ui")]
    pub fn ratio_label(self, numbers: &craft_ui::runtime::env::Units) -> String {
        if self.num == self.den || self.den == 0 {
            return numbers.number(1.0, 1..=1);
        }
        numbers.number(self.as_f64(), 4..=4)
    }
}
