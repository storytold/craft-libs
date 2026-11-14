//! Automatic UV unwrapping for meshes imported without texture coordinates.
//!
//! Triangles are grouped into charts by flood fill across shared (welded) edges while each face
//! normal stays within 60° of the chart's seed face, so every chart is a height field over its
//! seed plane and projects onto it without flipping. Charts are rotated to their principal axes
//! and shelf-packed per material into 0..1 with one uniform scale, so texel density is the same
//! everywhere within a material.

use std::collections::{HashMap, VecDeque};

use glam::{DVec2, Vec2, Vec3};

use crate::{Mesh, MeshOpsError, Result, reserved, try_filled, try_vec};

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
/// comes from the finite positions only. `OutOfMemory` when the memory for the ids or the grid
/// cannot be reserved.
pub fn weld_ids(positions: &[Vec3]) -> Result<Vec<u32>> {
    let (lo, hi) = positions
        .iter()
        .filter(|p| p.is_finite())
        .fold(None, |b: Option<(Vec3, Vec3)>, p| Some(b.map_or((*p, *p), |(lo, hi)| (lo.min(*p), hi.max(*p)))))
        .unwrap_or((Vec3::ZERO, Vec3::ZERO));
    let plain = lo.abs().max(hi.abs()).max_element() <= PLAIN_WELD_MAGNITUDE && (lo == hi || (hi - lo).length() >= PLAIN_WELD_DIAGONAL);
    let cell = ((hi - lo).length() * 1e-6).max(1e-12);
    let (wide_lo, wide_cell) = (lo.as_dvec3(), (hi.as_dvec3() - lo.as_dvec3()).length() * 1e-6);
    let mut map: HashMap<[i64; 3], u32> = HashMap::new();
    reserved(map.try_reserve(positions.len()), "welded vertices")?;
    let mut ids = try_vec(positions.len(), "welded vertices")?;
    ids.extend(positions.iter().map(|p| {
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
    }));
    Ok(ids)
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

/// A face whose projected bounds cover more grid cells than this is compared with every face of
/// the chart instead of being entered in the grid.
const MAX_GRID_CELLS: i64 = 64;
/// Face pairs the overlap test may compare: this many per input face, plus [`OVERLAP_PAIRS_BASE`].
/// Once they are used up, a face joins no chart but its own, which keeps the layout free of
/// overlaps at the cost of more charts.
const OVERLAP_PAIRS_PER_FACE: usize = 256;
const OVERLAP_PAIRS_BASE: usize = 1 << 20;

/// A face projected onto its chart's plane, in `f64`: corners that are equal in `f32` project to
/// equal points, and the rounding of a projection stays far below the spacing of `f32` input.
type Flat = [DVec2; 3];

/// The overlap test of the chart being grown: a grid of square cells, each listing the faces
/// whose projected bounds touch it. The cell side is the median longest edge of the mesh's
/// faces.
struct Overlap {
    cell: f64,
    grid: HashMap<(i64, i64), Vec<u32>>,
    /// The projection of each face, set when the face is added to its chart.
    projected: Vec<Flat>,
    /// Faces of the current chart, and those of them that are not in the grid.
    members: Vec<u32>,
    large: Vec<u32>,
    pairs_left: usize,
}

impl Overlap {
    /// The test for a mesh of `n` faces whose corners `corners` gives.
    fn new(n: usize, corners: impl Fn(usize) -> [Vec3; 3]) -> Result<Overlap> {
        let mut longest = try_vec(n, "UV charts")?;
        for t in 0..n {
            let [a, b, c] = corners(t);
            let l = (b - a).length().max((c - b).length()).max((a - c).length());
            if l > 0.0 && l.is_finite() {
                longest.push(l);
            }
        }
        let middle = longest.len() / 2;
        let cell = if longest.is_empty() { 1.0 } else { f64::from(*longest.select_nth_unstable_by(middle, f32::total_cmp).1) };
        Ok(Overlap {
            cell,
            grid: HashMap::new(),
            projected: try_filled(n, [DVec2::ZERO; 3], "UV charts")?,
            members: Vec::new(),
            large: Vec::new(),
            pairs_left: OVERLAP_PAIRS_BASE.saturating_add(n.saturating_mul(OVERLAP_PAIRS_PER_FACE)),
        })
    }

    /// Empty the grid for the next chart. A large grid is dropped rather than cleared, so that
    /// many small charts after one large one do not each pay for its capacity.
    fn start_chart(&mut self) {
        if self.grid.capacity() > 4096 {
            self.grid = HashMap::new();
        } else {
            self.grid.clear();
        }
        self.members.clear();
        self.large.clear();
    }

    /// Whether the projected face has an interior. One without area cannot cover a texel that
    /// another face covers.
    fn has_area(tri: &Flat) -> bool {
        height(tri) > 0.0
    }

    /// The grid cells the projected bounds cover: first and last column and row.
    fn cells(&self, tri: &Flat) -> [i64; 4] {
        let (lo, hi) = (tri[0].min(tri[1]).min(tri[2]), tri[0].max(tri[1]).max(tri[2]));
        // `as` saturates, so far-away bounds give the extreme cells.
        [(lo.x / self.cell).floor() as i64, (lo.y / self.cell).floor() as i64, (hi.x / self.cell).floor() as i64, (hi.y / self.cell).floor() as i64]
    }

    fn cell_count([x0, y0, x1, y1]: [i64; 4]) -> i64 {
        x1.saturating_sub(x0).saturating_add(1).saturating_mul(y1.saturating_sub(y0).saturating_add(1))
    }

    /// Whether a face with this projection can join the current chart: its interior overlaps
    /// none of the chart's faces. False when the comparisons allowed are used up.
    fn fits(&mut self, tri: &Flat) -> bool {
        if !Self::has_area(tri) {
            return true;
        }
        let cells = self.cells(tri);
        let (projected, pairs_left) = (&self.projected, &mut self.pairs_left);
        let mut clear = |ids: &[u32]| {
            ids.iter().all(|&id| {
                if *pairs_left == 0 {
                    return false;
                }
                *pairs_left -= 1;
                projected.get(id as usize).is_none_or(|other| !interiors_overlap(tri, other))
            })
        };
        if Self::cell_count(cells) > MAX_GRID_CELLS {
            return clear(&self.members);
        }
        let [x0, y0, x1, y1] = cells;
        if !clear(&self.large) {
            return false;
        }
        for x in x0..=x1 {
            for y in y0..=y1 {
                if let Some(ids) = self.grid.get(&(x, y))
                    && !clear(ids)
                {
                    return false;
                }
            }
        }
        true
    }

    /// Add face `face` with this projection to the current chart.
    fn add(&mut self, face: usize, tri: Flat) -> Result<()> {
        if let Some(slot) = self.projected.get_mut(face) {
            *slot = tri;
        }
        if !Self::has_area(&tri) {
            return Ok(());
        }
        let id = face as u32;
        reserved(self.members.try_reserve(1), "UV charts")?;
        self.members.push(id);
        let cells = self.cells(&tri);
        if Self::cell_count(cells) > MAX_GRID_CELLS {
            reserved(self.large.try_reserve(1), "UV charts")?;
            self.large.push(id);
            return Ok(());
        }
        let [x0, y0, x1, y1] = cells;
        for x in x0..=x1 {
            for y in y0..=y1 {
                if !self.grid.contains_key(&(x, y)) {
                    reserved(self.grid.try_reserve(1), "UV charts")?;
                }
                let ids = self.grid.entry((x, y)).or_default();
                reserved(ids.try_reserve(1), "UV charts")?;
                ids.push(id);
            }
        }
        Ok(())
    }
}

/// The smallest height of a triangle: twice its area over its longest edge. 0 for a triangle
/// without area.
fn height(t: &Flat) -> f64 {
    let longest = (t[1] - t[0]).length().max((t[2] - t[1]).length()).max((t[0] - t[2]).length());
    let twice_area = (t[1] - t[0]).perp_dot(t[2] - t[0]).abs();
    if longest > 0.0 { twice_area / longest } else { 0.0 }
}

/// Whether the interiors of two triangles in a plane overlap: no edge direction of either
/// separates them (separating axis test). Projections that overlap by no more than a tolerance
/// only touch, as faces that share an edge or a corner do: 1e-5 of the smaller of the two
/// triangles' heights, so a thin triangle's own width counts, plus the `f64` rounding of
/// coordinates of their magnitude.
fn interiors_overlap(a: &Flat, b: &Flat) -> bool {
    let magnitude = a.iter().chain(b).map(|p| p.abs().max_element()).fold(0.0f64, f64::max);
    let tol = 1e-5 * height(a).min(height(b)) + 8.0 * f64::EPSILON * magnitude;
    let span = |t: &Flat, axis: DVec2| t.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.dot(axis)), hi.max(p.dot(axis))));
    for tri in [a, b] {
        for k in 0..3 {
            let Some(axis) = (tri[(k + 1) % 3] - tri[k]).perp().try_normalize() else { continue };
            let ((a_lo, a_hi), (b_lo, b_hi)) = (span(a, axis), span(b, axis));
            if a_hi <= b_lo + tol || b_hi <= a_lo + tol {
                return false;
            }
        }
    }
    true
}

/// Replace the UVs of `mesh` with an automatic non-overlapping layout: each material gets its
/// own 0..1 space with charts at least `padding_texels` (minimum 1, which a NaN counts as) away
/// from each other and the borders at `resolution`. Vertices are split at chart borders;
/// tangents are recomputed.
/// Returns the number of charts. A mesh that fails the checks (indices, submesh ranges and
/// materials, finite positions, finite normals and colors when it has them, at least one
/// triangle that is not degenerate) is left unchanged, as is one whose layout runs out of
/// memory; running out of memory for the tangents comes after the new layout is in place.
pub fn auto_unwrap(mesh: &mut Mesh, padding_texels: f32, resolution: u32) -> Result<usize> {
    let (charts, _) = unwrap_core(mesh, padding_texels, resolution, usize::MAX)?;
    mesh.try_compute_tangents()?;
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
    let computed_normals = if mesh.normals.len() != mesh.positions.len() { Some(mesh.smooth_normals()?) } else { None };
    let old_normals = computed_normals.as_deref().unwrap_or(&mesh.normals);
    let n = mesh.triangle_count();
    let weld = weld_ids(&mesh.positions)?;
    // The layout depends on the shape only, so the charts are made from positions divided into
    // the range where face normals, areas and projections neither overflow nor round to zero.
    let divisor = crate::mesh::safe_divisor(crate::mesh::largest_abs(mesh.positions.iter().flat_map(|p| p.to_array())));
    let original = |v: usize| mesh.positions.get(v).copied().unwrap_or_default();
    let pos = |v: usize| original(v) / divisor;
    let wid = |v: usize| weld.get(v).copied().unwrap_or(u32::MAX);

    // Face normals (None for degenerate faces) and materials.
    let mut normal: Vec<Option<Vec3>> = try_vec(n, "UV charts")?;
    let mut area = try_vec(n, "UV charts")?;
    for t in 0..n {
        let [a, b, c] = mesh.tri(t).unwrap_or([0; 3]);
        let cr = (pos(b) - pos(a)).cross(pos(c) - pos(a));
        area.push(cr.length() * 0.5);
        normal.push(cr.try_normalize());
    }
    let mut material = try_filled(n, 0usize, "UV charts")?;
    for s in &mesh.submeshes {
        // Checked by validate_indices: whole triangles inside the index buffer.
        let end = (u64::from(s.start) + u64::from(s.count)) / 3;
        for t in (s.start / 3) as usize..(end as usize).min(n) {
            if let Some(m) = material.get_mut(t) {
                *m = s.material;
            }
        }
    }

    // Manifold edge adjacency over welded vertices: edge → (face, face, count). There are at
    // most three edges per face, so the map does not grow past what is reserved here.
    let mut edges: HashMap<(u32, u32), (u32, u32, u8)> = HashMap::new();
    reserved(edges.try_reserve(n.saturating_mul(3)), "UV charts")?;
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

    let faces = Faces { normal: &normal, area: &area, material: &material, edges: &edges };
    let corners = |f: usize| mesh.tri(f).unwrap_or([0; 3]).map(pos);
    let (chart_of, mut charts) = grow_charts(&faces, tri_edges, corners)?;

    // Principal axes and bounds of every chart.
    let mut cov = try_filled(charts.len(), (Vec2::ZERO, [0.0f64; 3], 0usize), "UV charts")?;
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
    let mut mats = try_vec(charts.len(), "UV charts")?;
    mats.extend(charts.iter().map(|c| c.material));
    mats.sort_unstable();
    mats.dedup();
    reserved(scale.try_reserve(mats.len()), "UV charts")?;
    for m in mats {
        let mut ids = try_vec(charts.len(), "UV charts")?;
        ids.extend((0..charts.len()).filter(|i| charts[*i].material == m));
        let mut sizes = try_vec(ids.len(), "UV charts")?;
        sizes.extend(ids.iter().map(|i| charts[*i].size()));
        let (s, res, offsets) = pack(&sizes, resolution.max(16) as f32, pad)?;
        for (i, o) in ids.iter().zip(offsets) {
            charts[*i].offset = o;
        }
        scale.insert(m, (s, res));
    }

    // Rebuild vertices: one per (vertex, chart). Each triangle reserves room for its three
    // corners before adding them.
    let mut remap: HashMap<(u32, u32), u32> = HashMap::new();
    reserved(remap.try_reserve(mesh.positions.len()), "UV charts")?;
    let (mut positions, mut normals, mut uvs, mut colors, mut source) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut indices = try_vec(mesh.indices.len(), "indices")?;
    for (t, &c) in chart_of.iter().enumerate() {
        let Some(ch) = charts.get(c as usize) else { continue };
        let (s, res) = scale.get(&ch.material).copied().unwrap_or((0.0, 1.0));
        let reserve = |r| reserved(r, "UV charts");
        reserve(remap.try_reserve(3))?;
        reserve(positions.try_reserve(3))?;
        reserve(normals.try_reserve(3))?;
        reserve(uvs.try_reserve(3))?;
        reserve(if mesh.colors.is_empty() { Ok(()) } else { colors.try_reserve(3) })?;
        reserve(source.try_reserve(3))?;
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

/// What chart growing reads per face: its normal (None when degenerate), its area and its
/// material, and the faces on each manifold edge (edge → face, face, count).
struct Faces<'a> {
    normal: &'a [Option<Vec3>],
    area: &'a [f32],
    material: &'a [usize],
    edges: &'a HashMap<(u32, u32), (u32, u32, u8)>,
}

/// Group the faces into charts by flood fill from the largest unassigned face, across the
/// welded edges `tri_edges` gives, while each face normal stays within 60° of the seed's and the
/// face's projection onto the seed plane overlaps no face already in the chart. `corners` gives
/// a face's divided positions. Returns each face's chart and the charts.
fn grow_charts(faces: &Faces, tri_edges: impl Fn(usize) -> [(u32, u32); 3], corners: impl Fn(usize) -> [Vec3; 3]) -> Result<(Vec<u32>, Vec<Chart>)> {
    let n = faces.area.len();
    let mut order: Vec<usize> = try_vec(n, "UV charts")?;
    order.extend(0..n);
    order.sort_by(|a, b| faces.area[*b].total_cmp(&faces.area[*a]));
    let mut chart_of = try_filled(n, u32::MAX, "UV charts")?;
    let mut charts: Vec<Chart> = Vec::new();
    let mut queue = VecDeque::new();
    let mut overlap = Overlap::new(n, &corners)?;
    for &seed in &order {
        if chart_of[seed] != u32::MAX {
            continue;
        }
        let id = charts.len() as u32;
        let sn = faces.normal[seed].unwrap_or(Vec3::Z);
        let t = sn.any_orthonormal_vector();
        let b = sn.cross(t);
        let material = faces.material[seed];
        reserved(charts.try_reserve(1), "UV charts")?;
        charts.push(Chart { material, t, b, rot: Vec2::X, lo: Vec2::ZERO, hi: Vec2::ZERO, offset: Vec2::ZERO });
        // A face joins only when its projection onto the chart's plane does not overlap the
        // faces already in it: a surface that winds around past a full turn stays within the
        // normal cone and would otherwise cover the same texels twice.
        // Relative to the seed's first corner, so the rounding of a projection follows the
        // chart's size rather than its distance from the origin.
        let (origin, td, bd) = (corners(seed)[0].as_dvec3(), t.as_dvec3(), b.as_dvec3());
        let project = |f: usize| corners(f).map(|p| DVec2::new((p.as_dvec3() - origin).dot(td), (p.as_dvec3() - origin).dot(bd)));
        overlap.start_chart();
        overlap.add(seed, project(seed))?;
        chart_of[seed] = id;
        reserved(queue.try_reserve(1), "UV charts")?;
        queue.push_back(seed);
        while let Some(f) = queue.pop_front() {
            for e in tri_edges(f) {
                let Some(&(f0, f1, 2)) = faces.edges.get(&e) else { continue };
                let g = if f0 as usize == f { f1 } else { f0 } as usize;
                let ok = faces.normal.get(g).is_some_and(|nn| nn.is_none_or(|nn| nn.dot(sn) >= CHART_COS));
                if !(ok && chart_of.get(g) == Some(&u32::MAX) && faces.material.get(g) == Some(&material)) {
                    continue;
                }
                let projected = project(g);
                if overlap.fits(&projected) {
                    overlap.add(g, projected)?;
                    chart_of[g] = id;
                    reserved(queue.try_reserve(1), "UV charts")?;
                    queue.push_back(g);
                }
            }
        }
    }
    Ok((chart_of, charts))
}

/// Shelf-pack rectangles of world `sizes` (each grown by `pad` texels per side) into a square of
/// `res` texels with the largest uniform scale that fits. Returns (texels per world unit,
/// resolution used, texel offset per rectangle). The resolution doubles while the padding leaves
/// no room for a positive scale (very many charts, or padding as wide as the square), so the
/// result always fits in 0..1 and charts with a size keep it.
fn pack(sizes: &[Vec2], mut res: f32, pad: f32) -> Result<(f32, f32, Vec<Vec2>)> {
    let mut order = try_vec(sizes.len(), "UV charts")?;
    order.extend(0..sizes.len());
    order.sort_by(|a, b| sizes[*b].y.total_cmp(&sizes[*a].y));
    let mut out = try_filled(sizes.len(), Vec2::ZERO, "UV charts")?;
    // Shelf-place every rectangle at scale `s` into `out`; false when they do not fit.
    let place = |s: f32, res: f32, out: &mut [Vec2]| -> bool {
        let (mut x, mut y, mut shelf) = (0.0f32, 0.0f32, 0.0f32);
        for &i in &order {
            let r = sizes[i] * s + Vec2::splat(2.0 * pad);
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
                *o = Vec2::new(x, y);
            }
            x += r.x;
            shelf = shelf.max(r.y);
        }
        true
    };
    let largest = sizes.iter().map(|s| s.max_element()).fold(0.0f32, f32::max);
    loop {
        if place(0.0, res, &mut out) {
            if largest <= 0.0 {
                return Ok((0.0, res, out));
            }
            // The largest scale that fits, by bisection; none above zero means a larger square.
            let (mut lo, mut hi) = (0.0f32, (res - 2.0 * pad).max(0.0) / largest);
            if hi > 0.0 && place(hi, res, &mut out) {
                lo = hi;
            }
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
        res *= 2.0;
        if res > MAX_PACK_RES {
            return Err(MeshOpsError::Invalid("too many UV charts to pack".into()));
        }
    }
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
    fn charts_do_not_overlap_where_a_flat_surface_winds_past_a_full_turn() {
        // A flat fan around the origin through 0°, 100°, 200°, 300° and 400°: every face normal is
        // +Z and each face shares an edge with the next, so the faces are one height field, but
        // the first and the last cover the same part of the plane.
        let mut positions = vec![[0.0f32, 0.0, 0.0]];
        for k in 0..5 {
            let a = (k as f32 * 100.0).to_radians();
            positions.push([a.cos(), a.sin(), 0.0]);
        }
        let indices = [0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 5];
        let out = crate::unwrap_uvs(&positions, &indices, 4.0, 256, &crate::Limits::default()).unwrap();
        assert!(out.charts >= 2, "{} charts", out.charts);
        let uvs: Vec<[f32; 2]> = out.uvs.clone();
        let g = UvGBuffer::from_slices(&uvs, &out.indices, 256, 256, &crate::Limits::default()).unwrap();
        assert!(g.covered_count() > 0);
        // Texels on an edge two triangles share count once per triangle; the fan has three such
        // edges, so allow for those and no more.
        let shared = 3 * 256 / 2;
        assert!((g.overlaps as usize) < shared, "{} overlapping texels", g.overlaps);
        // Each face covers its own texels: no face is drawn over another.
        let mut owners = vec![0u32; 4];
        for t in &g.tri {
            if let Some(o) = owners.get_mut(*t as usize) {
                *o += 1;
            }
        }
        assert!(owners.iter().all(|o| *o > 0), "{owners:?}");
    }

    #[test]
    fn small_faces_that_overlap_a_large_one_start_another_chart() {
        // Many unit faces set the grid, then a strip of faces 1e-4 across winds back over the
        // first face: the overlap test measures each pair by its own size.
        let mut positions = vec![[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let mut indices = vec![0u32, 1, 2];
        for k in 0..40u32 {
            let o = 10.0 + 3.0 * k as f32;
            let base = positions.len() as u32;
            positions.extend([[o, 0.0, 0.0], [o + 1.0, 0.0, 0.0], [o, 1.0, 0.0]]);
            indices.extend([base, base + 1, base + 2]);
        }
        // A tiny fan through 0°, 100°, 200°, 300° and 400° around (0.25, 0.25), on the first face.
        let s = 1e-4;
        let center = positions.len() as u32;
        positions.push([0.25, 0.25, 0.0]);
        for k in 0..5 {
            let a = (k as f32 * 100.0).to_radians();
            positions.push([0.25 + s * a.cos(), 0.25 + s * a.sin(), 0.0]);
        }
        for k in 0..4 {
            indices.extend([center, center + 1 + k, center + 2 + k]);
        }
        let out = crate::unwrap_uvs(&positions, &indices, 1.0, 4096, &crate::Limits::default()).unwrap();
        // The four fan faces share edges in a row; the first and the last overlap.
        let fan = (indices.len() / 3 - 4)..indices.len() / 3;
        let uv = |t: usize| [0, 1, 2].map(|k| Vec2::from_array(out.uvs[out.indices[t * 3 + k] as usize]).as_dvec2());
        let (first, last) = (uv(fan.start), uv(fan.end - 1));
        assert!(!interiors_overlap(&first, &last), "{first:?} {last:?}");
    }

    #[test]
    fn thin_faces_that_overlap_start_another_chart() {
        // The fan through 0° to 400°, squashed to 5e-6 and to 1e-7 of its width: its faces are
        // slivers, and the first and the last still overlap.
        for squash in [5e-6f32, 1e-7] {
            let mut positions = vec![[0.0f32, 0.0, 0.0]];
            for k in 0..5 {
                let a = (k as f32 * 100.0).to_radians();
                positions.push([a.cos(), squash * a.sin(), 0.0]);
            }
            let indices = [0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 5];
            let out = crate::unwrap_uvs(&positions, &indices, 1.0, 256, &crate::Limits::default()).unwrap();
            assert!(out.charts >= 2, "squash {squash}: {} charts", out.charts);
        }
    }

    #[test]
    fn faces_that_only_share_edges_and_corners_stay_in_one_chart() {
        // 2 × 64 × 64 triangles of a flat grid touch along edges and at corners only.
        let mut plane = primitives::plane(1.0, 64);
        assert_eq!(auto_unwrap(&mut plane, 4.0, 1024).unwrap(), 1);
    }

    #[test]
    fn a_layout_that_fits_only_at_zero_scale_is_not_accepted() {
        // With 8 texels of padding per side, a 16-texel square has no room left for the chart.
        let out = crate::unwrap_uvs(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], &[0, 1, 2], 8.0, 16, &crate::Limits::default()).unwrap();
        let [a, b, c] = [0, 1, 2].map(|k| Vec2::from_array(out.uvs[out.indices[k] as usize]));
        let area = (b - a).perp_dot(c - a).abs() * 0.5;
        assert!(area > 0.01, "UV area {area}: {:?}", out.uvs);
        assert!(out.uvs.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn weld_ids_survive_extreme_and_non_finite_positions() {
        let p = [Vec3::new(-3e38, 0.0, 0.0), Vec3::new(3e38, 0.0, 0.0), Vec3::new(3e38, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)];
        assert_eq!(weld_ids(&p).unwrap(), vec![0, 1, 1, 2], "far-apart points stay apart and equal points weld");
        let q = [Vec3::ZERO, Vec3::X, Vec3::new(f32::NAN, 0.0, 0.0), Vec3::X];
        let ids = weld_ids(&q).unwrap();
        assert_eq!((ids[0], ids[1], ids[3]), (0, 1, 1));
        assert!(crate::weld(&[[0.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0]], &crate::Limits::default()).is_err());
        // The grid follows a diagonal far below 1e-6, and a far-away point welds with its copy.
        for s in [1e-20f32, 1e-30, 1e-40] {
            let tiny = [Vec3::ZERO, Vec3::new(s, 0.0, 0.0), Vec3::new(s, 0.0, 0.0), Vec3::new(2.0 * s, 0.0, 0.0)];
            assert_eq!(weld_ids(&tiny).unwrap(), vec![0, 1, 1, 2], "scale {s}");
        }
        assert_eq!(weld_ids(&[Vec3::splat(3e38); 2]).unwrap(), vec![0, 0]);
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
