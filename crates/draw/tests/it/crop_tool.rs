//! The Crop tool and its session through the public API: the pass, the keys the overlay leaves to the tool, the
//! session's box history, drags through the overlay on a canvas, and the snapping of a Straighten line's ends.

use craft_behavior::CanvasView;
use craft_behavior::geometry::CropBox;
use craft_behavior::kurbo::{Point, Rect, Size};
use craft_draw::ui::crop::{CropDocKey, CropDocument, CropEvent, CropInput, CropRatio, CropSession, CropSettings, CropTool};
use craft_ui::canvas::{Canvas, NavigationOptions, Snap, ToolHost, tool_pass};
use craft_ui::contract::ShowContainer as _;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::env::{Env, Units};
use craft_ui::testing::{Fixture, Frames, pointer, trial};
use egui::{Key, Modifiers, Pos2, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const SIZE: Size = Size::new(400.0, 300.0);

/// The units of the default scope (PostScript points).
fn units() -> Units {
    let mut frames = Frames::new();
    let mut units = None;
    frames.frame(vec![], |ui| units = Some(Env::of(ui).units()));
    units.expect("the frame ran")
}

fn document(key: u64) -> CropDocument {
    CropDocument::new(CropDocKey::new(key), SIZE, 72.0)
}

fn upright(x0: f64, y0: f64, x1: f64, y1: f64) -> CropBox {
    CropBox::upright(Rect::new(x0, y0, x1, y1))
}

fn classic() -> CropSettings {
    let mut settings = CropSettings::default();
    settings.shield.classic_mode = true;
    settings
}

// The session cases.

fn sized(key: u64, width: f64, height: f64) -> CropDocument {
    CropDocument::new(CropDocKey::new(key), Size::new(width, height), 72.0)
}

#[trial]
fn the_first_set_of_a_session_is_a_box_step() {
    let mut session = CropSession::new();
    session.ensure_frame(&sized(1, 200.0, 100.0), false, None);
    let start = session.frame();
    session.set(upright(10.0, 10.0, 90.0, 60.0), false, "Crop");
    assert!(session.can_undo(), "the set is a step");
    assert!(session.undo());
    assert_eq!(session.frame(), start);
}

#[trial]
fn box_undo_redo_steps_committed_gestures_only() {
    let mut session = CropSession::new();
    let ctx = egui::Context::default();
    session.ensure_frame(&sized(1, 200.0, 100.0), false, None);
    let start = session.frame();
    assert!(!session.can_undo());

    session.nudge(&ctx, upright(10.0, 0.0, 100.0, 100.0), "Crop");
    session.nudge(&ctx, upright(20.0, 0.0, 100.0, 100.0), "Crop");
    session.nudge(&ctx, upright(30.0, 0.0, 100.0, 100.0), "Crop");
    assert!(session.can_undo(), "an open run counts, so Undo never falls through to the document");
    session.settle();
    assert!(session.pending());
    session.set(upright(31.0, 0.0, 100.0, 100.0), false, "Crop");

    assert!(session.undo());
    assert_eq!(session.frame(), Some(upright(30.0, 0.0, 100.0, 100.0)), "the run of nudges is one step");
    assert!(session.undo());
    assert_eq!(session.frame(), start);
    assert!(session.is_default(), "the default mark comes back with the step");
    assert!(!session.can_undo() && !session.undo());
    assert!(session.can_redo() && session.redo());
    assert_eq!(session.frame(), Some(upright(30.0, 0.0, 100.0, 100.0)));
    assert!(!session.is_default());

    session.set(upright(5.0, 5.0, 60.0, 60.0), false, "Crop");
    assert!(!session.can_redo() && !session.redo(), "a new step drops the redo side");
}

// One drag through the overlay on a canvas.

struct Host {
    tool: CropTool,
    settings: CropSettings,
    document: CropDocument,
    ctx: egui::Context,
    events: Vec<CropEvent>,
    /// The view the tool set last; the canvas shows it on the next pass.
    view: Option<CanvasView>,
    /// The canvas starts with its view turned.
    turned_start: bool,
    /// The overlay is shown: false after the tool was left, as the app stops showing it.
    shown: bool,
    /// The canvas's snapping.
    snap: Option<Snap>,
    /// The passes whose host response had the pointer past the canvas edge (an autoscroll).
    past_edge_passes: usize,
    /// The passes whose host response carried snap lines.
    snap_passes: usize,
    /// The canvas's id, from its last pass.
    canvas: Option<egui::Id>,
}

struct Rig {
    fixture: Fixture,
    host: Rc<RefCell<Host>>,
}

impl Rig {
    fn new(settings: CropSettings) -> Self {
        Self::build(settings, false)
    }

    /// A rig whose canvas starts with a turned view.
    fn turned(settings: CropSettings) -> Self {
        Self::build(settings, true)
    }

    fn build(settings: CropSettings, turned_start: bool) -> Self {
        let host = Rc::new(RefCell::new(Host {
            tool: CropTool::new(),
            settings,
            document: document(1),
            ctx: egui::Context::default(),
            events: Vec::new(),
            view: None,
            turned_start,
            shown: true,
            snap: None,
            past_edge_passes: 0,
            snap_passes: 0,
            canvas: None,
        }));
        let shown = Rc::clone(&host);
        let mut fixture = Fixture::new(move |ui| {
            let mut guard = shown.borrow_mut();
            let host = &mut *guard;
            let room = ui.available_rect_before_wrap();
            let base = CanvasView::new(room, Point::new(200.0, 150.0), 1.0);
            let view = host.view.unwrap_or(if host.turned_start { base.rotation(30.0) } else { base });
            let showing = host.shown;
            // Overscroll on: the rig's canvas shows the whole document at fit, where Overscroll off holds the view, and
            // image mode moves and turns the view.
            let mut navigation = NavigationOptions::default();
            navigation.overscroll = true;
            let mut canvas = Canvas::new("Document").document(SIZE).navigation(navigation).view(view);
            if let Some(snap) = host.snap.clone() {
                canvas = canvas.snap(snap);
            }
            // The body and the host run as an app with a split body runs them: the body's report reaches the host's
            // pass of the same frame.
            let tool = &mut host.tool;
            let (ctx, settings, document) = (&host.ctx, &host.settings, &host.document);
            let mut hosted = None;
            let output = canvas.show(ui, |ui, body| {
                let input = CropInput::new(ctx, settings, document);
                let frame = showing.then(|| tool.show_frame(ui, body.rect, &input));
                if let Some(canvas) = tool_pass(&body.env) {
                    let mut pass = tool.pass(input, frame);
                    let response = ToolHost::new().run(ui, &canvas, &mut pass);
                    hosted = Some((response, pass.finish().events));
                }
            });
            host.canvas = Some(output.response.id);
            let (response, events) = hosted.unwrap_or_default();
            host.past_edge_passes += usize::from(response.past_edge.is_some());
            host.snap_passes += usize::from(!response.snapped.is_empty());
            if let Some(view) = response.view {
                host.view = Some(view);
            }
            for event in &events {
                if let CropEvent::Settings(next) = event {
                    host.settings = next.clone();
                }
            }
            host.events.extend(events);
        });
        fixture.settle();
        Rig { fixture, host }
    }

    /// The screen point of document point `(x, y)`: the view shows (200, 150) at the canvas's centre.
    fn at(&self, x: f64, y: f64) -> Pos2 {
        let canvas = self.fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas");
        canvas.center() + Vec2::new(x as f32 - 200.0, y as f32 - 150.0)
    }

    /// Frames with no input, so the tool's answers to earlier calls show.
    fn run(&mut self) {
        for _ in 0..3 {
            self.fixture.frame(Vec::new());
        }
    }

    fn frame(&self) -> Option<CropBox> {
        self.host.borrow().tool.frame()
    }

    /// Gives the canvas the keyboard without a press, as a document window the app shows has it.
    fn focus_canvas(&mut self) {
        let canvas = self.host.borrow().canvas.expect("the canvas ran");
        craft_ui::testing::wheel::focus(self.fixture.ctx(), canvas);
        self.run();
        assert_eq!(self.fixture.focused(), Some(canvas), "the canvas has the keyboard");
    }

    /// The turn of the view the canvas shows, in degrees; `None` before the tool set one.
    fn view_turn(&self) -> Option<f64> {
        self.host.borrow().view.map(|view| view.rotation)
    }

    fn drag(&mut self, from: (f64, f64), to: (f64, f64)) {
        self.drag_with(from, to, Modifiers::NONE);
    }

    /// As [`drag`](Self::drag) with `mods` held through the drag.
    fn drag_with(&mut self, from: (f64, f64), to: (f64, f64), mods: Modifiers) {
        self.fixture.hold(mods);
        let (from, to) = (self.at(from.0, from.1), self.at(to.0, to.1));
        self.fixture.frame(vec![pointer::moved(from)]);
        self.fixture.frame(vec![pointer::press(from)]);
        for step in 1..=4 {
            self.fixture.frame(vec![pointer::moved(from + (to - from) * (step as f32 / 4.0))]);
        }
        self.fixture.frame(vec![pointer::release(to)]);
        self.fixture.frame(Vec::new());
        self.fixture.hold(Modifiers::NONE);
    }

    /// Where the frame's centre is on screen under the view the canvas shows.
    fn centre_on_screen(&self) -> Point {
        let host = self.host.borrow();
        let view = host.view.expect("the tool set the view");
        view.affine() * host.tool.frame().expect("a frame").center()
    }
}

/// A drag of the frame reaches the host in the pass that moved it: the snap lines of a frame edge on a guide, and the
/// autoscroll of a drag past the canvas edge.
#[trial]
fn a_frame_drag_hands_its_snap_lines_and_autoscroll_to_the_host_in_the_pass_that_moved_it() {
    use craft_behavior::geometry::{SnapKind, SnapOptions, SnapTarget, SnapTargets};
    use std::sync::Arc;

    let mut rig = Rig::new(classic());
    rig.drag((100.0, 80.0), (300.0, 220.0));
    let mut targets = SnapTargets::new();
    targets.add(SnapTarget::new(craft_behavior::Axis::Y, 350.0, SnapKind::Guide));
    rig.host.borrow_mut().snap = Some(Snap::new(Arc::new(targets), SnapOptions::default()));
    rig.run();
    rig.host.borrow_mut().snap_passes = 0;
    rig.drag((200.0, 150.0), (249.0, 150.0));
    assert!(rig.host.borrow().snap_passes > 0, "the frame's right edge on the guide reports its snap lines to the host");

    rig.host.borrow_mut().past_edge_passes = 0;
    let canvas = rig.fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas");
    let from = rig.at(200.0, 150.0);
    rig.fixture.frame(vec![pointer::moved(from)]);
    rig.fixture.frame(vec![pointer::press(from)]);
    rig.fixture.frame(vec![pointer::moved(from + Vec2::new(10.0, 0.0))]);
    rig.fixture.frame(vec![pointer::moved(Pos2::new(canvas.rect.right() + 20.0, from.y))]);
    rig.fixture.frame(Vec::new());
    assert!(rig.host.borrow().past_edge_passes > 0, "a move past the canvas edge autoscrolls");
    rig.fixture.frame(vec![pointer::release(Pos2::new(canvas.rect.right() + 20.0, from.y))]);
}

#[trial]
fn leaving_during_a_drag_ends_it_at_its_last_frame_and_applies() {
    let mut rig = Rig::new(classic());
    rig.drag((100.0, 80.0), (300.0, 220.0));
    let drawn = rig.frame().expect("the drawn frame");
    let from = rig.at(200.0, 150.0);
    rig.fixture.frame(vec![pointer::moved(from)]);
    rig.fixture.frame(vec![pointer::press(from)]);
    rig.fixture.frame(vec![pointer::moved(from + Vec2::new(30.0, 10.0))]);
    rig.fixture.frame(vec![pointer::moved(from + Vec2::new(40.0, 20.0))]);
    let last = rig.frame().expect("the moved frame");
    assert_ne!(last, drawn, "the drag moved the frame");
    let applied = {
        let mut host = rig.host.borrow_mut();
        let Host { tool, settings, document, .. } = &mut *host;
        assert!(tool.dragging());
        let applied = tool.leave(settings, document, &units(), false);
        assert!(!tool.dragging(), "leaving ends the drag");
        host.shown = false;
        applied
    };
    let Some(CropEvent::Apply(commit)) = applied else { panic!("leaving mid-drag applies, got {applied:?}") };
    assert_eq!(commit.frame, last, "the crop is the frame of the last update");
    let before = rig.host.borrow().events.len();
    rig.fixture.frame(vec![pointer::release(from + Vec2::new(40.0, 20.0))]);
    rig.fixture.frame(Vec::new());
    assert_eq!(rig.host.borrow().events.len(), before, "the later release changes nothing");
}

#[trial]
fn typing_a_ratio_in_several_steps_fits_from_the_frame_the_edit_began_with() {
    let mut rig = Rig::new(classic());
    rig.run();
    let mut host = rig.host.borrow_mut();
    let Host { tool, settings, document, .. } = &mut *host;
    let base = upright(20.0, 10.0, 180.0, 90.0);
    tool.crop_to(base);
    // 1 : 3 and then 16 : 3 on the way to 16 : 6: each step fits from the frame the edit started with, so the frame
    // does not shrink for good on the way.
    for (width, height) in [(1.0, 3.0), (16.0, 3.0), (16.0, 6.0)] {
        let mut ratio = settings.clone();
        ratio.ratio = CropRatio::sides(width, height);
        tool.refit(&ratio, document, &units(), true);
    }
    let frame = tool.frame().expect("a frame");
    assert!((frame.rect.width() / frame.rect.height() - 16.0 / 6.0).abs() < 1e-6, "{frame:?}");
    assert!((frame.rect.width() - base.rect.width()).abs() < 1e-6, "fitted from the frame the edit began with: {frame:?}");
    assert_eq!(frame.rect.center(), base.rect.center());
}

#[trial]
fn the_pens_eraser_end_standing_in_for_the_tool_keeps_the_frame_pending_until_the_tip_returns() {
    let mut rig = Rig::new(classic());
    rig.drag((100.0, 80.0), (300.0, 220.0));
    let drawn = rig.frame().expect("the drawn frame");
    let mut host = rig.host.borrow_mut();
    let Host { tool, settings, document, .. } = &mut *host;
    assert!(tool.leave(settings, document, &units(), true).is_none(), "the eraser end applies nothing");
    assert!(tool.session().pending());
    assert_eq!(tool.frame(), Some(drawn));
    // The pass after the tip comes back: leaving for another tool applies the frame.
    assert!(matches!(tool.leave(settings, document, &units(), false), Some(CropEvent::Apply(_))));
}

#[trial]
fn the_default_frame_fits_the_ratio_and_follows_the_selection() {
    // A new default frame fits the saved ratio about its centre.
    let mut square = classic();
    square.ratio = CropRatio::sides(1.0, 1.0);
    let rig = Rig::new(square);
    assert_eq!(rig.frame(), Some(upright(50.0, 0.0, 350.0, 300.0)));
    assert!(rig.host.borrow().tool.session().is_default());

    // With a selection the default frame is the selection's; Enter applies it until a ratio is picked.
    let mut rig = Rig::new(classic());
    let selection = Rect::new(100.0, 50.0, 300.0, 250.0);
    rig.host.borrow_mut().document = document(2).selection(Some(selection));
    rig.run();
    {
        let mut host = rig.host.borrow_mut();
        let Host { tool, settings, document, .. } = &mut *host;
        assert_eq!(tool.frame(), Some(CropBox::upright(selection)));
        assert!(tool.shows_selection_entry(settings));
        assert!(tool.session().applies());
        assert!(matches!(tool.apply(settings, document, &units()), Some(CropEvent::Apply(_))));
        tool.applied(false);
        let mut ratio = settings.clone();
        ratio.ratio = CropRatio::sides(1.0, 1.0);
        assert!(matches!(tool.refit(&ratio, document, &units(), false), Some(CropEvent::Frame(_))));
        assert_eq!(tool.frame(), Some(upright(50.0, 0.0, 350.0, 300.0)), "the ratio fits the canvas, not the selection");
        assert!(!tool.session().from_selection() && !tool.shows_selection_entry(&ratio) && !tool.session().applies());
    }
}

#[trial]
fn the_crop_keys_belong_to_the_crop_only_while_a_frame_is_pending() {
    // The untouched default frame (400 x 300, so X would turn it) leaves every key to the app, as Photoshop does:
    // P, H, O and I pick their tools, Shift+O cycles a tool group, X swaps the colours (photocraft#6536).
    let mut rig = Rig::new(classic());
    rig.run();
    rig.focus_canvas();
    let before = rig.host.borrow().settings.clone();
    rig.host.borrow_mut().events.clear();
    let keys = [
        (Key::P, Modifiers::NONE),
        (Key::H, Modifiers::NONE),
        (Key::O, Modifiers::NONE),
        (Key::O, Modifiers::SHIFT),
        (Key::I, Modifiers::NONE),
        (Key::X, Modifiers::NONE),
    ];
    for (key, mods) in keys {
        rig.fixture.key(key, mods);
        rig.run();
        let host = rig.host.borrow();
        assert!(host.events.is_empty(), "{key:?} with {mods:?} was the crop's: {:?}", host.events);
        assert_eq!(host.settings, before, "{key:?} with {mods:?} left the options alone");
        assert!(host.tool.session().is_default(), "{key:?} with {mods:?} left the untouched default frame");
    }

    // A pending frame takes O (the overlay), I (Front Image) and X (portrait and landscape).
    rig.drag((100.0, 80.0), (300.0, 220.0));
    rig.run();
    assert!(rig.host.borrow().tool.session().pending());
    rig.fixture.key(Key::O, Modifiers::NONE);
    rig.run();
    assert_ne!(rig.host.borrow().settings.guide, before.guide, "O cycles the overlay");
    rig.fixture.key(Key::I, Modifiers::NONE);
    rig.run();
    assert_ne!(rig.host.borrow().settings.ratio, before.ratio, "I is Front Image");
    let drawn = rig.frame().expect("a frame").rect.abs();
    rig.fixture.key(Key::X, Modifiers::NONE);
    rig.run();
    let swapped = rig.frame().expect("a frame").rect.abs();
    let turned = (swapped.width() - drawn.height()).abs() < 1.0 && (swapped.height() - drawn.width()).abs() < 1.0;
    assert!(turned, "X swaps {drawn:?} into {swapped:?}");

    // P and H on a pending frame.
    let mut rig = Rig::new(classic());
    rig.run();

    // A pending frame takes P (Classic Mode) and H (Show Cropped Area, inert in Classic Mode).
    rig.drag((100.0, 80.0), (300.0, 220.0));
    rig.run();
    assert!(rig.host.borrow().tool.session().pending());
    rig.fixture.key(Key::P, Modifiers::NONE);
    rig.run();
    assert!(!rig.host.borrow().settings.shield.classic_mode, "P left Classic Mode");
    let shown = rig.host.borrow().settings.shield.show_cropped_area;
    rig.fixture.key(Key::H, Modifiers::NONE);
    rig.run();
    assert_eq!(rig.host.borrow().settings.shield.show_cropped_area, !shown, "H toggles Show Cropped Area in image mode");
    rig.fixture.key(Key::P, Modifiers::NONE);
    rig.run();
    assert!(rig.host.borrow().settings.shield.classic_mode, "P again goes back");
}

#[trial]
fn the_default_frame_takes_no_box_step_and_hides_after_discard() {
    let selection = Rect::new(100.0, 50.0, 300.0, 250.0);
    // X on a square default frame makes no box step.
    let mut rig = Rig::new(classic());
    rig.host.borrow_mut().document = CropDocument::new(CropDocKey::new(3), Size::new(300.0, 300.0), 72.0);
    rig.run();
    rig.host.borrow_mut().events.clear();
    rig.fixture.key(Key::X, Modifiers::NONE);
    rig.run();
    {
        let host = rig.host.borrow();
        assert!(!host.tool.session().can_undo(), "no box step");
        assert!(!host.events.iter().any(|event| matches!(event, CropEvent::Frame(_))));
    }

    // Box Undo brings the selection state back.
    rig.host.borrow_mut().document = document(4).selection(Some(selection));
    rig.run();
    {
        let mut host = rig.host.borrow_mut();
        let tool = &mut host.tool;
        assert!(tool.session().from_selection());
        tool.crop_to(upright(10.0, 10.0, 90.0, 90.0));
        assert!(!tool.session().from_selection());
        assert!(tool.undo());
        assert!(tool.session().from_selection() && tool.session().is_default());
    }

    // Discard hides the default frame until a press.
    {
        let mut host = rig.host.borrow_mut();
        let event = host.tool.discard();
        assert!(matches!(event, Some(CropEvent::Discard(key)) if key == CropDocKey::new(4)));
    }
    rig.run();
    {
        let host = rig.host.borrow();
        assert!(host.tool.session().dismissed());
        assert!(!host.tool.session().applies() && !host.tool.modified());
    }
    rig.drag((100.0, 80.0), (300.0, 220.0));
    assert!(!rig.host.borrow().tool.session().dismissed(), "a press draws a new frame");

    // A ratio change on the same document shows a discarded frame again.
    {
        let mut host = rig.host.borrow_mut();
        assert!(host.tool.discard().is_some());
        assert!(host.tool.session().dismissed());
    }
    // The bar's ratio change comes in a later pass, which made the hidden default frame.
    rig.run();
    {
        let mut host = rig.host.borrow_mut();
        let Host { tool, settings, document, .. } = &mut *host;
        assert!(tool.session().dismissed());
        let mut ratio = settings.clone();
        ratio.ratio = CropRatio::sides(1.0, 1.0);
        assert!(matches!(tool.refit(&ratio, document, &units(), false), Some(CropEvent::Frame(_))));
        assert!(!tool.session().dismissed(), "a ratio change shows the frame");
    }
    rig.run();
    rig.drag((100.0, 80.0), (300.0, 220.0));

    // An accepted crop hides it too, for that document only; a size change shows it again.
    let cropped;
    {
        let mut host = rig.host.borrow_mut();
        let Host { tool, settings, document, .. } = &mut *host;
        let Some(CropEvent::Apply(commit)) = tool.apply(settings, document, &units()) else { panic!("a drawn frame applies") };
        assert!(commit.target.is_none());
        cropped = Size::new(commit.frame.rect.width(), commit.frame.rect.height());
        tool.applied(true);
        assert!(tool.session().dismissed());
    }
    // The cropped document keeps the frame hidden; another size shows it.
    rig.host.borrow_mut().document = CropDocument::new(CropDocKey::new(4), cropped, 72.0);
    rig.run();
    assert!(rig.host.borrow().tool.session().dismissed(), "the cropped document stays hidden");
    rig.host.borrow_mut().document = CropDocument::new(CropDocKey::new(4), Size::new(cropped.width + 50.0, cropped.height), 72.0);
    rig.run();
    assert!(!rig.host.borrow().tool.session().dismissed(), "a size change shows the frame");
    rig.host.borrow_mut().tool.discard();
    rig.host.borrow_mut().document = document(5).selection(Some(selection));
    rig.run();
    assert!(!rig.host.borrow().tool.session().dismissed(), "another document shows its frame");
    rig.host.borrow_mut().tool.discard();
    rig.host.borrow_mut().document = CropDocument::new(CropDocKey::new(5), Size::new(400.0, 200.0), 72.0);
    rig.run();
    assert!(!rig.host.borrow().tool.session().dismissed(), "another size shows the frame");
}

#[trial]
fn reset_returns_the_fitted_default_frame_as_one_box_step() {
    let mut rig = Rig::new(classic());
    {
        let mut host = rig.host.borrow_mut();
        let Host { tool, settings, document, .. } = &mut *host;
        assert_eq!(tool.reset_frame(settings, document, &units()), None, "an untouched frame has nothing to reset");
    }
    rig.drag((100.0, 80.0), (300.0, 220.0));
    {
        let mut host = rig.host.borrow_mut();
        let Host { tool, settings, document, .. } = &mut *host;
        assert!(tool.modified());
        assert!(matches!(tool.reset_frame(settings, document, &units()), Some(CropEvent::Frame(_))));
        assert_eq!(tool.frame(), Some(upright(0.0, 0.0, 400.0, 300.0)));
        assert!(!tool.modified());
        assert_eq!(tool.reset_frame(settings, document, &units()), None);
        assert!(tool.undo(), "Reset is one box step");
        assert!(tool.modified());
        assert!(tool.redo());
        assert!(tool.crop_to(CropBox::new(Rect::new(100.0, 50.0, 300.0, 250.0), 20.0)).is_some());
        assert!(tool.modified());
        tool.reset_frame(settings, document, &units());
        assert_eq!(tool.frame(), Some(upright(0.0, 0.0, 400.0, 300.0)), "the turn is gone");
    }
}

#[trial]
fn a_straighten_line_fits_when_armed_turns_in_place_when_held_and_always_spends_the_button() {
    // A horizon that falls 10° to the right levels with +10.
    let fall = 200.0 * 10f64.to_radians().tan();
    let (from, to) = ((100.0, 150.0), (300.0, 150.0 + fall));

    let mut rig = Rig::new(classic());
    rig.host.borrow_mut().tool.toggle_straighten();
    rig.run();
    rig.drag(from, to);
    let frame = rig.frame().expect("a frame");
    assert!((frame.angle - 10.0).abs() < 0.1, "angle {}", frame.angle);
    assert!(frame.rect.width() < 400.0 && frame.rect.height() < 300.0, "the armed line fits the canvas");
    {
        let mut host = rig.host.borrow_mut();
        assert!(!host.tool.straighten_armed());
        assert!(host.events.iter().any(|event| matches!(event, CropEvent::Straightened(_))));
        assert!(host.tool.undo(), "the line is one box step");
        assert_eq!(host.tool.frame().map(|f| f.angle), Some(0.0));
        assert!(!host.tool.session().can_undo());
    }

    // Cmd held: the frame turns in place.
    let mut rig = Rig::new(classic());
    rig.drag((100.0, 80.0), (300.0, 220.0));
    let before = rig.frame().expect("a drawn frame");
    rig.fixture.hold(Modifiers::COMMAND);
    rig.run();
    rig.drag_with(from, to, Modifiers::COMMAND);
    let frame = rig.frame().expect("a frame");
    assert!((frame.angle - 10.0).abs() < 0.1, "angle {}", frame.angle);
    assert_eq!(frame.rect, before.rect, "size and centre stay");

    // A click draws no line and still spends the armed button.
    let mut rig = Rig::new(classic());
    rig.host.borrow_mut().tool.toggle_straighten();
    rig.run();
    let start = rig.frame();
    rig.drag(from, from);
    assert!(!rig.host.borrow().tool.straighten_armed());
    assert_eq!(rig.frame(), start, "a click changes nothing");

    // Discard disarms.
    let mut host = rig.host.borrow_mut();
    host.tool.toggle_straighten();
    assert!(host.tool.straighten_armed());
    host.tool.discard();
    assert!(!host.tool.straighten_armed());
}

/// A Straighten line drawn inside a frame levels the frame on the line as drawn: its ends snap as points, never by the
/// offset that would snap the frame if the press moved it. Moved by the line's (277, 30), the frame's left edge would
/// land 3 units from the guide at x = 300.
#[trial]
fn a_straighten_line_inside_the_frame_snaps_its_ends_as_points_not_as_a_frame_move() {
    use craft_behavior::geometry::{SnapKind, SnapOptions, SnapTarget, SnapTargets};
    use std::sync::Arc;

    let angle_of = |from: (f64, f64), to: (f64, f64)| {
        let mut rig = Rig::new(classic());
        let mut targets = SnapTargets::new();
        targets.add(SnapTarget::new(craft_behavior::Axis::Y, 300.0, SnapKind::Guide));
        {
            let mut host = rig.host.borrow_mut();
            host.snap = Some(Snap::new(Arc::new(targets), SnapOptions::default()));
            let _ = host.tool.crop_to(upright(20.0, 20.0, 390.0, 290.0));
            host.tool.toggle_straighten();
        }
        rig.run();
        rig.drag(from, to);
        rig.frame().expect("a frame").angle
    };
    let level = |from: (f64, f64), to: (f64, f64)| CropBox::level_angle(Point::new(from.0, from.1), Point::new(to.0, to.1)).expect("a line");

    let angle = angle_of((60.0, 100.0), (337.0, 130.0));
    let drawn = level((60.0, 100.0), (337.0, 130.0));
    assert!((angle - drawn).abs() < 1e-3, "{angle} vs {drawn}: both ends clear of the guide stay where the pointer was");

    let angle = angle_of((60.0, 100.0), (298.0, 130.0));
    let snapped = level((60.0, 100.0), (300.0, 130.0));
    assert!((angle - snapped).abs() < 1e-3, "{angle} vs {snapped}: an end within reach of the guide snaps onto it");
}

#[trial]
fn a_line_started_with_cmd_keeps_turning_the_frame_in_place_after_cmd_is_let_go() {
    let fall = 200.0 * 10f64.to_radians().tan();
    let mut rig = Rig::new(classic());
    rig.drag((100.0, 80.0), (300.0, 220.0));
    let before = rig.frame().expect("a drawn frame");
    rig.fixture.hold(Modifiers::COMMAND);
    rig.run();
    let (from, to) = (rig.at(100.0, 150.0), rig.at(300.0, 150.0 + fall));
    rig.fixture.frame(vec![pointer::moved(from)]);
    rig.fixture.frame(vec![pointer::press(from)]);
    rig.fixture.frame(vec![pointer::moved(from + (to - from) * 0.5)]);
    // Cmd comes up before the line ends: it is still the line it started as.
    rig.fixture.hold(Modifiers::NONE);
    rig.fixture.frame(vec![pointer::moved(to)]);
    rig.fixture.frame(vec![pointer::release(to)]);
    rig.fixture.frame(Vec::new());
    let frame = rig.frame().expect("a frame");
    assert!((frame.angle - 10.0).abs() < 0.1, "angle {}", frame.angle);
    assert_eq!(frame.rect, before.rect, "size and centre stay");
}

#[trial]
fn image_mode_moves_and_turns_the_view_and_classic_mode_the_frame() {
    // A turned view comes upright when the tool runs, in either mode.
    for settings in [CropSettings::default(), classic()] {
        let mut rig = Rig::turned(settings);
        rig.run();
        let turn = rig.view_turn().expect("the tool set the view");
        assert!(turn.abs() < 1e-6, "the view is upright, turn {turn}");
    }

    // Classic Mode turns the frame on an upright view.
    let mut rig = Rig::new(classic());
    rig.drag((100.0, 80.0), (300.0, 220.0));
    rig.drag((350.0, 260.0), (350.0, 200.0));
    assert!(rig.frame().is_some_and(|frame| frame.angle.abs() > 1.0), "the frame turned");
    assert!(rig.view_turn().is_none_or(|turn| turn.abs() < 1e-6), "the view stayed upright");

    // A frame turned outside a gesture turns the view in image mode and stays put on the view in Classic Mode.
    let mut image = Rig::new(CropSettings::default());
    let turned = CropBox::new(Rect::new(100.0, 50.0, 300.0, 250.0), 20.0);
    image.host.borrow_mut().tool.crop_to(turned);
    image.run();
    assert!(image.view_turn().is_some_and(|turn| turn.abs() > 1.0), "image mode turns the view with the frame");
    assert_eq!(image.frame(), Some(turned));
    let mut classic_rig = Rig::new(classic());
    classic_rig.host.borrow_mut().tool.crop_to(turned);
    classic_rig.run();
    assert!(classic_rig.view_turn().is_none_or(|turn| turn.abs() < 1e-6), "Classic Mode leaves the view upright");

    // Image mode: a move drag moves the image under the frame, which stays put on screen and upright.
    let start = upright(100.0, 50.0, 300.0, 250.0);
    let mut moving = Rig::new(CropSettings::default());
    moving.host.borrow_mut().tool.crop_to(start);
    moving.run();
    let put = moving.centre_on_screen();
    moving.drag((200.0, 150.0), (230.0, 170.0));
    moving.run();
    let moved = moving.frame().expect("the frame");
    assert_ne!(moved.rect.origin(), start.rect.origin(), "the image moved under the frame");
    assert_eq!((moved.rect.width(), moved.rect.height()), (200.0, 200.0));
    assert!(moved.angle.abs() < 1e-6 && moving.view_turn().is_some_and(|turn| turn.abs() < 1e-6), "upright");
    assert!(moving.centre_on_screen().distance(put) < 1e-3, "the frame stayed put on screen");
    // Undo of the move brings the frame back and keeps it put.
    assert!(moving.host.borrow_mut().tool.undo());
    moving.run();
    assert_eq!(moving.frame(), Some(start));
    assert!(moving.centre_on_screen().distance(put) < 1e-3, "Undo kept the frame put");

    // A turn drag turns the image under the frame, which stays upright on screen; Shift turns in 15 degree steps.
    let mut turning = Rig::new(CropSettings::default());
    turning.host.borrow_mut().tool.crop_to(start);
    turning.run();
    let put = turning.centre_on_screen();
    turning.drag((340.0, 250.0), (340.0, 200.0));
    turning.run();
    let angle = turning.frame().expect("the frame").angle;
    assert!(angle.abs() > 1.0, "the image turned");
    assert!(turning.view_turn().is_some_and(|turn| (turn + angle).abs() < 1e-6), "the frame shows upright");
    assert!(turning.centre_on_screen().distance(put) < 1e-3, "the frame stayed put on screen");
    let mut stepped = Rig::new(CropSettings::default());
    stepped.host.borrow_mut().tool.crop_to(start);
    stepped.run();
    stepped.drag_with((340.0, 250.0), (340.0, 200.0), Modifiers::SHIFT);
    stepped.run();
    let steps = stepped.frame().expect("the frame").angle / 15.0;
    assert!(steps.round() != 0.0 && (steps - steps.round()).abs() < 1e-6, "Shift turns in 15 degree steps, got {steps}");

    // Apply and leaving the tool leave the view upright.
    for leave in [false, true] {
        let mut rig = Rig::new(CropSettings::default());
        rig.host.borrow_mut().tool.crop_to(start);
        rig.run();
        rig.drag((340.0, 250.0), (340.0, 200.0));
        rig.run();
        assert!(rig.view_turn().is_some_and(|turn| turn.abs() > 1.0), "the view is turned");
        {
            let mut host = rig.host.borrow_mut();
            let Host { tool, settings, document, .. } = &mut *host;
            let event = if leave { tool.leave(settings, document, &units(), false) } else { tool.apply(settings, document, &units()) };
            assert!(matches!(event, Some(CropEvent::Apply(_))));
            tool.applied(true);
        }
        rig.run();
        assert!(rig.view_turn().is_some_and(|turn| turn.abs() < 1e-6), "the view is upright after {}", if leave { "leave" } else { "Apply" });
    }
}

/// photocraft#16404: with the default settings (image mode, Auto Center Preview, Delete Cropped Pixels on), moving
/// the image under an upright frame takes the frame past the canvas, where the crop grows the canvas.
#[trial]
fn a_move_takes_an_upright_frame_past_the_canvas_with_delete_cropped_pixels_on() {
    use craft_behavior::geometry::{SnapKind, SnapOptions, SnapTargets};
    use std::sync::Arc;
    // With and without snapping to the document's edges, as View > Snap To > Document Bounds sets it.
    for snapping in [false, true] {
        let start = upright(20.0, 50.0, 120.0, 150.0);
        let mut rig = Rig::new(CropSettings::default());
        if snapping {
            let mut targets = SnapTargets::new();
            targets.add_rect(Rect::new(0.0, 0.0, SIZE.width, SIZE.height), SnapKind::Bounds, SnapKind::Center);
            rig.host.borrow_mut().snap = Some(Snap::new(Arc::new(targets), SnapOptions::default()));
        }
        rig.host.borrow_mut().tool.crop_to(start);
        rig.run();
        // At 31 %, as a large document fits the window.
        {
            let mut host = rig.host.borrow_mut();
            let view = host.view.expect("the tool set the view");
            let mut zoomed = view;
            zoomed.zoom = 0.31;
            host.view = Some(zoomed);
        }
        rig.run();
        // The image moves right under the frame: the frame goes left, past the canvas's left edge at 0.
        let (from, to) = {
            let host = rig.host.borrow();
            let view = host.view.expect("the view");
            (view.to_screen(Point::new(70.0, 100.0)), view.to_screen(Point::new(70.0, 100.0)) + Vec2::new(150.0, 0.0))
        };
        rig.fixture.frame(vec![pointer::moved(from)]);
        rig.fixture.frame(vec![pointer::press(from)]);
        for step in 1..=30 {
            rig.fixture.frame(vec![pointer::moved(from + (to - from) * (step as f32 / 30.0))]);
        }
        rig.fixture.frame(vec![pointer::release(to)]);
        rig.run();
        let moved = rig.frame().expect("the frame");
        assert!(moved.rect.x0 < -1.0, "snapping {snapping}: the frame moved past the canvas's left edge: {:?}", moved.rect);
        assert_eq!((moved.rect.width(), moved.rect.height()), (100.0, 100.0), "snapping {snapping}: a move keeps the frame's size");
    }
}
