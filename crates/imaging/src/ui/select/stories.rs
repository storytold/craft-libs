//! Stories of the selection tools: one per tool, run live on a canvas over the mock picture by the tool story host,
//! as an app runs them. The events each tool reports are applied to the story selection: the shapes the tools
//! committed, combined in order by their operations (so Shift adds and Alt subtracts) and moved by the selection
//! drags, which the selection overlay shows. It starts as a rectangle round the picture's house, so the first press
//! inside it drags it. The Magnetic Lasso's tracer follows the house's outline where its contrast is strong enough.

use craft_behavior::geometry::{Constraint, SelectionOperation};
use craft_behavior::kurbo::{Point, Rect, Vec2};
use craft_behavior::{CanvasView, EditEvent};
use craft_ui::canvas::RasterSource as _;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryHost, StoryOutput, ToolStory, story, tool_story};
use egui::Ui;

use super::{
    EdgeSearch, EdgeTracer, LassoTool, MagneticLassoTool, MarqueeKind, MarqueeTool, PolygonLassoTool, SelectEvent, SelectPhase, SelectToolPass,
    SelectionProbe, SelectionShape,
};
use crate::trace::{MaskRect, MaskSource};
use crate::ui::mock_image::MockImage;
use crate::ui::selection::SelectionOverlay;

craft_ui::words! {
    /// The captions and the status line of the selection tool stories.
    #[non_exhaustive]
    pub(crate) struct SelectToolStoryWords {
        /// The name of the selection overlay.
        selection: "selectToolStory.selection" = "Selection";
        /// The status line under the picture: the tool's phase and the last event it reported.
        status: template "selectToolStory.status" = "{phase}. Last event: {event}.";
        /// Nothing under way.
        idle: "selectToolStory.idle" = "Idle";
        /// A marquee being dragged.
        marquee: "selectToolStory.marquee" = "Dragging a marquee";
        /// An outline being drawn.
        outline: "selectToolStory.outline" = "Drawing an outline";
        /// The selection being dragged.
        dragging: "selectToolStory.dragging" = "Dragging the selection";
        /// No event so far.
        no_event: "selectToolStory.noEvent" = "none yet";
        /// A shape was selected.
        selected: "selectToolStory.selected" = "Selected";
        /// A click deselected.
        deselected: "selectToolStory.deselected" = "Deselected";
        /// The selection was dragged.
        dragged: "selectToolStory.dragged" = "Dragged the selection";
    }
}

/// A selection tool a story runs: its pass for the host, its events and its phase.
trait Shown: 'static {
    fn run(&mut self, host: &StoryHost<'_>, probe: &dyn SelectionProbe);
    fn take(&mut self) -> Vec<SelectEvent>;
    fn phase(&self) -> SelectPhase;
}

macro_rules! shown {
    ($tool:ty $(, $param:ident: $bound:path)?) => {
        impl$(<$param: $bound + 'static>)? Shown for $tool {
            fn run(&mut self, host: &StoryHost<'_>, probe: &dyn SelectionProbe) {
                let _ = host.run(&mut SelectToolPass::new(self, probe));
            }

            fn take(&mut self) -> Vec<SelectEvent> {
                self.take_events()
            }

            fn phase(&self) -> SelectPhase {
                <$tool>::phase(self)
            }
        }
    };
}

shown!(MarqueeTool);
shown!(LassoTool);
shown!(PolygonLassoTool);
shown!(MagneticLassoTool<T>, T: EdgeTracer);

/// What a story keeps of its tool: the tool and the kind of the last event it reported.
struct Kept<T> {
    tool: T,
    last: Option<Last>,
}

/// The kind of event the status line names.
#[derive(Clone, Copy, Debug)]
enum Last {
    Selected,
    Deselected,
    Dragged,
}

/// The story selection: shapes combined in order by their operations, then moved by `offset`. It is the tools'
/// [`SelectionProbe`] and the overlay's [`MaskSource`].
#[derive(Clone, Debug, PartialEq)]
struct Selection {
    size: [u32; 2],
    parts: Vec<(SelectionOperation, SelectionShape)>,
    offset: Vec2,
    revision: u64,
}

impl Selection {
    /// A rectangle round the picture's house.
    fn house(image: &MockImage) -> Self {
        let b = image.subject_bounds();
        let rect = Rect::new(f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1));
        let size = image.size();
        Selection {
            size: [size.width, size.height],
            parts: vec![(SelectionOperation::New, SelectionShape::Rectangle(rect))],
            offset: Vec2::ZERO,
            revision: 1,
        }
    }

    /// Applies what a tool reported, as an app does; returns the kind of the last event.
    fn apply(&mut self, events: Vec<SelectEvent>) -> Option<Last> {
        let mut last = None;
        for event in events {
            match event {
                SelectEvent::Commit { shape, operation } => {
                    if operation == SelectionOperation::New {
                        self.parts.clear();
                        self.offset = Vec2::ZERO;
                    }
                    self.parts.push((operation, shape));
                    last = Some(Last::Selected);
                }
                SelectEvent::Deselect => {
                    self.parts.clear();
                    self.offset = Vec2::ZERO;
                    last = Some(Last::Deselected);
                }
                SelectEvent::DragSelection(EditEvent::Commit { after, .. }) => {
                    self.offset += Vec2::new(f64::from(after.delta[0]), f64::from(after.delta[1]));
                    last = Some(Last::Dragged);
                }
                SelectEvent::DragSelection(_) => continue,
            }
            self.revision += 1;
        }
        last
    }

    /// The coverage at the document point `p`: 0 or 1, the hard coverage a marquee or an outline gives.
    fn coverage(&self, p: Point) -> f32 {
        let p = p - self.offset;
        self.parts.iter().fold(0.0, |acc, (operation, shape)| {
            let c = f32::from(inside(shape, p));
            match operation {
                SelectionOperation::Add => acc.max(c),
                SelectionOperation::Subtract => acc * (1.0 - c),
                SelectionOperation::Intersect => acc.min(c),
                _ => c,
            }
        })
    }
}

/// Whether `p` is inside `shape`: an outline by the even-odd rule.
fn inside(shape: &SelectionShape, p: Point) -> bool {
    match shape {
        SelectionShape::Rectangle(r) => r.x0 <= p.x && p.x < r.x1 && r.y0 <= p.y && p.y < r.y1,
        SelectionShape::Ellipse(r) => {
            let (c, rx, ry) = (r.center(), r.width() / 2.0, r.height() / 2.0);
            rx > 0.0 && ry > 0.0 && ((p.x - c.x) / rx).powi(2) + ((p.y - c.y) / ry).powi(2) <= 1.0
        }
        SelectionShape::Polygon(points) => {
            let mut inside = false;
            for (i, a) in points.iter().enumerate() {
                let b = points[(i + 1) % points.len()];
                if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                    inside = !inside;
                }
            }
            inside
        }
    }
}

/// The document rectangle `shape` covers.
fn bounds(shape: &SelectionShape) -> Rect {
    match shape {
        SelectionShape::Rectangle(r) | SelectionShape::Ellipse(r) => *r,
        SelectionShape::Polygon(points) => {
            let first = points.first().copied().unwrap_or(Point::ZERO);
            points.iter().fold(Rect::from_points(first, first), |rect, &p| rect.union_pt(p))
        }
    }
}

impl MaskSource for Selection {
    fn size(&self) -> [u32; 2] {
        self.size
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn content_bounds(&self) -> MaskRect {
        let grown = self.parts.iter().filter(|(operation, _)| !matches!(operation, SelectionOperation::Subtract | SelectionOperation::Intersect));
        match grown.map(|(_, shape)| bounds(shape) + self.offset).reduce(|a, b| a.union(b)) {
            Some(r) => MaskRect::new(r.x0.floor() as i32, r.y0.floor() as i32, r.x1.ceil() as i32, r.y1.ceil() as i32),
            None => MaskRect::new(0, 0, 0, 0),
        }
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        for (index, value) in out.iter_mut().enumerate() {
            let x = f64::from(x0) + (index as f64) * f64::from(step);
            *value = self.coverage(Point::new(x + 0.5, f64::from(y) + 0.5));
        }
    }
}

impl SelectionProbe for Selection {
    fn document(&self) -> u64 {
        1
    }

    fn size(&self) -> craft_behavior::PixelSize {
        craft_behavior::PixelSize::new(self.size[0], self.size[1])
    }

    fn has_selection(&self) -> bool {
        !self.parts.is_empty()
    }

    fn inside_selection(&self, at: Point) -> bool {
        self.coverage(at) > 0.0
    }
}

/// An [`EdgeTracer`] over the picture: the edges are the house's outline, and an edge counts where the brightness
/// across it differs by at least the search's contrast.
struct PictureEdges {
    image: MockImage,
}

impl PictureEdges {
    /// How far either side of the outline the brightness is compared, as a share of the picture's width.
    const PROBE: f64 = 0.002;

    fn luma(&self, p: Point) -> f64 {
        let [r, g, b, _] = self.image.pixel(p.x, p.y);
        (0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)) / 255.0
    }

    /// The outline's unit normal at `p`, pointing out of the house.
    fn normal(&self, p: Point) -> Vec2 {
        let d = |dx: f64, dy: f64| self.image.subject_distance(p.x + dx, p.y + dy);
        let g = Vec2::new(d(1.0, 0.0) - d(-1.0, 0.0), d(0.0, 1.0) - d(0.0, -1.0));
        if g.hypot() > 0.0 { g / g.hypot() } else { Vec2::new(0.0, -1.0) }
    }

    /// The point of the outline nearest `p`; `None` beyond the search's width or where the edge is weaker than its
    /// contrast.
    fn edge_near(&self, p: Point, search: EdgeSearch) -> Option<Point> {
        let mut q = p;
        for _ in 0..4 {
            q -= self.normal(q) * self.image.subject_distance(q.x, q.y);
        }
        let reach = Self::PROBE * f64::from(self.image.size().width);
        let n = self.normal(q);
        let contrast = (self.luma(q + n * reach) - self.luma(q - n * reach)).abs();
        ((q - p).hypot() <= search.width && contrast >= search.contrast).then_some(q)
    }
}

impl EdgeTracer for PictureEdges {
    fn snap(&mut self, at: Point, search: EdgeSearch) -> Option<Point> {
        self.edge_near(at, search)
    }

    fn trace(&mut self, from: Point, to: Point, _: &[Point], search: EdgeSearch) -> Vec<Point> {
        // Walks the straight line a pixel at a time and pulls each sample onto the edge.
        let steps = from.distance(to).ceil().max(1.0) as usize;
        let mut path = vec![from];
        for i in 1..steps {
            let sample = from.lerp(to, i as f64 / steps as f64);
            let point = self.edge_near(sample, search).unwrap_or(sample);
            if path.last().is_none_or(|last| last.distance(point) >= 1.0) {
                path.push(point);
            }
        }
        path.push(to);
        path
    }
}

/// The status line: the tool's phase and the last event.
fn status(words: &SelectToolStoryWords, phase: SelectPhase, last: Option<Last>) -> String {
    let phase = match phase {
        SelectPhase::Marquee { .. } => &words.marquee,
        SelectPhase::Outline { .. } => &words.outline,
        SelectPhase::Dragging(_) => &words.dragging,
        _ => &words.idle,
    };
    let event = match last {
        None => &words.no_event,
        Some(Last::Selected) => &words.selected,
        Some(Last::Deselected) => &words.deselected,
        Some(Last::Dragged) => &words.dragged,
    };
    craft_i18n::fmt(words.status.as_str(), &[("phase", phase.as_str()), ("event", event.as_str())])
}

/// Runs the tool `init` makes, set up each pass by `options`, live over the mock picture, with the story selection
/// drawn by the overlay.
fn on_canvas<T: Shown>(ui: &mut Ui, data: &mut StoryData, init: impl FnOnce() -> T, options: impl FnOnce(&mut T)) -> StoryOutput {
    let image = MockImage::default();
    let words = Env::of(ui).words::<SelectToolStoryWords>();
    let overlay = |ui: &mut Ui, at: egui::Rect, _: &CanvasView, _: &mut Kept<T>, selection: &Selection| {
        let _ = SelectionOverlay::new(words.selection.clone(), selection).show_at(ui, at);
    };
    let story = ToolStory::new("Document", &image).fit();
    tool_story(
        ui,
        data,
        story,
        || Kept { tool: init(), last: None },
        Selection::house(&image),
        overlay,
        |run| {
            options(&mut run.tool.tool);
            run.tool.tool.run(&run.host, &*run.document);
            if let Some(last) = run.document.apply(run.tool.tool.take()) {
                run.tool.last = Some(last);
            }
            StoryOutput::default().readout(status(&words, run.tool.tool.phase(), run.tool.last))
        },
    )
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn marquee(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let kinds = [
        ("Rectangular", MarqueeKind::Rectangle),
        ("Elliptical", MarqueeKind::Ellipse),
        ("Single Row", MarqueeKind::SingleRow),
        ("Single Column", MarqueeKind::SingleColumn),
    ];
    let kind = *data.choice("kind", MarqueeKind::Rectangle, &kinds);
    let styles = [("Normal", Constraint::Free), ("Fixed Ratio", Constraint::Ratio(4.0, 3.0)), ("Fixed Size", Constraint::Size(400.0, 300.0))];
    let style = *data.choice("style", Constraint::Free, &styles);
    on_canvas(
        ui,
        data,
        || MarqueeTool::new(kind),
        |tool| {
            tool.kind(kind).constraint(style);
        },
    )
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn lasso(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, LassoTool::new, |_| {})
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn polygonal_lasso(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, PolygonLassoTool::new, |_| {})
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn magnetic_lasso(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let width = *data.ranged("width", 10.0_f32, 1.0..=256.0);
    let contrast = *data.ranged("contrast", 10.0_f32, 1.0..=100.0);
    let frequency = *data.ranged("frequency", 57.0_f32, 0.0..=100.0);
    on_canvas(
        ui,
        data,
        || MagneticLassoTool::new(PictureEdges { image: MockImage::default() }),
        |tool| {
            let settings = tool.settings_mut();
            settings.width = width;
            settings.contrast = contrast;
            settings.frequency = frequency;
        },
    )
}
