//! `SymmetryTransform`: Transform Symmetry, a cancellable session over a copy of the symmetry, and `SymmetryTool`, the
//! session's keys while the canvas has the keyboard.

use super::menu::SymmetryWords;
use super::model::Symmetry;
use craft_behavior::{CancelReason, EditEvent, Mixed};
use craft_ui::canvas::{CanvasTool, NUDGE, NUDGE_FAR, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;
use std::sync::Arc;

/// What Transform Symmetry reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SymmetryEvent {
    /// The preview changed: a key step (`Set`). The app applies it with [`SymmetryTransform::apply`] and draws the
    /// preview; nothing is stored.
    Preview(EditEvent<Symmetry>),
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

/// The tool of an open [`SymmetryTransform`]: the canvas's keys while it has the keyboard. The arrows move the centre
/// 1 document unit, Shift 10; Enter commits; Escape discards. A press on the canvas away from the handles is not the
/// tool's. A session that is no longer valid (the document, the tool, the preset or the window changed since
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

    /// What the keys asked for since the last call, in order.
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

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        if !self.valid {
            return false;
        }
        match key {
            ToolKey::Nudge { direction, mods, .. } => {
                let Some(space) = cx.space() else { return false };
                let doc = space.view.unmap_vec(*direction);
                let length = doc.hypot();
                if length <= 0.0 {
                    return false;
                }
                let step = if mods.shift { NUDGE_FAR } else { NUDGE };
                let before = self.session.preview;
                let moved = Symmetry::new(before.mode, before.center + doc * (step / length), before.rotation).unwrap_or(before);
                if moved != before {
                    let name: Arc<str> = Arc::from(cx.words::<SymmetryWords>().group.as_str());
                    let event = EditEvent::set(Mixed::Same(before), moved, name, None);
                    self.events.push(SymmetryEvent::Preview(event));
                }
                true
            }
            ToolKey::Submit => {
                self.events.push(self.session.commit());
                true
            }
            ToolKey::Cancel => {
                self.events.push(self.session.discard());
                true
            }
            _ => false,
        }
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the arrow keys on the symmetry's centre and handle")
    }
}
