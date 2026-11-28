//! The planar-face check made while walking: a planar face [WD 7.8.9.1] must rebuild flat, so
//! a vertex that leaves its face's running plane fails the step that placed it, while the
//! search can still change the fold that caused it. [`super::search::flat`] then judges each
//! finished face against its final plane.

use std::collections::BTreeMap;

use super::normals::NormalArrays;
use super::{V, add, cross, dot, scale, sub, unit};

/// How far, in tolerances, a new vertex may stray from its planar face's running plane at the
/// face's centroid.
const BEND: f64 = 10.0;
/// How much that allowance grows per running face diameter of distance from the centroid: the
/// running plane is tilted by its own rounding, and the tilt grows with distance.
const BEND_SLOPE: f64 = 3.0;

/// Per planar face, the running sum of its triangles' area vectors (oriented to agree), the
/// sum of their corners and their count; and the first of its triangles taller than a
/// tolerance, as walked: its index and its normal oriented by its fold.
#[derive(Clone, Copy, Default)]
struct Plane {
    area: V,
    corners: V,
    count: usize,
    first: Option<(usize, V)>,
}

/// The running planes of a walk's planar faces, kept in step with its triangles. Keyed by face
/// so it holds only the faces walked, whatever their numbers.
#[derive(Clone, Default)]
pub(super) struct Planes {
    faces: BTreeMap<usize, Plane>,
    /// Per triangle walked, the face and contribution it added.
    log: Vec<Option<(usize, V, V)>>,
}

impl Planes {
    /// Checks triangle `ti` with corners `t` against its face's running plane, then adds it.
    /// Triangles at or after `ti` from an earlier, rewound attempt are taken out first, so a
    /// walk rewound to any triangle needs no further bookkeeping. `oriented` is the triangle's
    /// normal as folded, `None` for one no taller than a tolerance.
    ///
    /// # Errors
    ///
    /// When a corner strays more than its allowance (see [`BEND`]).
    pub(super) fn place(&mut self, n: &NormalArrays<'_>, ti: usize, t: [V; 3], tol: f64, oriented: Option<V>) -> Result<(), &'static str> {
        self.truncate(ti);
        let face = n.face_of.get(ti).map(|&f| f as usize);
        let Some(f) = face.filter(|&f| n.planar.get(f).copied().unwrap_or(false)) else {
            self.log.push(None);
            return Ok(());
        };
        let plane = self.faces.get(&f).copied().unwrap_or_default();
        if bends(&plane, t, tol) {
            return Err("a planar face bends while rebuilding");
        }
        let mut area = cross(sub(t[1], t[0]), sub(t[2], t[0]));
        if dot(area, plane.area) < 0.0 {
            area = scale(area, -1.0);
        }
        let corners = add(t[0], add(t[1], t[2]));
        let e = self.faces.entry(f).or_default();
        e.area = add(e.area, area);
        e.corners = add(e.corners, corners);
        e.count = e.count.saturating_add(1);
        if e.first.is_none() {
            e.first = oriented.map(|o| (ti, o));
        }
        self.log.push(Some((f, area, corners)));
        Ok(())
    }

    /// Takes out every triangle at or after `ti`.
    fn truncate(&mut self, ti: usize) {
        while self.log.len() > ti {
            let i = self.log.len().saturating_sub(1);
            if let Some((f, area, corners)) = self.log.pop().flatten()
                && let Some(e) = self.faces.get_mut(&f)
            {
                e.area = sub(e.area, area);
                e.corners = sub(e.corners, corners);
                e.count = e.count.saturating_sub(1);
                if e.first.is_some_and(|(at, _)| at == i) {
                    e.first = None;
                }
            }
        }
    }

    /// The oriented normal of planar face `f`'s first tall triangle, if one was placed before
    /// triangle `ti`. A planar face's one stored normal may be borrowed from a vertex read at
    /// another face, whose sign is that face's, or decoded at a collinear triangle; either way
    /// the face's own first tall triangle orients it.
    pub(super) fn oriented(&self, f: usize, ti: usize) -> Option<V> {
        let (at, n) = self.faces.get(&f)?.first?;
        (at < ti).then_some(n)
    }

    /// Budget elements (8 bytes each) held: 16 per face with its map entry, 8 per triangle
    /// logged.
    pub(super) fn held(&self) -> usize {
        self.faces.len().saturating_mul(16).saturating_add(self.log.len().saturating_mul(8))
    }
}

/// Whether a corner of `t` strays from `plane` by more than its allowance.
fn bends(plane: &Plane, t: [V; 3], tol: f64) -> bool {
    let ww = dot(plane.area, plane.area);
    if plane.count == 0 || ww <= 0.0 {
        return false;
    }
    let normal = unit(plane.area);
    #[allow(clippy::cast_precision_loss)] // A face's triangle count.
    let centre = scale(plane.corners, 1.0 / plane.count.saturating_mul(3) as f64);
    let diameter = ww.sqrt().sqrt();
    t.iter().any(|&v| {
        let off = sub(v, centre);
        let allow = BEND * (1.0 + BEND_SLOPE * dot(off, off).sqrt() / diameter);
        dot(off, normal).abs() > allow * tol
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const FACES: [u32; 4] = [0, 0, 0, 0];

    fn arrays(planar: &[bool]) -> NormalArrays<'_> {
        NormalArrays { bits: 10, binary: &[], angles: &[], planar, face_of: &FACES }
    }

    /// A triangle of the plane `z = h` with its apex at `apex`.
    fn tri(h: f64, apex: V) -> [V; 3] {
        [[0.0, 0.0, h], [100.0, 0.0, h], apex]
    }

    #[test]
    fn a_flat_planar_face_passes() {
        let n = arrays(&[true]);
        let mut p = Planes::default();
        p.place(&n, 0, tri(0.0, [0.0, 100.0, 0.0]), 1.0, None).unwrap();
        p.place(&n, 1, tri(0.0, [100.0, 100.0, 5.0]), 1.0, None).unwrap();
    }

    #[test]
    fn a_vertex_off_its_planar_face_fails() {
        let n = arrays(&[true]);
        let mut p = Planes::default();
        p.place(&n, 0, tri(0.0, [0.0, 100.0, 0.0]), 1.0, None).unwrap();
        assert!(p.place(&n, 1, tri(0.0, [50.0, 30.0, 25.0]), 1.0, None).is_err());
    }

    #[test]
    fn a_curved_face_is_not_checked() {
        let n = arrays(&[false]);
        let mut p = Planes::default();
        p.place(&n, 0, tri(0.0, [0.0, 100.0, 0.0]), 1.0, None).unwrap();
        p.place(&n, 1, tri(0.0, [50.0, 30.0, 500.0]), 1.0, None).unwrap();
    }

    #[test]
    fn the_allowance_grows_with_distance_from_the_centroid() {
        let n = arrays(&[true]);
        let mut p = Planes::default();
        p.place(&n, 0, tri(0.0, [0.0, 100.0, 0.0]), 1.0, None).unwrap();
        // 25 tolerances off is a bend beside the centroid, not 5 diameters away.
        p.place(&n, 1, tri(0.0, [600.0, 30.0, 25.0]), 1.0, None).unwrap();
    }

    #[test]
    fn a_rewound_triangle_leaves_its_face() {
        let n = arrays(&[true]);
        let mut p = Planes::default();
        p.place(&n, 0, tri(0.0, [0.0, 100.0, 0.0]), 1.0, None).unwrap();
        p.place(&n, 1, tri(0.0, [100.0, 100.0, 0.0]), 1.0, None).unwrap();
        // Walked again from triangle 0 on another plane: the rewound two do not bend it.
        p.place(&n, 0, tri(100.0, [0.0, 100.0, 100.0]), 1.0, None).unwrap();
        p.place(&n, 1, tri(100.0, [50.0, 30.0, 100.0]), 1.0, None).unwrap();
    }

    #[test]
    fn a_face_is_oriented_by_its_first_tall_triangle() {
        let n = arrays(&[true]);
        let mut p = Planes::default();
        p.place(&n, 0, tri(0.0, [0.0, 100.0, 0.0]), 1.0, None).unwrap();
        p.place(&n, 1, tri(0.0, [100.0, 100.0, 0.0]), 1.0, Some([0.0, 0.0, -1.0])).unwrap();
        p.place(&n, 2, tri(0.0, [50.0, 100.0, 0.0]), 1.0, Some([0.0, 0.0, 1.0])).unwrap();
        // Not yet visible at the triangle that set it; the first one wins.
        assert_eq!(p.oriented(0, 1), None);
        assert_eq!(p.oriented(0, 3), Some([0.0, 0.0, -1.0]));
        // Rewound past it, the face has no orientation until placed again.
        p.place(&n, 1, tri(0.0, [100.0, 100.0, 0.0]), 1.0, None).unwrap();
        assert_eq!(p.oriented(0, 2), None);
    }

    #[test]
    fn a_high_face_number_holds_only_its_face() {
        let planar = vec![true; (1 << 20) + 1];
        let face_of = [1u32 << 20];
        let n = NormalArrays { bits: 10, binary: &[], angles: &[], planar: &planar, face_of: &face_of };
        let mut p = Planes::default();
        p.place(&n, 0, tri(0.0, [0.0, 100.0, 0.0]), 1.0, None).unwrap();
        assert_eq!(p.held(), 24);
    }
}
