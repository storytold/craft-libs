//! A key press passes over a binding whose command is disabled where the press lands, and an app's catalog replaces
//! or drops a linked set's default chords: in both cases the app's own binding of the chord runs. The case is a
//! word processor's Alt+F7 (next misspelling) against the framework's `window.move` default, pressed inside a
//! floating window.

use craft_behavior::Input;
use craft_behavior::{GesturePhase, Sense};
use craft_ui::contract::overlay::{PlacementChange, PlacementKind};
use craft_ui::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, DragBand, HeaderRegion, Mount, Overlay, Props as _, Role, Show as _,
    ShowContainer as _, Sizing,
};
use craft_ui::runtime::commands::{
    AppCatalog, Binding, CommandDef, CommandInput, CommandSet, CommandState, Commands as _, Scope, TypingRule, WindowCommand,
};
use craft_ui::runtime::focus::{FocusTarget, KeyUse, request};
use craft_ui::runtime::frame::take_output;
use craft_ui::style::StyleCx;
use craft_ui::testing::focus::{Stop, named_id, register_stop, widget};
use craft_ui::testing::input::key_press;
use craft_ui::testing::layout::allocate;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use egui::{Event, Key, Modifiers, Pos2, Rect, Response, Ui, Vec2};
use std::cell::RefCell;

/// The app's command on Alt+F7.
const SPELLING: &str = "chordTrial.spelling";
const HEADER: Vec2 = Vec2::new(100.0, 24.0);
const BODY: Vec2 = Vec2::new(120.0, 60.0);
const START: Rect = Rect { min: Pos2::new(60.0, 70.0), max: Pos2::new(280.0, 210.0) };

/// A floating window that can say its Move command is disabled, and logs the placement changes it is given.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Floating<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    move_disabled: bool,
    log: &'a RefCell<Vec<PlacementChange>>,
}

impl Component for Floating<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: FloatingStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Floating<'_> {
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        allocate(ui, HEADER);
        if self.move_disabled {
            cx.publish(WindowCommand::Move, CommandState::new().enabled(false));
        }
    }

    fn placement_changed(&mut self, change: &PlacementChange) {
        self.log.borrow_mut().push(*change);
    }
}

/// What Alt+F7 does with the focus inside a floating window, whose Move is disabled with `move_disabled`, under an app
/// catalog binding it to `SPELLING` that drops `window.move`'s defaults with `dropped`: the commands it fired into
/// the output, and whether a keyboard move began.
fn alt_f7(move_disabled: bool, dropped: bool) -> (Vec<String>, bool) {
    alt_f7_at(Focus::Inside, move_disabled, dropped)
}

/// Where the focus is when Alt+F7 is pressed.
#[derive(Clone, Copy, PartialEq)]
enum Focus {
    /// In the floating window's body.
    Inside,
    /// On a control beside the floating window, in no window's scope.
    Beside,
    /// On a control, with no floating window shown.
    NoWindow,
    /// Nowhere, beside a floating window, under a word processor's typing rule, with the key's text in the press's
    /// frame.
    Nothing,
    /// On a text stop beside a floating window (a word processor's page), as `Nothing` otherwise.
    Typing,
}

impl Focus {
    /// The focus a word processor's page gives: its typing rule, and the text its platform sends with an Alt chord.
    fn page_like(self) -> bool {
        matches!(self, Focus::Nothing | Focus::Typing)
    }
}

/// `alt_f7` with the focus at `focus`.
fn alt_f7_at(focus: Focus, move_disabled: bool, dropped: bool) -> (Vec<String>, bool) {
    let Ok(binding) = Binding::parse("Alt+F7") else { return (vec!["the trial's chord does not parse".to_owned()], false) };
    let Ok(catalog) = AppCatalog::new(vec![CommandDef::new(SPELLING, "Spelling", Scope::app()).with_default(binding)], 1) else {
        return (vec!["the trial's catalog is refused".to_owned()], false);
    };
    let catalog = if dropped { catalog.replace_defaults(WindowCommand::Move.id(), Vec::new()) } else { catalog };
    let (fired, _, began) = press_alt_f7(focus, move_disabled, catalog);
    (fired, began)
}

/// What Alt+F7 does with the focus at `focus` inside or beside a floating window, whose Move is disabled with
/// `move_disabled`, under `catalog`: the commands it fired and the ones it hit disabled into the output, and whether a
/// keyboard move began.
fn press_alt_f7(focus: Focus, move_disabled: bool, catalog: AppCatalog) -> (Vec<String>, Vec<String>, bool) {
    let catalog = if focus.page_like() { catalog.typing_rule(TypingRule::PlainKeysWait) } else { catalog };
    let (log, seen, beside) = (RefCell::new(Vec::new()), RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let page = named_id("page");
    let mut show = |ui: &mut Ui| {
        if focus == Focus::Typing {
            widget(ui, page);
            register_stop(ui.ctx(), Stop::new(page, KeyUse::of(Input::Text)));
        } else {
            Probe::new(&beside).id_salt("beside").size(BODY).sense(Sense::CLICK | Sense::FOCUS).show(ui);
        }
        if focus == Focus::NoWindow {
            return;
        }
        let props = ContainerProps::new()
            .mount(Mount::Layer(Overlay::floating().movable(true).placed(START)))
            .sizing(Sizing::Content)
            .header(HeaderRegion::new().band(DragBand::Move))
            .command_scope(<WindowCommand as CommandSet>::SCOPE);
        Floating { common: Common::new(), props, move_disabled, log: &log }.id_salt("floating").show(ui, |ui, _| {
            Probe::new(&seen).size(BODY).sense(Sense::CLICK | Sense::FOCUS).show(ui);
        });
    };
    let mut frames = Frames::new();
    frames.commands(CommandInput::new().catalog(catalog));
    frames.idle(4, &mut show);
    // The focus goes into the window, where its commands are, or to the control beside it.
    let target = match focus {
        Focus::Inside => seen.borrow().id,
        Focus::Beside | Focus::NoWindow => beside.borrow().id,
        Focus::Typing => Some(page),
        Focus::Nothing => None,
    };
    if let Some(target) = target {
        request(&frames.ctx, FocusTarget::Id(target));
        frames.idle(2, &mut show);
    } else if focus != Focus::Nothing {
        return (vec!["the focus target never showed".to_owned()], Vec::new(), false);
    }
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), target, "the target has the focus");
    take_output(&frames.ctx);
    log.borrow_mut().clear();
    let mut events = vec![key_press(Key::F7, Modifiers::ALT)];
    if focus.page_like() {
        events.push(Event::Text("x".into()));
    }
    frames.frame(events, &mut show);
    frames.idle(2, &mut show);
    let output = take_output(&frames.ctx);
    let ids = |fired: Vec<craft_ui::runtime::commands::Fired>| fired.into_iter().map(|fired| fired.command.id).collect();
    let began = log.borrow().iter().any(|change| change.kind == PlacementKind::Keyboard && change.phase == GesturePhase::Begin);
    (ids(output.fired), ids(output.dimmed), began)
}

/// By default Alt+F7 inside a floating window is its Move (16.3), ahead of the app's binding. Move has no default
/// chord on macOS, so there the app's binding runs.
#[trial]
fn alt_f7_in_a_floating_window_moves_it_by_default() {
    if cfg!(target_os = "macos") {
        assert_eq!(alt_f7(false, false), (vec![SPELLING.to_owned()], false), "on macOS Alt+F7 is the app's");
    } else {
        assert_eq!(alt_f7(false, false), (Vec::new(), true), "the window's Move takes Alt+F7");
    }
}

/// A disabled Move leaves its chord to the next binding: the app's command runs, where the press used to be dropped.
#[trial]
fn a_disabled_binding_leaves_its_chord_to_the_next_one() {
    assert_eq!(alt_f7(true, false), (vec![SPELLING.to_owned()], false), "the disabled Move passes Alt+F7 to the app's command");
}

/// An app that drops `window.move`'s default chords through its catalog gets Alt+F7 for its own command, even where
/// Move is enabled.
#[trial]
fn an_app_catalog_drops_a_linked_set_s_default_chord() {
    assert_eq!(alt_f7(false, true), (vec![SPELLING.to_owned()], false), "with Move's defaults dropped, Alt+F7 is the app's");
}

/// fwstub-1 (3): outside a floating window `window.move` is disabled (no window is around the focus), so it hands
/// Alt+F7 on to the app's enabled binding, with a floating window beside the focus and with none shown, and the app
/// need not drop the default.
#[trial]
fn outside_a_floating_window_alt_f7_is_the_apps() {
    let cases = [
        (Focus::Beside, "beside a floating window"),
        (Focus::NoWindow, "with no floating window"),
        (Focus::Nothing, "with nothing focused"),
        (Focus::Typing, "from a text stop, with the key's text"),
    ];
    for (focus, name) in cases {
        assert_eq!(alt_f7_at(focus, false, false), (vec![SPELLING.to_owned()], false), "{name}: Alt+F7 runs the app's command");
    }
}

/// A chord whose only binding is disabled where the press lands runs nothing, and the output names the command it hit,
/// so the app can say why; a chord that runs its command names none.
#[trial]
fn a_press_on_a_chord_bound_only_to_a_disabled_command_reports_the_command() {
    let move_only = || {
        let binding = Binding::parse("Alt+F7").expect("the trial's chord parses");
        AppCatalog::new(Vec::new(), 1).expect("the trial's catalog is taken").replace_defaults(WindowCommand::Move.id(), vec![binding])
    };
    let moves = vec![WindowCommand::Move.id().to_owned()];
    assert_eq!(press_alt_f7(Focus::Inside, true, move_only()), (Vec::new(), moves, false), "the disabled Move is named and nothing runs");
    assert_eq!(press_alt_f7(Focus::Inside, false, move_only()), (Vec::new(), Vec::new(), true), "an enabled Move moves and is not named");
}
