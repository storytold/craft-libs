//! Pinned dependencies: the version, source and features of named packages in `cargo metadata`, and
//! packages an app forbids.
//!
//! The app passes a [`Policy`]: one [`Pin`] for each package it pins and one [`Forbidden`] rule for
//! each package (or family of packages) it keeps out. [`run`] runs
//! `cargo metadata --locked --format-version 1` with the app's `cargo` command and passes the JSON to
//! [`check`]:
//!
//! - Each pinned name has exactly one package in `packages`, with the pinned `version` and `source`.
//!   The source is compared as text: `git_source(url, rev)` gives the form Cargo writes for a package
//!   taken from a git repository at a full commit hash, and [`CRATES_IO`] the form for crates.io.
//! - [`FeatureRule::Contains`] and [`FeatureRule::Exactly`] read the features enabled on the package
//!   from its node in `resolve.nodes`. `Contains` allows features beyond the listed ones, and
//!   `Exactly` does not.
//! - [`Scope::Active`] rejects a forbidden package that has a node in `resolve.nodes`, development
//!   dependencies included. [`Scope::ProductionFrom`] rejects a forbidden package reached from a root
//!   package over normal and build edges; a package reached only through development edges is
//!   allowed.
//! - Packages the policy does not name are allowed. An empty policy accepts any metadata.
//!
//! A rule that needs `resolve.nodes`, a package `id` or a node's `features` or `deps` returns an
//! [`Error::Output`] when the field is missing, and so does JSON that does not parse.
//!
//! Cargo merges the features of every target into one metadata result. To check the features one
//! target enables, [`run_target_features`] reads `cargo tree` for that target and
//! [`check_target_features`] compares the features of each line for the package; [`host_target`]
//! reads the host target from `rustc -vV`.
//!
//! Every command's output is read within a [`Limits`] size: more standard output than the limit stops
//! the command and returns [`Error::TooLarge`]. Standard error is read to its end while the command
//! runs, and its first [`Limits::stderr_bytes`] bytes are kept for the error message.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;

use serde_json::Value;

/// The `source` Cargo writes for a package from crates.io.
pub const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// The `source` Cargo writes for a package from the git repository at `url`, pinned with
/// `rev = "<rev>"` where `rev` is a full commit hash: `git+<url>?rev=<rev>#<rev>`.
pub fn git_source(url: &str, rev: &str) -> String {
    format!("git+{url}?rev={rev}#{rev}")
}

/// One pinned package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin<'a> {
    /// The package name.
    pub name: &'a str,
    /// The exact `version`.
    pub version: &'a str,
    /// The exact `source`, for example from [`git_source`] or [`CRATES_IO`].
    pub source: &'a str,
    /// Which enabled features are allowed.
    pub features: FeatureRule<'a>,
}

/// How a [`Pin`] checks the package's enabled features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureRule<'a> {
    /// The features are not checked.
    Ignore,
    /// Each listed feature is enabled; others may be enabled too.
    Contains(&'a [&'a str]),
    /// The listed features, and no others, are enabled.
    Exactly(&'a [&'a str]),
}

/// Which package names a [`Forbidden`] rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Name<'a> {
    /// This name only.
    Exact(&'a str),
    /// This name and every name that starts with it followed by `-`: `Family("hayro")` matches
    /// `hayro` and `hayro-syntax`.
    Family(&'a str),
}

impl Name<'_> {
    /// Whether `name` matches.
    pub fn matches(&self, name: &str) -> bool {
        match *self {
            Name::Exact(n) => name == n,
            Name::Family(n) => name.strip_prefix(n).is_some_and(|rest| rest.is_empty() || rest.starts_with('-')),
        }
    }
}

impl fmt::Display for Name<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Name::Exact(n) => f.write_str(n),
            Name::Family(n) => write!(f, "{n} or {n}-*"),
        }
    }
}

/// Where a [`Forbidden`] rule looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope<'a> {
    /// Every package with a node in `resolve.nodes`, development dependencies included.
    Active,
    /// The packages reached from the named root package over normal and build edges.
    ProductionFrom(&'a str),
}

/// A package, or family of packages, that must not be in the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Forbidden<'a> {
    /// The names the rule matches.
    pub name: Name<'a>,
    /// Where it looks.
    pub scope: Scope<'a>,
}

/// An app's pins and forbidden packages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Policy<'a> {
    /// The pinned packages.
    pub pins: &'a [Pin<'a>],
    /// The forbidden packages.
    pub forbidden: &'a [Forbidden<'a>],
}

/// Size limits for the commands' output and the JSON [`check`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of `cargo metadata` JSON, checked before it is parsed. Default 64 MiB.
    pub metadata_bytes: usize,
    /// Bytes of output from one `cargo tree`. Default 64 MiB.
    pub tree_bytes: usize,
    /// Bytes of output from `rustc -vV`. Default 64 KiB.
    pub version_bytes: usize,
    /// Bytes of a command's standard error kept for its error message; the rest is read and dropped.
    /// Default 64 KiB.
    pub stderr_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { metadata_bytes: 64 << 20, tree_bytes: 64 << 20, version_bytes: 64 << 10, stderr_bytes: 64 << 10 }
    }
}

/// A package that breaks the [`Policy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// A pinned name, or a root, has this many packages instead of one.
    Count {
        /// The package name.
        name: String,
        /// The number of packages with that name.
        found: usize,
    },
    /// A pinned package has another version.
    Version {
        /// The package name.
        name: String,
        /// The pinned version.
        expected: String,
        /// The version in the metadata.
        found: String,
    },
    /// A pinned package comes from another source. `found` is `None` for a path package.
    Source {
        /// The package name.
        name: String,
        /// The pinned source.
        expected: String,
        /// The source in the metadata.
        found: Option<String>,
    },
    /// A package lacks required features or has features the rule does not allow.
    Features {
        /// The package name, or the `cargo tree` package text.
        name: String,
        /// Required features that are not enabled.
        missing: Vec<String>,
        /// Enabled features that are not allowed.
        extra: Vec<String>,
    },
    /// A forbidden package is in the graph.
    Forbidden {
        /// The package name.
        name: String,
        /// The rule's names.
        rule: String,
        /// The root it was reached from, for [`Scope::ProductionFrom`].
        root: Option<String>,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::Count { name, found } => write!(f, "{name}: expected one resolved package, found {found}"),
            Violation::Version { name, expected, found } => write!(f, "{name}: expected version {expected}, found {found}"),
            Violation::Source { name, expected, found } => {
                write!(f, "{name}: expected source {expected}, found {}", found.as_deref().unwrap_or("a path package"))
            }
            Violation::Features { name, missing, extra } => {
                write!(f, "{name}: ")?;
                if !missing.is_empty() {
                    write!(f, "missing features {}", missing.join(", "))?;
                }
                if !missing.is_empty() && !extra.is_empty() {
                    f.write_str("; ")?;
                }
                if !extra.is_empty() {
                    write!(f, "features not allowed {}", extra.join(", "))?;
                }
                Ok(())
            }
            Violation::Forbidden { name, rule, root: None } => write!(f, "{name}: forbidden ({rule}) and resolved"),
            Violation::Forbidden { name, rule, root: Some(root) } => {
                write!(f, "{name}: forbidden ({rule}) and reached from {root} over normal or build dependencies")
            }
        }
    }
}

/// Why a check stopped.
#[derive(Debug)]
pub enum Error {
    /// A command could not start.
    Start {
        /// The command, for example `cargo metadata`.
        command: String,
        /// The error starting it.
        source: io::Error,
    },
    /// A command's output could not be read, or the command could not be waited for.
    Read {
        /// The command.
        command: String,
        /// The error reading it.
        source: io::Error,
    },
    /// A command exited without success.
    Exit {
        /// The command.
        command: String,
        /// Its exit status.
        status: ExitStatus,
        /// The start of its standard error, at most [`Limits::stderr_bytes`].
        stderr: String,
    },
    /// A command printed more than its limit, or the JSON given to [`check`] is larger than
    /// [`Limits::metadata_bytes`].
    TooLarge {
        /// The command.
        command: String,
        /// The limit in bytes.
        limit: usize,
    },
    /// A command's output is not what was expected: JSON that does not parse, or lacks a field a
    /// rule needs.
    Output {
        /// The command.
        command: String,
        /// What is wrong.
        why: String,
    },
    /// The packages break the policy.
    Violation(Violation),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Start { command, source } => write!(f, "{command}: could not start: {source}"),
            Error::Read { command, source } => write!(f, "{command}: {source}"),
            Error::Exit { command, status, stderr } if stderr.is_empty() => write!(f, "{command}: exited with {status}"),
            Error::Exit { command, status, stderr } => write!(f, "{command}: exited with {status}: {stderr}"),
            Error::TooLarge { command, limit } => write!(f, "{command}: more than the limit of {limit} bytes of output"),
            Error::Output { command, why } => write!(f, "{command}: {why}"),
            Error::Violation(v) => v.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Start { source, .. } | Error::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<Violation> for Error {
    fn from(v: Violation) -> Self {
        Error::Violation(v)
    }
}

const METADATA: &str = "cargo metadata";

/// Runs `cargo metadata --locked --format-version 1` and checks its JSON against `policy`.
///
/// `cargo` builds the app's `cargo` command (program, folder and environment); the arguments are added
/// to it.
pub fn run(cargo: impl FnOnce() -> Command, policy: &Policy<'_>, limits: &Limits) -> Result<(), Error> {
    let json = metadata(cargo, limits)?;
    check(&json, policy, limits)
}

/// The output of `cargo metadata --locked --format-version 1`, read within
/// [`Limits::metadata_bytes`].
pub fn metadata(cargo: impl FnOnce() -> Command, limits: &Limits) -> Result<Vec<u8>, Error> {
    let mut cmd = cargo();
    cmd.args(["metadata", "--locked", "--format-version", "1"]);
    output(cmd, METADATA, limits.metadata_bytes, limits.stderr_bytes)
}

/// Checks `cargo metadata --format-version 1` JSON against `policy`: the pins first, in order, then
/// the forbidden rules. The first broken rule is returned.
pub fn check(metadata_json: &[u8], policy: &Policy<'_>, limits: &Limits) -> Result<(), Error> {
    if metadata_json.len() > limits.metadata_bytes {
        return Err(Error::TooLarge { command: METADATA.into(), limit: limits.metadata_bytes });
    }
    let meta: Value = serde_json::from_slice(metadata_json).map_err(|e| invalid(format!("not JSON: {e}")))?;
    let graph = Graph::new(&meta)?;
    for pin in policy.pins {
        graph.check_pin(pin)?;
    }
    for rule in policy.forbidden {
        match rule.scope {
            Scope::Active => graph.check_active(&rule.name)?,
            Scope::ProductionFrom(root) => graph.check_production(&rule.name, root)?,
        }
    }
    Ok(())
}

/// The packages and resolved nodes of one metadata result.
struct Graph<'m> {
    packages: &'m [Value],
    by_id: HashMap<&'m str, &'m Value>,
    /// `None` when the JSON has no `resolve.nodes` (`--no-deps`).
    nodes: Option<HashMap<&'m str, &'m Value>>,
}

impl<'m> Graph<'m> {
    fn new(meta: &'m Value) -> Result<Self, Error> {
        let packages = meta.get("packages").and_then(Value::as_array).ok_or_else(|| invalid("packages is missing"))?;
        let mut by_id = HashMap::new();
        for p in packages {
            str_field(p, "name", "a package")?;
            if let Some(id) = p.get("id").and_then(Value::as_str)
                && by_id.insert(id, p).is_some()
            {
                return Err(invalid(format!("package {id} is listed twice")));
            }
        }
        let nodes = match meta.pointer("/resolve/nodes").and_then(Value::as_array) {
            None => None,
            Some(list) => {
                let mut nodes = HashMap::new();
                for n in list {
                    let id = str_field(n, "id", "a resolved node")?;
                    if nodes.insert(id, n).is_some() {
                        return Err(invalid(format!("package {id} has two resolved nodes")));
                    }
                }
                Some(nodes)
            }
        };
        Ok(Graph { packages, by_id, nodes })
    }

    /// The packages named `name`.
    fn named(&self, name: &str) -> Vec<&'m Value> {
        self.packages.iter().filter(|p| p.get("name").and_then(Value::as_str) == Some(name)).collect()
    }

    /// The one package named `name`.
    fn one(&self, name: &str) -> Result<&'m Value, Error> {
        match self.named(name).as_slice() {
            [package] => Ok(package),
            other => Err(Violation::Count { name: name.into(), found: other.len() }.into()),
        }
    }

    fn node(&self, id: &str) -> Result<&'m Value, Error> {
        let nodes = self.nodes.as_ref().ok_or_else(|| invalid("resolve.nodes is missing; run cargo metadata without --no-deps"))?;
        nodes.get(id).copied().ok_or_else(|| invalid(format!("package {id} has no resolved node")))
    }

    fn check_pin(&self, pin: &Pin<'_>) -> Result<(), Error> {
        let package = self.one(pin.name)?;
        let version = str_field(package, "version", pin.name)?;
        if version != pin.version {
            return Err(Violation::Version { name: pin.name.into(), expected: pin.version.into(), found: version.into() }.into());
        }
        let source = package.get("source").and_then(Value::as_str);
        if source != Some(pin.source) {
            return Err(Violation::Source { name: pin.name.into(), expected: pin.source.into(), found: source.map(str::to_owned) }.into());
        }
        let (wanted, exact) = match pin.features {
            FeatureRule::Ignore => return Ok(()),
            FeatureRule::Contains(f) => (f, false),
            FeatureRule::Exactly(f) => (f, true),
        };
        let node = self.node(str_field(package, "id", pin.name)?)?;
        let list =
            node.get("features").and_then(Value::as_array).ok_or_else(|| invalid(format!("{}: the resolved node has no features", pin.name)))?;
        let mut enabled = BTreeSet::new();
        for f in list {
            enabled.insert(f.as_str().ok_or_else(|| invalid(format!("{}: a feature is not a string", pin.name)))?);
        }
        compare_features(pin.name, wanted, &enabled, exact)
    }

    fn check_active(&self, name: &Name<'_>) -> Result<(), Error> {
        let nodes = self.nodes.as_ref().ok_or_else(|| invalid("resolve.nodes is missing; run cargo metadata without --no-deps"))?;
        for package in self.packages {
            let package_name = str_field(package, "name", "a package")?;
            if !name.matches(package_name) {
                continue;
            }
            let id = str_field(package, "id", package_name)?;
            if nodes.contains_key(id) {
                return Err(Violation::Forbidden { name: package_name.into(), rule: name.to_string(), root: None }.into());
            }
        }
        Ok(())
    }

    fn check_production(&self, name: &Name<'_>, root: &str) -> Result<(), Error> {
        let root_id = str_field(self.one(root)?, "id", root)?;
        let mut pending = vec![root_id];
        let mut seen = HashSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let package = self.by_id.get(id).ok_or_else(|| invalid(format!("dependency {id} has no package entry")))?;
            let package_name = str_field(package, "name", id)?;
            if name.matches(package_name) {
                return Err(Violation::Forbidden { name: package_name.into(), rule: name.to_string(), root: Some(root.into()) }.into());
            }
            let deps =
                self.node(id)?.get("deps").and_then(Value::as_array).ok_or_else(|| invalid(format!("package {id} has no dependency edges")))?;
            for dep in deps {
                if production_edge(dep, id)? {
                    pending.push(str_field(dep, "pkg", id)?);
                }
            }
        }
        Ok(())
    }
}

/// Whether a `deps` entry is a normal or build dependency for at least one target. A kind that is not
/// `dev` counts as production, so an unknown kind is followed.
fn production_edge(dep: &Value, from: &str) -> Result<bool, Error> {
    let kinds = dep.get("dep_kinds").and_then(Value::as_array).ok_or_else(|| invalid(format!("a dependency of {from} has no dep_kinds")))?;
    if kinds.is_empty() {
        return Err(invalid(format!("a dependency of {from} has no dep_kinds")));
    }
    let mut production = false;
    for k in kinds {
        match k.get("kind") {
            Some(Value::Null) => production = true,
            Some(Value::String(kind)) => production |= kind != "dev",
            _ => return Err(invalid(format!("a dependency of {from} has a dep_kinds entry without a kind"))),
        }
    }
    Ok(production)
}

fn compare_features(name: &str, wanted: &[&str], enabled: &BTreeSet<&str>, exact: bool) -> Result<(), Error> {
    let wanted: BTreeSet<&str> = wanted.iter().copied().collect();
    let missing: Vec<String> = wanted.difference(enabled).map(|f| (*f).to_owned()).collect();
    let extra: Vec<String> = if exact { enabled.difference(&wanted).map(|f| (*f).to_owned()).collect() } else { Vec::new() };
    if missing.is_empty() && extra.is_empty() { Ok(()) } else { Err(Violation::Features { name: name.into(), missing, extra }.into()) }
}

fn str_field<'v>(v: &'v Value, key: &str, of: &str) -> Result<&'v str, Error> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| invalid(format!("{of}: {key} is missing or not a string")))
}

fn invalid(why: impl Into<String>) -> Error {
    Error::Output { command: METADATA.into(), why: why.into() }
}

/// The `cargo tree` call for [`run_target_features`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetFeatures<'a> {
    /// The workspace package the tree starts from (`--package`).
    pub root: &'a str,
    /// Levels of dependencies below the root (`--depth`).
    pub depth: u32,
    /// The package whose features are checked.
    pub package: &'a str,
    /// The features it must have, and no others, on this target.
    pub features: &'a [&'a str],
}

/// Runs `cargo tree` for one target and checks the features of `spec.package` with
/// [`check_target_features`]. The command is
/// `cargo tree --locked --target <target> --edges normal,build --package <root> --depth <depth>
/// --no-dedupe --prefix none --format {p}|{f}`, and its output is read within
/// [`Limits::tree_bytes`].
pub fn run_target_features(cargo: impl FnOnce() -> Command, target: &str, spec: &TargetFeatures<'_>, limits: &Limits) -> Result<(), Error> {
    let command = format!("cargo tree --target {target}");
    let mut cmd = cargo();
    cmd.args(["tree", "--locked", "--target", target, "--edges", "normal,build", "--package", spec.root, "--depth"])
        .arg(spec.depth.to_string())
        .args(["--no-dedupe", "--prefix", "none", "--format", "{p}|{f}"]);
    let out = output(cmd, &command, limits.tree_bytes, limits.stderr_bytes)?;
    let tree = String::from_utf8(out).map_err(|_| Error::Output { command: command.clone(), why: "not UTF-8".into() })?;
    check_target_features(&tree, spec.package, spec.features).map_err(|e| match e {
        Error::Output { why, .. } => Error::Output { command, why },
        other => other,
    })
}

/// Checks `cargo tree --format {p}|{f}` output: every line for `package` (text that starts with
/// `<package> v`) enables exactly `features`, and at least one line is for `package`.
pub fn check_target_features(tree: &str, package: &str, features: &[&str]) -> Result<(), Error> {
    let prefix = format!("{package} v");
    let mut found = 0usize;
    for line in tree.lines() {
        let Some((text, list)) = line.rsplit_once('|') else {
            continue;
        };
        if !text.starts_with(&prefix) {
            continue;
        }
        found += 1;
        let enabled: BTreeSet<&str> = list.split(',').filter(|f| !f.is_empty()).collect();
        compare_features(text, features, &enabled, true)?;
    }
    if found == 0 {
        return Err(Violation::Count { name: package.into(), found: 0 }.into());
    }
    Ok(())
}

/// The host target from `rustc -vV` (its `host:` line), read within [`Limits::version_bytes`].
/// `rustc` builds the app's `rustc` command; `-vV` is added to it.
pub fn host_target(rustc: impl FnOnce() -> Command, limits: &Limits) -> Result<String, Error> {
    const COMMAND: &str = "rustc -vV";
    let mut cmd = rustc();
    cmd.arg("-vV");
    let out = output(cmd, COMMAND, limits.version_bytes, limits.stderr_bytes)?;
    let text = String::from_utf8(out).map_err(|_| Error::Output { command: COMMAND.into(), why: "not UTF-8".into() })?;
    text.lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|host| host.trim().to_owned())
        .filter(|host| !host.is_empty())
        .ok_or_else(|| Error::Output { command: COMMAND.into(), why: "no host line".into() })
}

/// Runs `cmd` and returns its standard output, or an error when it fails or prints more than
/// `limit` bytes. Standard error is read on another thread to its end, so a command that writes a
/// lot there does not stop on a full pipe; its first `stderr_limit` bytes are kept.
fn output(mut cmd: Command, command: &str, limit: usize, stderr_limit: usize) -> Result<Vec<u8>, Error> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|source| Error::Start { command: command.into(), source })?;
    let stderr = child.stderr.take().map(|pipe| std::thread::spawn(move || read_head(pipe, stderr_limit)));
    let read = match child.stdout.take() {
        Some(stdout) => read_bounded(stdout, limit),
        None => Err(io::Error::other("no output pipe")),
    };
    // On an error the stderr thread is left to end when the pipe closes: a process the child started
    // may hold the pipe open after the child is stopped.
    let stdout = match read {
        Ok(Some(stdout)) => stdout,
        Ok(None) => {
            stop(&mut child);
            return Err(Error::TooLarge { command: command.into(), limit });
        }
        Err(source) => {
            stop(&mut child);
            return Err(Error::Read { command: command.into(), source });
        }
    };
    let status = child.wait().map_err(|source| Error::Read { command: command.into(), source })?;
    if status.success() {
        return Ok(stdout);
    }
    let stderr = stderr.map(join_head).unwrap_or_default();
    Err(Error::Exit { command: command.into(), status, stderr: String::from_utf8_lossy(&stderr).trim().to_owned() })
}

/// Everything `reader` gives, or `None` when that is more than `limit` bytes. The read stops one byte
/// past the limit.
fn read_bounded(reader: impl Read, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    reader.take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1)).read_to_end(&mut buf)?;
    Ok((buf.len() <= limit).then_some(buf))
}

/// Reads `reader` to its end and returns its first `limit` bytes.
fn read_head(mut reader: impl Read, limit: usize) -> Vec<u8> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let room = limit.saturating_sub(head.len());
                head.extend_from_slice(chunk.get(..n.min(room)).unwrap_or_default());
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    head
}

fn join_head(handle: JoinHandle<Vec<u8>>) -> Vec<u8> {
    handle.join().unwrap_or_default()
}

/// Ends a child whose output is no longer read.
fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::{CRATES_IO, Error, FeatureRule, Forbidden, Limits, Name, Pin, Policy, Scope, Violation, check, check_target_features, git_source};
    use serde_json::{Value, json};

    fn bytes(v: &Value) -> Vec<u8> {
        serde_json::to_vec(v).unwrap()
    }

    fn validate(m: &Value, policy: &Policy<'_>) -> Result<(), Error> {
        check(&bytes(m), policy, &Limits::default())
    }

    fn violation(r: Result<(), Error>) -> Violation {
        match r {
            Err(Error::Violation(v)) => v,
            other => panic!("expected a violation, got {other:?}"),
        }
    }

    fn output_error(r: Result<(), Error>) -> String {
        match r {
            Err(Error::Output { why, .. }) => why,
            other => panic!("expected an output error, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_policy_accepts_any_metadata() {
        let policy = Policy::default();
        assert!(validate(&json!({"packages": []}), &policy).is_ok());
        assert!(validate(&json!({"packages": [{"name": "a"}], "resolve": null}), &policy).is_ok());
    }

    #[test]
    fn rejects_json_that_does_not_parse_or_lacks_packages() {
        let policy = Policy::default();
        assert!(output_error(check(b"{not json", &policy, &Limits::default())).starts_with("not JSON"));
        assert!(output_error(validate(&json!({}), &policy)).contains("packages"));
        assert!(output_error(validate(&json!({"packages": [{"id": "x"}]}), &policy)).contains("name"));
    }

    #[test]
    fn rejects_metadata_larger_than_the_limit_before_parsing_it() {
        let json = bytes(&json!({"packages": []}));
        let limits = Limits { metadata_bytes: json.len() - 1, ..Limits::default() };
        assert!(matches!(check(&json, &Policy::default(), &limits), Err(Error::TooLarge { limit, .. }) if limit == json.len() - 1));
        let limits = Limits { metadata_bytes: json.len(), ..Limits::default() };
        assert!(check(&json, &Policy::default(), &limits).is_ok());
    }

    #[test]
    fn family_names_match_the_name_and_its_dash_suffixes_only() {
        let family = Name::Family("hayro");
        assert!(family.matches("hayro"));
        assert!(family.matches("hayro-syntax"));
        assert!(!family.matches("hayrox"));
        assert!(!family.matches("not-hayro"));
        assert!(Name::Exact("hayro").matches("hayro"));
        assert!(!Name::Exact("hayro").matches("hayro-syntax"));
    }

    #[test]
    fn git_sources_name_the_revision_twice() {
        assert_eq!(git_source("https://github.com/o/r", "abc"), "git+https://github.com/o/r?rev=abc#abc");
    }

    #[test]
    fn target_features_without_any_enabled_match_an_empty_list() {
        assert!(check_target_features("krilla v0.8.2|\n", "krilla", &[]).is_ok());
        assert!(check_target_features("krilla v0.8.2|\n", "krilla", &["rayon"]).is_err());
        // Another package whose name starts the same is not checked.
        assert!(check_target_features("krilla v0.8.2|a\nkrilla-svg v0.1.0|b\n", "krilla", &["a"]).is_ok());
    }

    #[test]
    fn a_production_root_must_resolve_once() {
        let m = json!({"packages": [], "resolve": {"nodes": []}});
        let policy = Policy { pins: &[], forbidden: &[Forbidden { name: Name::Exact("x"), scope: Scope::ProductionFrom("app") }] };
        assert_eq!(violation(validate(&m, &policy)), Violation::Count { name: "app".into(), found: 0 });
    }

    #[test]
    fn an_active_rule_needs_resolved_nodes_even_when_no_package_matches() {
        let policy = Policy { pins: &[], forbidden: &[Forbidden { name: Name::Exact("x"), scope: Scope::Active }] };
        assert!(output_error(validate(&json!({"packages": [{"id": "a", "name": "a"}]}), &policy)).contains("resolve.nodes"));
        assert!(validate(&json!({"packages": [{"id": "a", "name": "a"}], "resolve": {"nodes": []}}), &policy).is_ok());
    }

    #[test]
    fn rejects_edges_without_a_kind() {
        let m = json!({
            "packages": [{"id": "app", "name": "app"}, {"id": "dep", "name": "dep"}],
            "resolve": {"nodes": [{"id": "app", "deps": [{"pkg": "dep", "dep_kinds": [{}]}]}, {"id": "dep", "deps": []}]}
        });
        let policy = Policy { pins: &[], forbidden: &[Forbidden { name: Name::Exact("x"), scope: Scope::ProductionFrom("app") }] };
        assert!(output_error(validate(&m, &policy)).contains("kind"));
    }

    #[test]
    fn rejects_features_that_are_not_strings() {
        let m = json!({
            "packages": [{"id": "p", "name": "p", "version": "1.0.0", "source": CRATES_IO}],
            "resolve": {"nodes": [{"id": "p", "features": ["std", 1]}]}
        });
        let pins = [Pin { name: "p", version: "1.0.0", source: CRATES_IO, features: FeatureRule::Contains(&["std"]) }];
        assert!(output_error(validate(&m, &Policy { pins: &pins, forbidden: &[] })).contains("not a string"));
    }

    /// Fixtures and tests from DeckCraft's `xtask/src/pdf_deps.rs`.
    mod deckcraft {
        use super::super::{CRATES_IO, FeatureRule, Forbidden, Name, Pin, Policy, Scope, Violation, check_target_features, git_source};
        use super::{output_error, validate, violation};
        use serde_json::{Value, json};

        const SHARED_REV: &str = "9f75d3dc19b4b514c4f3d4847ed1ec1b0e86b4ef";
        const FORBIDDEN: &[Forbidden<'static>] = &[Forbidden { name: Name::Family("hayro"), scope: Scope::ProductionFrom("deckcraft-pdf") }];

        fn source() -> String {
            git_source("https://github.com/storytold/craft-libs", SHARED_REV)
        }

        fn pins(source: &str) -> [Pin<'_>; 3] {
            [
                Pin { name: "craft-pdf", version: "0.1.0", source, features: FeatureRule::Exactly(&["write", "write-simple-text"]) },
                Pin { name: "krilla", version: "0.8.2", source, features: FeatureRule::Exactly(&["raster-images", "rayon", "simple-text"]) },
                Pin { name: "hayro-syntax", version: "0.7.2", source: CRATES_IO, features: FeatureRule::Exactly(&["std"]) },
            ]
        }

        fn check(m: &Value) -> Result<(), super::Error> {
            let source = source();
            let pins = pins(&source);
            validate(m, &Policy { pins: &pins, forbidden: FORBIDDEN })
        }

        fn metadata() -> Value {
            let source = source();
            json!({
                "packages": [
                    {"id":"app", "name":"deckcraft-pdf"},
                    {"id":"writer", "name":"craft-pdf", "version":"0.1.0", "source":source},
                    {"id":"backend", "name":"krilla", "version":"0.8.2", "source":source},
                    {"id":"parser", "name":"hayro-syntax", "version":"0.7.2", "source":CRATES_IO}
                ],
                "resolve":{"nodes":[
                    {"id":"app", "deps":[{"pkg":"writer", "dep_kinds":[{"kind":null}]}, {"pkg":"parser", "dep_kinds":[{"kind":"dev"}]}]},
                    {"id":"writer", "features":["write","write-simple-text"], "deps":[{"pkg":"backend", "dep_kinds":[{"kind":null}]}]},
                    {"id":"backend", "features":["raster-images","rayon","simple-text"], "deps":[]},
                    {"id":"parser", "features":["std"], "deps":[]}
                ]}
            })
        }

        #[test]
        fn parallel_compression_is_required_only_on_native_targets() {
            let serial = "krilla v0.8.2 (pinned)|raster-images,simple-text\n";
            let parallel = "krilla v0.8.2 (pinned)|raster-images,rayon,simple-text\n";
            let native: &[&str] = &["raster-images", "rayon", "simple-text"];
            let wasm: &[&str] = &["raster-images", "simple-text"];
            assert!(check_target_features(serial, "krilla", wasm).is_ok());
            assert!(check_target_features(parallel, "krilla", native).is_ok());
            assert!(check_target_features(serial, "krilla", native).is_err());
            assert!(check_target_features(parallel, "krilla", wasm).is_err());
            assert!(check_target_features("", "krilla", wasm).is_err());
        }

        #[test]
        fn rejects_unpinned_or_duplicate_writer_sources() {
            assert!(check(&metadata()).is_ok());
            for source in [Value::Null, json!(CRATES_IO), json!("git+https://github.com/storytold/craft-libs#other")] {
                let mut m = metadata();
                m["packages"][1]["source"] = source;
                assert!(matches!(violation(check(&m)), Violation::Source { .. }));
            }
            let mut m = metadata();
            let mut duplicate = m["packages"][2].clone();
            duplicate["id"] = json!("backend-2");
            m["packages"].as_array_mut().unwrap().push(duplicate);
            assert_eq!(violation(check(&m)), Violation::Count { name: "krilla".into(), found: 2 });
        }

        #[test]
        fn rejects_import_features_and_missing_text_shaping() {
            for (node, features) in [
                (1, json!([])),
                (1, json!(["write"])),
                (1, json!(["write", "write-simple-text", "write-pdf"])),
                (2, json!(["raster-images", "rayon", "simple-text", "pdf"])),
                (2, json!(["raster-images"])),
                (3, json!(["std", "images"])),
            ] {
                let mut m = metadata();
                m["resolve"]["nodes"][node]["features"] = features;
                assert!(matches!(violation(check(&m)), Violation::Features { .. }));
            }
        }

        #[test]
        fn rejects_dev_parser_promoted_to_production_directly_or_transitively() {
            for kind in [Value::Null, json!("build")] {
                let mut m = metadata();
                m["resolve"]["nodes"][0]["deps"][1]["dep_kinds"][0]["kind"] = kind;
                assert!(matches!(violation(check(&m)), Violation::Forbidden { ref name, .. } if name == "hayro-syntax"));
            }
            let mut m = metadata();
            m["resolve"]["nodes"][2]["deps"] = json!([{"pkg":"parser", "dep_kinds":[{"kind":null}]}]);
            assert!(matches!(violation(check(&m)), Violation::Forbidden { ref name, .. } if name == "hayro-syntax"));
            // A dependency that is dev on one target and normal on another is production.
            let mut m = metadata();
            m["resolve"]["nodes"][0]["deps"][1]["dep_kinds"] = json!([{"kind":"dev"}, {"kind":null, "target":"cfg(unix)"}]);
            assert!(matches!(violation(check(&m)), Violation::Forbidden { .. }));
        }

        #[test]
        fn rejects_incomplete_dependency_metadata() {
            let mut m = metadata();
            m["resolve"]["nodes"][2]["deps"] = json!([{"pkg":"missing", "dep_kinds":[{"kind":null}]}]);
            assert!(output_error(check(&m)).contains("missing"));
            let mut m = metadata();
            m["resolve"] = Value::Null;
            assert!(output_error(check(&m)).contains("resolve.nodes"));
            let mut m = metadata();
            m["packages"].as_array_mut().unwrap().retain(|p| p["name"] != "hayro-syntax");
            assert_eq!(violation(check(&m)), Violation::Count { name: "hayro-syntax".into(), found: 0 });
            let mut m = metadata();
            m["resolve"]["nodes"][1]["deps"][0]["dep_kinds"] = json!([]);
            assert!(output_error(check(&m)).contains("dep_kinds"));
        }
    }

    /// Fixtures and tests from VectorCraft's `xtask/src/pdf_deps.rs`.
    mod vectorcraft {
        use super::super::{FeatureRule, Pin, Policy, git_source};
        use super::validate;
        use serde_json::{Value, json};

        const REV: &str = "9be9a63782298ab59cd9b7a42ee9cba4f42a5ccb";
        const PACKAGES: &[(&str, &str)] = &[
            ("craft-pdf", "0.1.0"),
            ("krilla", "0.8.2"),
            ("hayro-syntax", "0.7.2"),
            ("hayro-interpret", "0.7.0"),
            ("hayro-jbig2", "0.3.0"),
            ("hayro-cmap", "0.1.0"),
            ("hayro-postscript", "0.1.0"),
        ];

        fn source(rev: &str) -> String {
            git_source("https://github.com/storytold/craft-libs", rev)
        }

        fn check(m: &Value) -> Result<(), super::Error> {
            let source = source(REV);
            let pins: Vec<Pin<'_>> =
                PACKAGES.iter().map(|&(name, version)| Pin { name, version, source: &source, features: FeatureRule::Ignore }).collect();
            validate(m, &Policy { pins: &pins, forbidden: &[] })
        }

        fn metadata() -> Value {
            let packages: Vec<_> = PACKAGES.iter().map(|(name, version)| json!({"name":name, "version":version, "source":source(REV)})).collect();
            json!({"packages": packages})
        }

        #[test]
        fn accepts_pinned_pdf_packages_without_changing_printer_transport() {
            let mut m = metadata();
            m["packages"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name":"craft-print", "version":"0.1.0", "source":source("d991bf6792f8ade99c51358bf3c1d902c8cd96b5")}));
            assert!(check(&m).is_ok());
        }

        #[test]
        fn rejects_missing_or_duplicate_pdf_packages() {
            for index in 0..PACKAGES.len() {
                let mut m = metadata();
                m["packages"].as_array_mut().unwrap().remove(index);
                assert!(check(&m).is_err());
                let mut m = metadata();
                let duplicate = m["packages"][index].clone();
                m["packages"].as_array_mut().unwrap().push(duplicate);
                assert!(check(&m).is_err());
            }
        }

        #[test]
        fn rejects_registry_local_version_and_revision_drift() {
            for index in 0..PACKAGES.len() {
                for (field, value) in [
                    ("source", Value::Null),
                    ("source", json!("registry+https://github.com/rust-lang/crates.io-index")),
                    ("source", json!("git+https://github.com/storytold/craft-libs?branch=main#other")),
                    ("source", json!(source("92d392cff132eaf53e13afff77f38b5c11b73aa0"))),
                    ("version", json!("0.99.0")),
                ] {
                    let mut m = metadata();
                    m["packages"][index][field] = value;
                    assert!(check(&m).is_err());
                }
            }
        }
    }

    /// Fixtures and tests from DesignCraft's `xtask/src/pdf_deps.rs`.
    mod designcraft {
        use super::super::{FeatureRule, Pin, Policy, Violation, git_source};
        use super::{validate, violation};
        use serde_json::{Value, json};

        const REV: &str = "a0916808f7d78bed8c7b90cdf937176015454a7f";
        const PACKAGES: &[(&str, &str)] = &[
            ("craft-pdf", "0.1.0"),
            ("krilla", "0.8.2"),
            ("hayro", "0.7.1"),
            ("hayro-interpret", "0.7.0"),
            ("hayro-syntax", "0.7.2"),
            ("hayro-jbig2", "0.3.0"),
            ("hayro-cmap", "0.1.0"),
            ("hayro-postscript", "0.1.0"),
        ];
        const FACADE_FEATURES: &[&str] = &["read", "interpret", "raster", "write", "write-pdf"];

        fn check(m: &Value) -> Result<(), super::Error> {
            let source = git_source("https://github.com/storytold/craft-libs", REV);
            let pins: Vec<Pin<'_>> = PACKAGES
                .iter()
                .map(|&(name, version)| {
                    let features = if name == "craft-pdf" { FeatureRule::Contains(FACADE_FEATURES) } else { FeatureRule::Ignore };
                    Pin { name, version, source: &source, features }
                })
                .collect();
            validate(m, &Policy { pins: &pins, forbidden: &[] })
        }

        fn metadata() -> Value {
            json!({"packages": PACKAGES.iter().map(|(name, version)| json!({"id":name,"name":name,"version":version,
                "source":git_source("https://github.com/storytold/craft-libs", REV)})).collect::<Vec<_>>(), "resolve": {"nodes": [{"id": "craft-pdf", "features": ["read", "interpret", "raster", "write", "write-pdf"]}]}})
        }

        #[test]
        fn accepts_one_pinned_facade_and_reader() {
            assert!(check(&metadata()).is_ok());
            // Features beyond the required ones are allowed.
            let mut m = metadata();
            m["resolve"]["nodes"][0]["features"].as_array_mut().unwrap().push(json!("write-simple-text"));
            assert!(check(&m).is_ok());
        }

        #[test]
        fn rejects_unpatched_and_local_reader_sources() {
            for source in [Value::Null, json!("registry+https://github.com/rust-lang/crates.io-index")] {
                let mut m = metadata();
                m["packages"][1]["source"] = source;
                assert!(matches!(violation(check(&m)), Violation::Source { .. }));
            }
        }

        #[test]
        fn rejects_missing_duplicate_and_wrong_version_packages() {
            for index in 0..PACKAGES.len() {
                let mut m = metadata();
                m["packages"][index]["version"] = json!("99.0.0");
                assert!(matches!(violation(check(&m)), Violation::Version { .. }));
                let mut m = metadata();
                let mut duplicate = m["packages"][index].clone();
                duplicate["id"] = json!("duplicate");
                m["packages"].as_array_mut().unwrap().push(duplicate);
                assert!(matches!(violation(check(&m)), Violation::Count { found: 2, .. }));
                let mut m = metadata();
                m["packages"].as_array_mut().unwrap().remove(index);
                assert!(matches!(violation(check(&m)), Violation::Count { found: 0, .. }));
            }
        }

        #[test]
        fn rejects_missing_reader_or_writer_features() {
            for index in 0..5 {
                let mut m = metadata();
                m["resolve"]["nodes"][0]["features"].as_array_mut().unwrap().remove(index);
                assert!(matches!(violation(check(&m)), Violation::Features { .. }));
            }
            let mut m = metadata();
            m["resolve"] = Value::Null;
            assert!(check(&m).is_err());
        }

        #[test]
        fn rejects_facade_revision_drift_and_malformed_metadata() {
            let mut m = metadata();
            m["packages"][0]["source"] = json!("git+https://github.com/storytold/craft-libs?branch=main#other");
            assert!(check(&m).is_err());
            assert!(check(&json!({})).is_err());
        }
    }

    /// Fixtures and tests from EffectCraft's `xtask/src/pdf_deps.rs`.
    mod effectcraft {
        use super::super::{FeatureRule, Forbidden, Name, Pin, Policy, Scope, Violation, git_source};
        use super::{validate, violation};
        use serde_json::{Value, json};

        const REV: &str = "9f75d3dc19b4b514c4f3d4847ed1ec1b0e86b4ef";
        const PACKAGES: &[(&str, &str)] = &[("craft-pdf", "0.1.0"), ("hayro-syntax", "0.7.2"), ("hayro-jbig2", "0.3.0")];
        const FORBIDDEN: &[Forbidden<'static>] = &[
            Forbidden { name: Name::Exact("hayro"), scope: Scope::Active },
            Forbidden { name: Name::Exact("hayro-interpret"), scope: Scope::Active },
            Forbidden { name: Name::Exact("hayro-cmap"), scope: Scope::Active },
            Forbidden { name: Name::Exact("krilla"), scope: Scope::Active },
        ];

        fn check(m: &Value) -> Result<(), super::Error> {
            let source = git_source("https://github.com/storytold/craft-libs", REV);
            let pins: Vec<Pin<'_>> = PACKAGES
                .iter()
                .map(|&(name, version)| {
                    let features = match name {
                        "craft-pdf" => FeatureRule::Exactly(&["read"]),
                        "hayro-syntax" => FeatureRule::Exactly(&["std"]),
                        _ => FeatureRule::Ignore,
                    };
                    Pin { name, version, source: &source, features }
                })
                .collect();
            validate(m, &Policy { pins: &pins, forbidden: FORBIDDEN })
        }

        fn metadata() -> Value {
            let packages: Vec<_> = PACKAGES
                .iter()
                .map(|(name, version)| json!({"id": name, "name": name, "version": version, "source": git_source("https://github.com/storytold/craft-libs", REV)}))
                .collect();
            json!({"packages": packages, "resolve": {"nodes": [
                {"id": "hayro-syntax", "features": ["std"]}, {"id": "craft-pdf", "features": ["read"]}
            ]}})
        }

        #[test]
        fn accepts_the_pinned_parser_only_graph() {
            assert!(check(&metadata()).is_ok());
            // A forbidden package without a resolved node is not active.
            let mut m = metadata();
            m["packages"].as_array_mut().unwrap().push(json!({"id": "krilla", "name": "krilla"}));
            assert!(check(&m).is_ok());
        }

        #[test]
        fn rejects_unpinned_duplicate_and_missing_parsers() {
            let mut m = metadata();
            m["packages"][0]["source"] = json!("registry");
            assert!(check(&m).is_err());
            let mut m = metadata();
            let mut duplicate = m["packages"][0].clone();
            duplicate["id"] = json!("craft-pdf-2");
            m["packages"].as_array_mut().unwrap().push(duplicate);
            assert!(check(&m).is_err());
            m["packages"].as_array_mut().unwrap().retain(|p| p["name"] != "hayro-syntax");
            assert!(check(&m).is_err());
        }

        #[test]
        fn rejects_shared_reader_source_drift() {
            let mut m = metadata();
            m["packages"][0]["source"] = Value::Null;
            assert!(matches!(violation(check(&m)), Violation::Source { found: None, .. }));
        }

        #[test]
        fn rejects_extra_features_and_missing_active_nodes() {
            for (index, feature) in [(0, "images"), (1, "write"), (1, "raster")] {
                let mut m = metadata();
                m["resolve"]["nodes"][index]["features"].as_array_mut().unwrap().push(json!(feature));
                assert!(matches!(violation(check(&m)), Violation::Features { ref extra, .. } if extra == &[feature.to_owned()]));
            }
            let mut m = metadata();
            m["resolve"]["nodes"].as_array_mut().unwrap().pop();
            assert!(check(&m).is_err());
        }

        #[test]
        fn rejects_backend_asset_features_and_missing_resolution() {
            for name in ["hayro", "hayro-interpret", "hayro-cmap", "krilla"] {
                let mut m = metadata();
                m["packages"].as_array_mut().unwrap().push(json!({"id": name, "name": name}));
                m["resolve"]["nodes"].as_array_mut().unwrap().push(json!({"id": name}));
                assert!(matches!(violation(check(&m)), Violation::Forbidden { name: ref found, root: None, .. } if found == name));
            }
            let mut m = metadata();
            m["resolve"] = Value::Null;
            assert!(check(&m).is_err());
        }
    }

    /// The commands run as children: the test binary itself, running [`fake_child`].
    mod process {
        use super::super::{Error, Limits, TargetFeatures, Violation, host_target, metadata, run_target_features};
        use std::io::Write;
        use std::process::Command;

        const MODE: &str = "CRAFT_XTASK_PINS_FAKE";

        fn fake(mode: &str) -> Command {
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            cmd.args(["--ignored", "--exact", "--nocapture", "--test-threads=1", "pins::tests::process::fake_child", "--"]).env(MODE, mode);
            cmd
        }

        /// Stands in for `cargo` or `rustc` when [`MODE`] is set; does nothing otherwise. The harness
        /// reads the arguments the library adds after `--` as more test filters, and with `--exact`
        /// none of them selects another test.
        #[test]
        #[ignore = "a child process for the other process tests"]
        fn fake_child() {
            let Ok(mode) = std::env::var(MODE) else {
                return;
            };
            let chunk = [b'x'; 8192];
            match mode.as_str() {
                "endless" => {
                    let mut out = std::io::stdout().lock();
                    while out.write_all(&chunk).is_ok() {}
                }
                "fail" => {
                    let mut err = std::io::stderr().lock();
                    for _ in 0..256 {
                        let _ = err.write_all(&chunk);
                    }
                    std::process::exit(3);
                }
                "host" => println!("rustc 1.0.0\nhost: fake-target\nrelease: 1.0.0"),
                "tree" => println!("app-pdf v0.1.0|\ncraft-pdf v0.1.0|write\nkrilla v0.8.2|raster-images,simple-text"),
                _ => {}
            }
        }

        #[test]
        fn target_features_are_read_from_the_tree_command() {
            let spec = TargetFeatures { root: "app-pdf", depth: 2, package: "krilla", features: &["raster-images", "simple-text"] };
            assert!(run_target_features(|| fake("tree"), "wasm32-unknown-unknown", &spec, &Limits::default()).is_ok());
            let spec = TargetFeatures { features: &["raster-images", "rayon", "simple-text"], ..spec };
            match run_target_features(|| fake("tree"), "x86_64-unknown-linux-gnu", &spec, &Limits::default()) {
                Err(Error::Violation(Violation::Features { missing, extra, .. })) => {
                    assert_eq!(missing, ["rayon"]);
                    assert!(extra.is_empty());
                }
                other => panic!("expected a feature violation, got {other:?}"),
            }
        }

        #[test]
        fn output_past_the_limit_stops_the_command() {
            let limits = Limits { metadata_bytes: 64 << 10, ..Limits::default() };
            match metadata(|| fake("endless"), &limits) {
                Err(Error::TooLarge { command, limit }) => {
                    assert_eq!(command, "cargo metadata");
                    assert_eq!(limit, 64 << 10);
                }
                other => panic!("expected TooLarge, got {other:?}"),
            }
        }

        #[test]
        fn a_failing_command_keeps_the_start_of_a_long_stderr() {
            let limits = Limits { stderr_bytes: 1000, ..Limits::default() };
            match metadata(|| fake("fail"), &limits) {
                Err(Error::Exit { status, stderr, .. }) => {
                    assert_eq!(status.code(), Some(3));
                    assert_eq!(stderr, "x".repeat(1000));
                }
                other => panic!("expected Exit, got {other:?}"),
            }
        }

        #[test]
        fn a_command_that_cannot_start_is_an_error() {
            let r = metadata(|| Command::new("craft-xtask-pins-no-such-program"), &Limits::default());
            assert!(matches!(r, Err(Error::Start { .. })));
        }

        #[test]
        fn the_host_target_is_read_from_its_line() {
            assert_eq!(host_target(|| fake("host"), &Limits::default()).unwrap(), "fake-target");
            let limits = Limits { version_bytes: 4, ..Limits::default() };
            assert!(matches!(host_target(|| fake("host"), &limits), Err(Error::TooLarge { .. })));
        }
    }
}
