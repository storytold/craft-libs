//! Bounding volume hierarchy over a mesh's triangles: closest-hit and any-hit ray casts, and
//! box queries (used by picking, baking and brush stamps).
//!
//! Triangles with a non-finite corner are left out, and a query with a non-finite origin,
//! direction or point, a zero direction, a NaN distance or a NaN box finds nothing.

use glam::{DVec3, Vec3, Vec3A};

use crate::Mesh;

#[derive(Clone, Copy, Debug)]
struct Node {
    lo: Vec3A,
    hi: Vec3A,
    /// Leaf: first triangle in `order`; inner: index of the left child (right = left + 1).
    first: u32,
    /// Triangles in a leaf; 0 for inner nodes.
    count: u32,
}

/// A ray hit: distance, triangle and barycentrics (weights of vertices 1 and 2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub t: f32,
    pub tri: u32,
    pub u: f32,
    pub v: f32,
}

#[derive(Clone, Debug)]
pub struct Bvh {
    nodes: Vec<Node>,
    /// Triangle vertices in BVH order.
    tris: Vec<[Vec3; 3]>,
    /// Original triangle index of each entry in `tris`.
    order: Vec<u32>,
}

const LEAF: usize = 4;

/// Most triangles one tree holds: its node indices (up to twice the triangle count) are `u32`.
pub const MAX_BVH_TRIANGLES: usize = (u32::MAX / 2) as usize;

/// Traversal stack of (node, entry distance): inline for typical depths (no allocation per
/// ray), spilling to the heap for degenerate trees.
struct Stack {
    inline: [(u32, f32); 64],
    len: usize,
    spill: Vec<(u32, f32)>,
}

impl Default for Stack {
    fn default() -> Self {
        Stack { inline: [(0, 0.0); 64], len: 0, spill: Vec::new() }
    }
}

impl Stack {
    fn push(&mut self, v: (u32, f32)) {
        match self.inline.get_mut(self.len) {
            Some(slot) if self.spill.is_empty() => {
                *slot = v;
                self.len += 1;
            }
            _ => self.spill.push(v),
        }
    }

    fn pop(&mut self) -> Option<(u32, f32)> {
        self.spill.pop().or_else(|| {
            self.len = self.len.checked_sub(1)?;
            self.inline.get(self.len).copied()
        })
    }
}
const BINS: usize = 12;

fn grow(lo: &mut Vec3, hi: &mut Vec3, p: Vec3) {
    *lo = lo.min(p);
    *hi = hi.max(p);
}

fn area(lo: Vec3, hi: Vec3) -> f32 {
    let d = (hi - lo).max(Vec3::ZERO);
    d.x * d.y + d.y * d.z + d.z * d.x
}

/// The centroid of a triangle; thirds first when the sum would overflow.
fn centroid(t: &[Vec3; 3]) -> Vec3 {
    let c = (t[0] + t[1] + t[2]) / 3.0;
    if c.is_finite() { c } else { t[0] / 3.0 + t[1] / 3.0 + t[2] / 3.0 }
}

impl Bvh {
    /// The tree over a mesh's triangles. Triangles with a non-finite corner or an index out of
    /// range are left out, as is every triangle past [`MAX_BVH_TRIANGLES`].
    pub fn new(mesh: &Mesh) -> Bvh {
        let triangle_count = mesh.triangle_count().min(MAX_BVH_TRIANGLES);
        let mut tris = Vec::with_capacity(triangle_count);
        let mut ids: Vec<u32> = Vec::with_capacity(triangle_count);

        for triangle in 0..triangle_count {
            let Some([a, b, c]) = mesh.tri(triangle) else { continue };
            let (Some(a), Some(b), Some(c)) = (mesh.positions.get(a), mesh.positions.get(b), mesh.positions.get(c)) else { continue };
            if a.is_finite() && b.is_finite() && c.is_finite() {
                tris.push([*a, *b, *c]);
                ids.push(triangle as u32);
            }
        }

        if tris.is_empty() {
            return Bvh { nodes: Vec::new(), tris, order: ids };
        }

        let mut order: Vec<u32> = (0..tris.len() as u32).collect();
        let centroids: Vec<Vec3> = tris.iter().map(centroid).collect();

        let mut nodes = vec![Node { lo: Vec3A::ZERO, hi: Vec3A::ZERO, first: 0, count: tris.len() as u32 }];
        let mut stack = vec![0usize];

        while let Some(node_index) = stack.pop() {
            let Node { first, count, .. } = nodes[node_index];
            let (start, count) = (first as usize, count as usize);
            let (mut bounds_min, mut bounds_max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let (mut centroid_min, mut centroid_max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));

            for &triangle in &order[start..start + count] {
                for position in tris[triangle as usize] {
                    grow(&mut bounds_min, &mut bounds_max, position);
                }
                grow(&mut centroid_min, &mut centroid_max, centroids[triangle as usize]);
            }

            nodes[node_index].lo = bounds_min.into();
            nodes[node_index].hi = bounds_max.into();

            if count <= LEAF {
                continue;
            }

            let centroid_extent = centroid_max - centroid_min;
            let bin_of = |triangle: u32, axis: usize| {
                let offset = centroids[triangle as usize][axis] - centroid_min[axis];
                ((offset / centroid_extent[axis] * BINS as f32) as usize).min(BINS - 1)
            };

            // Reuse each triangle's bounds across all three axes
            let mut axis_bins = [[(0u32, Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)); BINS]; 3];

            for &triangle in &order[start..start + count] {
                let [a, b, c] = tris[triangle as usize];
                let (triangle_min, triangle_max) = (a.min(b).min(c), a.max(b).max(c));

                for axis in 0..3 {
                    if centroid_extent[axis] <= 1e-12 {
                        continue;
                    }

                    let bin = &mut axis_bins[axis][bin_of(triangle, axis)];
                    bin.0 += 1;
                    bin.1 = bin.1.min(triangle_min);
                    bin.2 = bin.2.max(triangle_max);
                }
            }

            let mut best_axis = 0;
            let mut best_split = 0;
            let mut best_cost = f32::MAX;

            for axis in 0..3 {
                if centroid_extent[axis] <= 1e-12 {
                    continue;
                }

                let bins = axis_bins[axis];

                // Cache right bounds once and advance the left bounds per split
                let mut suffix = bins;
                for i in (0..BINS - 1).rev() {
                    suffix[i] = (suffix[i].0 + suffix[i + 1].0, suffix[i].1.min(suffix[i + 1].1), suffix[i].2.max(suffix[i + 1].2));
                }

                let (mut left_count, mut left_min, mut left_max) = (0u32, Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));

                for split in 1..BINS {
                    let bin = bins[split - 1];
                    left_count += bin.0;
                    left_min = left_min.min(bin.1);
                    left_max = left_max.max(bin.2);

                    let (right_count, right_min, right_max) = suffix[split];
                    if left_count == 0 || right_count == 0 {
                        continue;
                    }

                    let cost = left_count as f32 * area(left_min, left_max) + right_count as f32 * area(right_min, right_max);
                    if cost < best_cost {
                        best_cost = cost;
                        best_axis = axis;
                        best_split = split;
                    }
                }
            }

            let node_order = &mut order[start..start + count];
            let left_count = if best_split == 0 || best_cost >= count as f32 * area(bounds_min, bounds_max) {
                // Split by count when SAH cannot help, including tied centroids
                let axis = if centroid_extent.x >= centroid_extent.y && centroid_extent.x >= centroid_extent.z {
                    0
                } else if centroid_extent.y >= centroid_extent.z {
                    1
                } else {
                    2
                };

                let mid = count / 2;
                node_order.select_nth_unstable_by(mid, |&a, &b| centroids[a as usize][axis].total_cmp(&centroids[b as usize][axis]));
                mid
            } else {
                let mut left_count = 0;

                for i in 0..node_order.len() {
                    if bin_of(node_order[i], best_axis) < best_split {
                        node_order.swap(left_count, i);
                        left_count += 1;
                    }
                }

                left_count
            };

            if left_count == 0 || left_count == count {
                continue;
            }

            let left_child = nodes.len();
            nodes.push(Node { lo: Vec3A::ZERO, hi: Vec3A::ZERO, first, count: left_count as u32 });
            nodes.push(Node { lo: Vec3A::ZERO, hi: Vec3A::ZERO, first: first + left_count as u32, count: (count - left_count) as u32 });

            nodes[node_index].first = left_child as u32;
            nodes[node_index].count = 0;

            stack.push(left_child);
            stack.push(left_child + 1);
        }

        let sorted = order.iter().map(|t| tris[*t as usize]).collect();
        // The triangle indices replace the positions in `order` one by one.
        for t in &mut order {
            *t = ids[*t as usize];
        }
        Bvh { nodes, tris: sorted, order }
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// The flattened tree (for GPU traversal), root first: bounds, then for a leaf the range
    /// `first..first + count` of [`Bvh::triangles`], for an inner node (`count` 0) the children
    /// `first` and `first + 1`. A tree without triangles has no nodes.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = (Vec3, Vec3, u32, u32)> + '_ {
        self.nodes.iter().map(|n| (n.lo.into(), n.hi.into(), n.first, n.count))
    }

    /// Triangle vertices in BVH order, and the original triangle index of each.
    pub fn triangles(&self) -> (&[[Vec3; 3]], &[u32]) {
        (&self.tris, &self.order)
    }

    fn slab(lo: Vec3A, hi: Vec3A, o: Vec3A, inv: DVec3, tmax: f32) -> Option<f32> {
        // The differences are taken in f64: `lo - o` of two finite f32 values can exceed f32::MAX.
        // `inv` is the f64 reciprocal of the direction, since 1 / d overflows f32 for |d| < 2.9e-39.
        let o = o.as_dvec3();
        let t0 = (lo.as_dvec3() - o) * inv;
        let t1 = (hi.as_dvec3() - o) * inv;
        // A ray lying in a slab plane gives 0·∞ = NaN: it is inside that slab, so the axis does
        // not bound the hit.
        let open = t0.is_nan_mask() | t1.is_nan_mask();
        let tn = DVec3::select(open, DVec3::NEG_INFINITY, t0.min(t1)).max_element().max(0.0);
        let tf = DVec3::select(open, DVec3::INFINITY, t0.max(t1)).min_element().min(f64::from(tmax));
        let margin = 4.0 * f64::from(f32::EPSILON) * tn.abs().max(tf.abs()).min(f64::MAX);
        if tn - margin > tf + margin {
            return None;
        }
        // The entry distance is rounded down to f32 so a node is never entered late.
        let entry = (tn - margin).max(0.0);
        let mut t = entry as f32;
        if f64::from(t) > entry {
            t = t.next_down();
        }
        Some(t)
    }

    /// Möller–Trumbore, double sided.
    fn hit_tri(t: &[Vec3; 3], o: Vec3, d: Vec3) -> Option<(f32, f32, f32)> {
        const NEAR: f32 = 1e-4;
        let e1 = t[1] - t[0];
        let e2 = t[2] - t[0];
        let p = d.cross(e2);
        let det = e1.dot(p);
        if !det.is_finite() || det.abs() <= 1e-6 * e1.length() * p.length() {
            return Self::hit_tri_wide(t, o, d);
        }
        let inv = 1.0 / det;
        if !inv.is_finite() {
            return Self::hit_tri_wide(t, o, d);
        }
        let s = o - t[0];
        let u = s.dot(p) * inv;
        let q = s.cross(e1);
        let v = d.dot(q) * inv;
        let w = 1.0 - u - v;
        if !(u.is_finite() && v.is_finite() && w.is_finite()) {
            return Self::hit_tri_wide(t, o, d);
        }
        if u.min(v).min(w) < -NEAR {
            return None;
        }
        if u.min(v).min(w) <= NEAR {
            return Self::hit_tri_wide(t, o, d);
        }
        let tt = e2.dot(q) * inv;
        if !tt.is_finite() {
            return Self::hit_tri_wide(t, o, d);
        }
        (tt > 0.0).then_some((tt, u, v))
    }

    fn hit_tri_wide(t: &[Vec3; 3], o: Vec3, d: Vec3) -> Option<(f32, f32, f32)> {
        const EDGE: f64 = 1e-9;
        let (a, b, c) = (t[0].as_dvec3(), t[1].as_dvec3(), t[2].as_dvec3());
        let (o, d) = (o.as_dvec3(), d.as_dvec3());
        let (e1, e2) = (b - a, c - a);
        let p = d.cross(e2);
        let det = e1.dot(p);
        let products = e1.abs().dot(glam::DVec3::new(
            (d.y * e2.z).abs() + (d.z * e2.y).abs(),
            (d.z * e2.x).abs() + (d.x * e2.z).abs(),
            (d.x * e2.y).abs() + (d.y * e2.x).abs(),
        ));
        if det == 0.0 || det.abs() <= 8.0 * f64::EPSILON * products {
            return None;
        }
        let inv = 1.0 / det;
        let s = o - a;
        let u = s.dot(p) * inv;
        let q = s.cross(e1);
        let v = d.dot(q) * inv;
        if u < -EDGE || v < -EDGE || u + v > 1.0 + EDGE {
            return None;
        }
        let tt = e2.dot(q) * inv;
        let distance = tt as f32;
        (distance > 0.0 && distance.is_finite()).then_some((distance, u as f32, v as f32))
    }

    fn traverse(&self, o: Vec3, d: Vec3, mut tmax: f32, any: bool, filter: &dyn Fn(u32) -> bool) -> Option<Hit> {
        if !o.is_finite() || !d.is_finite() || d == Vec3::ZERO || tmax.is_nan() {
            return None;
        }
        let root = self.nodes.first().filter(|_| !self.tris.is_empty())?;
        let (oa, inv) = (Vec3A::from(o), DVec3::ONE / d.as_dvec3());
        let mut best = None;
        let mut stack = Stack::default();
        stack.push((0, Self::slab(root.lo, root.hi, oa, inv, tmax)?));
        // Entries carry their entry distance: nodes behind a closer hit are skipped unopened.
        while let Some((ni, tn)) = stack.pop() {
            let Some(n) = self.nodes.get(ni as usize).filter(|_| tn <= tmax) else { continue };
            if n.count > 0 {
                let (f, c) = (n.first as usize, n.count as usize);
                for i in f..(f + c).min(self.tris.len()) {
                    let (Some(tri), Some(&orig)) = (self.tris.get(i), self.order.get(i)) else { continue };
                    if let Some((t, u, v)) = Self::hit_tri(tri, o, d)
                        && t < tmax
                        && filter(orig)
                    {
                        tmax = t;
                        best = Some(Hit { t, tri: orig, u, v });
                        if any {
                            return best;
                        }
                    }
                }
            } else {
                // Visit the nearer child first.
                let (l, r) = (n.first, n.first + 1);
                let dl = self.nodes.get(l as usize).and_then(|c| Self::slab(c.lo, c.hi, oa, inv, tmax));
                let dr = self.nodes.get(r as usize).and_then(|c| Self::slab(c.lo, c.hi, oa, inv, tmax));
                match (dl, dr) {
                    (Some(a), Some(b)) if a <= b => {
                        stack.push((r, b));
                        stack.push((l, a));
                    }
                    (Some(a), Some(b)) => {
                        stack.push((l, a));
                        stack.push((r, b));
                    }
                    (Some(a), None) => stack.push((l, a)),
                    (None, Some(b)) => stack.push((r, b)),
                    (None, None) => {}
                }
            }
        }
        best
    }

    /// Closest hit along `o + t·d` for `t` in (0, tmax).
    pub fn closest(&self, o: Vec3, d: Vec3, tmax: f32) -> Option<Hit> {
        self.traverse(o, d, tmax, false, &|_| true)
    }

    /// Closest hit on triangles accepted by `filter` (original triangle index).
    pub fn closest_filtered(&self, o: Vec3, d: Vec3, tmax: f32, filter: &dyn Fn(u32) -> bool) -> Option<Hit> {
        self.traverse(o, d, tmax, false, filter)
    }

    /// Whether anything is hit before `tmax` (occlusion rays).
    pub fn occluded(&self, o: Vec3, d: Vec3, tmax: f32) -> bool {
        self.traverse(o, d, tmax, true, &|_| true).is_some()
    }

    /// The point nearest to `p` within `max_dist` on a triangle accepted by `filter` (original
    /// index): a [`Hit`] with the distance in `t` and the barycentrics of vertices 1 and 2.
    pub fn nearest(&self, p: Vec3, max_dist: f32, filter: &dyn Fn(u32) -> bool) -> Option<Hit> {
        let root = self.nodes.first().filter(|_| !self.tris.is_empty() && p.is_finite())?;
        // Squared distances are taken in f64: past about 1.8e19 they overflow f32.
        let pd = p.as_dvec3();
        let box_d2 = |n: &Node| (n.lo.as_dvec3() - pd).max(pd - n.hi.as_dvec3()).max(DVec3::ZERO).length_squared();
        // The stack keeps f32 box distances, rounded down so no node is skipped early.
        let down = |d2: f64| {
            let d = d2.sqrt();
            let mut t = d as f32;
            if f64::from(t) > d {
                t = t.next_down();
            }
            t
        };
        let max_dist = f64::from(max_dist);
        let (mut best_d2, mut best) = (max_dist * max_dist, None);
        let mut stack = Stack::default();
        stack.push((0, down(box_d2(root))));
        while let Some((ni, d)) = stack.pop() {
            let Some(n) = self.nodes.get(ni as usize).filter(|_| f64::from(d) * f64::from(d) <= best_d2) else { continue };
            if n.count > 0 {
                for i in n.first as usize..(n.first as usize + n.count as usize).min(self.tris.len()) {
                    let (Some(tri), Some(&orig)) = (self.tris.get(i), self.order.get(i)) else { continue };
                    if !filter(orig) {
                        continue;
                    }
                    let corners = [tri[0].as_dvec3(), tri[1].as_dvec3(), tri[2].as_dvec3()];
                    let (q, u, v) = closest_on_triangle(pd, corners);
                    let dd = (q - pd).length_squared();
                    if dd < best_d2 {
                        best_d2 = dd;
                        best = Some(Hit { t: dd.sqrt() as f32, tri: orig, u: u as f32, v: v as f32 });
                    }
                }
            } else {
                // Push the farther child first so the nearer one is searched first.
                let child = |c: u32| self.nodes.get(c as usize).map(|n| (c, down(box_d2(n))));
                let (Some(l), Some(r)) = (child(n.first), child(n.first + 1)) else { continue };
                let (near, far) = if l.1 <= r.1 { (l, r) } else { (r, l) };
                stack.push(far);
                stack.push(near);
            }
        }
        best
    }

    /// Call `f` with every triangle (original index) whose bounds overlap the box. A box with a
    /// NaN coordinate, or a lower corner past the upper corner on any axis, overlaps nothing.
    pub fn query_box(&self, lo: Vec3, hi: Vec3, mut f: impl FnMut(u32)) {
        if lo.is_nan() || hi.is_nan() || lo.cmpgt(hi).any() {
            return;
        }
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let Some(n) = self.nodes.get(i as usize) else { continue };
            if self.tris.is_empty() || n.hi.cmplt(lo.into()).any() || n.lo.cmpgt(hi.into()).any() {
                continue;
            }
            if n.count > 0 {
                for k in n.first as usize..(n.first + n.count) as usize {
                    let (Some(t), Some(o)) = (self.tris.get(k), self.order.get(k)) else { continue };
                    let tlo = t[0].min(t[1]).min(t[2]);
                    let thi = t[0].max(t[1]).max(t[2]);
                    if !(thi.cmplt(lo).any() || tlo.cmpgt(hi).any()) {
                        f(*o);
                    }
                }
            } else {
                stack.push(n.first);
                stack.push(n.first + 1);
            }
        }
    }
}

/// The point of triangle `t` nearest to `p` and its barycentrics (weights of vertices 1 and 2),
/// by Voronoi region (Ericson, Real-Time Collision Detection, 5.1.5). Degenerate triangles give
/// a non-finite distance and are never nearest. The region tests multiply four coordinates,
/// which overflows f32 for finite inputs past about 1e9, so the test runs in f64. The signed
/// areas `va`, `vb` and `vc` are computed as `n.dot(x.cross(y))` with `n = ab.cross(ac)`, which equals
/// Ericson's dot-product differences (Binet-Cauchy) but does not cancel two nearly equal
/// products on long, thin triangles.
fn closest_on_triangle(p: DVec3, t: [DVec3; 3]) -> (DVec3, f64, f64) {
    let [a, b, c] = t;
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let n = ab.cross(ac);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, 0.0, 0.0);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return (b, 1.0, 0.0);
    }
    let vc = n.dot(ap.cross(bp));
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, v, 0.0);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return (c, 0.0, 1.0);
    }
    let vb = n.dot(cp.cross(ap));
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, 0.0, w);
    }
    let va = n.dot(bp.cross(cp));
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + (c - b) * w, 1.0 - w, w);
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    (a + ab * v + ac * w, v, w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    fn review_triangle(points: [Vec3; 3]) -> Bvh {
        Bvh::new(&Mesh { positions: points.to_vec(), indices: vec![0, 1, 2], ..Mesh::default() })
    }

    #[test]
    fn review_slab_accepts_an_exact_vertex_hit() {
        let bvh = review_triangle([Vec3::ZERO, Vec3::X, Vec3::Y]);
        let hit = bvh.closest(Vec3::new(-1.0, 0.0, 41.0), Vec3::new(1.0, 0.0, -41.0), 2.0).unwrap();
        assert!((hit.t - 1.0).abs() < 1e-6);
        assert!(bvh.closest(Vec3::new(-1.0, -0.1, 41.0), Vec3::new(1.0, 0.0, -41.0), 2.0).is_none());
    }

    #[test]
    fn nearest_point_survives_products_that_overflow_f32() {
        // The region tests multiply four coordinates: (2.5e9)^2 · (1e10)^2 is past f32::MAX.
        let bvh = review_triangle([Vec3::ZERO, Vec3::X * 1e10, Vec3::Y * 1e10]);
        let hit = bvh.nearest(Vec3::new(2.5e9, 2.5e9, 1.0), 10.0, &|_| true).unwrap();
        assert!((hit.t - 1.0).abs() < 1e-3, "t = {}", hit.t);
        assert!((hit.u - 0.25).abs() < 1e-6 && (hit.v - 0.25).abs() < 1e-6, "u = {}, v = {}", hit.u, hit.v);
    }

    #[test]
    fn slab_accepts_a_direction_whose_f32_reciprocal_overflows() {
        let bvh = review_triangle([Vec3::ZERO, Vec3::X, Vec3::Y]);
        let hit = bvh.closest(Vec3::new(0.25, 0.25, -1e-39), Vec3::new(0.0, 0.0, 1e-39), 2.0).unwrap();
        assert!((hit.t - 1.0).abs() < 1e-5, "t = {}", hit.t);
    }

    #[test]
    fn nearest_point_stays_inside_a_long_thin_triangle() {
        // The dot-product form of vc cancels to zero here and projects onto AB at distance 0.25.
        let s = 2.0_f32.powi(65);
        let bvh = review_triangle([Vec3::ZERO, Vec3::new(s, 0.0, 0.0), Vec3::new(s, 1.0, 0.0)]);
        let hit = bvh.nearest(Vec3::new(s / 2.0, 0.25, 0.0), 0.125, &|_| true).unwrap();
        assert!(hit.t.abs() < 1e-6, "t = {}", hit.t);
        assert!((hit.u - 0.25).abs() < 1e-6 && (hit.v - 0.25).abs() < 1e-6, "u = {}, v = {}", hit.u, hit.v);
    }

    #[test]
    fn nearest_point_finds_distances_whose_square_overflows_f32() {
        let bvh = review_triangle([Vec3::ZERO, Vec3::X, Vec3::Y]);
        let hit = bvh.nearest(Vec3::new(0.25, 0.25, 1e20), 2e20, &|_| true).unwrap();
        assert!((hit.t / 1e20 - 1.0).abs() < 1e-6, "t = {}", hit.t);
        assert!(bvh.nearest(Vec3::new(0.25, 0.25, 1e20), 9e19, &|_| true).is_none());
    }

    #[test]
    fn slab_distances_that_overflow_f32_still_enter_the_box() {
        // lo.z - o.z is -4e38, past f32::MAX, while the hit distance 1e38 fits.
        let z = 2.0e38_f32;
        let bvh = review_triangle([Vec3::new(0.0, 0.0, z), Vec3::new(1.0, 0.0, z), Vec3::new(0.0, 1.0, z)]);
        let hit = bvh.closest(Vec3::new(0.25, 0.25, -z), Vec3::new(0.0, 0.0, 4.0), f32::MAX).unwrap();
        assert!((hit.t / 1.0e38 - 1.0).abs() < 1e-5, "t = {}", hit.t);
    }

    #[test]
    fn review_ray_intersections_preserve_large_scales() {
        let scale = 2.0_f32.powi(65);
        let bvh = review_triangle([Vec3::ZERO, Vec3::X * scale, Vec3::Z * scale]);
        let hit = bvh.closest(Vec3::new(scale / 4.0, 1.0, scale / 4.0), -Vec3::Y, 2.0).unwrap();
        assert_eq!((hit.t, hit.u, hit.v), (1.0, 0.25, 0.25));
    }

    #[test]
    fn review_ray_intersections_preserve_small_scales() {
        let scale = 2.0_f32.powi(-65);
        let bvh = review_triangle([Vec3::ZERO, Vec3::X * scale, Vec3::Z * scale]);
        let hit = bvh.closest(Vec3::new(scale / 4.0, 1.0, scale / 4.0), -Vec3::Y, 2.0).unwrap();
        assert_eq!((hit.t, hit.u, hit.v), (1.0, 0.25, 0.25));
    }

    #[test]
    fn positive_ray_distances_that_underflow_are_not_zero_hits() {
        let bvh = review_triangle([Vec3::ZERO, Vec3::X, Vec3::Y]);
        assert!(bvh.closest(Vec3::new(0.0, 0.25, 1e-10), Vec3::new(0.0, 0.0, -f32::MAX), 1.0).is_none());
    }

    #[test]
    fn nearest_point_matches_brute_force() {
        let m = primitives::builtin("torus").unwrap();
        let bvh = Bvh::new(&m);
        let mut seed = 7u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 4.0 - 2.0
        };
        for _ in 0..200 {
            let p = Vec3::new(rnd(), rnd(), rnd());
            let hit = bvh.nearest(p, f32::MAX, &|_| true).unwrap();
            let brute = (0..m.triangle_count())
                .map(|t| {
                    let [a, b, c] = m.tri(t).unwrap().map(|i| m.positions[i]);
                    (closest_on_triangle(p.as_dvec3(), [a.as_dvec3(), b.as_dvec3(), c.as_dvec3()]).0 - p.as_dvec3()).length() as f32
                })
                .fold(f32::MAX, f32::min);
            assert!((hit.t - brute).abs() < 1e-5, "{} vs {brute}", hit.t);
            let [a, b, c] = m.tri(hit.tri as usize).unwrap().map(|i| m.positions[i]);
            let q = a * (1.0 - hit.u - hit.v) + b * hit.u + c * hit.v;
            assert!(((q - p).length() - hit.t).abs() < 1e-4);
        }
        assert!(bvh.nearest(Vec3::splat(100.0), 1.0, &|_| true).is_none(), "out of range");
        assert!(bvh.nearest(Vec3::ZERO, f32::MAX, &|_| false).is_none(), "filtered out");
    }

    #[test]
    fn traversal_stack_spills_in_lifo_order() {
        let mut st = Stack::default();
        for v in 0..200 {
            st.push((v, v as f32));
        }
        assert!((0..200).rev().all(|v| st.pop() == Some((v, v as f32))));
        assert_eq!(st.pop(), None);
    }

    #[test]
    fn ray_hits_sphere_at_expected_distance() {
        let m = primitives::sphere(1.0, 64, 32);
        let bvh = Bvh::new(&m);
        let h = bvh.closest(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z, 100.0).unwrap();
        assert!((h.t - 4.0).abs() < 0.01, "{}", h.t);
        assert!(bvh.closest(Vec3::new(0.0, 3.0, 5.0), Vec3::NEG_Z, 100.0).is_none());
        // Rays lying exactly in a node's bounding plane (y = 0 is the equator) hit the front.
        // Axis-aligned rays lying in a node's bounding plane (y = 0) still find the front.
        for x in [-0.5f32, -0.4, 0.3] {
            let h = bvh.closest(Vec3::new(x, 0.0, 5.0), Vec3::NEG_Z, 100.0).unwrap();
            assert!(h.t < 5.0, "x {x}: hit the back at t {}", h.t);
        }
        // Rays through shared edges (the equator) hit the front, never slip through to the back.
        let m = primitives::sphere(1.0, 48, 24);
        let bvh = Bvh::new(&m);
        for i in 0..200 {
            let x = i as f32 / 100.0 - 1.0;
            let o = Vec3::new(0.0, 0.0, 5.0);
            let d = (Vec3::new(x * 0.9, 0.0, 0.0) - o).normalize();
            let h = bvh.closest(o, d, 100.0).unwrap();
            assert!(h.t < 5.0, "x {x}: hit the back at t {}", h.t);
        }
        assert!(bvh.occluded(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z, 10.0));
        assert!(!bvh.occluded(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z, 3.0));
    }

    #[test]
    fn matches_brute_force() {
        let m = primitives::material_ball();
        let bvh = Bvh::new(&m);
        for i in 0..200 {
            let a = i as f32 * 0.731;
            let o = Vec3::new(a.cos() * 6.0, (i as f32 * 0.37).sin() * 3.0 + 1.0, a.sin() * 6.0);
            let d = (Vec3::new(0.0, 1.0, 0.0) + Vec3::new((a * 3.1).sin(), (a * 1.7).cos(), (a * 2.3).sin()) * 0.6 - o).normalize();
            let fast = bvh.closest(o, d, 1e9).map(|h| h.t);
            let mut slow: Option<f32> = None;
            for t in 0..m.triangle_count() {
                let [x, y, z] = m.tri(t).unwrap();
                if let Some((tt, _, _)) = Bvh::hit_tri(&[m.positions[x], m.positions[y], m.positions[z]], o, d) {
                    slow = Some(slow.map_or(tt, |s: f32| s.min(tt)));
                }
            }
            match (fast, slow) {
                (Some(a), Some(b)) => assert!((a - b).abs() < 1e-4, "{a} {b}"),
                (None, None) => {}
                other => panic!("mismatch {other:?}"),
            }
        }
    }

    #[test]
    fn box_query_finds_overlapping_triangles() {
        let m = primitives::plane(2.0, 4);
        let bvh = Bvh::new(&m);
        let mut found = Vec::new();
        bvh.query_box(Vec3::new(-0.1, -0.1, -0.1), Vec3::new(0.1, 0.1, 0.1), |t| found.push(t));
        assert!(!found.is_empty() && found.len() <= 8, "{found:?}");
        let empty = Bvh::new(&Mesh::default());
        assert!(empty.closest(Vec3::ZERO, Vec3::X, 1.0).is_none());
    }

    #[test]
    fn large_coplanar_plate_does_not_degrade_leaf_sizes() {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        let w = 200.0f32;
        let h = 100.0f32;

        // Large plate at z = 0
        let b = positions.len() as u32;
        positions.extend_from_slice(&[Vec3::new(0.0, 0.0, 0.0), Vec3::new(w, 0.0, 0.0), Vec3::new(w, h, 0.0), Vec3::new(0.0, h, 0.0)]);
        indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);

        // 500 small triangles sitting at z = 0.5
        for i in 0..500 {
            let x = 20.0 + (i % 160) as f32;
            let y = 10.0 + ((i / 160) % 80) as f32;
            let idx = positions.len() as u32;
            positions.push(Vec3::new(x, y, 0.5));
            positions.push(Vec3::new(x + 0.5, y, 0.5));
            positions.push(Vec3::new(x, y + 0.5, 0.6));
            indices.extend_from_slice(&[idx, idx + 1, idx + 2]);
        }

        let mesh = Mesh {
            positions,
            normals: Vec::new(),
            uvs: Vec::new(),
            indices,
            tangents: Vec::new(),
            colors: Vec::new(),
            submeshes: Vec::new(),
            materials: Vec::new(),
            diagonals: Vec::new(),
        };

        let bvh = Bvh::new(&mesh);

        for (_, _, _, count) in bvh.nodes() {
            if count > 0 {
                assert!(count <= LEAF as u32, "leaf has too many triangles: {count}");
            }
        }

        // The foreground triangle is the closest hit
        let hit = bvh.closest(Vec3::new(20.1, 10.1, 10.0), Vec3::NEG_Z, 100.0).unwrap();
        assert!((hit.t - 9.5).abs() < 0.1, "hit t: {}", hit.t);
    }

    #[test]
    fn clustered_coplanar_fallback_partitions_nodes() {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        let w = 200.0f32;
        let h = 100.0f32;

        let b = positions.len() as u32;
        positions.extend_from_slice(&[Vec3::new(0.0, 0.0, 0.0), Vec3::new(w, 0.0, 0.0), Vec3::new(w, h, 0.0), Vec3::new(0.0, h, 0.0)]);
        indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);

        // Shared bins keep every SAH split as costly as an unsplit leaf
        for i in 0..500 {
            let x = 130.0 + (i % 10) as f32 * 0.1;
            let y = 64.0 + (i / 10) as f32 * 0.02;
            let idx = positions.len() as u32;
            positions.push(Vec3::new(x, y, 0.0));
            positions.push(Vec3::new(x + 0.01, y, 0.0));
            positions.push(Vec3::new(x, y + 0.01, 0.0));
            indices.extend_from_slice(&[idx, idx + 1, idx + 2]);
        }

        let mesh = Mesh {
            positions,
            normals: Vec::new(),
            uvs: Vec::new(),
            indices,
            tangents: Vec::new(),
            colors: Vec::new(),
            submeshes: Vec::new(),
            materials: Vec::new(),
            diagonals: Vec::new(),
        };

        let bvh = Bvh::new(&mesh);

        for (_, _, _, count) in bvh.nodes() {
            if count > 0 {
                assert!(count <= LEAF as u32, "leaf has too many triangles: {count}");
            }
        }
    }

    #[test]
    fn coincident_centroids_still_partition_large_and_small_triangles() {
        let mut mesh = Mesh::default();

        // Different triangle sizes share an exact centroid
        for i in 0..=500 {
            let size = if i == 0 { 100.0 } else { 0.01 + i as f32 * 0.0001 };
            let first = mesh.positions.len() as u32;
            mesh.positions.extend([Vec3::new(-size, -size, 0.0), Vec3::new(size, -size, 0.0), Vec3::new(0.0, 2.0 * size, 0.0)]);
            mesh.indices.extend([first, first + 1, first + 2]);
        }

        mesh.normals.resize(mesh.positions.len(), Vec3::Z);
        mesh.uvs.resize(mesh.positions.len(), glam::Vec2::ZERO);
        mesh.tangents.resize(mesh.positions.len(), glam::Vec4::new(1.0, 0.0, 0.0, 1.0));
        mesh.validate().unwrap();

        let bvh = Bvh::new(&mesh);

        for (_, _, _, count) in bvh.nodes() {
            assert!(count <= LEAF as u32, "leaf has too many triangles: {count}");
        }

        // The outer ray only hits the large triangle
        let origin = Vec3::new(50.0, 0.0, 10.0);
        let hit = bvh.closest(origin, Vec3::NEG_Z, 100.0).unwrap();
        assert_eq!(hit.tri, 0);
        assert_eq!(hit.t, 10.0);

        assert!(bvh.occluded(origin, Vec3::NEG_Z, 100.0));
        assert!(!bvh.occluded(origin, Vec3::NEG_Z, 5.0));

        // Median partitions preserve original triangle IDs
        let hit = bvh.closest_filtered(Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z, 100.0, &|t| t == 500).unwrap();
        assert_eq!(hit.tri, 500);
        assert_eq!(hit.t, 10.0);
    }

    #[test]
    fn bvh_rejects_non_finite_queries_without_callbacks() {
        let m = primitives::plane(2.0, 4);
        let bvh = Bvh::new(&m);
        let mut calls = 0;
        bvh.query_box(Vec3::new(f32::NAN, -1.0, -1.0), Vec3::ONE, |_| calls += 1);
        bvh.query_box(Vec3::splat(-1.0), Vec3::new(1.0, f32::NAN, 1.0), |_| calls += 1);
        // A box whose lower corner is past its upper corner on one axis is empty, even where a
        // triangle spans both values.
        bvh.query_box(Vec3::new(0.3, -1.0, -1.0), Vec3::new(0.2, 1.0, 1.0), |_| calls += 1);
        bvh.query_box(Vec3::new(-1.0, -1.0, 0.3), Vec3::new(1.0, 1.0, 0.2), |_| calls += 1);
        assert_eq!(calls, 0);
        // An infinite box is a valid query and finds everything.
        bvh.query_box(Vec3::NEG_INFINITY, Vec3::INFINITY, |_| calls += 1);
        assert_eq!(calls, m.triangle_count());

        let (o, d) = (Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Y);
        assert!(bvh.closest(o, d, 10.0).is_some());
        let counted = std::cell::Cell::new(0);
        let filter = |_: u32| {
            counted.set(counted.get() + 1);
            true
        };
        for (o, d, tmax) in
            [(Vec3::NAN, d, 10.0), (o, Vec3::new(0.0, f32::NAN, 0.0), 10.0), (o, Vec3::ZERO, 10.0), (o, Vec3::NEG_INFINITY, 10.0), (o, d, f32::NAN)]
        {
            assert!(bvh.closest_filtered(o, d, tmax, &filter).is_none(), "{o} {d} {tmax}");
            assert!(!bvh.occluded(o, d, tmax));
        }
        assert_eq!(counted.get(), 0);
    }

    #[test]
    fn ray_hits_do_not_depend_on_the_size_of_the_triangle() {
        for exponent in [-10, -7, -3, 0, 3, 7, 10] {
            let s = 10f32.powi(exponent);
            let m = Mesh { positions: vec![Vec3::ZERO, Vec3::new(s, 0.0, 0.0), Vec3::new(0.0, 0.0, s)], indices: vec![0, 1, 2], ..Mesh::default() };
            let bvh = Bvh::new(&m);
            // Straight down onto the triangle, from one triangle size above it.
            let o = Vec3::new(0.25 * s, s, 0.25 * s);
            let hit = bvh.closest(o, Vec3::NEG_Y, f32::MAX).unwrap_or_else(|| panic!("no hit at 1e{exponent}"));
            assert!((hit.t / s - 1.0).abs() < 1e-4, "1e{exponent}: {hit:?}");
            assert!((hit.u - 0.25).abs() < 1e-4 && (hit.v - 0.25).abs() < 1e-4, "1e{exponent}: {hit:?}");
            assert!(bvh.occluded(o, Vec3::NEG_Y, 2.0 * s), "1e{exponent}");
            // Beside the triangle, and along its plane.
            assert!(bvh.closest(Vec3::new(0.75 * s, s, 0.75 * s), Vec3::NEG_Y, f32::MAX).is_none(), "1e{exponent}");
            assert!(bvh.closest(Vec3::new(-s, 0.0, 0.25 * s), Vec3::X, f32::MAX).is_none(), "1e{exponent}");
        }
    }

    #[test]
    fn a_tree_without_triangles_has_no_nodes() {
        assert_eq!(Bvh::new(&Mesh::default()).nodes().len(), 0);
        // Every triangle is left out: one corner is not finite.
        let rejected = Mesh { positions: vec![Vec3::ZERO, Vec3::X, Vec3::new(f32::NAN, 0.0, 0.0)], indices: vec![0, 1, 2], ..Mesh::default() };
        let bvh = Bvh::new(&rejected);
        assert_eq!(bvh.nodes().len(), 0);
        assert!(bvh.is_empty());
        assert!(bvh.closest(Vec3::Z, Vec3::NEG_Z, 10.0).is_none());
        assert!(bvh.nearest(Vec3::ZERO, 10.0, &|_| true).is_none());
        let mut found = 0;
        bvh.query_box(Vec3::splat(-1.0), Vec3::ONE, |_| found += 1);
        assert_eq!(found, 0);
    }

    #[test]
    fn bvh_leaves_out_non_finite_triangles_and_keeps_original_indices() {
        let mut m = primitives::plane(2.0, 4);
        // Vertex 0 is the corner of one triangle only.
        m.positions[0] = Vec3::new(f32::NAN, 0.0, 0.0);
        m.positions[24] = Vec3::new(0.0, f32::INFINITY, 0.0);
        // One triangle's index points past the vertices.
        let out_of_range = m.triangle_count() - 1;
        m.indices[out_of_range * 3] = m.positions.len() as u32;
        let skipped = |t: usize| m.tri(t).is_some_and(|c| c.contains(&0) || c.contains(&24) || c.iter().any(|&i| i >= m.positions.len()));
        let expected: Vec<_> = (0..m.triangle_count()).filter(|&t| !skipped(t)).map(|t| t as u32).collect();
        let bvh = Bvh::new(&m);
        let mut found = Vec::new();
        bvh.query_box(Vec3::splat(-10.0), Vec3::splat(10.0), |t| found.push(t));
        found.sort_unstable();
        assert_eq!(found, expected);
        for (lo, hi, _, _) in bvh.nodes() {
            assert!(lo.is_finite() && hi.is_finite(), "{lo} {hi}");
        }
        let (tris, order) = bvh.triangles();
        assert!(order.iter().all(|&t| !skipped(t as usize)));
        // Each triangle is stored with the corners of the mesh triangle its index names.
        for (corners, &t) in tris.iter().zip(order) {
            let [a, b, c] = m.tri(t as usize).unwrap();
            assert_eq!(*corners, [m.positions[a], m.positions[b], m.positions[c]]);
        }
    }
}
