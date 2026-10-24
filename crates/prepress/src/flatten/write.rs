//! Writes a display list, or a [`Flattened`] page, as PDF content through krilla.
//!
//! Colors print as [`InkSet::resolve`] makes them (aliases, spots printed as process color)
//! and are written by [`to_pdf_color`], with one exception: an overprinting process-only
//! color that names fewer than four process inks is written as a DeviceN color of just the
//! inks it names. DeviceCMYK would set the unnamed plates to 0 under `OPM 0`, and `OPM 1`
//! would also leave alone the plates of named inks at 0; a DeviceN color paints exactly its
//! colorants when it overprints (ISO 32000-2 §8.6.7), whatever `OPM` says.
//!
//! Overprint is written as `op`/`OP`/`OPM` on each item, never as blend modes or knockout
//! groups.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft_pdf::interpret::color::CmykConversion;
use craft_pdf::write::backend::blend::{self as kblend, GroupOptions};
use craft_pdf::write::backend::color::device_n::{self, Colorant, DeviceNSpace};
use craft_pdf::write::backend::color::{Color as PdfColor, RegularColor, cmyk};
use craft_pdf::write::backend::configure::PdfVersion;
use craft_pdf::write::backend::geom::{Path, PathBuilder, Point as KPoint, Size, Transform};
use craft_pdf::write::backend::image::{BitsPerComponent, CustomImage, Image, ImageColorspace};
use craft_pdf::write::backend::mask::{Mask as KMask, MaskType, TransferFunction};
use craft_pdf::write::backend::num::NormalizedF32;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::backend::paint::{Fill, FillRule as KFillRule, LinearGradient, Paint as KPaint, RadialGradient, SpreadMethod, Stop};
use craft_pdf::write::backend::surface::Surface;
use craft_pdf::write::backend::tagging::{ContentTag, SpanTag};
use craft_pdf::write::backend::text::{Font, GlyphId, KrillaGlyph};
use craft_pdf::write::backend::{Data, SerializeSettings};
use craft_pdf::write::{self as pdf_write, Metadata, Writer};
use kurbo::{Affine, BezPath, PathEl};

use super::{
    BlendMode, ComplexRegion, DisplayList, FillItem, FillRule, Flattened, FontBytes, GradientGeometry, GroupItem, ImageItem, ImageSpace, Item, Mask,
    MaskKind, Paint, TextFont, TextItem,
};
use crate::ink::{Ink, InkColor, InkId, InkSet, Overprint, Process, to_pdf_color};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WriteError {
    #[error(transparent)]
    Pdf(#[from] pdf_write::WriteError),
    /// An ink isn't in the set or a spot's Lab alternate can't be converted.
    #[error("color {0:?} can't be written")]
    Color(InkColor),
    /// A DeviceN space would have more colorants than the output's PDF version takes
    /// ([`max_colorants`]).
    #[error("a DeviceN space of {colorants} colorants; this output takes at most {max}")]
    Colorants { colorants: usize, max: usize },
    /// A complex region paints more spot inks than [`plates`](crate::plates::plates) separates;
    /// `unseparated` are the ones past the limit.
    #[error("a complex region paints {inks} spot inks; at most {max} are separated, not {}", some_names(.unseparated))]
    SpotInks { inks: usize, max: usize, unseparated: Vec<String> },
    #[error("the page box is empty or not finite")]
    Page,
    #[error("image: {0}")]
    Image(&'static str),
    /// Display-list features the writer doesn't write yet.
    #[error("not written yet: {0}")]
    Unsupported(&'static str),
    /// Raised by a [`write_flattened`] region callback.
    #[error("complex region: {0}")]
    Region(String),
}

/// The most colorants a DeviceN space takes at `version`: 8 up to PDF 1.4 (implementation
/// limits, PDF 1.4 reference Appendix C), 32 from PDF 1.5.
pub fn max_colorants(version: PdfVersion) -> usize {
    match version {
        PdfVersion::Pdf14 => 8,
        _ => DeviceNSpace::MAX_COLORANTS,
    }
}

fn some_names(names: &[String]) -> String {
    let mut listed = names.iter().take(3).map(String::as_str).collect::<Vec<_>>().join(", ");
    if names.len() > 3 {
        listed.push_str(", …");
    }
    listed
}

/// Draws `items` on `surface` in page space, with colors of `inks`, for an output of PDF
/// `version`. `cms` converts spot inks' Lab alternates into the printing condition. On error,
/// the items before the failing one are drawn and the surface is left balanced.
pub fn draw(surface: &mut Surface<'_>, inks: &InkSet, items: &[Item], cms: &dyn CmykConversion, version: PdfVersion) -> Result<(), WriteError> {
    let mut painter = Painter::new(inks, cms, version);
    items.iter().try_for_each(|item| painter.item(surface, item))
}

/// Draws `flat.items` in order and, before item `at` of each complex region, the image
/// `raster` returns for it, clipped to the region's path and without overprint. `raster`
/// returns an [`ImageItem`] whose space is [`ImageSpace::Inks`] of `list.inks` and which
/// covers the region, or `None` to leave the region empty. `version` is the output's PDF
/// version.
pub fn write_flattened(
    surface: &mut Surface<'_>,
    list: &DisplayList,
    flat: &Flattened,
    cms: &dyn CmykConversion,
    version: PdfVersion,
    raster: &mut dyn FnMut(&ComplexRegion) -> Result<Option<ImageItem>, WriteError>,
) -> Result<(), WriteError> {
    Painter::new(&list.inks, cms, version).flattened(surface, flat, raster)
}

/// [`write_flattened`] for one page of a document whose pages share `fonts`.
pub fn write_flattened_with_fonts(
    surface: &mut Surface<'_>,
    list: &DisplayList,
    flat: &Flattened,
    cms: &dyn CmykConversion,
    version: PdfVersion,
    fonts: &mut FontMap,
    raster: &mut dyn FnMut(&ComplexRegion) -> Result<Option<ImageItem>, WriteError>,
) -> Result<(), WriteError> {
    let mut painter = Painter::new(&list.inks, cms, version);
    painter.shared = Some(fonts);
    painter.flattened(surface, flat, raster)
}

/// krilla fonts shared by the pages of one document, so that a font used on several pages is
/// embedded once. Runs share a font only when their [`TextFont`] bytes are equal: equal keys on
/// two pages can name different bytes (per-page widths of a wrapped bare CFF).
#[derive(Default)]
pub struct FontMap {
    /// By [`TextFont::key`] and face index: each font program seen, `None` where krilla can't
    /// read it.
    fonts: HashMap<(u128, u32), Vec<Shared>>,
}

type Shared = (FontBytes, Option<Font>);

impl FontMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Distinct font programs seen, including those krilla can't read.
    pub fn len(&self) -> usize {
        self.fonts.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    fn get(&mut self, tf: &TextFont) -> Option<Font> {
        let same = |d: &FontBytes| std::ptr::addr_eq(Arc::as_ptr(d), Arc::as_ptr(&tf.data)) || (**d).as_ref() == (*tf.data).as_ref();
        let entries = self.fonts.entry((tf.key, tf.index)).or_default();
        if let Some((_, font)) = entries.iter().find(|(d, _)| same(d)) {
            return font.clone();
        }
        let font = Font::new(Data::from(tf.data.clone()), tf.index);
        entries.push((tf.data.clone(), font.clone()));
        font
    }
}

impl Painter<'_, '_> {
    fn flattened(
        &mut self,
        surface: &mut Surface<'_>,
        flat: &Flattened,
        raster: &mut dyn FnMut(&ComplexRegion) -> Result<Option<ImageItem>, WriteError>,
    ) -> Result<(), WriteError> {
        let mut regions: Vec<&ComplexRegion> = flat.complex.iter().collect();
        regions.sort_by_key(|r| r.at);
        let mut regions = regions.into_iter().peekable();
        for at in 0..=flat.items.len() {
            while let Some(region) = regions.next_if(|r| r.at <= at || at == flat.items.len()) {
                let Some(image) = raster(region)? else { continue };
                let Some(clip) = to_path(&region.path) else { continue };
                surface.push_clip_path(&clip, &KFillRule::NonZero);
                let drawn = self.image(surface, &ImageItem { overprint: Overprint::default(), ..image });
                surface.pop();
                drawn?;
            }
            if let Some(item) = flat.items.get(at) {
                self.item(surface, item)?;
            }
        }
        Ok(())
    }
}

/// `list` as a one-page PDF with `settings`.
pub fn page_pdf(list: &DisplayList, settings: SerializeSettings, cms: &dyn CmykConversion) -> Result<Vec<u8>, WriteError> {
    let page = list.page;
    let size = PageSettings::from_wh(page.width() as f32, page.height() as f32).filter(|_| page.is_finite()).ok_or(WriteError::Page)?;
    let version = settings.configuration.version();
    pdf_write::guard(|| {
        let mut writer = Writer::new(settings, Metadata::default());
        let mut page_builder = writer.document_mut().start_page_with(size);
        let mut surface = page_builder.surface();
        let shifted = page.x0 != 0.0 || page.y0 != 0.0;
        if shifted {
            surface.push_transform(&Transform::from_translate(-page.x0 as f32, -page.y0 as f32));
        }
        let drawn = draw(&mut surface, &list.inks, &list.items, cms, version);
        if shifted {
            surface.pop();
        }
        surface.finish();
        page_builder.finish();
        drawn?;
        Ok(writer.finish()?)
    })
}

struct Painter<'a, 'f> {
    inks: &'a InkSet,
    cms: &'a dyn CmykConversion,
    /// [`max_colorants`] of the output.
    max_colorants: usize,
    /// The document's fonts, consulted on a miss in `fonts`.
    shared: Option<&'f mut FontMap>,
    /// `None` for fonts krilla can't read; their runs are written as outlines. Keyed by
    /// [`TextFont::key`] for one page only: a font built for another page may have the same
    /// key and other bytes (per-page widths of a wrapped bare CFF).
    fonts: HashMap<(u128, u32), Option<Font>>,
    /// Inside an `/ActualText` span, which can't nest.
    tagged: bool,
}

impl<'a> Painter<'a, '_> {
    fn new(inks: &'a InkSet, cms: &'a dyn CmykConversion, version: PdfVersion) -> Self {
        Self { inks, cms, max_colorants: max_colorants(version), shared: None, fonts: HashMap::new(), tagged: false }
    }

    /// An error when a DeviceN space of `colorants` is more than the output takes.
    fn fits(&self, colorants: usize) -> Result<(), WriteError> {
        if colorants > self.max_colorants {
            return Err(WriteError::Colorants { colorants, max: self.max_colorants });
        }
        Ok(())
    }

    fn item(&mut self, s: &mut Surface<'_>, item: &Item) -> Result<(), WriteError> {
        self.item_with_opacity(s, item, 1.0)
    }

    /// `opacity` is a group's, applied to a fill or text drawn alone in it.
    fn item_with_opacity(&mut self, s: &mut Surface<'_>, item: &Item, opacity: f32) -> Result<(), WriteError> {
        match item {
            Item::Fill(f) => self.fill(s, f, opacity),
            Item::Text(t) => self.text(s, t, opacity),
            Item::Image(im) => self.image(s, im),
            Item::Group(g) => self.group(s, g),
        }
    }

    fn fill(&mut self, s: &mut Surface<'_>, f: &FillItem, opacity: f32) -> Result<(), WriteError> {
        let Some(path) = to_path(&f.path) else { return Ok(()) };
        let rule = match f.rule {
            FillRule::NonZero => KFillRule::NonZero,
            FillRule::EvenOdd => KFillRule::EvenOdd,
        };
        let (paint, overprint) = self.paint(&f.paint, f.overprint, Affine::IDENTITY)?;
        with_overprint(s, overprint, |s| {
            s.set_fill(Some(Fill { paint, opacity: normalized(opacity), rule }));
            s.draw_path(&path);
        });
        Ok(())
    }

    fn text(&mut self, s: &mut Surface<'_>, t: &TextItem, opacity: f32) -> Result<(), WriteError> {
        let text = t.text.as_deref().unwrap_or("");
        let font = t.font.as_deref().and_then(|tf| self.font(tf));
        let (Some(font), Some(first)) = (font, t.glyphs.first()) else {
            let Some(path) = to_path(&t.outlines) else { return Ok(()) };
            let (paint, overprint) = self.paint(&t.paint, t.overprint, Affine::IDENTITY)?;
            let tag = !self.tagged && t.text.is_some();
            with_overprint(s, overprint, |s| {
                if tag {
                    s.start_tagged(ContentTag::Span(SpanTag { actual_text: Some(text), ..SpanTag::empty() }));
                }
                s.set_fill(Some(Fill { paint, opacity: normalized(opacity), rule: KFillRule::NonZero }));
                s.draw_path(&path);
                if tag {
                    s.end_tagged();
                }
            });
            return Ok(());
        };
        let size = f64::from(t.size);
        if !(size.is_finite() && size > 0.0) {
            return Ok(());
        }
        let glyphs: Vec<KrillaGlyph> = t
            .glyphs
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let next_x = t.glyphs.get(i + 1).map_or(g.origin.x, |n| n.origin.x);
                let range = if text.get(g.text.clone()).is_some() { g.text.clone() } else { 0..0 };
                KrillaGlyph::new(GlyphId::new(g.id), ((next_x - g.origin.x) / size) as f32, 0.0, (g.origin.y / size) as f32, 0.0, range, None)
            })
            .collect();
        // Text space is y up; krilla places glyphs y down.
        let ctm = t.transform * Affine::scale_non_uniform(1.0, -1.0);
        let (paint, overprint) = self.paint(&t.paint, t.overprint, ctm)?;
        with_overprint(s, overprint, |s| {
            s.set_fill(Some(Fill { paint, opacity: normalized(opacity), rule: KFillRule::NonZero }));
            s.push_transform(&to_transform(ctm));
            s.draw_glyphs(KPoint::from_xy(first.origin.x as f32, 0.0), &glyphs, font, text, t.size, false);
            s.pop();
        });
        Ok(())
    }

    fn font(&mut self, tf: &TextFont) -> Option<Font> {
        if let Some(font) = self.fonts.get(&(tf.key, tf.index)) {
            return font.clone();
        }
        let font = match self.shared.as_deref_mut() {
            Some(shared) => shared.get(tf),
            None => Font::new(Data::from(tf.data.clone()), tf.index),
        };
        self.fonts.insert((tf.key, tf.index), font.clone());
        font
    }

    fn image(&mut self, s: &mut Surface<'_>, im: &ImageItem) -> Result<(), WriteError> {
        let (w, h) = im.size;
        let pixels = usize::try_from(u64::from(w) * u64::from(h)).map_err(|_| WriteError::Image("too large"))?;
        if pixels == 0 {
            return Ok(());
        }
        let draw = |s: &mut Surface<'_>, image: Image| {
            let Some(unit) = Size::from_wh(1.0, 1.0) else { return };
            with_overprint(s, im.overprint, |s| {
                s.push_transform(&to_transform(im.transform));
                s.draw_image(image, unit);
                s.pop();
            });
        };
        if let Some(image) = passthrough(im) {
            draw(s, image);
            return Ok(());
        }
        let (space, components, device_n, samples) = match &im.space {
            ImageSpace::Gray => (ImageColorspace::Luma, 1, None, im.samples.clone()),
            ImageSpace::Rgb => (ImageColorspace::Rgb, 3, None, im.samples.clone()),
            ImageSpace::Cmyk => (ImageColorspace::Cmyk, 4, None, im.samples.clone()),
            ImageSpace::Inks(ids) => {
                if im.samples.len() != pixels.saturating_mul(ids.len()) {
                    return Err(WriteError::Image("sample count doesn't match the size and inks"));
                }
                let (space, samples) = self.ink_samples(ids, &im.samples)?;
                (ImageColorspace::DeviceN, space.colorants().len(), Some(space), samples)
            }
        };
        if samples.len() != pixels.saturating_mul(components) {
            return Err(WriteError::Image("sample count doesn't match the size and space"));
        }
        if im.smask.as_ref().is_some_and(|a| a.len() != pixels) {
            return Err(WriteError::Image("smask size doesn't match the image"));
        }
        let image = Image::from_custom(Samples { samples, alpha: im.smask.clone(), size: im.size, space, device_n }, false)
            .map_err(|_| WriteError::Image("krilla rejected it"))?;
        draw(s, image);
        Ok(())
    }

    /// Samples of `ids` as samples of the inks they print on: each ink's tint moves to the
    /// inks [`InkSet::resolve`] gives a full tint of it, and tints on one ink combine as
    /// overlapping halftones, as `resolve` combines them.
    fn ink_samples(&self, ids: &[InkId], samples: &Arc<[u8]>) -> Result<(DeviceNSpace, Arc<[u8]>), WriteError> {
        let mut weights: Vec<Vec<(InkId, f32)>> = Vec::with_capacity(ids.len());
        for &id in ids {
            let full = InkColor::Tints(vec![(id, 1.0)]);
            match self.inks.resolve(&full, self.cms) {
                InkColor::Tints(t) => weights.push(t),
                _ => return Err(WriteError::Color(full)),
            }
        }
        let out: Vec<InkId> = weights.iter().flatten().map(|(id, _)| *id).collect::<BTreeSet<_>>().into_iter().collect();
        self.fits(out.len())?;
        let colorants = out
            .iter()
            .map(|&id| self.colorant(id).ok_or_else(|| WriteError::Color(InkColor::Tints(vec![(id, 1.0)]))))
            .collect::<Result<Vec<_>, _>>()?;
        let space = DeviceNSpace::new(colorants).ok_or(WriteError::Image("krilla rejected its DeviceN space"))?;
        let renames = weights.iter().all(|w| matches!(w[..], [(_, t)] if t == 1.0)) && out.len() == ids.len();
        if renames && weights.iter().zip(&out).all(|(w, o)| w.first().is_some_and(|(id, _)| id == o)) {
            return Ok((space, samples.clone()));
        }
        // Column of each ink's weights in `out`.
        let routes: Vec<Vec<(usize, f32)>> =
            weights.iter().map(|w| w.iter().filter_map(|(id, t)| out.iter().position(|o| o == id).map(|i| (i, *t))).collect()).collect();
        let n = ids.len();
        let mut result = Vec::with_capacity(samples.len() / n.max(1) * out.len());
        let mut keep = vec![1.0_f32; out.len()];
        for px in samples.chunks_exact(n.max(1)) {
            keep.fill(1.0);
            for (v, route) in px.iter().zip(&routes) {
                let v = f32::from(*v) / 255.0;
                for &(i, t) in route {
                    if let Some(k) = keep.get_mut(i) {
                        *k *= 1.0 - v * t;
                    }
                }
            }
            result.extend(keep.iter().map(|k| byte(1.0 - k)));
        }
        Ok((space, result.into()))
    }

    fn group(&mut self, s: &mut Surface<'_>, g: &GroupItem) -> Result<(), WriteError> {
        if g.children.is_empty() {
            return Ok(());
        }
        let mut pushed = 0;
        let clip = match &g.clip {
            Some(clip) => match to_path(clip) {
                Some(path) => Some(path),
                // An empty clip shows nothing.
                None => return Ok(()),
            },
            None => None,
        };
        if let Some(clip) = clip {
            s.push_clip_path(&clip, &KFillRule::NonZero);
            pushed += 1;
        }
        let tag = match &g.actual_text {
            Some(text) if !self.tagged => {
                s.start_tagged(ContentTag::Span(SpanTag { actual_text: Some(text), ..SpanTag::empty() }));
                self.tagged = true;
                true
            }
            _ => false,
        };
        let drawn = self.group_content(s, g, &mut pushed);
        for _ in 0..pushed {
            s.pop();
        }
        if tag {
            s.end_tagged();
            self.tagged = false;
        }
        drawn
    }

    /// Pushes the group's blend mode, mask, opacity and attributes, counted in `pushed`, and
    /// draws the children.
    fn group_content(&mut self, s: &mut Surface<'_>, g: &GroupItem, pushed: &mut usize) -> Result<(), WriteError> {
        let opacity = unit(g.opacity);
        let blended = g.blend != BlendMode::Normal;
        if blended {
            s.push_blend_mode(blend_mode(g.blend));
            *pushed += 1;
        }
        // One fill or text run alone in a plain group: opacity and blend mode on the paint
        // itself composite the same.
        if let ([child @ (Item::Fill(_) | Item::Text(_))], false, false, None) = (&g.children[..], g.isolated, g.knockout, &g.mask) {
            return self.item_with_opacity(s, child, opacity);
        }
        let mut isolated = false;
        if let Some(mask) = &g.mask {
            let mask = self.mask(s, mask, opacity)?;
            s.push_mask(mask);
            *pushed += 1;
        } else if opacity < 1.0 && g.isolated {
            s.push_opacity(normalized(opacity));
            *pushed += 1;
            isolated = true;
        } else if opacity < 1.0 || (blended && !g.isolated && !g.knockout) {
            // krilla writes opacity as an isolated group and a group without attributes as a
            // plain form; a constant soft mask makes a non-isolated transparency group.
            let mask = constant_mask(s, opacity);
            s.push_mask(mask);
            *pushed += 1;
        }
        if g.knockout || (g.isolated && !isolated) {
            s.push_group(GroupOptions { isolated: g.isolated, knockout: g.knockout });
            *pushed += 1;
        }
        g.children.iter().try_for_each(|child| self.item(s, child))
    }

    /// `m` as a krilla mask, with `opacity` folded into its transfer function.
    fn mask(&mut self, s: &mut Surface<'_>, m: &Mask, opacity: f32) -> Result<KMask, WriteError> {
        let mut builder = s.stream_builder();
        let mut sub = builder.surface();
        let tagged = std::mem::replace(&mut self.tagged, true);
        let drawn = self.group(&mut sub, &m.group);
        self.tagged = tagged;
        sub.finish();
        let stream = builder.finish();
        drawn?;
        let kind = match m.kind {
            MaskKind::Alpha => MaskType::Alpha,
            MaskKind::Luminosity => MaskType::Luminosity,
        };
        let mut mask = KMask::new(stream, kind);
        if let (MaskKind::Luminosity, Some(backdrop)) = (m.kind, &m.backdrop) {
            // The backdrop is a color of the CMYK blending space, so spots count as process.
            let mut process = self.inks.clone();
            process.set_all_to_process(true);
            let cmyk = process.resolve(backdrop, self.cms).as_cmyk().ok_or_else(|| WriteError::Color(backdrop.clone()))?;
            let [c, m, y, k] = cmyk.map(byte);
            mask = mask.with_backdrop(cmyk::Color::new(c, m, y, k));
        }
        let transfer = match &m.transfer {
            Some(samples) => Some(samples.iter().map(|v| f32::from(*v) / 255.0 * opacity).collect()),
            None if opacity < 1.0 => Some(vec![0.0, opacity]),
            None => None,
        };
        if let Some(transfer) = transfer {
            mask = mask.with_transfer(TransferFunction::Sampled(transfer.into_iter().map(normalized).collect()));
        }
        Ok(mask)
    }

    /// `paint` as krilla draws it under `ctm` (page space from the drawing space), and the
    /// overprint to draw it with.
    fn paint(&self, paint: &Paint, overprint: Overprint, ctm: Affine) -> Result<(KPaint, Overprint), WriteError> {
        match paint {
            Paint::Solid(color) => {
                let (color, overprint) = self.solid(color, overprint)?;
                Ok((color.into(), overprint))
            }
            Paint::Gradient { stops, geometry } => Ok((self.gradient(stops, geometry, overprint, ctm)?, overprint)),
        }
    }

    fn solid(&self, color: &InkColor, overprint: Overprint) -> Result<(PdfColor, Overprint), WriteError> {
        let resolved = self.inks.resolve(color, self.cms);
        if let InkColor::Tints(tints) = &resolved
            && overprint.fill
            && let Some(cmyk) = resolved.as_cmyk()
        {
            let named: Vec<usize> =
                (0..4).filter(|&i| tints.iter().any(|(id, _)| id.index() == i) && (!overprint.nonzero || cmyk[i] > 0.0)).collect();
            if named.len() < 4 {
                if named.is_empty() {
                    // Zero DeviceCMYK under OPM 1 leaves every plate alone.
                    return Ok((RegularColor::Cmyk(cmyk::Color::new(0, 0, 0, 0)).into(), Overprint { nonzero: true, ..overprint }));
                }
                return Ok((self.process_device_n(&named, cmyk).ok_or_else(|| WriteError::Color(color.clone()))?, overprint));
            }
        }
        if let InkColor::Tints(tints) = &resolved {
            // More inks than the output's limit include a spot, so `to_pdf_color` makes a
            // DeviceN space of them.
            self.fits(tints.iter().map(|(id, _)| *id).collect::<BTreeSet<_>>().len())?;
        }
        let pdf = to_pdf_color(&resolved, self.inks, self.cms).ok_or_else(|| WriteError::Color(color.clone()))?;
        Ok((pdf, overprint))
    }

    /// A DeviceN color of process inks `named` (indices in plate order) at their `cmyk` tints.
    fn process_device_n(&self, named: &[usize], cmyk: [f32; 4]) -> Option<PdfColor> {
        let colorants = named.iter().map(|&i| Process::ALL.get(i).map(|p| process_colorant(*p))).collect::<Option<Vec<_>>>()?;
        let tints = named.iter().map(|&i| cmyk.get(i).copied().map(byte)).collect::<Option<Vec<_>>>()?;
        Some(device_n::Color::new(tints, DeviceNSpace::new(colorants)?)?.into())
    }

    /// Stops in one color space: each lists every ink some stop lists, so krilla writes them
    /// all in the space a color of those inks gets.
    fn gradient(&self, stops: &[(f32, InkColor)], geometry: &GradientGeometry, overprint: Overprint, ctm: Affine) -> Result<KPaint, WriteError> {
        let mut resolved: Vec<(f32, BTreeMap<InkId, f32>)> = Vec::with_capacity(stops.len());
        for (offset, color) in stops {
            let InkColor::Tints(tints) = self.inks.resolve(color, self.cms) else {
                return Err(WriteError::Unsupported("a gradient stop of the All or None colorant"));
            };
            let mut merged = BTreeMap::new();
            for (id, t) in tints {
                let v = merged.entry(id).or_insert(0.0);
                *v = 1.0 - (1.0 - *v) * (1.0 - unit(t));
            }
            resolved.push((*offset, merged));
        }
        let all: BTreeSet<InkId> = resolved.iter().flat_map(|(_, m)| m.keys().copied()).collect();
        self.fits(all.len())?;
        let partial_process = overprint.fill && all.len() < 4 && all.iter().all(|id| id.index() < 4);
        let mut krilla_stops = Vec::with_capacity(resolved.len());
        for ((offset, merged), (_, original)) in resolved.iter().zip(stops) {
            let tint = |id: &InkId| merged.get(id).copied().unwrap_or(0.0);
            let color = if partial_process {
                let named: Vec<usize> = all.iter().map(|id| id.index()).collect();
                let mut cmyk = [0.0; 4];
                for id in &all {
                    if let Some(c) = cmyk.get_mut(id.index()) {
                        *c = tint(id);
                    }
                }
                self.process_device_n(&named, cmyk)
            } else {
                to_pdf_color(&InkColor::Tints(all.iter().map(|id| (*id, tint(id))).collect()), self.inks, self.cms)
            };
            let color = color.ok_or_else(|| WriteError::Color(original.clone()))?;
            krilla_stops.push(Stop { offset: normalized(*offset), color, opacity: NormalizedF32::ONE });
        }
        let to_drawing = |t: &Affine| to_transform(ctm.inverse() * *t);
        Ok(match geometry {
            GradientGeometry::Axial { start, end, extend: [true, true], transform } => LinearGradient {
                x1: start.x as f32,
                y1: start.y as f32,
                x2: end.x as f32,
                y2: end.y as f32,
                transform: to_drawing(transform),
                spread_method: SpreadMethod::Pad,
                stops: krilla_stops,
                anti_alias: false,
            }
            .into(),
            GradientGeometry::Radial { start, start_radius, end, end_radius, extend: [true, true], transform } => RadialGradient {
                fx: start.x as f32,
                fy: start.y as f32,
                fr: *start_radius as f32,
                cx: end.x as f32,
                cy: end.y as f32,
                cr: *end_radius as f32,
                transform: to_drawing(transform),
                spread_method: SpreadMethod::Pad,
                stops: krilla_stops,
                anti_alias: false,
            }
            .into(),
            _ => return Err(WriteError::Unsupported("a gradient that doesn't extend at both ends")),
        })
    }

    /// Ink `id` as a DeviceN colorant; a spot carries its CMYK alternate.
    fn colorant(&self, id: InkId) -> Option<Colorant> {
        match self.inks.get(id)? {
            Ink::Process(p) => Some(process_colorant(*p)),
            Ink::Spot(spot) => {
                let [c, m, y, k] = self.inks.spot_cmyk(id, self.cms)?.map(byte);
                (!spot.name.is_empty()).then(|| Colorant::Spot { name: spot.name.clone(), alternate: cmyk::Color::new(c, m, y, k) })
            }
        }
    }
}

/// A soft mask of `opacity` everywhere: an empty luminosity group whose transfer function
/// maps every value to `opacity`.
fn constant_mask(s: &mut Surface<'_>, opacity: f32) -> KMask {
    let builder = s.stream_builder();
    let stream = builder.finish();
    let o = normalized(opacity);
    KMask::new(stream, MaskType::Luminosity).with_transfer(TransferFunction::Sampled(vec![o, o]))
}

fn with_overprint(s: &mut Surface<'_>, overprint: Overprint, draw: impl FnOnce(&mut Surface<'_>)) {
    if overprint == Overprint::default() {
        draw(s);
        return;
    }
    s.push_overprint(kblend::Overprint { fill: overprint.fill, stroke: overprint.stroke, nonzero: overprint.nonzero });
    draw(s);
    s.pop();
}

fn process_colorant(p: Process) -> Colorant {
    Colorant::Process(match p {
        Process::Cyan => device_n::Process::Cyan,
        Process::Magenta => device_n::Process::Magenta,
        Process::Yellow => device_n::Process::Yellow,
        Process::Black => device_n::Process::Black,
    })
}

fn blend_mode(b: BlendMode) -> kblend::BlendMode {
    use kblend::BlendMode as K;
    match b {
        BlendMode::Normal => K::Normal,
        BlendMode::Multiply => K::Multiply,
        BlendMode::Screen => K::Screen,
        BlendMode::Overlay => K::Overlay,
        BlendMode::Darken => K::Darken,
        BlendMode::Lighten => K::Lighten,
        BlendMode::ColorDodge => K::ColorDodge,
        BlendMode::ColorBurn => K::ColorBurn,
        BlendMode::HardLight => K::HardLight,
        BlendMode::SoftLight => K::SoftLight,
        BlendMode::Difference => K::Difference,
        BlendMode::Exclusion => K::Exclusion,
        BlendMode::Hue => K::Hue,
        BlendMode::Saturation => K::Saturation,
        BlendMode::Color => K::Color,
        BlendMode::Luminosity => K::Luminosity,
    }
}

fn to_transform(a: Affine) -> Transform {
    let [a, b, c, d, e, f] = a.as_coeffs().map(|v| v as f32);
    Transform::from_row(a, b, c, d, e, f)
}

/// `None` for an empty path, which paints nothing.
fn to_path(path: &BezPath) -> Option<Path> {
    let mut b = PathBuilder::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => b.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => b.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(p1, p) => b.quad_to(p1.x as f32, p1.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(p1, p2, p) => b.cubic_to(p1.x as f32, p1.y as f32, p2.x as f32, p2.y as f32, p.x as f32, p.y as f32),
            PathEl::ClosePath => b.close(),
        }
    }
    b.finish()
}

fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

fn normalized(v: f32) -> NormalizedF32 {
    NormalizedF32::new(unit(v)).unwrap_or(NormalizedF32::ONE)
}

fn byte(v: f32) -> u8 {
    (unit(v) * 255.0).round() as u8
}

/// Image samples for krilla.
#[derive(Clone)]
struct Samples {
    samples: Arc<[u8]>,
    alpha: Option<Arc<[u8]>>,
    size: (u32, u32),
    space: ImageColorspace,
    device_n: Option<DeviceNSpace>,
}

impl Hash for Samples {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.samples.hash(state);
        self.alpha.hash(state);
        self.size.hash(state);
        self.space.hash(state);
        self.device_n.hash(state);
    }
}

impl CustomImage for Samples {
    fn color_channel(&self) -> &[u8] {
        &self.samples
    }
    fn alpha_channel(&self) -> Option<&[u8]> {
        self.alpha.as_deref()
    }
    fn bits_per_component(&self) -> BitsPerComponent {
        BitsPerComponent::Eight
    }
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn icc_profile(&self) -> Option<&[u8]> {
        None
    }
    fn color_space(&self) -> ImageColorspace {
        self.space
    }
    fn device_n_space(&self) -> Option<DeviceNSpace> {
        self.device_n.clone()
    }
}

/// `im`'s JPEG data as a krilla image, written as it is (DCTDecode), when it has one that
/// decodes to its size. The embedded ICC profile is left out, as the samples path leaves it.
fn passthrough(im: &ImageItem) -> Option<Image> {
    if im.smask.is_some() || !matches!(im.space, ImageSpace::Gray | ImageSpace::Cmyk) {
        return None;
    }
    let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(im.jpeg.clone()?);
    let image = Image::from_jpeg_with_icc(Data::from(data), None, false).ok()?;
    (image.size() == im.size).then_some(image)
}
