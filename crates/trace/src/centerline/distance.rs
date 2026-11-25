//! Exact squared Euclidean distance, Felzenszwalb & Huttenlocher, "Distance Transforms of
//! Sampled Functions", §3. Separable lower envelopes of parabolas, linear time and crop storage.

const FAR: f64 = 1e20;

/// Transform one row or column. Each envelope entry is (sample, left intersection).
fn transform(f: &[f64], out: &mut [f64], envelope: &mut Vec<(usize, f64)>) -> Option<()> {
    envelope.clear();
    for (q, &fq) in f.iter().enumerate() {
        let mut left = -FAR;
        while let Some(&(v, z)) = envelope.last() {
            let (qf, vf) = (q as f64, v as f64);
            left = ((fq + qf * qf) - (*f.get(v)? + vf * vf)) / (2.0 * (qf - vf));
            if left > z {
                break;
            }
            envelope.pop();
        }
        if envelope.is_empty() {
            left = -FAR;
        }
        envelope.push((q, left));
    }
    let mut k = 0;
    for (q, d) in out.iter_mut().enumerate() {
        while envelope.get(k + 1).is_some_and(|&(_, z)| z < q as f64) {
            k += 1;
        }
        let &(v, _) = envelope.get(k)?;
        *d = (q as f64 - v as f64).powi(2) + *f.get(v)?;
    }
    Some(())
}

/// Squared centre-to-paper distances for a validated, paper-bordered component crop.
/// Border paper also represents the space outside the raster. Returns None for invalid sizes
/// or a non-paper border, rather than accepting an unbounded allocation or an absent seed.
pub(super) fn squared(mask: &[bool], w: usize, h: usize) -> Option<Vec<f32>> {
    let size = super::crop_size(w, h)?;
    if mask.len() != size || mask.iter().enumerate().any(|(i, &on)| on && (i < w || i >= size - w || i % w == 0 || i % w + 1 == w)) {
        return None;
    }
    two_passes(mask.iter().map(|&on| if on { FAR } else { 0.0 }).collect(), w, h)
}

/// Squared distances from each pixel of a validated crop to the nearest `seeds` pixel (a very large
/// value everywhere when there is none).
pub(super) fn to_seeds(seeds: &[bool], w: usize, h: usize) -> Option<Vec<f32>> {
    let size = super::crop_size(w, h)?;
    if seeds.len() != size {
        return None;
    }
    two_passes(seeds.iter().map(|&seed| if seed { 0.0 } else { FAR }).collect(), w, h)
}

/// The column pass and then the row pass over `grid` (0 at a seed, `FAR` elsewhere).
fn two_passes(mut grid: Vec<f64>, w: usize, h: usize) -> Option<Vec<f32>> {
    let n = w.max(h);
    let (mut f, mut d, mut envelope) = (vec![0.0; n], vec![0.0; n], Vec::with_capacity(n));
    for x in 0..w {
        for (y, v) in f.get_mut(..h)?.iter_mut().enumerate() {
            *v = *grid.get(y * w + x)?;
        }
        transform(f.get(..h)?, d.get_mut(..h)?, &mut envelope)?;
        for (y, &v) in d.get(..h)?.iter().enumerate() {
            *grid.get_mut(y * w + x)? = v;
        }
    }
    for row in grid.chunks_exact_mut(w) {
        for (v, &g) in f.get_mut(..w)?.iter_mut().zip(row.iter()) {
            *v = g;
        }
        transform(f.get(..w)?, d.get_mut(..w)?, &mut envelope)?;
        for (g, &v) in row.iter_mut().zip(d.get(..w)?) {
            *g = v;
        }
    }
    Some(grid.into_iter().map(|v| v as f32).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edt_equals_brute_force_on_random_masks() {
        let mut seed = 19u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            (seed >> 33) as usize
        };
        for case in 0..200 {
            let (w, h) = (3 + rnd() % 24, 3 + rnd() % 24);
            let mask: Vec<bool> =
                (0..w * h).map(|i| i >= w && i < w * (h - 1) && i % w != 0 && i % w + 1 < w && (case % 5 == 0 || rnd() % 4 != 0)).collect();
            let d = squared(&mask, w, h).unwrap();
            for (i, &actual) in d.iter().enumerate() {
                let expected = mask
                    .iter()
                    .enumerate()
                    .filter(|&(_, &on)| !on)
                    .map(|(j, _)| {
                        let dx = (i % w).abs_diff(j % w);
                        let dy = (i / w).abs_diff(j / w);
                        (dx * dx + dy * dy) as f32
                    })
                    .min_by(f32::total_cmp)
                    .unwrap();
                assert_eq!(actual, expected, "case {case}, pixel {i}");
            }
        }
    }

    #[test]
    fn distances_to_seeds_equal_brute_force() {
        let mut seed = 7u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            (seed >> 33) as usize
        };
        for case in 0..100 {
            let (w, h) = (3 + rnd() % 20, 3 + rnd() % 20);
            let seeds: Vec<bool> = (0..w * h).map(|_| rnd() % 9 == 0).collect();
            let d = to_seeds(&seeds, w, h).unwrap();
            for (i, &actual) in d.iter().enumerate() {
                let expected = seeds
                    .iter()
                    .enumerate()
                    .filter(|&(_, &s)| s)
                    .map(|(j, _)| {
                        let dx = (i % w).abs_diff(j % w);
                        let dy = (i / w).abs_diff(j / w);
                        (dx * dx + dy * dy) as f32
                    })
                    .min_by(f32::total_cmp);
                match expected {
                    Some(e) => assert_eq!(actual, e, "case {case}, pixel {i}"),
                    None => assert!(actual > 1e18, "case {case}: no seed, pixel {i} at {actual}"),
                }
            }
        }
        assert!(to_seeds(&[true; 8], 3, 3).is_none());
        assert!(to_seeds(&[], 2, 2).is_none());
    }

    #[test]
    fn invalid_crops_are_refused() {
        assert!(squared(&[], 0, 0).is_none());
        assert!(squared(&[], usize::MAX, 3).is_none());
        assert!(squared(&[false; 9], 4, 3).is_none());
        assert!(squared(&[true; 9], 3, 3).is_none());
        assert!(squared(&[], crate::MAX_PIXELS as usize + 3, 3).is_none());
        assert_eq!(squared(&[false; 9], 3, 3), Some(vec![0.0; 9]));
    }
}
