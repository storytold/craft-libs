//! The editor part of the pipeline (18.4): `PartsCx::editor` registers the part, step 8b gives it key focus and
//! gathers the pass's ordered `EditorInput`, and `Cx::editor_state` reports the caret, the visible lines and the
//! undo state back to the IME engine and AccessKit.

use super::Component;
use super::cx::{Cx, Edit as _, PartsCx};
use crate::primitives::{EditorPurpose, EditorState, ImeSurface};
use crate::runtime::access::text::{self as text_access, Line, Run, TextFacts};
use crate::runtime::egui::{editor as egui_editor, focus as egui_focus};
use crate::runtime::focus::KeyUse;
use crate::runtime::{commands, ime};
use craft_behavior::{EditCommand, EditorInput, PartKey};
use egui::{Rect, Ui};

/// What `PartsCx::editor` leaves for step 8b.
pub(crate) struct EditorDecl {
    pub key: PartKey,
    pub rect: Rect,
    pub purpose: EditorPurpose,
}

/// The editor part as step 8b set it up, for `editor_state`.
pub(crate) struct EditorInfo {
    pub widget: egui::Id,
    pub rect: Rect,
    pub purpose: EditorPurpose,
    pub focused: bool,
    pub read_only: bool,
    /// An input that changes text was delivered this pass.
    pub edited: bool,
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
    let focused = egui_editor::show(ui, widget, decl.rect, egui_focus::event_filter(keys.keep, false));
    let read_only = cx.core.resolved.read_only || !cx.core.resolved.enabled;
    let mut inputs = Vec::new();
    if focused {
        inputs.extend(ime::take(&ctx, widget, read_only));
        while let Some(request) = cx.core.intents.take_access_where(text_access::is_text_request) {
            if let Some((anchor, focus)) = text_access::requested_selection(&ctx, id, &request) {
                inputs.push(EditorInput::select(anchor, focus));
            }
        }
        while let Some(input) = cx.core.intents.take_editor_input(keys.keep) {
            inputs.push(input);
        }
    }
    if read_only {
        inputs.retain(EditorInput::is_read_only_safe);
    } else {
        let (can_undo, can_redo) = ime::editor_history(&ctx, widget);
        inputs.retain(|input| {
            let EditorInput::Edit(command @ (EditCommand::Undo | EditCommand::Redo)) = input else { return true };
            let can = if *command == EditCommand::Undo { can_undo } else { can_redo };
            if !can {
                commands::fall_through(&ctx, *command);
            }
            can
        });
    }
    let edited = inputs.iter().any(|input| !input.is_read_only_safe());
    cx.core.editor = Some(EditorInfo { widget, rect: decl.rect, purpose: decl.purpose, focused, read_only, edited });
    cx.core.editor_inputs = inputs;
}

impl<C: Component> Cx<'_, C> {
    /// The editor part's inputs of this pass in the order they arrived; call it in `update`. The editor applies them
    /// to its own document.
    pub fn editor_input(&mut self) -> impl Iterator<Item = EditorInput> + '_ {
        std::mem::take(&mut self.core.editor_inputs).into_iter()
    }

    /// Reports the editor's caret, visible lines, selection and undo state once per pass, from `update` or `paint`.
    pub fn editor_state(&self, state: EditorState<'_>) {
        let Some(info) = &self.core.editor else {
            debug_assert!(false, "`editor_state` needs an editor part registered by `parts`");
            return;
        };
        let ctx = &self.core.ctx;
        let masked = info.purpose == EditorPurpose::Password;
        if info.focused {
            ime::note_caret(ctx, info.widget, info.rect, state.caret, masked);
        }
        ime::note_editor(ctx, info.widget, state.revision, info.edited, (state.can_undo, state.can_redo));
        if !text_access::exposed(ctx, info.widget) {
            return;
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
        *self.core.text_access.borrow_mut() = Some(TextFacts { widget: info.widget, text, masked, editable: !info.read_only, lines, selection });
    }
}
