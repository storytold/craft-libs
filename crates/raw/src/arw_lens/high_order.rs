//! Bounded degree-14 fallback for Sony curves that do not fit the four-term DNG warp.
//!
//! Fits decoded table mappings. Chebyshev coordinates avoid fitting an
//! ill-conditioned monomial system. The emitted DNG polynomial is separately checked in f64;
//! the existing WarpRectilinear2 pipeline already requires f64 rendering for these coefficients.
use super::{Opcode, Rect};

const TERMS: usize = 15;
const FIT_SAMPLES: usize = 512;
const CHECK_SAMPLES: usize = 32768;

pub(super) fn fit_map(active: Rect, crop: Rect, knots: impl Iterator<Item = f64>, source: impl Fn(f64) -> f64, tolerance_px: f64) -> Option<Opcode> {
    if active.width < 2 || active.height < 2 || crop.width < 2 || crop.height < 2 || crop.clipped(active.width, active.height) != crop {
        return None;
    }
    let (cw, ch) = ((crop.width - 1) as f64, (crop.height - 1) as f64);
    let radius = cw.hypot(ch) / 2.0;
    // Include every knot explicitly: a dense uniform grid alone can miss a sharp interpolation corner.
    let knots: Vec<_> = knots.collect();
    let mut radii: Vec<f64> = (0..=FIT_SAMPLES).map(|i| i as f64 / FIT_SAMPLES as f64).collect();
    for &r in &knots {
        if !radii.iter().any(|&v| (r - v).abs() < 1e-12) {
            radii.push(r);
        }
    }
    let rows: Vec<([f64; TERMS], f64)> = radii.iter().map(|&r| (basis(r).map(|v| v * r), source(r))).collect();
    let mut weights = vec![1.0; rows.len()];
    let mut coefficients = [0.0; TERMS];
    // Bounded Lawson iterations reduce peak positional error. They are an approximation method,
    // not a reason to accept a curve: dense post-conversion accuracy/fold checks remain decisive.
    for _ in 0..25 {
        let mut matrix = [[0.0; TERMS]; TERMS];
        let mut rhs = [0.0; TERMS];
        for ((terms, target), &weight) in rows.iter().zip(&weights) {
            for i in 0..TERMS {
                rhs[i] += weight * terms[i] * target;
                for j in 0..TERMS {
                    matrix[i][j] += weight * terms[i] * terms[j];
                }
            }
        }
        coefficients = solve(matrix, rhs)?;
        for ((terms, target), weight) in rows.iter().zip(&mut weights) {
            let predicted = terms.iter().zip(coefficients).map(|(a, b)| a * b).sum::<f64>();
            *weight *= (predicted - target).abs().max(1e-15).sqrt();
        }
        let largest = weights.iter().copied().fold(0.0, f64::max);
        if !largest.is_finite() || largest <= 0.0 {
            return None;
        }
        for weight in &mut weights {
            *weight = (*weight / largest).max(1e-12);
        }
    }
    let mut k = monomials(coefficients);
    let mut previous = -1.0;
    for i in 0..=CHECK_SAMPLES {
        let r = i as f64 / CHECK_SAMPLES as f64;
        let target = source(r);
        let fitted = r * polynomial(&k, r);
        let derivative = k.iter().enumerate().rev().fold(0.0, |v, (j, &c)| v * r + c * (j + 1) as f64);
        if target <= previous
            || !fitted.is_finite()
            || derivative <= 0.0
            || (fitted - target).abs() * radius * 1440.0 / crop.width as f64 > tolerance_px
        {
            return None;
        }
        previous = target;
    }
    if knots.iter().any(|&r| (r * polynomial(&k, r) - source(r)).abs() * radius * 1440.0 / crop.width as f64 > tolerance_px) {
        return None;
    }
    let (ax, ay) = ((active.width - 1) as f64, (active.height - 1) as f64);
    let (cx, cy) = (crop.x as f64 + cw / 2.0, crop.y as f64 + ch / 2.0);
    let ratio = cx.max(ax - cx).hypot(cy.max(ay - cy)) / radius;
    for (i, coefficient) in k.iter_mut().enumerate() {
        *coefficient *= ratio.powi(i as i32);
    }
    if k.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
        return None;
    }
    let mut plane = [0.0; crate::opcodes::WR2_LEN];
    plane[..TERMS].copy_from_slice(&k);
    plane[18] = 1.0 / ratio;
    Some(Opcode::WarpRectilinear2 { planes: vec![plane], center: [cx / ax, cy / ay], reciprocal: false, optional: false })
}

fn basis(r: f64) -> [f64; TERMS] {
    let x = 2.0 * r - 1.0;
    let mut terms = [0.0; TERMS];
    terms[0] = 1.0;
    terms[1] = x;
    for i in 2..TERMS {
        terms[i] = 2.0 * x * terms[i - 1] - terms[i - 2];
    }
    terms
}

fn polynomial(k: &[f64; TERMS], r: f64) -> f64 {
    k.iter().rev().fold(0.0, |v, &c| v * r + c)
}

fn monomials(coefficients: [f64; TERMS]) -> [f64; TERMS] {
    let mut previous = [0.0; TERMS];
    previous[0] = 1.0;
    let mut current = [0.0; TERMS];
    current[0] = -1.0;
    current[1] = 2.0;
    let mut output = [0.0; TERMS];
    output[0] = coefficients[0] - coefficients[1];
    output[1] = 2.0 * coefficients[1];
    for &coefficient in &coefficients[2..] {
        let mut next = [0.0; TERMS];
        for j in 0..TERMS {
            next[j] = -2.0 * current[j] - previous[j] + if j > 0 { 4.0 * current[j - 1] } else { 0.0 };
            output[j] += coefficient * next[j];
        }
        previous = current;
        current = next;
    }
    output
}

fn solve(mut matrix: [[f64; TERMS]; TERMS], mut rhs: [f64; TERMS]) -> Option<[f64; TERMS]> {
    for col in 0..TERMS {
        let pivot = (col..TERMS).max_by(|&a, &b| matrix[a][col].abs().total_cmp(&matrix[b][col].abs()))?;
        if !matrix[pivot][col].is_finite() || matrix[pivot][col].abs() < 1e-12 {
            return None;
        }
        matrix.swap(col, pivot);
        rhs.swap(col, pivot);
        let pivot_row = matrix[col];
        for row in col + 1..TERMS {
            let factor = matrix[row][col] / pivot_row[col];
            for (value, pivot_value) in matrix[row][col..].iter_mut().zip(&pivot_row[col..]) {
                *value -= factor * pivot_value;
            }
            rhs[row] -= factor * rhs[col];
        }
    }
    let mut answer = [0.0; TERMS];
    for i in (0..TERMS).rev() {
        answer[i] = (rhs[i] - (i + 1..TERMS).map(|j| matrix[i][j] * answer[j]).sum::<f64>()) / matrix[i][i];
    }
    answer.iter().all(|v| v.is_finite()).then_some(answer)
}
