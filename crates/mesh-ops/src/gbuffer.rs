//! The UV-space G-buffer: which triangle (and where on it) each texel of a texture set maps
//! to. Painting, compositing, baking and padding all work from this one buffer, so they agree
//! on which texel is which surface point.

use glam::{DVec2, Vec2, Vec3, Vec4};
use rayon::prelude::*;

use crate::Mesh;

mod angular;
use angular::VertexCoverage;

/// Texel not covered by any triangle.
pub const NO_TRI: u32 = u32::MAX;

const BAND: u32 = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct UvGBuffer {
    pub width: u32,
    pub height: u32,
    /// Triangle under each texel center, [`NO_TRI`] when uncovered.
    pub tri: Vec<u32>,
    /// Compact weights of triangle vertices 1 and 2. `barycentrics` computes all three in f64.
    pub bary: Vec<Vec2>,
    /// `raster_uvs` stores vertex IDs and UV corners for triangle IDs recorded in `tri`.
    pub raster_uvs: std::collections::HashMap<u32, [(usize, Vec2); 3]>,
    /// The count of texels covered by distinct triangles with overlapping interiors.
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

#[derive(Clone, Copy)]
pub(crate) struct Expansion<const N: usize> {
    parts: [f64; N],
    len: usize,
}

impl<const N: usize> Expansion<N> {
    pub(crate) fn new() -> Self {
        Self { parts: [0.0; N], len: 0 }
    }

    pub(crate) fn components(&self) -> &[f64] {
        &self.parts[..self.len]
    }

    fn add(&mut self, mut term: f64) {
        let mut kept = 0;
        for index in 0..self.len {
            let mut part = self.parts[index];
            if term.abs() < part.abs() {
                std::mem::swap(&mut term, &mut part);
            }
            let sum = term + part;
            let error = part - (sum - term);
            if error != 0.0 {
                self.parts[kept] = error;
                kept += 1;
            }
            term = sum;
        }
        self.len = kept;
        if term != 0.0 {
            self.parts[self.len] = term;
            self.len += 1;
        }
    }

    pub(crate) fn product(&mut self, left: f64, right: f64) {
        let high = left * right;
        self.add(high);
        self.add(left.mul_add(right, -high));
    }

    pub(crate) fn value(&self) -> f64 {
        if self.len == 0 {
            return 0.0;
        }
        let mut remaining = self.len - 1;
        let mut sum = self.parts[remaining];
        let mut error = 0.0;
        while remaining > 0 {
            remaining -= 1;
            let previous = sum;
            sum += self.parts[remaining];
            error = self.parts[remaining] - (sum - previous);
            if error != 0.0 {
                break;
            }
        }
        if remaining > 0 && ((error > 0.0 && self.parts[remaining - 1] > 0.0) || (error < 0.0 && self.parts[remaining - 1] < 0.0)) {
            let correction = error * 2.0;
            let rounded = sum + correction;
            if rounded - sum == correction {
                sum = rounded;
            }
        }
        sum
    }
}

fn edge_terms(a: DVec2, b: DVec2, p: DVec2) -> Expansion<12> {
    let mut result = Expansion::new();
    for (x, y) in [(b.x, p.y), (-b.x, a.y), (-a.x, p.y), (-b.y, p.x), (b.y, a.x), (a.y, p.x)] {
        result.product(x, y);
    }
    result
}

/// The edge function of `a → b` at `p`, `(b - a) × (p - a)`, rounded once from its expansion.
fn edge(a: DVec2, b: DVec2, p: DVec2) -> f64 {
    edge_terms(a, b, p).value()
}

pub(crate) fn face_normal(corners: [Vec3; 3]) -> glam::DVec3 {
    let corners = corners.map(|point| point.as_dvec3());
    glam::DVec3::from_array(std::array::from_fn(|axis| {
        let projected = corners.map(|point| DVec2::new(point[(axis + 1) % 3], point[(axis + 2) % 3]));
        edge(projected[0], projected[1], projected[2])
    }))
}

fn nearest_boundary_weights(corners: [DVec2; 3], p: DVec2) -> Option<([Expansion<16>; 3], f64)> {
    let dot = |a: DVec2, b: DVec2| {
        let mut result = Expansion::<16>::new();
        for (x, y) in [(p.x, b.x), (-p.x, a.x), (-a.x, b.x), (a.x, a.x), (p.y, b.y), (-p.y, a.y), (-a.y, b.y), (a.y, a.y)] {
            result.product(x, y);
        }
        result
    };
    let orientation = edge(corners[0], corners[1], corners[2]);
    if orientation == 0.0 || !orientation.is_finite() {
        return None;
    }
    let mut nearest_vertex = [true; 3];
    for (first, second) in [(0, 1), (1, 2), (2, 0)] {
        let (a, b) = (corners[first], corners[second]);
        let (from_a, from_b) = (dot(a, b), dot(b, a));
        let (past_a, past_b) = (from_a.value() > 0.0, from_b.value() > 0.0);
        nearest_vertex[first] &= !past_a;
        nearest_vertex[second] &= !past_b;
        if !past_a || !past_b || edge(a, b, p) * orientation.signum() > 0.0 {
            continue;
        }
        let mut weights = [Expansion::new(); 3];
        weights[first] = from_b;
        weights[second] = from_a;
        let mut sum = Expansion::<32>::new();
        for weight in [from_a, from_b] {
            for &part in &weight.parts[..weight.len] {
                sum.add(part);
            }
        }
        return Some((weights, sum.value()));
    }
    let nearest = nearest_vertex.into_iter().position(|candidate| candidate)?;
    let mut weights = [Expansion::new(); 3];
    weights[nearest].add(1.0);
    Some((weights, 1.0))
}

fn interpolate<const N: usize>(weights: &[Expansion<16>; 3], values: [[f64; N]; 3], denominator: f64) -> [f64; N] {
    std::array::from_fn(|axis| {
        let mut numerator = Expansion::<96>::new();
        for (weight, value) in weights.iter().zip(values) {
            for &part in &weight.parts[..weight.len] {
                numerator.product(part, value[axis]);
            }
        }
        numerator.value() / denominator
    })
}

struct BoundaryOwners {
    triangles: Vec<u32>,
    vertex: Option<VertexCoverage>,
}

impl BoundaryOwners {
    fn new(previous: u32, face: Option<[DVec2; 3]>, point: DVec2) -> Self {
        Self { triangles: vec![previous], vertex: face.and_then(|face| VertexCoverage::new(face, point)) }
    }

    fn insert(&mut self, triangle: u32, face: [DVec2; 3], point: DVec2, mut separate: impl FnMut(u32) -> bool) -> bool {
        let separated = match self.vertex.as_mut().and_then(|vertex| vertex.insert(face, point)) {
            Some(separated) => separated,
            None => {
                self.vertex = None;
                self.triangles.iter().all(|&owner| separate(owner))
            }
        };
        if separated {
            self.triangles.push(triangle);
        }
        separated
    }
}

impl UvGBuffer {
    /// Rasterize the triangles of material `material` at `width × height`.
    pub fn new(mesh: &Mesh, material: usize, width: u32, height: u32) -> UvGBuffer {
        let (w, h) = (width.max(1), height.max(1));
        let size = DVec2::new(f64::from(w), f64::from(h));
        let bands = h.div_ceil(BAND) as usize;
        let mut bins: Vec<Vec<u32>> = vec![Vec::new(); bands];
        let mut seen = std::collections::HashSet::new();
        for t in mesh.triangles_of(material) {
            if !seen.insert(t) {
                continue;
            }
            let Some([a, b, c]) = mesh.tri(t) else { continue };
            let (Some(ua), Some(ub), Some(uc)) = (mesh.uvs.get(a), mesh.uvs.get(b), mesh.uvs.get(c)) else { continue };
            let y0 = (f64::from(ua.y.min(ub.y).min(uc.y)) * size.y).floor().max(0.0) as i64;
            let y1 = (f64::from(ua.y.max(ub.y).max(uc.y)) * size.y).ceil().min(size.y) as i64;
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
        let mut bary = vec![Vec2::ZERO; n];
        let band_len = BAND as usize * w as usize;
        let overlaps: u32 = tri
            .par_chunks_mut(band_len)
            .zip(bary.par_chunks_mut(band_len))
            .enumerate()
            .map(|(bi, (tri, bary))| {
                let mut overlaps = 0u32;
                let mut overlapping = vec![false; tri.len()];
                let mut boundary_owners = std::collections::HashMap::<usize, BoundaryOwners>::new();
                let row0 = bi as u32 * BAND;
                let rows = (tri.len() as u32) / w;
                for &t in bins.get(bi).map(Vec::as_slice).unwrap_or(&[]) {
                    let Some([a, b, c]) = mesh.tri(t as usize) else { continue };
                    let (Some(&ua), Some(&ub), Some(&uc)) = (mesh.uvs.get(a), mesh.uvs.get(b), mesh.uvs.get(c)) else { continue };
                    let (pa, pb, pc) = (ua.as_dvec2() * size, ub.as_dvec2() * size, uc.as_dvec2() * size);
                    if !(pa.is_finite() && pb.is_finite() && pc.is_finite()) {
                        continue;
                    }
                    let (qa, qb, qc) = (pa, pb, pc);
                    let area = edge(qa, qb, qc);
                    if area == 0.0 || !area.is_finite() {
                        continue;
                    }
                    let x0 = pa.x.min(pb.x).min(pc.x).floor().max(0.0) as u32;
                    let x1 = (pa.x.max(pb.x).max(pc.x).ceil().min(f64::from(w)) as u32).min(w);
                    let y0 = (pa.y.min(pb.y).min(pc.y).floor().max(f64::from(row0)) as u32).max(row0);
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
                    let slack = (1e-6 + 8.0 * f64::EPSILON * size.max_element()).min(1e-3 * lowest);
                    let eps = lengths.map(|length| -slack * length);
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let d = DVec2::new(f64::from(x - x0), f64::from(y - y0));
                            let stepped = [0, 1, 2].map(|i| {
                                let dx = step[i].x * d.x;
                                let dy = step[i].y * d.y;
                                let value = base[i] + (dx + dy);
                                let uncertainty = 32.0 * f64::EPSILON * (base[i].abs() + dx.abs() + dy.abs());
                                (value, uncertainty)
                            });
                            if stepped.iter().enumerate().any(|(i, (value, uncertainty))| *value < eps[i] - uncertainty) {
                                continue;
                            }
                            let exact = stepped.iter().enumerate().any(|(i, (value, uncertainty))| {
                                value.abs() <= *uncertainty || (value - eps[i]).abs() <= *uncertainty || *uncertainty > area.abs() * 1e-12
                            });
                            let [w0, w1, w2] = if exact {
                                let point = DVec2::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
                                edges.map(|(a, b)| edge(a, b, point) * area.signum())
                            } else {
                                stepped.map(|(value, _)| value)
                            };
                            if w0 < eps[0] || w1 < eps[1] || w2 < eps[2] {
                                continue;
                            }
                            let compact = if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                                let point = DVec2::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
                                let Some((weights, denominator)) = nearest_boundary_weights([qa, qb, qc], point) else { continue };
                                Vec2::new((weights[1].value() / denominator) as f32, (weights[2].value() / denominator) as f32)
                            } else {
                                Vec2::new((w1 / area.abs()) as f32, (w2 / area.abs()) as f32)
                            };
                            let i = ((y - row0) * w + x) as usize;
                            if let (Some(slot), Some(bw)) = (tri.get_mut(i), bary.get_mut(i)) {
                                let separate_interiors = |previous| {
                                    let Some(previous) = mesh.tri(previous as usize) else { return false };
                                    let [Some(ua), Some(ub), Some(uc)] = previous.map(|v| mesh.uvs.get(v)) else { return false };
                                    let old = [ua, ub, uc].map(|uv| uv.as_dvec2() * size);
                                    let separates = |face: [DVec2; 3], other: [DVec2; 3]| {
                                        let positive = edge(face[0], face[1], face[2]) > 0.0;
                                        [(face[0], face[1]), (face[1], face[2]), (face[2], face[0])].into_iter().any(|(a, b)| {
                                            other.into_iter().all(|p| if positive { edge(a, b, p) <= 0.0 } else { edge(a, b, p) >= 0.0 })
                                        })
                                    };
                                    separates([qa, qb, qc], old) || separates(old, [qa, qb, qc])
                                };
                                if *slot != NO_TRI && !overlapping[i] {
                                    let point = DVec2::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
                                    let first_overlaps = !boundary_owners.contains_key(&i) && !separate_interiors(*slot);
                                    let separated = if first_overlaps {
                                        false
                                    } else {
                                        let owners = boundary_owners.entry(i).or_insert_with(|| {
                                            let face = mesh.tri(*slot as usize).and_then(|corners| {
                                                let [Some(a), Some(b), Some(c)] = corners.map(|v| mesh.uvs.get(v)) else { return None };
                                                Some([a, b, c].map(|uv| uv.as_dvec2() * size))
                                            });
                                            BoundaryOwners::new(*slot, face, point)
                                        });
                                        owners.insert(t, [qa, qb, qc], point, separate_interiors)
                                    };
                                    if !separated {
                                        overlaps += 1;
                                        overlapping[i] = true;
                                        boundary_owners.remove(&i);
                                    }
                                }
                                *slot = t;
                                *bw = compact;
                            }
                        }
                    }
                }
                overlaps
            })
            .sum();
        let mut raster_uvs = std::collections::HashMap::new();
        let mut previous = NO_TRI;
        for &t in &tri {
            if t == previous {
                continue;
            }
            previous = t;
            if t != NO_TRI
                && let std::collections::hash_map::Entry::Vacant(entry) = raster_uvs.entry(t)
                && let Some(vertices) = mesh.tri(t as usize)
            {
                entry.insert(vertices.map(|vertex| (vertex, mesh.uvs[vertex])));
            }
        }
        UvGBuffer { width: w, height: h, tri, bary, raster_uvs, overlaps }
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

    fn raster_weights(&self, i: usize) -> Option<(u32, [Expansion<16>; 3], f64)> {
        let t = *self.tri.get(i)?;
        if t == NO_TRI {
            return None;
        }
        let _ = self.bary.get(i)?;
        if self.width == 0 || self.height == 0 || i >= (self.width as usize).checked_mul(self.height as usize)? {
            return None;
        }
        let corners = *self.raster_uvs.get(&t)?;
        let size = DVec2::new(f64::from(self.width), f64::from(self.height));
        let [pa, pb, pc] = corners.map(|(_, uv)| uv.as_dvec2() * size);
        if ![pa, pb, pc].into_iter().all(|point| point.is_finite()) {
            return None;
        }
        let point = DVec2::new((i % self.width as usize) as f64 + 0.5, (i / self.width as usize) as f64 + 0.5);
        let area = edge(pa, pb, pc);
        if area == 0.0 || !area.is_finite() {
            return None;
        }
        let weights = [edge_terms(pb, pc, point), edge_terms(pc, pa, point), edge_terms(pa, pb, point)];
        if weights.iter().any(|weight| weight.value() * area.signum() < 0.0) {
            let (weights, denominator) = nearest_boundary_weights([pa, pb, pc], point)?;
            return Some((t, weights, denominator));
        }
        let weights = weights.map(|weight| {
            let mut wide = Expansion::<16>::new();
            for &part in &weight.parts[..weight.len] {
                wide.add(part);
            }
            wide
        });
        Some((t, weights, area))
    }

    /// The three weights describe texel `i` in the triangle's original corner order.
    /// An uncovered texel has no weights. Tolerated outside samples use the triangle boundary.
    pub fn barycentrics(&self, i: usize) -> Option<[f64; 3]> {
        let (_, weights, denominator) = self.raster_weights(i)?;
        Some(weights.map(|weight| weight.value() / denominator))
    }

    /// The surface at texel `i` uses its raster-time mapping and the mesh's current attributes.
    pub fn surface(&self, mesh: &Mesh, i: usize) -> Option<Surface> {
        let (t, weights, denominator) = self.raster_weights(i)?;
        let corners = *self.raster_uvs.get(&t)?;
        let current = mesh.tri(t as usize)?;
        if current[0] == current[1] || current[1] == current[2] || current[2] == current[0] {
            return None;
        }
        let [Some(a), Some(b), Some(c)] = current.map(|vertex| corners.iter().position(|(original, _)| *original == vertex)) else { return None };
        let weights = [weights[a], weights[b], weights[c]];
        surface_at_expanded(mesh, t, weights, denominator)
    }
}

/// Interpolate triangle `t` at barycentrics (weights of vertices 1 and 2).
pub fn surface_at(mesh: &Mesh, t: u32, b1: f32, b2: f32) -> Option<Surface> {
    if !b1.is_finite() || !b2.is_finite() {
        return None;
    }
    let mut weights = [Expansion::new(); 3];
    weights[0].add(1.0);
    weights[0].add(-f64::from(b1));
    weights[0].add(-f64::from(b2));
    weights[1].add(f64::from(b1));
    weights[2].add(f64::from(b2));
    if b1 >= 0.0 && b2 >= 0.0 && b1 + b2 == 1.0 && weights[0].value() < 0.0 {
        let major = if b1 <= b2 { 2 } else { 1 };
        let value = f64::from(b1.min(b2));
        weights[0] = Expansion::new();
        weights[major] = Expansion::new();
        weights[major].add(1.0);
        weights[major].add(-value);
    }
    surface_at_expanded(mesh, t, weights, 1.0)
}

fn surface_at_expanded(mesh: &Mesh, t: u32, weights: [Expansion<16>; 3], denominator: f64) -> Option<Surface> {
    let [a, b, c] = mesh.tri(t as usize)?;
    let p = |values: &[Vec3]| -> Option<glam::DVec3> {
        let [Some(a), Some(b), Some(c)] = [a, b, c].map(|vertex| values.get(vertex)) else { return None };
        Some(glam::DVec3::from_array(interpolate(&weights, [a, b, c].map(|value| value.to_array().map(f64::from)), denominator)))
    };
    let pos = p(&mesh.positions)?.as_vec3();
    if !pos.is_finite() {
        return None;
    }
    let nrm = p(&mesh.normals)?.try_normalize().map_or(Vec3::Y, |normal| normal.as_vec3());
    let normal = face_normal([*mesh.positions.get(a)?, *mesh.positions.get(b)?, *mesh.positions.get(c)?]);
    let geo_nrm = normal.try_normalize().map_or(nrm, |n| n.as_vec3());
    let [Some(ta), Some(tb), Some(tc)] = [a, b, c].map(|vertex| mesh.tangents.get(vertex)) else { return None };
    let tan = glam::DVec4::from_array(interpolate(&weights, [ta, tb, tc].map(|value| value.to_array().map(f64::from)), denominator));
    let [Some(ua), Some(ub), Some(uc)] = [a, b, c].map(|vertex| mesh.uvs.get(vertex)) else { return None };
    if [ua, ub, uc].iter().any(|uv| !uv.is_finite()) {
        return None;
    }
    let uv = DVec2::from_array(interpolate(&weights, [ua, ub, uc].map(|value| value.to_array().map(f64::from)), denominator)).as_vec2();
    if !uv.is_finite() {
        return None;
    }
    let normal = nrm.as_dvec3();
    let tdir = normal.cross(tan.truncate()).cross(normal).try_normalize().map_or_else(|| nrm.any_orthonormal_vector(), |tangent| tangent.as_vec3());
    let w = if tan.w < 0.0 { -1.0 } else { 1.0 };
    Some(Surface { pos, nrm, geo_nrm, tangent: tdir.extend(w), uv, tri: t })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn a_large_shared_vertex_fan_uses_bounded_face_comparisons() {
        let count = 10_000usize;
        for coordinate in [0.5f32, f32::from_bits(0.5f32.to_bits() + 1)] {
            let center = DVec2::splat(coordinate as f64);
            let sample = DVec2::splat(0.5);
            let ring: Vec<DVec2> = (0..count)
                .map(|i| {
                    let angle = i as f32 * std::f32::consts::TAU / count as f32;
                    (Vec2::splat(0.5) + Vec2::new(angle.cos(), angle.sin()) * 0.4).as_dvec2()
                })
                .collect();
            let faces: Vec<[DVec2; 3]> = (0..count).map(|i| [center, ring[i], ring[(i + 1) % count]]).collect();
            let mut owners = BoundaryOwners::new(0, Some(faces[0]), sample);
            let mut comparisons = 0usize;
            for index in (1..count).map(|i| i * 9973 % count) {
                assert!(owners.insert(index as u32, faces[index], sample, |_| {
                    comparisons += 1;
                    assert!(comparisons <= 4 * count, "shared-vertex fan used {comparisons} full-face comparisons");
                    true
                }));
            }
            let angular = owners.vertex.as_ref().unwrap().comparisons();
            assert!(angular <= 32 * count * (count.ilog2() as usize + 1), "shared-vertex fan used {angular} angular comparisons");
            let uv = std::iter::once(center.as_vec2()).chain(ring.iter().map(|point| point.as_vec2())).collect::<Vec<_>>();
            let positions = uv.iter().map(|point| point.extend(0.0)).collect();
            let indices = (0..count).map(|i| i * 9973 % count).flat_map(|i| [0, i as u32 + 1, ((i + 1) % count) as u32 + 1]).collect();
            let mesh = Mesh {
                positions,
                uvs: uv,
                indices,
                submeshes: vec![crate::SubMesh { name: "m".into(), material: 0, start: 0, count: (count * 3) as u32 }],
                ..Mesh::default()
            };
            let buffer = UvGBuffer::new(&mesh, 0, 1, 1);
            assert!(buffer.covered(0));
            assert_eq!(buffer.overlaps, 0);
        }
    }

    #[test]
    fn adjacent_uv_faces_cover_the_square_without_overlapping() {
        for indices in [vec![0, 1, 2, 0, 2, 3], vec![0, 2, 3, 0, 1, 2], vec![2, 1, 0, 3, 2, 0]] {
            let mesh = Mesh::from_parts(
                vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y],
                None,
                vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y],
                indices,
                "m",
            )
            .unwrap();
            let buffer = UvGBuffer::new(&mesh, 0, 8, 8);
            assert!(buffer.tri.iter().all(|&triangle| triangle != NO_TRI));
            assert_eq!(buffer.overlaps, 0);
        }
    }

    #[test]
    fn a_uv_vertex_shared_by_a_fan_is_not_an_overlap() {
        let uv = vec![Vec2::splat(0.5), Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y];
        let positions = uv.iter().map(|point| point.extend(0.0)).collect();
        let mesh = Mesh::from_parts(positions, None, uv, vec![0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1], "m").unwrap();
        let buffer = UvGBuffer::new(&mesh, 0, 1, 1);
        assert!(buffer.covered(0));
        assert_eq!(buffer.overlaps, 0);
    }

    #[test]
    fn the_first_vertex_weight_preserves_a_far_corners_offset() {
        let far = 2.0_f32.powi(60);
        let mut mesh = mesh_with_uvs([[far, far], [0.0, 0.0], [1.0, 0.0]]);
        mesh.positions = mesh.uvs.iter().map(|point| point.extend(0.0)).collect();
        let buffer = UvGBuffer::new(&mesh, 0, 8, 8);
        let surface = buffer.surface(&mesh, 1).unwrap();
        let expected = Vec2::new(0.1875, 0.0625);
        assert!((surface.pos.truncate() - expected).length() < 0.000001, "{}", surface.pos);
        assert!((surface.uv - expected).length() < 0.000001, "{}", surface.uv);
    }

    #[test]
    fn a_neighbor_between_duplicate_faces_keeps_the_overlap_count() {
        let build = |indices| {
            Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y], None, vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y], indices, "m")
                .unwrap()
        };
        let together = UvGBuffer::new(&build(vec![0, 1, 2, 0, 1, 2, 0, 2, 3]), 0, 8, 8);
        let separated = UvGBuffer::new(&build(vec![0, 1, 2, 0, 2, 3, 0, 1, 2]), 0, 8, 8);
        assert!(together.overlaps > 0);
        assert_eq!(separated.overlaps, together.overlaps);
        assert_eq!(separated.tri.iter().filter(|&&t| t != NO_TRI).count(), together.covered_count());
    }

    #[test]
    fn repeated_submesh_ranges_do_not_overlap_a_triangle_with_itself() {
        let mut mesh = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, vec![Vec2::ZERO, Vec2::X, Vec2::Y], vec![0, 1, 2], "m").unwrap();
        let first = UvGBuffer::new(&mesh, 0, 8, 8);
        assert!(first.covered_count() > 0);
        mesh.submeshes.push(mesh.submeshes[0].clone());
        let repeated = UvGBuffer::new(&mesh, 0, 8, 8);
        assert_eq!(repeated.overlaps, 0);
        assert_eq!(repeated.covered_count(), first.covered_count());
    }

    #[test]
    fn repeated_faces_count_each_overlapping_texel_once() {
        let mesh =
            Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, vec![Vec2::ZERO, Vec2::X, Vec2::Y], vec![0, 1, 2, 0, 1, 2, 0, 1, 2], "m")
                .unwrap();
        let buffer = UvGBuffer::new(&mesh, 0, 8, 8);
        assert!(buffer.covered_count() > 0);
        assert_eq!(buffer.overlaps as usize, buffer.covered_count());
    }

    #[test]
    fn stored_weights_preserve_the_local_surface_offset() {
        let mut mesh = mesh_with_uvs([[-1_073_741_824.0, 0.0], [1_073_741_824.0, 0.0], [-1_073_741_824.0, 1.0]]);
        mesh.positions = mesh.uvs.iter().map(|uv| uv.extend(0.0)).collect();
        let buffer = UvGBuffer::new(&mesh, 0, 8, 2);
        let surface = buffer.surface(&mesh, 0).unwrap();
        assert!((surface.pos.x - 0.0625).abs() < 0.000001, "{}", surface.pos);
        assert!((surface.uv.x - 0.0625).abs() < 0.000001, "{}", surface.uv);
    }

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
        for m in [1_048_576.0_f32, 1e15, 1.25e17, 1e30, f32::MAX / 4.0] {
            // Covers u + v <= 0: no texel center of an 8 x 8 buffer is inside it.
            let g = UvGBuffer::new(&mesh_with_uvs([[-m, -m], [m, -m], [-m, m]]), 0, 8, 8);
            assert_eq!(g.covered_count(), 0, "{m}");
            // The mirrored triangle covers u + v >= 0, which is every texel center.
            let g = UvGBuffer::new(&mesh_with_uvs([[m, m], [-m, m], [m, -m]]), 0, 8, 8);
            assert_eq!(g.covered_count(), 64, "{m}");
            assert!(g.bary.iter().all(|weights| weights.to_array().iter().all(|weight| (0.0..=1.0).contains(weight))), "{m}: {:?}", g.bary);
        }
        // A sliver 0.0125 texel high, 2e14 texels wide: rounding at those coordinates does not
        // widen it to the texel centers at y = 0.5.
        let g = UvGBuffer::new(&mesh_with_uvs([[-1e14, 0.0], [1e14, 0.0], [0.0, 0.0125]]), 0, 8, 8);
        assert_eq!(g.covered_count(), 0);
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
        // All cyclic orders retain the same face normal near a far corner.
        let far = 2.0_f32.powi(60);
        for positions in [
            vec![Vec3::new(far, far, 0.0), Vec3::ZERO, Vec3::X],
            vec![Vec3::ZERO, Vec3::X, Vec3::new(far, far, 0.0)],
            vec![Vec3::X, Vec3::new(far, far, 0.0), Vec3::ZERO],
        ] {
            let m = Mesh { positions, ..m.clone() };
            let surface = surface_at(&m, 0, 0.25, 0.25).unwrap();
            assert!(surface.geo_nrm.z > 0.999, "{}", surface.geo_nrm);
        }
        // Collinear corners still fall back to the shading normal.
        let m = Mesh { positions: vec![Vec3::ZERO, Vec3::X, Vec3::X * 2.0], ..m };
        assert_eq!(surface_at(&m, 0, 0.25, 0.25).unwrap().geo_nrm, Vec3::X);
    }

    #[test]
    fn face_normal_preserves_a_short_edge_far_from_the_origin() {
        let far = 2.0_f32.powi(60);
        let corners = [Vec3::new(0.0, -far, 0.0), Vec3::new(far, 1.0, 0.0), Vec3::new(far, 2.0, 0.0)];
        for first in 0..3 {
            let mesh = Mesh {
                positions: (0..3).map(|offset| corners[(first + offset) % 3]).collect(),
                normals: vec![Vec3::X; 3],
                tangents: vec![Vec4::new(0.0, 1.0, 0.0, 1.0); 3],
                uvs: vec![Vec2::ZERO, Vec2::X, Vec2::Y],
                indices: vec![0, 1, 2],
                ..Mesh::default()
            };
            let surface = surface_at(&mesh, 0, 0.25, 0.25).unwrap();
            assert!(surface.geo_nrm.z > 0.999, "{first}: {}", surface.geo_nrm);
        }
    }

    #[test]
    fn projected_normals_preserve_cancellation_across_coordinate_scales() {
        let small = 2.0_f32.powi(-60);
        let far = 2.0_f32.powi(60);
        let height = 2.0_f32.powi(-120);
        let corners = [Vec3::new(1.0, 1.0, 0.0), Vec3::new(small, small, 0.0), Vec3::new(far, far, height)];
        let expected = Vec3::new(-1.0, 1.0, 0.0).normalize();
        for first in 0..3 {
            let mesh = Mesh {
                positions: (0..3).map(|offset| corners[(first + offset) % 3]).collect(),
                normals: vec![Vec3::X; 3],
                tangents: vec![Vec4::new(0.0, 1.0, 0.0, 1.0); 3],
                uvs: vec![Vec2::ZERO, Vec2::X, Vec2::Y],
                indices: vec![0, 1, 2],
                ..Mesh::default()
            };
            let surface = surface_at(&mesh, 0, 0.25, 0.25).unwrap();
            assert!(surface.geo_nrm.dot(expected) > 0.999, "{first}: {}", surface.geo_nrm);
            let projected = mesh.positions.iter().map(|point| point.truncate().as_dvec2()).collect::<Vec<_>>();
            assert_eq!(edge(projected[0], projected[1], projected[2]), 0.0);
        }
    }

    #[test]
    fn surface_tangents_are_perpendicular_to_interpolated_normals() {
        let mut mesh = mesh_with_uvs([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        mesh.normals = vec![Vec3::Z, Vec3::X, Vec3::Z];
        mesh.tangents = [Vec3::X, Vec3::Y, Vec3::X].map(|tangent| tangent.extend(1.0)).to_vec();
        let surface = surface_at(&mesh, 0, 0.5, 0.25).unwrap();
        assert!(surface.nrm.dot(surface.tangent.truncate()).abs() < 1e-6);
        assert!((surface.tangent.truncate().length() - 1.0).abs() < 1e-6);
        assert_eq!(surface.tangent.w, 1.0);
        mesh.normals.fill(Vec3::Z);
        mesh.tangents.fill(Vec3::Z.extend(-1.0));
        let surface = surface_at(&mesh, 0, 0.5, 0.25).unwrap();
        assert!(surface.nrm.dot(surface.tangent.truncate()).abs() < 1e-6);
        assert_eq!(surface.tangent.w, -1.0);
    }

    #[test]
    fn surface_frames_preserve_large_finite_input_directions() {
        let mut mesh = mesh_with_uvs([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        mesh.normals.fill(Vec3::X * 1e30);
        mesh.tangents.fill((Vec3::Y * 1e30).extend(-1.0));
        let surface = surface_at(&mesh, 0, 0.5, 0.25).unwrap();
        assert_eq!(surface.nrm, Vec3::X);
        assert_eq!(surface.tangent, Vec3::Y.extend(-1.0));
    }

    #[test]
    fn raster_surface_preserves_local_offsets_between_far_uv_corners() {
        for far in [2.0_f32.powi(60), f32::MAX / 4.0] {
            let mut mesh = mesh_with_uvs([[-far, 0.0], [far, 0.0], [-far, 1.0]]);
            mesh.positions = mesh.uvs.iter().map(|uv| uv.extend(0.0)).collect();
            let buffer = UvGBuffer::new(&mesh, 0, 8, 2);
            let surface = buffer.surface(&mesh, 0).unwrap();
            let expected = Vec2::new(0.0625, 0.25);
            assert_eq!(surface.uv, expected, "{far}");
            assert_eq!(surface.pos.truncate(), expected, "{far}");
        }
    }

    #[test]
    fn raster_mapping_stays_fixed_when_mesh_uvs_change() {
        let mut mesh = mesh_with_uvs([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        let buffer = UvGBuffer::new(&mesh, 0, 8, 8);
        let before = buffer.surface(&mesh, 0).unwrap();
        mesh.uvs.fill(Vec2::ZERO);
        assert!(buffer.covered(0));
        let after = buffer.surface(&mesh, 0).expect("the stored triangle mapping still exists");
        assert_eq!(after.pos, before.pos);
        assert_eq!(after.nrm, before.nrm);
        assert_eq!(after.uv, Vec2::ZERO);
    }

    #[test]
    fn raster_mapping_follows_cyclic_corner_reordering() {
        let mut mesh = mesh_with_uvs([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        let buffer = UvGBuffer::new(&mesh, 0, 8, 8);
        let before = buffer.surface(&mesh, 0).unwrap();
        mesh.indices[..3].rotate_left(1);
        let after = buffer.surface(&mesh, 0).unwrap();
        assert_eq!(after.pos, before.pos);
        assert_eq!(after.uv, before.uv);
        mesh.indices[0] = mesh.indices[1];
        assert!(buffer.surface(&mesh, 0).is_none());
    }

    #[test]
    fn raster_uv_cache_is_bounded_by_covered_texels() {
        let mesh =
            Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, vec![Vec2::ZERO, Vec2::X, Vec2::Y], [0, 1, 2].repeat(1000), "m").unwrap();
        let buffer = UvGBuffer::new(&mesh, 0, 1, 1);
        assert!(buffer.raster_uvs.len() <= buffer.covered_count());
        for (i, &triangle) in buffer.tri.iter().enumerate().filter(|(_, triangle)| **triangle != NO_TRI) {
            assert!(buffer.raster_uvs.contains_key(&triangle));
            assert!(buffer.surface(&mesh, i).is_some());
        }
    }

    #[test]
    fn surface_at_keeps_positive_interior_corner_weights() {
        let mut mesh = mesh_with_uvs([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        mesh.positions = vec![Vec3::X * 2.0_f32.powi(25), Vec3::ZERO, Vec3::Y];
        let surface = surface_at(&mesh, 0, 0.5, 0.5 - 2.0_f32.powi(-25)).unwrap();
        assert_eq!(surface.pos.x, 1.0);
        mesh.positions = vec![Vec3::X * 2.0_f32.powi(60), Vec3::ZERO, Vec3::X * -2.0_f32.powi(60)];
        let small = surface_at(&mesh, 0, 2.0_f32.powi(-60), 0.5).unwrap();
        assert_eq!(small.pos.x, -1.0);
    }

    #[test]
    fn surface_at_retains_edge_complements_below_double_precision() {
        let mut mesh = mesh_with_uvs([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        mesh.positions = vec![Vec3::ZERO, Vec3::new(1.0, -2.0_f32.powi(40), 0.0), Vec3::Y * 2.0_f32.powi(100)];
        let surface = surface_at(&mesh, 0, 1.0, 2.0_f32.powi(-60)).unwrap();
        assert_eq!(surface.pos.y, 2.0_f32.powi(-20));
        mesh.indices.swap(1, 2);
        let swapped = surface_at(&mesh, 0, 2.0_f32.powi(-60), 1.0).unwrap();
        assert_eq!(swapped.pos.y, surface.pos.y);
    }

    #[test]
    fn raster_weights_keep_the_original_compact_storage_budget() {
        let mesh = mesh_with_uvs([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        let buffer = UvGBuffer::new(&mesh, 0, 2, 2);
        assert!(std::mem::size_of_val(buffer.bary.as_slice()) <= 4 * 8);
    }

    #[test]
    fn tolerated_outside_texels_sample_a_finite_triangle_boundary() {
        let base = 0.5000005;
        let mut mesh = mesh_with_uvs([[0.0, base], [1.0, base], [0.5, 0.6]]);
        mesh.positions = vec![Vec3::new(f32::MAX, 0.0, 0.0), Vec3::new(f32::MAX, 1.0, 0.0), Vec3::new(-f32::MAX, 0.0, 0.0)];
        let buffer = UvGBuffer::new(&mesh, 0, 1, 1);
        assert!(buffer.covered(0));
        let surface = buffer.surface(&mesh, 0).expect("a tolerated covered texel samples the triangle boundary");
        assert!(surface.pos.is_finite());
        assert_eq!(surface.pos.x, f32::MAX);
        assert!((surface.pos.y - 0.5).abs() < 1e-6);
    }

    #[test]
    fn diagonal_boundary_texels_remain_covered_across_raster_bands() {
        let offset = 2.0_f32.powi(-50);
        let mesh = mesh_with_uvs([[offset, offset], [1.0, 1.0], [0.0, 1.0]]);
        let buffer = UvGBuffer::new(&mesh, 0, 64, 64);
        for coordinate in 0..64 {
            let texel = coordinate * 65;
            assert!(buffer.covered(texel), "diagonal texel {coordinate}");
            assert_eq!(buffer.barycentrics(texel).unwrap()[2], 0.0);
        }
    }

    #[test]
    fn closest_boundary_projection_preserves_a_small_weight_at_an_endpoint() {
        let mut mesh = mesh_with_uvs([[0.5, 0.5 + 2.0_f32.powi(-24)], [1.0, 1.0], [-15.5, 0.5]]);
        mesh.positions = vec![Vec3::ZERO, Vec3::X, Vec3::Y];
        let first = UvGBuffer::new(&mesh, 0, 1, 1).surface(&mesh, 0).unwrap();
        assert!(first.pos.y > 0.0);
        mesh.indices.rotate_left(1);
        let rotated = UvGBuffer::new(&mesh, 0, 1, 1).surface(&mesh, 0).unwrap();
        assert_eq!(first.pos, rotated.pos);
    }

    #[test]
    fn tolerated_outside_samples_choose_the_closest_nearly_coincident_edge() {
        let mut mesh = mesh_with_uvs([[0.0, 2.0_f32.powi(-60)], [2.0_f32.powi(60), 2.0_f32.powi(60)], [0.0, 2.0_f32.powi(-71)]]);
        mesh.positions = vec![Vec3::ZERO, Vec3::X, Vec3::Y];
        let buffer = UvGBuffer::new(&mesh, 0, 1, 1);
        assert!(buffer.covered(0));
        assert!(buffer.surface(&mesh, 0).unwrap().pos.y > 0.99);
    }

    #[test]
    fn tolerated_outside_samples_do_not_slide_along_a_thin_edge() {
        let base = 0.5 + 2.0_f32.powi(-24);
        let mut mesh = mesh_with_uvs([[0.0, base], [1.0, base], [1.0, base + 2.0_f32.powi(-14)]]);
        mesh.positions = vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0)];
        let buffer = UvGBuffer::new(&mesh, 0, 8192, 1);
        let texel = 4096;
        assert!(buffer.covered(texel));
        let expected = (texel as f32 + 0.5) / 8192.0;
        let surface = buffer.surface(&mesh, texel).unwrap();
        assert!((surface.pos.x - expected).abs() < 1e-7, "{} != {expected}", surface.pos.x);
        assert!((buffer.bary[texel].x - expected).abs() < 1e-7);
        assert_eq!(surface.pos.y, 0.0);
    }

    #[test]
    fn thin_interior_raster_weights_keep_their_proportions() {
        let height = 2.0_f32.powi(-43) + 2.0_f32.powi(-53);
        let mesh = mesh_with_uvs([[0.0, -height / 2.0], [1.0, 1.0], [0.0, height / 2.0]]);
        let buffer = UvGBuffer::new(&mesh, 0, 2, 2);
        assert!(buffer.covered(3));
        assert!((buffer.bary[3].x - 0.75).abs() < 1e-6);
        assert!((buffer.bary[3].y - 0.125).abs() < 1e-6);
        for (actual, expected) in buffer.barycentrics(3).unwrap().into_iter().zip([0.125, 0.75, 0.125]) {
            assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
        }
    }

    #[test]
    fn fallback_raster_weights_keep_a_unit_sum() {
        let low = -2.0_f32.powi(-56);
        let high = 2.0_f32.powi(-43) + low;
        let far = 2.0_f32.powi(51);
        let mesh = mesh_with_uvs([[0.0, low], [far, far], [0.0, high]]);
        let buffer = UvGBuffer::new(&mesh, 0, 64, 64);
        let texel = 32 * 65;
        assert!(buffer.covered(texel));
        let sum: f64 = buffer.barycentrics(texel).unwrap().iter().sum();
        assert!((sum - 1.0).abs() < 1e-12, "{sum}");
    }

    #[test]
    fn raster_stepping_preserves_tiny_offsets_from_a_far_diagonal() {
        let tiny = 2.0_f32.powi(-60);
        let far = 2.0_f32.powi(60);
        let mesh = mesh_with_uvs([[0.0, tiny], [far, far], [0.0, tiny * 2.0]]);
        let buffer = UvGBuffer::new(&mesh, 0, 64, 64);
        for coordinate in 0..64 {
            assert!(!buffer.covered(coordinate * 65), "diagonal texel {coordinate}");
        }
    }

    #[test]
    fn a_small_nonzero_uv_island_covers_its_interior_texel_center() {
        let start = 0.5 - 2.0_f32.powi(-22);
        let length = 2.0_f32.powi(-20);
        let mesh = mesh_with_uvs([[start, start], [start + length, start], [start, start + length]]);
        let buffer = UvGBuffer::new(&mesh, 0, 1, 1);
        assert!(buffer.covered(0));
        assert!(buffer.surface(&mesh, 0).is_some());
    }

    #[test]
    fn surface_at_preserves_float_edge_weights_and_small_endpoint_offsets() {
        for far in [1.0, 2.0_f32.powi(32)] {
            for (height, u, v, expected_y) in [(1280.0, 0.2, 0.8, 1024.0), (67108864.0, 1.0, 2.0_f32.powi(-26), 1.0)] {
                let mut mesh = mesh_with_uvs([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
                mesh.positions = vec![Vec3::new(-far, 0.0, 0.0), Vec3::ZERO, Vec3::new(0.0, height, 0.0)];
                let surface = surface_at(&mesh, 0, u, v).unwrap();
                assert_eq!(surface.pos.x, 0.0, "{far} {height}");
                assert!((surface.pos.y - expected_y).abs() < 0.001, "{far} {height}");
            }
        }
    }

    #[test]
    fn surface_rejects_positions_that_overflow_during_interpolation() {
        let mut mesh = mesh_with_uvs([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        mesh.positions = vec![Vec3::new(-f32::MAX, 0.0, 0.0), Vec3::new(f32::MAX, 0.0, 0.0), Vec3::new(f32::MAX, 1.0, 0.0)];
        assert!(surface_at(&mesh, 0, 0.625, 0.5).is_none());
    }

    #[test]
    fn surface_rejects_uvs_that_overflow_during_interpolation() {
        let mesh = mesh_with_uvs([[-f32::MAX, 0.0], [f32::MAX, 0.0], [f32::MAX, 1.0]]);
        assert!(surface_at(&mesh, 0, 0.625, 0.5).is_none());
    }

    #[test]
    fn raster_surface_rejects_non_finite_current_uvs() {
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut mesh = mesh_with_uvs([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
            let buffer = UvGBuffer::new(&mesh, 0, 8, 8);
            mesh.uvs[0].x = invalid;
            assert!(buffer.surface(&mesh, 0).is_none(), "{invalid}");
        }
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
