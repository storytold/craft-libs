//! The pure rules of command routing (spec 11.3): which state a command resolves to, and which live instance
//! of a scope receives a command fired by a key, the menu bar, the palette or automation.

/// A command's enabled and checked state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommandFlags {
    /// Whether the command can fire.
    pub enabled: bool,
    /// The checked state of a toggle or radio command; `None` for a command with none.
    pub checked: Option<bool>,
}

impl CommandFlags {
    /// Flags with the given enabled and checked state.
    pub const fn new(enabled: bool, checked: Option<bool>) -> Self {
        CommandFlags { enabled, checked }
    }

    /// A command nobody published a state for: enabled, with no checked state.
    pub const DEFAULT: CommandFlags = CommandFlags { enabled: true, checked: None };
}

impl Default for CommandFlags {
    fn default() -> Self {
        CommandFlags::DEFAULT
    }
}

/// This frame's state, else last frame's, else the default.
pub const fn resolve_flags(this_frame: Option<CommandFlags>, last_frame: Option<CommandFlags>) -> CommandFlags {
    match (this_frame, last_frame) {
        (Some(flags), _) | (None, Some(flags)) => flags,
        (None, None) => CommandFlags::DEFAULT,
    }
}

/// The instance a non-component source delivers to, out of the live instances of the command's scope in
/// registration order: the one that contains key focus (`focused`, innermost last), else the one that contained
/// it most recently (`last_focused`), else the first live one. `None` when no instance is live.
pub fn pick_instance<I: Copy + PartialEq>(live: &[I], focused: &[I], last_focused: &[I]) -> Option<I> {
    [focused, last_focused].iter().find_map(|chain| chain.iter().rev().find(|id| live.contains(id)).copied()).or_else(|| live.first().copied())
}
