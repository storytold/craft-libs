//! A canvas under a modal dialog (15.5, 8.3): the view navigation passes, and so do the gestures the dialog names;
//! every other press stays blocked. The dialog's body keeps the canvas open the way a dialog does, through `BodyCx`.

use super::kit::{Leaf, Opened, PlainStyle, Seen};
use craft_behavior::{PointerButton, Sense};
use craft_keymap::Mods;
use craft_ui::contract::{
    Bind, CanvasGesture, Common, Component, Cx, MeasureCx, Overlay, OverlayUi, Overlays as _, Pointer as _, Props as _, Role, Sampler as _,
    Show as _, Trigger,
};
use craft_ui::runtime::focus::Initial;
use craft_ui::runtime::sampler;
use craft_ui::style::StyleCx;
use craft_ui::testing::focus::is_focused;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{middle, moved, wheel, zoom_wheel};
use craft_ui::testing::{Frames, trial};
use craft_ui::text::NumberField;
use egui::{Event, Id, Key, Modifiers, Pos2, Rect, Response, Ui, Vec2};
use std::cell::{Cell, RefCell};

const OPENER: Rect = Rect { min: Pos2::new(600.0, 100.0), max: Pos2::new(680.0, 120.0) };
const CANVAS: Rect = Rect { min: Pos2::new(20.0, 20.0), max: Pos2::new(320.0, 260.0) };

/// What the dialog's body asks of the canvas.
#[derive(Clone, Copy, PartialEq)]
enum Arm {
    Nothing,
    View,
    Gesture(CanvasGesture),
}

/// What reached the canvas.
#[derive(Default)]
struct Took {
    id: Option<Id>,
    /// The buttons of the presses it took.
    buttons: Vec<PointerButton>,
    dragged: bool,
    /// The passes that gave it a wheel share.
    wheels: u32,
    /// The passes whose wheel share zoomed.
    zooms: u32,
}

/// The canvas: a surface that takes presses of any button, their drags and the wheel. `outside` is a gesture it passes
/// as a control outside every modal body, as a picker armed in a panel does.
#[derive(Component)]
struct Surface<'a> {
    #[common]
    common: Common,
    took: &'a RefCell<Took>,
    outside: Option<CanvasGesture>,
}

impl Component for Surface<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: SurfaceStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        CANVAS.size()
    }

    fn sense(&self) -> Sense {
        Sense::PRESS | Sense::WHEEL
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(gesture) = self.outside {
            let id = cx.id;
            cx.pass_canvas_gesture(id, gesture);
        }
        let mut took = self.took.borrow_mut();
        took.id = Some(cx.id);
        let phase = cx.press().phase;
        if let Some(start) = phase.start()
            && !took.buttons.contains(&start.button)
        {
            took.buttons.push(start.button);
        }
        took.dragged |= phase.is_drag();
        if let Some(wheel) = cx.take_wheel() {
            took.wheels += 1;
            took.zooms += u32::from(wheel.zoom != 1.0);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
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
        // Focus starts on the armer: it is no field and no default button, where a dialog starts with nothing focused.
        let mut overlay = Overlay::modal();
        overlay.focus = overlay.focus.initial(Initial::First);
        let shown = ui.show(popup, overlay, |ui, body| {
            match scene.arm.get() {
                Arm::Nothing => {}
                Arm::View => body.keep_view_open(canvas),
                Arm::Gesture(gesture) => body.pass_canvas_gesture(canvas, gesture),
            }
            if let Some(value) = scene.field.borrow_mut().as_mut() {
                NumberField::new("Amount", Bind::bound(value)).show(ui);
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

/// The scene: the canvas and the dialog's opener.
struct Scene {
    canvas: RefCell<Took>,
    leaf: RefCell<Seen>,
    armer_id: Cell<Id>,
    open: Cell<bool>,
    arm: Cell<Arm>,
    /// The gesture the canvas passes as a control, outside the dialog's body.
    outside: Cell<Option<CanvasGesture>>,
    /// The gesture the canvas's host passes for its running tool.
    tool: Cell<Option<CanvasGesture>>,
    /// The value of a number field the dialog shows first, so it takes the dialog's first focus.
    field: RefCell<Option<Option<f64>>>,
    dialog: Opened,
}

impl Scene {
    fn new(arm: Arm) -> Self {
        Scene {
            canvas: RefCell::default(),
            leaf: RefCell::default(),
            armer_id: Cell::new(Id::NULL),
            open: Cell::new(false),
            arm: Cell::new(arm),
            outside: Cell::new(None),
            tool: Cell::new(None),
            field: RefCell::new(None),
            dialog: Opened::default(),
        }
    }

    fn show(&self, ui: &mut Ui) {
        self.dialog.shown.set(false);
        let canvas = self.canvas.borrow().id.unwrap_or(Id::NULL);
        let shown = Surface { common: Common::new(), took: &self.canvas, outside: self.outside.get() }.id_salt("canvas").show_at(ui, CANVAS);
        // The host's entry for its running tool, as `ToolHost::run` calls it each pass.
        if let Some(gesture) = self.tool.get() {
            sampler::pass_tool_gesture(ui.ctx(), shown.id, gesture);
        }
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
        let id = self.canvas.borrow().id;
        *self.canvas.borrow_mut() = Took { id, ..Took::default() };
    }

    fn took(&self, button: PointerButton) -> bool {
        self.canvas.borrow().buttons.contains(&button)
    }

    fn dragged_canvas(&self) -> bool {
        self.canvas.borrow().dragged
    }

    /// A press of `button` at `at` with `modifiers` held, a move by `by` and the release.
    fn drag_with(&self, frames: &mut Frames, button: egui::PointerButton, modifiers: Modifiers, at: Pos2, by: Vec2) {
        let event = |pos, pressed| Event::PointerButton { pos, button, pressed, modifiers };
        self.frame(frames, vec![moved(at)]);
        self.frame(frames, vec![event(at, true)]);
        self.frame(frames, vec![moved(at + by / 2.0)]);
        self.frame(frames, vec![moved(at + by)]);
        self.frame(frames, vec![event(at + by, false)]);
        self.settle(frames, 2);
    }

    /// A primary press at `at`, a move by `by` and the release.
    fn drag(&self, frames: &mut Frames, at: Pos2, by: Vec2) {
        self.drag_with(frames, egui::PointerButton::Primary, Modifiers::NONE, at, by);
    }

    /// The dialog is open and keeps key focus.
    fn assert_dialog_holds(&self, frames: &Frames) {
        assert!(self.dialog.shown.get(), "input on the canvas does not close the dialog");
        assert!(is_focused(&frames.ctx, self.armer_id.get()), "the dialog keeps key focus");
    }
}

/// With a gesture named, its press and drag on the canvas under the dialog reach it, the dialog keeps key focus and
/// Escape closes it; once it is closed the registration lapses, so a dialog that names nothing blocks the canvas again.
#[trial]
fn a_named_gesture_passes_its_press_and_drag_to_the_canvas_and_lapses_with_the_dialog() {
    let mut frames = Frames::new();
    let scene = Scene::new(Arm::Gesture(CanvasGesture::PRIMARY));
    scene.open_dialog(&mut frames);
    scene.drag(&mut frames, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(scene.took(PointerButton::Primary), "the press reached the canvas");
    assert!(scene.dragged_canvas(), "the drag stayed the canvas's");
    scene.assert_dialog_holds(&frames);
    scene.frame(&mut frames, vec![key_press(Key::Escape, Modifiers::NONE)]);
    scene.settle(&mut frames, 30);
    assert!(!scene.dialog.shown.get(), "Escape closes the dialog");
    // A dialog that opens later and does not arm gets no press through.
    scene.arm.set(Arm::Nothing);
    scene.open_dialog(&mut frames);
    scene.forget_canvas();
    scene.drag(&mut frames, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(!scene.took(PointerButton::Primary), "the old registration has lapsed");
}

/// A dialog that keeps the view open passes the wheel (scroll and zoom) and the middle-button pan to the canvas under
/// it, and blocks the primary press, which starts no gesture it named.
#[trial]
fn a_kept_view_passes_the_wheel_and_the_middle_pan_and_blocks_the_primary_press() {
    let mut frames = Frames::new();
    let scene = Scene::new(Arm::View);
    scene.open_dialog(&mut frames);
    scene.frame(&mut frames, vec![moved(CANVAS.center())]);
    scene.frame(&mut frames, vec![wheel(-40.0)]);
    scene.frame(&mut frames, vec![zoom_wheel(-40.0)]);
    scene.settle(&mut frames, 2);
    assert!(scene.canvas.borrow().wheels > scene.canvas.borrow().zooms, "the scroll reached the canvas");
    assert!(scene.canvas.borrow().zooms > 0, "the zoom reached the canvas");
    scene.drag_with(&mut frames, egui::PointerButton::Middle, Modifiers::NONE, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(scene.took(PointerButton::Middle) && scene.dragged_canvas(), "the middle-button pan reached the canvas");
    scene.forget_canvas();
    scene.drag(&mut frames, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(!scene.took(PointerButton::Primary), "the primary press stayed blocked");
    scene.assert_dialog_holds(&frames);
    // With nothing kept open the dialog blocks the view navigation as well.
    scene.arm.set(Arm::Nothing);
    scene.settle(&mut frames, 3);
    scene.forget_canvas();
    scene.frame(&mut frames, vec![wheel(-40.0)]);
    scene.frame(&mut frames, vec![middle(CANVAS.center(), true)]);
    scene.frame(&mut frames, vec![middle(CANVAS.center(), false)]);
    scene.settle(&mut frames, 2);
    assert!(scene.canvas.borrow().wheels == 0 && !scene.took(PointerButton::Middle), "the canvas is blocked");
}

/// A focused field in the dialog takes the wheel wherever the pointer is in the dialog, but over the canvas the dialog
/// keeps open the wheel goes by position: the canvas scrolls and the field keeps its value.
#[trial]
fn the_wheel_over_a_kept_canvas_scrolls_it_while_a_field_in_the_dialog_has_focus() {
    let mut frames = Frames::new();
    let scene = Scene::new(Arm::View);
    *scene.field.borrow_mut() = Some(Some(5.0));
    scene.open_dialog(&mut frames);
    let in_dialog = frames.ctx.read_response(scene.armer_id.get()).expect("the armer shows").rect.center();
    scene.frame(&mut frames, vec![moved(in_dialog)]);
    scene.frame(&mut frames, vec![wheel(-40.0)]);
    scene.settle(&mut frames, 2);
    let stepped = *scene.field.borrow();
    assert_ne!(stepped, Some(Some(5.0)), "the focused field takes the wheel away from it");
    frames.advance(2.0);
    scene.settle(&mut frames, 2);
    scene.frame(&mut frames, vec![moved(CANVAS.center())]);
    scene.frame(&mut frames, vec![wheel(-40.0)]);
    scene.settle(&mut frames, 2);
    assert!(scene.canvas.borrow().wheels > 0, "the wheel over the canvas scrolls it");
    assert_eq!(*scene.field.borrow(), stepped, "and leaves the field");
}

/// A gesture that needs a modifier passes only with it held at the press: Curves' ⌘-click reaches the canvas, a plain
/// press does not.
#[trial]
fn a_gesture_with_a_modifier_passes_only_with_the_modifier_held() {
    let mut frames = Frames::new();
    let scene = Scene::new(Arm::Gesture(CanvasGesture::PRIMARY.with(Mods::CMD)));
    scene.open_dialog(&mut frames);
    scene.drag(&mut frames, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(!scene.took(PointerButton::Primary), "a plain press stayed blocked");
    scene.drag_with(&mut frames, egui::PointerButton::Primary, Modifiers::SHIFT, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(!scene.took(PointerButton::Primary), "a press with another modifier stayed blocked");
    scene.drag_with(&mut frames, egui::PointerButton::Primary, Modifiers::COMMAND, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(scene.took(PointerButton::Primary) && scene.dragged_canvas(), "the ⌘-press and its drag reached the canvas");
    scene.assert_dialog_holds(&frames);
}

/// The gesture the canvas's host passes for its running tool joins the canvas a dialog keeps: the press and its drag
/// reach the canvas under the dialog, and once the dialog stops keeping the canvas the host's gesture lapses with it.
#[trial]
fn a_tool_gesture_the_host_passes_reaches_a_canvas_the_dialog_keeps_and_lapses_with_it() {
    let mut frames = Frames::new();
    let scene = Scene::new(Arm::View);
    scene.tool.set(Some(CanvasGesture::PRIMARY));
    scene.open_dialog(&mut frames);
    scene.drag(&mut frames, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(scene.took(PointerButton::Primary) && scene.dragged_canvas(), "the press and its drag reached the canvas");
    scene.assert_dialog_holds(&frames);
    scene.arm.set(Arm::Nothing);
    scene.settle(&mut frames, 3);
    scene.forget_canvas();
    scene.drag(&mut frames, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(!scene.took(PointerButton::Primary), "the host's gesture lapsed with the dialog's");
}

/// A dialog that keeps nothing (an alert) leaves the canvas blocked while the host goes on passing its tool's
/// gesture: no press and no wheel reach it.
#[trial]
fn a_tool_gesture_passes_nothing_under_a_dialog_that_keeps_no_canvas() {
    let mut frames = Frames::new();
    let scene = Scene::new(Arm::Nothing);
    scene.tool.set(Some(CanvasGesture::PRIMARY));
    scene.open_dialog(&mut frames);
    scene.forget_canvas();
    scene.drag(&mut frames, CANVAS.center(), Vec2::new(40.0, 10.0));
    scene.frame(&mut frames, vec![wheel(-40.0)]);
    scene.settle(&mut frames, 2);
    assert!(!scene.took(PointerButton::Primary), "the press stayed blocked");
    assert_eq!(scene.canvas.borrow().wheels, 0, "the wheel stayed blocked");
    scene.assert_dialog_holds(&frames);
}

/// A picker armed in a panel, outside every modal body, passes no press under a modal that opens while it stays armed.
#[trial]
fn a_picker_armed_outside_the_dialog_passes_no_press_under_it() {
    let mut frames = Frames::new();
    let scene = Scene::new(Arm::Nothing);
    scene.outside.set(Some(CanvasGesture::PRIMARY));
    scene.open_dialog(&mut frames);
    scene.forget_canvas();
    scene.drag(&mut frames, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(!scene.took(PointerButton::Primary), "the panel's picker passed no press");
    scene.assert_dialog_holds(&frames);
}

/// A press on the canvas starts there with no dialog, and starts nothing from the first pass a modal dialog shows in:
/// egui's modal gate leaves the canvas with no press target before the dialog's cover takes the hits.
#[trial]
fn a_press_under_a_modal_starts_nothing_on_the_canvas() {
    let mut frames = Frames::new();
    let scene = Scene::new(Arm::Nothing);
    scene.settle(&mut frames, 3);
    scene.drag(&mut frames, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(scene.took(PointerButton::Primary) && scene.dragged_canvas(), "with no dialog the press and drag start on the canvas");
    scene.forget_canvas();
    scene.open.set(true);
    while !scene.dialog.shown.get() {
        scene.frame(&mut frames, vec![]);
        scene.open.set(false);
    }
    let at = CANVAS.center();
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    scene.frame(&mut frames, vec![moved(at), press(at, true)]);
    scene.frame(&mut frames, vec![moved(at + Vec2::new(20.0, 5.0))]);
    scene.frame(&mut frames, vec![moved(at + Vec2::new(40.0, 10.0))]);
    scene.frame(&mut frames, vec![press(at + Vec2::new(40.0, 10.0), false)]);
    scene.settle(&mut frames, 2);
    assert!(!scene.took(PointerButton::Primary) && !scene.dragged_canvas(), "the press under the dialog started nothing");
    scene.drag(&mut frames, CANVAS.center(), Vec2::new(40.0, 10.0));
    assert!(!scene.took(PointerButton::Primary), "the press stays blocked while the dialog is open");
    assert!(scene.dialog.shown.get(), "the presses leave the dialog open");
}

/// A press on the canvas a modal dialog keeps open for its gesture is the canvas's (a colour sampled): it does not
/// nudge the dialog, which stays where it is. A press beside both the dialog and the canvas still nudges it.
#[trial]
fn a_press_the_kept_canvas_takes_does_not_nudge_the_dialog() {
    use craft_ui::testing::snapshot::layer_rects;
    let mut frames = Frames::new();
    frames.root(craft_ui::runtime::env::RootEnv::default().motion(craft_behavior::MotionMode::Full));
    let scene = Scene::new(Arm::Gesture(CanvasGesture::PRIMARY));
    scene.open_dialog(&mut frames);
    let rest = layer_rects(&frames.ctx);
    assert_eq!(rest.len(), 1, "the dialog's layer shows: {rest:?}");
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    // A press and release at `at`, then a few frames; whether the dialog's layer moved from its rest meanwhile.
    let moved_by_press = |frames: &mut Frames, at: Pos2| {
        let mut moved_away = false;
        for events in [vec![moved(at)], vec![press(at, true)], vec![press(at, false)], vec![], vec![], vec![]] {
            scene.frame(frames, events);
            moved_away |= layer_rects(&frames.ctx) != rest;
        }
        scene.settle(frames, 20);
        moved_away
    };

    assert!(!moved_by_press(&mut frames, CANVAS.center()), "a press the canvas takes does not nudge the dialog");
    assert!(scene.took(PointerButton::Primary), "the press reached the canvas");
    scene.assert_dialog_holds(&frames);

    let beside = Pos2::new(760.0, 560.0);
    assert!(!CANVAS.contains(beside) && !OPENER.contains(beside) && !rest[0].contains(beside), "the point lies beside dialog and canvas");
    assert!(moved_by_press(&mut frames, beside), "a press beside the dialog and the canvas nudges it");
    assert!(scene.dialog.shown.get(), "the press beside leaves the dialog open");
}
