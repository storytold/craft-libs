//! Command resolution runs only when an input of it changed: count trials over fifty shown commands.

use super::kit::{AppCommand, Pane, PaneCommand, Tap};
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _, Tracked};
use craft_ui::runtime::commands::{
    AppCatalog, Binding, CommandDef, CommandGate, CommandInput, CommandRef, CommandSet as _, CommandState, CommandStates, Commands as _, Scope,
};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::runtime::measure::{Resolve, last_pass_resolves};
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Frames, trial};
use egui::{Id, Pos2, Ui};
use std::cell::{Cell, RefCell};

/// The panes: each is an instance of the pane scope, so each is its own reader of the commands its taps show.
const PANES: [&str; 5] = ["a", "b", "c", "d", "e"];

/// The commands the taps of a pane show, each by two taps.
fn shown(at: usize) -> CommandRef {
    match at % 5 {
        0 => AppCommand::Save.into(),
        1 => AppCommand::Go.into(),
        2 => AppCommand::Play.into(),
        3 => AppCommand::Jump.into(),
        _ => PaneCommand::ZoomIn.into(),
    }
}

/// Readers kept: one per pane and command.
const KEPT: u32 = 25;
/// Readers of Save: one per pane.
const SAVE_READERS: u32 = 5;
/// Distinct commands shown.
const COMMANDS: u32 = 5;

/// Fifty taps in five panes; pane `a` publishes Zoom In enabled as `zoom` says. `ids` gets each tap's id.
struct Scene {
    zoom: Cell<bool>,
    ids: RefCell<Vec<Id>>,
}

impl Scene {
    fn new() -> Self {
        Scene { zoom: Cell::new(true), ids: RefCell::new(Vec::new()) }
    }

    fn show(&self, ui: &mut Ui) {
        self.ids.borrow_mut().clear();
        for (pane, salt) in PANES.iter().enumerate() {
            Pane::new().id_salt(*salt).show(ui, |ui, cx| {
                if pane == 0 {
                    cx.publish(PaneCommand::ZoomIn, CommandState::new().enabled(self.zoom.get()));
                }
                for at in 0..10 {
                    let id = Tap::new().id_salt(at).command(shown(at)).show(ui).id;
                    self.ids.borrow_mut().push(id);
                }
            });
        }
    }
}

fn counts(ctx: &egui::Context) -> (u32, u32) {
    (last_pass_resolves(ctx, Resolve::New), last_pass_resolves(ctx, Resolve::Again))
}

/// One frame of `scene`, and its new and repeated resolutions summed over every pass of the frame: each pass after
/// the first reads the one before it as it starts, and the last is read after the frame.
fn frame(frames: &mut Frames, scene: &Scene) -> (u32, u32) {
    let ctx = frames.ctx.clone();
    let (mut new, mut again, mut pass) = (0, 0, 0);
    frames.frame(vec![], |ui| {
        if pass > 0 {
            let (n, a) = counts(&ctx);
            (new, again) = (new + n, again + a);
        }
        pass += 1;
        scene.show(ui);
    });
    let (n, a) = counts(&ctx);
    (new + n, again + a)
}

thread_local! {
    static ASKS: Cell<u32> = const { Cell::new(0) };
}

/// The gate: refuses Save while `refuses`; counts its asks.
fn refuse_save(refuses: &bool, command: &CommandRef) -> bool {
    ASKS.with(|asks| asks.set(asks.get() + 1));
    !(*refuses && AppCommand::from_ref(command) == Some(AppCommand::Save))
}

fn gated(states: &Tracked<CommandStates>, refuses: bool) -> CommandInput {
    CommandInput::new().states(states).gate(CommandGate::new(refuses, refuse_save))
}

/// An idle frame with fifty shown commands resolves nothing, and neither does a pointer move over none of them.
#[trial]
fn an_idle_frame_resolves_no_command() {
    let scene = Scene::new();
    let mut frames = Frames::new();
    let states = Tracked::new(CommandStates::new());
    frames.commands(gated(&states, false));
    frames.idle(3, |ui| scene.show(ui));
    assert_eq!(scene.ids.borrow().len(), 50, "fifty commands are shown");
    assert_eq!(frames.frame(vec![], |ui| scene.show(ui)), 1, "an idle frame takes one pass");
    assert_eq!(counts(&frames.ctx), (0, 0), "an idle frame resolves nothing");
    assert_eq!(frames.frame(vec![moved(Pos2::new(900.0, 900.0))], |ui| scene.show(ui)), 1, "so does a pointer move");
    assert_eq!(counts(&frames.ctx), (0, 0), "a pointer move away from the taps resolves nothing");
}

/// Editing one app-scope state resolves again only that command's readers, once.
#[trial]
fn one_state_change_resolves_only_its_readers() {
    let scene = Scene::new();
    let mut frames = Frames::new();
    let mut states = Tracked::new(CommandStates::new());
    frames.commands(gated(&states, false));
    frames.idle(3, |ui| scene.show(ui));
    states.set(AppCommand::Save, CommandState::new().enabled(false));
    frames.commands(gated(&states, false));
    assert_eq!(frame(&mut frames, &scene), (0, SAVE_READERS), "only Save's readers resolve again");
    assert_eq!(frame(&mut frames, &scene), (0, 0), "the next frame resolves nothing");
}

/// An instance publishing a new state resolves again only the readers of that instance's state.
#[trial]
fn a_published_state_change_resolves_only_its_readers() {
    let scene = Scene::new();
    let mut frames = Frames::new();
    let states = Tracked::new(CommandStates::new());
    frames.commands(gated(&states, false));
    frames.idle(3, |ui| scene.show(ui));
    scene.zoom.set(false);
    assert_eq!(frame(&mut frames, &scene), (0, 1), "only pane a's Zoom In reader resolves again");
    assert_eq!(frame(&mut frames, &scene), (0, 0), "publishing the same state again resolves nothing");
}

/// A change of the gate's props asks the gate once per shown command and resolves again only the readers whose answer
/// changed; a new gate with the same props asks nothing.
#[trial]
fn a_gate_props_change_resolves_only_the_readers_whose_answer_changed() {
    let scene = Scene::new();
    let mut frames = Frames::new();
    let states = Tracked::new(CommandStates::new());
    frames.commands(gated(&states, false));
    frames.idle(3, |ui| scene.show(ui));
    ASKS.with(|asks| asks.set(0));
    frames.commands(gated(&states, true));
    assert_eq!(frame(&mut frames, &scene), (0, SAVE_READERS), "only Save's readers, which the gate now refuses, resolve again");
    assert_eq!(ASKS.with(Cell::get), COMMANDS, "the gate is asked once per shown command");
    ASKS.with(|asks| asks.set(0));
    frames.commands(gated(&states, true));
    assert_eq!(frame(&mut frames, &scene), (0, 0), "a gate with the same props resolves nothing");
    assert_eq!(ASKS.with(Cell::get), 0, "and is not asked");
}

/// Moving the focus between taps resolves none of them: each finds its command's instance in its own chain, or the
/// command is the app's, so none reads the focus.
#[trial]
fn a_focus_move_resolves_no_reader_that_does_not_read_the_focus() {
    let scene = Scene::new();
    let mut frames = Frames::new();
    let states = Tracked::new(CommandStates::new());
    frames.commands(gated(&states, false));
    frames.idle(3, |ui| scene.show(ui));
    for at in [0, 17, 42] {
        let id = scene.ids.borrow()[at];
        request(&frames.ctx, FocusTarget::Id(id));
        for _ in 0..2 {
            assert_eq!(frame(&mut frames, &scene), (0, 0), "focusing tap {at} resolves nothing");
        }
    }
}

/// The testing check every 100th pass compares every kept resolution with a full one, over state, gate, keymap,
/// publish and focus changes; a kept resolution that missed an input fails it.
#[trial]
fn the_hundredth_pass_check_finds_every_kept_resolution_current() {
    let scene = Scene::new();
    let mut frames = Frames::new();
    let mut states = Tracked::new(CommandStates::new());
    let mut refuses = false;
    // A new catalog generation rebuilds the keymap.
    let mut generation = 1;
    let input = |states: &Tracked<CommandStates>, refuses: bool, generation: u64| {
        let extra = CommandDef::new(CommandRef::new("trial.extra"), "Extra", Scope::app())
            .with_default(Binding::parse(if generation % 2 == 0 { "F7" } else { "F6" }).expect("the trial's chord parses"));
        gated(states, refuses).catalog(AppCatalog::new(vec![extra], generation).expect("the trial's catalog is valid"))
    };
    frames.commands(input(&states, refuses, generation));
    let mut checked = Vec::new();
    for frame in 0..250 {
        match frame % 50 {
            10 => states.set(AppCommand::Go, CommandState::new().checked(frame % 100 == 10)),
            20 => refuses = !refuses,
            30 => generation += 1,
            40 => scene.zoom.set(!scene.zoom.get()),
            45 => {
                let id = scene.ids.borrow()[frame % 50];
                request(&frames.ctx, FocusTarget::Id(id));
            }
            _ => {}
        }
        frames.commands(input(&states, refuses, generation));
        frames.frame(vec![], |ui| scene.show(ui));
        let compared = last_pass_resolves(&frames.ctx, Resolve::Checked);
        if compared > 0 {
            checked.push(compared);
        }
    }
    assert_eq!(checked, [KEPT, KEPT], "two checks ran, each over every kept resolution");
}
