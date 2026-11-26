//! The UV-space G-buffer: which triangle (and where on it) each texel of a texture set maps
//! to. Painting, compositing, baking and padding all work from this one buffer, so they agree
//! on which texel is which surface point.

use glam::{Vec2, Vec3, Vec4};
use rayon::prelude::*;

use crate::Mesh;

/// Texel not covered by any triangle.
pub const NO_TRI: u32 = u32::MAX;

const BAND: u32 = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct UvGBuffer {
    pub width: u32,
    pub height: u32,
    /// Triangle under each texel center, [`NO_TRI`] when uncovered.
    pub tri: Vec<u32>,
    /// Barycentric weights of the triangle's second and third vertices.
    pub bary: Vec<[f32; 2]>,
    /// Texels written by more than one triangle (overlapping UVs).
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

impl UvGBuffer {
    /// Rasterize the triangles of material `material` at `width × height`.
    pub fn new(mesh: &Mesh, material: usize, width: u32, height: u32) -> UvGBuffer {
        let (w, h) = (width.max(1), height.max(1));
        let size = Vec2::new(w as f32, h as f32);
        let bands = h.div_ceil(BAND) as usize;
        let mut bins: Vec<Vec<u32>> = vec![Vec::new(); bands];
        for t in mesh.triangles_of(material) {
            let Some([a, b, c]) = mesh.tri(t) else { continue };
            let (Some(ua), Some(ub), Some(uc)) = (mesh.uvs.get(a), mesh.uvs.get(b), mesh.uvs.get(c)) else { continue };
            let y0 = (ua.y.min(ub.y).min(uc.y) * size.y).floor().max(0.0) as i64;
            let y1 = (ua.y.max(ub.y).max(uc.y) * size.y).ceil().min(size.y) as i64;
            if y1 <= y0 || !ua.is_finite() || !ub.is_finite() || !uc.is_finite() {
                continue;
            }
            let (b0, b1) = ((y0 / BAND as i64) as usize, ((y1 - 1).max(0) / BAND as i64) as usize);
            for bin in bins.iter_mut().take(b1 + 1).skip(b0) {
                bin.push(t as u32);
            }
        }
        let n = w as usize * h as usize;
        let mut tri = vec![NO_TRI; n];
        let mut bary = vec![[0.0f32; 2]; n];
        let band_len = (BAND * w) as usize;
        let overlaps: u32 = tri
            .par_chunks_mut(band_len)
            .zip(bary.par_chunks_mut(band_len))
            .enumerate()
            .map(|(bi, (tri, bary))| {
                let mut overlaps = 0u32;
                let row0 = bi as u32 * BAND;
                let rows = (tri.len() as u32) / w;
                for &t in bins.get(bi).map(Vec::as_slice).unwrap_or(&[]) {
                    let Some([a, b, c]) = mesh.tri(t as usize) else { continue };
                    let (Some(&ua), Some(&ub), Some(&uc)) = (mesh.uvs.get(a), mesh.uvs.get(b), mesh.uvs.get(c)) else { continue };
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
                            let i = ((y - row0) * w + x) as usize;
                            if let (Some(slot), Some(bw)) = (tri.get_mut(i), bary.get_mut(i)) {
                                if *slot != NO_TRI {
                                    overlaps += 1;
                                }
                                *slot = t;
                                let inv = 1.0 / area.abs();
                                *bw = [w1 * inv, w2 * inv];
                            }
                        }
                    }
                }
                overlaps
            })
            .sum();
        UvGBuffer { width: w, height: h, tri, bary, overlaps }
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

/// Interpolate triangle `t` at barycentrics (weights of vertices 1 and 2).
pub fn surface_at(mesh: &Mesh, t: u32, b1: f32, b2: f32) -> Option<Surface> {
    let [a, b, c] = mesh.tri(t as usize)?;
    let b0 = 1.0 - b1 - b2;
    let p = |v: &[Vec3]| -> Option<Vec3> { Some(*v.get(a)? * b0 + *v.get(b)? * b1 + *v.get(c)? * b2) };
    let pos = p(&mesh.positions)?;
    let nrm = p(&mesh.normals)?.try_normalize().unwrap_or(Vec3::Y);
    let (pa, pb, pc) = (*mesh.positions.get(a)?, *mesh.positions.get(b)?, *mesh.positions.get(c)?);
    let geo_nrm = (pb - pa).cross(pc - pa).try_normalize().unwrap_or(nrm);
    let tan = *mesh.tangents.get(a)? * b0 + *mesh.tangents.get(b)? * b1 + *mesh.tangents.get(c)? * b2;
    let uv = *mesh.uvs.get(a)? * b0 + *mesh.uvs.get(b)? * b1 + *mesh.uvs.get(c)? * b2;
    let tdir = tan.truncate().try_normalize().unwrap_or_else(|| nrm.any_orthonormal_vector());
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
}
