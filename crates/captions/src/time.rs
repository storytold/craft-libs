//! Exact caption time.
//!
//! All time is an integer number of [`Tick`]s at [`TICKS_PER_SECOND`] = 254 016 000 000/s. That
//! rate divides evenly into a millisecond and into every broadcast frame duration (23.976, 24,
//! 25, 29.97, 30, 50, 59.94, 60 …), so SRT/WebVTT milliseconds and SCC/MCC/STL/TTML frames all
//! convert without rounding.
//!
//! Every conversion is total: a zero or negative rate (from a damaged file) counts as 23.976 fps
//! instead of dividing by zero, and results outside `i64` saturate.

use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Ticks per second.
pub const TICKS_PER_SECOND: i64 = 254_016_000_000;

/// A point or duration in time, in ticks.
/// Arithmetic saturates at the i64 storage limits; representable results remain exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(pub i64);

impl Tick {
    pub const ZERO: Tick = Tick(0);

    /// Seconds as a float, for display only: nothing in this crate computes with it.
    pub fn seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }
    pub fn min(self, o: Tick) -> Tick {
        Tick(self.0.min(o.0))
    }
    pub fn max(self, o: Tick) -> Tick {
        Tick(self.0.max(o.0))
    }
}

impl Add for Tick {
    type Output = Tick;
    fn add(self, o: Tick) -> Tick {
        Tick(self.0.saturating_add(o.0))
    }
}
impl Sub for Tick {
    type Output = Tick;
    fn sub(self, o: Tick) -> Tick {
        Tick(self.0.saturating_sub(o.0))
    }
}
impl Neg for Tick {
    type Output = Tick;
    fn neg(self) -> Tick {
        Tick(self.0.saturating_neg())
    }
}
impl AddAssign for Tick {
    fn add_assign(&mut self, o: Tick) {
        self.0 = self.0.saturating_add(o.0);
    }
}
impl SubAssign for Tick {
    fn sub_assign(&mut self, o: Tick) {
        self.0 = self.0.saturating_sub(o.0);
    }
}

/// A frame rate as an exact rational (`num / den` frames per second).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameRate {
    pub num: i64,
    pub den: i64,
}

impl Default for FrameRate {
    fn default() -> Self {
        FrameRate::FPS_23_976
    }
}

impl FrameRate {
    pub const FPS_23_976: FrameRate = FrameRate { num: 24000, den: 1001 };
    pub const FPS_24: FrameRate = FrameRate { num: 24, den: 1 };
    pub const FPS_25: FrameRate = FrameRate { num: 25, den: 1 };
    pub const FPS_29_97: FrameRate = FrameRate { num: 30000, den: 1001 };
    pub const FPS_30: FrameRate = FrameRate { num: 30, den: 1 };
    pub const FPS_50: FrameRate = FrameRate { num: 50, den: 1 };
    pub const FPS_59_94: FrameRate = FrameRate { num: 60000, den: 1001 };
    pub const FPS_60: FrameRate = FrameRate { num: 60, den: 1 };

    /// The rate in lowest terms.
    pub fn new(num: i64, den: i64) -> Self {
        let g = i128::from(gcd(num.unsigned_abs(), den.unsigned_abs()).max(1));
        FrameRate { num: (i128::from(num) / g) as i64, den: (i128::from(den) / g) as i64 }
    }

    /// This rate if it is positive, else the default (a damaged file can carry a zero
    /// or negative rate; every frame computation uses this so it can't divide by zero).
    pub fn sane(self) -> FrameRate {
        if self.num > 0 && self.den > 0 { self } else { FrameRate::default() }
    }

    /// Exact duration of one frame (rounded down only for exotic rates).
    /// Values outside the integer tick range saturate at its limit.
    pub fn frame_duration(self) -> Tick {
        let r = self.sane();
        Tick(clamp_i64((TICKS_PER_SECOND as i128 * r.den as i128) / r.num as i128))
    }

    /// Index of the frame containing `t` (floor), saturated to the integer frame range.
    pub fn frame_at(self, t: Tick) -> i64 {
        clamp_i64(frame_index(self.sane(), t))
    }

    /// Start tick of frame `f` (ceiling division).
    ///
    /// Uses ceiling (not floor) so that `frame_at(tick_of(f)) == f` holds for every
    /// representable frame boundary with at least one tick per frame, even when the frame duration is not an integer number of ticks
    /// (exotic rates like 37.516 fps where `TICKS_PER_SECOND * den / num` has a
    /// fractional remainder).  Floor division truncates the remainder, causing each
    /// frame boundary to drift backward by one tick until `frame_at` returns `f - 1`
    /// instead of `f` — frame stepping then stalls or moves backward.
    /// Values outside the integer tick range saturate at its limit.
    pub fn tick_of(self, f: i64) -> Tick {
        Tick(clamp_i64(frame_boundary(self.sane(), i128::from(f))))
    }

    /// Snap `t` down to a frame boundary.
    pub fn snap(self, t: Tick) -> Tick {
        let r = self.sane();
        Tick(clamp_i64(frame_boundary(r, frame_index(r, t))))
    }

    /// Snap `t` to the nearest frame boundary.
    pub fn snap_nearest(self, t: Tick) -> Tick {
        let r = self.sane();
        // Keep the frame index and both boundaries wide until choosing the nearest. Saturating
        // an unrepresentable upper boundary first would make it seem closer than it really is.
        let f = frame_index(r, t);
        let a = frame_boundary(r, f);
        let b = frame_boundary(r, f.saturating_add(1));
        let t = i128::from(t.0);
        Tick(clamp_i64(if t - a <= b - t { a } else { b }))
    }

    /// Timecode base (frames counted per timecode second): 30 for 29.97, 24 for 23.976.
    pub fn timecode_base(self) -> i64 {
        let r = self.sane();
        (r.num / r.den).saturating_add(i64::from(r.num % r.den != 0)).max(1)
    }

    /// NTSC (x/1001) rates can use drop-frame timecode.
    pub fn is_ntsc(self) -> bool {
        self.den == 1001
    }

    /// Whether drop-frame counting applies (29.97 / 59.94 / 119.88).
    pub fn supports_drop_frame(self) -> bool {
        self.is_ntsc() && self.timecode_base() % 30 == 0
    }
}

/// Narrow only after applying the storage bounds, never by a wrapping integer cast.
fn clamp_i64(value: i128) -> i64 {
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// Callers pass a positive rate. Products of two i64 values fit in i128.
fn frame_index(rate: FrameRate, t: Tick) -> i128 {
    (i128::from(t.0) * i128::from(rate.num)).div_euclid(i128::from(TICKS_PER_SECOND) * i128::from(rate.den))
}

/// Ceiling of the rational boundary. An overflowing numerator divided by an i64 rate cannot
/// fit in an i64 tick, so saturating that intermediate preserves the public result. Boundaries
/// derived from frame_index remain within i128 (at most one frame away from an i64 tick).
fn frame_boundary(rate: FrameRate, frame: i128) -> i128 {
    let Some(n) = frame.checked_mul(i128::from(TICKS_PER_SECOND)).and_then(|n| n.checked_mul(i128::from(rate.den))) else {
        return if frame < 0 { i128::MIN } else { i128::MAX };
    };
    let d = i128::from(rate.num);
    // Flooring a fractional boundary puts the tick in the preceding frame.
    n.div_euclid(d) + i128::from(n.rem_euclid(d) != 0)
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Drop-frame parameters: frames dropped per minute and nominal base.
fn df_params(rate: FrameRate) -> (i64, i64) {
    let base = rate.timecode_base();
    (base / 15, base) // 30 → 2, 60 → 4, 120 → 8
}

/// Convert a frame count to SMPTE fields `(negative, h, m, s, f)`.
pub fn frames_to_fields(frame: i64, rate: FrameRate, drop_frame: bool) -> (bool, i64, i64, i64, i64) {
    let neg = frame < 0;
    let mut n = i128::from(frame).abs();
    let (drop, base) = df_params(rate);
    let (drop, base) = (i128::from(drop), i128::from(base));
    if drop_frame && rate.supports_drop_frame() {
        let per_min = base * 60 - drop;
        let per_10 = per_min * 10 + drop;
        let d = n / per_10;
        let m = n % per_10;
        n = n.saturating_add(drop * 9 * d);
        if m > drop {
            n = n.saturating_add(drop * ((m - drop) / per_min));
        }
    }
    let f = n % base;
    let s = (n / base) % 60;
    let mi = (n / (base * 60)) % 60;
    let h = n / (base * 3600);
    (neg, clamp_i64(h), clamp_i64(mi), clamp_i64(s), clamp_i64(f))
}

/// Convert SMPTE fields back to a frame count.
pub fn fields_to_frames(h: i64, m: i64, s: i64, f: i64, rate: FrameRate, drop_frame: bool) -> i64 {
    let (drop, base) = df_params(rate);
    let seconds = i128::from(h) * 3600 + i128::from(m) * 60 + i128::from(s);
    // Nominal drop-frame labels can exceed i64 even when the final frame count fits.
    let mut n = seconds.saturating_mul(i128::from(base)).saturating_add(i128::from(f));
    if drop_frame && rate.supports_drop_frame() {
        let total_min = i128::from(h) * 60 + i128::from(m);
        n = n.saturating_sub(i128::from(drop) * (total_min - total_min / 10));
    }
    clamp_i64(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_rates_and_milliseconds_are_exact() {
        for r in [
            FrameRate::FPS_23_976,
            FrameRate::FPS_24,
            FrameRate::FPS_25,
            FrameRate::FPS_29_97,
            FrameRate::FPS_30,
            FrameRate::FPS_50,
            FrameRate::FPS_59_94,
            FrameRate::FPS_60,
        ] {
            let d = r.frame_duration();
            assert_eq!(i128::from(d.0) * i128::from(r.num), i128::from(TICKS_PER_SECOND) * i128::from(r.den), "{r:?}");
            for f in [-1000, -1, 0, 1, 1799, 107_892, 10_000_000] {
                assert_eq!(r.frame_at(r.tick_of(f)), f, "{r:?} {f}");
            }
        }
        assert_eq!(TICKS_PER_SECOND % 1000, 0);
    }

    #[test]
    fn drop_frame_known_values() {
        let r = FrameRate::FPS_29_97;
        let fields = |f, df| frames_to_fields(f, r, df);
        assert_eq!(fields(1799, true), (false, 0, 0, 59, 29));
        assert_eq!(fields(1800, true), (false, 0, 1, 0, 2));
        assert_eq!(fields(17982, true), (false, 0, 10, 0, 0));
        assert_eq!(fields(107_892, true), (false, 1, 0, 0, 0));
        assert_eq!(fields(1800, false), (false, 0, 1, 0, 0));
        assert_eq!(frames_to_fields(3600, FrameRate::FPS_59_94, true), (false, 0, 1, 0, 4));
        for f in (0..24 * 107_892).step_by(997) {
            let (_, h, m, s, ff) = fields(f, true);
            assert_eq!(fields_to_frames(h, m, s, ff, r, true), f);
        }
    }

    /// A damaged file can carry a zero or negative rate or a huge time: nothing divides by zero
    /// or wraps.
    #[test]
    fn hostile_rates_and_times_saturate() {
        for r in [FrameRate { num: 0, den: 0 }, FrameRate { num: 0, den: 1 }, FrameRate { num: 25, den: 0 }, FrameRate { num: -30, den: 1 }] {
            assert_eq!(r.tick_of(10), FrameRate::default().tick_of(10));
            assert_eq!(r.frame_at(Tick(TICKS_PER_SECOND)), FrameRate::default().frame_at(Tick(TICKS_PER_SECOND)));
            assert!(r.frame_duration() > Tick::ZERO);
            assert!(r.timecode_base() >= 1);
            let _ = r.snap_nearest(Tick(12345));
        }
        assert_eq!(Tick(i64::MAX) + Tick(1), Tick(i64::MAX));
        assert_eq!(Tick(i64::MIN) - Tick(1), Tick(i64::MIN));
        assert_eq!(-Tick(i64::MIN), Tick(i64::MAX));
        assert_eq!(FrameRate::FPS_25.tick_of(i64::MAX), Tick(i64::MAX));
        assert_eq!(FrameRate::new(i64::MAX, 1).frame_at(Tick(i64::MAX)), i64::MAX);
        let _ = frames_to_fields(i64::MIN, FrameRate::FPS_29_97, true);
        let _ = fields_to_frames(i64::MAX, i64::MAX, i64::MAX, i64::MAX, FrameRate::FPS_29_97, true);
    }
}
