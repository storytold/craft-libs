//! The reference writer: draws a scene with krilla through craft-pdf.
//!
//! Each scene feature maps to one plain krilla call, so the output shows what the shared writer
//! does on its own. Features the writer can't express yet return [`Error::Unsupported`] before
//! anything is drawn.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use craft_pdf::write::backend as krilla;
use craft_pdf::write::print::{self, PrintColor};
use craft_pdf::write::{self, Metadata, Writer};
use krilla::blend::{BlendMode, GroupOptions, Overprint};
use krilla::color::device_n::{self, Colorant, DeviceNSpace, Process};
use krilla::color::separation::{SeparationColorant, SeparationSpace};
use krilla::color::{RegularColor, cmyk, luma, rgb, separation};
use krilla::geom::{Path, PathBuilder, Point, Rect, Size, Transform};
use krilla::image::{BitsPerComponent, CustomImage, Image, ImageColorspace};
use krilla::mask::{Mask as KrillaMask, MaskType, TransferFunction};
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, LinearGradient, MeshGradient, MeshPatch, Paint as KrillaPaint, SpreadMethod, Stop, Stroke as KrillaStroke};
use krilla::pdf::{Pdf, PdfDocument};
use krilla::surface::Surface;
use krilla::text::{Font, TextDirection};

use crate::Error;
use crate::scene::{Blend, GroupCs, ImageSource, Kind, MAX_NESTING, MaskKind, Named, Object, Paint, Scene, Stroke};

/// The font used for text objects, from `scripts/fetch-pdf-test-fonts.py`.
pub const FONT_FILE: &str = "Inter-Regular.ttf";
/// Directory holding [`FONT_FILE`].
pub const FONT_DIR_ENV: &str = "CRAFT_PDF_TEST_FONT_DIR";

/// What a scene may refer to outside itself.
pub struct Context<'a> {
    /// Scenes that `pdf` objects can place.
    pub scenes: &'a [Scene],
    /// Font data for text objects; without it text is skipped with a note.
    pub font: Option<Arc<Vec<u8>>>,
}

impl<'a> Context<'a> {
    /// Reads the font from [`FONT_DIR_ENV`] when it is set and the file exists.
    pub fn from_env(scenes: &'a [Scene]) -> Self {
        let font =
            std::env::var_os(FONT_DIR_ENV).map(|dir| PathBuf::from(dir).join(FONT_FILE)).and_then(|path| std::fs::read(path).ok()).map(Arc::new);
        Self { scenes, font }
    }
}

/// A written scene.
#[derive(Debug)]
pub struct Output {
    pub pdf: Vec<u8>,
    /// What was left out, e.g. text without a font.
    pub notes: Vec<String>,
}

/// Write `scene` as a one-page PDF.
pub fn write(scene: &Scene, ctx: &Context<'_>) -> Result<Output, Error> {
    write_nested(scene, ctx, 0, false)
}

/// Write `scene` as far as the writer can, to see what a pending scene prints today. Features
/// the writer lacks but can leave out are left out with a note; the others still return
/// [`Error::Unsupported`].
pub fn write_partial(scene: &Scene, ctx: &Context<'_>) -> Result<Output, Error> {
    write_nested(scene, ctx, 0, true)
}

fn write_nested(scene: &Scene, ctx: &Context<'_>, depth: usize, partial: bool) -> Result<Output, Error> {
    let mut resolver = Resolver { scene, ctx, depth, partial, notes: Vec::new(), font: None };
    let nodes = resolver.objects(&scene.objects)?;
    let notes = resolver.notes;
    let [width, height] = scene.page;
    let page = PageSettings::from_wh(width, height).ok_or_else(|| Error::Scene(format!("{}: bad page size", scene.id)))?;
    let settings = settings(scene)?;
    let pdf = write::guard(|| {
        let mut writer = Writer::new(settings, Metadata::default());
        let mut page = writer.document_mut().start_page_with(page);
        let mut surface = page.surface();
        for node in &nodes {
            draw(node, &mut surface);
        }
        surface.finish();
        page.finish();
        writer.finish().map_err(Error::from)
    })?;
    Ok(Output { pdf, notes })
}

/// The writer's defaults, or the print settings for the scene's output intent; an explicit
/// `group_cs` overrides either.
fn settings(scene: &Scene) -> Result<krilla::SerializeSettings, Error> {
    let base = match scene.settings.output_intent.as_deref() {
        None => krilla::SerializeSettings::default(),
        Some(name) => {
            let color = print_color(name).ok_or_else(|| Error::Scene(format!("{}: unknown output intent {name}", scene.id)))?;
            print::settings(color)?
        }
    };
    let group_color_space = match scene.settings.group_cs {
        None => base.group_color_space,
        Some(GroupCs::Rgb) => krilla::GroupColorSpace::Rgb,
        Some(GroupCs::Cmyk) => krilla::GroupColorSpace::Cmyk,
    };
    Ok(krilla::SerializeSettings { group_color_space, ..base })
}

/// The printing condition for each name in [`OUTPUT_INTENTS`](crate::scene::OUTPUT_INTENTS).
fn print_color(name: &str) -> Option<PrintColor<'static>> {
    match name {
        "fogra39" => Some(PrintColor::fogra39(craft_cms::builtin::COATED_FOGRA39_ICC)),
        _ => None,
    }
}

fn unsupported(feature: &str) -> Error {
    Error::Unsupported(feature.to_owned())
}

/// A scene object resolved to krilla values, drawable without failing.
enum Node {
    Shape {
        path: Path,
        fill: Option<Fill>,
        stroke: Option<KrillaStroke>,
    },
    Text {
        at: Point,
        size: f32,
        text: String,
        font: Font,
        fill: Option<Fill>,
        stroke: Option<KrillaStroke>,
    },
    Image {
        image: Image,
        rect: [f32; 4],
    },
    Pdf {
        doc: PdfDocument,
        rect: [f32; 4],
    },
    Group {
        options: GroupOptions,
        children: Vec<Node>,
    },
    /// Outermost first: blend mode, soft mask, opacity group, overprint.
    Wrapped {
        blend: Option<BlendMode>,
        mask: Option<SoftMask>,
        opacity: Option<NormalizedF32>,
        overprint: Option<Overprint>,
        inner: Box<Node>,
    },
}

struct SoftMask {
    kind: MaskType,
    backdrop: Option<RegularColor>,
    transfer: Option<TransferFunction>,
    content: Vec<Node>,
}

struct Resolver<'s, 'c> {
    scene: &'s Scene,
    ctx: &'c Context<'c>,
    depth: usize,
    /// Leave out features the writer lacks instead of failing; see [`write_partial`].
    partial: bool,
    notes: Vec<String>,
    font: Option<Font>,
}

impl Resolver<'_, '_> {
    /// A feature the writer lacks: a note when writing partially, otherwise unsupported.
    // Every scene feature is written today; scenes for later work (the flattener) call it again.
    #[allow(dead_code)]
    fn lacks(&mut self, feature: &str) -> Result<(), Error> {
        if !self.partial {
            return Err(unsupported(feature));
        }
        let note = format!("{}: {feature} left out", self.scene.id);
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
        Ok(())
    }

    fn objects(&mut self, objects: &[Object]) -> Result<Vec<Node>, Error> {
        let mut nodes = Vec::new();
        for object in objects {
            if let Some(node) = self.object(object)? {
                nodes.push(node);
            }
        }
        Ok(nodes)
    }

    fn object(&mut self, o: &Object) -> Result<Option<Node>, Error> {
        let overprint = match o.overprint {
            Some(op) => Some(Overprint {
                fill: op.fill,
                stroke: op.stroke,
                nonzero: match op.mode {
                    0 => false,
                    1 => true,
                    _ => return Err(self.bad("overprint mode must be 0 or 1")),
                },
            }),
            None => None,
        };
        let Some(inner) = self.content(o)? else { return Ok(None) };
        let blend = o.blend.filter(|b| *b != Blend::Normal).map(blend_mode);
        let opacity = match o.opacity {
            Some(value) if value < 1.0 => Some(NormalizedF32::new(value).ok_or_else(|| self.bad("opacity must be 0–1"))?),
            _ => None,
        };
        let mask = match &o.mask {
            Some(mask) => {
                let kind = match mask.kind {
                    MaskKind::Luminosity => MaskType::Luminosity,
                    MaskKind::Alpha => MaskType::Alpha,
                };
                let backdrop = match &mask.backdrop {
                    Some(paint) => match self.color(paint)? {
                        krilla::color::Color::Regular(color) => Some(color),
                        krilla::color::Color::Special(_) => return Err(self.bad("a mask backdrop must be a process or gray color")),
                    },
                    None => None,
                };
                let transfer = match &mask.transfer {
                    Some(curve) => {
                        let samples = curve.iter().map(|v| NormalizedF32::new(*v)).collect::<Option<Vec<_>>>();
                        Some(TransferFunction::Sampled(samples.ok_or_else(|| self.bad("transfer samples must be 0–1"))?))
                    }
                    None => None,
                };
                Some(SoftMask { kind, backdrop, transfer, content: self.objects(&mask.content)? })
            }
            None => None,
        };
        if blend.is_none() && mask.is_none() && opacity.is_none() && overprint.is_none() {
            return Ok(Some(inner));
        }
        Ok(Some(Node::Wrapped { blend, mask, opacity, overprint, inner: Box::new(inner) }))
    }

    fn content(&mut self, o: &Object) -> Result<Option<Node>, Error> {
        Ok(Some(match o.kind {
            Kind::Rect | Kind::Ellipse => {
                let [x, y, w, h] = self.rect(o)?;
                let path = if o.kind == Kind::Rect { rect_path(x, y, w, h) } else { ellipse_path(x, y, w, h) };
                let path = path.ok_or_else(|| self.bad("degenerate shape"))?;
                let (fill, stroke) = self.fill_stroke(o)?;
                if fill.is_none() && stroke.is_none() {
                    return Ok(None);
                }
                Node::Shape { path, fill, stroke }
            }
            Kind::Text => {
                let (Some([x, y]), Some(size), Some(text)) = (o.at, o.size, o.text.clone()) else {
                    return Err(self.bad("text needs at, size and text"));
                };
                let Some(font) = self.font()? else {
                    self.notes.push(format!("{}: text {text:?} skipped ({FONT_DIR_ENV} not set)", self.scene.id));
                    return Ok(None);
                };
                let (fill, stroke) = self.fill_stroke(o)?;
                Node::Text { at: Point::from_xy(x, y), size, text, font, fill, stroke }
            }
            Kind::Image => {
                let source = o.image.as_ref().ok_or_else(|| self.bad("missing image"))?;
                let rect = self.rect(o)?;
                Node::Image { image: self.image(source, rect)?, rect }
            }
            Kind::Pdf => {
                let id = o.scene.as_deref().ok_or_else(|| self.bad("missing scene"))?;
                let placed = self.ctx.scenes.iter().find(|s| s.id == id).ok_or_else(|| self.bad(&format!("unknown scene {id}")))?;
                if self.depth >= MAX_NESTING {
                    return Err(self.bad("placed scenes nest too deep"));
                }
                let rect = self.rect(o)?;
                let output = write_nested(placed, self.ctx, self.depth + 1, self.partial)?;
                self.notes.extend(output.notes);
                let pdf = Pdf::new(Arc::new(output.pdf)).map_err(|e| Error::Read(format!("{id}: {e:?}")))?;
                Node::Pdf { doc: PdfDocument::new(Arc::new(pdf)), rect }
            }
            Kind::Group => Node::Group { options: GroupOptions { isolated: o.isolated, knockout: o.knockout }, children: self.objects(&o.objects)? },
        }))
    }

    fn rect(&self, o: &Object) -> Result<[f32; 4], Error> {
        o.rect.ok_or_else(|| self.bad("missing rect"))
    }

    fn font(&mut self) -> Result<Option<Font>, Error> {
        if self.font.is_none()
            && let Some(data) = &self.ctx.font
        {
            self.font = Some(write::font(data, 0, None)?);
        }
        Ok(self.font.clone())
    }

    fn fill_stroke(&self, o: &Object) -> Result<(Option<Fill>, Option<KrillaStroke>), Error> {
        let fill = o.fill.as_ref().map(|p| self.paint(p)).transpose()?.map(|paint| Fill { paint, ..Default::default() });
        let stroke = match &o.stroke {
            Some(Stroke { paint, width }) => Some(KrillaStroke { paint: self.paint(paint)?, width: *width, ..Default::default() }),
            None => None,
        };
        Ok((fill, stroke))
    }

    fn paint(&self, paint: &Paint) -> Result<KrillaPaint, Error> {
        match paint {
            Paint::Linear { linear } => {
                mixable(linear.stops.iter().map(|s| &s.paint))?;
                let mut stops = Vec::new();
                for stop in &linear.stops {
                    let offset = NormalizedF32::new(stop.at).ok_or_else(|| self.bad("gradient stop outside 0–1"))?;
                    stops.push(Stop { offset, color: self.color(&stop.paint)?, opacity: NormalizedF32::ONE });
                }
                Ok(LinearGradient {
                    x1: linear.from[0],
                    y1: linear.from[1],
                    x2: linear.to[0],
                    y2: linear.to[1],
                    transform: Transform::identity(),
                    spread_method: SpreadMethod::Pad,
                    stops,
                    anti_alias: false,
                }
                .into())
            }
            Paint::Mesh { mesh } => {
                mixable(mesh.corners.iter())?;
                let [x, y, w, h] = mesh.rect;
                let corners = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
                let mut points = [(0.0, 0.0); 12];
                for (i, &(ax, ay)) in corners.iter().enumerate() {
                    let (bx, by) = corners[(i + 1) % 4];
                    for step in 0..3 {
                        let t = step as f32 / 3.0;
                        points[3 * i + step] = (ax + (bx - ax) * t, ay + (by - ay) * t);
                    }
                }
                let [a, b, c, d] = &mesh.corners;
                let colors = [self.color(a)?, self.color(b)?, self.color(c)?, self.color(d)?];
                let patch = MeshPatch { points, colors, opacities: [NormalizedF32::ONE; 4] };
                Ok(MeshGradient { patches: vec![patch], transform: Transform::identity(), anti_alias: false }.into())
            }
            solid => Ok(self.color(solid)?.into()),
        }
    }

    fn color(&self, paint: &Paint) -> Result<krilla::color::Color, Error> {
        Ok(match paint {
            Paint::Named(Named::Registration) => {
                let all = SeparationSpace::new(SeparationColorant::AllColorants, cmyk::Color::new(255, 255, 255, 255).into());
                separation::Color::new(255, all).into()
            }
            Paint::Cmyk { cmyk } => cmyk_color(*cmyk).into(),
            Paint::Gray { gray } => luma::Color::new(byte(100.0 - gray)).into(),
            Paint::Rgb { rgb } => rgb::Color::new(channel(rgb[0]), channel(rgb[1]), channel(rgb[2])).into(),
            Paint::Spot { spot, tint } => {
                let alternate: RegularColor = self.spot_alternate(spot)?.into();
                separation::Color::new(byte(*tint), SeparationSpace::new(SeparationColorant::Custom(spot.clone()), alternate)).into()
            }
            Paint::Inks { inks } => self.device_n(inks)?.into(),
            Paint::Linear { .. } | Paint::Mesh { .. } => return Err(self.bad("gradients can't nest")),
        })
    }

    fn spot_alternate(&self, spot: &str) -> Result<cmyk::Color, Error> {
        let ink = self.scene.ink(spot).ok_or_else(|| self.bad(&format!("undeclared spot {spot}")))?;
        match ink.cmyk {
            Some(c) => Ok(cmyk_color(c)),
            None => Err(unsupported("Lab spot alternate")),
        }
    }

    /// A DeviceN color in the space of its inks (see [`Self::device_n_space`]).
    fn device_n(&self, inks: &BTreeMap<String, f32>) -> Result<device_n::Color, Error> {
        let space = self.device_n_space(inks.keys())?;
        let tints = space.colorants().iter().map(|c| byte(inks.get(c.name()).copied().unwrap_or(0.0))).collect();
        device_n::Color::new(tints, space).ok_or_else(|| self.bad("DeviceN tints don't match its inks"))
    }

    /// A DeviceN space of the named inks: process inks first, in CMYK order, then the spots in
    /// the order the scene declares them.
    fn device_n_space<'n>(&self, names: impl Iterator<Item = &'n String> + Clone) -> Result<DeviceNSpace, Error> {
        let named = |name: &str| names.clone().any(|n| n == name);
        let mut colorants = Vec::new();
        for process in Process::ALL {
            if named(process.name()) {
                colorants.push(Colorant::Process(process));
            }
        }
        for ink in &self.scene.inks {
            if named(&ink.name) {
                colorants.push(Colorant::Spot { name: ink.name.clone(), alternate: self.spot_alternate(&ink.name)? });
            }
        }
        if let Some(name) = names.clone().find(|name| !colorants.iter().any(|c| c.name() == name.as_str())) {
            return Err(self.bad(&format!("undeclared ink {name}")));
        }
        DeviceNSpace::new(colorants).ok_or_else(|| self.bad("bad DeviceN ink list"))
    }

    fn image(&self, source: &ImageSource, [x0, y0, w, h]: [f32; 4]) -> Result<Image, Error> {
        let [px, py] = source.px;
        let (width, height) = (px as usize, py as usize);
        let image = if let Some(radius) = source.feather {
            let mut data = Vec::with_capacity(width * height);
            for y in 0..height {
                for x in 0..width {
                    let (fx, fy) = ((x as f32 + 0.5) * w / px as f32, (y as f32 + 0.5) * h / py as f32);
                    let edge = fx.min(w - fx).min(fy).min(h - fy);
                    data.push(channel(255.0 * (edge / radius.max(f32::EPSILON)).min(1.0)));
                }
            }
            Synthetic { data, alpha: None, size: (px, py), space: ImageColorspace::Luma, device_n: None }
        } else {
            let (pair, ramp) = match (&source.ramp, &source.split) {
                (Some(pair), _) => (pair, true),
                (_, Some(pair)) => (pair, false),
                _ => return Err(self.bad("image needs ramp, split or feather")),
            };
            let (space, device_n, a, b) = match (ink_tints(&pair[0]), ink_tints(&pair[1])) {
                // Spot and DeviceN paints: one DeviceN image of every ink either names.
                (Some(a), Some(b)) => {
                    let space = self.device_n_space(a.keys().chain(b.keys()))?;
                    let samples = |inks: &BTreeMap<String, f32>| -> Vec<f32> {
                        space.colorants().iter().map(|c| inks.get(c.name()).copied().unwrap_or(0.0) * 2.55).collect()
                    };
                    let (a, b) = (samples(&a), samples(&b));
                    (ImageColorspace::DeviceN, Some(space), a, b)
                }
                _ => {
                    let (space, a) = self.components(&pair[0])?;
                    let (space_b, b) = self.components(&pair[1])?;
                    if space != space_b {
                        return Err(self.bad("image paints must share a color space"));
                    }
                    (space, None, a, b)
                }
            };
            let mut data = Vec::with_capacity(width * height * a.len());
            for _ in 0..height {
                for x in 0..width {
                    let t = if ramp { (x as f32 + 0.5) / px as f32 } else { f32::from(u8::from(2 * x >= width)) };
                    data.extend(a.iter().zip(&b).map(|(a, b)| channel(a + (b - a) * t)));
                }
            }
            let alpha = source.alpha.as_ref().map(|a| {
                let [ax, ay, aw, ah] = a.rect;
                let value = channel(a.opacity * 255.0);
                let mut alpha = Vec::with_capacity(width * height);
                for y in 0..height {
                    for x in 0..width {
                        let (fx, fy) = (x0 + (x as f32 + 0.5) * w / px as f32, y0 + (y as f32 + 0.5) * h / py as f32);
                        let inside = (ax..ax + aw).contains(&fx) && (ay..ay + ah).contains(&fy);
                        alpha.push(if inside { value } else { 0 });
                    }
                }
                alpha
            });
            Synthetic { data, alpha, size: (px, py), space, device_n }
        };
        Image::from_custom(image, false).map_err(|e| self.bad(&e))
    }

    /// Image samples (0–255) for a solid paint.
    fn components(&self, paint: &Paint) -> Result<(ImageColorspace, Vec<f32>), Error> {
        Ok(match paint {
            Paint::Cmyk { cmyk } => (ImageColorspace::Cmyk, cmyk.iter().map(|v| v * 2.55).collect()),
            Paint::Gray { gray } => (ImageColorspace::Luma, vec![(100.0 - gray) * 2.55]),
            Paint::Rgb { rgb } => (ImageColorspace::Rgb, rgb.to_vec()),
            Paint::Inks { .. } | Paint::Spot { .. } => return Err(self.bad("image paints must share a color space")),
            Paint::Named(_) => return Err(unsupported("registration image")),
            Paint::Linear { .. } | Paint::Mesh { .. } => return Err(self.bad("images take solid paints")),
        })
    }

    fn bad(&self, message: &str) -> Error {
        Error::Scene(format!("{}: {message}", self.scene.id))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Space<'a> {
    Cmyk,
    Gray,
    Rgb,
    Spot(&'a str),
    Other,
}

/// krilla brings ink-based gradient colors into one space; RGB and registration don't mix.
fn mixable<'a>(paints: impl Iterator<Item = &'a Paint> + Clone) -> Result<(), Error> {
    let space = paints.clone().next().map(space_of);
    let mixed = paints.clone().any(|p| Some(space_of(p)) != space);
    if mixed && paints.into_iter().any(|p| matches!(p, Paint::Rgb { .. } | Paint::Named(_))) {
        return Err(unsupported("gradient between RGB or registration and other color spaces"));
    }
    Ok(())
}

fn space_of(paint: &Paint) -> Space<'_> {
    match paint {
        Paint::Cmyk { .. } => Space::Cmyk,
        Paint::Gray { .. } => Space::Gray,
        Paint::Rgb { .. } => Space::Rgb,
        Paint::Spot { spot, .. } => Space::Spot(spot),
        _ => Space::Other,
    }
}

/// Ink percentages of a spot or DeviceN paint.
fn ink_tints(paint: &Paint) -> Option<BTreeMap<String, f32>> {
    match paint {
        Paint::Inks { inks } => Some(inks.clone()),
        Paint::Spot { spot, tint } => Some(BTreeMap::from([(spot.clone(), *tint)])),
        _ => None,
    }
}

fn byte(percent: f32) -> u8 {
    channel(percent * 2.55)
}

fn channel(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

fn cmyk_color([c, m, y, k]: [f32; 4]) -> cmyk::Color {
    cmyk::Color::new(byte(c), byte(m), byte(y), byte(k))
}

fn blend_mode(blend: Blend) -> BlendMode {
    match blend {
        Blend::Normal => BlendMode::Normal,
        Blend::Multiply => BlendMode::Multiply,
        Blend::Screen => BlendMode::Screen,
        Blend::Overlay => BlendMode::Overlay,
        Blend::Darken => BlendMode::Darken,
        Blend::Lighten => BlendMode::Lighten,
        Blend::ColorDodge => BlendMode::ColorDodge,
        Blend::ColorBurn => BlendMode::ColorBurn,
        Blend::HardLight => BlendMode::HardLight,
        Blend::SoftLight => BlendMode::SoftLight,
        Blend::Difference => BlendMode::Difference,
        Blend::Exclusion => BlendMode::Exclusion,
        Blend::Hue => BlendMode::Hue,
        Blend::Saturation => BlendMode::Saturation,
        Blend::Color => BlendMode::Color,
        Blend::Luminosity => BlendMode::Luminosity,
    }
}

fn rect_path(x: f32, y: f32, w: f32, h: f32) -> Option<Path> {
    let mut builder = PathBuilder::new();
    builder.push_rect(Rect::from_xywh(x, y, w, h)?);
    builder.finish()
}

fn ellipse_path(x: f32, y: f32, w: f32, h: f32) -> Option<Path> {
    // Four cubic arcs; KAPPA places the control points for a quarter circle.
    const KAPPA: f32 = 0.552_284_8;
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + rx, y + ry);
    let (ox, oy) = (rx * KAPPA, ry * KAPPA);
    let mut b = PathBuilder::new();
    b.move_to(cx + rx, cy);
    b.cubic_to(cx + rx, cy + oy, cx + ox, cy + ry, cx, cy + ry);
    b.cubic_to(cx - ox, cy + ry, cx - rx, cy + oy, cx - rx, cy);
    b.cubic_to(cx - rx, cy - oy, cx - ox, cy - ry, cx, cy - ry);
    b.cubic_to(cx + ox, cy - ry, cx + rx, cy - oy, cx + rx, cy);
    b.close();
    b.finish()
}

#[derive(Clone, Hash)]
struct Synthetic {
    data: Vec<u8>,
    alpha: Option<Vec<u8>>,
    size: (u32, u32),
    space: ImageColorspace,
    device_n: Option<DeviceNSpace>,
}

impl CustomImage for Synthetic {
    fn color_channel(&self) -> &[u8] {
        &self.data
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

fn draw(node: &Node, surface: &mut Surface<'_>) {
    match node {
        Node::Shape { path, fill, stroke } => {
            surface.set_fill(fill.clone());
            surface.set_stroke(stroke.clone());
            surface.draw_path(path);
        }
        Node::Text { at, size, text, font, fill, stroke } => {
            surface.set_fill(fill.clone());
            surface.set_stroke(stroke.clone());
            surface.draw_text(*at, font.clone(), *size, text, false, TextDirection::Auto);
        }
        Node::Image { image, rect: [x, y, w, h] } => {
            if let Some(size) = Size::from_wh(*w, *h) {
                surface.push_transform(&Transform::from_translate(*x, *y));
                surface.draw_image(image.clone(), size);
                surface.pop();
            }
        }
        Node::Pdf { doc, rect: [x, y, w, h] } => {
            if let Some(size) = Size::from_wh(*w, *h) {
                surface.push_transform(&Transform::from_translate(*x, *y));
                surface.draw_pdf_page(doc, size, 0);
                surface.pop();
            }
        }
        Node::Group { options, children } => {
            let grouped = *options != GroupOptions::default();
            if grouped {
                surface.push_group(*options);
            }
            for child in children {
                draw(child, surface);
            }
            if grouped {
                surface.pop();
            }
        }
        Node::Wrapped { blend, mask, opacity, overprint, inner } => {
            if let Some(blend) = blend {
                surface.push_blend_mode(*blend);
            }
            if let Some(mask) = mask {
                let mut builder = surface.stream_builder();
                let mut sub = builder.surface();
                for child in &mask.content {
                    draw(child, &mut sub);
                }
                sub.finish();
                let stream = builder.finish();
                let mut krilla_mask = KrillaMask::new(stream, mask.kind);
                if let Some(backdrop) = mask.backdrop {
                    krilla_mask = krilla_mask.with_backdrop(backdrop);
                }
                if let Some(transfer) = &mask.transfer {
                    krilla_mask = krilla_mask.with_transfer(transfer.clone());
                }
                surface.push_mask(krilla_mask);
            }
            if let Some(opacity) = opacity {
                surface.push_opacity(*opacity);
            }
            if let Some(overprint) = overprint {
                surface.push_overprint(*overprint);
            }
            draw(inner, surface);
            for pushed in [overprint.is_some(), opacity.is_some(), mask.is_some(), blend.is_some()] {
                if pushed {
                    surface.pop();
                }
            }
        }
    }
}
