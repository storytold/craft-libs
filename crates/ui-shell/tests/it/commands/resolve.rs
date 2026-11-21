//! Command resolution runs only when an input of it changed: count trials over fifty shown commands.

use super::kit::{AppCommand, DOCK, Dock, PANE, Pane, PaneCommand, Tap};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, MemoProps, MemoView, Trigger};
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _, Tracked};
use craft_ui::dialog::Dialog;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::runtime::commands::{
    AppCatalog, Binding, CommandDef, CommandGate, CommandInput, CommandRef, CommandSet as _, CommandState, CommandStates, Commands as _, Scope,
};
use craft_ui::runtime::commands::{EditCommands, declare_child, resolve, resolve_beneath};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::runtime::measure::last_pass_wake_walks;
use craft_ui::runtime::measure::{Resolve, last_pass_resolves};
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::pointer::{press, release};
use craft_ui::testing::{Frames, MemoRun, last_frame_memo_runs, trial};
use craft_ui::text::TextField;
use egui::{Id, Pos2, Ui};
use egui::{Key, Modifiers, ViewportId};
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

/// Fifty taps in five panes; pane `a` publishes Zoom In enabled as `zoom` says. `ids` gets each tap's id. With
/// `global`, a surface with no component of its own reads Zoom In after the panes, which picks an instance by the focus.
struct Scene {
    zoom: Cell<bool>,
    ids: RefCell<Vec<Id>>,
    global: bool,
    /// Pane `a` is left out of the next pass.
    skip_a: Cell<bool>,
    /// Salts the panes' ids, so two windows show panes of their own.
    window: &'static str,
}

impl Scene {
    fn new() -> Self {
        Scene { zoom: Cell::new(true), ids: RefCell::new(Vec::new()), global: false, skip_a: Cell::new(false), window: "" }
    }

    fn show(&self, ui: &mut Ui) {
        self.ids.borrow_mut().clear();
        let skip_a = self.skip_a.replace(false);
        for (pane, salt) in PANES.iter().enumerate() {
            if pane == 0 && skip_a {
                continue;
            }
            Pane::new().id_salt((self.window, *salt)).show(ui, |ui, cx| {
                if pane == 0 {
                    cx.publish(PaneCommand::ZoomIn, CommandState::new().enabled(self.zoom.get()));
                }
                for at in 0..10 {
                    let id = Tap::new().id_salt(at).command(shown(at)).show(ui).id;
                    self.ids.borrow_mut().push(id);
                }
            });
        }
        if self.global {
            resolve(ui.ctx(), PaneCommand::ZoomIn);
        }
    }
}

fn counts(ctx: &egui::Context) -> (u32, u32) {
    (last_pass_resolves(ctx, Resolve::New), last_pass_resolves(ctx, Resolve::Again))
}

/// One frame of `scene`, and its new and repeated resolutions summed over every pass of the frame: each pass after
/// the first reads the one before it as it starts, and the last is read after the frame.
fn frame(frames: &mut Frames, scene: &Scene) -> (u32, u32) {
    let (new, again, _) = frame_walks(frames, ViewportId::ROOT, &mut |ui| scene.show(ui));
    (new, again)
}

/// One frame of `viewport` running `show`, and its new and repeated resolutions and the kept resolutions its wakes
/// walked, summed over every pass of the frame.
fn frame_walks(frames: &mut Frames, viewport: ViewportId, show: &mut dyn FnMut(&mut Ui)) -> (u32, u32, u32) {
    let ctx = frames.ctx.clone();
    let (mut new, mut again, mut walks, mut pass) = (0, 0, 0, 0);
    let mut add = |ctx: &egui::Context| {
        let (n, a) = counts(ctx);
        (new, again, walks) = (new + n, again + a, walks + last_pass_wake_walks(ctx));
    };
    frames.frame_in_viewport(viewport, vec![], |ui| {
        if pass > 0 {
            add(&ctx);
        }
        pass += 1;
        show(ui);
    });
    add(&ctx);
    (new, again, walks)
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

/// Two windows with frames alternating between them, idle: each frame resolves nothing and walks nothing, as the
/// child stays open across the root's frames and the root's across the child's.
#[trial]
fn idle_frames_alternating_between_two_windows_resolve_nothing() {
    let root = Scene::new();
    let other = Scene { window: "child", ..Scene::new() };
    let child = ViewportId::from_hash_of("second window");
    let mut frames = Frames::new();
    let states = Tracked::new(CommandStates::new());
    frames.commands(gated(&states, false));
    for _ in 0..3 {
        frame_walks(&mut frames, ViewportId::ROOT, &mut |ui| root.show(ui));
        frame_walks(&mut frames, child, &mut |ui| other.show(ui));
    }
    for round in 0..4 {
        assert_eq!(frame_walks(&mut frames, ViewportId::ROOT, &mut |ui| root.show(ui)), (0, 0, 0), "root frame {round}");
        assert_eq!(frame_walks(&mut frames, child, &mut |ui| other.show(ui)), (0, 0, 0), "child frame {round}");
    }
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
    assert_eq!(
        frame_walks(&mut frames, ViewportId::ROOT, &mut |ui| scene.show(ui)),
        (0, SAVE_READERS, SAVE_READERS),
        "the wake walks and resolves again only Save's readers"
    );
    assert_eq!(frame_walks(&mut frames, ViewportId::ROOT, &mut |ui| scene.show(ui)), (0, 0, 0), "the next frame walks nothing");
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
    // The publish runs before its reader, which resolves again as it reads; a wake walks only that reader.
    let (new, again, first) = frame_walks(&mut frames, ViewportId::ROOT, &mut |ui| scene.show(ui));
    let (_, _, second) = frame_walks(&mut frames, ViewportId::ROOT, &mut |ui| scene.show(ui));
    assert_eq!((new, again), (0, 1), "only pane a's Zoom In reader resolves again");
    assert_eq!(first + second, 1, "the wakes walk only the reader of the published key");
    assert_eq!(frame_walks(&mut frames, ViewportId::ROOT, &mut |ui| scene.show(ui)), (0, 0, 0), "publishing the same state again walks nothing");
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

/// The catalog of the param trial: one app command, which the refs below carry different params for.
fn param_catalog() -> AppCatalog {
    let command = CommandDef::new(CommandRef::new("trial.param"), "Param", Scope::app());
    AppCatalog::new(vec![command], 1).expect("the trial's catalog is valid")
}

/// Whether each of `refs` resolves enabled after two frames with `states`.
fn enabled_of(frames: &mut Frames, states: &Tracked<CommandStates>, refs: &[CommandRef; 3]) -> [bool; 3] {
    frames.commands(CommandInput::new().states(states).catalog(param_catalog()));
    let seen = Cell::new([true; 3]);
    for _ in 0..2 {
        frames.frame(vec![], |ui| seen.set(std::array::from_fn(|at| resolve(ui.ctx(), refs[at].clone()).enabled)));
    }
    seen.get()
}

/// The app's state of a ref with params answers only that ref; a state with none answers every ref of the id.
#[trial]
fn app_states_answer_each_ref_by_its_params() {
    // PLAN: needs `serde_json = { workspace = true }` in this crate's dev-dependencies (crates/ui-shell/Cargo.toml).
    let refs = [
        CommandRef::with_params("trial.param", serde_json::json!({"a": 1})),
        CommandRef::with_params("trial.param", serde_json::json!({"a": 2})),
        CommandRef::new("trial.param"),
    ];
    let mut frames = Frames::new();
    let mut states = Tracked::new(CommandStates::new());
    states.set(refs[0].clone(), CommandState::new().enabled(false));
    assert_eq!(enabled_of(&mut frames, &states, &refs), [false, true, true], "only the ref the state was set for is disabled");
    states.set(CommandRef::new("trial.param"), CommandState::new().enabled(false));
    assert_eq!(enabled_of(&mut frames, &states, &refs), [false, false, false], "a state with no params disables every ref of the id");
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

/// An instance that misses exactly one pass and registers again is live again for every kept resolution in the
/// frame it comes back in: its return moves the inputs every resolution reads.
#[trial]
fn an_instance_back_after_one_missed_pass_is_live_again() {
    let shown = Cell::new(true);
    let enabled = Cell::new(false);
    let mut show = |ui: &mut Ui| {
        if shown.get() {
            Pane::new().id_salt("b").show(ui, |ui, _| {
                Tap::new().id_salt("zoom").command(PaneCommand::ZoomIn).show(ui);
            });
        }
        enabled.set(resolve(ui.ctx(), PaneCommand::ZoomIn).enabled);
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    assert!(enabled.get(), "Zoom In reaches the pane");
    shown.set(false);
    assert_eq!(frames.frame(vec![], &mut show), 1, "the pane misses one pass");
    shown.set(true);
    frames.frame(vec![], &mut show);
    assert!(enabled.get(), "the pane is live again in the frame it comes back in");
    frames.frame(vec![], &mut show);
    assert!(enabled.get(), "and stays so");
}

/// A view that shows Cut, a command the focused stop answers for.
#[derive(MemoProps)]
struct Edits {}

fn edits(_: &mut Edits, ui: &mut Ui) -> Id {
    Button::command(EditCommands::Cut).show(ui).id
}

/// Two idle frames of `show`, and how many times the `Edits` view ran in them.
fn edits_runs(frames: &mut Frames, show: &mut impl FnMut(&mut Ui)) -> usize {
    (0..2)
        .map(|_| {
            frames.frame(vec![], &mut *show);
            last_frame_memo_runs(&frames.ctx).iter().filter(|(name, _, run)| name.ends_with("Edits") && *run == MemoRun::Ran).count()
        })
        .sum()
}

/// A memoized view that shows a command the focused stop answers for (Cut, which a text field takes) runs when that
/// stop's answer changes: with the focus moving from the field to a tap, which takes no edits, Cut no longer waits for a
/// selection.
#[trial]
fn a_memoized_view_runs_when_the_focused_stops_answer_changes() {
    let text = RefCell::new("hello".to_owned());
    let (field, tap, cut) = (Cell::new(Id::NULL), Cell::new(Id::NULL), Cell::new(Id::NULL));
    let mut show = |ui: &mut Ui| {
        field.set(TextField::new("Name", Bind::bound(&mut *text.borrow_mut())).show(ui).response.id);
        tap.set(Tap::new().id_salt("plain").show(ui).id);
        if let Some(id) = MemoView::new("edits", Edits {}, edits).show(ui) {
            cut.set(id);
        }
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    request(&frames.ctx, FocusTarget::Id(field.get()));
    frames.idle(30, &mut show);
    assert!(frames.tree().is_disabled(cut.get()), "Cut does nothing with no text selected");
    assert_eq!(edits_runs(&mut frames, &mut show), 0, "the view replays");
    request(&frames.ctx, FocusTarget::Id(tap.get()));
    assert!(edits_runs(&mut frames, &mut show) > 0, "the new answer for Cut runs the view");
    assert!(!frames.tree().is_disabled(cut.get()), "which shows Cut enabled");
}

/// A kept resolution of a command the focused stop answers for (Cut, which a text field takes) follows that stop's
/// answer as the pass's end hook records it, so a read between passes (a test after its frames) is current: with the
/// field's text selected, Cut is enabled at once, not only once another pass begins.
#[trial]
fn a_read_between_passes_follows_the_focused_stops_answer() {
    let text = RefCell::new("hello".to_owned());
    let field = Cell::new(Id::NULL);
    let mut show = |ui: &mut Ui| {
        field.set(TextField::new("Name", Bind::bound(&mut *text.borrow_mut())).show(ui).response.id);
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    request(&frames.ctx, FocusTarget::Id(field.get()));
    frames.idle(30, &mut show);
    assert!(!resolve(&frames.ctx, EditCommands::Cut).enabled, "Cut does nothing with no text selected");
    frames.frame(vec![key_press(Key::A, Modifiers::COMMAND)], &mut show);
    // The field's new answer reaches the focus record in the next pass's end hook; no pass has begun since.
    frames.frame(vec![], &mut show);
    assert!(resolve(&frames.ctx, EditCommands::Cut).enabled, "Cut is enabled with the text selected");
}

/// The testing check every 100th pass compares every kept resolution with a full one, and every kept gate answer with
/// the gate's. The script changes, a few passes before each check, every kind of input a resolution reads: app and
/// published states, the gate, the keymap, the focus and the focused stop's edits, a menu and a modal layer with readers
/// inside (a palette's reader under the layer), an instance that misses one pass and a scope's parent.
#[trial]
fn the_hundredth_pass_check_finds_every_kept_resolution_current() {
    let scene = Scene { global: true, ..Scene::new() };
    let text = RefCell::new("hello".to_owned());
    let (field, opener) = (Cell::new(Id::NULL), Cell::new(Pos2::ZERO));
    let modal = Cell::new(false);
    let mut show = |ui: &mut Ui| {
        scene.show(ui);
        field.set(TextField::new("Name", Bind::bound(&mut *text.borrow_mut())).show(ui).response.id);
        Tap::new().id_salt("cut").command(EditCommands::Cut).show(ui);
        Dock::new().id_salt("dock").show(ui, |ui, _| {
            Tap::new().id_salt("zoom").command(PaneCommand::ZoomIn).show(ui);
        });
        let open = Tap::new().id_salt("opener").show(ui);
        opener.set(open.rect.center());
        Menu::new("Menu").opened_by(&open, Trigger::Click).show(ui, |ui, _| {
            MenuItem::command(AppCommand::Save).show(ui);
            MenuItem::command(EditCommands::Cut).show(ui);
        });
        let mut open = modal.get();
        Dialog::new("Options").opened(Bind::bound(&mut open)).buttons([FooterButton::new("OK", DialogRole::Accept)]).show(ui, |ui, cx| {
            cx.publish(EditCommands::Undo, CommandState::new());
            resolve_beneath(ui.ctx(), AppCommand::Save);
            Tap::new().id_salt("in dialog").command(EditCommands::Undo).show(ui);
        });
        modal.set(open);
    };
    let mut frames = Frames::new();
    let mut states = Tracked::new(CommandStates::new());
    let mut refuses = false;
    // A new catalog generation rebuilds the keymap.
    let mut generation = 1;
    let input = |states: &Tracked<CommandStates>, refuses: bool, generation: u64| {
        let extra = CommandDef::new(CommandRef::new("trial.extra"), "Extra", Scope::app())
            .with_default(Binding::parse(if generation.is_multiple_of(2) { "F7" } else { "F6" }).expect("the trial's chord parses"));
        gated(states, refuses).catalog(AppCatalog::new(vec![extra], generation).expect("the trial's catalog is valid"))
    };
    frames.commands(input(&states, refuses, generation));
    // Each step at the pass it names, relative to a check at 100 and 200.
    let script: Vec<(usize, &str)> = [100, 200]
        .into_iter()
        .flat_map(|check| {
            let steps: &[(usize, &str)] = if check == 100 {
                &[(14, "focus field"), (12, "select all"), (10, "press opener"), (9, "release opener"), (6, "parent"), (3, "skip a"), (1, "state")]
            } else {
                &[(14, "focus field"), (12, "collapse"), (10, "modal"), (8, "keymap"), (6, "gate"), (3, "skip a"), (1, "publish")]
            };
            steps.iter().map(move |(before, step)| (check - before, *step)).collect::<Vec<_>>()
        })
        .chain([(103, "escape"), (203, "close modal")])
        .collect();
    let mut script = script.into_iter().peekable();
    let (mut passes, mut checked) = (0, Vec::new());
    while passes < 260 {
        let mut events = Vec::new();
        if let Some((_, step)) = script.next_if(|(at, _)| passes + 1 >= *at) {
            match step {
                "focus field" => {
                    request(&frames.ctx, FocusTarget::Id(field.get()));
                }
                "select all" => events.push(key_press(Key::A, Modifiers::COMMAND)),
                "collapse" => events.push(key_press(Key::ArrowRight, Modifiers::NONE)),
                "press opener" => events.push(press(opener.get())),
                "release opener" => events.push(release(opener.get())),
                "escape" => events.push(key_press(Key::Escape, Modifiers::NONE)),
                "skip a" => scene.skip_a.set(true),
                "parent" => declare_child(&frames.ctx, DOCK, PANE),
                "state" => states.set(AppCommand::Go, CommandState::new().checked(true)),
                "gate" => refuses = !refuses,
                "keymap" => generation += 1,
                "publish" => scene.zoom.set(!scene.zoom.get()),
                "modal" => modal.set(true),
                _ => modal.set(false),
            }
        }
        frames.commands(input(&states, refuses, generation));
        passes += frames.frame(events, &mut show);
        let compared = last_pass_resolves(&frames.ctx, Resolve::Checked);
        if compared > 0 {
            checked.push(compared);
        }
    }
    assert_eq!(script.next().map(|(_, step)| step), None, "every step ran");
    assert_eq!(checked.len(), 2, "two checks ran: {checked:?}");
    assert!(checked.iter().all(|compared| *compared > KEPT), "each compared the panes' readers and the script's: {checked:?}");
}
