//! A flattened page as a PostScript page program, composite or for one plate.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use craft_pdf::interpret::color::CmykConversion;
use craft_ps::write::{self as psw, PaintOps, PathOps, ShadingKind};
use kurbo::{Affine, BezPath, Point, Rect, Shape};

use super::data::{self, Payload};
use super::font::FontSet;
use super::{Prepared, PsDataFormat, PsError, PsLevel, PsOptions, PsReport};
use crate::flatten::{BlendMode, FillRule, GradientGeometry, GroupItem, ImageItem, ImageSpace, Item, Paint, TextItem};
use crate::ink::{Ink, InkColor, InkId, InkSet, Overprint, Process};

/// Pixels a gradient rasterized at LanguageLevel 2 may hold; a larger one is sampled coarser.
pub(super) const MAX_GRADIENT_PIXELS: u64 = 16 << 20;

/// What a page program paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Composite,
    /// The plate of this ink of the page's [`InkSet`], in DeviceGray; `None` when the page
    /// prints nothing of the plate's ink, so only registration color reaches it.
    Plate(Option<InkId>),
}

/// [`PsReport`]'s counts, gathered while writing.
#[derive(Debug, Default)]
pub(super) struct Counts {
    pub fonts: usize,
    pub outlined_runs: usize,
    pub rasterized_gradients: usize,
    pub converted_to_process: usize,
    pub approximated_overprints: usize,
    /// Spot inks a composite paints, in the order first painted.
    pub spots: Vec<String>,
}

impl Counts {
    pub fn add_to(&self, report: &mut PsReport) {
        report.fonts += self.fonts;
        report.outlined_runs += self.outlined_runs;
        report.rasterized_gradients += self.rasterized_gradients;
        report.converted_to_process += self.converted_to_process;
        report.approximated_overprints += self.approximated_overprints;
    }
}

/// `page` as the program between `%%EndPageSetup` and `showpage`: in its own `save` and with the
/// prolog's names, in page space (y down).
pub(super) fn page(page: &Prepared, opts: &PsOptions, target: Target, counts: &mut Counts) -> Result<Vec<u8>, PsError> {
    let mut p = Painter {
        inks: &page.list.inks,
        cms: &page.color.cms,
        level: opts.level,
        binary: opts.data_format == PsDataFormat::Binary,
        target,
        fonts: &page.fonts,
        counts,
        page: page.index,
        dpi: opts.flatten.raster_dpi,
        area: page.list.page,
        out: Vec::new(),
        s: String::new(),
    };
    let g = page.geometry.page;
    p.s.push_str("save CraftPS begin\n");
    let _ = writeln!(p.s, "[1 0 0 -1 {} {}] cm", num(-g.x0), num(g.y1));
    p.s.push_str(&page.fonts.program);
    p.counts.fonts += page.fonts.count;
    for item in &page.items {
        p.item(item)?;
    }
    p.s.push_str("end restore\n");
    p.flush();
    Ok(p.out)
}

/// A color to paint with: the operators that set it, and whether it overprints.
struct SetColor {
    set: String,
    overprint: bool,
}

struct Painter<'a> {
    inks: &'a InkSet,
    cms: &'a dyn CmykConversion,
    level: PsLevel,
    binary: bool,
    target: Target,
    fonts: &'a FontSet,
    counts: &'a mut Counts,
    page: usize,
    dpi: f32,
    /// The page's visible area: what a rasterized gradient needs to cover at most.
    area: Rect,
    out: Vec<u8>,
    /// Text not yet moved to `out`.
    s: String,
}

impl Painter<'_> {
    fn flush(&mut self) {
        self.out.extend_from_slice(self.s.as_bytes());
        self.s.clear();
    }

    fn unsupported(&self, what: &'static str) -> PsError {
        PsError::Unsupported { page: self.page, what }
    }

    fn item(&mut self, item: &Item) -> Result<(), PsError> {
        match item {
            Item::Fill(f) => self.fill(&f.path, f.rule, &f.paint, f.overprint),
            Item::Text(t) => self.text(t),
            Item::Image(im) => self.image(im),
            Item::Group(g) => self.group(g),
        }
    }

    fn group(&mut self, g: &GroupItem) -> Result<(), PsError> {
        if g.opacity < 1.0 || g.blend != BlendMode::Normal || g.mask.is_some() {
            return Err(self.unsupported("transparency left after flattening"));
        }
        let Some(clip) = &g.clip else {
            return g.children.iter().try_for_each(|c| self.item(c));
        };
        // An empty clip shows nothing.
        if g.children.is_empty() || clip.elements().is_empty() {
            return Ok(());
        }
        self.s.push_str("q\n");
        self.clip(clip, g.clip_rule);
        g.children.iter().try_for_each(|c| self.item(c))?;
        self.s.push_str("Q\n");
        Ok(())
    }

    fn clip(&mut self, path: &BezPath, rule: FillRule) {
        psw::push_path(&mut self.s, path, PathOps::SHORT);
        self.s.push_str(psw::clip_op(ps_rule(rule), PaintOps::SHORT));
    }

    /// Starts a paint: `q true setoverprint` when it overprints.
    fn open(&mut self, overprint: bool) {
        if overprint {
            self.s.push_str("q true setoverprint\n");
        }
    }

    fn close(&mut self, overprint: bool) {
        if overprint {
            self.s.push_str("Q\n");
        }
    }

    fn fill(&mut self, path: &BezPath, rule: FillRule, paint: &Paint, overprint: Overprint) -> Result<(), PsError> {
        if path.elements().is_empty() {
            return Ok(());
        }
        match paint {
            Paint::Solid(color) => {
                let Some(color) = self.color(color, overprint)? else { return Ok(()) };
                self.open(color.overprint);
                self.s.push_str(&color.set);
                psw::push_path(&mut self.s, path, PathOps::SHORT);
                self.s.push_str(psw::fill_op(ps_rule(rule), PaintOps::SHORT));
                self.close(color.overprint);
                Ok(())
            }
            Paint::Gradient { stops, geometry } => self.gradient(path, rule, stops, geometry, overprint),
        }
    }

    fn text(&mut self, t: &TextItem) -> Result<(), PsError> {
        let solid = match &t.paint {
            Paint::Solid(color) => Some(color),
            Paint::Gradient { .. } => None,
        };
        let font = solid.and(t.font.as_deref()).and_then(|tf| self.fonts.get(tf));
        let (Some(color), Some(font)) = (solid, font) else {
            if !t.outlines.elements().is_empty() {
                self.counts.outlined_runs += 1;
            }
            return self.fill(&t.outlines, FillRule::NonZero, &t.paint, t.overprint);
        };
        let size = f64::from(t.size);
        if !(size.is_finite() && size > 0.0) || t.glyphs.is_empty() {
            return Ok(());
        }
        let Some(color) = self.color(color, t.overprint)? else { return Ok(()) };
        self.open(color.overprint);
        self.s.push_str("q\n");
        self.s.push_str(&color.set);
        let _ = writeln!(self.s, "{} cm", psw::matrix(t.transform));
        let mut current = None;
        for g in &t.glyphs {
            let Some((index, code)) = font.glyph(g.id) else { continue };
            if current != Some(index) {
                let _ = writeln!(self.s, "/{} {} selectfont", font.name(index), num(size));
                current = Some(index);
            }
            let _ = write!(self.s, "{} {} m <", num(g.origin.x), num(g.origin.y));
            for b in code {
                let _ = write!(self.s, "{b:02x}");
            }
            self.s.push_str("> S\n");
        }
        self.s.push_str("Q\n");
        self.close(color.overprint);
        Ok(())
    }

    /// `color` as it prints on the target, `None` when it leaves the target as it is.
    fn color(&mut self, color: &InkColor, overprint: Overprint) -> Result<Option<SetColor>, PsError> {
        let resolved = self.inks.resolve(color, self.cms);
        match self.target {
            Target::Plate(plate) => {
                Ok(plate_tint(&resolved, overprint, plate).map(|t| SetColor { set: format!("{} g\n", num(1.0 - f64::from(t))), overprint: false }))
            }
            Target::Composite => self.composite_color(&resolved, overprint),
        }
    }

    fn composite_color(&mut self, resolved: &InkColor, overprint: Overprint) -> Result<Option<SetColor>, PsError> {
        let op = overprint.fill;
        let tints = match resolved {
            InkColor::None => return Ok(None),
            InkColor::All(t) => {
                let set = format!("[/Separation /All /DeviceCMYK {{dup dup dup}}] setcolorspace {} setcolor\n", num(f64::from(unit(*t))));
                return Ok(Some(SetColor { set, overprint: op }));
            }
            InkColor::Tints(tints) => tints,
        };
        let merged = merge(tints);
        if merged.is_empty() {
            // Names no ink: knocks out every plate, or leaves them all.
            return Ok((!op).then(|| SetColor { set: "0 0 0 0 k\n".into(), overprint: false }));
        }
        if merged.keys().all(|id| id.index() < 4) {
            let mut cmyk = [0.0_f32; 4];
            for (id, t) in &merged {
                if let Some(c) = cmyk.get_mut(id.index()) {
                    *c = *t;
                }
            }
            if op {
                let named: Vec<InkId> = merged.iter().filter(|(_, t)| !overprint.nonzero || **t > 0.0).map(|(id, _)| *id).collect();
                if named.is_empty() {
                    // Zero DeviceCMYK under OPM 1 leaves every plate alone.
                    return Ok(None);
                }
                if named.len() < 4 {
                    // A Separation or DeviceN color paints just its colorants when it overprints.
                    let values: Vec<f32> = named.iter().map(|id| merged.get(id).copied().unwrap_or(0.0)).collect();
                    let space = match named[..] {
                        [id] => Some(self.separation(id)?),
                        _ if self.level == PsLevel::Level3 => Some(self.device_n(&named)?),
                        _ => None,
                    };
                    if let Some(space) = space {
                        return Ok(Some(SetColor { set: format!("{space} setcolorspace {} setcolor\n", nums(&values)), overprint: true }));
                    }
                    self.counts.approximated_overprints += 1;
                }
            }
            return Ok(Some(SetColor { set: format!("{} k\n", nums(&cmyk)), overprint: op }));
        }
        let ids: Vec<InkId> = merged.keys().copied().collect();
        let values: Vec<f32> = merged.values().copied().collect();
        if let [id] = ids[..] {
            let set = format!("{} setcolorspace {} setcolor\n", self.separation(id)?, nums(&values));
            return Ok(Some(SetColor { set, overprint: op }));
        }
        if self.level == PsLevel::Level3 {
            let set = format!("{} setcolorspace {} setcolor\n", self.device_n(&ids)?, nums(&values));
            return Ok(Some(SetColor { set, overprint: op }));
        }
        self.counts.converted_to_process += 1;
        let cmyk = self.cmyk_of(&merged)?;
        Ok(Some(SetColor { set: format!("{} k\n", nums(&cmyk)), overprint: op }))
    }

    /// Ink `id` as a colorant: its name as a PostScript name and its CMYK at 100%.
    fn colorant(&mut self, id: InkId) -> Result<(String, [f32; 4]), PsError> {
        match self.inks.get(id) {
            Some(Ink::Process(p)) => {
                let mut cmyk = [0.0; 4];
                if let Some(c) = cmyk.get_mut(p.index()) {
                    *c = 1.0;
                }
                Ok((format!("/{}", p.name()), cmyk))
            }
            Some(Ink::Spot(spot)) => {
                let cmyk = self.inks.spot_cmyk(id, self.cms).ok_or_else(|| self.unsupported("a spot ink without a CMYK alternate"))?;
                if spot.name.is_empty() {
                    return Err(self.unsupported("a spot ink without a name"));
                }
                if self.target == Target::Composite && !self.counts.spots.contains(&spot.name) {
                    self.counts.spots.push(spot.name.clone());
                }
                Ok((ps_name(&spot.name), cmyk))
            }
            None => Err(self.unsupported("a color of an ink not in the page's inks")),
        }
    }

    /// `[/Separation name /DeviceCMYK {…}]` for ink `id`.
    fn separation(&mut self, id: InkId) -> Result<String, PsError> {
        let (name, [c, m, y, k]) = self.colorant(id)?;
        Ok(format!(
            "[/Separation {name} /DeviceCMYK {{dup {} mul exch dup {} mul exch dup {} mul exch {} mul}}]",
            num(f64::from(c)),
            num(f64::from(m)),
            num(f64::from(y)),
            num(f64::from(k))
        ))
    }

    /// `[/DeviceN [names] /DeviceCMYK {…}]` for `ids`, whose tint transform combines the inks'
    /// CMYK as overlapping halftones, `1 − Π(1 − tᵢ·aᵢ)` per component.
    fn device_n(&mut self, ids: &[InkId]) -> Result<String, PsError> {
        let mut names = Vec::with_capacity(ids.len());
        let mut alternates = Vec::with_capacity(ids.len());
        for &id in ids {
            let (name, cmyk) = self.colorant(id)?;
            names.push(name);
            alternates.push(cmyk);
        }
        let mut proc = format!("{} array astore", ids.len());
        for j in 0..4 {
            proc.push_str(" 1");
            for (i, a) in alternates.iter().enumerate() {
                let a = a.get(j).copied().unwrap_or(0.0);
                if a > 0.0 {
                    let _ = write!(proc, " 1 index {i} get {} mul 1 exch sub mul", num(f64::from(a)));
                }
            }
            proc.push_str(" 1 exch sub exch");
        }
        proc.push_str(" pop");
        Ok(format!("[/DeviceN [{}] /DeviceCMYK {{{proc}}}]", names.join(" ")))
    }

    /// Tints as their CMYK appearance.
    fn cmyk_of(&mut self, tints: &BTreeMap<InkId, f32>) -> Result<[f32; 4], PsError> {
        let mut keep = [1.0_f32; 4];
        for (&id, &t) in tints {
            let (_, a) = self.colorant(id)?;
            for (k, a) in keep.iter_mut().zip(a) {
                *k *= 1.0 - t * a;
            }
        }
        Ok(keep.map(|k| 1.0 - k))
    }

    fn gradient(
        &mut self,
        path: &BezPath,
        rule: FillRule,
        stops: &[(f32, InkColor)],
        geometry: &GradientGeometry,
        overprint: Overprint,
    ) -> Result<(), PsError> {
        let (kind, coords, transform, extend) = match geometry {
            GradientGeometry::Axial { start, end, extend, transform } => {
                (ShadingKind::Axial, vec![start.x, start.y, end.x, end.y], *transform, *extend)
            }
            GradientGeometry::Radial { start, start_radius, end, end_radius, extend, transform } => {
                (ShadingKind::Radial, vec![start.x, start.y, *start_radius, end.x, end.y, *end_radius], *transform, *extend)
            }
        };
        if extend != [true, true] {
            return Err(self.unsupported("a gradient that doesn't extend at both ends"));
        }
        if stops.is_empty() {
            return Ok(());
        }
        let mut resolved: Vec<(f32, BTreeMap<InkId, f32>)> = Vec::with_capacity(stops.len());
        for (offset, color) in stops {
            let InkColor::Tints(tints) = self.inks.resolve(color, self.cms) else {
                return Err(self.unsupported("a gradient stop of the All or None colorant"));
            };
            resolved.push((*offset, merge(&tints)));
        }
        if self.level == PsLevel::Level2 {
            return self.raster_gradient(path, rule, &resolved, geometry, overprint);
        }
        let all: BTreeSet<InkId> = resolved.iter().flat_map(|(_, m)| m.keys().copied()).collect();
        let tint = |m: &BTreeMap<InkId, f32>, id: &InkId| m.get(id).copied().unwrap_or(0.0);
        let (space, components, op): (String, Vec<(f64, Vec<f32>)>, bool) = match self.target {
            Target::Plate(plate) => {
                let named = plate.is_some_and(|p| all.contains(&p));
                if overprint.fill && !named {
                    return Ok(());
                }
                let comps = resolved.iter().map(|(o, m)| (f64::from(*o), vec![1.0 - plate.map_or(0.0, |p| tint(m, &p))])).collect();
                ("/DeviceGray".into(), comps, false)
            }
            Target::Composite => {
                let ids: Vec<InkId> = all.iter().copied().collect();
                let process_only = ids.iter().all(|id| id.index() < 4);
                let space = if process_only && !(overprint.fill && ids.len() < 4) {
                    let comps =
                        resolved.iter().map(|(o, m)| (f64::from(*o), Process::ALL.iter().map(|p| tint(m, &InkId::process(*p))).collect())).collect();
                    ("/DeviceCMYK".to_string(), comps)
                } else {
                    let space = match ids[..] {
                        [] => return Ok(()),
                        [id] => self.separation(id)?,
                        _ => self.device_n(&ids)?,
                    };
                    (space, resolved.iter().map(|(o, m)| (f64::from(*o), ids.iter().map(|id| tint(m, id)).collect())).collect())
                };
                (space.0, space.1, overprint.fill)
            }
        };
        let n = components.first().map_or(0, |(_, c)| c.len());
        let components = spanning(components);
        self.open(op);
        self.s.push_str("q\n");
        self.clip(path, rule);
        let _ = writeln!(self.s, "{} cm", psw::matrix(transform));
        psw::push_shading(&mut self.s, kind, &space, &coords, n, &components);
        self.s.push_str("Q\n");
        self.close(op);
        Ok(())
    }

    /// A gradient as an image of its inks, clipped to `path`, at [`Painter::dpi`].
    fn raster_gradient(
        &mut self,
        path: &BezPath,
        rule: FillRule,
        stops: &[(f32, BTreeMap<InkId, f32>)],
        geometry: &GradientGeometry,
        overprint: Overprint,
    ) -> Result<(), PsError> {
        let bounds = path.bounding_box().intersect(self.area);
        if !(bounds.is_finite() && bounds.width() > 0.0 && bounds.height() > 0.0) {
            return Ok(());
        }
        let transform = match geometry {
            GradientGeometry::Axial { transform, .. } | GradientGeometry::Radial { transform, .. } => *transform,
        };
        if !transform.determinant().is_finite() || transform.determinant() == 0.0 {
            return Ok(());
        }
        let inverse = transform.inverse();
        let ids: Vec<InkId> = stops.iter().flat_map(|(_, m)| m.keys().copied()).collect::<BTreeSet<_>>().into_iter().collect();
        if ids.is_empty() {
            return Ok(());
        }
        let mut scale = f64::from(if self.dpi.is_finite() && self.dpi > 0.0 { self.dpi } else { 300.0 }) / 72.0;
        let size = |scale: f64| ((bounds.width() * scale).ceil().max(1.0) as u64, (bounds.height() * scale).ceil().max(1.0) as u64);
        let (mut w, mut h) = size(scale);
        if w * h > MAX_GRADIENT_PIXELS {
            scale *= (MAX_GRADIENT_PIXELS as f64 / (w * h) as f64).sqrt();
            (w, h) = size(scale);
        }
        let (Ok(w32), Ok(h32)) = (u32::try_from(w), u32::try_from(h)) else { return Ok(()) };
        let mut samples = Vec::with_capacity((w * h) as usize * ids.len());
        let mut values = vec![0.0_f32; ids.len()];
        for row in 0..h {
            for col in 0..w {
                let p = Point::new(bounds.x0 + (col as f64 + 0.5) / scale, bounds.y0 + (row as f64 + 0.5) / scale);
                let t = parameter(geometry, inverse * p);
                at_offset(stops, t as f32, &ids, &mut values);
                samples.extend(values.iter().map(|v| byte(*v)));
            }
        }
        let image = ImageItem {
            space: ImageSpace::Inks(ids),
            size: (w32, h32),
            samples: samples.into(),
            transform: Affine::new([w as f64 / scale, 0.0, 0.0, h as f64 / scale, bounds.x0, bounds.y0]),
            smask: None,
            overprint,
            jpeg: None,
        };
        self.counts.rasterized_gradients += 1;
        self.s.push_str("q\n");
        self.clip(path, rule);
        self.image(&image)?;
        self.s.push_str("Q\n");
        Ok(())
    }

    fn image(&mut self, im: &ImageItem) -> Result<(), PsError> {
        if im.smask.is_some() {
            return Err(self.unsupported("an image with a soft mask left after flattening"));
        }
        let (w, h) = im.size;
        let pixels = usize::try_from(u64::from(w) * u64::from(h)).map_err(|_| self.unsupported("an image this large"))?;
        if pixels == 0 {
            return Ok(());
        }
        let components = match &im.space {
            ImageSpace::Gray => 1,
            ImageSpace::Rgb => 3,
            ImageSpace::Cmyk => 4,
            ImageSpace::Inks(ids) => ids.len(),
        };
        if components == 0 || im.samples.len() != pixels.saturating_mul(components) {
            return Err(self.unsupported("an image whose samples don't match its size"));
        }
        match self.target {
            Target::Plate(plate) => self.plate_image(im, plate, pixels),
            Target::Composite => self.composite_image(im, pixels),
        }
    }

    fn plate_image(&mut self, im: &ImageItem, plate: Option<InkId>, pixels: usize) -> Result<(), PsError> {
        let process = plate.filter(|p| p.index() < 4).map(|p| p.index());
        let tints: Option<Vec<u8>> = match &im.space {
            ImageSpace::Gray => (process == Some(Process::Black.index())).then(|| im.samples.iter().map(|v| 255 - v).collect()),
            ImageSpace::Cmyk => process.map(|i| im.samples.iter().skip(i).step_by(4).copied().collect()),
            ImageSpace::Rgb => match process {
                Some(i) => {
                    let rgb: Vec<f32> = im.samples.iter().map(|v| f32::from(*v) / 255.0).collect();
                    let mut cmyk = vec![0.0_f32; pixels * 4];
                    self.cms.from_srgb(&rgb, &mut cmyk).ok_or_else(|| self.unsupported("an RGB image the printing condition can't convert"))?;
                    Some(cmyk.iter().skip(i).step_by(4).map(|v| byte(*v)).collect())
                }
                None => None,
            },
            ImageSpace::Inks(ids) => {
                let weights = self.weights(ids)?;
                let route: Vec<(usize, f32)> =
                    weights.iter().enumerate().filter_map(|(i, w)| w.iter().find(|(id, _)| Some(*id) == plate).map(|(_, t)| (i, *t))).collect();
                (!route.is_empty()).then(|| {
                    im.samples
                        .chunks_exact(ids.len())
                        .map(|px| {
                            let keep = route.iter().fold(1.0_f32, |k, &(i, t)| k * (1.0 - f32::from(px.get(i).copied().unwrap_or(0)) / 255.0 * t));
                            byte(1.0 - keep)
                        })
                        .collect()
                })
            }
        };
        let Some(tints) = tints else {
            if !im.overprint.fill {
                // A paint that doesn't name the plate and doesn't overprint knocks it out.
                let _ = writeln!(self.s, "q {} cm 1 g 0 0 m 1 0 l 1 1 l 0 1 l h f Q", psw::matrix(im.transform));
            }
            return Ok(());
        };
        self.write_image(im.transform, "/DeviceGray", im.size, &[1.0, 0.0], data::samples(self.level, &tints))
    }

    fn composite_image(&mut self, im: &ImageItem, pixels: usize) -> Result<(), PsError> {
        let op = im.overprint.fill;
        if let Some(jpeg) = &im.jpeg
            && matches!(im.space, ImageSpace::Gray | ImageSpace::Cmyk)
        {
            let (space, decode): (&str, &[f32]) = match im.space {
                ImageSpace::Gray => ("/DeviceGray", &[0.0, 1.0]),
                _ if data::adobe_inverted(jpeg) => ("/DeviceCMYK", &[1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0]),
                _ => ("/DeviceCMYK", &[0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0]),
            };
            self.open(op);
            self.write_image(im.transform, space, im.size, decode, Payload::dct(jpeg))?;
            self.close(op);
            return Ok(());
        }
        let (space, n, samples): (String, usize, std::borrow::Cow<'_, [u8]>) = match &im.space {
            ImageSpace::Gray => ("/DeviceGray".into(), 1, im.samples[..].into()),
            ImageSpace::Rgb => ("/DeviceRGB".into(), 3, im.samples[..].into()),
            ImageSpace::Cmyk => ("/DeviceCMYK".into(), 4, im.samples[..].into()),
            ImageSpace::Inks(ids) => {
                let (out, samples) = self.ink_samples(ids, &im.samples)?;
                let process_only = out.iter().all(|id| id.index() < 4);
                // An overprinting image of fewer than four process inks paints just those as a
                // Separation or (LanguageLevel 3) DeviceN image.
                let exact_cmyk = !op || out.len() == 4;
                let single = out.len() == 1;
                if process_only && (exact_cmyk || (!single && self.level == PsLevel::Level2)) {
                    if !exact_cmyk {
                        self.counts.approximated_overprints += 1;
                    }
                    let mut cmyk = vec![0_u8; pixels * 4];
                    for (px, src) in cmyk.as_chunks_mut::<4>().0.iter_mut().zip(samples.chunks_exact(out.len().max(1))) {
                        for (id, v) in out.iter().zip(src) {
                            if let Some(c) = px.get_mut(id.index()) {
                                *c = *v;
                            }
                        }
                    }
                    ("/DeviceCMYK".into(), 4, cmyk.into())
                } else if let [id] = out[..] {
                    (self.separation(id)?, 1, samples.into())
                } else if self.level == PsLevel::Level3 {
                    (self.device_n(&out)?, out.len(), samples.into())
                } else {
                    self.counts.converted_to_process += 1;
                    let alternates = out.iter().map(|&id| self.colorant(id).map(|(_, a)| a)).collect::<Result<Vec<_>, _>>()?;
                    let mut cmyk = Vec::with_capacity(pixels * 4);
                    for src in samples.chunks_exact(out.len().max(1)) {
                        let mut keep = [1.0_f32; 4];
                        for (v, a) in src.iter().zip(&alternates) {
                            let v = f32::from(*v) / 255.0;
                            for (k, a) in keep.iter_mut().zip(a) {
                                *k *= 1.0 - v * a;
                            }
                        }
                        cmyk.extend(keep.map(|k| byte(1.0 - k)));
                    }
                    ("/DeviceCMYK".into(), 4, cmyk.into())
                }
            }
        };
        let decode: Vec<f32> = std::iter::repeat_n([0.0, 1.0], n).flatten().collect();
        self.open(op);
        self.write_image(im.transform, &space, im.size, &decode, data::samples(self.level, &samples))?;
        self.close(op);
        Ok(())
    }

    /// For each of `ids`, the inks a full tint of it prints on and how much, as
    /// [`InkSet::resolve`] gives them.
    fn weights(&self, ids: &[InkId]) -> Result<Vec<Vec<(InkId, f32)>>, PsError> {
        ids.iter()
            .map(|&id| match self.inks.resolve(&InkColor::Tints(vec![(id, 1.0)]), self.cms) {
                InkColor::Tints(t) => Ok(t),
                _ => Err(self.unsupported("an image of an ink not in the page's inks")),
            })
            .collect()
    }

    /// Samples of `ids` as samples of the inks they print on (in ink order), tints on one ink
    /// combining as overlapping halftones, as the PDF writer combines them.
    fn ink_samples(&self, ids: &[InkId], samples: &[u8]) -> Result<(Vec<InkId>, Vec<u8>), PsError> {
        let weights = self.weights(ids)?;
        let out: Vec<InkId> = weights.iter().flatten().map(|(id, _)| *id).collect::<BTreeSet<_>>().into_iter().collect();
        let routes: Vec<Vec<(usize, f32)>> =
            weights.iter().map(|w| w.iter().filter_map(|(id, t)| out.iter().position(|o| o == id).map(|i| (i, *t))).collect()).collect();
        let mut result = Vec::with_capacity(samples.len() / ids.len().max(1) * out.len());
        let mut keep = vec![1.0_f32; out.len()];
        for px in samples.chunks_exact(ids.len().max(1)) {
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
        Ok((out, result))
    }

    /// An 8-bit image in `space` over the unit square `transform` maps; `decode` has two
    /// values per component.
    fn write_image(&mut self, transform: Affine, space: &str, (w, h): (u32, u32), decode: &[f32], payload: Payload<'_>) -> Result<(), PsError> {
        let _ = writeln!(self.s, "q {} cm {space} setcolorspace", psw::matrix(transform));
        let dict = format!("<< /ImageType 1 /Width {w} /Height {h} /BitsPerComponent 8 /Decode [{}] /ImageMatrix [{w} 0 0 {h} 0 0]", nums(decode));
        self.flush();
        data::write_image(&mut self.out, self.level, self.binary, &dict, &payload);
        self.s.push_str("Q\n");
        Ok(())
    }
}

/// The tint `resolved` paints on `plate`, `None` when it leaves the plate as it is: it paints
/// nothing, or it overprints and doesn't name the plate's ink (with nonzero overprint mode, a
/// zero DeviceCMYK component doesn't name its ink).
fn plate_tint(resolved: &InkColor, overprint: Overprint, plate: Option<InkId>) -> Option<f32> {
    match resolved {
        InkColor::None => None,
        InkColor::All(t) => Some(unit(*t)),
        InkColor::Tints(tints) => {
            let (mut tint, mut named) = (0.0_f32, false);
            for (id, t) in tints {
                if Some(*id) == plate {
                    tint = 1.0 - (1.0 - tint) * (1.0 - unit(*t));
                    named = true;
                }
            }
            if overprint.nonzero && resolved.as_cmyk().is_some() {
                named &= tint > 0.0;
            }
            (!(overprint.fill && !named)).then_some(tint)
        }
    }
}

/// Tints by ink, one listed twice combining as overlapping halftones.
fn merge(tints: &[(InkId, f32)]) -> BTreeMap<InkId, f32> {
    let mut merged = BTreeMap::new();
    for &(id, t) in tints {
        let v = merged.entry(id).or_insert(0.0);
        *v = 1.0 - (1.0 - *v) * (1.0 - unit(t));
    }
    merged
}

/// Stops that start at 0 and end at 1, so a stitching function over `[0 1]` holds them where
/// they are.
fn spanning(mut stops: Vec<(f64, Vec<f32>)>) -> Vec<(f64, Vec<f32>)> {
    if let Some((first, c)) = stops.first().cloned()
        && first > 0.0
    {
        stops.insert(0, (0.0, c));
    }
    if let Some((last, c)) = stops.last().cloned()
        && last < 1.0
    {
        stops.push((1.0, c));
    }
    stops
}

/// The gradient parameter at point `g` of gradient space, before clamping to `0..=1`.
fn parameter(geometry: &GradientGeometry, g: Point) -> f64 {
    match geometry {
        GradientGeometry::Axial { start, end, .. } => {
            let d = *end - *start;
            let len2 = d.hypot2();
            if len2 > 0.0 { (g - *start).dot(d) / len2 } else { 0.0 }
        }
        GradientGeometry::Radial { start, start_radius, end, end_radius, .. } => {
            // |g − c(t)| = r(t), with c and r interpolated from the start to the end circle; the
            // largest t with r(t) ≥ 0 paints (ISO 32000-2 §8.7.4.5.4).
            let (dc, dr, pd) = (*end - *start, end_radius - start_radius, g - *start);
            let a = dc.hypot2() - dr * dr;
            let b = pd.dot(dc) + start_radius * dr;
            let c = pd.hypot2() - start_radius * start_radius;
            let valid = |t: f64| start_radius + t * dr >= 0.0;
            if a.abs() < 1e-12 {
                return if b.abs() > 1e-12 { c / (2.0 * b) } else { 0.0 };
            }
            let disc = b * b - a * c;
            if disc < 0.0 {
                // Outside the cone the circles sweep: no color, written as the start's.
                return 0.0;
            }
            let root = disc.sqrt();
            let (t0, t1) = ((b + root) / a, (b - root) / a);
            let (hi, lo) = if t0 > t1 { (t0, t1) } else { (t1, t0) };
            if valid(hi) {
                hi
            } else if valid(lo) {
                lo
            } else {
                0.0
            }
        }
    }
}

/// The tint of each of `ids` at gradient offset `t`, clamped to the stops' ends.
fn at_offset(stops: &[(f32, BTreeMap<InkId, f32>)], t: f32, ids: &[InkId], out: &mut [f32]) {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    let tint = |m: &BTreeMap<InkId, f32>, id: &InkId| m.get(id).copied().unwrap_or(0.0);
    let after = stops.iter().position(|(o, _)| *o >= t);
    let (a, b, f) = match after {
        Some(0) | None => {
            let stop = if after.is_none() { stops.last() } else { stops.first() };
            let Some((_, m)) = stop else { return };
            for (o, id) in out.iter_mut().zip(ids) {
                *o = tint(m, id);
            }
            return;
        }
        Some(i) => {
            let (Some(a), Some(b)) = (stops.get(i - 1), stops.get(i)) else { return };
            let span = b.0 - a.0;
            (a, b, if span > 0.0 { (t - a.0) / span } else { 1.0 })
        }
    };
    for (o, id) in out.iter_mut().zip(ids) {
        *o = tint(&a.1, id) + (tint(&b.1, id) - tint(&a.1, id)) * f;
    }
}

/// A colorant name as a PostScript name: `/Name` when it is plain, `(…) cvn` otherwise.
fn ps_name(name: &str) -> String {
    if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b)) {
        format!("/{name}")
    } else {
        format!("{} cvn", psw::string(name))
    }
}

fn ps_rule(rule: FillRule) -> psw::FillRule {
    match rule {
        FillRule::NonZero => psw::FillRule::NonZero,
        FillRule::EvenOdd => psw::FillRule::EvenOdd,
    }
}

fn num(v: f64) -> String {
    psw::num(v)
}

fn nums(vs: &[f32]) -> String {
    vs.iter().map(|v| psw::num(f64::from(*v))).collect::<Vec<_>>().join(" ")
}

fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

fn byte(v: f32) -> u8 {
    (unit(v) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plate_is_left_alone_only_by_an_overprinting_paint_that_does_not_name_it() {
        let cyan = InkId::process(Process::Cyan);
        let black = InkId::process(Process::Black);
        let c = InkColor::Tints(vec![(cyan, 0.4)]);
        let knockout = Overprint::default();
        let over = Overprint { fill: true, ..Overprint::default() };
        assert_eq!(plate_tint(&c, knockout, Some(black)), Some(0.0), "knocks out");
        assert_eq!(plate_tint(&c, over, Some(black)), None, "overprints");
        assert!(plate_tint(&c, over, Some(cyan)).is_some_and(|t| t > 0.0));
        // Under OPM 1 a zero DeviceCMYK component doesn't name its ink.
        let zero_black = InkColor::Tints(vec![(cyan, 0.4), (black, 0.0)]);
        assert_eq!(plate_tint(&zero_black, over, Some(black)), Some(0.0));
        assert_eq!(plate_tint(&zero_black, Overprint { nonzero: true, ..over }, Some(black)), None);
        assert_eq!(plate_tint(&InkColor::All(1.0), over, None), Some(1.0), "registration reaches every plate");
    }

    #[test]
    fn a_radial_gradient_runs_from_the_start_circle_to_the_end_circle() {
        let geometry = GradientGeometry::Radial {
            start: Point::new(0.0, 0.0),
            start_radius: 0.0,
            end: Point::new(0.0, 0.0),
            end_radius: 10.0,
            extend: [true, true],
            transform: Affine::IDENTITY,
        };
        let near = parameter(&geometry, Point::new(2.0, 0.0));
        let far = parameter(&geometry, Point::new(0.0, 8.0));
        assert!(near < far && near > 0.0 && far < 1.0, "{near} {far}");
    }
}
