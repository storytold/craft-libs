//! `SymmetryTransform`: Transform Symmetry, a cancellable session over a copy of the symmetry, and `SymmetryTool`, the
//! canvas tool while the session is open.

use super::model::Symmetry;
use craft_behavior::{CancelReason, EditEvent};
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;

/// What Transform Symmetry reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SymmetryEvent {
    /// Enter or the commit button: the app stores the symmetry as one undo step and drops the session.
    Apply(Symmetry),
    /// Escape or the cancel button, or the session became invalid: the app drops the session.
    Discard,
}

/// Transform Symmetry, kept by the app while it is open (the menu's Transform Symmetry): a copy of the symmetry the
/// [`SymmetryHandles`](super::SymmetryHandles) edit, and the symmetry it began with.
#[derive(Clone, Debug)]
pub struct SymmetryTransform {
    original: Symmetry,
    preview: Symmetry,
}

impl SymmetryTransform {
    /// Opens the session on a copy of `symmetry`.
    pub fn begin(symmetry: Symmetry) -> Self {
        Self { original: symmetry, preview: symmetry }
    }

    /// The symmetry the session began with (the app compares it to the preset's to see the session is still valid).
    pub fn original(&self) -> Symmetry {
        self.original
    }

    /// The edited copy, for [`SymmetryOverlay`](super::SymmetryOverlay) and [`SymmetryHandles`](super::SymmetryHandles).
    pub fn preview(&self) -> Symmetry {
        self.preview
    }

    /// Applies an edit of the handles or a key step to the preview: a value sets it, a cancel puts back the value the
    /// gesture began with.
    pub fn apply(&mut self, event: &EditEvent<Symmetry>) {
        event.apply(&mut self.preview);
    }

    /// Enter or the commit button: the symmetry to store. The app drops the session.
    pub fn commit(&self) -> SymmetryEvent {
        SymmetryEvent::Apply(self.preview)
    }

    /// Escape or the cancel button. The app drops the session.
    pub fn discard(&self) -> SymmetryEvent {
        SymmetryEvent::Discard
    }
}

/// The tool of an open [`SymmetryTransform`]. The session's [`SymmetryHandles`](super::SymmetryHandles) (shown with
/// [`session`](super::SymmetryHandles::session)) take its keys: Enter, Escape and the arrows. A press on the canvas away
/// from the handles does nothing and never reaches a painting tool; the cursor is the arrow. A session that is no
/// longer valid (the document, the tool, the preset or the window changed since
/// [`begin`](SymmetryTransform::begin)) reports [`SymmetryEvent::Discard`] in its first pass.
pub struct SymmetryTool<'a> {
    session: &'a SymmetryTransform,
    valid: bool,
    events: Vec<SymmetryEvent>,
}

impl<'a> SymmetryTool<'a> {
    /// The tool of `session`. `valid` is false once the session no longer matches the document it began on.
    pub fn new(session: &'a SymmetryTransform, valid: bool) -> Self {
        Self { session, valid, events: Vec::new() }
    }

    /// What the tool reported since the last call, in order.
    pub fn take_events(&mut self) -> Vec<SymmetryEvent> {
        std::mem::take(&mut self.events)
    }
}

impl CanvasTool for SymmetryTool<'_> {
    fn each_pass(&mut self, _: &mut ToolContext<'_>) {
        if !self.valid {
            self.events.push(self.session.discard());
        }
    }

    /// Spends the press: Transform Symmetry paints nothing.
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Done
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the arrow keys on the symmetry's centre and handle")
    }
}
