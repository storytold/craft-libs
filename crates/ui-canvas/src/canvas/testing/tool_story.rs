//! The story host of canvas tools: a tool is no component, so its story runs it live on a canvas over a document the
//! story keeps, as an app runs it.

use crate::canvas::{Canvas, CanvasOutput, CanvasTool, RasterSource, ToolHost, ToolHostResponse};
use crate::contract::ShowContainer as _;
use crate::testing::{StoryData, StoryOutput, stage};
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CanvasView, Value};
use egui::{Rect, Ui};
use std::cell::RefCell;
use std::rc::Rc;

/// The canvas a tool story shows: its name, the picture under the document, and the view of it.
pub struct ToolStory<'a> {
    label: &'static str,
    picture: &'a dyn RasterSource,
    view: StoryZoom,
    /// The view's rotation in degrees, its mirroring and its pixel aspect.
    turn: (f64, bool, f64),
}

/// How a tool story's view shows its picture.
#[derive(Clone, Copy, Debug)]
enum StoryZoom {
    /// The document point at the canvas's middle, at a percent ([`story_view`](super::canvas_story::story_view)).
    At(Point, f64),
    /// The whole picture fits the canvas, centred.
    Fit,
}

impl<'a> ToolStory<'a> {
    /// A canvas named `label` over `picture` (shown on the story document's ground), at 100 % centred on the picture.
    pub fn new(label: &'static str, picture: &'a dyn RasterSource) -> Self {
        let size = picture.size();
        let centre = Point::new(f64::from(size.width) / 2.0, f64::from(size.height) / 2.0);
        ToolStory { label, picture, view: StoryZoom::At(centre, 100.0), turn: (0.0, false, 1.0) }
    }

    /// The same canvas with the document point `centre` at its middle at `percent`
    /// ([`story_view`](super::canvas_story::story_view)).
    #[must_use]
    pub fn view(mut self, centre: Point, percent: f64) -> Self {
        self.view = StoryZoom::At(centre, percent);
        self
    }

    /// The same canvas with the whole picture fitted to it, centred.
    #[must_use]
    pub fn fit(mut self) -> Self {
        self.view = StoryZoom::Fit;
        self
    }

    /// The same canvas with its view turned `rotation` degrees clockwise, mirrored when `flip_x`, with the pixel
    /// aspect `aspect`.
    #[must_use]
    pub fn turned(mut self, rotation: f64, flip_x: bool, aspect: f64) -> Self {
        self.turn = (rotation, flip_x, aspect);
        self
    }

    fn view_of(&self, room: Rect, pixels_per_point: f32) -> CanvasView {
        let size = self.picture.size();
        let (width, height) = (f64::from(size.width.max(1)), f64::from(size.height.max(1)));
        let (centre, percent) = match self.view {
            StoryZoom::At(centre, percent) => (centre, percent),
            StoryZoom::Fit => {
                let points_per_pixel = (f64::from(room.width()) / width).min(f64::from(room.height()) / height);
                (Point::new(width / 2.0, height / 2.0), points_per_pixel * f64::from(pixels_per_point) * 100.0)
            }
        };
        let mut view = super::canvas_story::story_view(room, pixels_per_point, centre, percent);
        (view.rotation, view.flip_x, view.aspect) = self.turn;
        view
    }
}

/// One pass of a tool story, after the canvas showed: the kept tool, the kept document, what the body's controls
/// reported, and the host that runs a tool on this pass's canvas.
#[non_exhaustive]
pub struct ToolRun<'a, T, D, R> {
    /// The tool, kept across passes.
    pub tool: &'a mut T,
    /// The document the tool edits, kept across passes as the story's value `"document"`.
    pub document: &'a mut D,
    /// What the body returned in the drawing pass: the outputs of the document's canvas controls.
    pub shown: Option<R>,
    /// The view the canvas showed.
    pub view: CanvasView,
    /// Runs a tool on the canvas.
    pub host: StoryHost<'a>,
}

/// The tool host of a tool story's canvas for one pass.
pub struct StoryHost<'a> {
    ui: &'a Ui,
    canvas: &'a CanvasOutput,
}

impl StoryHost<'_> {
    /// Runs `tool` as the canvas's active tool, as an app's [`ToolHost::run`] does.
    pub fn run(&self, tool: &mut dyn CanvasTool) -> ToolHostResponse {
        ToolHost::new().run(self.ui, self.canvas, tool)
    }
}

/// Shows a tool live on the canvas `story` over the stage of `ui`. The tool `tool` makes is kept as a note of `data`
/// (a note: Reset and the runner's drives compare values, and a tool's state in a gesture is no value); `document`
/// is kept as the value `"document"`, so Reset puts it back. Each pass `body` shows the document's canvas controls at
/// the canvas rect from the tool and the document (a tool whose pass shows its own controls shows them here), and `run` runs the tool through [`ToolRun::host`] and applies what it reported
/// to the document, as the app does, returning the story's output with its readout.
pub fn tool_story<T: 'static, D: Value, R>(
    ui: &mut Ui,
    data: &mut StoryData,
    story: ToolStory<'_>,
    tool: impl FnOnce() -> T,
    document: D,
    mut body: impl FnMut(&mut Ui, Rect, &CanvasView, &mut T, &D) -> R,
    run: impl FnOnce(ToolRun<'_, T, D, R>) -> StoryOutput,
) -> StoryOutput {
    let kept = Rc::clone(data.note::<Option<Rc<RefCell<T>>>>("tool", None).get_or_insert_with(|| Rc::new(RefCell::new(tool()))));
    let document = data.value("document", document);
    let room = stage(ui);
    let view = story.view_of(room, ui.pixels_per_point());
    let size = story.picture.size();
    let canvas = Canvas::new(story.label).document(Size::new(f64::from(size.width), f64::from(size.height))).view(view);
    let shown = canvas.show_at(ui, room, |ui, body_cx| {
        super::canvas_story::show_document(ui, body_cx.rect, story.picture);
        body(ui, body_cx.rect, &view, &mut kept.borrow_mut(), document)
    });
    let mut tool = kept.borrow_mut();
    run(ToolRun { tool: &mut tool, document, shown: shown.inner, view, host: StoryHost { ui, canvas: &shown.output } })
}
