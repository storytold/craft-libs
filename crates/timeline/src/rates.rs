//! Frame rates, sample rates and pixel aspect ratios, with the presets the settings dialogs and
//! menus offer (Premiere's sequence settings, After Effects' composition settings, Pro Tools'
//! session rates).

use crate::time::{TICKS_PER_SECOND, Tick};

/// A frame rate as an exact rational, `num / den` frames a second. Rates come from files, so
/// every frame computation goes through [`sane`](Self::sane) and a zero or negative rate counts
/// as the default (23.976, as FilmCraft).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct FrameRate {
    /// The numerator: frames in `den` seconds.
    pub num: i64,
    /// The denominator: seconds that `num` frames take.
    pub den: i64,
}

impl Default for FrameRate {
    fn default() -> Self {
        Self::FPS_23_976
    }
}

impl FrameRate {
    /// 23.976 frames a second (24000/1001).
    pub const FPS_23_976: FrameRate = FrameRate::new(24000, 1001);
    #[allow(missing_docs)]
    pub const FPS_24: FrameRate = FrameRate::new(24, 1);
    #[allow(missing_docs)]
    pub const FPS_25: FrameRate = FrameRate::new(25, 1);
    /// 29.97 frames a second (30000/1001).
    pub const FPS_29_97: FrameRate = FrameRate::new(30000, 1001);
    #[allow(missing_docs)]
    pub const FPS_30: FrameRate = FrameRate::new(30, 1);
    #[allow(missing_docs)]
    pub const FPS_48: FrameRate = FrameRate::new(48, 1);
    #[allow(missing_docs)]
    pub const FPS_50: FrameRate = FrameRate::new(50, 1);
    /// 59.94 frames a second (60000/1001).
    pub const FPS_59_94: FrameRate = FrameRate::new(60000, 1001);
    #[allow(missing_docs)]
    pub const FPS_60: FrameRate = FrameRate::new(60, 1);
    /// 119.88 frames a second (120000/1001).
    pub const FPS_119_88: FrameRate = FrameRate::new(120000, 1001);
    #[allow(missing_docs)]
    pub const FPS_120: FrameRate = FrameRate::new(120, 1);

    /// The rates a frame-rate menu offers, in Premiere's order.
    pub const COMMON: [FrameRate; 11] = [
        Self::FPS_23_976,
        Self::FPS_24,
        Self::FPS_25,
        Self::FPS_29_97,
        Self::FPS_30,
        Self::FPS_48,
        Self::FPS_50,
        Self::FPS_59_94,
        Self::FPS_60,
        Self::FPS_119_88,
        Self::FPS_120,
    ];

    /// `num / den` reduced by their greatest common divisor.
    // filmcraft: crates/time/src/lib.rs:198 (i128 division by the gcd, at least 1).
    pub const fn new(num: i64, den: i64) -> Self {
        let mut g = gcd(num.unsigned_abs(), den.unsigned_abs()) as i128;
        if g < 1 {
            g = 1;
        }
        FrameRate { num: (num as i128 / g) as i64, den: (den as i128 / g) as i64 }
    }

    /// The [`COMMON`](Self::COMMON) rate within 0.005 fps of `fps` (a container's average rate),
    /// else `round(fps × 1000) / 1000`.
    // filmcraft: crates/time/src/lib.rs:204.
    pub fn from_f64(fps: f64) -> Self {
        if let Some(r) = Self::COMMON.into_iter().find(|r| (r.as_f64() - fps).abs() < 0.005) {
            return r;
        }
        FrameRate::new((fps * 1000.0).round() as i64, 1000)
    }

    /// This rate when both parts are positive, else the default.
    pub fn sane(self) -> FrameRate {
        if self.num > 0 && self.den > 0 { self } else { FrameRate::default() }
    }

    /// The rate in frames a second.
    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// One frame's exact duration (floored only for exotic rates); saturates at the tick range.
    // filmcraft: crates/time/src/lib.rs:225 (`TICKS_PER_SECOND * den / num` in i128).
    pub fn frame_duration(self) -> Tick {
        let r = self.sane();
        Tick(clamp_i64(i128::from(TICKS_PER_SECOND) * i128::from(r.den) / i128::from(r.num)))
    }

    /// The index of the frame holding `t` (floor), saturated to the frame range.
    // filmcraft: crates/time/src/lib.rs:231 (`frame_index`: `t * num / (TICKS_PER_SECOND * den)`
    // with `div_euclid` in i128).
    pub fn frame_at(self, t: Tick) -> i64 {
        clamp_i64(frame_index(self.sane(), t))
    }

    /// Frames from `start` to `end`, counted between their frame indices. `frame_at(end - start)`
    /// can read one short when a frame is not a whole number of ticks: the ticks between two
    /// boundaries can be one fewer than the exact length of those frames.
    pub fn frames_between(self, start: Tick, end: Tick) -> i64 {
        self.frame_at(end).saturating_sub(self.frame_at(start))
    }

    /// The first tick of frame `f`, by ceiling division, so `frame_at(tick_of(f)) == f` holds
    /// for every rate with at least one tick per frame (37.516 fps included: flooring drifts each
    /// boundary back a tick and frame stepping stalls).
    // filmcraft: crates/time/src/lib.rs:244 (`frame_boundary`).
    pub fn tick_of(self, f: i64) -> Tick {
        Tick(clamp_i64(frame_boundary(self.sane(), i128::from(f))))
    }

    /// `t` moved down to its frame's first tick.
    // filmcraft: crates/time/src/lib.rs:249.
    pub fn snap(self, t: Tick) -> Tick {
        let r = self.sane();
        Tick(clamp_i64(frame_boundary(r, frame_index(r, t))))
    }

    /// `t` moved to the nearest frame boundary (the earlier one at the exact middle); both
    /// boundaries are compared in i128 before saturating.
    // filmcraft: crates/time/src/lib.rs:255.
    pub fn snap_nearest(self, t: Tick) -> Tick {
        let r = self.sane();
        // Both boundaries stay wide until compared: saturating an unrepresentable upper one first
        // would make it look closer than it is.
        let f = frame_index(r, t);
        let a = frame_boundary(r, f);
        let b = frame_boundary(r, f.saturating_add(1));
        let t = i128::from(t.0);
        Tick(clamp_i64(if t - a <= b - t { a } else { b }))
    }

    /// Frames counted per timecode second: 30 for 29.97, 24 for 23.976 (`num / den` rounded up,
    /// at least 1).
    // filmcraft: crates/time/src/lib.rs:267.
    pub fn timecode_base(self) -> i64 {
        let r = self.sane();
        (r.num / r.den).saturating_add(i64::from(r.num % r.den != 0)).max(1)
    }

    /// An NTSC (x/1001) rate.
    pub fn is_ntsc(self) -> bool {
        self.den == 1001
    }

    /// Whether drop-frame counting applies: NTSC rates with a base that is a multiple of 30
    /// (29.97, 59.94, 119.88).
    // filmcraft: crates/time/src/lib.rs:278.
    pub fn supports_drop_frame(self) -> bool {
        self.is_ntsc() && self.timecode_base() % 30 == 0
    }

    /// The menu label without the unit: `24`, `23.976`, `29.97` (up to three decimals, in the
    /// locale's digits and decimal mark). The unit ("fps") is the menu's word.
    // filmcraft: crates/time/src/lib.rs:282.
    #[cfg(feature = "ui")]
    pub fn label(self, numbers: &craft_ui::runtime::env::Units) -> String {
        if self.den == 1 {
            return numbers.number(self.num as f64, 0..=0);
        }
        numbers.number(self.as_f64(), 0..=3)
    }
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

/// Narrows after applying the storage bounds, never by a wrapping cast.
fn clamp_i64(value: i128) -> i64 {
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// The frame holding `t`, for a positive rate.
fn frame_index(rate: FrameRate, t: Tick) -> i128 {
    (i128::from(t.0) * i128::from(rate.num)).div_euclid(i128::from(TICKS_PER_SECOND) * i128::from(rate.den))
}

/// The ceiling of a frame's rational start. An overflowing numerator over an `i64` rate cannot
/// fit a tick, so saturating it keeps the public result.
fn frame_boundary(rate: FrameRate, frame: i128) -> i128 {
    let Some(n) = frame.checked_mul(i128::from(TICKS_PER_SECOND)).and_then(|n| n.checked_mul(i128::from(rate.den))) else {
        return if frame < 0 { i128::MIN } else { i128::MAX };
    };
    let d = i128::from(rate.num);
    n.div_euclid(d) + i128::from(n.rem_euclid(d) != 0)
}

const fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
