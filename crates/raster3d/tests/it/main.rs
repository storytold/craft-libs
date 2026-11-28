//! Integration tests: one test binary for the crate, one module per area.
// Unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod bounds;
mod differential;
mod moved;

/// A verbatim copy of the app's previous code, run by `differential`.
mod reference {
    pub mod effectcraft;
}

use craft_raster3d::{EnvInfo, Light, Limits, Material, Scene, TexInfo, Vertex};

/// The default limits with `f` applied.
pub fn limits(f: impl FnOnce(&mut Limits)) -> Limits {
    let mut l = Limits::default();
    f(&mut l);
    l
}

/// A scene of axis-aligned quads (z = camera depth) seen by an orthographic identity camera
/// mapping world x/y straight to raster pixels.
pub fn quad_scene(w: u32, h: u32, quads: &[([f32; 4], f32, [f32; 4])]) -> Scene {
    let mut s = Scene { width: w, height: h, ssaa: 1, linear_io: true, ..Scene::default() };
    // clip: x → 2x/w − 1, y → 1 − 2y/h, z: depth 0..1000 → 1..0 (nearer = larger).
    s.clip = [[2.0 / w as f32, 0.0, 0.0, -1.0], [0.0, -2.0 / h as f32, 0.0, 1.0], [0.0, 0.0, -0.001, 1.0], [0.0, 0.0, 0.0, 1.0]];
    s.view = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    s.ortho = true;
    s.cam_fwd = [0.0, 0.0, 1.0];
    for (rect, z, color) in quads {
        let m = s.materials.len() as u32;
        s.materials.push(Material {
            base: *color,
            unlit: true,
            double_sided: true,
            alpha_mode: if color[3] < 1.0 { 2 } else { 0 },
            ..Material::default()
        });
        let b = s.vertices.len() as u32;
        for (x, y) in [(rect[0], rect[1]), (rect[2], rect[1]), (rect[2], rect[3]), (rect[0], rect[3])] {
            s.vertices.push(Vertex { pos: [x, y, *z], normal: [0.0, 0.0, -1.0], uv: [0.0; 2], tangent: [1.0, 0.0, 0.0, 1.0], material: m });
        }
        s.indices.extend([b, b + 2, b + 1, b, b + 3, b + 2]);
    }
    // Opaque first, transparent (in given order) after.
    let tris: Vec<[u32; 3]> = s.indices.chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
    let (o, t): (Vec<_>, Vec<_>) = tris.into_iter().partition(|t| !s.materials[s.vertices[t[0] as usize].material as usize].transparent());
    s.opaque_count = 3 * o.len() as u32;
    s.indices = o.into_iter().chain(t).flatten().collect();
    s
}

/// Builds a lit scene quad by quad, opaque and transparent triangles kept apart.
struct Builder {
    s: Scene,
    opaque: Vec<u32>,
    trans: Vec<u32>,
}

impl Builder {
    /// A quad through corners `c` (in order around it) with material `m`, and its two triangles.
    fn quad(&mut self, c: [[f32; 3]; 4], normal: [f32; 3], m: u32) {
        let b = self.s.vertices.len() as u32;
        let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        for (p, uv) in c.into_iter().zip(uv) {
            self.s.vertices.push(Vertex { pos: p, normal, uv, tangent: [1.0, 0.0, 0.0, 1.0], material: m });
        }
        let list = if self.s.materials[m as usize].transparent() { &mut self.trans } else { &mut self.opaque };
        list.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    }

    /// A triangle with material `m`.
    fn tri(&mut self, c: [[f32; 3]; 3], normal: [f32; 3], m: u32) {
        let b = self.s.vertices.len() as u32;
        for (p, uv) in c.into_iter().zip([[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]]) {
            self.s.vertices.push(Vertex { pos: p, normal, uv, tangent: [0.0, 1.0, 0.0, -1.0], material: m });
        }
        let list = if self.s.materials[m as usize].transparent() { &mut self.trans } else { &mut self.opaque };
        list.extend([b, b + 1, b + 2]);
    }

    fn finish(mut self) -> Scene {
        self.s.opaque_count = self.opaque.len() as u32;
        self.s.indices = self.opaque.into_iter().chain(self.trans).collect();
        self.s
    }
}

/// Appends a `w × h` texture made by `f(x, y)` and returns its index.
fn texture(s: &mut Scene, w: u32, h: u32, wrap: (u32, u32), f: impl Fn(u32, u32) -> [f32; 4]) -> i32 {
    let offset = s.texels.len() as u32;
    for y in 0..h {
        for x in 0..w {
            s.texels.push(f(x, y));
        }
    }
    s.textures.push(TexInfo { offset, width: w, height: h, wrap_u: wrap.0, wrap_v: wrap.1 });
    s.textures.len() as i32 - 1
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / l)
}

/// A lit scene at `out` pixels times `ssaa`, seen in perspective from (0, 0, −500) along +z
/// (+y down), or orthographically: a floor that crosses the near plane and reaches past the
/// viewport, a textured, normal-mapped box face and top, an alpha-masked card, a transparent card,
/// a card that draws only shadows, two single-sided triangles of opposite winding, an unlit
/// emissive triangle, three shadow-casting lights (parallel, spot, point), an environment light
/// with a dominant direction and ambient light. Shadow maps are not rendered yet.
pub fn lit_scene(out: (u32, u32), ssaa: u32, ortho: bool) -> Scene {
    let (w, h) = (out.0 * ssaa, out.1 * ssaa);
    let mut s = Scene { width: w, height: h, ssaa, ..Scene::default() };
    s.view = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 500.0], [0.0, 0.0, 0.0, 1.0]];
    s.eye = [0.0, 0.0, -500.0];
    s.cam_fwd = [0.0, 0.0, 1.0];
    if ortho {
        s.ortho = true;
        s.clip = [[2.0 / 400.0, 0.0, 0.0, 0.0], [0.0, -2.0 / 300.0, 0.0, 0.0], [0.0, 0.0, -0.001, 0.5], [0.0, 0.0, 0.0, 1.0]];
    } else {
        let f = 0.8 * w as f32;
        s.clip = [[2.0 * f / w as f32, 0.0, 0.0, 0.0], [0.0, -2.0 * f / h as f32, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 500.0]];
    }
    // Environment: three radiance levels and an irradiance map, brighter toward −y (up).
    let sky = |x: u32, y: u32, w: u32, h: u32| {
        let v = 1.0 - (y as f32 + 0.5) / h as f32;
        let u = (x as f32 + 0.5) / w as f32;
        [0.2 + 1.5 * v * v, 0.25 + 1.2 * v, 0.3 + 0.4 * u, 1.0]
    };
    texture(&mut s, 8, 4, (0, 1), |x, y| sky(x, y, 8, 4));
    texture(&mut s, 4, 2, (0, 1), |x, y| sky(x, y, 4, 2));
    texture(&mut s, 2, 1, (0, 1), |x, y| sky(x, y, 2, 1));
    texture(&mut s, 4, 2, (0, 1), |x, y| sky(x, y, 4, 2).map(|c| c * 0.6));
    let mask = texture(&mut s, 2, 2, (0, 0), |x, y| [1.0, 0.9, 0.8, if (x + y) % 2 == 0 { 1.0 } else { 0.0 }]);
    let color = texture(&mut s, 2, 2, (2, 2), |x, y| [0.6 + 0.3 * x as f32, 0.5, 0.4 + 0.4 * y as f32, 1.0]);
    let normal = texture(&mut s, 2, 2, (1, 1), |x, y| [0.5 + 0.2 * x as f32, 0.5 - 0.15 * y as f32, 0.95, 1.0]);
    let mr = texture(&mut s, 2, 2, (0, 0), |x, _| [1.0, 0.6 + 0.3 * x as f32, 0.5, 1.0]);
    let occlusion = texture(&mut s, 1, 1, (0, 0), |_, _| [0.7, 0.7, 0.7, 1.0]);
    s.env = Some(EnvInfo {
        radiance: 0,
        mips: 3,
        irradiance: 3,
        intensity: 0.7,
        rotation: 0.4,
        sun: norm([0.2, -0.9, 0.3]),
        key: 0.6,
        shadow: 0,
        shadow_darkness: 0.8,
        shadow_diffusion: 6.0,
    });
    s.ambient = [0.05, 0.05, 0.06];
    let light = |kind: u32, pos: [f32; 3], dir: [f32; 3], color: [f32; 3]| Light {
        kind,
        pos,
        dir: norm(dir),
        color,
        cos_inner: 0.95,
        cos_outer: 0.85,
        falloff: kind.min(2),
        radius: 150.0,
        falloff_distance: 900.0,
        shadow: 0,
        shadow_darkness: 0.9,
        shadow_diffusion: 4.0,
    };
    s.lights = vec![
        light(0, [0.0; 3], [0.3, 0.8, 0.5], [0.9, 0.85, 0.8]),
        light(1, [-150.0, -300.0, -100.0], [150.0, 400.0, 200.0], [1.2, 1.0, 0.7]),
        light(2, [150.0, -150.0, 0.0], [0.0, 1.0, 0.0], [0.6, 0.7, 1.0]),
    ];
    let mats = [
        Material { base: [0.8, 0.8, 0.8, 1.0], roughness: 0.7, double_sided: true, ..Material::default() },
        Material {
            base: [0.9, 0.3, 0.2, 1.0],
            metallic: 0.3,
            roughness: 0.35,
            tex_base: color,
            tex_normal: normal,
            tex_mr: mr,
            tex_occlusion: occlusion,
            casts_shadows: true,
            double_sided: true,
            specular_k: 1.2,
            ..Material::default()
        },
        Material { alpha_mode: 1, tex_base: mask, emissive: [0.1, 0.0, 0.0], casts_shadows: true, double_sided: true, ..Material::default() },
        Material { base: [0.2, 0.4, 1.0, 0.5], alpha_mode: 2, roughness: 0.2, double_sided: true, ..Material::default() },
        Material { base: [0.3, 0.9, 0.3, 1.0], ..Material::default() },
        Material { shadows_only: true, double_sided: true, ..Material::default() },
        Material { base: [0.1, 0.1, 0.1, 1.0], emissive: [0.9, 0.8, 0.2], unlit: true, ..Material::default() },
    ];
    s.materials = mats.to_vec();
    let mut b = Builder { s, opaque: vec![], trans: vec![] };
    let up = [0.0, -1.0, 0.0];
    b.quad([[-400.0, 100.0, -800.0], [400.0, 100.0, -800.0], [400.0, 100.0, 600.0], [-400.0, 100.0, 600.0]], up, 0);
    b.quad([[-60.0, 100.0, 50.0], [-60.0, -20.0, 50.0], [60.0, -20.0, 50.0], [60.0, 100.0, 50.0]], [0.0, 0.0, -1.0], 1);
    b.quad([[-60.0, -20.0, 50.0], [-60.0, -20.0, 170.0], [60.0, -20.0, 170.0], [60.0, -20.0, 50.0]], up, 1);
    b.quad([[80.0, 100.0, 0.0], [80.0, -60.0, 0.0], [160.0, -60.0, 60.0], [160.0, 100.0, 60.0]], norm([-0.6, 0.0, -0.8]), 2);
    b.tri([[-180.0, 0.0, -50.0], [-120.0, 0.0, -50.0], [-150.0, -60.0, -50.0]], [0.0, 0.0, -1.0], 4);
    b.tri([[-180.0, 40.0, -50.0], [-150.0, -20.0, -50.0], [-120.0, 40.0, -50.0]], [0.0, 0.0, -1.0], 4);
    b.tri([[100.0, -100.0, 300.0], [180.0, -100.0, 300.0], [140.0, -160.0, 300.0]], [0.0, 0.0, -1.0], 6);
    // Transparent, back to front.
    b.quad([[-200.0, 100.0, 250.0], [-200.0, -40.0, 250.0], [-80.0, -40.0, 250.0], [-80.0, 100.0, 250.0]], [0.0, 0.0, -1.0], 5);
    b.quad([[-100.0, 60.0, -20.0], [-100.0, -80.0, -20.0], [20.0, -80.0, -20.0], [20.0, 60.0, -20.0]], [0.0, 0.0, -1.0], 3);
    b.finish()
}

/// The bit patterns of a pixel list, for exact comparison.
pub fn bits4(v: &[[f32; 4]]) -> Vec<[u32; 4]> {
    v.iter().map(|p| p.map(f32::to_bits)).collect()
}

/// The bit patterns of a value list, for exact comparison.
pub fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// 64-bit FNV-1a of the bit patterns of `pixels` and then `values`.
pub fn fnv(pixels: &[[f32; 4]], values: &[f32]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in pixels.iter().flatten().chain(values).flat_map(|x| x.to_bits().to_le_bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}
