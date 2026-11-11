//! The text part of the pipeline (18.2): `PartsCx::text` registers the part with its typing run, and step 8b runs the
//! text adapter, then the typing run over what the adapter recorded, and applies the run's events through `Cx`.

use super::cx::{Cx, PartsCx, RecordedText, gesture_key};
use super::{Component, Edit as _, Focus as _, Overlays as _, ValueOf};
use crate::primitives::text_adapter::{self as adapter, End, TextState};
use crate::primitives::{TextKey, TextOptions, TextOutcome};
use crate::runtime::access::text as text_access;
use crate::runtime::keys;
use crate::runtime::memory::sub;
use crate::runtime::motion::Animate as _;
use craft_behavior::{CancelReason, Caret, Codec, EditRun, Ended, EscapeRule, Escaped, Gesture, NotAValue, PartKey, Typed, TypingRun};
use craft_keymap::Mods;
use egui::{Rect, Ui};
use std::any::Any;
use std::sync::Arc;

/// The id of the text adapter's state in the component's memory.
fn state_id(id: egui::Id) -> egui::Id {
    sub(id, "text_state")
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
}

pub(crate) struct TextDecl {
    pub key: PartKey,
    pub rect: Rect,
    pub shown: String,
    pub options: TextOptions,
    pub caret: Caret,
    /// A `Resolve<V>` for the component's value type: the typing run, run once the adapter has recorded the pass.
    pub resolve: Box<dyn Any>,
    pub values: ValueOps,
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
}

/// The typing run over the state the adapter recorded this pass, the component's value (`None` while mixed) and
/// whether a commit is due.
type Resolve<V> = Box<dyn FnOnce(&mut TextState, Option<&V>, bool) -> Resolved<V>>;

/// Turns what the adapter recorded this pass into the typing run's events: a parsed draft applies in the pass it
/// is typed, and a commit proposes and commits in one pass. `fallback` is the value the part shows, the run's start
/// while the component's value is mixed.
fn resolver<K: Codec + Clone + 'static>(codec: K, options: &TextOptions, rect: Rect, fallback: K::Value) -> Resolve<K::Value> {
    let (live, limit, clear_on_escape) = (options.live, options.limit, options.clear_on_escape);
    Box::new(move |state, value, commit_now| {
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
        if std::mem::take(&mut state.dirty)
            && let Some(draft) = state.draft.clone()
        {
            let start = run.start.clone();
            if let Typed::Apply(applied) = typing.edit(&mut run, &start, &draft) {
                events.push(TextEvent::Propose(applied));
            }
        }
        let draft_text = run.draft.clone().unwrap_or_default();
        let (mut submitted, mut escaped) = (false, false);
        let mut committed_tag = None;
        let submit_mods = std::mem::take(&mut state.submit_mods);
        let end = match state.end.take() {
            None if commit_now && run.is_open() => Some(End::Blur),
            end => end,
        };
        match end {
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
            Some(end @ (End::Submit | End::Separator)) => match typing.commit_tagged(&mut run) {
                (Ended::Commit(committed), tag) => {
                    committed_tag = tag;
                    state.normalised = Some(typing.codec().normalise(&draft_text, &committed));
                    events.push(TextEvent::Commit(committed));
                    submitted = end == End::Submit;
                }
                (Ended::Rejected, _) => {}
                (Ended::Idle, _) => submitted = end == End::Submit,
                _ => {}
            },
            Some(End::Blur) => match typing.commit_tagged(&mut run) {
                (Ended::Commit(committed), tag) => {
                    committed_tag = tag;
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
        outcome.committed_tag = committed_tag;
        Resolved { events, outcome, submitted, submit_mods: if submitted { submit_mods } else { Mods::NONE }, escaped }
    })
}

impl<C: Component> PartsCx<'_, C> {
    /// Registers the component's one text part (`INPUT = Input::Text`).
    pub fn text<K: Codec + Clone + 'static>(&mut self, key: TextKey<K::Value>, rect: Rect, codec: &K, value: &K::Value, options: &TextOptions) {
        debug_assert!(self.text.decl.is_none(), "a component registers one text part");
        let shown = if options.mixed { String::new() } else { codec.format(value) };
        self.text.decl = Some(TextDecl {
            key: key.0,
            rect,
            caret: if options.select_on_focus { Caret::All } else { codec.caret_on_focus(&shown) },
            shown,
            options: options.clone(),
            resolve: Box::new(resolver(codec.clone(), options, rect, value.clone())),
            values: ValueOps::of::<K::Value>(),
        });
    }
}

/// Serves the AccessKit requests aimed at the text part: `SetTextSelection` moves the selection and `SetValue`
/// replaces the draft and commits it.
fn serve_access<C: Component>(cx: &mut Cx<'_, C>, state: &mut TextState, widget: egui::Id, shown: &str, read_only: bool) {
    let (ctx, id) = (cx.core.ctx.clone(), cx.core.id);
    while let Some(request) = cx.core.intents.take_access_where(text_access::is_text_request) {
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

/// The text facts AccessKit reads this pass, built only while AccessKit runs.
fn facts<C: Component>(
    cx: &Cx<'_, C>,
    state: &mut TextState,
    widget: egui::Id,
    out: &adapter::AdapterOut,
    rect: Rect,
    masked: bool,
    read_only: bool,
) {
    if !text_access::exposed(&cx.core.ctx, widget) {
        state.rows = None;
        return;
    }
    let rows = adapter::Rows::of(&out.layout, &out.layout.full, rect.min);
    let text = out.layout.full.clone();
    let selection = adapter::selection(&cx.core.ctx, widget).map(|(caret, anchor)| (rows.pos(&text, anchor), rows.pos(&text, caret)));
    *cx.core.text_access.borrow_mut() =
        Some(text_access::TextFacts { widget, text, masked, editable: !read_only, lines: rows.lines.clone(), selection });
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
    let Some(decl) = step.decl.take() else { return };
    let Ok(resolve) = decl.resolve.downcast::<Resolve<ValueOf<C>>>() else {
        debug_assert!(false, "the text part's codec value is the component's value");
        return;
    };
    let resolve = *resolve;
    let (ctx, id) = (cx.core.ctx.clone(), cx.core.id);
    let state_id = state_id(id);
    let mut state = std::mem::take(cx.core.memory.shown::<TextState>(state_id));
    (decl.values.resync)(&mut state, shown_copy(cx).as_ref());
    let widget = adapter::text_id(id, decl.key);
    let mut focused = crate::runtime::egui::focus::has_focus(&ctx, widget);
    let was_focused = state.focused;
    let now = cx.now();
    let caret_visible = !focused || cx.blink(state.caret_moved);
    let enabled = cx.core.resolved.enabled && decl.options.enabled;
    let focusable = super::steps::takes_focus(cx.core.resolved) && decl.options.enabled;
    // A component that senses a drag on its root scrubs its value by it: the field takes the pointer only once it is
    // in edit, and a click that enters it puts the caret where it went down.
    let scrubs = cx.core.root_sense.contains(craft_behavior::Sense::DRAG);
    if scrubs {
        let before = cx.core.memory.shown::<InEditBeforePress>(sub(id, "in_edit_before_press"));
        if cx.core.press.phase.is_drag() {
            // The root press focused the field; a press that travelled is a scrub, so the field is given back.
            if !before.0 {
                crate::runtime::egui::focus::surrender(&ctx, widget);
                focused = false;
            }
        } else if !cx.core.press.phase.button_down() {
            before.0 = focused;
        }
    }
    let pointer = !scrubs || (focused && !cx.core.press.phase.is_drag());
    let click = match cx.core.press.phase {
        craft_behavior::PressPhase::Release { pos, click: true, .. } if scrubs && focusable => Some(pos),
        _ => None,
    };
    let read_only = cx.core.resolved.read_only || !enabled;
    let ink = decl.options.ink.map_or(ink, |own| crate::style::ink_color(&crate::runtime::env::Env::of(ui).tokens, own));
    let ink = if enabled { ink } else { crate::style::rgba_color(crate::runtime::env::Env::of(ui).tokens.color.text.disabled) };
    serve_access(cx, &mut state, widget, &decl.shown, read_only);
    cx.core.surface_slot = Some(ui.painter().add(egui::Shape::Noop));
    let out = adapter::run(
        &mut state,
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
            click,
            rtl,
            focused,
            ink,
            intents: &mut cx.core.intents,
            placeholder: decl.options.placeholder.as_ref().map(|label| label.as_str()),
            caret_visible,
            underline,
            now,
        },
    );
    // Focus that moved to another part of the component leaves the text part like a focus loss: the draft commits.
    let left_for_part = out.focused && cx.active_part().is_some();
    if left_for_part {
        state.focused = false;
    }
    let commit_now = step.commit_now || (was_focused && left_for_part);
    if out.clicked_char.is_some() {
        state.click_caret = out.clicked_char;
    }
    if out.focused && !left_for_part && !was_focused {
        let len = decl.shown.chars().count();
        match (state.click_caret.take(), decl.caret) {
            (Some(at), _) => adapter::select(&ctx, widget, at, at),
            (None, Caret::All) => adapter::select(&ctx, widget, 0, len),
            (None, _) => adapter::select(&ctx, widget, len, len),
        }
    } else if out.focused
        && let Some(at) = state.click_caret.take()
    {
        adapter::select(&ctx, widget, at, at);
    }
    if decl.options.complete.is_some() {
        drive_completions(cx, &mut state);
    }
    let resolved = resolve(&mut state, cx.value(), commit_now);
    if let Some(baseline) = out.baseline {
        cx.core.paint.borrow_mut().baseline.get_or_insert(baseline);
    }
    facts(cx, &mut state, widget, &out, decl.rect, decl.options.mask, read_only);
    cx.core.text_invalid = state.invalid;
    cx.core.escape_to_list = state.completions.open;
    if state.draft.is_some() || (decl.options.clear_on_escape && !out.layout.full.is_empty()) {
        keys::hold_escape(&ctx, id);
    }
    let completions = state.completions.items.clone();
    *cx.core.memory.shown::<TextState>(state_id) = state;
    let mut outcome = resolved.outcome;
    outcome.layout = Some(out.layout);
    outcome.caret_visible = caret_visible;
    outcome.composing = crate::runtime::ime::composing(&ctx, widget);
    outcome.completions = completions;
    // The focus service moved focus off the widget with Tab before this pass's widgets ran.
    let left_by_tab = if was_focused && !out.focused { crate::runtime::focus::tabbed_off(&ctx, widget) } else { None };
    cx.core.texts = Some((decl.key, Box::new(RecordedText { outcome, submit_mods: resolved.submit_mods, left_by_tab })));
    apply_events(cx, resolved.events);
    cx.core.memory.shown::<TextState>(state_id).seen = shown_copy(cx);
    cx.core.submitted |= resolved.submitted;
    if keys.submits {
        cx.core.intents.take(craft_behavior::Intent::Submit);
    }
    cx.core.escaped |= resolved.escaped;
}

impl<C: Component> Cx<'_, C> {
    /// Accepts candidate `at` of the text part's completion list in the next pass: a click on a row of the list.
    #[doc(hidden)]
    pub fn pick_completion(&mut self, at: usize) {
        self.core.memory.shown::<TextState>(state_id(self.core.id)).completions.picked = Some(at);
        crate::runtime::egui::repaint::request(&self.core.ctx);
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
