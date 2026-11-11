//! Stories of the transform preview.
//!
//! The moving layer is `MockImage` uploaded as a mip pyramid in which every level below the first
//! has its own tint, so the level the preview picks shows. It sits in the middle of a blank
//! document (the checkerboard). [`Document`] is the one stage the warp stories share.

use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use craft_behavior::kurbo::{Point, Rect};
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::contract::Show as _;
use craft_ui::testing::canvas_story::StoryDocument;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Color32, ColorImage, Ui};

use super::{PreviewMap, PreviewQuality, PreviewTextures, StylePlan, StyledContent, StyledPreview, StyledSource, TransformPreview};
use crate::ui::mock_image::{MockImage, StoryView, story_canvas};
use crate::ui::warp::WarpMesh;
use crate::ui::warp::pin::stories::{IdwSolver, sample_pins};

/// The document's size in pixels (3:2).
const DOCUMENT: [u32; 2] = [960, 640];
/// The turn of the rotated box, in degrees.
const TURN: f64 = 20.0;
/// The skew of the skewed box, in degrees.
const SKEW: f64 = 20.0;
/// How far the perspective box's far edge is narrowed, in percent of the width.
const NARROWING: f64 = 25.0;
/// How far the warp lifts its top edge, in percent of the height.
const BEND: f64 = 30.0;
/// The opacity of the opacity story.
const OPACITY: f32 = 0.5;
/// The zoom of the mip story, in percent: far enough out for a mip level.
const MIP_ZOOM: f64 = 25.0;
/// How far the clipped box reaches past the layer on each side, as a fraction of the document's size.
const CLIP_REACH: f64 = 0.4;
/// The drop shadow's offset, as a fraction of the document's width.
const SHADOW_OFFSET: f64 = 1.0 / 60.0;
/// The tints of the mip levels below the first.
const TINTS: [[u8; 3]; 4] = [[255, 80, 60], [60, 200, 90], [70, 110, 255], [240, 200, 40]];

/// A blank document with a layer over its middle half: the stage of the warp stories. The layer's
/// pixels are `MockImage` drawn at the layer's size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Document {
    /// The document's size in pixels.
    pub(crate) size: [u32; 2],
}

/// How a story canvas is zoomed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Zoom {
    /// The whole document fits the canvas.
    Fit,
    /// Device pixels per document pixel, in percent.
    Percent(f64),
}

impl Document {
    /// The default document.
    pub(crate) fn standard() -> Self {
        Self { size: DOCUMENT }
    }

    /// The layer's frame: the middle half of the document.
    pub(crate) fn layer(&self) -> Rect {
        let (w, h) = (f64::from(self.size[0]), f64::from(self.size[1]));
        Rect::new(w / 4.0, h / 4.0, w * 0.75, h * 0.75)
    }

    /// The layer's size in pixels.
    pub(crate) fn layer_size(&self) -> [u32; 2] {
        let layer = self.layer();
        [layer.width().round().max(1.0) as u32, layer.height().round().max(1.0) as u32]
    }

    /// A story canvas showing the document under `zoom`, centred on `centre` (`None`: the middle),
    /// with `controls` shown in its body.
    pub(crate) fn canvas(
        &self,
        ui: &mut Ui,
        zoom: Zoom,
        centre: Option<Point>,
        controls: impl FnMut(&mut Ui, egui::Rect) -> StoryOutput,
    ) -> StoryOutput {
        let view = match (zoom, centre) {
            (Zoom::Fit, _) => StoryView::Fit,
            (Zoom::Percent(percent), None) => StoryView::Zoom(percent),
            (Zoom::Percent(percent), Some(centre)) => StoryView::At { zoom: percent, centre },
        };
        story_canvas(ui, Some(&StoryDocument::new(self.size[0], self.size[1])), view, controls)
    }

    /// The layer's textures, built once per layer size: `MockImage` at every mip level, each level
    /// below the first tinted.
    pub(crate) fn textures(&self) -> Arc<PreviewTextures> {
        type Kept = Mutex<Vec<([u32; 2], Arc<PreviewTextures>)>>;
        static KEPT: OnceLock<Kept> = OnceLock::new();
        let size = self.layer_size();
        let mut kept = KEPT.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, textures)) = kept.iter().find(|(s, _)| *s == size) {
            return Arc::clone(textures);
        }
        let name = format!("transform-story-{}x{}", size[0], size[1]);
        let textures = Arc::new(PreviewTextures::from_levels(name, pyramid(size), [1.0, 1.0]));
        kept.push((size, Arc::clone(&textures)));
        textures
    }
}

/// `MockImage` at `size` and every halving of it down to one texel, the levels below the first
/// tinted.
fn pyramid(size: [u32; 2]) -> Vec<Arc<ColorImage>> {
    let picture = MockImage::new(size);
    let region = PixelRect::new(0, 0, size[0] as i32, size[1] as i32);
    let mut levels = Vec::new();
    let mut factor = 1u32;
    loop {
        let (w, h) = (size[0].div_ceil(factor).max(1), size[1].div_ceil(factor).max(1));
        let mut image = ColorImage::filled([w as usize, h as usize], Color32::TRANSPARENT);
        picture.read(region, factor, &mut image);
        tint(&mut image, levels.len());
        levels.push(Arc::new(image));
        if w <= 1 && h <= 1 {
            break;
        }
        factor *= 2;
    }
    levels
}

/// Mixes level `level`'s tint into `image` (the picture is opaque, so premultiplied is the same).
fn tint(image: &mut ColorImage, level: usize) {
    if level == 0 {
        return;
    }
    let tint = TINTS[(level - 1) % TINTS.len()];
    let mix = |c: u8, t: u8| ((u16::from(c) * 3 + u16::from(t) * 2) / 5) as u8;
    for pixel in &mut image.pixels {
        let [r, g, b, a] = pixel.to_array();
        *pixel = Color32::from_rgba_premultiplied(mix(r, tint[0]), mix(g, tint[1]), mix(b, tint[2]), a);
    }
}

fn corners(r: Rect) -> [Point; 4] {
    [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]
}

fn rotated(r: Rect, degrees: f64) -> [Point; 4] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let c = r.center();
    corners(r).map(|p| {
        let (dx, dy) = (p.x - c.x, p.y - c.y);
        Point::new(c.x + dx * cos - dy * sin, c.y + dx * sin + dy * cos)
    })
}

fn skewed(r: Rect, degrees: f64) -> [Point; 4] {
    let shift = degrees.to_radians().tan() * r.height() / 2.0;
    let [tl, tr, br, bl] = corners(r);
    [Point::new(tl.x + shift, tl.y), Point::new(tr.x + shift, tr.y), Point::new(br.x - shift, br.y), Point::new(bl.x - shift, bl.y)]
}

/// The bottom corners crossed: a Distort dragged past the horizon.
fn crossed(r: Rect) -> [Point; 4] {
    let [tl, tr, br, bl] = corners(r);
    let inset = r.width() * 0.1;
    [tl, tr, Point::new(bl.x + inset, br.y), Point::new(br.x - inset, bl.y)]
}

/// The top edge narrowed by `percent` of the width on each side.
fn narrowed(r: Rect, percent: f64) -> [Point; 4] {
    let inset = r.width() * percent / 100.0;
    let [tl, tr, br, bl] = corners(r);
    [Point::new(tl.x + inset, tl.y), Point::new(tr.x - inset, tr.y), br, bl]
}

/// A one-patch warp mesh whose top edge bulges up by `percent` of the height.
fn bent(r: Rect, percent: f64) -> WarpMesh {
    let mut mesh = WarpMesh::identity(r, 1, 1);
    let (width, height) = (mesh.width(), mesh.height());
    let lift = r.height() * percent / 100.0;
    for (index, point) in mesh.points.iter_mut().enumerate() {
        let (i, j) = (index % width, index / width);
        let inside = if i == 0 || i + 1 == width { 0.0 } else { 1.0 };
        point.y -= lift * inside * (1.0 - j as f64 / (height - 1) as f64);
    }
    mesh
}

/// The layer through `map` on a story canvas at `at`, clipped to the document.
pub(crate) fn draw(ui: &mut Ui, at: egui::Rect, doc: &Document, textures: &PreviewTextures, map: PreviewMap<'_>, opacity: f32, nearest: bool) {
    TransformPreview::new("Moving layer", textures).map(map).opacity(opacity).nearest(nearest).clip(doc.size).show_at(ui, at);
}

/// One story canvas with the layer through `map`.
fn shown(ui: &mut Ui, zoom: Zoom, opacity: f32, nearest: bool, map: impl Fn(Rect) -> PreviewMap<'static>) -> StoryOutput {
    let doc = Document::standard();
    let textures = doc.textures();
    let layer = doc.layer();
    let centre = (zoom != Zoom::Fit).then(|| layer.center());
    doc.canvas(ui, zoom, centre, |ui, at| {
        draw(ui, at, &doc, &textures, map(layer), opacity, nearest);
        StoryOutput::default()
    })
}

#[story(TransformPreview<'static>, covers = [Default], drive = Drive::Never)]
fn rotate(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    shown(ui, Zoom::Fit, 1.0, false, |layer| PreviewMap::quad(rotated(layer, TURN)))
}

#[story(TransformPreview<'static>, covers = [Default], drive = Drive::Never)]
fn skew(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    shown(ui, Zoom::Fit, 1.0, false, |layer| PreviewMap::quad(skewed(layer, SKEW)))
}

#[story(TransformPreview<'static>, covers = [Default], drive = Drive::Never)]
fn distort_past_the_horizon(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    shown(ui, Zoom::Fit, 1.0, false, |layer| PreviewMap::quad(crossed(layer)))
}

#[story(TransformPreview<'static>, covers = [Default], drive = Drive::Never)]
fn perspective(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    shown(ui, Zoom::Fit, 1.0, false, |layer| PreviewMap::quad(narrowed(layer, NARROWING)))
}

#[story(TransformPreview<'static>, covers = [Default], drive = Drive::Never)]
fn opacity(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    shown(ui, Zoom::Fit, OPACITY, false, |layer| PreviewMap::quad(rotated(layer, TURN)))
}

#[story(TransformPreview<'static>, covers = [Default], drive = Drive::Never)]
fn nearest_neighbor(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    shown(ui, Zoom::Percent(400.0), 1.0, true, |layer| PreviewMap::quad(rotated(layer, TURN)))
}

/// At a small zoom the preview draws a tinted mip level.
#[story(TransformPreview<'static>, covers = [Default], drive = Drive::Never)]
fn mip_levels_at_small_zoom(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    shown(ui, Zoom::Percent(MIP_ZOOM), 1.0, false, |layer| PreviewMap::quad(rotated(layer, TURN)))
}

/// A box reaching past the document shows nothing past its edges.
#[story(TransformPreview<'static>, covers = [Default], drive = Drive::Never)]
fn clipped_to_the_canvas(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    shown(ui, Zoom::Percent(MIP_ZOOM), 1.0, false, |layer| {
        let reach = layer.inflate(f64::from(DOCUMENT[0]) * CLIP_REACH, f64::from(DOCUMENT[1]) * CLIP_REACH);
        PreviewMap::quad(rotated(reach, TURN))
    })
}

#[story(TransformPreview<'static>, covers = [Default], drive = Drive::Never)]
fn warp_mesh(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let doc = Document::standard();
    let textures = doc.textures();
    let layer = doc.layer();
    let mesh = bent(layer, BEND);
    doc.canvas(ui, Zoom::Fit, None, |ui, at| {
        draw(ui, at, &doc, &textures, PreviewMap::mesh(&mesh, layer), 1.0, false);
        StoryOutput::default()
    })
}

#[story(TransformPreview<'static>, covers = [Default], drive = Drive::Never)]
fn pin_mesh(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let doc = Document::standard();
    let textures = doc.textures();
    let layer = doc.layer();
    let solver = IdwSolver::solved(layer, &sample_pins(layer));
    doc.canvas(ui, Zoom::Fit, None, |ui, at| {
        draw(ui, at, &doc, &textures, PreviewMap::pins(solver.view(), layer), 1.0, false);
        StoryOutput::default()
    })
}

#[story(TransformPreview<'static>, covers = [Default], drive = Drive::Never)]
fn function_map(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let doc = Document::standard();
    let textures = doc.textures();
    let layer = doc.layer();
    let ripple = |p: Point| {
        let wave = (p.y - layer.y0) / layer.height() * std::f64::consts::TAU * 1.5;
        Point::new(p.x + layer.width() * 0.05 * wave.sin(), p.y)
    };
    doc.canvas(ui, Zoom::Fit, None, |ui, at| {
        draw(ui, at, &doc, &textures, PreviewMap::function(layer, PreviewQuality::PLANE_CELLS, &ripple), 1.0, false);
        StoryOutput::default()
    })
}

/// A drop shadow offset down and right, drawn by the story in place of the app's compositor.
struct Shadow {
    /// In document pixels.
    offset: f64,
}

impl StyledSource for Shadow {
    fn revision(&self) -> u64 {
        1
    }

    fn pixel_cost_ns(&self) -> f64 {
        20.0
    }

    fn reach(&self, scale: f64) -> i32 {
        (self.offset * scale).ceil() as i32 + 1
    }

    fn compose(&self, plan: &StylePlan, content: &StyledContent) -> Option<ColorImage> {
        let shown = plan.shown;
        let (w, h) = (shown.width() as usize, shown.height() as usize);
        let shift = (self.offset * plan.scale).round() as i32;
        let columns = content.rect.width() as usize;
        let texel = |x: i32, y: i32| -> [f32; 4] {
            if x < content.rect.x0 || y < content.rect.y0 || x >= content.rect.x1 || y >= content.rect.y1 {
                return [0.0; 4];
            }
            let index = (y - content.rect.y0) as usize * columns + (x - content.rect.x0) as usize;
            content.texels.get(index).copied().unwrap_or([0.0; 4])
        };
        let mut out = ColorImage::filled([w, h], Color32::TRANSPARENT);
        for (row, line) in out.pixels.chunks_mut(w.max(1)).enumerate() {
            let y = shown.y0 + row as i32;
            for (column, pixel) in line.iter_mut().enumerate() {
                let x = shown.x0 + column as i32;
                let [r, g, b, a] = texel(x, y);
                let shadow = texel(x - shift, y - shift)[3] * 0.5;
                let alpha = a + shadow * (1.0 - a);
                let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                // The shadow is black: it adds alpha and no colour.
                *pixel = Color32::from_rgba_premultiplied(to_byte(r * a), to_byte(g * a), to_byte(b * a), to_byte(alpha));
            }
        }
        Some(out)
    }
}

#[story(TransformPreview<'static>, covers = [Default], drive = Drive::Never)]
fn styled_layer(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    static PREVIEW: OnceLock<StyledPreview> = OnceLock::new();
    let preview = PREVIEW.get_or_init(|| StyledPreview::new("transform-story-styled"));
    let doc = Document::standard();
    let textures = doc.textures();
    let shadow = Shadow { offset: f64::from(doc.size[0]) * SHADOW_OFFSET };
    let map = PreviewMap::quad(rotated(doc.layer(), TURN));
    doc.canvas(ui, Zoom::Fit, None, |ui, at| {
        TransformPreview::new("Moving layer", &textures).map(map).clip(doc.size).styled(preview, &shadow).show_at(ui, at);
        StoryOutput::default()
    })
}
