//! Grey-scale morphology: square min (erosion) and max (dilation) filters.
//!
//! Separable van Herk / Gil-Werman filters: each 1-D pass splits the (edge-extended) line into
//! blocks of the window's width and keeps a running min from the left and from the right within
//! each block; any window then spans at most two blocks, so its min is the min of one suffix and
//! one prefix value. That is three comparisons per pixel per pass, whatever the radius.
//!
//! Edges are clamped (the edge pixel repeats), which for a min or max is the same as clipping the
//! window to the image. NaNs are skipped like [`f32::min`] does: a window yields NaN only when every
//! value in it is NaN.

use super::Plane;

/// Square min filter (erosion) of radius `r`: each output pixel is the minimum over the
/// `(2r+1)²` window around it, clipped to the image. `r == 0` returns a copy.
pub fn min_filter(p: &Plane, r: usize) -> Plane {
    filter(p, r, f32::min)
}

/// Square max filter (dilation) of radius `r` (see [`min_filter`]).
pub fn max_filter(p: &Plane, r: usize) -> Plane {
    filter(p, r, f32::max)
}

fn filter(p: &Plane, r: usize, pick: fn(f32, f32) -> f32) -> Plane {
    let (w, h) = (p.width, p.height);
    if r == 0 || p.data.is_empty() || p.data.len() != w.saturating_mul(h) {
        return p.clone();
    }
    let mut out = p.clone();
    rows(&mut out, r, pick);
    let mut t = out.transpose();
    rows(&mut t, r, pick);
    t.transpose()
}

/// [`line`] over every row of `p`, in place (row-parallel, one scratch per worker).
fn rows(p: &mut Plane, r: usize, pick: fn(f32, f32) -> f32) {
    let w = p.width;
    if w == 0 {
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        p.data.par_chunks_mut(w).for_each_init(Scratch::default, |s, row| line(row, r, pick, s));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let mut s = Scratch::default();
        p.data.chunks_mut(w).for_each(|row| line(row, r, pick, &mut s));
    }
}

#[derive(Default)]
struct Scratch {
    ext: Vec<f32>,
    pre: Vec<f32>,
    suf: Vec<f32>,
}

/// One 1-D pass over `row`, in place.
fn line(row: &mut [f32], r: usize, pick: fn(f32, f32) -> f32, s: &mut Scratch) {
    let n = row.len();
    if n == 0 {
        return;
    }
    // a radius past the line covers it all; capping keeps the scratch bounded
    let r = r.min(n - 1);
    if r == 0 {
        return;
    }
    let k = 2 * r + 1;
    // the line with `r` copies of each edge pixel on either side
    s.ext.clear();
    s.ext.extend(std::iter::repeat_n(row[0], r));
    s.ext.extend_from_slice(row);
    s.ext.extend(std::iter::repeat_n(row[n - 1], r));
    let m = s.ext.len();
    s.pre.clear();
    s.pre.resize(m, 0.0);
    s.suf.clear();
    s.suf.resize(m, 0.0);
    for (i, &v) in s.ext.iter().enumerate() {
        s.pre[i] = if i % k == 0 { v } else { pick(s.pre[i - 1], v) };
    }
    for i in (0..m).rev() {
        let v = s.ext[i];
        s.suf[i] = if i + 1 == m || (i + 1) % k == 0 { v } else { pick(s.suf[i + 1], v) };
    }
    // window [x, x + 2r] of `ext` is output x
    for (x, o) in row.iter_mut().enumerate() {
        *o = pick(s.suf[x], s.pre[x + 2 * r]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The definition: min (or max) over the clipped window.
    fn naive(p: &Plane, r: usize, max: bool) -> Plane {
        let (w, h) = (p.width, p.height);
        let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
        Plane::from_fn(w, h, |x, y| {
            let mut m = p.get(x, y);
            for yy in y.saturating_sub(r)..=(y + r).min(h - 1) {
                for xx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                    m = pick(m, p.get(xx, yy));
                }
            }
            m
        })
    }

    /// splitmix64-ish deterministic noise.
    fn noise(w: usize, h: usize, seed: u64) -> Plane {
        Plane::from_fn(w, h, |x, y| {
            let mut z = seed.wrapping_add((y * w + x) as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z ^= z >> 31;
            (z % 10_000) as f32 / 10_000.0
        })
    }

    #[test]
    fn matches_the_naive_filter() {
        for (w, h) in [(1, 1), (1, 9), (9, 1), (7, 5), (31, 17), (64, 40)] {
            for r in [1, 2, 3, 5, 8, 20, 100] {
                let p = noise(w, h, (w * 131 + h * 7 + r) as u64);
                assert_eq!(min_filter(&p, r), naive(&p, r, false), "min {w}x{h} r={r}");
                assert_eq!(max_filter(&p, r), naive(&p, r, true), "max {w}x{h} r={r}");
            }
        }
    }

    #[test]
    fn radius_zero_and_empty_are_copies() {
        let p = noise(6, 4, 1);
        assert_eq!(min_filter(&p, 0), p);
        let e = Plane::new(0, 0);
        assert_eq!(min_filter(&e, 3), e);
        let e = Plane::new(5, 0);
        assert_eq!(max_filter(&e, 3), e);
    }

    #[test]
    fn radius_past_the_image_is_the_global_extreme() {
        let p = noise(13, 7, 3);
        let lo = p.data.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = p.data.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(min_filter(&p, 1_000_000).data.iter().all(|&v| v == lo));
        assert!(max_filter(&p, usize::MAX).data.iter().all(|&v| v == hi));
    }

    #[test]
    fn a_dark_dot_grows_to_the_window() {
        let mut p = Plane::filled(11, 11, 1.0);
        p.set(5, 5, 0.0);
        let q = min_filter(&p, 2);
        for y in 0..11 {
            for x in 0..11 {
                let inside = (3..=7).contains(&x) && (3..=7).contains(&y);
                assert_eq!(q.get(x, y), if inside { 0.0 } else { 1.0 }, "({x}, {y})");
            }
        }
    }

    #[test]
    fn nans_are_skipped() {
        let mut p = Plane::filled(5, 5, 0.5);
        p.set(2, 2, f32::NAN);
        p.set(0, 0, 0.25);
        let q = min_filter(&p, 1);
        assert_eq!(q.get(2, 2), 0.5);
        assert_eq!(q.get(1, 1), 0.25);
        let all = Plane::filled(3, 3, f32::NAN);
        assert!(min_filter(&all, 1).data.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn a_short_buffer_is_returned_unchanged() {
        // a plane whose data doesn't fill its size must not panic
        let p = Plane { width: 4, height: 4, data: vec![1.0; 3] };
        assert_eq!(min_filter(&p, 2), p);
    }
}
