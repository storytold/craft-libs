//! Built-in meshes with clean, non-overlapping UVs: material ball, sphere, (rounded) cube,
//! cylinder, torus and plane. Y is up; triangles are counter-clockwise seen from outside.

use std::f32::consts::{PI, TAU};

use glam::{Vec2, Vec3};

use crate::mesh::{Mesh, SubMesh};

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
}

impl Builder {
    fn vert(&mut self, p: Vec3, n: Vec3, uv: Vec2) -> u32 {
        self.p.push(p);
        self.n.push(n.try_normalize().unwrap_or(Vec3::Y));
        self.uv.push(uv);
        (self.p.len() - 1) as u32
    }

    /// A grid of (rows+1)×(cols+1) vertices from `f(row, col)`, rows going "down" the texture.
    fn grid(&mut self, rows: usize, cols: usize, mut f: impl FnMut(usize, usize) -> (Vec3, Vec3, Vec2)) {
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
    fn finish(self, material: &str) -> Mesh {
        let mut m = self.into_mesh(material);
        m.compute_tangents();
        m
    }

    /// The mesh of one material, without tangents.
    fn into_mesh(self, material: &str) -> Mesh {
        let count = self.idx.len() as u32;
        Mesh {
            positions: self.p,
            normals: self.n,
            uvs: self.uv,
            indices: self.idx,
            tangents: Vec::new(),
            colors: Vec::new(),
            submeshes: vec![SubMesh { name: material.into(), material: 0, start: 0, count }],
            materials: vec![material.into()],
            diagonals: self.diag,
        }
    }
}

/// The angle around Y of grid column `j` of `segs`. The closing column (`j == segs`) takes the
/// first column's angle, so its vertices sit exactly on the first column's while its U is 1.
fn around(j: usize, segs: usize) -> f32 {
    (j % segs.max(1)) as f32 / segs.max(1) as f32 * TAU + PI * 1.5
}

/// UV sphere of `radius` centred at `center`.
pub fn sphere(radius: f32, segments: usize, rings: usize) -> Mesh {
    let mut b = Builder::default();
    sphere_into(&mut b, Vec3::ZERO, radius, segments.max(3), rings.max(2), UvRect::FULL);
    b.finish("Sphere")
}

fn sphere_into(b: &mut Builder, center: Vec3, radius: f32, segs: usize, rings: usize, r: UvRect) {
    b.grid(rings, segs, |i, j| {
        let (u, v) = (j as f32 / segs as f32, i as f32 / rings as f32);
        let (theta, phi) = (v * PI, around(j, segs));
        let n = Vec3::new(theta.sin() * phi.cos(), theta.cos(), -theta.sin() * phi.sin());
        (center + n * radius, n, r.map(u, v))
    });
}

/// A box with rounded edges (`bevel` = edge radius; 0 gives a plain cube). Faces are laid out
/// in a 3×2 UV atlas.
pub fn rounded_box(half: Vec3, bevel: f32) -> Mesh {
    let mut b = Builder::default();
    rounded_box_into(&mut b, Vec3::ZERO, half, bevel, UvRect::FULL);
    b.finish("Box")
}

/// The cube primitive (sharp edges).
pub fn cube(size: f32) -> Mesh {
    let mut m = rounded_box(Vec3::splat(size * 0.5), 0.0);
    m.materials = vec!["Cube".into()];
    if let Some(s) = m.submeshes.first_mut() {
        s.name = "Cube".into();
    }
    m
}

fn rounded_box_into(b: &mut Builder, center: Vec3, half: Vec3, bevel: f32, r: UvRect) {
    // `max`/`min` rather than `clamp`, which panics when a negative or NaN half-extent makes
    // the upper bound fall below the lower one.
    let bevel = bevel.max(0.0).min(half.min_element().max(0.0) * 0.95);
    // (normal, U = right, V = down) per face, seen from outside.
    let faces = [
        (Vec3::Z, Vec3::X, -Vec3::Y),
        (Vec3::X, -Vec3::Z, -Vec3::Y),
        (-Vec3::Z, -Vec3::X, -Vec3::Y),
        (-Vec3::X, Vec3::Z, -Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::Z),
        (-Vec3::Y, Vec3::X, -Vec3::Z),
    ];
    let (kb, kc) = if bevel > 0.0 { (6usize, 10usize) } else { (0, 12) };
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
            let dir = (q - clamped).try_normalize().unwrap_or(*n);
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
    let mut len = vec![0.0f32];
    for w in profile.windows(2) {
        let d = Vec2::new(w[1].0 - w[0].0, w[1].1 - w[0].1).length();
        len.push(len.last().copied().unwrap_or(0.0) + d);
    }
    let total = len.last().copied().unwrap_or(1.0).max(1e-6);
    let rows = profile.len() - 1;
    b.grid(rows, segs, |i, j| {
        let (rad, y) = profile[i];
        // Profile tangent (downwards) → outward normal in the (r, y) plane.
        let prev = profile[i.saturating_sub(1)];
        let next = profile[(i + 1).min(rows)];
        let t = Vec2::new(next.0 - prev.0, next.1 - prev.1).try_normalize().unwrap_or(Vec2::NEG_Y);
        let n2 = Vec2::new(-t.y, t.x);
        let u = j as f32 / segs as f32;
        let phi = around(j, segs);
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
        let (theta, phi) = (v * PI, around(j, ROWS * 2));
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

    let mut m = b.into_mesh("Preview");
    // Normals from the triangles: the panel walls are too steep for the sphere's directions.
    // Vertices at one position (the U seams' two columns, the poles) sum the faces of all of
    // them, so the shading has no seam.
    let key = |p: Vec3| p.to_array().map(|c| if c == 0.0 { 0 } else { c.to_bits() });
    let mut sums: std::collections::HashMap<[u32; 3], Vec3> = std::collections::HashMap::with_capacity(m.positions.len());
    for &[a, b, c] in m.indices.as_chunks::<3>().0 {
        let (Some(&pa), Some(&pb), Some(&pc)) = (m.positions.get(a as usize), m.positions.get(b as usize), m.positions.get(c as usize)) else {
            continue;
        };
        let face = (pb - pa).cross(pc - pa);
        for p in [pa, pb, pc] {
            *sums.entry(key(p)).or_default() += face;
        }
    }
    m.normals = m.positions.iter().map(|p| sums.get(&key(*p)).copied().unwrap_or_default().try_normalize().unwrap_or(Vec3::Y)).collect();
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

/// Distance from the centre of the [`preview_orb`] body along unit direction `d`.
fn preview_orb_radius(d: Vec3) -> f32 {
    // Rim blends: the sharp faces, and the softly blended left face
    const SHARP_BLEND: f32 = 0.035;
    const SOFT_BLEND: f32 = 0.18;
    // Domed panels: valley depth below the face, dome rise past it; the left panel is smaller
    const PANEL_DEPTH: f32 = 0.11;
    const PANEL_BULGE: f32 = 0.04;
    const LEFT_PANEL_SCALE: f32 = 0.9;
    // Slot around the middle: centre height, half-width, edge rounding, depth; it fades out over
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
    let mut b = Builder::default();
    let h = height * 0.5;
    lathe_into(&mut b, Vec3::ZERO, &[(radius, h), (radius, -h)], segs.max(3), UvRect { o: Vec2::ZERO, s: Vec2::new(1.0, 0.5) }.inset(0.01));
    disk_into(&mut b, Vec3::Y * h, radius, true, segs.max(3), UvRect { o: Vec2::new(0.0, 0.5), s: Vec2::splat(0.5) }.inset(0.01));
    disk_into(&mut b, -Vec3::Y * h, radius, false, segs.max(3), UvRect { o: Vec2::new(0.5, 0.5), s: Vec2::splat(0.5) }.inset(0.01));
    b.finish("Cylinder")
}

/// A torus around Y.
pub fn torus(major: f32, minor: f32, segs: usize, sides: usize) -> Mesh {
    let mut b = Builder::default();
    let (segs, sides) = (segs.max(3), sides.max(3));
    b.grid(sides, segs, |i, j| {
        let (u, v) = (j as f32 / segs as f32, i as f32 / sides as f32);
        let phi = around(j, segs);
        // The closing row takes the first row's angle as well.
        let theta = (i % sides) as f32 / sides as f32 * TAU;
        let dir = Vec3::new(phi.cos(), 0.0, -phi.sin());
        let n = dir * theta.cos() - Vec3::Y * theta.sin();
        (dir * major + n * minor, n, Vec2::new(u, v))
    });
    b.finish("Torus")
}

/// A subdivided plane facing +Y.
pub fn plane(size: f32, subdivisions: usize) -> Mesh {
    let mut b = Builder::default();
    let s = subdivisions.max(1);
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
    let mut ball = ball.finish("Ball");

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
    let base = base.finish("Base");
    ball.append(&base);
    ball
}

/// A small showcase scene: a beveled crate with a rounded lid and a cylinder handle, in three
/// materials (used for demo projects and tests of multiple texture sets).
pub fn crate_scene() -> Mesh {
    let mut body = Builder::default();
    rounded_box_into(&mut body, Vec3::new(0.0, 0.8, 0.0), Vec3::new(1.2, 0.8, 0.8), 0.12, UvRect::FULL);
    let mut m = body.finish("Crate");
    let mut lid = Builder::default();
    rounded_box_into(&mut lid, Vec3::new(0.0, 1.72, 0.0), Vec3::new(1.28, 0.12, 0.88), 0.08, UvRect::FULL);
    m.append(&lid.finish("Lid"));
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

    #[test]
    fn negative_or_nan_box_sizes_do_not_panic() {
        let _ = cube(-1.0);
        let _ = cube(f32::NAN);
        let _ = rounded_box(Vec3::new(1.0, f32::NAN, 1.0), 0.2);
        let _ = rounded_box(Vec3::ONE, f32::NAN);
    }

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

    /// Vertices of a grid's closing column (and closing row, when `wrap_rows`) sit exactly on the
    /// first column's (row's) positions and normals.
    fn closes_exactly(m: &Mesh, rows: usize, cols: usize, wrap_rows: bool) {
        let at = |i: usize, j: usize| i * (cols + 1) + j;
        for i in 0..=rows {
            let (a, b) = (at(i, 0), at(i, cols));
            assert_eq!((m.positions[a], m.normals[a]), (m.positions[b], m.normals[b]), "row {i}");
            assert!(m.uvs[a].x < m.uvs[b].x, "row {i}");
        }
        if wrap_rows {
            for j in 0..=cols {
                assert_eq!(m.positions[at(0, j)], m.positions[at(rows, j)], "column {j}");
            }
        }
    }

    #[test]
    fn closing_columns_match_the_first_column() {
        closes_exactly(&sphere(1.0, 7, 5), 5, 7, false);
        closes_exactly(&torus(1.0, 0.3, 7, 5), 5, 7, true);
        closes_exactly(&cylinder(1.0, 2.0, 7), 1, 7, false);
        closes_exactly(&preview_orb(), 300, 600, false);
    }

    #[test]
    fn material_ball_has_two_materials() {
        let m = material_ball();
        assert_eq!(m.materials, vec!["Ball".to_string(), "Base".to_string()]);
        assert!(m.triangles_of(0).count() > 0 && m.triangles_of(1).count() > 0);
    }
}
