//! The internal triangle mesh.

use glam::{DVec3, Vec2, Vec3, Vec4};

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

/// Positions and UVs go to MikkTSpace only when their largest magnitude lies between the
/// reciprocal of this and this, so edge lengths, cross products and their products with UVs
/// neither overflow nor underflow in `f32`. Other meshes get the `f64` per-vertex tangents.
const SAFE_MAGNITUDE: f32 = 1e6;

/// Most triangles around one vertex (as MikkTSpace welds vertices) for which tangents come from
/// MikkTSpace. It visits the triangles around a vertex recursively, one call per triangle, so a
/// larger fan could exhaust the stack; such meshes get the per-vertex fallback. This crate's own
/// value.
pub const MAX_MIKKTSPACE_VALENCE: usize = 2_048;

/// Most comparisons MikkTSpace may make welding corners, for which tangents come from it. It
/// compares each corner with the corners before it at the same position until one has the same
/// normal and UV, so `k` corners at one position can cost about `k²` comparisons; the sum of `k²`
/// over the positions stays within this. This crate's own value: the 6,144 corners at the pole of
/// a sphere with 2,048 segments take about 3.8e7.
pub const MAX_MIKKTSPACE_WELD_WORK: u64 = 1 << 27;

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
        let i = self.indices.get(t * 3..t * 3 + 3)?;
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
        let idx = (t * 3) as u32;
        self.submeshes.iter().find(|s| idx >= s.start && idx < s.start.saturating_add(s.count)).map(|s| s.material)
    }

    /// Triangle index range belonging to material `m` (all submeshes of that material).
    pub fn triangles_of(&self, m: usize) -> impl Iterator<Item = usize> + '_ {
        self.submeshes.iter().filter(move |s| s.material == m).flat_map(|s| (s.start / 3) as usize..((s.start + s.count) / 3) as usize)
    }

    /// Check that attribute arrays agree and every index is in range.
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
            if s.start as usize + s.count as usize > self.indices.len() || s.material >= self.materials.len().max(1) {
                return Err(GeomError::Invalid(format!("submesh {} is out of range", s.name)));
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

    /// MikkTSpace tangents (falls back to per-vertex tangents if generation fails). A mesh with
    /// an index out of range, a non-finite position or UV, a position or UV with a magnitude
    /// outside 1e-6 to 1e6 (zero is fine), only degenerate triangles, more than
    /// [`MAX_MIKKTSPACE_VALENCE`] triangles around one vertex or corners whose welding would take
    /// more than [`MAX_MIKKTSPACE_WELD_WORK`] comparisons does not go to MikkTSpace. Missing
    /// normals are computed and missing UVs are zero. A non-finite normal counts as +Y.
    ///
    /// MikkTSpace gives each triangle corner its own tangent frame. A vertex whose corners get
    /// different frames (UVs mirrored across it) is split: every further frame gets a copy of
    /// the vertex, and the corners with that frame use the copy. A copy whose index would not
    /// fit in `u32` is not made, and those corners keep the first frame.
    pub fn compute_tangents(&mut self) {
        let n = self.positions.len();
        if self.normals.len() != n {
            self.compute_normals();
        }
        if self.uvs.len() != n {
            self.uvs = vec![Vec2::ZERO; n];
        }
        let normals: Vec<Vec3> = self.normals.iter().map(|v| v.try_normalize().unwrap_or(Vec3::Y)).collect();
        let mut corners = vec![Vec4::ZERO; self.indices.len()];

        // mikktspace crashes (out-of-bounds) when every triangle is degenerate; skip it then. Its
        // callbacks read a missing vertex as the origin, which the degenerate check cannot see,
        // so an index past the last vertex skips it too.
        let usable = in_safe_range(self.positions.iter().flat_map(|p| p.to_array()))
            && in_safe_range(self.uvs.iter().flat_map(|uv| uv.to_array()))
            && self.indices.iter().all(|&i| (i as usize) < n);
        let mut g = Mikk { positions: &self.positions, normals: &normals, uvs: &self.uvs, indices: &self.indices, corners: &mut corners };
        if !(usable && !g.all_triangles_degenerate() && g.affordable() && mikktspace::generate_tangents(&mut g)) {
            let per_vertex = simple_tangents(&self.positions, &normals, &self.uvs, &self.indices);
            for (corner, v) in corners.iter_mut().zip(&self.indices) {
                *corner = per_vertex.get(*v as usize).copied().unwrap_or(Vec4::new(1.0, 0.0, 0.0, 1.0));
            }
        }

        // Orthonormalize a tangent against the normal of vertex `v`; repair degenerate results.
        let frame = |v: usize, t: Vec4| {
            let n = normals.get(v).copied().unwrap_or(Vec3::Y);
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
        let mut tangents = vec![None::<Vec4>; n];
        let mut copies: Vec<(u32, Vec4)> = Vec::new();
        let mut copy_of: std::collections::HashMap<(u32, [u32; 4]), u32> = std::collections::HashMap::new();
        for (index, &t) in self.indices.iter_mut().zip(&corners) {
            let v = *index;
            let Some(slot) = tangents.get_mut(v as usize) else { continue };
            match slot {
                None => *slot = Some(t),
                Some(first) if bits(*first) == bits(t) => {}
                Some(_) => {
                    if let Some(&copy) = copy_of.get(&(v, bits(t))) {
                        *index = copy;
                    } else if let Ok(copy) = u32::try_from(n + copies.len()) {
                        copies.push((v, t));
                        copy_of.insert((v, bits(t)), copy);
                        *index = copy;
                    }
                }
            }
        }

        // A vertex no triangle uses gets +X made perpendicular to its normal.
        let mut out: Vec<Vec4> = tangents.iter().enumerate().map(|(v, t)| t.unwrap_or_else(|| frame(v, Vec4::new(1.0, 0.0, 0.0, 1.0)))).collect();
        out.extend(copies.iter().map(|(_, t)| *t));
        let has_colors = self.colors.len() == n;
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
    /// materials are duplicated first, so the other triangles keep their UVs.
    pub fn map_material_uvs(&mut self, m: usize, f: impl Fn(Vec2) -> Vec2) {
        self.map_material_uvs_with(m, |_, uv| f(uv));
    }

    /// [`Mesh::map_material_uvs`] where `f` also gets the UV centroid of the first of the
    /// material's triangles using the vertex: piecewise maps (UV tiles of different sizes) then
    /// place a vertex on a tile border with its own triangle's tile.
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

        // (vertex, centroid of its first triangle), each vertex once
        let mut vertices = Vec::with_capacity(material_triangles.len() * 3);
        for t in &material_triangles {
            let Some(tri) = self.tri(*t) else { continue };
            let centroid = tri.iter().map(|v| self.uvs.get(*v).copied().unwrap_or_default()).sum::<Vec2>() / 3.0;
            vertices.extend(tri.map(|v| (v, centroid)));
        }

        vertices.sort_by_key(|(v, _)| *v);
        vertices.dedup_by_key(|(v, _)| *v);
        for (v, centroid) in vertices {
            if let Some(uv) = self.uvs.get_mut(v) {
                *uv = f(centroid, *uv);
            }
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
        for t in self.indices.as_chunks::<3>().0 {
            if t.iter().any(|&i| i as usize >= self.positions.len()) {
                continue;
            }
            let (a, b, c) = (t[0], t[1], t[2]);
            let (ab, bc, ca) = (mid(&mut m, a, b), mid(&mut m, b, c), mid(&mut m, c, a));
            m.indices.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
        }
        for s in &mut m.submeshes {
            s.start *= 4;
            s.count *= 4;
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
        if m.normals.len() != m.positions.len() {
            m.compute_normals();
        }
        m.compute_tangents();
        m.validate()?;
        Ok(m)
    }
}

/// Per-vertex tangents from the UV gradients of the triangles around each vertex: the fallback
/// when MikkTSpace does not run or fails. `w` is -1 where the summed direction up the image
/// (toward smaller v) lies against `cross(normal, tangent)`, and +1 otherwise, as MikkTSpace
/// sets it. Edges, UV differences and sums are taken in `f64`, and a triangle is skipped when
/// its UV edges are nearly parallel relative to their lengths, so tiny UV differences still
/// set the direction.
fn simple_tangents(positions: &[Vec3], normals: &[Vec3], uvs: &[Vec2], indices: &[u32]) -> Vec<Vec4> {
    let mut sums = vec![(DVec3::ZERO, DVec3::ZERO); positions.len()];
    for &[a, b, c] in indices.as_chunks::<3>().0 {
        let (a, b, c) = (a as usize, b as usize, c as usize);
        let (Some(pa), Some(pb), Some(pc)) = (positions.get(a), positions.get(b), positions.get(c)) else { continue };
        let (Some(ua), Some(ub), Some(uc)) = (uvs.get(a), uvs.get(b), uvs.get(c)) else { continue };
        let (e1, e2) = (pb.as_dvec3() - pa.as_dvec3(), pc.as_dvec3() - pa.as_dvec3());
        let (d1, d2) = (ub.as_dvec2() - ua.as_dvec2(), uc.as_dvec2() - ua.as_dvec2());
        let determinant = d1.x * d2.y - d2.x * d1.y;
        if !determinant.is_finite() || determinant.abs() <= 1e-12 * d1.length() * d2.length() {
            continue;
        }

        let tangent = (e1 * d2.y - e2 * d1.y) / determinant;
        // The direction in which v shrinks, which is up the image.
        let up = (e1 * d2.x - e2 * d1.x) / determinant;
        if !tangent.is_finite() || !up.is_finite() {
            continue;
        }
        for v in [a, b, c] {
            if let Some((t, u)) = sums.get_mut(v) {
                *t += tangent;
                *u += up;
            }
        }
    }

    sums.into_iter()
        .enumerate()
        .map(|(v, (t, up))| {
            let t = t.try_normalize().map(|t| t.as_vec3()).filter(|t| t.is_finite()).unwrap_or(Vec3::X);
            let up = up.try_normalize().map(|up| up.as_vec3()).unwrap_or(Vec3::ZERO);
            let n = normals.get(v).copied().unwrap_or(Vec3::Y);
            let w = if n.cross(t).dot(up) < 0.0 { -1.0 } else { 1.0 };
            t.extend(w)
        })
        .collect()
}

/// Whether the values are finite and their largest magnitude is zero or lies between the
/// reciprocal of [`SAFE_MAGNITUDE`] and it.
fn in_safe_range(values: impl Iterator<Item = f32>) -> bool {
    let mut largest = 0.0_f32;
    for c in values {
        if !c.is_finite() {
            return false;
        }
        largest = largest.max(c.abs());
    }
    largest == 0.0 || (1.0 / SAFE_MAGNITUDE..=SAFE_MAGNITUDE).contains(&largest)
}

/// The arrays MikkTSpace reads, and the tangent of every triangle corner it writes.
struct Mikk<'a> {
    positions: &'a [Vec3],
    normals: &'a [Vec3],
    uvs: &'a [Vec2],
    indices: &'a [u32],
    corners: &'a mut [Vec4],
}

impl Mikk<'_> {
    fn vi(&self, face: usize, vert: usize) -> usize {
        self.indices.get(face * 3 + vert).copied().unwrap_or(0) as usize
    }

    /// True when the mesh has triangles and every one of them has two corners at the same
    /// position as MikkTSpace gets it.
    fn all_triangles_degenerate(&self) -> bool {
        use mikktspace::Geometry;
        self.num_faces() > 0
            && (0..self.num_faces()).all(|f| {
                let p = |v| self.position(f, v);
                p(0) == p(1) || p(0) == p(2) || p(1) == p(2)
            })
    }

    /// Whether MikkTSpace's work on this input stays bounded: at most
    /// [`MAX_MIKKTSPACE_VALENCE`] triangles around one vertex, which it visits recursively, one
    /// call per triangle, and at most [`MAX_MIKKTSPACE_WELD_WORK`] for welding.
    fn affordable(&self) -> bool {
        let (valence, weld_work) = self.welding();
        valence <= MAX_MIKKTSPACE_VALENCE && weld_work <= MAX_MIKKTSPACE_WELD_WORK
    }

    /// The most triangles that share one vertex as MikkTSpace sees it, and the sum of the squared
    /// corner counts of its positions. It welds corners whose position, normal and UV, as this
    /// type passes them, are equal (0 and -0 alike).
    fn welding(&self) -> (usize, u64) {
        use mikktspace::Geometry;
        let bits = |x: f32| if x == 0.0 { 0 } else { x.to_bits() };
        let mut count: std::collections::HashMap<[u32; 8], usize> = std::collections::HashMap::new();
        let mut at_position: std::collections::HashMap<[u32; 3], u64> = std::collections::HashMap::new();
        let (mut most, mut work) = (0, 0u64);
        for face in 0..self.num_faces() {
            for vert in 0..3 {
                let ([px, py, pz], [nx, ny, nz], [u, v]) = (self.position(face, vert), self.normal(face, vert), self.tex_coord(face, vert));
                let c = count.entry([px, py, pz, nx, ny, nz, u, v].map(bits)).or_insert(0);
                *c += 1;
                most = most.max(*c);
                // k² grows by 2k + 1 with the (k + 1)th corner.
                let k = at_position.entry([px, py, pz].map(bits)).or_insert(0);
                work = work.saturating_add(k.saturating_mul(2).saturating_add(1));
                *k += 1;
            }
        }
        (most, work)
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
        (self.positions.get(self.vi(face, vert)).copied().unwrap_or_default()).to_array()
    }
    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        self.normals.get(self.vi(face, vert)).copied().unwrap_or(Vec3::Y).to_array()
    }
    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        // MikkTSpace expects OpenGL texture coordinates (v up); our UVs are glTF (v down). With
        // v negated, the bitangent `cross(N, T) * w` points up the image, as glTF requires.
        // Negating keeps differences in v that `1 - v` would round away.
        let uv = self.uvs.get(self.vi(face, vert)).copied().unwrap_or_default();
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
    fn a_vertex_with_mirrored_frames_is_split() {
        // Triangle (0, 1, 2) has u growing with x. Triangle (1, 3, 2) has the opposite UV
        // orientation, so vertices 1 and 2 need one tangent frame for each triangle.
        let corners = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.0)];
        let m = Mesh::from_parts(
            corners.to_vec(),
            None,
            vec![Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 0.0), Vec2::new(1.0, 2.0)],
            vec![0, 1, 2, 1, 3, 2],
            "Default",
        )
        .unwrap();
        assert_eq!(m.positions.len(), 6, "vertices 1 and 2 each get a copy");
        assert_eq!((m.normals.len(), m.uvs.len(), m.tangents.len()), (6, 6, 6));
        let signs = |t: usize| m.indices[t * 3..t * 3 + 3].iter().map(|&i| m.tangents[i as usize].w).collect::<Vec<_>>();
        assert!(signs(0).iter().all(|w| *w == signs(0)[0]), "{:?}", signs(0));
        assert!(signs(1).iter().all(|w| *w == signs(1)[0]), "{:?}", signs(1));
        assert_ne!(signs(0)[0], signs(1)[0]);
        let originals = [0, 1, 2, 3, 1, 2];
        for (i, p) in m.positions.iter().enumerate() {
            assert_eq!(*p, corners[originals[i]], "vertex {i}");
        }
        m.validate().unwrap();
    }

    #[test]
    fn split_vertices_keep_their_colors() {
        let mut m = Mesh {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.0)],
            uvs: vec![Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 0.0), Vec2::new(1.0, 2.0)],
            colors: (0..4).map(|i| Vec4::splat(i as f32 / 4.0)).collect(),
            indices: vec![0, 1, 2, 1, 3, 2],
            materials: vec!["Default".into()],
            submeshes: vec![SubMesh { name: "Default".into(), material: 0, start: 0, count: 6 }],
            ..Mesh::default()
        };
        m.compute_tangents();
        assert_eq!(m.positions.len(), 6);
        assert_eq!(m.colors, [0, 1, 2, 3, 1, 2].map(|i| Vec4::splat(i as f32 / 4.0)));
        m.validate().unwrap();
    }

    #[test]
    fn tangents_stay_finite_for_non_finite_input() {
        let mut m = quad();
        m.normals[0] = Vec3::NAN;
        m.normals[1] = Vec3::splat(f32::INFINITY);
        m.compute_tangents();
        assert!(m.tangents.iter().all(|t| t.is_finite() && (t.truncate().length() - 1.0).abs() < 1e-4), "{:?}", m.tangents);

        let mut m = quad();
        m.uvs[2] = Vec2::new(f32::NAN, 0.0);
        m.positions[1].x = f32::INFINITY;
        m.compute_tangents();
        assert!(m.tangents.iter().all(|t| t.is_finite()), "{:?}", m.tangents);
    }

    #[test]
    fn far_and_tiny_triangles_keep_their_tangent() {
        for edge in [1e-20_f32, 1e20] {
            let m = Mesh::from_parts(
                vec![Vec3::ZERO, Vec3::new(edge, 0.0, 0.0), Vec3::new(0.0, edge, 0.0)],
                Some(vec![Vec3::Z; 3]),
                vec![Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 0.0)],
                vec![0, 1, 2],
                "Default",
            )
            .unwrap();
            for t in &m.tangents {
                assert!((t.truncate() - Vec3::X).length() < 1e-4 && t.w == 1.0, "edge {edge}: {t:?}");
            }
        }
    }

    #[test]
    fn fallback_tangents_agree_with_mikktspace() {
        // Each mesh is the quad with its UVs flipped one way; the fallback sets the same
        // direction and bitangent sign MikkTSpace does.
        for flip in [Vec2::new(1.0, 1.0), Vec2::new(-1.0, 1.0), Vec2::new(1.0, -1.0)] {
            let mut m = quad();
            for uv in &mut m.uvs {
                *uv = Vec2::new(uv.x * flip.x, uv.y * flip.y);
            }
            m.compute_tangents();
            let fallback = simple_tangents(&m.positions, &m.normals, &m.uvs, &m.indices);
            assert_eq!(m.tangents.len(), 4);
            for (mikk, simple) in m.tangents.iter().zip(&fallback) {
                assert!((mikk.truncate() - simple.truncate()).length() < 1e-4, "flip {flip}: {mikk:?} {simple:?}");
                assert_eq!(mikk.w, simple.w, "flip {flip}");
            }
        }
    }

    #[test]
    fn fallback_tangents_use_tiny_uv_differences() {
        // u grows with y and v with x, over a range of 1e-7: the UV determinant is -1e-14.
        let s = 1e-7_f32;
        let positions = [Vec3::ZERO, Vec3::X, Vec3::Y];
        let uvs = [Vec2::ZERO, Vec2::new(0.0, s), Vec2::new(s, 0.0)];
        for t in simple_tangents(&positions, &[Vec3::Z; 3], &uvs, &[0, 1, 2]) {
            assert!((t.truncate() - Vec3::Y).length() < 1e-4, "{t:?}");
        }
    }

    #[test]
    fn a_fan_around_one_vertex_skips_mikktspace() {
        for (k, affordable) in [(MAX_MIKKTSPACE_VALENCE, true), (MAX_MIKKTSPACE_VALENCE + 1, false)] {
            let mut positions = vec![Vec3::ZERO];
            for i in 0..=k {
                let a = i as f32 / k as f32 * std::f32::consts::TAU;
                positions.push(Vec3::new(a.cos(), a.sin(), 0.0));
            }
            let indices: Vec<u32> = (0..k as u32).flat_map(|i| [0, i + 1, i + 2]).collect();
            let (normals, uvs) = (vec![Vec3::Z; positions.len()], vec![Vec2::ZERO; positions.len()]);
            let mut corners = vec![Vec4::ZERO; indices.len()];
            let g = Mikk { positions: &positions, normals: &normals, uvs: &uvs, indices: &indices, corners: &mut corners };
            assert_eq!(g.affordable(), affordable, "{k} triangles");
        }
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
}
