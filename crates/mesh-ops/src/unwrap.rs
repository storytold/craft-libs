//! Automatic UV unwrapping for meshes imported without texture coordinates.
//!
//! Triangles are grouped into charts by flood fill across shared (welded) edges while each face
//! normal stays within 60° of the chart's seed face. Charts are rotated to their principal axes
//! and shelf-packed per material into 0..1 with one uniform scale. Texel density is the same
//! everywhere within a material.

use std::collections::{HashMap, VecDeque};

use glam::{DVec2, DVec3, Vec2, Vec3};

use crate::{GeomError, Mesh, Result};

/// Vertices an unwrap accepts, and the most it produces.
const MAX_UNWRAP_VERTICES: usize = 20_000_000;
/// Triangles an unwrap accepts, counting a triangle once for each submesh that lists it.
const MAX_UNWRAP_TRIANGLES: usize = 4_000_000;

/// Faces of a chart stay within this angle of the seed face normal (cos 60°).
const CHART_COS: f32 = 0.5;

/// Welded vertex id per vertex: vertices at the same position (snapped to a grid of 1e-6 of
/// the bounding diagonal) share an id. Ids are dense, in order of first appearance.
pub fn weld_ids(positions: &[Vec3]) -> Vec<u32> {
    let (lo, hi) =
        positions.iter().map(|p| p.as_dvec3()).fold((DVec3::splat(f64::MAX), DVec3::splat(f64::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p)));
    let diagonal = (hi - lo).length();
    let cell = if diagonal > 0.0 && diagonal.is_finite() { diagonal * 1e-6 } else { 1.0 };
    let mut map: HashMap<[i64; 3], u32> = HashMap::with_capacity(positions.len());
    positions
        .iter()
        .map(|p| {
            let q = ((p.as_dvec3() - lo) / cell).round();
            let next = map.len() as u32;
            *map.entry([q.x as i64, q.y as i64, q.z as i64]).or_insert(next)
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

fn face_has_area(positions: [Vec3; 3]) -> bool {
    (0..3).any(|axis| {
        let next = (axis + 1) % 3;
        let [a, b, c] = positions.map(|position| [f64::from(position[axis]), f64::from(position[next])]);
        let products = [a[0] * b[1], b[0] * c[1], c[0] * a[1], -a[1] * b[0], -b[1] * c[0], -c[1] * a[0]];
        let (mut expansion, mut len) = ([0.0; 6], 0);
        for mut value in products {
            let mut kept = 0;
            for index in 0..len {
                let part = expansion[index];
                let sum = value + part;
                let part_virtual = sum - value;
                let value_virtual = sum - part_virtual;
                let error = (value - value_virtual) + (part - part_virtual);
                if error != 0.0 {
                    expansion[kept] = error;
                    kept += 1;
                }
                value = sum;
            }
            expansion[kept] = value;
            len = kept + 1;
        }
        expansion[..len].iter().any(|part| *part != 0.0)
    })
}

struct Chart {
    material: usize,
    /// The seed face's first corner. Projections are taken relative to it, in `f64`, so they
    /// keep the chart's own detail wherever the chart lies.
    origin: DVec3,
    /// Projection basis (right-handed with the seed normal).
    t: DVec3,
    b: DVec3,
    /// Rotation to the chart's principal axes (cos, sin).
    rot: DVec2,
    /// Bounds of the rotated chart. Charts are measured and packed in `f64`, where the extent of
    /// any chart of finite `f32` positions and the scale that packs it neither overflow nor
    /// round to zero.
    lo: DVec2,
    hi: DVec2,
    /// Texel offset of the chart's packed rectangle.
    offset: DVec2,
}

impl Chart {
    fn local(&self, p: Vec3) -> DVec2 {
        let d = p.as_dvec3() - self.origin;
        let q = DVec2::new(d.dot(self.t), d.dot(self.b));
        DVec2::new(q.x * self.rot.x - q.y * self.rot.y, q.x * self.rot.y + q.y * self.rot.x)
    }
    fn size(&self) -> DVec2 {
        (self.hi - self.lo).max(DVec2::ZERO)
    }
}

/// Area-weighted smooth normals over the index buffer, as [`Mesh::compute_normals`] computes
/// them, returned instead of stored.
fn smooth_normals(mesh: &Mesh) -> Vec<Vec3> {
    let mut normals = vec![DVec3::ZERO; mesh.positions.len()];
    for &[a, b, c] in mesh.indices.as_chunks::<3>().0 {
        let (a, b, c) = (a as usize, b as usize, c as usize);
        let (Some(pa), Some(pb), Some(pc)) = (mesh.positions.get(a), mesh.positions.get(b), mesh.positions.get(c)) else { continue };
        let face = (pb.as_dvec3() - pa.as_dvec3()).cross(pc.as_dvec3() - pa.as_dvec3());
        for v in [a, b, c] {
            if let Some(x) = normals.get_mut(v) {
                *x += face;
            }
        }
    }
    normals.into_iter().map(|v| v.try_normalize().map(|normal| normal.as_vec3()).unwrap_or(Vec3::Y)).collect()
}

/// The checks [`auto_unwrap`] makes before it changes anything. The size limits come first.
fn check_input(mesh: &Mesh) -> Result<()> {
    if mesh.positions.len() > MAX_UNWRAP_VERTICES {
        return Err(GeomError::Invalid(format!("mesh has {} vertices; unwrapping accepts at most {MAX_UNWRAP_VERTICES}", mesh.positions.len())));
    }
    if mesh.triangle_count() > MAX_UNWRAP_TRIANGLES {
        return Err(GeomError::Invalid(format!("mesh has {} triangles; unwrapping accepts at most {MAX_UNWRAP_TRIANGLES}", mesh.triangle_count())));
    }
    let listed = mesh.submeshes.iter().fold(0u64, |sum, s| sum.saturating_add(u64::from(s.count / 3)));
    if listed > MAX_UNWRAP_TRIANGLES as u64 {
        return Err(GeomError::Invalid(format!("submeshes list {listed} triangles; unwrapping accepts at most {MAX_UNWRAP_TRIANGLES}")));
    }
    mesh.validate_indices()?;
    for s in &mesh.submeshes {
        if !s.start.is_multiple_of(3) || !s.count.is_multiple_of(3) {
            return Err(GeomError::Invalid(format!("submesh {} is not triangle-aligned", s.name)));
        }
        if u64::from(s.start) + u64::from(s.count) > mesh.indices.len() as u64 || s.material >= mesh.materials.len().max(1) {
            return Err(GeomError::Invalid(format!("submesh {} is out of range", s.name)));
        }
    }
    if mesh.positions.iter().any(|p| !p.is_finite()) {
        return Err(GeomError::Invalid("mesh has non-finite positions".into()));
    }
    if mesh.normals.len() == mesh.positions.len() && mesh.normals.iter().any(|v| !v.is_finite()) {
        return Err(GeomError::Invalid("mesh has non-finite normals".into()));
    }
    if !mesh.colors.is_empty() && (mesh.colors.len() != mesh.positions.len() || mesh.colors.iter().any(|c| !c.is_finite())) {
        return Err(GeomError::Invalid("mesh colors are not one finite color per vertex".into()));
    }
    if all_triangles_degenerate(mesh) {
        return Err(GeomError::Invalid("every triangle of the mesh is degenerate".into()));
    }
    if !mesh.indices.is_empty()
        && mesh.indices.as_chunks::<3>().0.iter().all(|vertices| !face_has_area(vertices.map(|vertex| mesh.positions[vertex as usize])))
    {
        return Err(GeomError::Invalid("every triangle of the mesh is degenerate".into()));
    }
    Ok(())
}

/// The material of every triangle. Each material is packed into its own 0..1 square, so a
/// triangle that submeshes of two materials list would be laid out over another chart of one of
/// them, and is an error. Every triangle must be listed by a submesh.
fn face_materials(mesh: &Mesh) -> Result<Vec<usize>> {
    let n = mesh.triangle_count();
    let mut material = vec![usize::MAX; n];
    for s in &mesh.submeshes {
        let end = (u64::from(s.start) + u64::from(s.count)) / 3;
        for t in (s.start / 3) as usize..(end as usize).min(n) {
            if let Some(m) = material.get_mut(t) {
                if *m != usize::MAX && *m != s.material {
                    return Err(GeomError::Invalid(format!("triangle {t} is in submeshes of materials {} and {}", *m, s.material)));
                }
                *m = s.material;
            }
        }
    }
    if material.contains(&usize::MAX) {
        return Err(GeomError::Invalid("submeshes do not cover every triangle".into()));
    }
    Ok(material)
}

/// Replace the UVs of `mesh` with an automatic non-overlapping layout: each material gets its
/// own 0..1 space with charts at least `padding_texels` (minimum 1, which a NaN counts as) away
/// from each other and the borders at `resolution`. Vertices are split at chart borders;
/// tangents are recomputed. Returns the number of charts.
/// The effective resolution is at least 16. A layout that cannot fit with the requested
/// padding returns an error without changing the mesh.
///
/// A mesh that fails a check is returned unchanged: indices, triangle-aligned submesh ranges
/// that cover every triangle, materials, no
/// triangle listed by submeshes of two materials, finite positions, finite normals and colors
/// when it has them, at least one triangle that is not degenerate, and the size limits (20
/// million vertices; 4 million triangles, counting a triangle once for each submesh that lists
/// it). The limits are checked before any other work. More than 20 million new vertices is an
/// error as well.
pub fn auto_unwrap(mesh: &mut Mesh, padding_texels: f32, resolution: u32) -> Result<usize> {
    check_input(mesh)?;
    // Kept apart from the mesh until the layout is packed, so an error leaves the mesh as it was.
    let computed_normals = if mesh.normals.len() != mesh.positions.len() { Some(smooth_normals(mesh)) } else { None };
    let old_normals = computed_normals.as_deref().unwrap_or(&mesh.normals);
    let n = mesh.triangle_count();
    let weld = weld_ids(&mesh.positions);
    let pos = |v: usize| mesh.positions.get(v).copied().unwrap_or_default();
    let wid = |v: usize| weld.get(v).copied().unwrap_or(u32::MAX);

    // Face normals (None for degenerate faces) and materials.
    let mut normal: Vec<Option<Vec3>> = Vec::with_capacity(n);
    let mut area = Vec::with_capacity(n);
    for t in 0..n {
        let [a, b, c] = mesh.tri(t).unwrap_or([0; 3]).map(|v| pos(v).as_dvec3());
        let cr = (b - a).cross(c - a);
        area.push(cr.length() * 0.5);
        normal.push(cr.try_normalize().map(|n| n.as_vec3()));
    }
    let material = face_materials(mesh)?;

    // Manifold edge adjacency over welded vertices: edge → (face, face, count).
    let mut edges: HashMap<(u32, u32), (u32, u32, u8)> = HashMap::with_capacity(n * 2);
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
        let b = sn.cross(t);
        // Relative to the seed's first corner, so the rounding of a projection follows the
        // chart's size rather than its distance from the origin.
        let origin = pos(mesh.tri(seed).unwrap_or([0; 3])[0]).as_dvec3();
        charts.push(Chart {
            material: material[seed],
            origin,
            t: t.as_dvec3(),
            b: b.as_dvec3(),
            rot: DVec2::X,
            lo: DVec2::ZERO,
            hi: DVec2::ZERO,
            offset: DVec2::ZERO,
        });
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

    // Principal axes and bounds of every chart, in f64 from the positions as given.
    let mut cov = vec![(DVec2::ZERO, [0.0f64; 3], 0usize); charts.len()];
    for (t, &c) in chart_of.iter().enumerate() {
        let c = c as usize;
        for v in mesh.tri(t).unwrap_or([0; 3]) {
            if let (Some(ch), Some(acc)) = (charts.get(c), cov.get_mut(c)) {
                let q = ch.local(pos(v));
                acc.0 += q;
                acc.1[0] += q.x * q.x;
                acc.1[1] += q.y * q.y;
                acc.1[2] += q.x * q.y;
                acc.2 += 1;
            }
        }
    }
    for (ch, (sum, m, k)) in charts.iter_mut().zip(&cov) {
        let k = (*k).max(1) as f64;
        let (mx, my) = (sum.x / k, sum.y / k);
        let (cxx, cyy, cxy) = (m[0] / k - mx * mx, m[1] / k - my * my, m[2] / k - mx * my);
        let angle = 0.5 * (2.0 * cxy).atan2(cxx - cyy);
        // Rotate so the principal axis lies along x.
        ch.rot = DVec2::new((-angle).cos(), (-angle).sin());
        ch.lo = DVec2::splat(f64::MAX);
        ch.hi = DVec2::splat(f64::MIN);
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
            ch.rot = DVec2::new(ch.rot.y, -ch.rot.x);
            let (lo, hi) = (ch.lo, ch.hi);
            ch.lo = DVec2::new(lo.y, -hi.x);
            ch.hi = DVec2::new(hi.y, -lo.x);
        }
    }

    // Pack each material's charts into its own 0..1 square.
    let pad = padding_texels.max(1.0);
    let mut scale = HashMap::new();
    let mut by_material: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for (id, chart) in charts.iter().enumerate() {
        by_material.entry(chart.material).or_default().push(id);
    }
    for (m, ids) in by_material {
        let sizes: Vec<DVec2> = ids.iter().map(|i| charts[*i].size()).collect();
        let (s, res, offsets) = pack(&sizes, resolution.max(16) as f32, pad)?;
        for (i, o) in ids.iter().zip(offsets) {
            charts[*i].offset = o;
        }
        scale.insert(m, (s, res));
    }

    // Rebuild vertices: one per (vertex, chart).
    let mut remap: HashMap<(u32, u32), u32> = HashMap::with_capacity(mesh.positions.len().saturating_mul(2).min(mesh.indices.len()));
    let (mut positions, mut normals, mut uvs, mut colors) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut indices = Vec::with_capacity(mesh.indices.len());
    for (t, &c) in chart_of.iter().enumerate() {
        let Some(ch) = charts.get(c as usize) else { continue };
        let (s, res) = scale.get(&ch.material).copied().unwrap_or((0.0, 1.0f32));
        for v in mesh.tri(t).unwrap_or([0; 3]) {
            let idx = *remap.entry((v as u32, c)).or_insert_with(|| {
                let q = ch.local(pos(v));
                // Chart y points up; UV v points down the image.
                let texel = ch.offset + DVec2::splat(f64::from(pad)) + DVec2::new(q.x - ch.lo.x, ch.hi.y - q.y) * s;
                positions.push(pos(v));
                normals.push(old_normals.get(v).copied().unwrap_or(Vec3::Y));
                if let Some(c) = mesh.colors.get(v) {
                    colors.push(*c);
                }
                uvs.push((texel / f64::from(res)).as_vec2().clamp(Vec2::ZERO, Vec2::ONE));
                (positions.len() - 1) as u32
            });
            indices.push(idx);
        }
        if positions.len() > MAX_UNWRAP_VERTICES {
            return Err(GeomError::Invalid(format!("unwrapping would produce more than {MAX_UNWRAP_VERTICES} vertices")));
        }
    }
    let charts = charts.len();
    // The new arrays replace the mesh's only once the tangents and the final check have passed.
    let old = (
        std::mem::replace(&mut mesh.positions, positions),
        std::mem::replace(&mut mesh.normals, normals),
        std::mem::replace(&mut mesh.uvs, uvs),
        std::mem::replace(&mut mesh.colors, colors),
        std::mem::replace(&mut mesh.indices, indices),
        std::mem::take(&mut mesh.tangents),
    );
    mesh.compute_tangents();
    if let Err(e) = mesh.validate() {
        (mesh.positions, mesh.normals, mesh.uvs, mesh.colors, mesh.indices, mesh.tangents) = old;
        return Err(e);
    }
    Ok(charts)
}

/// Shelf-pack rectangles of world `sizes` (each grown by `pad` texels per side) into a square of
/// `res` texels with the largest uniform scale that fits. Returns (texels per world unit,
/// resolution used, texel offset per rectangle). A layout without room for a positive scale
/// returns an error. The sizes and the scale are in
/// `f64`, where the scale of any chart of finite `f32` positions is finite and above zero.
fn pack(sizes: &[DVec2], res: f32, pad: f32) -> Result<(f64, f32, Vec<DVec2>)> {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by(|a, b| sizes[*b].y.total_cmp(&sizes[*a].y));
    let mut out = vec![DVec2::ZERO; sizes.len()];
    let pad = f64::from(pad);
    // Shelf-place every rectangle at scale `s` into `out`; false when they do not fit.
    let place = |s: f64, res: f32, out: &mut [DVec2]| -> bool {
        let res = f64::from(res);
        let (mut x, mut y, mut shelf) = (0.0f64, 0.0f64, 0.0f64);
        for &i in &order {
            let r = sizes[i] * s + DVec2::splat(2.0 * pad);
            if r.x > res {
                return false;
            }
            if x + r.x > res {
                y += shelf;
                x = 0.0;
                shelf = 0.0;
            }
            if y + r.y > res {
                return false;
            }
            if let Some(o) = out.get_mut(i) {
                *o = DVec2::new(x, y);
            }
            x += r.x;
            shelf = shelf.max(r.y);
        }
        true
    };
    let largest = sizes.iter().map(|s| s.max_element()).fold(0.0f64, f64::max);
    if place(0.0, res, &mut out) {
        if largest <= 0.0 {
            return Ok((0.0, res, out));
        }
        let (mut lo, mut hi) = (0.0f64, (f64::from(res) - 2.0 * pad).max(0.0) / largest);
        // Halve the upper bound until a positive scale fits, so the refinement below starts
        // from a feasible lower bound however far below the first guess the answer lies.
        while hi > 0.0 {
            if place(hi, res, &mut out) {
                lo = hi;
                break;
            }
            hi *= 0.5;
        }
        hi = if lo > 0.0 { (2.0 * lo).min((f64::from(res) - 2.0 * pad).max(0.0) / largest) } else { hi };
        for _ in 0..40 {
            if hi - lo <= lo * 1e-4 {
                break;
            }
            let mid = 0.5 * (lo + hi);
            if place(mid, res, &mut out) { lo = mid } else { hi = mid }
        }
        if lo > 0.0 && place(lo, res, &mut out) {
            return Ok((lo, res, out));
        }
    }
    Err(GeomError::Invalid("UV charts cannot fit with the requested padding and resolution".into()))
}

impl Mesh {
    /// Index-range checks only (attribute arrays may be incomplete before unwrapping).
    fn validate_indices(&self) -> Result<()> {
        let n = self.positions.len();
        if !self.indices.len().is_multiple_of(3) || self.indices.iter().any(|i| *i as usize >= n) {
            return Err(GeomError::Invalid("mesh indices are out of range".into()));
        }
        if self.triangle_count() > crate::MAX_TRIANGLES {
            return Err(GeomError::Invalid(format!("mesh has more than {} triangles", crate::MAX_TRIANGLES)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tiny_quads_keep_their_shared_chart_edge() {
        let mut mesh = quad();
        for position in &mut mesh.positions {
            *position *= 2.0_f32.powi(-42);
        }
        assert_eq!(auto_unwrap(&mut mesh, 4.0, 16).unwrap(), 1);
        assert_eq!(weld_ids(&[Vec3::ONE; 4]), vec![0; 4]);
    }

    #[test]
    fn nonempty_all_collinear_meshes_are_rejected_unchanged() {
        for positions in [vec![Vec3::ZERO, Vec3::X, Vec3::X * 2.0], {
            let d = Vec3::new(1.0, 17.0, 0.0);
            vec![Vec3::ZERO, d * (3.0 * 2.0_f32.powi(-49)), d]
        }] {
            let mut mesh = Mesh::from_parts(positions, Some(vec![Vec3::Z; 3]), Vec::new(), vec![0, 1, 2], "m").unwrap();
            let before = mesh.clone();
            assert!(auto_unwrap(&mut mesh, 4.0, 256).is_err());
            assert_eq!(mesh, before);
        }
    }

    #[test]
    fn missing_normals_keep_tiny_and_large_face_directions() {
        for scale in [1e-25, 1e25] {
            let mesh = Mesh { positions: vec![Vec3::ZERO, Vec3::Y * scale, Vec3::Z * scale], indices: vec![0, 1, 2], ..Mesh::default() };
            assert_eq!(smooth_normals(&mesh), vec![Vec3::X; 3], "scale={scale}");
        }
    }

    #[test]
    fn incomplete_or_unaligned_submeshes_leave_the_layout_unchanged() {
        let original = primitives::cube(1.0);
        for variant in 0..3 {
            let mut mesh = original.clone();
            match variant {
                0 => mesh.submeshes.clear(),
                1 => mesh.submeshes[0].count -= 3,
                _ => {
                    mesh.submeshes[0].start = 1;
                    mesh.submeshes[0].count = 3;
                }
            }
            let before = mesh.clone();
            assert!(auto_unwrap(&mut mesh, 4.0, 256).is_err(), "variant {variant}");
            assert_eq!(mesh, before);
        }
    }

    #[test]
    fn packing_preserves_the_requested_resolution_and_border() {
        assert!(pack(&[DVec2::ONE; 2000], 128.0, 4.0).is_err());
        assert!(pack(&[DVec2::ONE], 16.0, 8.0).is_err());
        let (_, resolution, offsets) = pack(&[DVec2::ONE; 2], 128.0, 4.0).unwrap();
        assert_eq!(resolution, 128.0);
        assert_eq!((offsets[0].x + 4.0) / f64::from(resolution) * 128.0, 4.0);
    }

    #[test]
    fn packing_finds_a_small_scale_far_below_the_first_guess() {
        let pad = 1.0 + 262_657.0 / 8_388_608.0;
        let (scale, _, _) = pack(&vec![DVec2::ONE; 511 * 511], 1054.0, pad as f32).unwrap();
        assert!(scale > 0.0);
        assert!(pack(&vec![DVec2::ONE; 600 * 600], 1054.0, pad as f32).is_err());
    }

    #[test]
    fn weld_grid_keeps_distinct_extreme_positions() {
        let positions = [Vec3::splat(-f32::MAX), Vec3::ZERO, Vec3::splat(f32::MAX)];
        assert_eq!(weld_ids(&positions), vec![0, 1, 2]);
    }

    use glam::Vec4;

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
    fn many_tiny_charts_require_room_at_the_requested_resolution() {
        let mut p = Vec::new();
        let mut idx = Vec::new();
        for i in 0..2000u32 {
            let o = Vec3::new(i as f32 * 3.0, 0.0, 0.0);
            p.extend([o, o + Vec3::X, o + Vec3::Y]);
            idx.extend([i * 3, i * 3 + 1, i * 3 + 2]);
        }
        let mut m = Mesh::from_parts(p, None, Vec::new(), idx, "m").unwrap();
        let before = m.clone();
        assert!(auto_unwrap(&mut m, 4.0, 128).is_err());
        assert_eq!(m, before);
        auto_unwrap(&mut m, 4.0, 512).unwrap();
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

    fn quad() -> Mesh {
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y];
        Mesh::from_parts(p, None, Vec::new(), vec![0, 1, 2, 0, 2, 3], "m").unwrap()
    }

    fn submesh(material: usize, start: u32, count: u32) -> crate::SubMesh {
        crate::SubMesh { name: "s".into(), material, start, count }
    }

    #[test]
    fn rejected_meshes_are_left_unchanged() {
        let mut two_materials = quad();
        two_materials.materials = vec!["a".into(), "b".into()];
        two_materials.submeshes = vec![submesh(0, 0, 6), submesh(1, 3, 3)];
        let mut nan_normal = quad();
        nan_normal.normals = vec![Vec3::INFINITY; 4];
        let mut short_colors = quad();
        short_colors.colors = vec![Vec4::ONE; 2];
        let mut range = quad();
        range.submeshes = vec![submesh(0, 6, 3)];
        let mut nan_position = quad();
        nan_position.positions[2] = Vec3::INFINITY;
        for (name, mut m) in [
            ("two materials", two_materials),
            ("infinite normal", nan_normal),
            ("short colors", short_colors),
            ("range", range),
            ("infinite position", nan_position),
        ] {
            m.uvs.clear();
            let before = m.clone();
            assert!(auto_unwrap(&mut m, 4.0, 256).is_err(), "{name}");
            assert_eq!(m, before, "{name}");
        }
    }

    #[test]
    fn size_limits_are_checked_before_anything_else() {
        // Indices out of range as well: the limit is the error reported.
        let mut m = quad();
        m.indices = vec![0, 1, 9];
        m.submeshes = vec![submesh(0, 0, u32::MAX)];
        let before = m.clone();
        let message = auto_unwrap(&mut m, 4.0, 256).unwrap_err().to_string();
        assert!(message.contains("accepts at most"), "{message}");
        assert_eq!(m, before);
    }

    #[test]
    fn small_charts_far_from_the_origin_keep_their_shape() {
        // A right triangle with 16-unit legs at 1e8, where an f32 position is 8 units apart.
        let o = Vec3::splat(1e8);
        let p = vec![o, o + Vec3::X * 16.0, o + Vec3::Y * 16.0];
        let mut m = Mesh::from_parts(p, None, Vec::new(), vec![0, 1, 2], "m").unwrap();
        auto_unwrap(&mut m, 4.0, 256).unwrap();
        let (a, b, c) = (m.uvs[m.indices[0] as usize], m.uvs[m.indices[1] as usize], m.uvs[m.indices[2] as usize]);
        let (e1, e2) = (b - a, c - a);
        assert!(e1.length() > 0.1 && (e1.length() - e2.length()).abs() < 1e-3, "legs {e1} and {e2}");
        assert!(e1.dot(e2).abs() < 1e-3, "legs {e1} and {e2} are not at a right angle");
    }

    #[test]
    fn padding_that_fills_the_square_does_not_replace_the_mesh() {
        let mut m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, Vec::new(), vec![0, 1, 2], "m").unwrap();
        let before = m.clone();
        assert!(auto_unwrap(&mut m, 128.0, 256).is_err());
        assert_eq!(m, before);
        auto_unwrap(&mut m, 128.0, 512).unwrap();
        let (a, b, c) = (m.uvs[m.indices[0] as usize], m.uvs[m.indices[1] as usize], m.uvs[m.indices[2] as usize]);
        assert!((b - a).length() > 1e-4 && (c - a).length() > 1e-4, "uvs {a} {b} {c}");
    }
}
