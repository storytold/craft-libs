//! Triangle reconstruction for a `TESS_3D_Compressed` (173) [WD 7.8.9, 7.8.9.1-7.8.9.4,
//! 7.8.9.7].
//!
//! The WD names the arrays but not the traversal that wrote them; the rules below are the ones
//! the arrays fit when every slot, reference, point and stored normal is consumed exactly. They
//! are inferred from compressed tessellations and are not WD text.
//!
//! Traversal. A component starts with a seed triangle of three slots; every later triangle
//! takes one slot, its apex. A slot flagged in `point_is_reference` takes the next
//! `point_reference_array` value, a vertex number; any other slot makes the next vertex from
//! the next `point_array` triple. Triangle `[A B C]`, entered across `[A B]`, reads
//! `edge_status & 3`: bit 1 continues across `[C B]`, bit 0 across `[A C]`; with both,
//! `[C B]` is taken and `[A C]` pushed. With neither, pushed edges are popped, skipping any
//! already shared by two triangles; an empty stack starts the next component. A triangle
//! whose fold no stored normal signalled, whose continuation edge is already shared by two
//! triangles while the other edge is open, continues across the open one.
//!
//! Geometry, in units of `tolerance`:
//! - seed: `V0 = origin + d`, `V1 = V0 + d`, `V2 = mid(V0, V1) + d`;
//! - apex across `[P Q]` with the triangle's third vertex `W` [WD 7.8.9.2]: with
//!   `O = (P + Q) * 0.5`, `L` and `H` the lower- and higher-numbered of `P`, `Q`,
//!   `X = unitize(L - H)`, `Z = unitize((W - O) x X)` and `Y = unitize(Z x X)`, the apex is
//!   `((O + d.x X) - d.y Y) - d.z Z`.
//!
//! Arithmetic. The encoder frames each apex on the points it already approximated [WD 7.8.9],
//! so the decoder repeats the encoder's floating point: a last-bit difference compounds from
//! apex to apex. `unitize` therefore matches the WD's `PrcPt::Unitize` pseudocode [WD 12.3] as
//! read, whose length is not exact, and the operations keep the order above.
//!
//! Fold. A triangle folded against its parent swaps bits 1 and 0 for its own continuation. A
//! triangle whose walk-order winding `(B - A) x (C - A)` points against its stored normal
//! [WD 7.8.9, 7.8.9.1] is folded; [`decide`] says which normal applies. Without one, an apex
//! in its parent's plane on the parent's side (`d.z == 0`, `d.y > 0`) is folded. A component
//! that does not fit is walked again with the fold inverted at its second triangle, then at
//! its last unoriented triangle before the failure. A planar face must rebuild flat
//! ([`bend`], [`MAX_BEND`]). A mesh no retried walk fits is searched ([`search`]) and kept
//! only when one geometry fits.
//!
//! Only the three-statuses-per-triangle `edge_status` form is reconstructed (the first `T`
//! are read); the one-per-triangle form walks differently.

use std::cell::Cell;

mod bend;
mod decide;
mod normals;
mod search;
mod walk;

pub(super) use normals::NormalArrays;
use normals::{NormalReader, stored_normals};
use walk::Walk;

type V = [f64; 3];

/// Below this sine of its angle at `A` a triangle is a sliver whose winding no normal can
/// orient [WD 7.8.9].
const MIN_SINE: f64 = 1e-6;

/// Below this height within its planar face, in tolerances, a triangle's winding against the
/// face normal is quantisation noise: the WD quantises each vertex to the tolerance and
/// requires every triangle to be taller than one [WD 7.8.9], so a triangle tall in space but
/// flat along the face cannot be oriented by it.
const MIN_PLANE_HEIGHT: f64 = 0.5;

/// Below this cosine between a fresh record's normal and the triangle's winding, the record's
/// `triangle_normal_reversed` bit is noise: a vertex normal lying in the triangle's plane has
/// no side for the encoder to agree with [WD 7.8.9.4].
const BLIND_NORMAL: f64 = 0.01;

/// How far, in tolerances, a planar face's vertices may stray from the plane through their
/// centroid.
const MAX_BEND: f64 = 20.0;

/// The height of a triangle within the plane of normal `n`: `|w . n| / |n|`, twice its area
/// projected onto that plane, over its longest edge, where `s` is `w . n` for the winding `w`;
/// zero when either length is.
fn plane_height(s: f64, n: V, edges: [V; 3]) -> f64 {
    let long = edges.iter().map(|v| dot(*v, *v).sqrt()).fold(0.0, f64::max);
    let nn = dot(n, n).sqrt();
    if long > 0.0 && nn > 0.0 { s.abs() / nn / long } else { 0.0 }
}

/// `a` over its length, or `a` itself when that is zero.
fn unit(a: V) -> V {
    let l = dot(a, a).sqrt();
    if l > 0.0 { scale(a, 1.0 / l) } else { a }
}

/// The arrays of one `TESS_3D_Compressed` the walk reads.
#[derive(Clone, Copy)]
pub(super) struct Arrays<'a> {
    pub(super) tolerance: f64,
    pub(super) origin: V,
    pub(super) points: &'a [i64],
    pub(super) edge_status: &'a [i32],
    pub(super) triangles: usize,
    pub(super) is_reference: &'a [bool],
    pub(super) references: &'a [u32],
    /// The stored normals, when the mesh has them.
    pub(super) normals: Option<NormalArrays<'a>>,
    /// Whether a `MakeOrthoRep` frame whose residue is exactly zero is turned a half-turn
    /// about `X` (see [`apex`]).
    pub(super) ortho_turned: bool,
    /// Budget elements the walk may hold at once.
    pub(super) limit: usize,
    /// Triangle steps every walk of this rebuild may still take together; each step spends
    /// one.
    pub(super) steps: &'a Cell<u64>,
}

/// The rebuilt vertices and triangles in walk order, and the stored normals with each
/// triangle's corner normals; both empty when the normals are recalculated or do not fit.
#[derive(Debug)]
pub(super) struct Rebuilt {
    pub(super) positions: Vec<V>,
    pub(super) triangles: Vec<[u32; 3]>,
    pub(super) normals: Vec<V>,
    pub(super) triangle_normals: Vec<[u32; 3]>,
}

fn rebuilt(positions: Vec<V>, triangles: Vec<[u32; 3]>, stored: (Vec<V>, Vec<[u32; 3]>)) -> Rebuilt {
    Rebuilt { positions, triangles, normals: stored.0, triangle_normals: stored.1 }
}

/// Rebuilds the mesh, or says why the arrays do not fit the traversal. A mesh with stored
/// normals is walked oriented by them, which must then be consumed exactly too; failing that
/// it is walked on the default fold alone and its normals read afterwards, kept only if they
/// fit; failing that, searched. The WD frame is tried before the turned one
/// ([`Arrays::ortho_turned`]), which is kept only as a fit the stored normals confirm.
pub(super) fn rebuild(a: &Arrays<'_>) -> Result<Rebuilt, &'static str> {
    if a.edge_status.len() != a.triangles.saturating_mul(3) {
        return Err("the one-status-per-triangle edge form is not reconstructed");
    }
    reconstruct_oriented(a)
}

fn reconstruct_oriented(a: &Arrays<'_>) -> Result<Rebuilt, &'static str> {
    if let Some(m) = oriented_fit(a) {
        return Ok(m);
    }
    let turned = Arrays { ortho_turned: true, ..*a };
    match walk(a, None) {
        Ok(w) => {
            if let Some(m) = oriented_fit(&turned) {
                return Ok(m);
            }
            let stored = a.normals.as_ref().and_then(|n| stored_normals(&w.pos, &w.tris, n)).unwrap_or_default();
            Ok(rebuilt(w.pos, w.tris, stored))
        }
        Err(e) => search::unique_fit_either(a, &turned).ok_or(e),
    }
}

/// The mesh the stored normals orient, if every array and normal is consumed exactly.
fn oriented_fit(a: &Arrays<'_>) -> Option<Rebuilt> {
    let w = walk(a, Some(NormalReader::new(a.normals.as_ref()?))).ok()?;
    let stored = w.normals.as_ref()?.finished(&w.pos)?;
    Some(rebuilt(w.pos, w.tris, stored))
}

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: V, s: f64) -> V {
    a.map(|c| c * s)
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn key(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}

/// `PrcPt::Length` [WD 12.3]: the squared length summed in `f64` and rounded to `f32`, then
/// Newton's square root in `f64`, seeded with that `f32` with its exponent halved. The printed
/// loop never advances its iterate; it is read as iterating until two successive values are
/// equal, `None` (the WD's -1) when 100 steps do not settle.
fn wd_length(a: V) -> Option<f64> {
    let squared = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]) as f32;
    let bits = squared.to_bits();
    let exponent = (bits >> 23) & 0xff;
    let seed = if exponent > 127 { f32::from_bits((bits & !(0xff << 23)) | (((exponent - 127) / 2 + 127) << 23)) } else { squared };
    let squared = f64::from(squared);
    let mut x = f64::from(seed);
    for _ in 0..100 {
        let next = 0.5 * (x + squared / x);
        if next == x {
            return Some(next);
        }
        x = next;
    }
    None
}

/// `PrcPt::Unitize` [WD 12.3]: each component divided by [`wd_length`]; `None` below
/// `FLT_EPSILON`.
fn unitize(a: V) -> Option<V> {
    let l = wd_length(a).filter(|&l| l >= f64::from(f32::EPSILON))?;
    Some([a[0] / l, a[1] / l, a[2] / l])
}

/// The `Z` of `PrcPt::MakeOrthoRep` [WD 12.3] on `X`: `unitize(X x (0, 1, 0))`, or
/// `X x (1, 0, 0)` when that is null, from `X` unitized again as the pseudocode does.
fn make_ortho_rep_z(x: V) -> Option<V> {
    let x = unitize(x)?;
    unitize(cross(x, [0.0, 1.0, 0.0])).or_else(|| unitize(cross(x, [1.0, 0.0, 0.0])))
}

/// The apex across `[lo hi]` (`lo` the lower-numbered vertex) opposite `w`, `d` already scaled
/// by the tolerance [WD 7.8.9.2], and whether its frame was degenerate. A null `Z` or `Y` takes
/// `MakeOrthoRep(X)`'s `Z` as the WD prescribes, with `Y = unitize(Z x X)` from the frame's own
/// `X` (not the copy `MakeOrthoRep` unitized again, whose last bits differ), turned a
/// half-turn about `X` when the residue of `(W - O) x X` points against that `Z`, and by
/// `turned` when the residue is exactly zero; `invert` reverses the turn, for the search. A
/// null `X`, an edge the WD's non-degeneracy rule excludes, is used as it is.
fn apex(lo: V, hi: V, w: V, d: V, turned: bool, invert: bool) -> (V, bool) {
    let o = scale(add(lo, hi), 0.5);
    let x = sub(lo, hi);
    let x = unitize(x).unwrap_or(x);
    let residue = cross(sub(w, o), x);
    let frame = unitize(residue).and_then(|z| Some((unitize(cross(z, x))?, z)));
    let degenerate = frame.is_none();
    let (y, z) = frame
        .or_else(|| {
            let z = make_ortho_rep_z(x)?;
            let y = unitize(cross(z, x))?;
            let along = dot(residue, z);
            let turn = if along == 0.0 { turned } else { along < 0.0 };
            Some(if turn != invert { (scale(y, -1.0), scale(z, -1.0)) } else { (y, z) })
        })
        .unwrap_or_default();
    (sub(sub(add(o, scale(x, d[0])), scale(y, d[1])), scale(z, d[2])), degenerate)
}

/// Walks every component, oriented by `normals` when given, and checks every slot, reference
/// and point is consumed.
fn walk<'n>(a: &Arrays<'_>, normals: Option<NormalReader<'n>>) -> Result<Walk<'n>, &'static str> {
    let mut w = Walk::new(a.triangles, a.limit, normals);
    walk_into(a, &mut w)?;
    if !w.used_up(a) {
        return Err("the decoded triangles do not use up the stored arrays");
    }
    Ok(w)
}

/// Walks every component into `w`. One that fails, or whose planar faces bend, is rewound and
/// walked again with the fold inverted at its second triangle, then at the last sliver its
/// first walk met; so each component is walked at most three times.
fn walk_into(a: &Arrays<'_>, w: &mut Walk<'_>) -> Result<(), &'static str> {
    let mut start = w.mark(0);
    let (mut flip_at, mut sliver) = (None, None);
    let mut i = 0;
    while i < a.triangles {
        if w.next.is_none() && !(flip_at.is_some() && i == start.tri) {
            start = w.mark(i);
            flip_at = None;
        }
        let status = a.edge_status.get(i).copied().unwrap_or(0);
        let stepped = w.step(a, status, flip_at == Some(i)).and_then(|()| {
            if w.next.is_none() && !search::flat(a, &w.pos, &w.tris, start.tri) { Err("a planar face does not rebuild flat") } else { Ok(()) }
        });
        match stepped {
            Ok(()) => i = i.saturating_add(1),
            Err(e) => {
                let Some(f) = retry(i, start.tri, flip_at, &mut sliver, w.sliver) else {
                    return Err(e);
                };
                w.rewind(start);
                flip_at = Some(f);
                i = start.tri;
            }
        }
    }
    Ok(())
}

/// Where a component starting at `start` that failed at `i` is next walked with its fold
/// inverted, given where it was inverted (`flip_at`), the sliver saved for the third walk and
/// the walk's `last` sliver.
fn retry(i: usize, start: usize, flip_at: Option<usize>, sliver: &mut Option<usize>, last: Option<usize>) -> Option<usize> {
    let second = start.saturating_add(1);
    match flip_at {
        _ if i == start => None,
        None => {
            *sliver = last.filter(|&s| s > second);
            Some(second)
        }
        Some(f) if f == second => sliver.take(),
        Some(_) => None,
    }
}

#[cfg(test)]
mod independent;
#[cfg(test)]
mod tests;
