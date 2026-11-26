//! [`Easing`]: maps linear progress to eased progress.

use std::sync::Arc;

use crate::custom::{CustomEase, EasePoint, MAX_EASE_Y};
use crate::sampled::SampledEase;
use crate::spring::Spring;
use crate::{EaseError, Vec2, bez, finite, solve_monotonic, within};

/// The Back family's overshoot (about 10 %).
const BACK: f64 = 1.701_58;
/// Back In Out's overshoot, as the published equation scales it.
const BACK_IN_OUT: f64 = BACK * 1.525;
/// The smallest influence a [`TemporalEase`] may have.
pub const MIN_INFLUENCE: f64 = 0.001;

/// The families of named eases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EaseFamily {
    Quad,
    Cubic,
    Quart,
    Quint,
    Sine,
    Back,
    Circ,
    Bounce,
    Elastic,
}

impl EaseFamily {
    pub const ALL: [EaseFamily; 9] = [
        EaseFamily::Quad,
        EaseFamily::Cubic,
        EaseFamily::Quart,
        EaseFamily::Quint,
        EaseFamily::Sine,
        EaseFamily::Back,
        EaseFamily::Circ,
        EaseFamily::Bounce,
        EaseFamily::Elastic,
    ];

    /// The family's English name.
    pub fn label(self) -> &'static str {
        match self {
            EaseFamily::Quad => "Quad",
            EaseFamily::Cubic => "Cubic",
            EaseFamily::Quart => "Quart",
            EaseFamily::Quint => "Quint",
            EaseFamily::Sine => "Sine",
            EaseFamily::Back => "Back",
            EaseFamily::Circ => "Circ",
            EaseFamily::Bounce => "Bounce",
            EaseFamily::Elastic => "Elastic",
        }
    }

    /// The ease in, `In(t)`. With `in_out` set, the curve whose halves make In Out: the
    /// published In Out equations scale Back's overshoot by 1.525 and give Elastic a longer
    /// period (4.5 instead of 3 per unit) and its own phase; the other families use `In(t)`.
    fn ease_in(self, t: f64, in_out: bool) -> f64 {
        use std::f64::consts::PI;
        match self {
            EaseFamily::Quad => t * t,
            EaseFamily::Cubic => t * t * t,
            EaseFamily::Quart => t * t * t * t,
            EaseFamily::Quint => t * t * t * t * t,
            EaseFamily::Sine => 1.0 - (t * PI * 0.5).cos(),
            EaseFamily::Back => {
                let c = if in_out { BACK_IN_OUT } else { BACK };
                (c + 1.0) * t * t * t - c * t * t
            }
            EaseFamily::Circ => 1.0 - (1.0 - t * t).max(0.0).sqrt(),
            EaseFamily::Bounce => 1.0 - bounce_out(1.0 - t),
            EaseFamily::Elastic => {
                if t <= 0.0 {
                    0.0
                } else if t >= 1.0 {
                    1.0
                } else if in_out {
                    -(2f64.powf(10.0 * t - 10.0)) * ((10.0 * t - 11.125) * (2.0 * PI / 4.5)).sin()
                } else {
                    -(2f64.powf(10.0 * t - 10.0)) * ((10.0 * t - 10.75) * (2.0 * PI / 3.0)).sin()
                }
            }
        }
    }

    /// Where the ease in changes formula (other than 0 and 1): Bounce is four parabolas.
    fn breaks_in(self) -> &'static [f64] {
        match self {
            // 1 − the Out breakpoints (1/2.75, 2/2.75, 2.5/2.75), in order.
            EaseFamily::Bounce => &[0.25 / 2.75, 0.75 / 2.75, 1.75 / 2.75],
            _ => &[],
        }
    }

    /// Bezier segments per ease in that draw it to within a hair (see [`Easing::to_custom`]);
    /// cubic polynomials (Quad, Cubic, Back) and the parabolas of Bounce are exact with one.
    fn pieces_in(self) -> usize {
        match self {
            EaseFamily::Quad | EaseFamily::Cubic | EaseFamily::Back | EaseFamily::Bounce | EaseFamily::Circ => 1,
            EaseFamily::Quart | EaseFamily::Quint | EaseFamily::Sine => 4,
            EaseFamily::Elastic => 24,
        }
    }
}

fn bounce_out(t: f64) -> f64 {
    const N: f64 = 7.5625;
    const D: f64 = 2.75;
    if t < 1.0 / D {
        N * t * t
    } else if t < 2.0 / D {
        let u = t - 1.5 / D;
        N * u * u + 0.75
    } else if t < 2.5 / D {
        let u = t - 2.25 / D;
        N * u * u + 0.9375
    } else {
        let u = t - 2.625 / D;
        N * u * u + 0.984_375
    }
}

/// Which way a named ease eases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EaseDir {
    In,
    Out,
    InOut,
}

impl EaseDir {
    pub const ALL: [EaseDir; 3] = [EaseDir::In, EaseDir::Out, EaseDir::InOut];

    pub fn label(self) -> &'static str {
        match self {
            EaseDir::In => "In",
            EaseDir::Out => "Out",
            EaseDir::InOut => "In Out",
        }
    }
}

/// One side of a segment's ease as a keyframe velocity dialog gives it: `speed` normalized
/// (speed × segment duration ÷ value change, so 1 is the straight line's speed) and
/// `influence` the fraction of the segment its handle reaches.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TemporalEase {
    pub speed: f64,
    pub influence: f64,
}

impl TemporalEase {
    /// A side that leaves at rest with a third of the segment's influence.
    pub const EASY: TemporalEase = TemporalEase { speed: 0.0, influence: 1.0 / 3.0 };
    /// A linear side next to an eased one: the segment's average speed with a sixth of its
    /// influence.
    pub const LINEAR: TemporalEase = TemporalEase { speed: 1.0, influence: 1.0 / 6.0 };
}

/// A damped spring as an ease: a mass on a spring with damping, released from 0 toward 1 and
/// played over the time it takes to stay within 0.1 % of 1 (at most 10 seconds of its motion).
/// Values outside the ranges [`Spring::from_physical`] accepts are brought into them when the
/// ease is evaluated.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SpringEase {
    pub mass: f64,
    pub stiffness: f64,
    pub damping: f64,
}

impl SpringEase {
    /// A gentle spring with a small overshoot (mass 1, stiffness 100, damping 15).
    pub const GENTLE: SpringEase = SpringEase { mass: 1.0, stiffness: 100.0, damping: 15.0 };

    fn apply(self, t: f64) -> f64 {
        Spring::physical_lenient(self.mass, self.stiffness, self.damping).ease(t)
    }
}

/// Maps linear progress `t ∈ [0, 1]` to eased progress.
///
/// The unit variants are the named presets; their names are the file format and the names
/// commands use. [`Easing::apply`] returns exactly 0 for `t <= 0` (and for NaN) and exactly 1
/// for `t >= 1`, whatever the variant holds.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Easing {
    #[default]
    Linear,
    QuadIn,
    QuadOut,
    QuadInOut,
    CubicIn,
    CubicOut,
    CubicInOut,
    /// The traditional single-slider ease: −100 (ease in) … 0 (linear) … +100 (ease out).
    Classic(f64),
    /// A cubic Bezier through (0, 0), (x1, y1), (x2, y2), (1, 1), as the CSS Easing Functions
    /// specification defines `cubic-bezier()`. Evaluation keeps x1 and x2 within `0..=1` and
    /// y1 and y2 within ±[`MAX_EASE_Y`].
    CubicBezier(f64, f64, f64, f64),
    QuartIn,
    QuartOut,
    QuartInOut,
    QuintIn,
    QuintOut,
    QuintInOut,
    SineIn,
    SineOut,
    SineInOut,
    BackIn,
    BackOut,
    BackInOut,
    CircIn,
    CircOut,
    CircInOut,
    BounceIn,
    BounceOut,
    BounceInOut,
    ElasticIn,
    ElasticOut,
    ElasticInOut,
    /// A curve of several Bezier segments, shared when copied.
    Custom(Arc<CustomEase>),
    /// A table of values, read between samples by straight lines.
    Sampled(Arc<SampledEase>),
    /// A damped spring.
    Spring(SpringEase),
    /// Stays at the start until the end, then jumps: 0 for `t < 1`.
    Hold,
}

impl Easing {
    pub const LINEAR_LABEL: &'static str = "Linear";
    /// The `smooth` alias's curve.
    pub const SMOOTH: Easing = Easing::CubicBezier(0.25, 0.1, 0.25, 1.0);

    /// A cubic Bezier ease, checked: every value finite, `x1` and `x2` within `0..=1` (so time
    /// runs forwards), `y1` and `y2` within ±[`MAX_EASE_Y`].
    pub fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64) -> Result<Easing, EaseError> {
        within(x1, 0.0, 1.0, "x1")?;
        within(x2, 0.0, 1.0, "x2")?;
        within(y1, -MAX_EASE_Y, MAX_EASE_Y, "y1")?;
        within(y2, -MAX_EASE_Y, MAX_EASE_Y, "y2")?;
        Ok(Easing::CubicBezier(x1, y1, x2, y2))
    }

    /// The ease of a segment from its two sides: `out` leaving the first key, `into` arriving at
    /// the second. Each influence must be within [`MIN_INFLUENCE`] to 1 and each speed finite.
    /// The result is the cubic Bezier with handles `(i0, s0 × i0)` and `(1 − i1, 1 − s1 × i1)`.
    pub fn from_temporal(out: TemporalEase, into: TemporalEase) -> Result<Easing, EaseError> {
        finite(out.speed, "out speed")?;
        finite(into.speed, "in speed")?;
        let i0 = within(out.influence, MIN_INFLUENCE, 1.0, "out influence")?;
        let i1 = within(into.influence, MIN_INFLUENCE, 1.0, "in influence")?;
        let (y1, y2) = (out.speed * i0, 1.0 - into.speed * i1);
        if !(y1.abs() <= MAX_EASE_Y && y2.abs() <= MAX_EASE_Y) {
            return Err(EaseError::OutOfRange { what: "speed × influence", min: -MAX_EASE_Y, max: MAX_EASE_Y });
        }
        Ok(Easing::CubicBezier(i0, y1, 1.0 - i1, y2))
    }

    /// The two sides of an ease that one cubic Bezier draws (see [`Easing::from_temporal`] and
    /// [`Easing::as_bezier`]). A handle with no reach has speed 0. `None` when a handle's reach
    /// is so short that its speed is too large to represent.
    pub fn temporal(&self) -> Option<(TemporalEase, TemporalEase)> {
        let (x1, y1, x2, y2) = self.as_bezier()?;
        let (i0, i1) = (x1.clamp(0.0, 1.0), (1.0 - x2).clamp(0.0, 1.0));
        let s0 = if i0 > 0.0 { y1 / i0 } else { 0.0 };
        let s1 = if i1 > 0.0 { (1.0 - y2) / i1 } else { 0.0 };
        if !(s0.is_finite() && s1.is_finite()) {
            return None;
        }
        Some((TemporalEase { speed: s0, influence: i0 }, TemporalEase { speed: s1, influence: i1 }))
    }

    /// A spring ease, checked as [`Spring::from_physical`] checks its values.
    pub fn spring(mass: f64, stiffness: f64, damping: f64) -> Result<Easing, EaseError> {
        Spring::from_physical(mass, stiffness, damping)?;
        Ok(Easing::Spring(SpringEase { mass, stiffness, damping }))
    }

    /// The named ease of `family` easing `dir`.
    pub fn preset(family: EaseFamily, dir: EaseDir) -> Easing {
        use EaseDir::*;
        use EaseFamily::*;
        match (family, dir) {
            (Quad, In) => Easing::QuadIn,
            (Quad, Out) => Easing::QuadOut,
            (Quad, InOut) => Easing::QuadInOut,
            (Cubic, In) => Easing::CubicIn,
            (Cubic, Out) => Easing::CubicOut,
            (Cubic, InOut) => Easing::CubicInOut,
            (Quart, In) => Easing::QuartIn,
            (Quart, Out) => Easing::QuartOut,
            (Quart, InOut) => Easing::QuartInOut,
            (Quint, In) => Easing::QuintIn,
            (Quint, Out) => Easing::QuintOut,
            (Quint, InOut) => Easing::QuintInOut,
            (Sine, In) => Easing::SineIn,
            (Sine, Out) => Easing::SineOut,
            (Sine, InOut) => Easing::SineInOut,
            (Back, In) => Easing::BackIn,
            (Back, Out) => Easing::BackOut,
            (Back, InOut) => Easing::BackInOut,
            (Circ, In) => Easing::CircIn,
            (Circ, Out) => Easing::CircOut,
            (Circ, InOut) => Easing::CircInOut,
            (Bounce, In) => Easing::BounceIn,
            (Bounce, Out) => Easing::BounceOut,
            (Bounce, InOut) => Easing::BounceInOut,
            (Elastic, In) => Easing::ElasticIn,
            (Elastic, Out) => Easing::ElasticOut,
            (Elastic, InOut) => Easing::ElasticInOut,
        }
    }

    /// The family and direction of a named ease.
    pub fn family(&self) -> Option<(EaseFamily, EaseDir)> {
        use EaseDir::*;
        use EaseFamily::*;
        Some(match self {
            Easing::QuadIn => (Quad, In),
            Easing::QuadOut => (Quad, Out),
            Easing::QuadInOut => (Quad, InOut),
            Easing::CubicIn => (Cubic, In),
            Easing::CubicOut => (Cubic, Out),
            Easing::CubicInOut => (Cubic, InOut),
            Easing::QuartIn => (Quart, In),
            Easing::QuartOut => (Quart, Out),
            Easing::QuartInOut => (Quart, InOut),
            Easing::QuintIn => (Quint, In),
            Easing::QuintOut => (Quint, Out),
            Easing::QuintInOut => (Quint, InOut),
            Easing::SineIn => (Sine, In),
            Easing::SineOut => (Sine, Out),
            Easing::SineInOut => (Sine, InOut),
            Easing::BackIn => (Back, In),
            Easing::BackOut => (Back, Out),
            Easing::BackInOut => (Back, InOut),
            Easing::CircIn => (Circ, In),
            Easing::CircOut => (Circ, Out),
            Easing::CircInOut => (Circ, InOut),
            Easing::BounceIn => (Bounce, In),
            Easing::BounceOut => (Bounce, Out),
            Easing::BounceInOut => (Bounce, InOut),
            Easing::ElasticIn => (Elastic, In),
            Easing::ElasticOut => (Elastic, Out),
            Easing::ElasticInOut => (Elastic, InOut),
            Easing::Linear
            | Easing::Classic(_)
            | Easing::CubicBezier(..)
            | Easing::Custom(_)
            | Easing::Sampled(_)
            | Easing::Spring(_)
            | Easing::Hold => return None,
        })
    }

    /// Every named preset: no ease, then each family In, Out, In Out.
    pub fn named() -> impl Iterator<Item = Easing> {
        std::iter::once(Easing::Linear).chain(EaseFamily::ALL.into_iter().flat_map(|f| EaseDir::ALL.into_iter().map(move |d| Easing::preset(f, d))))
    }

    /// The preset's name in commands and files (`"QuadIn"`, `"ElasticInOut"`, `"Hold"`), `None`
    /// for eases with values.
    pub fn name(&self) -> Option<String> {
        match self {
            Easing::Linear => Some("Linear".into()),
            Easing::Hold => Some("Hold".into()),
            _ => self.family().map(|(f, d)| format!("{}{}", f.label(), d.label().replace(' ', ""))),
        }
    }

    /// The preset named `name`, ignoring case, spaces, `-` and `_` (`"back in out"`), with the
    /// aliases `none`, `easein`, `easeout`, `easeinout` and `smooth`.
    pub fn from_name(name: &str) -> Option<Easing> {
        // No preset or alias is longer than this, so a longer name is unknown and is not copied
        // past it.
        const LONGEST: usize = 12;
        let key: String = name.chars().filter(|c| !matches!(c, '-' | '_' | ' ')).flat_map(char::to_lowercase).take(LONGEST + 1).collect();
        if key.chars().count() > LONGEST {
            return None;
        }
        match key.as_str() {
            "none" | "noease" => return Some(Easing::Linear),
            "hold" => return Some(Easing::Hold),
            "easein" => return Some(Easing::QuadIn),
            "easeout" => return Some(Easing::QuadOut),
            "easeinout" => return Some(Easing::QuadInOut),
            "smooth" => return Some(Easing::SMOOTH),
            _ => {}
        }
        Easing::named().find(|e| e.name().is_some_and(|n| n.to_lowercase() == key))
    }

    /// An English label for menus and reports.
    pub fn label(&self) -> String {
        match self {
            Easing::Linear => Easing::LINEAR_LABEL.to_string(),
            Easing::Hold => "Hold".to_string(),
            Easing::Classic(s) => format!("Classic {s:+.0}"),
            Easing::CubicBezier(..) if *self == Easing::SMOOTH => "Smooth (bezier)".to_string(),
            Easing::CubicBezier(a, b, c, d) => format!("Bezier({a:.2}, {b:.2}, {c:.2}, {d:.2})"),
            Easing::Custom(_) => "Custom".to_string(),
            Easing::Sampled(s) => s.name().to_string(),
            Easing::Spring(s) => format!("Spring {:.0}, {:.0}, {:.0}", s.mass, s.stiffness, s.damping),
            other => other.family().map_or_else(|| "Custom".to_string(), |(f, d)| format!("{} {}", f.label(), d.label())),
        }
    }

    /// The single cubic Bezier `(x1, y1, x2, y2)` that draws this ease: exact for linear, the
    /// Classic slider, the Quad, Cubic and Back eases in and out, and a custom ease of one
    /// segment; the Quad and Cubic in-outs get their usual one-curve approximations. `None` for
    /// eases one curve cannot draw (Bounce, Elastic, the other families, hold, springs, sampled
    /// tables, custom eases of several segments, and a custom ease of one segment whose handle
    /// reaches past the unit box in x).
    pub fn as_bezier(&self) -> Option<(f64, f64, f64, f64)> {
        const THIRD: f64 = 1.0 / 3.0;
        Some(match self {
            Easing::Linear => (THIRD, THIRD, 2.0 * THIRD, 2.0 * THIRD),
            // t², t³ and (c+1)t³ − ct² and their mirrors, with x(s) = s.
            Easing::QuadIn => (THIRD, 0.0, 2.0 * THIRD, THIRD),
            Easing::QuadOut => (THIRD, 2.0 * THIRD, 2.0 * THIRD, 1.0),
            Easing::CubicIn => (THIRD, 0.0, 2.0 * THIRD, 0.0),
            Easing::CubicOut => (THIRD, 1.0, 2.0 * THIRD, 1.0),
            Easing::BackIn => (THIRD, 0.0, 2.0 * THIRD, -BACK * THIRD),
            Easing::BackOut => (THIRD, 1.0 + BACK * THIRD, 2.0 * THIRD, 1.0),
            Easing::QuadInOut => (0.45, 0.0, 0.55, 1.0),
            Easing::CubicInOut => (0.65, 0.0, 0.35, 1.0),
            Easing::Classic(strength) => {
                // The quadratic 2(1 − t)t·c + t², raised to a cubic.
                let c = 0.5 + 0.5 * classic_strength(*strength);
                (THIRD, 2.0 * c * THIRD, 2.0 * THIRD, (2.0 * c + 1.0) * THIRD)
            }
            Easing::CubicBezier(a, b, c, d) => bezier_handles(*a, *b, *c, *d),
            Easing::Custom(c) => match c.points() {
                [a, b] if a.out.x <= 1.0 && b.h_in.x >= -1.0 => (a.out.x, a.out.y, b.at.x + b.h_in.x, b.at.y + b.h_in.y),
                _ => return None,
            },
            _ => return None,
        })
    }

    /// Eased progress at `t`: exactly 0 for `t <= 0` or NaN, exactly 1 for `t >= 1`.
    pub fn apply(&self, t: f64) -> f64 {
        if t.is_nan() || t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        let v = match self {
            Easing::Linear => t,
            Easing::Hold => 0.0,
            Easing::Classic(strength) => {
                // Quadratic Bezier with the middle control point moved by `strength`.
                let c = 0.5 + 0.5 * classic_strength(*strength);
                2.0 * (1.0 - t) * t * c + t * t
            }
            Easing::CubicBezier(x1, y1, x2, y2) => {
                let (x1, y1, x2, y2) = bezier_handles(*x1, *y1, *x2, *y2);
                bez(solve_monotonic(t, [0.0, x1, x2, 1.0]), [0.0, y1, y2, 1.0])
            }
            Easing::Custom(c) => c.apply(t),
            Easing::Sampled(s) => s.apply(t),
            Easing::Spring(s) => s.apply(t),
            named => match named.family() {
                Some((f, d)) => eval_named(f, d, t),
                None => t,
            },
        };
        if v.is_finite() { v } else { t }
    }

    /// This ease as a custom curve for a graph editor: exact for linear, Classic, a Bezier,
    /// the Quad, Cubic, Back and Bounce families and custom eases; Circ is two circular arcs,
    /// and Quart, Quint, Sine and Elastic are fitted by matching values and slopes at a few
    /// points (all within 0.5 % of the range everywhere). A sampled ease or a spring becomes the
    /// straight segments through [`crate::POLYLINE_POINTS`] of its values, and a hold a corner
    /// at the end.
    pub fn to_custom(&self) -> CustomEase {
        match self {
            Easing::Custom(c) => (**c).clone(),
            Easing::Sampled(s) => s.to_polyline(),
            Easing::Spring(_) => {
                let n = crate::MAX_EASE_SAMPLES;
                let values = (0..n).map(|i| self.apply(i as f64 / (n - 1) as f64)).collect();
                SampledEase::new("Spring", "", "", values).to_polyline()
            }
            Easing::Hold => {
                // Flat at 0 across the segment, then straight up to 1 at the end.
                let third = 1.0 / 3.0;
                CustomEase::new(vec![
                    EasePoint { at: Vec2::ZERO, h_in: Vec2::ZERO, out: Vec2::new(third, 0.0), smooth: false },
                    EasePoint { at: Vec2::new(1.0, 0.0), h_in: Vec2::new(-third, 0.0), out: Vec2::ZERO, smooth: false },
                    EasePoint { at: Vec2::new(1.0, 1.0), h_in: Vec2::ZERO, out: Vec2::ZERO, smooth: false },
                ])
            }
            Easing::Linear => CustomEase::linear(),
            Easing::Classic(_) | Easing::CubicBezier(..) => {
                let Some((x1, y1, x2, y2)) = self.as_bezier() else { return CustomEase::linear() };
                CustomEase::from_bezier(x1, y1, x2, y2)
            }
            named => match named.family() {
                Some((f, d)) => fit_named(f, d),
                None => CustomEase::linear(),
            },
        }
    }

    /// The custom curve, when this is one.
    pub fn custom(&self) -> Option<&CustomEase> {
        match self {
            Easing::Custom(c) => Some(c),
            _ => None,
        }
    }

    /// The sampled curve, when this is one.
    pub fn sampled(&self) -> Option<&SampledEase> {
        match self {
            Easing::Sampled(s) => Some(s),
            _ => None,
        }
    }
}

/// A cubic Bezier's handles as evaluation uses them: x within `0..=1`, y within ±[`MAX_EASE_Y`],
/// and the straight line for a handle that is not finite.
fn bezier_handles(x1: f64, y1: f64, x2: f64, y2: f64) -> (f64, f64, f64, f64) {
    if ![x1, y1, x2, y2].iter().all(|v| v.is_finite()) {
        return (1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0);
    }
    (x1.clamp(0.0, 1.0), y1.clamp(-MAX_EASE_Y, MAX_EASE_Y), x2.clamp(0.0, 1.0), y2.clamp(-MAX_EASE_Y, MAX_EASE_Y))
}

fn classic_strength(s: f64) -> f64 {
    if s.is_finite() { s.clamp(-100.0, 100.0) / 100.0 } else { 0.0 }
}

fn eval_named(f: EaseFamily, d: EaseDir, t: f64) -> f64 {
    match d {
        EaseDir::In => f.ease_in(t, false),
        EaseDir::Out => 1.0 - f.ease_in(1.0 - t, false),
        EaseDir::InOut => {
            if t < 0.5 {
                0.5 * f.ease_in(2.0 * t, true)
            } else {
                1.0 - 0.5 * f.ease_in(2.0 - 2.0 * t, true)
            }
        }
    }
}

/// The slope of `y` at `x`, from inside `[lo, hi]`: one-sided, a step in from `x` (so a value
/// pinned at an end, as Elastic's are, does not count), exact for parabolas but for that step.
fn slope(y: &dyn Fn(f64) -> f64, x: f64, lo: f64, hi: f64) -> f64 {
    const H: f64 = 1e-6;
    let s = if x + 3.0 * H <= hi {
        let d = (-3.0 * y(x + H) + 4.0 * y(x + 2.0 * H) - y(x + 3.0 * H)) / (2.0 * H);
        let d2 = (y(x + H) - 2.0 * y(x + 2.0 * H) + y(x + 3.0 * H)) / (H * H);
        d - d2 * H
    } else if x - 3.0 * H >= lo {
        let d = (3.0 * y(x - H) - 4.0 * y(x - 2.0 * H) + y(x - 3.0 * H)) / (2.0 * H);
        let d2 = (y(x - H) - 2.0 * y(x - 2.0 * H) + y(x - 3.0 * H)) / (H * H);
        d + d2 * H
    } else {
        0.0
    };
    if s.is_finite() { s } else { 0.0 }
}

/// The ease in of `f` (its In Out form with `in_out`) as Bezier points: Hermite segments (value
/// and slope matched at each end, so exact for cubic polynomials and for each parabola of
/// Bounce), except Circ, whose quarter circle is two 45° arcs (within 4·10⁻⁶ of the circle).
fn fit_in(f: EaseFamily, in_out: bool) -> Vec<EasePoint> {
    let pt = |x: f64, y: f64, i: (f64, f64), o: (f64, f64)| EasePoint {
        at: Vec2::new(x, y),
        h_in: Vec2::new(i.0, i.1),
        out: Vec2::new(o.0, o.1),
        smooth: false,
    };
    if f == EaseFamily::Circ {
        // The circle about (0, 1) from (0, 0) to (1, 1); each arc's handles are k = 4/3·tan(φ/4)
        // long along the tangents.
        let k = 4.0 / 3.0 * (std::f64::consts::PI / 16.0).tan();
        let c = std::f64::consts::FRAC_1_SQRT_2;
        return vec![pt(0.0, 0.0, (0.0, 0.0), (k, 0.0)), pt(c, 1.0 - c, (-k * c, -k * c), (k * c, k * c)), pt(1.0, 1.0, (0.0, -k), (0.0, 0.0))];
    }
    let y = move |t: f64| f.ease_in(t, in_out);
    let mut xs: Vec<f64> = Vec::new();
    let pieces = f.pieces_in();
    let bounds: Vec<f64> = std::iter::once(0.0).chain(f.breaks_in().iter().copied()).chain(std::iter::once(1.0)).collect();
    for w in bounds.windows(2) {
        if let [a, c] = w {
            xs.extend((0..pieces).map(|k| a + (c - a) * k as f64 / pieces as f64));
        }
    }
    xs.push(1.0);
    let mut pts: Vec<EasePoint> = xs.iter().map(|&x| pt(x, y(x), (0.0, 0.0), (0.0, 0.0))).collect();
    for i in 0..xs.len().saturating_sub(1) {
        let (Some(&a), Some(&b)) = (xs.get(i), xs.get(i + 1)) else { continue };
        let h = (b - a) / 3.0;
        let (sa, sb) = (slope(&y, a, a, b), slope(&y, b, a, b));
        if let Some(p) = pts.get_mut(i) {
            p.out = Vec2::new(h, sa * h);
        }
        if let Some(p) = pts.get_mut(i + 1) {
            p.h_in = Vec2::new(-h, -sb * h);
        }
    }
    pts
}

/// The curve turned half a turn about (½, ½): an ease in becomes the ease out.
fn mirrored(pts: &[EasePoint]) -> Vec<EasePoint> {
    pts.iter().rev().map(|p| EasePoint { at: Vec2::new(1.0 - p.at.x, 1.0 - p.at.y), h_in: -p.out, out: -p.h_in, smooth: p.smooth }).collect()
}

fn scaled(pts: &[EasePoint], by: f64, from: Vec2) -> Vec<EasePoint> {
    pts.iter().map(|p| EasePoint { at: from + p.at * by, h_in: p.h_in * by, out: p.out * by, smooth: p.smooth }).collect()
}

/// A named ease as a custom curve (see [`Easing::to_custom`]).
fn fit_named(f: EaseFamily, d: EaseDir) -> CustomEase {
    let mut pts = match d {
        EaseDir::In => fit_in(f, false),
        EaseDir::Out => mirrored(&fit_in(f, false)),
        EaseDir::InOut => {
            let half = fit_in(f, true);
            let mut a = scaled(&half, 0.5, Vec2::ZERO);
            let b = scaled(&mirrored(&half), 0.5, Vec2::new(0.5, 0.5));
            // The two halves meet at (½, ½): one point, the first half's in, the second's out.
            if let (Some(last), Some(first)) = (a.last_mut(), b.first()) {
                last.out = first.out;
            }
            a.extend(b.into_iter().skip(1));
            a
        }
    };
    // Points where the slope carries on are smooth (Bounce's floor hits are corners).
    let n = pts.len();
    for (i, p) in pts.iter_mut().enumerate() {
        p.smooth =
            i > 0 && i + 1 < n && p.h_in.length() > 1e-9 && p.out.length() > 1e-9 && (p.h_in.normalized().dot(p.out.normalized()) + 1.0).abs() < 1e-3;
    }
    CustomEase::new(pts)
}
