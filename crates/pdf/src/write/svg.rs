//! Bounded SVG parsing and vector drawing on a PDF surface.
//!
//! This adapter owns the usvg version required by krilla-svg. Callers pass their font and
//! fallback options, while this module checks input, embedded SVGs, and output/filter budgets
//! before converting SVG to PDF drawing operations.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use krilla_svg::SurfaceExt;

pub use krilla_svg::SvgSettings;
pub use usvg;

/// Resource limits for parsing SVG and drawing it to PDF.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum decoded bytes in one SVG document.
    pub input_bytes: usize,
    /// Maximum SVG element and reference depth.
    pub depth: usize,
    /// Maximum XML nodes in one document.
    pub xml_nodes: u32,
    /// Maximum total decoded bytes of embedded SVGs.
    pub embedded_svg_bytes: usize,
    /// Maximum number of embedded SVG documents nested within one another.
    pub embedded_svg_depth: usize,
    /// Maximum encoded bytes retained for embedded raster images.
    pub embedded_image_bytes: usize,
    /// Maximum PDF surface area, measured in 96 dpi pixels.
    pub vector_pixels: u64,
    /// Maximum raster pixels used by filters after `filter_scale` is applied.
    pub filter_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            input_bytes: 16 * 1024 * 1024,
            depth: 64,
            xml_nodes: 100_000,
            embedded_svg_bytes: 32 * 1024 * 1024,
            embedded_svg_depth: 4,
            embedded_image_bytes: 32 * 1024 * 1024,
            vector_pixels: 256_000_000,
            filter_pixels: 256_000_000,
        }
    }
}

/// Why an SVG could not be parsed or drawn within its limits.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The bounded SVG text/XML preflight refused the input.
    #[error("SVG preflight failed: {0}")]
    Preflight(String),
    /// usvg could not construct the SVG tree.
    #[error("SVG parsing failed: {0}")]
    Parse(String),
    /// The requested PDF area or filter raster exceeds its budget.
    #[error("SVG drawing exceeds the PDF area or filter budget")]
    DrawLimit,
    /// krilla-svg refused to draw the parsed tree.
    #[error("krilla-svg could not draw the parsed tree")]
    Draw,
}

/// An SVG tree parsed within [`Limits`], ready to draw as PDF vectors.
pub struct Document {
    tree: usvg::Tree,
    limits: Limits,
}

impl Document {
    /// Parse SVG/SVGZ bytes after bounded shared preflight. `options` controls the caller's font
    /// database, family selection, and fallback policy; its file-reading and image resolvers are
    /// replaced with a data-only resolver governed by `limits`.
    pub fn parse<'a>(bytes: &[u8], limits: Limits, mut options: usvg::Options<'a>) -> Result<Self, Error> {
        let text = bounded_text(bytes, limits.input_bytes, limits).map_err(|e| Error::Preflight(e.to_string()))?;
        options.image_href_resolver = image_resolver(limits);
        let tree = usvg::Tree::from_str(&text, &options).map_err(|e| Error::Parse(e.to_string()))?;
        Ok(Self { tree, limits })
    }

    /// Draw the parsed tree as PDF vectors. The limits account for the destination size and the
    /// renderer's filter scale before krilla-svg allocates filter rasters.
    pub fn draw(&self, surface: &mut krilla::surface::Surface<'_>, size: krilla::geom::Size, settings: SvgSettings) -> Result<(), Error> {
        let width = f64::from(size.width());
        let height = f64::from(size.height());
        let scale = f64::from(settings.filter_scale);
        if !width.is_finite() || !height.is_finite() || !scale.is_finite() || width <= 0.0 || height <= 0.0 || scale <= 0.0 {
            return Err(Error::DrawLimit);
        }
        let area = width * height * (96.0_f64 / 72.0).powi(2);
        let filter_area = area * scale * scale;
        if !area.is_finite() || !filter_area.is_finite() || area > self.limits.vector_pixels as f64 || filter_area > self.limits.filter_pixels as f64
        {
            return Err(Error::DrawLimit);
        }
        surface.draw_svg(&self.tree, size, settings).ok_or(Error::Draw)
    }
}

#[derive(Default)]
struct Usage {
    svg_bytes: AtomicUsize,
    svg_depth: AtomicUsize,
    image_bytes: AtomicUsize,
}

fn shared_limits(bytes: usize, limits: Limits) -> craft_svg::tree::Limits {
    craft_svg::tree::Limits {
        bytes,
        depth: limits.depth,
        refuse_entity_markup: true,
        xml_nodes: limits.xml_nodes,
        embedded_bytes: limits.embedded_svg_bytes,
        embedded_depth: limits.embedded_svg_depth,
        ..Default::default()
    }
}

fn bounded_text(bytes: &[u8], byte_limit: usize, limits: Limits) -> Result<std::borrow::Cow<'_, str>, craft_svg::tree::Error> {
    craft_svg::tree::text(bytes, &shared_limits(byte_limit, limits))
}

fn reserve(counter: &AtomicUsize, amount: usize, limit: usize) -> bool {
    counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(amount).filter(|next| *next <= limit)).is_ok()
}

fn embedded_svg(usage: &Usage, limits: Limits, bytes: &[u8], options: &usvg::Options<'_>) -> Option<usvg::ImageKind> {
    let used = usage.svg_bytes.load(Ordering::Relaxed);
    let remaining = limits.embedded_svg_bytes.saturating_sub(used);
    let text = bounded_text(bytes, remaining.min(limits.input_bytes), limits).ok()?;
    if !reserve(&usage.svg_bytes, text.len(), limits.embedded_svg_bytes) {
        return None;
    }
    let depth = usage.svg_depth.fetch_add(1, Ordering::Relaxed);
    if depth >= limits.embedded_svg_depth {
        usage.svg_depth.fetch_sub(1, Ordering::Relaxed);
        return None;
    }
    let parsed = usvg::Tree::from_str(&text, options).ok();
    usage.svg_depth.fetch_sub(1, Ordering::Relaxed);
    parsed.map(usvg::ImageKind::SVG)
}

fn image_resolver(limits: Limits) -> usvg::ImageHrefResolver<'static> {
    let usage = Arc::new(Usage::default());
    let default_data = usvg::ImageHrefResolver::default_data_resolver();
    usvg::ImageHrefResolver {
        resolve_data: Box::new(move |mime, bytes, options| {
            if mime == "image/svg+xml" || (mime == "text/plain" && (craft_svg::sniff::is_svg(&bytes) || craft_svg::sniff::is_svgz(&bytes))) {
                return embedded_svg(&usage, limits, &bytes, options);
            }
            if !reserve(&usage.image_bytes, bytes.len(), limits.embedded_image_bytes) {
                return None;
            }
            default_data(mime, bytes, options)
        }),
        resolve_string: Box::new(|_, _| None),
    }
}
