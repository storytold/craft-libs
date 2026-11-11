//! A canvas under a modal dialog (15.5, 8.3): an armed sampler lets primary presses through. The dialog's body arms
//! the canvas the way a dialog does, through `BodyCx`.

use super::kit::{Leaf, Opened, PlainStyle, Seen};
use craft_behavior::Sense;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, Overlay, OverlayUi, Overlays as _, Props as _, Role, Sampler as _, Show as _, Trigger};
use craft_ui::style::StyleCx;
use craft_ui::testing::focus::is_focused;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{Probe, Seen as ProbeSeen};
use egui::{Event, Id, Key, Modifiers, Pos2, Rect, Response, Ui, Vec2};
use std::cell::{Cell, RefCell};

const OPENER: Rect = Rect { min: Pos2::new(600.0, 100.0), max: Pos2::new(680.0, 120.0) };
const CANVAS: Rect = Rect { min: Pos2::new(20.0, 20.0), max: Pos2::new(320.0, 260.0) };

/// What the dialog's body asks of the canvas.
#[derive(Clone, Copy, PartialEq)]
enum Arm {
    Nothing,
    Sampler,
}

/// A button that opens a modal dialog whose body arms the canvas, holds a focusable leaf.
#[derive(Component)]
#[component(overlay)]
struct Dialog<'a> {
    #[common]
    common: Common,
    scene: &'a Scene,
    canvas: Id,
}

impl Component for Dialog<'_> {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: DialogStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(80.0, 20.0)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("popup");
        popup.trigger(Trigger::Manual, cx);
        if self.scene.open.get() {
            popup.open(cx);
        }
    }

    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("popup");
        let scene = self.scene;
        let canvas = self.canvas;
        let shown = ui.show(popup, Overlay::modal(), |ui, body| {
            match scene.arm.get() {
                Arm::Nothing => {}
                Arm::Sampler => body.arm_sampler(canvas),
            }
            let leaf = Leaf::new(&scene.leaf, Vec2::new(80.0, 30.0)).id_salt("armer").show(ui);
            scene.armer_id.set(leaf.id);
        });
        if shown.is_some() {
            self.scene.dialog.shown.set(true);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The scene: a canvas probe and the dialog's opener.
struct Scene {
    area: Rect,
    canvas: RefCell<ProbeSeen>,
    leaf: RefCell<Seen>,
    armer_id: Cell<Id>,
    open: Cell<bool>,
    arm: Cell<Arm>,
    dialog: Opened,
}

impl Scene {
    fn new(arm: Arm) -> Self {
        Scene {
            area: CANVAS,
            canvas: RefCell::default(),
            leaf: RefCell::default(),
            armer_id: Cell::new(Id::NULL),
            open: Cell::new(false),
            arm: Cell::new(arm),
            dialog: Opened::default(),
        }
    }

    fn show(&self, ui: &mut Ui) {
        self.dialog.shown.set(false);
        let canvas = self.canvas.borrow().id.unwrap_or(Id::NULL);
        Probe::new(&self.canvas).size(self.area.size()).sense(Sense::CLICK | Sense::DRAG).id_salt("canvas").show_at(ui, self.area);
        Dialog { common: Common::new(), scene: self, canvas }.id_salt("dialog").show_at(ui, OPENER);
    }

    /// A frame with `events`; the scene's flags are read as they stand.
    fn frame(&self, frames: &mut Frames, events: Vec<Event>) {
        frames.frame(events, &mut |ui: &mut Ui| self.show(ui));
    }

    fn settle(&self, frames: &mut Frames, count: usize) {
        frames.idle(count, &mut |ui: &mut Ui| self.show(ui));
    }

    /// Opens the dialog and settles it.
    fn open_dialog(&self, frames: &mut Frames) {
        self.settle(frames, 3);
        self.open.set(true);
        self.frame(frames, vec![]);
        self.open.set(false);
        self.settle(frames, 30);
        assert!(self.dialog.shown.get(), "the dialog shows");
    }

    fn forget_canvas(&self) {
        self.canvas.borrow_mut().press.clear();
        self.leaf.borrow_mut().presses = 0;
        self.leaf.borrow_mut().activations = 0;
    }

    fn pressed_canvas(&self) -> bool {
        !self.canvas.borrow().press.is_empty()
    }

    fn dragged_canvas(&self) -> bool {
        self.canvas.borrow().press.iter().any(|phase| phase.is_drag())
    }

    /// A primary press at `at`, a move by `by` and the release.
    fn drag(&self, frames: &mut Frames, at: Pos2, by: Vec2) {
        self.frame(frames, vec![moved(at)]);
        self.frame(frames, vec![press(at)]);
        self.frame(frames, vec![moved(at + by / 2.0)]);
        self.frame(frames, vec![moved(at + by)]);
        self.frame(frames, vec![release(at + by)]);
        self.settle(frames, 2);
    }
}

/// With a sampler armed, a press and a drag on the canvas under the dialog reach it, the dialog keeps key focus and
/// Escape closes it; once it is closed the arm lapses, so a dialog that no longer arms blocks the canvas again.
#[trial]
fn an_armed_sampler_passes_a_press_and_a_drag_to_the_canvas_and_lapses_with_the_dialog() {
    let mut frames = Frames::new();
    let scene = Scene::new(Arm::Sampler);
    scene.open_dialog(&mut frames);
    scene.drag(&mut frames, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(scene.pressed_canvas(), "the press reached the canvas");
    assert!(scene.dragged_canvas(), "the drag stayed the canvas's");
    assert!(scene.dialog.shown.get(), "a press on the canvas does not close the dialog");
    assert!(is_focused(&frames.ctx, scene.armer_id.get()), "the dialog keeps key focus");
    scene.frame(&mut frames, vec![key_press(Key::Escape, Modifiers::NONE)]);
    scene.settle(&mut frames, 30);
    assert!(!scene.dialog.shown.get(), "Escape closes the dialog");
    // A dialog that opens later and does not arm gets no press through.
    scene.arm.set(Arm::Nothing);
    scene.open_dialog(&mut frames);
    scene.forget_canvas();
    scene.drag(&mut frames, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(!scene.pressed_canvas(), "the old arm has lapsed");
}
