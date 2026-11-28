//! The UV-space G-buffer: which triangle (and where on it) each texel of a texture set maps
//! to. Painting, compositing, baking and padding all work from this one buffer, so they agree
//! on which texel is which surface point.

use glam::{DVec2, Vec2, Vec3, Vec4};
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

/// The edge function of `a → b` at `p`, `(b - a) × (p - a)`, rounded once from the exact value
/// of its six products. Each product is split into its rounded value and its rounding error,
/// and the twelve parts are added with Neumaier's compensation, so corners far from `p` do not
/// cancel the digits that place `p` against the edge.
fn edge(a: DVec2, b: DVec2, p: DVec2) -> f64 {
    let products = [(b.x, p.y), (-b.x, a.y), (-a.x, p.y), (-b.y, p.x), (b.y, a.x), (a.y, p.x)];
    let (mut sum, mut compensation) = (0.0f64, 0.0f64);
    for (x, y) in products {
        let high = x * y;
        for part in [high, x.mul_add(y, -high)] {
            let next = sum + part;
            compensation += if sum.abs() >= part.abs() { (sum - next) + part } else { (part - next) + sum };
            sum = next;
        }
    }
    sum + compensation
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
                    if !(pa.is_finite() && pb.is_finite() && pc.is_finite()) {
                        continue;
                    }
                    let (qa, qb, qc) = (pa.as_dvec2(), pb.as_dvec2(), pc.as_dvec2());
                    let area = edge(qa, qb, qc);
                    if area.abs() < 1e-12 {
                        continue;
                    }
                    let x0 = pa.x.min(pb.x).min(pc.x).floor().max(0.0) as u32;
                    let x1 = (pa.x.max(pb.x).max(pc.x).ceil().min(w as f32) as u32).min(w);
                    let y0 = (pa.y.min(pb.y).min(pc.y).floor().max(row0 as f32) as u32).max(row0);
                    let y1 = (pa.y.max(pb.y).max(pc.y).ceil() as u32).min(row0 + rows);
                    if y0 >= y1 || x0 >= x1 {
                        continue;
                    }
                    // Each edge function is taken in full at the first texel center of the box and
                    // stepped from there along its gradient, so its rounding at the other centers
                    // is a few `f64` steps of the box's size rather than of the corners'
                    // coordinates. Each edge function is its distance times the edge's length. A
                    // texel center within 1e-6 texel of an edge, plus that rounding, counts as
                    // inside: both triangles that share the edge write it. The slack is at most
                    // 1e-3 of the triangle's smallest height, so a thin triangle is not widened.
                    let start = DVec2::new(f64::from(x0) + 0.5, f64::from(y0) + 0.5);
                    let edges = [(qb, qc), (qc, qa), (qa, qb)];
                    let base = edges.map(|(a, b)| edge(a, b, start) * area.signum());
                    let step = edges.map(|(a, b)| DVec2::new(a.y - b.y, b.x - a.x) * area.signum());
                    let lengths = edges.map(|(a, b)| (b - a).length());
                    let lowest = area.abs() / lengths.iter().copied().fold(0.0, f64::max);
                    let slack = (1e-6 + 8.0 * f64::EPSILON * f64::from(size.max_element())).min(1e-3 * lowest);
                    let eps = lengths.map(|length| -slack * length);
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let d = DVec2::new(f64::from(x - x0), f64::from(y - y0));
                            // A stepped value is within a few rounding steps of its largest term.
                            // When that bound reaches the threshold, the edge is taken again at
                            // this center, so cancellation cannot erase its signed distance.
                            let center = start + d;
                            let [w0, w1, w2] = [0, 1, 2].map(|i| {
                                let (sx, sy) = (step[i].x * d.x, step[i].y * d.y);
                                let w = base[i] + (sx + sy);
                                let bound = 4.0 * f64::EPSILON * (base[i].abs() + sx.abs() + sy.abs());
                                if (w - eps[i]).abs() <= bound {
                                    let (a, b) = edges[i];
                                    edge(a, b, center) * area.signum()
                                } else {
                                    w
                                }
                            });
                            if w0 < eps[0] || w1 < eps[1] || w2 < eps[2] {
                                continue;
                            }
                            let i = ((y - row0) * w + x) as usize;
                            if let (Some(slot), Some(bw)) = (tri.get_mut(i), bary.get_mut(i)) {
                                if *slot != NO_TRI {
                                    overlaps += 1;
                                }
                                *slot = t;
                                let inv = 1.0 / area.abs();
                                *bw = [(w1 * inv) as f32, (w2 * inv) as f32];
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
    // In `f64` the edges between any two finite `f32` positions and their cross product neither
    // overflow nor round to zero.
    let (pa, pb, pc) = (mesh.positions.get(a)?.as_dvec3(), mesh.positions.get(b)?.as_dvec3(), mesh.positions.get(c)?.as_dvec3());
    let geo_nrm = (pb - pa).cross(pc - pa).try_normalize().map_or(nrm, |n| n.as_vec3());
    let tan = *mesh.tangents.get(a)? * b0 + *mesh.tangents.get(b)? * b1 + *mesh.tangents.get(c)? * b2;
    let uv = *mesh.uvs.get(a)? * b0 + *mesh.uvs.get(b)? * b1 + *mesh.uvs.get(c)? * b2;
    // Interpolated tangents drift off the interpolated normal: take the part perpendicular to it.
    let tan3 = tan.truncate();
    let tdir = (tan3 - nrm * nrm.dot(tan3)).try_normalize().unwrap_or_else(|| nrm.any_orthonormal_vector());
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

    /// A one-triangle mesh whose UVs are set to `uvs` after it is built.
    fn mesh_with_uvs(uvs: [[f32; 2]; 3]) -> Mesh {
        let mut m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, vec![Vec2::ZERO, Vec2::X, Vec2::Y], vec![0, 1, 2], "m").unwrap();
        m.uvs = uvs.map(Vec2::from).to_vec();
        m
    }

    #[test]
    fn edge_tolerance_is_a_texel_distance_for_far_corners() {
        for m in [1_048_576.0_f32, 1e15, 1.25e17, 1e30] {
            // Covers u + v <= 0: no texel center of an 8 x 8 buffer is inside it.
            let g = UvGBuffer::new(&mesh_with_uvs([[-m, -m], [m, -m], [-m, m]]), 0, 8, 8);
            assert_eq!(g.covered_count(), 0, "{m}");
            // The mirrored triangle covers u + v >= 0, which is every texel center.
            let g = UvGBuffer::new(&mesh_with_uvs([[m, m], [-m, m], [m, -m]]), 0, 8, 8);
            assert_eq!(g.covered_count(), 64, "{m}");
            assert!(g.bary.iter().all(|[b1, b2]| (0.0..=1.0).contains(b1) && (0.0..=1.0).contains(b2)), "{m}: {:?}", g.bary);
        }
        // A sliver 0.0125 texel high, 2e14 texels wide: rounding at those coordinates does not
        // widen it to the texel centers at y = 0.5.
        let g = UvGBuffer::new(&mesh_with_uvs([[-1e14, 0.0], [1e14, 0.0], [0.0, 0.0125]]), 0, 8, 8);
        assert_eq!(g.covered_count(), 0);
    }

    #[test]
    fn stepped_edge_values_that_lose_the_distance_are_taken_again() {
        // Stepping from the box's first center rounds the first-to-second edge value at texel
        // (32, 32) to zero, although that center is far outside this sliver.
        let (lo, hi) = (2.0_f32.powi(-70), 2.0_f32.powi(60));
        let g = UvGBuffer::new(&mesh_with_uvs([[0.0, lo], [hi, hi], [0.0, 2.0 * lo]]), 0, 64, 64);
        assert!(!g.covered(32 * 64 + 32));
        assert_eq!(g.covered_count(), 0);
        // Two triangles that share the diagonal both still take its eight texel centers.
        let uv = vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0)];
        let m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z], None, uv, vec![0, 1, 2, 0, 3, 1], "m").unwrap();
        let g = UvGBuffer::new(&m, 0, 8, 8);
        assert_eq!(g.covered_count(), 64);
        assert_eq!(g.overlaps, 8);
    }

    #[test]
    fn surface_face_normal_of_corners_whose_differences_overflow_f32() {
        // pb - pa is 6e38, past f32::MAX: the face normal is still +Z, not the +X shading normal.
        let m = Mesh {
            positions: vec![Vec3::new(-3e38, 0.0, 0.0), Vec3::new(3e38, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)],
            normals: vec![Vec3::X; 3],
            tangents: vec![Vec4::new(0.0, 1.0, 0.0, 1.0); 3],
            uvs: vec![Vec2::ZERO, Vec2::X, Vec2::Y],
            indices: vec![0, 1, 2],
            ..Mesh::default()
        };
        let surface = surface_at(&m, 0, 0.25, 0.25).unwrap();
        assert!((surface.geo_nrm - Vec3::Z).length() < 1e-6, "{}", surface.geo_nrm);
        // A triangle a few subnormal steps wide keeps its face normal too.
        let tiny = f32::from_bits(1);
        let m = Mesh { positions: vec![Vec3::ZERO, Vec3::X * tiny, Vec3::Y * tiny], ..m };
        let surface = surface_at(&m, 0, 0.25, 0.25).unwrap();
        assert!((surface.geo_nrm - Vec3::Z).length() < 1e-6, "{}", surface.geo_nrm);
    }

    #[test]
    fn surface_tangent_is_perpendicular_to_the_interpolated_normal() {
        let m = Mesh {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            normals: vec![Vec3::Z, Vec3::X, Vec3::X],
            tangents: vec![Vec4::new(1.0, 0.0, 0.0, 1.0), Vec4::new(0.0, 1.0, 0.0, 1.0), Vec4::new(0.0, 1.0, 0.0, 1.0)],
            uvs: vec![Vec2::ZERO, Vec2::X, Vec2::Y],
            indices: vec![0, 1, 2],
            ..Mesh::default()
        };
        let s = surface_at(&m, 0, 0.5, 0.0).unwrap();
        let t = s.tangent.truncate();
        assert!(s.nrm.dot(t).abs() < 1e-6, "nrm {} tangent {}", s.nrm, t);
        assert!((t.length() - 1.0).abs() < 1e-6, "{t}");
        // A tangent parallel to the normal falls back to some perpendicular direction.
        let m = Mesh { tangents: vec![Vec4::new(0.0, 0.0, 1.0, 1.0); 3], normals: vec![Vec3::Z; 3], ..m };
        let s = surface_at(&m, 0, 0.25, 0.25).unwrap();
        let t = s.tangent.truncate();
        assert!(s.nrm.dot(t).abs() < 1e-6 && (t.length() - 1.0).abs() < 1e-6, "{t}");
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
