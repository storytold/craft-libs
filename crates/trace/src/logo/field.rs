//! Smooth sub-pixel fields: cubic B-spline interpolation of a coverage plane.
//!
//! Method: cubic B-spline interpolation with a recursive prefilter, as described in M. Unser, A.
//! Aldroubi and M. Eden, "B-spline signal processing" (IEEE Trans. Signal Processing, 1993) and P.
//! Thévenaz, T. Blu and M. Unser, "Interpolation revisited" (IEEE Trans. Medical Imaging, 2000).
//! Written from those papers.
//!
//! Interpolating coverage (an area fraction), not colour, and then comparing classes, puts an
//! edge where two classes are equally present, which is accurate to a fraction of a pixel.

/// Gaussian blur of a `w × h` plane (σ in pixels, edges repeated). Linear, so per-pixel weights that
/// sum to one across planes still do.
pub(crate) fn blur_plane(plane: &mut [f32], w: usize, h: usize, sigma: f64) {
    if w == 0 || h == 0 || plane.len() < w * h || sigma.is_nan() || sigma <= 0.0 {
        return;
    }
    let r = (3.0 * sigma).ceil() as i64;
    let k: Vec<f64> = (-r..=r).map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp()).collect();
    let sum: f64 = k.iter().sum();
    let at = |src: &[f32], x: i64, y: i64| {
        f64::from(src.get(y.clamp(0, h as i64 - 1) as usize * w + x.clamp(0, w as i64 - 1) as usize).copied().unwrap_or(0.0))
    };
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let v: f64 = k.iter().enumerate().map(|(j, kv)| kv * at(plane, x + j as i64 - r, y)).sum();
            if let Some(d) = tmp.get_mut(y as usize * w + x as usize) {
                *d = (v / sum) as f32;
            }
        }
    }
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let v: f64 = k.iter().enumerate().map(|(j, kv)| kv * at(&tmp, x, y + j as i64 - r)).sum();
            if let Some(d) = plane.get_mut(y as usize * w + x as usize) {
                *d = (v / sum) as f32;
            }
        }
    }
}

/// Replace samples by cubic B-spline coefficients so that interpolation passes through them
/// (Unser's recursive filter, mirror boundaries).
fn prefilter_line(c: &mut [f64]) {
    let n = c.len();
    if n < 2 {
        return;
    }
    let z = 3.0f64.sqrt() - 2.0;
    let gain = (1.0 - z) * (1.0 - 1.0 / z);
    for v in c.iter_mut() {
        *v *= gain;
    }
    // Causal initial value: the exact mirror-boundary sum for short lines, a truncated one for long.
    let at = |k: usize| c.get(k).copied().unwrap_or(0.0);
    let horizon = ((1e-9f64).ln() / z.abs().ln()).ceil() as usize;
    let first = if horizon < n {
        let mut zk = z;
        let mut sum = at(0);
        for k in 1..horizon {
            sum += zk * at(k);
            zk *= z;
        }
        sum
    } else {
        let iz = 1.0 / z;
        let mut zk = z;
        let mut z2n = z.powi(n as i32 - 1);
        let mut sum = at(0) + z2n * at(n - 1);
        z2n = z2n * z2n * iz;
        for k in 1..n - 1 {
            sum += (zk + z2n) * at(k);
            zk *= z;
            z2n *= iz;
        }
        sum / (1.0 - zk * zk)
    };
    if let Some(v) = c.first_mut() {
        *v = first;
    }
    for k in 1..n {
        let prev = c.get(k - 1).copied().unwrap_or(0.0);
        if let Some(v) = c.get_mut(k) {
            *v += z * prev;
        }
    }
    let (a, b) = (c.get(n - 2).copied().unwrap_or(0.0), c.get(n - 1).copied().unwrap_or(0.0));
    if let Some(last) = c.last_mut() {
        *last = (z / (z * z - 1.0)) * (z * a + b);
    }
    for k in (0..n - 1).rev() {
        let next = c.get(k + 1).copied().unwrap_or(0.0);
        if let Some(v) = c.get_mut(k) {
            *v = z * (next - *v);
        }
    }
}

/// B-spline coefficients of a `w × h` plane.
fn coefficients(plane: &[f32], w: usize, h: usize) -> Vec<f64> {
    let mut c: Vec<f64> = plane.iter().map(|&v| f64::from(v)).collect();
    for row in c.chunks_mut(w.max(1)) {
        prefilter_line(row);
    }
    let mut col = vec![0.0f64; h];
    for x in 0..w {
        for (y, v) in col.iter_mut().enumerate() {
            *v = c.get(y * w + x).copied().unwrap_or(0.0);
        }
        prefilter_line(&mut col);
        for (y, v) in col.iter().enumerate() {
            if let Some(dst) = c.get_mut(y * w + x) {
                *dst = *v;
            }
        }
    }
    c
}

/// First tap and the four cubic B-spline weights for every output sample along one axis.
fn taps(n_in: usize, u: usize) -> Vec<(i64, [f64; 4])> {
    (0..n_in * u)
        .map(|i| {
            let x = (i as f64 + 0.5) / u as f64 - 0.5;
            let i0 = x.floor();
            let t = x - i0;
            let t2 = t * t;
            let t3 = t2 * t;
            let w = [(1.0 - t).powi(3) / 6.0, (3.0 * t3 - 6.0 * t2 + 4.0) / 6.0, (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) / 6.0, t3 / 6.0];
            (i0 as i64 - 1, w)
        })
        .collect()
}

/// The plane smoothly upsampled `u` times (sample centres stay aligned), clamped to `0..=1`.
pub(crate) fn upsample(plane: &[f32], w: usize, h: usize, u: usize) -> Vec<f32> {
    if w == 0 || h == 0 || u == 0 || plane.len() < w * h {
        return Vec::new();
    }
    let coef = coefficients(plane, w, h);
    let (tx, ty) = (taps(w, u), taps(h, u));
    let ow = w * u;
    // Whole-sample mirror boundaries, matching the prefilter.
    let clampi = |v: i64, n: usize| {
        if n < 2 {
            return 0;
        }
        let period = 2 * (n as i64 - 1);
        let m = v.rem_euclid(period);
        (if m < n as i64 { m } else { period - m }) as usize
    };
    let mut tmp = vec![0.0f64; h * ow];
    for y in 0..h {
        let row = coef.get(y * w..(y + 1) * w).unwrap_or(&[]);
        for (ox, &(base, wt)) in tx.iter().enumerate() {
            let mut s = 0.0;
            for (j, wj) in wt.iter().enumerate() {
                s += wj * row.get(clampi(base + j as i64, w)).copied().unwrap_or(0.0);
            }
            if let Some(d) = tmp.get_mut(y * ow + ox) {
                *d = s;
            }
        }
    }
    let mut out = vec![0.0f32; ow * h * u];
    for (oy, &(base, wt)) in ty.iter().enumerate() {
        let rows: [usize; 4] = std::array::from_fn(|j| clampi(base + j as i64, h));
        for ox in 0..ow {
            let mut s = 0.0;
            for (j, wj) in wt.iter().enumerate() {
                s += wj * tmp.get(rows[j] * ow + ox).copied().unwrap_or(0.0);
            }
            if let Some(d) = out.get_mut(oy * ow + ox) {
                *d = s.clamp(0.0, 1.0) as f32;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_constant_plane_stays_constant() {
        let up = upsample(&[0.5; 6 * 5], 6, 5, 4);
        assert_eq!(up.len(), 24 * 20);
        assert!(up.iter().all(|&v| (v - 0.5).abs() < 1e-4));
    }

    #[test]
    fn interpolation_passes_through_the_samples() {
        // Sample (x, y) lives at the centre of its u×u block, i.e. between output samples; with
        // u = 1 the output is the input.
        let plane: Vec<f32> = (0..30).map(|i| ((i * 7) % 11) as f32 / 11.0).collect();
        let up = upsample(&plane, 6, 5, 1);
        for (a, b) in plane.iter().zip(&up) {
            assert!((a - b).abs() < 1e-4, "{a} vs {b}");
        }
    }

    #[test]
    fn a_blurred_step_crosses_half_where_the_edge_is() {
        // A vertical edge half-way through column 4: columns 0..4 are 0, column 4 is 0.5, then 1.
        let (w, h) = (9, 3);
        let plane: Vec<f32> = (0..w * h)
            .map(|i| match i % w {
                0..=3 => 0.0,
                4 => 0.5,
                _ => 1.0,
            })
            .collect();
        let u = 8;
        let up = upsample(&plane, w, h, u);
        let row = &up[(h * u / 2) * (w * u)..(h * u / 2 + 1) * (w * u)];
        let x = row.iter().position(|&v| v >= 0.5).unwrap_or(0);
        // The 0.5 crossing sits at the centre of column 4 → output sample 4.5 * u = 36.
        assert!((x as f64 - 36.0).abs() <= 1.5, "crossing at {x}");
    }

    #[test]
    fn degenerate_input_is_empty_not_a_panic() {
        assert!(upsample(&[], 0, 0, 4).is_empty());
        assert!(upsample(&[1.0], 1, 1, 0).is_empty());
        assert!(upsample(&[1.0], 2, 2, 3).is_empty());
    }
}
