//! The Quit menu item, a window close held for unsaved documents, and [`CloseGuard`]: the
//! unsaved-changes decision for closing documents, closing all, quitting and Revert.

#[cfg(feature = "ui")]
use egui::{Context, ViewportCommand, ViewportId};

/// The id of a Quit menu item that asks the app first.
///
/// An app that builds its own native app menu (muda, any version) gives its Quit item this id,
/// instead of using muda's `PredefinedMenuItem::quit`, which ends the process without asking, and
/// passes its menu events to [`on_menu_event`].
pub const QUIT_MENU_ID: &str = "craft-quit.quit";

/// Handle a native menu event by the id of the item chosen. For [`QUIT_MENU_ID`], asks the root
/// window to close (`ViewportCommand::Close`), so the app's unsaved-changes check runs as it does
/// for the close button. Returns whether the item was Quit;
/// other items are left to the caller. Safe to call from a menu-event handler between frames:
/// egui keeps the command for the next frame.
#[cfg(feature = "ui")]
pub fn on_menu_event(ctx: &Context, id: &str) -> bool {
    if id != QUIT_MENU_ID {
        return false;
    }
    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
    true
}

/// Holds a window close while the app asks about unsaved documents. Call in `logic` when the
/// root window was asked to close and `unsaved` documents exist: it sends `CancelClose` and
/// returns `true`, and the app starts a [`CloseGuard`] for [`CloseScope::Quit`]. Without unsaved
/// documents it returns `false` and the close goes on.
// pc: crates/ui-egui/src/dialogs/discard_ui.rs:124 `guard_window_close`.
#[cfg(feature = "ui")]
pub fn intercept_close(ctx: &Context, unsaved: bool) -> bool {
    if !unsaved || ctx.viewport_id() != ViewportId::ROOT || !ctx.input(|i| i.viewport().close_requested()) {
        return false;
    }
    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
    ctx.send_viewport_cmd(ViewportCommand::Focus);
    true
}

/// What the user set out to do; it decides the question's wording and what Cancel stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CloseScope {
    /// Close one or more documents (a tab's close box, File › Close, Close All).
    Close,
    /// File › Exit / Quit, the window's close box, the Quit menu item.
    Quit,
    /// File › Revert on one document: "Revert … to the last saved version?".
    Revert,
}

/// The user's answer for one document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CloseAnswer {
    /// Save the document, then close it.
    Save,
    /// Close the document and discard its changes.
    DontSave,
    /// Keep the document open and end the whole close operation.
    Cancel,
}

/// What the app does next after an answer or a save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardStep<D> {
    /// Save `D` (Save As when it has no file); then report [`CloseGuard::saved`] or
    /// [`CloseGuard::save_failed`].
    Save(D),
    /// Close `D` without saving.
    Discard(D),
    /// Ask about the next document: show the prompt for [`CloseGuard::current`].
    Ask(D),
    /// Cancel stopped the whole operation; nothing more closes and a quit does not happen.
    Cancelled,
    /// Every document is answered and closed: finish the operation (close the window, quit).
    Finished,
}

/// The unsaved-changes decision, one document at a time, egui-free. The app keeps one while
/// the operation runs: it asks the current document's question (craft-ui's
/// `UnsavedChangesDialog`), feeds the answer in, saves or discards as told, and finishes or stops
/// on the step it gets back. `D` is the app's document id.
///
/// - Documents are asked in the order given. [`CloseAnswer::Cancel`] at any point ends the
///   operation ([`GuardStep::Cancelled`]); documents already closed stay closed.
/// - "Apply to All" (spec question 3): [`answer_all`](CloseGuard::answer_all) applies one answer
///   to every remaining document, Save included: each is saved in turn, and a cancelled Save As
///   or a failed save stops the operation.
/// - A failed save keeps the document open and stops the operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseGuard<D> {
    scope: CloseScope,
    pending: std::collections::VecDeque<D>,
    saving: Option<D>,
    apply_to_all: Option<CloseAnswer>,
    cancelled: bool,
}

impl<D: Clone + PartialEq> CloseGuard<D> {
    /// Starts the decision for `modified`, the documents with unsaved changes in asking order.
    /// An empty list is already [`GuardStep::Finished`].
    pub fn new(scope: CloseScope, modified: impl IntoIterator<Item = D>) -> Self {
        Self { scope, pending: modified.into_iter().collect(), saving: None, apply_to_all: None, cancelled: false }
    }

    /// What the guard was started for.
    pub fn scope(&self) -> CloseScope {
        self.scope
    }

    /// The document whose question is open, if any.
    pub fn current(&self) -> Option<&D> {
        self.pending.front()
    }

    /// How many documents still wait, the current one included; the prompt shows "Apply to All"
    /// when it is more than one.
    pub fn remaining(&self) -> usize {
        self.pending.len()
    }

    /// The user answered for the current document.
    // pc: crates/ui-egui/src/dialogs/discard_ui.rs:343 `apply` and :149 `advance`.
    pub fn answer(&mut self, answer: CloseAnswer) -> GuardStep<D> {
        if self.cancelled {
            return GuardStep::Cancelled;
        }
        // A save is still running: the question is not open.
        if let Some(doc) = &self.saving {
            return GuardStep::Save(doc.clone());
        }
        let Some(doc) = self.pending.front().cloned() else { return GuardStep::Finished };
        match answer {
            CloseAnswer::Save => {
                self.saving = Some(doc.clone());
                GuardStep::Save(doc)
            }
            CloseAnswer::DontSave => GuardStep::Discard(doc),
            CloseAnswer::Cancel => self.cancel(),
        }
    }

    /// The user answered with "Apply to All": the same answer for every remaining document.
    // new: Photoshop's Apply to All; PhotoCraft asks one by one.
    pub fn answer_all(&mut self, answer: CloseAnswer) -> GuardStep<D> {
        self.apply_to_all = Some(answer);
        self.answer(answer)
    }

    /// The document from [`GuardStep::Discard`] is closed without saving: the guard moves on.
    // new: the step after a discard; `saved` is the same for a save.
    pub fn discarded(&mut self) -> GuardStep<D> {
        if self.saving.is_none() {
            self.pending.pop_front();
        }
        self.next_step()
    }

    /// The save of the document from [`GuardStep::Save`] finished: it is closed and the guard
    /// moves on.
    // pc: crates/ui-egui/src/dialogs/discard_ui.rs:244 `saved_document`.
    pub fn saved(&mut self) -> GuardStep<D> {
        if let Some(saved) = self.saving.take()
            && self.pending.front() == Some(&saved)
        {
            self.pending.pop_front();
        }
        self.next_step()
    }

    /// The save failed or Save As was cancelled: the document stays open and the operation stops.
    // pc: crates/ui-egui/src/dialogs/discard_ui.rs:210 `save` (the `false` branch).
    pub fn save_failed(&mut self) -> GuardStep<D> {
        self.cancel()
    }

    /// A document closed elsewhere while the guard ran (a script): it is no longer asked about.
    pub fn forget(&mut self, doc: &D) {
        self.pending.retain(|d| d != doc);
    }

    /// Whether no document is left to ask about or save, or the operation was cancelled.
    pub fn is_finished(&self) -> bool {
        self.cancelled || (self.pending.is_empty() && self.saving.is_none())
    }

    /// Whether the operation was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Ends the operation; the documents not yet answered stay open.
    fn cancel(&mut self) -> GuardStep<D> {
        self.cancelled = true;
        self.pending.clear();
        self.saving = None;
        GuardStep::Cancelled
    }

    /// The step for the front document once the previous one is settled.
    fn next_step(&mut self) -> GuardStep<D> {
        if self.cancelled {
            return GuardStep::Cancelled;
        }
        let Some(doc) = self.pending.front().cloned() else { return GuardStep::Finished };
        match self.apply_to_all {
            None => GuardStep::Ask(doc),
            Some(CloseAnswer::Save) => {
                self.saving = Some(doc.clone());
                GuardStep::Save(doc)
            }
            Some(CloseAnswer::DontSave) => GuardStep::Discard(doc),
            Some(CloseAnswer::Cancel) => self.cancel(),
        }
    }
}

#[cfg(test)]
mod guard_tests {
    use super::*;

    #[test]
    fn each_answer_moves_to_the_next_document_and_cancel_stops_the_rest() {
        let mut guard = CloseGuard::new(CloseScope::Close, [1, 2, 3, 4]);
        assert_eq!(guard.answer(CloseAnswer::Save), GuardStep::Save(1));
        assert_eq!(guard.saved(), GuardStep::Ask(2));
        assert_eq!(guard.answer(CloseAnswer::DontSave), GuardStep::Discard(2));
        assert_eq!(guard.discarded(), GuardStep::Ask(3));
        assert_eq!(guard.answer(CloseAnswer::Cancel), GuardStep::Cancelled);
        assert!(guard.is_cancelled() && guard.is_finished());
        assert_eq!(guard.current(), None);

        let mut guard = CloseGuard::new(CloseScope::Quit, [7]);
        assert_eq!(guard.answer(CloseAnswer::DontSave), GuardStep::Discard(7));
        assert_eq!(guard.discarded(), GuardStep::Finished);
        assert!(guard.is_finished() && !guard.is_cancelled());
        assert_eq!(CloseGuard::<u32>::new(CloseScope::Close, []).saved(), GuardStep::Finished);
    }
}

#[cfg(all(test, feature = "ui"))]
mod tests {
    use egui::RawInput;

    use super::*;

    #[test]
    fn quit_from_a_child_viewport_still_asks_the_root_to_close() {
        let ctx = Context::default();
        let child = ViewportId::from_hash_of("document-child");
        let mut input = RawInput { viewport_id: child, ..RawInput::default() };
        input.viewports.entry(child).or_default().parent = Some(ViewportId::ROOT);
        let mut out = ctx.run_ui(input, |ui| {
            assert_eq!(ui.ctx().viewport_id(), child);
            assert!(on_menu_event(ui.ctx(), QUIT_MENU_ID));
        });
        out.textures_delta.clear(); // not painted
        let root_commands = out.viewport_output.get(&ViewportId::ROOT).map(|o| o.commands.as_slice()).unwrap_or_default();
        let child_commands = out.viewport_output.get(&child).map(|o| o.commands.as_slice()).unwrap_or_default();
        assert!(root_commands.contains(&ViewportCommand::Close), "root unsaved handling decides whether the app exits");
        assert!(!child_commands.contains(&ViewportCommand::Close), "Quit must not just close the current child");
    }
}
