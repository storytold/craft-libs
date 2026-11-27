//! Triangulating one polygon face: a fan for a convex face, ear clipping for a concave one.
//!
//! The face is projected onto the plane of its two minor axes (dropping the largest component
//! of its Newell normal) and clipped there. Work is charged to a budget the caller shares
//! between the faces of a part, so a crafted face cannot make it quadratic in the file's size;
//! when the budget runs out, or no ear can be found (a self-intersecting face), what is left of
//! the face is fanned. A face always gives `corners − 2` triangles, wound as the face is.

use crate::Corner;

/// Steps (ear checks, containment tests, looks for a straight corner) allowed per corner of a
/// part, shared by its faces.
pub(crate) const WORK_PER_CORNER: usize = 256;

/// Relative tolerance under which a turn counts as straight.
const STRAIGHT: f64 = 1e-12;

/// The triangles of one face.
pub(crate) enum FaceTriangles<'a> {
    /// A fan from the first corner; `next` is the corner the next triangle starts its edge at.
    Fan { face: &'a [Corner], next: usize },
    /// Triangles clipped from a concave face.
    Ears(std::vec::IntoIter<[Corner; 3]>),
}

impl Iterator for FaceTriangles<'_> {
    type Item = [Corner; 3];

    fn next(&mut self) -> Option<[Corner; 3]> {
        match self {
            FaceTriangles::Fan { face, next } => {
                let t = [*face.first()?, *face.get(*next)?, *face.get(*next + 1)?];
                *next += 1;
                Some(t)
            }
            FaceTriangles::Ears(it) => it.next(),
        }
    }
}

/// Triangulates `face`, whose corners index `positions`, charging its work to `budget`.
pub(crate) fn face<'a>(face: &'a [Corner], positions: &[[f64; 3]], budget: &mut usize) -> FaceTriangles<'a> {
    let fan = FaceTriangles::Fan { face, next: 1 };
    if face.len() <= 3 {
        return fan;
    }
    let Some(pts) = project(face, positions) else { return fan };
    let area: f64 = (0..pts.len()).map(|i| cross(pts[i], pts[(i + 1) % pts.len()])).sum();
    if !area.is_finite() || area == 0.0 {
        return fan;
    }
    let ring = Ring::new(pts, area.signum());
    if ring.is_convex() {
        return fan;
    }
    FaceTriangles::Ears(ring.clip(face, budget).into_iter())
}

/// The face's corners in 2D, on the plane of the two axes its Newell normal leans on least;
/// `None` when a corner has no position or the face has no finite normal.
fn project(face: &[Corner], positions: &[[f64; 3]]) -> Option<Vec<[f64; 2]>> {
    let p: Vec<[f64; 3]> = face.iter().map(|c| positions.get(c.position as usize).copied()).collect::<Option<_>>()?;
    let mut n = [0.0f64; 3];
    for (i, a) in p.iter().enumerate() {
        let b = p[(i + 1) % p.len()];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let n = n.map(f64::abs);
    if !n.iter().all(|x| x.is_finite()) {
        return None;
    }
    // Drop the dominant axis, keeping the other two in cyclic order so the winding seen from the
    // normal's side is kept; the sign of the projected area decides orientation anyway.
    let (u, v) = if n[0] >= n[1] && n[0] >= n[2] {
        (1, 2)
    } else if n[1] >= n[2] {
        (2, 0)
    } else {
        (0, 1)
    };
    Some(p.iter().map(|q| [q[u], q[v]]).collect())
}

/// The z component of `a × b`.
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// Twice the signed area of triangle `a b c`.
fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// A face's projected corners as a doubly linked ring that ears are clipped from.
struct Ring {
    pts: Vec<[f64; 2]>,
    /// The sign of the face's projected area: turns of this sign are convex.
    sign: f64,
    prev: Vec<usize>,
    next: Vec<usize>,
}

impl Ring {
    fn new(pts: Vec<[f64; 2]>, sign: f64) -> Ring {
        let n = pts.len();
        Ring { pts, sign, prev: (0..n).map(|i| (i + n - 1) % n).collect(), next: (0..n).map(|i| (i + 1) % n).collect() }
    }

    /// The turn at corner `i`, positive when convex; and the tolerance under which it is straight.
    fn turn(&self, i: usize) -> (f64, f64) {
        let (a, b, c) = (self.pts[self.prev[i]], self.pts[i], self.pts[self.next[i]]);
        let (e, f) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
        (cross(e, f) * self.sign, STRAIGHT * e[0].hypot(e[1]) * f[0].hypot(f[1]))
    }

    /// No corner turns the other way. A face that winds round more than once (a pentagram)
    /// counts as convex too and keeps its fan.
    fn is_convex(&self) -> bool {
        (0..self.pts.len()).all(|i| {
            let (t, tol) = self.turn(i);
            t >= -tol
        })
    }

    /// Whether the triangle at corner `i` is an ear: convex, and no corner that is not convex
    /// lies in it or on its edges (corners at the same place as one of its own excepted).
    /// `None` when `budget` runs out.
    fn is_ear(&self, i: usize, reflex: &[usize], budget: &mut usize) -> Option<bool> {
        *budget = budget.checked_sub(1)?;
        let (t, tol) = self.turn(i);
        if t <= tol {
            return Some(false);
        }
        let (a, b, c) = (self.pts[self.prev[i]], self.pts[i], self.pts[self.next[i]]);
        for &r in reflex {
            *budget = budget.checked_sub(1)?;
            let p = self.pts[r];
            let (turn, tol) = self.turn(r);
            if p == a || p == b || p == c || turn > tol {
                continue;
            }
            if self.left_of(a, b, p) && self.left_of(b, c, p) && self.left_of(c, a, p) {
                return Some(false);
            }
        }
        Some(true)
    }

    /// Whether `p` lies on the inner side of edge `a b`, or on it within rounding (a corner on an
    /// ear's edge, after the face is projected, may land a hair outside).
    fn left_of(&self, a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
        let l1 = |u: [f64; 2], v: [f64; 2]| (v[0] - u[0]).abs() + (v[1] - u[1]).abs();
        orient(a, b, p) * self.sign >= -STRAIGHT * l1(a, b) * l1(a, p)
    }

    /// The first straight corner of the `left` corners from `from` on, charging each look to
    /// `budget`.
    fn straight(&self, from: usize, left: usize, budget: &mut usize) -> Option<usize> {
        let mut j = from;
        for _ in 0..left {
            *budget = budget.checked_sub(1)?;
            let (t, tol) = self.turn(j);
            if t.abs() <= tol {
                return Some(j);
            }
            j = self.next[j];
        }
        None
    }

    /// Unlinks corner `i`, returning its triangle.
    fn cut(&mut self, i: usize, face: &[Corner]) -> [Corner; 3] {
        let (a, c) = (self.prev[i], self.next[i]);
        self.next[a] = c;
        self.prev[c] = a;
        [face[a], face[i], face[c]]
    }

    /// Clips ears until a triangle is left. When a whole lap finds no ear, a straight corner is
    /// cut instead (its triangle has no area); failing that, or when the budget runs out, the
    /// rest is fanned.
    fn clip(mut self, face: &[Corner], budget: &mut usize) -> Vec<[Corner; 3]> {
        let mut out = Vec::with_capacity(face.len() - 2);
        let mut removed = vec![false; face.len()];
        let mut reflex: Vec<usize> = (0..face.len()).filter(|&i| self.turn(i).0 <= self.turn(i).1).collect();
        let (mut left, mut i, mut stop) = (face.len(), 0, 0);
        while left > 3 {
            let ear = match self.is_ear(i, &reflex, budget) {
                Some(true) => true,
                Some(false) => {
                    i = self.next[i];
                    if i != stop {
                        continue;
                    }
                    match self.straight(i, left, budget) {
                        Some(j) => {
                            i = j;
                            true
                        }
                        None => false,
                    }
                }
                None => false,
            };
            if !ear {
                break;
            }
            let c = self.next[i];
            out.push(self.cut(i, face));
            removed[i] = true;
            reflex.retain(|&r| !removed[r]);
            left -= 1;
            (i, stop) = (c, c);
        }
        // Fan whatever is left from corner `i`.
        let first = i;
        let mut b = self.next[first];
        while self.next[b] != first {
            out.push([face[first], face[b], face[self.next[b]]]);
            b = self.next[b];
        }
        out
    }
}
