//! The WGSL text of the shading functions and the GPU layouts of the values it reads.

use std::mem::offset_of;

use crate::{Material, PbrError};

/// WGSL source of the shading functions: `pbr_direct`, `pbr_environment_brdf`,
/// `pbr_environment_light`, `pbr_irradiance` and the helpers they call, with the structs
/// `PbrMaterial` and `PbrSh9`. The crate README lists each function next to its Rust
/// counterpart.
///
/// The text declares no bindings and no entry points, and every name in it starts with `pbr_`,
/// `PBR_` or `Pbr`. An app joins its own declarations, this text and its entry points with
/// newlines into one module and passes that to its shader compiler. No function loops or
/// recurses. Colors are linear.
pub const SHADE_WGSL: &str = include_str!("shade.wgsl");

/// A [`Material`] laid out as the WGSL struct `PbrMaterial`, 32 bytes. Made by [`Material::gpu`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuMaterial {
    base_color: [f32; 3],
    metallic: f32,
    roughness: f32,
    specular: f32,
    diffuse: f32,
    pad: f32,
}

impl GpuMaterial {
    /// Bytes of one `PbrMaterial`, which is also its array stride.
    pub const SIZE: usize = size_of::<GpuMaterial>();

    /// The little-endian bytes of `PbrMaterial`, for a uniform or storage buffer.
    pub fn to_bytes(&self) -> [u8; GpuMaterial::SIZE] {
        let mut out = [0u8; GpuMaterial::SIZE];
        let fields = [
            (offset_of!(GpuMaterial, base_color), self.base_color[0]),
            (offset_of!(GpuMaterial, base_color) + 4, self.base_color[1]),
            (offset_of!(GpuMaterial, base_color) + 8, self.base_color[2]),
            (offset_of!(GpuMaterial, metallic), self.metallic),
            (offset_of!(GpuMaterial, roughness), self.roughness),
            (offset_of!(GpuMaterial, specular), self.specular),
            (offset_of!(GpuMaterial, diffuse), self.diffuse),
            (offset_of!(GpuMaterial, pad), self.pad),
        ];
        for (at, v) in fields {
            if let Some(dst) = out.get_mut(at..at + 4) {
                dst.copy_from_slice(&v.to_le_bytes());
            }
        }
        out
    }
}

impl Material {
    /// The material in the layout of the WGSL struct `PbrMaterial`.
    pub fn gpu(&self) -> GpuMaterial {
        GpuMaterial {
            base_color: self.base_color(),
            metallic: self.metallic(),
            roughness: self.roughness(),
            specular: self.specular(),
            diffuse: self.diffuse(),
            pad: 0.0,
        }
    }
}

/// Nine SH coefficients from [`try_sh9`](crate::try_sh9) laid out as the WGSL struct `PbrSh9`,
/// 144 bytes: one `vec4<f32>` per coefficient with red, green and blue in `x`, `y` and `z` and 0
/// in `w`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuSh9 {
    c: [[f32; 4]; 9],
}

impl GpuSh9 {
    /// Bytes of one `PbrSh9`.
    pub const SIZE: usize = size_of::<GpuSh9>();

    /// The largest magnitude of a coefficient. The coefficients of [`try_sh9`](crate::try_sh9)
    /// for radiance up to [`MAX_RADIANCE`](crate::MAX_RADIANCE) stay under 4e28. The basis
    /// functions sum to less than 5 in magnitude in every direction, so `pbr_irradiance` sums
    /// to less than 5e30 in `f32`.
    pub const MAX_COEFFICIENT: f32 = 1e30;

    /// The coefficients in the layout of `PbrSh9`. Returns [`PbrError::BadOption`] when a
    /// coefficient is not finite or its magnitude is over [`GpuSh9::MAX_COEFFICIENT`].
    pub fn try_new(coefficients: &[[f32; 3]; 9]) -> Result<GpuSh9, PbrError> {
        if coefficients.iter().flatten().any(|c| !c.is_finite() || c.abs() > GpuSh9::MAX_COEFFICIENT) {
            return Err(PbrError::BadOption("an SH coefficient is not finite or its magnitude is over 1e30"));
        }
        Ok(GpuSh9 { c: coefficients.map(|[r, g, b]| [r, g, b, 0.0]) })
    }

    /// The little-endian bytes of `PbrSh9`, for a uniform or storage buffer.
    pub fn to_bytes(&self) -> [u8; GpuSh9::SIZE] {
        let mut out = [0u8; GpuSh9::SIZE];
        let base = offset_of!(GpuSh9, c);
        for (i, v) in self.c.iter().flatten().enumerate() {
            let at = base + 4 * i;
            if let Some(dst) = out.get_mut(at..at + 4) {
                dst.copy_from_slice(&v.to_le_bytes());
            }
        }
        out
    }
}
