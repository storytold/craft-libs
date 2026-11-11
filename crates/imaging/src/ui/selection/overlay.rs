//! [`SelectionOverlay`]: the selection on the canvas as a preview and marching ants.

use std::sync::Arc;

use craft_behavior::kurbo::Point;
use craft_behavior::{Input, Metrics, PixelSize};
use craft_ui::canvas::{PixelRect, RasterDisplay, RasterSource, canvas_space};
use craft_ui::canvas_controls::{Marquee, MarqueeLook};
use craft_ui::contract::{Access, Component, Cx, MeasureCx, NoValue, Outcome, Pattern, Props, Remember as _, Role, Show as _};
use craft_ui::primitives::Revision;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::{Color32, ColorImage, Rect, Response, Ui, Vec2};

use crate::selection::{PreviewMode, QuickMaskLook};
use crate::trace::{MaskRect, MaskSource, Segment, bounded_step, outline, quantised_region, trace_step};

craft_ui::words! {
    /// The accessible name and value of the selection overlay, English by default.
    #[non_exhaustive]
    pub struct SelectionWords {
        /// The preview's name.
        preview: "selectionOverlay.preview" = "Selection preview";
        /// The value with nothing selected.
        nothing: "selectionOverlay.nothing" = "No selection";
        /// The value: the content bounds, each number with the document's unit.
        bounds: template "selectionOverlay.bounds" = "{width} × {height} at {x}, {y}";
        /// Appended to the value while a preview shows.
        with_mode: template "selectionOverlay.withMode" = "{value}, {mode}";
        /// The grayscale preview.
        grayscale: "selectionOverlay.grayscale" = "Grayscale";
        /// The black matte preview.
        black_matte: "selectionOverlay.blackMatte" = "Black Matte";
        /// The white matte preview.
        white_matte: "selectionOverlay.whiteMatte" = "White Matte";
        /// The Quick Mask preview.
        quick_mask: "selectionOverlay.quickMask" = "Quick Mask";
    }
}

/// Where the overlay's outline comes from.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub enum OutlineSource<'a> {
    /// The app's mask: the overlay traces it ([`crate::trace::outline`]) and caches the segments, and builds the
    /// preview texture from it.
    Mask(&'a dyn MaskSource),
    /// Segments the app traced itself (in a job), in document pixels of the full-size document, with the content
    /// `bounds` for the accessible value. Drawn as they are: no cache, no preview texture.
    #[non_exhaustive]
    Segments {
        /// The traced edges.
        segments: &'a [Segment],
        /// The content bounds.
        bounds: MaskRect,
    },
}

impl<'a, M: MaskSource> From<&'a M> for OutlineSource<'a> {
    fn from(mask: &'a M) -> Self {
        OutlineSource::Mask(mask)
    }
}

/// A point to point map in document pixels: where a Free Transform puts the selection, or the app's warp.
pub struct PointMap<'a>(Box<dyn Fn(Point) -> Point + 'a>);

impl<'a> PointMap<'a> {
    /// The map that runs `map`.
    pub fn new(map: impl Fn(Point) -> Point + 'a) -> Self {
        PointMap(Box::new(map))
    }

    fn apply(&self, point: Point) -> Point {
        (self.0)(point)
    }
}

/// What a selection overlay reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SelectionOverlayOutput {
    /// The selection's content bounds in full-size document pixels, `None` when nothing is selected.
    pub bounds: Option<MaskRect>,
    /// The trace ran this pass (the mask's revision, the step, the region or the reduction changed); `false` when the
    /// cached segments were drawn. For a performance readout and tests.
    pub retraced: bool,
}

/// The selection on the canvas, shown once per pass over the canvas rect before the tool's paint.
///
/// What it draws, bottom first:
/// - The preview ([`preview`](Self::preview)), when the mode is not `None` and the source is a mask: a raster display
///   of [`PreviewMode::pixel`] over the whole document, rebuilt only when the mask's revision, the mode, the look or
///   the reduction changes.
/// - The ants ([`MarqueeLook::Ants`]) along the selection's boundary, when [`visible`](Self::visible). Hidden ants keep
///   the preview.
///
/// The trace, for a mask source: step [`trace_step`] of the view's device zoom and the [`reduction`](Self::reduction);
/// region [`quantised_region`] of the view's visible bounds shifted back by the [`offset`](Self::offset), so a small
/// pan reuses the trace; bounds the mask's content bounds (the whole canvas when [`MaskSource::outside`] is above 0)
/// within the region. With a [`mapped`](Self::mapped) outline the whole selection is traced instead (a Free Transform
/// may carry parts from outside the view into it) with [`bounded_step`]. Segments are scaled up by the reduction.
/// The trace is kept per overlay and runs again only when the mask's revision, the step, the region or the reduction
/// changes.
///
/// Each segment end moves by the [`offset`](Self::offset), then through the [`mapped`](Self::mapped) map, then through
/// the canvas view, so the ants are crisp on device pixels at any zoom and rotation.
///
/// AccessKit: the ants are one graphics object named by the label and valued with the content bounds in the
/// [`unit`](Self::unit) ("120 × 80 at 40, 20") or "No selection"; a preview adds its mode's name.
#[must_use = "show the overlay with `show_at`"]
#[derive(Pattern, Props)]
pub struct SelectionOverlay<'a> {
    label: Label,
    source: OutlineSource<'a>,
    /// Whether the ants show (View > Show > Selection Edges, and not while a polygonal or magnetic lasso outline
    /// replaces the selection). Hidden ants keep the preview.
    #[prop]
    visible: bool,
    /// Draws the ants this many whole document pixels from where the selection is: a floating piece, a selection drag
    /// in progress, layers moved together with the selection.
    #[prop(quantity = "offset")]
    offset: [i32; 2],
    /// The source is reduced this many times (Select and Mask's preview proxy): its pixels are that many document
    /// pixels each way. 0 counts as 1.
    #[prop(quantity = "reduction")]
    reduction: u32,
    /// Draws the outline through this map, document point to document point.
    #[prop(func = "Fn(Point) -> Point")]
    mapped: Option<PointMap<'a>>,
    /// The Selection Preview mode (Color Range's modes, or Quick Mask).
    #[prop]
    preview: PreviewMode,
    /// The Quick Mask tint, for [`PreviewMode::QuickMask`].
    #[prop]
    quick_mask: QuickMaskLook,
    /// The unit the accessible value's numbers are written in (the rulers' unit).
    #[prop]
    unit: UnitKind,
}

impl<'a> SelectionOverlay<'a> {
    /// The selection of `source` (a `&impl MaskSource`, or [`OutlineSource::Segments`]), named `label`: ants shown,
    /// no offset, no reduction, no map, no preview, in pixels.
    pub fn new(label: impl Into<Label>, source: impl Into<OutlineSource<'a>>) -> Self {
        Self {
            label: label.into(),
            source: source.into(),
            visible: true,
            offset: [0, 0],
            reduction: 1,
            mapped: None,
            preview: PreviewMode::None,
            quick_mask: QuickMaskLook::default(),
            unit: UnitKind::Pixels,
        }
    }

    /// Shows the overlay over `canvas`, the rect of the canvas it is shown in.
    pub fn show_at(self, ui: &mut Ui, canvas: Rect) -> SelectionOverlayOutput {
        let Self { label, source, visible, offset, reduction, mapped, preview, quick_mask, unit } = self;
        let k = reduction.max(1);
        let env = Env::of(ui);
        let words = env.words::<SelectionWords>();
        let bounds = match source {
            OutlineSource::Mask(mask) => content_bounds(mask, k),
            OutlineSource::Segments { bounds, .. } => (!bounds.is_empty()).then_some(bounds),
        };
        if let OutlineSource::Mask(mask) = source
            && preview != PreviewMode::None
            && mask.size().iter().all(|&side| side > 0)
        {
            let source = SelectionPreviewSource { mask, mode: preview, look: quick_mask, reduction: k };
            RasterDisplay::new(words.preview.clone(), &source).show_at(ui, canvas);
        }
        let has_outline = match source {
            OutlineSource::Mask(_) => bounds.is_some(),
            OutlineSource::Segments { segments, .. } => !segments.is_empty(),
        };
        let mut retraced = false;
        if visible && has_outline {
            let traced: Arc<Vec<Segment>> = match source {
                OutlineSource::Mask(mask) => {
                    let found = OutlineTrace::new(label.clone(), mask, offset, k, mapped.is_some()).show_at(ui, canvas);
                    retraced = found.retraced;
                    found.segments
                }
                OutlineSource::Segments { segments, .. } => Arc::new(segments.to_vec()),
            };
            let [dx, dy] = offset.map(f64::from);
            let place = |p: [i32; 2]| {
                let at = Point::new(f64::from(p[0]) + dx, f64::from(p[1]) + dy);
                mapped.as_ref().map_or(at, |map| map.apply(at))
            };
            let lines = traced.iter().map(|&(a, b)| (place(a), place(b)));
            let value = value_of(&env, &words, bounds, preview, unit);
            Marquee::segments(label, lines).look(MarqueeLook::Ants).value(value).show_at(ui, canvas);
        }
        SelectionOverlayOutput { bounds, retraced }
    }
}

/// The accessible value: the bounds in the unit (or "No selection"), plus the preview's name.
fn value_of(env: &Env, words: &SelectionWords, bounds: Option<MaskRect>, preview: PreviewMode, unit: UnitKind) -> String {
    let units = env.units();
    let base = match bounds {
        None => words.nothing.as_str().to_owned(),
        Some(b) => {
            let n = |v: i32| units.show(unit, f64::from(v));
            let (width, height) = (n(i32::try_from(b.width()).unwrap_or(i32::MAX)), n(i32::try_from(b.height()).unwrap_or(i32::MAX)));
            words.bounds.fill(&[("width", &width), ("height", &height), ("x", &n(b.x0)), ("y", &n(b.y0))])
        }
    };
    let mode = match preview {
        PreviewMode::None => return base,
        PreviewMode::Grayscale => &words.grayscale,
        PreviewMode::BlackMatte => &words.black_matte,
        PreviewMode::WhiteMatte => &words.white_matte,
        PreviewMode::QuickMask => &words.quick_mask,
    };
    words.with_mode.fill(&[("value", &base), ("mode", mode.as_str())])
}

/// The selection's content bounds in full-size document pixels (the whole canvas when the mask is selected outside its
/// content); `None` when nothing is selected.
fn content_bounds(mask: &dyn MaskSource, k: u32) -> Option<MaskRect> {
    let b = if mask.outside() > 0.0 { MaskRect::of_size(mask.size()) } else { mask.content_bounds() };
    let up = |v: i32| v.saturating_mul(i32::try_from(k).unwrap_or(i32::MAX));
    (!b.is_empty()).then(|| MaskRect::new(up(b.x0), up(b.y0), up(b.x1), up(b.y1)))
}

/// The preview of a mask as a document-sized picture: one texel per `PreviewMode::pixel` of the coverage under its
/// top-left document pixel. The revision changes with the mask, the mode, the look and the reduction.
struct SelectionPreviewSource<'a> {
    mask: &'a dyn MaskSource,
    mode: PreviewMode,
    look: QuickMaskLook,
    reduction: u32,
}

impl RasterSource for SelectionPreviewSource<'_> {
    fn size(&self) -> PixelSize {
        let [width, height] = self.mask.size();
        PixelSize::new(width.saturating_mul(self.reduction), height.saturating_mul(self.reduction))
    }

    fn revision(&self) -> Revision {
        let look = &self.look;
        let tint = (look.color.r.to_bits(), look.color.g.to_bits(), look.color.b.to_bits(), look.opacity.to_bits(), look.indicates as u8);
        Revision::of(&(self.mask.revision(), self.mode, self.reduction, self.mask.size(), tint))
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let [width, height] = out.size;
        let (k, factor) = (i64::from(self.reduction), i64::from(factor));
        let mut row = vec![0.0_f32; width];
        for ty in 0..height {
            let y = i32::try_from((i64::from(region.y0) + ty as i64 * factor) / k).unwrap_or(i32::MAX);
            let x0 = i64::from(region.x0);
            if k == 1 {
                self.mask.read_row(y, region.x0, u32::try_from(factor).unwrap_or(1), &mut row);
            } else {
                for (tx, value) in row.iter_mut().enumerate() {
                    *value = self.mask.at(i32::try_from((x0 + tx as i64 * factor) / k).unwrap_or(i32::MAX), y);
                }
            }
            for (pixel, &coverage) in out.pixels[ty * width..(ty + 1) * width].iter_mut().zip(&row) {
                let [r, g, b, a] = self.mode.pixel(coverage, &self.look).0;
                *pixel = Color32::from_rgba_unmultiplied(r, g, b, a);
            }
        }
    }
}

/// The key a trace is cached under: the mask's revision, the step, the region and the reduction.
type TraceKey = (u64, u32, MaskRect, u32);

/// The last trace of one overlay.
#[derive(Clone, Default)]
struct TraceCache {
    key: Option<TraceKey>,
    segments: Arc<Vec<Segment>>,
}

/// The segments a trace found and whether it ran this pass.
#[derive(Clone, Default)]
struct Traced {
    segments: Arc<Vec<Segment>>,
    retraced: bool,
}

/// What the trace leaf reports.
pub(super) struct TraceOutput {
    response: Response,
    segments: Arc<Vec<Segment>>,
    retraced: bool,
}

impl Outcome for TraceOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The style of the trace leaf: it paints nothing.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
pub(super) struct OutlineTraceStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A canvas control that traces a mask's outline for the visible region of the canvas it is shown in and keeps the
/// trace until the mask, the step, the region or the reduction changes.
#[derive(craft_ui::contract::Component, Props)]
#[component(canvas_control)]
pub(super) struct OutlineTrace<'a> {
    #[common]
    common: craft_ui::contract::Common,
    mask: &'a dyn MaskSource,
    offset: [i32; 2],
    reduction: u32,
    whole: bool,
    found: Traced,
}

impl<'a> OutlineTrace<'a> {
    /// A trace of `mask` named `label`, drawn `offset` from where it lies, reduced by `reduction`; `whole` traces the
    /// mask outside the visible region too.
    pub(super) fn new(label: impl Into<Label>, mask: &'a dyn MaskSource, offset: [i32; 2], reduction: u32, whole: bool) -> Self {
        OutlineTrace { common: craft_ui::contract::Common::new(), mask, offset, reduction, whole, found: Traced::default() }.label(label)
    }
}

impl Component for OutlineTrace<'_> {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = OutlineTraceStyle;
    type Output = TraceOutput;

    fn style(_: &StyleCx<'_>, _: OutlineTraceStyleKey) -> OutlineTraceStyle {
        OutlineTraceStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let (mask, k) = (self.mask, self.reduction);
        let device_zoom = space.zoom_device();
        let mut step = trace_step(device_zoom, k);
        let region = if self.whole {
            let far = 1 << 30;
            MaskRect::new(-far, -far, far, far)
        } else {
            // The mask lies `offset` from where it is drawn, so the part of it in view is the visible area shifted back.
            let v = space.view.visible();
            let [dx, dy] = self.offset.map(f64::from);
            quantised_region([v.x0 - dx, v.y0 - dy, v.x1 - dx, v.y1 - dy], step, k)
        };
        let content = if mask.outside() > 0.0 { MaskRect::of_size(mask.size()) } else { mask.content_bounds() };
        let bounds = content.intersect(&region);
        if self.whole {
            step = bounded_step(bounds, step);
        }
        let key = (mask.revision(), step, region, k);
        let cache = cx.memory::<TraceCache>();
        if cache.key == Some(key) {
            self.found = Traced { segments: cache.segments.clone(), retraced: false };
            return;
        }
        let mut segments = outline(mask, bounds, step);
        if k > 1 {
            let scale = i32::try_from(k).unwrap_or(i32::MAX);
            let up = |p: [i32; 2]| [p[0].saturating_mul(scale), p[1].saturating_mul(scale)];
            segments.iter_mut().for_each(|(a, b)| (*a, *b) = (up(*a), up(*b)));
        }
        let segments = Arc::new(segments);
        *cache = TraceCache { key: Some(key), segments: segments.clone() };
        self.found = Traced { segments, retraced: true };
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> TraceOutput {
        TraceOutput { response: cx.response.clone(), segments: self.found.segments, retraced: self.found.retraced }
    }
}
