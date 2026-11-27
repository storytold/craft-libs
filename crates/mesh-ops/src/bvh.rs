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

/// A hit identifies a triangle and the barycentric weights of vertices 1 and 2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// `t` is the ray parameter for ray queries and the distance for nearest queries.
    /// Ray queries exclude parameters that round to zero or infinity.
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

fn triple_permanent(a: DVec3, b: DVec3, c: DVec3) -> f64 {
    let (a, b, c) = (a.abs(), b.abs(), c.abs());
    (0..3).map(|axis| a[axis] * (b[(axis + 1) % 3] * c[(axis + 2) % 3] + b[(axis + 2) % 3] * c[(axis + 1) % 3])).sum()
}

fn exact_triple_terms<const N: usize>(terms: &[(Vec3, Vec3, Vec3, f64)]) -> crate::gbuffer::Expansion<N> {
    let mut sum = crate::gbuffer::Expansion::<N>::new();
    for &(left, middle, right, sign) in terms {
        let (left, middle, right) = (left.as_dvec3(), middle.as_dvec3(), right.as_dvec3());
        for axis in 0..3 {
            let j = (axis + 1) % 3;
            let k = (axis + 2) % 3;
            // Products of two f32 coordinates fit exactly in f64.
            sum.product(sign * left[axis], middle[j] * right[k]);
            sum.product(-sign * left[axis], middle[k] * right[j]);
        }
    }
    sum
}

fn exact_triple_sum<const N: usize>(terms: &[(Vec3, Vec3, Vec3, f64)]) -> f64 {
    exact_triple_terms::<N>(terms).value()
}

fn ray_parameter(t: &[Vec3; 3], o: Vec3, d: Vec3) -> (crate::gbuffer::Expansion<48>, crate::gbuffer::Expansion<36>) {
    (
        exact_triple_terms(&[(t[0], t[1], t[2], 1.0), (o, t[1], t[2], -1.0), (t[0], o, t[2], -1.0), (t[0], t[1], o, -1.0)]),
        exact_triple_terms(&[(d, t[1], t[2], 1.0), (d, t[2], t[0], 1.0), (d, t[0], t[1], 1.0)]),
    )
}

fn ray_parameter_less(t: &[Vec3; 3], previous: Option<&[Vec3; 3]>, o: Vec3, d: Vec3, cutoff: f64) -> bool {
    let (numerator, denominator) = ray_parameter(t, o, d);
    let mut difference = crate::gbuffer::Expansion::<128>::new();
    let previous_sign = if let Some(previous) = previous {
        let (previous_numerator, previous_denominator) = ray_parameter(previous, o, d);
        for &left in numerator.components() {
            for &right in previous_denominator.components() {
                difference.product(left, right);
            }
        }
        for &left in previous_numerator.components() {
            for &right in denominator.components() {
                difference.product(-left, right);
            }
        }
        previous_denominator.value().signum()
    } else {
        for &part in numerator.components() {
            difference.product(part, 1.0);
        }
        for &part in denominator.components() {
            difference.product(-part, cutoff);
        }
        1.0
    };
    difference.value() * denominator.value().signum() * previous_sign < 0.0
}

fn ray_edge_orientation(direction: Vec3, origin: Vec3, a: Vec3, b: Vec3) -> f64 {
    exact_triple_sum::<36>(&[(direction, a, b, 1.0), (direction, a, origin, -1.0), (direction, origin, b, -1.0)])
}
/// Traversal stack of (node, entry distance): inline for typical depths (no allocation per
/// ray), spilling to the heap for degenerate trees.
struct Stack {
    inline: [(u32, f64); 64],
    len: usize,
    spill: Vec<(u32, f64)>,
}

impl Default for Stack {
    fn default() -> Self {
        Stack { inline: [(0, 0.0); 64], len: 0, spill: Vec::new() }
    }
}

impl Stack {
    fn push(&mut self, v: (u32, f64)) {
        match self.inline.get_mut(self.len) {
            Some(slot) if self.spill.is_empty() => {
                *slot = v;
                self.len += 1;
            }
            _ => self.spill.push(v),
        }
    }

    fn pop(&mut self) -> Option<(u32, f64)> {
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
    /// The tree excludes triangles with nonfinite corners or out-of-range indices and every
    /// triangle past [`MAX_BVH_TRIANGLES`].
    pub fn new(mesh: &Mesh) -> Bvh {
        let triangle_count = mesh.triangle_count().min(MAX_BVH_TRIANGLES);
        let mut tris = Vec::with_capacity(triangle_count);
        // Original index of each kept triangle: a triangle with an out-of-range vertex index is left out.
        let mut original = Vec::with_capacity(triangle_count);

        for triangle in 0..triangle_count {
            let Some([a, b, c]) = mesh.tri(triangle) else { continue };
            let (Some(&pa), Some(&pb), Some(&pc)) = (mesh.positions.get(a), mesh.positions.get(b), mesh.positions.get(c)) else { continue };
            if !(pa.is_finite() && pb.is_finite() && pc.is_finite()) {
                continue;
            }
            tris.push([pa, pb, pc]);
            original.push(triangle as u32);
        }

        if tris.is_empty() {
            return Bvh { nodes: Vec::new(), tris, order: Vec::new() };
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
        let order = order.iter().map(|t| original[*t as usize]).collect();
        Bvh { nodes, tris: sorted, order }
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// The flattened tree (for GPU traversal), root first: bounds, then for a leaf the range
    /// `first..first + count` of [`Bvh::triangles`], for an inner node (`count` 0) the children
    /// `first` and `first + 1`. A tree without triangles has no nodes.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = (Vec3, Vec3, u32, u32)> + '_ {
        let len = if self.tris.is_empty() { 0 } else { self.nodes.len() };
        self.nodes.iter().take(len).map(|n| (n.lo.into(), n.hi.into(), n.first, n.count))
    }

    /// Triangle vertices in BVH order, and the original triangle index of each.
    pub fn triangles(&self) -> (&[[Vec3; 3]], &[u32]) {
        (&self.tris, &self.order)
    }

    fn slab(lo: Vec3A, hi: Vec3A, o: DVec3, inv: DVec3, tmax: f64) -> Option<f64> {
        let t0 = (lo.as_dvec3() - o) * inv;
        let t1 = (hi.as_dvec3() - o) * inv;
        // A ray lying in a slab plane gives 0·∞ = NaN: it is inside that slab, so the axis does
        // not bound the hit.
        let open = t0.is_nan_mask() | t1.is_nan_mask();
        let tn = DVec3::select(open, DVec3::NEG_INFINITY, t0.min(t1)).max_element().max(0.0);
        let tf = DVec3::select(open, DVec3::INFINITY, t0.max(t1)).min_element().min(tmax);
        let margin = 4.0 * f64::from(f32::EPSILON) * tn.abs().max(tf.abs()).min(f64::from(f32::MAX));
        (tn - margin <= tf + margin).then_some((tn - margin).max(0.0))
    }

    /// Double-sided triangle intersection. Near-edge rays use exact signs of the f32 inputs
    /// and f64 intersection weights.
    fn hit_tri(t: &[Vec3; 3], o: Vec3, d: Vec3) -> Option<(f64, f32, f32)> {
        let (e1, e2) = (t[1].as_dvec3() - t[0].as_dvec3(), t[2].as_dvec3() - t[0].as_dvec3());
        let direction = d.as_dvec3();
        let p = direction.cross(e2);
        let det = e1.dot(p);
        let error_scale = 64.0 * f64::EPSILON;
        let det_error = error_scale * triple_permanent(e1, direction, e2);
        let denominator_bound = det.abs() - det_error;
        if !denominator_bound.is_finite() || denominator_bound <= 0.0 {
            return Self::hit_tri_exact(t, o, d);
        }
        let s = o.as_dvec3() - t[0].as_dvec3();
        let u = s.dot(p) / det;
        let q = s.cross(e1);
        let v = direction.dot(q) / det;
        let w = 1.0 - u - v;
        if !(u.is_finite() && v.is_finite() && w.is_finite()) {
            return Self::hit_tri_exact(t, o, d);
        }
        let ratio_error =
            |a, b, c, value: f64| (error_scale * triple_permanent(a, b, c) + value.abs() * det_error) / denominator_bound + error_scale * value.abs();
        let u_error = ratio_error(s, direction, e2, u);
        let v_error = ratio_error(direction, s, e1, v);
        let w_error = u_error + v_error + error_scale * (1.0 + u.abs() + v.abs());
        if u < -u_error || v < -v_error || w < -w_error {
            return None;
        }
        let rounding_budget = f64::from(f32::EPSILON) * 0.125;
        if u.min(v).min(w) <= 1e-4
            || u_error > rounding_budget * u.abs()
            || v_error > rounding_budget * v.abs()
            || w_error > rounding_budget * w.abs()
        {
            return Self::hit_tri_exact(t, o, d);
        }
        let tt = e2.dot(q) / det;
        let distance_error = ratio_error(e2, s, e1, tt);
        if tt < -distance_error {
            return None;
        }
        if !tt.is_finite() || tt <= distance_error || distance_error > rounding_budget * tt.abs() {
            return Self::hit_tri_exact(t, o, d);
        }
        let distance = tt as f32;
        (distance > 0.0 && distance.is_finite()).then_some((tt, u as f32, v as f32))
    }

    fn hit_tri_exact(t: &[Vec3; 3], o: Vec3, d: Vec3) -> Option<(f64, f32, f32)> {
        let sides = [ray_edge_orientation(d, o, t[1], t[2]), ray_edge_orientation(d, o, t[2], t[0]), ray_edge_orientation(d, o, t[0], t[1])];
        let positive = sides.iter().any(|side| *side > 0.0);
        let negative = sides.iter().any(|side| *side < 0.0);
        if positive == negative {
            return None;
        }
        let denominator = sides.iter().sum::<f64>();
        let numerator = exact_triple_sum::<48>(&[(t[0], t[1], t[2], 1.0), (o, t[1], t[2], -1.0), (t[0], o, t[2], -1.0), (t[0], t[1], o, -1.0)]);
        let tt = numerator / denominator;
        let u = sides[1] / denominator;
        let v = sides[2] / denominator;
        let distance = tt as f32;
        (distance > 0.0 && distance.is_finite()).then_some((tt, u as f32, v as f32))
    }

    fn traverse(&self, o: Vec3, d: Vec3, tmax: f32, any: bool, filter: &dyn Fn(u32) -> bool) -> Option<Hit> {
        if !o.is_finite() || !d.is_finite() || d == Vec3::ZERO || tmax.is_nan() {
            return None;
        }
        let mut tmax = f64::from(tmax);
        let root = self.nodes.first().filter(|_| !self.tris.is_empty())?;
        let (oa, inv) = (o.as_dvec3(), DVec3::ONE / d.as_dvec3());
        let mut best = None;
        let mut best_triangle = None;
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
                        && (if tmax.is_finite() && (t - tmax).abs() <= 4.0 * f64::from(f32::EPSILON) * (t.abs() + tmax.abs()) {
                            ray_parameter_less(tri, best_triangle.map(|index| &self.tris[index]), o, d, tmax)
                        } else {
                            t < tmax
                        })
                        && filter(orig)
                    {
                        tmax = t;
                        best_triangle = Some(i);
                        best = Some(Hit { t: t as f32, tri: orig, u, v });
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
    /// Cutoffs and closest-hit comparisons use `f64` parameters before constructing the public `Hit`.
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

    /// The point nearest to `p` at most `max_dist` away on a triangle accepted by `filter`
    /// (original index): a [`Hit`] with the distance in `t` and the barycentrics of vertices 1
    /// and 2. Squared distances are compared in `f64`. A negative or NaN `max_dist` finds nothing.
    pub fn nearest(&self, p: Vec3, max_dist: f32, filter: &dyn Fn(u32) -> bool) -> Option<Hit> {
        if max_dist.is_nan() || max_dist < 0.0 {
            return None;
        }
        self.nodes.first().filter(|_| !self.tris.is_empty() && p.is_finite())?;
        let pd = p.as_dvec3();
        let box_d2 = |n: &Node| (n.lo.as_dvec3() - pd).max(pd - n.hi.as_dvec3()).max(DVec3::ZERO).length_squared();
        let max_dist = f64::from(max_dist);
        let (mut best_d2, mut best) = (max_dist * max_dist, None);
        let mut stack = Stack::default();
        stack.push((0, 0.0));
        while let Some((ni, _)) = stack.pop() {
            let Some(n) = self.nodes.get(ni as usize).filter(|n| box_d2(n) <= best_d2) else { continue };
            if n.count > 0 {
                for i in n.first as usize..(n.first as usize + n.count as usize).min(self.tris.len()) {
                    let (Some(tri), Some(&orig)) = (self.tris.get(i), self.order.get(i)) else { continue };
                    if !filter(orig) {
                        continue;
                    }
                    let (q, u, v) = closest_on_triangle_f64(pd, tri.map(|corner| corner.as_dvec3()));
                    let dd = (q - pd).length_squared();
                    // The first point found may lie exactly at `max_dist`.
                    if dd.is_finite() && (dd < best_d2 || (best.is_none() && dd <= best_d2)) {
                        best_d2 = dd;
                        best = Some(Hit { t: dd.sqrt() as f32, tri: orig, u: u as f32, v: v as f32 });
                    }
                }
            } else {
                // Push the farther child first so the nearer one is searched first.
                let child = |c: u32| self.nodes.get(c as usize).map(|n| (c, box_d2(n)));
                let (Some(l), Some(r)) = (child(n.first), child(n.first + 1)) else { continue };
                let (near, far) = if l.1 <= r.1 { (l, r) } else { (r, l) };
                stack.push((far.0, 0.0));
                stack.push((near.0, 0.0));
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
/// by Voronoi region (Ericson, Real-Time Collision Detection, 5.1.5), in `f64`: in `f32` the
/// region tests round to zero inside thin triangles. A triangle without area is measured as the
/// point or segment it is, and can be the nearest. An edge of zero length is skipped: the
/// denominator of its parameter is its squared length.
#[cfg(test)]
fn closest_on_triangle(p: Vec3, t: &[Vec3; 3]) -> (Vec3, f32, f32) {
    let (q, v, w) = closest_on_triangle_f64(p.as_dvec3(), t.map(|c| c.as_dvec3()));
    (q.as_vec3(), v as f32, w as f32)
}

fn closest_on_triangle_f64(p: DVec3, [a, b, c]: [DVec3; 3]) -> (DVec3, f64, f64) {
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
    // d1 - d3 = |ab|², d2 - d6 = |ac|² and (d4 - d3) + (d5 - d6) = |bc|².
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 && d1 - d3 > 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, v, 0.0);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return (c, 0.0, 1.0);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 && d2 - d6 > 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, 0.0, w);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 && (d4 - d3) + (d5 - d6) > 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + (c - b) * w, 1.0 - w, w);
    }
    let sum = va + vb + vc;
    if sum > 0.0 {
        let (v, w) = (vb / sum, vc / sum);
        // Project p onto the plane instead of rebuilding a + ab v + ac w, whose rounding moves a
        // point that lies on the plane off it. A point whose height above the plane is 0 stays p.
        let n = ab.cross(ac);
        let nn = n.length_squared();
        let q = if nn > 0.0 { p - n * (n.dot(ap) / nn) } else { a + ab * v + ac * w };
        return (q, v, w);
    }
    // No area and no region matched: the nearest of the three edges.
    let on_edge = |from: DVec3, to: DVec3| {
        let d = to - from;
        let s = if d.length_squared() > 0.0 { ((p - from).dot(d) / d.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
        (from + d * s, s)
    };
    let ((qab, s), (qac, r), (qbc, k)) = (on_edge(a, b), on_edge(a, c), on_edge(b, c));
    [(qab, s, 0.0), (qac, 0.0, r), (qbc, 1.0 - k, k)]
        .into_iter()
        .min_by(|x, y| (x.0 - p).length_squared().total_cmp(&(y.0 - p).length_squared()))
        .unwrap_or((a, 0.0, 0.0))
}

#[cfg(test)]
mod tests {
    #[test]
    fn ray_cutoffs_exclude_exact_endpoints_before_ratio_rounding() {
        let tree = ray_triangle(Vec3::ZERO, Vec3::new(12345.0, 0.0, 0.0), Vec3::new(0.0, 999999.0, 0.0));
        let (origin, direction) = (Vec3::new(4115.0, 666666.0, 2345679.0), Vec3::new(0.0, 0.0, -2345679.0));
        assert!(tree.closest(origin, direction, 1.0).is_none());
        assert!(!tree.occluded(origin, direction, 1.0));
    }

    #[test]
    fn ray_cutoffs_compare_before_hit_parameters_are_narrowed() {
        let tree = ray_triangle(Vec3::ZERO, Vec3::X, Vec3::Y);
        let (origin, direction) = (Vec3::new(0.25, 0.25, 1.0), Vec3::new(0.0, 0.0, -3.0));
        let cutoff = 1.0_f32 / 3.0;
        assert!(tree.closest(origin, direction, cutoff).is_some());
        assert!(tree.occluded(origin, direction, cutoff));
    }

    #[test]
    fn closest_ray_hit_keeps_order_when_parameters_round_equal() {
        let raised = f32::from_bits(1.0_f32.to_bits() + 1);
        let mesh = Mesh::from_parts(
            vec![
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(0.0, 1.0, raised),
                Vec3::Z,
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(0.0, 1.0, 1.0),
            ],
            None,
            Vec::new(),
            vec![0, 1, 2, 3, 4, 5],
            "pair",
        )
        .unwrap();
        let tree = Bvh::new(&mesh);
        let hit = tree.closest(Vec3::new(0.25, 0.25, 0.0), Vec3::new(0.0, 0.0, 3.0), f32::INFINITY).unwrap();
        assert_eq!(hit.tri, 1);
    }

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
    fn triangles_with_a_non_finite_corner_are_left_out() {
        let mut m = primitives::builtin("torus").unwrap();
        let bad = m.indices[0] as usize;
        m.positions[bad] = Vec3::NAN;
        let skipped: Vec<u32> = (0..m.triangle_count() as u32).filter(|t| m.tri(*t as usize).unwrap().contains(&bad)).collect();
        let bvh = Bvh::new(&m);
        let (tris, order) = bvh.triangles();
        assert_eq!(tris.len(), m.triangle_count() - skipped.len());
        assert!(order.iter().all(|o| !skipped.contains(o)));
        assert!(bvh.nodes.iter().all(|n| n.lo.is_finite() && n.hi.is_finite()));
        let hit = bvh.nearest(Vec3::ZERO, f32::MAX, &|_| true).unwrap();
        assert!(!skipped.contains(&hit.tri) && hit.t.is_finite());
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

    fn triangle_with_attributes(positions: [Vec3; 3]) -> Mesh {
        Mesh {
            positions: positions.to_vec(),
            indices: vec![0, 1, 2],
            normals: vec![Vec3::Z; 3],
            uvs: vec![glam::Vec2::ZERO; 3],
            tangents: vec![Vec3::X.extend(1.0); 3],
            ..Mesh::default()
        }
    }

    fn single_triangle(positions: Vec<Vec3>, indices: Vec<u32>) -> Bvh {
        Bvh::new(&Mesh { positions, indices, ..Mesh::default() })
    }

    #[test]
    fn nearest_includes_points_exactly_at_the_distance() {
        let m = primitives::plane(2.0, 4);
        let bvh = Bvh::new(&m);
        // A vertex of the plane, and a point 0.5 above it.
        let corner = m.positions[6];
        assert_eq!(bvh.nearest(corner, 0.0, &|_| true).map(|h| h.t), Some(0.0));
        let above = corner + Vec3::new(0.0, 0.5, 0.0);
        assert_eq!(bvh.nearest(above, 0.5, &|_| true).map(|h| h.t), Some(0.5));
        assert!(bvh.nearest(above, 0.499, &|_| true).is_none());
        // A triangle 1e30 away is outside a distance of 1e20, though both squares overflow.
        let far = single_triangle(vec![Vec3::new(1e30, 0.0, 0.0), Vec3::new(1e30, 1.0, 0.0), Vec3::new(1e30, 0.0, 1.0)], vec![0, 1, 2]);
        assert!(far.nearest(Vec3::ZERO, 1e20, &|_| true).is_none());
    }

    #[test]
    fn nearest_compares_the_unrounded_projection_distance() {
        let bvh = single_triangle(vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 1.0), Vec3::Y], vec![0, 1, 2]);
        let point = Vec3::new(0.5, 0.25, f32::from_bits(0.5f32.to_bits() + 1));
        assert!(bvh.nearest(point, 4e-8, &|_| true).is_none());
        let hit = bvh.nearest(point, 5e-8, &|_| true).expect("the plane projection is inside the distance limit");
        assert!(hit.t > 4e-8 && hit.t < 5e-8, "{hit:?}");
    }

    #[test]
    fn nearest_finds_a_point_inside_a_thin_triangle() {
        let bvh = single_triangle(vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1e-4, 0.0)], vec![0, 1, 2]);
        // p = a + 0.5 (b - a) + 0.25 (c - a) lies on the triangle.
        let p = Vec3::new(0.75, 2.5e-5, 0.0);
        let hit = bvh.nearest(p, 0.0, &|_| true).unwrap();
        assert_eq!(hit.t, 0.0);
        assert!((hit.u - 0.5).abs() < 1e-4 && (hit.v - 0.25).abs() < 1e-4, "{hit:?}");
    }

    #[test]
    fn nearest_finds_a_coplanar_point_inside_a_triangle_at_distance_zero() {
        let bvh = single_triangle(vec![Vec3::new(-1.0, -1.0, 0.0), Vec3::new(2.0, 0.0, 0.0), Vec3::new(0.0, 3.0, 0.0)], vec![0, 1, 2]);
        // The origin is a + 3/11 (b - a) + 2/11 (c - a); rebuilding it from those weights in
        // floating point gives x of about -5.6e-17 instead of 0.
        let hit = bvh.nearest(Vec3::ZERO, 0.0, &|_| true).unwrap();
        assert_eq!(hit.t, 0.0);
        assert!((hit.u - 3.0 / 11.0).abs() < 1e-6 && (hit.v - 2.0 / 11.0).abs() < 1e-6, "{hit:?}");
    }

    #[test]
    fn nearest_measures_a_triangle_with_a_collapsed_edge() {
        let p = Vec3::new(0.5, 1.0, 0.0);
        let (o, x) = (Vec3::ZERO, Vec3::X);
        // The surviving segment runs from the origin to +X; its point nearest p is (0.5, 0, 0).
        for (positions, indices) in [
            (vec![o, o, x], vec![0, 1, 2]),
            (vec![o, x, o], vec![0, 1, 2]),
            (vec![x, o, o], vec![0, 1, 2]),
            (vec![x, x, o], vec![0, 1, 2]),
            (vec![o, x], vec![0, 0, 1]),
        ] {
            let bvh = single_triangle(positions.clone(), indices.clone());
            let hit = bvh.nearest(p, 10.0, &|_| true);
            let hit = hit.unwrap_or_else(|| panic!("{positions:?} {indices:?}: no point found"));
            assert!((hit.t - 1.0).abs() < 1e-6, "{positions:?} {indices:?}: {hit:?}");
            let [a, b, c] = [0, 1, 2].map(|k| positions[indices[k] as usize]);
            let q = a * (1.0 - hit.u - hit.v) + b * hit.u + c * hit.v;
            assert!((q - Vec3::new(0.5, 0.0, 0.0)).length() < 1e-6, "{positions:?} {indices:?}: {q}");
        }
    }

    #[test]
    fn triangles_with_out_of_range_indices_are_left_out() {
        let positions = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(10.0, 0.0, 0.0), Vec3::new(11.0, 0.0, 0.0), Vec3::new(10.0, 1.0, 0.0)];
        // Triangle 1 would become (1, 0, 0), (0, 1, 0), origin if the missing vertex read as zero.
        let bvh = single_triangle(positions, vec![3, 4, 5, 1, 2, 9, 3, 4, 5]);
        assert!(bvh.closest(Vec3::new(0.2, 0.2, 5.0), Vec3::NEG_Z, 100.0).is_none());
        let hit = bvh.closest_filtered(Vec3::new(10.2, 0.2, 5.0), Vec3::NEG_Z, 100.0, &|t| t == 2).unwrap();
        assert_eq!(hit.tri, 2, "the triangles after a left-out one keep their original index");
        assert!(bvh.nearest(Vec3::new(0.2, 0.2, 0.0), 1.0, &|_| true).is_none());
    }

    #[test]
    fn an_empty_tree_has_no_nodes() {
        let bvh = Bvh::new(&Mesh::default());
        assert_eq!(bvh.nodes().len(), 0);
        assert!(bvh.is_empty());
    }

    #[test]
    fn rays_at_vertices_are_not_lost_to_slab_rounding() {
        let m = primitives::sphere(1.0, 24, 12);
        let bvh = Bvh::new(&m);
        let mut seed = 99u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        for &target in &m.positions {
            for _ in 0..8 {
                let o = target + Vec3::new(rnd(), rnd(), rnd()).normalize_or_zero() * 3.0;
                let d = (target - o).normalize();
                let slow = (0..m.triangle_count())
                    .filter_map(|t| {
                        let [x, y, z] = m.tri(t)?;
                        Bvh::hit_tri(&[m.positions[x], m.positions[y], m.positions[z]], o, d).map(|h| h.0)
                    })
                    .fold(None, |s: Option<f64>, t| Some(s.map_or(t, |s| s.min(t))));
                let fast = bvh.closest(o, d, 1e9).map(|h| h.t);
                assert_eq!(fast.is_some(), slow.is_some(), "{o:?} {d:?}: {fast:?} {slow:?}");
            }
        }
    }

    #[test]
    fn rays_hit_a_triangle_far_smaller_than_one_unit() {
        let bvh = single_triangle(vec![Vec3::ZERO, Vec3::new(1e-7, 0.0, 0.0), Vec3::new(0.0, 1e-7, 0.0)], vec![0, 1, 2]);
        let hit = bvh.closest(Vec3::new(2e-8, 2e-8, 1.0), Vec3::NEG_Z, 10.0).unwrap();
        assert!((hit.t - 1.0).abs() < 1e-6, "{hit:?}");
        // A ray in the triangle's plane still finds nothing.
        assert!(bvh.closest(Vec3::new(-1.0, 2e-8, 0.0), Vec3::X, 10.0).is_none());
    }

    #[test]
    fn barycentrics_of_a_hit_on_the_edge_stay_on_the_triangle() {
        // A large triangle below widens the tree's bounds past the small one's edge.
        let (lo, hi) = (Vec3::new(-1.0, -1.0, -5.0), Vec3::new(2.0, 2.0, -5.0));
        let bvh =
            single_triangle(vec![Vec3::ZERO, Vec3::X, Vec3::Y, lo, Vec3::new(hi.x, lo.y, lo.z), Vec3::new(lo.x, hi.y, lo.z)], vec![0, 1, 2, 3, 4, 5]);
        let hit = bvh.closest(Vec3::new(0.0, 0.5, 1.0), Vec3::NEG_Z, 10.0).unwrap();
        assert_eq!(hit.tri, 0, "the ray crosses the edge");
        assert!(hit.u >= 0.0 && hit.v >= 0.0 && hit.u + hit.v <= 1.0, "{hit:?}");
        assert_eq!(bvh.closest(Vec3::new(-5e-10, 0.5, 1.0), Vec3::NEG_Z, 10.0).map(|h| h.tri), Some(1));
        assert_eq!(bvh.closest(Vec3::new(-5e-6, 0.5, 1.0), Vec3::NEG_Z, 10.0).map(|h| h.tri), Some(1));
        assert_eq!(bvh.closest(Vec3::new(-1e-3, 0.5, 1.0), Vec3::NEG_Z, 10.0).map(|h| h.tri), Some(1));
    }

    #[test]
    fn rays_with_finite_subnormal_directions_cross_slabs() {
        let bvh = single_triangle(vec![Vec3::ZERO, Vec3::Y, Vec3::Z], vec![0, 1, 2]);
        for tiny in [f32::MIN_POSITIVE / 4.0, f32::from_bits(1)] {
            let direction = Vec3::new(tiny, 0.0, 0.0);
            let origin = Vec3::new(-tiny, 0.25, 0.25);
            let hit = bvh.closest(origin, direction, 2.0).unwrap();
            assert_eq!(hit.t, 1.0);
            assert!(bvh.occluded(origin, direction, 2.0));
            assert!(bvh.closest(Vec3::new(-tiny, 0.75, 0.75), direction, 2.0).is_none());
        }
    }

    #[test]
    fn near_endpoint_ray_surfaces_keep_the_minor_weight() {
        for extent in [1.0, 2.0_f32.powi(32)] {
            let mesh = triangle_with_attributes([Vec3::new(-extent, 0.0, 0.0), Vec3::ZERO, Vec3::new(0.0, 2.0_f32.powi(26), 0.0)]);
            let hit = Bvh::new(&mesh).closest(Vec3::new(0.0, 1.0, 1.0), Vec3::NEG_Z, 2.0).unwrap();
            let surface = crate::surface_at(&mesh, hit.tri, hit.u, hit.v).unwrap();
            assert_eq!(surface.pos.y, 1.0);
            assert_eq!(surface.pos.x, 0.0);
        }
    }

    #[test]
    fn near_endpoint_nearest_surfaces_keep_the_minor_weight() {
        for extent in [1.0, 2.0_f32.powi(32)] {
            let mesh = triangle_with_attributes([Vec3::new(-extent, 0.0, 0.0), Vec3::ZERO, Vec3::new(0.0, 2.0_f32.powi(26), 0.0)]);
            let hit = Bvh::new(&mesh).nearest(Vec3::new(1.0, 1.0, 0.0), 2.0, &|_| true).unwrap();
            let surface = crate::surface_at(&mesh, hit.tri, hit.u, hit.v).unwrap();
            assert_eq!(surface.pos.y, 1.0);
            assert_eq!(surface.pos.x, 0.0);
        }
    }

    #[test]
    fn nearest_edge_hit_keeps_zero_opposite_weight() {
        let mesh = Mesh {
            positions: vec![Vec3::new(-2.0_f32.powi(32), 0.0, 0.0), Vec3::ZERO, Vec3::new(0.0, 1280.0, 0.0)],
            indices: vec![0, 1, 2],
            normals: vec![Vec3::Z; 3],
            uvs: vec![glam::Vec2::ZERO; 3],
            tangents: vec![Vec3::X.extend(1.0); 3],
            ..Mesh::default()
        };
        let bvh = Bvh::new(&mesh);
        let hit = bvh.nearest(Vec3::new(1.0, 1024.0, 0.0), 2.0, &|_| true).unwrap();
        let surface = crate::surface_at(&mesh, hit.tri, hit.u, hit.v).unwrap();
        assert_eq!(surface.pos.x, 0.0);
        assert_eq!(hit.u + hit.v, 1.0);
        assert_eq!(hit.t, 1.0);
    }

    #[test]
    fn ray_weights_account_for_distant_origin_cancellation() {
        let positions = vec![Vec3::new(-20.28125, 0.0, 0.0), Vec3::new(-16.28125, 0.0, 0.0), Vec3::new(-20.28125, 0.75, 0.0)];
        for indices in [vec![0, 1, 2], vec![0, 2, 1], vec![1, 2, 0]] {
            let bvh = single_triangle(positions.clone(), indices);
            let origin = Vec3::new(-8_388_608.0, 0.25, 5_392_663.5);
            let direction = Vec3::new(1.75, 0.0, -1.125);
            assert!(bvh.closest(origin, direction, f32::MAX).is_none());
            assert!(!bvh.occluded(origin, direction, f32::MAX));
            assert!(bvh.closest(origin + Vec3::X, direction, f32::MAX).is_some());
        }
    }

    #[test]
    fn exact_ray_surface_preserves_a_zero_opposite_vertex() {
        let triangle = [Vec3::new(-2.0_f32.powi(32), 0.0, 0.0), Vec3::ZERO, Vec3::new(0.0, 1280.0, 0.0)];
        let (_, u, v) = Bvh::hit_tri_exact(&triangle, Vec3::new(0.0, 1024.0, 1.0), Vec3::NEG_Z).unwrap();
        let surface = crate::surface_at(&triangle_with_attributes(triangle), 0, u, v).unwrap();
        assert_eq!(surface.pos.x, 0.0);
        assert_eq!(surface.pos.y, 1024.0);
    }

    #[test]
    fn rays_hit_a_far_thin_triangle_without_cancelling_its_area() {
        let size = 2.0_f32.powi(60);
        let bvh = single_triangle(vec![Vec3::new(0.0, -size, 0.0), Vec3::new(size, 1.0, 0.0), Vec3::new(size, 2.0, 0.0)], vec![0, 1, 2]);
        let origin = Vec3::new(size, 1.5, 1.0);
        assert!(bvh.closest(origin, Vec3::NEG_Z, 2.0).is_some());
        assert!(bvh.occluded(origin, Vec3::NEG_Z, 2.0));
        assert!(bvh.closest(Vec3::new(size, 3.0, 1.0), Vec3::NEG_Z, 2.0).is_none());
    }

    #[test]
    fn slab_rounding_margin_does_not_expand_an_isolated_triangle() {
        let size = 2.0_f32.powi(32);
        for indices in [vec![0, 1, 2], vec![1, 2, 0], vec![2, 0, 1]] {
            let bvh = single_triangle(vec![Vec3::ZERO, Vec3::X * size, Vec3::Y * size], indices);
            let direction = Vec3::new(1.0, 0.0, -1.0);
            let outside = Vec3::new(-16_777_218.0, 1.0, 16_777_216.0);
            assert!(bvh.closest(outside, direction, f32::MAX).is_none());
            assert!(!bvh.occluded(outside, direction, f32::MAX));
            let edge = Vec3::new(-16_777_216.0, 1.0, 16_777_216.0);
            assert!(bvh.closest(edge, direction, f32::MAX).is_some());
        }
    }

    #[test]
    fn exact_ray_surface_remains_inside_after_widening() {
        let triangle = [Vec3::new(-1e6, 0.0, 0.0), Vec3::ZERO, Vec3::new(0.0, 3.0, 0.0)];
        let (_, u, v) = Bvh::hit_tri_exact(&triangle, Vec3::new(0.0, 1.0, 1.0), Vec3::NEG_Z).unwrap();
        assert!(u + v <= 1.0, "{u}, {v}");
        let surface = crate::surface_at(&triangle_with_attributes(triangle), 0, u, v).unwrap();
        assert_eq!(surface.pos.x, 0.0);
        assert_eq!(surface.pos.y, 1.0);
    }

    #[test]
    fn nearest_finds_nothing_within_a_negative_or_nan_distance() {
        let bvh = single_triangle(vec![Vec3::ZERO, Vec3::X, Vec3::Y], vec![0, 1, 2]);
        let p = Vec3::new(0.25, 0.25, 0.5);
        assert_eq!(bvh.nearest(p, 1.0, &|_| true).map(|h| h.t), Some(0.5));
        assert!(bvh.nearest(p, -1.0, &|_| true).is_none());
        assert!(bvh.nearest(p, f32::NAN, &|_| true).is_none());
    }

    #[test]
    fn nearest_compares_distances_that_square_to_zero_in_f32() {
        let bvh = single_triangle(vec![Vec3::ZERO, Vec3::X, Vec3::Y], vec![0, 1, 2]);
        let p = Vec3::new(0.25, 0.25, 1e-25);
        assert!(bvh.nearest(p, 1e-26, &|_| true).is_none());
        let hit = bvh.nearest(p, 2e-25, &|_| true).unwrap();
        assert!(hit.t > 5e-26 && hit.t < 2e-25, "{hit:?}");
    }

    #[test]
    fn nearest_finds_points_on_the_edges_of_a_triangle_at_distance_zero() {
        let (a, b, c) = (Vec3::new(-1.0, -1.0, 0.0), Vec3::new(2.0, 0.0, 0.0), Vec3::new(0.0, 3.0, 0.0));
        let bvh = single_triangle(vec![a, b, c], vec![0, 1, 2]);
        // Points built on every edge, rounded to f32 where each one is made.
        for (from, to) in [(a, b), (b, c), (c, a)] {
            for k in 0..=12 {
                let p = from + (to - from) * (k as f32 / 12.0);
                assert!(bvh.nearest(p, 1e-6, &|_| true).is_some(), "{p:?}");
            }
        }
        // The midpoint of an edge is exact in f32 and is found at exactly zero.
        let mid = (b + c) * 0.5;
        assert_eq!(bvh.nearest(mid, 0.0, &|_| true).map(|h| h.t), Some(0.0));
    }

    #[test]
    fn nearest_places_a_point_inside_a_sliver_on_the_triangle() {
        let (a, b, c) = (Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 1e-5, 0.0));
        let bvh = single_triangle(vec![a, b, c], vec![0, 1, 2]);
        let p = Vec3::new(0.5, 1e-6, 0.25);
        let hit = bvh.nearest(p, 1.0, &|_| true).unwrap();
        let q = a * (1.0 - hit.u - hit.v) + b * hit.u + c * hit.v;
        assert!((hit.t - 0.25).abs() < 1e-6, "{hit:?}");
        assert!((q - Vec3::new(0.5, 1e-6, 0.0)).length() < 1e-6, "{q:?}");
    }

    #[test]
    fn traversal_stack_spills_in_lifo_order() {
        let mut st = Stack::default();
        for v in 0..200 {
            st.push((v, v as f64));
        }
        assert!((0..200).rev().all(|v| st.pop() == Some((v, v as f64))));
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
            let mut slow: Option<f64> = None;
            for t in 0..m.triangle_count() {
                let [x, y, z] = m.tri(t).unwrap();
                if let Some((tt, _, _)) = Bvh::hit_tri(&[m.positions[x], m.positions[y], m.positions[z]], o, d) {
                    slow = Some(slow.map_or(tt, |s: f64| s.min(tt)));
                }
            }
            match (fast, slow) {
                (Some(a), Some(b)) => assert!((f64::from(a) - b).abs() < 1e-4, "{a} {b}"),
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

    fn ray_triangle(a: Vec3, b: Vec3, c: Vec3) -> Bvh {
        Bvh::new(&Mesh { positions: vec![a, b, c], indices: vec![0, 1, 2], ..Mesh::default() })
    }

    #[test]
    fn empty_mesh_exports_no_nodes() {
        let empty = Bvh::new(&Mesh::default());
        assert_eq!(empty.nodes().len(), 0);
        assert!(empty.is_empty());
        assert!(empty.nearest(Vec3::ZERO, 1.0, &|_| true).is_none());
        let mut called = false;
        empty.query_box(Vec3::splat(-1.0), Vec3::splat(1.0), |_| called = true);
        assert!(!called);
    }

    #[test]
    fn small_triangle_is_hit_inside_and_missed_outside() {
        let bvh = ray_triangle(Vec3::ZERO, Vec3::X * 1e-7, Vec3::Y * 1e-7);
        let hit = bvh.closest(Vec3::new(2.5e-8, 2.5e-8, 1.0), Vec3::NEG_Z, 10.0).unwrap();
        assert!((hit.t - 1.0).abs() < 1e-6, "{}", hit.t);
        assert!(bvh.occluded(Vec3::new(2.5e-8, 2.5e-8, 1.0), Vec3::NEG_Z, 10.0));
        assert!(!bvh.occluded(Vec3::new(8e-8, 8e-8, 1.0), Vec3::NEG_Z, 10.0), "outside the hypotenuse");
        assert!(!bvh.occluded(Vec3::new(2.5e-8, 2.5e-8, 1.0), Vec3::X, 10.0), "parallel ray");
    }

    #[test]
    fn shallow_ray_still_hits_a_triangle_it_crosses() {
        let bvh = ray_triangle(Vec3::ZERO, Vec3::X, Vec3::Y);
        let hit = bvh.closest(Vec3::new(-1.0, 0.25, 1e-7), Vec3::new(1.0, 0.0, -1e-7), 10.0).unwrap();
        assert!((hit.t - 1.0).abs() < 1e-3, "{}", hit.t);
        assert!(!bvh.occluded(Vec3::new(-1.0, 0.25, 1e-7), Vec3::X, 10.0), "a ray in the plane has no hit");
    }

    #[test]
    fn isolated_large_triangle_does_not_grow_past_its_edge() {
        let bvh = ray_triangle(Vec3::ZERO, Vec3::X * 1e6, Vec3::Y * 1e6);
        assert!(bvh.occluded(Vec3::new(400_000.0, 400_000.0, 1.0), Vec3::NEG_Z, 10.0));
        assert!(!bvh.occluded(Vec3::new(500_002.0, 500_002.0, 1.0), Vec3::NEG_Z, 10.0), "2.8 units past the diagonal");
    }

    #[test]
    fn ray_through_a_shared_edge_of_large_triangles_hits() {
        let s = 1e6;
        let mesh =
            Mesh { positions: vec![Vec3::ZERO, Vec3::X * s, Vec3::new(s, s, 0.0), Vec3::Y * s], indices: vec![0, 1, 2, 0, 2, 3], ..Mesh::default() };
        let bvh = Bvh::new(&mesh);
        for x in [100_000.0, 333_333.0, 500_000.0, 987_654.0] {
            assert!(bvh.occluded(Vec3::new(x, x, 1.0), Vec3::NEG_Z, 10.0), "diagonal point {x}");
        }
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
