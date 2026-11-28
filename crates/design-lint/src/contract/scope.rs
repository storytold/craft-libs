//! Where each rule's "allowed in" locations come from.
//!
//! The rules ask a [`RuleScope`]; they never name a path. The per-crate layer map
//! (`design/layers/<crate>/<family>.toml`) implements the trait, [`StaticScope`] is the in-memory
//! implementation for tests and small configs.

use std::path::{Path, PathBuf};

use std::collections::BTreeMap;

use crate::Error;
use crate::resolve::Index;
use crate::rules::selector_matches;

use super::types::Types;

/// Answers the questions the egui-ban rules ask about a file.
///
/// Paths are relative to the root the check was started from, as in findings.
pub trait RuleScope {
    /// Whether `file` may use what `key` forbids. `key` is a rule id (`contract/allocate`), or a rule id with a
    /// sub-permission after a colon (`contract/platform:target` for target checks, `contract/motion:clock` for clock reads).
    fn allows(&self, key: &str, file: &Path) -> bool;

    /// The layer (0 to 8) the layer map assigns to the file's module, when it has one.
    fn layer(&self, file: &Path) -> Option<u8>;

    /// Whether the file belongs to a crate that links egui.
    fn egui_linked(&self, file: &Path) -> bool;

    /// Whether the file belongs to a test target with `harness = false`.
    fn harness_free(&self, file: &Path) -> bool;

    /// Whether `name` is on the closed list of types that need no `#[non_exhaustive]`.
    fn is_closed(&self, name: &str) -> bool;

    /// What the crate's module tree says about the types `file` defines, imports or implements, when the scope knows it.
    /// Rules that need a type's definition read it here, and fall back to the file's own items without it.
    fn crate_types(&self, _file: &Path) -> Option<&Types> {
        None
    }
}

/// A [`RuleScope`] built in memory: path prefixes per permission key.
#[derive(Clone, Debug, Default)]
pub struct StaticScope {
    allowed: Vec<(String, PathBuf)>,
    layers: Vec<(PathBuf, u8)>,
    egui_linked: Vec<PathBuf>,
    harness_free: Vec<PathBuf>,
    closed: Vec<String>,
    types: BTreeMap<PathBuf, Types>,
}

impl StaticScope {
    /// A scope that allows nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Files under `prefix` may use what `key` forbids. `key` is a rule id, a family (`contract`) or a sub-permission;
    /// a rule id or family does not cover that rule's sub-permissions.
    #[must_use]
    pub fn allow(mut self, key: &str, prefix: impl Into<PathBuf>) -> Self {
        self.allowed.push((key.to_owned(), prefix.into()));
        self
    }

    /// Files under `prefix` are in `layer`; the longest matching prefix wins.
    #[must_use]
    pub fn layer(mut self, prefix: impl Into<PathBuf>, layer: u8) -> Self {
        self.layers.push((prefix.into(), layer));
        self
    }

    /// Files under `prefix` belong to a crate that links egui.
    #[must_use]
    pub fn egui_linked(mut self, prefix: impl Into<PathBuf>) -> Self {
        self.egui_linked.push(prefix.into());
        self
    }

    /// Files under `prefix` belong to a `harness = false` test target.
    #[must_use]
    pub fn harness_free(mut self, prefix: impl Into<PathBuf>) -> Self {
        self.harness_free.push(prefix.into());
        self
    }

    /// The crate `name` has its library root at `root`/`lib_rs`; its module tree gives the rules the facts of types defined
    /// and implemented in different files. Findings name files relative to `root`.
    pub fn crate_tree(mut self, root: &Path, name: &str, lib_rs: impl AsRef<Path>) -> Result<Self, Error> {
        let index = Index::build(&[(name.to_owned(), root.join(lib_rs))])?;
        let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_owned();
        self.types.extend(super::facts::build(&index, &rel));
        Ok(self)
    }

    /// `name` needs no `#[non_exhaustive]`.
    #[must_use]
    pub fn closed(mut self, name: &str) -> Self {
        self.closed.push(name.to_owned());
        self
    }
}

impl RuleScope for StaticScope {
    fn allows(&self, key: &str, file: &Path) -> bool {
        let rule = key.split_once(':').map_or(key, |(rule, _)| rule);
        self.allowed
            .iter()
            .any(|(k, prefix)| file.starts_with(prefix) && (k == key || (!key.contains(':') && !k.contains(':') && selector_matches(k, rule))))
    }

    fn layer(&self, file: &Path) -> Option<u8> {
        self.layers.iter().filter(|(p, _)| file.starts_with(p)).max_by_key(|(p, _)| p.components().count()).map(|(_, l)| *l)
    }

    fn egui_linked(&self, file: &Path) -> bool {
        self.egui_linked.iter().any(|p| file.starts_with(p))
    }

    fn harness_free(&self, file: &Path) -> bool {
        self.harness_free.iter().any(|p| file.starts_with(p))
    }

    fn is_closed(&self, name: &str) -> bool {
        self.closed.iter().any(|c| c == name)
    }

    fn crate_types(&self, file: &Path) -> Option<&Types> {
        self.types.get(file)
    }
}
