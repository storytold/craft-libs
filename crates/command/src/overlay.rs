//! Dynamic command overlay. The registry stores it behind `Arc` and replaces that `Arc` on write.

use std::sync::Arc;

use serde_json::Value;

/// Plugin generation that owns a dynamic command or a gate.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Owner {
    pub plugin_id: Arc<str>,
    pub generation: u64,
}

/// Declarative enablement. Menus must not call plugin code to compute this.
///
/// `Cached` is the only variant `Registry::set_enabled` writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Enablement {
    Always,
    HasDocument,
    HasSelection,
    Cached(Result<(), String>),
}

/// Where a dynamic command is offered. The app filters; the registry stores the flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Visibility {
    pub menu: bool,
    pub palette: bool,
    pub shortcut: bool,
}

/// Plugin command body. `Arc` so a running call keeps it after unregister.
pub type DynRun<S, E> = dyn Fn(&mut S, &Value) -> Result<Value, E> + Send + Sync;

/// A command registered by a plugin.
pub struct DynCommand<S, E> {
    pub id: Arc<str>,
    pub label: Arc<str>,
    pub params: Arc<str>,
    pub shortcut: Option<Arc<str>>,
    pub menu: Arc<[Arc<str>]>,
    pub owner: Owner,
    pub enablement: Enablement,
    pub visibility: Visibility,
    pub journal: bool,
    pub run: Arc<DynRun<S, E>>,
}

impl<S, E> Clone for DynCommand<S, E> {
    fn clone(&self) -> Self {
        Self {
            id: Arc::clone(&self.id),
            label: Arc::clone(&self.label),
            params: Arc::clone(&self.params),
            shortcut: self.shortcut.clone(),
            menu: Arc::clone(&self.menu),
            owner: self.owner.clone(),
            enablement: self.enablement.clone(),
            visibility: self.visibility,
            journal: self.journal,
            run: Arc::clone(&self.run),
        }
    }
}

impl<S, E> std::fmt::Debug for DynCommand<S, E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DynCommand")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("params", &self.params)
            .field("shortcut", &self.shortcut)
            .field("menu", &self.menu)
            .field("owner", &self.owner)
            .field("enablement", &self.enablement)
            .field("visibility", &self.visibility)
            .field("journal", &self.journal)
            .finish_non_exhaustive()
    }
}

/// Why `Registry::register` rejected a dynamic command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// The id is already a built-in command or a dynamic command.
    Duplicate,
    /// The id does not start with `plugin.<owner.plugin_id>.`.
    Reserved,
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterError::Duplicate => formatter.write_str("command id already registered"),
            RegisterError::Reserved => formatter.write_str("dynamic command id must start with plugin.<plugin_id>."),
        }
    }
}

impl std::error::Error for RegisterError {}

/// `id` is allowed only when it starts with `plugin.<plugin_id>.`.
pub(crate) fn dynamic_id_allowed(id: &str, owner: &Owner) -> bool {
    let Some(rest) = id.strip_prefix("plugin.") else {
        return false;
    };
    let Some(after) = rest.strip_prefix(owner.plugin_id.as_ref()) else {
        return false;
    };
    after.starts_with('.')
}

pub(crate) struct Overlay<S, E> {
    commands: Vec<Arc<DynCommand<S, E>>>,
}

impl<S, E> Clone for Overlay<S, E> {
    fn clone(&self) -> Self {
        Self { commands: self.commands.clone() }
    }
}

impl<S, E> Overlay<S, E> {
    pub(crate) fn empty() -> Self {
        Self { commands: Vec::new() }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Arc<DynCommand<S, E>>> {
        self.commands.iter()
    }

    pub(crate) fn get(&self, id: &str) -> Option<Arc<DynCommand<S, E>>> {
        self.commands.iter().find(|cmd| &*cmd.id == id).cloned()
    }

    pub(crate) fn contains(&self, id: &str) -> bool {
        self.commands.iter().any(|cmd| &*cmd.id == id)
    }

    pub(crate) fn insert(&mut self, cmd: Arc<DynCommand<S, E>>) {
        if self.contains(&cmd.id) {
            return;
        }
        self.commands.push(cmd);
    }

    pub(crate) fn remove_owner(&mut self, owner: &Owner) -> usize {
        let before = self.commands.len();
        self.commands.retain(|cmd| &cmd.owner != owner);
        before - self.commands.len()
    }

    /// Updates `Enablement::Cached` only. Other variants and unknown ids are left alone.
    pub(crate) fn set_cached(&mut self, id: &str, state: Result<(), String>) -> bool {
        let Some(index) = self.commands.iter().position(|cmd| &*cmd.id == id) else {
            return false;
        };
        let Some(slot) = self.commands.get(index) else {
            return false;
        };
        if !matches!(&slot.enablement, Enablement::Cached(_)) {
            return false;
        }
        let mut updated = (**slot).clone();
        updated.enablement = Enablement::Cached(state);
        if let Some(place) = self.commands.get_mut(index) {
            *place = Arc::new(updated);
            true
        } else {
            false
        }
    }
}
