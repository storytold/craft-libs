//! Built-in command table plus a copy-on-write dynamic overlay.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use serde_json::Value;

use crate::overlay::{Overlay, dynamic_id_allowed};
use crate::spec::{CommandSpec, CommandView, Menu};
use crate::{DynCommand, Enablement, Owner, RegisterError, read_guard, write_guard};

/// A resolved command. Static wins over the overlay.
///
/// `Dynamic` holds its own `Arc`, so a later `register` or `unregister_owner`
/// does not drop a command that is already running.
pub enum CommandEntry<'a, S, E> {
    Static(&'a CommandSpec<S, E>),
    Dynamic(Arc<DynCommand<S, E>>),
}

impl<S, E> std::fmt::Debug for CommandEntry<'_, S, E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandEntry::Static(spec) => formatter.debug_tuple("Static").field(&spec.id).finish(),
            CommandEntry::Dynamic(cmd) => formatter.debug_tuple("Dynamic").field(&cmd.id).finish(),
        }
    }
}

/// Static slice, an O(1) id index, and `RwLock<Arc<Overlay>>`.
///
/// A resolve or `for_each` clones the overlay `Arc` and then drops the lock.
/// A later register replaces the `Arc`. The in-flight clone is unchanged.
pub struct Registry<S: 'static, E: 'static> {
    builtin: &'static [CommandSpec<S, E>],
    index: OnceLock<HashMap<&'static str, usize>>,
    overlay: RwLock<Arc<Overlay<S, E>>>,
}

impl<S: 'static, E: 'static> Registry<S, E> {
    pub fn new(builtin: &'static [CommandSpec<S, E>]) -> Self {
        Self { builtin, index: OnceLock::new(), overlay: RwLock::new(Arc::new(Overlay::empty())) }
    }

    /// Static id first, then the overlay. The static row is `'static`.
    pub fn resolve(&self, id: &str) -> Option<CommandEntry<'static, S, E>> {
        if let Some(&index) = self.index().get(id) {
            return self.builtin.get(index).map(CommandEntry::Static);
        }
        self.overlay_arc().get(id).map(CommandEntry::Dynamic)
    }

    /// Built-in rows, then dynamic rows. No heap allocation per command.
    pub fn for_each(&self, mut visit: impl FnMut(CommandView<'_>)) {
        for spec in self.builtin {
            visit(view_static(spec));
        }
        let overlay = self.overlay_arc();
        for cmd in overlay.iter() {
            visit(view_dynamic(cmd));
        }
    }

    /// Insert a plugin command.
    ///
    /// Duplicate is checked before the reserved-prefix rule, so a built-in id
    /// cannot be re-registered even when the prefix is wrong.
    pub fn register(&self, cmd: DynCommand<S, E>) -> Result<(), RegisterError> {
        let allowed = dynamic_id_allowed(&cmd.id, &cmd.owner);
        let mut guard = write_guard(&self.overlay);
        if self.index().contains_key(cmd.id.as_ref()) || guard.contains(&cmd.id) {
            return Err(RegisterError::Duplicate);
        }
        if !allowed {
            return Err(RegisterError::Reserved);
        }
        Arc::make_mut(&mut guard).insert(Arc::new(cmd));
        Ok(())
    }

    /// Remove every dynamic command with this owner. Returns how many were removed.
    pub fn unregister_owner(&self, owner: &Owner) -> usize {
        let mut guard = write_guard(&self.overlay);
        if !guard.iter().any(|cmd| &cmd.owner == owner) {
            return 0;
        }
        Arc::make_mut(&mut guard).remove_owner(owner)
    }

    /// Write `state` into `Enablement::Cached` for `id`. Other variants are not changed.
    pub fn set_enabled(&self, id: &str, state: Result<(), String>) {
        let mut guard = write_guard(&self.overlay);
        let Some(cmd) = guard.get(id) else {
            return;
        };
        if !matches!(&cmd.enablement, Enablement::Cached(_)) {
            return;
        }
        drop(cmd);
        Arc::make_mut(&mut guard).set_cached(id, state);
    }

    /// Schema for a known id. `None` when the id is not in the registry.
    pub fn schema(&self, id: &str) -> Option<Arc<Value>> {
        if let Some(&index) = self.index().get(id) {
            return self.builtin.get(index).map(|spec| Arc::new(crate::schema::from_doc(spec.params).value));
        }
        self.overlay_arc().get(id).map(|cmd| Arc::new(crate::schema::from_doc(&cmd.params).value))
    }

    fn index(&self) -> &HashMap<&'static str, usize> {
        self.index.get_or_init(|| {
            let mut map = HashMap::with_capacity(self.builtin.len());
            for (index, spec) in self.builtin.iter().enumerate() {
                if let std::collections::hash_map::Entry::Vacant(slot) = map.entry(spec.id) {
                    slot.insert(index);
                }
            }
            map
        })
    }

    fn overlay_arc(&self) -> Arc<Overlay<S, E>> {
        let guard = read_guard(&self.overlay);
        Arc::clone(&guard)
    }
}

fn view_static<'a, S, E>(spec: &'a CommandSpec<S, E>) -> CommandView<'a> {
    CommandView {
        id: spec.id,
        label: spec.label,
        shortcut: spec.shortcut,
        alt_shortcuts: spec.alt_shortcuts,
        params: spec.params,
        journal: spec.journal,
        dynamic: false,
        menu: Menu::Static(spec.menu),
    }
}

fn view_dynamic<'a, S, E>(cmd: &'a DynCommand<S, E>) -> CommandView<'a> {
    CommandView {
        id: &cmd.id,
        label: &cmd.label,
        shortcut: cmd.shortcut.as_deref(),
        alt_shortcuts: &[],
        params: &cmd.params,
        journal: cmd.journal,
        dynamic: true,
        menu: Menu::Shared(&cmd.menu),
    }
}
