//! Easing curves, keyed tracks, motion paths and springs.
//!
//! * [`Easing`] maps linear progress `t` in `0..=1` to eased progress: no ease, hold, the
//!   Classic slider, nine named families (Quad, Cubic, Quart, Quint, Sine, Back, Circ, Bounce,
//!   Elastic), each In, Out and In Out, one cubic Bezier (also built from speed and influence,
//!   [`TemporalEase`]), an editable multi-segment [`CustomEase`], a [`SampledEase`] table and a
//!   [`SpringEase`]. [`Easing::apply`] is a pure function that does not allocate, and it returns
//!   exactly 0 at the start and exactly 1 at the end.
//! * [`Track`] holds keys of any [`Animatable`] value (`f32`, `f64`, [`Vec2`], `[f32; N]`,
//!   `[f64; N]`) at times in seconds, each easing into the next. Sampling is a binary search and
//!   does not allocate.
//! * [`MotionPath`] is a chain of cubic Bezier segments with arc-length tables built once, for
//!   constant-speed motion along a curve.
//! * [`Spring`] and [`SpringState`] move a value toward a target as a damped spring. Each step
//!   uses the exact solution of the spring equation, so a step costs the same at any frame rate
//!   and length.
//!
//! The named families are the classic public easing equations published by Robert Penner.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod custom;
mod easing;
mod path;
mod sampled;
mod spring;
mod track;
mod vec2;

pub use custom::{CustomEase, EasePoint, Handle, MAX_EASE_POINTS, MAX_EASE_Y};
pub use easing::{EaseDir, EaseFamily, Easing, MIN_INFLUENCE, SpringEase, TemporalEase};
pub use path::{MAX_PATH_SEGMENTS, MotionPath};
pub use sampled::{MAX_EASE_SAMPLES, POLYLINE_POINTS, SampledEase};
pub use spring::{MAX_DAMPING_RATIO, MAX_SPRING_HZ, Spring, SpringState};
pub use track::{Animatable, Key, MAX_TRACK_KEYS, Track};
pub use vec2::Vec2;

/// Why a value was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum EaseError {
    /// The named input is NaN or infinite.
    NotFinite(&'static str),
    /// The named input is outside the range it must lie in.
    OutOfRange { what: &'static str, min: f64, max: f64 },
    /// A track or path holds as many entries as it may.
    Full(usize),
    /// A path or curve needs more points than it was given.
    TooFew { what: &'static str, need: usize },
    /// The track has no keys.
    Empty,
    /// Two keys are at this time.
    SameTime(f64),
    /// The result is too large to represent.
    Overflow,
}

impl std::fmt::Display for EaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EaseError::NotFinite(what) => write!(f, "{what} is not a finite number"),
            EaseError::OutOfRange { what, min, max } => write!(f, "{what} must be between {min} and {max}"),
            EaseError::Full(n) => write!(f, "at most {n} entries are allowed"),
            EaseError::TooFew { what, need } => write!(f, "{what} needs at least {need} points"),
            EaseError::Empty => write!(f, "the track has no keys"),
            EaseError::SameTime(t) => write!(f, "two keys are at {t} s"),
            EaseError::Overflow => write!(f, "the result is too large"),
        }
    }
}

impl std::error::Error for EaseError {}

/// `v` itself when it is finite.
pub(crate) fn finite(v: f64, what: &'static str) -> Result<f64, EaseError> {
    if v.is_finite() { Ok(v) } else { Err(EaseError::NotFinite(what)) }
}

/// `v` itself when it is finite and within `min..=max`.
pub(crate) fn within(v: f64, min: f64, max: f64, what: &'static str) -> Result<f64, EaseError> {
    let v = finite(v, what)?;
    if (min..=max).contains(&v) { Ok(v) } else { Err(EaseError::OutOfRange { what, min, max }) }
}

/// The value `t` of the way from `a` to `b`. When a step on the way is too large to represent
/// (`a` and `b` near opposite ends of the `f64` range, or an overshooting `t`), it is worked
/// out at half scale, so only a result too large to represent is infinite.
pub(crate) fn lerp(a: f64, b: f64, t: f64) -> f64 {
    let r = a + (b - a) * t;
    if r.is_finite() { r } else { 2.0 * (0.5 * a + (0.5 * b - 0.5 * a) * t) }
}

/// Reads a list, keeping its first `max` entries and skipping the rest without storing them.
#[cfg(feature = "serde")]
pub(crate) fn read_first<'de, D, T>(d: D, max: usize) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    use serde::de::{IgnoredAny, SeqAccess, Visitor};
    struct V<T>(usize, std::marker::PhantomData<T>);
    impl<'de, T: serde::Deserialize<'de>> Visitor<'de> for V<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "a list")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(self.0).min(1024));
            while out.len() < self.0 {
                match seq.next_element()? {
                    Some(v) => out.push(v),
                    None => return Ok(out),
                }
            }
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(out)
        }
    }
    d.deserialize_seq(V(max, std::marker::PhantomData))
}

/// Reads a string and keeps only what `clean` makes of it, so a long string in a file is never
/// copied whole into the value.
///
/// This bounds the memory the value keeps. A format that has to decode escapes may still build
/// the whole string in a temporary buffer first; that buffer is no larger than the input, so the
/// caller bounds it by limiting the size of the file it reads.
#[cfg(feature = "serde")]
pub(crate) fn read_cleaned<'de, D: serde::Deserializer<'de>>(d: D, clean: fn(&str) -> String) -> Result<String, D::Error> {
    struct V(fn(&str) -> String);
    impl serde::de::Visitor<'_> for V {
        type Value = String;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "a string")
        }
        fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<String, E> {
            Ok((self.0)(s))
        }
    }
    d.deserialize_str(V(clean))
}

/// Cubic Bezier in one dimension.
pub(crate) fn bez(t: f64, p: [f64; 4]) -> f64 {
    let mt = 1.0 - t;
    mt * mt * mt * p[0] + 3.0 * mt * mt * t * p[1] + 3.0 * mt * t * t * p[2] + t * t * t * p[3]
}

/// The derivative of [`bez`].
pub(crate) fn dbez(t: f64, p: [f64; 4]) -> f64 {
    let mt = 1.0 - t;
    3.0 * mt * mt * (p[1] - p[0]) + 6.0 * mt * t * (p[2] - p[1]) + 3.0 * t * t * (p[3] - p[2])
}

/// Most steps [`solve_monotonic`] takes.
pub(crate) const SOLVE_STEPS: u32 = 64;
/// How close [`solve_monotonic`] brings `bez(s)` to `x`, relative to the curve's span.
pub(crate) const SOLVE_TOLERANCE: f64 = 1e-12;

/// The parameter where the Bezier `p` (non-decreasing, as it is when its inner points lie
/// between its ends) reaches `x`: Newton steps kept inside a shrinking bracket, halving when a step leaves it, to
/// within [`SOLVE_TOLERANCE`] of the span in at most [`SOLVE_STEPS`] steps. The curve with both
/// handles at full reach (`p1 = p3`, `p2 = p0`), whose slope is zero halfway, is inverted in
/// closed form.
pub(crate) fn solve_monotonic(x: f64, p: [f64; 4]) -> f64 {
    solve_counted(x, p).0
}

/// [`solve_monotonic`] and the number of steps it took.
pub(crate) fn solve_counted(x: f64, p: [f64; 4]) -> (f64, u32) {
    let span = p[3] - p[0];
    if span.is_nan() || span <= 0.0 {
        return (0.5, 0);
    }
    if x <= p[0] {
        return (0.0, 0);
    }
    if x >= p[3] {
        return (1.0, 0);
    }
    let u = (x - p[0]) / span;
    if p[1] == p[3] && p[2] == p[0] {
        // x(s) = 0.5 + 4 (s − 0.5)³ on the unit span.
        return ((0.5 + ((u - 0.5) * 0.25).cbrt()).clamp(0.0, 1.0), 0);
    }
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    let mut s = u.clamp(0.0, 1.0);
    let tol = SOLVE_TOLERANCE * span;
    for step in 1..=SOLVE_STEPS {
        let err = bez(s, p) - x;
        if err.abs() <= tol {
            return (s, step);
        }
        if err < 0.0 {
            lo = s;
        } else {
            hi = s;
        }
        let d = dbez(s, p);
        let next = if d.abs() > 1e-300 { s - err / d } else { f64::NAN };
        s = if next > lo && next < hi { next } else { 0.5 * (lo + hi) };
    }
    (s, SOLVE_STEPS)
}

#[cfg(test)]
#[path = "tests/easing_tests.rs"]
mod easing_tests;

#[cfg(test)]
#[path = "tests/track_tests.rs"]
mod track_tests;

#[cfg(test)]
#[path = "tests/path_tests.rs"]
mod path_tests;

#[cfg(test)]
#[path = "tests/spring_tests.rs"]
mod spring_tests;
