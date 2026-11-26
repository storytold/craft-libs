//! # craft-pbr
//!
//! Image-based lighting for physically based rendering, computed on the CPU. An [`EnvImage`] is a
//! linear RGB equirect (latitude-longitude) environment. [`try_prefilter`] turns it into a
//! GGX-prefiltered chain for specular light, one level per roughness, and [`try_sh9`] projects it
//! onto nine spherical-harmonics coefficients for diffuse light. [`Environment`] renders six
//! procedural environments with no image assets.
//!
//! The equirect frame has +Y up. Column `u = φ / 2π` and row `v = θ / π` map to the direction
//! `(sin θ cos φ, cos θ, sin θ sin φ)`: row 0 is straight up, `u = 0` points to +X and `u`
//! increases toward +Z. Lookups wrap horizontally and clamp vertically. [`dir_of`] and [`uv_of`]
//! convert between the two. An image in another frame, such as an OpenEXR latitude-longitude
//! image, is converted by the caller before it is passed in.
//!
//! The specular chain follows the split-sum approximation of Karis, *Real Shading in Unreal
//! Engine 4* (SIGGRAPH 2013,
//! <https://de45xmedrsdbp.cloudfront.net/Resources/files/2013SiggraphPresentationsNotes-26915738.pdf>):
//! level `l` of `n` is filtered for roughness `l / (n - 1)` with the view and reflection
//! directions taken equal to the normal, and samples weighted by `n·l`. The distribution is GGX
//! with `α = roughness²`, as glTF 2.0 Appendix B.2.3 defines it
//! (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#microfacet-surfaces>), and its
//! distribution D is the one section B.3.2 writes
//! (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#specular-brdf>).
//! Samples are importance-sampled from Hammersley points and read from a box-filtered pyramid
//! of the source at a level chosen from each sample's density. [`try_environment_brdf`] gives
//! the other term of the split sum, the scale and bias of `f0`.
//!
//! [`Material`] holds a glTF 2.0 metallic-roughness material, and [`Material::try_direct`]
//! evaluates its BRDF for one light in the final form of section B.3.5
//! (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#metal-brdf-and-dielectric-brdf>),
//! with the Smith-Schlick geometry term Karis gives for analytic lights by default. [`Visibility`]
//! selects section B.3.2's separable Smith form or the height-correlated Smith form instead.
//! [`default_environment`]
//! is a small studio environment for an environment light without an image.
//!
//! [`SHADE_WGSL`] is the WGSL text of the same shading functions for an app's own shaders, and
//! [`GpuMaterial`] and [`GpuSh9`] lay out the values it reads.
//!
//! The diffuse coefficients are the nine real SH coefficients of the radiance, convolved with the
//! clamped cosine lobe (band factors `π`, `2π/3` and `π/4`, Ramamoorthi and Hanrahan 2001,
//! <https://graphics.stanford.edu/papers/envmap/envmap.pdf>) and divided by `π`. [`irradiance`]
//! evaluates them: for a white surface its result is the diffuse radiance it reflects.
//!
//! Sizes and work are bounded. [`Limits`] caps the side, the pixel count, the bytes and the
//! sample work, and every `try_` function returns a [`PbrError`] before it allocates anything
//! over them. Pixel buffers are reserved fallibly and give [`PbrError::OutOfMemory`] when the
//! reservation fails. Radiance in every [`EnvImage`] is between 0 and [`MAX_RADIANCE`]: the
//! constructors reject or replace other values. Rows are computed on several threads only with
//! the `parallel` feature, and the results are the same either way.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod brdf;
mod environment;
mod prefilter;
mod sh;
mod vec3;
mod wgsl;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

pub use brdf::{MAX_SPECULAR, MIN_DIRECT_ROUGHNESS, Material, Visibility, try_environment_brdf};
pub use environment::{ColorEncoding, ENVIRONMENTS, EnvImage, Environment, MAX_RADIANCE, default_environment, dir_of, uv_of};
pub use prefilter::{PrefilterOptions, ggx_distribution, ggx_distribution_between, hammersley, try_prefilter};
pub use sh::{irradiance, sh_basis, try_sh9};
pub use wgsl::{GpuMaterial, GpuSh9, SHADE_WGSL};

/// Why an environment could not be made, filtered or read, or a material could not be shaded.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum PbrError {
    /// A side is zero or longer than [`Limits::max_side`].
    #[error("environment size {0}x{1} has a zero side or a side over the limit")]
    BadSize(u32, u32),
    /// The pixel count or the byte count is over [`Limits`], or a buffer would be larger than
    /// `isize::MAX` bytes, the largest allocation the target allows.
    #[error("environment size {0}x{1} needs more pixels or bytes than the limits or the target allow")]
    TooLarge(u32, u32),
    /// The pixel buffer does not hold width × height pixels.
    #[error("environment data holds {found} pixels where {expected} are needed")]
    BadDataLength { expected: usize, found: usize },
    /// A linear radiance value is NaN, negative or above [`MAX_RADIANCE`].
    #[error("pixel {index} has a radiance that is not a number, is negative or is over the maximum")]
    BadRadiance { index: usize },
    /// A direction has a non-finite component or zero length.
    #[error("the direction is not finite or has zero length")]
    BadDirection,
    /// An option is out of its range; the text names it.
    #[error("{0}")]
    BadOption(&'static str),
    /// A material value is not a number or is out of its range; the text names it.
    #[error("{0}")]
    BadMaterial(&'static str),
    /// The filtering would take more sample evaluations than [`Limits::max_work`].
    #[error("the filtering needs {needed} sample evaluations, over the limit of {limit}")]
    TooMuchWork { needed: u64, limit: u64 },
    /// Memory for the pixels could not be reserved.
    #[error("not enough memory for the environment")]
    OutOfMemory,
}

/// Caps on sizes and work, checked before anything is allocated or computed.
/// `#[non_exhaustive]`: start from [`Limits::default`] and set the fields you change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Longest side in pixels. Default 2048.
    pub max_side: u32,
    /// Most pixels (width × height) of one image. Default 2^22, which is 2048 × 2048.
    pub max_pixels: u64,
    /// Most bytes of pixel data one call holds at once: for [`try_prefilter`] the source copies,
    /// the pyramid, every output level and the sample table together, and for [`try_sh9`] its
    /// table of column integrals (48 bytes per column). Default 256 MiB.
    pub max_bytes: u64,
    /// Most levels [`try_prefilter`] makes. Default 8.
    pub max_levels: u32,
    /// Most samples per output pixel. Default 1024.
    pub max_samples: u32,
    /// Most sample evaluations of one [`try_prefilter`] call: output pixels of the filtered levels
    /// times samples. Default 2^28.
    pub max_work: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_side: 2048, max_pixels: 1 << 22, max_bytes: 256 << 20, max_levels: 8, max_samples: 1024, max_work: 1 << 28 }
    }
}

/// Bytes of one pixel: three `f32` channels.
const PIXEL_BYTES: u64 = 12;

/// The largest allocation in bytes that a `Vec` can make on the target.
const MAX_ALLOC_BYTES: u64 = isize::MAX as u64;

impl Limits {
    /// The pixel count of a `w × h` image, or the error the limits give for that size. A size
    /// whose bytes are over `isize::MAX` is [`PbrError::TooLarge`] whatever the limits are.
    /// Nothing is allocated.
    pub fn check(&self, w: u32, h: u32) -> Result<usize, PbrError> {
        if w == 0 || h == 0 || w > self.max_side || h > self.max_side {
            return Err(PbrError::BadSize(w, h));
        }
        let pixels = u64::from(w) * u64::from(h);
        let bytes = pixels.checked_mul(PIXEL_BYTES).ok_or(PbrError::TooLarge(w, h))?;
        if pixels > self.max_pixels || bytes > self.max_bytes || bytes > MAX_ALLOC_BYTES {
            return Err(PbrError::TooLarge(w, h));
        }
        usize::try_from(pixels).map_err(|_| PbrError::TooLarge(w, h))
    }

    /// Checks a total of `bytes` held at once against [`Limits::max_bytes`] and `isize::MAX`.
    fn check_total_bytes(&self, bytes: Option<u64>, w: u32, h: u32) -> Result<(), PbrError> {
        match bytes {
            Some(b) if b <= self.max_bytes && b <= MAX_ALLOC_BYTES => Ok(()),
            _ => Err(PbrError::TooLarge(w, h)),
        }
    }
}

/// A zeroed pixel buffer of `len` pixels, reserved fallibly.
fn try_pixels(len: usize) -> Result<Vec<[f32; 3]>, PbrError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| PbrError::OutOfMemory)?;
    v.resize(len, [0.0; 3]);
    Ok(v)
}

/// Runs `f(row index, row)` for each row of `width` pixels, on several threads with the
/// `parallel` feature. `width` is at least 1.
fn for_each_row<F>(data: &mut [[f32; 3]], width: usize, f: F)
where
    F: Fn(usize, &mut [[f32; 3]]) + Sync + Send,
{
    let width = width.max(1);
    #[cfg(feature = "parallel")]
    data.par_chunks_mut(width).enumerate().for_each(|(y, row)| f(y, row));
    #[cfg(not(feature = "parallel"))]
    data.chunks_mut(width).enumerate().for_each(|(y, row)| f(y, row));
}
