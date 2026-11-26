// SPDX-License-Identifier: MIT OR Apache-2.0
//! The transcendental functions the engine uses (`sin`, `cos`, `exp`, `ln`, `powf`, `atan2` ...), from one pure-Rust
//! implementation (the `libm` crate) for every target, so a graph renders to the same bits on any operating system,
//! C library, compiler version and on the web.
//!
//! `f32::sin` and the like call the platform's maths library (glibc, the macOS or Windows one; none on wasm), and
//! those round some results differently by an ulp, which changes render hashes from one machine to the next. The
//! operations IEEE 754 defines exactly (`+ - * /`, `sqrt`, `floor`, `round`, `abs`, `%` ...) give the same bits
//! everywhere and stay as they are. `clippy.toml` in this crate forbids the platform versions.
//!
//! Each function works on `f32` and `f64` alike (the single-precision functions for `f32`).

/// The `f32` and `f64` versions of each function.
pub trait Real: Copy {
    fn sin_(self) -> Self;
    fn cos_(self) -> Self;
    fn tan_(self) -> Self;
    fn asin_(self) -> Self;
    fn acos_(self) -> Self;
    fn atan_(self) -> Self;
    fn atan2_(self, x: Self) -> Self;
    fn exp_(self) -> Self;
    fn ln_(self) -> Self;
    fn log2_(self) -> Self;
    fn powf_(self, e: Self) -> Self;
    fn cbrt_(self) -> Self;
    fn hypot_(self, y: Self) -> Self;
    fn one() -> Self;
    fn mul(self, b: Self) -> Self;
    fn div(self, b: Self) -> Self;
}

macro_rules! real {
    ($t:ty, $sin:ident, $cos:ident, $tan:ident, $asin:ident, $acos:ident, $atan:ident, $atan2:ident, $exp:ident, $ln:ident, $log2:ident, $pow:ident, $cbrt:ident, $hypot:ident) => {
        impl Real for $t {
            #[inline]
            fn sin_(self) -> Self {
                libm::$sin(self)
            }
            #[inline]
            fn cos_(self) -> Self {
                libm::$cos(self)
            }
            #[inline]
            fn tan_(self) -> Self {
                libm::$tan(self)
            }
            #[inline]
            fn asin_(self) -> Self {
                libm::$asin(self)
            }
            #[inline]
            fn acos_(self) -> Self {
                libm::$acos(self)
            }
            #[inline]
            fn atan_(self) -> Self {
                libm::$atan(self)
            }
            #[inline]
            fn atan2_(self, x: Self) -> Self {
                libm::$atan2(self, x)
            }
            #[inline]
            fn exp_(self) -> Self {
                libm::$exp(self)
            }
            #[inline]
            fn ln_(self) -> Self {
                libm::$ln(self)
            }
            #[inline]
            fn log2_(self) -> Self {
                libm::$log2(self)
            }
            #[inline]
            fn powf_(self, e: Self) -> Self {
                libm::$pow(self, e)
            }
            #[inline]
            fn cbrt_(self) -> Self {
                libm::$cbrt(self)
            }
            #[inline]
            fn hypot_(self, y: Self) -> Self {
                libm::$hypot(self, y)
            }
            #[inline]
            fn one() -> Self {
                1.0
            }
            #[inline]
            fn mul(self, b: Self) -> Self {
                self * b
            }
            #[inline]
            fn div(self, b: Self) -> Self {
                self / b
            }
        }
    };
}

real!(f32, sinf, cosf, tanf, asinf, acosf, atanf, atan2f, expf, logf, log2f, powf, cbrtf, hypotf);
real!(f64, sin, cos, tan, asin, acos, atan, atan2, exp, log, log2, pow, cbrt, hypot);

#[inline]
pub fn sin<T: Real>(x: T) -> T {
    x.sin_()
}

#[inline]
pub fn cos<T: Real>(x: T) -> T {
    x.cos_()
}

/// `(sin x, cos x)`, as `f32::sin_cos` gives them (each on its own).
#[inline]
pub fn sin_cos<T: Real>(x: T) -> (T, T) {
    (x.sin_(), x.cos_())
}

#[inline]
pub fn tan<T: Real>(x: T) -> T {
    x.tan_()
}

#[inline]
pub fn asin<T: Real>(x: T) -> T {
    x.asin_()
}

#[inline]
pub fn acos<T: Real>(x: T) -> T {
    x.acos_()
}

#[inline]
pub fn atan<T: Real>(x: T) -> T {
    x.atan_()
}

/// The angle of `(x, y)`, as `y.atan2(x)`.
#[inline]
pub fn atan2<T: Real>(y: T, x: T) -> T {
    y.atan2_(x)
}

#[inline]
pub fn exp<T: Real>(x: T) -> T {
    x.exp_()
}

/// The natural logarithm.
#[inline]
pub fn ln<T: Real>(x: T) -> T {
    x.ln_()
}

#[inline]
pub fn log2<T: Real>(x: T) -> T {
    x.log2_()
}

#[inline]
pub fn powf<T: Real>(x: T, e: T) -> T {
    x.powf_(e)
}

#[inline]
pub fn cbrt<T: Real>(x: T) -> T {
    x.cbrt_()
}

#[inline]
pub fn hypot<T: Real>(x: T, y: T) -> T {
    x.hypot_(y)
}

/// `x` to the integer power `n` by repeated squaring, the same multiplications in the same order as the compiler's
/// own `powi` (`__powisf2`/`__powidf2`, which constant exponents also expand to), but fixed here: LLVM leaves the order
/// of `powi`'s multiplications open.
#[inline]
pub fn powi<T: Real>(x: T, n: i32) -> T {
    let mut a = x;
    let mut b = n;
    let mut r = T::one();
    loop {
        if b & 1 != 0 {
            r = r.mul(a);
        }
        b /= 2;
        if b == 0 {
            break;
        }
        a = a.mul(a);
    }
    if n < 0 { T::one().div(r) } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<f64> {
        let mut v = vec![0.0, -0.0, 0.5, -0.5, 1.0, -1.0, 2.0, 1e-3, 3.25, -7.5, 1e3, f64::INFINITY, f64::NEG_INFINITY, f64::NAN];
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..20_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            v.push((x >> 11) as f64 / (1u64 << 53) as f64 * 40.0 - 20.0);
        }
        v
    }

    /// The fixed `powi` gives what the compiler's gives (on the machine the tests run on), constant exponents included.
    /// Windows' MSVC toolchain multiplies negative powers in another order (0.001^-12 lands 2 ulp away), so the
    /// comparison only holds where `powi` is compiler-rt's `__powidf2`.
    #[test]
    #[cfg_attr(target_env = "msvc", ignore = "the MSVC toolchain's powi is not compiler-rt's")]
    #[allow(clippy::disallowed_methods)]
    fn powi_matches_the_compilers() {
        for x in samples() {
            let xf = x as f32;
            for n in -12..=24 {
                assert_eq!(powi(x, n).to_bits(), x.powi(n).to_bits(), "{x}^{n}");
                assert_eq!(powi(xf, n).to_bits(), xf.powi(n).to_bits(), "{xf}^{n} (f32)");
            }
            assert_eq!(powi(x, 2).to_bits(), (x * x).to_bits());
            assert_eq!(powi(xf, 3).to_bits(), xf.powi(3).to_bits());
            assert_eq!(powi(xf, 4).to_bits(), xf.powi(4).to_bits());
        }
    }

    /// Within an ulp or two of the platform's functions: the same functions, rounded the way one implementation does.
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn close_to_the_platform_functions() {
        let ulps32 = |a: f32, b: f32| if a.is_nan() && b.is_nan() { 0 } else { (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs() };
        for x in samples() {
            let f = x as f32;
            let g = (f * 0.05).clamp(-1.0, 1.0);
            for (name, ours, theirs) in [
                ("sin", sin(f), f.sin()),
                ("cos", cos(f), f.cos()),
                ("tan", tan(f * 0.07), (f * 0.07).tan()),
                ("asin", asin(g), g.asin()),
                ("acos", acos(g), g.acos()),
                ("atan", atan(f), f.atan()),
                ("atan2", atan2(f, 0.75), f.atan2(0.75)),
                ("exp", exp(f), f.exp()),
                ("ln", ln(f.abs()), f.abs().ln()),
                ("log2", log2(f.abs()), f.abs().log2()),
                ("powf", powf(f.abs(), 2.2), f.abs().powf(2.2)),
                ("cbrt", cbrt(f), f.cbrt()),
                ("hypot", hypot(f, 3.0), f.hypot(3.0)),
            ] {
                assert!(ulps32(ours, theirs) <= 2, "{name}({f}): {ours} vs {theirs}");
            }
            if !x.is_finite() {
                continue;
            }
            assert!((sin(x) - x.sin()).abs() <= 2.0 * f64::EPSILON * x.sin().abs().max(1e-300), "sin({x})");
            assert!((powf(x.abs(), 1.7) - x.abs().powf(1.7)).abs() <= 4.0 * f64::EPSILON * x.abs().powf(1.7), "pow({x})");
        }
    }
}
