//! Bounded extraction of page vector paths from PDF content streams.
//!
//! Paths are returned in page-oriented points with `/UserUnit`, rotation and crop origin applied.
//! Text glyphs and raster images are counted as omitted content; clipping and paint are not applied.

use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, RectExt, SoftMask, TransformExt,
};
use kurbo::{Affine, BezPath, Rect};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Limits for vector path extraction. They bound the output model, not interpreter peak memory.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_pages: usize,
    pub max_paths: usize,
    pub max_path_elements: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_input_bytes: 64 * 1024 * 1024, max_pages: 1000, max_paths: 200_000, max_path_elements: 2_000_000 }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Open(#[from] crate::read::OpenError),
    #[error(transparent)]
    Interpret(#[from] crate::read::ExecutionError),
    #[error("PDF contains more than {limit} pages")]
    Pages { limit: usize },
    #[error("PDF has no page at index {index}")]
    PageIndex { index: usize },
    #[error("PDF path output exceeds configured limit")]
    OutputLimit,
}

/// Extracted paths for one PDF page, normalized to an origin-zero page in physical points.
#[derive(Debug, Clone)]
pub struct Page {
    pub crop_box: Rect,
    pub paths: Vec<BezPath>,
    pub omitted_text: usize,
    pub omitted_images: usize,
    pub clips_seen: usize,
}

/// Pages from one PDF. Paths from separate pages are never combined.
#[derive(Debug, Clone)]
pub struct Imported {
    pub pages: Vec<Page>,
    pub warnings: Vec<String>,
}

/// Open and interpret vector content using shared PDF parsing and interpreter APIs.
pub fn import(bytes: &[u8], limits: Limits) -> Result<Imported, Error> {
    import_selected(bytes, limits, None)
}

/// Interpret only the requested page and apply path and element limits to that page's output.
pub fn import_page(bytes: &[u8], page_index: usize, limits: Limits) -> Result<Imported, Error> {
    import_selected(bytes, limits, Some(page_index))
}

fn import_selected(bytes: &[u8], limits: Limits, selected: Option<usize>) -> Result<Imported, Error> {
    let options = crate::read::OpenOptions { max_input_bytes: limits.max_input_bytes, ..Default::default() };
    let pdf = crate::read::open_slice_with_options(bytes, None, &options)?;
    if pdf.pages().len() > limits.max_pages {
        return Err(Error::Pages { limit: limits.max_pages });
    }
    if selected.is_some_and(|index| index >= pdf.pages().len()) {
        return Err(Error::PageIndex { index: selected.unwrap_or_default() });
    }
    let cache = InterpreterCache::new();
    let truncated = Arc::new(AtomicBool::new(false));
    let truncation_flag = truncated.clone();
    let settings = InterpreterSettings {
        warning_sink: Arc::new(move |warning| {
            if matches!(warning, hayro_interpret::InterpreterWarning::ContentTruncated) {
                truncation_flag.store(true, Ordering::Relaxed);
            }
        }),
        ..InterpreterSettings::default()
    };
    let mut pages = Vec::with_capacity(pdf.pages().len());
    let mut total_paths = 0usize;
    let mut total_elements = 0usize;
    for (page_index, page) in pdf.pages().iter().enumerate() {
        if selected.is_some_and(|index| index != page_index) {
            continue;
        }
        let source_crop_box = page.intersected_crop_box().to_kurbo();
        let (width, height) = page.render_dimensions();
        let crop_box = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let initial_transform = page.initial_transform(true).to_kurbo();
        let mut context = hayro_interpret::Context::new(initial_transform, source_crop_box, &cache, pdf.xref(), settings.clone());
        let mut device = Collector {
            max_paths: if selected.is_some() { limits.max_paths } else { limits.max_paths.saturating_sub(total_paths) },
            max_elements: if selected.is_some() { limits.max_path_elements } else { limits.max_path_elements.saturating_sub(total_elements) },
            ..Collector::default()
        };
        crate::read::interpret_page(page, &mut context, &mut device)?;
        if device.exceeded {
            return Err(Error::OutputLimit);
        }
        if device.paths.len() > limits.max_paths || device.paths.iter().map(|path| path.elements().len()).sum::<usize>() > limits.max_path_elements {
            return Err(Error::OutputLimit);
        }
        total_paths = total_paths.saturating_add(device.paths.len());
        total_elements = device.paths.iter().fold(total_elements, |count, path| count.saturating_add(path.elements().len()));
        if total_paths > limits.max_paths || total_elements > limits.max_path_elements {
            return Err(Error::OutputLimit);
        }
        pages.push(Page {
            crop_box,
            paths: device.paths,
            omitted_text: device.omitted_text,
            omitted_images: device.omitted_images,
            clips_seen: device.clips_seen,
        });
    }
    let mut warnings = Vec::new();
    if pages.iter().any(|page| page.omitted_text > 0) {
        warnings.push("text glyphs are not returned as paths".into());
    }
    if pages.iter().any(|page| page.omitted_images > 0) {
        warnings.push("raster images are not returned as paths".into());
    }
    if pages.iter().any(|page| page.clips_seen > 0) {
        warnings.push("clip paths are not applied to extracted geometry".into());
    }
    if truncated.load(Ordering::Relaxed) {
        warnings.push("PDF content was truncated by the interpreter".into());
    }
    Ok(Imported { pages, warnings })
}

#[derive(Default)]
struct Collector {
    paths: Vec<BezPath>,
    last_fill_path: Option<BezPath>,
    omitted_text: usize,
    omitted_images: usize,
    clips_seen: usize,
    max_paths: usize,
    max_elements: usize,
    elements: usize,
    exceeded: bool,
}

impl Device<'_> for Collector {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, path: &BezPath, transform: Affine, _: &Paint<'_>, mode: &PathDrawMode) {
        if path.is_empty() {
            return;
        }
        let path = transform * path.clone();
        match mode {
            PathDrawMode::Fill(_) => self.last_fill_path = Some(path.clone()),
            PathDrawMode::Stroke(_) => {
                if self.last_fill_path.take().as_ref() == Some(&path) {
                    return;
                }
            }
        }
        let count = path.elements().len();
        if self.paths.len() >= self.max_paths || self.elements.saturating_add(count) > self.max_elements {
            self.exceeded = true;
            return;
        }
        self.elements = self.elements.saturating_add(count);
        self.paths.push(path);
    }
    fn push_clip_path(&mut self, _: &ClipPath) {
        self.clips_seen = self.clips_seen.saturating_add(1);
    }
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(&mut self, _: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, _: &GlyphDrawMode) {
        self.last_fill_path = None;
        self.omitted_text = self.omitted_text.saturating_add(1);
    }
    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {
        self.last_fill_path = None;
        self.omitted_images = self.omitted_images.saturating_add(1);
    }
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}
