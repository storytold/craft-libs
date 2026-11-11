//! The registry of command definitions: the sets `#[derive(CommandSet)]` registers, loaded at install.

use craft_behavior::EditCommand;
use craft_keymap::{Binding, BindingError, CommandDef, Platform};
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
        platform_defaults: fn() -> Result<Vec<PlatformDefault>, CommandDefError>,
    ) -> Self {
        SetEntry { name, defs, no_default_on, edits, platform_defaults }
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

/// Registers the command set `$ty`, so `install` loads it.
#[doc(hidden)]
#[macro_export]
macro_rules! __register_command_set {
    ($ty:ty) => {
        const _: () = {
            $crate::__private::inventory::submit! {
                $crate::runtime::commands::SetEntry::new(
                    ::std::any::type_name::<$ty>,
                    <$ty as $crate::runtime::commands::CommandSet>::defs,
                    <$ty as $crate::runtime::commands::CommandSet>::no_default_on,
                    <$ty as $crate::runtime::commands::CommandSet>::edits,
                    <$ty as $crate::runtime::commands::CommandSet>::platform_defaults,
                )
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
        }
    }
}

impl std::error::Error for CommandDefError {}

static LOADED: OnceLock<Arc<Registry>> = OnceLock::new();

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
}

impl Registry {
    /// Loads every linked set.
    pub(crate) fn load() -> Result<Registry, CommandDefError> {
        let mut registry = Registry::default();
        for entry in sets() {
            for def in (entry.defs)()? {
                registry.add(def)?;
            }
            registry.no_default.extend((entry.no_default_on)().into_iter().map(|(id, platform)| (id.to_owned(), platform)));
            registry.edits.extend((entry.edits)().into_iter().map(|(id, edit)| (id.to_owned(), edit)));
            registry
                .platform_defaults
                .extend((entry.platform_defaults)()?.into_iter().map(|(id, platform, binding)| (id.to_owned(), platform, binding)));
        }
        Ok(registry)
    }

    /// The registry of every linked set, loaded once per process: the sets are static, so every context shares one.
    /// A load that fails is not kept, so each install reports its error.
    pub(crate) fn shared() -> Result<Arc<Registry>, CommandDefError> {
        if let Some(loaded) = LOADED.get() {
            return Ok(loaded.clone());
        }
        let registry = Arc::new(Registry::load()?);
        Ok(LOADED.get_or_init(|| registry).clone())
    }

    /// Whether `registry` is the one [`Registry::shared`] loaded.
    pub(crate) fn is_shared(registry: &Arc<Registry>) -> bool {
        LOADED.get().is_some_and(|loaded| Arc::ptr_eq(loaded, registry))
    }

    fn add(&mut self, def: CommandDef) -> Result<(), CommandDefError> {
        let id = def.command.id.clone();
        if self.index.insert(id.clone(), self.defs.len()).is_some() {
            return Err(CommandDefError::Duplicate { command: id });
        }
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
