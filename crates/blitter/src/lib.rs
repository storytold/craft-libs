//! `craft-blitter`: Glacier-style zero-allocation SIMD blending, compositing, and raster acceleration engine.
//!
//! Provides:
//! - All 27 Photoshop blend modes + 6 Paint.NET modes with exact Photoshop oracle parity.
//! - Fixed-point integer fast paths for RGBA8 and RGBA16 surfaces with zero float conversion round-trips.
//! - Float HDR and LDR blitting for RGBA32F (`[[f32; 4]]`).
//! - Special Eight Fill dynamics, Straight and Premultiplied alpha, clipping group isolation, and text gamma blending.
//! - Safe SIMD chunk auto-vectorization and Rayon parallelization on native targets.
//! - Clean, safe compilation on `wasm32-unknown-unknown`.
//! - `#![forbid(unsafe_code)]` and zero panics.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod blend;
pub mod fixed;
pub mod float;
pub mod simd;
pub mod special;

pub use blend::{
    EDGE, TEXT_GAMMA, blend_channel, blend_channel_hdr, blend_rgb, blend_rgb_hdr, blend_rgb_within, clip_color, color_burn, color_dodge, divide,
    hard_mix_ps, lum, sat, set_lum, set_sat, soft_light_ps, vivid_light_generic, vivid_light_ps,
};
pub use fixed::{div255, div65535, lerp255, lerp65535};
pub use float::{composite_f32, composite_fill_f32, composite_gamma_f32, composite_normal_f32, composite_premultiplied_f32};
pub use special::{blend_rgb_fill, fill_is_special};

/// All 27 Photoshop blend modes + 6 Paint.NET modes + PassThrough.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Dissolve,
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
    // Paint.NET compatibility modes
    Reflect,
    Glow,
    Negation,
    Xor,
    PaintNetColorBurn,
    PaintNetColorDodge,
    PassThrough,
}

impl BlendMode {
    /// The 27 Photoshop layer blend modes in canonical UI menu order.
    pub const LAYER_MODES: [BlendMode; 27] = [
        BlendMode::Normal,
        BlendMode::Dissolve,
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

    /// Returns true if this blend mode is one of Photoshop's "Special Eight" fill modes.
    #[inline]
    pub const fn is_special_fill(self) -> bool {
        special::fill_is_special(self)
    }

    /// Returns true if this blend mode requires non-separable (full RGB) evaluation.
    #[inline]
    pub const fn is_non_separable(self) -> bool {
        matches!(
            self,
            BlendMode::DarkerColor | BlendMode::LighterColor | BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity
        )
    }
}

/// Alpha compositing representation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum AlphaMode {
    /// Straight (unassociated) alpha: color channels are stored un-premultiplied.
    #[default]
    Straight,
    /// Premultiplied (associated) alpha: color channels are multiplied by alpha.
    Premultiplied,
}

// ---- Public Slice Blitting APIs ----

/// Blits an RGBA8 source slice into destination in-place using the fixed-point integer fast path.
///
/// Guaranteed zero heap allocations and non-panicking slice bounds clamping.
pub fn blit_rgba8(dst: &mut [u8], src: &[u8], mode: BlendMode, opacity: f32, fill: f32) {
    blit_rgba8_opts(dst, src, mode, opacity, fill, AlphaMode::Straight, false);
}

/// Blits an RGBA8 source slice with configurable alpha mode and clipping mask isolation.
pub fn blit_rgba8_opts(dst: &mut [u8], src: &[u8], mode: BlendMode, opacity: f32, fill: f32, alpha_mode: AlphaMode, clipping: bool) {
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
            fixed::blit_rgba8_fixed(d, s, mode, opacity, fill, alpha_mode, clipping);
        });
        return;
    }

    fixed::blit_rgba8_fixed(dst_slice, src_slice, mode, opacity, fill, alpha_mode, clipping);
}

/// Blits an RGBA8 slice with clipping mask silhouette isolation.
pub fn blit_rgba8_clipped(dst: &mut [u8], src: &[u8], mode: BlendMode, opacity: f32, fill: f32) {
    blit_rgba8_opts(dst, src, mode, opacity, fill, AlphaMode::Straight, true);
}

/// Fast premultiplied alpha blit for RGBA8.
pub fn blit_rgba8_premultiplied(dst: &mut [u8], src: &[u8], opacity: f32) {
    blit_rgba8_opts(dst, src, BlendMode::Normal, opacity, 1.0, AlphaMode::Premultiplied, false);
}

/// Blits an RGBA16 source slice into destination in-place using the 64-bit integer fast path.
///
/// Guaranteed zero heap allocations and non-panicking slice bounds clamping.
pub fn blit_rgba16(dst: &mut [u16], src: &[u16], mode: BlendMode, opacity: f32, fill: f32) {
    blit_rgba16_opts(dst, src, mode, opacity, fill, AlphaMode::Straight, false);
}

/// Blits an RGBA16 source slice with configurable alpha mode and clipping mask isolation.
pub fn blit_rgba16_opts(dst: &mut [u16], src: &[u16], mode: BlendMode, opacity: f32, fill: f32, alpha_mode: AlphaMode, clipping: bool) {
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
            fixed::blit_rgba16_fixed(d, s, mode, opacity, fill, alpha_mode, clipping);
        });
        return;
    }

    fixed::blit_rgba16_fixed(dst_slice, src_slice, mode, opacity, fill, alpha_mode, clipping);
}

/// Blits an RGBA16 slice with clipping mask silhouette isolation.
pub fn blit_rgba16_clipped(dst: &mut [u16], src: &[u16], mode: BlendMode, opacity: f32, fill: f32) {
    blit_rgba16_opts(dst, src, mode, opacity, fill, AlphaMode::Straight, true);
}

/// Fast premultiplied alpha blit for RGBA16.
pub fn blit_rgba16_premultiplied(dst: &mut [u16], src: &[u16], opacity: f32) {
    blit_rgba16_opts(dst, src, BlendMode::Normal, opacity, 1.0, AlphaMode::Premultiplied, false);
}

/// Blits an RGBA32F source slice into destination in-place.
///
/// If `hdr` is true, values above 1.0 are preserved without premature clamping.
pub fn blit_rgba32f(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, fill: f32, hdr: bool) {
    blit_rgba32f_opts(dst, src, mode, opacity, fill, 1.0, hdr, AlphaMode::Straight, false);
}

/// Blits an RGBA32F source slice with optical text gamma blending.
pub fn blit_rgba32f_gamma(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, fill: f32, gamma: f32, hdr: bool) {
    blit_rgba32f_opts(dst, src, mode, opacity, fill, gamma, hdr, AlphaMode::Straight, false);
}

/// Blits an RGBA32F slice with full options (gamma, hdr, alpha mode, clipping).
#[allow(clippy::too_many_arguments)]
pub fn blit_rgba32f_opts(
    dst: &mut [[f32; 4]],
    src: &[[f32; 4]],
    mode: BlendMode,
    opacity: f32,
    fill: f32,
    gamma: f32,
    hdr: bool,
    alpha_mode: AlphaMode,
    clipping: bool,
) {
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
            float::blit_rgba32f_inner(d, s, mode, opacity, fill, gamma, hdr, alpha_mode, clipping);
        });
        return;
    }

    float::blit_rgba32f_inner(dst_slice, src_slice, mode, opacity, fill, gamma, hdr, alpha_mode, clipping);
}

/// Blits an RGBA32F slice with clipping mask silhouette isolation.
pub fn blit_rgba32f_clipped(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, fill: f32, hdr: bool) {
    blit_rgba32f_opts(dst, src, mode, opacity, fill, 1.0, hdr, AlphaMode::Straight, true);
}

/// Fast premultiplied alpha blit for RGBA32F.
pub fn blit_rgba32f_premultiplied(dst: &mut [[f32; 4]], src: &[[f32; 4]], opacity: f32) {
    blit_rgba32f_opts(dst, src, BlendMode::Normal, opacity, 1.0, 1.0, false, AlphaMode::Premultiplied, false);
}

// ---- Standalone Pixel Compositors for Drop-In photocraft-compose Parity ----

/// Straight-alpha source over straight-alpha backdrop (W3C/PDF general formula).
#[inline]
pub fn composite(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    composite_f32(mode, backdrop, source, opacity, false)
}

/// Composite with optical text gamma blending.
#[inline]
pub fn composite_gamma(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32, gamma: f32) -> [f32; 4] {
    composite_gamma_f32(mode, backdrop, source, opacity, gamma, false)
}

/// Composite with separate opacity and fill, supporting Special Eight fill dynamics.
#[inline]
pub fn composite_fill(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32, fill: f32, gamma: f32) -> [f32; 4] {
    composite_fill_f32(mode, backdrop, source, opacity, fill, gamma, false)
}

/// Normal mode mixing for RGBA32F pixels.
#[inline]
pub fn composite_normal_rgb(backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    composite_normal_f32(backdrop, source, opacity)
}
