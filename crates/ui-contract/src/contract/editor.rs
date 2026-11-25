//! The editor part of the pipeline (18.4): `PartsCx::editor` registers the part, step 8b gives it key focus and
//! gathers the pass's ordered `EditorInput`, and `Cx::editor_state` reports the caret, the visible lines and the
//! undo state back to the IME engine and AccessKit.

use super::Component;
use super::cx::{Cx, Edit as _, PartsCx};
use crate::primitives::{EditorPurpose, EditorState, ImeStamp, ImeSurface};
use crate::runtime::access::text::{self as text_access, Line, Run, TextFacts};

use crate::runtime::commands::{self, CommandSet, Commands as _, Fired};
use crate::runtime::egui::{editor as egui_editor, focus as egui_focus};
use crate::runtime::focus::KeyUse;
use crate::runtime::ime;
use craft_behavior::{EditCommand, EditorInput, PartKey};
use egui::{Rect, Ui};

/// What `PartsCx::editor` leaves for step 8b.
pub(crate) struct EditorDecl {
    pub key: PartKey,
    pub rect: Rect,
    pub purpose: EditorPurpose,
}

/// Where an editor input or a command goes among the pass's events: the event's place, then its kind (IME edits, an
/// AccessKit selection, then keys, texts and the commands keys fired). An input or command of no one event goes last.
pub(crate) type Order = (usize, u8);

/// One step of an editor's pass, in the order of the events it came from ([`Cx::editor_steps`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum EditorStep<S> {
    /// An input of the editor part.
    Input(EditorInput),
    /// A command of the component's scope that a key or another source fired.
    Command(S, Fired),
}

/// The editor part as step 8b set it up, for `editor_state`.
pub(crate) struct EditorInfo {
    pub widget: egui::Id,
    pub rect: Rect,
    pub purpose: EditorPurpose,
    pub focused: bool,
    pub read_only: bool,
    pub commit_now: bool,
}

impl<C: Component> PartsCx<'_, C> {
    /// Registers the component's one editor part (`INPUT = Input::Editor`).
    pub fn editor(&mut self, key: PartKey, rect: Rect, purpose: EditorPurpose) {
        debug_assert!(self.text.editor.is_none() && self.text.decl.is_none(), "a component registers one text or editor part");
        self.text.editor = Some(EditorDecl { key, rect, purpose });
    }
}

/// The keys an editor part keeps: a code editor keeps Tab.
pub(crate) fn keys_of(base: KeyUse, purpose: EditorPurpose) -> KeyUse {
    match purpose {
        EditorPurpose::Code => base.with_keep(crate::runtime::focus::KeepKeys::Tab),
        _ => base,
    }
}

/// Step 8b for an editor part: its focus widget, then the pass's inputs in order (IME edits, AccessKit selection,
/// then keys, text and edit commands). A read-only editor receives only inputs that change no text; undo and redo
/// the editor cannot do go to the app command.
pub(crate) fn run<C: Component>(cx: &mut Cx<'_, C>, ui: &mut Ui, step: &mut super::text::TextStep, keys: KeyUse) {
    let Some(decl) = step.editor.take() else { return };
    // A press that leaves the editor ends its gesture, as a focus loss ends a text field's draft.
    if step.commit_now {
        cx.commit();
    }
    let (ctx, id) = (cx.core.ctx.clone(), cx.core.id);
    let widget = ImeSurface::of(id, decl.key).widget();
    let focusable = super::steps::takes_focus(cx.core.resolved);
    let focused = egui_editor::show(ui, widget, decl.rect, egui_focus::event_filter(keys.keep, false), focusable);
    // The focus widget is the component's, enabled or not: it is never one of the app's own widgets (`take_app_widgets`).
    crate::runtime::pointer::record_text_widget(&ctx, widget, id);
    let read_only = cx.core.resolved.read_only || !cx.core.resolved.enabled;
    let mut inputs: Vec<(Order, EditorInput)> = Vec::new();
    if focused {
        // Each input goes where its event arrived among the frame's events: an IME edit before the press or text that
        // came after it, and keys and texts in their own order. An AccessKit selection comes after the IME edits that
        // came before any key, and an intent of no one event (an Escape, a queued edit command) last, as routed.
        let mut placed: Vec<(Order, EditorInput)> = Vec::new();
        placed.extend(ime::take_placed(&ctx, widget, read_only).into_iter().map(|(input, place)| ((place, 0), input)));
        while let Some(request) = cx.core.intents.take_access_where(text_access::is_text_request) {
            if let Some((anchor, focus)) = text_access::requested_selection(&ctx, id, &request) {
                placed.push(((0, 1), EditorInput::select(anchor, focus)));
            }
        }
        while let Some((input, place)) = cx.core.intents.take_editor_input(keys.keep) {
            placed.push(((place.unwrap_or(usize::MAX), 2), input));
        }
        // A stable sort: inputs of one place keep the order they were taken in.
        placed.sort_by_key(|(order, _)| *order);
        inputs = placed;
    }
    if read_only {
        inputs.retain(|(_, input)| input.is_read_only_safe());
    } else {
        let (can_undo, can_redo) = ime::editor_history(&ctx, widget);
        inputs.retain(|(_, input)| {
            let EditorInput::Edit(command @ (EditCommand::Undo | EditCommand::Redo)) = input else { return true };
            let can = if *command == EditCommand::Undo { can_undo } else { can_redo };
            if !can {
                commands::fall_through(&ctx, *command);
            }
            can
        });
    }
    cx.core.editor = Some(EditorInfo { widget, rect: decl.rect, purpose: decl.purpose, focused, read_only, commit_now: step.commit_now });
    cx.core.editor_inputs = inputs;
}

/// The editor part's widget, rect, purpose, focus and read-only state of this pass, which a body that runs inside a
/// component reports through for the component's editor part.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorHandle {
    widget: egui::Id,
    rect: Rect,
    purpose: EditorPurpose,
    focused: bool,
    read_only: bool,
}

impl EditorHandle {
    /// The handle of the editor part step 8b set up.
    fn of(info: &EditorInfo) -> EditorHandle {
        EditorHandle { widget: info.widget, rect: info.rect, purpose: info.purpose, focused: info.focused, read_only: info.read_only }
    }

    /// Reports the caret and composition to the input-method engine and puts the text facts in the running pass's
    /// editor record, which the owning component's report step reads in the same pass.
    pub fn report(&self, ctx: &egui::Context, state: EditorState<'_>) {
        if let Some(facts) = self.note(ctx, state) {
            // PLAN: assumed `pub(crate) fn note_editor_facts(ctx: &egui::Context, widget: egui::Id, facts: TextFacts)` in `contract/steps/report.rs` (ribbon-w5): the pass-scoped slot step 16 reads for `core.editor`'s widget.
            super::steps::report::note_editor_facts(ctx, self.widget, facts);
        }
    }

    /// Notes the input-method receipt between the stamps `before` and `after` for this editor part; true when the
    /// stamps prove the engine applied it. A body that applies the input method's edits calls it with `ctx`.
    pub fn ime_receipt(&self, ctx: &egui::Context, before: ImeStamp, after: ImeStamp) -> bool {
        ime::note_receipt(ctx, self.widget, before, after)
    }

    /// Notes an application change of this editor part's committed text.
    pub fn ime_application_change(&self, ctx: &egui::Context) {
        ime::note_application_change(ctx, self.widget);
    }

    /// Asks for the OS composition to be interrupted this pass, because this editor part refused an input method's
    /// edit and the input method's view of the text is stale. The pass's change cause stays what its receipts prove:
    /// a commit applied after the refusal is still the input method's. A composition still running is ended and its
    /// late events are dropped until the reset completes; with none running, later input is not held back.
    pub fn ime_interrupt(&self, ctx: &egui::Context) {
        ime::request_interrupt(ctx, self.widget);
    }

    /// The one editor report path: notes the caret, the composition, the undo state and the surrounding text with the
    /// IME engine, and returns the text facts when AccessKit reads the part.
    ///
    /// A report with no lines has no text facts: an editor whose view describes its text (a page editor, through the
    /// page view's describer) reports its caret to the input method only, and no caret is announced in an empty text.
    fn note(&self, ctx: &egui::Context, state: EditorState<'_>) -> Option<TextFacts> {
        let masked = self.purpose == EditorPurpose::Password;
        if self.focused {
            ime::note_caret(ctx, self.widget, self.rect, state.caret, masked);
        }
        let surrounding = state.surrounding.as_ref().map(|(epoch, report)| (*epoch, report));
        let composing = ime::Composing { now: state.composing, shows_marked: state.shows_marked };
        ime::note_editor(ctx, self.widget, composing, (state.can_undo, state.can_redo), surrounding);
        if state.lines.is_empty() || !text_access::exposed(ctx, self.widget) {
            return None;
        }
        let lines = state
            .lines
            .iter()
            .map(|line| {
                let runs = if line.runs.is_empty() {
                    vec![Run { bytes: 0..0, rtl: false, positions: Vec::new(), widths: Vec::new() }]
                } else {
                    line.runs
                        .iter()
                        .map(|run| Run { bytes: run.bytes.clone(), rtl: run.rtl, positions: run.positions.to_vec(), widths: run.widths.to_vec() })
                        .collect()
                };
                Line { text: line.text.to_owned(), rect: line.rect, runs }
            })
            .collect();
        let text = state.lines.iter().map(|line| line.text).collect::<Vec<_>>().join("\n");
        let selection = state.selection.or(Some((state.caret_at, state.caret_at)));
        Some(TextFacts { widget: self.widget, text, masked, editable: !self.read_only, disabled: false, lines, selection })
    }
}

impl<C: Component> Cx<'_, C> {
    /// The editor part of this pass, `None` when the component registered none.
    pub fn editor_handle(&self) -> Option<EditorHandle> {
        self.core.editor.as_ref().map(EditorHandle::of)
    }

    /// Whether a press moved focus away from the editor and asks it to commit its session in this pass.
    pub fn editor_commit_requested(&self) -> bool {
        self.core.editor.as_ref().is_some_and(|editor| editor.commit_now)
    }

    /// Notes the input-method receipt between the stamps `before` and `after` on the egui fork's tracker for this
    /// viewport and frame; true when the stamps prove the engine applied it.
    pub fn ime_receipt(&self, before: ImeStamp, after: ImeStamp) -> bool {
        self.editor_handle().is_some_and(|handle| handle.ime_receipt(&self.core.ctx, before, after))
    }

    /// Notes an input-method application change on the egui fork's tracker for this viewport and frame.
    pub fn ime_application_change(&self) {
        if let Some(handle) = self.editor_handle() {
            handle.ime_application_change(&self.core.ctx);
        }
    }

    /// Asks for the OS composition to be interrupted this pass, as [`EditorHandle::ime_interrupt`].
    pub fn ime_interrupt(&self) {
        if let Some(handle) = self.editor_handle() {
            handle.ime_interrupt(&self.core.ctx);
        }
    }

    /// The editor part's inputs of this pass in the order they arrived; call it in `update`. The editor applies them
    /// to its own document.
    pub fn editor_input(&mut self) -> impl Iterator<Item = EditorInput> + '_ {
        std::mem::take(&mut self.core.editor_inputs).into_iter().map(|(_, input)| input)
    }

    /// The editor part's inputs of this pass and the commands of set `S` routed to this component, as one stream in the
    /// order their events arrived: a command a key fired comes between the inputs typed before and after its chord, so
    /// an editor that Cmd+Enter ends applies none of the frame's typing after it. A command of no one key event (a menu
    /// pick, the palette) comes last. Call it in `update` in place of `editor_input` and `take_command`.
    pub fn editor_steps<S: CommandSet>(&mut self) -> Vec<EditorStep<S>> {
        let mut steps: Vec<(Order, EditorStep<S>)> =
            std::mem::take(&mut self.core.editor_inputs).into_iter().map(|(order, input)| (order, EditorStep::Input(input))).collect();
        while let Some((command, fired)) = self.take_command::<S>() {
            steps.push((fired.place.map_or((usize::MAX, 3), |place| (place, 2)), EditorStep::Command(command, fired)));
        }
        // A stable sort: the steps of one place keep the order they were taken in.
        steps.sort_by_key(|(order, _)| *order);
        steps.into_iter().map(|(_, step)| step).collect()
    }

    /// Reports the editor's caret, visible lines, selection and undo state once per pass, from `update` or `paint`.
    pub fn editor_state(&self, state: EditorState<'_>) {
        let Some(handle) = self.editor_handle() else {
            debug_assert!(false, "`editor_state` needs an editor part registered by `parts`");
            return;
        };
        // The component's own report reaches its report step directly; a body's goes through the record.
        if let Some(facts) = handle.note(&self.core.ctx, state) {
            *self.core.text_access.borrow_mut() = Some(facts);
        }
    }
}
