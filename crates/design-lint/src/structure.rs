//! The structure checks as one run over a repository: layer maps, imports, exceptions with the ratchet,
//! the crate graph, features and clones.
//!
//! The repository keeps its configuration under `design/`:
//!
//! | Path | Holds |
//! |---|---|
//! | `design/layers/<crate>/crate.toml`, `<family>.toml` | the layer map ([`crate::layers`]) |
//! | `design/exceptions/<crate>/<family>.toml`, `milestones.toml` | exception entries and landed milestones ([`crate::exceptions`]) |
//! | `design/clones/<crate>/<family>.toml` | clone baselines ([`crate::clones`]) |

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::Error;
use crate::clones::{check_clones, load_families};
use crate::exceptions::{Exceptions, Origin, Site, scan_parsed};
use crate::graph::{Metadata, check_features, check_graph};
use crate::layers::{LayerMaps, finding, relative};
use crate::report::{Finding, Severity};
use crate::resolve::{Index, check_coverage, check_imports};

/// Where the layer maps live, relative to the repository root.
pub const LAYERS_DIR: &str = "design/layers";
/// Where the exception files live.
pub const EXCEPTIONS_DIR: &str = "design/exceptions";
/// Where the clone baselines live.
pub const CLONES_DIR: &str = "design/clones";

/// A layer map for a crate the workspace does not have.
pub const UNKNOWN_CRATE: &str = "layer/map";

/// Runs every structure check on the repository at `root`.
///
/// `extra_sites` are allowances found elsewhere (the `[[exempt]]` entries of a `design-lint.toml`, see
/// [`exempt_sites`]). Findings are sorted by path and line.
pub fn check_structure(root: &Path, meta: &Metadata, extra_sites: &[Site]) -> Result<Vec<Finding>, Error> {
    let (maps, findings) = LayerMaps::load(root, &root.join(LAYERS_DIR))?;
    let index = Index::build(&mapped_crates(meta, &maps))?;
    structure_findings(root, meta, &maps, &index, findings, extra_sites)
}

/// The `(package name, lib.rs)` of each workspace crate with a layer map: the crates the module index parses.
pub(crate) fn mapped_crates(meta: &Metadata, maps: &LayerMaps) -> Vec<(String, PathBuf)> {
    meta.packages.iter().filter(|p| maps.get(&p.name).is_some()).filter_map(|p| p.lib_root.clone().map(|r| (p.name.clone(), r))).collect()
}

/// [`check_structure`] with the layer maps and the index of [`mapped_crates`] already loaded; `findings` are the
/// maps' own.
pub(crate) fn structure_findings(
    root: &Path,
    meta: &Metadata,
    maps: &LayerMaps,
    index: &Index,
    mut findings: Vec<Finding>,
    extra_sites: &[Site],
) -> Result<Vec<Finding>, Error> {
    let (exceptions, mut invalid) = Exceptions::load(root, &root.join(EXCEPTIONS_DIR))?;
    findings.append(&mut invalid);
    findings.extend(unknown_crates(root, meta, maps));

    findings.extend(check_coverage(index, maps, root));
    findings.extend(check_imports(index, maps, root));

    let mut sites = extra_sites.to_vec();
    sites.extend(source_sites(root, index));
    sites.extend(check_graph(meta, maps));
    sites.extend(check_features(meta).into_iter().filter(|s| maps.get(s.subject.split('/').next().unwrap_or_default()).is_some()));
    findings.extend(exceptions.apply(&sites));

    findings.extend(check_clones(root, &load_families(root, &root.join(CLONES_DIR))?)?);
    findings.sort_by(|a, b| (&a.path, a.line, a.rule).cmp(&(&b.path, b.line, b.rule)));
    Ok(findings)
}

fn unknown_crates(root: &Path, meta: &Metadata, maps: &LayerMaps) -> Vec<Finding> {
    maps.crates()
        .filter(|c| !meta.packages.iter().any(|p| p.name == c.name))
        .map(|c| {
            finding(
                UNKNOWN_CRATE,
                relative(root, &root.join(LAYERS_DIR).join(&c.name)),
                format!("layer map for `{}`, which is not a workspace crate", c.name),
            )
        })
        .collect()
}

fn source_sites(root: &Path, index: &Index) -> Vec<Site> {
    // Source paths come from cargo metadata, which resolves `..` and links; the root must be resolved the same way.
    let root = &std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    let mut sites = Vec::new();
    for tree in index.trees.values() {
        let mut files: Vec<&PathBuf> = tree.modules.values().map(|m| &m.file).collect();
        files.sort();
        files.dedup();
        for file in files {
            if let Some(parsed) = index.files.get(file) {
                sites.extend(scan_parsed(&relative(root, file).to_string_lossy(), &parsed.src, Some(&parsed.file)));
            }
        }
    }
    sites
}

/// One allowance site per rule of each `[[exempt]]` entry of `config` (`*` for an entry without rules).
pub fn exempt_sites(config: &crate::Config) -> Vec<Site> {
    let mut sites = Vec::new();
    for e in &config.exempt {
        let subject = e.path.to_string_lossy().replace('\\', "/");
        let rules = if e.rules.is_empty() { vec!["*".to_owned()] } else { e.rules.clone() };
        sites.extend(rules.into_iter().map(|rule| Site { rule, subject: subject.clone(), line: 1, origin: Origin::Allowance }));
    }
    sites
}

const USAGE: &str = "usage: craft-design-lint structure [--root <dir>] [--config <design-lint.toml>]
  runs the layer-map, import, exception, crate-graph, features and clone checks of the repository at <dir> (default .);
  with --config it also runs the egui-ban and API rules over the config's paths (relative to <dir>)";

/// The `structure` command: arguments after the command name.
pub fn run_structure(args: &[String]) -> Result<ExitCode, Error> {
    let mut root = PathBuf::from(".");
    let mut config = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let value = args.next().map(PathBuf::from);
        match (arg.as_str(), value) {
            ("--root", Some(v)) => root = v,
            ("--config", Some(v)) => config = Some(v),
            _ => {
                eprintln!("{USAGE}");
                return Ok(ExitCode::from(2));
            }
        }
    }
    let meta = Metadata::run(&root)?;
    let findings = match config {
        Some(path) => crate::check_workspace(&root, &crate::Config::load(&path)?, &meta)?,
        None => check_structure(&root, &meta, &[])?,
    };
    for f in &findings {
        println!("{f}");
    }
    let errors = findings.iter().filter(|f| f.severity == Severity::Error).count();
    println!("design-lint structure: {errors} error(s)");
    Ok(ExitCode::from(u8::from(errors > 0)))
}
