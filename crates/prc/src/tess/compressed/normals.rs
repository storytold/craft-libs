//! The stored normals of a `TESS_3D_Compressed`, read triangle by triangle as the walk
//! produces them [WD 7.8.9.3-7.8.9.4].
//!
//! Each triangle `[P Q R]`, entered across `[P Q]`, visits its corners as `min(P, Q)`,
//! `max(P, Q)`, `R`. A vertex met for the first time reads `has_multiple_normal` and a record.
//! Met again, a vertex without multiple normals reads nothing and reuses its normal; one with
//! multiple normals reads `is_a_reference`, then either a record or a reference, an index
//! counted back from the vertex's most recently stored normal. On a planar face only the first
//! corner of its first triangle is visited, and every corner of the face takes that normal. A
//! record is three bits (reversed, x reversed, y reversed) and two angles. These rules are the
//! ones the arrays fit when every bit and angle is consumed exactly; they are not WD text.

use super::{V, add, cross, dot, scale, sub, unit};

/// The stored-normal arrays of a mesh that does not ask for recalculation [WD 7.8.9.3].
#[derive(Clone, Copy)]
pub(in crate::tess) struct NormalArrays<'a> {
    /// `normal_angle_number_of_bits`.
    pub(in crate::tess) bits: u32,
    /// `normal_binary_data`.
    pub(in crate::tess) binary: &'a [bool],
    /// `normal_angle_array`: theta then phi per stored normal.
    pub(in crate::tess) angles: &'a [i32],
    /// `is_face_planar`, per face.
    pub(in crate::tess) planar: &'a [bool],
    /// The face of each triangle.
    pub(in crate::tess) face_of: &'a [u32],
}

/// Width in bits of a normal reference on a vertex that already stores `n` normals: one more
/// than the bit length of `n - 2`, and 1 for `n <= 2`.
fn reference_bits(n: usize) -> u32 {
    1 + (usize::BITS - n.saturating_sub(2).leading_zeros())
}

/// One stored normal record: `triangle_normal_reversed`, `x_is_reversed`, `y_is_reversed`,
/// theta, phi, and the triangle whose frame it is in.
type Record = (bool, bool, bool, i32, i32, [u32; 3]);

/// What a triangle's stored normals say about its winding: the triangle normal is the cross
/// product of its vertices oriented by one of its normals, and left and right for the
/// continuation follow from it [WD 7.8.9, 7.8.9.1].
pub(super) enum Orientation {
    /// A record read at this triangle: its `triangle_normal_reversed`, relative to
    /// `(max - min) x (R - min)` of the triangle's own corners [WD 7.8.9.4], and its normal.
    Reversed(bool, V),
    /// The normal of a planar face read at an earlier triangle.
    Normal(V),
    /// A planar face's normal taken from a vertex whose one normal was read at another face,
    /// with the face: its sign is the other face's and may not be this one's.
    Borrowed(usize, V),
    /// A planar face's normal decoded at a collinear triangle, with the face: its direction is
    /// rounding residue.
    Blind(usize),
    /// The sum of a curved triangle's corner normals, every one read at an earlier triangle.
    Reused(V),
    /// Nothing new was read and no face normal applies.
    Unknown,
}

enum Undo {
    First(u32),
    Pushed(u32),
    Face(usize),
}

/// Reader state to rewind to.
#[derive(Clone, Copy)]
pub(super) struct NormalMark {
    bit: usize,
    angle: usize,
    records: usize,
    corners: usize,
    undo: usize,
}

/// Reads the stored normals one triangle at a time.
pub(super) struct NormalReader<'a> {
    a: &'a NormalArrays<'a>,
    bit: usize,
    angle: usize,
    records: Vec<Record>,
    by_vertex: Vec<(bool, Vec<u32>)>,
    by_face: Vec<Option<u32>>,
    borrowed: Vec<bool>,
    blind: Vec<bool>,
    corners: Vec<[u32; 3]>,
    undo: Vec<Undo>,
}

impl<'a> NormalReader<'a> {
    /// A reader at the start of `a`'s bit and angle streams.
    pub(super) fn new(a: &'a NormalArrays<'a>) -> Self {
        // Per face as many as `is_face_planar` decoded, which the element allowance bounds.
        let faces = a.planar.len();
        Self {
            a,
            bit: 0,
            angle: 0,
            records: Vec::new(),
            by_vertex: Vec::new(),
            by_face: vec![None; faces],
            borrowed: vec![false; faces],
            blind: vec![false; faces],
            corners: Vec::new(),
            undo: Vec::new(),
        }
    }

    pub(super) fn mark(&self) -> NormalMark {
        NormalMark { bit: self.bit, angle: self.angle, records: self.records.len(), corners: self.corners.len(), undo: self.undo.len() }
    }

    /// Returns to `m`, forgetting every vertex and face first read since.
    pub(super) fn rewind(&mut self, m: NormalMark) {
        let undo = self.undo.split_off(m.undo.min(self.undo.len()));
        for u in undo.into_iter().rev() {
            match u {
                Undo::First(v) => {
                    if let Some(s) = self.by_vertex.get_mut(v as usize) {
                        *s = (false, Vec::new());
                    }
                }
                Undo::Pushed(v) => {
                    if let Some(s) = self.by_vertex.get_mut(v as usize) {
                        s.1.pop();
                    }
                }
                Undo::Face(f) => {
                    if let Some(s) = self.by_face.get_mut(f) {
                        *s = None;
                    }
                }
            }
        }
        (self.bit, self.angle) = (m.bit, m.angle);
        self.records.truncate(m.records);
        self.corners.truncate(m.corners);
    }

    /// Budget elements (8 bytes each) the reader holds: a record and its vertex entry take 4,
    /// a vertex 4, a corner triple or an undo entry 2, and a face 2.
    pub(super) fn held(&self) -> usize {
        let records = self.records.len().saturating_mul(4);
        let vertices = self.by_vertex.len().saturating_mul(4);
        let corners = self.corners.len().saturating_add(self.undo.len()).saturating_mul(2);
        let faces = self.by_face.len().saturating_mul(5);
        records.saturating_add(vertices).saturating_add(corners).saturating_add(faces)
    }

    fn bit(&mut self) -> Option<bool> {
        let b = *self.a.binary.get(self.bit)?;
        self.bit = self.bit.saturating_add(1);
        Some(b)
    }

    fn record(&mut self, t: [u32; 3]) -> Option<u32> {
        let r = (self.bit()?, self.bit()?, self.bit()?);
        let th = *self.a.angles.get(self.angle)?;
        let ph = *self.a.angles.get(self.angle.saturating_add(1))?;
        self.angle = self.angle.saturating_add(2);
        let n = u32::try_from(self.records.len()).ok()?;
        self.records.push((r.0, r.1, r.2, th, ph, t));
        Some(n)
    }

    /// The normal at corner `v` of `t`, and whether a record was read.
    fn corner(&mut self, v: u32, t: [u32; 3]) -> Option<(u32, bool)> {
        let vi = v as usize;
        if self.by_vertex.len() <= vi {
            self.by_vertex.resize(vi.saturating_add(1), (false, Vec::new()));
        }
        let (multi, count) = self.by_vertex.get(vi).map(|s| (s.0, s.1.len()))?;
        if count == 0 {
            let m = self.bit()?;
            let n = self.record(t)?;
            *self.by_vertex.get_mut(vi)? = (m, vec![n]);
            self.undo.push(Undo::First(v));
            return Some((n, true));
        }
        if !multi {
            return Some((self.stored(vi, count.saturating_sub(1))?, false));
        }
        if self.bit()? {
            let mut back = 0usize;
            for i in 0..reference_bits(count) {
                if self.bit()? {
                    back |= 1usize.checked_shl(i)?;
                }
            }
            return Some((self.stored(vi, back)?, false));
        }
        let n = self.record(t)?;
        self.by_vertex.get_mut(vi)?.1.push(n);
        self.undo.push(Undo::Pushed(v));
        Some((n, true))
    }

    /// Vertex `vi`'s normal `back` places before its most recent one.
    fn stored(&self, vi: usize, back: usize) -> Option<u32> {
        let s = &self.by_vertex.get(vi)?.1;
        s.get(s.len().checked_sub(back.checked_add(1)?)?).copied()
    }

    /// Reads triangle `ti`, `t`, entered across `[t0 t1]`; `None` when the arrays run out.
    pub(super) fn read(&mut self, ti: usize, t: [u32; 3], pos: &[V]) -> Option<Orientation> {
        let face = *self.a.face_of.get(ti)? as usize;
        let planar = self.a.planar.get(face).copied().unwrap_or(false);
        if planar && let Some(n) = self.by_face.get(face).copied().flatten() {
            return Some(self.face_normal(face, n, pos));
        }
        let lo = usize::from(t[0] > t[1]);
        let (p, q) = if lo == 0 { (t[0], t[1]) } else { (t[1], t[0]) };
        let mut out = [0u32; 3];
        let mut fresh = None;
        for (slot, v) in [(lo, p), (lo ^ 1, q), (2, t[2])] {
            let (n, new) = self.corner(v, t)?;
            if new && fresh.is_none() {
                fresh = self.records.get(n as usize).map(|r| (r.0, decode(pos, r, self.step())));
            }
            if planar {
                *self.by_face.get_mut(face)? = Some(n);
                *self.borrowed.get_mut(face)? = !new;
                *self.blind.get_mut(face)? = false;
                self.undo.push(Undo::Face(face));
                out = [n; 3];
                break;
            }
            *out.get_mut(slot)? = n;
        }
        self.corners.push(out);
        if let Some((rev, n)) = fresh {
            return Some(Orientation::Reversed(rev, n));
        }
        if planar {
            return Some(Orientation::Unknown);
        }
        Some(Orientation::Reused(out.iter().fold([0.0; 3], |s, &c| add(s, self.normal(c, pos)))))
    }

    /// A triangle of planar face `face`, whose one normal `n` was read before.
    fn face_normal(&mut self, face: usize, n: u32, pos: &[V]) -> Orientation {
        self.corners.push([n; 3]);
        let v = self.normal(n, pos);
        if self.blind.get(face).copied().unwrap_or(false) {
            Orientation::Blind(face)
        } else if self.borrowed.get(face).copied().unwrap_or(false) {
            Orientation::Borrowed(face, v)
        } else {
            Orientation::Normal(v)
        }
    }

    /// Marks the planar face of triangle `ti`, whose normal `ti` just decoded, as
    /// [`Orientation::Blind`]: `ti` is collinear.
    pub(super) fn blind(&mut self, ti: usize) {
        if let Some(&f) = self.a.face_of.get(ti)
            && let Some(b) = self.blind.get_mut(f as usize)
        {
            *b = true;
        }
    }

    /// The angle quantum: a quarter turn over the largest `bits`-bit value.
    fn step(&self) -> f64 {
        let levels = 1u32.checked_shl(self.a.bits).map_or(u32::MAX, |l| l.saturating_sub(1)).max(1);
        std::f64::consts::FRAC_PI_2 / f64::from(levels)
    }

    fn normal(&self, n: u32, pos: &[V]) -> V {
        self.records.get(n as usize).map_or([0.0; 3], |r| decode(pos, r, self.step()))
    }

    /// The normals and each triangle's corner normals so far, or `None` when bits or angles are
    /// left over.
    pub(super) fn finished(&self, pos: &[V]) -> Option<(Vec<V>, Vec<[u32; 3]>)> {
        if self.bit != self.a.binary.len() || self.angle != self.a.angles.len() {
            return None;
        }
        let step = self.step();
        Some((self.records.iter().map(|r| decode(pos, r, step)).collect(), self.corners.clone()))
    }
}

/// Reads the stored normals of an already rebuilt mesh, or `None` when the arrays do not fit
/// it.
pub(super) fn stored_normals(pos: &[V], tris: &[[u32; 3]], a: &NormalArrays<'_>) -> Option<(Vec<V>, Vec<[u32; 3]>)> {
    let mut r = NormalReader::new(a);
    for (ti, t) in tris.iter().enumerate() {
        r.read(ti, *t, pos)?;
    }
    r.finished(pos)
}

/// One record's normal in model space: the local frame of the triangle it was read in
/// [WD 7.8.9.4], `Z` its normal (reversed when flagged), and the spherical angles
/// `(cos phi cos theta, cos phi sin theta, sin phi)` with the flagged axes negated. A
/// degenerate triangle gives the zero vector.
fn decode(pos: &[V], r: &Record, step: f64) -> V {
    let &(rev, xr, yr, th, ph, t) = r;
    let (lo, hi) = (t[0].min(t[1]), t[0].max(t[1]));
    let p = |i: u32| pos.get(i as usize).copied().unwrap_or([0.0; 3]);
    let (p0, p1, p2) = (p(lo), p(hi), p(t[2]));
    let (v1, v2, v3) = (unit(sub(p1, p0)), unit(sub(p2, p0)), unit(sub(p2, p1)));
    let half = std::f64::consts::FRAC_PI_2;
    let angle = |a: V, b: V| dot(a, b).clamp(-1.0, 1.0).acos() - half;
    let (t1, t2, t3) = (angle(v1, v2), angle(v3, scale(v1, -1.0)), angle(scale(v2, -1.0), scale(v3, -1.0)));
    let (x, z) = if t1 < t2 && t1 < t3 {
        (v1, cross(v1, v2))
    } else if t2 < t3 {
        (v3, scale(cross(v3, v1), -1.0))
    } else {
        (scale(v2, -1.0), cross(v2, v3))
    };
    let z = unit(z);
    if dot(z, z) < 0.5 {
        return [0.0; 3];
    }
    let z = if rev { scale(z, -1.0) } else { z };
    let y = cross(z, x);
    let (th, ph) = (f64::from(th) * step, f64::from(ph) * step);
    let lx = ph.cos() * th.cos() * if xr { -1.0 } else { 1.0 };
    let ly = ph.cos() * th.sin() * if yr { -1.0 } else { 1.0 };
    add(add(scale(x, lx), scale(y, ly)), scale(z, ph.sin()))
}

#[cfg(test)]
mod tests;
