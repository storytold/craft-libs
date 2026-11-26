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
//! - Packages the policy does not name are allowed. An empty policy sets no rule on the packages of
//!   metadata that is within [`Limits::metadata_bytes`], parses and has a `packages` list with a
//!   `name` for each package.
//!
//! [`check_commits`] reads the same JSON against a list of [`OneCommit`] rules: every package from
//! the rule's git repository resolves from one commit, apart from the [`Extra`] packages the rule
//! names at their own commit.
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
//! the command and returns [`Error::TooLarge`]. Standard output that stays open for 2 seconds after
//! the command exits, held by a process it started, returns [`Error::Read`]. Standard error is read to
//! its end while the command runs, and its first [`Limits::stderr_bytes`] bytes are kept for the error
//! message; after a failing exit it is read for at most 2 seconds more.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

/// The `source` Cargo writes for a package from crates.io.
pub const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// The `source` Cargo writes for a package from the git repository at `url`, pinned with
/// `rev = "<rev>"` where `rev` is a full commit hash: `git+<url>?rev=<rev>#<rev>`, with `url` written
/// as Cargo writes it. For `http` and `https` that is the scheme and host in lowercase, without the
/// default port, and with the `.` and `..` segments of the path resolved:
/// `https://github.com:443/o/./r` becomes `https://github.com/o/r`.
pub fn git_source(url: &str, rev: &str) -> String {
    format!("git+{}?rev={rev}#{rev}", cargo_url(url))
}

/// `url` as Cargo writes it in a `source`: the scheme in lowercase, the `.` and `..` segments of the
/// path resolved, and, for `http` and `https`, the host in lowercase, a port of digits written as its
/// number, an empty port or the scheme's default port (80 or 443) left out, and `/` for an empty
/// path. Other URLs keep their authority as
/// written.
fn cargo_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return url.to_owned() };
    let scheme = scheme.to_ascii_lowercase();
    let (authority, tail) = rest.split_at_checked(rest.find(['/', '?', '#']).unwrap_or(rest.len())).unwrap_or((rest, ""));
    let (path, query) = tail.split_at_checked(tail.find(['?', '#']).unwrap_or(tail.len())).unwrap_or((tail, ""));
    let path = if path.is_empty() { String::new() } else { resolve_dot_segments(path) };
    let default_port: u16 = match scheme.as_str() {
        "http" => 80,
        "https" => 443,
        _ => return format!("{scheme}://{authority}{path}{query}"),
    };
    let (user, host_port) = match authority.rsplit_once('@') {
        Some((user, host_port)) => (Some(user), host_port),
        None => (None, authority),
    };
    // A `:` inside an IPv6 address's brackets does not start the port.
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) if !port.contains(']') => (host, Some(port)),
        _ => (host_port, None),
    };
    let mut out = format!("{scheme}://");
    if let Some(user) = user {
        out.push_str(user);
        out.push('@');
    }
    out.push_str(&host.to_ascii_lowercase());
    // A port of digits is written as its number, so `0443` is the default port 443.
    let port = port.filter(|port| !port.is_empty()).map(|port| match port.parse::<u16>() {
        Ok(number) if port.bytes().all(|b| b.is_ascii_digit()) => (number != default_port).then(|| number.to_string()),
        _ => Some(port.to_owned()),
    });
    if let Some(port) = port.flatten() {
        out.push(':');
        out.push_str(&port);
    }
    if path.is_empty() {
        out.push('/');
    }
    out.push_str(&path);
    out.push_str(query);
    out
}

/// `path`, which starts with `/`, with its `.` and `..` segments resolved as Cargo's URL parser
/// resolves them: `.` is dropped, `..` drops the segment before it, and either one as the last segment
/// leaves the path ending in `/`. `%2e` counts as a `.`, in either case.
fn resolve_dot_segments(path: &str) -> String {
    let segments: Vec<&str> = path.strip_prefix('/').unwrap_or(path).split('/').collect();
    let last = segments.len().saturating_sub(1);
    let mut out: Vec<&str> = Vec::with_capacity(segments.len());
    for (i, segment) in segments.iter().enumerate() {
        let dots = match segment.to_ascii_lowercase().as_str() {
            "." | "%2e" => 1,
            ".." | ".%2e" | "%2e." | "%2e%2e" => 2,
            _ => 0,
        };
        if dots == 2 {
            out.pop();
        }
        if dots == 0 {
            out.push(segment);
        } else if i == last {
            out.push("");
        }
    }
    format!("/{}", out.join("/"))
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

/// Packages from one git repository that resolve from one commit, for [`check_commits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OneCommit<'a> {
    /// The repository URL as the app's `Cargo.toml` writes it, for example
    /// `https://github.com/storytold/craft-libs`.
    pub url: &'a str,
    /// Packages allowed at another commit.
    pub extra: &'a [Extra<'a>],
}

/// A package a [`OneCommit`] rule allows at a commit other than the one the rest resolve from, for a
/// dependency the app declares at a second revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extra<'a> {
    /// The package name.
    pub name: &'a str,
    /// The full commit hash.
    pub rev: &'a str,
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
    /// Packages from one git repository resolve from more than one commit.
    Commits {
        /// The repository URL.
        url: String,
        /// Each commit, in sorted order, with the sorted names of the packages resolved from it.
        commits: Vec<(String, Vec<String>)>,
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
            Violation::Commits { url, commits } => {
                write!(f, "{url}: packages resolve from {} commits:", commits.len())?;
                for (i, (commit, names)) in commits.iter().enumerate() {
                    let sep = if i == 0 { " " } else { "; " };
                    write!(f, "{sep}{commit} ({})", names.join(", "))?;
                }
                Ok(())
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
    /// A command's output could not be read, the command could not be waited for, or its standard
    /// output was still open 2 seconds after it exited (an error of kind
    /// [`io::ErrorKind::TimedOut`]).
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

/// Checks `cargo metadata --format-version 1` JSON against `rules`, in order: every package whose
/// `source` is the rule's git repository resolves from one commit, except the packages its
/// [`OneCommit::extra`] list names at their own commit. The commit is the part of the `source` after
/// its `#`. Packages from other sources, path packages included, are not checked. The first broken
/// rule is returned.
pub fn check_commits(metadata_json: &[u8], rules: &[OneCommit<'_>], limits: &Limits) -> Result<(), Error> {
    if metadata_json.len() > limits.metadata_bytes {
        return Err(Error::TooLarge { command: METADATA.into(), limit: limits.metadata_bytes });
    }
    let meta: Value = serde_json::from_slice(metadata_json).map_err(|e| invalid(format!("not JSON: {e}")))?;
    let packages = meta.get("packages").and_then(Value::as_array).ok_or_else(|| invalid("packages is missing"))?;
    let names = packages.iter().map(|package| str_field(package, "name", "a package")).collect::<Result<Vec<_>, _>>()?;
    for rule in rules {
        let url = cargo_url(rule.url);
        let mut commits: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (package, &name) in packages.iter().zip(&names) {
            let Some(source) = package.get("source").and_then(Value::as_str) else { continue };
            let Some(commit) = git_commit(source, &url)? else { continue };
            if rule.extra.iter().any(|extra| extra.name == name && extra.rev == commit) {
                continue;
            }
            commits.entry(commit).or_default().insert(name);
        }
        if commits.len() > 1 {
            let commits = commits.into_iter().map(|(commit, names)| (commit.to_owned(), names.into_iter().map(str::to_owned).collect())).collect();
            return Err(Violation::Commits { url: rule.url.into(), commits }.into());
        }
    }
    Ok(())
}

/// The commit of a `source` from the git repository at `url` (as [`cargo_url`] writes it): the text
/// after its last `#`. `None` for a source from anywhere else.
fn git_commit<'s>(source: &'s str, url: &str) -> Result<Option<&'s str>, Error> {
    let Some(rest) = source.strip_prefix("git+").and_then(|s| s.strip_prefix(url)) else { return Ok(None) };
    if !rest.is_empty() && !rest.starts_with(['?', '#']) {
        return Ok(None);
    }
    match rest.rsplit_once('#') {
        Some((_, commit)) if !commit.is_empty() => Ok(Some(commit)),
        _ => Err(invalid(format!("source {source} names no commit"))),
    }
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

/// How long [`output`] waits for a command's pipes to close after the command exits. A process the
/// command started can hold them open.
const DRAIN: Duration = Duration::from_secs(2);

/// How often [`output`] checks whether the command exited while its standard output is read.
const POLL: Duration = Duration::from_millis(10);

/// Runs `cmd` and returns its standard output, or an error when it fails or prints more than
/// `limit` bytes. Standard output is read on another thread while the command's exit is checked, and
/// once it exits the output must reach its end within [`DRAIN`]. Standard error is read on another
/// thread to its end, so a command that writes a lot there does not stop on a full pipe; its first
/// `stderr_limit` bytes are kept, waiting at most [`DRAIN`] after a failing exit.
fn output(cmd: Command, command: &str, limit: usize, stderr_limit: usize) -> Result<Vec<u8>, Error> {
    output_with(cmd, command, limit, stderr_limit, Instant::now, Head::finish)
}

fn output_with(
    mut cmd: Command,
    command: &str,
    limit: usize,
    stderr_limit: usize,
    mut now: impl FnMut() -> Instant,
    finish: impl FnOnce(Head, Duration) -> Vec<u8>,
) -> Result<Vec<u8>, Error> {
    let read_error = |source: io::Error| Error::Read { command: command.into(), source };
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|source| Error::Start { command: command.into(), source })?;
    // Reader threads that cannot start stop the command. On an error the threads already started are
    // left to end when their pipes close: a process the child started may hold a pipe open after the
    // child is stopped.
    let stderr = match child.stderr.take().map(|pipe| Head::start(pipe, stderr_limit)).transpose() {
        Ok(head) => head,
        Err(source) => {
            stop(&mut child);
            return Err(read_error(source));
        }
    };
    let Some(stdout) = child.stdout.take() else {
        stop(&mut child);
        return Err(read_error(io::Error::other("no output pipe")));
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::Builder::new().name("craft-xtask stdout".into()).spawn(move || {
        let _ = sender.send(read_bounded(stdout, limit));
    });
    if let Err(source) = reader {
        stop(&mut child);
        return Err(read_error(source));
    }
    // The output, or `None` when it is still open `DRAIN` after the command exited, and the exit with
    // the time it was seen when that came first.
    let (read, exited) = loop {
        match receiver.recv_timeout(POLL) {
            Ok(read) => break (Some(read), None),
            Err(RecvTimeoutError::Disconnected) => break (Some(Err(io::Error::other("the output reader stopped"))), None),
            Err(RecvTimeoutError::Timeout) => match child.try_wait() {
                Ok(Some(status)) => {
                    let exit = now();
                    break (receiver.recv_timeout(DRAIN).ok(), Some((status, exit)));
                }
                Ok(None) => {}
                Err(source) => {
                    stop(&mut child);
                    return Err(read_error(source));
                }
            },
        }
    };
    let stdout = match read {
        Some(Ok(Some(stdout))) => Some(stdout),
        Some(Ok(None)) => {
            stop(&mut child);
            return Err(Error::TooLarge { command: command.into(), limit });
        }
        Some(Err(source)) => {
            stop(&mut child);
            return Err(read_error(source));
        }
        None => None,
    };
    let (status, exit) = match exited {
        Some(exited) => exited,
        None => (child.wait().map_err(read_error)?, now()),
    };
    let Some(stdout) = stdout else {
        return Err(read_error(io::Error::new(io::ErrorKind::TimedOut, "the output stayed open after the command exited")));
    };
    if status.success() {
        return Ok(stdout);
    }
    let stderr = stderr.map(|head| finish(head, drain_left(exit, now()))).unwrap_or_default();
    let mut stderr = String::from_utf8_lossy(&stderr).into_owned();
    // An incomplete character at the end of the kept bytes decodes to a three-byte replacement.
    while stderr.len() > stderr_limit {
        stderr.pop();
    }
    Err(Error::Exit { command: command.into(), status, stderr: stderr.trim().to_owned() })
}

/// The part of [`DRAIN`] after a command's exit at `exit` that is left at `now`; zero once it has
/// passed.
fn drain_left(exit: Instant, now: Instant) -> Duration {
    (exit + DRAIN).saturating_duration_since(now)
}

/// Everything `reader` gives, or `None` when that is more than `limit` bytes. The read stops one byte
/// past the limit.
fn read_bounded(reader: impl Read, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    reader.take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1)).read_to_end(&mut buf)?;
    Ok((buf.len() <= limit).then_some(buf))
}

/// The first bytes of a pipe that another thread reads to its end.
struct Head {
    bytes: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

impl Head {
    /// Starts the thread that reads `reader` and keeps its first `limit` bytes.
    fn start(reader: impl Read + Send + 'static, limit: usize) -> io::Result<Head> {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let (sender, done) = mpsc::sync_channel(1);
        let head = Arc::clone(&bytes);
        thread::Builder::new().name("craft-xtask stderr".into()).spawn(move || {
            read_head(reader, limit, &head);
            let _ = sender.send(());
        })?;
        Ok(Head { bytes, done })
    }

    /// The bytes kept once the pipe reaches its end, or those kept after `wait` when it is still open.
    fn finish(self, wait: Duration) -> Vec<u8> {
        let _ = self.done.recv_timeout(wait);
        self.bytes.lock().map(|bytes| bytes.to_vec()).unwrap_or_default()
    }
}

/// Reads `reader` to its end and keeps its first `limit` bytes in `head`.
fn read_head(mut reader: impl Read, limit: usize, head: &Mutex<Vec<u8>>) {
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let Ok(mut head) = head.lock() else { break };
                let room = limit.saturating_sub(head.len());
                head.extend_from_slice(chunk.get(..n.min(room)).unwrap_or_default());
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

/// Ends a child whose output is no longer read.
fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::{
        CRATES_IO, Error, Extra, FeatureRule, Forbidden, Limits, Name, OneCommit, Pin, Policy, Scope, Violation, check, check_commits,
        check_target_features, git_source,
    };
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
    fn git_sources_write_the_url_as_cargo_does() {
        assert_eq!(git_source("https://github.com:443/o/r", "abc"), "git+https://github.com/o/r?rev=abc#abc");
        assert_eq!(git_source("HTTPS://GitHub.com/O/r", "abc"), "git+https://github.com/O/r?rev=abc#abc");
        assert_eq!(git_source("http://host:80/r", "abc"), "git+http://host/r?rev=abc#abc");
        assert_eq!(git_source("https://host:/r", "abc"), "git+https://host/r?rev=abc#abc");
        assert_eq!(git_source("https://user@[::1]:443/r", "abc"), "git+https://user@[::1]/r?rev=abc#abc");
        assert_eq!(git_source("https://host", "abc"), "git+https://host/?rev=abc#abc");
        // Other ports, and schemes without a default port, are kept.
        assert_eq!(git_source("https://host:8443/r", "abc"), "git+https://host:8443/r?rev=abc#abc");
        assert_eq!(git_source("ssh://git@Host:22/r", "abc"), "git+ssh://git@Host:22/r?rev=abc#abc");
    }

    #[test]
    fn git_sources_write_a_port_as_its_number() {
        assert_eq!(git_source("https://host:0443/r", "abc"), "git+https://host/r?rev=abc#abc");
        assert_eq!(git_source("http://host:080/r", "abc"), "git+http://host/r?rev=abc#abc");
        assert_eq!(git_source("https://host:08443/r", "abc"), "git+https://host:8443/r?rev=abc#abc");
    }

    #[test]
    fn git_sources_resolve_dot_segments_as_cargo_does() {
        assert_eq!(git_source("https://github.com/o/./r", "abc"), "git+https://github.com/o/r?rev=abc#abc");
        assert_eq!(git_source("https://github.com/o/x/../r", "abc"), "git+https://github.com/o/r?rev=abc#abc");
        assert_eq!(git_source("https://github.com/o/%2E/r/%2e%2E/r", "abc"), "git+https://github.com/o/r?rev=abc#abc");
        assert_eq!(git_source("https://host/o/r/.", "abc"), "git+https://host/o/r/?rev=abc#abc");
        assert_eq!(git_source("https://host/o/..", "abc"), "git+https://host/?rev=abc#abc");
        assert_eq!(git_source("https://host/../r", "abc"), "git+https://host/r?rev=abc#abc");
        assert_eq!(git_source("ssh://git@Host/o/./r", "abc"), "git+ssh://git@Host/o/r?rev=abc#abc");
        // Segments that only contain dots are kept.
        assert_eq!(git_source("https://host/o/.../r.", "abc"), "git+https://host/o/.../r.?rev=abc#abc");
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

    const LIBS: &str = "https://github.com/storytold/craft-libs";

    fn libs_metadata(packages: &[(&str, &str)]) -> Value {
        let packages: Vec<Value> = packages.iter().map(|(name, source)| json!({"name": name, "source": source})).collect();
        json!({ "packages": packages })
    }

    fn check_libs(m: &Value, extra: &[Extra<'_>]) -> Result<(), Error> {
        check_commits(&bytes(m), &[OneCommit { url: LIBS, extra }], &Limits::default())
    }

    #[test]
    fn packages_from_one_repository_resolve_from_one_commit() {
        let a = git_source(LIBS, "aaa");
        let b = git_source(LIBS, "bbb");
        let ok = libs_metadata(&[("craft-xml", &a), ("craft-zip", &a), ("serde", CRATES_IO), ("app", "")]);
        assert!(check_libs(&ok, &[]).is_ok());
        assert!(check_libs(&json!({"packages": [{"name": "app"}]}), &[]).is_ok(), "a path package has no source");

        let split = libs_metadata(&[("craft-xml", &a), ("craft-zip", &b), ("craft-xml", &b)]);
        assert_eq!(
            violation(check_libs(&split, &[])),
            Violation::Commits {
                url: LIBS.into(),
                commits: vec![("aaa".into(), vec!["craft-xml".into()]), ("bbb".into(), vec!["craft-xml".into(), "craft-zip".into()])]
            }
        );
        assert_eq!(
            violation(check_libs(&split, &[])).to_string(),
            "https://github.com/storytold/craft-libs: packages resolve from 2 commits: aaa (craft-xml); bbb (craft-xml, craft-zip)"
        );
    }

    #[test]
    fn an_extra_allows_one_package_at_its_own_commit_only() {
        let main = git_source(LIBS, "aaa");
        let dtd = git_source(LIBS, "ddd");
        let m = libs_metadata(&[("craft-xml", &main), ("craft-zip", &main), ("craft-xml", &dtd)]);
        assert!(check_libs(&m, &[Extra { name: "craft-xml", rev: "ddd" }]).is_ok());
        // The extra names one package at one commit: another package there is still a second commit.
        let m = libs_metadata(&[("craft-xml", &main), ("craft-xml", &dtd), ("craft-zip", &dtd)]);
        assert!(matches!(violation(check_libs(&m, &[Extra { name: "craft-xml", rev: "ddd" }])), Violation::Commits { .. }));
        assert!(matches!(violation(check_libs(&m, &[Extra { name: "craft-xml", rev: "eee" }])), Violation::Commits { .. }));
    }

    #[test]
    fn a_commit_is_read_from_any_git_source_of_the_repository_only() {
        // A branch or tag dependency still names its commit after `#`.
        let m = libs_metadata(&[("craft-xml", &git_source(LIBS, "aaa")), ("craft-zip", &format!("git+{LIBS}?branch=main#bbb"))]);
        assert!(matches!(violation(check_libs(&m, &[])), Violation::Commits { .. }));
        // A repository whose URL only starts the same is another repository.
        let m =
            libs_metadata(&[("craft-xml", &git_source(LIBS, "aaa")), ("other", &git_source("https://github.com/storytold/craft-libs-extra", "bbb"))]);
        assert!(check_libs(&m, &[]).is_ok());
        let m = libs_metadata(&[("craft-xml", &format!("git+{LIBS}?rev=aaa"))]);
        assert!(output_error(check_libs(&m, &[])).contains("names no commit"));
        let m = libs_metadata(&[("craft-xml", &format!("git+{LIBS}"))]);
        assert!(output_error(check_libs(&m, &[])).contains("names no commit"));
        assert!(output_error(check_libs(&json!({"packages": [{"source": CRATES_IO}]}), &[])).contains("name"));
    }

    #[test]
    fn a_package_without_a_name_is_an_error_without_rules_too() {
        let limits = Limits::default();
        assert!(output_error(check_commits(&bytes(&json!({"packages": [{}]})), &[], &limits)).contains("name"));
        assert!(check_commits(&bytes(&json!({"packages": [{"name": "app"}]})), &[], &limits).is_ok());
    }

    #[test]
    fn a_rule_url_with_the_default_port_matches_the_source_cargo_writes() {
        let m = libs_metadata(&[("craft-xml", &git_source(LIBS, "aaa")), ("craft-zip", &git_source(LIBS, "bbb"))]);
        let rule = OneCommit { url: "https://github.com:443/storytold/craft-libs", extra: &[] };
        assert!(matches!(violation(check_commits(&bytes(&m), &[rule], &Limits::default())), Violation::Commits { .. }));
    }

    #[test]
    fn a_rule_url_with_dot_segments_matches_the_source_cargo_writes() {
        let m = libs_metadata(&[
            ("craft-xml", "git+https://github.com/storytold/craft-libs?rev=aaa#aaa"),
            ("craft-zip", "git+https://github.com/storytold/craft-libs?rev=bbb#bbb"),
            ("other", "git+https://github.com/storytold/craft-libs-extra?rev=ccc#ccc"),
        ]);
        let rule = OneCommit { url: "https://github.com/storytold/./x/../craft-libs", extra: &[] };
        match violation(check_commits(&bytes(&m), &[rule], &Limits::default())) {
            Violation::Commits { commits, .. } => {
                assert_eq!(commits, vec![("aaa".into(), vec!["craft-xml".into()]), ("bbb".into(), vec!["craft-zip".into()])]);
            }
            other => panic!("expected a commits violation, got {other:?}"),
        }
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
            assert_eq!(violation(check(&m)), Violation::Count { name: "craft-pdf".into(), found: 2 });
            let mut m = metadata();
            m["packages"].as_array_mut().unwrap().retain(|p| p["name"] != "hayro-syntax");
            assert_eq!(violation(check(&m)), Violation::Count { name: "hayro-syntax".into(), found: 0 });
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
        #[cfg(unix)]
        use super::super::output_with;
        use super::super::{DRAIN, Error, Limits, TargetFeatures, Violation, drain_left, host_target, metadata, run_target_features};
        use std::io::{BufRead, BufReader, Read, Write};
        use std::process::{Command, Stdio};

        const MODE: &str = "CRAFT_XTASK_PINS_FAKE";
        /// The file whose lock a `linger` child waits for.
        const RELEASE: &str = "CRAFT_XTASK_PINS_RELEASE";
        const RELEASE_OUT: &str = "CRAFT_XTASK_PINS_RELEASE_OUT";
        const RELEASE_ERR: &str = "CRAFT_XTASK_PINS_RELEASE_ERR";
        /// The line a `linger` child prints once it holds its pipe.
        const READY: &str = "craft-xtask-pins-linger-ready";

        fn fake(mode: &str) -> Command {
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            cmd.args(["--ignored", "--exact", "--nocapture", "--test-threads=1", "pins::tests::process::fake_child", "--"]).env(MODE, mode);
            cmd
        }

        fn pipe_holder(holds_stdout: bool, release: Option<std::ffi::OsString>) -> bool {
            let mut linger = fake(if holds_stdout { "linger-out" } else { "linger-err" });
            linger.stdin(Stdio::null());
            if let Some(release) = release {
                linger.env(RELEASE, release);
            }
            if holds_stdout {
                linger.stdout(Stdio::inherit()).stderr(Stdio::piped());
            } else {
                linger.stdout(Stdio::piped()).stderr(Stdio::inherit());
            }
            let Ok(mut child) = linger.spawn() else {
                return false;
            };
            let signal: Option<Box<dyn Read>> = if holds_stdout {
                child.stderr.take().map(|pipe| Box::new(pipe) as Box<dyn Read>)
            } else {
                child.stdout.take().map(|pipe| Box::new(pipe) as Box<dyn Read>)
            };
            signal.is_some_and(|pipe| BufReader::new(pipe).lines().any(|line| line.is_ok_and(|line| line.contains(READY))))
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
                "utf8" => {
                    let _ = std::io::stderr().write_all("é".as_bytes());
                    std::process::exit(3);
                }
                // Starts a `linger` child that keeps one of this process's pipes open after it exits,
                // and waits until it reports that it holds the pipe. Exits with code 4 when it does not.
                "orphan-out" | "orphan-out-fail" | "orphan-err" => {
                    let holds_stdout = mode != "orphan-err";
                    if !pipe_holder(holds_stdout, None) {
                        std::process::exit(4);
                    }
                    if mode == "orphan-out" {
                        std::process::exit(0);
                    }
                    eprint!("failed");
                    std::process::exit(3);
                }
                "orphan-both" => {
                    let (Some(out), Some(err)) = (std::env::var_os(RELEASE_OUT), std::env::var_os(RELEASE_ERR)) else {
                        std::process::exit(4);
                    };
                    if !pipe_holder(true, Some(out)) || !pipe_holder(false, Some(err)) {
                        std::process::exit(4);
                    }
                    eprint!("failed");
                    std::process::exit(3);
                }
                // Holds the pipe it inherited until the test drops its lock on the release file, once
                // the call it checks has returned. It reports [`READY`] on its other pipe once it has
                // found the file locked, and exits without reporting when it cannot.
                "linger-out" | "linger-err" => {
                    let Ok(release) = std::env::var(RELEASE) else {
                        return;
                    };
                    let Ok(file) = std::fs::File::open(release) else {
                        return;
                    };
                    if !matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)) {
                        return;
                    }
                    if mode == "linger-out" {
                        let mut err = std::io::stderr().lock();
                        let _ = writeln!(err, "{READY}").and_then(|()| err.flush());
                    } else {
                        let mut out = std::io::stdout().lock();
                        let _ = writeln!(out, "{READY}").and_then(|()| out.flush());
                    }
                    let _ = file.lock();
                    std::process::exit(0);
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
        fn the_kept_stderr_fits_the_limit_after_decoding() {
            let limits = Limits { stderr_bytes: 1, ..Limits::default() };
            match metadata(|| fake("utf8"), &limits) {
                Err(Error::Exit { stderr, .. }) => assert!(stderr.len() <= 1, "{stderr:?}"),
                other => panic!("expected Exit, got {other:?}"),
            }
        }

        /// Runs `mode` while the test holds a lock on the release file, and drops the lock once the call
        /// has returned.
        fn orphaned(mode: &str) -> Result<Vec<u8>, Error> {
            let dir = tempfile::tempdir().unwrap();
            let release = dir.path().join("release");
            let held = std::fs::File::create(&release).unwrap();
            held.lock().unwrap();
            let r = metadata(
                || {
                    let mut cmd = fake(mode);
                    cmd.env(RELEASE, &release);
                    cmd
                },
                &Limits::default(),
            );
            held.unlock().unwrap();
            r
        }

        #[test]
        fn output_that_stays_open_after_the_command_exits_is_an_error() {
            match orphaned("orphan-out") {
                Err(Error::Read { source, .. }) => assert_eq!(source.kind(), std::io::ErrorKind::TimedOut),
                other => panic!("expected Read, got {other:?}"),
            }
        }

        #[test]
        fn output_that_stays_open_after_a_failing_exit_is_an_error() {
            match orphaned("orphan-out-fail") {
                Err(Error::Read { source, .. }) => assert_eq!(source.kind(), std::io::ErrorKind::TimedOut),
                other => panic!("expected Read, got {other:?}"),
            }
        }

        /// Unix only: on Windows a child inherits every inheritable handle of the process that starts it,
        /// so the `linger` child also holds the command's standard output and the call ends with
        /// [`Error::Read`].
        #[cfg(unix)]
        #[test]
        fn a_failing_command_returns_while_another_process_holds_its_stderr() {
            match orphaned("orphan-err") {
                Err(Error::Exit { status, stderr, .. }) => {
                    assert_eq!(status.code(), Some(3));
                    assert_eq!(stderr, "failed");
                }
                other => panic!("expected Exit, got {other:?}"),
            }
        }

        #[test]
        fn the_stderr_wait_ends_drain_after_the_exit() {
            let exit = std::time::Instant::now();
            let half = DRAIN / 2;
            // Standard output closed half of DRAIN after the exit, so stderr gets only the other half.
            assert_eq!(drain_left(exit, exit + half), DRAIN - half);
            assert_eq!(drain_left(exit, exit), DRAIN);
            assert_eq!(drain_left(exit, exit + DRAIN), std::time::Duration::ZERO);
            assert_eq!(drain_left(exit, exit + DRAIN * 2), std::time::Duration::ZERO);
        }

        #[cfg(unix)]
        #[test]
        fn delayed_stdout_leaves_only_the_remaining_stderr_drain() {
            let dir = tempfile::tempdir().unwrap();
            let (out_path, err_path) = (dir.path().join("stdout"), dir.path().join("stderr"));
            let out = std::fs::File::create(&out_path).unwrap();
            let err = std::fs::File::create(&err_path).unwrap();
            out.lock().unwrap();
            err.lock().unwrap();
            let mut cmd = fake("orphan-both");
            cmd.env(RELEASE_OUT, &out_path).env(RELEASE_ERR, &err_path);
            let exit = std::time::Instant::now();
            let mut calls = 0;
            let result = output_with(
                cmd,
                "fake command",
                64 << 10,
                1000,
                || {
                    calls += 1;
                    if calls == 1 {
                        out.unlock().unwrap();
                        exit
                    } else {
                        exit + DRAIN / 2
                    }
                },
                |head, wait| {
                    err.unlock().unwrap();
                    assert_eq!(wait, DRAIN / 2, "stderr received a second full drain window");
                    head.finish(wait)
                },
            );
            match result {
                Err(Error::Exit { status, stderr, .. }) => {
                    assert_eq!(status.code(), Some(3));
                    assert_eq!(stderr, "failed");
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
