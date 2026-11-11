//! `Pasteboard`: the area around a document's pages or artboards, and the frames of the pages on it, a canvas
//! control (20.6). The pasteboard colour and the page border come from the application's preferences; artboards
//! show their names above them, and a click on a name selects the artboard.

#[cfg(feature = "testing")]
mod stories;

use super::space::canvas_space;
use crate::contract::{Access, Common, Component, Cx, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Props as _, Role};
use crate::primitives::{PaintShape, Rule, StrokeWidth, TextRun};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, DataColor, RuleLook, StyleColor, StyleCx, TypeRole};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{CanvasView, HitLevel, HitPriority, Metrics, PartKey, Sense};
use craft_theme::Rgba;
use egui::{Color32, Pos2, Rect, Response, Vec2};

/// A page name: `NAME.at(i)` for page `i`, in the order of the frames given.
const NAME: PartKey = PartKey::new("name");

/// The pasteboard's colour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum PasteboardColor {
    /// The theme's canvas surface.
    #[default]
    Default,
    /// Black.
    Black,
    /// Grey 40.
    DarkGray,
    /// Grey 83.
    MediumGray,
    /// Grey 184.
    LightGray,
    /// A colour the user picked.
    Custom(DataColor),
}

impl PasteboardColor {
    /// The fill, or `None` for the theme's.
    pub fn fill(self) -> Option<DataColor> {
        match self {
            PasteboardColor::Default => None,
            PasteboardColor::Black => Some(DataColor::new(Color32::from_gray(0))),
            PasteboardColor::DarkGray => Some(DataColor::new(Color32::from_gray(40))),
            PasteboardColor::MediumGray => Some(DataColor::new(Color32::from_gray(83))),
            PasteboardColor::LightGray => Some(DataColor::new(Color32::from_gray(184))),
            PasteboardColor::Custom(color) => Some(color),
        }
    }
}

/// The border around each page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PageBorder {
    /// A shadow offset down and across.
    #[default]
    DropShadow,
    /// A thin line round the page.
    Line,
    /// No border.
    None,
}

/// One page or artboard on the pasteboard, in document units.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PageFrame {
    /// The page's rectangle in document space.
    pub rect: DocRect,
    /// Shown above the page's top-left corner (artboards, spreads); `None` shows nothing.
    pub name: Option<String>,
    /// The page's own fill under the document (an artboard's background), or `None` for none.
    pub fill: Option<DataColor>,
    /// An artboard's edge: `Some(true)` for the active board (an accent edge, its name in the normal text colour),
    /// `Some(false)` for another board (a faint dark edge, its name muted); `None` for a plain page, which only the
    /// border shows.
    pub active: Option<bool>,
}

impl PageFrame {
    /// A page at `rect` with no name, fill or active mark.
    pub fn new(rect: DocRect) -> Self {
        Self { rect, name: None, fill: None, active: None }
    }

    /// Marks the frame as an artboard, the active one when `active`.
    #[must_use]
    pub fn active(mut self, active: bool) -> Self {
        self.active = Some(active);
        self
    }

    /// Sets the name shown above the page's top-left corner.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the page's own fill under the document.
    #[must_use]
    pub fn fill(mut self, fill: DataColor) -> Self {
        self.fill = Some(fill);
        self
    }
}

/// The style of the pasteboard.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PasteboardStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The pasteboard unless the preference sets a colour.
    fill: StyleColor,
    /// A page's drop shadow, and how far it lies from the page.
    shadow: StyleColor,
    shadow_offset: Vec2,
    /// A page's outline, and the edge of an artboard that is not the active one.
    outline: StyleColor,
    /// The edge of the active artboard.
    active_edge: StyleColor,
    /// The width of a page or artboard edge.
    edge: f32,
    /// A page name in the muted ink, and in the normal ink for the active artboard.
    name: StyleColor,
    name_active: StyleColor,
    /// Between a page's top edge and its name.
    name_gap: f32,
}

/// What the pasteboard reports: its response, the page name clicked and the pasteboard menu request.
#[derive(Debug)]
#[non_exhaustive]
pub struct PasteboardOutput {
    /// The pasteboard's response.
    pub response: Response,
    /// A page name was clicked: the index of its frame. The app selects the artboard or page.
    pub name_clicked: Option<usize>,
    /// The secondary button was released on the canvas background outside every page and name (a press no canvas
    /// control claimed), at this document point: the app opens its pasteboard menu there.
    pub context_at: Option<Point>,
}

impl Outcome for PasteboardOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The pasteboard and page frames under a document. Show it first in the canvas body, then the document, then the
/// overlays. The canvas must be given a view; without one nothing is shown.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct Pasteboard<'a> {
    #[common]
    common: Common,
    pages: &'a [PageFrame],
    /// The pasteboard's colour; the theme's canvas surface by default.
    #[prop]
    color: PasteboardColor,
    /// The border round each page; a drop shadow by default.
    #[prop]
    border: PageBorder,
    /// The size of each page's name, measured in `parts`; zero for a page without one.
    name_sizes: Vec<Vec2>,
    name_clicked: Option<usize>,
    context_at: Option<Point>,
}

impl<'a> Pasteboard<'a> {
    /// A pasteboard named `label` around `pages`. A single-image document passes one page: its bounds.
    pub fn new(label: impl Into<Label>, pages: &'a [PageFrame]) -> Self {
        Pasteboard {
            common: Common::new(),
            pages,
            color: PasteboardColor::Default,
            border: PageBorder::DropShadow,
            name_sizes: Vec::new(),
            name_clicked: None,
            context_at: None,
        }
        .label(label)
    }

    /// The pasteboard as screen quads: the canvas minus the pages, each piece as its four corners (a turned view needs
    /// quads, not rects). For an app that draws the pasteboard itself.
    pub fn quads(view: &CanvasView, pages: &[PageFrame]) -> Vec<[Pos2; 4]> {
        let canvas = view.visible();
        if !(canvas.x1 > canvas.x0 && canvas.y1 > canvas.y0 && canvas.area().is_finite()) {
            return Vec::new();
        }
        let boards: Vec<DocRect> = pages.iter().filter_map(|page| clip(page.rect.abs(), canvas)).collect();
        let mut xs = vec![canvas.x0, canvas.x1];
        for board in &boards {
            xs.extend([board.x0, board.x1]);
        }
        xs.sort_by(f64::total_cmp);
        xs.dedup();
        let mut out = Vec::new();
        for pair in xs.windows(2) {
            let (x0, x1) = (pair[0], pair[1]);
            let mut spans: Vec<(f64, f64)> =
                boards.iter().filter(|board| board.x0 <= x0 && board.x1 >= x1).map(|board| (board.y0, board.y1)).collect();
            spans.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut y = canvas.y0;
            for (top, bottom) in spans {
                if top > y {
                    out.push(view.quad(DocRect::new(x0, y, x1, top)));
                }
                y = y.max(bottom);
            }
            if y < canvas.y1 {
                out.push(view.quad(DocRect::new(x0, y, x1, canvas.y1)));
            }
        }
        out
    }

    /// The page under the document point `at`, if any; of overlapping pages the last one.
    pub fn page_at(pages: &[PageFrame], at: Point) -> Option<usize> {
        pages.iter().rposition(|page| page.rect.abs().contains(at))
    }
}

/// `rect` clipped to `canvas`, or `None` when they share no area.
fn clip(rect: DocRect, canvas: DocRect) -> Option<DocRect> {
    let clipped = DocRect::new(rect.x0.max(canvas.x0), rect.y0.max(canvas.y0), rect.x1.min(canvas.x1), rect.y1.min(canvas.y1));
    (clipped.x1 > clipped.x0 && clipped.y1 > clipped.y0).then_some(clipped)
}

fn bounds(quad: &[Pos2; 4]) -> Rect {
    Rect::from_points(quad)
}

/// The run a page name is shown and measured with.
fn name_run<'n>(name: &'n str, color: StyleColor) -> TextRun<'n> {
    TextRun::new(name).role(TypeRole::Caption).color(color)
}

fn name_color(style: &PasteboardStyle, active: Option<bool>) -> StyleColor {
    if active == Some(true) { style.name_active } else { style.name }
}

/// The rect of a page's name: above its top-left corner on screen, `size` big.
fn name_rect(view: &CanvasView, page: &PageFrame, gap: f32, size: Vec2) -> Rect {
    let corner = view.to_screen(Point::new(page.rect.x0.min(page.rect.x1), page.rect.y0.min(page.rect.y1)));
    Rect::from_min_size(Pos2::new(corner.x, corner.y - gap - size.y), size)
}

impl Component for Pasteboard<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    type Style = PasteboardStyle;
    type Output = PasteboardOutput;

    fn style(cx: &StyleCx<'_>, _: PasteboardStyleKey) -> PasteboardStyle {
        let tokens = cx.tokens;
        let (color, shadow) = (&tokens.color, tokens.elevation.floating);
        let alpha = (shadow.alpha * f32::from(u8::MAX)).round().clamp(0.0, f32::from(u8::MAX)) as u8;
        PasteboardStyle {
            metrics: Metrics::new(),
            fill: cx.color(color.surface.canvas),
            shadow: cx.color(Rgba([0, 0, 0, alpha])),
            shadow_offset: Vec2::new(shadow.offset[0], shadow.offset[1]),
            outline: cx.color(color.line.border),
            active_edge: cx.color(color.accent.solid),
            edge: tokens.stroke.thin,
            name: cx.color(color.text.muted),
            name_active: cx.color(color.text.normal),
            name_gap: tokens.space.gap.inline,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// The page names: a click away from the tool. They have no keyboard path of their own; the app's artboard list
    /// selects the same page.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        self.name_sizes = self
            .pages
            .iter()
            .map(|page| page.name.as_ref().map_or(Vec2::ZERO, |name| cx.layout(&name_run(name, name_color(cx.style, page.active))).size()))
            .collect();
        for (at, page) in self.pages.iter().enumerate() {
            if page.name.is_none() {
                continue;
            }
            cx.part(NAME.at(at as u64), name_rect(&space.view, page, cx.style.name_gap, self.name_sizes[at]))
                .sense(Sense::CLICK)
                .priority(HitPriority::new(HitLevel::Body, 0))
                .pointer_only("the application's artboard list selects the same page");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        for (at, page) in self.pages.iter().enumerate() {
            if page.name.is_none() {
                continue;
            }
            if cx.part_activated(NAME.at(at as u64)).is_some() {
                self.name_clicked = Some(at);
            }
        }
        if let Some(pos) = space.background().context {
            let doc = space.view.to_doc(pos);
            if Self::page_at(self.pages, doc).is_none() {
                self.context_at = Some(doc);
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let (view, style) = (space.view, cx.style);
        let draw = ui.draw();
        let polygon = |quad: &[Pos2; 4]| PaintShape::Polygon(quad.to_vec());
        for quad in Self::quads(&view, self.pages).iter().filter(|quad| bounds(quad).intersects(view.rect)) {
            match self.color.fill() {
                Some(color) => draw.data_fill(&polygon(quad), color),
                None => draw.fill(&polygon(quad), style.fill),
            }
        }
        let edges = ui.content_painter(cx);
        let aligned = view.axis_aligned();
        let edge = |quad: &[Pos2; 4], color: StyleColor| {
            if aligned {
                let rect = bounds(quad);
                let look = RuleLook { color: color.egui(), width: style.edge };
                let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
                for (from, to) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
                    edges.painter.extend(Rule::shapes(&look, from, to, edges.pixels_per_point()));
                }
            } else {
                draw.stroke(&polygon(quad), StrokeWidth::Thin, color);
            }
        };
        for (at, page) in self.pages.iter().enumerate() {
            let quad = view.quad(page.rect);
            let named = page.name.is_some();
            if !bounds(&quad).intersects(view.rect) && !named {
                continue;
            }
            if self.border == PageBorder::DropShadow {
                draw.fill(&polygon(&quad.map(|corner| corner + style.shadow_offset)), style.shadow);
            }
            if let Some(color) = page.fill {
                draw.data_fill(&polygon(&quad), color);
            }
            if self.border == PageBorder::Line {
                edge(&quad, style.outline);
            }
            if let Some(active) = page.active {
                edge(&quad, if active { style.active_edge } else { style.outline });
            }
            if let Some(name) = &page.name {
                let run = name_run(name, name_color(style, page.active));
                let rect = name_rect(&view, page, style.name_gap, self.name_sizes.get(at).copied().unwrap_or(Vec2::ZERO));
                if rect.intersects(view.rect) {
                    draw.text(rect, &run);
                }
            }
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        for (at, page) in self.pages.iter().enumerate() {
            if let Some(name) = &page.name {
                access.node(NAME.at(at as u64), Role::BUTTON).name = Name::Text(name.clone());
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PasteboardOutput {
        PasteboardOutput { response: cx.response.clone(), name_clicked: self.name_clicked, context_at: self.context_at }
    }
}
