//! A canvas tool that counts the presses that reach it, for trials of controls that must leave a press to the tool.

use craft_behavior::CancelReason;
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;

/// A tool that counts the presses that reach it.
#[derive(Default)]
pub struct Presses {
    /// The presses so far.
    pub count: usize,
}

impl CanvasTool for Presses {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.count += 1;
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Select Area... command")
    }
}
