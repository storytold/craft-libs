//! How a walked triangle's stored normal decides its fold [WD 7.8.9, 7.8.9.1].

use super::normals::Orientation;
use super::walk::Walk;
use super::{BLIND_NORMAL, MIN_PLANE_HEIGHT, MIN_SINE, V, cross, dot, plane_height, sub};

impl Walk<'_> {
    /// Whether `t`, read as `o`, folds; `fold` is the default and `thin` whether `t` is
    /// collinear. A record read at `t` decides by its reversed bit unless its normal lies in
    /// `t`'s plane ([`BLIND_NORMAL`]). A planar face whose normal is borrowed or blind is
    /// oriented by its own first tall triangle ([`super::bend::Planes::oriented`]) once one
    /// was walked; before that a borrowed normal is used as read and a blind one leaves the
    /// default. A face normal signals only a triangle taller than [`MIN_PLANE_HEIGHT`] within
    /// the face. A triangle no normal orients is recorded as [`Walk::sliver`].
    pub(super) fn decide(&mut self, o: Orientation, t: [u32; 3], fold: bool, thin: bool) -> Result<bool, &'static str> {
        let ti = self.tris.len();
        let own = match o {
            Orientation::Borrowed(face, _) | Orientation::Blind(face) => self.planes.oriented(face, ti),
            _ => None,
        };
        if let Some(f) = own {
            return self.by_own_face(t, f, fold, thin);
        }
        match o {
            Orientation::Reversed(..) if thin => {
                self.sliver = Some(ti);
                if let Some(r) = self.normals.as_mut() {
                    r.blind(ti);
                }
                Ok(fold)
            }
            Orientation::Reversed(rev, n) => self.by_record(t, rev, n, fold),
            Orientation::Normal(n) | Orientation::Reused(n) | Orientation::Borrowed(_, n) => self.by_normal(t, n, fold, thin),
            Orientation::Blind(_) => {
                self.sliver = Some(ti);
                Ok(fold)
            }
            Orientation::Unknown => Ok(fold),
        }
    }

    /// The corners of `t` and its edges `ab`, `ac`.
    fn edges_of(&self, t: [u32; 3]) -> Result<(V, V, V), &'static str> {
        let (a, b, c) = (self.get(t[0])?, self.get(t[1])?, self.get(t[2])?);
        Ok((sub(b, a), sub(c, a), sub(c, b)))
    }

    /// A record's reversed bit, framed on `(max - min) x (R - min)` [WD 7.8.9.4].
    fn by_record(&mut self, t: [u32; 3], rev: bool, n: V, fold: bool) -> Result<bool, &'static str> {
        let (ab, ac, _) = self.edges_of(t)?;
        let w = cross(ab, ac);
        if dot(n, w).abs() < BLIND_NORMAL * dot(w, w).sqrt() {
            self.sliver = Some(self.tris.len());
            return Ok(fold);
        }
        self.signalled = true;
        Ok(rev != (t[0] > t[1]))
    }

    /// A planar face's normal `f` as its own first tall triangle oriented it.
    fn by_own_face(&mut self, t: [u32; 3], f: V, fold: bool, thin: bool) -> Result<bool, &'static str> {
        let (ab, ac, bc) = self.edges_of(t)?;
        let s = dot(cross(ab, ac), f);
        let short = plane_height(s, f, [ab, ac, bc]) <= MIN_PLANE_HEIGHT * self.tol;
        if s == 0.0 || thin || short {
            self.sliver = Some(self.tris.len());
            return Ok(fold);
        }
        self.signalled = true;
        Ok(s < 0.0)
    }

    /// A face normal or summed corner normals `n`: the winding against it folds.
    fn by_normal(&mut self, t: [u32; 3], n: V, fold: bool, thin: bool) -> Result<bool, &'static str> {
        let (ab, ac, bc) = self.edges_of(t)?;
        let w = cross(ab, ac);
        let sine = (dot(w, w) / (dot(ab, ab) * dot(ac, ac))).sqrt();
        let s = dot(w, n);
        let tall = plane_height(s, n, [ab, ac, bc]) > MIN_PLANE_HEIGHT * self.tol;
        if sine > MIN_SINE && s != 0.0 && !thin && tall {
            self.signalled = true;
            return Ok(s < 0.0);
        }
        self.sliver = Some(self.tris.len());
        Ok(fold)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::normals::NormalArrays;
    use super::*;

    #[test]
    fn a_borrowing_face_signals_only_its_tall_triangles() {
        let faces = [0, 0];
        let n = NormalArrays { bits: 10, binary: &[], angles: &[], planar: &[true], face_of: &faces };
        // Planar face 0, oriented +z by triangle 0, with one triangle walked.
        let mut w = Walk::new(2, usize::MAX, None);
        w.tol = 1.0;
        let (a, b, c) = ([0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]);
        w.pos = vec![a, b, c, [100.0, 0.2, 0.0]];
        w.planes.place(&n, 0, [a, b, c], 1.0, Some([0.0, 0.0, 1.0])).unwrap();
        w.tris.push([0, 1, 2]);
        let borrowed = || Orientation::Borrowed(0, [0.0, 0.0, 1.0]);
        // Wound against the face: tall, so the face's sign folds it.
        assert!(w.decide(borrowed(), [0, 2, 1], false, false).unwrap());
        assert!(w.signalled);
        // 0.2 tolerances tall: a sliver, left at the default.
        w.signalled = false;
        assert!(!w.decide(borrowed(), [0, 3, 1], false, false).unwrap());
        assert!(!w.signalled);
        assert_eq!(w.sliver, Some(1));
    }

    /// A collinear triangle whose record its normal cannot orient is a sliver the retried walk
    /// may invert, as a blind one is.
    #[test]
    fn a_thin_reversed_record_is_a_sliver() {
        let mut w = Walk::new(2, usize::MAX, None);
        w.tris.push([0, 1, 2]);
        assert!(!w.decide(Orientation::Reversed(true, [0.0, 0.0, 1.0]), [0, 2, 3], false, true).unwrap());
        assert!(!w.signalled);
        assert_eq!(w.sliver, Some(1));
    }
}
