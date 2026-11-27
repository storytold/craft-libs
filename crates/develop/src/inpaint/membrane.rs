//! Membrane (harmonic) fill: `Δv = 0` inside a hole with the known pixels as Dirichlet boundary.
//! Used to initialise the coarsest level of [`crate::inpaint::complete`] and as its fallback.
//!
//! Sources: the membrane interpolant of P. Pérez, M. Gangnet, A. Blake, *Poisson Image Editing*,
//! ACM SIGGRAPH 2003 (§3), solved by successive over-relaxation on the 4-connected grid, accelerated
//! cascadically (solve on a half-resolution grid, prolong bilinearly, a few SOR sweeps per level).
//! Ported from PhotoCraft's `algo::poisson` (MIT OR Apache-2.0).

fn allocation_fits<T>(len: usize) -> bool {
    len.checked_mul(std::mem::size_of::<T>()).is_some_and(|bytes| bytes <= isize::MAX as usize)
}

/// Convergence threshold (max per-sweep update) for the SOR sweeps.
const TOL: f32 = 2e-5;
/// SOR sweeps (cap) and relaxation factor on every level above the coarsest: tuned so a 96×72 disc
/// is within ~1e-3 of a fully converged Gauss–Seidel solve (below one 8-bit step).
const FINE_ITERS: usize = 30;
const FINE_OMEGA: f32 = 1.7;

/// Solve `Δv = 0` on `unknown` pixels with Dirichlet values from the known pixels of `v`
/// (out-of-grid neighbours are ignored, i.e. a Neumann boundary). `v` holds the known values on
/// entry and the solution on exit. Cascadic multigrid + SOR.
pub(crate) fn solve_membrane(w: usize, h: usize, unknown: &[bool], v: &mut [f32]) {
    if Some(unknown.len()) != w.checked_mul(h) || v.len() != unknown.len() {
        return;
    }
    if !allocation_fits::<(usize, u8)>(unknown.iter().filter(|&&hole| hole).count()) {
        return;
    }
    // Keep the ordinary path bit-for-bit unchanged. Extremely large finite boundaries can
    // overflow even their mean or a four-neighbour sum; harmonic interpolation is linear,
    // so solve those channels at unit scale instead. The final clamp enforces the harmonic
    // maximum principle if the finite-iteration relaxation overshoots a boundary extremum.
    let mut low = f32::INFINITY;
    let mut high = f32::NEG_INFINITY;
    for (&value, &hole) in v.iter().zip(unknown) {
        if !hole {
            if !value.is_finite() {
                solve_rec(w, h, unknown, v, 0, &|| false);
                return;
            }
            low = low.min(value);
            high = high.max(value);
        }
    }
    let scale = low.abs().max(high.abs());
    let needs_scale = scale.is_finite() && f64::from(scale) > f64::from(f32::MAX) / v.len().max(16) as f64;
    if needs_scale {
        let mut normalized: Vec<f32> = v.iter().map(|&value| value / scale).collect();
        solve_rec(w, h, unknown, &mut normalized, 0, &|| false);
        for ((value, solved), &hole) in v.iter_mut().zip(normalized).zip(unknown) {
            if hole {
                *value = (f64::from(solved) * f64::from(scale)).clamp(f64::from(low), f64::from(high)) as f32;
            }
        }
    } else {
        solve_rec(w, h, unknown, v, 0, &|| false);
    }
}

fn solve_rec(w: usize, h: usize, unknown: &[bool], v: &mut [f32], depth: usize, stop: &(dyn Fn() -> bool + Sync)) {
    if stop() {
        return;
    }
    let n_unknown = unknown.iter().filter(|u| **u).count();
    if n_unknown == 0 {
        return;
    }
    // Like the cell list below, this filtered collection must not grow geometrically.
    let mut known = Vec::with_capacity(v.len() - n_unknown);
    known.extend(v.iter().zip(unknown).filter(|(_, u)| !**u).map(|(x, _)| *x));
    if known.is_empty() {
        return; // nothing to interpolate from
    }
    if w >= 8 && h >= 8 && n_unknown > 64 && depth < 16 {
        // Restrict: a coarse pixel is known if any of its children is (value = mean of known children).
        let (cw, chh) = (w.div_ceil(2), h.div_ceil(2));
        let mut cu = vec![true; cw * chh];
        let mut cv = vec![0.0f32; cw * chh];
        for cy in 0..chh {
            for cx in 0..cw {
                let (mut s, mut n) = (0.0f32, 0u32);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (x, y) = (cx * 2 + dx, cy * 2 + dy);
                    if x < w && y < h && !unknown[y * w + x] {
                        s += v[y * w + x];
                        n += 1;
                    }
                }
                if n > 0 {
                    cu[cy * cw + cx] = false;
                    cv[cy * cw + cx] = s / n as f32;
                }
            }
        }
        solve_rec(cw, chh, &cu, &mut cv, depth + 1, stop);
        // Prolong: bilinear sample of the coarse solution as the initial guess.
        for y in 0..h {
            let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (chh - 1) as f32);
            let y0 = fy.floor() as usize;
            let y1 = (y0 + 1).min(chh - 1);
            let ty = fy - y0 as f32;
            for x in 0..w {
                if !unknown[y * w + x] {
                    continue;
                }
                let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (cw - 1) as f32);
                let x0 = fx.floor() as usize;
                let x1 = (x0 + 1).min(cw - 1);
                let tx = fx - x0 as f32;
                let a = cv[y0 * cw + x0] + (cv[y0 * cw + x1] - cv[y0 * cw + x0]) * tx;
                let b = cv[y1 * cw + x0] + (cv[y1 * cw + x1] - cv[y1 * cw + x0]) * tx;
                v[y * w + x] = a + (b - a) * ty;
            }
        }
        sor(w, h, unknown, v, FINE_ITERS, FINE_OMEGA, stop);
    } else {
        let mean = known.iter().sum::<f32>() / known.len() as f32;
        for (x, u) in v.iter_mut().zip(unknown) {
            if *u {
                *x = mean;
            }
        }
        sor(w, h, unknown, v, 2000, 1.85, stop);
    }
}

/// SOR sweeps over the unknown pixels until the largest update drops below [`TOL`].
fn sor(w: usize, h: usize, unknown: &[bool], v: &mut [f32], max_iter: usize, omega: f32, stop: &(dyn Fn() -> bool + Sync)) {
    // Precompute the unknown pixels and which neighbours exist.
    // The caller validated this exact count; reserve it before extend so geometric growth
    // cannot round the allocation above the target's capacity limit.
    let mut cells = Vec::with_capacity(unknown.iter().filter(|&&hole| hole).count());
    cells.extend((0..w * h).filter(|&i| unknown[i]).map(|i| {
        let (x, y) = (i % w, i / w);
        let m = u8::from(x > 0) | (u8::from(x + 1 < w) << 1) | (u8::from(y > 0) << 2) | (u8::from(y + 1 < h) << 3);
        (i, m)
    }));
    for _ in 0..max_iter {
        if stop() {
            return;
        }
        let mut max_d = 0.0f32;
        for &(i, m) in &cells {
            let (s, n) = if m == 0b1111 {
                (v[i - 1] + v[i + 1] + v[i - w] + v[i + w], 4.0)
            } else {
                let mut s = 0.0;
                let mut n = 0.0;
                if m & 1 != 0 {
                    s += v[i - 1];
                    n += 1.0;
                }
                if m & 2 != 0 {
                    s += v[i + 1];
                    n += 1.0;
                }
                if m & 4 != 0 {
                    s += v[i - w];
                    n += 1.0;
                }
                if m & 8 != 0 {
                    s += v[i + w];
                    n += 1.0;
                }
                if n == 0.0 {
                    continue;
                }
                (s, n)
            };
            let d = omega * (s / n - v[i]);
            v[i] += d;
            max_d = max_d.max(d.abs());
        }
        if max_d < TOL {
            break;
        }
    }
}

/// Map `f` over the channels of an interleaved buffer as planar slices (parallel on native).
fn per_channel(w: usize, h: usize, ch: usize, f: impl Fn(usize) -> Vec<f32> + Sync + Send) -> Vec<f32> {
    #[cfg(not(target_arch = "wasm32"))]
    let planes: Vec<Vec<f32>> = if w * h >= 64 * 64 {
        use rayon::prelude::*;
        (0..ch).into_par_iter().map(&f).collect()
    } else {
        (0..ch).map(&f).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let planes: Vec<Vec<f32>> = (0..ch).map(&f).collect();
    let mut out = vec![0.0f32; w * h * ch];
    for (c, p) in planes.iter().enumerate() {
        for (i, v) in p.iter().enumerate() {
            out[i * ch + c] = *v;
        }
    }
    out
}

/// Membrane fill (no source texture): harmonic interpolation of the known pixels into `hole`.
/// `img` is interleaved `w × h × ch`, `hole` has `w × h` entries; on a size mismatch `img` comes back
/// unchanged.
pub fn membrane_fill(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool]) -> Vec<f32> {
    if Some(hole.len()) != w.checked_mul(h) || Some(img.len()) != hole.len().checked_mul(ch) {
        return img.to_vec();
    }
    // no pixels: nothing to fill (and `ch` would otherwise size one plane per channel, whatever it claims)
    if hole.is_empty() {
        return img.to_vec();
    }
    // A valid input slice does not prove the wider index/plane vectors fit the target.
    if !allocation_fits::<Vec<f32>>(ch) || !allocation_fits::<(usize, u8)>(hole.iter().filter(|&&hole| hole).count()) {
        return img.to_vec();
    }
    per_channel(w, h, ch, |c| {
        let mut v: Vec<f32> = (0..w * h).map(|i| if hole[i] { 0.0 } else { img[i * ch + c] }).collect();
        solve_membrane(w, h, hole, &mut v);
        v
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    fn disc(w: usize, h: usize, cx: f32, cy: f32, r: f32) -> Vec<bool> {
        (0..w * h).map(|i| ((i % w) as f32 - cx).hypot((i / w) as f32 - cy) < r).collect()
    }

    #[test]
    fn auxiliary_vector_limits_are_checked_without_allocating() {
        let max_cells = isize::MAX as usize / std::mem::size_of::<(usize, u8)>();
        assert!(allocation_fits::<(usize, u8)>(max_cells));
        assert!(!allocation_fits::<(usize, u8)>(max_cells + 1));
        assert!(!allocation_fits::<Vec<f32>>(usize::MAX));
    }

    #[test]
    fn a_zero_area_image_comes_back_unchanged_whatever_its_channel_count() {
        assert!(membrane_fill(0, 5, usize::MAX, &[], &[]).is_empty());
        assert!(membrane_fill(3, 0, usize::MAX / 2, &[], &[]).is_empty());
    }

    #[test]
    fn membrane_reproduces_linear_boundary() {
        // A linear function is harmonic: the solve must reproduce it exactly inside the hole.
        let (w, h) = (80, 60);
        let unknown = disc(w, h, 40.0, 30.0, 20.0);
        let truth: Vec<f32> = (0..w * h).map(|i| 0.2 + 0.005 * (i % w) as f32 + 0.003 * (i / w) as f32).collect();
        let mut v: Vec<f32> = truth.iter().zip(&unknown).map(|(t, u)| if *u { 0.0 } else { *t }).collect();
        solve_membrane(w, h, &unknown, &mut v);
        let err = v.iter().zip(&truth).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(err < 2e-3, "{err}");
    }
}
