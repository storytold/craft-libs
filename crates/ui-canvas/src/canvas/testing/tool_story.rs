//! The story host of canvas tools: a tool is no component, so its story runs it live on a canvas over a document the
//! story keeps, as an app runs it.

use crate::buttons::Button;
use crate::canvas::{Canvas, CanvasOutput, CanvasTool, RasterSource, ToolHost, ToolHostResponse, ToolInput};
use crate::contract::{BodyCx, Gap, Show as _, ShowContainer as _, clicked};
use crate::layout_components::{Row, Stack};
use crate::runtime::{frame, pointer};
use crate::testing::{StoryData, StoryOutput, stage};
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CanvasView, PointerButton, Value};
use craft_keymap::Mods;
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
    /// The gesture the story's Play button sends.
    play: Option<ToolScript>,
    /// The story's own buttons under the canvas, after Play.
    buttons: &'a [&'static str],
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
        ToolStory { label, picture, view: StoryZoom::At(centre, 100.0), turn: (0.0, false, 1.0), play: None, buttons: &[] }
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

    /// The same canvas with a Play button under it that sends `script` to the tool as a hand would, through
    /// [`ToolHost::inject`].
    #[must_use]
    pub fn play(mut self, script: ToolScript) -> Self {
        self.play = Some(script);
        self
    }

    /// The same canvas with the story's own buttons `labels` in the row under it (an app's Apply, Undo); the one
    /// pressed reaches `run` as [`ToolRun::pressed`].
    #[must_use]
    pub fn buttons(mut self, labels: &'a [&'static str]) -> Self {
        self.buttons = labels;
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

/// A gesture a tool story's Play button sends, in document points, spread over its duration.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolScript {
    steps: Vec<ScriptStep>,
    seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ScriptStep {
    Down(PointerButton, Point, Mods),
    Move(Point, Mods),
    Up(Point, Mods),
}

/// The moves a stroke sends along each of its segments.
const MOVES_PER_SEGMENT: usize = 12;

impl ToolScript {
    /// An empty script that takes `seconds` to play.
    pub fn new(seconds: f64) -> Self {
        ToolScript { steps: Vec::new(), seconds }
    }

    /// Adds a press of `button` at the first point of `path` with `press` held, moves along it and the release at its
    /// last point with `held` held (a marquee's Shift after the press).
    #[must_use]
    pub fn stroke(self, button: PointerButton, path: &[Point], press: Mods, held: Mods) -> Self {
        let (Some(&first), Some(&last)) = (path.first(), path.last()) else { return self };
        self.press(button, first, press).moves(&path[1..], held).release(last, held)
    }

    /// Adds a press of `button` at `at` with `mods` held.
    #[must_use]
    pub fn press(mut self, button: PointerButton, at: Point, mods: Mods) -> Self {
        self.steps.push(ScriptStep::Down(button, at, mods));
        self
    }

    /// Adds moves from the script's last point through each point of `path`, in even steps, with `mods` held.
    #[must_use]
    pub fn moves(mut self, path: &[Point], mods: Mods) -> Self {
        for &to in path {
            let Some(from) = self.last_point() else {
                self.steps.push(ScriptStep::Move(to, mods));
                continue;
            };
            for step in 1..=MOVES_PER_SEGMENT {
                self.steps.push(ScriptStep::Move(from.lerp(to, step as f64 / MOVES_PER_SEGMENT as f64), mods));
            }
        }
        self
    }

    /// Adds the release at `at` with `mods` held.
    #[must_use]
    pub fn release(mut self, at: Point, mods: Mods) -> Self {
        self.steps.push(ScriptStep::Up(at, mods));
        self
    }

    /// Adds a click at `at` with `mods` held.
    #[must_use]
    pub fn click(self, at: Point, mods: Mods) -> Self {
        self.press(PointerButton::Primary, at, mods).release(at, mods)
    }

    fn last_point(&self) -> Option<Point> {
        self.steps.last().map(|step| match *step {
            ScriptStep::Down(_, at, _) | ScriptStep::Move(at, _) | ScriptStep::Up(at, _) => at,
        })
    }

    fn input(step: ScriptStep, view: &CanvasView) -> ToolInput {
        match step {
            ScriptStep::Down(button, at, mods) => ToolInput::down(button, view.to_screen(at)).mods(mods),
            ScriptStep::Move(at, mods) => ToolInput::moved(view.to_screen(at)).mods(mods),
            ScriptStep::Up(at, mods) => ToolInput::up(view.to_screen(at)).mods(mods),
        }
    }
}

/// A script that is playing: when it began and how many steps it has sent.
#[derive(Clone, Copy, Debug)]
struct Playing {
    start: f64,
    sent: usize,
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
    /// The story button ([`ToolStory::buttons`]) pressed this pass.
    pub pressed: Option<&'static str>,
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
/// to the document, as the app does, returning the story's output with its readout. The story's buttons show in a row
/// under the canvas: with [`ToolStory::play`] first a Play button, which sends the script's steps as they fall due,
/// one run at a time.
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
    let mut playing = *data.note::<Option<Playing>>("play", None);
    let document = data.value("document", document);
    let room = stage(ui);
    let pixels_per_point = ui.pixels_per_point();
    let size = story.picture.size();
    let mut canvas = |ui: &mut Ui, rect: Rect| {
        let view = story.view_of(rect, pixels_per_point);
        let canvas = Canvas::new(story.label).document(Size::new(f64::from(size.width), f64::from(size.height))).view(view);
        let shown = canvas.show_at(ui, rect, |ui, body_cx| {
            super::canvas_story::show_document(ui, body_cx.rect, story.picture);
            body(ui, body_cx.rect, &view, &mut kept.borrow_mut(), &*document)
        });
        (shown, view)
    };
    let mut pressed = None;
    let labels: Vec<&'static str> = story.play.as_ref().map(|_| "Play").into_iter().chain(story.buttons.iter().copied()).collect();
    // The story's buttons stand in the stage's footer: a row as wide as the stage, under the canvas.
    let (shown, view) = if labels.is_empty() {
        canvas(ui, room)
    } else {
        let buttons = |ui: &mut Ui, _: &mut BodyCx<'_>| {
            Row::new().fill_width().show(ui, |ui, _| {
                for &label in &labels {
                    if clicked(&Button::new(label).show(ui)) {
                        pressed = Some(label);
                    }
                }
            });
        };
        // A measuring run of the stage runs no body: no canvas, so no tool runs either.
        let stage = Stack::new().fill().footer(buttons).footer_gap(Gap::Stack).show_at(ui, room, |ui, stack| canvas(ui, stack.rect));
        let Some(shown) = stage.inner else { return StoryOutput::default() };
        shown
    };
    if let Some(script) = &story.play {
        let now = pointer::frame(ui.ctx()).time;
        if pressed == Some("Play") && playing.is_none() {
            playing = Some(Playing { start: now, sent: 0 });
        }
        if let Some(run) = playing.as_mut() {
            let share = ((now - run.start) / script.seconds.max(0.1)).clamp(0.0, 1.0);
            let due = ((share * script.steps.len() as f64) as usize + 1).min(script.steps.len());
            for &step in &script.steps[run.sent..due] {
                ToolHost::inject(ui.ctx(), shown.output.response.id, ToolScript::input(step, &view));
            }
            run.sent = due;
            if due == script.steps.len() {
                playing = None;
            }
            frame::request_repaint(ui.ctx());
        }
        pressed = pressed.filter(|label| *label != "Play");
    }
    let output = {
        let mut tool = kept.borrow_mut();
        run(ToolRun { tool: &mut tool, document, shown: shown.inner, view, pressed, host: StoryHost { ui, canvas: &shown.output } })
    };
    *data.note::<Option<Playing>>("play", None) = playing;
    output
}
