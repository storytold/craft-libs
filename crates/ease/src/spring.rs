//! Damped springs: [`Spring`] and [`SpringState`].
//!
//! A spring pulls a value `x` toward a target with
//!
//! ```text
//! x'' = −ω² (x − target) − 2ζω x'
//! ```
//!
//! where `ω` is the natural angular frequency and `ζ` the damping ratio (below 1 the value
//! overshoots and swings back, at 1 it arrives as fast as it can without overshooting, above 1
//! it creeps in). [`SpringState::step`] moves the value by the exact solution of that equation
//! over the step, so a step costs O(1), any step length is stable, and many short steps give the
//! same value as one long one.

use crate::{EaseError, within};

/// Highest natural frequency a spring may have, in swings per second.
pub const MAX_SPRING_HZ: f64 = 10_000.0;
/// Highest damping ratio a spring may have.
pub const MAX_DAMPING_RATIO: f64 = 1e6;
/// The ranges [`Spring::from_physical`] accepts: mass, stiffness and damping coefficient.
const MASS: (f64, f64) = (0.01, 1e4);
const STIFFNESS: (f64, f64) = (0.01, 1e6);
const DAMPING: (f64, f64) = (0.0, 1e6);
/// Longest motion a spring ease plays, in seconds of the spring's own time.
const MAX_SETTLE: f64 = 10.0;

/// A damped spring's constants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    omega: f64,
    zeta: f64,
}

impl Spring {
    /// A spring that would swing `frequency` times a second without damping, with damping ratio
    /// `damping`. `frequency` must be above 0 and at most [`MAX_SPRING_HZ`], `damping` within 0
    /// to [`MAX_DAMPING_RATIO`].
    pub fn new(frequency: f64, damping: f64) -> Result<Spring, EaseError> {
        let f = within(frequency, 0.0, MAX_SPRING_HZ, "frequency")?;
        if f <= 0.0 {
            return Err(EaseError::OutOfRange { what: "frequency", min: f64::MIN_POSITIVE, max: MAX_SPRING_HZ });
        }
        let z = within(damping, 0.0, MAX_DAMPING_RATIO, "damping")?;
        Ok(Spring::raw(f, z))
    }

    /// A spring from two `0..=1` controls: `strength` sets the frequency from 0.5 to 10 swings a
    /// second, evenly spread in pitch, and `damping` the ratio (0 swings on, 2/3 is critical,
    /// 1 gives [`MAX_DAMPING_RATIO`]).
    pub fn from_controls(strength: f64, damping: f64) -> Result<Spring, EaseError> {
        let s = within(strength, 0.0, 1.0, "strength")?;
        let d = within(damping, 0.0, 1.0, "damping")?;
        let zeta = if d >= 1.0 { MAX_DAMPING_RATIO } else { (0.5 * d / (1.0 - d)).min(MAX_DAMPING_RATIO) };
        Ok(Spring::raw(0.5 * 20f64.powf(s), zeta))
    }

    /// A mass on a spring with damping: `mass` within 0.01 to 10,000, `stiffness` within 0.01
    /// to 1,000,000 and the damping coefficient `damping` within 0 to 1,000,000. An error when
    /// the three give a damping ratio above [`MAX_DAMPING_RATIO`].
    pub fn from_physical(mass: f64, stiffness: f64, damping: f64) -> Result<Spring, EaseError> {
        let m = within(mass, MASS.0, MASS.1, "mass")?;
        let k = within(stiffness, STIFFNESS.0, STIFFNESS.1, "stiffness")?;
        let c = within(damping, DAMPING.0, DAMPING.1, "damping")?;
        if c / (2.0 * (k * m).sqrt()) > MAX_DAMPING_RATIO {
            return Err(EaseError::OutOfRange { what: "damping ratio", min: 0.0, max: MAX_DAMPING_RATIO });
        }
        Ok(Spring::physical(m, k, c))
    }

    /// [`Spring::from_physical`] with each value brought into its range (a value that is not
    /// finite takes the default: mass 1, stiffness 100, damping 15).
    pub(crate) fn physical_lenient(mass: f64, stiffness: f64, damping: f64) -> Spring {
        let ok = |v: f64, (lo, hi): (f64, f64), d: f64| if v.is_finite() { v.clamp(lo, hi) } else { d };
        Spring::physical(ok(mass, MASS, 1.0), ok(stiffness, STIFFNESS, 100.0), ok(damping, DAMPING, 15.0))
    }

    fn physical(m: f64, k: f64, c: f64) -> Spring {
        Spring { omega: (k / m).sqrt(), zeta: (c / (2.0 * (k * m).sqrt())).min(MAX_DAMPING_RATIO) }
    }

    /// A spring from values already in range.
    pub(crate) fn raw(frequency: f64, damping: f64) -> Spring {
        Spring { omega: std::f64::consts::TAU * frequency, zeta: damping }
    }

    /// Seconds until a spring released from rest stays within about 0.1 % of its target, at
    /// least 0.05 and at most 10.
    pub fn settle_time(&self) -> f64 {
        let (w, z) = (self.omega, self.zeta);
        // The slowest mode's decay rate; e^−6.9 is 0.1 %, and a critically or over-damped
        // spring's larger lead factor needs about 9.2. Released from rest, an under-damped
        // spring's offset stays within e^−ζωt · min(1/√(1 − ζ²), 1 + ζωt), so its lead factor
        // grows by ln(1/√(1 − ζ²)) up to the critical spring's 9.2.
        let (rate, k) = if z < 1.0 { (z * w, (6.9 - 0.5 * (1.0 - z * z).ln()).min(9.2)) } else { (w / (z + (z * z - 1.0).sqrt()), 9.2) };
        let settle = if rate > 1e-6 { (k / rate).clamp(0.05, MAX_SETTLE) } else { MAX_SETTLE };
        if settle.is_finite() { settle } else { MAX_SETTLE }
    }

    /// The spring as an ease: released from rest at 0 toward 1, `t` of the way through its
    /// [`Spring::settle_time`]. Exactly 0 at `t <= 0` and exactly 1 at `t >= 1`. A spring with
    /// so little damping that it is still moving after 10 seconds has the distance it is still
    /// off at that point taken out evenly over the ease, so it reaches 1 without a jump.
    pub fn ease(&self, t: f64) -> f64 {
        if t.is_nan() || t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        let settle = self.settle_time();
        let mut x = 1.0 + self.response(-1.0, 0.0, t * settle).0;
        if settle >= MAX_SETTLE {
            x -= t * self.response(-1.0, 0.0, settle).0;
        }
        if x.is_finite() { x } else { 1.0 }
    }

    /// The natural angular frequency `ω`, in radians a second.
    pub fn omega(&self) -> f64 {
        self.omega
    }

    /// The damping ratio `ζ`.
    pub fn damping(&self) -> f64 {
        self.zeta
    }

    /// Offset and velocity `(y, v)` after `t` seconds, from offset `y0` (value minus target) and
    /// velocity `v0`: the exact solution of the spring equation.
    pub(crate) fn response(&self, y0: f64, v0: f64, t: f64) -> (f64, f64) {
        let (w, z) = (self.omega, self.zeta);
        if t <= 0.0 {
            return (y0, v0);
        }
        // Past this much decay e^(−decay) is below the smallest f64: the spring has settled.
        const SETTLED: f64 = 800.0;
        // Damping ratios near 1 use the under- and over-damped solutions below, which stay
        // precise there, so every ratio is solved as given; only exactly 1 is critical.
        if z == 1.0 {
            if w * t > SETTLED {
                return (0.0, 0.0);
            }
            let b = v0 + w * y0;
            let e = (-w * t).exp();
            // The velocity is e^−ωt (v0 − ω b t), written so v0 is not recovered by subtracting
            // the rounded position terms.
            return (e * (y0 + b * t), e * (v0 - w * (b * t)));
        }
        if z < 1.0 {
            if z * w * t > SETTLED {
                return (0.0, 0.0);
            }
            // 1 − ζ² as (1 − ζ)(1 + ζ), precise for ζ near 1.
            let wd = w * ((1.0 - z) * (1.0 + z)).sqrt();
            let q = v0 + z * w * y0;
            let e = (-z * w * t).exp();
            // A step so long that the phase overflows (a spring with little or no damping) is
            // reduced by whole periods first.
            let reduced = !(wd * t).is_finite();
            let phase = if reduced { wd * (t % (std::f64::consts::TAU / wd)) } else { wd * t };
            let (s, c) = phase.sin_cos();
            // sin(phase) / ωd is at most t, so it stays finite for a spring of any low frequency,
            // where q / ωd alone would overflow. A small unreduced phase uses t · sin(phase) / phase
            // from its series, which is t when ωd or the phase rounds to 0.
            let sw = if !reduced && phase < 1e-4 {
                let p2 = phase * phase;
                t * (1.0 - p2 / 6.0 * (1.0 - p2 / 20.0))
            } else {
                s / wd
            };
            return (e * (y0 * c + q * sw), e * (v0 * c - (y0 * wd * s + z * w * q * sw)));
        }
        // ζ² − 1 as (ζ − 1)(ζ + 1), precise for ζ near 1.
        let root = ((z - 1.0) * (z + 1.0)).sqrt();
        // r1 = −ω(ζ − √(ζ² − 1)), written without the cancellation for large ζ.
        let (r1, r2) = (-w / (z + root), -w * (z + root));
        if -r1 * t > SETTLED {
            return (0.0, 0.0);
        }
        // Written with the divided difference q = (e1 − e2) / (r1 − r2) rather than the two modes'
        // coefficients, which overflow (in opposite signs) when r1 − r2 is tiny.
        let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
        let x = (r1 - r2) * t;
        let q = if x <= 1.0 {
            // e2 · t · (e^x − 1) / x, with its series where x is too small for expm1's quotient.
            let g = if x < 1e-5 { 1.0 + x * (0.5 + x / 6.0) } else { x.exp_m1() / x };
            e2 * t * g
        } else {
            (e1 - e2) / (r1 - r2)
        };
        (y0 * (e1 - r1 * q) + v0 * q, v0 * (e2 + r1 * q) - y0 * (w * w) * q)
    }
}

/// A value moved by a spring: where it is and how fast it moves.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpringState {
    pub value: f64,
    pub velocity: f64,
}

impl SpringState {
    /// A value at rest at `value`.
    pub fn at(value: f64) -> SpringState {
        SpringState { value, velocity: 0.0 }
    }

    /// Moves the value `dt` seconds toward `target` (which holds still over the step) and
    /// returns the new value. An error, with the state unchanged, for a `target` or `dt` that is
    /// not finite, a negative `dt`, or a result too large to represent. A state whose value or
    /// velocity is not finite is refused with [`EaseError::Overflow`] at every step length, and
    /// stays as it is.
    pub fn step(&mut self, spring: &Spring, target: f64, dt: f64) -> Result<f64, EaseError> {
        let target = crate::finite(target, "target")?;
        let dt = within(dt, 0.0, f64::MAX, "step")?;
        if !(self.value.is_finite() && self.velocity.is_finite()) {
            return Err(EaseError::Overflow);
        }
        if dt == 0.0 {
            return Ok(self.value);
        }
        let (y, v) = spring.response(self.value - target, self.velocity, dt);
        let (mut value, mut v) = (target + y, v);
        if !(value.is_finite() && v.is_finite()) {
            // The motion is linear in the offset and velocity, so they are moved again scaled by
            // a power of two to at most 2, for a value and target near opposite ends of the f64
            // range whose step overflowed on the way.
            let (hy, hv) = (0.5 * self.value - 0.5 * target, 0.5 * self.velocity);
            let m = hy.abs().max(hv.abs());
            let scale = if m > 0.0 && m.is_finite() { 2f64.powi((m.log2().ceil() as i32).min(1023)) } else { 1.0 };
            let (y, w) = spring.response(hy / scale, hv / scale, dt);
            (value, v) = (2.0 * (0.5 * target + y * scale), 2.0 * (w * scale));
        }
        if !(value.is_finite() && v.is_finite()) {
            return Err(EaseError::Overflow);
        }
        *self = SpringState { value, velocity: v };
        Ok(value)
    }

    /// Whether the value is within `tolerance` of `target` and moves slower than `tolerance`
    /// a second.
    pub fn settled(&self, target: f64, tolerance: f64) -> bool {
        (self.value - target).abs() <= tolerance && self.velocity.abs() <= tolerance
    }
}
