//! Bounded, premultiplied RGBA8 raster output, with explicit viewport and background.

use crate::{
    interpret::{InterpreterSettings, InterpreterWarning},
    syntax::page::Page,
};
pub use hayro::{ColorReplacement, RenderCache};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

/// Hard bounds on output allocation. These do not bound every backend scratch allocation.
pub const MAX_SIDE: u32 = 8192;
pub const MAX_PIXELS: u64 = 64_000_000;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub scale: [f32; 2],
    /// Explicit device-pixel viewport. When absent the scaled page size is floored, with
    /// at least one pixel per side. Callers that need edge coverage can supply a rounded-up size.
    pub size: Option<[u32; 2]>,
    /// Top-left viewport origin in device pixels of the scaled page.
    pub offset: [f32; 2],
    /// Straight-alpha sRGB bytes; output pixels are always premultiplied.
    pub background: [u8; 4],
    /// Generic solid-colour mapping; reading presets and print bypass policy belong to callers.
    /// It also maps a plate's gray film (`InterpreterSettings::plate`), so a plate render that
    /// must keep its tints leaves it `None`. Knockout clears are never mapped.
    pub color_replacement: Option<ColorReplacement>,
}

impl Default for Options {
    fn default() -> Self {
        Self { scale: [1.0, 1.0], size: None, offset: [0.0, 0.0], background: [0; 4], color_replacement: None }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub image_decode_failed: bool,
    /// Reserved by the backend, which currently does not emit this warning.
    pub unsupported_font: bool,
    /// The backend reported omitted, failed, or budget-limited content.
    pub content_truncated: bool,
}

impl Diagnostics {
    pub fn is_empty(self) -> bool {
        !self.image_decode_failed && !self.unsupported_font && !self.content_truncated
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("PDF page has an empty or non-finite crop box")]
    InvalidPage,
    #[error("PDF raster scale or viewport offset is invalid")]
    InvalidOptions,
    #[error("PDF raster exceeds the output size limit")]
    OutputLimit,
    #[error("PDF raster buffer length does not match its dimensions")]
    BufferLength,
    #[error("PDF raster buffer could not be allocated")]
    Allocation,
    #[error("PDF rendering was cancelled")]
    Cancelled,
    /// After this error callers must discard the document parser and render cache.
    #[error("PDF renderer failed internally")]
    BackendPanic,
}

/// Validate geometry before converting sizes or allocating. A zero-size output is rejected.
pub fn dimensions(page: &Page<'_>, options: &Options) -> Result<[u32; 2], Error> {
    let (w, h) = page.render_dimensions();
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return Err(Error::InvalidPage);
    }
    if options.scale.iter().any(|s| !s.is_finite() || *s <= 0.0) || options.offset.iter().any(|s| !s.is_finite()) {
        return Err(Error::InvalidOptions);
    }
    let size = if let Some(size) = options.size {
        size
    } else {
        let scaled = [w * options.scale[0], h * options.scale[1]];
        if scaled.iter().any(|s| !s.is_finite() || *s > MAX_SIDE as f32) {
            return Err(Error::OutputLimit);
        }
        [scaled[0].floor().max(1.0) as u32, scaled[1].floor().max(1.0) as u32]
    };
    if size.iter().any(|s| *s == 0 || *s > MAX_SIDE) || u64::from(size[0]) * u64::from(size[1]) > MAX_PIXELS {
        return Err(Error::OutputLimit);
    }
    Ok(size)
}

/// Render directly into a caller-owned buffer. The buffer must be discarded after any error.
/// Settings preserve caller fonts, annotation policy, OCG identities, cancellation and warning
/// callbacks. Diagnostics report only warnings the backend actually emits, not all fidelity loss.
pub fn render_into<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    options: &Options,
    buffer: &mut [u8],
) -> Result<Diagnostics, Error> {
    let [width, height] = dimensions(page, options)?;
    let expected = u64::from(width) * u64::from(height) * 4;
    if u64::try_from(buffer.len()).ok() != Some(expected) {
        return Err(Error::BufferLength);
    }
    let cancelled = || settings.cancelled.as_ref().is_some_and(|c| c.load(Ordering::Relaxed));
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let warnings = Arc::new(AtomicU8::new(0));
    let sink = warnings.clone();
    let original = settings.warning_sink.clone();
    let settings = InterpreterSettings {
        warning_sink: Arc::new(move |warning| {
            let bit = match warning {
                InterpreterWarning::ImageDecodeFailure => 1,
                InterpreterWarning::UnsupportedFont => 2,
                InterpreterWarning::ContentTruncated => 4,
            };
            sink.fetch_or(bit, Ordering::Relaxed);
            original(warning);
        }),
        ..settings.clone()
    };
    let rs = hayro::RenderSettings {
        color_replacement: options.color_replacement,
        x_scale: options.scale[0],
        y_scale: options.scale[1],
        width: Some(width as u16),
        height: Some(height as u16),
        x_offset: options.offset[0],
        y_offset: options.offset[1],
        bg_color: hayro::vello_cpu::color::AlphaColor::from_rgba8(
            options.background[0],
            options.background[1],
            options.background[2],
            options.background[3],
        ),
    };
    catch_unwind(AssertUnwindSafe(|| hayro::render_into(page, cache, &settings, &rs, buffer))).map_err(|_| Error::BackendPanic)?;
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let bits = warnings.load(Ordering::Relaxed);
    Ok(Diagnostics { image_decode_failed: bits & 1 != 0, unsupported_font: bits & 2 != 0, content_truncated: bits & 4 != 0 })
}

/// Pixel-word aligned premultiplied RGBA8 output; no GUI or image encoding dependency.
#[derive(Debug)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    words: Vec<u32>,
    pub diagnostics: Diagnostics,
}
impl Raster {
    pub fn bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.words)
    }
    pub fn into_words(self) -> Vec<u32> {
        self.words
    }
}

pub fn render<'a>(page: &'a Page<'a>, cache: &RenderCache<'a>, settings: &InterpreterSettings, options: &Options) -> Result<Raster, Error> {
    let [width, height] = dimensions(page, options)?;
    let count = usize::try_from(u64::from(width) * u64::from(height)).map_err(|_| Error::OutputLimit)?;
    let mut words = Vec::new();
    words.try_reserve_exact(count).map_err(|_| Error::Allocation)?;
    words.resize(count, 0u32);
    let diagnostics = render_into(page, cache, settings, options, bytemuck::cast_slice_mut(&mut words))?;
    Ok(Raster { width, height, words, diagnostics })
}
