//! One PDF page as an Enhanced Metafile of vector drawing calls (feature `emf`), for printing
//! through GDI: paths and glyph outlines become filled and stroked paths, images keep their
//! pixels, and clips become path clips. GDI has no transparency model, so a page that needs one
//! (translucent paint, a blend mode, a soft mask, a group with opacity, a pattern or shading,
//! overprint) is refused with [`Error::NeedsRaster`], and the caller draws that page as pixels.
//!
//! The picture is in points, y down, over the page's displayed box (`/UserUnit`, `/Rotate` and
//! the crop origin applied), as the `paths` module returns paths. [`Pictures`] opens a PDF once for
//! all its pages.

use craft_metafile::write::{Dash, Image as Bitmap, Op, Pen};
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Device, FillRule, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, LumaData, Paint, PathDrawMode, RectExt,
    SoftMask, StrokeProps, TransformExt,
};
use kurbo::{Affine, BezPath, Rect};
use std::sync::Arc;

/// Bounds on one page's picture.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The largest PDF read.
    pub max_input_bytes: usize,
    /// The most drawing operations in the picture.
    pub max_ops: usize,
    /// The most path elements over all paths.
    pub max_path_elements: usize,
    /// The most image pixels over all images.
    pub max_image_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_input_bytes: 512 * 1024 * 1024, max_ops: 500_000, max_path_elements: 8_000_000, max_image_pixels: 100_000_000 }
    }
}

/// Why a page has no picture.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Open(#[from] crate::read::OpenError),
    #[error(transparent)]
    Interpret(#[from] crate::read::ExecutionError),
    #[error("PDF has no page at index {index}")]
    PageIndex { index: usize },
    #[error("the page's picture would exceed the configured limits")]
    OutputLimit,
    #[error("the page has an empty or unusable size")]
    EmptyPage,
    /// The page uses something GDI cannot draw; draw it as pixels instead.
    #[error("the page needs raster printing: {0}")]
    NeedsRaster(&'static str),
}

/// A page's picture.
#[derive(Debug, Clone)]
pub struct PageEmf {
    /// The EMF file.
    pub bytes: Vec<u8>,
    /// The page's displayed size in points: the picture's size.
    pub size: (f64, f64),
}

/// A PDF opened to draw its pages as pictures.
pub struct Pictures {
    pdf: crate::syntax::Pdf,
    limits: Limits,
}

impl Pictures {
    /// Opens `bytes`.
    pub fn open(bytes: &[u8], limits: Limits) -> Result<Self, Error> {
        let options = crate::read::OpenOptions { max_input_bytes: limits.max_input_bytes, ..Default::default() };
        Ok(Self { pdf: crate::read::open_slice_with_options(bytes, None, &options)?, limits })
    }

    /// The number of pages.
    pub fn len(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Whether the PDF has no pages.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Page `index` as an EMF whose description names `app` and `title`.
    pub fn page(&self, index: usize, app: &str, title: &str) -> Result<PageEmf, Error> {
        let page = self.pdf.pages().get(index).ok_or(Error::PageIndex { index })?;
        let (width, height) = page.render_dimensions();
        let size = (f64::from(width), f64::from(height));
        if !(size.0.is_finite() && size.1.is_finite() && size.0 > 0.0 && size.1 > 0.0) {
            return Err(Error::EmptyPage);
        }
        let cache = InterpreterCache::new();
        let initial = page.initial_transform(true).to_kurbo();
        let crop = page.intersected_crop_box().to_kurbo();
        let mut context = hayro_interpret::Context::new(initial, crop, &cache, self.pdf.xref(), InterpreterSettings::default());
        let mut device = Emitter::new(self.limits);
        crate::read::interpret_page(page, &mut context, &mut device)?;
        if let Some(reason) = device.refused {
            return Err(Error::NeedsRaster(reason));
        }
        if device.exceeded {
            return Err(Error::OutputLimit);
        }
        let mut ops = device.ops;
        ops.extend(std::iter::repeat_n(Op::Unclip, device.clips));
        let emf = craft_metafile::write::emf(&ops, Rect::new(0.0, 0.0, size.0, size.1), app, title).ok_or(Error::EmptyPage)?;
        Ok(PageEmf { bytes: emf, size })
    }
}

/// The device the page is interpreted into: it collects operations, or the first reason the
/// page can't be drawn with them.
struct Emitter {
    limits: Limits,
    ops: Vec<Op>,
    elements: usize,
    pixels: u64,
    /// Clips pushed and not yet popped.
    clips: usize,
    refused: Option<&'static str>,
    exceeded: bool,
}

impl Emitter {
    fn new(limits: Limits) -> Self {
        Self { limits, ops: Vec::new(), elements: 0, pixels: 0, clips: 0, refused: None, exceeded: false }
    }

    fn stopped(&self) -> bool {
        self.refused.is_some() || self.exceeded
    }

    fn refuse(&mut self, reason: &'static str) {
        self.refused.get_or_insert(reason);
    }

    fn push(&mut self, op: Op) {
        if self.stopped() {
            return;
        }
        let elements = match &op {
            Op::Fill { path, .. } | Op::Stroke { path, .. } | Op::Clip { path, .. } => path.elements().len(),
            Op::Image { .. } | Op::Unclip => 0,
        };
        self.elements = self.elements.saturating_add(elements);
        if self.ops.len() >= self.limits.max_ops || self.elements > self.limits.max_path_elements {
            self.exceeded = true;
            return;
        }
        self.ops.push(op);
    }

    /// The opaque colour of `paint`; refuses patterns and translucent colours.
    fn colour(&mut self, paint: &Paint<'_>) -> Option<[u8; 3]> {
        match paint {
            Paint::Color(c) => {
                let [r, g, b, a] = c.to_rgba().to_rgba8();
                if a == 255 {
                    Some([r, g, b])
                } else {
                    self.refuse("translucent paint");
                    None
                }
            }
            Paint::Pattern(_) => {
                self.refuse("a pattern or shading");
                None
            }
        }
    }

    fn fill(&mut self, path: &BezPath, transform: Affine, paint: &Paint<'_>, rule: FillRule) {
        if let Some(rgb) = self.colour(paint) {
            self.push(Op::Fill { path: transform * path.clone(), even_odd: matches!(rule, FillRule::EvenOdd), rgb });
        }
    }

    fn stroke(&mut self, path: &BezPath, transform: Affine, paint: &Paint<'_>, props: &StrokeProps) {
        let Some(rgb) = self.colour(paint) else { return };
        if !pen_is_finite(props) {
            return self.refuse("a stroke with a width, miter limit or dash that is not a number");
        }
        match uniform_scale(transform) {
            Some(scale) => {
                let width = f64::from(props.line_width) * scale;
                let dash = dash(props, scale);
                let pen = Pen { rgb, width, cap: cap(props.line_cap), join: join(props.line_join), miter: f64::from(props.miter_limit), dash };
                self.push(Op::Stroke { path: transform * path.clone(), pen });
            }
            // A pen is round in the picture's space; under a squashed or skewed transform the
            // stroke's outline is filled instead. Dashing that outline has no bound on the number of
            // dashes, so a dashed stroke is handed back.
            None if !props.dash_array.is_empty() => self.refuse("a dashed stroke under a squashed or skewed transform"),
            None => {
                let outline = kurbo::stroke(path.iter(), &stroke_style(props), &kurbo::StrokeOpts::default(), 0.01);
                self.push(Op::Fill { path: transform * outline, even_odd: false, rgb });
            }
        }
    }

    fn image(&mut self, image: Image<'_, '_>, transform: Affine) {
        let mut decoded: Option<(Bitmap, (f32, f32))> = None;
        match image {
            Image::Raster(raster) => raster.with_rgba(
                |data, alpha| {
                    let (width, height, factors) = (data.width(), data.height(), data.scale_factors());
                    decoded = rgba(&data, alpha.as_ref()).map(|rgba| (Bitmap { width, height, rgba }, factors));
                },
                None,
            ),
            Image::Stencil(stencil) => {
                let mut paint_rgb = None;
                stencil.with_stencil(
                    |luma, paint| {
                        paint_rgb = Some(match paint {
                            Paint::Color(c) => Ok(c.to_rgba().to_rgba8()),
                            Paint::Pattern(_) => Err(()),
                        });
                        if let Some(Ok([r, g, b, 255])) = paint_rgb {
                            let rgba = luma.data.iter().flat_map(|&a| [r, g, b, a]).collect();
                            decoded = Some((Bitmap { width: luma.width, height: luma.height, rgba }, luma.scale_factors));
                        }
                    },
                    None,
                );
                match paint_rgb {
                    Some(Err(())) => return self.refuse("a pattern or shading"),
                    Some(Ok([.., a])) if a != 255 => return self.refuse("translucent paint"),
                    _ => {}
                }
            }
        }
        let Some((bitmap, (sx, sy))) = decoded else {
            return self.refuse("an image that could not be decoded");
        };
        self.pixels = self.pixels.saturating_add(u64::from(bitmap.width) * u64::from(bitmap.height));
        if self.pixels > self.limits.max_image_pixels {
            self.exceeded = true;
            return;
        }
        let xf = transform * Affine::scale_non_uniform(f64::from(sx), f64::from(sy));
        self.push(Op::Image { image: Arc::new(bitmap), xf, opacity: 1.0 });
    }
}

/// Straight-alpha RGBA of an image and its alpha (resampled to the image's size when it has
/// another); `None` when the samples don't fill the image.
fn rgba(data: &hayro_interpret::ImageData, alpha: Option<&LumaData>) -> Option<Vec<u8>> {
    let (width, height) = (data.width() as usize, data.height() as usize);
    let count = width.checked_mul(height)?;
    let mut out = Vec::with_capacity(count.checked_mul(4)?);
    match data {
        hayro_interpret::ImageData::Rgb(rgb) => {
            for px in rgb.data.get(..count.checked_mul(3)?)?.as_chunks::<3>().0 {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
        hayro_interpret::ImageData::Luma(luma) => {
            for &v in luma.data.get(..count)? {
                out.extend_from_slice(&[v, v, v, 255]);
            }
        }
    }
    if let Some(alpha) = alpha {
        let (aw, ah) = (alpha.width as usize, alpha.height as usize);
        if aw == 0 || ah == 0 || alpha.data.len() < aw.checked_mul(ah)? {
            return Some(out);
        }
        for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = ((i % width) as u64, (i / width) as u64);
            let (aw64, ah64) = (aw as u64, ah as u64);
            let at = (y * ah64 / height.max(1) as u64) * aw64 + x * aw64 / width.max(1) as u64;
            px[3] = usize::try_from(at).ok().and_then(|at| alpha.data.get(at)).copied().unwrap_or(255);
        }
    }
    Some(out)
}

/// The scale of a transform that scales both axes alike (turns and mirrors allowed); `None` for
/// one that squashes or skews.
fn uniform_scale(t: Affine) -> Option<f64> {
    let [a, b, c, d, ..] = t.as_coeffs();
    let (x, y) = ((a * a + b * b).sqrt(), (c * c + d * d).sqrt());
    let square = (a * c + b * d).abs() <= 1e-6 * x * y;
    (x.is_finite() && x > 0.0 && (x - y).abs() <= 1e-3 * x && square).then_some(x)
}

/// Whether the stroke's width, miter limit and dash pattern are all finite and none negative.
fn pen_is_finite(props: &StrokeProps) -> bool {
    let ok = |v: f32| v.is_finite() && v >= 0.0;
    ok(props.line_width) && ok(props.miter_limit) && props.dash_offset.is_finite() && props.dash_array.iter().all(|v| ok(*v))
}

fn dash(props: &StrokeProps, scale: f64) -> Option<Dash> {
    (!props.dash_array.is_empty())
        .then(|| Dash { pattern: props.dash_array.iter().map(|v| f64::from(*v) * scale).collect(), offset: f64::from(props.dash_offset) * scale })
}

fn cap(c: kurbo::Cap) -> craft_metafile::Cap {
    match c {
        kurbo::Cap::Butt => craft_metafile::Cap::Flat,
        kurbo::Cap::Round => craft_metafile::Cap::Round,
        kurbo::Cap::Square => craft_metafile::Cap::Square,
    }
}

fn join(j: kurbo::Join) -> craft_metafile::Join {
    match j {
        kurbo::Join::Bevel => craft_metafile::Join::Bevel,
        kurbo::Join::Miter => craft_metafile::Join::Miter,
        kurbo::Join::Round => craft_metafile::Join::Round,
    }
}

fn stroke_style(props: &StrokeProps) -> kurbo::Stroke {
    let style = kurbo::Stroke::new(f64::from(props.line_width))
        .with_caps(props.line_cap)
        .with_join(props.line_join)
        .with_miter_limit(f64::from(props.miter_limit));
    if props.dash_array.is_empty() { style } else { style.with_dashes(f64::from(props.dash_offset), props.dash_array.iter().map(|v| f64::from(*v))) }
}

impl<'a> Device<'a> for Emitter {
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>) {
        if mask.is_some() {
            self.refuse("a soft mask");
        }
    }

    fn set_blend_mode(&mut self, blend_mode: BlendMode) {
        if blend_mode != BlendMode::Normal {
            self.refuse("a blend mode");
        }
    }

    fn set_overprint(&mut self, fill: bool, stroke: bool, _mode: u8) {
        if fill || stroke {
            self.refuse("overprint");
        }
    }

    fn set_knockout(&mut self) {
        self.refuse("a knockout group");
    }

    fn draw_path(&mut self, path: &BezPath, transform: Affine, paint: &Paint<'a>, draw_mode: &PathDrawMode) {
        match draw_mode {
            PathDrawMode::Fill(rule) => self.fill(path, transform, paint, *rule),
            PathDrawMode::Stroke(props) => self.stroke(path, transform, paint, props),
        }
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.clips = self.clips.saturating_add(1);
        self.push(Op::Clip { path: clip_path.path.clone(), even_odd: matches!(clip_path.fill, FillRule::EvenOdd) });
    }

    fn push_transparency_group(&mut self, opacity: f32, mask: Option<SoftMask<'a>>, blend_mode: BlendMode) {
        if opacity < 1.0 || mask.is_some() || blend_mode != BlendMode::Normal {
            self.refuse("a transparency group");
        }
    }

    fn draw_glyph(&mut self, glyph: &Glyph<'a>, transform: Affine, glyph_transform: Affine, paint: &Paint<'a>, draw_mode: &GlyphDrawMode) {
        match (glyph, draw_mode) {
            (_, GlyphDrawMode::Clip | GlyphDrawMode::Invisible) => {}
            (Glyph::Type3(g), _) => g.interpret(self, transform, glyph_transform, paint),
            (Glyph::Outline(g), GlyphDrawMode::Fill) => self.fill(&g.outline(), transform * glyph_transform, paint, FillRule::NonZero),
            (Glyph::Outline(g), GlyphDrawMode::Stroke(props)) => self.stroke(&(glyph_transform * g.outline()), transform, paint, props),
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, transform: Affine) {
        if !self.stopped() {
            self.image(image, transform);
        }
    }

    fn pop_clip_path(&mut self) {
        if self.clips > 0 {
            self.clips -= 1;
            self.push(Op::Unclip);
        }
    }

    fn pop_transparency_group(&mut self) {}
}

#[cfg(test)]
mod tests;
