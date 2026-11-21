//! The internal triangle mesh.

use glam::{DVec3, Vec2, Vec3, Vec4};

use crate::{Limits, MeshOpsError, Result, check_limit, reserved, try_filled, try_vec};

/// An edge by the bits of its two end positions, the lower one first
type EdgePositions = ([u32; 3], [u32; 3]);

/// A contiguous index range drawn with one material.
#[derive(Clone, Debug, PartialEq)]
pub struct SubMesh {
    pub name: String,
    pub material: usize,
    /// First index (into [`Mesh::indices`]) and index count.
    pub start: u32,
    pub count: u32,
}

/// Triangle mesh in world space. UVs use the glTF convention (origin top-left, v down).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    /// xyz tangent, w bitangent sign.
    pub tangents: Vec<Vec4>,
    pub uvs: Vec<Vec2>,
    /// RGBA vertex colors in 0..1 (glTF: linear; OBJ, PLY: as stored): one per vertex, or empty.
    pub colors: Vec<Vec4>,
    pub indices: Vec<u32>,
    /// Index ranges per material and part (node / object); `name` is the part name.
    pub submeshes: Vec<SubMesh>,
    pub materials: Vec<String>,
    /// Per triangle, bit `i` set when edge `i` (corner `i` to corner `i + 1`) is a diagonal the
    /// importer added to split a polygon, which wireframes leave out. Empty: every edge is real.
    pub diagonals: Vec<u8>,
}

/// Hard cap on mesh size accepted from files (triangles).
pub const MAX_TRIANGLES: usize = 20_000_000;

/// Positions and UVs whose largest magnitude lies between the reciprocal of this and this go
/// into normal, tangent and unwrap arithmetic as they are. Others are divided by the largest
/// magnitude first, so edge lengths, cross products, their products with UVs and the squares of
/// those neither overflow nor underflow. The results depend only on directions, which a uniform
/// scale keeps.
const SAFE_MAGNITUDE: f32 = 1e6;

/// The smallest value [`Mesh::radius`] returns.
const MIN_RADIUS: f32 = 1e-4;

/// Normals whose largest component lies between the reciprocal of this and this go to MikkTSpace
/// as they are: every unit vector does. MikkTSpace multiplies a normal twice into products of
/// positions and UVs, so longer normals are divided first.
const SAFE_NORMAL_MAGNITUDE: f32 = 2.0;

/// Most triangles around one vertex (as MikkTSpace welds vertices) for which tangents come from
/// MikkTSpace. It visits the triangles around a vertex recursively, one call per triangle, so a
/// larger fan could exhaust the stack; such meshes get the per-vertex fallback. This crate's own
/// value, the same as the default [`Limits::max_segments`].
pub const MAX_MIKKTSPACE_VALENCE: usize = 2_048;

/// Most comparisons MikkTSpace may make welding corners, for which tangents come from it. It
/// compares each corner with the corners before it at the same position until one has the same
/// normal and UV, so `k` corners at one position can cost about `k²` comparisons; the sum of `k²`
/// over the positions stays within this. This crate's own value: the 6,144 corners at the pole of
/// a sphere with 2,048 segments take about 3.8e7.
pub const MAX_MIKKTSPACE_WELD_WORK: u64 = 1 << 27;

/// The vertex copies [`Mesh::try_compute_tangents_splitting`] may make for different tangent
/// frames.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Split {
    /// Copies while the vertex count stays within this; a copy past it is `TooLarge`.
    UpTo(usize),
    /// No copies: a vertex that needs one is an `Invalid` error naming it.
    Refuse,
}

/// `v` at unit length, also when its squared length overflows or rounds to zero. `None` for a
/// zero or non-finite vector.
pub(crate) fn unit(v: Vec3) -> Option<Vec3> {
    v.try_normalize().or_else(|| (v / v.abs().max_element()).try_normalize())
}

/// The largest absolute value, or `None` when a value is not finite.
pub(crate) fn largest_abs(values: impl IntoIterator<Item = f32>) -> Option<f32> {
    let mut largest = 0.0f32;
    for v in values {
        if !v.is_finite() {
            return None;
        }
        largest = largest.max(v.abs());
    }
    Some(largest)
}

/// The divisor that brings positions or UVs of magnitude `largest` into the safe range: 1 when
/// they are already in it, are all zero or are not finite.
pub(crate) fn safe_divisor(largest: Option<f32>) -> f32 {
    divisor_into(largest, SAFE_MAGNITUDE)
}

/// The divisor that brings values of magnitude `largest` to between `1 / limit` and `limit`: 1
/// when they are already there, are all zero or are not finite.
fn divisor_into(largest: Option<f32>, limit: f32) -> f32 {
    match largest {
        Some(m) if m > limit || (m > 0.0 && m < 1.0 / limit) => m,
        _ => 1.0,
    }
}

/// The index range of a submesh, when it starts and ends on a triangle boundary inside `len`
/// indices.
fn submesh_range(s: &SubMesh, len: usize) -> Option<std::ops::Range<usize>> {
    let end = u64::from(s.start) + u64::from(s.count);
    (s.start.is_multiple_of(3) && s.count.is_multiple_of(3) && end <= len as u64).then_some(s.start as usize..end as usize)
}

/// `listed + add` triangle visits, or `TooLarge` when that is over [`Limits::max_triangles`]
/// (`got` is `u64::MAX` when the sum overflows).
fn within_triangle_limit(listed: u64, add: usize, limits: &Limits) -> Result<u64> {
    let (got, limit) = (listed.saturating_add(add as u64), limits.max_triangles as u64);
    if got > limit { Err(MeshOpsError::TooLarge { what: "triangle count", got, limit }) } else { Ok(got) }
}

/// The parts of each of `ranges` that no earlier one holds: in the order of `ranges`, and
/// ascending within each, so every value the ranges hold comes once, from the first range that
/// holds it. Memory and time grow with the number of ranges, not with their lengths.
fn first_listings(ranges: &[std::ops::Range<usize>]) -> Result<Vec<std::ops::Range<usize>>> {
    use std::cmp::Reverse;
    let what = "wire edges";
    // Range starts in ascending order, and every start and end once, ascending.
    let mut starts = try_vec(ranges.len(), what)?;
    let mut points = try_vec(ranges.len().saturating_mul(2), what)?;
    for (i, r) in ranges.iter().enumerate().filter(|(_, r)| r.start < r.end) {
        starts.push((r.start, i));
        points.extend([r.start, r.end]);
    }
    starts.sort_unstable();
    points.sort_unstable();
    points.dedup();

    // Sweep the points, keeping the ranges that hold the current point with the earliest on top.
    let mut active = std::collections::BinaryHeap::new();
    reserved(active.try_reserve(starts.len()), what)?;
    let mut parts: Vec<(usize, std::ops::Range<usize>)> = Vec::new();
    let mut next = starts.iter().peekable();
    for (k, &p) in points.iter().enumerate() {
        while let Some(&(_, i)) = next.next_if(|(s, _)| *s == p) {
            active.push(Reverse(i));
        }
        while active.peek().is_some_and(|Reverse(i)| ranges.get(*i).is_none_or(|r| r.end <= p)) {
            active.pop();
        }
        let (Some(&Reverse(owner)), Some(&end)) = (active.peek(), points.get(k + 1)) else { continue };
        match parts.last_mut() {
            Some((last, part)) if *last == owner && part.end == p => part.end = end,
            _ => {
                reserved(parts.try_reserve(1), what)?;
                parts.push((owner, p..end));
            }
        }
    }
    parts.sort_unstable_by_key(|(owner, part)| (*owner, part.start));
    let mut out = try_vec(parts.len(), what)?;
    out.extend(parts.into_iter().map(|(_, part)| part));
    Ok(out)
}

/// Whether an array of `len` entries has one per vertex of a mesh with `n` vertices. A mesh
/// without vertices has no attributes to pass on to a mesh it is appended to.
fn per_vertex(len: usize, n: usize) -> bool {
    n > 0 && len == n
}

/// The centroid of a triangle's three UVs, summed in f64: the centroid of finite UVs is finite
/// even when their f32 sum would overflow.
fn uv_centroid(uvs: [Vec2; 3]) -> Vec2 {
    (uvs.iter().map(Vec2::as_dvec2).sum::<glam::DVec2>() / 3.0).as_vec2()
}

/// A copy of `from` in a reserved array, or `OutOfMemory` naming `what`. Copying a `Copy` element
/// does not allocate; [`try_names`] and [`try_submeshes`] copy elements that own a string.
fn try_copy<T: Copy>(from: &[T], what: &'static str) -> Result<Vec<T>> {
    let mut v = try_vec(from.len(), what)?;
    v.extend_from_slice(from);
    Ok(v)
}

/// A copy of `name` in a reserved string, or `OutOfMemory` naming `what`.
fn try_name(name: &str, what: &'static str) -> Result<String> {
    let mut s = String::new();
    reserved(s.try_reserve_exact(name.len()), what)?;
    #[cfg(test)]
    tests::name_copy(what)?;
    s.push_str(name);
    Ok(s)
}

/// Copies of `names`, each in a reserved string, or `OutOfMemory` naming `what`.
fn try_names(names: &[String], what: &'static str) -> Result<Vec<String>> {
    let mut v = try_vec(names.len(), what)?;
    for name in names {
        v.push(try_name(name, what)?);
    }
    Ok(v)
}

/// Copies of `submeshes` with their names in reserved strings, or `OutOfMemory` naming `what`.
fn try_submeshes(submeshes: &[SubMesh], what: &'static str) -> Result<Vec<SubMesh>> {
    let mut v = try_vec(submeshes.len(), what)?;
    for s in submeshes {
        v.push(SubMesh { name: try_name(&s.name, what)?, material: s.material, start: s.start, count: s.count });
    }
    Ok(v)
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Axis-aligned bounds of the finite positions (zero box when there are none). Non-finite
    /// positions are left out, so one cannot turn the box into NaN or hide a coordinate.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.positions
            .iter()
            .filter(|p| p.is_finite())
            .fold(None, |b: Option<(Vec3, Vec3)>, p| Some(b.map_or((*p, *p), |(lo, hi)| (lo.min(*p), hi.max(*p)))))
            .unwrap_or((Vec3::ZERO, Vec3::ZERO))
    }

    /// Bounding-sphere radius around the bounds center, at least 1e-4. Bounds wider than
    /// `f32::MAX` give `f32::MAX`.
    pub fn radius(&self) -> f32 {
        let (lo, hi) = self.bounds();
        let half = hi * 0.5 - lo * 0.5;
        let largest = half.abs().max_element();
        let length = if largest > SAFE_MAGNITUDE { (half / largest).length() * largest } else { (hi - lo).length() * 0.5 };
        length.clamp(MIN_RADIUS, f32::MAX)
    }

    /// Vertex indices of triangle `t`.
    pub fn tri(&self, t: usize) -> Option<[usize; 3]> {
        let start = t.checked_mul(3)?;
        let i = self.indices.get(start..start.checked_add(3)?)?;
        Some([i[0] as usize, i[1] as usize, i[2] as usize])
    }

    /// The edges wireframes draw: each vertex pair once (smaller index first), leaving out
    /// polygon diagonals ([`Mesh::diagonals`]) unless only one triangle has the edge (a UV seam
    /// split it from its twin), per material in `0..materials.len().max(1)`. A triangle that
    /// several submeshes of one material list counts once for an edge. The triangles the
    /// submeshes list (a triangle counts again for each submesh that lists it) over
    /// [`Limits::max_triangles`] are `TooLarge`.
    pub fn wire_edges(&self, limits: &Limits) -> Result<Vec<Vec<(u32, u32)>>> {
        let groups = self.ranges_by_material(limits)?;
        let mut out = try_vec(groups.len(), "wire edges")?;
        for ranges in &groups {
            // Edge → (real in some triangle, triangles using it), in first-seen order.
            let mut edges: Vec<((u32, u32), bool, u8)> = Vec::new();
            let mut at: std::collections::HashMap<(u32, u32), usize> = std::collections::HashMap::new();
            // Each listed triangle once, where its first submesh lists it.
            for r in first_listings(ranges)? {
                for t in r {
                    let Some(v) = self.indices.get(t * 3..t * 3 + 3) else { continue };
                    let hidden = self.diagonals.get(t).copied().unwrap_or(0);
                    for (i, (a, b)) in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])].into_iter().enumerate() {
                        let key = (a.min(b), a.max(b));
                        let real = hidden >> i & 1 == 0;
                        let e = match at.get(&key) {
                            Some(&e) => e,
                            None => {
                                reserved(edges.try_reserve(1), "wire edges")?;
                                reserved(at.try_reserve(1), "wire edges")?;
                                edges.push((key, false, 0));
                                at.insert(key, edges.len() - 1);
                                edges.len() - 1
                            }
                        };
                        if let Some((_, r, n)) = edges.get_mut(e) {
                            *r |= real;
                            *n = n.saturating_add(1);
                        }
                    }
                }
            }
            let mut kept = try_vec(edges.len(), "wire edges")?;
            kept.extend(edges.into_iter().filter(|(_, real, n)| *real || *n == 1).map(|(k, ..)| k));
            out.push(kept);
        }
        Ok(out)
    }

    /// Edges where the UV layout splits: two triangles meet there in 3D but not in UV space, per
    /// material in `0..materials.len().max(1)`. The triangles the submeshes list (a triangle
    /// counts again for each submesh that lists it) over [`Limits::max_triangles`] are
    /// `TooLarge`.
    pub fn uv_seam_edges(&self, limits: &Limits) -> Result<Vec<Vec<(u32, u32)>>> {
        // Signed zero denotes the same position on either side of an imported seam
        let bits = |position: Vec3| position.to_array().map(|coordinate| if coordinate == 0.0 { 0 } else { coordinate.to_bits() });
        let groups = self.ranges_by_material(limits)?;
        let mut seams: Vec<Vec<(u32, u32)>> = try_filled(groups.len(), Vec::new(), "UV seams")?;
        // Match across materials before grouping the lines for texture-set visibility
        let mut sides: std::collections::HashMap<EdgePositions, Vec<(u32, u32, usize)>> = std::collections::HashMap::new();
        for (material, ranges) in groups.iter().enumerate() {
            for triangle in ranges.iter().cloned().flatten() {
                let Some(corners) = self.indices.get(triangle * 3..triangle * 3 + 3) else { continue };

                for (from, to) in [(corners[0], corners[1]), (corners[1], corners[2]), (corners[2], corners[0])] {
                    let (Some(from_position), Some(to_position)) = (self.positions.get(from as usize), self.positions.get(to as usize)) else {
                        continue;
                    };

                    let (from_bits, to_bits) = (bits(*from_position), bits(*to_position));
                    let (edge, pair) = if from_bits <= to_bits { ((from_bits, to_bits), (from, to)) } else { ((to_bits, from_bits), (to, from)) };
                    reserved(sides.try_reserve(1), "UV seams")?;
                    let list = sides.entry(edge).or_default();
                    reserved(list.try_reserve(1), "UV seams")?;
                    list.push((pair.0, pair.1, material));
                }
            }
        }

        let uv = |index: u32| self.uvs.get(index as usize).copied();
        for pairs in sides.into_values() {
            let Some(&(first_from, first_to, _)) = pairs.first() else { continue };

            if pairs.iter().any(|&(from, to, _)| (uv(from), uv(to)) != (uv(first_from), uv(first_to))) {
                for (from, to, material) in pairs {
                    if let Some(edges) = seams.get_mut(material) {
                        reserved(edges.try_reserve(1), "UV seams")?;
                        edges.push((from.min(to), from.max(to)));
                    }
                }
            }
        }
        for edges in &mut seams {
            edges.sort_unstable();
            edges.dedup();
        }

        Ok(seams)
    }

    /// Material of triangle `t`.
    pub fn material_of(&self, t: usize) -> Option<usize> {
        let idx = u64::try_from(t.checked_mul(3)?).ok()?;
        self.submeshes.iter().find(|s| idx >= u64::from(s.start) && idx < u64::from(s.start) + u64::from(s.count)).map(|s| s.material)
    }

    /// Triangle index range belonging to material `m` (all submeshes of that material), within
    /// the index buffer.
    pub fn triangles_of(&self, m: usize) -> impl Iterator<Item = usize> + '_ {
        self.submeshes.iter().filter(move |s| s.material == m).flat_map(move |s| self.listed(s))
    }

    /// The triangles of submesh `s` within the index buffer, as [`Mesh::triangles_of`] visits
    /// them.
    fn listed(&self, s: &SubMesh) -> std::ops::Range<usize> {
        let end = (u64::from(s.start) + u64::from(s.count)).min(self.indices.len() as u64);
        (s.start / 3) as usize..(end / 3) as usize
    }

    /// How many triangles [`Mesh::triangles_of`] visits for material `m` (a triangle counts again
    /// for each submesh that lists it), or `TooLarge` when that is over [`Limits::max_triangles`].
    fn listed_of(&self, m: usize, limits: &Limits) -> Result<usize> {
        let mut listed = 0u64;
        for s in self.submeshes.iter().filter(|s| s.material == m) {
            listed = within_triangle_limit(listed, self.listed(s).len(), limits)?;
        }
        // At most `max_triangles`, a usize.
        Ok(listed as usize)
    }

    /// Whether a submesh of a material other than `m` lists a triangle that a submesh of `m`
    /// lists. The memory used grows with the number of `m`'s submeshes, not with the mesh.
    fn listed_by_other(&self, m: usize) -> Result<bool> {
        let mut own = Vec::new();
        for s in self.submeshes.iter().filter(|s| s.material == m) {
            let r = self.listed(s);
            if r.start < r.end {
                reserved(own.try_reserve(1), "material triangles")?;
                own.push(r);
            }
        }
        if own.is_empty() {
            return Ok(false);
        }
        own.sort_unstable_by_key(|r| r.start);
        // Merge overlapping and touching ranges, so the ends ascend with the starts.
        let mut merged: Vec<std::ops::Range<usize>> = try_vec(own.len(), "material triangles")?;
        for r in own {
            match merged.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => merged.push(r),
            }
        }
        Ok(self.submeshes.iter().filter(|s| s.material != m).any(|s| {
            let r = self.listed(s);
            // The first merged range that ends after `r` starts overlaps `r` when it starts before `r` ends.
            let i = merged.partition_point(|x| x.end <= r.start);
            r.start < r.end && merged.get(i).is_some_and(|x| x.start < r.end)
        }))
    }

    /// The triangle ranges of each material in `0..materials.len().max(1)`, in submesh order,
    /// from one pass over the submeshes. The triangles they list (a triangle counts again for
    /// each submesh that lists it) over [`Limits::max_triangles`] are `TooLarge`, before any
    /// memory is reserved.
    fn ranges_by_material(&self, limits: &Limits) -> Result<Vec<Vec<std::ops::Range<usize>>>> {
        let materials = self.materials.len().max(1);
        let mut listed = 0u64;
        for s in self.submeshes.iter().filter(|s| s.material < materials) {
            listed = within_triangle_limit(listed, self.listed(s).len(), limits)?;
        }
        let mut groups = try_vec(materials, "submesh ranges")?;
        groups.resize_with(materials, Vec::new);
        for s in &self.submeshes {
            if let Some(group) = groups.get_mut(s.material) {
                reserved(group.try_reserve(1), "submesh ranges")?;
                group.push(self.listed(s));
            }
        }
        Ok(groups)
    }

    /// Check that attribute arrays agree, every value is finite, every index is in range and
    /// every submesh covers whole triangles inside the index buffer.
    pub fn validate(&self) -> Result<()> {
        let n = self.positions.len();
        if self.normals.len() != n || self.uvs.len() != n || self.tangents.len() != n {
            return Err(MeshOpsError::Invalid(format!(
                "attribute counts differ: {} positions, {} normals, {} uvs, {} tangents",
                n,
                self.normals.len(),
                self.uvs.len(),
                self.tangents.len()
            )));
        }
        if !self.colors.is_empty() && self.colors.len() != n {
            return Err(MeshOpsError::Invalid(format!("{} vertex colors for {n} vertices", self.colors.len())));
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err(MeshOpsError::Invalid("index count is not a multiple of 3".into()));
        }
        if self.triangle_count() > MAX_TRIANGLES {
            return Err(MeshOpsError::Invalid(format!("mesh has more than {MAX_TRIANGLES} triangles")));
        }
        if let Some(bad) = self.indices.iter().find(|i| **i as usize >= n) {
            return Err(MeshOpsError::Invalid(format!("index {bad} out of range ({n} vertices)")));
        }
        if self.positions.iter().any(|p| !p.is_finite()) {
            return Err(MeshOpsError::Invalid("mesh has non-finite positions".into()));
        }
        if self.normals.iter().any(|v| !v.is_finite())
            || self.uvs.iter().any(|v| !v.is_finite())
            || self.tangents.iter().any(|v| !v.is_finite())
            || self.colors.iter().any(|v| !v.is_finite())
        {
            return Err(MeshOpsError::Invalid("mesh has non-finite normals, UVs, tangents or colors".into()));
        }
        for s in &self.submeshes {
            if submesh_range(s, self.indices.len()).is_none() || s.material >= self.materials.len().max(1) {
                return Err(MeshOpsError::Invalid(format!("submesh {} is out of range", s.name)));
            }
        }
        Ok(())
    }

    /// Area-weighted smooth normals over the index buffer. Leaves the normals unchanged when the
    /// memory for them cannot be reserved.
    pub fn compute_normals(&mut self) {
        let _ = self.try_compute_normals();
    }

    /// [`Mesh::compute_normals`], or `OutOfMemory` with the mesh unchanged.
    pub fn try_compute_normals(&mut self) -> Result<()> {
        self.normals = self.smooth_normals()?;
        Ok(())
    }

    /// The normals [`Mesh::compute_normals`] stores. The face products are summed in `f64`, where
    /// the cross product of any two `f32` edges neither overflows nor rounds to zero, so a small
    /// face far from the origin keeps its normal.
    pub(crate) fn smooth_normals(&self) -> Result<Vec<Vec3>> {
        let mut sums = try_filled(self.positions.len(), DVec3::ZERO, "normals")?;
        for &[a, b, c] in self.indices.as_chunks::<3>().0 {
            let (a, b, c) = (a as usize, b as usize, c as usize);
            let (Some(pa), Some(pb), Some(pc)) = (self.positions.get(a), self.positions.get(b), self.positions.get(c)) else { continue };
            let (pa, pb, pc) = (pa.as_dvec3(), pb.as_dvec3(), pc.as_dvec3());
            let face = (pb - pa).cross(pc - pa);
            for v in [a, b, c] {
                if let Some(x) = sums.get_mut(v) {
                    *x += face;
                }
            }
        }
        let mut normals = try_vec(sums.len(), "normals")?;
        normals.extend(sums.into_iter().map(|v| v.try_normalize().map_or(Vec3::Y, |n| n.as_vec3())));
        Ok(normals)
    }

    /// MikkTSpace tangents (falls back to per-triangle tangents if generation fails). A mesh with
    /// an index out of range, a non-finite position, normal or UV, only degenerate triangles,
    /// more than [`MAX_MIKKTSPACE_VALENCE`] triangles around one vertex or corners whose welding
    /// would take more than [`MAX_MIKKTSPACE_WELD_WORK`] comparisons does not go to MikkTSpace.
    /// Missing normals are computed and missing UVs are zero.
    ///
    /// MikkTSpace gives each triangle corner its own tangent frame. A vertex whose corners get
    /// different frames (UVs mirrored across it) is split: every further frame gets a copy of
    /// the vertex, and the corners with that frame use the copy. A copy whose index would not
    /// fit in `u32` is not made, and those corners keep the first frame.
    ///
    /// Leaves the mesh unchanged when the memory for the result cannot be reserved.
    pub fn compute_tangents(&mut self) {
        let _ = self.try_compute_tangents();
    }

    /// [`Mesh::compute_tangents`], or `OutOfMemory` with the mesh unchanged. MikkTSpace's own
    /// working memory is not reserved this way.
    pub fn try_compute_tangents(&mut self) -> Result<()> {
        self.try_compute_tangents_splitting(Split::UpTo(usize::MAX))
    }

    /// [`Mesh::try_compute_tangents`] with `split` deciding what happens at the first copy a
    /// split would make that `split` does not allow. The error comes before that copy is
    /// reserved, with the mesh unchanged.
    pub(crate) fn try_compute_tangents_splitting(&mut self, split: Split) -> Result<()> {
        let n = self.positions.len();
        let computed_normals = if self.normals.len() != n { Some(self.smooth_normals()?) } else { None };
        let zero_uvs = if self.uvs.len() != n { Some(try_filled(n, Vec2::ZERO, "UVs")?) } else { None };
        let normals = computed_normals.as_deref().unwrap_or(&self.normals);
        let uvs = zero_uvs.as_deref().unwrap_or(&self.uvs);
        let scales = Scales::of(&self.positions, normals, uvs);
        let mut corners = try_filled(self.indices.len(), Vec4::ZERO, "tangents")?;

        // mikktspace crashes (out-of-bounds) when every triangle is degenerate; skip it then. It
        // reads the divided positions, so the check does too. Its callbacks read a missing vertex
        // as the origin, which the degenerate check cannot see, so an index past the last vertex
        // skips it too.
        let mut usable =
            scales.finite && self.indices.iter().all(|&i| (i as usize) < n) && !self.all_triangles_degenerate_divided_by(scales.position);
        let mut g = Mikk { positions: &self.positions, normals, uvs, indices: &self.indices, scales, corners: &mut corners };
        usable = usable && g.affordable()?;
        if !usable || !mikktspace::generate_tangents(&mut g) {
            let per_vertex = simple_tangents(&self.positions, normals, uvs, &self.indices, scales)?;
            for (corner, v) in corners.iter_mut().zip(&self.indices) {
                *corner = per_vertex.get(*v as usize).copied().unwrap_or(Vec4::new(1.0, 0.0, 0.0, 1.0));
            }
        }
        // Orthonormalize a tangent against the normal of vertex `v`; repair degenerate results.
        let frame = |v: usize, t: Vec4| {
            let n = normals.get(v).copied().unwrap_or(Vec3::Y);
            // Normals outside the safe range are brought to unit length first, so the projection
            // below stays finite. A normal that is not finite counts as +Y.
            let n = if scales.normal == 1.0 && n.is_finite() { n } else { (n / scales.normal).try_normalize().unwrap_or(Vec3::Y) };
            let d = t.truncate();
            let o = (d - n * n.dot(d)).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
            o.extend(if t.w < 0.0 { -1.0 } else { 1.0 })
        };
        for (t, v) in corners.iter_mut().zip(&self.indices) {
            *t = frame(*v as usize, *t);
        }

        // One tangent per vertex: the first corner's frame, and a copy of the vertex for each
        // other frame its corners have. Corners that share a frame get bit-identical tangents.
        let bits = |t: Vec4| t.to_array().map(f32::to_bits);
        let mut tangents = try_filled(n, None::<Vec4>, "tangents")?;
        let mut copies: Vec<(u32, Vec4)> = Vec::new();
        let mut copy_of: std::collections::HashMap<(u32, [u32; 4]), u32> = std::collections::HashMap::new();
        let mut indices = try_vec(self.indices.len(), "indices")?;
        for (&v, &t) in self.indices.iter().zip(&corners) {
            let slot = tangents.get_mut(v as usize);
            let index = match slot {
                Some(slot @ None) => {
                    *slot = Some(t);
                    v
                }
                Some(Some(first)) if bits(*first) == bits(t) => v,
                Some(Some(_)) => match copy_of.get(&(v, bits(t))) {
                    Some(&copy) => copy,
                    None => match u32::try_from(n + copies.len()) {
                        Ok(copy) => {
                            match split {
                                Split::UpTo(max_vertices) => check_limit("vertex count", (copy as usize).saturating_add(1), max_vertices)?,
                                Split::Refuse => {
                                    return Err(MeshOpsError::Invalid(format!("vertex {v} is shared by triangles with different tangent frames")));
                                }
                            }
                            reserved(copies.try_reserve(1), "tangents")?;
                            reserved(copy_of.try_reserve(1), "tangents")?;
                            copies.push((v, t));
                            copy_of.insert((v, bits(t)), copy);
                            copy
                        }
                        // No index for another copy: the corner keeps the first frame.
                        Err(_) => v,
                    },
                },
                None => v,
            };
            indices.push(index);
        }

        // Reserve every array before changing any of them: reserving leaves their contents as
        // they are.
        let extra = copies.len();
        let mut out = try_vec(n + extra, "tangents")?;
        // A vertex no triangle uses gets +X made perpendicular to its normal.
        out.extend(tangents.iter().enumerate().map(|(v, t)| t.unwrap_or_else(|| frame(v, Vec4::new(1.0, 0.0, 0.0, 1.0)))));
        out.extend(copies.iter().map(|(_, t)| *t));
        let (mut normals, mut uvs) = (computed_normals, zero_uvs);
        let has_colors = self.colors.len() == n;
        let reserve = |r| reserved(r, "tangents");
        reserve(self.positions.try_reserve(extra))?;
        reserve(normals.as_mut().unwrap_or(&mut self.normals).try_reserve(extra))?;
        reserve(uvs.as_mut().unwrap_or(&mut self.uvs).try_reserve(extra))?;
        if has_colors {
            reserve(self.colors.try_reserve(extra))?;
        }

        if let Some(normals) = normals.take() {
            self.normals = normals;
        }
        if let Some(uvs) = uvs.take() {
            self.uvs = uvs;
        }
        for &(v, _) in &copies {
            let v = v as usize;
            self.positions.push(self.positions.get(v).copied().unwrap_or_default());
            self.normals.push(self.normals.get(v).copied().unwrap_or(Vec3::Y));
            self.uvs.push(self.uvs.get(v).copied().unwrap_or_default());
            if has_colors {
                self.colors.push(self.colors.get(v).copied().unwrap_or(Vec4::ONE));
            }
        }
        self.tangents = out;
        self.indices = indices;
        Ok(())
    }

    /// True when the mesh has triangles and every one of them has two corners at the same
    /// position after the positions are divided by `divisor`.
    fn all_triangles_degenerate_divided_by(&self, divisor: f32) -> bool {
        let p = |i: &u32| self.positions.get(*i as usize).map(|p| *p / divisor);
        !self.indices.is_empty()
            && self.indices.chunks(3).all(|t| match t {
                [a, b, c] => p(a) == p(b) || p(a) == p(c) || p(b) == p(c),
                _ => true,
            })
    }

    /// The UV tiles material `m` covers: integer parts of its triangles' UV centroids (v down),
    /// ascending. Non-finite UVs are skipped. The triangles the material's submeshes list (a
    /// triangle counts again for each submesh that lists it) over [`Limits::max_triangles`] are
    /// `TooLarge`.
    pub fn uv_tiles(&self, m: usize, limits: &Limits) -> Result<std::collections::BTreeSet<(i32, i32)>> {
        self.listed_of(m, limits)?;
        Ok(self
            .triangles_of(m)
            .filter_map(|t| {
                let [a, b, c] = self.tri(t)?;
                let uv = uv_centroid([*self.uvs.get(a)?, *self.uvs.get(b)?, *self.uvs.get(c)?]);
                uv.is_finite().then(|| (uv.x.floor() as i32, uv.y.floor() as i32))
            })
            .collect())
    }

    /// Remap the UVs of material `m`'s triangles with `f`. Vertices they share with other
    /// materials are duplicated first, so the other triangles keep their UVs. Errors as
    /// [`Mesh::map_material_uvs_with`].
    pub fn map_material_uvs(&mut self, m: usize, f: impl Fn(Vec2) -> Vec2, limits: &Limits) -> Result<()> {
        self.map_material_uvs_with(m, |_, uv| f(uv), limits)
    }

    /// [`Mesh::map_material_uvs`] where `f` also gets the UV centroid of the first of the
    /// material's triangles using the vertex: piecewise maps (UV tiles of different sizes) then
    /// place a vertex on a tile border with its own triangle's tile.
    ///
    /// The triangles the material's submeshes list (a triangle counts again for each submesh that
    /// lists it) over [`Limits::max_triangles`] are `TooLarge`, as are the mesh's vertices and
    /// the copies of its shared ones over [`Limits::max_vertices`]. A triangle that a submesh of
    /// another material also lists is `Invalid`, and a reservation that fails is `OutOfMemory`.
    /// Every error leaves the mesh unchanged and does not call `f`.
    pub fn map_material_uvs_with(&mut self, m: usize, f: impl Fn(Vec2, Vec2) -> Vec2, limits: &Limits) -> Result<()> {
        let listed = self.listed_of(m, limits)?;
        let n = self.positions.len();
        check_limit("vertex count", n, limits.max_vertices)?;
        if self.listed_by_other(m)? {
            return Err(MeshOpsError::Invalid("triangle belongs to more than one material".into()));
        }
        let mut material_triangles = try_vec(listed, "material triangles")?;
        material_triangles.extend(self.triangles_of(m));
        let mut in_material = try_filled(self.triangle_count(), false, "material triangles")?;
        for t in &material_triangles {
            if let Some(x) = in_material.get_mut(*t) {
                *x = true;
            }
        }

        // (vertex, place of its triangle's first corner, centroid of its triangle), three per
        // triangle. Reserved before the mesh changes, and sorted in place without allocating.
        let mut vertices = try_vec(listed.saturating_mul(3), "material vertices")?;
        // Per vertex: NOT_SHARED, SHARED with a triangle outside the material, or COPIED when it
        // is shared and a triangle of the material uses it, so it gets one copy.
        const NOT_SHARED: u8 = 0;
        const SHARED: u8 = 1;
        const COPIED: u8 = 2;
        let what = "material vertices";
        let mut shared = try_filled(n, NOT_SHARED, what)?;
        for (_, tri) in self.indices.as_chunks::<3>().0.iter().enumerate().filter(|(t, _)| !in_material.get(*t).copied().unwrap_or(false)) {
            for v in tri {
                if let Some(s) = shared.get_mut(*v as usize) {
                    *s = SHARED;
                }
            }
        }
        let mut copied = 0usize;
        for k in material_triangles.iter().flat_map(|t| t * 3..t * 3 + 3) {
            if let Some(s) = self.indices.get(k).and_then(|v| shared.get_mut(*v as usize))
                && *s == SHARED
            {
                *s = COPIED;
                copied += 1;
            }
        }

        // The vertices with their copies over `max_vertices` are `TooLarge`. The copies' storage
        // is reserved before the mesh changes; only the copies whose index fits in u32 are made.
        check_limit("vertex count", n.saturating_add(copied), limits.max_vertices)?;
        let copies = copied.min(usize::try_from(u64::from(u32::MAX) + 1).unwrap_or(usize::MAX).saturating_sub(n));
        reserved(self.positions.try_reserve(copies), what)?;
        if self.normals.len() == n {
            reserved(self.normals.try_reserve(copies), what)?;
        }
        if self.tangents.len() == n {
            reserved(self.tangents.try_reserve(copies), what)?;
        }
        if self.uvs.len() == n {
            reserved(self.uvs.try_reserve(copies), what)?;
        }
        if self.colors.len() == n {
            reserved(self.colors.try_reserve(copies), what)?;
        }
        let mut duplicates = std::collections::HashMap::new();
        reserved(duplicates.try_reserve(copies), what)?;
        for k in material_triangles.iter().flat_map(|t| t * 3..t * 3 + 3) {
            let Some(&v) = self.indices.get(k) else { continue };
            let v = if shared.get(v as usize) == Some(&COPIED) {
                match duplicates.get(&v) {
                    Some(&copy) => copy,
                    // A vertex past u32::MAX cannot be indexed: the triangle keeps the shared one.
                    None => match self.duplicate_vertex(v as usize) {
                        Some(copy) => *duplicates.entry(v).or_insert(copy),
                        None => v,
                    },
                }
            } else {
                v
            };
            if let Some(i) = self.indices.get_mut(k) {
                *i = v;
            }
        }

        for t in &material_triangles {
            let Some(tri) = self.tri(*t) else { continue };
            let centroid = uv_centroid(tri.map(|v| self.uvs.get(v).copied().unwrap_or_default()));
            let place = vertices.len();
            vertices.extend(tri.map(|v| (v, place, centroid)));
        }

        // Each vertex once, with the centroid of the first triangle using it.
        vertices.sort_unstable_by_key(|&(v, place, _)| (v, place));
        vertices.dedup_by_key(|(v, _, _)| *v);
        for (v, _, centroid) in vertices {
            if let Some(uv) = self.uvs.get_mut(v) {
                *uv = f(centroid, *uv);
            }
        }
        Ok(())
    }

    /// Append a copy of vertex `v`; returns its index, or `None` when the new index would not
    /// fit in `u32`. Normals, tangents, UVs and colors are copied where the mesh has one per
    /// vertex; an array without one per vertex is left as it is.
    fn duplicate_vertex(&mut self, v: usize) -> Option<u32> {
        let n = self.positions.len();
        let i = u32::try_from(n).ok()?;
        self.positions.push(self.positions.get(v).copied().unwrap_or_default());
        if self.normals.len() == n {
            self.normals.push(self.normals.get(v).copied().unwrap_or(Vec3::Y));
        }
        if self.tangents.len() == n {
            self.tangents.push(self.tangents.get(v).copied().unwrap_or(Vec4::new(1.0, 0.0, 0.0, 1.0)));
        }
        if self.uvs.len() == n {
            self.uvs.push(self.uvs.get(v).copied().unwrap_or_default());
        }
        if self.colors.len() == n {
            self.colors.push(self.colors.get(v).copied().unwrap_or(Vec4::ONE));
        }
        Some(i)
    }

    /// Append `other`, offsetting indices; its submeshes keep their material indices shifted by
    /// this mesh's material count. Leaves this mesh unchanged when [`Mesh::try_append`] fails.
    pub fn append(&mut self, other: &Mesh) {
        let _ = self.try_append(other);
    }

    /// [`Mesh::append`], or an error when an index of `other` is out of range or a rebased
    /// index or submesh range would not fit in `u32`. When only one side has normals, UVs or
    /// tangents for every vertex, the other side gets them (computed normals and tangents, zero
    /// UVs), so the attribute arrays stay as long as the positions; computing tangents can split
    /// vertices ([`Mesh::compute_tangents`]). A side with submeshes and no material list gets
    /// one unnamed material. A reservation that fails is `OutOfMemory`. Every error leaves this
    /// mesh unchanged.
    pub fn try_append(&mut self, other: &Mesh) -> Result<()> {
        if let Some(bad) = other.indices.iter().find(|&&i| i as usize >= other.positions.len()) {
            return Err(MeshOpsError::Invalid(format!("index {bad} out of range ({} vertices)", other.positions.len())));
        }
        let too_large = |what| MeshOpsError::Invalid(format!("the appended {what} do not fit in u32"));
        u32::try_from(self.positions.len()).map_err(|_| too_large("vertices"))?;
        // Splitting adds at most one vertex per triangle corner of a side whose tangents are
        // computed here.
        let split = |m: &Mesh, computed: bool| if computed { m.indices.len() } else { 0 };
        let tangents_needed = |m: &Mesh| !per_vertex(m.tangents.len(), m.positions.len());
        let any_tangents = per_vertex(self.tangents.len(), self.positions.len()) || per_vertex(other.tangents.len(), other.positions.len());
        let vertices = [
            self.positions.len(),
            other.positions.len(),
            split(self, any_tangents && tangents_needed(self)),
            split(other, any_tangents && tangents_needed(other)),
        ]
        .into_iter()
        .try_fold(0usize, usize::checked_add)
        .ok_or(too_large("vertices"))?;
        // The last vertex index is `vertices - 1`.
        if vertices > 0 && u32::try_from(vertices - 1).is_err() {
            return Err(too_large("vertices"));
        }
        let ibase = u32::try_from(self.indices.len()).map_err(|_| too_large("indices"))?;
        let index_count = self.indices.len().checked_add(other.indices.len()).ok_or(too_large("indices"))?;
        u32::try_from(index_count).map_err(|_| too_large("indices"))?;
        // Submeshes of a mesh without a material list use material 0, which gets an unnamed entry
        // so that it stays apart from the other mesh's materials.
        let unnamed = |m: &Mesh| m.materials.is_empty() && !m.submeshes.is_empty();
        let (self_unnamed, other_unnamed) = (unnamed(self), unnamed(other));
        let mbase = self.materials.len() + usize::from(self_unnamed);
        let mut submeshes = try_vec(other.submeshes.len(), "submeshes")?;
        for s in &other.submeshes {
            let start = s.start.checked_add(ibase).ok_or(too_large("submeshes"))?;
            start.checked_add(s.count).ok_or(too_large("submeshes"))?;
            let material = s.material.checked_add(mbase).ok_or(too_large("submeshes"))?;
            submeshes.push(SubMesh { name: try_name(&s.name, "submeshes")?, material, start, count: s.count });
        }
        let names = try_names(&other.materials, "materials")?;

        let complete = |m: &Mesh| {
            (
                per_vertex(m.normals.len(), m.positions.len()),
                per_vertex(m.uvs.len(), m.positions.len()),
                per_vertex(m.tangents.len(), m.positions.len()),
            )
        };
        let ((sn, su, st), (on, ou, ot)) = (complete(self), complete(other));
        let need_tangents = st || ot;
        let (need_normals, need_uvs) = (sn || on || need_tangents, su || ou || need_tangents);
        // A side that lacks an attribute the result needs is filled in on a copy of its vertex
        // arrays, so an error leaves both meshes unchanged.
        let filled = |m: &Mesh, (has_normals, has_uvs, has_tangents): (bool, bool, bool)| -> Result<Option<Mesh>> {
            if (has_normals || !need_normals) && (has_uvs || !need_uvs) && (has_tangents || !need_tangents) {
                return Ok(None);
            }
            let mut w = m.vertex_copy()?;
            if need_normals && !has_normals {
                w.try_compute_normals()?;
            }
            if need_uvs && !has_uvs {
                w.uvs = try_filled(w.positions.len(), Vec2::ZERO, "UVs")?;
            }
            if need_tangents && !has_tangents {
                w.try_compute_tangents()?;
            }
            Ok(Some(w))
        };
        let other_filled = filled(other, (on, ou, ot))?;
        let mut self_filled = filled(self, (sn, su, st))?;
        let o = other_filled.as_ref().unwrap_or(other);
        // Computing tangents can split vertices, so the base is taken now (checked to fit above).
        let base = self_filled.as_ref().map_or(self.positions.len(), |w| w.positions.len());
        let has_colors = !(self.colors.is_empty() && other.colors.is_empty());
        let has_diagonals = !(self.diagonals.is_empty() && other.diagonals.is_empty());

        // Reserve every array before changing any of them: reserving leaves their contents as
        // they are.
        let wanted = |on: bool, len: usize| if on { len } else { 0 };
        let diagonals = wanted(has_diagonals, (ibase as usize / 3 + other.triangle_count()).saturating_sub(self.diagonals.len()));
        reserved(self.diagonals.try_reserve(diagonals), "diagonals")?;
        let materials = other.materials.len() + usize::from(self_unnamed) + usize::from(other_unnamed);
        reserved(self.materials.try_reserve(materials), "materials")?;
        reserved(self.submeshes.try_reserve(submeshes.len()), "submeshes")?;
        let target: &mut Mesh = match self_filled.as_mut() {
            Some(w) => w,
            None => self,
        };
        reserved(target.positions.try_reserve(o.positions.len()), "positions")?;
        reserved(target.normals.try_reserve(wanted(need_normals, o.normals.len())), "normals")?;
        reserved(target.tangents.try_reserve(wanted(need_tangents, o.tangents.len())), "tangents")?;
        reserved(target.uvs.try_reserve(wanted(need_uvs, o.uvs.len())), "UVs")?;
        let colors = wanted(has_colors, (base + o.positions.len()).saturating_sub(target.colors.len()));
        reserved(target.colors.try_reserve(colors), "colors")?;
        reserved(target.indices.try_reserve(o.indices.len()), "indices")?;

        if let Some(w) = self_filled {
            self.positions = w.positions;
            self.normals = w.normals;
            self.tangents = w.tangents;
            self.uvs = w.uvs;
            self.colors = w.colors;
            self.indices = w.indices;
        }
        let base = base as u32;
        self.positions.extend_from_slice(&o.positions);
        if need_normals {
            self.normals.extend_from_slice(&o.normals);
        } else {
            self.normals.clear();
        }
        if need_tangents {
            self.tangents.extend_from_slice(&o.tangents);
        } else {
            self.tangents.clear();
        }
        if need_uvs {
            self.uvs.extend_from_slice(&o.uvs);
        } else {
            self.uvs.clear();
        }
        if has_colors {
            // Vertices of the side without colors become white.
            self.colors.resize(base as usize, Vec4::ONE);
            self.colors.extend(o.colors.iter().copied().chain(std::iter::repeat(Vec4::ONE)).take(o.positions.len()));
        }
        if has_diagonals {
            self.diagonals.resize(ibase as usize / 3, 0);
            self.diagonals.extend(other.diagonals.iter().copied().chain(std::iter::repeat(0)).take(other.triangle_count()));
        }
        self.indices.extend(o.indices.iter().map(|i| i + base));
        if self_unnamed {
            self.materials.push(String::new());
        }
        self.materials.extend(names);
        if other_unnamed {
            self.materials.push(String::new());
        }
        self.submeshes.extend(submeshes);
        Ok(())
    }

    /// A mesh holding reserved copies of this mesh's vertex attributes and indices, and nothing
    /// else, or `OutOfMemory`.
    fn vertex_copy(&self) -> Result<Mesh> {
        Ok(Mesh {
            positions: try_copy(&self.positions, "positions")?,
            normals: try_copy(&self.normals, "normals")?,
            tangents: try_copy(&self.tangents, "tangents")?,
            uvs: try_copy(&self.uvs, "UVs")?,
            colors: try_copy(&self.colors, "colors")?,
            indices: try_copy(&self.indices, "indices")?,
            ..Mesh::default()
        })
    }

    /// Uniformly scale and translate so the mesh is centered with the given bounding radius. A
    /// radius that is not a finite positive number, or is so large that a corner of the bounds
    /// would pass `f32::MAX`, leaves the mesh unchanged.
    pub fn normalize_to(&mut self, radius: f32) {
        if !(radius.is_finite() && radius > 0.0) {
            return;
        }
        let (lo, hi) = self.bounds();
        // `Mesh::radius` is the real radius above its floor; a smaller mesh, or a scale past
        // f32::MAX, takes the second form.
        let r = self.radius();
        let s = radius / r;
        if lo.abs().max(hi.abs()).max_element() <= SAFE_MAGNITUDE && r > MIN_RADIUS && s.is_finite() {
            let c = (lo + hi) * 0.5;
            if !(((lo - c) * s).is_finite() && ((hi - c) * s).is_finite()) {
                return;
            }
            for p in &mut self.positions {
                *p = (*p - c) * s;
            }
            return;
        }
        // Half-sums and a divided length keep every intermediate value finite near f32::MAX and
        // nonzero for a mesh far smaller than the floor.
        let c = lo * 0.5 + hi * 0.5;
        let half = hi * 0.5 - lo * 0.5;
        let largest = half.abs().max_element();
        if largest == 0.0 {
            // Every finite position is the center.
            for p in &mut self.positions {
                *p = *p * 0.5 - c * 0.5;
            }
            return;
        }
        let s = radius / (half / largest).length();
        let moved = |p: Vec3| (p * 0.5 - c * 0.5) / largest * s * 2.0;
        if !(moved(lo).is_finite() && moved(hi).is_finite()) {
            return;
        }
        for p in &mut self.positions {
            *p = moved(*p);
        }
    }

    /// A single-material mesh from raw attributes; computes missing normals and tangents.
    pub fn from_parts(positions: Vec<Vec3>, normals: Option<Vec<Vec3>>, uvs: Vec<Vec2>, indices: Vec<u32>, material: &str) -> Result<Mesh> {
        Self::from_parts_capped(positions, normals, uvs, indices, material, MAX_TRIANGLES)
    }

    /// [`Mesh::from_parts`] with the triangle cap as a parameter, so a test can reach it with a
    /// small mesh.
    fn from_parts_capped(
        positions: Vec<Vec3>,
        normals: Option<Vec<Vec3>>,
        uvs: Vec<Vec2>,
        indices: Vec<u32>,
        material: &str,
        max_triangles: usize,
    ) -> Result<Mesh> {
        if let Some(bad) = indices.iter().find(|&&i| i as usize >= positions.len()) {
            return Err(MeshOpsError::Invalid(format!("index {bad} out of range ({} vertices)", positions.len())));
        }
        let count = u32::try_from(indices.len()).map_err(|_| MeshOpsError::Invalid("index count does not fit in u32".into()))?;
        // The triangle cap of `validate`, checked before the normals and tangents are computed.
        if indices.len() / 3 > max_triangles {
            return Err(MeshOpsError::Invalid(format!("mesh has more than {max_triangles} triangles")));
        }
        let mut m = Mesh {
            normals: normals.unwrap_or_default(),
            uvs,
            positions,
            indices,
            tangents: Vec::new(),
            colors: Vec::new(),
            submeshes: vec![SubMesh { name: material.into(), material: 0, start: 0, count }],
            materials: vec![material.into()],
            diagonals: Vec::new(),
        };
        if m.normals.len() != m.positions.len() {
            m.try_compute_normals()?;
        }
        m.try_compute_tangents()?;
        m.validate()?;
        Ok(m)
    }

    /// A single-material mesh from plain slices, checked against `limits`: the counts, every
    /// index, every value finite, and normals and UVs (when given) one per position. Leaves
    /// normals, UVs and tangents empty when they are not given.
    pub fn from_slices(
        positions: &[[f32; 3]],
        normals: Option<&[[f32; 3]]>,
        uvs: Option<&[[f32; 2]]>,
        indices: &[u32],
        limits: &Limits,
    ) -> Result<Mesh> {
        check_limit("vertex count", positions.len(), limits.max_vertices)?;
        if !indices.len().is_multiple_of(3) {
            return Err(MeshOpsError::Invalid("index count is not a multiple of 3".into()));
        }
        check_limit("triangle count", indices.len() / 3, limits.max_triangles)?;
        let count = u32::try_from(indices.len()).map_err(|_| MeshOpsError::Invalid("index count does not fit in u32".into()))?;
        if let Some(bad) = indices.iter().find(|&&i| i as usize >= positions.len()) {
            return Err(MeshOpsError::Invalid(format!("index {bad} out of range ({} vertices)", positions.len())));
        }
        let n = positions.len();
        for (what, len) in [("normals", normals.map(<[_]>::len)), ("UVs", uvs.map(<[_]>::len))] {
            if let Some(len) = len.filter(|len| *len != n) {
                return Err(MeshOpsError::Invalid(format!("{len} {what} for {n} vertices")));
            }
        }
        let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
        if !positions.iter().all(|p| finite(p)) {
            return Err(MeshOpsError::Invalid("non-finite position".into()));
        }
        if !normals.unwrap_or_default().iter().all(|p| finite(p)) {
            return Err(MeshOpsError::Invalid("non-finite normal".into()));
        }
        if !uvs.unwrap_or_default().iter().all(|p| finite(p)) {
            return Err(MeshOpsError::Invalid("non-finite UV".into()));
        }

        let mut mesh = Mesh {
            submeshes: vec![SubMesh { name: "default".into(), material: 0, start: 0, count }],
            materials: vec!["default".into()],
            ..Mesh::default()
        };
        mesh.positions = try_vec(n, "positions")?;
        mesh.positions.extend(positions.iter().map(|p| Vec3::from_array(*p)));
        if let Some(normals) = normals {
            mesh.normals = try_vec(n, "normals")?;
            mesh.normals.extend(normals.iter().map(|p| Vec3::from_array(*p)));
        }
        if let Some(uvs) = uvs {
            mesh.uvs = try_vec(n, "UVs")?;
            mesh.uvs.extend(uvs.iter().map(|p| Vec2::from_array(*p)));
        }
        mesh.indices = try_vec(indices.len(), "indices")?;
        mesh.indices.extend_from_slice(indices);
        Ok(mesh)
    }

    /// Each triangle split into four at its edge midpoints, shared between neighbors. Leaves an
    /// unchanged copy when [`Mesh::try_subdivided`] fails.
    pub fn subdivided(&self) -> Mesh {
        self.try_subdivided(&Limits::default()).unwrap_or_else(|_| self.clone())
    }

    /// [`Mesh::subdivided`] for a mesh that passes the index and submesh checks of
    /// [`Mesh::validate`], with the result checked against `limits`. Normals, tangents, UVs and
    /// colors are interpolated when the mesh has one per vertex; submeshes keep their triangles
    /// and polygon diagonals stay hidden on the halves of their edges.
    pub fn try_subdivided(&self, limits: &Limits) -> Result<Mesh> {
        let n = self.positions.len();
        if !self.indices.len().is_multiple_of(3) {
            return Err(MeshOpsError::Invalid("index count is not a multiple of 3".into()));
        }
        if let Some(bad) = self.indices.iter().find(|&&i| i as usize >= n) {
            return Err(MeshOpsError::Invalid(format!("index {bad} out of range ({n} vertices)")));
        }
        for s in &self.submeshes {
            if submesh_range(s, self.indices.len()).is_none() || s.material >= self.materials.len().max(1) {
                return Err(MeshOpsError::Invalid(format!("submesh {} is out of range", s.name)));
            }
        }
        let triangles = self.triangle_count();
        let out_triangles = triangles.checked_mul(4).ok_or(MeshOpsError::Invalid("too many triangles".into()))?;
        check_limit("triangle count", out_triangles, limits.max_triangles)?;
        check_limit("vertex count", n, limits.max_vertices)?;
        // Each triangle adds at most three midpoints; shared edges add fewer.
        let most_vertices = n.saturating_add(triangles.saturating_mul(3));
        let out_indices = out_triangles.checked_mul(3).ok_or(MeshOpsError::Invalid("too many indices".into()))?;
        let fits = |count: usize| u32::try_from(count).is_ok();
        if !fits(out_indices) || !fits(most_vertices) {
            return Err(MeshOpsError::Invalid("the subdivided mesh does not fit in u32 indices".into()));
        }

        // Diagonals stay empty here and are built below when the input has them. The loop below
        // reserves room for each triangle's midpoints before adding them.
        let mut m = Mesh {
            positions: try_copy(&self.positions, "positions")?,
            normals: try_copy(&self.normals, "normals")?,
            tangents: try_copy(&self.tangents, "tangents")?,
            uvs: try_copy(&self.uvs, "UVs")?,
            colors: try_copy(&self.colors, "colors")?,
            indices: try_vec(out_indices, "indices")?,
            submeshes: try_submeshes(&self.submeshes, "submeshes")?,
            materials: try_names(&self.materials, "materials")?,
            diagonals: Vec::new(),
        };
        let has = |len: usize| len == n;
        let (has_n, has_t, has_uv, has_c) = (has(self.normals.len()), has(self.tangents.len()), has(self.uvs.len()), has(self.colors.len()));
        let mut mids: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
        // Half-sums: (a + b) * 0.5 overflows for coordinates near f32::MAX.
        let mid = |m: &mut Mesh, mids: &mut std::collections::HashMap<(u32, u32), u32>, a: u32, b: u32| -> u32 {
            *mids.entry((a.min(b), a.max(b))).or_insert_with(|| {
                let (i, j) = (a as usize, b as usize);
                let half = |x: Vec3, y: Vec3| x * 0.5 + y * 0.5;
                m.positions.push(half(self.positions[i], self.positions[j]));
                if has_n {
                    m.normals.push(unit(half(self.normals[i], self.normals[j])).unwrap_or(Vec3::ZERO));
                }
                if has_t {
                    let (ti, tj) = (self.tangents[i], self.tangents[j]);
                    m.tangents.push(unit(half(ti.truncate(), tj.truncate())).unwrap_or(Vec3::ZERO).extend(ti.w));
                }
                if has_uv {
                    m.uvs.push(self.uvs[i] * 0.5 + self.uvs[j] * 0.5);
                }
                if has_c {
                    m.colors.push(self.colors[i] * 0.5 + self.colors[j] * 0.5);
                }
                // At most n + 3 × triangles vertices, checked to fit above.
                (m.positions.len() - 1) as u32
            })
        };
        let oom = |_| MeshOpsError::OutOfMemory("subdivided vertices");
        for &[a, b, c] in self.indices.as_chunks::<3>().0 {
            // A triangle adds at most three midpoints.
            mids.try_reserve(3).map_err(oom)?;
            m.positions.try_reserve(3).map_err(oom)?;
            if has_n {
                m.normals.try_reserve(3).map_err(oom)?;
            }
            if has_t {
                m.tangents.try_reserve(3).map_err(oom)?;
            }
            if has_uv {
                m.uvs.try_reserve(3).map_err(oom)?;
            }
            if has_c {
                m.colors.try_reserve(3).map_err(oom)?;
            }
            let (ab, bc, ca) = (mid(&mut m, &mut mids, a, b), mid(&mut m, &mut mids, b, c), mid(&mut m, &mut mids, c, a));
            // Checked per triangle, so the result stops growing at most three vertices past the limit.
            check_limit("vertex count", m.positions.len(), limits.max_vertices)?;
            m.indices.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
        }
        // Children 0 to 2 keep the halves of the parent's edges; child 3 and the edges between
        // children are new, real edges.
        if !self.diagonals.is_empty() {
            m.diagonals = try_vec(out_triangles, "diagonals")?;
            for t in 0..triangles {
                let d = self.diagonals.get(t).copied().unwrap_or(0);
                m.diagonals.extend_from_slice(&[d & 0b101, d & 0b011, d & 0b110, 0]);
            }
        }
        for s in &mut m.submeshes {
            // Validated above: whole triangles inside an index buffer that fits in u32 after × 4.
            s.start *= 4;
            s.count *= 4;
        }
        Ok(m)
    }
}

/// The divisors that bring a mesh's positions, normals and UVs into the safe range, and whether
/// all of them are finite.
#[derive(Clone, Copy)]
struct Scales {
    position: f32,
    normal: f32,
    uv: f32,
    finite: bool,
}

impl Scales {
    fn of(positions: &[Vec3], normals: &[Vec3], uvs: &[Vec2]) -> Scales {
        let positions = largest_abs(positions.iter().flat_map(|p| p.to_array()));
        let normals = largest_abs(normals.iter().flat_map(|p| p.to_array()));
        let uvs = largest_abs(uvs.iter().flat_map(|p| p.to_array()));
        Scales {
            position: safe_divisor(positions),
            normal: divisor_into(normals, SAFE_NORMAL_MAGNITUDE),
            uv: safe_divisor(uvs),
            finite: positions.is_some() && normals.is_some() && uvs.is_some(),
        }
    }
}

/// Per-vertex tangents from the UV gradients of the triangles around each vertex: the fallback
/// when MikkTSpace does not run or fails. `w` is -1 where the summed direction up the image
/// (toward smaller v) lies against `cross(normal, tangent)`, and +1 otherwise, as MikkTSpace
/// sets it. Edges, UV differences and sums are taken in `f64`, so an edge too small to survive
/// division by the position scale in `f32` still sets the direction.
fn simple_tangents(positions: &[Vec3], normals: &[Vec3], uvs: &[Vec2], indices: &[u32], scales: Scales) -> Result<Vec<Vec4>> {
    let mut sums = try_filled(positions.len(), (DVec3::ZERO, DVec3::ZERO), "tangents")?;
    let (position_scale, uv_scale) = (f64::from(scales.position), f64::from(scales.uv));
    for &[a, b, c] in indices.as_chunks::<3>().0 {
        let (a, b, c) = (a as usize, b as usize, c as usize);
        let (Some(pa), Some(pb), Some(pc)) = (positions.get(a), positions.get(b), positions.get(c)) else { continue };
        let (Some(ua), Some(ub), Some(uc)) = (uvs.get(a), uvs.get(b), uvs.get(c)) else { continue };
        let (pa, pb, pc) = (pa.as_dvec3(), pb.as_dvec3(), pc.as_dvec3());
        let (ua, ub, uc) = (ua.as_dvec2(), ub.as_dvec2(), uc.as_dvec2());
        let (e1, e2) = ((pb - pa) / position_scale, (pc - pa) / position_scale);
        let (d1, d2) = ((ub - ua) / uv_scale, (uc - ua) / uv_scale);
        let determinant = d1.x * d2.y - d2.x * d1.y;
        if determinant.abs() < 1e-12 {
            continue;
        }

        let tangent = (e1 * d2.y - e2 * d1.y) / determinant;
        // The direction in which v shrinks, which is up the image.
        let up = (e1 * d2.x - e2 * d1.x) / determinant;
        for v in [a, b, c] {
            if let Some((t, u)) = sums.get_mut(v) {
                *t += tangent;
                *u += up;
            }
        }
    }

    let mut tangents = try_vec(sums.len(), "tangents")?;
    tangents.extend(sums.into_iter().enumerate().map(|(v, (t, up))| {
        let t = t.try_normalize().map(|t| t.as_vec3()).and_then(unit).unwrap_or(Vec3::X);
        let up = up.try_normalize().map(|up| up.as_vec3()).unwrap_or(Vec3::ZERO);
        let n = normals.get(v).copied().and_then(unit).unwrap_or(Vec3::Y);
        let w = if n.cross(t).dot(up) < 0.0 { -1.0 } else { 1.0 };
        t.extend(w)
    }));
    Ok(tangents)
}

/// The arrays MikkTSpace reads, and the tangent of every triangle corner it writes.
struct Mikk<'a> {
    positions: &'a [Vec3],
    normals: &'a [Vec3],
    uvs: &'a [Vec2],
    indices: &'a [u32],
    scales: Scales,
    corners: &'a mut [Vec4],
}

impl Mikk<'_> {
    fn vi(&self, face: usize, vert: usize) -> usize {
        self.indices.get(face * 3 + vert).copied().unwrap_or(0) as usize
    }

    /// Whether MikkTSpace's work on this input stays bounded: at most
    /// [`MAX_MIKKTSPACE_VALENCE`] triangles around one vertex, which it visits recursively, one
    /// call per triangle, and at most [`MAX_MIKKTSPACE_WELD_WORK`] for welding.
    fn affordable(&self) -> Result<bool> {
        let (valence, weld_work) = self.welding()?;
        Ok(valence <= MAX_MIKKTSPACE_VALENCE && weld_work <= MAX_MIKKTSPACE_WELD_WORK)
    }

    /// The most triangles that share one vertex as MikkTSpace sees it, and the sum of the squared
    /// corner counts of its positions. It welds corners whose position, normal and UV, as this
    /// type passes them, are equal (0 and -0 alike).
    fn welding(&self) -> Result<(usize, u64)> {
        use mikktspace::Geometry;
        let bits = |x: f32| if x == 0.0 { 0 } else { x.to_bits() };
        let room = self.positions.len().min(self.indices.len());
        let mut count: std::collections::HashMap<[u32; 8], usize> = std::collections::HashMap::new();
        let mut at_position: std::collections::HashMap<[u32; 3], u64> = std::collections::HashMap::new();
        reserved(count.try_reserve(room), "tangents")?;
        reserved(at_position.try_reserve(room), "tangents")?;
        let (mut most, mut work) = (0, 0u64);
        for face in 0..self.num_faces() {
            for vert in 0..3 {
                let ([px, py, pz], [nx, ny, nz], [u, v]) = (self.position(face, vert), self.normal(face, vert), self.tex_coord(face, vert));
                let key = [px, py, pz, nx, ny, nz, u, v].map(bits);
                if !count.contains_key(&key) {
                    reserved(count.try_reserve(1), "tangents")?;
                }
                let c = count.entry(key).or_insert(0);
                *c += 1;
                most = most.max(*c);
                let position = [px, py, pz].map(bits);
                if !at_position.contains_key(&position) {
                    reserved(at_position.try_reserve(1), "tangents")?;
                }
                // k² grows by 2k + 1 with the (k + 1)th corner.
                let k = at_position.entry(position).or_insert(0);
                work = work.saturating_add(k.saturating_mul(2).saturating_add(1));
                *k += 1;
            }
        }
        Ok((most, work))
    }
}

impl mikktspace::Geometry for Mikk<'_> {
    fn num_faces(&self) -> usize {
        self.indices.len() / 3
    }
    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }
    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        (self.positions.get(self.vi(face, vert)).copied().unwrap_or_default() / self.scales.position).to_array()
    }
    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        (self.normals.get(self.vi(face, vert)).copied().unwrap_or(Vec3::Y) / self.scales.normal).to_array()
    }
    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        // MikkTSpace expects OpenGL texture coordinates (v up); our UVs are glTF (v down). With
        // v negated, the bitangent `cross(N, T) * w` points up the image, as glTF requires.
        // Negating keeps differences in v that `1 - v` would round away.
        let uv = self.uvs.get(self.vi(face, vert)).copied().unwrap_or_default() / self.scales.uv;
        [uv.x, -uv.y]
    }
    fn set_tangent_encoded(&mut self, tangent: [f32; 4], face: usize, vert: usize) {
        // One tangent per corner: corners of a vertex can get different frames.
        if let Some(t) = self.corners.get_mut(face * 3 + vert) {
            *t = Vec4::from_array(tangent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Name copies that succeed before the next one fails as out of memory; `None` lets every
        /// copy succeed.
        static NAME_COPIES_LEFT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    }

    /// Let `copies` name copies on this thread succeed and fail the next one, or none with `None`.
    fn fail_name_copy_after(copies: Option<usize>) {
        NAME_COPIES_LEFT.with(|left| left.set(copies));
    }

    /// `OutOfMemory` naming `what` when [`fail_name_copy_after`] says this name copy fails.
    pub(super) fn name_copy(what: &'static str) -> Result<()> {
        NAME_COPIES_LEFT.with(|left| match left.get() {
            Some(0) => {
                left.set(None);
                Err(MeshOpsError::OutOfMemory(what))
            }
            Some(k) => {
                left.set(Some(k - 1));
                Ok(())
            }
            None => Ok(()),
        })
    }

    fn quad() -> Mesh {
        Mesh::from_parts(
            vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)],
            None,
            vec![Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 0.0)],
            vec![0, 1, 2, 0, 2, 3],
            "Default",
        )
        .unwrap()
    }

    #[test]
    fn normals_and_tangents_of_a_quad() {
        let m = quad();
        for n in &m.normals {
            assert!((*n - Vec3::Z).length() < 1e-5);
        }
        for t in &m.tangents {
            assert!((t.truncate() - Vec3::X).length() < 1e-4, "{t:?}");
        }
        assert_eq!(m.material_of(1), Some(0));
        assert_eq!(m.triangles_of(0).collect::<Vec<_>>(), vec![0, 1]);
    }

    #[test]
    fn wire_edges_leave_out_polygon_diagonals() {
        let l = Limits::default();
        let mut m = quad();
        assert_eq!(m.wire_edges(&l).unwrap(), vec![vec![(0, 1), (1, 2), (0, 2), (2, 3), (0, 3)]], "triangles show every edge");
        // Edge 2 of (0, 1, 2) and edge 0 of (0, 2, 3) split the quad.
        m.diagonals = vec![0b100, 0b001];
        assert_eq!(m.wire_edges(&l).unwrap(), vec![vec![(0, 1), (1, 2), (2, 3), (0, 3)]]);
        // A diagonal whose twin a seam split off (vertex 4 copies vertex 0) stays.
        let mut seam = m.clone();
        seam.indices[3] = 4;
        assert_eq!(seam.wire_edges(&l).unwrap()[0].len(), 6);
        // Appending keeps each side's diagonals on its own triangles.
        let mut a = Mesh::from_parts(m.positions.clone(), None, m.uvs.clone(), vec![0, 1, 2], "A").unwrap();
        a.append(&m);
        assert_eq!(a.diagonals, vec![0, 0b100, 0b001]);
        assert_eq!(crate::primitives::plane(1.0, 2).wire_edges(&l).unwrap()[0].len(), 12, "a 2×2 grid of quads has 12 edges");
    }

    #[test]
    fn uv_seams_are_where_the_uv_layout_splits() {
        let l = Limits::default();
        let mesh = quad();
        assert_eq!(mesh.uv_seam_edges(&l).unwrap(), vec![Vec::<(u32, u32)>::new()], "one UV island has no seam");

        // Vertex 4 copies vertex 0's position with another UV: the diagonal (0, 2) / (4, 2) is cut
        let mut seam = mesh.clone();
        seam.positions.push(seam.positions[0]);
        seam.normals.push(seam.normals[0]);
        seam.tangents.push(seam.tangents[0]);
        seam.uvs.push(Vec2::new(0.9, 0.9));
        seam.indices[3] = 4;
        assert_eq!(seam.uv_seam_edges(&l).unwrap(), vec![vec![(0, 2), (2, 4)]]);

        // The same split with the same UV is a hard edge, not a seam
        let mut hard = seam.clone();
        hard.uvs[4] = hard.uvs[0];
        assert_eq!(hard.uv_seam_edges(&l).unwrap(), vec![Vec::<(u32, u32)>::new()]);
    }

    #[test]
    fn uv_seams_join_positions_with_opposite_signed_zero() {
        let mut mesh = quad();
        mesh.positions.push(Vec3::new(-0.0, 0.0, 0.0));
        mesh.normals.push(mesh.normals[0]);
        mesh.tangents.push(mesh.tangents[0]);
        mesh.uvs.push(Vec2::new(0.9, 0.9));
        mesh.indices[3] = 4;
        assert_eq!(mesh.positions[0], mesh.positions[4]);
        assert_eq!(mesh.uv_seam_edges(&Limits::default()).unwrap(), vec![vec![(0, 2), (2, 4)]]);
    }

    #[test]
    fn uv_seams_between_materials_belong_to_both_materials() {
        let mut mesh = quad();
        mesh.positions.push(mesh.positions[0]);
        mesh.normals.push(mesh.normals[0]);
        mesh.tangents.push(mesh.tangents[0]);
        mesh.uvs.push(Vec2::new(0.9, 0.9));
        mesh.indices[3] = 4;
        mesh.materials.push("second".into());
        mesh.submeshes[0].count = 3;
        mesh.submeshes.push(SubMesh { name: "second".into(), material: 1, start: 3, count: 3 });

        let l = Limits::default();
        assert_eq!(mesh.uv_seam_edges(&l).unwrap(), vec![vec![(0, 2)], vec![(2, 4)]]);
        mesh.uvs[4] = mesh.uvs[0];
        assert_eq!(mesh.uv_seam_edges(&l).unwrap(), vec![Vec::<(u32, u32)>::new(); 2], "a material boundary alone is not a UV seam");
    }

    #[test]
    fn material_uvs_leave_absent_attributes_absent() {
        // UVs only, two materials sharing the diagonal's vertices 0 and 2.
        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let mut m = Mesh::from_slices(&positions, None, Some(&uvs), &[0, 1, 2, 0, 2, 3], &Limits::default()).unwrap();
        m.materials = vec!["a".into(), "b".into()];
        m.submeshes =
            vec![SubMesh { name: "a".into(), material: 0, start: 0, count: 3 }, SubMesh { name: "b".into(), material: 1, start: 3, count: 3 }];
        m.map_material_uvs(1, |uv| uv * 0.5, &Limits::default()).unwrap();
        assert_eq!(m.positions.len(), 6, "vertices 0 and 2 are copied for material 1");
        assert_eq!(m.uvs.len(), 6);
        assert!(m.normals.is_empty() && m.tangents.is_empty() && m.colors.is_empty(), "{m:?}");
    }

    #[test]
    fn validate_catches_bad_indices() {
        let mut m = quad();
        m.indices[0] = 99;
        assert!(m.validate().is_err());
        let mut m = quad();
        m.uvs.pop();
        assert!(m.validate().is_err());
    }

    #[test]
    fn append_offsets_indices_and_materials() {
        let mut a = quad();
        let b = quad();
        a.append(&b);
        assert_eq!(a.indices[6], 4);
        assert_eq!(a.submeshes[1].material, 1);
        assert_eq!(a.submeshes[1].start, 6);
        a.validate().unwrap();
        // Colors on one side only: the other side's vertices become white.
        let mut red = quad();
        red.colors = vec![Vec4::new(1.0, 0.0, 0.0, 1.0); 4];
        a.append(&red);
        assert_eq!(a.colors.len(), 12);
        assert_eq!((a.colors[0], a.colors[8]), (Vec4::ONE, Vec4::new(1.0, 0.0, 0.0, 1.0)));
        a.append(&quad());
        assert_eq!(a.colors[15], Vec4::ONE);
        a.validate().unwrap();
        a.colors.pop();
        assert!(a.validate().is_err());
    }

    #[test]
    fn material_uvs_remap_and_split_shared_vertices() {
        // quad(): two triangles sharing an edge; put the second in its own material.
        let mut m = quad();
        m.materials = vec!["a".into(), "b".into()];
        m.submeshes =
            vec![SubMesh { name: "a".into(), material: 0, start: 0, count: 3 }, SubMesh { name: "b".into(), material: 1, start: 3, count: 3 }];
        let before = m.clone();
        let l = Limits::default();
        m.map_material_uvs(1, |uv| uv + Vec2::X, &l).unwrap();
        m.validate().unwrap();
        assert_eq!(m.vertex_count(), 6, "the two shared vertices are split");
        assert_eq!(&m.indices[..3], &before.indices[..3]);
        for k in 0..3 {
            let (old, new) = (before.indices[k] as usize, m.indices[k] as usize);
            assert_eq!(m.uvs[new], before.uvs[old], "material a keeps its UVs");
            let (old, new) = (before.indices[3 + k] as usize, m.indices[3 + k] as usize);
            assert_eq!((m.uvs[new], m.positions[new]), (before.uvs[old] + Vec2::X, before.positions[old]));
        }
        assert_eq!(m.uv_tiles(1, &l).unwrap().into_iter().collect::<Vec<_>>(), vec![(1, 0)]);
        assert_eq!(m.uv_tiles(0, &l).unwrap().into_iter().collect::<Vec<_>>(), vec![(0, 0)]);
    }

    #[test]
    fn material_uvs_pass_each_vertex_the_centroid_of_its_first_triangle() {
        // A fan of triangles t = (0, t + 1, t + 2): vertex 0 is in every triangle, and vertex
        // v >= 2 is first in triangle v - 2.
        let fan = 40u32;
        let positions = (0..=fan + 1).map(|i| Vec3::new(i as f32, (i * i) as f32, 0.0)).collect();
        let uvs: Vec<Vec2> = (0..=fan + 1).map(|i| Vec2::new(i as f32, 0.5 * i as f32 + 0.25)).collect();
        let indices = (0..fan).flat_map(|t| [0, t + 1, t + 2]).collect();
        let mut m = Mesh::from_parts(positions, None, uvs.clone(), indices, "a").unwrap();
        let centroid = |t: u32| (uvs[0] + uvs[t as usize + 1] + uvs[t as usize + 2]) / 3.0;
        m.map_material_uvs_with(0, |c, _| c, &Limits::default()).unwrap();
        assert_eq!((m.uvs[0], m.uvs[1]), (centroid(0), centroid(0)));
        for v in 2..=fan + 1 {
            assert_eq!(m.uvs[v as usize], centroid(v - 2), "vertex {v}");
        }
    }

    #[test]
    fn uv_centroids_of_large_finite_uvs_stay_finite() {
        // Three UVs of 1.5e38 sum past f32::MAX; their centroid is 1.5e38.
        let big = Vec2::splat(1.5e38);
        let mut m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, vec![big; 3], vec![0, 1, 2], "a").unwrap();
        let l = Limits::default();
        assert_eq!(m.uv_tiles(0, &l).unwrap().into_iter().collect::<Vec<_>>(), vec![(i32::MAX, i32::MAX)]);
        m.map_material_uvs_with(0, |c, _| c, &l).unwrap();
        assert_eq!(m.uvs, vec![big; 3]);
    }

    /// One triangle in UV tile (0, 0) that `listings` submeshes of material 0 all list.
    fn listed_triangle(listings: usize) -> Mesh {
        let uvs = vec![Vec2::new(0.1, 0.1), Vec2::new(0.9, 0.1), Vec2::new(0.1, 0.9)];
        let mut m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, uvs, vec![0, 1, 2], "a").unwrap();
        m.submeshes = vec![SubMesh { name: "a".into(), material: 0, start: 0, count: 3 }; listings];
        m
    }

    fn at_most(triangles: usize) -> Limits {
        Limits { max_triangles: triangles, ..Limits::default() }
    }

    fn too_many(got: u64, limit: u64) -> MeshOpsError {
        MeshOpsError::TooLarge { what: "triangle count", got, limit }
    }

    #[test]
    fn listed_triangles_bound_wire_edges() {
        let m = listed_triangle(5);
        m.validate().unwrap();
        assert_eq!(m.wire_edges(&at_most(4)), Err(too_many(5, 4)));
        assert_eq!(too_many(5, 4).to_string(), "triangle count 5 is over the limit of 4");
        assert_eq!(m.wire_edges(&at_most(5)).unwrap(), vec![vec![(0, 1), (1, 2), (0, 2)]]);
    }

    #[test]
    fn listed_triangles_bound_uv_seams() {
        let m = listed_triangle(5);
        assert_eq!(m.uv_seam_edges(&at_most(4)), Err(too_many(5, 4)));
        assert_eq!(m.uv_seam_edges(&at_most(5)).unwrap(), vec![Vec::<(u32, u32)>::new()]);
    }

    #[test]
    fn listed_triangles_bound_uv_tiles() {
        let m = listed_triangle(5);
        assert_eq!(m.uv_tiles(0, &at_most(4)), Err(too_many(5, 4)));
        assert_eq!(m.uv_tiles(0, &at_most(5)).unwrap().into_iter().collect::<Vec<_>>(), vec![(0, 0)]);
    }

    #[test]
    fn material_mapping_bounds_split_vertices() {
        // quad(): the second triangle in its own material shares two vertices with the first.
        let mut m = quad();
        m.materials = vec!["a".into(), "b".into()];
        m.submeshes =
            vec![SubMesh { name: "a".into(), material: 0, start: 0, count: 3 }, SubMesh { name: "b".into(), material: 1, start: 3, count: 3 }];
        let before = m.clone();
        let calls = std::cell::Cell::new(0);
        let f = |uv: Vec2| {
            calls.set(calls.get() + 1);
            uv + Vec2::X
        };
        let vertices = |max_vertices| Limits { max_vertices, ..Limits::default() };
        for limit in [4, 5] {
            let too_large = MeshOpsError::TooLarge { what: "vertex count", got: 6, limit: limit as u64 };
            assert_eq!(m.map_material_uvs(1, f, &vertices(limit)), Err(too_large));
            assert_eq!((calls.get(), &m), (0, &before), "nothing is mapped or changed");
        }
        m.map_material_uvs(1, f, &vertices(6)).unwrap();
        assert_eq!((calls.get(), m.vertex_count()), (3, 6));
        m.validate().unwrap();

        // A mesh over the limit without any shared vertex.
        let mut one = listed_triangle(1);
        let before = one.clone();
        assert_eq!(one.map_material_uvs(0, f, &vertices(2)), Err(MeshOpsError::TooLarge { what: "vertex count", got: 3, limit: 2 }));
        assert_eq!((calls.get(), &one), (3, &before));
        one.map_material_uvs(0, f, &vertices(3)).unwrap();
        assert_eq!((calls.get(), one.vertex_count()), (6, 3));
    }

    #[test]
    fn listed_triangles_bound_material_mapping() {
        let mut m = listed_triangle(5);
        let before = m.clone();
        let calls = std::cell::Cell::new(0);
        let f = |uv: Vec2| {
            calls.set(calls.get() + 1);
            uv + Vec2::X
        };
        assert_eq!(m.map_material_uvs(0, f, &at_most(4)), Err(too_many(5, 4)));
        assert_eq!((calls.get(), &m), (0, &before), "nothing is mapped or changed");
        m.map_material_uvs(0, f, &at_most(5)).unwrap();
        assert_eq!(calls.get(), 3, "each vertex is mapped once");
        m.validate().unwrap();
        assert_eq!(m.uvs, before.uvs.iter().map(|uv| *uv + Vec2::X).collect::<Vec<_>>());
    }

    #[test]
    fn repeated_listing_keeps_single_hidden_edge() {
        let mut once = listed_triangle(1);
        once.diagonals = vec![0b001];
        let mut twice = once.clone();
        twice.submeshes.push(twice.submeshes[0].clone());
        let edges = once.wire_edges(&at_most(1)).unwrap();
        assert_eq!(edges, vec![vec![(0, 1), (1, 2), (0, 2)]], "a hidden edge with one triangle stays");
        assert_eq!(twice.wire_edges(&at_most(2)).unwrap(), edges);

        // A quad whose diagonal a seam split (vertex 4 copies vertex 0): each half keeps its
        // diagonal, also when a second submesh lists the second half again.
        let mut seam = quad();
        seam.positions.push(seam.positions[0]);
        seam.normals.push(seam.normals[0]);
        seam.tangents.push(seam.tangents[0]);
        seam.uvs.push(seam.uvs[0]);
        seam.indices[3] = 4;
        seam.diagonals = vec![0b100, 0b001];
        let edges = seam.wire_edges(&at_most(2)).unwrap();
        assert_eq!(edges[0].len(), 6, "{edges:?}");
        seam.submeshes.push(SubMesh { name: "again".into(), material: 0, start: 3, count: 3 });
        assert_eq!(seam.wire_edges(&at_most(3)).unwrap(), edges);
    }

    #[test]
    fn first_listings_of_overlapping_ranges() {
        // A repeated range, the gaps around and between earlier ranges, a range touching an
        // earlier one, a range over all of them and an empty range.
        let ranges = [10..20, 10..20, 30..40, 5..45, 45..50, 0..60, 7..7];
        assert_eq!(first_listings(&ranges).unwrap(), vec![10..20, 30..40, 5..10, 20..30, 40..45, 45..50, 0..5, 50..60]);
        // A later range inside an earlier one adds nothing; an earlier range inside a later one
        // splits it.
        assert_eq!(first_listings(&[0..10, 2..4]).unwrap(), vec![0..10]);
        assert_eq!(first_listings(&[2..4, 0..10]).unwrap(), vec![2..4, 0..2, 4..10]);
        assert_eq!(first_listings(&[]).unwrap(), Vec::<std::ops::Range<usize>>::new());
    }

    #[test]
    fn mapping_rejects_cross_material_triangle_overlap() {
        let mut m = listed_triangle(1);
        m.materials.push("b".into());
        m.submeshes.push(SubMesh { name: "b".into(), material: 1, start: 0, count: 3 });
        m.validate().unwrap();
        let before = m.clone();
        let calls = std::cell::Cell::new(0);
        let result = m.map_material_uvs(
            0,
            |uv| {
                calls.set(calls.get() + 1);
                uv
            },
            &Limits::default(),
        );
        assert_eq!(result, Err(MeshOpsError::Invalid("triangle belongs to more than one material".into())));
        assert_eq!((calls.get(), &m), (0, &before), "nothing is mapped or changed");

        // Material a lists both triangles of a quad; b lists the second one, after a range of a
        // that ends where b starts.
        let mut quad2 = quad();
        quad2.materials = vec!["a".into(), "b".into()];
        let sub = |material, start| SubMesh { name: String::new(), material, start, count: 3 };
        quad2.submeshes = vec![sub(0, 0), sub(1, 3)];
        assert_eq!(quad2.listed_by_other(0), Ok(false), "adjacent ranges do not overlap");
        quad2.submeshes.push(sub(0, 3));
        let invalid = Err(MeshOpsError::Invalid("triangle belongs to more than one material".into()));
        assert_eq!(quad2.map_material_uvs(0, |uv| uv, &Limits::default()), invalid);
        assert_eq!(quad2.map_material_uvs(1, |uv| uv, &Limits::default()), invalid);
    }

    #[test]
    fn edges_group_submesh_ranges_by_material_in_order() {
        // Material b lists triangle 1, then triangle 0; material c lists none.
        let mut m = quad();
        m.materials = vec!["a".into(), "b".into(), "c".into()];
        m.submeshes = vec![
            SubMesh { name: "b".into(), material: 1, start: 3, count: 3 },
            SubMesh { name: "a".into(), material: 0, start: 0, count: 3 },
            SubMesh { name: "b".into(), material: 1, start: 0, count: 3 },
        ];
        assert_eq!(m.ranges_by_material(&at_most(3)).unwrap(), vec![vec![0..1], vec![1..2, 0..1], vec![]]);
        assert_eq!(m.ranges_by_material(&at_most(2)), Err(too_many(3, 2)));
        assert_eq!(m.wire_edges(&at_most(3)).unwrap(), vec![vec![(0, 1), (1, 2), (0, 2)], vec![(0, 2), (2, 3), (0, 3), (0, 1), (1, 2)], vec![]]);
        assert_eq!(m.uv_seam_edges(&at_most(3)).unwrap(), vec![Vec::<(u32, u32)>::new(); 3]);
    }

    #[test]
    fn subdivision_shares_edge_midpoints() {
        let m = quad().subdivided();
        // 4 corners + 5 edge midpoints (the diagonal is shared), 8 triangles.
        assert_eq!((m.positions.len(), m.triangle_count()), (9, 8));
        assert_eq!(m.uvs.len(), 9);
        assert_eq!((m.submeshes[0].start, m.submeshes[0].count), (0, 24));
        m.validate().unwrap();
    }

    /// An index past the last vertex reads as the origin in MikkTSpace's callbacks. With two
    /// positions at the origin and one missing, the degenerate-mesh guard (which sees the missing
    /// vertex as different) let it through while MikkTSpace saw every triangle as degenerate and
    /// underflowed: a panic in debug builds, an out-of-bounds walk in release.
    #[test]
    fn from_parts_refuses_an_out_of_range_index_without_crashing() {
        let positions = vec![Vec3::ZERO, Vec3::X];
        let uvs = vec![Vec2::ZERO, Vec2::X];
        let e = Mesh::from_parts(positions, None, uvs, vec![0, 1, 2], "M").unwrap_err();
        assert!(e.to_string().contains("out of range"), "{e}");
    }

    /// `from_parts` refuses a mesh over its triangle cap before it computes normals and tangents.
    /// With a cap of one triangle, a two-triangle mesh is refused, though `validate` passes it.
    #[test]
    fn from_parts_checks_the_triangle_cap_before_computing_attributes() {
        let positions = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::ONE];
        let uvs = vec![Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::ONE];
        let e = Mesh::from_parts_capped(positions, None, uvs, vec![0, 1, 2, 1, 3, 2], "M", 1).unwrap_err();
        assert!(e.to_string().contains("more than 1 triangles"), "{e}");
    }

    #[test]
    fn tangents_of_a_mesh_with_an_out_of_range_index_fall_back_without_crashing() {
        for indices in [vec![0, 1, 2], vec![0, 0, 2], vec![0, 1, 5, 0, 1, 1]] {
            let mut m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, vec![Vec2::ZERO, Vec2::X, Vec2::Y], vec![0, 1, 2], "M").unwrap();
            m.positions.truncate(2);
            m.normals.truncate(2);
            m.uvs.truncate(2);
            m.indices = indices;
            m.compute_tangents();
            assert_eq!(m.tangents.len(), 2);
            assert!(m.tangents.iter().all(|t| t.is_finite()), "{:?}", m.tangents);
        }
    }

    #[test]
    fn submesh_ranges_and_append_reject_overflow() {
        // A submesh whose end passes u32::MAX: no triangles, and no overflow.
        let mut m = quad();
        m.submeshes[0] = SubMesh { name: "far".into(), material: 0, start: u32::MAX - 2, count: 6 };
        assert_eq!(m.triangles_of(0).count(), 0);
        assert_eq!(m.material_of(1), None);

        // Rebasing a submesh past u32::MAX is an error that leaves the mesh as it was.
        let mut far = quad();
        far.submeshes[0].start = u32::MAX - 5;
        let mut a = quad();
        let before = a.clone();
        assert!(a.try_append(&far).is_err());
        a.append(&far);
        assert_eq!(a, before);
    }

    #[test]
    fn tangents_reject_non_finite_and_survive_extreme_positions() {
        let quad_at = |s: f32| [[0.0, 0.0, 0.0], [s, 0.0, 0.0], [s, s, 0.0], [0.0, s, 0.0]];
        let normals = [[0.0, 0.0, 1.0]; 4];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let indices = [0, 1, 2, 0, 2, 3];
        let limits = Limits::default();
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut positions = quad_at(1.0);
            positions[2][1] = bad;
            assert!(crate::tangents(&positions, &normals, &uvs, &indices, &limits).is_err(), "{bad}");
            let mut normal = normals;
            normal[1][0] = bad;
            assert!(crate::tangents(&quad_at(1.0), &normal, &uvs, &indices, &limits).is_err(), "normal {bad}");
            let mut uv = uvs;
            uv[3][1] = bad;
            assert!(crate::tangents(&quad_at(1.0), &normals, &uv, &indices, &limits).is_err(), "uv {bad}");
            // The Mesh form falls back instead of handing the value to MikkTSpace.
            let mut m = Mesh { positions: positions.iter().map(|p| Vec3::from_array(*p)).collect(), indices: indices.to_vec(), ..Mesh::default() };
            m.compute_tangents();
            assert_eq!(m.tangents.len(), 4);
        }
        // Near f32::MAX the edge products overflow unless the positions are scaled first.
        for s in [3e38f32, 1e30, 1e9, 1e6, 1e-6, 1e-9, 1e-30] {
            let t = crate::tangents(&quad_at(s), &normals, &uvs, &indices, &limits).unwrap();
            for t in &t {
                assert!((Vec4::from_array(*t) - Vec4::new(1.0, 0.0, 0.0, 1.0)).length() < 1e-4, "scale {s}: {t:?}");
            }
            let n = crate::smooth_normals(&quad_at(s), &indices, &limits).unwrap();
            assert!(n.iter().all(|n| (Vec3::from_array(*n) - Vec3::Z).length() < 1e-5), "scale {s}: {n:?}");
        }
        let large_uvs = uvs.map(|uv| [uv[0] * 3e38, uv[1] * 3e38]);
        let t = crate::tangents(&quad_at(1.0), &normals, &large_uvs, &indices, &limits).unwrap();
        assert!(t.iter().all(|t| (Vec4::from_array(*t) - Vec4::new(1.0, 0.0, 0.0, 1.0)).length() < 1e-4), "{t:?}");
        // The largest positions and UVs that are not divided first.
        let edge_uvs = uvs.map(|uv| [uv[0] * 1e6, uv[1] * 1e6]);
        let t = crate::tangents(&quad_at(1e6), &normals, &edge_uvs, &indices, &limits).unwrap();
        assert!(t.iter().all(|t| (Vec4::from_array(*t) - Vec4::new(1.0, 0.0, 0.0, 1.0)).length() < 1e-4), "{t:?}");
    }

    #[test]
    fn tangents_stay_finite_beside_a_non_finite_normal() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut m = quad();
            m.normals[1] = Vec3::new(bad, 0.0, 1.0);
            m.compute_tangents();
            assert_eq!(m.tangents.len(), 4, "{bad}");
            for (v, t) in m.tangents.iter().enumerate() {
                assert!(t.is_finite() && t.w.abs() == 1.0, "{bad}, vertex {v}: {t:?}");
                assert!((t.truncate().length() - 1.0).abs() < 1e-5, "{bad}, vertex {v}: {t:?}");
            }
        }
    }

    #[test]
    fn normals_of_many_large_triangles_at_one_vertex_do_not_overflow() {
        // Twenty copies of one triangle: the summed face normal at each corner is 2e19 long, and
        // its square is past f32::MAX.
        let positions = [[0.0, 0.0, 0.0], [1e9, 0.0, 0.0], [0.0, 1e9, 0.0]];
        let indices = [0u32, 1, 2].repeat(20);
        let n = crate::smooth_normals(&positions, &indices, &Limits::default()).unwrap();
        assert!(n.iter().all(|n| (Vec3::from_array(*n) - Vec3::Z).length() < 1e-6), "{n:?}");
        // The same sum normalizes without the division, as it does for a vector whose square
        // rounds to zero.
        assert_eq!(unit(Vec3::new(0.0, 0.0, 2e19)), Some(Vec3::Z));
        assert_eq!(unit(Vec3::new(-1e-25, 0.0, 0.0)), Some(Vec3::NEG_X));
        assert_eq!((unit(Vec3::ZERO), unit(Vec3::new(f32::NAN, 0.0, 0.0)), unit(Vec3::new(f32::INFINITY, 0.0, 0.0))), (None, None, None));
    }

    #[test]
    fn bounds_reject_non_finite_positions() {
        let mut m = quad();
        m.positions[1] = Vec3::new(f32::NAN, 5.0, 0.0);
        m.positions[2] = Vec3::new(9.0, f32::INFINITY, 0.0);
        assert_eq!(m.bounds(), (Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0)), "only the finite corners count");
        assert!(crate::bounds(&[[0.0, 0.0, 0.0], [f32::NAN, 1.0, 1.0]]).is_err());
        assert!(crate::bounds(&[[0.0, 0.0, 0.0], [1.0, f32::INFINITY, 1.0]]).is_err());
        assert_eq!(crate::bounds(&[[1.0, -2.0, 3.0], [-1.0, 2.0, 0.0]]).unwrap(), Some(([-1.0, -2.0, 0.0], [1.0, 2.0, 3.0])));
        assert_eq!(crate::bounds(&[]).unwrap(), None);
    }

    #[test]
    fn validate_rejects_unaligned_and_overflowing_submeshes() {
        for (start, count) in [(1, 3), (0, 4), (u32::MAX - 2, 6), (u32::MAX, u32::MAX), (3, 6)] {
            let mut m = quad();
            m.submeshes[0].start = start;
            m.submeshes[0].count = count;
            assert!(m.validate().is_err(), "start {start}, count {count}");
        }
        let mut m = quad();
        m.normals[0] = Vec3::NAN;
        assert!(m.validate().is_err(), "non-finite normal");
        let mut m = quad();
        m.uvs[0] = Vec2::new(0.0, f32::INFINITY);
        assert!(m.validate().is_err(), "non-finite UV");
    }

    #[test]
    fn append_keeps_attributes_aligned() {
        // A mesh without tangents gets them; one without normals or UVs gets computed normals
        // and zero UVs.
        let mut bare = quad();
        bare.tangents.clear();
        let mut a = quad();
        a.append(&bare);
        a.validate().unwrap();
        assert_eq!((a.positions.len(), a.tangents.len()), (8, 8));
        let mut bare = quad();
        bare.normals.clear();
        bare.uvs.clear();
        bare.tangents.clear();
        let mut b = quad();
        b.append(&bare);
        b.validate().unwrap();
        assert_eq!(b.uvs[4..], [Vec2::ZERO; 4]);
        assert!((b.normals[5] - Vec3::Z).length() < 1e-5);
        // Neither side has tangents: the result has none either.
        let mut c = quad();
        c.tangents.clear();
        c.append(&bare);
        assert_eq!((c.positions.len(), c.normals.len(), c.uvs.len(), c.tangents.len()), (8, 8, 8, 0));
        // An empty mesh has no attributes to pass on: appending it to a mesh without tangents
        // leaves the mesh as it was, and the other way round the result has no tangents either.
        let mut d = quad();
        d.tangents.clear();
        let before = d.clone();
        d.append(&Mesh::default());
        assert_eq!(d, before);
        let mut e = Mesh::default();
        e.append(&before);
        assert_eq!((e.positions.len(), e.normals.len(), e.uvs.len(), e.tangents.len()), (4, 4, 4, 0));
        // An index past the other mesh's vertices would point into this one after rebasing.
        let mut broken = quad();
        broken.indices[0] = 4;
        assert!(quad().try_append(&broken).is_err());
    }

    #[test]
    fn append_fills_in_this_mesh_when_its_tangents_split_vertices() {
        // UVs mirrored across the diagonal: computing this mesh's tangents splits the shared
        // vertices, and the copies keep their colors.
        let mut a = quad();
        a.normals.clear();
        a.tangents.clear();
        a.uvs[3] = Vec2::new(1.0, 1.0);
        a.colors = vec![Vec4::new(1.0, 0.0, 0.0, 1.0); 4];
        a.diagonals = vec![0b100, 0];
        let other = quad();
        a.try_append(&other).unwrap();
        a.validate().unwrap();
        let n = a.positions.len();
        assert!(n > 8, "{n} vertices");
        assert_eq!((a.normals.len(), a.uvs.len(), a.tangents.len(), a.colors.len()), (n, n, n, n));
        assert!(a.colors[..n - 4].iter().all(|&c| c == Vec4::new(1.0, 0.0, 0.0, 1.0)), "{:?}", a.colors);
        assert_eq!(a.colors[n - 4..], [Vec4::ONE; 4]);
        assert_eq!(a.diagonals, vec![0b100, 0, 0, 0]);
        let base = (n - 4) as u32;
        assert_eq!(a.indices[6..], other.indices.iter().map(|i| i + base).collect::<Vec<_>>()[..]);
    }

    #[test]
    fn append_keeps_unnamed_materials_apart() {
        // A submesh may use material 0 of an empty material list.
        let mut unnamed = quad();
        unnamed.materials.clear();
        unnamed.validate().unwrap();
        let mut a = unnamed.clone();
        a.append(&quad());
        a.validate().unwrap();
        assert_eq!(a.materials, vec![String::new(), "Default".to_string()]);
        assert_eq!((a.submeshes[0].material, a.submeshes[1].material), (0, 1));
        let mut b = quad();
        b.append(&unnamed);
        b.validate().unwrap();
        assert_eq!(b.materials, vec!["Default".to_string(), String::new()]);
        assert_eq!((b.submeshes[0].material, b.submeshes[1].material), (0, 1));
    }

    /// A quad whose two triangles are submeshes named `c` and `d`, with materials `a` and `b`.
    fn named_quad(a: String, b: String, c: String, d: String) -> Mesh {
        let mut m = quad();
        m.materials = vec![a, b];
        m.submeshes = vec![SubMesh { name: c, material: 1, start: 0, count: 3 }, SubMesh { name: d, material: 0, start: 3, count: 3 }];
        m
    }

    #[test]
    fn append_copies_long_names_in_order() {
        let long = |c: char| c.to_string().repeat(4096);
        let other = named_quad(long('a'), long('b'), long('c'), long('d'));
        let mut m = quad();
        m.materials.clear();
        m.try_append(&other).unwrap();
        m.validate().unwrap();
        assert_eq!(m.materials, vec![String::new(), long('a'), long('b')]);
        let submeshes = m.submeshes.iter().map(|s| (s.name.clone(), s.material, s.start, s.count)).collect::<Vec<_>>();
        assert_eq!(submeshes[1..], [(long('c'), 2, 6, 3), (long('d'), 1, 9, 3)]);
    }

    #[test]
    fn append_leaves_the_mesh_unchanged_when_a_name_copy_fails() {
        let other = named_quad("a".into(), "b".into(), "c".into(), "d".into());
        let mut m = quad();
        let before = m.clone();
        // The two submesh names are copied first, then the two material names.
        for (copies, what) in [(0, "submeshes"), (1, "submeshes"), (2, "materials"), (3, "materials")] {
            fail_name_copy_after(Some(copies));
            assert_eq!(m.try_append(&other), Err(MeshOpsError::OutOfMemory(what)), "after {copies} copies");
            assert_eq!(m, before, "after {copies} copies");
        }
        fail_name_copy_after(None);
        m.try_append(&other).unwrap();
        assert_eq!(m.materials, ["Default", "a", "b"]);
    }

    #[test]
    fn subdivision_reports_a_failed_name_copy() {
        let m = named_quad("a".into(), "b".into(), "c".into(), "d".into());
        // Both submesh names are copied before the material names.
        for (copies, what) in [(0, "submeshes"), (1, "submeshes"), (2, "materials"), (3, "materials")] {
            fail_name_copy_after(Some(copies));
            assert_eq!(m.try_subdivided(&Limits::default()), Err(MeshOpsError::OutOfMemory(what)), "after {copies} copies");
        }
        fail_name_copy_after(None);
        let s = m.try_subdivided(&Limits::default()).unwrap();
        assert_eq!((s.materials, s.submeshes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>()), (m.materials, vec!["c", "d"]));
    }

    #[test]
    fn subdivision_preserves_large_finite_midpoints() {
        let mut m = quad();
        m.positions = vec![Vec3::new(3e38, 0.0, 0.0), Vec3::new(3e38, 3e38, 0.0), Vec3::new(-3e38, 3e38, 0.0), Vec3::new(-3e38, -3e38, 0.0)];
        m.uvs = vec![Vec2::new(3e38, 0.0), Vec2::new(3e38, 3e38), Vec2::ZERO, Vec2::ZERO];
        let s = m.try_subdivided(&Limits::default()).unwrap();
        assert!(s.positions.iter().all(|p| p.is_finite()), "{:?}", s.positions);
        assert!(s.uvs.iter().all(|p| p.is_finite()), "{:?}", s.uvs);
        assert!(s.positions.contains(&Vec3::new(3e38, 1.5e38, 0.0)));
    }

    #[test]
    fn subdivision_keeps_midpoint_normals_and_tangents_of_long_vectors() {
        let mut m = quad();
        // Edge 0-1: the sum's squared length overflows; edge 2-3: the sum itself overflows.
        m.normals = vec![Vec3::new(1e20, 0.0, 0.0), Vec3::new(1e20, 0.0, 0.0), Vec3::new(3e38, 0.0, 0.0), Vec3::new(3e38, 0.0, 0.0)];
        m.tangents = vec![Vec4::new(1e20, 0.0, 0.0, 1.0); 4];
        let s = m.try_subdivided(&Limits::default()).unwrap();
        assert_eq!(s.positions.len(), 9);
        for n in &s.normals[4..] {
            assert!((*n - Vec3::X).length() < 1e-6, "{:?}", s.normals);
        }
        for t in &s.tangents[4..] {
            assert!((*t - Vec4::new(1.0, 0.0, 0.0, 1.0)).length() < 1e-6, "{:?}", s.tangents);
        }
    }

    #[test]
    fn subdivision_keeps_the_input_vertices_and_materials() {
        let mut m = quad();
        m.colors = vec![Vec4::X, Vec4::Y, Vec4::Z, Vec4::W];
        let s = m.try_subdivided(&Limits::default()).unwrap();
        let n = m.positions.len();
        assert_eq!(s.positions[..n], m.positions[..]);
        assert_eq!(s.normals[..n], m.normals[..]);
        assert_eq!(s.tangents[..n], m.tangents[..]);
        assert_eq!(s.uvs[..n], m.uvs[..]);
        assert_eq!(s.colors[..n], m.colors[..]);
        assert_eq!(s.colors.len(), s.positions.len());
        assert_eq!(s.materials, m.materials);
        assert_eq!(s.submeshes.iter().map(|s| (&s.name, s.material)).collect::<Vec<_>>(), vec![(&m.submeshes[0].name, 0)]);
        // An input with no diagonals gives a result with none.
        assert!(m.diagonals.is_empty() && s.diagonals.is_empty());
    }

    #[test]
    fn subdivision_checks_submeshes_and_diagonals() {
        // Two materials, one triangle each: each keeps its own four children.
        let mut m = quad();
        m.materials = vec!["a".into(), "b".into()];
        m.submeshes =
            vec![SubMesh { name: "a".into(), material: 0, start: 0, count: 3 }, SubMesh { name: "b".into(), material: 1, start: 3, count: 3 }];
        m.diagonals = vec![0b100, 0b001];
        let s = m.try_subdivided(&Limits::default()).unwrap();
        s.validate().unwrap();
        assert_eq!(s.triangles_of(0).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        assert_eq!(s.triangles_of(1).collect::<Vec<_>>(), vec![4, 5, 6, 7]);
        assert_eq!(s.diagonals.len(), s.triangle_count());
        // The diagonal (0, 2) splits at vertex 6; both halves stay hidden. A subdivided quad has
        // 16 edges.
        let whole =
            Mesh { materials: vec!["a".into()], submeshes: vec![SubMesh { name: "a".into(), material: 0, start: 0, count: 24 }], ..s.clone() };
        let edges = &whole.wire_edges(&Limits::default()).unwrap()[0];
        assert!(!edges.contains(&(0, 6)) && !edges.contains(&(2, 6)), "{edges:?}");
        assert_eq!(edges.len(), 16 - 2, "{edges:?}");

        // A submesh that splits a triangle, ends past the indices or overflows is an error.
        for (start, count) in [(1, 3), (0, 9), (u32::MAX - 2, 6)] {
            let mut bad = quad();
            bad.submeshes[0].start = start;
            bad.submeshes[0].count = count;
            assert!(bad.try_subdivided(&Limits::default()).is_err(), "start {start}, count {count}");
            assert_eq!(bad.subdivided(), bad, "the infallible form leaves the mesh unchanged");
        }
        let mut bad = quad();
        bad.indices[1] = 9;
        assert!(bad.try_subdivided(&Limits::default()).is_err());
        // A submesh naming a material the mesh lacks fails, as it fails validate.
        let mut bad = quad();
        bad.submeshes[0].material = bad.materials.len().max(1);
        assert!(bad.validate().is_err());
        assert!(matches!(bad.try_subdivided(&Limits::default()), Err(MeshOpsError::Invalid(_))));
        assert_eq!(bad.subdivided(), bad);
        let small = Limits { max_triangles: 7, ..Limits::default() };
        assert!(quad().try_subdivided(&small).is_err());
        // The quad's two triangles share one of their six midpoints: 9 vertices, not 10.
        let exact = Limits { max_vertices: 9, ..Limits::default() };
        assert_eq!(quad().try_subdivided(&exact).unwrap().positions.len(), 9);
        let under = Limits { max_vertices: 8, ..Limits::default() };
        assert!(matches!(quad().try_subdivided(&under), Err(MeshOpsError::TooLarge { .. })));
    }

    #[test]
    fn tri_rejects_overflowing_triangle_index() {
        let m = quad();
        assert_eq!(m.tri(usize::MAX), None);
        assert_eq!(m.tri(usize::MAX / 3 + 1), None);
        assert_eq!(m.material_of(usize::MAX), None);
    }

    #[test]
    fn normalize_large_finite_meshes() {
        let mut m = quad();
        m.positions = vec![Vec3::new(-3e38, -3e38, 0.0), Vec3::new(3e38, -3e38, 0.0), Vec3::new(3e38, 3e38, 0.0), Vec3::new(-3e38, 3e38, 0.0)];
        assert!(m.radius().is_finite());
        m.normalize_to(2.0);
        assert!(m.positions.iter().all(|p| p.is_finite()), "{:?}", m.positions);
        assert!((m.radius() - 2.0).abs() < 1e-4, "{}", m.radius());
        assert!((m.positions[2] - Vec3::new(2.0, 2.0, 0.0).normalize() * 2.0).length() < 1e-4, "{:?}", m.positions);
        let before = m.clone();
        m.normalize_to(f32::NAN);
        m.normalize_to(-1.0);
        assert_eq!(m, before);
        // One vertex far from the origin moves to the center.
        let mut point = Mesh { positions: vec![Vec3::splat(3e38)], ..Mesh::default() };
        point.normalize_to(2.0);
        assert_eq!(point.positions, vec![Vec3::ZERO]);
    }

    #[test]
    fn normalize_handles_the_largest_radii_and_the_smallest_meshes() {
        // The unit quad has a radius of about 0.707, so f32::MAX divided by it passes f32::MAX;
        // its corners still fit.
        let mut unit = quad();
        unit.normalize_to(f32::MAX);
        let corner = unit.positions[2] / f32::MAX;
        assert!((corner - Vec3::new(1.0, 1.0, 0.0).normalize()).length() < 1e-4, "{:?}", unit.positions);
        unit.normalize_to(1e30);
        assert!(unit.positions.iter().all(|p| p.is_finite()), "{:?}", unit.positions);
        assert!((unit.radius() / 1e30 - 1.0).abs() < 1e-4, "{}", unit.radius());
        // A mesh smaller than the floor of `radius` reaches the requested radius too.
        for size in [1e-5f32, 1e-30] {
            let mut small = crate::primitives::plane(1.0, 2);
            for p in &mut small.positions {
                *p *= size;
            }
            small.normalize_to(1.0);
            assert!((small.radius() - 1.0).abs() < 1e-4, "{size}: {}", small.radius());
        }
        // A plane spanning ±0.001 is above the floor, and 1e38 divided by its radius passes
        // f32::MAX; its corners still fit.
        let mut tiny = crate::primitives::plane(0.002, 2);
        tiny.normalize_to(1e38);
        assert!(tiny.positions.iter().all(|p| p.is_finite()), "{:?}", tiny.positions);
        assert!((tiny.radius() / 1e38 - 1.0).abs() < 1e-4, "{}", tiny.radius());
        // A mesh already near f32::MAX reaches the largest radius: its corners fit.
        let mut wide = quad();
        wide.positions = vec![Vec3::new(-3e38, -3e38, 0.0), Vec3::new(3e38, -3e38, 0.0), Vec3::new(3e38, 3e38, 0.0), Vec3::new(-3e38, 3e38, 0.0)];
        wide.normalize_to(f32::MAX);
        let corner = wide.positions[2] / f32::MAX;
        assert!((corner - Vec3::new(1.0, 1.0, 0.0).normalize()).length() < 1e-4, "{:?}", wide.positions);
    }

    #[test]
    fn tangents_skip_mikktspace_when_dividing_makes_every_triangle_degenerate() {
        // Two different x coordinates near f32::MAX with the same quotient by the largest one.
        let largest = 3.1e38f32;
        let mut a = 1.6e38f32;
        let mut b = f32::from_bits(a.to_bits() + 1);
        for _ in 0..1000 {
            if a / largest == b / largest {
                break;
            }
            a = b;
            b = f32::from_bits(a.to_bits() + 1);
        }
        assert!(a != b && a / largest == b / largest, "{a} {b}");
        let positions = [[a, 0.0, 0.0], [b, 0.0, 0.0], [0.0, largest, 0.0]];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]];
        let t = crate::tangents(&positions, &[[0.0, 0.0, 1.0]; 3], &uvs, &[0, 1, 2], &Limits::default()).unwrap();
        assert_eq!(t.len(), 3);
        assert!(t.iter().flatten().all(|v| v.is_finite()), "{t:?}");
    }

    #[test]
    fn tangents_stay_perpendicular_to_very_long_normals() {
        let direction = Vec3::new(1.0, 0.0, 3.0).normalize();
        let expected = (Vec3::X - direction * direction.x).normalize().extend(1.0);
        // Normals near f32::MAX on a unit quad, and normals about 1e9 long on the largest quad
        // and UVs that are not divided: MikkTSpace multiplies the normal twice into their product.
        for (length, size) in [(1e38f32, 1.0f32), (3e8, 1e6)] {
            let positions = [[0.0, 0.0, 0.0], [size, 0.0, 0.0], [size, size, 0.0], [0.0, size, 0.0]];
            let uvs = [[0.0, size], [size, size], [size, 0.0], [0.0, 0.0]];
            let normal = [length, 0.0, 3.0 * length];
            let t = crate::tangents(&positions, &[normal; 4], &uvs, &[0, 1, 2, 0, 2, 3], &Limits::default()).unwrap();
            for t in &t {
                assert!((Vec4::from_array(*t) - expected).length() < 1e-3, "{length} {size}: {t:?}");
            }
        }
    }

    #[test]
    fn a_small_triangle_far_from_the_origin_keeps_its_normal_and_tangent() {
        // Unit edges along y and z at x = 1e24: divided by 1e24, their cross product underflows.
        let positions = [[1e24, 0.0, 0.0], [1e24, 1.0, 0.0], [1e24, 0.0, 1.0]];
        let l = Limits::default();
        for n in crate::smooth_normals(&positions, &[0, 1, 2], &l).unwrap() {
            assert_eq!(n, [1.0, 0.0, 0.0]);
        }
        // u grows along +y and v along +z: the bitangent points up the image, along -z, and
        // cross(+x, +y) = +z, so w is -1.
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        for t in crate::tangents(&positions, &[[1.0, 0.0, 0.0]; 3], &uvs, &[0, 1, 2], &l).unwrap() {
            assert!((Vec4::from_array(t) - Vec4::new(0.0, 1.0, 0.0, -1.0)).length() < 1e-5, "{t:?}");
        }
    }

    #[test]
    fn fallback_tangents_keep_edges_that_vanish_when_divided() {
        // Edges of 1e-10 at x = 1e38: divided by 1e38 in f32 they underflow to zero, so the
        // triangle counts as degenerate and the fallback computes its tangent.
        let positions = [[1e38, 0.0, 0.0], [1e38, 1e-10, 0.0], [1e38, 0.0, 1e-10]];
        // u grows along +z and v along +y: the tangent is +z, the bitangent points along -y, and
        // cross(+x, +z) = -y, so w is +1.
        let uvs = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0]];
        for t in crate::tangents(&positions, &[[1.0, 0.0, 0.0]; 3], &uvs, &[0, 1, 2], &Limits::default()).unwrap() {
            assert!((Vec4::from_array(t) - Vec4::new(0.0, 0.0, 1.0, 1.0)).length() < 1e-5, "{t:?}");
        }
    }

    /// Positions, normals, UVs and indices.
    type Slices = ([[f32; 3]; 4], [[f32; 3]; 4], [[f32; 2]; 4], [u32; 6]);

    /// Two triangles on vertices 0 and 1 whose UVs run in opposite directions: v grows toward +Y
    /// on the first and toward -Y on the second. Bitangents point up the image (glTF 2.0), so the
    /// first triangle's tangent frame has w = -1 and the second's w = +1.
    fn opposite_handedness() -> Slices {
        let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0]];
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.0, 1.0]];
        (positions, [[0.0, 0.0, 1.0]; 4], uvs, [0, 1, 2, 1, 0, 3])
    }

    #[test]
    fn tangents_split_vertices_shared_by_opposite_frames() {
        let (positions, normals, uvs, indices) = opposite_handedness();
        let mut m = Mesh::from_slices(&positions, Some(&normals), Some(&uvs), &indices, &Limits::default()).unwrap();
        m.compute_tangents();
        m.validate().unwrap();
        // Vertices 0 and 1 get a copy each for the second triangle.
        assert_eq!(m.positions.len(), 6);
        for (t, w) in [(0, -1.0), (1, 1.0)] {
            for (k, v) in m.tri(t).unwrap().into_iter().enumerate() {
                let tangent = m.tangents[v];
                assert!((tangent - Vec3::X.extend(w)).length() < 1e-5, "triangle {t}, vertex {v}: {tangent}");
                let original = indices[t * 3 + k] as usize;
                assert_eq!((m.positions[v], m.uvs[v]), (Vec3::from_array(positions[original]), Vec2::from_array(uvs[original])));
            }
        }
        // The slice form returns one tangent per input vertex, which cannot hold both frames.
        assert!(matches!(crate::tangents(&positions, &normals, &uvs, &indices, &Limits::default()), Err(MeshOpsError::Invalid(_))));
    }

    #[test]
    fn tangent_splits_stop_at_the_first_copy_they_may_not_make() {
        let (positions, normals, uvs, indices) = opposite_handedness();
        // The second triangle's first corner, vertex 1, is the first to need a copy.
        let e = crate::tangents(&positions, &normals, &uvs, &indices, &Limits::default()).unwrap_err();
        assert_eq!(e.to_string(), "invalid mesh: vertex 1 is shared by triangles with different tangent frames");
        let mesh = Mesh::from_slices(&positions, Some(&normals), Some(&uvs), &indices, &Limits::default()).unwrap();
        // Room for one of the two copies: the second is `TooLarge` and the mesh stays as it was.
        let mut m = mesh.clone();
        let r = m.try_compute_tangents_splitting(Split::UpTo(5));
        assert!(matches!(r, Err(MeshOpsError::TooLarge { what: "vertex count", got: 6, limit: 5 })), "{r:?}");
        assert_eq!(m, mesh);
        let mut m = mesh.clone();
        m.try_compute_tangents_splitting(Split::UpTo(6)).unwrap();
        assert_eq!(m.positions.len(), 6);
    }

    #[test]
    fn tangents_of_a_fan_around_one_vertex_do_not_exhaust_the_stack() {
        // Every triangle shares vertex 0 with the same normal and UV; MikkTSpace would visit
        // the fan recursively, one call per triangle.
        let n = 200_000u32;
        let mut positions = vec![[0.0f32, 0.0, 0.0]];
        for i in 0..=n {
            let a = i as f32 / n as f32 * 6.0;
            positions.push([a.cos(), a.sin(), 0.0]);
        }
        let indices: Vec<u32> = (0..n).flat_map(|i| [0, i + 1, i + 2]).collect();
        let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
        // With v growing down the image along -y the bitangent points along +y and w is +1; with
        // v mirrored to grow along +y, w is -1.
        for (v_along_y, w) in [(-0.5f32, 1.0f32), (0.5, -1.0)] {
            let uvs: Vec<[f32; 2]> = positions.iter().map(|p| [p[0] * 0.5 + 0.5, 0.5 + p[1] * v_along_y]).collect();
            let t = crate::tangents(&positions, &normals, &uvs, &indices, &Limits::default()).unwrap();
            assert_eq!(t.len(), positions.len());
            for t in &t {
                assert!((Vec4::from_array(*t) - Vec4::new(1.0, 0.0, 0.0, w)).length() < 1e-2, "w {w}: {t:?}");
            }
        }
    }

    #[test]
    fn fallback_tangents_have_the_handedness_mikktspace_gives() {
        // Small fans go to MikkTSpace; past the valence limit the same frames come from the
        // fallback, mirrored UVs included.
        let fan = |n: u32, v_along_y: f32| {
            let mut positions = vec![[0.0f32, 0.0, 0.0]];
            for i in 0..=n {
                let a = i as f32 / n as f32 * 6.0;
                positions.push([a.cos(), a.sin(), 0.0]);
            }
            let uvs: Vec<[f32; 2]> = positions.iter().map(|p| [0.5 - p[1] * 0.5, 0.5 + p[0] * v_along_y]).collect();
            let indices: Vec<u32> = (0..n).flat_map(|i| [0, i + 1, i + 2]).collect();
            let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
            crate::tangents(&positions, &normals, &uvs, &indices, &Limits::default()).unwrap()
        };
        for v_along_y in [0.5, -0.5] {
            let (mikk, fallback) = (fan(16, v_along_y), fan(MAX_MIKKTSPACE_VALENCE as u32 + 1, v_along_y));
            let w = mikk[0][3];
            assert!(mikk.iter().chain(&fallback).all(|t| t[3] == w), "{v_along_y}: {:?} {:?}", mikk[0], fallback[0]);
            assert!((Vec4::from_array(fallback[0]) - Vec4::from_array(mikk[0])).length() < 1e-2, "{:?} {:?}", fallback[0], mikk[0]);
        }
    }

    #[test]
    fn tangents_keep_uv_differences_far_below_one() {
        // V differs by 1e-8 between two corners: 1 - v rounds both to 1, and u grows along +y.
        let m = Mesh::from_parts(
            vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            Some(vec![Vec3::Z; 3]),
            vec![Vec2::new(0.0, 0.0), Vec2::new(0.0, 1e-8), Vec2::new(1.0, 0.0)],
            vec![0, 1, 2],
            "m",
        )
        .unwrap();
        for t in &m.tangents {
            assert!((t.truncate() - Vec3::Y).length() < 1e-5, "{t}");
        }
    }

    #[test]
    fn coincident_copies_with_different_uvs_skip_mikktspace() {
        // MikkTSpace compares the corners at one position with each other: this many copies of
        // one triangle at the same place would cost it about 3 × 7,000² / 2 comparisons.
        let copies = 7_000u32;
        let positions = [Vec3::ZERO, Vec3::X, Vec3::Y].repeat(copies as usize);
        let uvs: Vec<Vec2> = (0..copies).flat_map(|i| [Vec2::ZERO, Vec2::X, Vec2::Y].map(|uv| uv + Vec2::splat(i as f32))).collect();
        let indices: Vec<u32> = (0..copies * 3).collect();
        let normals = vec![Vec3::Z; positions.len()];
        let scales = Scales::of(&positions, &normals, &uvs);
        let mut corners = vec![Vec4::ZERO; indices.len()];
        let g = Mikk { positions: &positions, normals: &normals, uvs: &uvs, indices: &indices, scales, corners: &mut corners };
        assert!(!g.affordable().unwrap());
        // The corners at the pole of a sphere at the segment limit share one position, and their
        // UVs differ: MikkTSpace still gets them.
        let pole = crate::primitives::try_sphere(1.0, MAX_MIKKTSPACE_VALENCE, 2, &Limits::default()).unwrap();
        let scales = Scales::of(&pole.positions, &pole.normals, &pole.uvs);
        let mut corners = vec![Vec4::ZERO; pole.indices.len()];
        let g = Mikk { positions: &pole.positions, normals: &pole.normals, uvs: &pole.uvs, indices: &pole.indices, scales, corners: &mut corners };
        assert!(g.affordable().unwrap());
        // The fallback gives the copies their own frames: +X, with v growing down the image.
        let mut m = Mesh { positions, normals, uvs, indices, ..Mesh::default() };
        m.try_compute_tangents().unwrap();
        assert!(m.tangents.iter().all(|t| (*t - Vec4::new(1.0, 0.0, 0.0, -1.0)).length() < 1e-5));
    }
}
