//! Stories of `Marquee`, and of `StraightLinePreview` under a live stroke tool: a drag paints a stroke that Shift
//! holds to 0, 45 or 90 degrees with `StrokeLock`, and Shift shows the straight line a click draws from the last
//! stroke's end.

use super::{Marquee, MarqueeLook};
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas};
use crate::canvas::testing::tool_story::{ToolStory, tool_story};
use crate::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use crate::canvas_controls::StraightLinePreview;
use crate::contract::Show as _;
use crate::primitives::{Draw, PaintShape, StrokeWidth};
use crate::style::DataColor;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::geometry::{ConstrainStyle, StrokeLock};
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CancelReason, CanvasView, Cursor, CursorIcon};
use egui::{Color32, Ui};

fn selection() -> Rect {
    Rect::new(240.0, 145.0, 400.0, 255.0)
}

fn lasso() -> Vec<Point> {
    vec![Point::new(240.0, 175.0), Point::new(320.0, 135.0), Point::new(390.0, 205.0), Point::new(310.0, 265.0)]
}

#[story(Marquee, covers = [Default], drive = Drive::Never)]
fn ants(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |ui, at, _| Marquee::rect("Selection", selection()).show_at(ui, at))
}

#[story(Marquee, covers = [Default], drive = Drive::Never)]
fn elliptical_ants(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |ui, at, _| Marquee::ellipse("Selection", selection()).show_at(ui, at))
}

#[story(Marquee, covers = [Default], drive = Drive::Never)]
fn band(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |ui, at, _| Marquee::polygon("Selection", lasso()).look(MarqueeLook::Band).show_at(ui, at))
}

#[story(Marquee, covers = [Default], drive = Drive::Never)]
fn drawing(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |ui, at, _| Marquee::polygon("Lasso", lasso()).look(MarqueeLook::Drawing).show_at(ui, at))
}

#[story(Marquee, covers = [Default], drive = Drive::Never)]
fn contrast(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let color = DataColor::new(Color32::from_rgb(230, 70, 70));
    on_canvas(ui, |ui, at, _| Marquee::rect("Shape", selection()).look(MarqueeLook::Contrast(color)).show_at(ui, at))
}

#[story(StraightLinePreview, covers = [Default], drive = Drive::Never)]
fn shift_click_line(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |ui, at, _| StraightLinePreview::new("Straight line", Point::new(240.0, 235.0), Point::new(400.0, 155.0)).show_at(ui, at))
}

/// The strokes the stroke story painted, each its points in document units.
type Strokes = Vec<Vec<Point>>;

/// The stroke tool the story keeps: the stroke being painted with its Shift lock, and the pointer and Shift between
/// strokes.
#[derive(Default)]
struct StrokeTool {
    lock: Option<StrokeLock>,
    stroke: Vec<Point>,
    pointer: Option<Point>,
    shift: bool,
}

impl StrokeTool {
    /// The straight line a Shift-click at the pointer draws: from the last stroke's end.
    fn straight_line(&self, strokes: &Strokes) -> Option<(Point, Point)> {
        let end = strokes.last().and_then(|stroke| stroke.last()).copied()?;
        self.pointer.filter(|_| self.shift && self.lock.is_none()).map(|at| (end, at))
    }
}

/// The stroke tool for one pass, painting into the story's strokes.
struct StrokePass<'a> {
    tool: &'a mut StrokeTool,
    strokes: &'a mut Strokes,
}

impl CanvasTool for StrokePass<'_> {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        self.tool.shift = cx.mods.shift;
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        let at = space.view.to_doc(pointer.pos);
        if pointer.mods.shift
            && let Some(end) = self.strokes.last().and_then(|stroke| stroke.last()).copied()
        {
            self.strokes.push(vec![end, at]);
            return ToolAction::Claimed;
        }
        self.tool.lock = Some(StrokeLock::new(&ConstrainStyle::new(f64::from(cx.slop), f64::from(cx.slop))));
        self.tool.stroke = vec![at];
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (Some(space), Some(lock), Some(&last)) = (cx.space(), self.tool.lock.as_mut(), self.tool.stroke.last()) else { return };
        let at = lock.constrain(last, space.view.to_doc(pointer.pos), pointer.mods.shift, space.view.zoom);
        if at != last {
            self.tool.stroke.push(at);
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.drag(pointer, cx);
        if self.tool.lock.take().is_some() && self.tool.stroke.len() > 1 {
            self.strokes.push(std::mem::take(&mut self.tool.stroke));
        }
        self.tool.stroke.clear();
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.tool.pointer = cx.space().map(|space| space.view.to_doc(pointer.pos));
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.tool.lock = None;
        self.tool.stroke.clear();
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let (Some(space), Some(style)) = (cx.space(), cx.style()) else { return };
        let color = style.color(style.tokens.color.accent.solid);
        for stroke in self.strokes.iter().chain(std::iter::once(&self.tool.stroke)).filter(|stroke| stroke.len() > 1) {
            draw.stroke(&PaintShape::Path(stroke.iter().map(|&p| space.view.to_screen(p)).collect()), StrokeWidth::Thick, color);
        }
    }

    fn cursor(&self, _: &ToolContext<'_>) -> Cursor {
        Cursor::System(CursorIcon::Crosshair)
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("a stroke follows the pointer")
    }
}

#[story(StraightLinePreview, covers = [], drive = Drive::Never)]
fn strokes(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let document = StoryDocument::STANDARD;
    let body = |ui: &mut Ui, at: egui::Rect, _: &CanvasView, tool: &mut StrokeTool, strokes: &Strokes| {
        if let Some((from, to)) = tool.straight_line(strokes) {
            let _ = StraightLinePreview::new("Straight line", from, to).show_at(ui, at);
        }
    };
    tool_story(ui, data, ToolStory::new("Document", &document), StrokeTool::default, Strokes::new(), body, |run| {
        let _ = run.host.run(&mut StrokePass { tool: &mut *run.tool, strokes: &mut *run.document });
        StoryOutput::default()
    })
}
