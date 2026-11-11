//! The sensing core over probe roots: the press lifecycle, arenas, press targets, parts and the shape record.

use craft_behavior::{CanvasTransform, HitLevel, PartKey, PressPhase, Sense};
use craft_ui::testing::pointer::{self, drag};
use craft_ui::testing::sensing::{ProbeMember, ProbePart, ProbeRoot, RootReport, SenseReport, egui_click_target, show};
use craft_ui::testing::{Frames, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const BACKGROUND: PartKey = PartKey::new("background");

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
}

/// The reports of every frame of a scenario.
#[derive(Default)]
struct Log {
    frames: Vec<Vec<RootReport>>,
}

fn press_phases(report: &SenseReport, key: Option<PartKey>) -> Vec<PressPhase> {
    let phase = match key {
        None => report.press.phase,
        Some(key) => report.phase(key),
    };
    phase.is_press().then_some(phase).into_iter().collect()
}

impl Log {
    fn last(&self) -> &RootReport {
        &self.frames[self.frames.len() - 1][0]
    }

    fn root_phases(&self, root: usize, key: Option<PartKey>) -> Vec<PressPhase> {
        self.frames.iter().flat_map(|reports| press_phases(&reports[root].root, key)).collect()
    }

    fn member_clicks(&self, root: usize, member: &'static str) -> usize {
        self.frames.iter().filter_map(|reports| reports[root].member(member)).filter(|report| report.clicked).count()
    }

    fn activated_members(&self, root: usize) -> usize {
        self.frames.iter().filter(|reports| reports[root].activated_member.is_some()).count()
    }

    fn root_activations(&self, root: usize) -> usize {
        self.frames.iter().filter(|reports| reports[root].root.activated).count()
    }

    fn clear(&mut self) {
        self.frames.clear();
    }
}

fn drags(phases: &[PressPhase]) -> usize {
    phases.iter().filter(|phase| matches!(phase, PressPhase::DragStart { .. })).count()
}

/// A frame closure that shows `roots` and logs what they read.
fn scene(roots: Vec<ProbeRoot>) -> (impl FnMut(&mut Ui), Rc<RefCell<Log>>) {
    let log = Rc::new(RefCell::new(Log::default()));
    let shared = log.clone();
    let frame = move |ui: &mut Ui| {
        let reports = show(ui, &roots);
        shared.borrow_mut().frames.push(reports);
    };
    (frame, log)
}

fn canvas(transform: Option<CanvasTransform>) -> ProbeRoot {
    let root = ProbeRoot::new("canvas", rect(100.0, 100.0, 500.0, 300.0), Sense::empty())
        .arena()
        .part(ProbePart::new(BACKGROUND, rect(100.0, 100.0, 500.0, 300.0), Sense::DRAG).level(HitLevel::Background, 0))
        .member(ProbeMember::new("tool", rect(160.0, 160.0, 90.0, 30.0), Sense::CLICK | Sense::FOCUS));
    match transform {
        Some(transform) => root.transformed(transform).blocker(Vec2::new(500.0, 300.0)),
        None => root,
    }
}

const ZOOM: CanvasTransform = CanvasTransform::new(2.0, Vec2::new(10.0, 5.0));

/// Skeleton: `frames_pointer::button_on_canvas_click_and_slop_drag` and `..._on_zoomed_canvas_...`.
fn member_on_canvas(transform: Option<CanvasTransform>) {
    let (mut show_frame, log) = scene(vec![canvas(transform)]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    let button = log.borrow().last().to_screen(Pos2::new(205.0, 175.0));
    let root_rect = rect(100.0, 100.0, 500.0, 300.0);
    assert!(root_rect.contains(button), "the button lies on the canvas: {button:?}");
    log.borrow_mut().clear();

    // A click on the member activates the member, not the canvas tool.
    pointer::click(&mut frames, button, &mut show_frame);
    {
        let log = log.borrow();
        assert_eq!(log.member_clicks(0, "tool"), 1, "the member was clicked once");
        assert!(log.root_phases(0, Some(BACKGROUND)).is_empty(), "the tool saw nothing: {:?}", log.root_phases(0, Some(BACKGROUND)));
        assert_eq!(log.root_activations(0), 0, "the root's own activation stays empty");
        assert_eq!(log.activated_members(0), 1, "the root's activated member named the member once");
    }
    if transform.is_some() {
        assert_eq!(
            egui_click_target(&frames.ctx),
            Some(ProbeRoot::transform_widget("canvas")),
            "egui's click target is the forwarding widget on the body layer"
        );
    }

    // A click has no duration limit: a press held for two seconds still activates once.
    frames.frame(vec![pointer::press(button)], &mut show_frame);
    frames.advance(2.0);
    frames.frame(vec![pointer::release(button)], &mut show_frame);
    frames.idle(1, &mut show_frame);
    assert_eq!(log.borrow().member_clicks(0, "tool"), 2, "a long press released within slop is a click");
    log.borrow_mut().clear();

    // A press on the member that moves past the slop starts no canvas drag and no click.
    drag(&mut frames, button, Vec2::new(40.0, 30.0), &mut show_frame);
    assert_eq!(log.borrow().member_clicks(0, "tool"), 0, "a dragged press is no click");
    assert!(log.borrow().root_phases(0, Some(BACKGROUND)).is_empty(), "the canvas tool saw nothing");
    assert_eq!(log.borrow().activated_members(0), 0);

    // Control: a drag that starts on the canvas background starts the tool's drag.
    let free = Pos2::new(160.0, 360.0);
    log.borrow_mut().clear();
    drag(&mut frames, free, Vec2::new(40.0, 30.0), &mut show_frame);
    let phases = log.borrow().root_phases(0, Some(BACKGROUND));
    assert_eq!(drags(&phases), 1, "a drag from the background starts one tool drag: {phases:?}");
    // The root's own parts live in the root's space (the parent layer), so the release lands where the pointer is.
    match phases.last() {
        Some(PressPhase::Release { pos, .. }) => assert!((*pos - (free + Vec2::new(40.0, 30.0))).length() < 0.5, "release at {pos:?}"),
        other => panic!("the drag ended with {other:?}"),
    }
}

#[trial]
fn member_on_zoomed_canvas_click_and_slop_drag() {
    member_on_canvas(Some(ZOOM));
}

/// Skeleton: `frames_pointer::touchpad_tap_activates_one_component`.
fn tap_on_canvas(transform: Option<CanvasTransform>, gone: bool) {
    let (mut show_frame, log) = scene(vec![canvas(transform)]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    let button = log.borrow().last().to_screen(Pos2::new(205.0, 175.0));
    log.borrow_mut().clear();

    // A tap on the member activates it once and nothing else.
    pointer::tap(&mut frames, button, gone, &mut show_frame);
    frames.idle(1, &mut show_frame);
    assert_eq!(log.borrow().member_clicks(0, "tool"), 1, "one tap, one member activation");
    assert_eq!(log.borrow().activated_members(0), 1, "the root names the member once");
    assert!(log.borrow().root_phases(0, Some(BACKGROUND)).is_empty(), "the tool starts no press lifecycle");

    // A tap on the background starts exactly one press lifecycle, on the tool, and activates no member.
    let free = Pos2::new(160.0, 360.0);
    pointer::tap(&mut frames, free, gone, &mut show_frame);
    frames.idle(1, &mut show_frame);
    let phases = log.borrow().root_phases(0, Some(BACKGROUND));
    // The input hook splits a one-frame tap over two passes, so the one lifecycle shows Down then Release.
    assert_eq!(
        phases.iter().filter(|phase| matches!(phase, PressPhase::Down { .. })).count(),
        1,
        "one tap, one press lifecycle on the tool: {phases:?}"
    );
    assert!(matches!(phases.last(), Some(PressPhase::Release { .. })), "{phases:?}");
    assert_eq!(log.borrow().member_clicks(0, "tool"), 1, "the member saw nothing more");
    assert_eq!(log.borrow().activated_members(0), 1);
}

#[trial]
fn touchpad_tap_activates_one_component() {
    tap_on_canvas(None, false);
    tap_on_canvas(Some(ZOOM), false);
    // A touch screen reports the pointer gone in the frame of the lift.
    tap_on_canvas(None, true);
    tap_on_canvas(Some(ZOOM), true);
}

/// A press reads `Down` in one pass only, the pass it went down, though its frame runs a discard pass and the
/// button stays down for later frames: an owner that acts on `Down` acts once per press.
#[trial]
fn a_held_press_reads_down_once_through_a_discard_pass_and_later_frames() {
    let (mut show_frame, log) = scene(vec![canvas(None)]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    log.borrow_mut().clear();

    let at = Pos2::new(160.0, 360.0);
    let mut first = true;
    let passes = frames.frame(vec![pointer::moved(at), pointer::press(at)], |ui| {
        if std::mem::take(&mut first) {
            ui.ctx().request_discard("test");
        }
        show_frame(ui);
    });
    assert_eq!(passes, 2, "the press frame runs a discard pass");
    frames.idle(3, &mut show_frame);
    frames.frame(vec![pointer::release(at)], &mut show_frame);
    frames.idle(1, &mut show_frame);

    let phases = log.borrow().root_phases(0, Some(BACKGROUND));
    assert!(matches!(phases.first(), Some(PressPhase::Down { .. })), "{phases:?}");
    assert_eq!(phases.iter().filter(|phase| matches!(phase, PressPhase::Down { .. })).count(), 1, "{phases:?}");
    let held = phases.iter().filter(|phase| matches!(phase, PressPhase::Held { .. })).count();
    assert!(held > 3, "the discard pass and the held frames read Held: {phases:?}");
    assert!(phases.iter().all(|phase| phase.button_down() || matches!(phase, PressPhase::Release { .. })), "{phases:?}");
    assert!(matches!(phases.last(), Some(PressPhase::Release { .. })), "{phases:?}");
}
