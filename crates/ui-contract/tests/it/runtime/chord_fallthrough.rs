//! A key press passes over a binding whose command is disabled where the press lands, and an app's catalog replaces
//! or drops a linked set's default chords: in both cases the app's own binding of the chord runs. The case is a
//! word processor's Alt+F7 (next misspelling) against the framework's `window.move` default, pressed inside a
//! floating window. An app catalog's panel-scope commands fire only inside their scope, and a fired chord commits the
//! focused draft first.

use craft_behavior::Input;
use craft_behavior::{EditPolicy, GesturePhase, PartKey, PlainText, Sense};
use craft_ui::contract::overlay::{PlacementChange, PlacementKind};
use craft_ui::contract::{
    Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, DragBand, Edit as _, EditResponse, HeaderRegion, MeasureCx, Mount,
    Overlay, PartsCx, Props as _, Role, Show as _, ShowContainer as _, Sizing, Tracked,
};
use craft_ui::primitives::{TextKey, TextOptions};
use craft_ui::runtime::commands::{
    AppCatalog, Binding, CommandDef, CommandDefError, CommandInput, CommandScope, CommandSet, CommandState, CommandStates, Commands as _,
    EditCommands, FirePhase, Scope, TypingRule, WindowCommand,
};
use craft_ui::runtime::focus::{FocusTarget, KeyUse, request};
use craft_ui::runtime::frame::take_output;
use craft_ui::runtime::windows::{hold_close, root_close_requested};
use craft_ui::style::StyleCx;
use craft_ui::testing::focus::{Stop, key_release, named_id, register_stop, widget};
use craft_ui::testing::input::key_press;
use craft_ui::testing::layout::allocate;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use craft_ui_test_kit::windows::Windows;
use egui::{Event, Id, Key, Modifiers, Pos2, Rect, Response, Ui, Vec2, ViewportEvent, ViewportInfo};
use std::cell::{Cell, RefCell};

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

/// The panel scope of the app's command on F8, as a node editor's canvas declares its own.
const PANEL: CommandScope = CommandScope::new("chordTrial.panel");
const PANEL_COMMAND: &str = "chordTrial.panelCommand";
/// The scope `HoldCommand::Peek`'s held key makes work.
const HELD: CommandScope = CommandScope::new("chordTrial.held");

/// A linked set whose scope is a typing scope: a context of the keymap, which no app def may sit in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui::contract::CommandSet)]
#[command(prefix = "chordTyping", scope = "chordTrial.typing", typing)]
#[non_exhaustive]
enum TypingCommand {
    #[command(label = "Mark")]
    Mark,
}

/// A linked set with a momentary command whose held key makes `HELD` work: a context of the keymap too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui::contract::CommandSet)]
#[command(prefix = "chordHold", scope = "chordTrial.holder")]
#[non_exhaustive]
enum HoldCommand {
    #[command(label = "Peek", kind = "momentary", holds = "chordTrial.held")]
    Peek,
}

/// An inline container, an instance of the scope its props name.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Scoped {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Component for Scoped {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ScopedStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Scoped {}

/// What F8 fires, command and phase, with the keyboard on the control inside the container of `PANEL` (`inside`) or
/// on the one in a plain container beside it, under a catalog that binds F8 to `PANEL_COMMAND` in `PANEL`; with
/// `disabled` the app's states disable the command.
fn press_in_panel(inside: bool, disabled: bool) -> Vec<(String, FirePhase)> {
    fired_in_panel(inside, disabled, false)
}

/// As [`press_in_panel`]; with `hold` the catalog binds a hold of F8 instead, and F8 stays down past the hold delay.
fn fired_in_panel(inside: bool, disabled: bool, hold: bool) -> Vec<(String, FirePhase)> {
    const DELAY: f64 = 0.3;
    let scope = PANEL.scope().expect("the trial's scope name is valid");
    let binding = Binding::parse(if hold { "Hold F8" } else { "F8" }).expect("the trial's chord parses");
    let catalog =
        AppCatalog::new(vec![CommandDef::new(PANEL_COMMAND, "Panel command", scope).with_default(binding)], 1).expect("a panel-scope def is taken");
    let mut states = CommandStates::new();
    if disabled {
        states.set(PANEL_COMMAND, CommandState::new().enabled(false));
    }
    let (scoped, plain) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let mut show = |ui: &mut Ui| {
        Scoped { common: Common::new(), props: ContainerProps::new().command_scope(PANEL) }.id_salt("scoped").show(ui, |ui, _| {
            Probe::new(&scoped).id_salt("in scope").size(BODY).sense(Sense::CLICK | Sense::FOCUS).show(ui);
        });
        Scoped { common: Common::new(), props: ContainerProps::new() }.id_salt("plain").show(ui, |ui, _| {
            Probe::new(&plain).id_salt("beside").size(BODY).sense(Sense::CLICK | Sense::FOCUS).show(ui);
        });
    };
    let mut frames = Frames::new();
    frames.commands(CommandInput::new().catalog(catalog).states(&Tracked::new(states)).holds(hold.then_some(DELAY)));
    frames.idle(4, &mut show);
    let target = if inside { scoped.borrow().id } else { plain.borrow().id };
    request(&frames.ctx, FocusTarget::Id(target.expect("the probe showed")));
    frames.idle(2, &mut show);
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), target, "the probe has the focus");
    take_output(&frames.ctx);
    frames.frame(vec![key_press(Key::F8, Modifiers::NONE)], &mut show);
    frames.idle(1, &mut show);
    if hold {
        frames.advance(DELAY * 2.0);
        frames.idle(2, &mut show);
        frames.frame(vec![key_release(Key::F8, Modifiers::NONE)], &mut show);
        frames.idle(1, &mut show);
    }
    take_output(&frames.ctx).fired.into_iter().map(|fired| (fired.command.id, fired.phase)).collect()
}

/// sh.C9 for a hold binding: a panel-scope def's hold of F8 runs only while the keyboard is inside a container of its
/// scope; beside it the key is not the hold's and its deadline runs nothing.
#[trial]
fn a_panel_scope_hold_binding_runs_only_while_the_keyboard_is_inside_its_container() {
    let ran = |fired: Vec<(String, FirePhase)>| fired.into_iter().filter(|(command, _)| command == PANEL_COMMAND).count();
    assert_eq!(ran(fired_in_panel(true, false, true)), 1, "inside its container the hold runs the panel command");
    assert_eq!(ran(fired_in_panel(false, false, true)), 0, "in the plain container beside it the hold runs nothing");
}

/// sh.C9: an app catalog's def in a panel scope fires only while the keyboard is inside a container of that scope,
/// reaches the app as a press, and follows the app's state of it; a def in a linked set's held or typing scope is
/// refused.
#[trial]
fn a_panel_scope_app_command_fires_only_while_the_keyboard_is_inside_its_container() {
    let pressed = vec![(PANEL_COMMAND.to_owned(), FirePhase::Press)];
    assert_eq!(press_in_panel(true, false), pressed, "inside its container F8 fires the panel command");
    assert_eq!(press_in_panel(false, false), Vec::new(), "in the plain container beside it F8 fires nothing");
    assert_eq!(press_in_panel(true, true), Vec::new(), "the app's state disables the panel command inside its container too");
    for context in [<TypingCommand as CommandSet>::SCOPE, HELD] {
        let def = CommandDef::new("chordTrial.contextual", "Contextual", context.scope().expect("the trial's scope name is valid"));
        assert!(
            matches!(AppCatalog::new(vec![def], 1), Err(CommandDefError::NotApp { .. })),
            "{}: a def in a linked set's context scope is refused",
            context.name()
        );
    }
}

/// The app's command on F9, a save.
const SAVE: &str = "chordTrial.save";
/// What the person types into the field before the save.
const DRAFT: &str = "draft";
const FIELD_TEXT: PartKey = PartKey::new("text");

/// A one-line text field over a string that keeps its typed text as a draft until the edit ends (`OnCommit`).
#[derive(craft_ui::contract::Component)]
struct Field<'a> {
    #[common]
    common: Common,
    value: Bind<'a, String>,
}

impl Component for Field<'_> {
    const ROLE: Role = Role::TEXT_INPUT;
    const INPUT: Input = Input::Text;
    type Style = PlainStyle;
    type Output = EditResponse<String>;

    fn style(_: &StyleCx<'_>, _: FieldStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(120.0, 24.0)
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let (shown, rect) = (cx.value().cloned().unwrap_or_default(), cx.content_rect);
        cx.text(TextKey::<String>::new(FIELD_TEXT), rect, &PlainText, &shown, &TextOptions::default());
    }

    fn bind(&mut self) -> Option<BindMut<'_, String>> {
        Some(self.value.reborrow())
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<String> {
        cx.edit_response()
    }
}

/// A draft field across a trial's frames: its value, the commits its output carried, and its id.
#[derive(Default)]
struct Draft {
    value: RefCell<String>,
    commits: RefCell<Vec<String>>,
    id: Cell<Option<Id>>,
}

impl Draft {
    fn show(&self, ui: &mut Ui) {
        let mut value = self.value.borrow_mut();
        let out = Field { common: Common::new(), value: Bind::bound(&mut *value).policy(EditPolicy::OnCommit) }.id_salt("draft").show(ui);
        self.commits.borrow_mut().extend(out.events.committed().cloned());
        self.id.set(Some(out.response.id));
    }

    /// The typed text is still a draft: the field committed nothing.
    fn uncommitted(&self) -> bool {
        self.commits.borrow().is_empty() && self.value.borrow().is_empty()
    }
}

/// Focuses the draft field shown by `show` in `frames` and types `DRAFT` into it.
fn type_draft(frames: &mut Frames, draft: &Draft, mut show: impl FnMut(&mut Ui)) {
    frames.idle(2, &mut show);
    request(&frames.ctx, FocusTarget::Id(draft.id.get().expect("the field showed")));
    frames.idle(2, &mut show);
    frames.frame(vec![Event::Text(DRAFT.into())], &mut show);
    frames.idle(1, &mut show);
}

/// sh.C10: a command a chord fires, a held close of the root window and a file drop each commit the focused field's
/// draft before the app reads them, as the press that leaves a field does.
#[trial]
fn a_chord_that_fires_a_command_commits_the_focused_text_draft_first() {
    // A chord: the firing frame runs a second pass, which neither fires nor commits again.
    let draft = Draft::default();
    let discard = Cell::new(false);
    let mut show = |ui: &mut Ui| {
        draft.show(ui);
        if discard.take() {
            ui.ctx().request_discard("the firing frame runs twice");
        }
    };
    let binding = Binding::parse("F9").expect("the trial's chord parses");
    let save = CommandDef::new(SAVE, "Save", Scope::app()).with_default(binding);
    let catalog = AppCatalog::new(vec![save], 1).expect("the trial's catalog is taken");
    let mut frames = Frames::new();
    frames.commands(CommandInput::new().catalog(catalog));
    type_draft(&mut frames, &draft, &mut show);
    assert!(draft.uncommitted(), "the typed text is a draft before the chord");
    take_output(&frames.ctx);
    discard.set(true);
    let passes = frames.frame(vec![key_press(Key::F9, Modifiers::NONE)], &mut show);
    let fired: Vec<String> = take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.id).collect();
    assert!(passes > 1, "the firing frame ran a second pass");
    assert_eq!(fired, [SAVE], "the chord fired the save once");
    assert_eq!(*draft.commits.borrow(), [DRAFT], "the draft committed once, in the frame that fired the save");

    // A close request the app holds, as `embed::logic` does before the UI: the commit lands by the frame after.
    let draft = Draft::default();
    let held = Cell::new(0);
    let mut show = |ui: &mut Ui| {
        if root_close_requested(ui.ctx()) {
            hold_close(ui.ctx());
            held.set(held.get() + 1);
        }
        draft.show(ui);
    };
    let windows = Windows::new();
    for _ in 0..2 {
        windows.frame(Vec::new(), Vec::new(), &mut show);
    }
    request(&windows.ctx, FocusTarget::Id(draft.id.get().expect("the field showed")));
    for _ in 0..2 {
        windows.frame(Vec::new(), Vec::new(), &mut show);
    }
    windows.frame(vec![Event::Text(DRAFT.into())], Vec::new(), &mut show);
    windows.frame(Vec::new(), Vec::new(), &mut show);
    assert!(draft.uncommitted(), "the typed text is a draft before the close");
    windows.root_info(ViewportInfo { events: vec![ViewportEvent::Close], ..ViewportInfo::default() });
    windows.frame(Vec::new(), Vec::new(), &mut show);
    windows.root_info(ViewportInfo::default());
    windows.frame(Vec::new(), Vec::new(), &mut show);
    assert_eq!(held.get(), 1, "the app held the close once");
    assert_eq!(*draft.commits.borrow(), [DRAFT], "the held close committed the draft");

    // A file drop: the drop step runs before the pass's focus stage, and the draft commits in the pass that reads it.
    let draft = Draft::default();
    let mut show = |ui: &mut Ui| draft.show(ui);
    let mut frames = Frames::new();
    type_draft(&mut frames, &draft, &mut show);
    assert!(draft.uncommitted(), "the typed text is a draft before the drop");
    let dropped = std::env::temp_dir().join("chord-trial-drop.txt").to_string_lossy().into_owned();
    frames.drop_files(&[dropped.as_str()]);
    frames.frame(Vec::new(), &mut show);
    assert_eq!(*draft.commits.borrow(), [DRAFT], "the draft committed in the pass that read the drop");

    // A chord the focused field takes itself (Select All) is no command for the app: the draft stays.
    let draft = Draft::default();
    let mut show = |ui: &mut Ui| draft.show(ui);
    let mut frames = Frames::new();
    type_draft(&mut frames, &draft, &mut show);
    frames.frame(vec![key_press(Key::A, Modifiers::COMMAND)], &mut show);
    frames.idle(1, &mut show);
    assert!(draft.uncommitted(), "a chord the field takes itself leaves its draft uncommitted");
}

/// The text the platform's paste event carries.
const PASTED: &str = "pasted words";

/// The web sends a clipboard chord's key press and then the clipboard event for it, in one batch: the command runs
/// once, and a paste carries the event's content.
#[trial]
fn a_web_press_and_its_clipboard_event_run_once() {
    let cases = [(Key::C, Event::Copy, EditCommands::Copy, None), (Key::V, Event::Paste(PASTED.into()), EditCommands::Paste, Some(PASTED))];
    for (key, clip, command, content) in cases {
        let mut frames = Frames::new();
        let mut show = |_: &mut Ui| {};
        frames.idle(2, &mut show);
        take_output(&frames.ctx);
        frames.frame(vec![key_press(key, Modifiers::COMMAND), clip], &mut show);
        frames.idle(1, &mut show);
        let fired: Vec<(String, Option<String>)> = take_output(&frames.ctx)
            .fired
            .into_iter()
            .map(|fired| (fired.command.id, fired.pasted.and_then(|pasted| pasted.text).map(|text| text.to_string())))
            .collect();
        let id = command.id().to_owned();
        assert_eq!(fired, [(id.clone(), content.map(str::to_owned))], "{id}: the press and its clipboard event run the command once");
    }
}
