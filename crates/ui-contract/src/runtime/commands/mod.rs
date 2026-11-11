//! The command board: the registry and typed sets, per-frame state, the queue, firing and the gate (spec 11).

mod board;
mod clipboard;
mod library;
mod registry;

pub use clipboard::{ClipContent, ClipError, ClipItem, ClipboardHandle};
pub use craft_behavior::EditCommand;
pub use craft_keymap::{Binding, CommandDef, CommandKind, CommandRef, CommandScope, ModalRule, Platform};
pub use library::ChromeCommand;
pub use library::{EditCommands, WindowCommand};
pub use registry::CommandDefError;
#[doc(hidden)]
pub use registry::{SetEntry, sets};

#[doc(hidden)]
pub use board::declare_child;
#[doc(hidden)]
pub use board::focus_within;
pub(crate) use board::publish as publish_for;
pub(crate) use board::take as take_for_instance;
#[doc(hidden)]
pub use board::take_around;
pub(crate) use board::{begin, binds_scoped, edit_chords, end, install, note_focus, platform, register_instance, set_input, take_fired};
pub(crate) use board::{fire_bound, fire_resolved, resolve_bound, resolve_scoped};

use craft_behavior::CommandFlags;
use craft_keymap::{Chord, Overrides};
use egui::{Context, Id, ViewportId};
use std::borrow::Cow;
use std::sync::Arc;

/// A typed set of commands, written by `#[derive(CommandSet)]`.
pub trait CommandSet: Copy + Send + Sync + 'static {
    /// The scope the set's commands belong to.
    const SCOPE: CommandScope;

    /// The command of this set that `command` names.
    fn from_ref(command: &CommandRef) -> Option<Self>;

    #[doc(hidden)]
    fn id(self) -> &'static str;

    #[doc(hidden)]
    fn defs() -> Result<Vec<CommandDef>, CommandDefError>;

    /// The commands that have no default binding on a platform, as `(id, platform)`.
    #[doc(hidden)]
    fn no_default_on() -> Vec<(&'static str, Platform)>;

    /// The commands a focused component takes as an `Intent::Edit`, as `(id, edit)`: the `edit` option of the
    /// command's attribute.
    #[doc(hidden)]
    fn edits() -> Vec<(&'static str, EditCommand)>;

    /// The defaults of one platform, as `(id, platform, binding)`: on that platform they replace the command's other
    /// defaults.
    #[doc(hidden)]
    fn platform_defaults() -> Result<Vec<(&'static str, Platform, Binding)>, CommandDefError>;
}

/// A command a surface or a key fired, as the instance or the app takes it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Fired {
    /// The command.
    pub command: CommandRef,
    /// The scope instance the command was routed to; `None` for app-scope commands.
    pub target: Option<Id>,
    /// Where it came from.
    pub source: FireSource,
    /// The key phase that fired it.
    pub phase: FirePhase,
    /// Tools of one group share the chord: the app picks the next one.
    pub cycle: bool,
    /// The chord pressed was a bound chord of the command with Alt added, and the command opted in.
    pub alt_variant: bool,
}

/// Where a fired command came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FireSource {
    /// A shortcut.
    Key,
    /// A menu row.
    Menu,
    /// The command palette.
    Palette,
    /// A component the pointer or the keyboard activated.
    Pointer,
    /// An automation script.
    Automation,
    /// An AccessKit action on a part of a component.
    Access,
}

/// The phase of the key press that fired a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirePhase {
    /// The press.
    Press,
    /// A repeat of a held key.
    Repeat,
    /// The release; momentary tools act here.
    Release,
}

/// What the app or an instance says about a command this frame.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct CommandState {
    flags: CommandFlags,
    title: Option<Arc<str>>,
    hint: Option<Arc<str>>,
    marked: bool,
}

impl CommandState {
    /// An enabled, unchecked-less state with the command's own label.
    pub fn new() -> Self {
        CommandState::default()
    }

    /// Whether the command can fire.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.flags.enabled = enabled;
        self
    }

    /// The checked state of a toggle or radio command.
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.flags.checked = Some(checked);
        self
    }

    /// Whether the command's content has something to show, such as comments on a panel. A rail marks the command's
    /// button; the marker's description is the command's `hint`.
    #[must_use]
    pub fn marked(mut self, marked: bool) -> Self {
        self.marked = marked;
        self
    }

    /// A label for this frame in place of the command's own, such as "Undo Move".
    #[must_use]
    pub fn title(mut self, title: impl Into<Arc<str>>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// The accessible description and second tooltip line.
    #[must_use]
    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub(crate) fn is_marked(&self) -> bool {
        self.marked
    }

    pub(crate) fn flags(&self) -> CommandFlags {
        self.flags
    }

    pub(crate) fn title_ref(&self) -> Option<&Arc<str>> {
        self.title.as_ref()
    }

    pub(crate) fn hint_ref(&self) -> Option<&Arc<str>> {
        self.hint.as_ref()
    }
}

/// A command as a surface shows and fires it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ResolvedCommand {
    /// The state's title, else the command's label, else its id.
    pub label: Arc<str>,
    /// The icon name.
    pub icon: Option<Cow<'static, str>>,
    /// The first single-press binding.
    pub chord: Option<Chord>,
    /// The state ANDed with the gate, the modal rule and the instance's liveness.
    pub enabled: bool,
    /// The checked state of a toggle or radio command.
    pub checked: Option<bool>,
    /// The command's content has something to show; the marker's description is `hint`.
    pub marked: bool,
    /// The accessible description and second tooltip line.
    pub hint: Option<Arc<str>>,
    /// What the command is.
    pub kind: CommandKind,
    pub(crate) command: CommandRef,
    #[doc(hidden)]
    pub instance: Option<Id>,
}

/// The app's one gate over commands: a dialog is open, a workspace is not ready, a recorder has the keys or an
/// operation is pending. A command the gate refuses resolves disabled and does not fire.
#[derive(Clone)]
pub struct CommandGate(Arc<dyn Fn(&CommandRef) -> bool + Send + Sync>);

impl CommandGate {
    /// A gate that lets `allows` commands through.
    pub fn new(allows: impl Fn(&CommandRef) -> bool + Send + Sync + 'static) -> Self {
        CommandGate(Arc::new(allows))
    }

    pub(crate) fn allows(&self, command: &CommandRef) -> bool {
        (self.0)(command)
    }
}

/// App-scope command state for this frame, set from app state before the pass.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct CommandInput {
    pub(crate) states: Vec<(CommandRef, CommandState)>,
    pub(crate) overrides: Option<Arc<Overrides>>,
    pub(crate) gate: Option<CommandGate>,
}

impl CommandInput {
    /// No state, no overrides and no gate.
    pub fn new() -> Self {
        CommandInput::default()
    }

    /// The state of an app-scope command.
    #[must_use]
    pub fn state(mut self, command: impl Into<CommandRef>, state: CommandState) -> Self {
        self.states.push((command.into(), state));
        self
    }

    /// The user's binding overrides; the keymap is rebuilt when the `Arc` changes.
    #[must_use]
    pub fn overrides(mut self, overrides: Arc<Overrides>) -> Self {
        self.overrides = Some(overrides);
        self
    }

    /// The one gate over commands.
    #[must_use]
    pub fn gate(mut self, gate: CommandGate) -> Self {
        self.gate = Some(gate);
        self
    }
}

/// The chain of container instances that receive scoped commands, innermost last. A private inherited field of
/// the env.
#[doc(hidden)]
#[derive(Clone, Default)]
pub struct CommandChain(Arc<[(CommandScope, Id)]>);

impl CommandChain {
    pub(crate) fn pushed(&self, scope: CommandScope, id: Id) -> Self {
        CommandChain(self.0.iter().copied().chain([(scope, id)]).collect())
    }

    /// The innermost instance of `scope`.
    #[doc(hidden)]
    pub fn innermost(&self, scope: &str) -> Option<Id> {
        self.0.iter().rev().find(|(s, _)| s.name() == scope).map(|(_, id)| *id)
    }

    /// The innermost instance of any scope.
    pub(crate) fn innermost_any(&self) -> Option<Id> {
        self.0.last().map(|(_, id)| *id)
    }

    /// The innermost instance whose scope name `is`.
    pub(crate) fn innermost_where(&self, mut is: impl FnMut(&str) -> bool) -> Option<Id> {
        self.0.iter().rev().find(|(s, _)| is(s.name())).map(|(_, id)| *id)
    }

    pub(crate) fn ids(&self) -> impl Iterator<Item = Id> + '_ {
        self.0.iter().map(|(_, id)| *id)
    }
}

/// How the contexts reach the board. Sealed: only the pipeline's contexts implement it.
#[doc(hidden)]
pub trait CommandAccess: crate::runtime::memory::sealed::Sealed {
    /// The egui context.
    fn command_ctx(&self) -> &Context;
    /// The instances above this context.
    fn command_chain(&self) -> &CommandChain;
    /// The component this context belongs to, when it has an id of its own.
    fn command_owner(&self) -> Option<Id>;
}

/// The chapter-11 context trait that `Cx`, `BodyCx` and `ItemCx` implement. "This instance" is the innermost
/// container at or above the context whose `ContainerProps.command_scope` is the command's scope.
pub trait Commands: CommandAccess {
    /// The one clipboard path.
    fn clipboard(&self) -> ClipboardHandle {
        ClipboardHandle::new(self.command_ctx(), self.command_owner())
    }

    /// This frame's state of a scoped command, for this instance. Publishing an app-scope command here is a
    /// debug assertion.
    fn publish(&mut self, command: impl Into<CommandRef>, state: CommandState) {
        board::publish(self.command_ctx(), self.command_chain(), &command.into(), state);
    }

    /// The next command of set `S` routed to this instance this pass; each is taken once, so a discard pass does
    /// not repeat it.
    fn take_command<S: CommandSet>(&mut self) -> Option<(S, Fired)> {
        let chain = self.command_chain();
        let ctx = self.command_ctx();
        if S::SCOPE != CommandScope::APP {
            return board::take(ctx, board::innermost(ctx, chain, S::SCOPE.name()), false, S::from_ref);
        }
        // An app-scope command an instance declares (`ModalRule::Declared`) is delivered to that instance, the
        // innermost one around the context that published it. The rest go to the app instance.
        chain
            .innermost_any()
            .and_then(|declaring| board::take(ctx, Some(declaring), false, S::from_ref))
            .or_else(|| board::take(ctx, board::innermost(ctx, chain, S::SCOPE.name()), true, S::from_ref))
    }

    /// `command` as this context shows it: the label, icon and chord with the scoped bindings of the scopes above
    /// the context, and the enabled state of its instance. `None` when the command is scoped and no instance of its
    /// scope is above the context.
    fn resolve_command(&self, command: impl Into<CommandRef>) -> Option<ResolvedCommand> {
        board::resolve_scoped(self.command_ctx(), self.command_chain(), &command.into())
    }
}

/// Fires `command` from a surface that has no component of its own: a menu, the palette or automation. Scoped
/// commands go to the live instance that contains key focus, else the one that contained it most recently, else
/// the first live one. Returns whether the command was queued: a disabled command, or a scoped command with no
/// live instance, is not.
pub fn fire(ctx: &Context, command: impl Into<CommandRef>, source: FireSource) -> bool {
    if !crate::runtime::measure::effect(ctx, "commands::fire") {
        return false;
    }
    board::fire_from(ctx, &command.into(), source)
}

/// `command` as a surface with no component of its own shows it: the label, icon, chord, enabled and checked state
/// and hint, against the instance that [`fire`] would deliver it to. A scoped command with no live instance
/// resolves disabled.
pub fn resolve(ctx: &Context, command: impl Into<CommandRef>) -> ResolvedCommand {
    board::resolve_global(ctx, &command.into())
}

/// Declares that a modal layer is open in some viewport, so `ModalRule::Blocked` commands do not fire outside it;
/// `root` is the top modal layer of `viewport`, if it has one.
pub(crate) fn modal_root(ctx: &Context, viewport: ViewportId) -> Option<Id> {
    board::modal_root(ctx, viewport)
}

pub(crate) fn set_modal_open(ctx: &Context, open: bool, viewport: ViewportId, root: Option<Id>) {
    board::set_modal_open(ctx, open, viewport, root);
}

/// An undo or redo a text part or editor had nothing to do for goes to the app command of the same id (11.3).
pub(crate) fn fall_through(ctx: &Context, command: craft_behavior::EditCommand) {
    board::fall_through(ctx, command);
}
