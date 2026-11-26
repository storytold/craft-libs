//! Diffuse irradiance as nine spherical-harmonics coefficients.

use std::f64::consts::{PI, TAU};

use crate::environment::EnvImage;
use crate::vec3::{V3, unit};
use crate::{Limits, PbrError};

/// Diffuse irradiance of `env` as nine SH coefficients, in the order of [`sh_basis`], already
/// convolved with the cosine lobe and divided by `π`: `Σ c_k Y_k(n)` times the albedo is the
/// diffuse radiance a surface with normal `n` reflects. Every pixel of the source is projected,
/// with each basis function integrated over the area of the pixel. Returns an error when the
/// source is over `limits` or its table of column integrals, one per column, is over
/// [`Limits::max_bytes`].
pub fn try_sh9(env: &EnvImage, limits: &Limits) -> Result<[[f32; 3]; 9], PbrError> {
    let (w, h) = (env.width(), env.height());
    limits.check(w, h)?;
    limits.check_total_bytes(u64::from(w).checked_mul(size_of::<Column>() as u64), w, h)?;
    project(env)
}

/// Integrals over the longitudes `φ0..φ1` of one column.
struct Column {
    /// `∫ dφ`
    len: f64,
    /// `∫ cos φ dφ`
    cos: f64,
    /// `∫ sin φ dφ`
    sin: f64,
    /// `∫ cos² φ dφ`
    cos2: f64,
    /// `∫ sin² φ dφ`
    sin2: f64,
    /// `∫ sin φ cos φ dφ`
    sin_cos: f64,
}

impl Column {
    fn new(x: usize, w: usize) -> Column {
        let (p0, p1) = (TAU * x as f64 / w as f64, TAU * (x + 1) as f64 / w as f64);
        let len = p1 - p0;
        let half_sin2 = ((2.0 * p1).sin() - (2.0 * p0).sin()) / 4.0;
        Column {
            len,
            cos: p1.sin() - p0.sin(),
            sin: p0.cos() - p1.cos(),
            cos2: len / 2.0 + half_sin2,
            sin2: len / 2.0 - half_sin2,
            sin_cos: (p1.sin().powi(2) - p0.sin().powi(2)) / 2.0,
        }
    }
}

/// Integrals over the colatitudes `θ0..θ1` of one row, each with the `sin θ` of the solid angle.
struct Row {
    /// `∫ sin θ dθ`
    s1: f64,
    /// `∫ cos θ sin θ dθ`
    c1s1: f64,
    /// `∫ sin² θ dθ`
    s2: f64,
    /// `∫ cos θ sin² θ dθ`
    c1s2: f64,
    /// `∫ sin³ θ dθ`
    s3: f64,
    /// `∫ cos² θ sin θ dθ`
    c2s1: f64,
}

impl Row {
    fn new(y: usize, h: usize) -> Row {
        let (t0, t1) = (PI * y as f64 / h as f64, PI * (y + 1) as f64 / h as f64);
        let (c0, c1, s0, s1) = (t0.cos(), t1.cos(), t0.sin(), t1.sin());
        Row {
            s1: c0 - c1,
            c1s1: (s1 * s1 - s0 * s0) / 2.0,
            s2: (t1 - t0) / 2.0 - ((2.0 * t1).sin() - (2.0 * t0).sin()) / 4.0,
            c1s2: (s1.powi(3) - s0.powi(3)) / 3.0,
            s3: (c0 - c1) - (c0.powi(3) - c1.powi(3)) / 3.0,
            c2s1: (c0.powi(3) - c1.powi(3)) / 3.0,
        }
    }
}

/// The integral of each function of [`basis`] over one pixel, from the column and row integrals
/// of `x = sin θ cos φ`, `y = cos θ` and `z = sin θ sin φ`.
fn pixel_basis(c: &Column, r: &Row) -> [f64; 9] {
    [
        0.282_095 * c.len * r.s1,
        0.488_603 * c.len * r.c1s1,
        0.488_603 * c.sin * r.s2,
        0.488_603 * c.cos * r.s2,
        1.092_548 * c.cos * r.c1s2,
        1.092_548 * c.sin * r.c1s2,
        0.315_392 * (3.0 * c.sin2 * r.s3 - c.len * r.s1),
        1.092_548 * c.sin_cos * r.s3,
        0.546_274 * (c.cos2 * r.s3 - c.len * r.c2s1),
    ]
}

/// Projects each pixel with the integral of the basis over its area rather than the basis at its
/// center, so a coarse image of uniform radiance has no directional terms. Sums in `f64`, so
/// the sums cannot overflow before the result is clamped to `f32`.
fn project(src: &EnvImage) -> Result<[[f32; 3]; 9], PbrError> {
    let (w, h) = (src.width() as usize, src.height() as usize);
    let mut columns = Vec::new();
    columns.try_reserve_exact(w).map_err(|_| PbrError::OutOfMemory)?;
    columns.extend((0..w).map(|x| Column::new(x, w)));
    let mut c = [[0.0f64; 3]; 9];
    for (y, pixels) in src.pixels().chunks_exact(w).enumerate() {
        let row = Row::new(y, h);
        for (col, p) in columns.iter().zip(pixels) {
            let l = p.map(f64::from);
            for (ck, ylm) in c.iter_mut().zip(pixel_basis(col, &row)) {
                for (cc, lc) in ck.iter_mut().zip(l) {
                    *cc += lc * ylm;
                }
            }
        }
    }
    let a = [1.0, 2.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, 0.25, 0.25, 0.25, 0.25, 0.25];
    let max = f64::from(f32::MAX);
    let mut out = [[0.0; 3]; 9];
    for ((o, ck), ak) in out.iter_mut().zip(c).zip(a) {
        *o = ck.map(|v| (v * ak).clamp(-max, max) as f32);
    }
    Ok(out)
}

/// The nine real SH basis functions at direction `d`, bands 0 to 2 in the order
/// `Y00, Y1-1 (y), Y10 (z), Y11 (x), Y2-2 (xy), Y2-1 (yz), Y20, Y21 (xz), Y22`. `d` is normalized
/// first. Returns [`PbrError::BadDirection`] when it is not finite or has zero length.
pub fn sh_basis(d: [f32; 3]) -> Result<[f32; 9], PbrError> {
    unit(d).map(basis).ok_or(PbrError::BadDirection)
}

fn basis(d: V3) -> [f32; 9] {
    let (x, y, z) = (d.x, d.y, d.z);
    [
        0.282_095,
        0.488_603 * y,
        0.488_603 * z,
        0.488_603 * x,
        1.092_548 * x * y,
        1.092_548 * y * z,
        0.315_392 * (3.0 * z * z - 1.0),
        1.092_548 * x * z,
        0.546_274 * (x * x - y * y),
    ]
}

/// `Σ c_k Y_k(n)` for coefficients from [`try_sh9`]: the diffuse radiance a white surface with
/// normal `n` reflects. Negative results, which ringing can give, are kept. Returns
/// [`PbrError::BadDirection`] when `n` is not finite or has zero length, and
/// [`PbrError::BadOption`] when a coefficient is not finite.
pub fn irradiance(coefficients: &[[f32; 3]; 9], n: [f32; 3]) -> Result<[f32; 3], PbrError> {
    let b = sh_basis(n)?;
    if coefficients.iter().flatten().any(|c| !c.is_finite()) {
        return Err(PbrError::BadOption("an SH coefficient is not finite"));
    }
    let mut e = [0.0f64; 3];
    for (c, y) in coefficients.iter().zip(b) {
        for (ec, cc) in e.iter_mut().zip(c) {
            *ec += f64::from(*cc) * f64::from(y);
        }
    }
    let max = f64::from(f32::MAX);
    Ok(e.map(|v| v.clamp(-max, max) as f32))
}
