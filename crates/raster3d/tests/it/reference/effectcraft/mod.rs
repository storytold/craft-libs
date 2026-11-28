//! EffectCraft's Advanced 3D renderer at 2f16feb12f (`crates/render/src/three_d/adv.rs`,
//! `adv/raster.rs` and `adv/shade.rs`; `adv/raster.rs` and `adv/shade.rs` are unchanged since
//! 8fc2bc99), for the differential tests, with the alpha-mask change of storytold/effectcraft#4075
//! applied to `shade::shade` and `raster::shadow_raster`: a masked fragment that passes the
//! cutoff keeps only the layer's opacity, and a masked caster tests the material's alpha against
//! its cutoff. `resolve`, `encode` and `render_cpu` are copied from
//! `adv.rs` with `Image` defined here; `encode`'s channel loop iterates over the channels
//! instead of indexing them, which clippy asks for. `geom` holds the parts of `effectcraft_geom`
//! and `camera::basis_view` that the shadow maps use.

pub mod raster;
pub mod shade;

use rayon::prelude::*;

use craft_raster3d::Scene;
use raster::Target;

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<[f32; 4]>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        Image { width, height, data: vec![[0.0; 4]; (width * height) as usize] }
    }
}

pub fn resolve(t: &Target, k: u32) -> (Image, Vec<f32>) {
    let (w, h) = (t.width / k, t.height / k);
    let mut img = Image::new(w, h);
    let mut depth = vec![f32::INFINITY; (w * h) as usize];
    let n = (k * k) as f32;
    img.data.par_chunks_mut(w as usize).zip(depth.par_chunks_mut(w as usize)).enumerate().for_each(|(y, (row, drow))| {
        for x in 0..w as usize {
            let mut acc = [0.0f32; 4];
            let (mut dz, mut dn) = (0.0f32, 0.0f32);
            for j in 0..k as usize {
                for i in 0..k as usize {
                    let s = (y * k as usize + j) * t.width as usize + x * k as usize + i;
                    let c = t.color[s];
                    for q in 0..4 {
                        acc[q] += c[q];
                    }
                    if t.depth[s].is_finite() {
                        dz += t.depth[s];
                        dn += 1.0;
                    }
                }
            }
            row[x] = acc.map(|v| v / n);
            if dn > 0.0 {
                drow[x] = dz / dn;
            }
        }
    });
    (img, depth)
}

pub fn encode(img: &mut Image) {
    img.data.par_iter_mut().for_each(|p| {
        let a = p[3];
        if a > 1e-6 {
            for c in &mut p[..3] {
                *c = shade::linear_to_srgb(*c / a) * a;
            }
        }
    });
}

pub fn render_cpu(s: &Scene) -> (Image, Vec<f32>) {
    resolve(&raster::render(s), s.ssaa.max(1))
}

/// `effectcraft_geom`'s `Vec3` and `Mat4` operations and `camera::basis_view`.
pub mod geom {
    use std::ops::{Add, Div, Mul, Neg, Sub};

    #[derive(Clone, Copy, Debug)]
    pub struct Vec3 {
        pub x: f64,
        pub y: f64,
        pub z: f64,
    }

    pub const fn vec3(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }

    impl Vec3 {
        pub fn dot(self, o: Vec3) -> f64 {
            self.x * o.x + self.y * o.y + self.z * o.z
        }
        pub fn cross(self, o: Vec3) -> Vec3 {
            vec3(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
        }
        pub fn length(self) -> f64 {
            self.dot(self).sqrt()
        }
        pub fn normalize(self) -> Vec3 {
            let l = self.length();
            if l > 0.0 { self / l } else { self }
        }
    }

    impl Add for Vec3 {
        type Output = Vec3;
        fn add(self, o: Vec3) -> Vec3 {
            Vec3 { x: self.x + o.x, y: self.y + o.y, z: self.z + o.z }
        }
    }
    impl Sub for Vec3 {
        type Output = Vec3;
        fn sub(self, o: Vec3) -> Vec3 {
            Vec3 { x: self.x - o.x, y: self.y - o.y, z: self.z - o.z }
        }
    }
    impl Mul<f64> for Vec3 {
        type Output = Vec3;
        fn mul(self, k: f64) -> Vec3 {
            Vec3 { x: self.x * k, y: self.y * k, z: self.z * k }
        }
    }
    impl Div<f64> for Vec3 {
        type Output = Vec3;
        fn div(self, k: f64) -> Vec3 {
            Vec3 { x: self.x / k, y: self.y / k, z: self.z / k }
        }
    }
    impl Neg for Vec3 {
        type Output = Vec3;
        fn neg(self) -> Vec3 {
            Vec3 { x: -self.x, y: -self.y, z: -self.z }
        }
    }

    #[derive(Clone, Copy, Debug)]
    pub struct Mat4(pub [[f64; 4]; 4]);

    impl Mat4 {
        pub const IDENTITY: Mat4 = Mat4([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]);
        pub fn translate(t: Vec3) -> Mat4 {
            let mut m = Mat4::IDENTITY;
            m.0[0][3] = t.x;
            m.0[1][3] = t.y;
            m.0[2][3] = t.z;
            m
        }
    }

    impl Mul for Mat4 {
        type Output = Mat4;
        fn mul(self, o: Mat4) -> Mat4 {
            let mut r = [[0.0; 4]; 4];
            for i in 0..4 {
                for j in 0..4 {
                    r[i][j] = (0..4).map(|k| self.0[i][k] * o.0[k][j]).sum();
                }
            }
            Mat4(r)
        }
    }

    pub fn basis_view(eye: Vec3, forward: Vec3, down_hint: Vec3) -> Mat4 {
        let f = forward.normalize();
        let f = if f.length() == 0.0 { vec3(0.0, 0.0, 1.0) } else { f };
        let mut d = down_hint - f * down_hint.dot(f);
        if d.length() < 1e-9 {
            d = if f.y.abs() < 0.99 { vec3(0.0, 1.0, 0.0) } else { vec3(0.0, 0.0, -1.0) };
            d = d - f * d.dot(f);
        }
        let d = d.normalize();
        let r = d.cross(f).normalize();
        Mat4([[r.x, r.y, r.z, 0.0], [d.x, d.y, d.z, 0.0], [f.x, f.y, f.z, 0.0], [0.0, 0.0, 0.0, 1.0]]) * Mat4::translate(-eye)
    }
}
