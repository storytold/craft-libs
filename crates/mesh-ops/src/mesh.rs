//! The internal triangle mesh.

use glam::{Vec2, Vec3, Vec4};

use crate::{GeomError, Result};

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

/// A material's base color (what a color-map bake transfers): `factor` (linear RGBA) times an
/// optional texture sampled at the mesh UVs.
#[derive(Clone, Debug, PartialEq)]
pub struct BaseColor {
    pub factor: Vec4,
    pub width: u32,
    pub height: u32,
    /// `width × height` RGBA8 texels (color sRGB-encoded, alpha linear), rows top → bottom like
    /// the UVs' v; empty without a texture.
    pub texels: Vec<[u8; 4]>,
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

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Axis-aligned bounds (zero box for an empty mesh).
    pub fn bounds(&self) -> (Vec3, Vec3) {
        if self.positions.is_empty() {
            return (Vec3::ZERO, Vec3::ZERO);
        }
        self.positions.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
    }

    /// Bounding-sphere radius around the bounds center (at least a small epsilon).
    pub fn radius(&self) -> f32 {
        let (lo, hi) = self.bounds();
        ((hi - lo).length() * 0.5).max(1e-4)
    }

    /// Vertex indices of triangle `t`.
    pub fn tri(&self, t: usize) -> Option<[usize; 3]> {
        let start = t.checked_mul(3)?;
        let i = self.indices.get(start..start.checked_add(3)?)?;
        Some([i[0] as usize, i[1] as usize, i[2] as usize])
    }

    /// The edges wireframes draw: each vertex pair once (smaller index first), leaving out
    /// polygon diagonals ([`Mesh::diagonals`]) unless only one triangle has the edge (a UV seam
    /// split it from its twin), per material in `0..materials.len().max(1)`.
    pub fn wire_edges(&self) -> Vec<Vec<(u32, u32)>> {
        (0..self.materials.len().max(1))
            .map(|m| {
                // Edge → (real in some triangle, triangles using it), in first-seen order.
                let mut edges: Vec<((u32, u32), bool, u8)> = Vec::new();
                let mut at: std::collections::HashMap<(u32, u32), usize> = std::collections::HashMap::new();
                for t in self.triangles_of(m) {
                    let Some(v) = self.indices.get(t * 3..t * 3 + 3) else { continue };
                    let hidden = self.diagonals.get(t).copied().unwrap_or(0);
                    for (i, (a, b)) in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])].into_iter().enumerate() {
                        let key = (a.min(b), a.max(b));
                        let real = hidden >> i & 1 == 0;
                        let e = *at.entry(key).or_insert_with(|| {
                            edges.push((key, false, 0));
                            edges.len() - 1
                        });
                        if let Some((_, r, n)) = edges.get_mut(e) {
                            *r |= real;
                            *n = n.saturating_add(1);
                        }
                    }
                }
                edges.into_iter().filter(|(_, real, n)| *real || *n == 1).map(|(k, ..)| k).collect()
            })
            .collect()
    }

    /// Edges where the UV layout splits: two triangles meet there in 3D but not in UV space, per material in `0..materials.len().max(1)`
    pub fn uv_seam_edges(&self) -> Vec<Vec<(u32, u32)>> {
        // Signed zero denotes the same position on either side of an imported seam
        let bits = |position: Vec3| position.to_array().map(|coordinate| if coordinate == 0.0 { 0 } else { coordinate.to_bits() });
        let mut seams = vec![Vec::new(); self.materials.len().max(1)];
        // Match across materials before grouping the lines for texture-set visibility
        let mut sides: std::collections::HashMap<EdgePositions, Vec<(u32, u32, usize)>> = std::collections::HashMap::new();
        for material in 0..seams.len() {
            for triangle in self.triangles_of(material) {
                let Some(corners) = self.indices.get(triangle * 3..triangle * 3 + 3) else { continue };

                for (from, to) in [(corners[0], corners[1]), (corners[1], corners[2]), (corners[2], corners[0])] {
                    let (Some(from_position), Some(to_position)) = (self.positions.get(from as usize), self.positions.get(to as usize)) else {
                        continue;
                    };

                    let (from_bits, to_bits) = (bits(*from_position), bits(*to_position));
                    let (edge, pair) = if from_bits <= to_bits { ((from_bits, to_bits), (from, to)) } else { ((to_bits, from_bits), (to, from)) };
                    sides.entry(edge).or_default().push((pair.0, pair.1, material));
                }
            }
        }

        let uv = |index: u32| self.uvs.get(index as usize).copied();
        for pairs in sides.into_values() {
            let Some(&(first_from, first_to, _)) = pairs.first() else { continue };

            if pairs.iter().any(|&(from, to, _)| (uv(from), uv(to)) != (uv(first_from), uv(first_to))) {
                for (from, to, material) in pairs {
                    if let Some(edges) = seams.get_mut(material) {
                        edges.push((from.min(to), from.max(to)));
                    }
                }
            }
        }
        for edges in &mut seams {
            edges.sort_unstable();
            edges.dedup();
        }

        seams
    }

    /// Material of triangle `t`.
    pub fn material_of(&self, t: usize) -> Option<usize> {
        if t >= self.triangle_count() {
            return None;
        }
        let idx = u32::try_from(t * 3).ok()?;
        self.submeshes.iter().find(|s| idx >= s.start && idx < s.start.saturating_add(s.count)).map(|s| s.material)
    }

    /// Triangle index range belonging to material `m` (all submeshes of that material).
    pub fn triangles_of(&self, m: usize) -> impl Iterator<Item = usize> + '_ {
        self.submeshes.iter().filter(move |s| s.material == m).flat_map(|s| (s.start / 3) as usize..((s.start + s.count) / 3) as usize)
    }

    /// Check that attribute arrays agree, every index is in range, and every submesh is a
    /// triangle-aligned range that shares no triangle with a submesh of another material.
    pub fn validate(&self) -> Result<()> {
        let n = self.positions.len();
        if self.normals.len() != n || self.uvs.len() != n || self.tangents.len() != n {
            return Err(GeomError::Invalid(format!(
                "attribute counts differ: {} positions, {} normals, {} uvs, {} tangents",
                n,
                self.normals.len(),
                self.uvs.len(),
                self.tangents.len()
            )));
        }
        if !self.colors.is_empty() && self.colors.len() != n {
            return Err(GeomError::Invalid(format!("{} vertex colors for {n} vertices", self.colors.len())));
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err(GeomError::Invalid("index count is not a multiple of 3".into()));
        }
        if self.triangle_count() > MAX_TRIANGLES {
            return Err(GeomError::Invalid(format!("mesh has more than {MAX_TRIANGLES} triangles")));
        }
        if let Some(bad) = self.indices.iter().find(|i| **i as usize >= n) {
            return Err(GeomError::Invalid(format!("index {bad} out of range ({n} vertices)")));
        }
        if self.positions.iter().any(|p| !p.is_finite()) {
            return Err(GeomError::Invalid("mesh has non-finite positions".into()));
        }
        for s in &self.submeshes {
            if s.start as usize + s.count as usize > self.indices.len()
                || !s.start.is_multiple_of(3)
                || !s.count.is_multiple_of(3)
                || s.material >= self.materials.len().max(1)
            {
                return Err(GeomError::Invalid(format!("submesh {} is out of range", s.name)));
            }
        }
        // Each triangle has one material. Submeshes of one material may list the same triangle.
        let mut ranges: Vec<&SubMesh> = self.submeshes.iter().filter(|s| s.count > 0).collect();
        ranges.sort_unstable_by_key(|s| s.start);
        // The two largest range ends of distinct materials among the ranges seen so far.
        let (mut first, mut second) = ((0u32, usize::MAX), (0u32, usize::MAX));
        for s in ranges {
            let other = if first.1 != s.material { first.0 } else { second.0 };
            if other > s.start {
                return Err(GeomError::Invalid(format!("submesh {} shares triangles with a submesh of another material", s.name)));
            }
            let end = s.start + s.count;
            if s.material == first.1 {
                first.0 = first.0.max(end);
            } else if end > first.0 {
                second = first;
                first = (end, s.material);
            } else if end > second.0 {
                second = (end, s.material);
            }
        }
        Ok(())
    }

    /// Area-weighted smooth normals over the index buffer.
    pub fn compute_normals(&mut self) {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for &[a, b, c] in self.indices.as_chunks::<3>().0 {
            let (a, b, c) = (a as usize, b as usize, c as usize);
            let (Some(pa), Some(pb), Some(pc)) = (self.positions.get(a), self.positions.get(b), self.positions.get(c)) else { continue };
            let face = (*pb - *pa).cross(*pc - *pa);
            for v in [a, b, c] {
                if let Some(x) = normals.get_mut(v) {
                    *x += face;
                }
            }
        }

        self.normals = normals.into_iter().map(|v| v.try_normalize().unwrap_or(Vec3::Y)).collect();
    }

    /// MikkTSpace tangents (falls back to per-triangle tangents if generation fails or a position,
    /// normal or UV is not finite). A vertex whose corners get tangent frames of opposite
    /// handedness (a mirrored UV join) is split, one copy per handedness.
    pub fn compute_tangents(&mut self) {
        self.tangents = vec![Vec4::new(1.0, 0.0, 0.0, 1.0); self.positions.len()];
        if self.normals.len() != self.positions.len() {
            self.compute_normals();
        }
        if self.uvs.len() != self.positions.len() {
            self.uvs = vec![Vec2::ZERO; self.positions.len()];
        }
        let all: Vec<usize> = (0..self.triangle_count()).collect();
        self.generate_tangents_for(&all);
    }

    /// Rewrite the tangents of the vertices `triangles` use from per-corner tangents, splitting a
    /// vertex whose corners disagree in handedness. Other vertices keep their tangents.
    fn generate_tangents_for(&mut self, triangles: &[usize]) {
        let corners = self.corner_tangents();
        // Per vertex: (vertex holding this handedness, w, summed tangent), in first-seen order
        let mut variants: std::collections::HashMap<u32, Vec<(u32, f32, Vec3)>> = std::collections::HashMap::new();
        for &t in triangles {
            let Some(end) = t.checked_mul(3).and_then(|s| s.checked_add(3)) else { continue };
            for k in end - 3..end {
                let (Some(&v), Some(&tangent)) = (self.indices.get(k), corners.get(k)) else { continue };
                let w = if tangent.w < 0.0 { -1.0 } else { 1.0 };
                let list = variants.entry(v).or_default();
                let target = if let Some(entry) = list.iter_mut().find(|(_, sign, _)| *sign == w) {
                    entry.2 += tangent.truncate();
                    entry.0
                } else {
                    let i = if list.is_empty() { v } else { self.duplicate_vertex(v as usize) };
                    list.push((i, w, tangent.truncate()));
                    i
                };
                if let Some(i) = self.indices.get_mut(k) {
                    *i = target;
                }
            }
        }

        for (vertex, sign, sum) in variants.into_values().flatten() {
            let n = self.normals.get(vertex as usize).copied().unwrap_or(Vec3::Y);
            // Orthonormalize against the normal; repair degenerate results.
            let o = (sum - n * n.dot(sum)).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
            if let Some(t) = self.tangents.get_mut(vertex as usize) {
                *t = o.extend(sign);
            }
        }
    }

    /// One tangent per index (face corner): MikkTSpace's when it can run, else the per-vertex
    /// tangents of [`Mesh::simple_tangents`].
    fn corner_tangents(&self) -> Vec<Vec4> {
        // mikktspace crashes (out-of-bounds) when every triangle is degenerate, and non-finite
        // input breaks its comparisons; skip it then.
        let finite =
            self.positions.iter().all(|p| p.is_finite()) && self.normals.iter().all(|n| n.is_finite()) && self.uvs.iter().all(|uv| uv.is_finite());
        if self.validate_indices().is_ok() && finite && !crate::unwrap::all_triangles_degenerate(self) {
            let mut g = Mikk { mesh: self, corners: vec![Vec4::new(1.0, 0.0, 0.0, 1.0); self.indices.len()] };
            if mikktspace::generate_tangents(&mut g) {
                return g.corners;
            }
        }

        let simple = self.simple_tangents();
        self.indices.iter().map(|&v| simple.get(v as usize).copied().unwrap_or(Vec4::new(1.0, 0.0, 0.0, 1.0))).collect()
    }

    fn simple_tangents(&self) -> Vec<Vec4> {
        let mut tangents = vec![Vec3::ZERO; self.positions.len()];
        for &[a, b, c] in self.indices.as_chunks::<3>().0 {
            let (a, b, c) = (a as usize, b as usize, c as usize);
            let (Some(pa), Some(pb), Some(pc)) = (self.positions.get(a), self.positions.get(b), self.positions.get(c)) else { continue };
            let (Some(ua), Some(ub), Some(uc)) = (self.uvs.get(a), self.uvs.get(b), self.uvs.get(c)) else { continue };
            let (e1, e2) = (*pb - *pa, *pc - *pa);
            let (d1, d2) = (*ub - *ua, *uc - *ua);
            let determinant = d1.x * d2.y - d2.x * d1.y;
            if determinant.abs() < 1e-12 {
                continue;
            }

            let tangent = (e1 * d2.y - e2 * d1.y) / determinant;
            for v in [a, b, c] {
                if let Some(x) = tangents.get_mut(v) {
                    *x += tangent;
                }
            }
        }

        tangents.into_iter().map(|t| t.try_normalize().unwrap_or(Vec3::X).extend(1.0)).collect()
    }

    /// The UV tiles material `m` covers: integer parts of its triangles' UV centroids (v down),
    /// ascending. Non-finite UVs are skipped.
    pub fn uv_tiles(&self, m: usize) -> std::collections::BTreeSet<(i32, i32)> {
        self.triangles_of(m)
            .filter_map(|t| {
                let [a, b, c] = self.tri(t)?;
                let uv = (*self.uvs.get(a)? + *self.uvs.get(b)? + *self.uvs.get(c)?) / 3.0;
                uv.is_finite().then(|| (uv.x.floor() as i32, uv.y.floor() as i32))
            })
            .collect()
    }

    /// Remap the UVs of material `m`'s triangles with `f`. Vertices they share with other
    /// materials are duplicated first, so the other triangles keep their UVs and tangents. The
    /// material's tangents are regenerated for the new UVs.
    pub fn map_material_uvs(&mut self, m: usize, f: impl Fn(Vec2) -> Vec2) {
        self.map_material_uvs_with(m, |_, uv| f(uv));
    }

    /// [`Mesh::map_material_uvs`] where `f` also gets the UV centroid of the triangle each corner
    /// belongs to: piecewise maps (UV tiles of different sizes) then place a corner on a tile
    /// border with its own triangle's tile. A vertex whose corners map to different UVs is split.
    pub fn map_material_uvs_with(&mut self, m: usize, f: impl Fn(Vec2, Vec2) -> Vec2) {
        let material_triangles: Vec<usize> = self.triangles_of(m).collect();
        let mut in_material = vec![false; self.triangle_count()];
        for t in &material_triangles {
            if let Some(x) = in_material.get_mut(*t) {
                *x = true;
            }
        }

        let mut shared = vec![false; self.positions.len()];
        for (_, tri) in self.indices.as_chunks::<3>().0.iter().enumerate().filter(|(t, _)| !in_material.get(*t).copied().unwrap_or(false)) {
            for v in tri {
                if let Some(s) = shared.get_mut(*v as usize) {
                    *s = true;
                }
            }
        }

        let mut duplicates = std::collections::HashMap::new();
        for k in material_triangles.iter().flat_map(|t| t * 3..t * 3 + 3) {
            let Some(&v) = self.indices.get(k) else { continue };
            let v = if shared.get(v as usize).copied().unwrap_or(false) {
                *duplicates.entry(v).or_insert_with(|| self.duplicate_vertex(v as usize))
            } else {
                v
            };
            if let Some(i) = self.indices.get_mut(k) {
                *i = v;
            }
        }

        // The mapped UV of every corner, from the UVs before any is replaced.
        let mut corners = Vec::with_capacity(material_triangles.len() * 3);
        for t in &material_triangles {
            let Some(tri) = self.tri(*t) else { continue };
            let centroid = tri.iter().map(|v| self.uvs.get(*v).copied().unwrap_or_default()).sum::<Vec2>() / 3.0;
            for (k, v) in (t * 3..t * 3 + 3).zip(tri) {
                corners.push((k, v, f(centroid, self.uvs.get(v).copied().unwrap_or_default())));
            }
        }

        // A vertex whose corners map to different UVs (a tile border under a piecewise map) gets
        // one copy per distinct result.
        // Keyed by vertex and the result's bit pattern, so each corner is one lookup.
        let mut results: std::collections::HashMap<(usize, [u32; 2]), u32> = std::collections::HashMap::new();
        let mut used = std::collections::HashSet::new();
        let mut new_uvs = Vec::with_capacity(corners.len());
        for (k, v, uv) in corners {
            let key = (v, uv.to_array().map(f32::to_bits));
            let target = if let Some(&i) = results.get(&key) {
                i
            } else {
                let i = if used.insert(v) { v as u32 } else { self.duplicate_vertex(v) };
                results.insert(key, i);
                new_uvs.push((i, uv));
                i
            };
            if let Some(i) = self.indices.get_mut(k) {
                *i = target;
            }
        }
        for (i, uv) in new_uvs {
            if let Some(x) = self.uvs.get_mut(i as usize) {
                *x = uv;
            }
        }

        // The old tangents follow the old UVs; only this material's vertices get new ones.
        if self.tangents.len() == self.positions.len() && self.normals.len() == self.positions.len() {
            self.generate_tangents_for(&material_triangles);
        }
    }

    /// Append a copy of vertex `v`; returns its index.
    fn duplicate_vertex(&mut self, v: usize) -> u32 {
        let i = self.positions.len() as u32;
        self.positions.push(self.positions.get(v).copied().unwrap_or_default());
        self.normals.push(self.normals.get(v).copied().unwrap_or(Vec3::Y));
        self.tangents.push(self.tangents.get(v).copied().unwrap_or(Vec4::new(1.0, 0.0, 0.0, 1.0)));
        self.uvs.push(self.uvs.get(v).copied().unwrap_or_default());
        if !self.colors.is_empty() {
            self.colors.push(self.colors.get(v).copied().unwrap_or(Vec4::ONE));
        }
        i
    }

    /// Append `other`, offsetting indices; its submeshes keep their material indices shifted by
    /// this mesh's material count.
    pub fn append(&mut self, other: &Mesh) {
        // A mesh with submeshes and no material names uses one implicit material 0; give it a
        // slot so the other side's materials do not take its index.
        if self.materials.is_empty() && !self.submeshes.is_empty() {
            self.materials.push(String::new());
        }
        let base = self.positions.len() as u32;
        let ibase = self.indices.len() as u32;
        let mbase = self.materials.len();
        self.positions.extend_from_slice(&other.positions);
        self.normals.extend_from_slice(&other.normals);
        self.tangents.extend_from_slice(&other.tangents);
        self.uvs.extend_from_slice(&other.uvs);
        if !(self.colors.is_empty() && other.colors.is_empty()) {
            // Vertices of the side without colors become white.
            self.colors.resize(base as usize, Vec4::ONE);
            self.colors.extend(other.colors.iter().copied().chain(std::iter::repeat(Vec4::ONE)).take(other.positions.len()));
        }
        if !(self.diagonals.is_empty() && other.diagonals.is_empty()) {
            self.diagonals.resize(ibase as usize / 3, 0);
            self.diagonals.extend(other.diagonals.iter().copied().chain(std::iter::repeat(0)).take(other.triangle_count()));
        }
        self.indices.extend(other.indices.iter().map(|i| i + base));
        self.materials.extend(other.materials.iter().cloned());
        if other.materials.is_empty() && !other.submeshes.is_empty() {
            self.materials.push(String::new());
        }
        self.submeshes.extend(other.submeshes.iter().map(|s| SubMesh {
            name: s.name.clone(),
            material: s.material + mbase,
            start: s.start + ibase,
            count: s.count,
        }));
    }

    /// Each triangle split into four at its edge midpoints (shared edges share their midpoint);
    /// attributes are interpolated and submesh ranges keep their triangles.
    pub fn subdivided(&self) -> Mesh {
        let mut m = Mesh { indices: Vec::with_capacity(self.indices.len() * 4), ..self.clone() };
        let has = |len: usize| len == self.positions.len();
        let (has_n, has_t, has_uv, has_c) = (has(self.normals.len()), has(self.tangents.len()), has(self.uvs.len()), has(self.colors.len()));
        let mut mids = std::collections::HashMap::new();
        let mut mid = |m: &mut Mesh, a: u32, b: u32| -> u32 {
            *mids.entry((a.min(b), a.max(b))).or_insert_with(|| {
                let (i, j) = (a as usize, b as usize);
                m.positions.push((self.positions[i] + self.positions[j]) * 0.5);
                if has_n {
                    m.normals.push((self.normals[i] + self.normals[j]).normalize_or_zero());
                }
                if has_t {
                    let (ti, tj) = (self.tangents[i], self.tangents[j]);
                    m.tangents.push((ti.truncate() + tj.truncate()).normalize_or_zero().extend(ti.w));
                }
                if has_uv {
                    m.uvs.push((self.uvs[i] + self.uvs[j]) * 0.5);
                }
                if has_c {
                    m.colors.push((self.colors[i] + self.colors[j]) * 0.5);
                }
                (m.positions.len() - 1) as u32
            })
        };
        // Each child keeps the hidden bits of the halves of the parent's edges it has; the edges
        // inside the parent are real.
        let mut diagonals = Vec::with_capacity(if self.diagonals.is_empty() { 0 } else { self.diagonals.len().saturating_mul(4) });
        // First child index of each parent triangle (and the end): ranges are rebuilt from the
        // children actually emitted, since a parent with an index out of range emits none.
        let mut child_start = Vec::with_capacity(self.triangle_count() + 1);
        for (k, t) in self.indices.as_chunks::<3>().0.iter().enumerate() {
            child_start.push(m.indices.len() as u32);
            if t.iter().any(|&i| i as usize >= self.positions.len()) {
                continue;
            }
            let (a, b, c) = (t[0], t[1], t[2]);
            let (ab, bc, ca) = (mid(&mut m, a, b), mid(&mut m, b, c), mid(&mut m, c, a));
            m.indices.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
            if !self.diagonals.is_empty() {
                let h = self.diagonals.get(k).copied().unwrap_or(0);
                diagonals.extend_from_slice(&[h & 0b101, h & 0b011, h & 0b110, 0]);
            }
        }
        m.diagonals = diagonals;
        child_start.push(m.indices.len() as u32);
        let child = |index: u32| child_start.get((index / 3) as usize).or(child_start.last()).copied().unwrap_or(0);
        for s in &mut m.submeshes {
            let (start, end) = (child(s.start), child(s.start.saturating_add(s.count)));
            s.start = start;
            s.count = end.saturating_sub(start);
        }
        m
    }

    /// Uniformly scale and translate so the mesh is centered with the given bounding radius.
    pub fn normalize_to(&mut self, radius: f32) {
        let (lo, hi) = self.bounds();
        let c = (lo + hi) * 0.5;
        let s = radius / self.radius();
        for p in &mut self.positions {
            *p = (*p - c) * s;
        }
    }

    /// A single-material mesh from raw attributes; computes missing normals and tangents.
    pub fn from_parts(positions: Vec<Vec3>, normals: Option<Vec<Vec3>>, uvs: Vec<Vec2>, indices: Vec<u32>, material: &str) -> Result<Mesh> {
        let count = indices.len() as u32;
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
        m.validate_indices()?;
        if m.positions.iter().any(|p| !p.is_finite()) {
            return Err(GeomError::Invalid("mesh has non-finite positions".into()));
        }
        if m.normals.len() != m.positions.len() {
            m.compute_normals();
        }
        m.compute_tangents();
        m.validate()?;
        Ok(m)
    }
}

struct Mikk<'a> {
    mesh: &'a Mesh,
    /// The tangent of each index (face corner)
    corners: Vec<Vec4>,
}

impl Mikk<'_> {
    fn vi(&self, face: usize, vert: usize) -> usize {
        self.mesh.indices.get(face * 3 + vert).copied().unwrap_or(0) as usize
    }
}

impl mikktspace::Geometry for Mikk<'_> {
    fn num_faces(&self) -> usize {
        self.mesh.indices.len() / 3
    }
    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }
    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        self.mesh.positions.get(self.vi(face, vert)).copied().unwrap_or_default().to_array()
    }
    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        self.mesh.normals.get(self.vi(face, vert)).copied().unwrap_or(Vec3::Y).to_array()
    }
    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        // MikkTSpace expects OpenGL texture coordinates (v up); our UVs are glTF (v down). With
        // v flipped, the bitangent `cross(N, T) * w` points up the image, as glTF requires.
        let uv = self.mesh.uvs.get(self.vi(face, vert)).copied().unwrap_or_default();
        [uv.x, 1.0 - uv.y]
    }
    fn set_tangent_encoded(&mut self, tangent: [f32; 4], face: usize, vert: usize) {
        if let Some(t) = face.checked_mul(3).and_then(|k| k.checked_add(vert)).and_then(|k| self.corners.get_mut(k)) {
            *t = Vec4::from_array(tangent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut m = quad();
        assert_eq!(m.wire_edges(), vec![vec![(0, 1), (1, 2), (0, 2), (2, 3), (0, 3)]], "triangles show every edge");
        // Edge 2 of (0, 1, 2) and edge 0 of (0, 2, 3) split the quad.
        m.diagonals = vec![0b100, 0b001];
        assert_eq!(m.wire_edges(), vec![vec![(0, 1), (1, 2), (2, 3), (0, 3)]]);
        // A diagonal whose twin a seam split off (vertex 4 copies vertex 0) stays.
        let mut seam = m.clone();
        seam.indices[3] = 4;
        assert_eq!(seam.wire_edges()[0].len(), 6);
        // Appending keeps each side's diagonals on its own triangles.
        let mut a = Mesh::from_parts(m.positions.clone(), None, m.uvs.clone(), vec![0, 1, 2], "A").unwrap();
        a.append(&m);
        assert_eq!(a.diagonals, vec![0, 0b100, 0b001]);
        assert_eq!(crate::primitives::plane(1.0, 2).wire_edges()[0].len(), 12, "a 2×2 grid of quads has 12 edges");
        // Subdividing hides both halves of the diagonal and nothing else: 16 edges less 2.
        let sub = m.subdivided();
        assert_eq!(sub.diagonals.len(), sub.triangle_count());
        let edges = &sub.wire_edges()[0];
        assert_eq!(edges.len(), 14);
        assert!(edges.iter().all(|&(a, b)| !matches!((a, b), (0, 6) | (2, 6))), "diagonal halves {edges:?}");
    }

    #[test]
    fn uv_seams_are_where_the_uv_layout_splits() {
        let mesh = quad();
        assert_eq!(mesh.uv_seam_edges(), vec![Vec::<(u32, u32)>::new()], "one UV island has no seam");

        // Vertex 4 copies vertex 0's position with another UV: the diagonal (0, 2) / (4, 2) is cut
        let mut seam = mesh.clone();
        seam.positions.push(seam.positions[0]);
        seam.normals.push(seam.normals[0]);
        seam.tangents.push(seam.tangents[0]);
        seam.uvs.push(Vec2::new(0.9, 0.9));
        seam.indices[3] = 4;
        assert_eq!(seam.uv_seam_edges(), vec![vec![(0, 2), (2, 4)]]);

        // The same split with the same UV is a hard edge, not a seam
        let mut hard = seam.clone();
        hard.uvs[4] = hard.uvs[0];
        assert_eq!(hard.uv_seam_edges(), vec![Vec::<(u32, u32)>::new()]);
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
        assert_eq!(mesh.uv_seam_edges(), vec![vec![(0, 2), (2, 4)]]);
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

        assert_eq!(mesh.uv_seam_edges(), vec![vec![(0, 2)], vec![(2, 4)]]);
        mesh.uvs[4] = mesh.uvs[0];
        assert_eq!(mesh.uv_seam_edges(), vec![Vec::<(u32, u32)>::new(); 2], "a material boundary alone is not a UV seam");
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
    fn from_parts_rejects_invalid_indices_before_generating_tangents() {
        for indices in [vec![0, 1, 99], vec![0, 1]] {
            let result = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, vec![Vec2::ZERO, Vec2::X, Vec2::Y], indices, "m");
            assert!(matches!(result, Err(GeomError::Invalid(_))));
        }
    }

    #[test]
    fn compute_tangents_with_out_of_range_indices_uses_fallback() {
        let mut mesh = Mesh {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            uvs: vec![Vec2::ZERO, Vec2::X, Vec2::Y],
            indices: vec![0, 1, 99],
            ..Mesh::default()
        };
        mesh.compute_tangents();
        assert_eq!(mesh.tangents.len(), mesh.positions.len());
        assert!(mesh.tangents.iter().all(|tangent| tangent.is_finite()));
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

        // A side with submeshes and no material names keeps its implicit material.
        let mut unnamed = quad();
        unnamed.materials.clear();
        let mut named = quad();
        named.append(&unnamed);
        named.validate().unwrap();
        assert_eq!((named.materials.len(), named.submeshes[1].material), (2, 1));
        unnamed.append(&quad());
        unnamed.validate().unwrap();
        assert_eq!((unnamed.materials.len(), unnamed.submeshes[1].material), (2, 1));
    }

    #[test]
    fn material_uvs_remap_and_split_shared_vertices() {
        // quad(): two triangles sharing an edge; put the second in its own material.
        let mut m = quad();
        m.materials = vec!["a".into(), "b".into()];
        m.submeshes =
            vec![SubMesh { name: "a".into(), material: 0, start: 0, count: 3 }, SubMesh { name: "b".into(), material: 1, start: 3, count: 3 }];
        let before = m.clone();
        m.map_material_uvs(1, |uv| uv + Vec2::X);
        m.validate().unwrap();
        assert_eq!(m.vertex_count(), 6, "the two shared vertices are split");
        assert_eq!(&m.indices[..3], &before.indices[..3]);
        for k in 0..3 {
            let (old, new) = (before.indices[k] as usize, m.indices[k] as usize);
            assert_eq!(m.uvs[new], before.uvs[old], "material a keeps its UVs");
            let (old, new) = (before.indices[3 + k] as usize, m.indices[3 + k] as usize);
            assert_eq!((m.uvs[new], m.positions[new]), (before.uvs[old] + Vec2::X, before.positions[old]));
        }
        assert_eq!(m.uv_tiles(1).into_iter().collect::<Vec<_>>(), vec![(1, 0)]);
        assert_eq!(m.uv_tiles(0).into_iter().collect::<Vec<_>>(), vec![(0, 0)]);
    }

    #[test]
    fn mirrored_material_uvs_get_new_tangents_and_the_other_material_keeps_its_own() {
        let mut m = quad();
        m.materials = vec!["a".into(), "b".into()];
        m.submeshes =
            vec![SubMesh { name: "a".into(), material: 0, start: 0, count: 3 }, SubMesh { name: "b".into(), material: 1, start: 3, count: 3 }];
        let before = m.clone();
        m.map_material_uvs(1, |uv| Vec2::new(1.0 - uv.x, uv.y));
        m.validate().unwrap();
        for k in 0..3 {
            let (old, new) = (before.indices[k] as usize, m.indices[k] as usize);
            assert_eq!(m.tangents[new], before.tangents[old], "material a keeps its tangents");
            let t = m.tangents[m.indices[3 + k] as usize];
            assert!((t.truncate() + Vec3::X).length() < 1e-4, "mirrored u runs along -x: {t:?}");
        }
    }

    #[test]
    fn a_mirrored_uv_join_splits_the_shared_vertices() {
        // The fourth UV equals the second: the second triangle's UVs wind the other way.
        let positions = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)];
        let uvs = vec![Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0)];
        let m = Mesh::from_parts(positions, None, uvs, vec![0, 1, 2, 0, 2, 3], "Default").unwrap();
        assert_eq!(m.vertex_count(), 6, "vertices 0 and 2 get one copy per handedness");
        let w = |t: usize| m.tri(t).unwrap().map(|v| m.tangents[v].w);
        let (first, second) = (w(0), w(1));
        assert!(first.iter().all(|x| *x == first[0]) && second.iter().all(|x| *x == second[0]), "{first:?} {second:?}");
        assert_ne!(first[0], second[0]);
        assert_eq!(quad().vertex_count(), 4, "corners that agree keep their shared vertex");
    }

    #[test]
    fn non_finite_positions_skip_mikktspace() {
        let positions = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(f32::NAN, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)];
        let uvs = vec![Vec2::ZERO; 4];
        assert!(Mesh::from_parts(positions.clone(), None, uvs.clone(), vec![0, 1, 2, 0, 2, 3], "Default").is_err());
        let mut m = quad();
        m.positions = positions;
        m.compute_tangents();
        assert_eq!(m.tangents.len(), 4);
    }

    #[test]
    fn triangle_lookups_out_of_range_return_none() {
        let m = quad();
        assert_eq!(m.tri(usize::MAX), None);
        assert_eq!(m.material_of(usize::MAX), None);
        assert_eq!(m.material_of(2), None);
        let mut bad = quad();
        bad.submeshes[0].start = 1;
        bad.submeshes[0].count = 3;
        assert!(bad.validate().is_err(), "a range off the triangle grid");
    }

    #[test]
    fn validate_rejects_submeshes_that_share_triangles() {
        let mut m = quad();
        m.materials = vec!["a".into(), "b".into()];
        m.submeshes =
            vec![SubMesh { name: "a".into(), material: 0, start: 0, count: 6 }, SubMesh { name: "b".into(), material: 1, start: 3, count: 3 }];
        assert!(m.validate().is_err());
        m.submeshes[0].count = 3;
        m.validate().unwrap();
        // One material may list a triangle twice; a long range of it still hides no overlap.
        m.submeshes =
            vec![SubMesh { name: "a".into(), material: 0, start: 0, count: 6 }, SubMesh { name: "a2".into(), material: 0, start: 0, count: 3 }];
        m.validate().unwrap();
        m.submeshes.push(SubMesh { name: "b".into(), material: 1, start: 3, count: 3 });
        assert!(m.validate().is_err());
    }

    #[test]
    fn subdivision_ranges_follow_the_children_emitted() {
        // The first triangle has an index out of range and emits no children.
        let mut m = quad();
        m.materials = vec!["a".into(), "b".into()];
        m.submeshes =
            vec![SubMesh { name: "a".into(), material: 0, start: 0, count: 3 }, SubMesh { name: "b".into(), material: 1, start: 3, count: 3 }];
        m.indices[0] = 99;
        let s = m.subdivided();
        assert_eq!(s.triangle_count(), 4);
        assert_eq!((s.submeshes[0].start, s.submeshes[0].count), (0, 0));
        assert_eq!((s.submeshes[1].start, s.submeshes[1].count), (0, 12));
    }

    #[test]
    fn material_uvs_split_vertices_on_a_tile_border() {
        // Two triangles share the edge at u = 1; the first lies in tile 0, the second in tile 1.
        let positions = vec![Vec3::new(0.5, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(1.5, 0.0, 0.0)];
        let uvs = vec![Vec2::new(0.5, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(1.5, 0.0)];
        let mut m = Mesh::from_parts(positions, None, uvs, vec![0, 1, 2, 1, 3, 2], "m").unwrap();
        // Tile 0 packs into 0..0.5 and tile 1 into 0.6..1.1.
        m.map_material_uvs_with(0, |c, uv| if c.x < 1.0 { uv * 0.5 } else { (uv - Vec2::X) * 0.5 + Vec2::new(0.6, 0.0) });
        m.validate().unwrap();
        let u = |t: usize| m.tri(t).unwrap().map(|v| m.uvs[v].x);
        assert_eq!(u(0), [0.25, 0.5, 0.5]);
        assert_eq!(u(1), [0.6, 0.85, 0.6]);
    }
}
