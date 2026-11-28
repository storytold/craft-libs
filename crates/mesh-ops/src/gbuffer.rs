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

/// Where the triangles that wrote a texel hold its center.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Contact {
    /// Some triangle holds the center inside it.
    Interior,
    /// Every triangle holds the center on its boundary; their wedges are in the band's map.
    Boundary,
    /// The texel already counts as an overlap.
    Counted,
}

/// The angular wedge (start angle, counterclockwise sweep, radians) a triangle covers around a
/// texel center on its boundary: a half-plane on an edge, the corner's angle on a corner.
type Wedge = (f64, f64);

fn angle(v: DVec2) -> f64 {
    v.y.atan2(v.x)
}

/// The wedge around a texel center on edge `e0`–`e1` on the side of corner `c`.
fn edge_wedge(e0: DVec2, e1: DVec2, c: DVec2) -> Wedge {
    let start = if edge(e0, e1, c) > 0.0 { angle(e1 - e0) } else { angle(e0 - e1) };
    (start, std::f64::consts::PI)
}

/// The wedge of the triangle's corner at `v` between its edges toward `q` and `r`.
fn corner_wedge(v: DVec2, q: DVec2, r: DVec2) -> Wedge {
    let (a, b) = (angle(q - v), angle(r - v));
    let sweep = (b - a).rem_euclid(std::f64::consts::TAU);
    if sweep <= std::f64::consts::PI { (a, sweep) } else { (b, std::f64::consts::TAU - sweep) }
}

/// Whether the open wedges share more than a rounding error of angle.
fn wedges_overlap((a1, s1): Wedge, (a2, s2): Wedge) -> bool {
    const TOLERANCE: f64 = 1e-7;
    let d = (a2 - a1).rem_euclid(std::f64::consts::TAU);
    d < s1 - TOLERANCE || d > std::f64::consts::TAU - s2 + TOLERANCE
}

fn edge(a: DVec2, b: DVec2, p: DVec2) -> f64 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

impl UvGBuffer {
    /// Rasterize the triangles of material `material` at `width × height`.
    pub fn new(mesh: &Mesh, material: usize, width: u32, height: u32) -> UvGBuffer {
        let (w, h) = (width.max(1), height.max(1));
        let size = Vec2::new(w as f32, h as f32);
        let bands = h.div_ceil(BAND) as usize;
        let mut bins: Vec<Vec<u32>> = vec![Vec::new(); bands];
        // Each triangle once, even when two of the material's submeshes list it; distinct
        // triangles with the same UVs still overlap.
        let mut triangles: Vec<usize> = mesh.triangles_of(material).collect();
        triangles.sort_unstable();
        triangles.dedup();
        for t in triangles {
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
                // Where the triangles that wrote each texel hold its center, within the inside
                // test's tolerance.
                let mut contact: Vec<Contact> = vec![Contact::Interior; tri.len()];
                let mut wedges: std::collections::HashMap<usize, Vec<Wedge>> = std::collections::HashMap::new();
                let row0 = bi as u32 * BAND;
                let rows = (tri.len() as u32) / w;
                for &t in bins.get(bi).map(Vec::as_slice).unwrap_or(&[]) {
                    let Some([a, b, c]) = mesh.tri(t as usize) else { continue };
                    let (Some(&ua), Some(&ub), Some(&uc)) = (mesh.uvs.get(a), mesh.uvs.get(b), mesh.uvs.get(c)) else { continue };
                    // In f64, where the edge products of any finite f32 UVs neither overflow nor
                    // lose the triangle's area.
                    let dsize = size.as_dvec2();
                    let (pa, pb, pc) = (ua.as_dvec2() * dsize, ub.as_dvec2() * dsize, uc.as_dvec2() * dsize);
                    let area = edge(pa, pb, pc);
                    if area.abs() < 1e-12 {
                        continue;
                    }
                    let eps = -1e-6 * area.abs();
                    let x0 = pa.x.min(pb.x).min(pc.x).floor().max(0.0) as u32;
                    let x1 = (pa.x.max(pb.x).max(pc.x).ceil().min(f64::from(w)) as u32).min(w);
                    let y0 = (pa.y.min(pb.y).min(pc.y).floor().max(f64::from(row0)) as u32).max(row0);
                    let y1 = (pa.y.max(pb.y).max(pc.y).ceil().min(f64::from(row0 + rows)) as u32).min(row0 + rows);
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let p = DVec2::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
                            let (w0, w1, w2) = (edge(pb, pc, p) * area.signum(), edge(pc, pa, p) * area.signum(), edge(pa, pb, p) * area.signum());
                            if w0 < eps || w1 < eps || w2 < eps {
                                continue;
                            }
                            let i = ((y - row0) * w + x) as usize;
                            // A center on a triangle's boundary gets the wedge of directions the
                            // triangle covers around it. Triangles whose wedges at a center do not
                            // overlap (two sides of a shared edge, a fan around a shared corner)
                            // meet there without overlapping. Every other second write is an
                            // overlap, including stacked copies that touch only boundary centers
                            // and edges that merely cross. A texel counts once however many
                            // triangles write it.
                            let hit = match [w0, w1, w2].map(|v| v <= -eps) {
                                [false, false, false] => None,
                                [true, false, false] => Some(edge_wedge(pb, pc, pa)),
                                [false, true, false] => Some(edge_wedge(pc, pa, pb)),
                                [false, false, true] => Some(edge_wedge(pa, pb, pc)),
                                [true, true, false] => Some(corner_wedge(pc, pa, pb)),
                                [true, false, true] => Some(corner_wedge(pb, pc, pa)),
                                _ => Some(corner_wedge(pa, pb, pc)),
                            };
                            if let (Some(slot), Some(bw), Some(seen)) = (tri.get_mut(i), bary.get_mut(i), contact.get_mut(i)) {
                                if *slot == NO_TRI {
                                    *seen = match hit {
                                        Some(wedge) => {
                                            wedges.insert(i, vec![wedge]);
                                            Contact::Boundary
                                        }
                                        None => Contact::Interior,
                                    };
                                } else if *seen != Contact::Counted {
                                    let earlier = if *seen == Contact::Boundary { wedges.get_mut(&i) } else { None };
                                    match (earlier, hit) {
                                        (Some(earlier), Some(wedge)) if !earlier.iter().any(|e| wedges_overlap(*e, wedge)) => earlier.push(wedge),
                                        _ => {
                                            overlaps += 1;
                                            *seen = Contact::Counted;
                                            wedges.remove(&i);
                                        }
                                    }
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
    let (pa, pb, pc) = (*mesh.positions.get(a)?, *mesh.positions.get(b)?, *mesh.positions.get(c)?);
    // In f64, so tiny and huge faces keep their normal instead of underflowing or overflowing.
    let (da, db, dc) = (pa.as_dvec3(), pb.as_dvec3(), pc.as_dvec3());
    let geo_nrm = (db - da).cross(dc - da).try_normalize().map_or(nrm, |n| n.as_vec3());
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
    fn shared_diagonal_is_not_an_overlap() {
        // Two triangles that split the unit square share its diagonal. Both write the eight
        // texel centers on it, and neither interior overlaps the other.
        let uv = vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0)];
        let m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z], None, uv.clone(), vec![0, 1, 2, 0, 3, 1], "m").unwrap();
        let g = UvGBuffer::new(&m, 0, 8, 8);
        assert_eq!(g.covered_count(), 64);
        assert_eq!(g.overlaps, 0);
        // A third triangle whose interior holds part of the diagonal overlaps both.
        let m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z], None, uv, vec![0, 1, 2, 0, 3, 1, 2, 3, 0], "m").unwrap();
        assert!(UvGBuffer::new(&m, 0, 8, 8).overlaps > 0);
        // Two stacked copies whose only covered center lies on their shared hypotenuse still overlap.
        let uv = vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)];
        let m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, uv, vec![0, 1, 2, 0, 1, 2], "m").unwrap();
        let g = UvGBuffer::new(&m, 0, 1, 1);
        assert_eq!(g.covered_count(), 1);
        assert_eq!(g.overlaps, 1);
        // Edges that cross at a shallow angle through the only center are not a shared edge.
        let uv = vec![Vec2::new(0.0, 0.5), Vec2::new(1.0, 0.5), Vec2::new(0.5, 1.0), Vec2::new(0.0, 0.49), Vec2::new(1.0, 0.51), Vec2::new(0.5, 0.0)];
        let m = Mesh::from_parts(vec![Vec3::ZERO; 6], None, uv, vec![0, 1, 2, 3, 4, 5], "m").unwrap();
        assert_eq!(UvGBuffer::new(&m, 0, 1, 1).overlaps, 1);
    }

    #[test]
    fn corners_meet_without_overlap_and_a_texel_counts_once() {
        // A 2×2 grid of quads at 3×3 puts the middle texel center on the shared grid vertex.
        let g = UvGBuffer::new(&crate::primitives::plane(2.0, 2), 0, 3, 3);
        assert_eq!(g.covered_count(), 9);
        assert_eq!(g.overlaps, 0);
        // Three stacked copies overlap on every covered texel, each counted once.
        let uv = vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)];
        let m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, uv, vec![0, 1, 2, 0, 1, 2, 0, 1, 2], "m").unwrap();
        let g = UvGBuffer::new(&m, 0, 8, 8);
        assert_eq!(g.overlaps as usize, g.covered_count());
        // Stacked copies whose only covered center is a shared corner overlap there.
        let uv = vec![Vec2::new(0.5, 0.5), Vec2::new(1.0, 0.5), Vec2::new(0.5, 1.0)];
        let m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, uv, vec![0, 1, 2, 0, 1, 2], "m").unwrap();
        let g = UvGBuffer::new(&m, 0, 1, 1);
        assert_eq!((g.covered_count(), g.overlaps), (1, 1));
    }

    #[test]
    fn a_triangle_listed_twice_is_rasterized_once() {
        let uv = vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)];
        let mut m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, uv, vec![0, 1, 2], "m").unwrap();
        let once = m.submeshes[0].clone();
        m.submeshes.push(once);
        let g = UvGBuffer::new(&m, 0, 8, 8);
        assert!(g.covered_count() > 0);
        assert_eq!(g.overlaps, 0);
    }

    #[test]
    fn large_finite_uvs_rasterize_without_overflow() {
        // Edge products of these UVs at 8 texels overflow f32.
        let uv = vec![Vec2::new(-1e37, -1e37), Vec2::new(3e37, -1e37), Vec2::new(-1e37, 3e37)];
        let m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, uv, vec![0, 1, 2], "m").unwrap();
        let g = UvGBuffer::new(&m, 0, 8, 8);
        assert_eq!(g.covered_count(), 64);
        for (i, [b1, b2]) in g.bary.iter().enumerate() {
            assert!(b1.is_finite() && b2.is_finite() && *b1 >= 0.0 && *b2 >= 0.0 && b1 + b2 <= 1.0, "texel {i}: {b1} {b2}");
        }
    }

    #[test]
    fn tiny_and_huge_triangles_keep_their_geometric_normal() {
        for s in [1e-20f32, 1e19] {
            let p = vec![Vec3::ZERO, Vec3::new(s, 0.0, 0.0), Vec3::new(0.0, 0.0, -s)];
            let mut m = Mesh::from_parts(p, None, vec![Vec2::ZERO, Vec2::X, Vec2::Y], vec![0, 1, 2], "m").unwrap();
            m.normals = vec![Vec3::X; 3];
            let surface = surface_at(&m, 0, 0.25, 0.25).unwrap();
            assert!((surface.geo_nrm - Vec3::Y).length() < 1e-6, "scale {s}: {}", surface.geo_nrm);
        }
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
