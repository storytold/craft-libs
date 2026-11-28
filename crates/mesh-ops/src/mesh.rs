//! The internal triangle mesh.

use glam::{DVec3, Vec2, Vec3, Vec4};

use crate::{GeomError, Result};

/// An edge by the bits of its two end positions, the lower one first
type EdgePositions = ([u32; 3], [u32; 3]);

/// A wireframe corner: the bits of its position and UV, `None` past the end of either array.
type WireCorner = (Option<[u32; 3]>, Option<[u32; 2]>);

/// The two corners of a wireframe edge, the lower one first.
type WireEnds = (WireCorner, WireCorner);

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

/// `Invalid` unless the index count is a multiple of 3, the triangles are at most
/// [`MAX_TRIANGLES`], every index names one of `positions` and every position is finite. Reads
/// the slices only, so it runs before any copy is made.
pub(crate) fn check_triangles(positions: &[Vec3], indices: &[u32]) -> Result<()> {
    let n = positions.len();
    if !indices.len().is_multiple_of(3) {
        return Err(GeomError::Invalid("index count is not a multiple of 3".into()));
    }
    if indices.len() / 3 > MAX_TRIANGLES {
        return Err(GeomError::Invalid(format!("mesh has more than {MAX_TRIANGLES} triangles")));
    }
    if let Some(bad) = indices.iter().find(|&&i| i as usize >= n) {
        return Err(GeomError::Invalid(format!("index {bad} out of range ({n} vertices)")));
    }
    if positions.iter().any(|p| !p.is_finite()) {
        return Err(GeomError::Invalid("mesh has non-finite positions".into()));
    }
    Ok(())
}

/// A component of [`face_cross`] computed from rounded edges is kept when its magnitude is more
/// than this times the sum of the magnitudes of its two products. The rounding error of such a
/// component is at most `(3 + 16ε)ε` times that sum (`ε = 2^-53`; Shewchuk, "Adaptive Precision
/// Floating-Point Arithmetic and Fast Robust Geometric Predicates", 1997, the bound for `orient2d`),
/// so a kept component is within about `2^-20` of its exact value. This crate's own value.
const FAST_CROSS_MARGIN: f64 = 1.0 / (1u64 << 30) as f64;

/// The cross product of the edges of the triangle `a, b, c` in `f64`: twice its area times its
/// unit normal, on the side from which its corners run counterclockwise. The edges are taken from
/// the corner opposite the longest edge, so the two shorter edges keep their direction when a
/// difference of corners far apart rounds: for corners `(2^80, 2^80, 0)`, `(0, 0, 0)` and
/// `(0, 1, 0)` the edges from the first corner round to one direction, and the edges from the
/// third corner give -Z. A component whose two products cancel past [`FAST_CROSS_MARGIN`] is
/// computed again from the corners with [`exact_cross_component`], so a component the rounded
/// edges lose keeps its sign: for corners `(2^80, 2^80, 0)`, `(2^-80, 0, 0)` and `(1, 1, 0)` every
/// pair of edges rounds to one direction, and the exact product `-1 + 2^-80` gives -Z.
fn face_cross(a: Vec3, b: Vec3, c: Vec3) -> DVec3 {
    let (da, db, dc) = (a.as_dvec3(), b.as_dvec3(), c.as_dvec3());
    let (ab, bc, ca) = (db - da, dc - db, da - dc);
    let (ab2, bc2, ca2) = (ab.length_squared(), bc.length_squared(), ca.length_squared());
    let (u, v) = if ab2 >= bc2 && ab2 >= ca2 {
        (ca, -bc)
    } else if bc2 >= ca2 {
        (ab, -ca)
    } else {
        (bc, -ab)
    };
    let (u, v, corners) = (u.to_array(), v.to_array(), [a.to_array(), b.to_array(), c.to_array()]);
    // Component k of u × v is u[i] v[j] - u[j] v[i] for the axes (i, j) that follow k.
    let component = |i: usize, j: usize| {
        let (Some(&ui), Some(&uj), Some(&vi), Some(&vj)) = (u.get(i), u.get(j), v.get(i), v.get(j)) else { return 0.0 };
        let (p, q) = (ui * vj, uj * vi);
        let (d, sum) = (p - q, p.abs() + q.abs());
        if sum == 0.0 || d.abs() > FAST_CROSS_MARGIN * sum { d } else { exact_cross_component(corners, i, j) }
    };
    DVec3::new(component(1, 2), component(2, 0), component(0, 1))
}

/// The component of the cross product of the triangle `corners` along the axis that follows the
/// axes `i` and `j`, as `a × b + b × c + c × a`. Each product of two `f32` values is exact in
/// `f64`, and their sum is kept exactly as an expansion of non-overlapping `f64` parts
/// (Shewchuk's `grow_expansion`), so the result is the exact component rounded once more. `f32`
/// products stay within `2^-298` and `2^256`, where `f64` neither underflows nor overflows.
fn exact_cross_component(corners: [[f32; 3]; 3], i: usize, j: usize) -> f64 {
    let mut parts = [0.0f64; 6];
    let mut len = 0;
    let mut term = |x: f64| {
        let mut carry = x;
        let mut kept = 0;
        for k in 0..len {
            let Some(&part) = parts.get(k) else { break };
            let (sum, err) = two_sum(carry, part);
            if err != 0.0 {
                if let Some(slot) = parts.get_mut(kept) {
                    *slot = err;
                }
                kept += 1;
            }
            carry = sum;
        }
        if let Some(slot) = parts.get_mut(kept) {
            *slot = carry;
        }
        len = kept + 1;
    };
    for k in 0..3 {
        let (Some(p), Some(q)) = (corners.get(k), corners.get((k + 1) % 3)) else { continue };
        let (Some(&pi), Some(&pj), Some(&qi), Some(&qj)) = (p.get(i), p.get(j), q.get(i), q.get(j)) else { continue };
        term(f64::from(pi) * f64::from(qj));
        term(-(f64::from(pj) * f64::from(qi)));
    }
    // The parts grow in magnitude, so the smaller ones are added first.
    parts.iter().take(len).sum()
}

/// `a + b` rounded, and the error of that rounding: the two add up to `a + b` exactly (Knuth's
/// two-sum).
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let sum = a + b;
    let b_part = sum - a;
    let a_part = sum - b_part;
    (sum, (a - a_part) + (b - b_part))
}

/// The unit normal of the triangle `a, b, c`, on the side from which its corners run
/// counterclockwise, or +Z when the triangle has no area. The cross product of its edges is
/// taken in `f64` with [`face_cross`], where it neither overflows nor rounds to zero for finite
/// `f32` corners.
fn face_normal(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    face_cross(a, b, c).try_normalize().map_or(Vec3::Z, |n| n.as_vec3())
}

/// A mesh with one vertex per triangle corner, as [`flat_corners`] makes it.
pub(crate) struct FlatCorners {
    pub positions: Vec<Vec3>,
    /// The [`face_normal`] of each vertex's triangle.
    pub normals: Vec<Vec3>,
    /// `0, 1, 2, ...`: the triangles in the input's order.
    pub indices: Vec<u32>,
    /// The input vertex each vertex copies.
    pub source: Vec<u32>,
}

/// One vertex per corner of the triangles `indices` lists, with its triangle's [`face_normal`].
/// The triangles are checked with [`check_triangles`] on the borrowed slices before any memory
/// is reserved.
pub(crate) fn flat_corners(positions: &[Vec3], indices: &[u32]) -> Result<FlatCorners> {
    check_triangles(positions, indices)?;
    let count = u32::try_from(indices.len()).map_err(|_| GeomError::Invalid("index count does not fit in u32".into()))?;
    let mut out = FlatCorners {
        positions: Vec::with_capacity(indices.len()),
        normals: Vec::with_capacity(indices.len()),
        indices: Vec::with_capacity(indices.len()),
        source: Vec::with_capacity(indices.len()),
    };
    for &[a, b, c] in indices.as_chunks::<3>().0 {
        let (Some(&pa), Some(&pb), Some(&pc)) = (positions.get(a as usize), positions.get(b as usize), positions.get(c as usize)) else {
            return Err(GeomError::Invalid(format!("triangle ({a}, {b}, {c}) is out of range ({} vertices)", positions.len())));
        };
        let normal = face_normal(pa, pb, pc);
        out.positions.extend([pa, pb, pc]);
        out.normals.extend([normal; 3]);
        out.source.extend([a, b, c]);
    }
    out.indices.extend(0..count);
    Ok(out)
}

/// The value of `values` for each of `source`, or an empty array when `values` is empty.
fn gather<T: Copy>(values: &[T], source: &[u32], what: &'static str) -> Result<Vec<T>> {
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(source.len());
    for &s in source {
        let value = values.get(s as usize).ok_or_else(|| GeomError::Invalid(format!("vertex {s} has no {what}")))?;
        out.push(*value);
    }
    Ok(out)
}

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
    /// polygon diagonals ([`Mesh::diagonals`]) unless only one triangle hides an edge between
    /// corners at those positions with those UVs (a UV seam split it from its twin), per material
    /// in `0..materials.len().max(1)`. Twins are matched by position and UV, so a diagonal stays
    /// hidden when each of its triangles has its own copies of the vertices, as after
    /// [`Mesh::try_flat_normals`].
    pub fn wire_edges(&self) -> Vec<Vec<(u32, u32)>> {
        // Signed zero is the same position or UV on either side of a split vertex.
        let bits = |value: f32| if value == 0.0 { 0 } else { value.to_bits() };
        let corner = |index: u32| -> WireCorner {
            (self.positions.get(index as usize).map(|p| p.to_array().map(bits)), self.uvs.get(index as usize).map(|uv| uv.to_array().map(bits)))
        };
        (0..self.materials.len().max(1))
            .map(|m| {
                // Edge → (real in some triangle, its corners), in first-seen order.
                let mut edges: Vec<((u32, u32), bool, WireEnds)> = Vec::new();
                let mut at: std::collections::HashMap<(u32, u32), usize> = std::collections::HashMap::new();
                // Corners of a diagonal → triangles that hide an edge between such corners.
                let mut twins: std::collections::HashMap<WireEnds, u8> = std::collections::HashMap::new();
                for t in self.triangles_of(m) {
                    let Some(v) = self.indices.get(t * 3..t * 3 + 3) else { continue };
                    let hidden = self.diagonals.get(t).copied().unwrap_or(0);
                    for (i, (a, b)) in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])].into_iter().enumerate() {
                        let key = (a.min(b), a.max(b));
                        let real = hidden >> i & 1 == 0;
                        let (ca, cb) = (corner(a), corner(b));
                        let ends = if ca <= cb { (ca, cb) } else { (cb, ca) };
                        let e = *at.entry(key).or_insert_with(|| {
                            edges.push((key, false, ends));
                            edges.len() - 1
                        });
                        if let Some((_, r, _)) = edges.get_mut(e) {
                            *r |= real;
                        }
                        if !real {
                            let n = twins.entry(ends).or_insert(0);
                            *n = n.saturating_add(1);
                        }
                    }
                }
                edges.into_iter().filter(|(_, real, ends)| *real || twins.get(ends).copied() == Some(1)).map(|(k, ..)| k).collect()
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

    /// Flat normals: every triangle corner gets its own vertex, with its triangle's unit normal
    /// (on the side from which its corners run counterclockwise, from the cross product of its
    /// edges in `f64`) and the UV and color of the vertex it copies. A triangle without area gets
    /// +Z. The indices become `0, 1, 2, ...`, so submeshes, materials and polygon diagonals
    /// keep their triangles, and vertices no triangle uses are dropped. When the mesh has one
    /// tangent per vertex, tangents are computed again for the new normals
    /// ([`Mesh::compute_tangents`], from zero UVs when the mesh has none, and the UVs stay
    /// empty); otherwise they stay empty.
    ///
    /// An index count that is not a multiple of 3, more than [`MAX_TRIANGLES`] triangles, an
    /// index past the last vertex, a non-finite position, UV or color, or UVs, colors or tangents
    /// that are neither empty nor one per vertex are `Invalid`, with the mesh unchanged.
    pub fn try_flat_normals(&mut self) -> Result<()> {
        let n = self.positions.len();
        for (what, len) in [("UVs", self.uvs.len()), ("vertex colors", self.colors.len()), ("tangents", self.tangents.len())] {
            if len != 0 && len != n {
                return Err(GeomError::Invalid(format!("{len} {what} for {n} vertices")));
            }
        }
        if self.uvs.iter().any(|v| !v.is_finite()) || self.colors.iter().any(|v| !v.is_finite()) {
            return Err(GeomError::Invalid("non-finite UV or color".into()));
        }
        let corners = flat_corners(&self.positions, &self.indices)?;
        let mut flat = Mesh {
            uvs: gather(&self.uvs, &corners.source, "UVs")?,
            colors: gather(&self.colors, &corners.source, "colors")?,
            positions: corners.positions,
            normals: corners.normals,
            indices: corners.indices,
            ..Mesh::default()
        };
        if !self.tangents.is_empty() {
            flat.compute_tangents();
            // Tangents for a mesh without UVs come from zero UVs, which the mesh does not keep.
            if self.uvs.is_empty() {
                flat.uvs = Vec::new();
            }
        }
        flat.submeshes = std::mem::take(&mut self.submeshes);
        flat.materials = std::mem::take(&mut self.materials);
        flat.diagonals = std::mem::take(&mut self.diagonals);
        *self = flat;
        Ok(())
    }

    /// MikkTSpace tangents (falls back to per-triangle tangents if generation fails).
    pub fn compute_tangents(&mut self) {
        self.tangents = vec![Vec4::new(1.0, 0.0, 0.0, 1.0); self.positions.len()];
        if self.normals.len() != self.positions.len() {
            self.compute_normals();
        }
        if self.uvs.len() != self.positions.len() {
            self.uvs = vec![Vec2::ZERO; self.positions.len()];
        }
        // mikktspace crashes (out-of-bounds) when every triangle is degenerate; skip it then.
        let usable = !crate::unwrap::all_triangles_degenerate(self);
        let mut g = Mikk { mesh: self };
        if !usable || !mikktspace::generate_tangents(&mut g) {
            self.simple_tangents();
        }
        for (t, n) in self.tangents.iter_mut().zip(&self.normals) {
            // Orthonormalize against the normal; repair degenerate results.
            let v = t.truncate();
            let o = (v - *n * n.dot(v)).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
            *t = o.extend(if t.w < 0.0 { -1.0 } else { 1.0 });
        }
    }

    fn simple_tangents(&mut self) {
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

        self.tangents = tangents.into_iter().map(|t| t.try_normalize().unwrap_or(Vec3::X).extend(1.0)).collect();
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

struct Mikk<'a> {
    mesh: &'a mut Mesh,
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
        let i = self.vi(face, vert);
        if let Some(t) = self.mesh.tangents.get_mut(i) {
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

    /// A cube from -1 to 1: eight shared corners (bit 0 of the index is +X, bit 1 +Y, bit 2 +Z)
    /// and two triangles per face, counterclockwise seen from outside.
    fn cube_corners() -> (Vec<Vec3>, Vec<u32>) {
        let positions = (0..8u32).map(|i| Vec3::from_array([0, 1, 2].map(|axis| if i >> axis & 1 == 1 { 1.0 } else { -1.0 }))).collect();
        let quads = [[0, 4, 6, 2], [1, 3, 7, 5], [0, 1, 5, 4], [2, 6, 7, 3], [0, 2, 3, 1], [4, 5, 7, 6]];
        (positions, quads.iter().flat_map(|[a, b, c, d]| [*a, *b, *c, *a, *c, *d]).collect())
    }

    fn flat(positions: &[Vec3], indices: &[u32]) -> crate::FlatNormals {
        crate::flat_normals(positions, indices).unwrap()
    }

    #[test]
    fn flat_normals_of_a_cube_are_its_face_normals() {
        let (positions, indices) = cube_corners();
        let f = flat(&positions, &indices);
        assert_eq!(f.indices, (0..36).collect::<Vec<u32>>());
        assert_eq!(f.source, indices);
        for (k, n) in f.normals.iter().enumerate() {
            assert_eq!(f.positions[k], positions[f.source[k] as usize]);
            // The face's center lies on the axis it faces.
            let corners = &indices[k / 3 * 3..k / 3 * 3 + 3];
            let center = corners.iter().map(|&v| positions[v as usize]).sum::<Vec3>() / 3.0;
            let axis = (0..3).max_by(|&a, &b| center[a].abs().total_cmp(&center[b].abs())).unwrap();
            let mut face = Vec3::ZERO;
            face[axis] = center[axis].signum();
            assert_eq!(*n, face, "corner {k}");
        }
    }

    #[test]
    fn a_zero_area_flat_triangle_gets_z_and_makes_no_nan() {
        // A unit square in the YZ plane facing +X, a triangle along its edge from vertex 0 through
        // vertex 1 to vertex 4, and a triangle with a repeated corner.
        let positions = [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 1.0], [0.0, 0.0, 1.0], [0.0, 2.0, 0.0]].map(Vec3::from_array);
        let f = flat(&positions, &[0, 1, 2, 0, 2, 3, 0, 1, 4, 2, 2, 3]);
        let expected = [Vec3::X; 6].into_iter().chain([Vec3::Z; 6]).collect::<Vec<_>>();
        assert_eq!(f.normals, expected);
    }

    #[test]
    fn flat_normals_of_an_empty_mesh_are_empty() {
        assert_eq!(
            crate::flat_normals(&[], &[]).unwrap(),
            crate::FlatNormals { positions: vec![], normals: vec![], indices: vec![], source: vec![] }
        );
        // A vertex without triangles is left out.
        assert!(crate::flat_normals(&[Vec3::new(1.0, 2.0, 3.0)], &[]).unwrap().positions.is_empty());
        let mut m = Mesh::default();
        m.try_flat_normals().unwrap();
        assert_eq!(m, Mesh::default());
    }

    #[test]
    fn flat_normals_reject_bad_indices_and_non_finite_positions() {
        let good = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]].map(Vec3::from_array);
        let nan = [[0.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0], [0.0, 1.0, 0.0]].map(Vec3::from_array);
        let infinite = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, f32::INFINITY, 0.0]].map(Vec3::from_array);
        let malformed: [(&[Vec3], &[u32]); 5] =
            [(&good, &[0, 1, 3]), (&good, &[0, 1, 2, 0]), (&good, &[0, 1, 2, 2, 1]), (&nan, &[0, 1, 2]), (&infinite, &[0, 1, 2])];
        for (positions, indices) in malformed {
            assert!(matches!(crate::flat_normals(positions, indices), Err(GeomError::Invalid(_))), "{positions:?} {indices:?}");
            let mut m = Mesh { positions: positions.to_vec(), normals: vec![Vec3::X; 3], indices: indices.to_vec(), ..Mesh::default() };
            // Debug output compares NaN positions too.
            let before = format!("{m:?}");
            assert!(matches!(m.try_flat_normals(), Err(GeomError::Invalid(_))), "{positions:?} {indices:?}");
            assert_eq!(format!("{m:?}"), before);
        }
    }

    #[test]
    fn flat_normals_check_indices_before_copying_many_positions() {
        // Many positions and an index buffer that is malformed only at its end: every check fails
        // on the borrowed slices, so no output is built.
        let positions = vec![Vec3::ONE; 1 << 20];
        let mut indices: Vec<u32> = (0..3 * 1000).map(|i| i % (1 << 20)).collect();
        indices.push(0);
        assert!(matches!(crate::flat_normals(&positions, &indices), Err(GeomError::Invalid(m)) if m.contains("multiple of 3")));
        indices.extend([0, 1 << 20]);
        assert!(matches!(crate::flat_normals(&positions, &indices), Err(GeomError::Invalid(m)) if m.contains("out of range")));
        let mut far = positions.clone();
        far[(1 << 20) - 1].x = f32::NAN;
        assert!(matches!(crate::flat_normals(&far, &indices[..3000]), Err(GeomError::Invalid(m)) if m.contains("non-finite")));
        let mut m = Mesh { positions, indices, ..Mesh::default() };
        let before = m.clone();
        assert!(matches!(m.try_flat_normals(), Err(GeomError::Invalid(_))));
        assert_eq!(m, before);
    }

    #[test]
    fn flat_normals_carry_vertex_data_and_compute_tangents_again() {
        // The quad's two triangles in two materials, its diagonal hidden, and tangents that no
        // longer match its normals.
        let mut m = quad();
        m.materials = vec!["a".into(), "b".into()];
        m.submeshes = vec![
            SubMesh { name: "first".into(), material: 0, start: 0, count: 3 },
            SubMesh { name: "second".into(), material: 1, start: 3, count: 3 },
        ];
        m.diagonals = vec![0b100, 0b001];
        m.colors = (0..4).map(|i| Vec4::new(i as f32 / 4.0, 0.5, 0.25, 1.0)).collect();
        m.tangents = vec![Vec4::new(0.0, 1.0, 0.0, 1.0); 4];
        let before = m.clone();
        m.try_flat_normals().unwrap();
        m.validate().unwrap();
        assert_eq!(m.indices, (0..6).collect::<Vec<u32>>());
        for (k, s) in [0, 1, 2, 0, 2, 3].into_iter().enumerate() {
            assert_eq!((m.positions[k], m.uvs[k], m.colors[k], m.normals[k]), (before.positions[s], before.uvs[s], before.colors[s], Vec3::Z));
            assert!((m.tangents[k] - quad().tangents[s]).length() < 1e-4, "{:?}", m.tangents[k]);
        }
        assert_eq!((&m.submeshes, &m.materials, &m.diagonals), (&before.submeshes, &before.materials, &before.diagonals));
        // A mesh without tangents, UVs or colors gets none.
        let mut bare = Mesh { positions: before.positions.clone(), indices: before.indices.clone(), ..Mesh::default() };
        bare.try_flat_normals().unwrap();
        assert_eq!((bare.normals.len(), bare.uvs.len(), bare.colors.len(), bare.tangents.len()), (6, 0, 0, 0));
        // A mesh with tangents and no UVs keeps tangents and gets no UVs.
        let mut no_uvs = Mesh { uvs: Vec::new(), ..before };
        no_uvs.try_flat_normals().unwrap();
        assert_eq!((no_uvs.uvs.len(), no_uvs.tangents.len()), (0, 6));
    }

    #[test]
    fn flat_normals_reject_short_or_non_finite_vertex_data() {
        let changes: [fn(&mut Mesh); 4] =
            [|m| m.uvs.truncate(3), |m| m.colors = vec![Vec4::ONE; 2], |m| m.tangents.truncate(3), |m| m.uvs[0].x = f32::NAN];
        for (k, change) in changes.into_iter().enumerate() {
            let mut m = quad();
            change(&mut m);
            let before = format!("{m:?}");
            assert!(matches!(m.try_flat_normals(), Err(GeomError::Invalid(_))), "change {k}");
            assert_eq!(format!("{m:?}"), before, "change {k}");
        }
    }

    #[test]
    fn flat_normals_keep_polygon_diagonals_hidden_in_wireframes() {
        let mut m = quad();
        m.diagonals = vec![0b100, 0b001];
        assert_eq!(m.wire_edges()[0].len(), 4);
        m.try_flat_normals().unwrap();
        // Vertices 3 and 4 copy the diagonal's corners 0 and 2; the four sides are drawn once each.
        assert_eq!(m.wire_edges(), vec![vec![(0, 1), (1, 2), (4, 5), (3, 5)]]);
        // A diagonal whose copies have other UVs on one side is a seam and is drawn on both sides.
        let mut seam = m.clone();
        seam.uvs[3] = Vec2::new(0.9, 0.9);
        assert_eq!(seam.wire_edges(), vec![vec![(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5)]]);
        // A drawn edge of another triangle is no twin: only the first triangle hides the edge.
        let mut lone = m.clone();
        lone.diagonals = vec![0b100, 0];
        assert_eq!(lone.wire_edges()[0].len(), 6);
    }

    #[test]
    fn flat_normals_keep_a_short_side_next_to_far_corners() {
        // The edges from the first corner both round to (-2^80, -2^80, 0) in f64.
        let far = 2.0f32.powi(80);
        let positions = [Vec3::new(far, far, 0.0), Vec3::ZERO, Vec3::Y];
        for indices in [[0, 1, 2], [1, 2, 0], [2, 0, 1]] {
            assert_eq!(flat(&positions, &indices).normals, vec![Vec3::NEG_Z; 3], "{indices:?}");
        }
    }

    #[test]
    fn flat_normals_keep_terms_that_differences_of_corners_lose() {
        // Every difference of two corners rounds in f64 to a multiple of (1, 1, 0), so the edges
        // from any corner are parallel. The exact cross product is (0, 0, -1 + 2^-80).
        let far = 2.0f32.powi(80);
        let positions = [[far, far, 0.0], [far.recip(), 0.0, 0.0], [1.0, 1.0, 0.0]];
        for indices in [[0, 1, 2], [1, 2, 0], [2, 0, 1]] {
            assert_eq!(flat(&positions.map(Vec3::from_array), &indices).normals, vec![Vec3::NEG_Z; 3], "{indices:?}");
        }
        // The same triangle turned so that its normal is -X and then -Y.
        let turned = |f: fn([f32; 3]) -> [f32; 3]| positions.map(|p| Vec3::from_array(f(p)));
        assert_eq!(flat(&turned(|[x, y, z]| [z, x, y]), &[0, 1, 2]).normals, vec![Vec3::NEG_X; 3]);
        assert_eq!(flat(&turned(|[x, y, z]| [y, z, x]), &[0, 1, 2]).normals, vec![Vec3::NEG_Y; 3]);
    }
}
