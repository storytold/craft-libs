//! The editor contract: a component with an editor part receives one ordered input stream, with the commands of its
//! scope that keys fired in their place, a read-only editor receives only inputs that change no text, and its visible
//! lines become AccessKit run nodes.

use craft_behavior::{EditCommand, EditorInput, Granularity, PartKey, Sense, SurroundingUnit, TextPos};
use craft_host::target::ShellInfo;
use craft_ui::contract::{CommandSet, Common, Component, Cx, EditorHandle, EditorStep, PaintUi, PartsCx, Props as _, Role, Show as _};
use craft_ui::primitives::{EditorLine, EditorPurpose, EditorRun, EditorState, EditorSurrounding, ImeStamp};
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::runtime::frame::discard_pass;
use craft_ui::style::StyleCx;
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{click, moved, press, release};
use craft_ui::testing::text::ImeSeen;
use craft_ui::testing::windows::FakeSystem;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe as Button, Seen};
use egui::output::{IMESurroundingText, ImeChangeCause};
use egui::{Event, Id, ImeEvent, Key, Modifiers, Pos2, Rect, Response};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

const AT: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(210.0, 90.0) };
/// Where the control after the probe sits, for the trials that Tab away from it.
const NEXT: Rect = Rect { min: Pos2::new(10.0, 110.0), max: Pos2::new(90.0, 130.0) };
const TEXT: &str = "fn main";
const EDITOR: PartKey = PartKey::new("editor");

thread_local! {
    /// Whether the probe's editor part held the key focus in its last pass.
    static EDITOR_FOCUSED: Cell<Option<bool>> = const { Cell::new(None) };
    /// The window focus the pass's input reported, so a trial can tell a window in the background from one with focus.
    static WINDOW_FOCUSED: Cell<Option<bool>> = const { Cell::new(None) };
}

/// An editor that logs the inputs it receives and, while `report` is set, reports its one line: `TEXT`, or the text
/// of its `doc` when it keeps one.
#[derive(craft_ui::contract::Component)]
struct Probe<'a> {
    #[common]
    common: Common,
    log: &'a RefCell<Vec<EditorInput>>,
    report: bool,
    purpose: EditorPurpose,
    doc: Option<&'a Doc>,
    /// The probe is a document canvas's editor: it marks its window as showing a document and keeps Tab.
    canvas: bool,
}

impl<'a> Probe<'a> {
    /// A code editor that logs into `log` and keeps no text.
    fn new(log: &'a RefCell<Vec<EditorInput>>, report: bool) -> Self {
        Probe { common: Common::new(), log, report, purpose: EditorPurpose::Code, doc: None, canvas: false }
    }
}

impl Component for Probe<'_> {
    const ROLE: Role = Role::MULTILINE_TEXT_INPUT;
    const INPUT: craft_ui::contract::Input = craft_ui::contract::Input::Editor;
    const KEYS: KeyUse = KeyUse::CODE;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ProbeStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn keys(&self) -> KeyUse {
        // A prose field leaves Tab to the focus order, so Tab can take the keyboard away from it.
        if self.canvas {
            KeyUse::CODE.with_chrome_key(true)
        } else if self.purpose == EditorPurpose::Code {
            KeyUse::CODE
        } else {
            KeyUse::of(craft_ui::contract::Input::Editor)
        }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.editor(EDITOR, rect, self.purpose);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        EDITOR_FOCUSED.set(cx.editor_handle().map(|handle| handle.focused()));
        WINDOW_FOCUSED.set(Some(cx.core.ctx.input(|input| input.focused)));
        let inputs: Vec<EditorInput> = cx.editor_input().collect();
        if let Some(doc) = self.doc {
            doc.apply(cx, &inputs);
        }
        self.log.borrow_mut().extend(inputs);
    }

    fn paint(&mut self, _: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if !self.report {
            return;
        }
        let text = self.doc.map(|doc| doc.text.borrow());
        let shown = text.as_deref().map_or(TEXT, String::as_str);
        let positions: Vec<f32> = (0..shown.len()).map(|at| at as f32 * 8.0).collect();
        let widths = vec![8.0; shown.len()];
        let runs = [EditorRun::new(0..shown.len(), false, &positions, &widths)];
        let line = Rect::from_min_size(cx.content_rect.min, egui::vec2(cx.content_rect.width(), 16.0));
        let lines = [EditorLine::new(shown, line, line.bottom(), &runs)];
        let end = TextPos::new(0, shown.len());
        let caret = Rect::from_min_size(line.min + egui::vec2(shown.len() as f32 * 8.0, 0.0), egui::vec2(0.0, 16.0));
        let mut state = EditorState::new(caret, &lines, end);
        let Some(doc) = self.doc else {
            cx.editor_state(state);
            return;
        };
        if doc.marks.get() {
            state = state.marked(doc.composing.get());
        } else {
            state.composing = doc.composing.get();
        }
        if let Some(epoch) = doc.epoch.get() {
            state = state.surrounding(epoch, EditorSurrounding::text(shown, shown.len(), shown.len()));
        }
        match cx.editor_handle().filter(|_| doc.body.get()) {
            Some(handle) => body(&handle, &cx.response.ctx, state),
            None => cx.editor_state(state),
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A body inside the probe that has the editor handle and the context but no `Cx`, as a canvas tool's body does.
fn body(handle: &EditorHandle, ctx: &egui::Context, state: EditorState<'_>) {
    handle.report(ctx, state);
}

/// The committed text of a probe that keeps one, and how the probe reports it.
#[derive(Default)]
struct Doc {
    /// The committed text; IME commits and typing append to it.
    text: RefCell<String>,
    /// The epoch of the input-method target the text is reported at; `None` reports no surrounding text.
    epoch: Cell<Option<u64>>,
    /// The probe shows marked text: a preedit started it, a commit or a press in the text ends it.
    composing: Cell<bool>,
    /// The probe reports from a body through `EditorHandle::report` rather than through `Cx::editor_state`.
    body: Cell<bool>,
    /// The probe discards the next pass it runs, so its frame runs a second pass.
    discard: Cell<bool>,
    /// What each IME receipt the probe noted answered.
    receipts: RefCell<Vec<bool>>,
    /// The probe says it draws marked text (`EditorState::marked`).
    marks: Cell<bool>,
    /// The probe refuses an input method's deletion after the caret, as a word processor refuses one into a locked
    /// control: the refusal ends the marked text it shows.
    refuses_after: Cell<bool>,
    /// The probe refuses an input method's byte-counted deletion, as a word processor refuses one inside an equation:
    /// it drops the commit right after it (the refused deletion's replacement) and asks for an interrupt.
    refuses_bytes: Cell<bool>,
}

impl Doc {
    fn new(text: &str, epoch: Option<u64>) -> Doc {
        Doc { text: RefCell::new(text.to_owned()), epoch: Cell::new(epoch), ..Doc::default() }
    }

    /// Applies the pass's inputs: text after an `ImeCommit` is the input method's and notes a receipt, any other text
    /// is the application's.
    fn apply<C: Component>(&self, cx: &Cx<'_, C>, inputs: &[EditorInput]) {
        let mut by_ime = false;
        let mut drop_replacement = false;
        for input in inputs {
            match input {
                EditorInput::DeleteSurrounding { unit: SurroundingUnit::Bytes, .. } if self.refuses_bytes.get() => {
                    drop_replacement = true;
                    self.composing.set(false);
                    cx.ime_interrupt();
                }
                EditorInput::ImeCommit if drop_replacement => {}
                EditorInput::Text(_) if std::mem::take(&mut drop_replacement) => {}
                EditorInput::Preedit { text, .. } => self.composing.set(!text.is_empty()),
                EditorInput::DeleteSurrounding { after, .. } if *after > 0 && self.refuses_after.get() => self.composing.set(false),
                EditorInput::ImeCommit => {
                    by_ime = true;
                    self.composing.set(false);
                }
                EditorInput::Text(typed) => {
                    let mut text = self.text.borrow_mut();
                    let before = stamp(&text);
                    text.push_str(typed);
                    let after = stamp(&text);
                    if std::mem::take(&mut by_ime) {
                        self.receipts.borrow_mut().push(cx.ime_receipt(before, after));
                    } else {
                        cx.ime_application_change();
                    }
                }
                _ => {}
            }
        }
        // A press in the text moves the caret, which ends the marked text the probe shows.
        if cx.response.clicked() {
            self.composing.set(false);
        }
        if self.discard.take() {
            discard_pass(&cx.response.ctx, "trial");
        }
    }
}

/// The receipt stamp of `text` with the caret at its end, where the probe reports it.
fn stamp(text: &str) -> ImeStamp {
    ImeStamp::from_committed(text, text.len(), text.len(), None)
}

/// What one frame of the rig shows, borrowed from the rig apart from its frames.
#[derive(Clone, Copy)]
struct View<'r> {
    log: &'r RefCell<Vec<EditorInput>>,
    id: &'r Cell<Id>,
    read_only: bool,
    purpose: EditorPurpose,
    doc: Option<&'r Doc>,
    next: Option<&'r RefCell<Seen>>,
    canvas: bool,
}

impl View<'_> {
    fn show(self, ui: &mut egui::Ui) {
        let probe = Probe { purpose: self.purpose, doc: self.doc, canvas: self.canvas, ..Probe::new(self.log, true) };
        let response = probe.label("Source").read_only(self.read_only).id_salt("code").show_at(ui, AT);
        self.id.set(response.id);
        if let Some(seen) = self.next {
            let _ = Button::new(seen).id_salt("next").show_at(ui, NEXT);
        }
    }
}

struct Rig {
    frames: Frames,
    log: RefCell<Vec<EditorInput>>,
    id: Cell<Id>,
    read_only: bool,
    purpose: EditorPurpose,
    doc: Option<Doc>,
    /// The control after the probe that Tab takes the keyboard to, when the rig shows one.
    next: Option<RefCell<Seen>>,
    /// The probe is a document canvas's editor.
    canvas: bool,
}

impl Rig {
    fn new() -> Rig {
        Rig::with(EditorPurpose::Code, None, false)
    }

    /// A rig whose probe edits `purpose`, keeps `doc` and, with `next`, has a control after it.
    fn with(purpose: EditorPurpose, doc: Option<Doc>, next: bool) -> Rig {
        let mut rig = Rig {
            frames: Frames::new(),
            log: RefCell::new(Vec::new()),
            id: Cell::new(Id::NULL),
            read_only: false,
            purpose,
            doc,
            next: next.then(|| RefCell::new(Seen::default())),
            canvas: false,
        };
        for _ in 0..3 {
            rig.frame(Vec::new());
        }
        rig
    }

    /// A frame with `events`; returns the passes it ran.
    fn frame(&mut self, events: Vec<Event>) -> usize {
        let view = View {
            log: &self.log,
            id: &self.id,
            read_only: self.read_only,
            purpose: self.purpose,
            doc: self.doc.as_ref(),
            next: self.next.as_ref(),
            canvas: self.canvas,
        };
        self.frames.frame(events, |ui| view.show(ui))
    }

    /// A rig whose probe is a document canvas's editor, with a control after it.
    fn canvas_editor() -> Rig {
        let mut rig = Rig::with(EditorPurpose::Text, None, true);
        rig.canvas = true;
        rig.frame(Vec::new());
        rig
    }

    fn focus(&mut self) {
        let view = View {
            log: &self.log,
            id: &self.id,
            read_only: self.read_only,
            purpose: self.purpose,
            doc: self.doc.as_ref(),
            next: self.next.as_ref(),
            canvas: self.canvas,
        };
        click(&mut self.frames, AT.center(), &mut |ui| view.show(ui));
        self.log.borrow_mut().clear();
    }

    fn doc(&self) -> &Doc {
        self.doc.as_ref().expect("the rig's probe keeps a text")
    }

    /// The IME output of the last frame, which a focused probe always writes.
    fn ime(&self) -> ImeSeen {
        self.frames.ime().cloned().expect("the focused editor writes the IME output")
    }

    /// Whether the last frame reset the input method as egui-winit counts it: the IME coming back on, or an
    /// interrupt while it is on. `on` is whether it was on, and becomes whether it is.
    fn resets_ime(&self, on: &mut bool) -> bool {
        let ime = self.frames.ime();
        let reset = ime.is_some_and(|ime| !*on || ime.interrupt);
        *on = ime.is_some();
        reset
    }

    fn take(&self) -> Vec<EditorInput> {
        self.log.take()
    }

    /// A frame with `events`, and the reset request its IME output carries.
    fn frame_reset(&mut self, events: Vec<Event>) -> Option<egui::ImeResetId> {
        self.frame(events);
        self.frames.ime().and_then(|ime| ime.reset)
    }

    /// Hover, press, release and an idle frame at `at`, and the reset requests they carry.
    fn click_resets(&mut self, at: Pos2) -> Vec<egui::ImeResetId> {
        [vec![moved(at)], vec![press(at)], vec![release(at)], Vec::new()].into_iter().filter_map(|events| self.frame_reset(events)).collect()
    }
}

/// Switching to another app and back is no focus move for an editor: it keeps the key focus, and its input-method
/// context runs on through it, with no new generation to reset the input method when the window comes back.
#[trial]
fn switching_apps_keeps_the_editor_focused_and_its_input_method_context() {
    let mut rig = Rig::with(EditorPurpose::Code, Some(Doc::new(TEXT, Some(1))), false);
    rig.focus();
    rig.frame(Vec::new());
    let generation = rig.ime().generation;
    rig.frame(vec![craft_ui::testing::pointer::window_blurred()]);
    for _ in 0..3 {
        rig.frame(Vec::new());
        assert_eq!(WINDOW_FOCUSED.get(), Some(false), "the window's input reports the background, as a host's does");
        assert_eq!(EDITOR_FOCUSED.get(), Some(true), "the editor keeps the key focus while the window is in the background");
    }
    rig.frame(vec![Event::WindowFocused(true)]);
    rig.frame(Vec::new());
    assert_eq!(WINDOW_FOCUSED.get(), Some(true), "the window has the focus again");
    assert_eq!(rig.ime().generation, generation, "the input method's context ran on through the switch");
    rig.frame(vec![Event::Text("x".into())]);
    assert!(rig.take().iter().any(|input| matches!(input, EditorInput::Text(text) if &**text == "x")), "typing goes on in the editor");
}

/// Shift+Enter reaches an editor as a newline with Shift held, told from a plain Enter in the same frame, in the order
/// they were pressed: an editor gives Shift+Enter its own meaning (a line break within the paragraph). Guards against
/// the press's Shift being lost between the press and the editor input.
#[trial]
fn shift_enter_reaches_an_editor_as_a_newline_with_shift_in_order() {
    let mut rig = Rig::new();
    rig.focus();
    rig.frame(vec![key_press(Key::Enter, Modifiers::SHIFT), key_press(Key::Enter, Modifiers::NONE), key_press(Key::Enter, Modifiers::SHIFT)]);
    let inputs = rig.take();
    assert!(
        matches!(
            inputs.as_slice(),
            [EditorInput::Newline { shift: true, .. }, EditorInput::Newline { shift: false, .. }, EditorInput::Newline { shift: true, .. }]
        ),
        "each Enter keeps its own Shift, in order: {inputs:?}"
    );
}

/// A paste reaches an editor in its place among the frame's typing: text typed before it comes before it and text
/// typed after it comes after. Guards against the clipboard command being routed after all of the frame's keys and
/// texts.
#[trial]
fn a_paste_reaches_an_editor_in_order_with_the_typing_around_it() {
    let mut rig = Rig::new();
    rig.focus();
    rig.frame(vec![Event::Text("a".into()), Event::Paste("b".into()), Event::Text("c".into())]);
    let inputs = rig.take();
    assert!(
        matches!(inputs.as_slice(), [EditorInput::Text(a), EditorInput::Edit(EditCommand::Paste), EditorInput::Text(c)] if &**a == "a" && &**c == "c"),
        "the paste sits between the texts it came between: {inputs:?}"
    );
}

/// Text, Backspace, Tab and Enter reach an editor as one input stream in the order they were typed; a read-only
/// editor still receives moves but none of the inputs that change text.
#[trial]
fn editor_input_reaches_an_editor_part_and_read_only_filtering_drops_edits() {
    let mut rig = Rig::new();
    rig.focus();
    rig.frame(vec![Event::Text("x".into())]);
    rig.frame(vec![key_press(Key::Backspace, Modifiers::NONE)]);
    rig.frame(vec![key_press(Key::Tab, Modifiers::NONE)]);
    rig.frame(vec![key_press(Key::Enter, Modifiers::NONE)]);
    let inputs = rig.take();
    assert!(
        matches!(inputs.as_slice(), [EditorInput::Text(text), EditorInput::Delete { forward: false, .. }, EditorInput::Tab, EditorInput::Newline { shift: false, .. }] if &**text == "x"),
        "the inputs arrive in the pass they are typed, in order: {inputs:?}"
    );

    rig.read_only = true;
    rig.frame(vec![Event::Text("y".into())]);
    rig.frame(vec![key_press(Key::Tab, Modifiers::NONE)]);
    rig.frame(vec![key_press(Key::Backspace, Modifiers::NONE)]);
    assert!(rig.take().is_empty(), "a read-only editor receives no edit");
    rig.frame(vec![key_press(Key::ArrowUp, Modifiers::NONE)]);
    let moves = rig.take();
    assert!(matches!(moves.as_slice(), [EditorInput::Move(..)]), "moves still arrive: {moves:?}");
}

fn preedit(text: &str) -> Event {
    Event::Ime(ImeEvent::Preedit { text: text.to_owned(), active_range_chars: None })
}

/// The IME events of one raw batch reach the editor together, in the pass of the frame they arrived in: the one-press
/// rule holds what follows a pointer press, not IME text, so an editor reads a replacement (a byte deletion and its
/// commit) beside the fresh commit after it and can tell them apart by their order. Events given in frames of their
/// own arrive in passes of their own.
#[trial]
fn one_raw_batch_of_ime_events_reaches_the_editor_in_one_pass() {
    for (name, mut rig) in [("a field's editor", Rig::new()), ("a canvas's editor", Rig::canvas_editor())] {
        rig.focus();
        let batch = vec![Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }), commit("Z"), commit("Y")];
        rig.frame(batch.clone());
        let inputs = rig.take();
        let expected = [
            EditorInput::delete_surrounding_bytes(1, 0),
            EditorInput::ImeCommit,
            EditorInput::Text("Z".into()),
            EditorInput::ImeCommit,
            EditorInput::Text("Y".into()),
        ];
        assert_eq!(inputs, expected, "{name}: the whole batch arrives in its own frame's pass, in order");
        rig.frame(Vec::new());
        assert!(rig.take().is_empty(), "{name}: nothing of the batch is held for a later frame");
        for event in batch {
            rig.frame(vec![event]);
            assert!(!rig.take().is_empty(), "{name}: an event given in its own frame arrives in that frame's pass");
        }
    }
}

/// A reset fences the interrupted composition until its own request completes. Focus leaving an editor while it
/// composes asks for a reset; a pass with no IME output drops the request, which the integration reports retired, so
/// focus coming back asks again. The retired request's completion is stale and lets nothing through; the
/// new request's completion lets the composition that follows it in.
#[trial]
fn a_reset_ends_only_with_the_completion_of_its_own_request() {
    let mut rig = Rig::new();
    rig.focus();
    rig.frame(vec![preedit("k")]);
    assert!(matches!(rig.take().as_slice(), [EditorInput::Preedit { .. }]), "the editor composes");

    let left = rig.click_resets(Pos2::new(400.0, 300.0));
    let [first] = left.as_slice() else { panic!("focus leaving a composition asks for one reset: {left:?}") };
    for _ in 0..2 {
        assert_eq!(rig.frame_reset(Vec::new()), None, "no editor owns the IME, so no reset is asked for");
    }
    let back = rig.click_resets(AT.center());
    let [second] = back.as_slice() else { panic!("focus coming back while the reset is pending asks again: {back:?}") };
    assert_ne!(first, second);
    rig.take();

    rig.frame(vec![Event::ImeResetCompleted { request_id: *first }, preedit("x")]);
    assert!(rig.take().is_empty(), "the retired request's completion is stale: the composition stays fenced");
    rig.frame(vec![Event::ImeResetCompleted { request_id: *second }, preedit("y")]);
    let inputs = rig.take();
    assert!(
        matches!(inputs.as_slice(), [EditorInput::Preedit { text, .. }] if &**text == "y"),
        "the completion of the latest request ends the fence in its place in the stream: {inputs:?}"
    );
}

/// Focus moving from a composing editor to one that reports no caret in that pass interrupts the old editor's
/// composition. The next pass's IME output is the new editor's, a new IME context, so the integration drops the
/// request and reports it retired; the engine then asks again, and the new request's completion gives the new editor
/// its IME.
#[trial]
fn a_reset_the_integration_retires_is_asked_again() {
    const OTHER: Rect = Rect { min: Pos2::new(10.0, 110.0), max: Pos2::new(210.0, 190.0) };
    let (first, second) = (RefCell::new(Vec::new()), RefCell::new(Vec::new()));
    let report = Cell::new(true);
    let mut ui = |ui: &mut egui::Ui| {
        Probe::new(&first, true).label("First").id_salt("first").show_at(ui, AT);
        Probe::new(&second, report.get()).label("Second").id_salt("second").show_at(ui, OTHER);
    };
    let mut frames = Frames::new();
    click(&mut frames, AT.center(), &mut ui);
    frames.frame(vec![preedit("k")], &mut ui);
    assert!(matches!(first.take().as_slice(), [EditorInput::Preedit { .. }]), "the first editor composes");

    report.set(false);
    let mut resets = Vec::new();
    for events in [vec![moved(OTHER.center())], vec![press(OTHER.center())], vec![release(OTHER.center())], Vec::new(), Vec::new(), Vec::new()] {
        frames.frame(events, &mut ui);
        resets.push(frames.ime().and_then(|ime| ime.reset));
        // The new editor reports no caret only in the pass that interrupts.
        if resets.last().is_some_and(Option::is_some) {
            report.set(true);
        }
    }
    let asked: Vec<_> = resets.iter().skip_while(|reset| reset.is_none()).collect();
    let [Some(interrupted), None, Some(again), ..] = asked.as_slice() else {
        panic!(
            "focus leaving the composition asks for a reset, the new editor's next output asks nothing, and the retired request is asked again: {resets:?}"
        )
    };
    assert_ne!(again, interrupted);
    frames.frame(Vec::new(), &mut ui);
    second.take();

    frames.frame(vec![Event::ImeResetCompleted { request_id: *again }, preedit("y")], &mut ui);
    let inputs = second.take();
    assert!(
        matches!(inputs.as_slice(), [EditorInput::Preedit { text, .. }] if &**text == "y"),
        "the new request's completion gives the new editor its IME: {inputs:?}"
    );
}

/// Focus moving from a composing editor to another ends the old one's input: it receives nothing after the move, and
/// the new editor receives nothing of the old composition's late preedit until the reset's own completion arrives.
#[trial]
fn an_owner_change_mid_composition_interrupts_it_and_drops_its_late_input() {
    const OTHER: Rect = Rect { min: Pos2::new(10.0, 110.0), max: Pos2::new(210.0, 190.0) };
    let (first, second) = (RefCell::new(Vec::new()), RefCell::new(Vec::new()));
    let mut ui = |ui: &mut egui::Ui| {
        Probe::new(&first, true).label("First").id_salt("first").show_at(ui, AT);
        Probe::new(&second, true).label("Second").id_salt("second").show_at(ui, OTHER);
    };
    let mut frames = Frames::new();
    click(&mut frames, AT.center(), &mut ui);
    frames.frame(vec![preedit("k")], &mut ui);
    assert!(matches!(first.take().as_slice(), [EditorInput::Preedit { .. }]), "the first editor composes");

    let mut request = None;
    for events in [vec![moved(OTHER.center())], vec![press(OTHER.center())], vec![release(OTHER.center())]] {
        frames.frame(events, &mut ui);
        request = request.or(frames.ime().and_then(|ime| ime.reset));
    }
    let Some(request) = request else { panic!("the owner change asks for a reset") };
    first.take();
    second.take();

    frames.frame(vec![preedit("late")], &mut ui);
    assert!(first.take().is_empty(), "the old editor gets no input after the owner change");
    assert!(second.take().is_empty(), "the old composition's late preedit reaches no editor before the reset completes");

    frames.frame(vec![Event::ImeResetCompleted { request_id: request }, preedit("y")], &mut ui);
    assert!(first.take().is_empty(), "the old editor still gets nothing");
    let inputs = second.take();
    assert!(
        matches!(inputs.as_slice(), [EditorInput::Preedit { text, .. }] if &**text == "y"),
        "after the completion the new editor receives its own preedit: {inputs:?}"
    );
}

/// An editor that stops being shown while it composes is interrupted in the pass it vanished in. egui keeps its focus
/// until the end of that pass, so waiting for the owner change at the next begin interrupted it a frame late. Guards
/// against the interrupt depending on a second pass of the frame.
#[trial]
fn an_editor_that_vanishes_while_composing_is_interrupted_in_that_pass() {
    let log = RefCell::new(Vec::new());
    let shown = Cell::new(true);
    let mut ui = |ui: &mut egui::Ui| {
        if shown.get() {
            Probe::new(&log, true).label("Source").id_salt("code").show_at(ui, AT);
        }
    };
    let mut frames = Frames::new();
    click(&mut frames, AT.center(), &mut ui);
    frames.frame(vec![preedit("k")], &mut ui);
    assert!(matches!(log.take().as_slice(), [EditorInput::Preedit { .. }]), "the editor composes");

    shown.set(false);
    let passes = frames.frame(Vec::new(), &mut ui);
    let seen = frames.ime().cloned();
    assert!(seen.as_ref().is_some_and(|ime| ime.interrupt && ime.reset.is_some()), "the pass the editor vanished in interrupts it: {seen:?}");
    assert_eq!(passes, 1, "the interrupt needs no second pass");
    frames.frame(Vec::new(), &mut ui);
    assert!(frames.ime().is_none(), "with the editor gone, no one owns the IME: {:?}", frames.ime());
}

/// An editor losing focus while it composes asks the system once to discard the marked text: later frames and a retired
/// request asked again add no discard.
#[trial]
fn an_interrupted_composition_asks_the_system_to_discard_its_marked_text_once() {
    let system = Arc::new(FakeSystem::new());
    let mut frames = Frames::uninstalled();
    let installed = craft_ui::install(&frames.ctx, InstallSpec::new(ShellInfo::default(), system.clone()));
    assert!(installed.is_ok(), "the context takes the runtime once");
    let (log, id) = (RefCell::new(Vec::new()), Cell::new(Id::NULL));
    let view = View { log: &log, id: &id, read_only: false, purpose: EditorPurpose::Code, doc: None, next: None, canvas: false };
    let mut ui = |ui: &mut egui::Ui| view.show(ui);
    frames.idle(3, &mut ui);
    click(&mut frames, AT.center(), &mut ui);
    frames.frame(vec![preedit("k")], &mut ui);
    assert_eq!(system.discards(), 0, "a composition under way is not discarded");

    let outside = Pos2::new(400.0, 300.0);
    for events in [vec![moved(outside)], vec![press(outside)], vec![release(outside)]] {
        frames.frame(events, &mut ui);
    }
    frames.idle(3, &mut ui);
    assert_eq!(system.discards(), 1, "focus leaving the composition asks for one discard");
    frames.idle(6, &mut ui);
    assert_eq!(system.discards(), 1, "idle frames and the end hook's re-ask add none");
}

fn commit(text: &str) -> Event {
    Event::Ime(ImeEvent::Commit(text.to_owned()))
}

/// The surrounding text of an IME output with its cursor and anchor; `None` when it is unavailable.
fn window(seen: &ImeSeen) -> Option<(String, usize, usize)> {
    seen.surrounding.available().map(|payload| (payload.text.clone(), payload.cursor, payload.anchor))
}

/// The editor that owns the IME publishes its committed text around the caret with the cursor and anchor offsets. A
/// commit the editor applies with a receipt is the input method's change, also in a frame that runs a second pass,
/// which gets no input and notes no receipt; typing is the application's. Edits keep the context, a new epoch starts
/// a new one, and an owner that reports no surrounding text publishes none.
#[trial]
fn an_editors_surrounding_text_and_receipts_reach_the_ime_output() {
    let mut rig = Rig::with(EditorPurpose::Code, Some(Doc::new(TEXT, Some(1))), false);
    rig.focus();
    rig.frame(Vec::new());
    let seen = rig.ime();
    assert_eq!(window(&seen), Some((TEXT.to_owned(), 7, 7)), "the owner's text and caret reach the output");
    let generation = seen.generation;

    rig.doc().discard.set(true);
    assert_eq!(rig.frame(vec![commit("(")]), 2, "the commit's frame runs a second pass");
    assert_eq!(*rig.doc().receipts.borrow(), [true], "the first pass's receipt continues the observed text; the second notes none");
    let seen = rig.ime();
    assert_eq!(window(&seen), Some(("fn main(".to_owned(), 8, 8)));
    assert_eq!(seen.cause, ImeChangeCause::InputMethod, "the receipt proves the input method's change");
    assert_eq!(seen.generation, generation, "an edit keeps the context");

    rig.frame(vec![Event::Text(")".into())]);
    let seen = rig.ime();
    assert_eq!(window(&seen), Some(("fn main()".to_owned(), 9, 9)));
    assert_eq!(seen.cause, ImeChangeCause::Other, "typing is the application's change");
    assert_eq!(seen.generation, generation, "typing keeps the context");

    rig.doc().epoch.set(Some(2));
    rig.frame(Vec::new());
    let seen = rig.ime();
    assert_ne!(seen.generation, generation, "a new epoch starts a new context");
    assert_eq!(window(&seen), Some(("fn main()".to_owned(), 9, 9)));

    let second = seen.generation;
    rig.doc().epoch.set(None);
    rig.frame(Vec::new());
    let seen = rig.ime();
    assert_eq!(seen.surrounding, IMESurroundingText::Unavailable, "an owner that reports nothing publishes no text");
    assert_eq!(seen.generation, second, "a pass with no report keeps the target's epoch, so the context stays");
}

fn delete_bytes(before: usize) -> Event {
    Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes: before, after_bytes: 0 })
}

/// An editor that refuses an input method's edit asks for the interrupt itself (`Cx::ime_interrupt`). One raw batch of
/// a refused deletion, its replacement commit and a fresh commit ends with nothing composing, so the engine could not
/// see the refusal: the editor's request interrupts, and the fresh commit it applied stays the input method's change.
/// With no composition running, the next frame's commit is not fenced; with one running, the request ends it and its
/// late events are dropped until the reset completes.
#[trial]
fn an_editor_that_refuses_an_input_method_edit_asks_for_the_interrupt() {
    let mut rig = Rig::with(EditorPurpose::Code, Some(Doc::new(TEXT, Some(1))), false);
    rig.doc().refuses_bytes.set(true);
    rig.focus();
    rig.frame(Vec::new());

    rig.frame(vec![delete_bytes(1), commit("Z"), commit("W")]);
    assert_eq!(*rig.doc().text.borrow(), "fn mainW", "the replacement is dropped and the fresh commit applies");
    let seen = rig.ime();
    assert!(seen.interrupt && seen.reset.is_some(), "the editor's refusal interrupts the input method: {seen:?}");
    assert_eq!(seen.cause, ImeChangeCause::InputMethod, "the fresh commit is still the input method's change");

    rig.frame(vec![commit("Y")]);
    assert_eq!(*rig.doc().text.borrow(), "fn mainWY", "with nothing composing, the next commit is not fenced");
    let seen = rig.ime();
    assert!(!seen.interrupt, "the interrupt is asked once");
    assert_eq!(seen.cause, ImeChangeCause::InputMethod);

    rig.frame(vec![preedit("p")]);
    let reset = rig.frame_reset(vec![delete_bytes(1)]).expect("a refusal while composing interrupts and asks for a reset");
    rig.frame(vec![preedit("pq"), commit("pq")]);
    assert_eq!(*rig.doc().text.borrow(), "fn mainWY", "the interrupted composition's late events are fenced");
    rig.frame(vec![Event::ImeResetCompleted { request_id: reset }]);
    rig.frame(vec![commit("V")]);
    assert_eq!(*rig.doc().text.borrow(), "fn mainWYV", "after the reset, input applies again");
}

/// While the editor composes, and for one guard frame after a commit, Enter, Space, Backspace, Escape and Delete
/// never become editor inputs and do not interrupt: the composition goes on. A press in the editor's text does
/// interrupt it: the press moves the caret, the editor ends the marked text it showed, and the engine asks for one
/// reset and fences the composition's late events.
#[trial]
fn keys_that_end_a_composition_do_not_reach_the_editor_while_it_composes() {
    let mut rig = Rig::with(EditorPurpose::Code, Some(Doc::new(TEXT, None)), false);
    rig.focus();
    rig.frame(vec![preedit("k")]);
    assert!(matches!(rig.take().as_slice(), [EditorInput::Preedit { .. }]), "the editor composes");
    for key in [Key::Enter, Key::Space, Key::Backspace, Key::Escape, Key::Delete] {
        assert_eq!(rig.frame_reset(vec![key_press(key, Modifiers::NONE)]), None, "{key:?} interrupts the composition");
        let inputs = rig.take();
        assert!(inputs.is_empty(), "{key:?} pressed while composing reaches the editor: {inputs:?}");
    }
    rig.frame(vec![preedit("ka")]);
    let inputs = rig.take();
    assert!(matches!(inputs.as_slice(), [EditorInput::Preedit { text, .. }] if &**text == "ka"), "the composition goes on: {inputs:?}");

    rig.frame(vec![commit("ka")]);
    let inputs = rig.take();
    assert!(matches!(inputs.as_slice(), [EditorInput::ImeCommit, EditorInput::Text(text)] if &**text == "ka"), "the commit lands: {inputs:?}");
    rig.frame(vec![key_press(Key::Enter, Modifiers::NONE)]);
    let inputs = rig.take();
    assert!(inputs.is_empty(), "the Enter that confirmed the commit, a frame late, is no newline: {inputs:?}");
    rig.frame(vec![key_press(Key::Enter, Modifiers::NONE)]);
    let inputs = rig.take();
    assert!(matches!(inputs.as_slice(), [EditorInput::Newline { .. }]), "after the guard frame Enter is a newline: {inputs:?}");

    rig.frame(vec![preedit("x")]);
    assert!(matches!(rig.take().as_slice(), [EditorInput::Preedit { .. }]), "a new composition starts");
    let resets = rig.click_resets(AT.center());
    assert_eq!(resets.len(), 1, "a press in the text interrupts the composition with one reset: {resets:?}");
    rig.frame(vec![preedit("xy")]);
    let inputs = rig.take();
    assert!(inputs.is_empty(), "the interrupted composition's late preedit is fenced: {inputs:?}");
}

/// An empty commit with nothing marked is no edit, so a selection (the Type tool's placeholder) survives it and the
/// Enter after it is a newline; an empty commit that ends a composition still ends it.
#[trial]
fn an_empty_commit_with_nothing_marked_reaches_no_editor() {
    for (name, mut rig) in [("a field's editor", Rig::new()), ("a canvas's editor", Rig::canvas_editor())] {
        rig.focus();
        rig.frame(vec![commit("")]);
        let inputs = rig.take();
        assert!(inputs.is_empty(), "{name}: an empty commit with nothing marked reaches the editor: {inputs:?}");
        rig.frame(vec![key_press(Key::Enter, Modifiers::NONE)]);
        let inputs = rig.take();
        assert!(matches!(inputs.as_slice(), [EditorInput::Newline { shift: false, .. }]), "{name}: the Enter after it is a newline: {inputs:?}");

        rig.frame(vec![preedit("k")]);
        assert!(matches!(rig.take().as_slice(), [EditorInput::Preedit { .. }]), "{name}: the editor composes");
        rig.frame(vec![commit("")]);
        let inputs = rig.take();
        assert!(
            matches!(inputs.as_slice(), [EditorInput::ImeCommit, EditorInput::Text(text)] if text.is_empty()),
            "{name}: an empty commit ends the composition: {inputs:?}"
        );
    }
}

/// A body that runs inside the component reports through `EditorHandle::report`: its caret reaches the IME output
/// and its lines reach AccessKit as the component's own `Cx::editor_state` report does.
#[trial]
fn a_report_through_the_editor_handle_reaches_the_ime_note_and_the_text_facts() {
    let reported = |through_body: bool| {
        let doc = Doc::new(TEXT, None);
        doc.body.set(through_body);
        let mut rig = Rig::with(EditorPurpose::Code, Some(doc), false);
        rig.focus();
        rig.frame(Vec::new());
        let caret = rig.frames.ime().map(|ime| ime.caret);
        (caret, rig.frames.tree().run_texts(rig.id.get()))
    };
    let (own_caret, own_runs) = reported(false);
    let (body_caret, body_runs) = reported(true);
    assert!(own_caret.is_some_and(|caret| caret.is_finite()), "the component's own report notes the caret: {own_caret:?}");
    assert_eq!(own_runs, [TEXT.to_owned()], "the component's own report reaches AccessKit");
    assert_eq!(body_caret, own_caret, "the body's caret reaches the IME note");
    assert_eq!(body_runs, own_runs, "the body's text facts reach the pass's record and AccessKit");
}

/// #2986: a field that keeps the keyboard does not reset the input method every frame. Once it has focus no frame
/// interrupts and typing lands; when Tab takes the keyboard to the next control and Shift+Tab brings it back, the
/// input method is reset once, counted as egui-winit counts it, and not again in the frames after.
#[trial]
fn a_focused_text_field_does_not_interrupt_the_input_method_every_frame() {
    let mut rig = Rig::with(EditorPurpose::Text, None, true);
    let mut on = false;
    let mut resets = 0;
    for events in [vec![moved(AT.center())], vec![press(AT.center())], vec![release(AT.center())]] {
        rig.frame(events);
        resets += usize::from(rig.resets_ime(&mut on));
    }
    assert!(on, "the focused field owns the input method");
    assert_eq!(resets, 1, "taking focus turns the input method on once");
    for frame in 0..6 {
        rig.frame(Vec::new());
        let seen = rig.ime();
        assert!(!seen.interrupt && seen.reset.is_none(), "frame {frame} with focus interrupts the input method: {seen:?}");
    }
    rig.take();
    rig.frame(vec![key_press(Key::A, Modifiers::NONE), Event::Text("a".into())]);
    assert!(!rig.resets_ime(&mut on), "typing resets the input method");
    let inputs = rig.take();
    assert!(inputs.iter().any(|input| matches!(input, EditorInput::Text(text) if &**text == "a")), "typing lands: {inputs:?}");

    let (mut left, mut round) = (false, 0);
    for events in [vec![key_press(Key::Tab, Modifiers::NONE)], Vec::new(), vec![key_press(Key::Tab, Modifiers::SHIFT)], Vec::new()] {
        rig.frame(events);
        round += usize::from(rig.resets_ime(&mut on));
        left |= !on;
    }
    assert!(left, "Tab takes the keyboard to the next control");
    assert!(on, "Shift+Tab gives the field the input method back");
    assert_eq!(round, 1, "Tab and back reset the input method once");
    for frame in 0..4 {
        rig.frame(Vec::new());
        assert!(!rig.resets_ime(&mut on), "frame {frame} after the round trip resets the input method");
    }
}

/// canvas-w10 F17: Shift+Tab reaches a document canvas's editor as `BackTab` (a word processor's outdent, the
/// previous slot of an equation), where an editor that is no canvas's does not read it. A text field keeps Shift+Tab
/// as a focus move: the keyboard leaves it, so typing after it no longer lands there.
#[trial]
fn shift_tab_is_a_canvas_editors_back_tab_and_moves_focus_out_of_a_text_field() {
    let mut canvas = Rig::canvas_editor();
    canvas.focus();
    canvas.frame(vec![key_press(Key::Tab, Modifiers::SHIFT)]);
    canvas.frame(vec![Event::Text("x".into())]);
    let inputs = canvas.take();
    assert!(
        matches!(inputs.as_slice(), [EditorInput::BackTab, EditorInput::Text(text)] if &**text == "x"),
        "the canvas editor reads Shift+Tab and keeps the keyboard: {inputs:?}"
    );

    let mut code = Rig::new();
    code.focus();
    code.frame(vec![key_press(Key::Tab, Modifiers::SHIFT)]);
    let inputs = code.take();
    assert!(!inputs.contains(&EditorInput::BackTab), "only a canvas's editor reads Shift+Tab: {inputs:?}");

    let mut field = Rig::with(EditorPurpose::Text, None, true);
    field.focus();
    field.frame(vec![key_press(Key::Tab, Modifiers::SHIFT)]);
    field.frame(Vec::new());
    field.frame(vec![Event::Text("x".into())]);
    let inputs = field.take();
    assert!(inputs.is_empty(), "Shift+Tab takes the keyboard out of the text field: {inputs:?}");
}

/// fwstub-1 (1): Shift+Delete reaches an editor as a delete that says Shift was held, so a word processor's page reads
/// it as Cut (which deletes nothing without a selection) and a plain Delete stays a delete.
#[trial]
fn shift_delete_reaches_an_editor_with_its_shift() {
    let mut rig = Rig::canvas_editor();
    rig.focus();
    rig.frame(vec![key_press(Key::Delete, Modifiers::SHIFT)]);
    rig.frame(vec![key_press(Key::Delete, Modifiers::NONE)]);
    let inputs = rig.take();
    assert_eq!(
        inputs,
        [EditorInput::delete_with_shift(true, Granularity::Unit, true), EditorInput::delete(true, Granularity::Unit)],
        "Shift+Delete and Delete arrive told apart"
    );
}

/// fwstub-1 (2): the editor reads one frame's input-method edits, typed text and keys in the order they arrived, as
/// `Cx::editor_input` says: a platform that sends a replacement's text before its commit has it read between the
/// deletion and the commit, and a key between a deletion and its text comes between them.
#[trial]
fn one_frames_ime_edits_text_and_keys_reach_the_editor_in_arrival_order() {
    for (name, mut rig) in [("a field's editor", Rig::new()), ("a canvas's editor", Rig::canvas_editor())] {
        rig.focus();
        let delete = || Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 });
        rig.frame(vec![delete(), Event::Text("Q".into()), commit("Q2"), commit("R")]);
        let expected = [
            EditorInput::delete_surrounding_bytes(1, 0),
            EditorInput::Text("Q".into()),
            EditorInput::ImeCommit,
            EditorInput::Text("Q2".into()),
            EditorInput::ImeCommit,
            EditorInput::Text("R".into()),
        ];
        assert_eq!(rig.take(), expected, "{name}: the text comes where it arrived, between the deletion and the commits");
        // A commit guards the keys that end a composition for a frame.
        rig.frame(Vec::new());
        rig.frame(Vec::new());
        rig.take();

        rig.frame(vec![delete(), key_press(Key::Delete, Modifiers::NONE), Event::Text("K".into())]);
        let expected = [EditorInput::delete_surrounding_bytes(1, 0), EditorInput::delete(true, Granularity::Unit), EditorInput::Text("K".into())];
        assert_eq!(rig.take(), expected, "{name}: the key comes between the deletion and the text");

        rig.frame(vec![Event::Text("a".into()), key_press(Key::Backspace, Modifiers::NONE), Event::Text("b".into())]);
        let expected = [EditorInput::Text("a".into()), EditorInput::delete(false, Granularity::Unit), EditorInput::Text("b".into())];
        assert_eq!(rig.take(), expected, "{name}: a key between two texts stays between them");
    }
}

/// fwstub-1 (4): an editor that draws marked text and refuses a deletion in the frame its pre-edit arrived in ends the
/// composition it showed for a moment: the OS composition is interrupted, as when the two come in frames of their own.
/// An editor that shows no marked text is never interrupted for reporting none.
#[trial]
fn a_pre_edit_and_a_refused_deletion_in_one_frame_interrupt_the_composition() {
    let delete_after = || Event::Ime(ImeEvent::DeleteSurrounding { before_chars: 0, after_chars: 1 });
    for (marks, split) in [(true, false), (true, true), (false, false)] {
        let mut rig = Rig::with(EditorPurpose::Text, Some(Doc::new("ab", Some(1))), false);
        rig.doc().marks.set(marks);
        rig.doc().refuses_after.set(true);
        rig.focus();
        let interrupted = if split {
            rig.frame(vec![preedit("に")]);
            rig.frame(vec![delete_after()]);
            rig.ime().interrupt
        } else {
            rig.frame(vec![preedit("に"), delete_after()]);
            rig.ime().interrupt
        };
        assert!(!rig.doc().composing.get(), "the probe shows no marked text after the refusal");
        assert_eq!(interrupted, marks, "marks {marks}, split {split}: the composition is interrupted only by an editor that shows marked text");
    }
}

/// The commands of an editor's typing scope: Commit on Cmd+Enter, as a type tool ends editing with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "editorTrial", scope = "trial.editor", typing)]
#[non_exhaustive]
enum EditorCommand {
    #[command(label = "Commit", default = "Cmd+Enter")]
    Commit,
}

/// An editor that is the instance of `EditorCommand`'s scope and logs the steps of each pass.
#[derive(craft_ui::contract::Component)]
struct Typer<'a> {
    #[common]
    common: Common,
    steps: &'a RefCell<Vec<EditorStep<EditorCommand>>>,
}

impl Component for Typer<'_> {
    const ROLE: Role = Role::MULTILINE_TEXT_INPUT;
    const INPUT: craft_ui::contract::Input = craft_ui::contract::Input::Editor;
    const KEYS: KeyUse = KeyUse::CODE;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: TyperStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.editor(EDITOR, rect, EditorPurpose::Text);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let steps = cx.editor_steps::<EditorCommand>();
        self.steps.borrow_mut().extend(steps);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A command of the editor's scope that a chord fires reaches the editor between the inputs typed before and after
/// the chord in the same frame, so an editor that Cmd+Enter ends applies none of the typing after it. Guards against
/// the editor reading a frame's inputs and its commands as two batches.
#[trial]
fn a_command_a_chord_fires_reaches_its_editor_between_the_typing_around_it() {
    let steps = RefCell::new(Vec::new());
    let mut show = |ui: &mut egui::Ui| {
        let _ = Typer { common: Common::new(), steps: &steps }.label("Type").command_scope(<EditorCommand as CommandSet>::SCOPE).show_at(ui, AT);
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    click(&mut frames, AT.center(), &mut show);
    steps.take();
    let cmd_enter = [
        Event::ModifiersChanged(Modifiers::COMMAND),
        key_press(Key::Enter, Modifiers::COMMAND),
        key_release(Key::Enter, Modifiers::COMMAND),
        Event::ModifiersChanged(Modifiers::NONE),
    ];
    frames.frame([vec![Event::Text("a".into())], cmd_enter.to_vec(), vec![Event::Text("b".into())]].concat(), &mut show);
    let steps = steps.take();
    assert!(
        matches!(
            steps.as_slice(),
            [EditorStep::Input(EditorInput::Text(a)), EditorStep::Command(EditorCommand::Commit, _), EditorStep::Input(EditorInput::Text(b))]
                if &**a == "a" && &**b == "b"
        ),
        "the command sits between the texts its chord came between: {steps:?}"
    );
}

/// A type editor's typing scope with a chord that types a character on some layouts: a dash command on Alt+Shift+-,
/// where Option+Shift+- types an em dash on the Mac.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "dashTrial", scope = "trial.dashes", typing)]
#[non_exhaustive]
enum DashCommand {
    #[command(label = "Dash", default = "Alt+Shift+-")]
    Dash,
}

/// The text of an Alt chord bound only in a scope that is not around the focus (the typing scope of a type editor shown
/// beside the focused field) is typed in the field: the press runs nothing there. Guards a key stage that withheld the
/// text for any enabled binding of the chord, whatever its scope, so the character went nowhere.
#[trial]
fn an_alt_chord_bound_only_out_of_scope_leaves_its_text_typed() {
    use craft_behavior::Input;
    use craft_ui::runtime::focus::{FocusTarget, request};
    use craft_ui::runtime::frame::take_output;
    use craft_ui::testing::focus::{Stop, intents_of, named_id, register_stop, widget};

    let field = named_id("dash field");
    let (typed, focus, steps) = (RefCell::new(String::new()), Cell::new(true), RefCell::new(Vec::new()));
    let mut show = |ui: &mut egui::Ui| {
        let _ = Typer { common: Common::new(), steps: &steps }.label("Type").command_scope(<DashCommand as CommandSet>::SCOPE).show_at(ui, AT);
        widget(ui, field);
        register_stop(ui.ctx(), Stop::new(field, KeyUse::of(Input::Text)));
        if focus.take() {
            request(ui.ctx(), FocusTarget::Id(field));
        }
        let mut intents = intents_of(ui.ctx(), Some(field), false);
        while let Some(text) = intents.take_text() {
            typed.borrow_mut().push_str(&text);
        }
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    take_output(&frames.ctx);
    let alt_shift = Modifiers { alt: true, shift: true, ..Modifiers::NONE };
    frames.frame(vec![key_press(Key::Minus, alt_shift), Event::Text("\u{2014}".into())], &mut show);
    frames.frame(vec![key_release(Key::Minus, alt_shift)], &mut show);
    frames.idle(1, &mut show);
    assert_eq!(typed.take(), "\u{2014}", "the field types the chord's text");
    let fired: Vec<String> = take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.id).collect();
    assert!(fired.is_empty(), "nothing ran: {fired:?}");
}

/// A disabled editor registers no stop, and its focus widget is still the component's own and no egui focus candidate:
/// it never becomes a Tab stop of the app's own widgets, and egui's arrows from the app's own focused widget never
/// reach it. With every control in Tab order, Tab and Shift+Tab land on the button beside it each time.
#[trial]
fn tab_never_focuses_a_disabled_editor_s_widget() {
    let log = RefCell::new(Vec::new());
    let seen = RefCell::new(Seen::default());
    let (editor, button) = (Cell::new(Id::NULL), Cell::new(Id::NULL));
    // The app's own egui button, level with the editor on its right: the editor lies straight to its left.
    let app = craft_ui::testing::focus::named_id("app button");
    let app_at = Rect::from_min_size(Pos2::new(AT.right() + 40.0, AT.center().y - 10.0), egui::vec2(60.0, 20.0));
    let mut show = |ui: &mut egui::Ui| {
        editor.set(Probe::new(&log, true).label("Source").enabled(false).id_salt("code").show_at(ui, AT).id);
        button.set(Button::new(&seen).id_salt("next").show_at(ui, NEXT).id);
        ui.interact(app_at, app, egui::Sense::click());
    };
    let mut frames = Frames::new();
    frames.root({
        let mut root = craft_ui::runtime::env::RootEnv::default();
        root.tab_all_controls = true;
        root
    });
    frames.idle(3, &mut show);
    for shift in [false, false, true, true] {
        let mods = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
        frames.frame(vec![key_press(Key::Tab, mods), key_release(Key::Tab, mods)], &mut show);
        frames.frame(Vec::new(), &mut show);
        let focused = craft_ui::testing::focus::egui_focused(&frames.ctx);
        assert!(
            focused == Some(button.get()) || focused == Some(app),
            "Tab (shift {shift}) lands on a control, never on the disabled editor {:?}: {focused:?}",
            editor.get()
        );
    }
    craft_ui::runtime::focus::request(&frames.ctx, craft_ui::runtime::focus::FocusTarget::Id(app));
    frames.idle(2, &mut show);
    assert_eq!(craft_ui::testing::focus::egui_focused(&frames.ctx), Some(app), "the app's button has the focus");
    frames.frame(vec![key_press(Key::ArrowLeft, Modifiers::NONE), key_release(Key::ArrowLeft, Modifiers::NONE)], &mut show);
    frames.frame(Vec::new(), &mut show);
    let focused = craft_ui::testing::focus::egui_focused(&frames.ctx);
    assert!(focused == Some(app) || focused == Some(button.get()), "the arrow never reaches the disabled editor {:?}: {focused:?}", editor.get());
}
