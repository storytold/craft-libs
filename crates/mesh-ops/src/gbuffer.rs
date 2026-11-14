//! The UV-space G-buffer: which triangle (and where on it) each texel of a texture set maps
//! to. Painting, compositing, baking and padding all work from this one buffer, so they agree
//! on which texel is which surface point.

use glam::{Vec2, Vec3, Vec4};
#[cfg(feature = "parallel")]
use rayon::prelude::*;

use crate::mesh::unit;
use crate::{Limits, Mesh, MeshOpsError, Result, check_limit, try_vec};

/// Texel not covered by any triangle.
pub const NO_TRI: u32 = u32::MAX;

const BAND: u32 = 32;

/// A triangle with a corner farther than this many texels from the UV origin is not rasterized:
/// within it, the products of two texel coordinates in the edge functions stay finite.
const MAX_TEXEL_COORDINATE: f32 = 1e18;

#[derive(Clone, Debug, PartialEq)]
pub struct UvGBuffer {
    pub width: u32,
    pub height: u32,
    /// Triangle under each texel center, [`NO_TRI`] when uncovered.
    pub tri: Vec<u32>,
    /// Barycentric weights of the triangle's second and third vertices.
    pub bary: Vec<[f32; 2]>,
    /// Texels written by more than one triangle (overlapping UVs). A texel whose center lies on
    /// an edge two triangles share is written by both and counts; the later triangle keeps it.
    pub overlaps: u32,
}

/// Interpolated surface attributes at a texel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub pos: Vec3,
    /// Interpolated shading normal (normalized).
    pub nrm: Vec3,
    /// Flat triangle normal (for ray offsets).
    pub geo_nrm: Vec3,
    pub tangent: Vec4,
    pub uv: Vec2,
    pub tri: u32,
}

fn edge(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// Vertex indices of triangle `t` of an index buffer.
fn corners(indices: &[u32], t: usize) -> Option<[usize; 3]> {
    let start = t.checked_mul(3)?;
    let i = indices.get(start..start.checked_add(3)?)?;
    Some([i[0] as usize, i[1] as usize, i[2] as usize])
}

/// Texel count of a `width × height` buffer, checked against `limits`. A count whose
/// barycentric array would be over `isize::MAX` bytes, which no allocation can hold, is
/// `OutOfMemory`.
fn texels(width: u32, height: u32, limits: &Limits) -> Result<usize> {
    if width == 0 || height == 0 {
        return Err(MeshOpsError::Invalid(format!("G-buffer size {width} × {height} is empty")));
    }
    let side = limits.max_gbuffer_side as usize;
    check_limit("G-buffer width", width as usize, side)?;
    check_limit("G-buffer height", height as usize, side)?;
    let n = u64::from(width) * u64::from(height);
    if n > limits.max_gbuffer_texels {
        return Err(MeshOpsError::TooLarge { what: "G-buffer texel count", got: n, limit: limits.max_gbuffer_texels });
    }
    let n = usize::try_from(n).map_err(|_| MeshOpsError::TooLarge { what: "G-buffer texel count", got: n, limit: usize::MAX as u64 })?;
    match n.checked_mul(size_of::<[f32; 2]>()) {
        Some(bytes) if bytes <= isize::MAX as usize => Ok(n),
        _ => Err(MeshOpsError::OutOfMemory("G-buffer")),
    }
}

impl UvGBuffer {
    /// Rasterize the triangles of material `material` at `width × height`, each side clamped to
    /// `1..=8192` (the default [`Limits::max_gbuffer_side`]). When the mesh is over another
    /// default [`Limits`] field or the memory for that size cannot be reserved, the buffer is
    /// 1 × 1 and uncovered.
    pub fn new(mesh: &Mesh, material: usize, width: u32, height: u32) -> UvGBuffer {
        let side = Limits::default().max_gbuffer_side;
        let (w, h) = (width.clamp(1, side), height.clamp(1, side));
        UvGBuffer::try_new(mesh, material, w, h, &Limits::default()).unwrap_or_else(|_| UvGBuffer {
            width: 1,
            height: 1,
            tri: vec![NO_TRI],
            bary: vec![[0.0; 2]],
            overlaps: 0,
        })
    }

    /// [`UvGBuffer::new`] with the size checked against `limits` instead of clamped: a zero
    /// side is an error. The mesh's vertex and triangle counts and the rasterizer's work and band
    /// lists are checked before the texel buffers are reserved.
    pub fn try_new(mesh: &Mesh, material: usize, width: u32, height: u32, limits: &Limits) -> Result<UvGBuffer> {
        let n = texels(width, height, limits)?;
        check_limit("vertex count", mesh.positions.len(), limits.max_vertices)?;
        check_limit("triangle count", mesh.triangle_count(), limits.max_triangles)?;
        let bins = Bins::new(&mesh.uvs, &mesh.indices, || mesh.triangles_of(material), (width, height), limits)?;
        UvGBuffer::rasterize(&mesh.uvs, &mesh.indices, &bins, (width, height), n)
    }

    /// Rasterize every triangle of `indices` over `uvs` at `width × height`. The size, the
    /// triangle count, every index and every UV (finite) are checked first, and the
    /// rasterizer's work and band lists before the texel buffers are reserved.
    pub fn from_slices(uvs: &[[f32; 2]], indices: &[u32], width: u32, height: u32, limits: &Limits) -> Result<UvGBuffer> {
        let n = texels(width, height, limits)?;
        check_limit("vertex count", uvs.len(), limits.max_vertices)?;
        if !indices.len().is_multiple_of(3) {
            return Err(MeshOpsError::Invalid("index count is not a multiple of 3".into()));
        }
        check_limit("triangle count", indices.len() / 3, limits.max_triangles)?;
        if let Some(bad) = indices.iter().find(|&&i| i as usize >= uvs.len()) {
            return Err(MeshOpsError::Invalid(format!("index {bad} out of range ({} vertices)", uvs.len())));
        }
        if !uvs.iter().flatten().all(|v| v.is_finite()) {
            return Err(MeshOpsError::Invalid("non-finite UV".into()));
        }
        let mut points = try_vec(uvs.len(), "UVs")?;
        points.extend(uvs.iter().map(|uv| Vec2::from_array(*uv)));
        let bins = Bins::new(&points, indices, || 0..indices.len() / 3, (width, height), limits)?;
        UvGBuffer::rasterize(&points, indices, &bins, (width, height), n)
    }

    /// Reserve the `n` texels of a `width × height` buffer and draw the triangles that `bins`
    /// lists.
    fn rasterize(uvs: &[Vec2], indices: &[u32], bins: &Bins, (width, height): (u32, u32), n: usize) -> Result<UvGBuffer> {
        let (mut tri, mut bary) = (try_vec(n, "G-buffer")?, try_vec(n, "G-buffer")?);
        tri.resize(n, NO_TRI);
        bary.resize(n, [0.0f32; 2]);
        let overlaps = bins.draw(uvs, indices, (width, height), &mut tri, &mut bary);
        Ok(UvGBuffer { width, height, tri, bary, overlaps })
    }

    pub fn covered(&self, i: usize) -> bool {
        self.tri.get(i).is_some_and(|t| *t != NO_TRI)
    }

    pub fn coverage_mask(&self) -> Vec<bool> {
        self.tri.iter().map(|t| *t != NO_TRI).collect()
    }

    pub fn covered_count(&self) -> usize {
        self.tri.iter().filter(|t| **t != NO_TRI).count()
    }

    /// Surface attributes at texel index `i` (None when uncovered).
    pub fn surface(&self, mesh: &Mesh, i: usize) -> Option<Surface> {
        let t = *self.tri.get(i)?;
        if t == NO_TRI {
            return None;
        }
        let [b1, b2] = *self.bary.get(i)?;
        surface_at(mesh, t, b1, b2)
    }
}

/// `total + add`, or `TooLarge` naming `what` when that is over `limit` (`got` is `u64::MAX`
/// when the sum overflows).
fn add_within(what: &'static str, total: u64, add: u64, limit: u64) -> Result<u64> {
    let got = total.saturating_add(add);
    if got > limit { Err(MeshOpsError::TooLarge { what, got, limit }) } else { Ok(got) }
}

/// Triangle `t` in the texels of a `size` buffer: its corners, its signed doubled area and its
/// bounding box clipped to the buffer as `[x0, x1, y0, y1]` (ends exclusive). `None` when a
/// corner is missing, not finite or farther than [`MAX_TEXEL_COORDINATE`], when the triangle
/// has no area or when the clipped box is empty.
fn texel_triangle(uvs: &[Vec2], indices: &[u32], t: usize, size: Vec2) -> Option<([Vec2; 3], f32, [u32; 4])> {
    let [a, b, c] = corners(indices, t)?;
    let p = [*uvs.get(a)? * size, *uvs.get(b)? * size, *uvs.get(c)? * size];
    if !p.iter().all(|q| q.is_finite() && q.abs().max_element() <= MAX_TEXEL_COORDINATE) {
        return None;
    }
    let area = edge(p[0], p[1], p[2]);
    if area.abs() < 1e-12 {
        return None;
    }
    let lo = p[0].min(p[1]).min(p[2]).floor().clamp(Vec2::ZERO, size);
    let hi = p[0].max(p[1]).max(p[2]).ceil().clamp(Vec2::ZERO, size);
    let [x0, x1, y0, y1] = [lo.x as u32, hi.x as u32, lo.y as u32, hi.y as u32];
    (x1 > x0 && y1 > y0).then_some((p, area, [x0, x1, y0, y1]))
}

/// The bands of rows of a G-buffer (each [`BAND`] rows high), each with the triangles whose
/// clipped boxes reach it, in triangle order.
struct Bins {
    /// Band `b`'s triangles are `ids[offsets[b]..offsets[b + 1]]`.
    offsets: Vec<usize>,
    ids: Vec<u32>,
}

impl Bins {
    /// Bin the triangles that `triangles` returns (the same ones on every call) for a `w × h`
    /// buffer. A first pass checks the triangles visited against [`Limits::max_triangles`], the
    /// box texels against [`Limits::max_gbuffer_work`] and the band entries against
    /// [`Limits::max_gbuffer_bin_entries`] and reserves nothing. A second pass counts each band's
    /// entries, so the lists are reserved in one piece before a third pass fills them.
    fn new<I: Iterator<Item = usize>>(uvs: &[Vec2], indices: &[u32], triangles: impl Fn() -> I, (w, h): (u32, u32), limits: &Limits) -> Result<Bins> {
        let size = Vec2::new(w as f32, h as f32);
        let bands = h.div_ceil(BAND) as usize;
        // NO_TRI marks an uncovered texel, so it cannot name a triangle.
        let triangles = || triangles().take_while(|t| *t < NO_TRI as usize);
        let band_range = |y0: u32, y1: u32| (y0 / BAND) as usize..=((y1 - 1) / BAND) as usize;
        let (mut visited, mut work, mut entries) = (0usize, 0u64, 0u64);
        // Every entry count and offset below then fits in a usize.
        let entry_limit = limits.max_gbuffer_bin_entries.min(usize::MAX as u64);
        for t in triangles() {
            visited = visited.saturating_add(1);
            check_limit("triangle count", visited, limits.max_triangles)?;
            let Some((_, _, [x0, x1, y0, y1])) = texel_triangle(uvs, indices, t, size) else { continue };
            work = add_within("G-buffer work", work, u64::from(x1 - x0) * u64::from(y1 - y0), limits.max_gbuffer_work)?;
            let reach = band_range(y0, y1);
            entries = add_within("G-buffer band entries", entries, (reach.end() - reach.start() + 1) as u64, entry_limit)?;
        }
        // offsets[b + 1] counts band b's entries, then becomes the end of its run in `ids`.
        let mut offsets = try_vec(bands + 1, "G-buffer bands")?;
        offsets.resize(bands + 1, 0usize);
        for t in triangles() {
            let Some((_, _, [_, _, y0, y1])) = texel_triangle(uvs, indices, t, size) else { continue };
            for b in band_range(y0, y1) {
                if let Some(n) = offsets.get_mut(b + 1) {
                    *n += 1;
                }
            }
        }
        let mut run = 0;
        for n in &mut offsets {
            run += *n;
            *n = run;
        }
        let entries = usize::try_from(entries).map_err(|_| MeshOpsError::OutOfMemory("G-buffer bands"))?;
        let mut ids = try_vec(entries, "G-buffer bands")?;
        ids.resize(entries, NO_TRI);
        let mut next = try_vec(offsets.len(), "G-buffer bands")?;
        next.extend_from_slice(&offsets);
        for t in triangles() {
            let Some((_, _, [_, _, y0, y1])) = texel_triangle(uvs, indices, t, size) else { continue };
            for b in band_range(y0, y1) {
                if let Some(at) = next.get_mut(b) {
                    if let Some(id) = ids.get_mut(*at) {
                        *id = t as u32;
                    }
                    *at += 1;
                }
            }
        }
        Ok(Bins { offsets, ids })
    }

    /// Write the triangle and barycentrics under each texel center of `w × h` buffers (`tri`
    /// and `bary` hold `w × h` entries) for the binned triangles; returns the overlap count.
    fn draw(&self, uvs: &[Vec2], indices: &[u32], (w, h): (u32, u32), tri: &mut [u32], bary: &mut [[f32; 2]]) -> u32 {
        let size = Vec2::new(w as f32, h as f32);
        let band_len = BAND as usize * w as usize;
        let band = |(bi, (tri, bary)): (usize, (&mut [u32], &mut [[f32; 2]]))| {
            let mut overlaps = 0u32;
            let row0 = bi as u32 * BAND;
            let rows = (tri.len() / w as usize) as u32;
            let list = match (self.offsets.get(bi), self.offsets.get(bi + 1)) {
                (Some(&start), Some(&end)) => self.ids.get(start..end).unwrap_or(&[]),
                _ => &[],
            };
            for &t in list {
                let Some(([pa, pb, pc], area, [x0, x1, y0, y1])) = texel_triangle(uvs, indices, t as usize, size) else { continue };
                let eps = -1e-6 * area.abs();
                let (y0, y1) = (y0.max(row0), y1.min(row0 + rows));
                for y in y0..y1 {
                    for x in x0..x1 {
                        let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let (w0, w1, w2) = (edge(pb, pc, p) * area.signum(), edge(pc, pa, p) * area.signum(), edge(pa, pb, p) * area.signum());
                        if w0 < eps || w1 < eps || w2 < eps {
                            continue;
                        }
                        let i = (y - row0) as usize * w as usize + x as usize;
                        if let (Some(slot), Some(bw)) = (tri.get_mut(i), bary.get_mut(i)) {
                            if *slot != NO_TRI {
                                overlaps = overlaps.saturating_add(1);
                            }
                            *slot = t;
                            let inv = 1.0 / area.abs();
                            *bw = [w1 * inv, w2 * inv];
                        }
                    }
                }
            }
            overlaps
        };
        each_band(tri, bary, band_len, band)
    }
}

/// Run `band` on each band of `len` texels (on several threads with `parallel`) and add up the
/// overlap counts.
#[cfg(feature = "parallel")]
fn each_band(tri: &mut [u32], bary: &mut [[f32; 2]], len: usize, band: impl Fn((usize, (&mut [u32], &mut [[f32; 2]]))) -> u32 + Sync + Send) -> u32 {
    tri.par_chunks_mut(len).zip(bary.par_chunks_mut(len)).enumerate().map(band).reduce(|| 0, u32::saturating_add)
}

/// Run `band` on each band of `len` texels (on several threads with `parallel`) and add up the
/// overlap counts.
#[cfg(not(feature = "parallel"))]
fn each_band(tri: &mut [u32], bary: &mut [[f32; 2]], len: usize, band: impl Fn((usize, (&mut [u32], &mut [[f32; 2]]))) -> u32) -> u32 {
    tri.chunks_mut(len).zip(bary.chunks_mut(len)).enumerate().map(band).fold(0, u32::saturating_add)
}

/// Interpolate triangle `t` at barycentrics (weights of vertices 1 and 2). The tangent keeps the
/// direction of the interpolated vertex tangents and is not made perpendicular to the
/// interpolated normal: MikkTSpace bakers and shaders use the interpolated vectors as they are.
/// The bitangent sign is the one of the triangle's first vertex.
pub fn surface_at(mesh: &Mesh, t: u32, b1: f32, b2: f32) -> Option<Surface> {
    let [a, b, c] = mesh.tri(t as usize)?;
    let b0 = 1.0 - b1 - b2;
    let p = |v: &[Vec3]| -> Option<Vec3> { Some(*v.get(a)? * b0 + *v.get(b)? * b1 + *v.get(c)? * b2) };
    let pos = p(&mesh.positions)?;
    // `unit` divides by the largest component first, so a long finite vector keeps its direction.
    let nrm = unit(p(&mesh.normals)?).unwrap_or(Vec3::Y);
    let (pa, pb, pc) = (*mesh.positions.get(a)?, *mesh.positions.get(b)?, *mesh.positions.get(c)?);
    // In `f64` the edges between any two finite `f32` positions and their cross product neither
    // overflow nor round to zero.
    let (pa, pb, pc) = (pa.as_dvec3(), pb.as_dvec3(), pc.as_dvec3());
    let geo_nrm = (pb - pa).cross(pc - pa).try_normalize().map_or(nrm, |n| n.as_vec3());
    let tan = *mesh.tangents.get(a)? * b0 + *mesh.tangents.get(b)? * b1 + *mesh.tangents.get(c)? * b2;
    let uv = *mesh.uvs.get(a)? * b0 + *mesh.uvs.get(b)? * b1 + *mesh.uvs.get(c)? * b2;
    let tdir = unit(tan.truncate()).unwrap_or_else(|| nrm.any_orthonormal_vector());
    let w = mesh.tangents.get(a).map(|t| if t.w < 0.0 { -1.0 } else { 1.0 }).unwrap_or(1.0);
    Some(Surface { pos, nrm, geo_nrm, tangent: tdir.extend(w), uv, tri: t })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn half_triangle_covers_texels_with_centers_inside() {
        // One triangle covering the lower-left half of UV space (v down: y = v·H).
        let m = Mesh::from_parts(
            vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            None,
            vec![Vec2::new(0.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0)],
            vec![0, 1, 2],
            "m",
        )
        .unwrap();
        let g = UvGBuffer::new(&m, 0, 8, 8);
        for y in 0..8u32 {
            for x in 0..8u32 {
                let inside = (x as f32 + 0.5) <= (y as f32 + 0.5);
                assert_eq!(g.covered((y * 8 + x) as usize), inside, "({x},{y})");
            }
        }
        assert_eq!(g.overlaps, 0);
    }

    #[test]
    fn overlapping_and_degenerate_triangles() {
        let uv = vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(0.5, 0.5)];
        let m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z], None, uv, vec![0, 1, 2, 0, 1, 2, 0, 3, 3], "m").unwrap();
        let g = UvGBuffer::new(&m, 0, 16, 16);
        assert!(g.overlaps > 0);
        assert!(g.covered_count() > 0);
    }

    #[test]
    fn sphere_surface_lies_on_the_sphere() {
        let m = primitives::sphere(2.0, 64, 32);
        let g = UvGBuffer::new(&m, 0, 128, 128);
        assert!(g.covered_count() > 128 * 128 * 9 / 10, "{}", g.covered_count());
        for i in (0..g.tri.len()).step_by(97) {
            if let Some(s) = g.surface(&m, i) {
                assert!((s.pos.length() - 2.0).abs() < 0.02);
                assert!(s.nrm.dot(s.pos.normalize()) > 0.99);
            }
        }
    }

    #[test]
    fn surface_keeps_the_direction_of_long_finite_vectors() {
        // Squaring 1e30 overflows f32: the normal, the tangent and the face normal come out along
        // the vectors all the same.
        let s = 1e30;
        let m = Mesh {
            positions: vec![Vec3::ZERO, Vec3::X * s, Vec3::Y * s],
            normals: vec![Vec3::X * s; 3],
            tangents: vec![Vec4::new(0.0, s, 0.0, -1.0); 3],
            uvs: vec![Vec2::ZERO, Vec2::X, Vec2::Y],
            indices: vec![0, 1, 2],
            ..Mesh::default()
        };
        let surface = surface_at(&m, 0, 0.25, 0.25).unwrap();
        assert!((surface.nrm - Vec3::X).length() < 1e-6, "{}", surface.nrm);
        assert!((surface.tangent - Vec4::new(0.0, 1.0, 0.0, -1.0)).length() < 1e-6, "{}", surface.tangent);
        assert!((surface.geo_nrm - Vec3::Z).length() < 1e-6, "{}", surface.geo_nrm);
    }

    #[test]
    fn face_normal_of_a_triangle_wider_than_the_f32_range() {
        // Its x extent of 6e38 is past f32::MAX, while every corner is finite. The shading
        // normals point along +X, away from the face.
        let m = Mesh {
            positions: vec![Vec3::new(-3e38, 0.0, 0.0), Vec3::new(3e38, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)],
            normals: vec![Vec3::X; 3],
            tangents: vec![Vec4::new(0.0, 1.0, 0.0, 1.0); 3],
            uvs: vec![Vec2::ZERO, Vec2::X, Vec2::Y],
            indices: vec![0, 1, 2],
            ..Mesh::default()
        };
        m.validate().unwrap();
        let surface = surface_at(&m, 0, 0.25, 0.25).unwrap();
        assert!((surface.nrm - Vec3::X).length() < 1e-6, "{}", surface.nrm);
        assert!((surface.geo_nrm - Vec3::Z).length() < 1e-6, "{}", surface.geo_nrm);
        // A triangle a few subnormal steps wide keeps its face normal too.
        let tiny = f32::from_bits(1);
        let m = Mesh { positions: vec![Vec3::ZERO, Vec3::X * tiny, Vec3::Y * tiny], ..m };
        let surface = surface_at(&m, 0, 0.25, 0.25).unwrap();
        assert!((surface.geo_nrm - Vec3::Z).length() < 1e-6, "{}", surface.geo_nrm);
    }

    #[test]
    fn gbuffer_rejects_huge_dimensions() {
        let m = primitives::plane(1.0, 2);
        let l = Limits::default();
        for (w, h) in [(u32::MAX, u32::MAX), (l.max_gbuffer_side + 1, 1), (1, 0), (0, 1)] {
            assert!(UvGBuffer::try_new(&m, 0, w, h, &l).is_err(), "{w} × {h}");
            assert!(UvGBuffer::from_slices(&[[0.0, 0.0]; 3], &[0, 1, 2], w, h, &l).is_err(), "{w} × {h}");
        }
        let few = Limits { max_gbuffer_texels: 63, ..l };
        assert!(UvGBuffer::try_new(&m, 0, 8, 8, &few).is_err());
        // The infallible form clamps each side.
        let g = UvGBuffer::new(&m, 0, u32::MAX, 0);
        assert_eq!((g.width, g.height, g.tri.len()), (l.max_gbuffer_side, 1, l.max_gbuffer_side as usize));
        // The checked forms rasterize as the clamped one does.
        assert_eq!(UvGBuffer::try_new(&m, 0, 16, 16, &l).unwrap(), UvGBuffer::new(&m, 0, 16, 16));
        let uvs: Vec<[f32; 2]> = m.uvs.iter().map(|uv| uv.to_array()).collect();
        assert_eq!(UvGBuffer::from_slices(&uvs, &m.indices, 16, 16, &l).unwrap(), UvGBuffer::new(&m, 0, 16, 16));
    }

    #[test]
    fn gbuffer_slices_reject_bad_indices_and_uvs() {
        let l = Limits::default();
        assert!(UvGBuffer::from_slices(&[[0.0, 0.0]; 3], &[0, 1, 3], 4, 4, &l).is_err());
        assert!(UvGBuffer::from_slices(&[[0.0, 0.0]; 3], &[0, 1], 4, 4, &l).is_err());
        assert!(UvGBuffer::from_slices(&[[0.0, 0.0], [f32::NAN, 0.0], [0.0, 1.0]], &[0, 1, 2], 4, 4, &l).is_err());
        let empty = UvGBuffer::from_slices(&[], &[], 4, 4, &l).unwrap();
        assert_eq!((empty.covered_count(), empty.overlaps), (0, 0));
    }

    #[test]
    fn gbuffer_skips_triangles_whose_texel_coordinates_overflow() {
        let l = Limits::default();
        // Corners that become infinite in texels, and corners whose products overflow.
        for far in [1e38f32, 1e30] {
            let g = UvGBuffer::from_slices(&[[far, 0.0], [-far, 0.0], [0.0, far]], &[0, 1, 2], 8, 8, &l).unwrap();
            assert_eq!(g.covered_count(), 0, "{far}");
            assert!(g.bary.iter().flatten().all(|w| w.is_finite()), "{far}");
        }
        // A large triangle within reach covers every texel with weights that add up to one.
        let g = UvGBuffer::from_slices(&[[-1e6, -1e6], [1e6, -1e6], [0.0, 1e6]], &[0, 1, 2], 8, 8, &l).unwrap();
        assert_eq!(g.covered_count(), 64);
        assert!(g.bary.iter().all(|[b1, b2]| (0.0..=1.0).contains(b1) && (0.0..=1.0).contains(b2) && b1 + b2 <= 1.0 + 1e-6), "{:?}", g.bary);
    }

    /// The `what`, `got` and `limit` of a `TooLarge` error, `None` for any other result.
    fn too_large(r: Result<UvGBuffer>) -> Option<(&'static str, u64, u64)> {
        match r {
            Err(MeshOpsError::TooLarge { what, got, limit }) => Some((what, got, limit)),
            _ => None,
        }
    }

    #[test]
    fn gbuffer_rejects_overlapping_box_work_before_rasterization() {
        // Five triangles whose boxes each cover the whole 8 × 8 buffer: 320 texel visits.
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let indices = [0, 1, 2].repeat(5);
        let l = Limits::default();
        let four = Limits { max_gbuffer_work: 4 * 64, ..l };
        assert_eq!(too_large(UvGBuffer::from_slices(&uvs, &indices, 8, 8, &four)), Some(("G-buffer work", 320, 256)));
        let five = Limits { max_gbuffer_work: 5 * 64, ..l };
        assert_eq!(UvGBuffer::from_slices(&uvs, &indices, 8, 8, &five).unwrap(), UvGBuffer::from_slices(&uvs, &indices, 8, 8, &l).unwrap());
    }

    #[test]
    fn gbuffer_rejects_excess_band_entries() {
        // Three triangles that each reach the four bands of a 128-row buffer: 12 entries.
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let indices = [0, 1, 2].repeat(3);
        let l = Limits::default();
        let under = Limits { max_gbuffer_bin_entries: 11, ..l };
        assert_eq!(too_large(UvGBuffer::from_slices(&uvs, &indices, 16, 128, &under)), Some(("G-buffer band entries", 12, 11)));
        let exact = Limits { max_gbuffer_bin_entries: 12, ..l };
        let g = UvGBuffer::from_slices(&uvs, &indices, 16, 128, &exact).unwrap();
        assert_eq!(g, UvGBuffer::from_slices(&uvs, &indices, 16, 128, &l).unwrap());
        // Every band draws the triangles in order: the last one keeps each texel.
        let covered = g.covered_count();
        assert!(covered > 16 * 128 / 3, "{covered}");
        assert!(g.tri.iter().all(|t| *t == 2 || *t == NO_TRI));
        assert_eq!(g.overlaps as usize, 2 * covered);
    }

    #[test]
    fn gbuffer_mesh_constructor_respects_count_limits() {
        let m = primitives::plane(1.0, 2);
        let l = Limits::default();
        let (v, t) = (m.vertex_count(), m.triangle_count());
        let few_triangles = Limits { max_triangles: t - 1, ..l };
        assert_eq!(too_large(UvGBuffer::try_new(&m, 0, 16, 16, &few_triangles)), Some(("triangle count", t as u64, t as u64 - 1)));
        let few_vertices = Limits { max_vertices: v - 1, ..l };
        assert_eq!(too_large(UvGBuffer::try_new(&m, 0, 16, 16, &few_vertices)), Some(("vertex count", v as u64, v as u64 - 1)));
        let exact = Limits { max_vertices: v, max_triangles: t, ..l };
        assert_eq!(UvGBuffer::try_new(&m, 0, 16, 16, &exact).unwrap(), UvGBuffer::new(&m, 0, 16, 16));
        // Submeshes that list the same triangles again count every time they are visited.
        let mut twice = m.clone();
        twice.submeshes.push(twice.submeshes[0].clone());
        assert_eq!(too_large(UvGBuffer::try_new(&twice, 0, 16, 16, &exact)), Some(("triangle count", t as u64 + 1, t as u64)));
    }

    #[test]
    fn gbuffer_work_counts_clipped_boxes_and_skips_off_image() {
        let uvs = [
            // A 2 × 2 texel box.
            [0.5, 0.5],
            [0.75, 0.5],
            [0.5, 0.75],
            // Entirely past the right and bottom edges.
            [2.0, 2.0],
            [3.0, 2.0],
            [2.0, 3.0],
            // Infinite once scaled to texels.
            [1e38, 0.0],
            [-1e38, 0.0],
            [0.0, 1e38],
            // A triangle over the whole image whose box is clipped to its 64 texels.
            [-1.0, -1.0],
            [3.0, -1.0],
            [-1.0, 3.0],
        ];
        let indices: Vec<u32> = (0..12).collect();
        let l = Limits::default();
        let under = Limits { max_gbuffer_work: 67, ..l };
        assert_eq!(too_large(UvGBuffer::from_slices(&uvs, &indices, 8, 8, &under)), Some(("G-buffer work", 68, 67)));
        let exact = Limits { max_gbuffer_work: 68, ..l };
        let g = UvGBuffer::from_slices(&uvs, &indices, 8, 8, &exact).unwrap();
        assert_eq!(g, UvGBuffer::from_slices(&uvs, &indices, 8, 8, &l).unwrap());
        // The small triangle covers 3 texels, which the large one draws over.
        assert_eq!((g.covered_count(), g.overlaps), (64, 3));
    }

    /// Texel buffers of `2^28 × 2^28` (2^58 and 2^59 bytes) are past every 64-bit address space,
    /// so their reservation fails, and a work or band-entry rejection shows that the budgets are
    /// checked first.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn gbuffer_budgets_are_checked_before_the_texels_are_reserved() {
        let huge = Limits { max_gbuffer_side: u32::MAX, max_gbuffer_texels: u64::MAX, max_gbuffer_work: u64::MAX, ..Limits::default() };
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let m = primitives::plane(1.0, 2);
        let side = 1 << 28;
        let cases =
            [(Limits { max_gbuffer_work: 0, ..huge }, "G-buffer work"), (Limits { max_gbuffer_bin_entries: 0, ..huge }, "G-buffer band entries")];
        for (limits, what) in cases {
            let slices = too_large(UvGBuffer::from_slices(&uvs, &[0, 1, 2], side, side, &limits));
            assert_eq!(slices.map(|(w, _, l)| (w, l)), Some((what, 0)));
            let mesh = too_large(UvGBuffer::try_new(&m, 0, side, side, &limits));
            assert_eq!(mesh.map(|(w, _, l)| (w, l)), Some((what, 0)));
        }
        // A size whose texel arrays no allocation can hold is refused before the band lists are
        // reserved, even with no triangles.
        for (w, h) in [(u32::MAX, u32::MAX), (1 << 31, 1 << 30)] {
            assert!(matches!(UvGBuffer::from_slices(&[], &[], w, h, &huge), Err(MeshOpsError::OutOfMemory("G-buffer"))), "{w} × {h}");
            assert!(matches!(UvGBuffer::try_new(&m, 0, w, h, &huge), Err(MeshOpsError::OutOfMemory("G-buffer"))), "{w} × {h}");
        }
    }
}
