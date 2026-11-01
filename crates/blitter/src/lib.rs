//! `craft-blitter`: CPU raster blending and Porter-Duff compositing for the Craft apps.
//!
//! Covers in-place slice blitting and single-pixel compositing across `RGBA8` (`&mut [u8]`),
//! `RGBA16` (`&mut [u16]`), and `RGBA32F` (`&mut [[f32; 4]]`), with:
//! - 32 documented blend modes from W3C Compositing and Blending Level 1, ISO 32000-1 / PDF
//!   Blend Modes, Khronos `GL_NV_blend_equation_advanced` / `VG_KHR_advanced_blending`, Adobe's
//!   public Blending Modes reference, Pegtop Blend Modes, and Paint.NET's Blend Modes reference.
//! - Straight (unassociated) and premultiplied (associated) alpha conventions ([`AlphaMode`]).
//! - Porter-Duff `source-over` and `source-atop` compositing operators ([`CompositeOp`]).
//! - Fixed-point integer fast paths for `RGBA8` and `RGBA16` with deterministic nearest-integer
//!   rounding ([`div255`], [`div65535`]).
//! - Safe 8-pixel Structure-of-Arrays (SoA) chunked auto-vectorization (`slice::as_chunks::<8>()`)
//!   with an exact IEEE-754 scalar fallback (`float::blit_rgba32f_scalar`) and native Rayon
//!   parallelization above [`simd::PAR_THRESHOLD`].
//! - `#![forbid(unsafe_code)]`, zero panics on malformed or mismatched slices, zero heap
//!   allocations on serial paths (and on parallel paths once Rayon's global pool is initialized),
//!   and `wasm32-unknown-unknown` support.
//!
//! Spatial dithering (`Dissolve`) and layer-tree group scoping (`PassThrough`) stay in each
//! application's compositor.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod blend;
pub mod fixed;
pub mod float;
pub mod simd;

pub use blend::{
    blend_channel, blend_channel_hdr, blend_rgb, blend_rgb_hdr, blend_rgb_within, classic_color_burn, classic_color_dodge, clip_color, color_burn,
    color_dodge, divide, glow, hard_light, hard_mix, lum, negation, pin_light, reflect, sat, set_lum, set_sat, soft_light, vivid_light, xor,
};
pub use fixed::{div255, div65535, lerp255, lerp65535};
pub use float::{composite_atop_f32, composite_f32, composite_normal_f32, composite_premultiplied_f32};

/// Separable and non-separable color blend modes.
///
/// Each variant's mathematical definition is cited by URL on its corresponding kernel in
/// [`crate::blend`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
    Reflect,
    Glow,
    Negation,
    Xor,
    ClassicColorBurn,
    ClassicColorDodge,
}

impl BlendMode {
    /// The 16 blend modes defined in W3C Compositing and Blending Level 1 (§10):
    /// <https://www.w3.org/TR/compositing-1/#blending>
    pub const W3C_MODES: [BlendMode; 16] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    /// The 26 non-spatial Adobe layer blend modes (excluding `Dissolve` and `PassThrough`, which
    /// are spatial/tree operations owned by each app).
    pub const LAYER_MODES: [BlendMode; 26] = [
        BlendMode::Normal,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::DarkerColor,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
        BlendMode::LighterColor,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    /// Every [`BlendMode`] variant supported by `craft-blitter`.
    pub const ALL: [BlendMode; 32] = [
        BlendMode::Normal,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::DarkerColor,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
        BlendMode::LighterColor,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
        BlendMode::Reflect,
        BlendMode::Glow,
        BlendMode::Negation,
        BlendMode::Xor,
        BlendMode::ClassicColorBurn,
        BlendMode::ClassicColorDodge,
    ];

    /// Returns `true` if this blend mode evaluates each color channel independently.
    #[inline]
    pub const fn is_separable(self) -> bool {
        !self.is_non_separable()
    }

    /// Returns `true` if this blend mode requires all three RGB channels together.
    #[inline]
    pub const fn is_non_separable(self) -> bool {
        matches!(
            self,
            BlendMode::DarkerColor | BlendMode::LighterColor | BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity
        )
    }
}

/// Alpha representation of source and destination buffers.
///
/// Reference: W3C Compositing and Blending Level 1, §5 (Simple alpha compositing):
/// <https://www.w3.org/TR/compositing-1/#simplealphacompositing>
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum AlphaMode {
    /// Straight (unassociated) alpha: RGB channels store un-premultiplied color values in
    /// `[0, 1]` (`0..=255` for `RGBA8`, `0..=65535` for `RGBA16`).
    #[default]
    Straight,
    /// Premultiplied (associated) alpha: RGB channels store `color * alpha`.
    Premultiplied,
}

/// Porter-Duff compositing operator combined with color blending.
///
/// Reference:
/// - W3C Compositing and Blending Level 1, §9.1.2 (`source-over`) & §9.1.6 (`source-atop`), and
///   §10 (General Formula for Compositing and Blending):
///   <https://www.w3.org/TR/compositing-1/>
/// - W3C SVG Compositing Specification (`comp-op: src-over`, `comp-op: src-atop`):
///   <https://www.w3.org/TR/SVGCompositing/>
/// - Khronos `GL_NV_blend_equation_advanced`, Table X.1 (`(X, Y, Z)` weights):
///   <https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt>
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum CompositeOp {
    /// Porter-Duff `source-over` (`(X, Y, Z) = (1, 1, 1)`):
    /// `ao = as + ab * (1 - as)` and
    /// `ao * Co = (1 - as) * ab * Cb + (1 - ab) * as * Cs + as * ab * B(Cb, Cs)`.
    #[default]
    SourceOver,
    /// Porter-Duff `source-atop` (`(X, Y, Z) = (1, 0, 1)`), preserving destination alpha (used
    /// for alpha-locked painting and clipping masks):
    /// `ao = ab` and `ao * Co = (1 - as) * ab * Cb + as * ab * B(Cb, Cs)`.
    SourceAtop,
}

// ---- Public Slice Blitting APIs ----

/// Blits a straight-alpha `RGBA8` source slice over `dst` (`SourceOver`) using integer arithmetic.
pub fn blit_rgba8(dst: &mut [u8], src: &[u8], mode: BlendMode, opacity: f32) {
    blit_rgba8_opts(dst, src, mode, opacity, AlphaMode::Straight, CompositeOp::SourceOver);
}

/// Blits a straight-alpha `RGBA8` source slice onto `dst` with `SourceAtop` (preserving `dst` alpha).
pub fn blit_rgba8_atop(dst: &mut [u8], src: &[u8], mode: BlendMode, opacity: f32) {
    blit_rgba8_opts(dst, src, mode, opacity, AlphaMode::Straight, CompositeOp::SourceAtop);
}

/// Blits a premultiplied-alpha `RGBA8` source slice over `dst` (`Normal`, `SourceOver`).
pub fn blit_rgba8_premultiplied(dst: &mut [u8], src: &[u8], opacity: f32) {
    blit_rgba8_opts(dst, src, BlendMode::Normal, opacity, AlphaMode::Premultiplied, CompositeOp::SourceOver);
}

/// Blits an `RGBA8` source slice into `dst` with explicit [`AlphaMode`] and [`CompositeOp`].
///
/// Clamps to `min(dst.len() / 4, src.len() / 4)` whole pixels without panicking.
pub fn blit_rgba8_opts(dst: &mut [u8], src: &[u8], mode: BlendMode, opacity: f32, alpha_mode: AlphaMode, op: CompositeOp) {
    let num_pixels = (dst.len() / 4).min(src.len() / 4);
    if num_pixels == 0 {
        return;
    }
    let total_bytes = num_pixels * 4;
    let dst_slice = &mut dst[..total_bytes];
    let src_slice = &src[..total_bytes];

    #[cfg(not(target_arch = "wasm32"))]
    if num_pixels >= simd::PAR_THRESHOLD {
        use rayon::prelude::*;
        const CHUNK_BYTES: usize = simd::PAR_THRESHOLD * 4;
        dst_slice.par_chunks_mut(CHUNK_BYTES).zip(src_slice.par_chunks(CHUNK_BYTES)).for_each(|(d, s)| {
            fixed::blit_rgba8_fixed(d, s, mode, opacity, alpha_mode, op);
        });
        return;
    }

    fixed::blit_rgba8_fixed(dst_slice, src_slice, mode, opacity, alpha_mode, op);
}

/// Blits a straight-alpha `RGBA16` source slice over `dst` (`SourceOver`) using integer arithmetic.
pub fn blit_rgba16(dst: &mut [u16], src: &[u16], mode: BlendMode, opacity: f32) {
    blit_rgba16_opts(dst, src, mode, opacity, AlphaMode::Straight, CompositeOp::SourceOver);
}

/// Blits a straight-alpha `RGBA16` source slice onto `dst` with `SourceAtop` (preserving `dst` alpha).
pub fn blit_rgba16_atop(dst: &mut [u16], src: &[u16], mode: BlendMode, opacity: f32) {
    blit_rgba16_opts(dst, src, mode, opacity, AlphaMode::Straight, CompositeOp::SourceAtop);
}

/// Blits a premultiplied-alpha `RGBA16` source slice over `dst` (`Normal`, `SourceOver`).
pub fn blit_rgba16_premultiplied(dst: &mut [u16], src: &[u16], opacity: f32) {
    blit_rgba16_opts(dst, src, BlendMode::Normal, opacity, AlphaMode::Premultiplied, CompositeOp::SourceOver);
}

/// Blits an `RGBA16` source slice into `dst` with explicit [`AlphaMode`] and [`CompositeOp`].
///
/// Clamps to `min(dst.len() / 4, src.len() / 4)` whole pixels without panicking.
pub fn blit_rgba16_opts(dst: &mut [u16], src: &[u16], mode: BlendMode, opacity: f32, alpha_mode: AlphaMode, op: CompositeOp) {
    let num_pixels = (dst.len() / 4).min(src.len() / 4);
    if num_pixels == 0 {
        return;
    }
    let total_words = num_pixels * 4;
    let dst_slice = &mut dst[..total_words];
    let src_slice = &src[..total_words];

    #[cfg(not(target_arch = "wasm32"))]
    if num_pixels >= simd::PAR_THRESHOLD {
        use rayon::prelude::*;
        const CHUNK_WORDS: usize = simd::PAR_THRESHOLD * 4;
        dst_slice.par_chunks_mut(CHUNK_WORDS).zip(src_slice.par_chunks(CHUNK_WORDS)).for_each(|(d, s)| {
            fixed::blit_rgba16_fixed(d, s, mode, opacity, alpha_mode, op);
        });
        return;
    }

    fixed::blit_rgba16_fixed(dst_slice, src_slice, mode, opacity, alpha_mode, op);
}

/// Blits a straight-alpha `RGBA32F` source slice over `dst` (`SourceOver`).
///
/// When `hdr` is `true`, modes that support unbounded light (`LinearDodge`, `Divide`, and the
/// non-separable W3C modes) do not clamp positive channel values at `1.0`.
pub fn blit_rgba32f(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, hdr: bool) {
    blit_rgba32f_opts(dst, src, mode, opacity, hdr, AlphaMode::Straight, CompositeOp::SourceOver);
}

/// Blits a straight-alpha `RGBA32F` source slice onto `dst` with `SourceAtop` (preserving `dst` alpha).
pub fn blit_rgba32f_atop(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, hdr: bool) {
    blit_rgba32f_opts(dst, src, mode, opacity, hdr, AlphaMode::Straight, CompositeOp::SourceAtop);
}

/// Blits a premultiplied-alpha `RGBA32F` source slice over `dst` (`Normal`, `SourceOver`).
pub fn blit_rgba32f_premultiplied(dst: &mut [[f32; 4]], src: &[[f32; 4]], opacity: f32) {
    blit_rgba32f_opts(dst, src, BlendMode::Normal, opacity, false, AlphaMode::Premultiplied, CompositeOp::SourceOver);
}

/// Blits an `RGBA32F` source slice into `dst` with explicit `hdr`, [`AlphaMode`], and [`CompositeOp`].
pub fn blit_rgba32f_opts(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, hdr: bool, alpha_mode: AlphaMode, op: CompositeOp) {
    let n = dst.len().min(src.len());
    if n == 0 {
        return;
    }
    let dst_slice = &mut dst[..n];
    let src_slice = &src[..n];

    #[cfg(not(target_arch = "wasm32"))]
    if n >= simd::PAR_THRESHOLD {
        use rayon::prelude::*;
        dst_slice.par_chunks_mut(simd::PAR_THRESHOLD).zip(src_slice.par_chunks(simd::PAR_THRESHOLD)).for_each(|(d, s)| {
            float::blit_rgba32f_serial(d, s, mode, opacity, hdr, alpha_mode, op);
        });
        return;
    }

    float::blit_rgba32f_serial(dst_slice, src_slice, mode, opacity, hdr, alpha_mode, op);
}

// ---- Single-Pixel Compositing Helpers ----

/// Straight-alpha `source-over` compositing for a single SDR `RGBA32F` pixel (`W3C` §10).
#[inline]
pub fn composite(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    composite_f32(mode, backdrop, source, opacity, false)
}

/// Straight-alpha `source-atop` compositing for a single SDR `RGBA32F` pixel (`W3C` §9.1.6 + §10).
#[inline]
pub fn composite_atop(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    composite_atop_f32(mode, backdrop, source, opacity, false)
}

/// Straight-alpha `Normal` `source-over` compositing for a single `RGBA32F` pixel.
#[inline]
pub fn composite_normal_rgb(backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    composite_normal_f32(backdrop, source, opacity)
}
