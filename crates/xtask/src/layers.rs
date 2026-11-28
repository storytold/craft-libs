//! The crate-layer check: each workspace crate depends only on crates in lower layers.
//!
//! The app supplies a [`Policy`]: its table of exact package names and their [`Class`], the
//! same-layer edges it allows, its UI crates and the layer they start at, and its exceptions. This
//! module reads the workspace from `cargo metadata` ([`cargo_metadata`], [`from_metadata`]), checks
//! it against the policy ([`check`]) and picks the packages the app's wasm step builds
//! ([`wasm_set`]). The app keeps its `cargo xtask layers` command, the printing, the gate step and
//! the `cargo check --target` call.
//!
//! The rule engine works on a small metadata-independent model ([`Crate`], [`Dep`]), so an app's
//! tests can check its policy on synthetic graphs.
//!
//! ```
//! use craft_xtask::layers::{self, Class, Crate, Dep, DepKind, Policy};
//!
//! const TABLE: &[(&str, Class)] = &[
//!     ("app-geom", Class::Layer(0)),
//!     ("app-doc", Class::Layer(1)),
//!     ("app", Class::Exempt),
//!     ("xtask", Class::BuildTool),
//! ];
//! const POLICY: Policy<'static> = Policy { ui_crates: &["egui*", "winit"], ui_min_layer: 2, ..Policy::new(TABLE) };
//!
//! let dep = |name: &str| Dep { name: name.into(), kind: DepKind::Normal, workspace: name.starts_with("app") };
//! let crates = [
//!     Crate { name: "app-geom".into(), deps: vec![dep("app-doc")] },
//!     Crate { name: "app-doc".into(), deps: vec![dep("app-geom")] },
//!     Crate { name: "app".into(), deps: vec![dep("app-doc"), dep("egui")] },
//!     Crate { name: "xtask".into(), deps: vec![] },
//! ];
//! let violations = layers::check(&POLICY, &crates)?;
//! assert_eq!(violations.len(), 1);
//! assert_eq!(violations[0].to_string(), "app-geom (L0) -> app-doc (L1) [normal]: a crate may depend only on lower layers, or on a same-layer crate the policy allows");
//! # Ok::<(), layers::Error>(())
//! ```

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

/// The default limit on the bytes [`cargo_metadata`] reads from `cargo metadata`: 64 MiB.
pub const DEFAULT_METADATA_LIMIT: usize = 64 * 1024 * 1024;

/// Where a workspace package sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// A layered crate. It may depend on crates in lower layers, and on same-layer crates the
    /// policy allows.
    Layer(u8),
    /// A crate in the given layer that may depend only on the workspace crates
    /// [`Policy::standalone_deps`] lists for it, and on test tooling as a dev-dependency.
    Standalone(u8),
    /// Test tooling. It sits at [`Policy::testkit_layer`] for its own dependencies. Other crates
    /// may use it only as a dev-dependency, except those in [`Policy::testkit_normal_users`].
    Testkit,
    /// Build tooling such as the app's `xtask`: exempt from the layer rules, and the only class
    /// that may depend on another build-tool package or on [`Policy::build_tool_crates`].
    BuildTool,
    /// Apps and other top-level packages: exempt from the layer and UI rules. The test-tooling and
    /// build-tool rules still apply.
    Exempt,
}

impl Class {
    fn layer(self, policy: &Policy<'_>) -> Option<u8> {
        match self {
            Class::Layer(l) | Class::Standalone(l) => Some(l),
            Class::Testkit => Some(policy.testkit_layer),
            Class::BuildTool | Class::Exempt => None,
        }
    }

    fn same_layer_member(self) -> Option<u8> {
        match self {
            Class::Layer(l) | Class::Standalone(l) => Some(l),
            Class::Testkit | Class::BuildTool | Class::Exempt => None,
        }
    }
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Class::Layer(l) => write!(f, "L{l}"),
            Class::Standalone(l) => write!(f, "L{l} standalone"),
            Class::Testkit => f.write_str("testkit"),
            Class::BuildTool => f.write_str("build tool"),
            Class::Exempt => f.write_str("exempt"),
        }
    }
}

/// Which dependencies between two crates of the same layer pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameLayer {
    /// Any same-layer dependency passes, as long as the same-layer edges form no cycle.
    Allowed,
    /// Only a later member of an [`Policy::ordered_chains`] chain using an earlier member, and the
    /// [`Policy::sideways`] pairs, pass.
    Ordered,
}

/// The kind of a dependency, as `cargo metadata` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DepKind {
    /// A `[dependencies]` entry.
    Normal,
    /// A `[dev-dependencies]` entry.
    Dev,
    /// A `[build-dependencies]` entry.
    Build,
}

impl DepKind {
    /// `normal`, `dev` or `build`.
    pub fn as_str(self) -> &'static str {
        match self {
            DepKind::Normal => "normal",
            DepKind::Dev => "dev",
            DepKind::Build => "build",
        }
    }
}

/// One dependency of a workspace crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dep {
    /// The package name of the dependency (not a `rename` alias).
    pub name: String,
    /// The dependency's kind.
    pub kind: DepKind,
    /// `true` when the dependency is a workspace member.
    pub workspace: bool,
}

/// One workspace package and its dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crate {
    /// The package name.
    pub name: String,
    /// Its dependencies, every kind.
    pub deps: Vec<Dep>,
}

/// An app's layer policy. Every name is an exact package name.
///
/// Start from [`Policy::new`] and set the fields the app needs:
/// `Policy { ui_crates: UI, ui_min_layer: 6, ..Policy::new(TABLE) }`.
#[derive(Debug, Clone, Copy)]
pub struct Policy<'a> {
    /// Every workspace package and its class. A name may appear once.
    pub table: &'a [(&'a str, Class)],
    /// Table entries that may be missing from the workspace. Any other table entry that matches no
    /// workspace package is an error.
    pub optional: &'a [&'a str],
    /// Which same-layer dependencies pass.
    pub same_layer: SameLayer,
    /// Chains of same-layer crates: a later member may depend on an earlier one.
    pub ordered_chains: &'a [&'a [&'a str]],
    /// Allowed same-layer dependencies, as `(from, to)`.
    pub sideways: &'a [(&'a str, &'a str)],
    /// External UI crates. An entry ending in `*` matches names that start with the rest; any other
    /// entry matches one name exactly.
    pub ui_crates: &'a [&'a str],
    /// The first layer that may depend on a UI crate.
    pub ui_min_layer: u8,
    /// UI crates that any crate may use as a dev-dependency.
    pub ui_dev_allowed: &'a [&'a str],
    /// Dev-dependencies `(from, to)` that pass whatever the layers of the two crates.
    pub dev_exceptions: &'a [(&'a str, &'a str)],
    /// The workspace crates each [`Class::Standalone`] crate may depend on. Each one must itself be
    /// standalone, in the same layer or lower.
    pub standalone_deps: &'a [(&'a str, &'a [&'a str])],
    /// Crates whose normal and build dependencies on workspace crates are limited to a list, on top
    /// of the layer rule.
    pub format_allowed: &'a [(&'a str, &'a [&'a str])],
    /// The layer [`Class::Testkit`] crates sit at for their own dependencies.
    pub testkit_layer: u8,
    /// External test-tooling crates, which follow the same rule as [`Class::Testkit`] packages.
    pub testkit_crates: &'a [&'a str],
    /// Packages that may depend on test tooling as a normal or build dependency.
    pub testkit_normal_users: &'a [&'a str],
    /// External build-tool crates (such as `craft-xtask`), which only [`Class::BuildTool`]
    /// packages may depend on.
    pub build_tool_crates: &'a [&'a str],
    /// The highest layer [`wasm_set`] selects, or `None` for no layered crate.
    pub wasm_max_layer: Option<u8>,
    /// Further packages [`wasm_set`] selects.
    pub wasm_extra: &'a [&'a str],
}

impl<'a> Policy<'a> {
    /// A policy with `table`, [`SameLayer::Ordered`], no exceptions, no UI crates, test tooling
    /// above every layer and no wasm layer.
    pub const fn new(table: &'a [(&'a str, Class)]) -> Self {
        Policy {
            table,
            optional: &[],
            same_layer: SameLayer::Ordered,
            ordered_chains: &[],
            sideways: &[],
            ui_crates: &[],
            ui_min_layer: 0,
            ui_dev_allowed: &[],
            dev_exceptions: &[],
            standalone_deps: &[],
            format_allowed: &[],
            testkit_layer: u8::MAX,
            testkit_crates: &[],
            testkit_normal_users: &[],
            build_tool_crates: &[],
            wasm_max_layer: None,
            wasm_extra: &[],
        }
    }

    /// The class of the package `name`, or `None` when the table does not list it.
    pub fn classify(&self, name: &str) -> Option<Class> {
        self.table.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
    }

    fn is_ui_crate(&self, name: &str) -> bool {
        self.ui_crates.iter().any(|p| match p.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => name == *p,
        })
    }

    fn same_layer_allowed(&self, from: &str, to: &str) -> bool {
        match self.same_layer {
            SameLayer::Allowed => true,
            SameLayer::Ordered => {
                self.sideways.contains(&(from, to))
                    || self.ordered_chains.iter().any(|chain| match (chain.iter().position(|n| *n == from), chain.iter().position(|n| *n == to)) {
                        (Some(f), Some(t)) => t < f,
                        _ => false,
                    })
            }
        }
    }

    fn standalone_allowed(&self, from: &str, to: &str) -> bool {
        self.standalone_deps.iter().any(|(f, ok)| *f == from && ok.contains(&to))
    }

    fn may_use_testkit_normally(&self, class: Class, name: &str) -> bool {
        class == Class::Testkit || self.testkit_normal_users.contains(&name)
    }
}

/// A broken rule.
///
/// Every field is a plain value, and [`check`] returns the violations sorted by variant and then by
/// field, without duplicates.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Violation {
    /// A workspace package that is not in the table.
    Unregistered {
        /// The package.
        krate: String,
    },
    /// A dependency on a workspace package that is neither in the table nor among the packages
    /// checked.
    UnknownDependency {
        /// The depending package.
        krate: String,
        /// The dependency.
        dep: String,
        /// The dependency's kind.
        kind: DepKind,
    },
    /// A dependency on a crate in the same layer or higher that the policy does not allow.
    Upward {
        /// The depending package.
        krate: String,
        /// The dependency.
        dep: String,
        /// The depending package's layer.
        from: u8,
        /// The dependency's layer.
        to: u8,
        /// The dependency's kind.
        kind: DepKind,
    },
    /// A layered crate that depends on an [`Class::Exempt`] package.
    OnExempt {
        /// The depending package.
        krate: String,
        /// The dependency.
        dep: String,
        /// The dependency's kind.
        kind: DepKind,
    },
    /// A standalone crate's workspace dependency that [`Policy::standalone_deps`] does not list.
    StandaloneHasWorkspaceDep {
        /// The standalone package.
        krate: String,
        /// The dependency.
        dep: String,
        /// The dependency's kind.
        kind: DepKind,
    },
    /// A normal or build dependency that is not in the crate's [`Policy::format_allowed`] list.
    FormatDep {
        /// The limited package.
        krate: String,
        /// The dependency.
        dep: String,
        /// The dependency's kind.
        kind: DepKind,
    },
    /// Test tooling used as a normal or build dependency.
    TestkitAsNormalDep {
        /// The depending package.
        krate: String,
        /// The test tooling.
        dep: String,
        /// The dependency's kind.
        kind: DepKind,
    },
    /// A build-tool package or crate used by a package that is not build tooling.
    BuildToolDep {
        /// The depending package.
        krate: String,
        /// The build tool.
        dep: String,
        /// The dependency's kind.
        kind: DepKind,
    },
    /// A UI crate used below [`Policy::ui_min_layer`].
    UiBelowMinLayer {
        /// The depending package.
        krate: String,
        /// The UI crate.
        dep: String,
        /// The depending package's layer.
        layer: u8,
        /// The first layer that may use a UI crate.
        min: u8,
        /// The dependency's kind.
        kind: DepKind,
    },
    /// Same-layer dependencies that each pass on their own but form a cycle.
    SameLayerCycle {
        /// The packages in the cycle, sorted.
        crates: Vec<String>,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::Unregistered { krate } => write!(f, "{krate}: not in the layer table"),
            Violation::UnknownDependency { krate, dep, kind } => {
                write!(f, "{krate} -> {dep} [{}]: {dep} is a workspace package that is not in the layer table", kind.as_str())
            }
            Violation::Upward { krate, dep, from, to, kind } => write!(
                f,
                "{krate} (L{from}) -> {dep} (L{to}) [{}]: a crate may depend only on lower layers, or on a same-layer crate the policy allows",
                kind.as_str()
            ),
            Violation::OnExempt { krate, dep, kind } => {
                write!(f, "{krate} -> {dep} [{}]: a layered crate may not depend on an exempt package", kind.as_str())
            }
            Violation::StandaloneHasWorkspaceDep { krate, dep, kind } => {
                write!(f, "{krate} -> {dep} [{}]: a standalone crate may depend only on the workspace crates the policy lists for it", kind.as_str())
            }
            Violation::FormatDep { krate, dep, kind } => {
                write!(f, "{krate} -> {dep} [{}]: not in the workspace dependencies the policy allows {krate}", kind.as_str())
            }
            Violation::TestkitAsNormalDep { krate, dep, kind } => {
                write!(f, "{krate} -> {dep} [{}]: test tooling may only be a dev-dependency", kind.as_str())
            }
            Violation::BuildToolDep { krate, dep, kind } => {
                write!(f, "{krate} -> {dep} [{}]: only build tooling may depend on {dep}", kind.as_str())
            }
            Violation::UiBelowMinLayer { krate, dep, layer, min, kind } => {
                write!(f, "{krate} (L{layer}) -> {dep} [{}]: UI crates are allowed only from L{min}", kind.as_str())
            }
            Violation::SameLayerCycle { crates } => write!(f, "same-layer dependency cycle among {}", crates.join(", ")),
        }
    }
}

/// Why the check could not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The policy, or the policy together with the workspace, cannot be checked: an empty table, a
    /// repeated or unknown name, a table entry that matches no package, an empty workspace.
    Policy(String),
    /// The `cargo metadata` output is missing a field this module needs, or holds a value it does
    /// not know.
    Metadata(String),
    /// The metadata command could not be started, failed, or wrote more than the byte limit.
    Process(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Policy(m) => write!(f, "layer policy: {m}"),
            Error::Metadata(m) => write!(f, "cargo metadata: {m}"),
            Error::Process(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

fn policy_error(message: impl Into<String>) -> Error {
    Error::Policy(message.into())
}

/// Checks the policy against itself and the workspace, and returns the table as a map.
fn validate<'p>(policy: &Policy<'p>, crates: &[Crate]) -> Result<HashMap<&'p str, Class>, Error> {
    if policy.table.is_empty() {
        return Err(policy_error("the table is empty"));
    }
    if crates.is_empty() {
        return Err(policy_error("the workspace has no packages"));
    }
    let mut table = HashMap::with_capacity(policy.table.len());
    for &(name, class) in policy.table {
        if name.is_empty() {
            return Err(policy_error("the table has an empty name"));
        }
        if table.insert(name, class).is_some() {
            return Err(policy_error(format!("{name} appears more than once in the table")));
        }
    }
    let mut members = HashSet::with_capacity(crates.len());
    for c in crates {
        if !members.insert(c.name.as_str()) {
            return Err(policy_error(format!("the workspace lists {} more than once", c.name)));
        }
    }
    let known = |field: &str, name: &str| -> Result<Class, Error> {
        table.get(name).copied().ok_or_else(|| policy_error(format!("{field} names {name}, which is not in the table")))
    };
    for name in policy.optional {
        known("optional", name)?;
    }
    for &(name, _) in policy.table {
        if !members.contains(name) && !policy.optional.contains(&name) {
            return Err(policy_error(format!("{name} is in the table but not in the workspace; list it in `optional` if it may be missing")));
        }
    }
    for chain in policy.ordered_chains {
        let mut layer = None;
        for name in *chain {
            let l = known("ordered_chains", name)?
                .same_layer_member()
                .ok_or_else(|| policy_error(format!("ordered_chains names {name}, which is not a layered crate")))?;
            if layer.is_some_and(|first| first != l) {
                return Err(policy_error(format!("ordered_chains puts {name} in a chain with crates of another layer")));
            }
            layer = Some(l);
        }
    }
    for &(from, to) in policy.sideways {
        let (f, t) = (known("sideways", from)?.same_layer_member(), known("sideways", to)?.same_layer_member());
        if f.is_none() || f != t {
            return Err(policy_error(format!("sideways pair {from} -> {to} is not two layered crates of one layer")));
        }
    }
    for &(from, to) in policy.dev_exceptions {
        known("dev_exceptions", from)?;
        known("dev_exceptions", to)?;
    }
    for &(from, list) in policy.standalone_deps {
        let Class::Standalone(from_layer) = known("standalone_deps", from)? else {
            return Err(policy_error(format!("standalone_deps names {from}, which is not standalone")));
        };
        for to in list {
            match known("standalone_deps", to)? {
                Class::Standalone(l) if l <= from_layer => {}
                _ => return Err(policy_error(format!("standalone_deps lets {from} use {to}, which is not standalone at L{from_layer} or lower"))),
            }
        }
    }
    for &(from, list) in policy.format_allowed {
        if known("format_allowed", from)?.layer(policy).is_none() {
            return Err(policy_error(format!("format_allowed names {from}, which is exempt from the layer rules")));
        }
        for to in list {
            known("format_allowed", to)?;
        }
    }
    for name in policy.testkit_normal_users {
        known("testkit_normal_users", name)?;
    }
    for name in policy.wasm_extra {
        if matches!(known("wasm_extra", name)?, Class::Testkit | Class::BuildTool) {
            return Err(policy_error(format!("wasm_extra names {name}, which is test or build tooling")));
        }
    }
    for p in policy.ui_crates {
        let stem = p.strip_suffix('*').unwrap_or(p);
        if stem.is_empty() || stem.contains('*') {
            return Err(policy_error(format!("ui_crates entry `{p}` must be a name, or a name prefix followed by one `*`")));
        }
    }
    Ok(table)
}

/// The checks of one package, with what they need from the policy.
struct Checker<'a, 'p> {
    policy: &'a Policy<'p>,
    table: &'a HashMap<&'p str, Class>,
    members: &'a HashSet<&'a str>,
    out: Vec<Violation>,
    same_layer_edges: BTreeMap<&'a str, BTreeSet<&'a str>>,
}

impl<'a> Checker<'a, '_> {
    fn workspace_dep(&mut self, c: &'a Crate, class: Class, d: &'a Dep) {
        let (krate, dep, kind) = (c.name.clone(), d.name.clone(), d.kind);
        // The allowlist applies to every normal and build dependency on a workspace crate, before
        // any class rule can accept the edge.
        if kind != DepKind::Dev
            && let Some((_, allowed)) = self.policy.format_allowed.iter().find(|(n, _)| *n == c.name)
            && !allowed.contains(&d.name.as_str())
        {
            self.out.push(Violation::FormatDep { krate: krate.clone(), dep: dep.clone(), kind });
        }
        let Some(dep_class) = self.table.get(d.name.as_str()).copied() else {
            // A member that is not in the table is reported once, on its own entry.
            if !self.members.contains(d.name.as_str()) {
                self.out.push(Violation::UnknownDependency { krate, dep, kind });
            }
            return;
        };
        match dep_class {
            Class::BuildTool => {
                if class != Class::BuildTool {
                    self.out.push(Violation::BuildToolDep { krate, dep, kind });
                }
                return;
            }
            Class::Testkit => {
                if kind != DepKind::Dev && !self.policy.may_use_testkit_normally(class, &c.name) {
                    self.out.push(Violation::TestkitAsNormalDep { krate, dep, kind });
                }
                return;
            }
            Class::Layer(_) | Class::Standalone(_) | Class::Exempt => {}
        }
        let Some(from) = class.layer(self.policy) else { return };
        if let Class::Standalone(_) = class {
            if !self.policy.standalone_allowed(&c.name, &d.name) {
                self.out.push(Violation::StandaloneHasWorkspaceDep { krate, dep, kind });
            } else if dep_class.layer(self.policy) == Some(from) {
                self.same_layer_edges.entry(&c.name).or_default().insert(&d.name);
            }
            return;
        }
        let Some(to) = dep_class.layer(self.policy) else {
            self.out.push(Violation::OnExempt { krate, dep, kind });
            return;
        };
        if kind == DepKind::Dev && self.policy.dev_exceptions.contains(&(c.name.as_str(), d.name.as_str())) {
            if to == from {
                self.same_layer_edges.entry(&c.name).or_default().insert(&d.name);
            }
            return;
        }
        if to < from {
            return;
        }
        if to == from && class.same_layer_member().is_some() && self.policy.same_layer_allowed(&c.name, &d.name) {
            self.same_layer_edges.entry(&c.name).or_default().insert(&d.name);
            return;
        }
        self.out.push(Violation::Upward { krate, dep, from, to, kind });
    }

    fn external_dep(&mut self, c: &Crate, class: Class, d: &Dep) {
        let (name, kind) = (d.name.as_str(), d.kind);
        if self.policy.build_tool_crates.contains(&name) && class != Class::BuildTool {
            self.out.push(Violation::BuildToolDep { krate: c.name.clone(), dep: d.name.clone(), kind });
        }
        if self.policy.testkit_crates.contains(&name) && kind != DepKind::Dev && !self.policy.may_use_testkit_normally(class, &c.name) {
            self.out.push(Violation::TestkitAsNormalDep { krate: c.name.clone(), dep: d.name.clone(), kind });
        }
        let Some(layer) = class.layer(self.policy) else { return };
        let dev_allowed = kind == DepKind::Dev && self.policy.ui_dev_allowed.contains(&name);
        if layer < self.policy.ui_min_layer && self.policy.is_ui_crate(name) && !dev_allowed {
            self.out.push(Violation::UiBelowMinLayer { krate: c.name.clone(), dep: d.name.clone(), layer, min: self.policy.ui_min_layer, kind });
        }
    }
}

/// Checks every package in `crates` against `policy`.
///
/// Returns the violations sorted, without duplicates. Every dependency kind is checked; a crate's
/// workspace dependency on itself (a dev-dependency that turns on its own features) is skipped.
/// Same-layer edges that pass, dev exceptions included, are checked for cycles.
///
/// Returns an error when the policy cannot be applied to this workspace: see [`Error::Policy`].
pub fn check(policy: &Policy<'_>, crates: &[Crate]) -> Result<Vec<Violation>, Error> {
    let table = validate(policy, crates)?;
    let members: HashSet<&str> = crates.iter().map(|c| c.name.as_str()).collect();
    let mut checker = Checker { policy, table: &table, members: &members, out: Vec::new(), same_layer_edges: BTreeMap::new() };
    for c in crates {
        let Some(class) = table.get(c.name.as_str()).copied() else {
            checker.out.push(Violation::Unregistered { krate: c.name.clone() });
            continue;
        };
        for d in &c.deps {
            if d.workspace && d.name == c.name {
                continue;
            }
            if d.workspace {
                checker.workspace_dep(c, class, d);
            } else {
                checker.external_dep(c, class, d);
            }
        }
    }
    let mut out = checker.out;
    for crates in cycles(&checker.same_layer_edges) {
        out.push(Violation::SameLayerCycle { crates });
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The strongly connected components with more than one member, each sorted, found with an
/// iterative Tarjan walk so that a long chain cannot overflow the stack.
fn cycles(edges: &BTreeMap<&str, BTreeSet<&str>>) -> Vec<Vec<String>> {
    let nodes: Vec<&str> =
        edges.iter().flat_map(|(from, to)| std::iter::once(*from).chain(to.iter().copied())).collect::<BTreeSet<_>>().into_iter().collect();
    let id: HashMap<&str, usize> = nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let adj: Vec<Vec<usize>> =
        nodes.iter().map(|n| edges.get(n).map(|to| to.iter().filter_map(|t| id.get(t).copied()).collect()).unwrap_or_default()).collect();
    const UNSEEN: usize = usize::MAX;
    let mut index = vec![UNSEEN; nodes.len()];
    let mut low = vec![0; nodes.len()];
    let mut on_stack = vec![false; nodes.len()];
    let mut stack = Vec::new();
    let mut next = 0;
    let mut out = Vec::new();
    for root in 0..nodes.len() {
        if index[root] != UNSEEN {
            continue;
        }
        let mut work = vec![(root, 0usize)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(top) = work.last_mut() {
            let v = top.0;
            if let Some(&w) = adj[v].get(top.1) {
                top.1 += 1;
                if index[w] == UNSEEN {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut component = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    component.push(nodes[w].to_owned());
                    if w == v {
                        break;
                    }
                }
                if component.len() > 1 {
                    component.sort();
                    out.push(component);
                }
            }
        }
    }
    out
}

/// The packages the app's wasm step builds: the workspace's layered and standalone crates at or
/// below [`Policy::wasm_max_layer`], and the [`Policy::wasm_extra`] packages the workspace has,
/// sorted and without duplicates. Test and build tooling are never selected.
///
/// The set says which packages the app means to build for wasm, not that they build. An empty set
/// is an error, as is a policy that [`check`] would refuse.
pub fn wasm_set(policy: &Policy<'_>, crates: &[Crate]) -> Result<Vec<String>, Error> {
    let table = validate(policy, crates)?;
    let mut out = BTreeSet::new();
    for c in crates {
        let selected = match table.get(c.name.as_str()) {
            Some(Class::Layer(l) | Class::Standalone(l)) => policy.wasm_max_layer.is_some_and(|max| *l <= max),
            Some(Class::Exempt) => false,
            Some(Class::Testkit | Class::BuildTool) | None => continue,
        };
        if selected || policy.wasm_extra.contains(&c.name.as_str()) {
            out.insert(c.name.clone());
        }
    }
    if out.is_empty() {
        return Err(policy_error("the wasm set is empty"));
    }
    Ok(out.into_iter().collect())
}

fn metadata_error(message: impl Into<String>) -> Error {
    Error::Metadata(message.into())
}

fn str_field<'v>(v: &'v Value, key: &str, what: &str) -> Result<&'v str, Error> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| metadata_error(format!("{what} has no `{key}` string")))
}

/// A path with `/` separators and no trailing separator, so that Windows and Unix paths compare.
fn normalize_path(path: &str) -> String {
    let p = path.replace('\\', "/");
    p.trim_end_matches('/').to_owned()
}

fn dep_kind(d: &Value, what: &str) -> Result<DepKind, Error> {
    match d.get("kind") {
        Some(Value::Null) => Ok(DepKind::Normal),
        Some(Value::String(k)) if k == "dev" => Ok(DepKind::Dev),
        Some(Value::String(k)) if k == "build" => Ok(DepKind::Build),
        Some(other) => Err(metadata_error(format!("{what} has the unknown kind {other}"))),
        None => Err(metadata_error(format!("{what} has no `kind`"))),
    }
}

/// Builds the model from the output of `cargo metadata --format-version 1 --no-deps`.
///
/// A dependency is a workspace member when its `path` is the directory of a member's
/// `manifest_path`; a registry or git dependency with a member's name, and a path dependency
/// outside the workspace, are external. A renamed dependency is matched by its package `name`.
/// Missing or unknown fields, a package that is not in `workspace_members` and a repeated package
/// name are errors. The crates come back sorted by name.
pub fn from_metadata(meta: &Value) -> Result<Vec<Crate>, Error> {
    let pkgs = meta.get("packages").and_then(Value::as_array).ok_or_else(|| metadata_error("no `packages` array"))?;
    let member_ids = meta.get("workspace_members").and_then(Value::as_array).ok_or_else(|| metadata_error("no `workspace_members` array"))?;
    let mut ids = HashSet::with_capacity(member_ids.len());
    for id in member_ids {
        let id = id.as_str().ok_or_else(|| metadata_error("a `workspace_members` entry is not a string"))?;
        ids.insert(id);
    }
    let mut dirs: HashMap<String, &str> = HashMap::with_capacity(pkgs.len());
    let mut names = HashSet::with_capacity(pkgs.len());
    let mut seen_ids = HashSet::with_capacity(pkgs.len());
    for p in pkgs {
        let name = str_field(p, "name", "a package")?;
        if name.is_empty() {
            return Err(metadata_error("a package has an empty name"));
        }
        let what = format!("package {name}");
        let id = str_field(p, "id", &what)?;
        if !ids.contains(id) {
            return Err(metadata_error(format!("{what} is not in `workspace_members`")));
        }
        if !names.insert(name) || !seen_ids.insert(id) {
            return Err(metadata_error(format!("{what} appears more than once")));
        }
        let manifest = normalize_path(str_field(p, "manifest_path", &what)?);
        let Some(dir) = manifest.strip_suffix("/Cargo.toml") else {
            return Err(metadata_error(format!("{what} has a `manifest_path` that does not end in Cargo.toml")));
        };
        dirs.insert(dir.to_owned(), name);
    }
    if let Some(id) = ids.iter().find(|id| !seen_ids.contains(*id)) {
        return Err(metadata_error(format!("workspace member {id} has no package entry")));
    }
    let mut out = Vec::with_capacity(pkgs.len());
    for p in pkgs {
        let name = str_field(p, "name", "a package")?;
        let what = format!("package {name}");
        let raw = p.get("dependencies").and_then(Value::as_array).ok_or_else(|| metadata_error(format!("{what} has no `dependencies` array")))?;
        let mut deps = Vec::with_capacity(raw.len());
        for d in raw {
            let dname = str_field(d, "name", &format!("a dependency of {name}"))?;
            if dname.is_empty() {
                return Err(metadata_error(format!("a dependency of {name} has an empty name")));
            }
            let dwhat = format!("dependency {dname} of {name}");
            let kind = dep_kind(d, &dwhat)?;
            let member = match d.get("path") {
                None | Some(Value::Null) => None,
                Some(Value::String(path)) => dirs.get(&normalize_path(path)).copied(),
                Some(_) => return Err(metadata_error(format!("{dwhat} has a `path` that is not a string"))),
            };
            if let Some(member) = member
                && member != dname
            {
                return Err(metadata_error(format!("{dwhat} points at the directory of member {member}")));
            }
            deps.push(Dep { name: dname.to_owned(), kind, workspace: member.is_some() });
        }
        out.push(Crate { name: name.to_owned(), deps });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Runs `cmd` with stdout piped and stderr inherited, reads at most `max_bytes` of its output and
/// parses it as JSON. A stderr setting on `cmd` is replaced.
///
/// Returns [`Error::Process`] when the command cannot be started, writes more than `max_bytes`
/// (the child is then ended), or exits with a failure status, and [`Error::Metadata`] when its
/// output is not JSON. `label` names the command in each message.
pub fn read_json(mut cmd: Command, label: &str, max_bytes: usize) -> Result<Value, Error> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::inherit());
    let mut child = cmd.spawn().map_err(|e| Error::Process(format!("{label}: failed to spawn: {e}")))?;
    let mut buf = Vec::new();
    let read = match child.stdout.take() {
        Some(stdout) => stdout.take(u64::try_from(max_bytes).unwrap_or(u64::MAX).saturating_add(1)).read_to_end(&mut buf).map(|_| ()),
        None => Err(std::io::Error::other("no stdout pipe")),
    };
    let failure = match read {
        Err(e) => Some(format!("{label}: reading its output failed: {e}")),
        Ok(()) if buf.len() > max_bytes => Some(format!("{label}: wrote more than {max_bytes} bytes")),
        Ok(()) => None,
    };
    if let Some(message) = failure {
        // The child may still be writing; end it so that the wait returns.
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::Process(message));
    }
    let status = child.wait().map_err(|e| Error::Process(format!("{label}: waiting for it failed: {e}")))?;
    if !status.success() {
        return Err(Error::Process(format!("{label}: exited with {status}")));
    }
    serde_json::from_slice(&buf).map_err(|e| metadata_error(format!("{label}: the output is not JSON: {e}")))
}

/// Runs `cargo metadata --format-version 1 --no-deps` in `root` (through [`crate::runner::cargo`])
/// and returns its parsed output, reading at most `max_bytes` (see [`DEFAULT_METADATA_LIMIT`]).
pub fn cargo_metadata(root: &Path, max_bytes: usize) -> Result<Value, Error> {
    let mut cmd = crate::runner::cargo(root);
    cmd.args(["metadata", "--format-version", "1", "--no-deps"]);
    read_json(cmd, "cargo metadata --format-version 1 --no-deps", max_bytes)
}

#[cfg(test)]
mod tests {
    use super::DepKind::{Build, Dev, Normal};
    use super::*;

    /// A synthetic policy in the shape of the class-based apps: two L0 foundations in a chain, two
    /// L1 crates in a chain, sideways-free L2 crates, UI from L6, test tooling at L6.
    const TABLE: &[(&str, Class)] = &[
        ("app-geom", Class::Layer(0)),
        ("app-color", Class::Layer(0)),
        ("app-doc", Class::Layer(1)),
        ("app-fonts", Class::Layer(1)),
        ("app-compose", Class::Layer(2)),
        ("app-images", Class::Layer(2)),
        ("app-render", Class::Layer(3)),
        ("app-engine", Class::Layer(5)),
        ("app-ui-egui", Class::Layer(6)),
        ("app-mcp", Class::Layer(6)),
        ("app-testkit", Class::Testkit),
        ("app", Class::Exempt),
        ("app-cli", Class::Exempt),
        ("app-web", Class::Exempt),
        ("xtask", Class::BuildTool),
    ];
    const ALL: &[&str] = &[
        "app-geom",
        "app-color",
        "app-doc",
        "app-fonts",
        "app-compose",
        "app-images",
        "app-render",
        "app-engine",
        "app-ui-egui",
        "app-mcp",
        "app-testkit",
        "app",
        "app-cli",
        "app-web",
        "xtask",
    ];
    const POLICY: Policy<'static> = Policy {
        optional: ALL,
        ordered_chains: &[&["app-geom", "app-color"], &["app-doc", "app-fonts"]],
        ui_crates: &["craft-ui", "craft-window", "egui", "eframe", "winit", "egui_kittest", "rfd", "bevy*"],
        ui_min_layer: 6,
        testkit_layer: 6,
        ..Policy::new(TABLE)
    };

    fn c(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate { name: name.into(), deps: deps.iter().map(|(n, k, w)| Dep { name: (*n).into(), kind: *k, workspace: *w }).collect() }
    }

    fn check_with(policy: &Policy<'_>, crates: &[Crate]) -> Vec<Violation> {
        match super::check(policy, crates) {
            Ok(v) => v,
            Err(e) => panic!("{e}"),
        }
    }

    fn check(crates: &[Crate]) -> Vec<Violation> {
        check_with(&POLICY, crates)
    }

    #[test]
    fn clean_downward_graph_passes() {
        let g = [
            c("app-geom", &[("kurbo", Normal, false)]),
            c("app-doc", &[("app-geom", Normal, true)]),
            c("app-engine", &[("app-doc", Normal, true), ("app-testkit", Dev, true)]),
            c("app-ui-egui", &[("app-engine", Normal, true), ("egui", Normal, false)]),
            c("app-cli", &[("app-ui-egui", Normal, true)]),
        ];
        assert!(check(&g).is_empty(), "{:?}", check(&g));
    }

    #[test]
    fn upward_dependency_flagged() {
        let v = check(&[c("app-doc", &[("app-engine", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 1, to: 5, .. }]), "{v:?}");
    }

    #[test]
    fn sideways_dependency_flagged() {
        let v = check(&[c("app-compose", &[("app-images", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 2, to: 2, .. }]), "{v:?}");
    }

    #[test]
    fn l0_foundation_chain_allowed_one_way() {
        assert!(check(&[c("app-color", &[("app-geom", Normal, true)])]).is_empty());
        let v = check(&[c("app-geom", &[("app-color", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 0, to: 0, .. }]), "{v:?}");
    }

    #[test]
    fn self_dev_dependency_ignored() {
        assert!(check(&[c("app-doc", &[("app-doc", Dev, true)])]).is_empty());
    }

    #[test]
    fn external_dependency_named_like_its_crate_is_checked() {
        let policy = Policy { ui_crates: &["winit"], ui_min_layer: 6, ..Policy::new(&[("winit", Class::Layer(1))]) };
        let v = check_with(&policy, &[c("winit", &[("winit", Normal, false)])]);
        assert!(matches!(v[..], [Violation::UiBelowMinLayer { .. }]), "{v:?}");
    }

    #[test]
    fn upward_dev_dependency_flagged() {
        let v = check(&[c("app-geom", &[("app-doc", Dev, true)])]);
        assert!(matches!(v[..], [Violation::Upward { kind: Dev, .. }]), "{v:?}");
    }

    #[test]
    fn ui_crates_below_l6_flagged() {
        for dep in ["craft-ui", "egui", "eframe", "winit", "egui_kittest", "rfd", "bevy_ecs", "bevy"] {
            let v = check(&[c("app-engine", &[(dep, Normal, false)])]);
            assert!(matches!(v[..], [Violation::UiBelowMinLayer { layer: 5, min: 6, .. }]), "{dep}: {v:?}");
        }
        assert!(check(&[c("app-engine", &[("egui_extras_not", Normal, false)])]).is_empty());
        assert!(check(&[c("app-mcp", &[("winit", Normal, false)])]).is_empty());
    }

    #[test]
    fn unregistered_crate_is_error() {
        let v = check(&[c("app-mystery", &[])]);
        assert!(matches!(&v[..], [Violation::Unregistered { krate }] if krate == "app-mystery"), "{v:?}");
        assert!(v[0].to_string().contains("not in the layer table"));
    }

    #[test]
    fn testkit_only_as_dev_dependency() {
        let v = check(&[c("app-render", &[("app-testkit", Normal, true)])]);
        assert!(matches!(v[..], [Violation::TestkitAsNormalDep { .. }]), "{v:?}");
        let v = check(&[c("app-render", &[("app-testkit", Build, true)])]);
        assert!(matches!(v[..], [Violation::TestkitAsNormalDep { kind: Build, .. }]), "{v:?}");
        assert!(check(&[c("app-render", &[("app-testkit", Dev, true)])]).is_empty());
        // Test tooling may use anything below its own layer, but not a crate in that layer.
        assert!(check(&[c("app-testkit", &[("app-engine", Normal, true)])]).is_empty());
        assert!(!check(&[c("app-testkit", &[("app-ui-egui", Normal, true)])]).is_empty());
        // An app is exempt from the layers, not from the test-tooling rule.
        let v = check(&[c("app", &[("app-testkit", Normal, true)])]);
        assert!(matches!(v[..], [Violation::TestkitAsNormalDep { .. }]), "{v:?}");
    }

    #[test]
    fn apps_and_xtask_exempt() {
        for app in ["app", "app-cli", "app-web", "xtask"] {
            let v = check(&[c(app, &[("egui", Normal, false), ("app-ui-egui", Normal, true)])]);
            assert!(v.is_empty(), "{app}: {v:?}");
        }
    }

    fn package(name: &str, dir: &str, deps: Value) -> Value {
        serde_json::json!({ "name": name, "id": format!("path+file://{dir}#{name}@0.1.0"), "manifest_path": format!("{dir}/Cargo.toml"), "dependencies": deps })
    }

    fn metadata(packages: Vec<Value>) -> Value {
        let ids: Vec<Value> = packages.iter().map(|p| p["id"].clone()).collect();
        serde_json::json!({ "packages": packages, "workspace_members": ids })
    }

    fn parse(meta: &Value) -> Vec<Crate> {
        match from_metadata(meta) {
            Ok(g) => g,
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn metadata_parsing() {
        let meta = metadata(vec![
            package(
                "app-doc",
                "/x/crates/doc",
                serde_json::json!([
                    { "name": "app-geom", "kind": null, "path": "/x/crates/geom" },
                    { "name": "serde", "kind": null },
                    { "name": "proptest", "kind": "dev" },
                    { "name": "cc", "kind": "build" },
                ]),
            ),
            package("app-geom", "/x/crates/geom", serde_json::json!([])),
        ]);
        let g = parse(&meta);
        assert_eq!(g.len(), 2);
        let doc = g.iter().find(|c| c.name == "app-doc").unwrap();
        assert!(doc.deps[0].workspace && !doc.deps[1].workspace);
        assert_eq!(doc.deps[2].kind, Dev);
        assert_eq!(doc.deps[3].kind, Build);
        assert!(check(&g).is_empty());
    }

    #[test]
    fn registry_name_collision_is_external() {
        let meta = metadata(vec![
            package(
                "app-doc",
                "/x/crates/doc",
                serde_json::json!([{ "name": "app-engine", "kind": null, "source": "registry+https://github.com/rust-lang/crates.io-index" }]),
            ),
            package("app-engine", "/x/crates/engine", serde_json::json!([])),
        ]);
        let g = parse(&meta);
        assert!(!g[0].deps[0].workspace, "{g:?}");
        assert!(check(&g).is_empty());
    }

    #[test]
    fn renamed_workspace_dependency_is_checked() {
        let meta = metadata(vec![
            package(
                "app-doc",
                "/x/crates/doc",
                serde_json::json!([{ "name": "app-engine", "rename": "engine", "kind": null, "path": "/x/crates/engine" }]),
            ),
            package("app-engine", "/x/crates/engine", serde_json::json!([])),
        ]);
        let g = parse(&meta);
        assert!(g[0].deps[0].workspace && g[0].deps[0].name == "app-engine", "{g:?}");
        let v = check(&g);
        assert!(matches!(v[..], [Violation::Upward { from: 1, to: 5, .. }]), "{v:?}");
    }

    #[test]
    fn outside_path_dependency_is_external() {
        let meta = metadata(vec![
            package("app-doc", "/x/crates/doc", serde_json::json!([{ "name": "app-engine", "kind": null, "path": "/elsewhere/engine" }])),
            package("app-engine", "/x/crates/engine", serde_json::json!([])),
        ]);
        assert!(!parse(&meta)[0].deps[0].workspace);
    }

    #[test]
    fn windows_manifest_path_matches_member() {
        let meta = serde_json::json!({
            "packages": [
                { "name": "app-doc", "id": "a", "manifest_path": "C:\\x\\crates\\doc\\Cargo.toml",
                  "dependencies": [{ "name": "app-geom", "kind": null, "path": "C:\\x\\crates\\geom\\" }] },
                { "name": "app-geom", "id": "b", "manifest_path": "C:\\x\\crates\\geom\\Cargo.toml", "dependencies": [] },
            ],
            "workspace_members": ["a", "b"],
        });
        assert!(parse(&meta)[0].deps[0].workspace);
    }

    fn metadata_err(meta: &Value) -> String {
        match from_metadata(meta) {
            Ok(g) => panic!("parsed: {g:?}"),
            Err(Error::Metadata(m)) => m,
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn missing_dependencies_is_error() {
        let mut p = package("app-doc", "/x/doc", serde_json::json!([]));
        if let Some(o) = p.as_object_mut() {
            o.remove("dependencies");
        }
        assert!(metadata_err(&metadata(vec![p])).contains("dependencies"));
    }

    #[test]
    fn unknown_kind_is_error() {
        let meta = metadata(vec![package("app-doc", "/x/doc", serde_json::json!([{ "name": "serde", "kind": "target" }]))]);
        assert!(metadata_err(&meta).contains("unknown kind"));
        let meta = metadata(vec![package("app-doc", "/x/doc", serde_json::json!([{ "name": "serde" }]))]);
        assert!(metadata_err(&meta).contains("no `kind`"));
    }

    #[test]
    fn missing_dependency_name_is_error() {
        let meta = metadata(vec![package("app-doc", "/x/doc", serde_json::json!([{ "kind": null }]))]);
        assert!(metadata_err(&meta).contains("`name`"));
        let meta = metadata(vec![package("app-doc", "/x/doc", serde_json::json!([{ "name": "", "kind": null }]))]);
        assert!(metadata_err(&meta).contains("empty name"));
    }

    #[test]
    fn unregistered_member_is_error() {
        // A package that `workspace_members` does not list, and a member without a package entry.
        let mut meta = metadata(vec![package("app-doc", "/x/doc", serde_json::json!([]))]);
        meta["workspace_members"] = serde_json::json!([]);
        assert!(metadata_err(&meta).contains("not in `workspace_members`"));
        let mut meta = metadata(vec![package("app-doc", "/x/doc", serde_json::json!([]))]);
        meta["workspace_members"] = serde_json::json!(["path+file:///x/doc#app-doc@0.1.0", "gone"]);
        assert!(metadata_err(&meta).contains("gone"));
    }

    #[test]
    fn duplicate_member_is_error() {
        let meta = metadata(vec![package("app-doc", "/x/a", serde_json::json!([])), package("app-doc", "/x/b", serde_json::json!([]))]);
        assert!(metadata_err(&meta).contains("more than once"));
        let Err(Error::Policy(m)) = super::check(&POLICY, &[c("app-doc", &[]), c("app-doc", &[])]) else { panic!("a repeated crate passed") };
        assert!(m.contains("more than once"), "{m}");
    }

    #[test]
    fn unknown_workspace_dependency_is_error() {
        let v = check(&[c("app-doc", &[("app-ghost", Normal, true)])]);
        assert!(matches!(&v[..], [Violation::UnknownDependency { dep, .. }] if dep == "app-ghost"), "{v:?}");
        // When the unknown crate is checked too, it is reported once, on its own entry.
        let v = check(&[c("app-doc", &[("app-ghost", Normal, true)]), c("app-ghost", &[])]);
        assert!(matches!(&v[..], [Violation::Unregistered { krate }] if krate == "app-ghost"), "{v:?}");
    }

    fn policy_err(policy: &Policy<'_>, crates: &[Crate]) -> String {
        match super::check(policy, crates) {
            Ok(v) => panic!("checked: {v:?}"),
            Err(Error::Policy(m)) => m,
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn empty_policy_is_error() {
        assert!(policy_err(&Policy::new(&[]), &[c("app-doc", &[])]).contains("table is empty"));
        assert!(policy_err(&POLICY, &[]).contains("no packages"));
    }

    #[test]
    fn duplicate_policy_entry_is_error() {
        let table = [("app-doc", Class::Layer(1)), ("app-doc", Class::Layer(2))];
        assert!(policy_err(&Policy::new(&table), &[c("app-doc", &[])]).contains("more than once"));
    }

    #[test]
    fn unknown_policy_entry_is_error() {
        let table = [("app-doc", Class::Layer(1)), ("app-dco", Class::Layer(1))];
        assert!(policy_err(&Policy::new(&table), &[c("app-doc", &[])]).contains("app-dco"));
        // An entry the policy names as optional may be missing.
        assert!(check_with(&Policy { optional: &["app-dco"], ..Policy::new(&table) }, &[c("app-doc", &[])]).is_empty());
        // Every list must name table entries.
        for policy in [
            Policy { ordered_chains: &[&["app-doc", "app-x"]], ..Policy::new(&table[..1]) },
            Policy { sideways: &[("app-doc", "app-x")], ..Policy::new(&table[..1]) },
            Policy { dev_exceptions: &[("app-doc", "app-x")], ..Policy::new(&table[..1]) },
            Policy { optional: &["app-x"], ..Policy::new(&table[..1]) },
            Policy { testkit_normal_users: &["app-x"], ..Policy::new(&table[..1]) },
            Policy { wasm_extra: &["app-x"], ..Policy::new(&table[..1]) },
            Policy { format_allowed: &[("app-doc", &["app-x"])], ..Policy::new(&table[..1]) },
        ] {
            assert!(policy_err(&policy, &[c("app-doc", &[])]).contains("app-x"), "{policy:?}");
        }
        assert!(policy_err(&Policy { ui_crates: &["*"], ..Policy::new(&table[..1]) }, &[c("app-doc", &[])]).contains("ui_crates"));
        assert!(policy_err(&Policy { format_allowed: &[("app", &[])], ..POLICY }, &[c("app", &[])]).contains("exempt"));
    }

    #[test]
    fn allowed_same_layer_cycle_is_error() {
        let flat = Policy { same_layer: SameLayer::Allowed, ..POLICY };
        assert!(check_with(&flat, &[c("app-compose", &[("app-images", Normal, true)])]).is_empty());
        let g = [c("app-compose", &[("app-images", Normal, true)]), c("app-images", &[("app-compose", Dev, true)])];
        let v = check_with(&flat, &g);
        assert!(matches!(&v[..], [Violation::SameLayerCycle { crates }] if crates == &["app-compose", "app-images"]), "{v:?}");
        // Overlapping sideways pairs under the ordered policy form the same kind of cycle.
        let pairs = Policy { sideways: &[("app-compose", "app-images"), ("app-images", "app-compose")], ..POLICY };
        let v = check_with(&pairs, &g);
        assert!(matches!(&v[..], [Violation::SameLayerCycle { .. }]), "{v:?}");
        // So does a same-layer dev exception that closes a loop.
        let excepted = Policy { sideways: &[("app-compose", "app-images")], dev_exceptions: &[("app-images", "app-compose")], ..POLICY };
        let v = check_with(&excepted, &g);
        assert!(matches!(&v[..], [Violation::SameLayerCycle { .. }]), "{v:?}");
    }

    #[test]
    fn large_workspace_is_bounded() {
        // A same-layer chain far deeper than a recursive walk could follow, closed into one cycle.
        const N: usize = 50_000;
        let names: Vec<String> = (0..N).map(|i| format!("p{i:05}")).collect();
        let table: Vec<(&str, Class)> = names.iter().map(|n| (n.as_str(), Class::Layer(0))).collect();
        let crates: Vec<Crate> = (0..N)
            .map(|i| Crate { name: names[i].clone(), deps: vec![Dep { name: names[(i + 1) % N].clone(), kind: Normal, workspace: true }] })
            .collect();
        let v = check_with(&Policy { same_layer: SameLayer::Allowed, ..Policy::new(&table) }, &crates);
        assert!(matches!(&v[..], [Violation::SameLayerCycle { crates }] if crates.len() == N));
    }

    #[test]
    fn build_dependency_is_checked() {
        let v = check(&[c("app-doc", &[("app-engine", Build, true)])]);
        assert!(matches!(v[..], [Violation::Upward { kind: Build, .. }]), "{v:?}");
        let v = check(&[c("app-engine", &[("winit", Build, false)])]);
        assert!(matches!(v[..], [Violation::UiBelowMinLayer { kind: Build, .. }]), "{v:?}");
    }

    #[test]
    fn testkit_normal_only_for_named_tooling() {
        let policy = Policy { testkit_crates: &["craft-testkit"], testkit_normal_users: &["xtask"], build_tool_crates: &["craft-xtask"], ..POLICY };
        assert!(
            check_with(&policy, &[c("xtask", &[("craft-testkit", Normal, false), ("app-testkit", Normal, true), ("craft-xtask", Normal, false)])])
                .is_empty()
        );
        let v = check_with(&policy, &[c("app-engine", &[("craft-testkit", Normal, false)])]);
        assert!(matches!(v[..], [Violation::TestkitAsNormalDep { .. }]), "{v:?}");
        assert!(check_with(&policy, &[c("app-engine", &[("craft-testkit", Dev, false)])]).is_empty());
        // The build-tool library is for build tooling only, whatever the kind or the class.
        for user in ["app-engine", "app", "app-testkit"] {
            for kind in [Normal, Dev, Build] {
                let v = check_with(&policy, &[c(user, &[("craft-xtask", kind, false)])]);
                assert!(matches!(v[..], [Violation::BuildToolDep { .. }]), "{user} {kind:?}: {v:?}");
            }
        }
        let v = check_with(&policy, &[c("app", &[("xtask", Normal, true)])]);
        assert!(matches!(v[..], [Violation::BuildToolDep { .. }]), "{v:?}");
    }

    #[test]
    fn dev_exception_allows_only_the_named_dev_edge() {
        let policy = Policy { dev_exceptions: &[("app-render", "app-engine")], ..POLICY };
        assert!(check_with(&policy, &[c("app-render", &[("app-engine", Dev, true)])]).is_empty());
        // Not as a normal or build dependency, and not for another pair.
        for (from, kind) in [("app-render", Normal), ("app-render", Build), ("app-doc", Dev)] {
            let v = check_with(&policy, &[c(from, &[("app-engine", kind, true)])]);
            assert!(matches!(v[..], [Violation::Upward { .. }]), "{from} {kind:?}: {v:?}");
        }
    }

    #[test]
    fn layered_crate_may_not_use_an_exempt_package() {
        let v = check(&[c("app-ui-egui", &[("app-cli", Normal, true)])]);
        assert!(matches!(v[..], [Violation::OnExempt { .. }]), "{v:?}");
    }

    #[test]
    fn wasm_set_is_sorted_and_unique() {
        let policy = Policy { wasm_max_layer: Some(5), wasm_extra: &["app-web", "app-ui-egui"], ..POLICY };
        let g = [
            c("app-web", &[]),
            c("app-engine", &[]),
            c("app-geom", &[]),
            c("app-ui-egui", &[]),
            c("app-mcp", &[]),
            c("app-testkit", &[]),
            c("xtask", &[]),
            c("app", &[]),
        ];
        assert_eq!(wasm_set(&policy, &g), Ok(["app-engine", "app-geom", "app-ui-egui", "app-web"].map(String::from).to_vec()));
        // No layer and no extra the workspace has selects nothing, which is an error.
        assert!(matches!(wasm_set(&POLICY, &g), Err(Error::Policy(_))));
        let tooling = Policy { wasm_extra: &["app-testkit"], ..POLICY };
        assert!(matches!(wasm_set(&tooling, &g), Err(Error::Policy(m)) if m.contains("tooling")));
    }

    #[test]
    fn oversized_metadata_is_error() {
        let _guard = crate::spawn_guard();
        let exe = std::env::current_exe().expect("test executable");
        let mut cmd = Command::new(exe);
        cmd.arg("--list");
        assert!(matches!(read_json(cmd, "child", 8),Err(Error::Process(m)) if m == "child: wrote more than 8 bytes"));
    }

    #[test]
    fn non_json_metadata_is_error() {
        let _guard = crate::spawn_guard();
        let exe = std::env::current_exe().expect("test executable");
        let mut cmd = Command::new(exe);
        cmd.arg("--list");
        assert!(matches!(read_json(cmd, "child", DEFAULT_METADATA_LIMIT), Err(Error::Metadata(m)) if m.starts_with("child: the output is not JSON")));
    }

    #[test]
    fn metadata_exit_failure_is_error() {
        let _guard = crate::spawn_guard();
        let exe = std::env::current_exe().expect("test executable");
        let mut cmd = Command::new(exe);
        cmd.arg("--craft-xtask-unknown-flag");
        assert!(matches!(read_json(cmd, "child", DEFAULT_METADATA_LIMIT), Err(Error::Process(m)) if m.starts_with("child: exited with ")));
        let missing = std::env::temp_dir().join("craft-xtask-no-such-program");
        assert!(matches!(read_json(Command::new(missing), "missing", 1), Err(Error::Process(m)) if m.starts_with("missing: failed to spawn: ")));
    }

    const PIPED_STDERR_READER: &str = "CRAFT_XTASK_PIPED_STDERR_READER";
    const STDERR_FLOOD_CHILD: &str = "CRAFT_XTASK_STDERR_FLOOD_CHILD";

    /// This test binary, run as a child that runs only the ignored test `name`.
    fn test_child(name: &str, env: &str) -> Command {
        let exe = std::env::current_exe().expect("test executable");
        let mut c = Command::new(exe);
        c.args(["--ignored", "--exact", name, "--nocapture"]).env(env, "1").stdin(Stdio::null());
        c
    }

    #[test]
    fn read_json_inherits_stderr_when_the_caller_pipes_it() {
        let _guard = crate::spawn_guard();
        // The reader runs in a child whose stderr is null, so the flood it inherits is discarded.
        let mut reader = test_child("layers::tests::piped_stderr_reader", PIPED_STDERR_READER);
        reader.stdout(Stdio::null()).stderr(Stdio::null());
        assert!(reader.status().expect("reader child").success(), "read_json did not return for a child that floods a piped stderr");
    }

    /// Runs `read_json` on a child that floods its stderr, with stderr set to a pipe that nobody
    /// reads; run by `read_json_inherits_stderr_when_the_caller_pipes_it`.
    #[test]
    #[ignore = "child process of read_json_inherits_stderr_when_the_caller_pipes_it"]
    fn piped_stderr_reader() {
        if std::env::var_os(PIPED_STDERR_READER).is_some() {
            let mut cmd = test_child("layers::tests::stderr_flood_child", STDERR_FLOOD_CHILD);
            cmd.stderr(Stdio::piped());
            // The child's stdout is the test harness's report, which is not JSON.
            assert!(
                matches!(read_json(cmd, "flood", DEFAULT_METADATA_LIMIT), Err(Error::Metadata(m)) if m.starts_with("flood: the output is not JSON"))
            );
        }
    }

    /// Writes 4 MiB to stderr, more than a pipe holds; run by `piped_stderr_reader`.
    #[test]
    #[ignore = "child process of piped_stderr_reader"]
    fn stderr_flood_child() {
        if std::env::var_os(STDERR_FLOOD_CHILD).is_some() {
            let chunk = [b'.'; 64 * 1024];
            let mut err = std::io::stderr().lock();
            for _ in 0..64 {
                std::io::Write::write_all(&mut err, &chunk).expect("stderr");
            }
        }
    }

    /// Generic moves of PdfCraft's tests: numbered standalone crates, listed sideways pairs, UI
    /// dev-dependencies and test support at the UI layer.
    mod pdfcraft {
        use super::super::DepKind::{Dev, Normal};
        use super::super::{Class, Crate, Policy, Violation};
        use super::{c, check_with};

        const TABLE: &[(&str, Class)] = &[
            ("p-geom", Class::Standalone(0)),
            ("p-filters", Class::Standalone(0)),
            ("p-extra", Class::Standalone(0)),
            ("p-cos", Class::Standalone(1)),
            ("p-model", Class::Layer(2)),
            ("p-forms", Class::Layer(3)),
            ("p-js", Class::Layer(3)),
            ("p-edit", Class::Layer(4)),
            ("p-redact", Class::Layer(4)),
            ("p-engine", Class::Layer(6)),
            ("p-ui", Class::Layer(7)),
            ("p-testkit", Class::Testkit),
            ("p-oracle", Class::Testkit),
            ("p", Class::Exempt),
            ("xtask", Class::BuildTool),
        ];
        const ALL: &[&str] = &[
            "p-geom",
            "p-filters",
            "p-extra",
            "p-cos",
            "p-model",
            "p-forms",
            "p-js",
            "p-edit",
            "p-redact",
            "p-engine",
            "p-ui",
            "p-testkit",
            "p-oracle",
            "p",
            "xtask",
        ];
        const POLICY: Policy<'static> = Policy {
            optional: ALL,
            sideways: &[("p-redact", "p-edit")],
            standalone_deps: &[("p-cos", &["p-filters", "p-geom"])],
            ui_crates: &["egui", "winit", "egui_kittest", "wgpu*"],
            ui_min_layer: 7,
            ui_dev_allowed: &["egui_kittest"],
            testkit_layer: 7,
            ..Policy::new(TABLE)
        };

        fn check(crates: &[Crate]) -> Vec<Violation> {
            check_with(&POLICY, crates)
        }

        #[test]
        fn upward_and_sideways_edges_flagged() {
            let v = check(&[c("p-model", &[("p-engine", Normal, true)])]);
            assert!(matches!(v[..], [Violation::Upward { from: 2, to: 6, .. }]), "{v:?}");
            let v = check(&[c("p-forms", &[("p-js", Normal, true)])]);
            assert!(matches!(v[..], [Violation::Upward { from: 3, to: 3, .. }]), "{v:?}");
        }

        #[test]
        fn listed_sideways_edges_allowed() {
            assert!(check(&[c("p-redact", &[("p-edit", Normal, true)])]).is_empty());
            assert!(!check(&[c("p-edit", &[("p-redact", Normal, true)])]).is_empty());
        }

        #[test]
        fn cos_may_use_only_its_listed_foundation() {
            assert!(check(&[c("p-cos", &[("p-filters", Normal, true), ("p-geom", Dev, true)])]).is_empty());
            let v = check(&[c("p-cos", &[("p-extra", Normal, true)])]);
            assert!(matches!(v[..], [Violation::StandaloneHasWorkspaceDep { .. }]), "{v:?}");
            let v = check(&[c("p-filters", &[("p-geom", Normal, true)])]);
            assert!(matches!(v[..], [Violation::StandaloneHasWorkspaceDep { .. }]), "{v:?}");
            // A standalone crate is usable from higher layers.
            assert!(check(&[c("p-model", &[("p-cos", Normal, true)])]).is_empty());
        }

        #[test]
        fn ui_crates_below_l7_flagged() {
            for dep in ["egui", "winit", "wgpu", "wgpu-core"] {
                let v = check(&[c("p-engine", &[(dep, Normal, false)])]);
                assert!(matches!(v[..], [Violation::UiBelowMinLayer { layer: 6, min: 7, .. }]), "{dep}: {v:?}");
            }
            assert!(check(&[c("p-ui", &[("winit", Normal, false)])]).is_empty());
            assert!(check(&[c("p-engine", &[("egui_kittest", Dev, false)])]).is_empty());
            assert!(!check(&[c("p-engine", &[("egui_kittest", Normal, false)])]).is_empty());
            assert!(check(&[c("p-testkit", &[("egui", Normal, false)])]).is_empty());
        }

        #[test]
        fn test_support_only_as_dev_dependency() {
            assert!(!check(&[c("p-cos", &[("p-testkit", Normal, true)])]).is_empty());
            assert!(check(&[c("p-model", &[("p-oracle", Dev, true)])]).is_empty());
            let v = check(&[c("p-model", &[("p-oracle", Normal, true)])]);
            assert!(matches!(v[..], [Violation::TestkitAsNormalDep { .. }]), "{v:?}");
            // A standalone crate may use test support in its own tests, and no other workspace
            // crate outside its list however it is depended on.
            assert!(check(&[c("p-cos", &[("p-oracle", Dev, true)])]).is_empty());
            assert!(check(&[c("p-geom", &[("p-testkit", Dev, true)])]).is_empty());
            let v = check(&[c("p-cos", &[("p-model", Dev, true)])]);
            assert!(matches!(v[..], [Violation::StandaloneHasWorkspaceDep { .. }]), "{v:?}");
            // Test support may use other test support.
            assert!(check(&[c("p-oracle", &[("p-testkit", Normal, true)])]).is_empty());
        }

        #[test]
        fn unregistered_crate_is_error() {
            let v = check(&[c("p-mystery", &[])]);
            assert!(matches!(&v[..], [Violation::Unregistered { krate }] if krate == "p-mystery"), "{v:?}");
        }

        #[test]
        fn apps_exempt() {
            for app in ["p", "xtask"] {
                assert!(check(&[c(app, &[("egui", Normal, false), ("p-ui", Normal, true)])]).is_empty(), "{app}");
            }
        }
    }

    /// Generic moves of PhotoCraft's standalone tests.
    mod photocraft {
        use super::super::DepKind::{Dev, Normal};
        use super::super::{Class, Crate, Policy, Violation};
        use super::{c, check_with};

        const TABLE: &[(&str, Class)] = &[
            ("h-geom", Class::Layer(0)),
            ("h-io", Class::Layer(1)),
            ("h-psd", Class::Standalone(0)),
            ("h-codecs", Class::Standalone(0)),
            ("h-heif", Class::Standalone(0)),
        ];
        const POLICY: Policy<'static> = Policy {
            optional: &["h-geom", "h-io", "h-psd", "h-codecs", "h-heif"],
            standalone_deps: &[("h-codecs", &["h-heif"])],
            ..Policy::new(TABLE)
        };

        fn check(crates: &[Crate]) -> Vec<Violation> {
            check_with(&POLICY, crates)
        }

        #[test]
        fn standalone_crates_have_no_workspace_deps() {
            for s in ["h-psd", "h-codecs", "h-heif"] {
                let v = check(&[c(s, &[("h-geom", Normal, true)])]);
                assert!(matches!(v[..], [Violation::StandaloneHasWorkspaceDep { .. }]), "{s}: {v:?}");
                assert!(check(&[c(s, &[("image", Normal, false)])]).is_empty());
            }
        }

        #[test]
        fn standalone_exception_is_exactly_codecs_to_heif() {
            assert!(check(&[c("h-codecs", &[("h-heif", Normal, true)])]).is_empty());
            assert!(check(&[c("h-codecs", &[("h-heif", Dev, true)])]).is_empty());
            for (from, to) in [("h-heif", "h-codecs"), ("h-psd", "h-heif"), ("h-heif", "h-geom")] {
                let v = check(&[c(from, &[(to, Normal, true)])]);
                assert!(matches!(v[..], [Violation::StandaloneHasWorkspaceDep { .. }]), "{from} -> {to}: {v:?}");
            }
            // The exception does not open the standalone crate to layered crates.
            let v = check(&[c("h-codecs", &[("h-geom", Normal, true)])]);
            assert!(matches!(v[..], [Violation::StandaloneHasWorkspaceDep { .. }]), "{v:?}");
            // A standalone dependency list may only name standalone crates.
            let bad = Policy { standalone_deps: &[("h-codecs", &["h-geom"])], ..POLICY };
            assert!(super::super::check(&bad, &[c("h-codecs", &[])]).is_err());
        }

        #[test]
        fn format_allowlist_applies_to_standalone_crates() {
            let limited = Policy { format_allowed: &[("h-codecs", &[])], ..POLICY };
            let v = check_with(&limited, &[c("h-codecs", &[("h-heif", Normal, true)])]);
            assert!(matches!(&v[..], [Violation::FormatDep { dep, .. }] if dep == "h-heif"), "{v:?}");
            assert!(check_with(&limited, &[c("h-codecs", &[("h-heif", Dev, true)])]).is_empty());
        }

        #[test]
        fn format_allowlist_applies_to_allowed_test_tooling() {
            const TABLE: &[(&str, Class)] = &[("t-format", Class::Layer(1)), ("t-testkit", Class::Testkit)];
            let policy = Policy { testkit_normal_users: &["t-format"], format_allowed: &[("t-format", &[])], ..Policy::new(TABLE) };
            let v = check_with(&policy, &[c("t-format", &[("t-testkit", Normal, true)]), c("t-testkit", &[])]);
            assert!(matches!(&v[..], [Violation::FormatDep { dep, .. }] if dep == "t-testkit"), "{v:?}");
        }

        #[test]
        fn standalone_is_usable_from_higher_layers() {
            assert!(check(&[c("h-io", &[("h-psd", Normal, true)])]).is_empty());
            let v = check(&[c("h-geom", &[("h-codecs", Normal, true)])]);
            assert!(matches!(v[..], [Violation::Upward { from: 0, to: 0, .. }]), "{v:?}");
        }
    }

    /// Generic moves of GridCraft's chain and format tests.
    mod gridcraft {
        use super::super::DepKind::{Dev, Normal};
        use super::super::{Class, Crate, Policy, Violation};
        use super::{c, check_with};

        const TABLE: &[(&str, Class)] = &[
            ("g-core", Class::Layer(0)),
            ("g-numfmt", Class::Layer(0)),
            ("g-model", Class::Layer(1)),
            ("g-formula", Class::Layer(1)),
            ("g-calc", Class::Layer(1)),
            ("g-xlsx", Class::Layer(2)),
        ];
        const POLICY: Policy<'static> = Policy {
            optional: &["g-core", "g-numfmt", "g-model", "g-formula", "g-calc", "g-xlsx"],
            ordered_chains: &[&["g-core", "g-numfmt"], &["g-model", "g-calc"]],
            format_allowed: &[("g-xlsx", &["g-core", "g-formula", "g-model"])],
            ..Policy::new(TABLE)
        };

        fn check(crates: &[Crate]) -> Vec<Violation> {
            check_with(&POLICY, crates)
        }

        #[test]
        fn intra_layer_chains_allowed_one_way() {
            assert!(check(&[c("g-numfmt", &[("g-core", Normal, true)])]).is_empty());
            let v = check(&[c("g-core", &[("g-numfmt", Normal, true)])]);
            assert!(matches!(v[..], [Violation::Upward { from: 0, to: 0, .. }]), "{v:?}");
            assert!(check(&[c("g-calc", &[("g-model", Normal, true)])]).is_empty());
            assert!(!check(&[c("g-model", &[("g-calc", Normal, true)])]).is_empty());
            assert!(!check(&[c("g-calc", &[("g-xlsx", Normal, true)])]).is_empty());
        }

        #[test]
        fn format_crate_limited_to_core_formula_model() {
            let v = check(&[c("g-xlsx", &[("g-numfmt", Normal, true)])]);
            assert!(matches!(&v[..], [Violation::FormatDep { dep, .. }] if dep == "g-numfmt"), "{v:?}");
            assert!(check(&[c("g-xlsx", &[("g-numfmt", Dev, true)])]).is_empty());
            assert!(check(&[c("g-xlsx", &[("g-model", Normal, true)])]).is_empty());
        }
    }
}
