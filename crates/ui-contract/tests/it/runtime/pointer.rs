//! The sensing core over probe roots: the press lifecycle, arenas, press targets, parts and the shape record.

use craft_behavior::{CanvasTransform, HitLevel, PartKey, PointerButton, PressPhase, Sense};
use craft_ui::testing::pointer::{self, drag};
use craft_ui::testing::sensing::{ProbeMember, ProbePart, ProbeRoot, RootReport, SenseReport, egui_click_target, show};
use craft_ui::testing::{Frames, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const BACKGROUND: PartKey = PartKey::new("background");

#[trial]
fn a_context_only_part_receives_a_secondary_click_in_one_frame() {
    let page = rect(100.0, 100.0, 400.0, 300.0);
    let field = rect(150.0, 150.0, 60.0, 50.0);
    let root = ProbeRoot::new("page", page, Sense::CLICK | Sense::DRAG)
        .part(ProbePart::new(BACKGROUND, field, Sense::CONTEXT_CLICK).level(HitLevel::Body, 0));
    let (mut show, log) = scene(vec![root]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    let at = field.center();
    frames.frame(vec![pointer::moved(at)], &mut show);
    log.borrow_mut().clear();
    frames.frame(vec![pointer::secondary(at, true), pointer::secondary(at, false)], &mut show);
    frames.idle(2, &mut show);
    let phases = log.borrow().root_phases(0, Some(BACKGROUND));
    assert!(
        phases.iter().any(|phase| matches!(phase, PressPhase::Release { start, click: true, .. } if start.button == PointerButton::Secondary)),
        "the context part receives the click: {phases:?}"
    );
    log.borrow_mut().clear();
    pointer::click(&mut frames, at, &mut show);
    let log = log.borrow();
    assert!(log.root_phases(0, Some(BACKGROUND)).is_empty(), "a primary click passes the context-only part");
    assert!(
        log.root_phases(0, None).iter().any(|phase| matches!(phase, PressPhase::Release { click: true, .. })),
        "the surface receives the primary click"
    );
}

#[trial]
fn quick_presses_count_clicks_with_their_own_modifiers() {
    let mut frames = Frames::new();
    let at = Pos2::new(100.0, 100.0);
    let mut counts = Vec::new();
    let mut show = |ui: &mut Ui| {
        craft_ui::runtime::pointer::with_pass_pointer(ui.ctx(), |pass| {
            for event in &pass.events {
                if let craft_behavior::PointerEvent::Pressed { clicks, .. } = event {
                    counts.push(*clicks);
                }
            }
        });
    };
    let button = |pressed, modifiers| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
    frames.frame(
        vec![pointer::moved(at), button(true, egui::Modifiers::SHIFT), button(false, egui::Modifiers::SHIFT), button(true, egui::Modifiers::NONE)],
        &mut show,
    );
    frames.idle(1, &mut show);
    assert_eq!(counts, vec![1, 1], "a change in modifiers starts a new click sequence");
}

#[trial]
fn a_press_uses_its_event_modifiers_when_the_frame_has_released_them() {
    let area = rect(100.0, 100.0, 80.0, 80.0);
    let (mut show, log) = scene(vec![ProbeRoot::new("modified-press", area, Sense::CLICK)]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    let at = area.center();
    frames.frame(vec![pointer::moved(at)], &mut show);
    frames.frame(
        vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::SHIFT }],
        &mut show,
    );
    assert!(log.borrow().last().root.press.mods.shift, "the press reads Shift even after the frame has released it");
}

/// A hover-revealed part must remain identifiable when a drag leaves its shape.
#[trial]
fn active_part_survives_a_drag_outside_its_shape() {
    let area = rect(100.0, 100.0, 80.0, 80.0);
    let root = ProbeRoot::new("active-part", area, Sense::empty()).part(ProbePart::new(BACKGROUND, area, Sense::DRAG));
    let (mut show_frame, _) = scene(vec![root]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    let point = area.center();
    frames.frame(vec![pointer::moved(point)], &mut show_frame);
    assert_eq!(craft_ui::runtime::pointer::active_part(&frames.ctx, ProbeRoot::id("active-part")), Some(BACKGROUND.into()));
    frames.frame(vec![pointer::press(point)], &mut show_frame);
    frames.frame(vec![pointer::moved(point + Vec2::new(150.0, 0.0))], &mut show_frame);
    assert_eq!(craft_ui::runtime::pointer::active_part(&frames.ctx, ProbeRoot::id("active-part")), Some(BACKGROUND.into()));
    frames.frame(vec![pointer::release(point + Vec2::new(150.0, 0.0))], &mut show_frame);
    frames.idle(1, &mut show_frame);
    assert_eq!(craft_ui::runtime::pointer::active_part(&frames.ctx, ProbeRoot::id("active-part")), None);
}

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
        self.last_root(0)
    }

    fn last_root(&self, root: usize) -> &RootReport {
        &self.frames[self.frames.len() - 1][root]
    }

    fn root_phases(&self, root: usize, key: Option<PartKey>) -> Vec<PressPhase> {
        self.frames.iter().flat_map(|reports| press_phases(&reports[root].root, key)).collect()
    }

    fn member_phases(&self, root: usize, member: &'static str, key: Option<PartKey>) -> Vec<PressPhase> {
        self.frames.iter().filter_map(|reports| reports[root].member(member)).flat_map(|report| press_phases(report, key)).collect()
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

/// A root's egui widget, and its transformed body's forwarding widget, sense what the members that joined it in the
/// same pass sense: a member that clicks makes them click from the first frame, and one gone or added changes them in
/// that frame.
#[trial]
fn a_root_widget_senses_what_its_members_sense_in_the_same_pass() {
    let with_member = Rc::new(std::cell::Cell::new(true));
    let shown = with_member.clone();
    let mut show_frame = move |ui: &mut Ui| {
        let root = ProbeRoot::new("canvas", rect(100.0, 100.0, 500.0, 300.0), Sense::empty())
            .arena()
            .part(ProbePart::new(BACKGROUND, rect(100.0, 100.0, 500.0, 300.0), Sense::DRAG).level(HitLevel::Background, 0))
            .transformed(ZOOM)
            .blocker(Vec2::new(500.0, 300.0));
        let root = if shown.get() { root.member(ProbeMember::new("tool", rect(160.0, 160.0, 90.0, 30.0), Sense::CLICK)) } else { root };
        show(ui, &[root]);
    };
    let clicks = |frames: &Frames| {
        [ProbeRoot::id("canvas"), ProbeRoot::transform_widget("canvas")]
            .map(|id| frames.ctx.widget_rect_last_pass(id).map(|widget| widget.sense.senses_click()))
    };
    let mut frames = Frames::new();
    frames.idle(1, &mut show_frame);
    assert_eq!(clicks(&frames), [Some(true); 2], "the first frame's widgets click with their member");
    with_member.set(false);
    frames.idle(1, &mut show_frame);
    assert_eq!(clicks(&frames), [Some(false); 2], "the widgets stop clicking in the frame the member goes");
    with_member.set(true);
    frames.idle(1, &mut show_frame);
    assert_eq!(clicks(&frames), [Some(true); 2], "the widgets click again in the frame the member comes back");
}

/// A root its clip cuts registers its whole rect as its egui widget's rect, and the part inside the clip as the rect
/// egui's hit test uses, as egui's own widgets do: its response reports and reveals where the root is.
#[trial]
fn a_root_cut_by_its_clip_keeps_its_whole_rect_and_senses_inside_the_clip() {
    let (whole, clip) = (rect(100.0, 100.0, 200.0, 100.0), rect(0.0, 0.0, 400.0, 150.0));
    let mut show_frame = move |ui: &mut Ui| {
        ui.set_clip_rect(clip);
        show(ui, &[ProbeRoot::new("cut", whole, Sense::CLICK)]);
    };
    let mut frames = Frames::new();
    frames.idle(1, &mut show_frame);
    let widget = frames.ctx.widget_rect_last_pass(ProbeRoot::id("cut")).expect("the root's widget");
    assert_eq!((widget.rect, widget.interact_rect), (whole, whole.intersect(clip)), "the whole rect, sensed inside the clip");
}

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

/// A click on any part of a root's own reports on the root's response, whichever egui widget took it: a part inside
/// the root rect and an edge part reaching past its end alike; the part result says which part.
#[trial]
fn a_click_on_an_own_part_reports_on_the_root_wherever_the_part_lies() {
    const INSIDE: PartKey = PartKey::new("inside");
    const PAST: PartKey = PartKey::new("past");
    let lane = ProbeRoot::new("lane", rect(100.0, 100.0, 200.0, 40.0), Sense::CLICK)
        .part(ProbePart::new(INSIDE, rect(120.0, 110.0, 20.0, 20.0), Sense::CLICK | Sense::DRAG).level(HitLevel::Edge, 0))
        .part(ProbePart::new(PAST, rect(290.0, 110.0, 20.0, 20.0), Sense::CLICK | Sense::DRAG).level(HitLevel::Edge, 0));
    let (mut show_frame, log) = scene(vec![lane]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    for (key, at) in [(INSIDE, Pos2::new(130.0, 120.0)), (PAST, Pos2::new(305.0, 120.0))] {
        log.borrow_mut().clear();
        pointer::click(&mut frames, at, &mut show_frame);
        let log = log.borrow();
        let clicks: Vec<_> = log.frames.iter().filter(|reports| reports[0].root.clicked).collect();
        assert_eq!(clicks.len(), 1, "{key:?}: the root's response reports the click once");
        assert!(
            matches!(clicks[0][0].root.phase(key), PressPhase::Release { click: true, .. }),
            "{key:?}: the part result names the part: {:?}",
            clicks[0][0].root.phase(key)
        );
        let other = if key == INSIDE { PAST } else { INSIDE };
        assert!(log.root_phases(0, Some(other)).is_empty(), "{key:?}: the other part saw no press");
    }
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

/// Input enters the first pass of a frame only: a frame with a pointer move, a wheel notch, a key press and an
/// injected step, forced to rerun, hands each to its consumer once. The rerun pass is not stirred, its wheel taker
/// has no share, its focused component has no intent and it carries no injected step.
#[trial]
fn a_rerun_pass_delivers_none_of_the_frames_input_again() {
    use craft_behavior::{Input, WheelTaker};
    use craft_ui::runtime::focus::{FocusTarget, KeyUse};
    use craft_ui::testing::focus::{Stop, intents_of, named_id, register_stop, widget};
    use craft_ui::testing::input::key_press;
    use craft_ui::testing::wheel::{register, take};

    #[derive(Default)]
    struct Pass {
        stirred: bool,
        wheel: bool,
        moves: usize,
        injected: bool,
    }
    let field = named_id("rerun_field");
    let log: Rc<RefCell<Vec<Pass>>> = Rc::default();
    let sink = log.clone();
    let mut show = move |ui: &mut Ui| {
        let ctx = ui.ctx().clone();
        widget(ui, field);
        register_stop(&ctx, Stop::new(field, KeyUse::of(Input::Text)));
        register(&ctx, WheelTaker::new(field, Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0))).vertical(true));
        let mut intents = intents_of(&ctx, Some(field), false);
        let mut moves = 0;
        while intents.take_move().is_some() {
            moves += 1;
        }
        sink.borrow_mut().push(Pass {
            stirred: craft_ui::runtime::pointer::with_pass_pointer(&ctx, |pass| pass.stirred),
            wheel: take(&ctx, field).is_some(),
            moves,
            injected: craft_ui::runtime::windows::WindowHost::of(&ctx).injected_pass(),
        });
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    craft_ui::runtime::focus::request(&frames.ctx, FocusTarget::Id(field));
    frames.idle(2, &mut show);
    craft_ui::runtime::windows::WindowHost::of(&frames.ctx).inject_text(egui::ViewportId::ROOT, "x");
    log.borrow_mut().clear();
    let mut first = true;
    let events = vec![pointer::moved(Pos2::new(20.0, 20.0)), pointer::wheel(-30.0), key_press(egui::Key::ArrowRight, egui::Modifiers::NONE)];
    let passes = frames.frame(events, |ui| {
        if std::mem::take(&mut first) {
            craft_ui::runtime::frame::discard_pass(ui.ctx(), "test");
        }
        show(ui);
    });
    assert_eq!(passes, 2, "the frame reruns");
    let log = log.borrow();
    assert_eq!(log.len(), 2);
    assert_eq!(log.iter().filter(|pass| pass.stirred).count(), 1, "one pass saw the pointer move");
    assert_eq!(log.iter().filter(|pass| pass.wheel).count(), 1, "one pass took the wheel notch");
    assert_eq!(log.iter().map(|pass| pass.moves).sum::<usize>(), 1, "the key's intent arrived once");
    assert_eq!(log.iter().filter(|pass| pass.injected).count(), 1, "the injected step entered once");
    assert!(log[0].stirred && log[0].injected && !log[1].stirred && !log[1].injected, "the first pass carries the input");
}

/// An injected step that enters a frame behind a real pointer press waits for the next frame (one press per pass), and
/// the pass that carries it acts for the automation client. Guards against a step whose flight ended with the frame it
/// entered, so the pass its text arrived in read as the user's (`injected_pass` false).
#[trial]
fn an_injected_step_held_behind_a_press_arrives_in_an_injected_pass() {
    let log: Rc<RefCell<Vec<(bool, bool)>>> = Rc::default();
    let sink = log.clone();
    let mut show = move |ui: &mut Ui| {
        let ctx = ui.ctx().clone();
        let typed = ctx.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Text(text) if text == "x")));
        sink.borrow_mut().push((typed, craft_ui::runtime::windows::WindowHost::of(&ctx).injected_pass()));
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    craft_ui::runtime::windows::WindowHost::of(&frames.ctx).inject_text(egui::ViewportId::ROOT, "x");
    log.borrow_mut().clear();
    let at = Pos2::new(20.0, 20.0);
    frames.frame(vec![pointer::moved(at), pointer::press(at)], &mut show);
    frames.frame(vec![pointer::release(at)], &mut show);
    frames.idle(2, &mut show);
    let log = log.borrow();
    let carrying: Vec<bool> = log.iter().filter(|(typed, _)| *typed).map(|(_, injected)| *injected).collect();
    assert_eq!(carrying, [true], "one pass carries the text, and it is the client's: {log:?}");
}

fn downs(phases: &[PressPhase]) -> usize {
    phases.iter().filter(|phase| matches!(phase, PressPhase::Down { .. })).count()
}

/// The pointer moving to `at`, then one idle frame: the frames after it read the hover at `at`.
fn move_to(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![pointer::moved(at)], &mut *show);
    frames.idle(1, show);
}

/// A press that enters and goes down in one frame, as an automation click does, then its release.
fn enter_and_click(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![pointer::moved(at), pointer::press(at)], &mut *show);
    frames.frame(vec![pointer::release(at)], &mut *show);
    frames.idle(1, show);
}

/// A control in a panel body takes its press though egui's click target there is a later widget over the whole body
/// (a container whose own click part lies in its header) with no shape that takes a press at that point: the press
/// goes to the shape on top under it, whatever egui's widget order says.
#[trial]
fn a_control_takes_its_press_under_a_later_body_sized_widget() {
    const TAB: PartKey = PartKey::new("tab");
    let panel = ProbeRoot::new("panel", rect(100.0, 100.0, 300.0, 300.0), Sense::empty())
        .member(ProbeMember::new("well", rect(150.0, 200.0, 40.0, 20.0), Sense::CLICK))
        .member(ProbeMember::new("body", rect(100.0, 100.0, 300.0, 300.0), Sense::HOVER).part(ProbePart::new(
            TAB,
            rect(100.0, 100.0, 80.0, 24.0),
            Sense::CLICK,
        )));
    let (mut show_frame, log) = scene(vec![panel]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    log.borrow_mut().clear();

    let well = Pos2::new(170.0, 210.0);
    enter_and_click(&mut frames, well, &mut show_frame);
    assert_eq!(egui_click_target(&frames.ctx), Some(ProbeRoot::id("body")), "egui's click target over the well is the body's widget");
    let log = log.borrow();
    assert_eq!(log.member_clicks(0, "well"), 1, "the well takes the click");
    assert_eq!(log.member_clicks(0, "body"), 0, "the body is not clicked");
    assert!(log.member_phases(0, "body", Some(TAB)).is_empty(), "the body's part saw no press");
    assert!(log.member_phases(0, "body", None).is_empty(), "the body's own shape saw no press");
}

const CORNER: PartKey = PartKey::new("corner");
const OVER_CORNER: Pos2 = Pos2::new(202.0, 152.0);
const OVER_EDITOR: Pos2 = Pos2::new(300.0, 200.0);
const OVER_CANVAS: Pos2 = Pos2::new(150.0, 350.0);

/// A canvas with a background part, a root nested in its body (a text editor) and a member shown after it (a text
/// frame with a corner handle at the editor's corner).
fn overlapping_canvas() -> ProbeRoot {
    ProbeRoot::new("canvas", rect(100.0, 100.0, 500.0, 300.0), Sense::empty())
        .arena()
        .part(ProbePart::new(BACKGROUND, rect(100.0, 100.0, 500.0, 300.0), Sense::CLICK | Sense::DRAG).level(HitLevel::Background, 0))
        .member(ProbeMember::new("editor", rect(200.0, 150.0, 200.0, 100.0), Sense::CLICK | Sense::DRAG).nested())
        .member(
            ProbeMember::new("frame", rect(100.0, 100.0, 500.0, 300.0), Sense::empty())
                .part(ProbePart::new(CORNER, rect(194.0, 144.0, 12.0, 12.0), Sense::CLICK | Sense::DRAG).level(HitLevel::Handle, 1)),
        )
}

/// Over the overlapping canvas: exactly one of the canvas background, the editor and the frame's corner starts each
/// press, the one whose shape is on top under it and takes both its click and its drag.
#[trial]
fn one_root_starts_a_press_where_roots_overlap_on_a_layer() {
    let (mut show_frame, log) = scene(vec![overlapping_canvas()]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);

    for (at, expected) in [(OVER_CORNER, [0, 0, 1]), (OVER_EDITOR, [0, 1, 0]), (OVER_CANVAS, [1, 0, 0])] {
        log.borrow_mut().clear();
        enter_and_click(&mut frames, at, &mut show_frame);
        let log = log.borrow();
        let started = [
            downs(&log.root_phases(0, Some(BACKGROUND))),
            downs(&log.member_phases(0, "editor", None)),
            downs(&log.member_phases(0, "frame", Some(CORNER))),
        ];
        assert_eq!(started, expected, "at {at:?}: presses started on [canvas background, editor, frame corner]");
    }
}

/// Hover shows where a press goes: over the overlapping canvas, the shapes that read the pointer over them before a
/// press are the ones the press then starts on, and nothing a press would not reach hovers.
#[trial]
fn hover_shows_where_a_press_goes_where_roots_overlap() {
    let (mut show_frame, log) = scene(vec![overlapping_canvas()]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);

    for at in [OVER_CORNER, OVER_EDITOR, OVER_CANVAS] {
        move_to(&mut frames, at, &mut show_frame);
        let hovered = {
            let log = log.borrow();
            let report = log.last_root(0);
            let editor = report.member("editor").expect("the editor shows");
            let frame = report.member("frame").expect("the frame shows");
            [report.root.part_hovered(BACKGROUND), editor.hovered, frame.part_hovered(CORNER)]
        };
        log.borrow_mut().clear();
        enter_and_click(&mut frames, at, &mut show_frame);
        let log = log.borrow();
        let started = [
            downs(&log.root_phases(0, Some(BACKGROUND))) > 0,
            downs(&log.member_phases(0, "editor", None)) > 0,
            downs(&log.member_phases(0, "frame", Some(CORNER))) > 0,
        ];
        assert_eq!(hovered, started, "at {at:?}: [canvas background, editor, frame corner] hovered as a press there starts");
        assert_eq!(hovered.iter().filter(|hovered| **hovered).count(), 1, "at {at:?}: one shape hovers");
    }
}

/// A clickable row with a nested button and a nested label that only shows a tooltip: over the button only the button
/// hovers, though the row contains the pointer; over the label both the label and the row hover, and the click goes to
/// the row.
#[trial]
fn a_root_under_a_nested_control_hovers_only_where_its_press_lands() {
    let row = ProbeRoot::new("row", rect(100.0, 100.0, 300.0, 40.0), Sense::CLICK)
        .member(ProbeMember::new("button", rect(320.0, 108.0, 60.0, 24.0), Sense::CLICK))
        .member(ProbeMember::new("label", rect(110.0, 108.0, 120.0, 24.0), Sense::HOVER));
    let (mut show_frame, log) = scene(vec![row]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);

    let hovered = |log: &Log| {
        let report = log.last_root(0);
        let (button, label) = (report.member("button").expect("the button shows"), report.member("label").expect("the label shows"));
        [report.root.hovered, button.hovered, label.hovered]
    };
    move_to(&mut frames, Pos2::new(350.0, 120.0), &mut show_frame);
    assert_eq!(hovered(&log.borrow()), [false, true, false], "over the button: [row, button, label] hovered");
    assert!(log.borrow().last_root(0).root.contains_pointer, "the row contains the pointer over its button");
    move_to(&mut frames, Pos2::new(150.0, 120.0), &mut show_frame);
    assert_eq!(hovered(&log.borrow()), [true, false, true], "over the label: [row, button, label] hovered");
    move_to(&mut frames, Pos2::new(260.0, 120.0), &mut show_frame);
    assert_eq!(hovered(&log.borrow()), [true, false, false], "over the row alone: [row, button, label] hovered");

    log.borrow_mut().clear();
    enter_and_click(&mut frames, Pos2::new(150.0, 120.0), &mut show_frame);
    let log = log.borrow();
    assert_eq!(downs(&log.root_phases(0, None)), 1, "a press on the label starts on the row");
    assert!(log.member_phases(0, "label", None).is_empty(), "the label takes no press");
}

/// Hover and presses read one resolution of what lies under the pointer per pass, however many roots stack there.
#[trial]
fn one_pointer_resolution_per_pass_whatever_the_number_of_roots() {
    const NAMES: [&str; 8] = ["r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7"];
    let counts = |roots: usize| {
        let stack = NAMES[..roots].iter().map(|name| ProbeRoot::new(name, rect(100.0, 100.0, 200.0, 100.0), Sense::CLICK)).collect();
        let (mut show_frame, _log) = scene(stack);
        let mut frames = Frames::new();
        frames.idle(2, &mut show_frame);
        let at = Pos2::new(200.0, 150.0);
        let mut counts = Vec::new();
        for events in [vec![pointer::moved(at)], vec![pointer::press(at)], vec![pointer::release(at)], Vec::new()] {
            frames.frame(events, &mut show_frame);
            counts.push(pointer::resolutions(&frames.ctx));
        }
        counts
    };
    let (one, eight) = (counts(1), counts(8));
    assert_eq!(one, eight, "resolutions per pass with one root and with eight stacked roots");
    assert!(one.iter().all(|count| *count <= 1), "at most one resolution per pass: {one:?}");
    assert!(one.contains(&1), "the pointer in the window is resolved: {one:?}");
}

const LINK: PartKey = PartKey::new("link");
const SURFACE: PartKey = PartKey::new("surface");

#[trial]
fn passed_drag_reaches_an_arena_roots_background_part() {
    let page = rect(100.0, 100.0, 400.0, 300.0);
    let area = rect(150.0, 150.0, 60.0, 40.0);
    let background = ProbePart::new(BACKGROUND, page, Sense::CLICK | Sense::DRAG).level(HitLevel::Background, 0);
    let field = ProbePart::new(SURFACE, area, Sense::CLICK | Sense::DRAG);
    let overlay = ProbePart::new(LINK, area, Sense::CLICK | Sense::PASSES_DRAG).level(HitLevel::Edge, 0).drag_to_root();
    let root = ProbeRoot::new("canvas", page, Sense::FOCUS | Sense::WHEEL)
        .arena()
        .part(background)
        .member(ProbeMember::new("parts", page, Sense::empty()).part(field).part(overlay));
    let (mut show_frame, log) = scene(vec![root]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    drag(&mut frames, area.center(), Vec2::new(40.0, 30.0), &mut show_frame);
    assert_eq!(drags(&log.borrow().root_phases(0, Some(BACKGROUND))), 1, "background part takes the drag");
    assert_eq!(drags(&log.borrow().member_phases(0, "parts", Some(SURFACE))), 0, "field does not drag");
    log.borrow_mut().clear();
    pointer::click(&mut frames, area.center(), &mut show_frame);
    assert!(log.borrow().member_phases(0, "parts", Some(LINK)).iter().any(|phase| matches!(phase, PressPhase::Release { click: true, .. })));
}

/// A clickable overlay can send its drag to the canvas root past a draggable field.
#[trial]
fn passed_drag_can_target_the_arena_root_past_a_field() {
    let page = rect(100.0, 100.0, 400.0, 300.0);
    let area = rect(150.0, 150.0, 60.0, 40.0);
    let field = ProbePart::new(SURFACE, area, Sense::CLICK | Sense::DRAG);
    let comment = ProbePart::new(LINK, area, Sense::CLICK | Sense::PASSES_DRAG).level(HitLevel::Edge, 0).drag_to_root();
    let root = ProbeRoot::new("canvas", page, Sense::DRAG).arena().member(ProbeMember::new("parts", page, Sense::empty()).part(field).part(comment));
    let (mut show_frame, log) = scene(vec![root]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    drag(&mut frames, area.center(), Vec2::new(40.0, 30.0), &mut show_frame);
    assert_eq!(drags(&log.borrow().root_phases(0, None)), 1, "canvas takes the drag");
    assert_eq!(drags(&log.borrow().member_phases(0, "parts", Some(SURFACE))), 0, "field does not drag");
    log.borrow_mut().clear();
    pointer::click(&mut frames, area.center(), &mut show_frame);
    assert!(log.borrow().member_phases(0, "parts", Some(LINK)).iter().any(|phase| matches!(phase, PressPhase::Release { click: true, .. })));
}

/// L-19: a link on a page clicks, but a drag that starts on it belongs to the page under it (production's
/// `Hit::declines_drag`). A press past slop on a click part that passes its drag gives the `DragStart` to the next
/// shape below that drags, whether that shape is the same root's own part or a root shown below; the link sees no
/// press of its own then, and a release within slop is the link's click with nothing below pressed.
#[trial]
fn a_press_over_a_passing_click_part_hands_its_drag_to_the_shape_below_in_its_root_and_in_the_root_below() {
    let page = rect(100.0, 100.0, 400.0, 300.0);
    let link_rect = rect(150.0, 150.0, 60.0, 20.0);
    let link = || ProbePart::new(LINK, link_rect, Sense::CLICK | Sense::PASSES_DRAG).level(HitLevel::Edge, 0);
    let surface = || ProbePart::new(SURFACE, page, Sense::DRAG).level(HitLevel::Background, 0);
    let own = vec![ProbeRoot::new("page", page, Sense::empty()).part(surface()).part(link())];
    let stacked = vec![ProbeRoot::new("page", page, Sense::empty()).part(surface()), ProbeRoot::new("link", link_rect, Sense::empty()).part(link())];
    // Each case: its roots, the root of the link and the root of the dragging shape below it.
    for (case, roots, link_root, below_root) in [("in its own root", own, 0, 0), ("in the root below", stacked, 1, 0)] {
        let (mut show_frame, log) = scene(roots);
        let mut frames = Frames::new();
        frames.idle(2, &mut show_frame);
        let on_link = link_rect.center();
        log.borrow_mut().clear();

        drag(&mut frames, on_link, Vec2::new(40.0, 30.0), &mut show_frame);
        {
            let log = log.borrow();
            let below = log.root_phases(below_root, Some(SURFACE));
            assert_eq!(drags(&below), 1, "{case}: the shape below takes the drag: {below:?}");
            assert!(matches!(below.last(), Some(PressPhase::Release { .. })), "{case}: and its release: {below:?}");
            let on = log.root_phases(link_root, Some(LINK));
            assert!(
                !on.iter().any(|phase| matches!(phase, PressPhase::DragStart { .. } | PressPhase::Release { click: true, .. })),
                "{case}: the link neither drags nor clicks: {on:?}"
            );
        }

        log.borrow_mut().clear();
        pointer::click(&mut frames, on_link, &mut show_frame);
        let log = log.borrow();
        let on = log.root_phases(link_root, Some(LINK));
        assert!(
            on.iter().any(|phase| matches!(phase, PressPhase::Release { click: true, .. })),
            "{case}: a release within slop clicks the link: {on:?}"
        );
        let below = log.root_phases(below_root, Some(SURFACE));
        assert!(below.is_empty(), "{case}: the shape below sees no press of a click: {below:?}");
    }
}

/// A secondary press over a part that drags and clicks but takes no context click (a transform box's) goes to the
/// context target under it, in its own root or in the root below, and the box sees none of it; a primary press there
/// is still the box's.
#[trial]
fn a_right_click_over_a_drag_only_part_reaches_the_context_target_under_it() {
    let page = rect(100.0, 100.0, 400.0, 300.0);
    let box_rect = rect(150.0, 150.0, 120.0, 80.0);
    let handle = || ProbePart::new(CORNER, box_rect, Sense::DRAG | Sense::CLICK).level(HitLevel::Handle, 0);
    let surface = || ProbePart::new(SURFACE, page, Sense::DRAG | Sense::CLICK | Sense::CONTEXT_CLICK).level(HitLevel::Background, 0);
    let own = vec![ProbeRoot::new("page", page, Sense::empty()).part(surface()).part(handle())];
    let stacked = vec![ProbeRoot::new("page", page, Sense::empty()).part(surface()), ProbeRoot::new("box", box_rect, Sense::empty()).part(handle())];
    for (case, roots, box_root) in [("in its own root", own, 0), ("in the root below", stacked, 1)] {
        let (mut show_frame, log) = scene(roots);
        let mut frames = Frames::new();
        frames.idle(2, &mut show_frame);
        let on_box = box_rect.center();
        move_to(&mut frames, on_box, &mut show_frame);
        log.borrow_mut().clear();
        frames.frame(vec![pointer::secondary(on_box, true)], &mut show_frame);
        frames.frame(vec![pointer::secondary(on_box, false)], &mut show_frame);
        frames.idle(1, &mut show_frame);
        {
            let log = log.borrow();
            let below = log.root_phases(0, Some(SURFACE));
            let released = below.iter().any(|phase| matches!(phase, PressPhase::Release { start, .. } if start.button == PointerButton::Secondary));
            assert!(released, "{case}: the context target under the box takes the right-click: {below:?}");
            let on = log.root_phases(box_root, Some(CORNER));
            assert!(on.is_empty(), "{case}: the box sees none of it: {on:?}");
        }
        log.borrow_mut().clear();
        enter_and_click(&mut frames, on_box, &mut show_frame);
        let log = log.borrow();
        assert_eq!(downs(&log.root_phases(box_root, Some(CORNER))), 1, "{case}: a primary press is still the box's");
    }
}

/// While a press on one root is held, another root the pointer moves over does not hover; it hovers once the press
/// ends.
#[trial]
fn a_held_press_keeps_hover_off_other_roots() {
    let a = ProbeRoot::new("a", rect(100.0, 100.0, 100.0, 40.0), Sense::CLICK);
    let b = ProbeRoot::new("b", rect(250.0, 100.0, 100.0, 40.0), Sense::CLICK);
    let (mut show_frame, log) = scene(vec![a, b]);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    let (on_a, on_b) = (Pos2::new(150.0, 120.0), Pos2::new(300.0, 120.0));

    frames.frame(vec![pointer::moved(on_a), pointer::press(on_a)], &mut show_frame);
    log.borrow_mut().clear();
    frames.frame(vec![pointer::moved(on_b)], &mut show_frame);
    frames.idle(1, &mut show_frame);
    assert!(log.borrow().frames.iter().all(|reports| !reports[1].root.hovered), "b does not hover while a's press is held");
    assert!(log.borrow().last_root(1).root.contains_pointer, "b contains the pointer");

    frames.frame(vec![pointer::release(on_b)], &mut show_frame);
    frames.idle(1, &mut show_frame);
    assert!(log.borrow().last_root(1).root.hovered, "b hovers once the press ends");
}

/// A pick's modifiers are the ones its release carried: Shift that came after the button went down and is let go in
/// the release's pass belongs to the pick in that pass only; every other pass reads the held modifiers.
#[trial]
fn a_click_carries_the_modifiers_its_release_carried() {
    let seen: Rc<RefCell<Vec<(bool, bool, bool)>>> = Rc::default();
    let sink = seen.clone();
    let mut show = move |ui: &mut Ui| {
        let frame = craft_ui::runtime::pointer::frame(ui.ctx());
        sink.borrow_mut().push((frame.release.is_some(), frame.pick_mods.shift, frame.mods.shift));
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    let at = Pos2::new(150.0, 120.0);
    frames.frame(vec![pointer::moved(at), pointer::press(at)], &mut show);
    seen.borrow_mut().clear();
    let release = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::SHIFT };
    frames.frame(vec![release], &mut show);
    frames.idle(1, &mut show);
    let seen = seen.borrow();
    assert!(seen.iter().any(|(released, ..)| *released), "the release pass ran: {seen:?}");
    for (released, pick, held) in seen.iter() {
        assert!(!held, "no Shift is held: {seen:?}");
        assert_eq!(pick, released, "Shift is the pick's in the release pass only: {seen:?}");
    }
}

/// A press of another button counts in the click run as a primary one does: a primary click, a middle click (a pan the
/// view tools take) and a primary click make the last a triple click, as the old apps counted them. Only primary
/// presses carry the count.
#[trial]
fn a_middle_click_between_primary_clicks_counts_in_the_click_run() {
    let at = Pos2::new(50.0, 50.0);
    let counts: Rc<RefCell<Vec<u32>>> = Rc::default();
    let sink = counts.clone();
    let mut show = move |ui: &mut Ui| {
        if let Some(press) = craft_ui::runtime::pointer::frame(ui.ctx()).press.filter(|press| press.button == craft_behavior::PointerButton::Primary)
        {
            sink.borrow_mut().push(press.clicks);
        }
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    frames.frame(vec![pointer::moved(at)], &mut show);
    for button in ["primary", "middle", "primary"] {
        let (down, up) =
            if button == "middle" { (pointer::middle(at, true), pointer::middle(at, false)) } else { (pointer::press(at), pointer::release(at)) };
        frames.frame(vec![down], &mut show);
        frames.frame(vec![up], &mut show);
    }
    let counts = counts.borrow();
    assert!(counts.len() == 2 && counts[1] == counts[0] + 2, "the middle press counts in the run: {counts:?}");
}

/// A press that became a drag leaves the click run: a press at the same place right after the drag's release is a
/// single press, as only clicks counted toward a double click in the old apps.
#[trial]
fn a_press_after_a_drag_starts_a_new_click_run() {
    let at = Pos2::new(50.0, 50.0);
    let counts: Rc<RefCell<Vec<u32>>> = Rc::default();
    let sink = counts.clone();
    let mut show = move |ui: &mut Ui| {
        if let Some(press) = craft_ui::runtime::pointer::frame(ui.ctx()).press {
            sink.borrow_mut().push(press.clicks);
        }
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    frames.frame(vec![pointer::moved(at), pointer::press(at)], &mut show);
    frames.frame(vec![pointer::moved(at + Vec2::new(60.0, 0.0))], &mut show);
    frames.frame(vec![pointer::moved(at), pointer::release(at)], &mut show);
    for _ in 0..2 {
        frames.frame(vec![pointer::press(at)], &mut show);
        frames.frame(vec![pointer::release(at)], &mut show);
    }
    let counts = counts.borrow();
    assert!(counts.len() == 3 && counts[1] == 1 && counts[2] == 2, "the drag's press is out of the run, the clicks after it count: {counts:?}");
}
