//! Design-system lint for Craft apps.
//!
//! The clippy half is the template `clippy.toml` entries ([`CLIPPY_TEMPLATE`]),
//! the check that an app's `clippy.toml` carries them ([`check_clippy`]) and the
//! writer that merges them in ([`write_clippy`]). Apps call these from their
//! `xtask`; the binary exposes the same operations.
//!
//! The syntax half is [`check`]: it parses the configured directories and reports
//! literal sizes, named colour constants, style mutation and crate-wide clippy allows as a [`Report`].
//! A [`Baseline`] records the findings of an existing code base so a check fails only on new ones.

mod baseline;
mod clippy;
mod config;
mod contract;
mod markers;
mod report;
mod rules;

// --- structure checks (layer map, exceptions, crate graph, clones): one `mod` and `pub use` line per concern ---
mod clones;
mod exceptions;
mod graph;
mod layers;
mod resolve;
mod structure;
mod workspace;

pub use clones::{Family, check_clones, load_families};
pub use exceptions::{Entry, Exceptions, Origin, Site, scan_sites};
pub use graph::{Metadata, check_features, check_graph};
pub use layers::{CrateLayers, LayerInfo, LayerMaps, ModuleLayer, Placement};
pub use structure::{check_structure, exempt_sites, run_structure};
pub use workspace::{WorkspaceScope, check_workspace};
// --- end of structure checks ---

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use baseline::{Baseline, BaselineCheck, Increase};
pub use clippy::{ClippyDrift, ClippyShadow, DriftKind, check_clippy, clippy_drift, clippy_shadows, merge_clippy, write_clippy};
pub use config::{Config, Exemption};
pub use contract::{RuleScope, StaticScope, Types, analyze_contract};
use markers::FilePlace;
pub use markers::{Marked, analyze_with_markers};
pub use report::{Finding, Report, Severity};
use resolve::Parsed;
pub use rules::{Analysis, RULES, analyze_source};

/// The `disallowed-methods` and `disallowed-types` entries every app's `clippy.toml` carries.
pub const CLIPPY_TEMPLATE: &str = include_str!("../clippy.template.toml");

/// The `[workspace.lints.clippy]` lines an app adds to its workspace `Cargo.toml`.
pub const LINTS_TEMPLATE: &str = include_str!("../lints.template.toml");

/// Reported for an `[[exempt]]` that matches no checked file or silences no finding.
pub const UNUSED_EXEMPTION: &str = "exemption/unused";

/// Checks the configured paths (relative to `root`) and returns the findings.
///
/// Test files and `#[cfg(test)]` code are skipped unless the config includes them.
/// `// design: allow(<reason>)` markers and `[[exempt]]` paths silence findings; see [`analyze_with_markers`].
/// A file that does not parse is a `parse` finding that nothing silences; an unreadable path is an error.
/// An exemption that matches no checked file or silences nothing is reported as `exemption/unused`
/// (a warning, an error with `deny_unused`). Each file is parsed once.
pub fn check(config: &Config, root: &Path) -> Result<Report, Error> {
    check_with(config, root, None, &BTreeMap::new())
}

/// [`check`] plus the egui-ban and API-shape rules (`contract/*`, `test/*`, `api/non-exhaustive`),
/// with their "allowed in" locations read from `scope`. Markers and exemptions apply to their findings.
pub fn check_with_scope(config: &Config, root: &Path, scope: &dyn RuleScope) -> Result<Report, Error> {
    check_with(config, root, Some(scope), &BTreeMap::new())
}

/// The checks of [`check`], with the scoped rules when `scope` is given. A file in `parsed` (keyed by canonical
/// path) is read from there; any other file is read and parsed here.
fn check_with(config: &Config, root: &Path, scope: Option<&dyn RuleScope>, parsed: &BTreeMap<PathBuf, Parsed>) -> Result<Report, Error> {
    let mut report = Report::default();
    let hits = RefCell::new(vec![0_usize; config.exempt.len()]);
    let mut matched_files = vec![0_usize; config.exempt.len()];
    for entry in &config.paths {
        let base = root.join(entry);
        let mut files = Vec::new();
        collect_rust_files(&base, &mut files)?;
        for file in files {
            if !config.include_tests && is_test_path(file.strip_prefix(root).unwrap_or(&file)) {
                continue;
            }
            let known = if parsed.is_empty() { None } else { std::fs::canonicalize(&file).ok().and_then(|c| parsed.get(&c)) };
            let read;
            let (src, tree) = match known {
                Some(p) => (p.src.as_str(), Ok(&p.file)),
                None => {
                    let src = std::fs::read_to_string(&file).map_err(|source| Error::Io { path: file.clone(), source })?;
                    let tree = syn::parse_file(&src);
                    read = (src, tree);
                    (read.0.as_str(), read.1.as_ref())
                }
            };
            report.files += 1;
            let area = area_of(&base, &file);
            let shown = file.strip_prefix(root).unwrap_or(&file).to_owned();
            for (n, e) in matched_files.iter_mut().zip(&config.exempt) {
                *n += usize::from(shown.starts_with(&e.path));
            }
            let exempt = |rule: &str| {
                let mut any = false;
                for (n, e) in hits.borrow_mut().iter_mut().zip(&config.exempt) {
                    if e.covers(&shown, rule) {
                        *n += 1;
                        any = true;
                    }
                }
                any
            };
            let extra = |parsed: &syn::File| scope.map_or_else(Vec::new, |s| contract::analyze_file(parsed, &shown, s, config.include_tests));
            let place = scope.map_or_else(FilePlace::default, |s| FilePlace {
                style_writer: s.allows(rules::STYLE_MUTATION, &shown),
                tests_scoped: true,
                test_file: {
                    let relative = file.strip_prefix(root).unwrap_or(&file);
                    is_test_path(relative) || is_testing_module(relative)
                },
            });
            let marked = markers::analyze_with_extra(src, tree, config.include_tests, config.deny_unused, &exempt, &extra, place);
            report.markers += marked.markers;
            report.clippy_allows += marked.clippy_allows;
            for mut finding in marked.findings {
                finding.path.clone_from(&shown);
                finding.area.clone_from(&area);
                if config.warn.iter().any(|w| rules::selector_matches(w, finding.rule)) {
                    finding.severity = Severity::Warning;
                }
                report.findings.push(finding);
            }
        }
    }
    let severity = if config.deny_unused { Severity::Error } else { Severity::Warning };
    for ((exemption, files), hits) in config.exempt.iter().zip(&matched_files).zip(hits.into_inner()) {
        let message = match (files, hits) {
            (0, _) => format!("exemption for `{}` matches no checked file; fix the path or remove it", exemption.path.display()),
            (_, 0) => format!("exemption for `{}` silences no finding; remove it", exemption.path.display()),
            _ => continue,
        };
        let mut finding = Finding::new(UNUSED_EXEMPTION, 1, 1, 1, message);
        finding.path.clone_from(&exemption.path);
        finding.area = "(config)".to_owned();
        finding.severity = severity;
        report.findings.push(finding);
    }
    report.exemptions.clone_from(&config.exempt);
    report.findings.sort_by(|a, b| (&a.path, a.line, a.column).cmp(&(&b.path, b.line, b.column)));
    Ok(report)
}

/// Test files: `*_test.rs`, `*_tests.rs`, `tests.rs`, and anything under `tests/` or `benches/`. `relative` is the
/// path from the workspace root, so a configured entry inside `tests/` (such as `crates/x/tests/it`) is still found.
fn is_test_path(relative: &Path) -> bool {
    let in_test_dir = relative.parent().is_some_and(|p| p.components().any(|c| matches!(c.as_os_str().to_str(), Some("tests" | "benches"))));
    let name = relative.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    in_test_dir || name == "tests.rs" || name.ends_with("_test.rs") || name.ends_with("_tests.rs")
}

/// A file of a `testing` module directory below `src/` (`src/testing/`, `src/canvas/testing/`): the test kit behind
/// feature `testing`, which literal sizes may fill like `tests/`.
fn is_testing_module(relative: &Path) -> bool {
    let parts: Vec<_> = relative.components().filter_map(|c| c.as_os_str().to_str()).collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    dirs.iter().position(|part| *part == "src").is_some_and(|src| dirs[src + 1..].contains(&"testing"))
}

fn area_of(base: &Path, file: &Path) -> String {
    let relative = file.strip_prefix(base).unwrap_or(file);
    let mut parts = relative.components();
    match (parts.next(), parts.next()) {
        (Some(first), Some(_)) => first.as_os_str().to_string_lossy().into_owned(),
        _ => "(root)".to_owned(),
    }
}

/// Every `.rs` file under `path` (or `path` itself), sorted, skipping hidden directories and `target`.
fn collect_rust_files(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), Error> {
    let io = |source| Error::Io { path: path.to_owned(), source };
    if path.is_file() {
        out.push(path.to_owned());
        return Ok(());
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(path).map_err(io)?.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>().map_err(io)?;
    entries.sort();
    for entry in entries {
        let name = entry.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if entry.is_dir() {
            if !name.starts_with('.') && name != "target" {
                collect_rust_files(&entry, out)?;
            }
        } else if entry.extension().is_some_and(|e| e == "rs") {
            out.push(entry);
        }
    }
    Ok(())
}

/// Why a command could not run to completion.
#[derive(Debug)]
pub enum Error {
    /// A file could not be read or written.
    Io {
        /// The file.
        path: std::path::PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A TOML file does not parse or does not have the expected shape.
    Toml {
        /// The file.
        path: std::path::PathBuf,
        /// The parser's message.
        message: String,
    },
    /// The app's `clippy.toml` lacks or differs from template entries.
    ClippyDrift(Vec<ClippyDrift>),
    /// Another `clippy.toml`, or `CLIPPY_CONF_DIR`, makes clippy skip the app's one.
    ClippyShadowed(Vec<ClippyShadow>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Toml { path, message } => write!(f, "{}: {message}", path.display()),
            Self::ClippyShadowed(shadows) => {
                for s in shadows {
                    writeln!(f, "{s}")?;
                }
                write!(f, "design-lint: clippy would not read the root clippy.toml for {} location(s)", shadows.len())
            }
            Self::ClippyDrift(drift) => {
                for d in drift {
                    writeln!(f, "{d}")?;
                }
                write!(
                    f,
                    "design-lint: clippy.toml is out of step with the template in {} entries; run `craft-design-lint clippy --write`",
                    drift.len()
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
extern crate self as craft_design_lint;

#[cfg(test)]
#[path = "../tests/main.rs"]
mod integration;
