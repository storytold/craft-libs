//! `ToolFixture` (22.4, the tool row): drives a [`CanvasTool`] on a [`Canvas`] through press, drag, release and
//! Escape, and checks what every tool owes the canvas.

use crate::canvas::{Canvas, CanvasTool, ToolHost, ToolPath};
use crate::contract::{BodyCx, ShowContainer as _};
use crate::runtime::access::Role;
use crate::testing::{Fixture, Target, pointer};
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, GesturePhase};
use egui::{Event, Id, Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::rc::Rc;

/// The size of the document the fixture's canvas shows, in document units: what the zoom tools read from the canvas's
/// space.
pub const DOCUMENT: Size = Size::new(1200.0, 900.0);

/// Where the Escape drag of [`ToolFixture::check`] starts, as a fraction of the canvas from its top left corner: away
/// from the path of the first drag, which a tool may leave a hit part on.
const ESCAPE_DRAG_START: Vec2 = Vec2::new(0.25, 0.25);

/// A tool shown on a canvas that fills the fixture's room, with the gesture phases the host reported.
#[non_exhaustive]
pub struct ToolFixture<T: CanvasTool + 'static> {
    fixture: Fixture,
    tool: Rc<RefCell<T>>,
    label: &'static str,
    gestures: Rc<RefCell<Vec<GesturePhase>>>,
    canvas: Rc<Cell<Id>>,
    enabled: Rc<Cell<bool>>,
}

impl<T: CanvasTool + 'static> ToolFixture<T> {
    /// A canvas named `label` whose active tool is `tool`.
    pub fn new(label: &'static str, tool: T) -> Self {
        Self::with_body(label, tool, |_, _| {})
    }

    /// A canvas named `label` whose body `body` fills: canvas controls shown at the body's rect join the canvas's
    /// arena, and a press on one of their parts never reaches the tool.
    pub fn with_body(label: &'static str, tool: T, body: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'static) -> Self {
        Self::shown(label, tool, None, body)
    }

    /// [`ToolFixture::with_body`] over a canvas given the view `view` makes of the canvas's room, so the body's
    /// controls and the tool see document coordinates (the view's document point at the room's centre, for one).
    pub fn with_view(
        label: &'static str,
        tool: T,
        view: impl Fn(Rect) -> CanvasView + 'static,
        body: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'static,
    ) -> Self {
        Self::shown(label, tool, Some(Box::new(view)), body)
    }

    fn shown(
        label: &'static str,
        tool: T,
        view: Option<Box<dyn Fn(Rect) -> CanvasView>>,
        mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'static,
    ) -> Self {
        let tool = Rc::new(RefCell::new(tool));
        let gestures = Rc::new(RefCell::new(Vec::new()));
        let canvas = Rc::new(Cell::new(Id::NULL));
        let enabled = Rc::new(Cell::new(true));
        let (shown, seen, id, on) = (Rc::clone(&tool), Rc::clone(&gestures), Rc::clone(&canvas), Rc::clone(&enabled));
        let mut fixture = Fixture::new(move |ui| {
            let mut shown_canvas = Canvas::new(label).document(DOCUMENT);
            if let Some(view) = &view {
                shown_canvas = shown_canvas.view(view(ui.available_rect_before_wrap()));
            }
            let canvas = shown_canvas.show(ui, &mut body);
            id.set(canvas.output.response.id);
            let reply = ToolHost::new().enabled(on.get()).run(ui, &canvas.output, &mut *shown.borrow_mut());
            if let Some(phase) = reply.gesture {
                seen.borrow_mut().push(phase);
            }
        });
        fixture.settle();
        ToolFixture { fixture, tool, label, gestures, canvas, enabled }
    }

    /// The fixture underneath, for input the helpers here do not cover.
    pub fn fixture(&mut self) -> &mut Fixture {
        &mut self.fixture
    }

    /// The tool.
    pub fn tool(&self) -> Ref<'_, T> {
        self.tool.borrow()
    }

    /// The tool, to change its options between gestures.
    pub fn tool_mut(&self) -> RefMut<'_, T> {
        self.tool.borrow_mut()
    }

    /// The canvas's node.
    pub fn canvas(&self) -> Target {
        let canvases = self.fixture.by_role(Role::CANVAS);
        assert!(!canvases.is_empty(), "the fixture shows a canvas");
        canvases[0]
    }

    /// The canvas's id, which keys the host's state: what [`ToolHost::inject`], [`ToolHost::cancel`] and
    /// [`ToolHost::in_gesture`] take.
    pub fn canvas_id(&self) -> Id {
        self.canvas.get()
    }

    /// Runs the host from the next frame on, or leaves it disabled (the app's tool is busy elsewhere).
    pub fn enabled(&mut self, enabled: bool) -> &mut Self {
        self.enabled.set(enabled);
        self
    }

    /// The gesture phases the host reported, one per pass that began, moved or ended a gesture, since the last
    /// [`ToolFixture::forget`].
    pub fn gestures(&self) -> Vec<GesturePhase> {
        self.gestures.borrow().clone()
    }

    /// Forgets the phases reported so far.
    pub fn forget(&mut self) -> &mut Self {
        self.gestures.borrow_mut().clear();
        self
    }

    fn count(&self, phase: GesturePhase) -> usize {
        self.gestures.borrow().iter().filter(|seen| **seen == phase).count()
    }

    /// Hover and press at `at` with the primary button, in separate frames.
    pub fn press_at(&mut self, at: Pos2) -> &mut Self {
        self.fixture.frame(vec![pointer::moved(at)]);
        self.fixture.frame(vec![pointer::press(at)]);
        self
    }

    /// Hover at `at`, then press and release twice there in successive frames: the runtime's click count reads the
    /// second press as 2, and the tool sees it as a press with `clicks` 2.
    pub fn double_press_at(&mut self, at: Pos2) -> &mut Self {
        self.fixture.frame(vec![pointer::moved(at)]);
        for _ in 0..2 {
            self.fixture.frame(vec![pointer::press(at)]);
            self.fixture.frame(vec![pointer::release(at)]);
        }
        self.fixture.frame(Vec::new());
        self
    }

    /// Moves to `to` in one frame.
    pub fn move_to(&mut self, to: Pos2) -> &mut Self {
        self.fixture.frame(vec![pointer::moved(to)]);
        self
    }

    /// Releases the primary button at `at`, then one idle frame.
    pub fn release_at(&mut self, at: Pos2) -> &mut Self {
        self.fixture.frame(vec![pointer::release(at)]);
        self.fixture.frame(Vec::new());
        self
    }

    /// Presses at `from`, moves by `by` in four steps and releases.
    pub fn drag_from(&mut self, from: Pos2, by: Vec2) -> &mut Self {
        self.drag_with(from, by, |at, down| if down { pointer::press(at) } else { pointer::release(at) })
    }

    /// [`ToolFixture::drag_from`] from the middle of the canvas.
    pub fn drag(&mut self, by: Vec2) -> &mut Self {
        let from = self.canvas().center();
        self.drag_from(from, by)
    }

    /// [`ToolFixture::drag_from`] with the middle button, which a canvas keeps for panning.
    pub fn middle_drag_from(&mut self, from: Pos2, by: Vec2) -> &mut Self {
        self.drag_with(from, by, pointer::middle)
    }

    /// [`ToolFixture::drag_from`] with the secondary button.
    pub fn secondary_drag_from(&mut self, from: Pos2, by: Vec2) -> &mut Self {
        self.drag_with(from, by, pointer::secondary)
    }

    fn drag_with(&mut self, from: Pos2, by: Vec2, button: fn(Pos2, bool) -> Event) -> &mut Self {
        self.fixture.frame(vec![pointer::moved(from)]);
        self.fixture.frame(vec![button(from, true)]);
        for step in 1..=4 {
            self.fixture.frame(vec![pointer::moved(from + by * (step as f32 / 4.0))]);
        }
        self.fixture.frame(vec![button(from + by, false)]);
        self.fixture.frame(Vec::new());
        self
    }

    /// Presses at `from`, moves halfway by `by`, presses Escape, moves the rest of the way and releases.
    pub fn drag_and_escape_from(&mut self, from: Pos2, by: Vec2) -> &mut Self {
        self.press_at(from);
        self.move_to(from + by * 0.25);
        self.move_to(from + by * 0.5);
        self.escape();
        self.move_to(from + by);
        self.release_at(from + by)
    }

    /// Presses Escape.
    pub fn escape(&mut self) -> &mut Self {
        self.fixture.key(Key::Escape, Modifiers::NONE);
        self
    }

    /// The checks every tool owes the canvas, on a drag from the middle of the canvas and an Escape drag from a quarter of the way in:
    ///
    /// - the tool has a command form: `ToolPath::Command` with a text, or `ToolPath::Freehand` with its reason;
    /// - a drag reaches the tool as one `Begin` and one `Commit`, with updates between;
    /// - Escape during a drag cancels it once, and a second Escape cancels nothing.
    pub fn check(&mut self) {
        self.check_command_form();
        let by = Vec2::new(60.0, 40.0);
        self.forget().drag(by);
        let seen = self.gestures();
        assert_eq!(self.count(GesturePhase::Begin), 1, "a drag begins once: {seen:?}");
        assert_eq!(self.count(GesturePhase::Commit), 1, "a drag commits once: {seen:?}");
        assert_eq!(self.count(GesturePhase::Cancel), 0, "a drag that ends in a release is not cancelled: {seen:?}");
        assert_eq!(seen.first(), Some(&GesturePhase::Begin), "a drag begins first: {seen:?}");
        assert_eq!(seen.last(), Some(&GesturePhase::Commit), "a drag commits last: {seen:?}");

        let canvas = self.canvas();
        let from = canvas.rect.min + canvas.rect.size() * ESCAPE_DRAG_START;
        self.forget().drag_and_escape_from(from, by);
        let seen = self.gestures();
        assert_eq!(self.count(GesturePhase::Begin), 1, "the drag began once: {seen:?}");
        assert_eq!(self.count(GesturePhase::Cancel), 1, "Escape cancels the drag once: {seen:?}");
        assert_eq!(self.count(GesturePhase::Commit), 0, "the release after Escape does not commit: {seen:?}");
        self.escape();
        assert_eq!(self.count(GesturePhase::Cancel), 1, "a second Escape cancels nothing: {:?}", self.gestures());
    }

    fn check_command_form(&self) {
        match self.tool.borrow().command_form() {
            ToolPath::Command(text) => assert!(!text.trim().is_empty(), "{}: the command form names its command", self.label),
            ToolPath::Freehand(reason) => {
                assert!(!reason.trim().is_empty(), "{}: a freehand tool names the command that does its job, or says there is none", self.label)
            }
        }
    }
}
