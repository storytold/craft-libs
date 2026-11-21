//! Small geometry helpers for outlines: resampling, smoothing, corners, line and circle fits.

use craft_geom::Point;
use kurbo::Vec2;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// One cubic Bézier segment: start, two handles, end.
pub(crate) type Cubic = [Point; 4];

/// Longest outline (in points) any step will build, whatever the input.
pub(crate) const MAX_POINTS: usize = 400_000;

pub(crate) fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

pub(crate) fn bez(c: &Cubic, t: f64) -> Point {
    let u = 1.0 - t;
    let (a, b, cc, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point::new(a * c[0].x + b * c[1].x + cc * c[2].x + d * c[3].x, a * c[0].y + b * c[1].y + cc * c[2].y + d * c[3].y)
}

/// A closed polyline resampled at (about) `step` spacing; also returns the exact spacing used.
pub(crate) fn resample_closed(p: &[Point], step: f64) -> Option<(Vec<Point>, f64)> {
    let n = p.len();
    if n < 3 || step.is_nan() || step <= 0.0 || !p.iter().all(|&q| finite(q)) {
        return None;
    }
    let mut cum = Vec::with_capacity(n + 1);
    cum.push(0.0);
    for i in 0..n {
        let (a, b) = (*p.get(i)?, *p.get((i + 1) % n)?);
        cum.push(cum.last().copied().unwrap_or(0.0) + a.distance(b));
    }
    let total = cum.last().copied().unwrap_or(0.0);
    if total.is_nan() || total <= step {
        return None;
    }
    let m = ((total / step) as usize).clamp(8, MAX_POINTS);
    let mut out = Vec::with_capacity(m);
    let mut j = 0usize;
    for i in 0..m {
        let s = total * i as f64 / m as f64;
        while cum.get(j + 1).is_some_and(|&c| c < s) {
            j += 1;
        }
        let (c0, c1) = (*cum.get(j)?, *cum.get(j + 1)?);
        let t = if c1 > c0 { (s - c0) / (c1 - c0) } else { 0.0 };
        let (a, b) = (*p.get(j % n)?, *p.get((j + 1) % n)?);
        out.push(a + (b - a) * t);
    }
    Some((out, total / m as f64))
}

/// An open polyline resampled at (about) `step` spacing, both ends kept; also returns the exact
/// spacing used.
pub(crate) fn resample_open(p: &[Point], step: f64) -> Option<(Vec<Point>, f64)> {
    let n = p.len();
    let (first, last) = (*p.first()?, *p.last()?);
    if n < 2 || step.is_nan() || step <= 0.0 || !p.iter().all(|&q| finite(q)) {
        return None;
    }
    let mut cum = Vec::with_capacity(n);
    cum.push(0.0);
    for pair in p.windows(2) {
        cum.push(cum.last().copied().unwrap_or(0.0) + pair[0].distance(pair[1]));
    }
    let total = cum.last().copied().unwrap_or(0.0);
    if total.is_nan() || total <= 0.0 {
        return None;
    }
    let m = ((total / step).round() as usize).clamp(1, MAX_POINTS);
    let mut out = Vec::with_capacity(m + 1);
    out.push(first);
    let mut j = 0usize;
    for i in 1..m {
        let s = total * i as f64 / m as f64;
        while cum.get(j + 1).is_some_and(|&c| c < s) {
            j += 1;
        }
        let (c0, c1) = (*cum.get(j)?, *cum.get(j + 1)?);
        let t = if c1 > c0 { (s - c0) / (c1 - c0) } else { 0.0 };
        let (a, b) = (*p.get(j)?, *p.get(j + 1)?);
        out.push(a + (b - a) * t);
    }
    out.push(last);
    Some((out, total / m as f64))
}

fn kernel(sigma: f64) -> Vec<f64> {
    let s = sigma.max(0.3);
    let r = ((3.0 * s).ceil() as i64).max(1);
    let k: Vec<f64> = (-r..=r).map(|i| (-0.5 * (i as f64 / s).powi(2)).exp()).collect();
    let sum: f64 = k.iter().sum();
    k.iter().map(|v| v / sum).collect()
}

/// Gaussian smoothing of a closed polyline (σ in samples).
pub(crate) fn smooth_closed(p: &[Point], sigma: f64) -> Vec<Point> {
    let n = p.len() as i64;
    if n < 3 {
        return p.to_vec();
    }
    let k = kernel(sigma);
    let r = (k.len() as i64 - 1) / 2;
    (0..n)
        .map(|i| {
            let mut s = Vec2::ZERO;
            for (j, w) in k.iter().enumerate() {
                let idx = (i + j as i64 - r).rem_euclid(n) as usize;
                if let Some(q) = p.get(idx) {
                    s += q.to_vec2() * *w;
                }
            }
            s.to_point()
        })
        .collect()
}

/// Gaussian smoothing of an open run with both ends pinned (the run is extended by odd reflection
/// so tangents at the ends survive), by at most an eighth of the run's samples.
pub(crate) fn smooth_open(p: &[Point], sigma: f64) -> Vec<Point> {
    smooth_pinned(p, sigma.max(0.4).min(p.len() as f64 / 8.0))
}

/// [`smooth_open`] by `sigma` (in samples, at least 0.4) whatever the run's length.
pub(crate) fn smooth_pinned(p: &[Point], sigma: f64) -> Vec<Point> {
    let n = p.len();
    if n < 5 {
        return p.to_vec();
    }
    let s = sigma.max(0.4);
    let k = kernel(s);
    let r = (k.len() - 1) / 2;
    let r_ext = r.min(n - 2);
    let (first, last) = match (p.first(), p.last()) {
        (Some(&f), Some(&l)) => (f, l),
        _ => return p.to_vec(),
    };
    let mut ext: Vec<Point> = Vec::with_capacity(n + 2 * r_ext);
    for i in (1..=r_ext).rev() {
        ext.push(first + (first - p.get(i).copied().unwrap_or(first)));
    }
    ext.extend_from_slice(p);
    for i in 1..=r_ext {
        ext.push(last + (last - p.get(n - 1 - i).copied().unwrap_or(last)));
    }
    let kk = if r_ext == r { k } else { kernel_of_radius(s, r_ext) };
    let rr = (kk.len() - 1) / 2;
    let mut out: Vec<Point> = (0..n)
        .map(|i| {
            let mut acc = Vec2::ZERO;
            for (j, w) in kk.iter().enumerate() {
                let idx = i + r_ext + j;
                if let Some(q) = idx.checked_sub(rr).and_then(|x| ext.get(x)) {
                    acc += q.to_vec2() * *w;
                }
            }
            acc.to_point()
        })
        .collect();
    if let (Some(f), Some(l)) = (out.first_mut(), p.first()) {
        *f = *l;
    }
    if let (Some(f), Some(l)) = (out.last_mut(), p.last()) {
        *f = *l;
    }
    out
}

fn kernel_of_radius(sigma: f64, r: usize) -> Vec<f64> {
    let k: Vec<f64> = (-(r as i64)..=r as i64).map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp()).collect();
    let sum: f64 = k.iter().sum();
    k.iter().map(|v| v / sum).collect()
}

/// Turning angle (degrees) at each point of a closed polyline over a window of `w` samples.
pub(crate) fn turning(p: &[Point], w: usize) -> Vec<f64> {
    let n = p.len();
    (0..n)
        .map(|i| {
            let (Some(&c), Some(&a), Some(&b)) = (p.get(i), p.get((i + n - w % n) % n), p.get((i + w) % n)) else { return 0.0 };
            let (v1, v2) = (c - a, b - c);
            (v1.cross(v2)).atan2(v1.dot(v2)).abs().to_degrees()
        })
        .collect()
}

/// Local maxima of the turning angle above `thr` degrees, at least `w` samples apart.
pub(crate) fn find_corners(p: &[Point], w: usize, thr: f64) -> Vec<usize> {
    let n = p.len();
    if n < 8 || w == 0 {
        return Vec::new();
    }
    let ang = turning(p, w);
    let at = |i: usize| ang.get(i % n).copied().unwrap_or(0.0);
    let mut idx: Vec<usize> = Vec::new();
    for i in 0..n {
        let a = at(i);
        if a >= thr && a >= at(i + n - 1) && a > at(i + 1) && idx.last().is_none_or(|&l| i - l > w) {
            idx.push(i);
        }
    }
    if idx.len() > 1
        && let (Some(&f), Some(&l)) = (idx.first(), idx.last())
        && f + n - l <= w
    {
        idx.pop();
    }
    idx
}

/// A straight line through `pts`: centroid, unit direction (first → last) and the largest
/// distance of any point from the line.
pub(crate) struct Line {
    pub c: Point,
    pub d: Vec2,
    pub resid: f64,
}

pub(crate) fn line_fit(pts: &[Point]) -> Option<Line> {
    let n = pts.len();
    if n < 2 {
        return None;
    }
    let c = (pts.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / n as f64).to_point();
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for p in pts {
        let v = *p - c;
        sxx += v.x * v.x;
        sxy += v.x * v.y;
        syy += v.y * v.y;
    }
    let ang = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let mut d = Vec2::new(ang.cos(), ang.sin());
    if d.dot(*pts.last()? - *pts.first()?) < 0.0 {
        d = -d;
    }
    let nrm = Vec2::new(-d.y, d.x);
    let resid = pts.iter().map(|p| (*p - c).dot(nrm).abs()).fold(0.0, f64::max);
    (resid.is_finite()).then_some(Line { c, d, resid })
}

fn solve3(m: [f64; 9], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6]) + m[2] * (m[3] * m[7] - m[4] * m[6]);
    if det.abs() < 1e-12 {
        return None;
    }
    let rep = |col: usize| {
        let mut t = m;
        for r in 0..3 {
            t[r * 3 + col] = b[r];
        }
        t[0] * (t[4] * t[8] - t[5] * t[7]) - t[1] * (t[3] * t[8] - t[5] * t[6]) + t[2] * (t[3] * t[7] - t[4] * t[6])
    };
    Some([rep(0) / det, rep(1) / det, rep(2) / det])
}

/// A circle through `pts` (algebraic fit): centre, radius, largest radial error.
pub(crate) struct Circle {
    pub c: Point,
    pub r: f64,
    pub resid: f64,
}

pub(crate) fn circle_fit(pts: &[Point]) -> Option<Circle> {
    if pts.len() < 3 {
        return None;
    }
    // Centre the data for conditioning.
    let m = (pts.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / pts.len() as f64).to_point();
    let (mut a, mut b) = ([0.0f64; 9], [0.0f64; 3]);
    for p in pts {
        let (x, y) = (p.x - m.x, p.y - m.y);
        let row = [2.0 * x, 2.0 * y, 1.0];
        let rhs = x * x + y * y;
        for i in 0..3 {
            for j in 0..3 {
                a[i * 3 + j] += row[i] * row[j];
            }
            b[i] += row[i] * rhs;
        }
    }
    let s = solve3(a, b)?;
    let r2 = s[2] + s[0] * s[0] + s[1] * s[1];
    if r2.is_nan() || r2 <= 1e-12 {
        return None;
    }
    let (c, r) = (Point::new(s[0] + m.x, s[1] + m.y), r2.sqrt());
    let resid = pts.iter().map(|p| (p.distance(c) - r).abs()).fold(0.0, f64::max);
    (resid.is_finite() && r.is_finite()).then_some(Circle { c, r, resid })
}

/// Cubic Béziers approximating the arc of radius `r` about `c` from angle `a0` to `a1`.
pub(crate) fn arc_cubics(c: Point, r: f64, a0: f64, a1: f64) -> Vec<Cubic> {
    let n = ((a1 - a0).abs() / std::f64::consts::FRAC_PI_2).ceil().clamp(1.0, 64.0) as usize;
    let da = (a1 - a0) / n as f64;
    let k = 4.0 / 3.0 * (da / 4.0).tan();
    (0..n)
        .map(|j| {
            let (s, e) = (a0 + j as f64 * da, a0 + (j + 1) as f64 * da);
            let p0 = Point::new(c.x + r * s.cos(), c.y + r * s.sin());
            let p3 = Point::new(c.x + r * e.cos(), c.y + r * e.sin());
            [p0, p0 + Vec2::new(-s.sin(), s.cos()) * (k * r), p3 - Vec2::new(-e.sin(), e.cos()) * (k * r), p3]
        })
        .collect()
}

/// Points along the curves, about `spacing` apart.
pub(crate) fn sample_cubics(cs: &[Cubic], spacing: f64) -> Vec<Point> {
    let mut out = Vec::new();
    for c in cs {
        let len = (c[1] - c[0]).hypot() + (c[2] - c[1]).hypot() + (c[3] - c[2]).hypot();
        let n = ((len / spacing.max(1e-3)) as usize).clamp(8, 20_000);
        out.extend((0..=n).map(|i| bez(c, i as f64 / n as f64)));
        if out.len() > MAX_POINTS * 4 {
            break;
        }
    }
    out
}

/// Multiply-and-rotate hasher for the small integer cell keys of [`all_within`]. The grid is only ever looked up, never
/// iterated, so the hash cannot change a result; std's SipHash took about a quarter of the time of a noisy trace.
#[derive(Default)]
struct CellHasher(u64);

impl CellHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for CellHasher {
    fn write(&mut self, bytes: &[u8]) {
        bytes.iter().for_each(|&b| self.add(u64::from(b)));
    }

    fn write_i64(&mut self, word: i64) {
        self.add(word as u64);
    }

    fn finish(&self) -> u64 {
        // The table picks its bucket from the low bits, which a product leaves weak: bring the strong high bits down.
        self.0.rotate_left(26)
    }
}

/// True when every point of `a` has a point of `b` within `lim`.
pub(crate) fn all_within(a: &[Point], b: &[Point], lim: f64) -> bool {
    if lim.is_nan() || lim <= 0.0 {
        return false;
    }
    let key = |p: Point| ((p.x / lim).floor() as i64, (p.y / lim).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<Point>, BuildHasherDefault<CellHasher>> = HashMap::default();
    for &q in b {
        grid.entry(key(q)).or_default().push(q);
    }
    a.iter().all(|&p| {
        let (kx, ky) = key(p);
        (-1..=1).any(|dx| (-1..=1).any(|dy| grid.get(&(kx + dx, ky + dy)).is_some_and(|v| v.iter().any(|q| q.distance(p) <= lim))))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circle_pts(n: usize, r: f64) -> Vec<Point> {
        (0..n)
            .map(|i| {
                Point::new(
                    50.0 + r * (i as f64 / n as f64 * std::f64::consts::TAU).cos(),
                    50.0 + r * (i as f64 / n as f64 * std::f64::consts::TAU).sin(),
                )
            })
            .collect()
    }

    #[test]
    fn resampling_makes_even_spacing() {
        let (r, step) = resample_closed(&circle_pts(37, 20.0), 0.5).expect("resample");
        assert!((step - 0.5).abs() < 0.01);
        let gaps: Vec<f64> = (0..r.len()).map(|i| r[i].distance(r[(i + 1) % r.len()])).collect();
        assert!(gaps.iter().all(|g| (g - step).abs() < 0.02), "{gaps:?}");
    }

    #[test]
    fn resampling_refuses_degenerate_input() {
        assert!(resample_closed(&[], 1.0).is_none());
        assert!(resample_closed(&[Point::new(0.0, 0.0); 5], 1.0).is_none());
        assert!(resample_closed(&circle_pts(10, 5.0), 0.0).is_none());
        assert!(resample_closed(&[Point::new(f64::NAN, 0.0), Point::new(1.0, 0.0), Point::new(0.0, 1.0)], 0.1).is_none());
    }

    #[test]
    fn smoothing_a_circle_keeps_its_radius() {
        let s = smooth_closed(&circle_pts(600, 20.0), 4.0);
        for p in &s {
            assert!((p.distance(Point::new(50.0, 50.0)) - 20.0).abs() < 0.05);
        }
    }

    #[test]
    fn open_smoothing_pins_the_ends() {
        let run: Vec<Point> = (0..60).map(|i| Point::new(i as f64 * 0.2, (i as f64 * 0.7).sin() * 0.05)).collect();
        let s = smooth_open(&run, 3.0);
        assert_eq!((s[0], s[59]), (run[0], run[59]));
        assert_eq!(s.len(), run.len());
    }

    #[test]
    fn a_square_has_four_corners() {
        let sq: Vec<Point> = (0..400)
            .map(|i| {
                let t = i as f64 / 100.0;
                match t as usize {
                    0 => Point::new(t * 10.0, 0.0),
                    1 => Point::new(10.0, (t - 1.0) * 10.0),
                    2 => Point::new(10.0 - (t - 2.0) * 10.0, 10.0),
                    _ => Point::new(0.0, 10.0 - (t - 3.0) * 10.0),
                }
            })
            .collect();
        assert_eq!(find_corners(&sq, 6, 60.0).len(), 4);
    }

    #[test]
    fn a_circle_has_no_corners() {
        assert!(find_corners(&circle_pts(500, 20.0), 6, 40.0).is_empty());
    }

    #[test]
    fn line_and_circle_fits_recover_their_shapes() {
        let line: Vec<Point> = (0..50).map(|i| Point::new(i as f64, 2.0 * i as f64 + 1.0)).collect();
        let l = line_fit(&line).expect("line");
        assert!(l.resid < 1e-9 && l.d.x > 0.0);
        let c = circle_fit(&circle_pts(100, 12.5)).expect("circle");
        assert!((c.r - 12.5).abs() < 1e-6 && c.c.distance(Point::new(50.0, 50.0)) < 1e-6 && c.resid < 1e-6);
        assert!(circle_fit(&line).is_none() || circle_fit(&line).is_some_and(|c| c.r > 1e3 || c.resid > 1.0));
        assert!(line_fit(&[Point::new(0.0, 0.0)]).is_none());
    }

    #[test]
    fn arcs_stay_on_the_circle() {
        let cs = arc_cubics(Point::new(5.0, 5.0), 10.0, 0.3, 4.0);
        for p in sample_cubics(&cs, 0.05) {
            assert!((p.distance(Point::new(5.0, 5.0)) - 10.0).abs() < 0.01);
        }
    }

    #[test]
    fn within_checks_both_near_and_far() {
        let a = circle_pts(100, 10.0);
        let b = circle_pts(100, 10.2);
        assert!(all_within(&a, &b, 0.5));
        assert!(!all_within(&a, &b, 0.1));
        assert!(!all_within(&a, &b, 0.0));
    }

    #[test]
    fn within_agrees_with_a_brute_force_search() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut rand = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for round in 0..60 {
            // Negative and large coordinates, clustered and sparse clouds.
            let spread = [3.0, 40.0, 1.0e5][round % 3];
            let cloud = |n: usize, rand: &mut dyn FnMut() -> f64| -> Vec<Point> {
                (0..n).map(|_| Point::new((rand() - 0.5) * spread, (rand() - 0.5) * spread)).collect()
            };
            let a = cloud(1 + round % 7, &mut rand);
            let mut b = cloud(200, &mut rand);
            b.extend(a.iter().map(|p| Point::new(p.x + (rand() - 0.5) * 0.3, p.y + (rand() - 0.5) * 0.3)));
            for lim in [0.05, 0.2, 1.5, 30.0] {
                let brute = a.iter().all(|&p| b.iter().any(|q| q.distance(p) <= lim));
                assert_eq!(all_within(&a, &b, lim), brute, "round {round}, lim {lim}");
                assert_eq!(all_within(&b, &a, lim), b.iter().all(|&p| a.iter().any(|q| q.distance(p) <= lim)), "round {round}, lim {lim}");
            }
        }
    }

    #[test]
    fn cell_hasher_spreads_keys_whose_low_bits_are_zero() {
        use std::hash::BuildHasher;
        // Cell indices that are all multiples of 1024 (pixel-exact points with a power-of-two limit).
        let build = BuildHasherDefault::<CellHasher>::default();
        let buckets: std::collections::HashSet<u64> =
            (0..64i64).flat_map(|i| (0..64i64).map(move |j| (i << 10, j << 10))).map(|k| build.hash_one(k) & 1023).collect();
        assert!(buckets.len() > 600, "{} of 1024 buckets used", buckets.len());
    }
}
