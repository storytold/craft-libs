//! The data of a tool rail: tools, slots and the tool each slot showed last.

use crate::runtime::commands::CommandRef;

/// Names a slot of the rail. The host numbers its slots; [`LastUsed`] remembers a tool under it.
pub type SlotId = u32;

/// One tool of a slot. A tool is a command: the command gives the label, the icon, the chord and the enabled state,
/// and picking the tool runs it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RailTool<T> {
    /// The host's identifier for the tool, reported when the tool is selected.
    pub id: T,
    /// The command the tool runs.
    pub command: CommandRef,
}

impl<T> RailTool<T> {
    /// A tool reported as `id` that runs `command`.
    pub fn new(id: T, command: impl Into<CommandRef>) -> Self {
        RailTool { id, command: command.into() }
    }
}

/// A group of tools that share one button; its flyout lists them.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ToolSlot<T> {
    /// The slot's name, chosen by the host; keys the remembered last-used tool.
    pub id: SlotId,
    /// The tools the slot's flyout lists. The button shows the current or last-used tool, else the first enabled one.
    pub tools: Vec<RailTool<T>>,
    /// The slot ends a group of tools: the rail leaves a gap after it.
    pub group_end: bool,
}

impl<T> ToolSlot<T> {
    /// A slot that does not end a group.
    pub fn new(id: SlotId, tools: Vec<RailTool<T>>) -> Self {
        ToolSlot { id, tools, group_end: false }
    }

    /// A slot that ends a group of tools: the rail leaves a gap after it, before the next slot. In two columns the
    /// gap follows the row that holds the slot.
    pub fn ending_group(id: SlotId, tools: Vec<RailTool<T>>) -> Self {
        ToolSlot { id, tools, group_end: true }
    }
}

/// The tool each slot showed last. Plain data the host keeps with its workspace.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LastUsed<T> {
    tools: Vec<(SlotId, T)>,
}

impl<T> Default for LastUsed<T> {
    fn default() -> Self {
        LastUsed { tools: Vec::new() }
    }
}

impl<T: PartialEq> LastUsed<T> {
    /// The tool remembered for `slot`.
    pub fn get(&self, slot: SlotId) -> Option<&T> {
        self.tools.iter().find(|(id, _)| *id == slot).map(|(_, tool)| tool)
    }

    /// Remembers `tool` for `slot`.
    pub fn set(&mut self, slot: SlotId, tool: T) {
        match self.tools.iter_mut().find(|(id, _)| *id == slot) {
            Some(entry) => entry.1 = tool,
            None => self.tools.push((slot, tool)),
        }
    }
}

/// The slots with their tools that are not hidden, each with its index in `slots`. A slot keeps only its visible
/// tools, so its flyout lists just those; a slot with none left is dropped.
pub fn visible_slots<'a, T: PartialEq>(slots: &'a [ToolSlot<T>], hidden: &[T]) -> Vec<(usize, Vec<&'a RailTool<T>>)> {
    slots
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| {
            let tools: Vec<&RailTool<T>> = slot.tools.iter().filter(|tool| !hidden.contains(&tool.id)).collect();
            (!tools.is_empty()).then_some((index, tools))
        })
        .collect()
}

/// The tool a slot shows: the current tool when the slot has it, else the one remembered in `last_used` if it is
/// still among the visible `tools` and `enabled`, else the first enabled tool, else the first. `None` for an empty
/// slot. `enabled` is the command state of a tool.
pub fn shown_tool<'a, T: PartialEq>(
    current: &T,
    slot: SlotId,
    tools: &[&'a RailTool<T>],
    last_used: &LastUsed<T>,
    enabled: impl Fn(&RailTool<T>) -> bool,
) -> Option<&'a RailTool<T>> {
    let find = |id: &T| tools.iter().copied().find(|tool| tool.id == *id);
    let stored = last_used.get(slot).and_then(find).filter(|tool| enabled(tool));
    find(current).or(stored).or_else(|| tools.iter().copied().find(|tool| enabled(tool))).or_else(|| tools.first().copied())
}

/// `last_used` with the slot that holds `current` remembering it; `None` when nothing changes.
pub fn remembered<T: Clone + PartialEq>(slots: &[ToolSlot<T>], current: &T, last_used: &LastUsed<T>) -> Option<LastUsed<T>> {
    let mut next = last_used.clone();
    for slot in slots.iter().filter(|slot| slot.tools.iter().any(|tool| tool.id == *current)) {
        next.set(slot.id, current.clone());
    }
    (next != *last_used).then_some(next)
}
