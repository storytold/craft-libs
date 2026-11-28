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
//! [`try_render`] checks the scene, size, references, spans, finite values, geometry counts,
//! lights, and coverage work before allocating the raster target. Shading can still return
//! [`RenderError::BadPixel`] after rasterization. [`try_resolve`] checks its supplied target and
//! factor; [`try_encode`] checks an image's pixels and does not take a scene or [`Limits`]. Rows
//! run on several threads with rayon.
//!
//! [`Camera::fit`] and [`Camera::fit_scene`] frame a box or a scene's drawn vertices from a view
//! direction, and [`Camera::apply`] writes a camera's clip and view transforms into a scene.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod camera;
mod raster;
mod resolve;
mod scene;
mod shade;
mod shadow;

pub use camera::{Bounds, Camera, CameraError, FIT_FOV_Y, Projection};
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
    /// A light has a kind or falloff other than 0 to 2, a value that is not finite, a cone cosine outside −1
    /// to 1, (a spot light) an inner cone cosine below the outer one, a shadow darkness outside 0
    /// to 1, or (a parallel or spot light) a direction whose length differs from 1 by more than
    /// 0.001.
    #[error(
        "light {0} has an unknown kind or falloff, a value that is not finite, a cone cosine outside -1 to 1 or out of order, a shadow darkness outside 0 to 1 or a direction that is not unit length"
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
    /// maps, counted on several threads, within one triangle per thread), so `needed` is at least
    /// `limit + 1` and may be below the scene's total. The search under discarded opaque pixels
    /// is charged as one step, the opaque pass's whole count added at once, so when that charge
    /// passes the limit `needed` is twice the opaque pass's tests and can be well above
    /// `limit + 1`.
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
    #[error("pixel {0} has a color that is not finite, a rendered depth that is NaN, or an alpha out of range")]
    BadPixel(usize),
    /// A camera could not be fitted or could not form a scene's transforms.
    #[error("invalid camera: {0}")]
    BadCamera(CameraError),
    /// The renderer could not reserve memory for screen triangles.
    #[error("not enough memory for the render")]
    OutOfMemory,
}

/// The default [`Limits::max_pixels`]: 2^24 raster pixels.
pub const MAX_RENDER_PIXELS: u64 = 1 << 24;

/// The default [`Limits::max_work`]: 2^31 coverage tests, 128 full layers of the largest raster.
pub const MAX_RENDER_WORK: u64 = 1 << 31;

/// The default [`Limits::max_triangles`]: 2^24 triangles.
pub const MAX_RENDER_TRIANGLES: u64 = 1 << 24;

/// The default [`Limits::max_vertices`]: 2^24 vertices.
pub const MAX_RENDER_VERTICES: u64 = 1 << 24;

/// Caps on sizes and work, checked before the target is allocated.
/// `#[non_exhaustive]`: start from [`Limits::default`] and set the fields you change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Most raster pixels (width × height). Default [`MAX_RENDER_PIXELS`].
    pub max_pixels: u64,
    /// Most triangles in a scene. Default [`MAX_RENDER_TRIANGLES`].
    pub max_triangles: u64,
    /// Most vertices in a scene, used or not. Default [`MAX_RENDER_VERTICES`].
    pub max_vertices: u64,
    /// Most lights in a scene. Default 1024.
    pub max_lights: usize,
    /// Most coverage tests of one [`try_render`]: the pixels of every clipped triangle's bounding
    /// box, plus one per clipped triangle, for the visibility and transparent passes, and the
    /// opaque triangles' count once more when an opaque triangle's material can discard a
    /// fragment (single-sided, or an alpha mode other than 0), for the search under discarded
    /// pixels. [`try_shadow_maps`] counts its own tests against the same limit. Default
    /// [`MAX_RENDER_WORK`].
    pub max_work: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_pixels: MAX_RENDER_PIXELS,
            max_triangles: MAX_RENDER_TRIANGLES,
            max_vertices: MAX_RENDER_VERTICES,
            max_lights: 1024,
            max_work: MAX_RENDER_WORK,
        }
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

    /// The error the limits give for a scene of `triangles` triangles and `vertices` vertices,
    /// if any. Nothing is allocated.
    pub fn check_geometry(&self, triangles: u64, vertices: u64) -> Result<(), RenderError> {
        if triangles > self.max_triangles {
            return Err(RenderError::TooManyTriangles { count: triangles, limit: self.max_triangles });
        }
        if vertices > self.max_vertices {
            return Err(RenderError::TooManyVertices { count: vertices, limit: self.max_vertices });
        }
        Ok(())
    }
}
