//! Bounding volume hierarchy over a mesh's triangles: closest-hit and any-hit ray casts, and
//! box queries (used by picking, baking and brush stamps).

use glam::{Vec3, Vec3A};

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

impl Bvh {
    pub fn new(mesh: &Mesh) -> Bvh {
        let triangle_count = mesh.triangle_count();
        let mut tris = Vec::with_capacity(triangle_count);

        for triangle in 0..triangle_count {
            let Some([a, b, c]) = mesh.tri(triangle) else { continue };
            let position = |i: usize| mesh.positions.get(i).copied().unwrap_or(Vec3::ZERO);
            tris.push([position(a), position(b), position(c)]);
        }

        let mut order: Vec<u32> = (0..tris.len() as u32).collect();
        let centroids: Vec<Vec3> = tris.iter().map(|t| (t[0] + t[1] + t[2]) / 3.0).collect();

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

        let tris = order.iter().map(|t| tris[*t as usize]).collect();
        Bvh { nodes, tris, order }
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// The flattened tree (for GPU traversal), root first: bounds, then for a leaf the range
    /// `first..first + count` of [`Bvh::triangles`], for an inner node (`count` 0) the children
    /// `first` and `first + 1`.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = (Vec3, Vec3, u32, u32)> + '_ {
        self.nodes.iter().map(|n| (n.lo.into(), n.hi.into(), n.first, n.count))
    }

    /// Triangle vertices in BVH order, and the original triangle index of each.
    pub fn triangles(&self) -> (&[[Vec3; 3]], &[u32]) {
        (&self.tris, &self.order)
    }

    fn slab(lo: Vec3A, hi: Vec3A, o: Vec3A, inv: Vec3A, tmax: f32) -> Option<f32> {
        let t0 = (lo - o) * inv;
        let t1 = (hi - o) * inv;
        // A ray lying in a slab plane gives 0·∞ = NaN: it is inside that slab, so the axis does
        // not bound the hit.
        let open = t0.is_nan_mask() | t1.is_nan_mask();
        let tn = Vec3A::select(open, Vec3A::NEG_INFINITY, t0.min(t1)).max_element().max(0.0);
        let tf = Vec3A::select(open, Vec3A::INFINITY, t0.max(t1)).min_element().min(tmax);
        (tn <= tf).then_some(tn)
    }

    /// Möller–Trumbore, double sided.
    fn hit_tri(t: &[Vec3; 3], o: Vec3, d: Vec3) -> Option<(f32, f32, f32)> {
        let e1 = t[1] - t[0];
        let e2 = t[2] - t[0];
        let p = d.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        let s = o - t[0];
        // A little slack on the edges: rays through a shared edge must not slip between its
        // two triangles to a surface behind.
        const EDGE: f32 = 1e-5;
        let u = s.dot(p) * inv;
        if !(-EDGE..=1.0 + EDGE).contains(&u) {
            return None;
        }
        let q = s.cross(e1);
        let v = d.dot(q) * inv;
        if v < -EDGE || u + v > 1.0 + EDGE {
            return None;
        }
        let tt = e2.dot(q) * inv;
        (tt > 0.0).then_some((tt, u, v))
    }

    fn traverse(&self, o: Vec3, d: Vec3, mut tmax: f32, any: bool, filter: &dyn Fn(u32) -> bool) -> Option<Hit> {
        let root = self.nodes.first().filter(|_| !self.tris.is_empty())?;
        let (oa, inv) = (Vec3A::from(o), Vec3A::ONE / Vec3A::from(d));
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
        let pa = Vec3A::from(p);
        let box_d2 = |n: &Node| (n.lo - pa).max(pa - n.hi).max(Vec3A::ZERO).length_squared();
        let (mut best_d2, mut best) = (max_dist * max_dist, None);
        let mut stack = Stack::default();
        stack.push((0, box_d2(root)));
        while let Some((ni, d2)) = stack.pop() {
            let Some(n) = self.nodes.get(ni as usize).filter(|_| d2 <= best_d2) else { continue };
            if n.count > 0 {
                for i in n.first as usize..(n.first as usize + n.count as usize).min(self.tris.len()) {
                    let (Some(tri), Some(&orig)) = (self.tris.get(i), self.order.get(i)) else { continue };
                    if !filter(orig) {
                        continue;
                    }
                    let (q, u, v) = closest_on_triangle(p, tri);
                    let dd = (q - p).length_squared();
                    if dd < best_d2 {
                        best_d2 = dd;
                        best = Some(Hit { t: dd.sqrt(), tri: orig, u, v });
                    }
                }
            } else {
                // Push the farther child first so the nearer one is searched first.
                let child = |c: u32| self.nodes.get(c as usize).map(|n| (c, box_d2(n)));
                let (Some(l), Some(r)) = (child(n.first), child(n.first + 1)) else { continue };
                let (near, far) = if l.1 <= r.1 { (l, r) } else { (r, l) };
                stack.push(far);
                stack.push(near);
            }
        }
        best
    }

    /// Call `f` with every triangle (original index) whose bounds overlap the box.
    pub fn query_box(&self, lo: Vec3, hi: Vec3, mut f: impl FnMut(u32)) {
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
/// a non-finite distance and are never nearest.
fn closest_on_triangle(p: Vec3, t: &[Vec3; 3]) -> (Vec3, f32, f32) {
    let [a, b, c] = *t;
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, 0.0, 0.0);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return (b, 1.0, 0.0);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, v, 0.0);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return (c, 0.0, 1.0);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, 0.0, w);
    }
    let va = d3 * d6 - d5 * d4;
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
                    (closest_on_triangle(p, &[a, b, c]).0 - p).length()
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
}
