//! Shared format selection and bounded decoding into neutral vector scenes or raster pixels.
//!
//! Applications own only the mapping from these generic results into their document models and
//! product-specific placement options. Format detection, supported import capabilities, reader
//! selection, unit normalization, and raster codec dispatch live here.

use kurbo::{Affine, BezPath, PathEl, Point};

pub mod write;
pub use craft_file_formats::Format as DetectedFormat;

/// The maximum encoded input accepted by the import facade.
pub const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_SHAPES: usize = 200_000;
const MM_PER_PT: f64 = 25.4 / 72.0;

/// A product-neutral input family supported by this reader registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Svg,
    Svgz,
    Dxf,
    Pdf,
    Eps,
    Raster,
}

/// Reader and writer capabilities for one family in the shared registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    pub format: ImportFormat,
    pub detected_formats: &'static [craft_file_formats::Format],
    pub extensions: &'static [&'static str],
    pub write_extensions: &'static [&'static str],
    pub mime_types: &'static [&'static str],
    pub raster: bool,
    pub vector: bool,
}

/// Format, reader and writer support for callers that build dialogs or query capabilities.
pub const CAPABILITIES: &[Capability] = &[
    Capability {
        format: ImportFormat::Svg,
        detected_formats: &[craft_file_formats::Format::Svg],
        extensions: &["svg"],
        write_extensions: &["svg"],
        mime_types: &["image/svg+xml"],
        raster: false,
        vector: true,
    },
    Capability {
        format: ImportFormat::Svgz,
        detected_formats: &[craft_file_formats::Format::Svgz],
        extensions: &["svgz"],
        write_extensions: &["svgz"],
        mime_types: &["image/svg+xml"],
        raster: false,
        vector: true,
    },
    Capability {
        format: ImportFormat::Dxf,
        detected_formats: &[craft_file_formats::Format::Dxf],
        extensions: &["dxf"],
        write_extensions: &["dxf"],
        mime_types: &["image/vnd.dxf", "application/dxf"],
        raster: false,
        vector: true,
    },
    Capability {
        format: ImportFormat::Pdf,
        detected_formats: &[craft_file_formats::Format::Pdf],
        extensions: &["pdf"],
        write_extensions: &["pdf"],
        mime_types: &["application/pdf"],
        raster: false,
        vector: true,
    },
    Capability {
        format: ImportFormat::Eps,
        detected_formats: &[craft_file_formats::Format::Eps, craft_file_formats::Format::PostScript, craft_file_formats::Format::Illustrator],
        extensions: &["eps", "ps", "ai"],
        write_extensions: &[],
        mime_types: &["application/postscript"],
        raster: false,
        vector: true,
    },
    Capability {
        format: ImportFormat::Raster,
        detected_formats: &[
            craft_file_formats::Format::Png,
            craft_file_formats::Format::Jpeg,
            craft_file_formats::Format::Bmp,
            craft_file_formats::Format::Gif,
            craft_file_formats::Format::Tiff,
            craft_file_formats::Format::WebP,
        ],
        extensions: &["png", "jpg", "jpeg", "bmp", "gif", "tif", "tiff", "webp"],
        write_extensions: &["png", "jpg", "jpeg"],
        mime_types: &["image/png", "image/jpeg", "image/bmp", "image/gif", "image/tiff", "image/webp"],
        raster: true,
        vector: false,
    },
];

/// A vector path with generic source metadata. `tag` and `layer` are source labels; applications
/// may interpret their own conventions while the geometry and parsing stay shared.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneShape {
    pub path: BezPath,
    pub tag: Option<String>,
    pub class: Option<String>,
    pub stroke: Option<String>,
    /// Parsed RGB stroke color when the source stroke is a concrete CSS color.
    pub stroke_rgb: Option<[u8; 3]>,
    pub layer: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorScene {
    pub format: ImportFormat,
    pub width_mm: f64,
    pub height_mm: f64,
    pub shapes: Vec<SceneShape>,
    pub warnings: Vec<String>,
    /// Additional warnings omitted by the SVG reader's bounded warning collection.
    pub dropped_warnings: usize,
}

#[derive(Debug, Clone)]
pub struct RasterImage {
    pub format: ImportFormat,
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA pixels, row-major.
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum Imported {
    Vector(VectorScene),
    Raster(RasterImage),
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_shapes: usize,
    pub max_raster_width: u32,
    pub max_raster_height: u32,
    pub max_raster_pixels: u64,
    pub max_raster_alloc_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: MAX_INPUT_BYTES,
            max_shapes: MAX_SHAPES,
            max_raster_width: 10_000,
            max_raster_height: 10_000,
            max_raster_pixels: 16_000_000,
            max_raster_alloc_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unrecognized or unsupported import format")]
    Unsupported,
    #[error("input is {size} bytes; limit is {limit}")]
    InputLimit { size: usize, limit: usize },
    #[error("{0}")]
    Decode(String),
    #[error("import produced invalid or non-finite geometry")]
    InvalidGeometry,
    #[error("import exceeds the configured shape limit")]
    ShapeLimit,
    #[error("format writer failed: {0}")]
    Write(String),
}

impl From<craft_pdf::write::WriteError> for Error {
    fn from(error: craft_pdf::write::WriteError) -> Self {
        Self::Write(error.to_string())
    }
}

/// Resolve detected file kinds to this reader registry. Illustrator files with PDF content use
/// the PDF path reader; PostScript-family files use the EPS/PostScript interpreter.
pub fn format_for(name: &str, bytes: &[u8]) -> Option<ImportFormat> {
    use craft_file_formats::Format as F;
    match craft_file_formats::detect(name, bytes)? {
        F::Svg => Some(ImportFormat::Svg),
        F::Svgz => Some(ImportFormat::Svgz),
        F::Dxf => Some(ImportFormat::Dxf),
        F::Pdf => Some(ImportFormat::Pdf),
        F::Illustrator if bytes.starts_with(b"%PDF-") => Some(ImportFormat::Pdf),
        F::Eps | F::PostScript | F::Illustrator => Some(ImportFormat::Eps),
        F::Png | F::Jpeg | F::Bmp | F::Gif | F::Tiff | F::WebP => Some(ImportFormat::Raster),
        _ => None,
    }
}

/// Return supported extensions in the same registry used by [`format_for`].
pub fn supported_extensions() -> Vec<&'static str> {
    CAPABILITIES.iter().flat_map(|capability| capability.extensions.iter().copied()).collect()
}

/// Raw detected kinds routed to one reader family, for app open-dialog and capability adapters.
pub fn supported_formats(format: ImportFormat) -> &'static [craft_file_formats::Format] {
    CAPABILITIES.iter().find(|capability| capability.format == format).map_or(&[], |capability| capability.detected_formats)
}

/// Whether the registry has a writer for an extension.
pub fn supports_write_extension(extension: &str) -> bool {
    let extension = extension.trim_start_matches('.');
    CAPABILITIES.iter().any(|capability| capability.write_extensions.iter().any(|candidate| candidate.eq_ignore_ascii_case(extension)))
}

/// Whether a family has at least one output writer in the shared registry.
pub fn supports_writer(format: ImportFormat) -> bool {
    CAPABILITIES.iter().find(|capability| capability.format == format).is_some_and(|capability| !capability.write_extensions.is_empty())
}

/// Detect, select a reader, and decode the input into a neutral vector scene or RGBA image.
pub fn import(name: &str, bytes: &[u8], limits: Limits) -> Result<Imported, Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputLimit { size: bytes.len(), limit: limits.max_input_bytes });
    }
    let format = format_for(name, bytes).ok_or(Error::Unsupported)?;
    let imported = match format {
        ImportFormat::Svg | ImportFormat::Svgz => import_svg(name, bytes, format, limits),
        ImportFormat::Dxf => import_dxf(bytes, format, limits),
        ImportFormat::Pdf => import_pdf(bytes, format, limits),
        ImportFormat::Eps => import_eps(bytes, format, limits),
        ImportFormat::Raster => import_raster(bytes, format, limits),
    }?;
    if let Imported::Vector(scene) = &imported
        && (!(scene.width_mm.is_finite() && scene.height_mm.is_finite() && scene.width_mm > 0.0 && scene.height_mm > 0.0)
            || scene.shapes.iter().any(|shape| !path_is_finite(&shape.path)))
    {
        return Err(Error::InvalidGeometry);
    }
    Ok(imported)
}

#[derive(Debug, Clone, Default)]
struct SvgTag {
    data_kind: Option<String>,
    class: Option<String>,
}

impl craft_svg::paths::Kind for SvgTag {
    const PROPERTIES: &'static [&'static str] = &["data-kind"];

    fn of(style: &craft_svg::paths::StyleMap, class: &str) -> Option<Self> {
        let data_kind = style.get("data-kind").cloned();
        let class = (!class.is_empty()).then(|| class.to_owned());
        (data_kind.is_some() || class.is_some()).then_some(Self { data_kind, class })
    }
}

fn import_svg(name: &str, bytes: &[u8], format: ImportFormat, limits: Limits) -> Result<Imported, Error> {
    let text = craft_svg::svgz::text(bytes, limits.max_input_bytes).map_err(|error| Error::Decode(error.to_string()))?;
    let imported = craft_svg::paths::import::<SvgTag>(&text, name).map_err(|error| Error::Decode(error.to_string()))?;
    if imported.shapes.len() > limits.max_shapes {
        return Err(Error::ShapeLimit);
    }
    let mut shapes = Vec::with_capacity(imported.shapes.len());
    for shape in &imported.shapes {
        let path = imported.clipped_path(shape).map_err(|error| Error::Decode(error.to_string()))?;
        shapes.push(SceneShape {
            path,
            tag: shape.kind.data_kind.clone(),
            class: shape.kind.class.clone(),
            stroke: shape.stroke.clone(),
            stroke_rgb: shape.stroke.as_deref().and_then(|value| craft_svg::color::parse_color(value).ok()).map(|color| [color.r, color.g, color.b]),
            layer: shape.layer.clone(),
        });
    }
    Ok(Imported::Vector(VectorScene {
        format,
        width_mm: imported.width_mm,
        height_mm: imported.height_mm,
        shapes,
        warnings: imported.warnings,
        dropped_warnings: imported.dropped_warnings,
    }))
}

fn import_dxf(bytes: &[u8], format: ImportFormat, limits: Limits) -> Result<Imported, Error> {
    let imported = craft_dxf::paths::import(bytes).map_err(|error| Error::Decode(error.to_string()))?;
    if imported.paths.len() > limits.max_shapes {
        return Err(Error::ShapeLimit);
    }
    let (width_mm, height_mm, transform) = match imported.bounds {
        Some(bounds) => (
            (bounds.width() * imported.unit_to_mm).max(1.0),
            (bounds.height() * imported.unit_to_mm).max(1.0),
            Affine::new([imported.unit_to_mm, 0.0, 0.0, -imported.unit_to_mm, -bounds.x0 * imported.unit_to_mm, bounds.y1 * imported.unit_to_mm]),
        ),
        None => (100.0, 150.0, Affine::IDENTITY),
    };
    let shapes = imported
        .paths
        .into_iter()
        .map(|path| SceneShape { path: transform * path, tag: None, class: None, stroke: None, stroke_rgb: None, layer: None })
        .collect();
    Ok(Imported::Vector(VectorScene { format, width_mm, height_mm, shapes, warnings: imported.warnings, dropped_warnings: 0 }))
}

fn import_pdf(bytes: &[u8], format: ImportFormat, limits: Limits) -> Result<Imported, Error> {
    let imported = craft_pdf::paths::import_page(
        bytes,
        0,
        craft_pdf::paths::Limits { max_input_bytes: limits.max_input_bytes, max_paths: limits.max_shapes, ..Default::default() },
    )
    .map_err(|error| Error::Decode(error.to_string()))?;
    let page = imported.pages.into_iter().next().ok_or_else(|| Error::Decode("PDF has no pages".into()))?;
    let bounds = page.crop_box;
    let mut warnings = imported.warnings;
    if page.omitted_text > 0 || page.omitted_images > 0 {
        warnings.push(format!("PDF omitted {} text and {} image items", page.omitted_text, page.omitted_images));
    }
    let shapes = page
        .paths
        .into_iter()
        .map(|path| SceneShape { path: Affine::scale(MM_PER_PT) * path, tag: None, class: None, stroke: None, stroke_rgb: None, layer: None })
        .collect();
    Ok(Imported::Vector(VectorScene {
        format,
        width_mm: bounds.width() * MM_PER_PT,
        height_mm: bounds.height() * MM_PER_PT,
        shapes,
        warnings,
        dropped_warnings: 0,
    }))
}

fn import_eps(bytes: &[u8], format: ImportFormat, limits: Limits) -> Result<Imported, Error> {
    let imported = craft_eps::paths::import(
        bytes,
        craft_eps::paths::Limits { max_input_bytes: limits.max_input_bytes, max_paths: limits.max_shapes, ..Default::default() },
    )
    .map_err(|error| Error::Decode(error.to_string()))?;
    let bounds = imported.page_bounds;
    let transform = Affine::scale(MM_PER_PT);
    let mut warnings = imported.warnings;
    if imported.preview_fallback {
        warnings.push("embedded raster preview was not imported as paths".into());
    }
    let shapes = imported
        .paths
        .into_iter()
        .map(|path| SceneShape { path: transform * path, tag: None, class: None, stroke: None, stroke_rgb: None, layer: None })
        .collect();
    Ok(Imported::Vector(VectorScene {
        format,
        width_mm: (bounds.width() * MM_PER_PT).max(1.0),
        height_mm: (bounds.height() * MM_PER_PT).max(1.0),
        shapes,
        warnings,
        dropped_warnings: 0,
    }))
}

fn import_raster(bytes: &[u8], format: ImportFormat, limits: Limits) -> Result<Imported, Error> {
    let rgba = craft_raster_codecs::common::decode_rgba(
        bytes,
        craft_raster_codecs::common::DecodeLimits {
            max_file_bytes: limits.max_input_bytes,
            max_width: limits.max_raster_width,
            max_height: limits.max_raster_height,
            max_pixels: limits.max_raster_pixels,
            max_alloc_bytes: limits.max_raster_alloc_bytes,
        },
    )
    .map_err(|error| Error::Decode(error.to_string()))?;
    let (width, height) = rgba.dimensions();
    Ok(Imported::Raster(RasterImage { format, width, height, rgba: rgba.into_raw() }))
}

fn path_is_finite(path: &BezPath) -> bool {
    path.elements().iter().all(|element| {
        let finite = |point: Point| point.x.is_finite() && point.y.is_finite();
        match *element {
            PathEl::MoveTo(point) | PathEl::LineTo(point) => finite(point),
            PathEl::QuadTo(a, b) => finite(a) && finite(b),
            PathEl::CurveTo(a, b, c) => finite(a) && finite(b) && finite(c),
            PathEl::ClosePath => true,
        }
    })
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn pdf_based_illustrator_is_routed_to_pdf_reader() {
        let bytes = b"%PDF-1.7\n/AIPrivateData";
        assert_eq!(format_for("illustration.ai", bytes), Some(ImportFormat::Pdf));
    }
}
