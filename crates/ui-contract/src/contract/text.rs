//! The text part of the pipeline (18.2): `PartsCx::text` registers the part with its typing run, and step 8b runs the
//! text adapter, then the typing run over what the adapter recorded, and applies the run's events through `Cx`.
//!
//! Selection parts are read-only text with no typing run, any number per component: `PartsCx::selectable_text`
//! declares one and its press-only stop, step 8b gives the focused one Select All and Copy, and
//! `PaintUi::selectable_text` shows it, so it paints in the state's colours.

use super::cx::{Cx, CxCore, PartsCx, RecordedText, gesture_key};
use super::{Component, Edit as _, Focus as _, Overlays as _, ValueOf};
use crate::primitives::text_adapter::{self as adapter, End, TextState};
use crate::primitives::{TextKey, TextOptions, TextOutcome, TextRun, Wrap};
use crate::runtime::access::text as text_access;
use crate::runtime::focus::{EditSet, KeepKeys, KeyUse};
use crate::runtime::keys;
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::motion::Animate as _;
use craft_behavior::{
    CancelReason, Caret, Codec, EditCommand, EditRun, Ended, EscapeRule, Escaped, Gesture, NotAValue, PartKey, PartRef, Typed, TypingRun,
};
use craft_keymap::Mods;
use egui::{Rect, Ui};
use std::any::Any;
use std::sync::Arc;

/// The popup name of a text part's edit menu: step 8b registers it for every text part, and the text control opens
/// and shows it from its `overlay` hook.
pub const EDIT_MENU: &str = "edit";

/// The id of the text adapter's state in the component's memory.
fn state_id(id: egui::Id) -> egui::Id {
    sub(id, "text_state")
}

/// The selection the text part of the component `id` held when its edit menu took the focus, while the menu is open.
pub(crate) fn held_selection(memory: &mut MemoryStage, id: egui::Id) -> Option<(usize, usize)> {
    memory.shown::<TextState>(state_id(id)).held_selection
}

/// Whether the text part of the component `id` has a typing run open, as its last pass left it.
pub(crate) fn run_open(memory: &MemoryStage, id: egui::Id) -> bool {
    memory.recall::<TextState>(state_id(id)).start.is_some()
}

/// The draft the text part of the component `id` holds while a typing run is open.
pub(crate) fn draft(memory: &mut MemoryStage, id: egui::Id) -> Option<String> {
    memory.shown::<TextState>(state_id(id)).draft.clone()
}

/// Whether a scrubbing field was in edit before the press that is now down: a press that becomes a drag scrubs and
/// leaves edit as it found it, and only a click without travel enters it.
#[derive(Clone, Default)]
struct InEditBeforePress(bool);

/// What `PartsCx::text` leaves for step 8b.
#[derive(Default)]
pub(crate) struct TextStep {
    /// A press elsewhere or another focus loss asks the run to commit now.
    pub commit_now: bool,
    pub decl: Option<TextDecl>,
    pub editor: Option<super::editor::EditorDecl>,
    pub selections: Vec<SelectDecl>,
}

/// A selection part as `PartsCx::selectable_text` declared it.
pub(crate) struct SelectDecl {
    pub part: PartRef,
    pub text: String,
    /// The text wraps: the arrows move the caret between lines.
    pub multiline: bool,
}

impl SelectDecl {
    /// The keys a focused selection part keeps: a read-only text's, with Select All, Copy and Cut its only edit
    /// commands. Cut copies, as in a label's selection: the text removes nothing, and the app's Cut never runs on what
    /// lies behind it.
    pub(crate) fn keys(&self) -> KeyUse {
        let edits = EditSet::NONE.with(EditCommand::SelectAll).with(EditCommand::Copy).with(EditCommand::Cut);
        let keys = KeyUse::of(crate::contract::Input::Text).with_edits(edits);
        if self.multiline { keys.with_keep(KeepKeys::BlockArrows) } else { keys }
    }
}

/// The egui text widget of the selection part `part` of component `id`, which holds key focus.
pub(crate) fn selection_id(id: egui::Id, part: PartRef) -> egui::Id {
    sub(id, ("selection", part.key, part.index))
}

/// Step 8b for selection parts: the focused one takes this pass's Select All, Copy and Cut.
pub(crate) fn run_selections(core: &mut super::cx::CxCore<'_>, selections: &[SelectDecl]) {
    let ctx = core.ctx.clone();
    let Some(focused) = selections.iter().find(|decl| crate::runtime::egui::focus::has_focus(&ctx, selection_id(core.id, decl.part))) else {
        return;
    };
    let widget = selection_id(core.id, focused.part);
    while let Some(command) = core.intents.take_edit() {
        adapter::select_or_copy(&ctx, widget, &focused.text, command);
    }
}

pub(crate) struct TextDecl {
    pub part: TextPart,
    /// A `Resolve<V>` for the component's value type, which the text step takes and invokes after the adapter records
    /// the pass; the part stays declared, so the stops read its keys as the pass's edits left them.
    resolve: Option<Box<dyn Any>>,
    values: ValueOps,
}

pub(crate) struct TextPart {
    pub key: PartKey,
    rect: Rect,
    pub shown: String,
    pub options: TextOptions,
    caret: Caret,
}

/// The shown copy of a component's value, type-erased.
pub(crate) type Shown = Arc<dyn Any + Send + Sync>;

/// What the typing run does with the component's value, written once for its type: the run keeps the value it last
/// showed, and a different one at the next pass was set outside the run.
#[derive(Clone, Copy)]
pub(crate) struct ValueOps {
    /// Ends the run when the value is not the one the run last showed.
    resync: fn(&mut TextState, Option<&Shown>),
}

impl ValueOps {
    fn of<V: Clone + PartialEq + Send + Sync + 'static>() -> ValueOps {
        ValueOps {
            resync: |state, shown| {
                let (Some(seen), Some(shown)) = (state.seen.as_ref(), shown) else { return };
                if Arc::ptr_eq(seen, shown) {
                    return;
                }
                if let (Some(seen), Some(shown)) = (seen.downcast_ref::<V>(), shown.downcast_ref::<V>())
                    && seen != shown
                {
                    state.draft = None;
                    state.start = None;
                    state.normalised = None;
                    state.dirty = false;
                    state.invalid = false;
                }
            },
        }
    }
}

enum TextEvent<V> {
    Propose(V),
    Commit(V),
    Cancel(CancelReason),
}

/// What the typing run decided this pass.
struct Resolved<V> {
    events: Vec<TextEvent<V>>,
    outcome: TextOutcome<V>,
    submitted: bool,
    /// The modifiers of the submitting Enter; none unless `submitted`.
    submit_mods: Mods,
    escaped: bool,
    /// The draft became the codec's normal form of its value: `run` selects the whole text.
    rewritten: bool,
    /// The pass refused input (a character the codec's filter dropped, a rewriting Tab): `run` calls
    /// `frame::refuse`, since the resolver has no context.
    refused: bool,
}

/// The typing run over the state the adapter recorded this pass, the component's value (`None` while mixed) and
/// whether a commit is due.
type Resolve<V> = Box<dyn FnOnce(&mut TextState, Option<&V>, bool, bool) -> Resolved<V>>;

/// Turns what the adapter recorded this pass into the typing run's events: a parsed draft applies in the pass it
/// is typed, and a commit proposes and commits in one pass. `fallback` is the value the part shows, the run's start
/// while the component's value is mixed. The closure's last argument says whether the part keeps its text on Escape
/// this pass: the option alone is not enough, since a part that went read-only or disabled mid-run takes Escape as a
/// plain field does.
fn resolver<K: Codec + Clone + 'static>(codec: K, options: &TextOptions, rect: Rect, fallback: K::Value) -> Resolve<K::Value> {
    let (live, limit, clear_on_escape) = (options.live, options.limit, options.clear_on_escape);
    Box::new(move |state, value, commit_now, keep_on_escape| {
        let value = value.unwrap_or(&fallback);
        let mut typing = TypingRun::new(codec).live(live);
        if let Some(limit) = limit {
            typing = typing.limit(limit);
        }
        if clear_on_escape {
            typing = typing.escape_rule(EscapeRule::Clear);
        }
        let started = state.start.as_ref().and_then(|start| start.downcast_ref::<K::Value>()).cloned();
        let mut run = EditRun::new(started.unwrap_or_else(|| value.clone()));
        run.draft = state.draft.clone();
        if state.invalid {
            run.error = Some(NotAValue);
        }
        let mut events = Vec::new();
        let (mut rewritten, mut refused) = (false, false);
        if std::mem::take(&mut state.dirty)
            && let Some(draft) = state.draft.clone()
        {
            let start = run.start.clone();
            let (typed, dropped) = typing.edit(&mut run, &start, &draft);
            refused |= dropped;
            if let Typed::Apply(applied) = typed {
                events.push(TextEvent::Propose(applied));
            }
        }
        let draft_text = run.draft.clone().unwrap_or_default();
        let (mut submitted, mut escaped) = (false, false);
        let mut committed_note = None;
        let submit_mods = std::mem::take(&mut state.submit_mods);
        let end = match state.end.take() {
            None if commit_now && run.is_open() => Some(End::Blur),
            end => end,
        };
        match end {
            Some(End::Revert) => {
                if let Escaped::Restore(_) = typing.escape(&mut run) {
                    events.push(TextEvent::Cancel(CancelReason::Escape));
                }
            }
            Some(End::Escape) if keep_on_escape => {
                // The text stays: an open run commits once, neither submitted nor cancelled.
                escaped = true;
                if let (Ended::Commit(committed), note) = typing.commit_tagged(&mut run) {
                    committed_note = Some(note);
                    state.normalised = Some(typing.codec().normalise(&draft_text, &committed));
                    events.push(TextEvent::Commit(committed));
                }
            }
            Some(End::Escape) => {
                escaped = true;
                if clear_on_escape && !run.is_open() {
                    let start = run.start.clone();
                    typing.open(&mut run, &start);
                }
                match typing.escape(&mut run) {
                    Escaped::Restore(_) => events.push(TextEvent::Cancel(CancelReason::Escape)),
                    Escaped::Cleared(cleared) => {
                        state.normalised = Some(typing.codec().normalise("", &cleared));
                        events.push(TextEvent::Commit(cleared));
                    }
                    _ => {}
                }
            }
            Some(End::Rewrite { refuses }) => {
                rewritten = typing.rewrite(&mut run);
                refused |= refuses;
            }
            Some(end @ (End::Submit | End::Separator)) => match typing.commit_tagged(&mut run) {
                (Ended::Commit(committed), note) => {
                    committed_note = Some(note);
                    state.normalised = Some(typing.codec().normalise(&draft_text, &committed));
                    events.push(TextEvent::Commit(committed));
                    submitted = end == End::Submit;
                }
                (Ended::Rejected, _) => {}
                (Ended::Idle, _) => submitted = end == End::Submit,
                _ => {}
            },
            Some(End::Blur) => match typing.commit_tagged(&mut run) {
                (Ended::Commit(committed), note) => {
                    committed_note = Some(note);
                    state.normalised = Some(typing.codec().normalise(&draft_text, &committed));
                    events.push(TextEvent::Commit(committed));
                }
                (Ended::Rejected, _) => {
                    typing.cancel(&mut run);
                    events.push(TextEvent::Cancel(CancelReason::Blur));
                }
                _ => {}
            },
            None => {}
        }
        state.draft = run.draft.clone();
        state.start = run.is_open().then(|| Arc::new(run.start.clone()) as Arc<dyn Any + Send + Sync>);
        state.invalid = run.error.is_some();
        let mut outcome = TextOutcome::new(rect);
        outcome.draft = run.draft.clone();
        outcome.invalid = state.invalid;
        outcome.committed = committed_note;
        Resolved { events, outcome, submitted, submit_mods: if submitted { submit_mods } else { Mods::NONE }, escaped, rewritten, refused }
    })
}

impl<C: Component> PartsCx<'_, C> {
    /// Declares `run` as a selection part: read-only text that a press focuses, drag and double click select and Copy
    /// copies, and that Tab and F6 pass over. `paint` shows it with `PaintUi::selectable_text` and the same run in
    /// the state's colours. Its text is the component's own: it has no node of its own, so the component's node
    /// names it. An elided run cannot be one, since the selection counts the characters shown.
    pub fn selectable_text(&mut self, part: impl Into<PartRef>, run: &TextRun<'_>) {
        debug_assert!(!(run.wrap.cuts() || matches!(run.wrap, Wrap::Clamp(_))), "a selection part shows its whole text");
        let part = part.into();
        debug_assert!(self.text.selections.iter().all(|decl| decl.part != part), "selection part {part:?} declared twice");
        self.text.selections.push(SelectDecl { part, text: run.full_text(), multiline: run.wrap == Wrap::Wrap });
    }

    /// Registers the component's one text part (`INPUT = Input::Text`).
    pub fn text<K: Codec + Clone + 'static>(&mut self, key: TextKey<K::Value>, rect: Rect, codec: &K, value: &K::Value, options: &TextOptions) {
        debug_assert!(self.text.decl.is_none(), "a component registers one text part");
        let shown = if options.mixed { String::new() } else { codec.format(value) };
        let reads = codec.clone();
        let declared = TextOptions { whole_paste: Some(Arc::new(move |pasted: &str| reads.whole_paste(pasted))), ..options.clone() };
        let resolve = Box::new(resolver(codec.clone(), &declared, rect, value.clone()));
        self.text.decl = Some(TextDecl {
            part: TextPart {
                key: key.0,
                rect,
                caret: if options.select_on_focus { Caret::All } else { codec.caret_on_focus(&shown) },
                shown,
                options: declared,
            },
            resolve: Some(resolve),
            values: ValueOps::of::<K::Value>(),
        });
    }
}

/// Serves the AccessKit requests aimed at the text part: `SetTextSelection` moves the selection and `SetValue`
/// replaces the draft and commits it.
fn serve_access(core: &mut CxCore<'_>, state: &mut TextState, widget: egui::Id, shown: &str, read_only: bool) {
    let (ctx, id) = (core.ctx.clone(), core.id);
    while let Some(request) = core.intents.take_access_where(text_access::is_text_request) {
        if let Some((anchor, focus)) = text_access::requested_selection(&ctx, id, &request) {
            let text = state.draft.as_deref().unwrap_or(shown);
            if let Some(rows) = &state.rows {
                adapter::select(&ctx, widget, rows.chars(text, anchor), rows.chars(text, focus));
            }
        } else if let Some(text) = text_access::requested_value(&request)
            && !read_only
        {
            state.draft = Some(text);
            state.dirty = true;
            state.end = Some(End::Blur);
        }
    }
}

/// What a field says about itself to AccessKit besides its text.
struct FieldFacts {
    masked: bool,
    read_only: bool,
    enabled: bool,
}

/// The text facts AccessKit reads this pass, built only while AccessKit runs.
fn facts(core: &CxCore<'_>, state: &mut TextState, widget: egui::Id, out: &adapter::AdapterOut, rect: Rect, field: FieldFacts) {
    let FieldFacts { masked, read_only, enabled } = field;
    if !text_access::exposed(&core.ctx, widget) {
        state.rows = None;
        return;
    }
    let rows = adapter::Rows::of(&out.layout, out.layout.full(), rect.min);
    let text = out.layout.full().to_owned();
    let selection = adapter::selection(&core.ctx, widget).map(|(caret, anchor)| (rows.pos(&text, anchor), rows.pos(&text, caret)));
    *core.text_access.borrow_mut() =
        Some(text_access::TextFacts { widget, text, masked, editable: !read_only, disabled: !enabled, lines: rows.lines.clone(), selection });
    state.rows = Some(rows);
}

/// The component's shown copy of its value, shared.
fn shown_copy<C: Component>(cx: &Cx<'_, C>) -> Option<Shown> {
    cx.shown.clone().map(|shown| shown as Shown)
}

/// Opens the completion list's popup while the adapter holds candidates and closes it otherwise. A popup the host
/// closed (Escape, a press outside) hides the list until the draft changes.
fn drive_completions<C: Component>(cx: &mut Cx<'_, C>, state: &mut TextState) {
    let list = cx.popup("completions");
    let shown = list.is_open(cx);
    if state.completions.sync(shown) {
        list.open(cx);
    } else {
        list.close(cx);
    }
}

/// Applies the typing run's events to the component's value: a parsed draft reaches the binding in the pass it is
/// typed, and a commit proposes and commits in one pass. Returns whether a commit changed the value, after the
/// parts of this pass were declared.
fn apply_events<C: Component>(cx: &mut Cx<'_, C>, events: Vec<TextEvent<ValueOf<C>>>) -> bool {
    let id = cx.core.id;
    let mut committed = false;
    for event in events {
        let running = cx.core.memory.recall::<Gesture<ValueOf<C>>>(gesture_key(id)).is_running();
        match event {
            TextEvent::Propose(value) => cx.propose(value),
            TextEvent::Commit(value) => {
                committed |= cx.value() != Some(&value);
                if running {
                    cx.propose(value);
                    cx.commit();
                } else if cx.value() != Some(&value) {
                    cx.set(value);
                }
            }
            TextEvent::Cancel(reason) if running => cx.cancel(reason),
            TextEvent::Cancel(_) => {}
        }
    }
    committed
}

struct AdapterSettings {
    keys: KeyUse,
    rtl: bool,
    ink: egui::Color32,
    underline: f32,
}

struct PreparedText {
    widget: egui::Id,
    out: adapter::AdapterOut,
    read_only: bool,
    list_open: bool,
    caret_visible: bool,
    commit_now: bool,
    was_focused: bool,
    /// The window lost focus in this pass while the text part had it.
    window_blur: bool,
    /// The part's edit menu is open and holds its editing focus.
    menu_open: bool,
    /// Focus moved from the text part to another part of its component in this pass.
    left_for_part: bool,
}

/// Run the text adapter and adjust focus before the component's typed resolver handles its value.
fn prepare(core: &mut CxCore<'_>, ui: &mut Ui, decl: &TextPart, state: &mut TextState, settings: AdapterSettings, commit_now: bool) -> PreparedText {
    let (ctx, id) = (core.ctx.clone(), core.id);
    let AdapterSettings { keys, rtl, ink, underline } = settings;
    let widget = adapter::text_id(id, decl.key);
    // A list the component opened on the text (a dropdown, the completions) takes Escape before the text does.
    let list_open = crate::runtime::focus::holds_anchored_layer(&ctx, id);
    let mut focused = crate::runtime::egui::focus::has_focus(&ctx, widget);
    let was_focused = state.focused;
    let now = core.now();
    let caret_visible = !focused || core.blink(state.caret_moved);
    let enabled = core.resolved.enabled && decl.options.enabled;
    let focusable = super::steps::takes_focus(core.resolved) && decl.options.enabled;
    // A component that senses a drag on its root scrubs its value by it: the field takes the pointer only once it is
    // in edit, a press that begins in edit stays the text's (a drag selects), and a click that enters it selects as
    // focus does (`decl.caret`).
    let scrubs = core.root_sense.contains(craft_behavior::Sense::DRAG);
    let mut in_edit_before = false;
    if scrubs {
        let before = core.memory.shown::<InEditBeforePress>(sub(id, "in_edit_before_press"));
        if core.press.phase.is_drag() {
            // The root press focused the field; a press that travelled is a scrub, so the field is given back.
            if !before.0 {
                crate::runtime::egui::focus::surrender(&ctx, widget);
                focused = false;
            }
        } else if !core.press.phase.button_down() {
            before.0 = focused;
        }
        in_edit_before = before.0;
    }
    let pointer = !scrubs || in_edit_before || (focused && !core.press.phase.is_drag());
    core.text_takes_pointer = pointer;
    let read_only = core.resolved.read_only || !enabled;
    // The tokens of the place the field is shown in, read only for an ink of its own or a disabled field.
    let place = (decl.options.ink.is_some() || !enabled).then(|| crate::runtime::env::Env::of(ui));
    let ink = match (decl.options.ink, &place) {
        (Some(own), Some(place)) => crate::style::ink_color(&place.tokens, own),
        _ => ink,
    };
    let ink = match &place {
        Some(place) if !enabled => crate::style::rgba_color(place.tokens.color.text.disabled),
        _ => ink,
    };
    let held = core.popup(EDIT_MENU).is_open_in(core);
    // The text the part shows now: its resting text while it is idle and has one, else the value's own.
    let current = decl.options.resting.as_deref().filter(|_| !focused && !held && state.draft.is_none()).unwrap_or(&decl.shown);
    // A menu bar or a menu the keyboard goes back to the part from has the focus: their Cut and Copy act on the
    // selection the part keeps for them. Only a part that had the focus last pass, or keeps a selection already, can be
    // gone back to: asking reads the focus, which a part that never held it has not read, so its replay stays true.
    let returns_here = (state.focused || state.held_selection.is_some())
        .then(|| crate::runtime::focus::focused_stop(&ctx))
        .flatten()
        .filter(|(focused, _)| *focused != id)
        .and_then(|(focused, _)| crate::runtime::focus::returning(&ctx, focused))
        .is_some_and(|((to, _), _)| to == id);
    // The bound caret is compared with the widget's before anything else this pass moves it (AccessKit, Select All,
    // the typing): last pass reported the widget's caret, so only an app write differs here. A composition keeps its
    // caret.
    let caret_placed =
        state.preedit.is_none() && adapter::place_caret(&ctx, widget, state.draft.as_deref().unwrap_or(&decl.shown), decl.options.caret);
    serve_access(core, state, widget, current, read_only);
    let select_all = (std::mem::take(&mut state.select_all) || decl.options.select_all) && !crate::runtime::ime::composing(&ctx, widget);
    if select_all {
        let len = state.draft.as_deref().unwrap_or(current).chars().count();
        adapter::select(&ctx, widget, 0, len);
    }
    core.surface_slot = Some(ui.painter().add(egui::Shape::Noop));
    let menu_held = state.held_selection.is_some();
    // Focus that arrived between passes (a request after the field was drawn, which takes effect at the end of that
    // pass) places the caret before this pass's input is applied, so the first text typed replaces a selection. Focus
    // back from the edit menu is no arrival.
    let arrived_before = focused && !was_focused && !held && !menu_held && core.active_part().is_none();
    // Focus that arrives while a button is down over the field came with a press on it.
    let frame = crate::runtime::pointer::frame_raw(&ctx);
    let by_pointer = !frame.buttons.is_empty() && frame.pos.is_some_and(|at| core.visible.contains(at));
    if arrived_before {
        arrive(&ctx, widget, decl, state, by_pointer, select_all);
    }
    let out = adapter::run(
        state,
        adapter::AdapterIn {
            ctx: &ctx,
            ui,
            id,
            key: decl.key,
            rect: decl.rect,
            shown: &decl.shown,
            options: &decl.options,
            keys,
            read_only,
            enabled,
            focusable,
            pointer,
            rtl,
            focused,
            held,
            keeps_selection: held || returns_here,
            ink,
            intents: &mut core.intents,
            list_open,
            placeholder: decl.options.placeholder.as_ref().map(|label| label.as_str()),
            caret_visible,
            underline,
            now,
            caret_placed,
        },
    );
    // Focus that moved to another part of the component leaves the text part like a focus loss: the draft commits.
    let left_for_part = out.focused && core.active_part().is_some();
    if left_for_part {
        state.focused = false;
    }
    // A window deactivation is no edit's end: the draft stays for the next Enter or a press elsewhere.
    let window_blur = was_focused && crate::runtime::pointer::with_pass_raw(&ctx, |pointer| pointer.focus_lost);
    let commit_now = (commit_now || (was_focused && left_for_part)) && !window_blur;
    // Focus back from the edit menu finds the selection the part held for it, which the adapter put back.
    let back_from_menu = menu_held && state.held_selection.is_none();
    // A caller can request focus after showing the field. Retry the selection in the next pass so that focus gain
    // cannot replace it with the default caret, while still selecting before typing in this pass.
    if decl.options.select_all && select_all && !out.focused {
        state.select_all = true;
        crate::runtime::egui::repaint::request(&ctx);
    }
    // A select asked for in the pass that brings focus (`Cx::select_all_text`) is kept, as `select_on_focus` keeps it.
    if out.focused && !left_for_part && !was_focused && !back_from_menu && !arrived_before {
        arrive(&ctx, widget, decl, state, by_pointer, select_all);
    }
    if state.select_on_click {
        if out.clicked && out.focused {
            adapter::select(&ctx, widget, 0, decl.shown.chars().count());
            state.select_on_click = false;
        } else if !out.focused || frame.buttons.is_empty() {
            // A drag, or a press that ended elsewhere: the pointer's caret or range stays.
            state.select_on_click = false;
        }
    }
    PreparedText { widget, out, read_only, list_open, caret_visible, commit_now, was_focused, window_blur, menu_open: held, left_for_part }
}

/// Places the caret of the text part's widget `widget` as focus arrives on the part: where the codec puts it, or over
/// the whole text for `select_on_focus`, or when a select was asked for (`select_all`). `select_on_focus` is for focus
/// by key or request: focus a pointer press brings (`by_pointer`) keeps the caret or range the pointer makes, and a
/// plain click selects the whole text once it is released (`TextState::select_on_click`).
fn arrive(ctx: &egui::Context, widget: egui::Id, decl: &TextPart, state: &mut TextState, by_pointer: bool, select_all: bool) {
    let len = decl.shown.chars().count();
    // A dialog that opens on this field selects its whole text whatever the codec's caret, so typing replaces it
    // (photocraft#15540).
    let opened = !decl.options.keep_caret_on_open
        && crate::runtime::focus::focused_rec(ctx)
            .and_then(|rec| crate::runtime::focus::origin(ctx, &rec))
            .is_some_and(|origin| origin == crate::runtime::focus::FocusOrigin::Open);
    if by_pointer && decl.options.select_on_focus && !opened && !select_all {
        state.select_on_click = true;
        return;
    }
    match if opened || select_all { Caret::All } else { decl.caret } {
        Caret::All => adapter::select(ctx, widget, 0, len),
        // A bound caret is where the app put it: focus arriving keeps it.
        _ if decl.options.caret.is_some() => {}
        _ => adapter::select(ctx, widget, len, len),
    }
}

/// Step 8b: runs the text adapter at the part's rect, then the typing run over what it recorded, and applies the
/// run's events: a typed value reaches the binding in the pass it is typed.
pub(crate) fn run<C: Component>(
    cx: &mut Cx<'_, C>,
    ui: &mut Ui,
    step: &mut TextStep,
    keys: crate::runtime::focus::KeyUse,
    rtl: bool,
    ink: egui::Color32,
    underline: f32,
) {
    let Some(decl) = step.decl.as_mut() else { return };
    let Some(resolve) = decl.resolve.take() else { return };
    let Ok(resolve) = resolve.downcast::<Resolve<ValueOf<C>>>() else {
        debug_assert!(false, "the text part's codec value is the component's value");
        return;
    };
    let resolve = *resolve;
    let part = &decl.part;
    let (ctx, id) = (cx.core.ctx.clone(), cx.core.id);
    let state_id = state_id(id);
    let mut state = std::mem::take(cx.core.memory.shown::<TextState>(state_id));
    // A part that was not shown last pass lost its focus, and its typing run with it: the run is forgotten (step 4
    // ended its gesture without putting back the value from before it), and the part starts afresh from its value.
    if !crate::runtime::memory::shown_last_pass(&ctx, id) {
        state = TextState::default();
    } else if state.edit_target != part.options.edit_target {
        // The old target's draft and gesture must never reach the new target's binding.
        cx.cancel(CancelReason::HostChanged);
        state = TextState::default();
    }
    state.edit_target = part.options.edit_target;
    (decl.values.resync)(&mut state, shown_copy(cx).as_ref());
    let text = prepare(cx.core, ui, part, &mut state, AdapterSettings { keys, rtl, ink, underline }, step.commit_now);
    let PreparedText { widget, out, read_only, list_open, caret_visible, commit_now, was_focused, window_blur, menu_open, left_for_part } = text;
    if part.options.complete.is_some() {
        drive_completions(cx, &mut state);
    }
    if window_blur && state.end == Some(End::Blur) {
        state.end = None;
    }
    if part.options.holds_drafts {
        // A press the focus service held for this part keeps focus; the draft shows as its value would.
        if state.end.is_none() && crate::runtime::focus::held_draft(&ctx) == Some(id) {
            state.end = Some(End::Rewrite { refuses: false });
        }
    }
    let keeps_escape = part.options.keep_on_escape && !part.options.clear_on_escape && !read_only && cx.core.resolved.enabled && part.options.enabled;
    let resolved = resolve(&mut state, cx.value(), commit_now, keeps_escape);
    if resolved.escaped && keeps_escape && !state.invalid {
        // The same Escape goes on to the enclosing scope in this pass, which still finds this focused stop; the field
        // lets focus go once the scopes have run.
        cx.core.intents.pass_on(craft_behavior::Intent::Cancel);
        crate::runtime::focus::surrender_after_scopes(&ctx, widget);
    }
    if resolved.refused {
        crate::runtime::frame::refuse(&ctx);
    }
    if resolved.rewritten {
        let len = state.draft.as_deref().map_or(0, |draft| draft.chars().count());
        adapter::select(&ctx, widget, 0, len);
        crate::runtime::egui::repaint::request(&ctx);
    }
    if let Some(baseline) = out.baseline {
        cx.core.paint.borrow_mut().baseline.get_or_insert(baseline);
    }
    let enabled = cx.core.resolved.enabled && part.options.enabled;
    facts(cx.core, &mut state, widget, &out, part.rect, FieldFacts { masked: part.options.mask, read_only, enabled });
    cx.core.text_invalid = state.invalid;
    cx.core.text_unparsed = part.options.unparsed.clone().filter(|_| state.invalid);
    cx.core.escape_to_list = state.completions.open || list_open;
    if state.draft.is_some() || (part.options.clear_on_escape && !out.layout.full().is_empty()) || (keeps_escape && out.focused) {
        keys::hold_escape(&ctx, id);
    }
    let completions = state.completions.items.clone();
    // The caret after every move of this pass, which a bound caret is written back from.
    let caret = adapter::caret_byte(&ctx, widget, state.draft.as_deref().unwrap_or(out.layout.full()));
    *cx.core.memory.shown::<TextState>(state_id) = state;
    let mut outcome = resolved.outcome;
    outcome.layout = Some(out.layout);
    outcome.caret_visible = caret_visible;
    outcome.composing = crate::runtime::ime::composing(&ctx, widget);
    outcome.completions = completions;
    outcome.clicked = out.clicked;
    // The focus service moved focus off the widget with Tab before this pass's widgets ran.
    let left_by_tab = if was_focused && !out.focused { crate::runtime::focus::tabbed_off(&ctx, widget) } else { None };
    // The edit menu holds the part's editing focus while it is open (`was_focused` stays true through it): key focus
    // that is with the menu is not lost until the menu is gone and focus is not back.
    let text_lost_focus = was_focused && !menu_open && (!out.focused || left_for_part);
    cx.core.texts =
        Some((part.key, Box::new(RecordedText { outcome, submit_mods: resolved.submit_mods, left_by_tab, tab: out.tab, caret, text_lost_focus })));
    apply_events(cx, resolved.events);
    cx.core.memory.shown::<TextState>(state_id).seen = shown_copy(cx);
    cx.core.submitted |= resolved.submitted;
    // A read-only multi-line part makes no line break, so its Enter goes on to the default button.
    if keys.submits && !(read_only && part.options.multiline()) {
        cx.core.intents.take(craft_behavior::Intent::Submit);
    }
    cx.core.escaped |= resolved.escaped;
}

impl<C: Component> Cx<'_, C> {
    /// Changes the text part's draft in the next pass: `Some(text)` puts `text` in as a change of the typing run (one
    /// that begins the run when none is under way), the caret at its end; `None` ends the run with the text from
    /// before it, without the run's Escape reaching the host. A combo box's keys in its open list put the active
    /// option's text in, and its list's Escape puts the typed text back.
    pub fn set_draft(&mut self, text: Option<String>) {
        self.core.memory.shown::<TextState>(state_id(self.core.id)).change =
            Some(text.map_or(adapter::DraftChange::Revert, adapter::DraftChange::Text));
        crate::runtime::egui::repaint::request(&self.core.ctx);
    }

    /// Selects the whole text of the text part at the start of the next pass, as a field that takes focus with
    /// `select_on_focus` does: a font field's click that opens its list selects the name, so typing replaces it.
    pub fn select_all_text(&mut self) {
        self.core.memory.shown::<TextState>(state_id(self.core.id)).select_all = true;
        crate::runtime::egui::repaint::request(&self.core.ctx);
    }

    /// Accepts candidate `at` of the text part's completion list in the next pass: a click on a row of the list.
    #[doc(hidden)]
    pub fn pick_completion(&mut self, at: usize) {
        self.core.memory.shown::<TextState>(state_id(self.core.id)).completions.picked = Some(at);
        crate::runtime::egui::repaint::request(&self.core.ctx);
    }

    /// The key of the text part step 8b ran in this pass; `None` for a component that declared none.
    pub fn text_part(&self) -> Option<PartKey> {
        self.core.texts.as_ref().map(|(part, _)| *part)
    }

    /// The result of the text part `key` this pass: the draft, the laid-out text and the caret blink. `None` while
    /// the text service has not produced one (a part first registered this pass), and for a key `parts` never
    /// registered, which is a debug assertion.
    pub fn text<V: 'static>(&self, key: &TextKey<V>) -> Option<&TextOutcome<V>> {
        let outcome =
            self.core.texts.as_ref().filter(|(part, _)| *part == key.key()).and_then(|(_, text)| text.as_any().downcast_ref::<RecordedText<V>>());
        let outcome = outcome.map(|recorded| &recorded.outcome);
        debug_assert!(outcome.is_some() || self.core.texts.is_none(), "the text part {:?} was never registered by `parts`", key.key());
        outcome
    }
}
