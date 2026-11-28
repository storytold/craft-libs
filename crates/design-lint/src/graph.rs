//! The crate-graph test and the features test, from `cargo metadata --no-deps`.
//!
//! Both read normal dependencies and declared features of the workspace crates and return the breaches as
//! [`Site`]s, so named, expiring exceptions can excuse them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::Error;
use crate::exceptions::Site;
use crate::layers::{LayerMaps, Row};

/// A dependency on a crate whose lowest layer is above the depender's highest layer.
pub const GRAPH_LAYER: &str = "graph/layer";
/// A tokens-row crate of layer 0 depending on a services-row crate.
pub const GRAPH_ROW: &str = "graph/tokens-services";
/// A crate of layers 0 and 1 depending on an egui crate (or, in layer 0, on `accesskit`).
pub const GRAPH_EGUI: &str = "graph/egui";
/// A Cargo feature that gates no dependency.
pub const FEATURES_GATE: &str = "features/gate";

/// Crates of the egui family that layers 0 and 1 may depend on.
const EGUI_FREE: [&str; 2] = ["emath", "ecolor"];

/// Other crates layers 0 and 1 may not depend on.
const DENIED_EXACT: [&str; 3] = ["eframe", "winit", "wgpu"];

/// The data crate layer 1 may depend on and layer 0 may not.
const ACCESSKIT: &str = "accesskit";

/// What the checks read from `cargo metadata --no-deps`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Metadata {
    /// The workspace crates.
    pub packages: Vec<Package>,
}

/// A workspace crate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Package {
    /// Package name.
    pub name: String,
    /// The directory of the crate's `Cargo.toml`, when the metadata names the manifest.
    pub dir: Option<PathBuf>,
    /// The library target's root file, when the crate has one.
    pub lib_root: Option<PathBuf>,
    /// Declared features and what each enables.
    pub features: BTreeMap<String, Vec<String>>,
    /// Declared dependencies.
    pub dependencies: Vec<Dependency>,
}

/// One declared dependency.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dependency {
    /// Package name of the dependency.
    pub name: String,
    /// Whether it is a normal dependency (not dev or build).
    pub normal: bool,
    /// Whether it is optional.
    pub optional: bool,
    /// The features of the dependency this declaration enables.
    pub features: Vec<String>,
    /// Whether the declaration keeps the dependency's default features.
    pub default_features: bool,
}

#[derive(Deserialize)]
struct RawMetadata {
    packages: Vec<RawPackage>,
}

#[derive(Deserialize)]
struct RawPackage {
    name: String,
    manifest_path: Option<PathBuf>,
    #[serde(default)]
    features: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
    #[serde(default)]
    targets: Vec<RawTarget>,
}

#[derive(Deserialize)]
struct RawDependency {
    name: String,
    kind: Option<String>,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default = "yes")]
    uses_default_features: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct RawTarget {
    kind: Vec<String>,
    src_path: PathBuf,
}

impl Metadata {
    /// Runs `cargo metadata --no-deps` for the workspace at `root`.
    pub fn run(root: &Path) -> Result<Self, Error> {
        let manifest = root.join("Cargo.toml");
        let output = Command::new("cargo")
            .args(["metadata", "--no-deps", "--format-version", "1", "--manifest-path"])
            .arg(&manifest)
            .output()
            .map_err(|source| Error::Io { path: manifest.clone(), source })?;
        if !output.status.success() {
            return Err(Error::Toml { path: manifest, message: String::from_utf8_lossy(&output.stderr).into_owned() });
        }
        Self::parse(root, &String::from_utf8_lossy(&output.stdout))
    }

    /// Parses `cargo metadata` JSON; relative target paths are taken from `root`.
    pub fn parse(root: &Path, json: &str) -> Result<Self, Error> {
        let raw: RawMetadata = serde_json::from_str(json).map_err(|e| Error::Toml { path: root.join("metadata.json"), message: e.to_string() })?;
        let packages = raw.packages.into_iter().map(|p| package(root, p)).collect();
        Ok(Self { packages })
    }

    pub(crate) fn package(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.name == name)
    }
}

fn package(root: &Path, raw: RawPackage) -> Package {
    let lib_root = raw.targets.iter().find(|t| t.kind.iter().any(|k| k == "lib" || k == "rlib")).map(|t| root.join(&t.src_path));
    let dependencies = raw
        .dependencies
        .into_iter()
        .map(|d| Dependency {
            name: d.name,
            normal: d.kind.is_none(),
            optional: d.optional,
            features: d.features,
            default_features: d.uses_default_features,
        })
        .collect();
    let dir = raw.manifest_path.and_then(|m| m.parent().map(|d| root.join(d)));
    Package { name: raw.name, dir, lib_root, features: raw.features, dependencies }
}

fn denied_egui(name: &str) -> bool {
    !EGUI_FREE.contains(&name) && (DENIED_EXACT.contains(&name) || name.starts_with("egui") || name.starts_with("epaint"))
}

/// The crate-graph breaches: layer order, tokens before services in layer 0, and egui below layer 2.
///
/// Only normal dependencies count. Crates without a layer map are not judged.
pub fn check_graph(meta: &Metadata, maps: &LayerMaps) -> Vec<Site> {
    let mut sites = Vec::new();
    for package in &meta.packages {
        let Some((from, from_highest)) = maps.get(&package.name).and_then(|c| Some((c, c.range?.1))) else { continue };
        for dep in package.dependencies.iter().filter(|d| d.normal) {
            let subject = format!("{}->{}", package.name, dep.name);
            if from_highest <= 1 && (denied_egui(&dep.name) || (from_highest == 0 && dep.name == ACCESSKIT)) {
                sites.push(Site::violation(GRAPH_EGUI, subject));
                continue;
            }
            let Some((to, to_lowest)) = maps.get(&dep.name).and_then(|c| Some((c, c.range?.0))) else { continue };
            if to_lowest > from_highest {
                sites.push(Site::violation(GRAPH_LAYER, subject));
            } else if from.row == Some(Row::Tokens) && to.row == Some(Row::Services) {
                sites.push(Site::violation(GRAPH_ROW, subject));
            }
        }
    }
    sites
}

/// Features that gate no dependency. A feature is allowed when it enables an optional dependency, directly
/// or through another feature, or forwards to a feature of a workspace crate that is allowed. A feature that gates
/// nothing itself is also allowed when an allowed feature of another workspace crate forwards to it: the lower
/// crate of a split keeps the cfg of a feature whose dependency the crate above it enables.
pub fn check_features(meta: &Metadata) -> Vec<Site> {
    let mut sites = Vec::new();
    for package in &meta.packages {
        for feature in package.features.keys().filter(|f| *f != "default") {
            if !allowed(meta, package, feature, &mut BTreeSet::new()) {
                sites.push(Site::violation(FEATURES_GATE, format!("{}/{feature}", package.name)));
            }
        }
    }
    sites
}

/// Whether `feature` of `package` gates a dependency, or an allowed feature of another workspace crate forwards
/// to it. `visiting` holds the features already on the forwarding chain, so a cycle allows nothing.
fn allowed(meta: &Metadata, package: &Package, feature: &str, visiting: &mut BTreeSet<(String, String)>) -> bool {
    if !visiting.insert((package.name.clone(), feature.to_owned())) {
        return false;
    }
    // The features on the forwarding chain may not count as this feature's gate, or each would allow the other.
    let mut chain = visiting.clone();
    chain.remove(&(package.name.clone(), feature.to_owned()));
    if gates(meta, package, feature, &mut chain) {
        return true;
    }
    meta.packages.iter().filter(|upper| upper.name != package.name).any(|upper| {
        upper.features.iter().any(|(name, members)| {
            members.iter().any(|m| m.split_once('/').is_some_and(|(dep, sub)| dep.trim_end_matches('?') == package.name && sub == feature))
                && allowed(meta, upper, name, visiting)
        })
    })
}

fn gates(meta: &Metadata, package: &Package, feature: &str, visiting: &mut BTreeSet<(String, String)>) -> bool {
    if !visiting.insert((package.name.clone(), feature.to_owned())) {
        return false;
    }
    let optional = |name: &str| package.dependencies.iter().any(|d| d.name == name && d.optional);
    let members = package.features.get(feature).map_or(&[][..], Vec::as_slice);
    members.iter().any(|member| {
        if let Some(dep) = member.strip_prefix("dep:") {
            return optional(dep);
        }
        match member.split_once('/') {
            Some((dep, sub)) => {
                let dep = dep.trim_end_matches('?');
                optional(dep) || meta.package(dep).is_some_and(|q| gates(meta, q, sub, visiting))
            }
            None if package.features.contains_key(member) => gates(meta, package, member, visiting),
            None => optional(member),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flagged(json: &str) -> Vec<String> {
        check_features(&Metadata::parse(Path::new("."), json).unwrap()).into_iter().map(|s| s.subject).collect()
    }

    #[test]
    fn a_feature_an_upper_crates_gating_feature_forwards_to_is_allowed() {
        let json = r#"{"packages":[
            {"name":"lower","features":{"testing":[],"loose":[]},"dependencies":[],"targets":[]},
            {"name":"upper","features":{"testing":["lower/testing","dep:harness"],"echo":["lower/loose"]},
             "dependencies":[{"name":"lower","kind":null,"optional":false},{"name":"harness","kind":null,"optional":true}],"targets":[]}
        ]}"#;
        assert_eq!(flagged(json), ["lower/loose", "upper/echo"]);
    }
}
