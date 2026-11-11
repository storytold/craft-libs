//! Type facts across the modules of a crate.
//!
//! A type is defined in one file and often implemented in others (`impl Button { .. }` in `builder.rs`, `impl Component for
//! Button` in `component.rs`). The facts are keyed by the defining crate and module path, found through the crate's module
//! index, and every file gets the facts of the names it defines, imports or implements.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use syn::Item;

use crate::resolve::{Index, Module};

use super::types::{TypeFacts, Types, add_impl, definition, registered_idents, self_name};

type Key = (String, Vec<String>, String);

fn key_of(index: &Index, krate: &str, module: &Module, name: &str) -> Option<Key> {
    let resolved = index.resolve(krate, &module.path, &[name.to_owned()]).filter(|r| !r.is_module)?;
    Some((resolved.krate, resolved.module, name.to_owned()))
}

/// The facts each file of the index's crates sees, by file path as `rel` shows it.
pub(crate) fn build(index: &Index, rel: &dyn Fn(&Path) -> PathBuf) -> BTreeMap<PathBuf, Types> {
    let mut defs: HashMap<Key, TypeFacts> = HashMap::new();
    for (krate, tree) in &index.trees {
        for module in tree.modules.values() {
            for (name, facts) in index.items(module).into_iter().filter_map(definition) {
                defs.insert((krate.clone(), module.path.clone(), name), facts);
            }
        }
    }
    for (krate, tree) in &index.trees {
        for module in tree.modules.values() {
            for item in index.items(module) {
                if let Item::Impl(imp) = item
                    && let Some(name) = self_name(imp)
                    && let Some(facts) = key_of(index, krate, module, &name).and_then(|k| defs.get_mut(&k))
                {
                    add_impl(facts, imp);
                }
            }
        }
    }
    for (krate, tree) in &index.trees {
        for module in tree.modules.values() {
            for name in registered_idents(index.items(module)) {
                if let Some(facts) = key_of(index, krate, module, &name).and_then(|k| defs.get_mut(&k)) {
                    facts.registered = true;
                }
            }
        }
    }
    let mut files: BTreeMap<PathBuf, Types> = BTreeMap::new();
    for (krate, tree) in &index.trees {
        for module in tree.modules.values() {
            let types = files.entry(rel(&module.file)).or_default();
            let impl_names = index.items(module).into_iter().filter_map(|i| if let Item::Impl(imp) = i { self_name(imp) } else { None });
            for name in module.names.keys().cloned().chain(impl_names) {
                if let Some(facts) = key_of(index, krate, module, &name).and_then(|k| defs.get(&k)) {
                    types.insert(name, facts.clone());
                }
            }
        }
    }
    files
}
