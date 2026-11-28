//! The headless test scene, shared by `tests/it/headless.rs` and `examples/headless_scene.rs`.
//!
//! `craft-pbr` makes its environment light: a procedural environment, its GGX-prefiltered levels
//! and an irradiance map from its SH9 coefficients. A parallel light, a spot light and the
//! environment light each cast shadows, and the scene holds an alpha-masked card and a blended
//! card.
//!
//! The shadow texels match bit for bit on every target tested, and so do the depth and the
//! encoded image rounded to 8 bits per channel. The encoded `f32` colors do not: Rust's
//! transcendental functions (`acos`, `atan2`, `exp2`, `log2`, `powf`, `sin`, `cos`) come from the
//! platform's math library natively and from Rust's own in a wasm32 module, and the two round
//! some results differently in the last bit. The prefiltered environment and the shaded colors
//! then differ by a few units in the last place, and no 8-bit value changes. The constants below
//! were measured on x86_64 Linux and in the wasm32 module.
//!
//! `craft-pbr` puts +Y up and `u = 0` at +X, and `craft-raster3d` puts +Y down and `u = 0.5` at
//! +Z, so each environment texture is resampled into `craft-raster3d`'s directions.

use craft_pbr::{EnvImage, Environment, PrefilterOptions, irradiance, try_prefilter, try_sh9};
use craft_raster3d::{EnvInfo, Light, Limits, Material, Scene, TexInfo, Vertex, try_encode, try_render_cpu, try_shadow_maps};

/// FNV-1a hash of the test scene's shadow texels.
pub const SHADOW_HASH: u64 = 0x6461_2d0d_da9e_fb0f;
/// FNV-1a hash of the test scene's encoded image at 8 bits per channel and its depth.
pub const IMAGE_HASH: u64 = 0xafa2_3a2d_4eb3_9453;

/// A render of the scene: the shadow texels, the encoded pixels and their depth.
pub type Render = (Vec<f32>, Vec<[f32; 4]>, Vec<f32>);

/// 64-bit FNV-1a of `bytes`.
pub fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// The little-endian bytes of the bit patterns of `values`.
pub fn f32_bytes(values: &[f32]) -> impl Iterator<Item = u8> + '_ {
    values.iter().flat_map(|x| x.to_bits().to_le_bytes())
}

/// A channel in `0..=1` rounded to 8 bits.
pub fn to_u8(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The encoded pixels rounded to 8 bits per channel, in RGBA order.
pub fn image_bytes(pixels: &[[f32; 4]]) -> Vec<u8> {
    pixels.iter().flatten().map(|&c| to_u8(c)).collect()
}

/// The hashes `SHADOW_HASH` and `IMAGE_HASH` are measured with.
pub fn hashes((shadows, pixels, depth): &Render) -> (u64, u64) {
    (fnv(f32_bytes(shadows)), fnv(image_bytes(pixels).into_iter().chain(f32_bytes(depth))))
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / l)
}

/// Appends the texels of a `w × h` texture and returns its index.
fn push_texture(s: &mut Scene, w: u32, h: u32, wrap: (u32, u32), texels: impl IntoIterator<Item = [f32; 4]>) -> u32 {
    let offset = s.texels.len() as u32;
    s.texels.extend(texels);
    assert_eq!(s.texels.len() as u32 - offset, w * h);
    s.textures.push(TexInfo { offset, width: w, height: h, wrap_u: wrap.0, wrap_v: wrap.1 });
    s.textures.len() as u32 - 1
}

/// The `craft-pbr` direction (+Y up) of the center of pixel `(x, y)` of a `w × h` equirect
/// texture as `craft-raster3d` reads it (+Y down, `u = 0.5 + atan2(x, z) / 2π`). The world maps
/// to the `craft-pbr` frame by a half turn about the x axis: `(x, −y, −z)`.
fn pbr_dir(x: u32, y: u32, w: u32, h: u32) -> [f32; 3] {
    let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
    let (theta, a) = (v * std::f32::consts::PI, (u - 0.5) * std::f32::consts::TAU);
    let d = [theta.sin() * a.sin(), -theta.cos(), theta.sin() * a.cos()];
    [d[0], -d[1], -d[2]]
}

/// Appends a `w × h` opaque texture that repeats across and clamps down, each pixel made by
/// `f` from its `craft-pbr` direction.
fn push_env(s: &mut Scene, w: u32, h: u32, f: impl Fn([f32; 3]) -> [f32; 3]) -> u32 {
    let texels: Vec<[f32; 4]> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let c = f(pbr_dir(x, y, w, h));
            [c[0], c[1], c[2], 1.0]
        })
        .collect();
    push_texture(s, w, h, (0, 1), texels)
}

/// Appends an environment level resampled into `craft-raster3d`'s columns at its own size.
fn push_level(s: &mut Scene, img: &EnvImage) -> u32 {
    push_env(s, img.width(), img.height(), |d| img.sample(d).unwrap())
}

/// The environment light: the Studio Contrast environment at 32 × 16, prefiltered into four
/// levels, and an 8 × 4 irradiance map evaluated from its SH9 coefficients at the pixel centers,
/// all resampled into the directions `craft-raster3d` looks them up in.
fn environment(s: &mut Scene) -> EnvInfo {
    let limits = craft_pbr::Limits::default();
    let env = Environment::StudioContrast.try_render(32, &limits).unwrap();
    let mut options = PrefilterOptions::default();
    options.levels = 4;
    options.samples = 32;
    let levels = try_prefilter(&env, &options, &limits).unwrap();
    let radiance = push_level(s, &levels[0]);
    for level in &levels[1..] {
        push_level(s, level);
    }
    let sh = try_sh9(&env, &limits).unwrap();
    let irradiance = push_env(s, 8, 4, |d| irradiance(&sh, d).unwrap().map(|c| c.max(0.0)));
    EnvInfo {
        radiance,
        mips: levels.len() as u32,
        irradiance,
        intensity: 0.6,
        rotation: 0.3,
        sun: norm([0.3, -0.8, 0.4]),
        key: 0.5,
        shadow: 0,
        shadow_darkness: 0.7,
        shadow_diffusion: 3.0,
    }
}

/// Builds the scene's triangles quad by quad, opaque and transparent kept apart.
struct Builder {
    s: Scene,
    opaque: Vec<u32>,
    trans: Vec<u32>,
}

impl Builder {
    /// A quad through corners `c` (in order around it) with material `m`.
    fn quad(&mut self, c: [[f32; 3]; 4], normal: [f32; 3], m: u32) {
        let b = self.s.vertices.len() as u32;
        for (pos, uv) in c.into_iter().zip([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]) {
            self.s.vertices.push(Vertex { pos, normal, uv, tangent: [1.0, 0.0, 0.0, 1.0], material: m });
        }
        let list = if self.s.materials[m as usize].transparent() { &mut self.trans } else { &mut self.opaque };
        list.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    }

    fn finish(mut self) -> Scene {
        self.s.opaque_count = self.opaque.len() as u32;
        self.s.indices = self.opaque.into_iter().chain(self.trans).collect();
        self.s
    }
}

/// The test scene at 48 × 36 output pixels, supersampled 2 × 2, seen in perspective from
/// (0, 0, −500) along +z with +y down: a floor, a textured metallic box face and top, an
/// alpha-masked checker card and a blended card, under a parallel light, a spot light and the
/// environment light, each casting shadows. Shadow maps are not rendered yet.
pub fn scene() -> Scene {
    let (ssaa, w, h) = (2, 96, 72);
    let mut s = Scene { width: w, height: h, ssaa, ..Scene::default() };
    s.view = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 500.0], [0.0, 0.0, 0.0, 1.0]];
    s.eye = [0.0, 0.0, -500.0];
    s.cam_fwd = [0.0, 0.0, 1.0];
    let f = 0.8 * w as f32;
    s.clip = [[2.0 * f / w as f32, 0.0, 0.0, 0.0], [0.0, -2.0 * f / h as f32, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 500.0]];
    s.env = Some(environment(&mut s));
    s.ambient = [0.03, 0.03, 0.04];
    let checker = push_texture(&mut s, 4, 4, (0, 0), (0..16).map(|i| [0.9, 0.8, 0.7, if (i % 4 + i / 4) % 2 == 0 { 1.0 } else { 0.0 }]));
    let color = push_texture(&mut s, 2, 2, (0, 0), (0..4).map(|i| [0.5 + 0.2 * (i % 2) as f32, 0.45, 0.3 + 0.3 * (i / 2) as f32, 1.0]));
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
        shadow_diffusion: 2.0,
    };
    s.lights =
        vec![light(0, [0.0; 3], [0.3, 0.8, 0.5], [0.9, 0.85, 0.8]), light(1, [-150.0, -300.0, -100.0], [150.0, 400.0, 200.0], [1.2, 1.0, 0.7])];
    s.materials = vec![
        Material { base: [0.8, 0.8, 0.8, 1.0], roughness: 0.7, double_sided: true, ..Material::default() },
        Material {
            base: [0.9, 0.5, 0.3, 1.0],
            metallic: 0.8,
            roughness: 0.3,
            tex_base: color as i32,
            casts_shadows: true,
            double_sided: true,
            ..Material::default()
        },
        Material { alpha_mode: 1, tex_base: checker as i32, casts_shadows: true, double_sided: true, ..Material::default() },
        Material { base: [0.2, 0.4, 1.0, 0.5], alpha_mode: 2, roughness: 0.2, double_sided: true, ..Material::default() },
    ];
    let mut b = Builder { s, opaque: vec![], trans: vec![] };
    let up = [0.0, -1.0, 0.0];
    let front = [0.0, 0.0, -1.0];
    b.quad([[-400.0, 100.0, -300.0], [400.0, 100.0, -300.0], [400.0, 100.0, 600.0], [-400.0, 100.0, 600.0]], up, 0);
    b.quad([[-60.0, 100.0, 50.0], [-60.0, -20.0, 50.0], [60.0, -20.0, 50.0], [60.0, 100.0, 50.0]], front, 1);
    b.quad([[-60.0, -20.0, 50.0], [-60.0, -20.0, 170.0], [60.0, -20.0, 170.0], [60.0, -20.0, 50.0]], up, 1);
    b.quad([[80.0, 100.0, 0.0], [80.0, -60.0, 0.0], [160.0, -60.0, 60.0], [160.0, 100.0, 60.0]], norm([-0.6, 0.0, -0.8]), 2);
    b.quad([[-100.0, 60.0, -20.0], [-100.0, -80.0, -20.0], [20.0, -80.0, -20.0], [20.0, 60.0, -20.0]], front, 3);
    b.finish()
}

/// Shadow maps, the render at output size and the encoding of `s`.
pub fn render(mut s: Scene) -> Render {
    try_shadow_maps(&mut s, false, &Limits::default()).unwrap();
    let (mut img, depth) = try_render_cpu(&s, &Limits::default()).unwrap();
    try_encode(&mut img).unwrap();
    assert_eq!((img.width, img.height), (48, 36));
    (s.shadow_texels, img.data, depth)
}
