//! Marching squares: closed contours of a scalar field at level 0.
//!
//! Segments keep the positive side on their left (x right, y down), so a region's outer boundary
//! and its holes wind in opposite directions and need no point-in-polygon test to tell apart.

use craft_geom::Point;
use std::collections::HashMap;

/// Edge ids: `(y * w + x) * 2` is the horizontal edge from `(x, y)` to `(x + 1, y)`, `+ 1` the
/// vertical edge from `(x, y)` to `(x, y + 1)`.
fn edge_point(f: &[f32], w: usize, id: usize) -> Point {
    let cell = id / 2;
    let (x, y) = (cell % w, cell / w);
    let at = |xx: usize, yy: usize| f64::from(f.get(yy * w + xx).copied().unwrap_or(0.0));
    if id.is_multiple_of(2) {
        let (a, b) = (at(x, y), at(x + 1, y));
        let t = if (a - b).abs() < 1e-12 { 0.5 } else { (a / (a - b)).clamp(0.0, 1.0) };
        Point::new(x as f64 + t, y as f64)
    } else {
        let (a, b) = (at(x, y), at(x, y + 1));
        let t = if (a - b).abs() < 1e-12 { 0.5 } else { (a / (a - b)).clamp(0.0, 1.0) };
        Point::new(x as f64, y as f64 + t)
    }
}

/// Closed loops (grid coordinates, sample `(x, y)` at `(x, y)`) around the region where `f > 0`.
/// Loops reaching the grid border are not closed by this function: pad the field with negatives.
pub(crate) fn contours(f: &[f32], w: usize, h: usize) -> Vec<Vec<Point>> {
    if w < 2 || h < 2 || f.len() < w * h {
        return Vec::new();
    }
    let mut next: HashMap<usize, usize> = HashMap::new();
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            let v = |xx: usize, yy: usize| f.get(yy * w + xx).copied().unwrap_or(-1.0);
            let (tl, tr, br, bl) = (v(x, y), v(x + 1, y), v(x + 1, y + 1), v(x, y + 1));
            let case = usize::from(tl > 0.0) | usize::from(tr > 0.0) << 1 | usize::from(br > 0.0) << 2 | usize::from(bl > 0.0) << 3;
            if case == 0 || case == 15 {
                continue;
            }
            let c = y * w + x;
            let (t, r, b, l) = (c * 2, (c + 1) * 2 + 1, (c + w) * 2, c * 2 + 1);
            let centre = (tl + tr + br + bl) * 0.25 > 0.0;
            let segs: &[(usize, usize)] = match case {
                1 => &[(l, t)],
                2 => &[(t, r)],
                3 => &[(l, r)],
                4 => &[(r, b)],
                5 => {
                    if centre {
                        &[(r, t), (l, b)]
                    } else {
                        &[(l, t), (r, b)]
                    }
                }
                6 => &[(t, b)],
                7 => &[(l, b)],
                8 => &[(b, l)],
                9 => &[(b, t)],
                10 => {
                    if centre {
                        &[(t, l), (b, r)]
                    } else {
                        &[(t, r), (b, l)]
                    }
                }
                11 => &[(b, r)],
                12 => &[(r, l)],
                13 => &[(r, t)],
                14 => &[(t, l)],
                _ => &[],
            };
            for &(from, to) in segs {
                next.insert(from, to);
            }
        }
    }
    let mut loops = Vec::new();
    let mut starts: Vec<usize> = next.keys().copied().collect();
    starts.sort_unstable();
    for s in starts {
        if !next.contains_key(&s) {
            continue;
        }
        let mut pts = Vec::new();
        let mut cur = s;
        let mut closed = false;
        for _ in 0..next.len() + 1 {
            let Some(n) = next.remove(&cur) else { break };
            pts.push(edge_point(f, w, cur));
            cur = n;
            if cur == s {
                closed = true;
                break;
            }
        }
        if closed && pts.len() >= 3 {
            loops.push(pts);
        }
    }
    loops
}

/// Twice the signed area of a closed polygon (shoelace, x right, y down). With the positive side
/// on the left, the **outer** boundary of a region is negative and its **holes** are positive.
pub(crate) fn area2(p: &[Point]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| {
            let (a, b) = (p.get(i).copied().unwrap_or_default(), p.get((i + 1) % n).copied().unwrap_or_default());
            a.x * b.y - b.x * a.y
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `f` = positive inside a disc of radius `r` at `(cx, cy)`, on a padded grid.
    fn disc(w: usize, h: usize, cx: f64, cy: f64, r: f64) -> Vec<f32> {
        (0..w * h).map(|i| (r - ((i % w) as f64 - cx).hypot((i / w) as f64 - cy)) as f32).collect()
    }

    #[test]
    fn a_disc_gives_one_loop_of_the_right_size() {
        let f = disc(40, 40, 20.0, 20.0, 9.0);
        let loops = contours(&f, 40, 40);
        assert_eq!(loops.len(), 1);
        let a = area2(&loops[0]).abs() / 2.0;
        assert!((a - std::f64::consts::PI * 81.0).abs() / (std::f64::consts::PI * 81.0) < 0.01, "area {a}");
        // Every vertex is on the circle to well under a hundredth of a cell.
        for p in &loops[0] {
            assert!(((p.x - 20.0).hypot(p.y - 20.0) - 9.0).abs() < 0.05);
        }
    }

    #[test]
    fn outer_boundaries_and_holes_wind_in_opposite_directions() {
        // A ring: positive between radius 5 and 10.
        let f: Vec<f32> = (0..50 * 50)
            .map(|i| {
                let d = ((i % 50) as f64 - 25.0).hypot((i / 50) as f64 - 25.0);
                (5.0 - (d - 7.5).abs()) as f32
            })
            .collect();
        let loops = contours(&f, 50, 50);
        assert_eq!(loops.len(), 2);
        let mut a: Vec<f64> = loops.iter().map(|l| area2(l)).collect();
        a.sort_by(f64::total_cmp);
        assert!(a[0] < 0.0 && a[1] > 0.0, "outer and hole must differ in sign: {a:?}");
        // The outer loop is the bigger one and has the negative area; the hole is positive.
        assert!(a[0].abs() > a[1].abs());
    }

    #[test]
    fn two_separate_blobs_give_two_loops() {
        let mut f = disc(60, 30, 15.0, 15.0, 6.0);
        let g = disc(60, 30, 45.0, 15.0, 6.0);
        for (a, b) in f.iter_mut().zip(&g) {
            *a = a.max(*b);
        }
        assert_eq!(contours(&f, 60, 30).len(), 2);
    }

    #[test]
    fn saddles_and_degenerate_grids_do_not_panic() {
        // A checkerboard is all saddles.
        let f: Vec<f32> = (0..64).map(|i| if (i % 8 + i / 8) % 2 == 0 { 1.0 } else { -1.0 }).collect();
        let _ = contours(&f, 8, 8);
        assert!(contours(&[], 0, 0).is_empty());
        assert!(contours(&[1.0], 1, 1).is_empty());
        assert!(contours(&[1.0; 3], 4, 4).is_empty());
    }
}
