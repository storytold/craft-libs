//! A command definition: its kind, modal and typing rules.

use super::*;

/// What a binding does when pressed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum CommandKind {
    #[default]
    /// Runs the command.
    Command,
    /// Selects a tool. Tools of one group share keys and cycle; tools without a group name form one
    /// group together.
    Tool {
        /// The group the tool belongs to; `None` joins the group of tools without one.
        group: Option<String>,
    },
    /// Switches something on or off.
    Toggle,
    /// One choice of a group where one holds; the group's other choices clear when it is picked.
    Radio {
        /// The group the choice belongs to; `None` joins the group of choices without one.
        group: Option<String>,
    },
    /// Acts while its key is held and ends on the release.
    Momentary,
}

/// Whether a command fires while a dialog is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ModalRule {
    /// Does not fire while a dialog is open.
    #[default]
    Blocked,
    /// Fires from within a dialog and while none is open.
    InDialogs,
    /// Fires whatever is open.
    Always,
    /// Does not fire while a dialog is open, except to an instance inside the top modal layer that declares the
    /// command (it publishes the command's state, or its focused component takes the command as an edit).
    Declared,
}

/// A command as the registry describes it: the catalog entry of the keymap and of every command surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandDef {
    /// The command's id and parameters.
    pub command: CommandRef,
    /// The name the UI shows.
    pub label: String,
    /// Menu paths, the main one first; used by search and import.
    #[serde(default)]
    pub paths: Vec<Vec<String>>,
    /// Paths only import matching reads: where foreign sets file the command and the app's menus
    /// do not, such as `["Tools", label]` for a tool. They make no areas or groups in the dialog.
    #[serde(default)]
    pub import_paths: Vec<Vec<String>>,
    /// Other names: labels in other UI languages, older names.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Home scope.
    pub scope: Scope,
    #[serde(default)]
    /// What the command is: a plain command, a tool, a toggle, a radio choice or a momentary key.
    pub kind: CommandKind,
    /// Alt added to one of the command's chords runs it as its last-settings variant
    /// ([`Hit::alt_variant`](crate::Hit::alt_variant)), unless a binding owns the Alt chord.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_alt_variant: bool,
    #[serde(default)]
    /// The bindings the command has before the user changes any.
    pub defaults: Vec<Binding>,
    /// The icon the surfaces show beside the label, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<std::borrow::Cow<'static, str>>,
    /// Whether the command fires while a dialog is open.
    #[serde(default)]
    pub modal: ModalRule,
}

impl CommandDef {
    /// A command in `scope` without default bindings.
    pub fn new(command: impl Into<CommandRef>, label: impl Into<String>, scope: Scope) -> CommandDef {
        CommandDef {
            command: command.into(),
            label: label.into(),
            paths: Vec::new(),
            import_paths: Vec::new(),
            aliases: Vec::new(),
            scope,
            kind: CommandKind::Command,
            has_alt_variant: false,
            defaults: Vec::new(),
            icon: None,
            modal: ModalRule::default(),
        }
    }

    /// Sets the [`icon`](CommandDef::icon).
    pub fn with_icon(mut self, icon: impl Into<std::borrow::Cow<'static, str>>) -> CommandDef {
        self.icon = Some(icon.into());
        self
    }

    /// Sets the [`modal`](CommandDef::modal) rule.
    pub fn with_modal(mut self, modal: ModalRule) -> CommandDef {
        self.modal = modal;
        self
    }

    /// Marks the command as showing and hiding something (a panel, guides, rulers): kind
    /// [`CommandKind::Toggle`]. Import matching reads a foreign name without a leading `Show`,
    /// `Hide` or `Show/Hide` only for such commands.
    pub fn toggle(mut self) -> CommandDef {
        if self.kind == CommandKind::Command {
            self.kind = CommandKind::Toggle;
        }
        self
    }

    /// Opts the command in to the Alt variant ([`CommandDef::has_alt_variant`]): Photoshop opens an
    /// adjustment at its last settings on its key with Alt added.
    pub fn with_alt_variant(mut self) -> CommandDef {
        self.has_alt_variant = true;
        self
    }

    /// Sets the [`kind`](CommandDef::kind).
    pub fn with_kind(mut self, kind: CommandKind) -> CommandDef {
        self.kind = kind;
        self
    }

    /// Adds a default binding.
    pub fn with_default(mut self, binding: Binding) -> CommandDef {
        self.defaults.push(binding);
        self
    }

    /// Adds a menu path; the first one added is the main path.
    pub fn with_path(mut self, path: Vec<String>) -> CommandDef {
        self.paths.push(path);
        self
    }

    /// Adds a path only import matching reads ([`CommandDef::import_paths`]).
    pub fn with_import_path(mut self, path: Vec<String>) -> CommandDef {
        self.import_paths.push(path);
        self
    }

    /// Adds an alias.
    pub fn with_alias(mut self, alias: impl Into<String>) -> CommandDef {
        self.aliases.push(alias.into());
        self
    }
}

/// Which chords work while a text field has the keyboard. Under every rule, the
/// [`text_editing_chords`] go to the text field too. Bindings in an active context always work: a
/// `Text: Cmd+B` binding is meant for typing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum TypingRule {
    /// Plain chords (no cmd, ctrl or alt) go to the text; function keys still fire.
    #[default]
    PlainKeysWait,
    /// Only chords with the command modifier and function keys fire.
    CommandKeysOnly,
    /// Nothing fires.
    AllWait,
}

impl TypingRule {
    /// Whether `chord` may fire while typing on `platform`.
    pub fn fires(self, chord: &Chord, platform: Platform) -> bool {
        if text_editing_chords(platform).contains(&chord.effective(platform)) {
            return false;
        }
        match self {
            TypingRule::PlainKeysWait => !chord.is_plain() || chord.is_function_key(),
            TypingRule::CommandKeysOnly => chord.has_command() || chord.is_function_key(),
            TypingRule::AllWait => false,
        }
    }
}
