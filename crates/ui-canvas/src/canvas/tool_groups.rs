//! Tool groups and their keys (8.3, "Keyboard path for tools"): a plain letter picks the group's last-used tool and
//! Shift with the letter, or a repeated chord, cycles to the next enabled tool.
//!
//! A group is a rail slot ([`ToolSlot`]), a tool is a command ([`RailTool`]), and the tool each slot showed last is
//! [`LastUsed`]: [`tool_pick`] reads those and keeps no store of its own.
//!
//! The app declares the commands in its `#[derive(CommandSet)]` enum, because a command set is static and the
//! groups are the app's data. Per group it declares:
//!
//! - one command per tool, `kind = "tool"` with the group name: the tool's menu row and rail button run it, and it
//!   carries no letter of its own when the group has a pick command;
//! - a pick command, `kind = "command"`, bound to the letter: its handler calls [`tool_pick`] with
//!   [`PickStep::Last`];
//! - a cycle command, `kind = "command"`, bound to Shift with the letter: its handler calls [`tool_pick`] with
//!   [`PickStep::Next`].
//!
//! Tools of one group that share a chord reach the app as one fired command with `Fired::cycle` set;
//! [`PickStep::of`] reads it.

use crate::rails::{LastUsed, RailTool, SlotId, shown_tool};
use crate::runtime::commands::Fired;

/// Which tool of a group a key picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PickStep {
    /// The tool the group shows: the current tool when the group has it, else the last-used one, else the first
    /// enabled one. The plain letter.
    Last,
    /// The enabled tool after the current one, wrapping. From outside the group it picks what `Last` picks. Shift
    /// with the letter, or the chord pressed again.
    Next,
}

impl PickStep {
    /// The step of a fired tool command: a chord shared by the tools of one group (`Fired::cycle`) cycles.
    pub const fn of(fired: &Fired) -> PickStep {
        if fired.cycle { PickStep::Next } else { PickStep::Last }
    }
}

/// The tool a key picks in the group `slot`, whose visible tools are `tools` (`visible_slots` gives them).
/// `current` is the selected tool, `enabled` the command state of a tool. `None` when the group has no enabled
/// tool. The app selects the tool by running its command and stores `remembered(..)`.
pub fn tool_pick<'a, T: PartialEq>(
    tools: &[&'a RailTool<T>],
    slot: SlotId,
    current: &T,
    last_used: &LastUsed<T>,
    step: PickStep,
    enabled: impl Fn(&RailTool<T>) -> bool,
) -> Option<&'a RailTool<T>> {
    let shown = shown_tool(current, slot, tools, last_used, &enabled).filter(|tool| enabled(tool));
    let from = tools.iter().position(|tool| tool.id == *current);
    match (step, from) {
        (PickStep::Next, Some(at)) => (1..=tools.len()).map(|offset| tools[(at + offset) % tools.len()]).find(|tool| enabled(tool)),
        _ => shown,
    }
}
