//! Automatic UV unwrapping for meshes imported without texture coordinates.
//!
//! Triangles are grouped into charts by flood fill across shared (welded) edges while each face
//! normal stays within 60° of the chart's seed face, so every chart is a height field over its
//! seed plane and projects onto it without flipping. Charts are rotated to their principal axes
//! and shelf-packed per material into 0..1 with one uniform scale, so texel density is the same
//! everywhere within a material.

use std::collections::{HashMap, VecDeque};

use glam::{Vec2, Vec3};

use crate::{Mesh, MeshOpsError, Result};

/// Faces of a chart stay within this angle of the seed face normal (cos 60°).
const CHART_COS: f32 = 0.5;
/// Packing never goes beyond this virtual resolution (texels per side).
const MAX_PACK_RES: f32 = 65_536.0;
/// Bounds within this magnitude, with a diagonal of at least [`PLAIN_WELD_DIAGONAL`], are welded
/// in `f32`. Larger and smaller ones are welded in `f64`, where the diagonal and the grid cell
/// neither overflow nor round to zero.
const PLAIN_WELD_MAGNITUDE: f32 = 1e9;
const PLAIN_WELD_DIAGONAL: f32 = 1e-6;

/// Welded vertex id per vertex: vertices at the same position (snapped to a grid of 1e-6 of
/// the bounding diagonal) share an id. Ids are dense, in order of first appearance. The grid
/// comes from the finite positions only.
pub fn weld_ids(positions: &[Vec3]) -> Vec<u32> {
    let (lo, hi) = positions
        .iter()
        .filter(|p| p.is_finite())
        .fold(None, |b: Option<(Vec3, Vec3)>, p| Some(b.map_or((*p, *p), |(lo, hi)| (lo.min(*p), hi.max(*p)))))
        .unwrap_or((Vec3::ZERO, Vec3::ZERO));
    let plain = lo.abs().max(hi.abs()).max_element() <= PLAIN_WELD_MAGNITUDE && (lo == hi || (hi - lo).length() >= PLAIN_WELD_DIAGONAL);
    let cell = ((hi - lo).length() * 1e-6).max(1e-12);
    let (wide_lo, wide_cell) = (lo.as_dvec3(), (hi.as_dvec3() - lo.as_dvec3()).length() * 1e-6);
    let mut map: HashMap<[i64; 3], u32> = HashMap::with_capacity(positions.len());
    positions
        .iter()
        .map(|p| {
            let key = if plain {
                let q = ((*p - lo) / cell).round();
                [q.x as i64, q.y as i64, q.z as i64]
            } else {
                // A zero cell (every finite position the same) divides 0 by 0; NaN casts to 0.
                let q = ((p.as_dvec3() - wide_lo) / wide_cell).round();
                [q.x as i64, q.y as i64, q.z as i64]
            };
            let next = map.len() as u32;
            *map.entry(key).or_insert(next)
        })
        .collect()
}

/// True when the mesh has triangles and every one of them has two corners at the same position.
/// MikkTSpace (and so [`Mesh::compute_tangents`]) crashes on such meshes, so callers reject them.
pub fn all_triangles_degenerate(mesh: &Mesh) -> bool {
    let p = |i: &u32| mesh.positions.get(*i as usize);
    !mesh.indices.is_empty()
        && mesh.indices.chunks(3).all(|t| match t {
            [a, b, c] => p(a) == p(b) || p(a) == p(c) || p(b) == p(c),
            _ => true,
        })
}

struct Chart {
    material: usize,
    /// Projection basis (right-handed with the seed normal).
    t: Vec3,
    b: Vec3,
    /// Rotation to the chart's principal axes (cos, sin).
    rot: Vec2,
    lo: Vec2,
    hi: Vec2,
    /// Texel offset of the chart's packed rectangle and the material's scale and resolution.
    offset: Vec2,
}

impl Chart {
    fn local(&self, p: Vec3) -> Vec2 {
        let q = Vec2::new(p.dot(self.t), p.dot(self.b));
        Vec2::new(q.x * self.rot.x - q.y * self.rot.y, q.x * self.rot.y + q.y * self.rot.x)
    }
    fn size(&self) -> Vec2 {
        (self.hi - self.lo).max(Vec2::ZERO)
    }
}

/// Replace the UVs of `mesh` with an automatic non-overlapping layout: each material gets its
/// own 0..1 space with charts at least `padding_texels` (minimum 1, which a NaN counts as) away
/// from each other and the borders at `resolution`. Vertices are split at chart borders;
/// tangents are recomputed.
/// Returns the number of charts. A mesh that fails the checks (indices, submesh ranges and
/// materials, finite positions, finite normals and colors when it has them, at least one
/// triangle that is not degenerate) is left unchanged.
pub fn auto_unwrap(mesh: &mut Mesh, padding_texels: f32, resolution: u32) -> Result<usize> {
    let (charts, _) = unwrap_core(mesh, padding_texels, resolution, usize::MAX)?;
    mesh.compute_tangents();
    mesh.validate()?;
    Ok(charts)
}

/// [`auto_unwrap`] without the tangents and the final check: returns the chart count and, per
/// new vertex, the old vertex it copies. More than `max_vertices` new vertices is an error; the
/// rebuilt arrays stop growing at most three vertices past it.
pub(crate) fn unwrap_core(mesh: &mut Mesh, padding_texels: f32, resolution: u32, max_vertices: usize) -> Result<(usize, Vec<u32>)> {
    mesh.validate_indices()?;
    if mesh.positions.iter().any(|p| !p.is_finite()) {
        return Err(MeshOpsError::Invalid("mesh has non-finite positions".into()));
    }
    if all_triangles_degenerate(mesh) {
        return Err(MeshOpsError::Invalid("every triangle of the mesh is degenerate".into()));
    }
    // Kept apart from the mesh until the layout is packed, so an error leaves the mesh as it was.
    let computed_normals = (mesh.normals.len() != mesh.positions.len()).then(|| mesh.smooth_normals());
    let old_normals = computed_normals.as_deref().unwrap_or(&mesh.normals);
    let n = mesh.triangle_count();
    let weld = weld_ids(&mesh.positions);
    // The layout depends on the shape only, so the charts are made from positions divided into
    // the range where face normals, areas and projections neither overflow nor round to zero.
    let divisor = crate::mesh::safe_divisor(crate::mesh::largest_abs(mesh.positions.iter().flat_map(|p| p.to_array())));
    let original = |v: usize| mesh.positions.get(v).copied().unwrap_or_default();
    let pos = |v: usize| original(v) / divisor;
    let wid = |v: usize| weld.get(v).copied().unwrap_or(u32::MAX);

    // Face normals (None for degenerate faces) and materials.
    let mut normal: Vec<Option<Vec3>> = Vec::with_capacity(n);
    let mut area = Vec::with_capacity(n);
    for t in 0..n {
        let [a, b, c] = mesh.tri(t).unwrap_or([0; 3]);
        let cr = (pos(b) - pos(a)).cross(pos(c) - pos(a));
        area.push(cr.length() * 0.5);
        normal.push(cr.try_normalize());
    }
    let mut material = vec![0usize; n];
    for s in &mesh.submeshes {
        // Checked by validate_indices: whole triangles inside the index buffer.
        let end = (u64::from(s.start) + u64::from(s.count)) / 3;
        for t in (s.start / 3) as usize..(end as usize).min(n) {
            if let Some(m) = material.get_mut(t) {
                *m = s.material;
            }
        }
    }

    // Manifold edge adjacency over welded vertices: edge → (face, face, count).
    let mut edges: HashMap<(u32, u32), (u32, u32, u8)> = HashMap::with_capacity(n.saturating_mul(2));
    let tri_edges = |t: usize| -> [(u32, u32); 3] {
        let [a, b, c] = mesh.tri(t).unwrap_or([0; 3]);
        let key = |x: usize, y: usize| {
            let (p, q) = (wid(x), wid(y));
            (p.min(q), p.max(q))
        };
        [key(a, b), key(b, c), key(c, a)]
    };
    for t in 0..n {
        for e in tri_edges(t) {
            if e.0 == e.1 {
                continue;
            }
            let slot = edges.entry(e).or_insert((t as u32, u32::MAX, 0));
            if slot.2 == 1 {
                slot.1 = t as u32;
            }
            slot.2 = slot.2.saturating_add(1);
        }
    }

    // Grow charts from the largest unassigned faces.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| area[*b].total_cmp(&area[*a]));
    let mut chart_of = vec![u32::MAX; n];
    let mut charts: Vec<Chart> = Vec::new();
    let mut queue = VecDeque::new();
    for &seed in &order {
        if chart_of[seed] != u32::MAX {
            continue;
        }
        let id = charts.len() as u32;
        let sn = normal[seed].unwrap_or(Vec3::Z);
        let t = sn.any_orthonormal_vector();
        charts.push(Chart { material: material[seed], t, b: sn.cross(t), rot: Vec2::X, lo: Vec2::ZERO, hi: Vec2::ZERO, offset: Vec2::ZERO });
        chart_of[seed] = id;
        queue.push_back(seed);
        while let Some(f) = queue.pop_front() {
            for e in tri_edges(f) {
                let Some(&(f0, f1, 2)) = edges.get(&e) else { continue };
                let g = if f0 as usize == f { f1 } else { f0 } as usize;
                let ok = normal.get(g).is_some_and(|nn| nn.is_none_or(|nn| nn.dot(sn) >= CHART_COS));
                if ok && chart_of.get(g) == Some(&u32::MAX) && material.get(g) == Some(&material[seed]) {
                    chart_of[g] = id;
                    queue.push_back(g);
                }
            }
        }
    }

    // Principal axes and bounds of every chart.
    let mut cov = vec![(Vec2::ZERO, [0.0f64; 3], 0usize); charts.len()];
    for (t, &c) in chart_of.iter().enumerate() {
        let c = c as usize;
        for v in mesh.tri(t).unwrap_or([0; 3]) {
            if let (Some(ch), Some(acc)) = (charts.get(c), cov.get_mut(c)) {
                let q = ch.local(pos(v));
                acc.0 += q;
                acc.1[0] += (q.x * q.x) as f64;
                acc.1[1] += (q.y * q.y) as f64;
                acc.1[2] += (q.x * q.y) as f64;
                acc.2 += 1;
            }
        }
    }
    for (ch, (sum, m, k)) in charts.iter_mut().zip(&cov) {
        let k = (*k).max(1) as f64;
        let (mx, my) = (sum.x as f64 / k, sum.y as f64 / k);
        let (cxx, cyy, cxy) = (m[0] / k - mx * mx, m[1] / k - my * my, m[2] / k - mx * my);
        let angle = 0.5 * (2.0 * cxy).atan2(cxx - cyy);
        // Rotate so the principal axis lies along x.
        ch.rot = Vec2::new((-angle).cos() as f32, (-angle).sin() as f32);
        ch.lo = Vec2::splat(f32::MAX);
        ch.hi = Vec2::splat(f32::MIN);
    }
    for (t, &c) in chart_of.iter().enumerate() {
        let c = c as usize;
        for v in mesh.tri(t).unwrap_or([0; 3]) {
            if let Some(ch) = charts.get_mut(c) {
                let q = ch.local(pos(v));
                ch.lo = ch.lo.min(q);
                ch.hi = ch.hi.max(q);
            }
        }
    }
    for ch in &mut charts {
        if ch.size().y > ch.size().x {
            // Quarter turn (x, y) → (y, −x) so shelves stay low.
            ch.rot = Vec2::new(ch.rot.y, -ch.rot.x);
            let (lo, hi) = (ch.lo, ch.hi);
            ch.lo = Vec2::new(lo.y, -hi.x);
            ch.hi = Vec2::new(hi.y, -lo.x);
        }
    }

    // Pack each material's charts into its own 0..1 square.
    let pad = padding_texels.max(1.0);
    let mut scale = HashMap::new();
    let mats: std::collections::BTreeSet<usize> = charts.iter().map(|c| c.material).collect();
    for m in mats {
        let ids: Vec<usize> = (0..charts.len()).filter(|i| charts[*i].material == m).collect();
        let sizes: Vec<Vec2> = ids.iter().map(|i| charts[*i].size()).collect();
        let (s, res, offsets) = pack(&sizes, resolution.max(16) as f32, pad)?;
        for (i, o) in ids.iter().zip(offsets) {
            charts[*i].offset = o;
        }
        scale.insert(m, (s, res));
    }

    // Rebuild vertices: one per (vertex, chart).
    let mut remap: HashMap<(u32, u32), u32> = HashMap::with_capacity(mesh.positions.len().saturating_mul(2));
    let (mut positions, mut normals, mut uvs, mut colors, mut source) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut indices = Vec::with_capacity(mesh.indices.len());
    for (t, &c) in chart_of.iter().enumerate() {
        let Some(ch) = charts.get(c as usize) else { continue };
        let (s, res) = scale.get(&ch.material).copied().unwrap_or((0.0, 1.0));
        for v in mesh.tri(t).unwrap_or([0; 3]) {
            let idx = *remap.entry((v as u32, c)).or_insert_with(|| {
                let q = ch.local(pos(v));
                // Chart y points up; UV v points down the image.
                let texel = ch.offset + Vec2::splat(pad) + Vec2::new(q.x - ch.lo.x, ch.hi.y - q.y) * s;
                positions.push(original(v));
                normals.push(old_normals.get(v).copied().unwrap_or(Vec3::Y));
                if let Some(c) = mesh.colors.get(v) {
                    colors.push(*c);
                }
                uvs.push((texel / res).clamp(Vec2::ZERO, Vec2::ONE));
                source.push(v as u32);
                (positions.len() - 1) as u32
            });
            indices.push(idx);
        }
        crate::check_limit("vertex count", positions.len(), max_vertices)?;
    }
    mesh.positions = positions;
    mesh.normals = normals;
    mesh.uvs = uvs;
    mesh.colors = colors;
    mesh.indices = indices;
    Ok((charts.len(), source))
}

/// Shelf-pack rectangles of world `sizes` (each grown by `pad` texels per side) into a square of
/// `res` texels with the largest uniform scale that fits. Returns (texels per world unit,
/// resolution used, texel offset per rectangle). The resolution doubles when the padding alone
/// cannot fit (very many charts), so the result always fits in 0..1.
fn pack(sizes: &[Vec2], mut res: f32, pad: f32) -> Result<(f32, f32, Vec<Vec2>)> {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by(|a, b| sizes[*b].y.total_cmp(&sizes[*a].y));
    let place = |s: f32, res: f32| -> Option<Vec<Vec2>> {
        let mut out = vec![Vec2::ZERO; sizes.len()];
        let (mut x, mut y, mut shelf) = (0.0f32, 0.0f32, 0.0f32);
        for &i in &order {
            let r = sizes[i] * s + Vec2::splat(2.0 * pad);
            if r.x > res {
                return None;
            }
            if x + r.x > res {
                y += shelf;
                x = 0.0;
                shelf = 0.0;
            }
            if y + r.y > res {
                return None;
            }
            out[i] = Vec2::new(x, y);
            x += r.x;
            shelf = shelf.max(r.y);
        }
        Some(out)
    };
    while place(0.0, res).is_none() {
        res *= 2.0;
        if res > MAX_PACK_RES {
            return Err(MeshOpsError::Invalid("too many UV charts to pack".into()));
        }
    }
    let largest = sizes.iter().map(|s| s.max_element()).fold(0.0f32, f32::max);
    if largest <= 0.0 {
        return Ok((0.0, res, place(0.0, res).unwrap_or_default()));
    }
    let (mut lo, mut hi) = (0.0f32, (res - 2.0 * pad).max(0.0) / largest);
    if place(hi, res).is_some() {
        lo = hi;
    }
    for _ in 0..40 {
        if hi - lo <= lo * 1e-4 {
            break;
        }
        let mid = 0.5 * (lo + hi);
        if place(mid, res).is_some() { lo = mid } else { hi = mid }
    }
    Ok((lo, res, place(lo, res).unwrap_or_default()))
}

impl Mesh {
    /// The checks before unwrapping: indices, submesh ranges and materials, and the normals and
    /// colors the unwrap keeps. UVs and tangents are replaced, so they may be incomplete.
    fn validate_indices(&self) -> Result<()> {
        let n = self.positions.len();
        if !self.indices.len().is_multiple_of(3) || self.indices.iter().any(|i| *i as usize >= n) {
            return Err(MeshOpsError::Invalid("mesh indices are out of range".into()));
        }
        if self.triangle_count() > crate::MAX_TRIANGLES {
            return Err(MeshOpsError::Invalid(format!("mesh has more than {} triangles", crate::MAX_TRIANGLES)));
        }
        for s in &self.submeshes {
            let end = u64::from(s.start) + u64::from(s.count);
            if s.start % 3 != 0 || s.count % 3 != 0 || end > self.indices.len() as u64 || s.material >= self.materials.len().max(1) {
                return Err(MeshOpsError::Invalid(format!("submesh {} is out of range", s.name)));
            }
        }
        if !self.colors.is_empty() && self.colors.len() != n {
            return Err(MeshOpsError::Invalid(format!("{} vertex colors for {n} vertices", self.colors.len())));
        }
        // Normals of another length are recomputed.
        let kept_normals = if self.normals.len() == n { self.normals.as_slice() } else { &[] };
        if kept_normals.iter().any(|v| !v.is_finite()) || self.colors.iter().any(|v| !v.is_finite()) {
            return Err(MeshOpsError::Invalid("mesh has non-finite normals or colors".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UvGBuffer, primitives};

    fn check(mut m: Mesh, min_coverage: f32) {
        m.uvs.clear();
        let charts = auto_unwrap(&mut m, 4.0, 512).unwrap();
        assert!(charts >= 1);
        m.validate().unwrap();
        assert!(m.uvs.iter().all(|uv| (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y)));
        let g = UvGBuffer::new(&m, 0, 512, 512);
        let covered = g.covered_count() as f32;
        let coverage = covered / (512.0 * 512.0);
        assert!(coverage > min_coverage, "coverage {coverage} with {charts} charts");
        assert!((g.overlaps as f32) < covered * 0.005, "overlaps {} of {covered}", g.overlaps);
    }

    #[test]
    fn sphere_unwraps_without_overlaps() {
        check(primitives::sphere(1.0, 48, 24), 0.3);
    }

    #[test]
    fn cube_unwraps_without_overlaps() {
        check(primitives::cube(2.0), 0.3);
    }

    #[test]
    fn torus_and_material_ball_unwrap() {
        check(primitives::torus(1.0, 0.3, 48, 24), 0.2);
        let mut ball = primitives::material_ball();
        ball.uvs.clear();
        auto_unwrap(&mut ball, 2.0, 1024).unwrap();
        for m in 0..ball.materials.len() {
            let g = UvGBuffer::new(&ball, m, 512, 512);
            assert!((g.overlaps as f32) < g.covered_count() as f32 * 0.005 + 1.0, "material {m}: {} overlaps", g.overlaps);
        }
    }

    #[test]
    fn many_tiny_charts_still_fit() {
        // 2000 disconnected triangles: the packer grows its virtual resolution.
        let mut p = Vec::new();
        let mut idx = Vec::new();
        for i in 0..2000u32 {
            let o = Vec3::new(i as f32 * 3.0, 0.0, 0.0);
            p.extend([o, o + Vec3::X, o + Vec3::Y]);
            idx.extend([i * 3, i * 3 + 1, i * 3 + 2]);
        }
        let mut m = Mesh::from_parts(p, None, Vec::new(), idx, "m").unwrap();
        auto_unwrap(&mut m, 4.0, 128).unwrap();
        assert!(m.uvs.iter().all(|uv| (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y)));
    }

    #[test]
    fn degenerate_and_empty_meshes_do_not_fail() {
        let mut empty = Mesh::default();
        assert_eq!(auto_unwrap(&mut empty, 4.0, 256).unwrap(), 0);
        // A sliver (zero-area) triangle next to a real one.
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::X * 2.0];
        let mut m = Mesh::from_parts(p, None, Vec::new(), vec![0, 1, 2, 0, 1, 3], "m").unwrap();
        auto_unwrap(&mut m, 4.0, 256).unwrap();
        m.validate().unwrap();
        let mut point = Mesh { positions: vec![Vec3::ONE; 3], indices: vec![0, 1, 2], ..Mesh::default() };
        assert!(all_triangles_degenerate(&point));
        assert!(auto_unwrap(&mut point, 4.0, 256).is_err());
    }

    #[test]
    fn unwrap_rejects_bad_submeshes_and_positions_before_changing_the_mesh() {
        let base = primitives::plane(1.0, 1);
        // An unaligned split, an overflowing range and a non-finite position.
        let mut unaligned = base.clone();
        unaligned.materials = vec!["a".into(), "b".into()];
        let part = |name: &str, material, start, count| crate::SubMesh { name: name.into(), material, start, count };
        unaligned.submeshes = vec![part("a", 0, 0, 4), part("b", 1, 4, 2)];
        let mut overflowing = base.clone();
        overflowing.submeshes[0].start = u32::MAX - 2;
        overflowing.submeshes[0].count = 6;
        let mut nan = base.clone();
        nan.positions[1].y = f32::NAN;
        // A material past the list, colors for some vertices only and a non-finite normal.
        let mut material = base.clone();
        material.submeshes[0].material = 1;
        let mut colors = base.clone();
        colors.colors = vec![glam::Vec4::ONE; 2];
        let mut normal = base.clone();
        normal.normals[2].x = f32::INFINITY;
        let cases =
            [("unaligned", unaligned), ("overflowing", overflowing), ("nan", nan), ("material", material), ("colors", colors), ("normal", normal)];
        let mut wrong = Vec::new();
        for (name, mut m) in cases {
            // Compared as text: NaN is not equal to itself.
            let before = format!("{m:?}");
            if auto_unwrap(&mut m, 4.0, 256).is_ok() || format!("{m:?}") != before {
                wrong.push(name);
            }
        }
        assert!(wrong.is_empty(), "accepted or changed: {wrong:?}");
        // Padding no resolution can hold fails after the charts are made: the mesh still has no
        // normals.
        let mut bare = Mesh { positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y], indices: vec![0, 1, 2], ..Mesh::default() };
        let before = bare.clone();
        assert!(auto_unwrap(&mut bare, 65_536.0, 256).is_err());
        assert_eq!(bare, before);
    }

    #[test]
    fn unwrap_slices_return_source_vertices_and_respect_limits() {
        let cube = primitives::cube(2.0);
        let positions: Vec<[f32; 3]> = cube.positions.iter().map(|p| p.to_array()).collect();
        let l = crate::Limits::default();
        let out = crate::unwrap_uvs(&positions, &cube.indices, 4.0, 256, &l).unwrap();
        assert_eq!(out.indices.len(), cube.indices.len());
        assert_eq!(out.source.len(), out.uvs.len());
        for (k, &new) in out.indices.iter().enumerate() {
            assert_eq!(out.source[new as usize], cube.indices[k], "corner {k} copies its old vertex");
        }
        assert!(out.uvs.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
        assert!(out.charts >= 6);
        let few = crate::Limits { max_unwrap_triangles: cube.triangle_count() - 1, ..l };
        assert!(matches!(crate::unwrap_uvs(&positions, &cube.indices, 4.0, 256, &few), Err(MeshOpsError::TooLarge { .. })));
        assert!(crate::unwrap_uvs(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, f32::INFINITY, 0.0]], &[0, 1, 2], 4.0, 256, &l).is_err());
        assert_eq!(crate::unwrap_uvs(&[], &[], 4.0, 256, &l).unwrap().charts, 0);
        // A padding that is not finite is an error; a negative one counts as 1 texel.
        for padding in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(crate::unwrap_uvs(&positions, &cube.indices, padding, 256, &l).is_err(), "{padding}");
        }
        let with_padding = |padding: f32| crate::unwrap_uvs(&positions, &cube.indices, padding, 256, &l).unwrap();
        assert_eq!(with_padding(-3.0), with_padding(1.0));
        // In the mesh form a NaN padding counts as 1 texel too.
        let (mut with_nan, mut with_one) = (cube.clone(), cube.clone());
        assert_eq!(auto_unwrap(&mut with_nan, f32::NAN, 256).unwrap(), auto_unwrap(&mut with_one, 1.0, 256).unwrap());
        assert_eq!(with_nan, with_one);
        // A cube of 8 shared corners splits into 4 vertices per face: the limit counts those.
        let corners: Vec<[f32; 3]> = (0..8).map(|i| [(i & 1) as f32, (i >> 1 & 1) as f32, (i >> 2) as f32]).collect();
        let faces = [0, 2, 3, 0, 3, 1, 4, 5, 7, 4, 7, 6, 0, 1, 5, 0, 5, 4, 2, 6, 7, 2, 7, 3, 0, 4, 6, 0, 6, 2, 1, 3, 7, 1, 7, 5];
        let split = crate::unwrap_uvs(&corners, &faces, 4.0, 256, &l).unwrap();
        assert_eq!((split.charts, split.uvs.len()), (6, 24));
        let exact = crate::Limits { max_vertices: 24, ..l };
        assert!(crate::unwrap_uvs(&corners, &faces, 4.0, 256, &exact).is_ok());
        let under = crate::Limits { max_vertices: 23, ..l };
        assert!(matches!(crate::unwrap_uvs(&corners, &faces, 4.0, 256, &under), Err(MeshOpsError::TooLarge { .. })));
    }

    #[test]
    fn weld_ids_survive_extreme_and_non_finite_positions() {
        let p = [Vec3::new(-3e38, 0.0, 0.0), Vec3::new(3e38, 0.0, 0.0), Vec3::new(3e38, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)];
        assert_eq!(weld_ids(&p), vec![0, 1, 1, 2], "far-apart points stay apart and equal points weld");
        let q = [Vec3::ZERO, Vec3::X, Vec3::new(f32::NAN, 0.0, 0.0), Vec3::X];
        let ids = weld_ids(&q);
        assert_eq!((ids[0], ids[1], ids[3]), (0, 1, 1));
        assert!(crate::weld(&[[0.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0]], &crate::Limits::default()).is_err());
        // The grid follows a diagonal far below 1e-6, and a far-away point welds with its copy.
        for s in [1e-20f32, 1e-30, 1e-40] {
            let tiny = [Vec3::ZERO, Vec3::new(s, 0.0, 0.0), Vec3::new(s, 0.0, 0.0), Vec3::new(2.0 * s, 0.0, 0.0)];
            assert_eq!(weld_ids(&tiny), vec![0, 1, 1, 2], "scale {s}");
        }
        assert_eq!(weld_ids(&[Vec3::splat(3e38); 2]), vec![0, 0]);
    }

    #[test]
    fn unwrap_gives_the_same_layout_at_extreme_scales() {
        let cube = primitives::cube(2.0);
        let l = crate::Limits::default();
        let at = |s: f32| -> Vec<[f32; 3]> { cube.positions.iter().map(|p| (*p * s).to_array()).collect() };
        let unit = crate::unwrap_uvs(&at(1.0), &cube.indices, 4.0, 256, &l).unwrap();
        assert!(unit.charts >= 6);
        // Powers of two scale and divide back without rounding.
        for exponent in [126, 100, -100] {
            let scaled = crate::unwrap_uvs(&at(2f32.powi(exponent)), &cube.indices, 4.0, 256, &l).unwrap();
            assert_eq!(scaled, unit, "scale 2^{exponent}");
        }
    }
}
