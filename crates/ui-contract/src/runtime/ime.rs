//! The IME engine (18.4): who owns the IME this frame, the composing flag, preedit and commit delivery, and the
//! caret rect for the OS. It is the one writer of `PlatformOutput.ime`.

use crate::runtime::egui::ime as os;
use crate::runtime::egui::pass;
use crate::runtime::frame::PerViewport;
use craft_behavior::EditorInput;
use egui::{Context, Event, Id, Key, Rect, ViewportId};

/// Frames after a commit in which the keys that end a composition stay guarded.
const GUARD_FRAMES: u8 = 1;

#[derive(Default)]
struct Engine {
    /// Events taken from raw input, delivered at the begin hook.
    queue: Vec<os::Raw>,
    owner: Option<Id>,
    generation: u64,
    composing: bool,
    guard: u8,
    /// A reset was asked for; events of the old composition are dropped until the OS answers or the IME disables.
    stale: bool,
    /// Edits for `owner`, in arrival order, until its run of this pass takes them.
    edits: Vec<EditorInput>,
    /// `owner` took its edits this pass; an owner not shown for a pass never does, and its edits are dropped.
    ran: bool,
    /// The preedit of the owner that lost focus, for its adapter to append as typed text.
    leaving: Option<(Id, String)>,
    preedit: String,
    caret: Option<(Id, Rect, Rect, bool)>,
    interrupt: Option<Id>,
    /// What the editor that owns the IME reported at its last pass.
    editor: Option<EditorFacts>,
}

#[derive(Clone, Copy)]
struct EditorFacts {
    owner: Id,
    revision: u64,
    can_undo: bool,
    can_redo: bool,
}

fn with<R>(ctx: &Context, viewport: ViewportId, f: impl FnOnce(&mut Engine) -> R) -> R {
    PerViewport::<Engine>::with(ctx, viewport, f)
}

fn current<R>(ctx: &Context, f: impl FnOnce(&mut Engine) -> R) -> R {
    with(ctx, pass::current_viewport(ctx), f)
}

/// The input hook: takes every IME event out of raw input and removes the keys that would end the composition.
pub(crate) fn input_hook(ctx: &Context, events: &mut Vec<Event>, viewport: ViewportId) {
    let taken = os::take_events(events);
    with(ctx, viewport, |engine| {
        engine.queue.extend(taken);
        let guarded = engine.composing || engine.guard > 0;
        if guarded {
            events.retain(|event| {
                !matches!(event, Event::Key { key: Key::Enter | Key::Escape | Key::Backspace | Key::Delete | Key::Space, pressed: true, .. })
            });
        }
        engine.guard = engine.guard.saturating_sub(1);
    });
}

/// The begin hook, after pending focus moves are applied: resolves the owner and turns the queue into edits.
pub(crate) fn begin(ctx: &Context, owner: Option<Id>) {
    current(ctx, |engine| {
        if engine.owner != owner {
            if engine.composing
                && let Some(old) = engine.owner
            {
                engine.leaving = Some((old, std::mem::take(&mut engine.preedit)));
                engine.interrupt = Some(old);
                engine.stale = true;
            }
            engine.composing = false;
            engine.edits.clear();
            engine.owner = owner;
            engine.generation += 1;
        }
        for raw in std::mem::take(&mut engine.queue) {
            match raw {
                os::Raw::ResetCompleted => engine.stale = false,
                _ if engine.stale || engine.owner.is_none() => {}
                os::Raw::Preedit { text, cursor_chars } => {
                    engine.composing = !text.is_empty();
                    let cursor = cursor_chars.map(|range| {
                        let at = |chars: usize| text.char_indices().nth(chars).map_or(text.len(), |(byte, _)| byte);
                        at(range.start)..at(range.end)
                    });
                    engine.preedit = text.clone();
                    engine.edits.push(EditorInput::preedit(text, cursor));
                }
                os::Raw::Commit(text) => {
                    engine.composing = false;
                    engine.preedit.clear();
                    engine.guard = GUARD_FRAMES;
                    engine.edits.push(EditorInput::ImeCommit);
                    engine.edits.push(EditorInput::Text(text.into()));
                }
            }
        }
    });
}

/// The edits waiting for `owner`; empty for any other component. A read-only owner takes none and never composes:
/// a composition it was in is interrupted.
pub(crate) fn take(ctx: &Context, owner: Id, read_only: bool) -> Vec<EditorInput> {
    current(ctx, |engine| {
        if engine.owner != Some(owner) {
            return Vec::new();
        }
        engine.ran = true;
        if read_only {
            engine.edits.clear();
            if engine.composing {
                engine.composing = false;
                engine.stale = true;
                engine.interrupt = Some(owner);
            }
            engine.preedit.clear();
            return Vec::new();
        }
        std::mem::take(&mut engine.edits)
    })
}

/// What an editor that draws its own caret reports once per pass. A new `revision` that follows no edit the engine
/// delivered (`edited`) is an app change: the composition context is stale, so the OS composition is interrupted
/// and its late events are dropped.
pub(crate) fn note_editor(ctx: &Context, owner: Id, revision: u64, edited: bool, history: (bool, bool)) {
    current(ctx, |engine| {
        let changed = engine.editor.is_some_and(|last| last.owner == owner && last.revision != revision);
        if changed && !edited && engine.owner == Some(owner) && engine.composing {
            engine.composing = false;
            engine.preedit.clear();
            engine.stale = true;
            engine.interrupt = Some(owner);
        }
        engine.editor = Some(EditorFacts { owner, revision, can_undo: history.0, can_redo: history.1 });
    });
}

/// Whether the editor `owner` had something to undo and to redo at its last report.
pub(crate) fn editor_history(ctx: &Context, owner: Id) -> (bool, bool) {
    current(ctx, |engine| engine.editor.filter(|facts| facts.owner == owner).map_or((false, false), |facts| (facts.can_undo, facts.can_redo)))
}

/// The preedit of `owner` when it lost focus while composing, once: its adapter appends it to the draft.
pub(crate) fn take_leaving(ctx: &Context, owner: Id) -> Option<String> {
    current(ctx, |engine| match &engine.leaving {
        Some((old, _)) if *old == owner => engine.leaving.take().map(|(_, text)| text),
        _ => None,
    })
}

/// Whether `owner` is composing.
pub(crate) fn composing(ctx: &Context, owner: Id) -> bool {
    current(ctx, |engine| engine.owner == Some(owner) && engine.composing)
}

/// Records the caret rect for the OS candidate window.
pub(crate) fn note_caret(ctx: &Context, owner: Id, field: Rect, caret: Rect, password: bool) {
    current(ctx, |engine| engine.caret = Some((owner, field, caret, password)));
}

/// The end hook: writes the pass's IME output.
pub(crate) fn end_pass(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let (owner, interrupt, caret, generation) = with(ctx, viewport, |engine| {
        if !std::mem::take(&mut engine.ran) {
            engine.edits.clear();
        }
        (engine.owner, engine.interrupt.take(), engine.caret.take(), engine.generation)
    });
    os::clear(ctx);
    match (caret, interrupt, owner) {
        (Some((id, field, caret, password)), interrupt, _) if Some(id) == owner => {
            os::write(ctx, viewport, os::Output { owner: id, field, caret, interrupt: interrupt.is_some(), generation, password });
        }
        // The old owner still gets an output in the pass it lost focus, or the OS never interrupts.
        (_, Some(old), _) => os::write(
            ctx,
            viewport,
            os::Output { owner: old, field: Rect::NOTHING, caret: Rect::NOTHING, interrupt: true, generation, password: false },
        ),
        _ => {}
    }
}
