//! # craft-raster3d
//!
//! A CPU renderer for a prepared 3D scene. A [`Scene`] holds world-space triangles, their
//! materials and textures, lights, an optional environment light and shadow maps, as plain
//! arrays. [`try_render`] clips, rasterizes and shades it into a [`Target`] of premultiplied
//! linear RGBA and camera depth; [`try_resolve`] box-filters a supersampled target down to an
//! [`Image`]; [`try_encode`] converts linear pixels to the sRGB transfer curve; and
//! [`try_render_cpu`] renders and resolves in one call. [`try_shadow_maps`] renders the depth maps
//! of the shadow-casting lights into the scene before it is rendered.
//!
//! - **Depth.** Reversed: a larger normalized device z is nearer. Triangles are clipped against
//!   `0 ≤ z ≤ w` and against a guard band of `±GUARD_BAND × w` in x and y before the
//!   perspective divide.
//! - **Coverage.** One sample at each pixel center, with the top-left rule on shared edges.
//!   Attributes are interpolated with perspective-correct weights.
//! - **Passes.** The first [`Scene::opaque_count`] indices are opaque triangles: visibility is
//!   resolved first and each pixel is shaded once. The rest are transparent triangles, which the
//!   caller sorts back to front; they are shaded per fragment, depth-tested against the opaque
//!   surfaces and blended over them.
//! - **Facing.** A triangle whose screen-space winding is counterclockwise with y down is front
//!   facing. A single-sided material does not draw back faces.
//! - **Textures.** [`Scene::texels`] holds linear straight RGBA, row-major from the top row;
//!   [`TexInfo`] gives each texture's span and wrap modes.
//!
//! [`try_render`], [`try_render_cpu`] and [`try_shadow_maps`] check the scene against [`Limits`]
//! and return a [`RenderError`] before they allocate the final target or the shadow maps:
//! malformed indices, references and spans, values that are not finite, sizes, triangle counts,
//! light counts and the coverage work of setup over the limits. They can allocate scratch buffers
//! first, and some errors (`OutOfMemory`, a value that overflows while rendering, and
//! `TooMuchWork` for the search under discarded opaque pixels) arise only during rendering. [`try_resolve`] and [`try_encode`] take no scene or limits and check their own
//! arguments; [`try_env_shadow_map`] checks its own arguments. Buffers whose
//! size grows with the raster, the vertex or triangle count or a shadow map's texels are
//! reserved fallibly and give [`RenderError::OutOfMemory`] when the reservation fails. A
//! triangle's clipped polygon (at most nine corners) and the shadow pass's lists of lights and
//! maps (a few entries per light of the scene) are allocated as usual. Rows and shadow maps are
//! computed on several threads only with the `parallel` feature. A render that succeeds gives the
//! same output either way. Errors are the same too, except the `needed` count of
//! [`RenderError::TooMuchWork`] for shadow maps past the limit, which depends on where each
//! thread stops counting, and [`RenderError::OutOfMemory`], which the parallel build can return
//! where the serial build succeeds because it can hold the scratch buffers of two bands at once.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod raster;
mod resolve;
mod scene;
mod shade;
mod shadow;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

pub use raster::{GUARD_BAND, Target, try_render};
pub use resolve::{Image, try_encode, try_render_cpu, try_resolve};
pub use scene::{EnvInfo, Light, Material, Scene, ShadowMap, TexInfo, Vertex};
pub use shade::{linear_to_srgb, sample, srgb_to_linear};
pub use shadow::{MAX_SHADOW_TEXELS, shadow_map_size, shadow_point_face_size, shadow_sizes, try_env_shadow_map, try_shadow_maps};

/// Why a scene could not be rendered, resolved or encoded.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum RenderError {
    /// A side of the raster is zero.
    #[error("raster size {0}x{1} has a zero side")]
    BadSize(u32, u32),
    /// The pixel count is over [`Limits::max_pixels`], a side is over 2^30, or a buffer would be
    /// larger than `isize::MAX` bytes.
    #[error("raster size {0}x{1} needs more pixels than the limits or the target allow")]
    TooLarge(u32, u32),
    /// The index list is not a whole number of triangles.
    #[error("the scene holds {0} indices, which is not a multiple of 3")]
    BadIndexCount(usize),
    /// [`Scene::opaque_count`] is over the index count or not a multiple of 3.
    #[error("the opaque count {opaque} does not split the {len} indices into whole triangles")]
    BadOpaqueCount { opaque: u32, len: usize },
    /// A triangle (by number) in the first [`Scene::opaque_count`] indices has a visible
    /// shadows-only material, which is drawn only in the transparent pass. [`try_render`] returns
    /// it.
    #[error("triangle {0} is in the opaque range but its material draws only shadows")]
    ShadowsOnlyOpaque(usize),
    /// The triangle count is over [`Limits::max_triangles`].
    #[error("the scene holds {count} triangles, over the limit of {limit}")]
    TooManyTriangles { count: u64, limit: u64 },
    /// The vertex count is over [`Limits::max_vertices`].
    #[error("the scene holds {count} vertices, over the limit of {limit}")]
    TooManyVertices { count: u64, limit: u64 },
    /// An index names a vertex that does not exist.
    #[error("index {position} names vertex {index}, which does not exist")]
    BadIndex { position: usize, index: u32 },
    /// A vertex has a value that is not finite or names a material that does not exist.
    #[error("vertex {0} is not finite or names a material that does not exist")]
    BadVertex(usize),
    /// A vertex's clip-space position is not finite.
    #[error("vertex {0} is outside the range the clip transform can hold")]
    BadClipPosition(usize),
    /// The clip or view transform, the eye or the camera direction is not finite.
    #[error("the camera transform is not finite")]
    BadTransform,
    /// A material value is not finite or out of range (an alpha mode other than 0 to 2, or a
    /// metallic, roughness or occlusion strength outside 0 to 1, for example), or names a texture
    /// that does not exist.
    #[error("material {0} has a value that is not finite or out of range, or names a texture that does not exist")]
    BadMaterial(usize),
    /// A texture has a zero side, or its span runs past [`Scene::texels`].
    #[error("texture {0} has a zero side or runs past the texels")]
    BadTexture(usize),
    /// A texel is not finite, or its alpha is outside 0 to 1.
    #[error("texel {0} is not finite or has an alpha outside 0 to 1")]
    BadTexel(usize),
    /// A light has a kind other than 0 to 2, a value that is not finite, a cone cosine outside −1
    /// to 1, (a spot light) an inner cone cosine below the outer one, a shadow darkness outside 0
    /// to 1, or (a parallel or spot light) a direction whose length differs from 1 by more than
    /// 0.001.
    #[error(
        "light {0} has an unknown kind, a value that is not finite, a cone cosine outside -1 to 1 or out of order, a shadow darkness outside 0 to 1 or a direction that is not unit length"
    )]
    BadLight(usize),
    /// The light count is over [`Limits::max_lights`].
    #[error("the scene holds {count} lights, over the limit of {limit}")]
    TooManyLights { count: usize, limit: usize },
    /// A light (by index) or the environment light (`None`) refers to a shadow map that is not in
    /// [`Scene::shadows`]: [`try_render`] needs the maps [`try_shadow_maps`] makes, or the mark
    /// −1 for no shadow.
    #[error("a light refers to a shadow map that does not exist")]
    MissingShadowMap(Option<usize>),
    /// The environment light has a value that is not finite, a dominant direction that is neither
    /// zero nor unit length (within 0.001), a key or shadow darkness outside 0 to 1, no radiance
    /// level, or a radiance level or irradiance map that is not in [`Scene::textures`].
    #[error(
        "the environment light has a value that is not finite or out of range, a direction that is not unit length or names a texture that does not exist"
    )]
    BadEnvironment,
    /// A shadow map has a zero size, a scale or transform that is not finite, a negative penumbra
    /// radius, or a span that runs past [`Scene::shadow_texels`].
    #[error("shadow map {0} has a bad size, scale, transform, radius or span")]
    BadShadowMap(usize),
    /// A shadow texel is NaN.
    #[error("shadow texel {0} is NaN")]
    BadShadowTexel(usize),
    /// The inputs of a shadow map are out of range: a zero size, a radius that is not positive and
    /// finite, a center that is not finite, a direction whose length differs from 1 by more than
    /// 0.001, or inputs (a scene's vertices far from the origin, for one) whose map would hold a
    /// value that is not finite in `f32`.
    #[error("shadow map inputs are out of range")]
    BadShadowInputs,
    /// The shadow texels do not fit `u32` offsets.
    #[error("the shadow texels do not fit 32-bit offsets")]
    TooManyShadowTexels,
    /// Rasterizing or rendering shadow maps would take more coverage or depth tests than
    /// [`Limits::max_work`]. The count stops at the first triangle past the limit (for shadow
    /// maps counted on several threads with the `parallel` feature, within one triangle per
    /// thread), so `needed` is at least
    /// `limit + 1` and may be below the scene's total.
    #[error("the render needs at least {needed} tests, over the limit of {limit}")]
    TooMuchWork { needed: u64, limit: u64 },
    /// A resolve factor is zero or does not divide both sides of the target.
    #[error("resolve factor {k} does not divide the target size {width}x{height}")]
    BadFactor { k: u32, width: u32, height: u32 },
    /// A buffer does not hold width × height values.
    #[error("the buffer holds {found} values where {expected} are needed")]
    BadDataLength { expected: usize, found: usize },
    /// A pixel's color is not finite, or its alpha is negative or over 1. [`try_render`] returns
    /// it for a rendered pixel whose color or depth overflowed `f32` (finite inputs near the top
    /// of its range).
    #[error("pixel {0} has a color that is not finite or an alpha out of range")]
    BadPixel(usize),
    /// Memory for a target, an image, a shadow map or a working buffer could not be reserved.
    #[error("not enough memory for the render")]
    OutOfMemory,
}

/// Caps on sizes and work, checked before the target is allocated.
/// `#[non_exhaustive]`: start from [`Limits::default`] and set the fields you change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Most raster pixels (width × height). Default 2^24.
    pub max_pixels: u64,
    /// Most triangles in a scene. Default 2^22.
    pub max_triangles: u64,
    /// Most vertices in a scene, used or not. Default 3 × 2^22.
    pub max_vertices: u64,
    /// Most lights in a scene. Default 1024.
    pub max_lights: usize,
    /// Most coverage tests of the visibility and transparent passes: the pixels of every clipped
    /// triangle's bounding box, plus one per clipped triangle. Default 2^31.
    pub max_work: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_pixels: 1 << 24, max_triangles: 1 << 22, max_vertices: 3 << 22, max_lights: 1024, max_work: 1 << 31 }
    }
}

/// The largest allocation in bytes that a `Vec` can make on the target.
const MAX_ALLOC_BYTES: u64 = isize::MAX as u64;

/// The longest raster side, whatever the limits: row and band arithmetic in `i32` has room
/// above it.
const MAX_SIDE: u32 = 1 << 30;

/// Bytes the renderer holds per raster pixel: color (16), depth (4) and visibility (20).
const BYTES_PER_PIXEL: u64 = 40;

impl Limits {
    /// The pixel count of a `w × h` raster, or the error the limits give for that size.
    /// Nothing is allocated.
    pub fn check(&self, w: u32, h: u32) -> Result<usize, RenderError> {
        if w == 0 || h == 0 {
            return Err(RenderError::BadSize(w, h));
        }
        let pixels = u64::from(w) * u64::from(h);
        let bytes = pixels.checked_mul(BYTES_PER_PIXEL).ok_or(RenderError::TooLarge(w, h))?;
        if pixels > self.max_pixels || bytes > MAX_ALLOC_BYTES || w > MAX_SIDE || h > MAX_SIDE {
            return Err(RenderError::TooLarge(w, h));
        }
        usize::try_from(pixels).map_err(|_| RenderError::TooLarge(w, h))
    }
}

/// An empty `Vec` with room for `len` items, or [`RenderError::OutOfMemory`] when the
/// reservation fails.
fn try_with_capacity<T>(len: usize) -> Result<Vec<T>, RenderError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| RenderError::OutOfMemory)?;
    Ok(v)
}

/// A `Vec` of `len` copies of `value`, reserved fallibly.
fn filled<T: Clone>(len: usize, value: T) -> Result<Vec<T>, RenderError> {
    let mut v = try_with_capacity(len)?;
    v.resize(len, value);
    Ok(v)
}

/// Runs `f(chunk index, chunk of a, chunk of b)` over `a` and `b` split into chunks of `len`
/// items, on several threads with the `parallel` feature. `len` is at least 1. Returns the error
/// of the chunk with the lowest index that fails, so the result is the same either way.
fn for_each_chunk_pair<A, B, F>(a: &mut [A], b: &mut [B], len: usize, f: F) -> Result<(), RenderError>
where
    A: Send,
    B: Send,
    F: Fn(usize, &mut [A], &mut [B]) -> Result<(), RenderError> + Sync + Send,
{
    let len = len.max(1);
    #[cfg(feature = "parallel")]
    let first = a
        .par_chunks_mut(len)
        .zip(b.par_chunks_mut(len))
        .enumerate()
        .filter_map(|(i, (a, b))| f(i, a, b).err().map(|e| (i, e)))
        .min_by_key(|&(i, _)| i);
    #[cfg(not(feature = "parallel"))]
    let first = a.chunks_mut(len).zip(b.chunks_mut(len)).enumerate().find_map(|(i, (a, b))| f(i, a, b).err().map(|e| (i, e)));
    first.map_or(Ok(()), |(_, e)| Err(e))
}

/// Runs `f` on each item, on several threads with the `parallel` feature, otherwise in the order
/// of `items`.
fn for_each_item<T, F>(items: &[T], f: F)
where
    T: Sync,
    F: Fn(&T) + Sync + Send,
{
    #[cfg(feature = "parallel")]
    items.par_iter().for_each(f);
    #[cfg(not(feature = "parallel"))]
    items.iter().for_each(f);
}

/// `f` of each item, in the order of `items`, on several threads. The output is reserved
/// fallibly.
#[cfg(feature = "parallel")]
fn map_collect<T, U, F>(items: &[T], f: F) -> Result<Vec<U>, RenderError>
where
    T: Sync,
    U: Send,
    F: Fn(&T) -> U + Sync + Send,
{
    let mut out = try_with_capacity(items.len())?;
    out.par_extend(items.par_iter().map(f));
    Ok(out)
}

/// `f` of each item, in the order of `items`. The output is reserved fallibly.
#[cfg(not(feature = "parallel"))]
fn map_collect<T, U, F>(items: &[T], f: F) -> Result<Vec<U>, RenderError>
where
    T: Sync,
    U: Send,
    F: Fn(&T) -> U + Sync + Send,
{
    let mut out = try_with_capacity(items.len())?;
    out.extend(items.iter().map(f));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_that_cannot_be_reserved_is_out_of_memory() {
        assert_eq!(filled(usize::MAX / 2, [0.0f32; 4]).err(), Some(RenderError::OutOfMemory));
        assert_eq!(try_with_capacity::<f32>(usize::MAX / 2).err(), Some(RenderError::OutOfMemory));
        assert_eq!(map_collect(&[1u8, 2, 3], |&x| u32::from(x) * 2), Ok(vec![2, 4, 6]));
        assert_eq!(filled(3, 1.5f32), Ok(vec![1.5; 3]));
    }

    #[test]
    fn the_first_failing_chunk_gives_the_error() {
        let (mut a, mut b) = (vec![0u8; 10], vec![0u8; 10]);
        let r = for_each_chunk_pair(&mut a, &mut b, 2, |i, a, _| {
            a.fill(1);
            if i >= 2 { Err(RenderError::BadPixel(i)) } else { Ok(()) }
        });
        assert_eq!(r, Err(RenderError::BadPixel(2)));
        assert_eq!(for_each_chunk_pair(&mut a, &mut b, 2, |_, _, _| Ok(())), Ok(()));
    }
}
