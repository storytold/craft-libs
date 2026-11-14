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

/// Texel count of a `width × height` buffer, checked against `limits`.
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
    usize::try_from(n).map_err(|_| MeshOpsError::TooLarge { what: "G-buffer texel count", got: n, limit: usize::MAX as u64 })
}

impl UvGBuffer {
    /// Rasterize the triangles of material `material` at `width × height`, each side clamped to
    /// `1..=8192` (the default [`Limits::max_gbuffer_side`]). When the memory for that size
    /// cannot be reserved, the buffer is 1 × 1 and uncovered.
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
    /// side is an error, and the buffers are reserved before rasterizing.
    pub fn try_new(mesh: &Mesh, material: usize, width: u32, height: u32, limits: &Limits) -> Result<UvGBuffer> {
        let n = texels(width, height, limits)?;
        let (mut tri, mut bary) = (try_vec(n, "G-buffer")?, try_vec(n, "G-buffer")?);
        tri.resize(n, NO_TRI);
        bary.resize(n, [0.0f32; 2]);
        let overlaps = rasterize(&mesh.uvs, &mesh.indices, mesh.triangles_of(material), width, height, &mut tri, &mut bary)?;
        Ok(UvGBuffer { width, height, tri, bary, overlaps })
    }

    /// Rasterize every triangle of `indices` over `uvs` at `width × height`. The size, the
    /// triangle count, every index and every UV (finite) are checked first.
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
        let (mut tri, mut bary) = (try_vec(n, "G-buffer")?, try_vec(n, "G-buffer")?);
        tri.resize(n, NO_TRI);
        bary.resize(n, [0.0f32; 2]);
        let overlaps = rasterize(&points, indices, 0..indices.len() / 3, width, height, &mut tri, &mut bary)?;
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

/// Write the triangle and barycentrics under each texel center of `w × h` buffers (`tri` and
/// `bary` hold `w × h` entries) for `triangles`; returns the overlap count. Each band of rows
/// lists the triangles that reach it, and the lists reserve their memory as they grow.
fn rasterize(
    uvs: &[Vec2],
    indices: &[u32],
    triangles: impl Iterator<Item = usize>,
    w: u32,
    h: u32,
    tri: &mut [u32],
    bary: &mut [[f32; 2]],
) -> Result<u32> {
    let size = Vec2::new(w as f32, h as f32);
    let bands = h.div_ceil(BAND) as usize;
    let mut bins: Vec<Vec<u32>> = vec![Vec::new(); bands];
    // NO_TRI marks an uncovered texel, so it cannot name a triangle.
    for t in triangles.take_while(|t| *t < NO_TRI as usize) {
        let Some([a, b, c]) = corners(indices, t) else { continue };
        let (Some(ua), Some(ub), Some(uc)) = (uvs.get(a), uvs.get(b), uvs.get(c)) else { continue };
        let y0 = (ua.y.min(ub.y).min(uc.y) * size.y).floor().max(0.0) as i64;
        let y1 = (ua.y.max(ub.y).max(uc.y) * size.y).ceil().min(size.y) as i64;
        let in_reach = |uv: &Vec2| uv.is_finite() && (*uv * size).abs().max_element() <= MAX_TEXEL_COORDINATE;
        if y1 <= y0 || !in_reach(ua) || !in_reach(ub) || !in_reach(uc) {
            continue;
        }
        let (b0, b1) = ((y0 / BAND as i64) as usize, ((y1 - 1).max(0) / BAND as i64) as usize);
        for bin in bins.iter_mut().take(b1 + 1).skip(b0) {
            bin.try_reserve(1).map_err(|_| MeshOpsError::OutOfMemory("G-buffer bands"))?;
            bin.push(t as u32);
        }
    }
    let band_len = BAND as usize * w as usize;
    let band = |(bi, (tri, bary)): (usize, (&mut [u32], &mut [[f32; 2]]))| {
        let mut overlaps = 0u32;
        let row0 = bi as u32 * BAND;
        let rows = (tri.len() / w as usize) as u32;
        for &t in bins.get(bi).map(Vec::as_slice).unwrap_or(&[]) {
            let Some([a, b, c]) = corners(indices, t as usize) else { continue };
            let (Some(&ua), Some(&ub), Some(&uc)) = (uvs.get(a), uvs.get(b), uvs.get(c)) else { continue };
            let (pa, pb, pc) = (ua * size, ub * size, uc * size);
            let area = edge(pa, pb, pc);
            if area.abs() < 1e-12 {
                continue;
            }
            let eps = -1e-6 * area.abs();
            let x0 = pa.x.min(pb.x).min(pc.x).floor().max(0.0) as u32;
            let x1 = (pa.x.max(pb.x).max(pc.x).ceil().min(w as f32) as u32).min(w);
            let y0 = (pa.y.min(pb.y).min(pc.y).floor().max(row0 as f32) as u32).max(row0);
            let y1 = (pa.y.max(pb.y).max(pc.y).ceil() as u32).min(row0 + rows);
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
    Ok(each_band(tri, bary, band_len, band))
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
    // The edges are divided by their largest component before the cross product, which can
    // overflow where the edges themselves do not.
    let (e1, e2) = (pb - pa, pc - pa);
    let largest = e1.abs().max(e2.abs()).max_element();
    let geo_nrm = (largest > 0.0).then(|| unit((e1 / largest).cross(e2 / largest))).flatten().unwrap_or(nrm);
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
}
