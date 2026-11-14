//! Built-in meshes with clean, non-overlapping UVs: material ball, sphere, (rounded) cube,
//! cylinder, torus and plane. Y is up; triangles are counter-clockwise seen from outside.
//!
//! Each `try_` builder checks its sizes (finite and positive) and its segment counts against
//! [`Limits`] and returns an error instead of building. A size so large that a coordinate
//! overflows `f32` is an error too. The builders without `try_` clamp the segment counts to the
//! default [`Limits::max_segments`] and return an empty mesh for an invalid size.

use std::f32::consts::{PI, TAU};

use glam::{Vec2, Vec3};

use crate::mesh::{Mesh, SubMesh, unit};
use crate::{Limits, MeshOpsError, Result};

/// A UV-space rectangle: origin and size.
#[derive(Clone, Copy, Debug)]
struct UvRect {
    o: Vec2,
    s: Vec2,
}

impl UvRect {
    const FULL: UvRect = UvRect { o: Vec2::ZERO, s: Vec2::ONE };
    fn map(&self, u: f32, v: f32) -> Vec2 {
        self.o + self.s * Vec2::new(u, v)
    }
    fn inset(&self, m: f32) -> UvRect {
        UvRect { o: self.o + Vec2::splat(m), s: self.s - Vec2::splat(2.0 * m) }
    }
}

#[derive(Default)]
struct Builder {
    p: Vec<Vec3>,
    n: Vec<Vec3>,
    uv: Vec<Vec2>,
    idx: Vec<u32>,
    /// Per triangle: its [`Mesh::diagonals`] bits.
    diag: Vec<u8>,
    limits: Limits,
    /// Set when a part would pass the limits; later parts are skipped.
    error: Option<MeshOpsError>,
}

impl Builder {
    fn new(limits: &Limits) -> Builder {
        Builder { limits: *limits, ..Builder::default() }
    }

    /// Whether `vertices` and `triangles` more fit the limits and `u32` indices, with the memory
    /// for them reserved; records the error when they do not.
    fn room(&mut self, vertices: Option<usize>, triangles: Option<usize>) -> bool {
        if self.error.is_some() {
            return false;
        }
        let total_vertices = vertices.and_then(|v| v.checked_add(self.p.len())).unwrap_or(usize::MAX);
        let total_triangles = triangles.and_then(|t| t.checked_add(self.idx.len() / 3)).unwrap_or(usize::MAX);
        let error = if total_vertices > self.limits.max_vertices || u32::try_from(total_vertices).is_err() {
            Some(MeshOpsError::TooLarge { what: "vertex count", got: total_vertices as u64, limit: self.limits.max_vertices as u64 })
        } else if total_triangles > self.limits.max_triangles || total_triangles.checked_mul(3).and_then(|i| u32::try_from(i).ok()).is_none() {
            Some(MeshOpsError::TooLarge { what: "triangle count", got: total_triangles as u64, limit: self.limits.max_triangles as u64 })
        } else {
            // Both counts fit the limits, so they and three times the triangles are finite.
            let (v, t) = (vertices.unwrap_or(0), triangles.unwrap_or(0));
            let reserved = self.p.try_reserve(v).is_ok()
                && self.n.try_reserve(v).is_ok()
                && self.uv.try_reserve(v).is_ok()
                && self.idx.try_reserve(t * 3).is_ok()
                && self.diag.try_reserve(t).is_ok();
            (!reserved).then_some(MeshOpsError::OutOfMemory("primitive"))
        };
        self.error = error;
        self.error.is_none()
    }

    fn vert(&mut self, p: Vec3, n: Vec3, uv: Vec2) -> u32 {
        self.p.push(p);
        self.n.push(n.try_normalize().unwrap_or(Vec3::Y));
        self.uv.push(uv);
        (self.p.len() - 1) as u32
    }

    /// A grid of (rows+1)×(cols+1) vertices from `f(row, col)`, rows going "down" the texture.
    fn grid(&mut self, rows: usize, cols: usize, mut f: impl FnMut(usize, usize) -> (Vec3, Vec3, Vec2)) {
        let vertices = rows.checked_add(1).zip(cols.checked_add(1)).and_then(|(r, c)| r.checked_mul(c));
        if !self.room(vertices, rows.checked_mul(cols).and_then(|q| q.checked_mul(2))) {
            return;
        }
        let base = self.p.len() as u32;
        for i in 0..=rows {
            for j in 0..=cols {
                let (p, n, uv) = f(i, j);
                self.vert(p, n, uv);
            }
        }
        let w = (cols + 1) as u32;
        for i in 0..rows as u32 {
            for j in 0..cols as u32 {
                let a = base + i * w + j;
                let (b, c, d) = (a + 1, a + w, a + w + 1);
                self.idx.extend_from_slice(&[a, c, d, a, d, b]);
                // Edge d-a of the first triangle and a-d of the second split the quad.
                self.diag.extend_from_slice(&[0b100, 0b001]);
            }
        }
    }

    /// The mesh of one material, with tangents.
    fn finish(self, material: &str) -> Result<Mesh> {
        let mut m = self.into_mesh(material)?;
        m.try_compute_tangents()?;
        Ok(m)
    }

    /// The mesh of one material, without tangents.
    fn into_mesh(self, material: &str) -> Result<Mesh> {
        if let Some(e) = self.error {
            return Err(e);
        }
        // A finite size near f32::MAX can overflow where the builders add or scale coordinates.
        if !(self.p.iter().all(|p| p.is_finite()) && self.n.iter().all(|n| n.is_finite()) && self.uv.iter().all(|uv| uv.is_finite())) {
            return Err(MeshOpsError::Invalid("the primitive's coordinates overflow f32".into()));
        }
        let count = self.idx.len() as u32;
        Ok(Mesh {
            positions: self.p,
            normals: self.n,
            uvs: self.uv,
            indices: self.idx,
            tangents: Vec::new(),
            colors: Vec::new(),
            submeshes: vec![SubMesh { name: material.into(), material: 0, start: 0, count }],
            materials: vec![material.into()],
            diagonals: self.diag,
        })
    }
}

/// A mesh of one material with no vertices: what the builders without `try_` return for an
/// invalid size.
fn empty(material: &str) -> Mesh {
    Mesh { submeshes: vec![SubMesh { name: material.into(), material: 0, start: 0, count: 0 }], materials: vec![material.into()], ..Mesh::default() }
}

/// `value` when it is finite and greater than zero.
fn positive(what: &str, value: f32) -> Result<f32> {
    if value.is_finite() && value > 0.0 { Ok(value) } else { Err(MeshOpsError::Invalid(format!("{what} {value} is not a finite positive number"))) }
}

/// `count`, raised to `min`, when the raised count is within the segment limit.
fn segments(what: &'static str, count: usize, min: usize, limits: &Limits) -> Result<usize> {
    let count = count.max(min);
    crate::check_limit(what, count, limits.max_segments)?;
    Ok(count)
}

/// Segments along one axis of a box face: in each of the two bevels, and between them.
fn box_segments(bevel: f32) -> (usize, usize) {
    if bevel > 0.0 { (6, 10) } else { (0, 12) }
}

/// A segment count clamped to the default limit, for the builders without `try_`.
fn clamped(count: usize) -> usize {
    count.min(Limits::default().max_segments)
}

/// UV sphere of `radius` around the origin.
pub fn sphere(radius: f32, segments: usize, rings: usize) -> Mesh {
    try_sphere(radius, clamped(segments), clamped(rings), &Limits::default()).unwrap_or_else(|_| empty("Sphere"))
}

/// [`sphere`] with its parameters checked.
pub fn try_sphere(radius: f32, segs: usize, rings: usize, limits: &Limits) -> Result<Mesh> {
    let radius = positive("sphere radius", radius)?;
    let (segs, rings) = (segments("sphere segments", segs, 3, limits)?, segments("sphere rings", rings, 2, limits)?);
    let mut b = Builder::new(limits);
    sphere_into(&mut b, Vec3::ZERO, radius, segs, rings, UvRect::FULL);
    b.finish("Sphere")
}

fn sphere_into(b: &mut Builder, center: Vec3, radius: f32, segs: usize, rings: usize, r: UvRect) {
    b.grid(rings, segs, |i, j| {
        let (u, v) = (j as f32 / segs as f32, i as f32 / rings as f32);
        let (theta, phi) = (v * PI, u * TAU + PI * 1.5);
        let n = Vec3::new(theta.sin() * phi.cos(), theta.cos(), -theta.sin() * phi.sin());
        (center + n * radius, n, r.map(u, v))
    });
}

/// A box with rounded edges (`bevel` = edge radius; 0 gives a plain cube). Faces are laid out
/// in a 3×2 UV atlas.
pub fn rounded_box(half: Vec3, bevel: f32) -> Mesh {
    try_rounded_box(half, bevel, &Limits::default()).unwrap_or_else(|_| empty("Box"))
}

/// [`rounded_box`] with its parameters checked: every half size finite and positive, the bevel
/// finite (it is clamped to 0..0.95 of the smallest half size). A face has 12 segments along
/// each axis, or 22 with a bevel, which [`Limits::max_segments`] has to allow.
pub fn try_rounded_box(half: Vec3, bevel: f32, limits: &Limits) -> Result<Mesh> {
    for v in half.to_array() {
        positive("box half size", v)?;
    }
    if !bevel.is_finite() {
        return Err(MeshOpsError::Invalid(format!("box bevel {bevel} is not finite")));
    }
    let (in_bevel, between) = box_segments(bevel);
    crate::check_limit("box segments", 2 * in_bevel + between, limits.max_segments)?;
    let mut b = Builder::new(limits);
    rounded_box_into(&mut b, Vec3::ZERO, half, bevel, UvRect::FULL);
    b.finish("Box")
}

/// The cube primitive (sharp edges).
pub fn cube(size: f32) -> Mesh {
    try_cube(size, &Limits::default()).unwrap_or_else(|_| empty("Cube"))
}

/// [`cube`] with its size checked.
pub fn try_cube(size: f32, limits: &Limits) -> Result<Mesh> {
    let mut m = try_rounded_box(Vec3::splat(positive("cube size", size)? * 0.5), 0.0, limits)?;
    m.materials = vec!["Cube".into()];
    if let Some(s) = m.submeshes.first_mut() {
        s.name = "Cube".into();
    }
    Ok(m)
}

fn rounded_box_into(b: &mut Builder, center: Vec3, half: Vec3, bevel: f32, r: UvRect) {
    let bevel = bevel.clamp(0.0, half.min_element() * 0.95);
    // (normal, U = right, V = down) per face, seen from outside.
    let faces = [
        (Vec3::Z, Vec3::X, -Vec3::Y),
        (Vec3::X, -Vec3::Z, -Vec3::Y),
        (-Vec3::Z, -Vec3::X, -Vec3::Y),
        (-Vec3::X, Vec3::Z, -Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::Z),
        (-Vec3::Y, Vec3::X, -Vec3::Z),
    ];
    let (kb, kc) = box_segments(bevel);
    // Coordinates along one face axis from -h to h, dense in the bevels.
    let coords = |h: f32| -> Vec<f32> {
        let inner = h - bevel;
        let mut v = Vec::new();
        for i in 0..kb {
            v.push(-h + bevel * i as f32 / kb as f32);
        }
        for i in 0..=kc {
            v.push(-inner + 2.0 * inner * i as f32 / kc as f32);
        }
        for i in 1..=kb {
            v.push(inner + bevel * i as f32 / kb as f32);
        }
        v
    };
    let inner = half - Vec3::splat(bevel);
    for (f, (n, u, v)) in faces.iter().enumerate() {
        let cell = UvRect { o: r.map((f % 3) as f32 / 3.0, (f / 3) as f32 / 2.0), s: r.s * Vec2::new(1.0 / 3.0, 0.5) }.inset(0.01 * r.s.x);
        let (hu, hv, hn) = ((*u * half).abs().max_element(), (*v * half).abs().max_element(), (*n * half).abs().max_element());
        let (cu, cv) = (coords(hu), coords(hv));
        let (cols, rows) = (cu.len() - 1, cv.len() - 1);
        b.grid(rows, cols, |i, j| {
            let (a, c) = (cu[j], cv[i]);
            let q = *n * hn + *u * a + *v * c;
            let clamped = q.clamp(-inner, inner);
            // `unit` gives the same direction where the squared offset overflows, so faces that
            // share a bevel edge place its vertices at the same positions.
            let dir = unit(q - clamped).unwrap_or(*n);
            let p = if bevel > 0.0 { clamped + dir * bevel } else { q };
            let nn = if bevel > 0.0 { dir } else { *n };
            (center + p, nn, cell.map((a + hu) / (2.0 * hu), (c + hv) / (2.0 * hv)))
        });
    }
}

/// Surface of revolution of a profile `(radius, y)` listed top → bottom, mapped into `r`
/// (u around, v down the profile by arc length).
fn lathe_into(b: &mut Builder, center: Vec3, profile: &[(f32, f32)], segs: usize, r: UvRect) {
    if profile.len() < 2 {
        return;
    }
    // Arc lengths of the profile divided by its largest coordinate: the UVs of a uniformly scaled
    // profile stay the same, and the squared steps neither overflow nor round to zero.
    let largest = profile.iter().map(|(r, y)| r.abs().max(y.abs())).fold(0.0f32, f32::max);
    let divisor = if largest > 0.0 && largest.is_finite() { largest } else { 1.0 };
    let mut len = vec![0.0f32];
    for w in profile.windows(2) {
        let d = (Vec2::new(w[1].0, w[1].1) / divisor - Vec2::new(w[0].0, w[0].1) / divisor).length();
        len.push(len.last().copied().unwrap_or(0.0) + d);
    }
    // A profile of zero length maps every row to the top of `r`.
    let total = len.last().copied().filter(|t| *t > 0.0).unwrap_or(1.0);
    let rows = profile.len() - 1;
    b.grid(rows, segs, |i, j| {
        let (rad, y) = profile[i];
        // Profile tangent (downwards) → outward normal in the (r, y) plane.
        let prev = profile[i.saturating_sub(1)];
        let next = profile[(i + 1).min(rows)];
        let t = (Vec2::new(next.0, next.1) / divisor - Vec2::new(prev.0, prev.1) / divisor).try_normalize().unwrap_or(Vec2::NEG_Y);
        let n2 = Vec2::new(-t.y, t.x);
        let u = j as f32 / segs as f32;
        let phi = u * TAU + PI * 1.5;
        let dir = Vec3::new(phi.cos(), 0.0, -phi.sin());
        let p = center + dir * rad + Vec3::Y * y;
        let n = dir * n2.x + Vec3::Y * n2.y;
        (p, n, r.map(u, len[i] / total))
    });
}

/// [`preview_orb`]: half-size of the cube that cuts the unit sphere flat (each face rim is the
/// circle where they meet).
const ORB_CUT: f32 = 0.8;
/// [`preview_orb`] domed panels: where the valley sits, as a fraction of the face rim radius.
const ORB_PANEL_VALLEY: f32 = 0.88;

/// The preview orb of the smart mask and smart material previews: a sphere cut flat by a cube on six sides, on a thin round base
/// Each feature gives one kind of mask something to show: sharp face rims (edges), a softly
/// blended left face (soft edges), domed panels inside the front, left and top faces whose valleys
/// hold dirt (cavities), a slot around the middle (cavity), the base (thin part, the seam where it
/// meets the body) and plain rounded corners (gradients). Faces +Z (front), -X (left) and +Y (top)
/// carry the panels, so the shape is made to be seen from the front left, slightly from above.
/// The body takes the top 79 % of the UV square, the base the bottom 19 %.
pub fn preview_orb() -> Mesh {
    // Grid rows (columns: twice as many); fewer blur the panel walls, more cost bake time
    const ROWS: usize = 300;
    const BODY_UV_HEIGHT: f32 = 0.79;
    const BASE_UV: UvRect = UvRect { o: Vec2::new(0.0, 0.81), s: Vec2::new(1.0, 0.19) };
    const BASE_RADIUS: f32 = 0.95;
    const BASE_HALF_HEIGHT: f32 = 0.025;
    const BASE_Y: f32 = -0.6;
    const BASE_RIM_STEPS: usize = 12;
    const BASE_SEGMENTS: usize = 256;

    let mut b = Builder::default();
    b.grid(ROWS, ROWS * 2, |i, j| {
        let (u, v) = (j as f32 / (ROWS * 2) as f32, i as f32 / ROWS as f32);
        let (theta, phi) = (v * PI, u * TAU + PI * 1.5);
        let dir = Vec3::new(theta.sin() * phi.cos(), theta.cos(), -theta.sin() * phi.sin());
        (dir * preview_orb_radius(dir), dir, Vec2::new(u, v * BODY_UV_HEIGHT))
    });

    // Base: a thin disc with a rounded rim (a half circle from its top face to its bottom face)
    let mut profile = vec![(0.0, BASE_HALF_HEIGHT)];
    for k in 0..=BASE_RIM_STEPS {
        let a = k as f32 / BASE_RIM_STEPS as f32 * PI;
        profile.push((BASE_RADIUS - BASE_HALF_HEIGHT + BASE_HALF_HEIGHT * a.sin(), BASE_HALF_HEIGHT * a.cos()));
    }
    profile.push((0.0, -BASE_HALF_HEIGHT));
    lathe_into(&mut b, Vec3::Y * BASE_Y, &profile, BASE_SEGMENTS, BASE_UV);

    let Ok(mut m) = b.into_mesh("Preview") else { return empty("Preview") };
    // Normals from the triangles: the panel walls are too steep for the sphere's directions
    m.compute_normals();
    // Body and base both run U around the vertical axis, so the tangent is that direction (a
    // fraction of the cost of the general tangent pass on this many triangles)
    m.tangents = m
        .positions
        .iter()
        .zip(&m.normals)
        .map(|(p, n)| {
            let around = Vec3::new(p.z, 0.0, -p.x).try_normalize().unwrap_or(Vec3::X);
            (around - *n * n.dot(around)).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector()).extend(1.0)
        })
        .collect();
    m
}

/// Distance from the center of the [`preview_orb`] body along unit direction `d`.
fn preview_orb_radius(d: Vec3) -> f32 {
    // Rim blends: the sharp faces, and the softly blended left face
    const SHARP_BLEND: f32 = 0.035;
    const SOFT_BLEND: f32 = 0.18;
    // Domed panels: valley depth below the face, dome rise past it; the left panel is smaller
    const PANEL_DEPTH: f32 = 0.11;
    const PANEL_BULGE: f32 = 0.04;
    const LEFT_PANEL_SCALE: f32 = 0.9;
    // Slot around the middle: center height, half-width, edge rounding, depth; it fades out over
    // this fraction of the left panel's rim
    const SLOT_Y: f32 = -0.03;
    const SLOT_HALF_WIDTH: f32 = 0.05;
    const SLOT_ROUNDING: f32 = 0.015;
    const SLOT_DEPTH: f32 = 0.12;
    const SLOT_FADE: f32 = 0.04;

    let rim = (1.0 - ORB_CUT * ORB_CUT).sqrt() / ORB_CUT;
    let smooth = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    // Polynomial smooth minimum with blend radius k
    let smin = |a: f32, b: f32, k: f32| {
        let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
        b + (a - b) * h - k * h * (1.0 - h)
    };
    // Domed panel at `rho` (radius in face units) inside a face of rim `rim`: a steep wall from
    // the rim into a valley, then a dome rising slightly past the face over the middle
    let panel = |rho: f32, rim: f32| {
        let valley = rim * ORB_PANEL_VALLEY;
        if rho < valley {
            PANEL_BULGE - (PANEL_BULGE + PANEL_DEPTH) * (rho / valley).powi(3)
        } else {
            -PANEL_DEPTH * (1.0 - ((rho - valley) / (rim - valley)).min(1.0))
        }
    };
    let face_rho = |along: f32, u: f32, v: f32| (u * u + v * v).sqrt() / along.max(1e-3);

    // The cut: sharp rims except the softly blended left face
    let mut radius = 1.0f32;
    for (along, blend) in [(d.x, SHARP_BLEND), (-d.x, SOFT_BLEND), (d.y, SHARP_BLEND), (-d.y, SHARP_BLEND), (d.z, SHARP_BLEND), (-d.z, SHARP_BLEND)] {
        // Directions facing away from a face stay on the sphere (9 = far past it)
        let to_face = if along > 1e-3 { ORB_CUT / along } else { 9.0 };
        radius = smin(radius, to_face, blend);
    }

    let a = d.abs();
    let left_rim = rim * LEFT_PANEL_SCALE;
    let mut relief = 0.0;
    if d.z > 0.0 && a.z >= a.x && a.z >= a.y {
        relief += panel(face_rho(d.z, d.x, d.y), rim);
    }
    if d.x < 0.0 && a.x >= a.y && a.x >= a.z {
        relief += panel(face_rho(-d.x, d.y, d.z), left_rim);
    }
    if d.y > 0.0 && a.y >= a.x && a.y >= a.z {
        relief += panel(face_rho(d.y, d.x, d.z), rim);
    }

    // Slot around the middle, stopping at the left panel
    let in_left_panel =
        if d.x < 0.0 { 1.0 - smooth(left_rim * (1.0 - SLOT_FADE), left_rim * (1.0 + SLOT_FADE), face_rho(-d.x, d.y, d.z)) } else { 0.0 };
    let slot_band = 1.0 - smooth(SLOT_HALF_WIDTH - SLOT_ROUNDING, SLOT_HALF_WIDTH + SLOT_ROUNDING, (d.y - SLOT_Y).abs());
    relief -= SLOT_DEPTH * slot_band * (1.0 - in_left_panel);

    radius + relief
}

/// A flat disk facing up (+Y) or down, with planar UVs inside `r`.
fn disk_into(b: &mut Builder, center: Vec3, radius: f32, up: bool, segs: usize, r: UvRect) {
    if !b.room(segs.checked_add(1), Some(segs)) {
        return;
    }
    let n = if up { Vec3::Y } else { Vec3::NEG_Y };
    let c = b.vert(center, n, r.map(0.5, 0.5));
    let first = b.p.len() as u32;
    for j in 0..segs {
        let a = j as f32 / segs as f32 * TAU;
        let (x, z) = (a.cos(), a.sin());
        // Top view: +X right, +Z down the texture; bottom view mirrors X.
        let uv = if up { r.map(0.5 + x * 0.5, 0.5 + z * 0.5) } else { r.map(0.5 + x * 0.5, 0.5 - z * 0.5) };
        b.vert(center + Vec3::new(x, 0.0, z) * radius, n, uv);
    }
    for j in 0..segs as u32 {
        let (a, bb) = (first + j, first + (j + 1) % segs as u32);
        if up {
            b.idx.extend_from_slice(&[c, bb, a]);
        } else {
            b.idx.extend_from_slice(&[c, a, bb]);
        }
        b.diag.push(0);
    }
}

/// A capped cylinder along Y.
pub fn cylinder(radius: f32, height: f32, segs: usize) -> Mesh {
    try_cylinder(radius, height, clamped(segs), &Limits::default()).unwrap_or_else(|_| empty("Cylinder"))
}

/// [`cylinder`] with its parameters checked.
pub fn try_cylinder(radius: f32, height: f32, segs: usize, limits: &Limits) -> Result<Mesh> {
    let (radius, height) = (positive("cylinder radius", radius)?, positive("cylinder height", height)?);
    let segs = segments("cylinder segments", segs, 3, limits)?;
    let mut b = Builder::new(limits);
    let h = height * 0.5;
    lathe_into(&mut b, Vec3::ZERO, &[(radius, h), (radius, -h)], segs, UvRect { o: Vec2::ZERO, s: Vec2::new(1.0, 0.5) }.inset(0.01));
    disk_into(&mut b, Vec3::Y * h, radius, true, segs, UvRect { o: Vec2::new(0.0, 0.5), s: Vec2::splat(0.5) }.inset(0.01));
    disk_into(&mut b, -Vec3::Y * h, radius, false, segs, UvRect { o: Vec2::new(0.5, 0.5), s: Vec2::splat(0.5) }.inset(0.01));
    b.finish("Cylinder")
}

/// A torus around Y.
pub fn torus(major: f32, minor: f32, segs: usize, sides: usize) -> Mesh {
    try_torus(major, minor, clamped(segs), clamped(sides), &Limits::default()).unwrap_or_else(|_| empty("Torus"))
}

/// [`torus`] with its parameters checked.
pub fn try_torus(major: f32, minor: f32, segs: usize, sides: usize, limits: &Limits) -> Result<Mesh> {
    let (major, minor) = (positive("torus major radius", major)?, positive("torus minor radius", minor)?);
    let (segs, sides) = (segments("torus segments", segs, 3, limits)?, segments("torus sides", sides, 3, limits)?);
    let mut b = Builder::new(limits);
    b.grid(sides, segs, |i, j| {
        let (u, v) = (j as f32 / segs as f32, i as f32 / sides as f32);
        let phi = u * TAU + PI * 1.5;
        let theta = v * TAU;
        let dir = Vec3::new(phi.cos(), 0.0, -phi.sin());
        let n = dir * theta.cos() - Vec3::Y * theta.sin();
        (dir * major + n * minor, n, Vec2::new(u, v))
    });
    b.finish("Torus")
}

/// A subdivided plane facing +Y.
pub fn plane(size: f32, subdivisions: usize) -> Mesh {
    try_plane(size, clamped(subdivisions), &Limits::default()).unwrap_or_else(|_| empty("Plane"))
}

/// [`plane`] with its parameters checked.
pub fn try_plane(size: f32, subdivisions: usize, limits: &Limits) -> Result<Mesh> {
    let size = positive("plane size", size)?;
    let s = segments("plane subdivisions", subdivisions, 1, limits)?;
    let mut b = Builder::new(limits);
    b.grid(s, s, |i, j| {
        let (u, v) = (j as f32 / s as f32, i as f32 / s as f32);
        (Vec3::new((u - 0.5) * size, 0.0, (v - 0.5) * size), Vec3::Y, Vec2::new(u, v))
    });
    b.finish("Plane")
}

/// Two planes side by side on UV tiles 1001 and 1002 (UDIM): one material, 4 × 2 units.
pub fn udim_plane() -> Mesh {
    let mut m = plane(2.0, 16);
    let mut right = m.clone();
    for p in &mut m.positions {
        p.x -= 1.0;
    }
    for p in &mut right.positions {
        p.x += 1.0;
    }
    for uv in &mut right.uvs {
        uv.x += 1.0;
    }
    right.materials.clear();
    right.submeshes.clear();
    m.append(&right);
    let count = m.indices.len() as u32;
    if let Some(s) = m.submeshes.first_mut() {
        s.count = count;
    }
    m
}

/// The default project mesh: a ball on a beveled pedestal, two materials ("Ball", "Base").
pub fn material_ball() -> Mesh {
    let mut ball = Builder::default();
    sphere_into(&mut ball, Vec3::new(0.0, 1.32, 0.0), 1.0, 128, 64, UvRect::FULL);
    let Ok(mut ball) = ball.finish("Ball") else { return empty("Ball") };

    let mut base = Builder::default();
    let (r, top, bot, bev) = (1.05f32, 0.32f32, 0.0f32, 0.07f32);
    let mut profile = vec![(0.42, top + 0.06), (0.62, top + 0.06)];
    // Collar bevel and the ring the ball sits on, then the beveled drum.
    for k in 1..=6 {
        let a = k as f32 / 6.0 * PI * 0.5;
        profile.push((0.62 + 0.06 * a.sin(), top + 0.06 * a.cos()));
    }
    profile.push((r - bev, top));
    for k in 1..=6 {
        let a = k as f32 / 6.0 * PI * 0.5;
        profile.push((r - bev + bev * a.sin(), top - bev + bev * a.cos()));
    }
    profile.push((r, bot + bev));
    for k in 1..=6 {
        let a = k as f32 / 6.0 * PI * 0.5;
        profile.push((r - bev + bev * a.cos(), bot + bev - bev * a.sin()));
    }
    lathe_into(&mut base, Vec3::ZERO, &profile, 128, UvRect { o: Vec2::ZERO, s: Vec2::new(1.0, 0.5) }.inset(0.01));
    disk_into(&mut base, Vec3::new(0.0, top + 0.06, 0.0), 0.42, true, 128, UvRect { o: Vec2::new(0.0, 0.5), s: Vec2::splat(0.5) }.inset(0.01));
    disk_into(&mut base, Vec3::new(0.0, bot, 0.0), r - bev, false, 128, UvRect { o: Vec2::new(0.5, 0.5), s: Vec2::splat(0.5) }.inset(0.01));
    let Ok(base) = base.finish("Base") else { return ball };
    ball.append(&base);
    ball
}

/// A small showcase scene: a beveled crate with a rounded lid and a cylinder handle, in three
/// materials (used for demo projects and tests of multiple texture sets).
pub fn crate_scene() -> Mesh {
    let mut body = Builder::default();
    rounded_box_into(&mut body, Vec3::new(0.0, 0.8, 0.0), Vec3::new(1.2, 0.8, 0.8), 0.12, UvRect::FULL);
    let Ok(mut m) = body.finish("Crate") else { return empty("Crate") };
    let mut lid = Builder::default();
    rounded_box_into(&mut lid, Vec3::new(0.0, 1.72, 0.0), Vec3::new(1.28, 0.12, 0.88), 0.08, UvRect::FULL);
    if let Ok(lid) = lid.finish("Lid") {
        m.append(&lid);
    }
    let mut handle = torus(0.32, 0.07, 64, 24);
    for p in &mut handle.positions {
        *p = Vec3::new(p.x, p.z, -p.y) + Vec3::new(0.0, 1.84, 0.0);
    }
    for n in &mut handle.normals {
        *n = Vec3::new(n.x, n.z, -n.y);
    }
    handle.materials = vec!["Handle".into()];
    if let Some(s) = handle.submeshes.first_mut() {
        s.name = "Handle".into();
    }
    handle.compute_tangents();
    m.append(&handle);
    m
}

/// Built-in mesh by id (`builtin:<id>` in project files).
pub fn builtin(id: &str) -> Option<Mesh> {
    Some(match id {
        "material_ball" | "materialball" | "ball" => material_ball(),
        "sphere" => sphere(1.0, 128, 64),
        "cube" => cube(2.0),
        "rounded_cube" | "roundedcube" | "box" => rounded_box(Vec3::ONE, 0.15),
        "cylinder" => cylinder(1.0, 2.0, 96),
        "torus" => torus(1.0, 0.35, 128, 48),
        "plane" => plane(2.0, 16),
        "crate" => crate_scene(),
        "udim_plane" | "udimplane" => udim_plane(),
        // Not offered for new projects (not in BUILTINS): the smart mask preview session's mesh
        "preview_orb" => preview_orb(),
        _ => return None,
    })
}

/// Ids accepted by [`builtin`], in menu order.
pub const BUILTINS: [(&str, &str); 9] = [
    ("material_ball", "Material ball"),
    ("sphere", "Sphere"),
    ("cube", "Cube"),
    ("rounded_cube", "Rounded cube"),
    ("cylinder", "Cylinder"),
    ("torus", "Torus"),
    ("plane", "Plane"),
    ("crate", "Crate"),
    ("udim_plane", "UDIM plane"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every triangle's winding agrees with its vertex normals, UVs stay in 0..1 (u in 0..max_u),
    /// and the UV mapping is not mirrored (tangent handedness +1).
    fn check(m: &Mesh, max_u: f32) {
        m.validate().unwrap();
        let mut bad = 0;
        for t in 0..m.triangle_count() {
            let [a, b, c] = m.tri(t).unwrap();
            let face = (m.positions[b] - m.positions[a]).cross(m.positions[c] - m.positions[a]);
            if face.length() < 1e-9 {
                continue;
            }
            let n = m.normals[a] + m.normals[b] + m.normals[c];
            if face.dot(n) <= 0.0 {
                bad += 1;
            }
        }
        assert_eq!(bad, 0, "{} triangles wound against their normals", bad);
        for uv in &m.uvs {
            assert!((0.0..=max_u).contains(&uv.x) && (0.0..=1.0).contains(&uv.y), "{uv:?}");
        }
        let mirrored = m.tangents.iter().filter(|t| t.w < 0.0).count();
        assert!(mirrored * 50 < m.tangents.len().max(1), "{mirrored} mirrored tangents");
    }

    #[test]
    fn primitives_are_well_formed() {
        for (id, _) in BUILTINS {
            let m = builtin(id).unwrap();
            assert!(m.triangle_count() > 0, "{id}");
            check(&m, if id == "udim_plane" { 2.0 } else { 1.0 });
        }
        assert!(builtin("nope").is_none());
        assert_eq!(udim_plane().uv_tiles(0).into_iter().collect::<Vec<_>>(), vec![(0, 0), (1, 0)]);
    }

    #[test]
    fn preview_orb_is_well_formed_with_domed_panels() {
        let m = preview_orb();
        check(&m, 1.0);

        // Above the middle slot, the front panel's dome rises past the cut face and its valley
        // sits below it
        let front_depth = |rho: f32| {
            let d = Vec3::new(0.0, rho, 1.0).normalize();
            preview_orb_radius(d) * d.z
        };
        let rim = (1.0 - ORB_CUT * ORB_CUT).sqrt() / ORB_CUT;
        let (dome, valley) = (front_depth(0.2), front_depth(rim * ORB_PANEL_VALLEY));
        assert!(dome > ORB_CUT && valley < ORB_CUT - 0.05, "dome {dome}, valley {valley}");
    }

    #[test]
    fn material_ball_has_two_materials() {
        let m = material_ball();
        assert_eq!(m.materials, vec!["Ball".to_string(), "Base".to_string()]);
        assert!(m.triangles_of(0).count() > 0 && m.triangles_of(1).count() > 0);
    }

    #[test]
    fn primitives_reject_non_finite_parameters() {
        let l = Limits::default();
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(try_sphere(bad, 16, 8, &l).is_err(), "sphere {bad}");
            assert!(try_rounded_box(Vec3::new(1.0, bad, 1.0), 0.1, &l).is_err(), "box {bad}");
            assert!(try_rounded_box(Vec3::ONE, bad, &l).is_err(), "bevel {bad}");
            assert!(try_cube(bad, &l).is_err(), "cube {bad}");
            assert!(try_cylinder(bad, 1.0, 16, &l).is_err() && try_cylinder(1.0, bad, 16, &l).is_err(), "cylinder {bad}");
            assert!(try_torus(bad, 0.3, 16, 8, &l).is_err() && try_torus(1.0, bad, 16, 8, &l).is_err(), "torus {bad}");
            assert!(try_plane(bad, 4, &l).is_err(), "plane {bad}");
            for m in [sphere(bad, 16, 8), cube(bad), cylinder(1.0, bad, 16), torus(bad, 0.3, 16, 8), plane(bad, 4)] {
                assert_eq!(m.triangle_count(), 0);
                m.validate().unwrap();
            }
        }
    }

    #[test]
    fn box_rejects_negative_nan_and_zero_half_size() {
        let l = Limits::default();
        for half in [Vec3::new(-1.0, 1.0, 1.0), Vec3::new(1.0, 1.0, f32::NAN), Vec3::ZERO, Vec3::new(1.0, 0.0, 1.0)] {
            assert!(try_rounded_box(half, 0.0, &l).is_err(), "{half}");
            assert!(try_rounded_box(half, 0.2, &l).is_err(), "{half}");
            let m = rounded_box(half, 0.2);
            assert_eq!(m.triangle_count(), 0, "{half}");
        }
        for size in [-2.0, 0.0] {
            assert!(try_cube(size, &l).is_err(), "{size}");
            assert!(cube(size).uvs.iter().all(|uv| uv.is_finite()));
        }
        // A negative bevel is clamped to none, as before.
        assert_eq!(try_rounded_box(Vec3::ONE, -1.0, &l).unwrap(), try_rounded_box(Vec3::ONE, 0.0, &l).unwrap());
    }

    #[test]
    fn primitives_reject_sizes_that_overflow() {
        let l = Limits::default();
        // The torus's two radii and the box's doubled half size pass f32::MAX.
        assert!(try_torus(3e38, 3e38, 16, 8, &l).is_err());
        assert!(try_rounded_box(Vec3::splat(3e38), 0.0, &l).is_err());
        assert_eq!(torus(3e38, 3e38, 16, 8).triangle_count(), 0);
        // The cylinder's profile length is measured on the divided profile, so a height whose
        // length alone would pass f32::MAX builds.
        let tall = try_cylinder(1.0, 3e38, 16, &l).unwrap();
        tall.validate().unwrap();
        assert!(tall.uvs.iter().all(|uv| (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y)));
        // Large sizes that stay finite build.
        let large = [
            try_sphere(1e30, 16, 8, &l).unwrap(),
            try_cylinder(1e18, 1e18, 16, &l).unwrap(),
            try_cube(1e30, &l).unwrap(),
            try_torus(1e30, 1e29, 16, 8, &l).unwrap(),
            try_plane(1e30, 4, &l).unwrap(),
        ];
        for m in large {
            assert!(m.triangle_count() > 0);
            m.validate().unwrap();
        }
    }

    #[test]
    fn a_scaled_primitive_is_the_unit_primitive_scaled() {
        let l = Limits::default();
        let close = |a: &Mesh, b: &Mesh, scale: f32| {
            assert_eq!(a.positions.len(), b.positions.len());
            for (i, (p, q)) in a.positions.iter().zip(&b.positions).enumerate() {
                assert!((*p / scale - *q).length() < 1e-5, "position {i}: {} vs {q}", *p / scale);
            }
            for (i, (u, v)) in a.uvs.iter().zip(&b.uvs).enumerate() {
                assert!((*u - *v).length() < 1e-5, "UV {i}: {u} vs {v}");
            }
        };
        // Squared bevel offsets overflow at this size: adjacent faces still meet.
        close(&try_rounded_box(Vec3::splat(1e30), 1e29, &l).unwrap(), &try_rounded_box(Vec3::ONE, 0.1, &l).unwrap(), 1e30);
        // A profile far shorter than 1e-6 keeps its share of the UV height.
        close(&try_cylinder(1e-8, 1e-8, 16, &l).unwrap(), &try_cylinder(1.0, 1.0, 16, &l).unwrap(), 1e-8);
        close(&try_cylinder(1e-30, 2e-30, 16, &l).unwrap(), &try_cylinder(1.0, 2.0, 16, &l).unwrap(), 1e-30);
    }

    #[test]
    fn primitives_reject_oversized_segment_counts() {
        let l = Limits::default();
        assert!(try_sphere(1.0, l.max_segments + 1, 8, &l).is_err());
        assert!(try_sphere(1.0, 8, usize::MAX, &l).is_err());
        assert!(try_torus(1.0, 0.3, usize::MAX, 8, &l).is_err());
        assert!(try_cylinder(1.0, 1.0, usize::MAX, &l).is_err());
        assert!(try_plane(1.0, usize::MAX, &l).is_err());
        // Within the segment limit, the vertex limit still applies.
        let few = Limits { max_vertices: 100, ..l };
        assert!(matches!(try_plane(1.0, 10, &few), Err(MeshOpsError::TooLarge { .. })));
        assert!(try_plane(1.0, 8, &few).is_ok());
        // A count under the builder's minimum is raised first, and the raised count is checked.
        let two = Limits { max_segments: 2, ..l };
        assert!(try_sphere(1.0, 1, 2, &two).is_err(), "a sphere has at least 3 segments");
        assert!(try_plane(1.0, 0, &two).is_ok());
        // A box face has 12 segments along each axis, or 22 with a bevel.
        let eleven = Limits { max_segments: 11, ..l };
        assert!(try_cube(1.0, &eleven).is_err());
        let twelve = Limits { max_segments: 12, ..l };
        assert!(try_cube(1.0, &twelve).is_ok());
        assert!(try_rounded_box(Vec3::ONE, 0.1, &twelve).is_err());
        // The builders without `try_` clamp to the limit.
        assert_eq!(clamped(usize::MAX), l.max_segments);
        assert_eq!(clamped(7), 7);
    }
}
