//! The IME engine (18.4): who owns the IME this frame, the composing flag, preedit and commit delivery, and the
//! caret rect for the OS. While a framework editor owns the IME, the engine is the one reader of the IME events and
//! the one writer of `PlatformOutput.ime`; otherwise both stay with egui and the app's own widgets.

use crate::runtime::egui::ime as os;
use crate::runtime::egui::input::Arrived;
use crate::runtime::egui::pass;
use crate::runtime::frame::{PerViewport, Stored};
use craft_behavior::EditorInput;
use egui::ime_context::{CommittedText, ImeContextTracker, ImeContextUpdate, ImeMutationStamp, ImeObservation, ImeTarget};
use egui::output::{IMESurroundingText, ImeChangeCause};
use egui::{Context, Event, IMEPurpose, Id, Key, Rect, ViewportId};

/// Frames after a commit in which the keys that end a composition stay guarded.
const GUARD_FRAMES: u8 = 1;

#[derive(Default)]
struct Engine {
    /// Events taken from raw input, delivered at the begin hook, each with its place among the frame's other events
    /// (`os::take_events`).
    queue: Vec<(os::Raw, usize)>,
    owner: Option<Id>,
    /// The egui fork's input-method context tracker of this viewport: each end hook observes the owner once, and the
    /// tracker gives the output its context generation, its bounded surrounding text and the cause of a change.
    tracker: ImeContextTracker,
    /// The target the owner was observed as at the last end hook, and whether it was a password. A receipt or an
    /// application change names this target, because the tracker accepts one only for the context it last observed;
    /// a pass whose owner reports no caret keeps the purpose, so the context does not change for it.
    observed: Option<(ImeTarget, bool)>,
    /// The context generation of the last output written for an owner. The interrupt the old owner gets in the pass
    /// it lost focus names this context: the one the OS is composing in. A new generation, or none, names a context
    /// the OS never saw.
    published: Option<u64>,
    composing: bool,
    guard: u8,
    /// A reset was asked for; events of the old composition are dropped until its completion arrives.
    stale: bool,
    /// The reset request `stale` waits for, as the pass that published it got it back. `None` before it is published,
    /// and once the integration reports it retired (`Event::ImeResetRetired`): the next pass with an owner asks again.
    reset: Option<egui::ImeResetId>,
    /// Edits for `owner`, in arrival order, each with its event's place, until its run of this pass takes them.
    edits: Vec<(EditorInput, usize)>,
    /// `owner` took its edits this pass; an owner not shown for a pass never does, and its edits are dropped.
    ran: bool,
    /// The preedit of the owner that lost focus, for its adapter to append as typed text.
    leaving: Option<(Id, String)>,
    preedit: String,
    caret: Option<(Id, Rect, Rect, bool)>,
    interrupt: Option<Id>,
    /// An interrupt asked the platform to discard its marked text; `take_discard` reads it once.
    discard: bool,
    /// What the editor that owns the IME reported at its last pass.
    editor: Option<EditorFacts>,
    /// The app's own editor that wrote the IME output of the last pass while no framework editor owned the IME.
    host: Option<HostIme>,
    /// The reset requests of the app's own editor that a published IME output carried, its own or a framework editor's
    /// it interrupted, oldest first, unanswered yet: their completions and retirements are the app's and stay in
    /// egui's input for it, fence or not. The latest one published while the fence is up is the fence's `reset`.
    host_resets: Vec<egui::ImeResetId>,
}

/// An app's own editor holding the IME: the owner and context generation of the IME output it wrote, and whether it
/// is composing, as the IME events left to it in egui's input say.
#[derive(Clone, Copy)]
struct HostIme {
    owner: Id,
    generation: u64,
    composing: bool,
}

impl Stored for Engine {}

#[derive(Clone)]
struct EditorFacts {
    owner: Id,
    composing: bool,
    can_undo: bool,
    can_redo: bool,
    /// The committed text around the caret the owner reported this pass; the end hook takes it, so a pass whose
    /// owner reports nothing observes no text.
    surrounding: Option<Surrounding>,
}

/// An owner's surrounding report as the engine keeps it for one pass: an owned copy of at most
/// `MAX_IME_SURROUNDING_TEXT_BYTES` around the selection, never the whole document, and the epoch of the target.
/// The committed window around the caret that an editor reports to the input method, or why there is none.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum EditorSurrounding<'a> {
    /// The text around the caret, with UTF-8 offsets of the cursor and the selection anchor into it.
    #[non_exhaustive]
    Text {
        /// The committed text.
        text: &'a str,
        /// The cursor's byte offset into `text`.
        cursor: usize,
        /// The selection anchor's byte offset into `text`.
        anchor: usize,
    },
    /// The editor has no text to report.
    #[non_exhaustive]
    Unavailable {
        /// The revision the editor reports it at, if it keeps one.
        revision: Option<u64>,
    },
}

impl<'a> EditorSurrounding<'a> {
    /// The text around the caret, with byte offsets of the cursor and the selection anchor into `text`.
    pub const fn text(text: &'a str, cursor: usize, anchor: usize) -> Self {
        EditorSurrounding::Text { text, cursor, anchor }
    }

    /// No text to report, at `revision` if the editor keeps one.
    pub const fn unavailable(revision: Option<u64>) -> Self {
        EditorSurrounding::Unavailable { revision }
    }
}

#[derive(Clone)]
struct Surrounding {
    epoch: u64,
    text: IMESurroundingText,
    /// The editor's revision of an unavailable report, which orders two unavailable stamps of one receipt.
    revision: Option<u64>,
}

impl Surrounding {
    /// Copies the bounded window of `report`. Offsets that are not character boundaries give no text.
    fn of(epoch: u64, report: &EditorSurrounding<'_>) -> Surrounding {
        match report {
            EditorSurrounding::Text { text, cursor, anchor, .. } => {
                Surrounding { epoch, text: IMESurroundingText::from_text(text, *cursor, *anchor, None), revision: None }
            }
            EditorSurrounding::Unavailable { revision, .. } => Surrounding { epoch, text: IMESurroundingText::Unavailable, revision: *revision },
        }
    }
}

impl Engine {
    /// Observes this pass's owner on the tracker, once per pass. The owner's target is its id with the epoch of its
    /// report; an owner that reported nothing this pass (a text field, an editor with no surrounding report) keeps its
    /// last epoch (0 before any report) and is observed with no committed text (`committed(None)`). No owner observes `None`, so the next owner
    /// starts a new context.
    ///
    /// A second pass of a frame is the tracker's next pass of the same frame: it applies only the receipts noted in
    /// that pass, and an unchanged snapshot keeps the cause the frame's first pass found.
    fn observe(&mut self, viewport: ViewportId, frame_nr: u64, pass_nr: u64, password: Option<bool>) -> Option<ImeContextUpdate> {
        let Some(owner) = self.owner else {
            self.observed = None;
            return self.tracker.observe(viewport, None);
        };
        let kept = self.observed.is_some_and(|(target, password)| target.owner == owner && password);
        let password = password.unwrap_or(kept);
        let report = self.editor.as_mut().filter(|facts| facts.owner == owner).and_then(|facts| facts.surrounding.take());
        // Only a report moves the epoch: an owner that reports nothing this pass stays the same target.
        let kept_epoch = self.observed.map(|(target, _)| target).filter(|target| target.owner == owner).map_or(0, |target| target.replacement_epoch);
        let target = ImeTarget::new(owner, report.as_ref().map_or(kept_epoch, |report| report.epoch));
        let purpose = if password { IMEPurpose::Password } else { IMEPurpose::Normal };
        let mut observation = ImeObservation::new(frame_nr, pass_nr, target).purpose(purpose);
        if let Some(report) = &report {
            if let Some(payload) = report.text.available() {
                observation = observation.committed(Some(CommittedText::new(&payload.text, payload.cursor, payload.anchor)));
            } else if let Some(revision) = report.revision {
                observation = observation.unavailable_revision(revision);
            }
        }
        self.observed = Some((target, password));
        self.tracker.observe(viewport, Some(observation))
    }

    /// The target `owner` was observed as at the last end hook; `None` when it was not the observed owner.
    fn target_of(&self, owner: Id) -> Option<ImeTarget> {
        self.observed.map(|(target, _)| target).filter(|target| target.owner == owner)
    }

    /// Hands the IME to `owner`. The old owner, if it was composing, keeps its preedit for its adapter and is
    /// interrupted. The tracker starts the new owner's context when the end hook observes it.
    fn set_owner(&mut self, owner: Option<Id>) {
        if self.owner == owner {
            return;
        }
        if self.composing
            && let Some(old) = self.owner
        {
            self.leaving = Some((old, std::mem::take(&mut self.preedit)));
            self.interrupt(old);
        }
        self.composing = false;
        self.edits.clear();
        self.owner = owner;
    }

    /// Ends the composition of `owner` here and has the end hook interrupt the OS composition; its late events are
    /// dropped until the reset completes.
    fn interrupt(&mut self, owner: Id) {
        self.composing = false;
        self.preedit.clear();
        self.stale = true;
        self.interrupt = Some(owner);
        self.discard = true;
    }

    /// Interrupts the owner's input method context: a running composition is ended and fenced (`interrupt`); with none
    /// running there are no late events to drop, and only the interrupt goes out.
    fn interrupt_owner(&mut self, owner: Id) {
        if self.composing {
            self.interrupt(owner);
        } else {
            self.interrupt = Some(owner);
        }
    }

    /// Notes an answer to one of the app's own reset requests. A completion answers that request and every older one,
    /// as completions arrive in request order, and ends the fence when it waits for one of them; a retirement answers
    /// only its own request and, when the fence waits for it, leaves the end hook to ask again.
    fn host_answered(&mut self, event: &Event) {
        let (Event::ImeResetCompleted { request_id: id } | Event::ImeResetRetired { request_id: id }) = event else { return };
        let Some(at) = self.host_resets.iter().position(|own| own == id) else { return };
        let answered: Vec<_> = match event {
            Event::ImeResetCompleted { .. } => self.host_resets.drain(..=at).collect(),
            _ => vec![self.host_resets.remove(at)],
        };
        if self.reset.is_some_and(|reset| answered.contains(&reset)) {
            self.reset = None;
            if matches!(event, Event::ImeResetCompleted { .. }) {
                self.stale = false;
            }
        }
    }

    /// Follows the composition of the app's own editor through the IME events left to it in egui's input, and
    /// interrupts it at a pointer press, as a press ends a framework editor's composition. The one-press rule (H0)
    /// holds the events after the press for the next frame, where the fence drops the composition's late deletions and
    /// commits until the reset completes.
    fn watch_host(&mut self, events: &[Arrived]) {
        let Some(host) = self.host.as_mut() else { return };
        for arrived in events {
            match (os::read(&arrived.event), &arrived.event) {
                (Some(os::Raw::Preedit { text, .. }), _) => host.composing = !text.is_empty(),
                (Some(os::Raw::Commit(_)), _) => host.composing = false,
                (None, Event::PointerButton { pressed: true, .. }) if host.composing => {
                    host.composing = false;
                    let owner = host.owner;
                    self.interrupt(owner);
                    return;
                }
                _ => {}
            }
        }
    }
}

fn with<R>(ctx: &Context, viewport: ViewportId, f: impl FnOnce(&mut Engine) -> R) -> R {
    PerViewport::<Engine>::with(ctx, viewport, f)
}

fn current<R>(ctx: &Context, f: impl FnOnce(&mut Engine) -> R) -> R {
    with(ctx, pass::current_viewport(ctx), f)
}

/// The input hook: takes the IME events out of raw input while a framework editor owns the IME, or while the reset
/// the engine asked for is pending (the late events of the interrupted composition, and the OS's answer), and removes
/// the keys that would end the composition. With no framework editor, the events after the reset's completion are
/// the app's own editor's again, and the hook follows that editor's composition (`Engine::watch_host`). The answers to
/// the app's own reset requests always stay in egui's input for it; when the fence waits for one of them, its
/// completion ends the fence too.
pub(crate) fn input_hook(ctx: &Context, events: &mut Vec<Arrived>, viewport: ViewportId) {
    with(ctx, viewport, |engine| {
        let take = engine.owner.is_some() || engine.reset.is_some();
        let host_resets = engine.host_resets.clone();
        let host_answer = |event: &Event| match event {
            Event::ImeResetCompleted { request_id } | Event::ImeResetRetired { request_id } => host_resets.contains(request_id),
            _ => false,
        };
        let completed =
            |id: egui::ImeResetId| move |arrived: &Arrived| matches!(arrived.event, Event::ImeResetCompleted { request_id } if request_id == id);
        // The events after the completion are the app's; the completion too when it answers the app's own request.
        let mut after = match engine.reset {
            Some(id) if engine.owner.is_none() => {
                let own = host_resets.contains(&id);
                events.iter().position(completed(id)).map(|at| events.split_off(if own { at } else { at + 1 }))
            }
            _ => None,
        };
        let guarded = engine.composing || engine.guard > 0;
        let ends_composition = |event: &Event| {
            matches!(event, Event::Key { key: Key::Enter | Key::Escape | Key::Backspace | Key::Delete | Key::Space, pressed: true, .. })
        };
        let taken = os::take_events(events, |event| take && !host_answer(event), |event| guarded && ends_composition(event));
        engine.queue.extend(taken);
        engine.guard = engine.guard.saturating_sub(1);
        for arrived in events.iter().chain(after.iter().flatten()) {
            engine.host_answered(&arrived.event);
        }
        if engine.owner.is_none() {
            match &mut after {
                Some(after) => engine.watch_host(after),
                None if !take => engine.watch_host(events),
                None => {}
            }
        }
        events.extend(after.into_iter().flatten());
    });
}

/// Whether an `interrupt` of the engine of `viewport` asks the platform to discard the marked text: once for each
/// interrupt. The engine is per viewport and the caller runs before any pass, so it names the viewport.
pub(crate) fn take_discard(ctx: &Context, viewport: ViewportId) -> bool {
    with(ctx, viewport, |engine| std::mem::take(&mut engine.discard))
}

/// The begin hook, after pending focus moves are applied: resolves the owner and turns the queue into edits.
pub(crate) fn begin(ctx: &Context, owner: Option<Id>) {
    current(ctx, |engine| {
        engine.set_owner(owner);
        for (raw, place) in std::mem::take(&mut engine.queue) {
            match raw {
                // A completion of an earlier request is stale: the latest request still fences.
                os::Raw::ResetCompleted(id) => {
                    if engine.reset == Some(id) {
                        engine.stale = false;
                        engine.reset = None;
                    }
                }
                // The integration dropped the request without completing it; the end hook asks again.
                os::Raw::ResetRetired(id) => {
                    if engine.reset == Some(id) {
                        engine.reset = None;
                    }
                }
                _ if engine.stale || engine.owner.is_none() => {}
                os::Raw::Preedit { text, cursor_chars } => {
                    engine.composing = !text.is_empty();
                    let cursor = cursor_chars.map(|range| {
                        let at = |chars: usize| text.char_indices().nth(chars).map_or(text.len(), |(byte, _)| byte);
                        at(range.start)..at(range.end)
                    });
                    engine.preedit = text.clone();
                    engine.edits.push((EditorInput::preedit(text, cursor), place));
                }
                os::Raw::DeleteSurrounding { before, after } => engine.edits.push((EditorInput::delete_surrounding(before, after), place)),
                os::Raw::DeleteSurroundingBytes { before, after } => {
                    engine.edits.push((EditorInput::delete_surrounding_bytes(before, after), place));
                }
                os::Raw::Commit(text) => {
                    let marked = std::mem::take(&mut engine.composing);
                    engine.preedit.clear();
                    // An empty commit with nothing marked inserts nothing, so it replaces no selection either, and
                    // ends no composition whose confirming key needs the guard.
                    if text.is_empty() && !marked {
                        continue;
                    }
                    engine.guard = GUARD_FRAMES;
                    engine.edits.push((EditorInput::ImeCommit, place));
                    engine.edits.push((EditorInput::Text(text.into()), place));
                }
            }
        }
    });
}

/// The edits waiting for `owner`, in arrival order; empty for any other component. A read-only owner takes none and
/// never composes: a composition it was in is interrupted.
pub(crate) fn take(ctx: &Context, owner: Id, read_only: bool) -> Vec<EditorInput> {
    take_placed(ctx, owner, read_only).into_iter().map(|(edit, _)| edit).collect()
}

/// `take`, each edit with its place among the frame's other events: the key stage's event places (`keys::Typed`)
/// count the same events, so an editor part reads the edits and the keys in the order they arrived.
pub(crate) fn take_placed(ctx: &Context, owner: Id, read_only: bool) -> Vec<(EditorInput, usize)> {
    current(ctx, |engine| {
        if engine.owner != Some(owner) {
            return Vec::new();
        }
        engine.ran = true;
        if read_only {
            engine.edits.clear();
            if engine.composing {
                engine.interrupt(owner);
            }
            engine.preedit.clear();
            return Vec::new();
        }
        std::mem::take(&mut engine.edits)
    })
}

/// What an editor reports of the input method's composition (`EditorState::composing`, `EditorState::shows_marked`).
#[derive(Clone, Copy)]
pub(crate) struct Composing {
    /// The editor shows marked text now.
    pub(crate) now: bool,
    /// The editor draws the marked text of the pre-edits it is given.
    pub(crate) shows_marked: bool,
}

/// What an editor that draws its own caret reports once per pass; only the report of the editor that owns the IME
/// counts. An editor that reported composing and now does not ended the composition itself (a press in its text, a
/// selection, an app edit over the marked text): the OS composition is interrupted and its late events are dropped.
/// An editor that says it draws marked text (`Composing::shows_marked`) and reports none while the engine composes
/// ended it too, in the pass its pre-edit arrived in as well (a pre-edit and a refused deletion in one frame). An editor
/// that shows no marked text reports `false` throughout and the OS composes for it until the commit.
///
/// `surrounding` is the committed text around the caret with its target's epoch; the engine copies its bounded window
/// here, once per pass, for the end hook's observation.
pub(crate) fn note_editor(ctx: &Context, owner: Id, composing: Composing, history: (bool, bool), surrounding: Option<(u64, &EditorSurrounding<'_>)>) {
    current(ctx, |engine| {
        if engine.owner != Some(owner) {
            return;
        }
        let was_composing = engine.editor.as_ref().is_some_and(|last| last.owner == owner && last.composing);
        let ended = !composing.now && (was_composing || composing.shows_marked);
        let composing = composing.now;
        if ended && engine.composing {
            engine.interrupt(owner);
        }
        let surrounding = surrounding.map(|(epoch, report)| Surrounding::of(epoch, report));
        engine.editor = Some(EditorFacts { owner, composing, can_undo: history.0, can_redo: history.1, surrounding });
    });
}

/// The editor `owner` asks for the OS composition to be interrupted this pass: it refused an input method's edit, so
/// the input method's view of the text is stale. The engine folds a frame's events before the editor runs, so a refused
/// deletion followed by commits in one batch ends with nothing composing, and only the editor knows it refused one.
///
/// The end hook writes the interrupt beside this pass's change cause, so a commit the editor applied after the refusal
/// keeps its receipt (`InputMethod`). A composition still running is ended and fenced as when the editor stops showing
/// it (`note_editor`); with none running there are no late events to drop, so the editor's later input is not fenced.
/// Nothing happens for an editor that does not own the IME.
pub(crate) fn request_interrupt(ctx: &Context, owner: Id) {
    current(ctx, |engine| {
        if engine.owner != Some(owner) {
            return;
        }
        engine.interrupt_owner(owner);
    });
}

/// Whether the editor `owner` had something to undo and to redo at its last report as the IME owner.
pub(crate) fn editor_history(ctx: &Context, owner: Id) -> (bool, bool) {
    current(ctx, |engine| {
        engine.editor.as_ref().filter(|facts| facts.owner == owner).map_or((false, false), |facts| (facts.can_undo, facts.can_redo))
    })
}

/// Notes that the editor `owner` applied an input-method commit or deletion, which took its committed text from
/// `before` to `after`. True when the tracker accepted the receipt: `owner` is the context it last observed and the
/// stamps prove the change. Only the pass that applied the input notes it; a second pass of the frame gets no input
/// and notes none.
pub(crate) fn note_receipt(ctx: &Context, owner: Id, before: ImeMutationStamp, after: ImeMutationStamp) -> bool {
    let viewport = pass::current_viewport(ctx);
    let frame_nr = ctx.cumulative_frame_nr_for(viewport);
    with(ctx, viewport, |engine| {
        let Some(target) = engine.target_of(owner) else { return false };
        engine.tracker.note_input_method_apply(viewport, frame_nr, target, before, after)
    })
}

/// Notes that the editor `owner` changed its committed text for a reason other than the input method (typing, a
/// paste, undo, an app edit): the frame's change cause is `Other`.
pub(crate) fn note_application_change(ctx: &Context, owner: Id) {
    let viewport = pass::current_viewport(ctx);
    let frame_nr = ctx.cumulative_frame_nr_for(viewport);
    with(ctx, viewport, |engine| {
        if let Some(target) = engine.target_of(owner) {
            engine.tracker.note_application_change(viewport, frame_nr, target);
        }
    });
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

/// Records the caret rect for the OS candidate window. A replay of the run that recorded it records it again, so the
/// pass writes the IME output and the OS keeps the IME open for a focused field that replays.
pub(crate) fn note_caret(ctx: &Context, owner: Id, field: Rect, caret: Rect, password: bool) {
    crate::runtime::replay::journal(
        ctx,
        || (owner, field, caret, password),
        |ctx, &(owner, field, caret, password)| note_caret(ctx, owner, field, caret, password),
    );
    current(ctx, |engine| engine.caret = Some((owner, field, caret, password)));
}

/// The end hook: observes the owner on the tracker, then writes the pass's IME output while a framework editor owns the
/// IME or an interrupt is due. Otherwise the output is egui's: an app's own `TextEdit` or IME reader writes it, and the
/// engine records that editor (`Engine::host`). An interrupt of the app's editor rides on the output it wrote this
/// pass, so its context does not change.
pub(crate) fn end_pass(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let (frame_nr, pass_nr) = (ctx.cumulative_frame_nr_for(viewport), ctx.cumulative_pass_nr_for(viewport));
    let app_wrote = ctx.output(|output| output.ime.as_ref().map(|ime| (ime.owner, ime.context_generation)));
    // egui publishes the app's request only in a pass that has an IME output.
    let app_reset = app_wrote.and(os::app_reset(ctx, viewport));
    let (owner, interrupt, caret, update, published, host) = with(ctx, viewport, |engine| {
        let ran = std::mem::take(&mut engine.ran);
        if !ran {
            engine.edits.clear();
        }
        // An owner that neither took its edits nor noted its caret was not shown this pass (a replay notes the caret
        // again). egui keeps its focus until after this hook, so it loses the IME here: the interrupt goes out in the
        // pass it vanished, not at the next begin.
        if let Some(owner) = engine.owner
            && !ran
            && engine.caret.is_none_or(|(id, ..)| id != owner)
        {
            engine.set_owner(None);
        }
        // The app's editor as the last pass recorded it: an interrupt in a pass it wrote nothing names its context.
        let last_host = engine.host;
        let kept = engine.host.filter(|host| app_wrote.is_some_and(|(owner, _)| owner == host.owner)).is_some_and(|host| host.composing);
        engine.host = app_wrote.filter(|_| engine.owner.is_none()).map(|(owner, generation)| HostIme { owner, generation, composing: kept });
        // The app's request: every output this pass publishes carries it, the app's own or the engine's, and a new one
        // becomes the fence's request after the write. While a framework editor owns the IME, egui publishes it on that
        // editor's output as an interrupt, so the engine takes it as its own interrupt of the owner.
        let fresh = app_reset.filter(|id| !engine.host_resets.contains(id));
        if fresh.is_some()
            && let Some(owner) = engine.owner
        {
            engine.interrupt_owner(owner);
        }
        // The fence has no request the integration can answer: the owner's output asks again, a framework editor's
        // or the app's own.
        if engine.stale
            && engine.reset.is_none()
            && fresh.is_none()
            && let Some(owner) = engine.owner.or(engine.host.map(|host| host.owner))
        {
            engine.interrupt.get_or_insert(owner);
        }
        let caret = engine.caret.take();
        let password = caret.filter(|(id, ..)| Some(*id) == engine.owner).map(|(.., password)| password);
        let update = engine.observe(viewport, frame_nr, pass_nr, password);
        (engine.owner, engine.interrupt.take(), caret, update, engine.published, last_host)
    });
    let host_interrupt = interrupt.filter(|old| owner.is_none() && app_wrote.is_some_and(|(id, _)| id == *old));
    let written = if owner.is_none() && interrupt.is_none() {
        None
    } else if host_interrupt.is_some() {
        Some((os::interrupt_written(ctx, viewport, app_reset), None))
    } else {
        os::clear(ctx);
        match (caret, interrupt, update) {
            (Some((id, field, caret, password)), interrupt, Some(update)) if Some(id) == owner => {
                let generation = update.context_generation;
                let out = os::Output {
                    owner: id,
                    field,
                    caret,
                    interrupt: interrupt.is_some(),
                    app_reset,
                    generation,
                    password,
                    surrounding: update.surrounding_text,
                    cause: update.change_cause,
                };
                Some((os::write(ctx, viewport, out), Some(generation)))
            }
            // The old owner still gets an output in the pass it lost focus, or the OS never interrupts. It names the
            // context the OS composes in: the last one published, or the one the app's editor wrote last.
            (_, Some(old), _) => {
                let generation = host.filter(|host| host.owner == old).map_or(published.unwrap_or_default(), |host| host.generation);
                let out = os::Output {
                    owner: old,
                    field: Rect::NOTHING,
                    caret: Rect::NOTHING,
                    interrupt: true,
                    app_reset,
                    generation,
                    password: false,
                    surrounding: IMESurroundingText::Unavailable,
                    cause: ImeChangeCause::Other,
                };
                Some((os::write(ctx, viewport, out), None))
            }
            _ => None,
        }
    };
    // The app's request counts as its own only when the published output carries it: egui's own output when the
    // engine wrote none, else the engine's.
    let carried = app_reset.filter(|id| written.as_ref().is_none_or(|(reset, _)| *reset == Some(*id)));
    with(ctx, viewport, |engine| {
        if let Some((reset, generation)) = written {
            if generation.is_some() {
                engine.published = generation;
            }
            if interrupt.is_some() {
                engine.reset = reset;
            }
        }
        if let Some(id) = carried
            && !engine.host_resets.contains(&id)
        {
            engine.host_resets.push(id);
            // Completions arrive in request order: the latest request's completion ends the interrupted composition,
            // whichever request the fence waited for.
            if engine.stale {
                engine.reset = Some(id);
            }
        }
    });
}
