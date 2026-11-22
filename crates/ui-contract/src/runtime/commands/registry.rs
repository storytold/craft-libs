//! The registry of command definitions: the sets `#[derive(CommandSet)]` registers, loaded at install.

use craft_behavior::EditCommand;
use craft_keymap::{Binding, BindingError, CommandDef, CommandScope, Platform};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

/// A command's default binding on one platform: the command id, the platform and the binding.
type PlatformDefault = (&'static str, Platform, Binding);

/// One registered command set.
#[doc(hidden)]
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct SetEntry {
    /// Returns the set's type name.
    pub name: fn() -> &'static str,
    /// The set's definitions.
    pub defs: fn() -> Result<Vec<CommandDef>, CommandDefError>,
    /// The commands of the set without a default binding on a platform.
    pub no_default_on: fn() -> Vec<(&'static str, Platform)>,
    /// The commands of the set a focused component takes as an edit.
    pub edits: fn() -> Vec<(&'static str, EditCommand)>,
    /// The momentary commands of the set whose held key makes a scope's keys work.
    pub holds: fn() -> Vec<(&'static str, CommandScope)>,
    /// The set's scope when its keys are meant for typing (`CommandSet::TYPING`).
    pub typing: Option<CommandScope>,
    /// The defaults of one platform, which replace the command's other defaults there.
    pub platform_defaults: fn() -> Result<Vec<PlatformDefault>, CommandDefError>,
}

impl SetEntry {
    /// The entry of the set named `name`.
    pub const fn new(
        name: fn() -> &'static str,
        defs: fn() -> Result<Vec<CommandDef>, CommandDefError>,
        no_default_on: fn() -> Vec<(&'static str, Platform)>,
        edits: fn() -> Vec<(&'static str, EditCommand)>,
        holds: fn() -> Vec<(&'static str, CommandScope)>,
        typing: Option<CommandScope>,
        platform_defaults: fn() -> Result<Vec<PlatformDefault>, CommandDefError>,
    ) -> Self {
        SetEntry { name, defs, no_default_on, edits, holds, typing, platform_defaults }
    }
}

inventory::collect!(SetEntry);

/// Every command set linked into the running binary, by type name.
#[doc(hidden)]
pub fn sets() -> Vec<&'static SetEntry> {
    let mut sets: Vec<_> = inventory::iter::<SetEntry>().collect();
    sets.sort_by_key(|entry| (entry.name)());
    sets
}

/// Registers the command set `$ty`, so `install` loads it; `story` registers a story's set, which only an install
/// that shows stories loads.
#[doc(hidden)]
#[macro_export]
macro_rules! __register_command_set {
    ($ty:ty) => {
        $crate::__register_command_set!(@entry $ty, ::core::convert::identity);
    };
    (@entry $ty:ty, $mark:expr) => {
        const _: () = {
            $crate::__private::inventory::submit! {
                $mark($crate::runtime::commands::SetEntry::new(
                    ::std::any::type_name::<$ty>,
                    <$ty as $crate::runtime::commands::CommandSet>::defs,
                    <$ty as $crate::runtime::commands::CommandSet>::no_default_on,
                    <$ty as $crate::runtime::commands::CommandSet>::edits,
                    <$ty as $crate::runtime::commands::CommandSet>::holds,
                    if <$ty as $crate::runtime::commands::CommandSet>::TYPING {
                        ::core::option::Option::Some(<$ty as $crate::runtime::commands::CommandSet>::SCOPE)
                    } else {
                        ::core::option::Option::None
                    },
                    <$ty as $crate::runtime::commands::CommandSet>::platform_defaults,
                ))
            }
        };
    };
}

/// Why a command definition is unusable.
#[derive(Debug)]
#[non_exhaustive]
pub enum CommandDefError {
    /// A default binding's text does not parse.
    #[non_exhaustive]
    Binding {
        /// The command's id.
        command: &'static str,
        /// What is wrong with the text.
        error: BindingError,
    },
    /// The set's scope name is not a valid scope name.
    #[non_exhaustive]
    Scope {
        /// The command's id.
        command: &'static str,
    },
    /// Two commands have one id.
    #[non_exhaustive]
    Duplicate {
        /// The id.
        command: String,
    },
    /// An app command is defined in a scope other than the app's.
    #[non_exhaustive]
    NotApp {
        /// The command's id.
        command: String,
    },
}

impl CommandDefError {
    #[doc(hidden)]
    pub fn binding(command: &'static str, error: BindingError) -> Self {
        CommandDefError::Binding { command, error }
    }

    #[doc(hidden)]
    pub fn scope(command: &'static str) -> Self {
        CommandDefError::Scope { command }
    }
}

impl std::fmt::Display for CommandDefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandDefError::Binding { command, error } => write!(f, "the default binding of {command}: {error}"),
            CommandDefError::Scope { command } => write!(f, "the scope of {command} is not a valid scope name"),
            CommandDefError::Duplicate { command } => write!(f, "two commands have the id {command}"),
            CommandDefError::NotApp { command } => write!(f, "the command {command} is not in the app scope"),
        }
    }
}

impl std::error::Error for CommandDefError {}

/// The shared registries: the app's sets, and the app's sets with the stories' (`[apps, with stories]`).
static LOADED: [OnceLock<Arc<Registry>>; 2] = [OnceLock::new(), OnceLock::new()];
/// One selected set of story commands and its lazily loaded definitions.
#[cfg(feature = "suite")]
pub(crate) struct StorySelection {
    sets: Vec<SetEntry>,
    loaded: OnceLock<Arc<Registry>>,
}

#[cfg(feature = "suite")]
impl StorySelection {
    pub(crate) fn new(sets: Vec<SetEntry>) -> Self {
        Self { sets, loaded: OnceLock::new() }
    }

    fn shared(&self) -> Result<Arc<Registry>, CommandDefError> {
        if let Some(loaded) = self.loaded.get() {
            return Ok(loaded.clone());
        }
        let loaded = Arc::new(Registry::load_selected(true, &self.sets)?);
        Ok(self.loaded.get_or_init(|| loaded).clone())
    }
}

#[cfg(feature = "suite")]
thread_local! {
    static STORY_SETS: std::cell::RefCell<Option<Arc<StorySelection>>> = const { std::cell::RefCell::new(None) };
}

/// Selects story commands for contexts created on this thread.
#[cfg(feature = "suite")]
pub(crate) fn configure_story_sets(selection: Arc<StorySelection>) -> bool {
    if selection.shared().is_err() {
        return false;
    }
    STORY_SETS.with(|current| *current.borrow_mut() = Some(selection));
    true
}

/// Runs one trial with its selection and restores the caller's selection.
#[cfg(feature = "suite")]
pub(crate) fn with_story_sets<R>(selection: &Arc<StorySelection>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Arc<StorySelection>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            STORY_SETS.with(|current| *current.borrow_mut() = self.0.take());
        }
    }
    let previous = STORY_SETS.with(|current| current.borrow_mut().replace(selection.clone()));
    let _restore = Restore(previous);
    f()
}

/// Indexes the definition at `at` under its id `id`; an id already in `index` is refused (`Duplicate`). The registry
/// and an app's catalog index their definitions through it.
pub(crate) fn index_id(index: &mut HashMap<String, usize>, id: &str, at: usize) -> Result<(), CommandDefError> {
    if index.insert(id.to_owned(), at).is_some() {
        return Err(CommandDefError::Duplicate { command: id.to_owned() });
    }
    Ok(())
}

/// The loaded definitions, by id.
#[derive(Default)]
pub(crate) struct Registry {
    defs: Vec<CommandDef>,
    index: HashMap<String, usize>,
    /// Commands whose default bindings do not apply on a platform.
    no_default: HashSet<(String, Platform)>,
    /// The commands a focused component takes as an edit, by id.
    edits: HashMap<String, EditCommand>,
    /// Defaults that one platform has instead of the command's others.
    platform_defaults: Vec<(String, Platform, Binding)>,
    /// The scope each momentary command's held key makes work, by the command's id.
    holds: HashMap<String, &'static str>,
    /// The scopes whose keys are meant for typing (`CommandSet::TYPING`).
    typing: HashSet<&'static str>,
    /// Default keymaps are shared by contexts using this exact registry.
    pub(crate) keymaps: std::sync::Mutex<Vec<(Platform, super::board::BuiltKeymap)>>,
}

impl Registry {
    /// Loads every linked set; the stories' sets only with `stories`.
    pub(crate) fn load(stories: bool) -> Result<Registry, CommandDefError> {
        Self::load_selected(stories, &[])
    }

    fn load_selected(stories: bool, story_sets: &[SetEntry]) -> Result<Registry, CommandDefError> {
        let mut registry = Registry::default();
        let mut entries = sets();
        if stories {
            entries.extend(story_sets.iter());
            entries.sort_by_key(|entry| (entry.name)());
        }
        for entry in entries {
            for def in (entry.defs)()? {
                registry.add(def)?;
            }
            registry.no_default.extend((entry.no_default_on)().into_iter().map(|(id, platform)| (id.to_owned(), platform)));
            registry.edits.extend((entry.edits)().into_iter().map(|(id, edit)| (id.to_owned(), edit)));
            registry.holds.extend((entry.holds)().into_iter().map(|(id, scope)| (id.to_owned(), scope.name())));
            registry.typing.extend(entry.typing.map(CommandScope::name));
            registry
                .platform_defaults
                .extend((entry.platform_defaults)()?.into_iter().map(|(id, platform, binding)| (id.to_owned(), platform, binding)));
        }
        Ok(registry)
    }

    /// The registry of every linked set (the stories' sets only with `stories`), loaded once per process: the sets are
    /// static, so every context shares one. A load that fails is not kept, so each install reports its error.
    pub(crate) fn shared(stories: bool) -> Result<Arc<Registry>, CommandDefError> {
        #[cfg(feature = "suite")]
        if stories {
            let selection = STORY_SETS.with(|current| current.borrow().clone());
            if let Some(selection) = selection {
                return selection.shared();
            }
        }
        let slot = &LOADED[usize::from(stories)];
        if let Some(loaded) = slot.get() {
            return Ok(loaded.clone());
        }
        let registry = Arc::new(Registry::load(stories)?);
        Ok(slot.get_or_init(|| registry).clone())
    }

    fn add(&mut self, def: CommandDef) -> Result<(), CommandDefError> {
        index_id(&mut self.index, &def.command.id, self.defs.len())?;
        self.defs.push(def);
        Ok(())
    }

    pub(crate) fn get(&self, id: &str) -> Option<&CommandDef> {
        self.index.get(id).map(|&i| &self.defs[i])
    }

    /// The edit a focused component takes the command `id` as; `None` for a command with no `EditCommand` link.
    pub(crate) fn edit_of(&self, id: &str) -> Option<EditCommand> {
        self.edits.get(id).copied()
    }

    /// The scope whose keys work while the key of the momentary command `id` is held.
    pub(crate) fn held_scope(&self, id: &str) -> Option<&'static str> {
        self.holds.get(id).copied()
    }

    /// Whether `scope` is a held scope: its keys work while a momentary command's key is held, and its commands go to
    /// the application, as no instance is ever of it.
    pub(crate) fn is_held_scope(&self, scope: &str) -> bool {
        self.holds.values().any(|held| *held == scope)
    }

    /// Whether `scope` is a typing scope (`CommandSet::TYPING`): its keys work while an instance of it is around the
    /// focus, typing included.
    pub(crate) fn is_typing_scope(&self, scope: &str) -> bool {
        self.typing.contains(scope)
    }

    pub(crate) fn defs(&self) -> &[CommandDef] {
        &self.defs
    }

    /// The definitions as `platform` binds them: a command with no default there has none, and one with defaults of
    /// that platform has those only.
    pub(crate) fn defs_for(&self, platform: Platform) -> Vec<CommandDef> {
        let mut defs = self.defs.clone();
        for def in &mut defs {
            let own: Vec<Binding> = self
                .platform_defaults
                .iter()
                .filter(|(id, on, _)| *on == platform && *id == def.command.id)
                .map(|(.., binding)| binding.clone())
                .collect();
            if !own.is_empty() {
                def.defaults = own;
            } else if self.no_default.contains(&(def.command.id.clone(), platform)) {
                def.defaults.clear();
            }
        }
        defs
    }
}
