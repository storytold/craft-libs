//! The frame loop: installed options, the one-press rule, the per-frame output, and the command states a settled
//! memoized view shows.

use craft_ui::contract::{
    CommandSet, Common, Component, Container, ContainerProps, Cx, MemoProps, MemoView, Mount, Overlay, Props as _, Role, Show as _,
    ShowContainer as _,
};
use craft_ui::runtime::commands::{CommandGate, CommandInput, CommandRef};
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, MemoRun, last_frame_memo_runs, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use egui::{Event, Id, Modifiers, PointerButton, Pos2, Rect, Response, Ui, Vec2};
use std::cell::{Cell, RefCell};

const AT: Pos2 = Pos2::new(20.0, 12.0);

fn press(pressed: bool) -> Event {
    Event::PointerButton { pos: AT, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn kinds(events: &[Event]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            Event::PointerButton { pressed: true, .. } => "press",
            Event::PointerButton { pressed: false, .. } => "release",
            Event::PointerMoved(_) => "move",
            _ => "other",
        })
        .collect()
}

/// H0: a pass carries at most one press, as its last pointer event; later events lead the next frame, and a
/// discard pass receives none of the held events.
#[trial]
fn one_press_per_pass_and_a_discard_pass_gets_no_held_events() {
    let mut frames = Frames::new();
    let seen = std::cell::RefCell::new(Vec::<Vec<&'static str>>::new());
    let run = |frames: &mut Frames, events: Vec<Event>, discard_first: bool| {
        seen.borrow_mut().clear();
        let mut first = true;
        frames.frame(events, |ui| {
            seen.borrow_mut().push(kinds(&ui.input(|i| i.events.clone())));
            if discard_first && std::mem::take(&mut first) {
                ui.ctx().request_discard("test");
            }
        });
        seen.borrow().clone()
    };
    assert_eq!(run(&mut frames, vec![Event::PointerMoved(AT), press(true), press(false)], false), vec![vec!["move", "press"]]);
    assert_eq!(run(&mut frames, vec![], false), vec![vec!["release"]], "the release comes one pass after its press");
    assert_eq!(
        run(&mut frames, vec![press(true), press(false), press(true)], true),
        vec![vec!["press"], vec![]],
        "the discard pass receives no held events"
    );
    assert_eq!(run(&mut frames, vec![], false), vec![vec!["release", "press"]], "the held events lead the next frame, and its own press is the last");
    assert_eq!(run(&mut frames, vec![], false), vec![Vec::<&str>::new()]);
}

/// Pass-local state: inside a pass the pipeline looks up no service in context data per run, so a settled pass of
/// forty leaves makes as many lookups as one of ten.
#[trial]
fn a_settled_pass_looks_up_as_many_services_for_forty_leaves_as_for_ten() {
    use craft_ui::runtime::egui::memory::service_lookups;
    use craft_ui_test_kit::contract::Panel;
    let settled_lookups = |leaves: usize| {
        let mut frames = Frames::new();
        let mut show = |ui: &mut egui::Ui| {
            Panel::new().id_salt("leaves").show(ui, |ui, _| {
                for at in 0..leaves {
                    let seen = std::cell::RefCell::new(Seen::default());
                    Probe::new(&seen).label(format!("Leaf {at}")).show(ui);
                }
            });
        };
        for _ in 0..3 {
            frames.frame(vec![], &mut show);
        }
        let before = service_lookups();
        frames.frame(vec![], &mut show);
        service_lookups() - before
    };
    let (ten, forty) = (settled_lookups(10), settled_lookups(40));
    assert_eq!(ten, forty, "context-data lookups grow with the runs: {ten} for ten leaves, {forty} for forty");
}

/// The app command the settled view's probe is bound to; its modal rule is the default one, `Blocked`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "frameTrial")]
#[non_exhaustive]
enum Shown {
    #[command(label = "Apply")]
    Apply,
}

/// A gate that refuses `Shown::Apply` when `refuses`, else lets every command through. Each call makes a new gate.
fn gate(refuses: bool) -> CommandInput {
    CommandInput::new().gate(CommandGate::new(refuses, |refuses: &bool, command: &CommandRef| !*refuses || Shown::from_ref(command).is_none()))
}

/// A modal layer with an empty body.
#[derive(craft_ui::contract::Component)]
#[component(container, unregistered)]
struct Modal {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Component for Modal {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ModalStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(120.0, 60.0))
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Modal {}

thread_local! {
    /// What the probe in the command view recorded when it last ran.
    static COMMAND_SEEN: RefCell<Seen> = RefCell::new(Seen::default());
    /// What the probe in the routed view recorded when it last ran.
    static INSIDE_SEEN: RefCell<Seen> = RefCell::new(Seen::default());
}

/// The calls of every pass of the last frame in which a memoized view ran, measuring runs included.
fn view_runs(frames: &Frames) -> usize {
    last_frame_memo_runs(&frames.ctx).iter().filter(|(_, _, run)| *run == MemoRun::Ran).count()
}

/// The command view: a probe bound to `Shown::Apply`.
#[derive(MemoProps)]
struct CommandView {}

fn command_view(_: &mut CommandView, ui: &mut Ui) {
    COMMAND_SEEN.with(|seen| Probe::new(seen).command(Shown::Apply).show(ui));
}

/// The command view in a region of room `room`, and a modal layer beside it while `open` holds.
struct CommandScene {
    open: Cell<bool>,
    room: Cell<Vec2>,
}

impl CommandScene {
    fn new() -> CommandScene {
        CommandScene { open: Cell::new(false), room: Cell::new(Vec2::new(200.0, 80.0)) }
    }

    fn show(&self, ui: &mut Ui) {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), self.room.get()), |ui| {
            MemoView::new("commands", CommandView {}, command_view).show(ui);
        });
        if self.open.get() {
            let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
            Modal { common: Common::new(), props }.id_salt("modal").show(ui, |_, _| {});
        }
    }

    /// The view's runs in one frame with no input.
    fn frame(&self, frames: &mut Frames) -> usize {
        frames.frame(vec![], |ui| self.show(ui));
        view_runs(frames)
    }

    fn settle(&self, frames: &mut Frames) {
        frames.idle(30, |ui| self.show(ui));
    }

    /// Whether the probe's node is disabled in the last frame's tree.
    fn disabled(&self, frames: &mut Frames) -> bool {
        let id: Id = COMMAND_SEEN.with(|seen| seen.borrow().id.expect("the probe ran"));
        frames.tree().is_disabled(id)
    }
}

/// fx.F23: a settled view shows a command's new enabled state after a frame whose only change is the app's gate, and
/// after a modal layer beside it closes with no input. Each pass resolves again, before any component runs, what every
/// reader was shown, and the settled view whose resolution changed runs. The access tree hides what sits behind an
/// open modal layer, so the region's run under the modal (its room changed) is read only through what it shows once
/// the layer closed.
#[trial]
fn a_command_state_change_with_no_input_repaints_the_settled_region() {
    let scene = CommandScene::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames), 0, "the region settles");
    assert!(!scene.disabled(&mut frames), "Apply starts enabled");

    frames.commands(gate(true));
    assert!(scene.frame(&mut frames) > 0, "the gate refusing Apply runs the settled view");
    assert!(scene.disabled(&mut frames), "the region shows Apply disabled in the frame the gate refuses it");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames), 0, "it settles again");
    assert!(scene.disabled(&mut frames), "a settled frame keeps Apply disabled");
    frames.commands(gate(false));
    scene.frame(&mut frames);
    assert!(!scene.disabled(&mut frames), "the gate letting Apply through shows it enabled");

    scene.settle(&mut frames);
    scene.open.set(true);
    scene.settle(&mut frames);
    scene.room.set(Vec2::new(220.0, 80.0));
    assert!(scene.frame(&mut frames) > 0, "the region runs at its new room, under the modal layer that blocks Apply");
    scene.settle(&mut frames);
    scene.open.set(false);
    scene.frame(&mut frames);
    scene.frame(&mut frames);
    assert!(!scene.disabled(&mut frames), "once the modal layer closed, the region shows Apply enabled");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames), 0, "it settles again");
    assert!(!scene.disabled(&mut frames), "a settled frame keeps Apply enabled");
}

/// fx.F23's count relation: a frame given a new gate closure that answers as the last one did resolves every reader to
/// what it was shown, so no settled view runs, in a frame's first pass or in a discard pass the frame reruns. A gate
/// that answers otherwise runs the region.
#[trial]
fn an_unchanged_gate_reruns_no_settled_region() {
    let scene = CommandScene::new();
    let mut frames = Frames::new();
    frames.commands(gate(false));
    scene.settle(&mut frames);
    for frame in 0..4 {
        frames.commands(gate(false));
        assert_eq!(scene.frame(&mut frames), 0, "frame {frame} with a fresh, equal gate ran the settled view");
    }
    frames.commands(gate(false));
    let mut first = true;
    let passes = frames.frame(vec![], |ui| {
        scene.show(ui);
        if std::mem::take(&mut first) {
            ui.ctx().request_discard("command trial");
        }
    });
    assert_eq!(passes, 2, "the frame reruns its pass");
    assert_eq!(view_runs(&frames), 0, "neither pass of the frame ran the settled view");
    frames.commands(gate(true));
    assert!(scene.frame(&mut frames) > 0, "a gate that refuses the region's command runs it");
}

/// Frames over a given context run on it: its passes are the frames', the runtime is installed on it once, and a
/// second driver over the same context runs on it without installing again.
#[trial]
fn frames_over_a_given_context_run_on_it() {
    let ctx = egui::Context::default();
    let mut frames = Frames::over(ctx.clone());
    let mut ran_on = Vec::new();
    frames.frame(vec![], |ui| ran_on.push(ui.ctx() == &ctx));
    assert!(!ran_on.is_empty() && ran_on.iter().all(|on| *on), "every pass ran on the given context");
    assert!(
        matches!(craft_ui::install(&ctx, craft_ui::testing::install_spec()), Err(craft_ui::runtime::InstallError::AlreadyInstalled)),
        "the driver installed the runtime on the given context"
    );
    let mut again = Frames::over(ctx.clone());
    let mut second = false;
    again.frame(vec![], |ui| second |= ui.ctx() == &ctx);
    assert!(second, "a second driver over the installed context runs on it");
}

/// The routed view: a probe labelled Inside that senses clicks.
#[derive(MemoProps)]
struct Routed {}

fn routed(_: &mut Routed, ui: &mut Ui) {
    INSIDE_SEEN.with(|seen| Probe::new(seen).label("Inside").sense(craft_behavior::Sense::CLICK).show(ui));
}

/// An AccessKit request routed to a component inside a settled memoized view reaches it: the view runs once in the
/// pass that routes a Focus to its probe's node, and not at all in the same frame whose Focus goes to a probe outside
/// it. The probes take no focus, so no focus request runs the view in their place.
#[trial]
fn a_routed_access_request_runs_its_settled_view_once() {
    use craft_ui::testing::focus::{AccessActionKind, access_request};
    let outside = RefCell::new(Seen::default());
    let show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(200.0, 80.0)), |ui| {
            MemoView::new("routed", Routed {}, routed).show(ui);
            Probe::new(&outside).label("Outside").sense(craft_behavior::Sense::CLICK).show(ui);
        });
    };
    let mut frames = Frames::new();
    frames.idle(30, show);
    let inside_id = INSIDE_SEEN.with(|seen| seen.borrow().id.expect("the inside probe ran"));
    let outside_id = outside.borrow().id.expect("the outside probe ran");
    assert_ne!(inside_id, outside_id, "two probes");
    let count = |frames: &mut Frames, target: Id| {
        let passes = frames.frame(vec![access_request(AccessActionKind::Focus, target)], show);
        (view_runs(frames), passes)
    };
    let (elsewhere, _) = count(&mut frames, outside_id);
    assert_eq!(elsewhere, 0, "a Focus routed outside the settled view runs it in no pass");
    frames.idle(30, show);
    let (routed, passes) = count(&mut frames, inside_id);
    assert_eq!(routed, 1, "a Focus routed into the settled view runs it once ({passes} passes in the frame)");
}
