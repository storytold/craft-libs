//! Test-only resolved-edge checks. No Cargo subprocess or global version count.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PackageId {
    name: String,
    version: String,
    source: Option<String>,
}
struct Package {
    id: PackageId,
    dependencies: Vec<String>,
}
pub(crate) struct LockGraph {
    packages: Vec<Package>,
}
// Cargo dependency edges may omit the resolved Git fragment even though the
// selected package ID always records it. Other source identities stay exact.
fn source_matches(edge: &str, package: Option<&str>) -> bool {
    let Some(package) = package else { return false };
    if edge.starts_with("git+") && !edge.contains('#') { package.split_once('#').map_or(package, |(base, _)| base) == edge } else { package == edge }
}
impl LockGraph {
    pub(crate) fn parse(text: &str) -> Self {
        let lock: toml::Value = toml::from_str(text).unwrap();
        let packages = lock
            .get("package")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|package| {
                let string = |key| package.get(key).unwrap().as_str().unwrap().to_owned();
                Package {
                    id: PackageId {
                        name: string("name"),
                        version: string("version"),
                        source: package.get("source").map(|v| v.as_str().unwrap().to_owned()),
                    },
                    dependencies: package
                        .get("dependencies")
                        .map(|d| d.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect())
                        .unwrap_or_default(),
                }
            })
            .collect();
        Self { packages }
    }
    pub(crate) fn workspace_package(&self, name: &str) -> PackageId {
        let found: Vec<_> = self.packages.iter().filter(|p| p.id.name == name && p.id.source.is_none()).collect();
        assert_eq!(found.len(), 1, "one exact workspace package required: {name}");
        found[0].id.clone()
    }
    pub(crate) fn dependency(&self, from: &PackageId, name: &str) -> PackageId {
        let package = self.packages.iter().find(|p| &p.id == from).unwrap();
        let edges: Vec<_> = package.dependencies.iter().filter(|edge| edge.split_whitespace().next() == Some(name)).collect();
        assert_eq!(edges.len(), 1, "one {name} dependency edge required from {from:?}");
        let fields: Vec<_> = edges[0].split_whitespace().collect();
        let version = fields.get(1).copied();
        let source = fields.get(2).map(|s| s.trim_start_matches('(').trim_end_matches(')'));
        let found: Vec<_> = self
            .packages
            .iter()
            .filter(|p| {
                p.id.name == name && version.is_none_or(|v| p.id.version == v) && source.is_none_or(|s| source_matches(s, p.id.source.as_deref()))
            })
            .collect();
        assert_eq!(found.len(), 1, "ambiguous or absent resolved edge {} from {from:?}", edges[0]);
        found[0].id.clone()
    }
}

pub(crate) fn assert_renderer_edges(graph: &LockGraph, svg: &PackageId) {
    let renderer = graph.dependency(svg, "resvg");
    assert_eq!(graph.dependency(svg, "usvg"), graph.dependency(&renderer, "usvg"), "shared and renderer trees must be the same resolved package");
    for codec in ["gif", "image-webp"] {
        assert_eq!(graph.dependency(svg, codec), graph.dependency(&renderer, codec), "validator and selected renderer differ for {codec}");
    }
    let codecs = graph.dependency(svg, "craft-image-codecs");
    assert_eq!(
        graph.dependency(&codecs, "zune-jpeg"),
        graph.dependency(&renderer, "zune-jpeg"),
        "JPEG validator and renderer must use the same backend"
    );
}

#[test]
fn validated_renderer_dependency_edges_are_the_same_resolved_packages() {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let graph = LockGraph::parse(&std::fs::read_to_string(file).unwrap());
    assert_renderer_edges(&graph, &graph.workspace_package("craft-svg"));
}

#[test]
fn dependency_lookup_follows_the_selected_edge_and_rejects_ambiguity() {
    let graph = LockGraph::parse(
        r#"
[[package]]
name = "consumer"
version = "1"
dependencies = ["codec 1 (registry+one)"]
[[package]]
name = "codec"
version = "1"
source = "registry+one"
[[package]]
name = "codec"
version = "1"
source = "registry+two"
[[package]]
name = "other"
version = "1"
dependencies = ["codec"]
"#,
    );
    let selected = graph.dependency(&graph.workspace_package("consumer"), "codec");
    assert_eq!(selected.source.as_deref(), Some("registry+one"));
    assert!(std::panic::catch_unwind(|| graph.dependency(&graph.workspace_package("other"), "codec")).is_err());
}

#[test]
fn selected_renderer_graph_rejects_a_split_validator_backend() {
    use std::fmt::Write;
    let make = |split: bool| {
        let mut text = String::from(
            "[[package]]\nname='craft-svg'\nversion='1'\ndependencies=['resvg', 'usvg', 'gif', 'image-webp', 'craft-image-codecs']\n\
             [[package]]\nname='craft-image-codecs'\nversion='1'\ndependencies=['",
        );
        text.push_str(if split { "zune-jpeg 2" } else { "zune-jpeg 1" });
        text.push_str(
            "']\n[[package]]\nname='resvg'\nversion='1'\nsource='registry+one'\ndependencies=['usvg', 'gif', 'image-webp', 'zune-jpeg 1']\n",
        );
        for name in ["usvg", "gif", "image-webp", "zune-jpeg"] {
            write!(&mut text, "[[package]]\nname='{name}'\nversion='1'\nsource='registry+one'\n").unwrap();
        }
        // An unrelated second codec version is valid; only the selected edges count.
        text.push_str("[[package]]\nname='zune-jpeg'\nversion='2'\nsource='registry+one'\n");
        LockGraph::parse(&text)
    };
    let matching = make(false);
    assert_renderer_edges(&matching, &matching.workspace_package("craft-svg"));
    let split = make(true);
    assert!(std::panic::catch_unwind(|| assert_renderer_edges(&split, &split.workspace_package("craft-svg"))).is_err());
}

#[test]
fn git_edges_resolve_full_package_ids_and_reject_canonical_ambiguity() {
    let fixture = |second: bool| {
        let mut text = String::from(
            r#"
[[package]]
name = "consumer"
version = "1"
dependencies = ["codec 1 (git+https://example.invalid/codecs?rev=chosen)"]
[[package]]
name = "explicit"
version = "1"
dependencies = ["codec 1 (git+https://example.invalid/codecs?rev=chosen#aaaa)"]
[[package]]
name = "wrong"
version = "1"
dependencies = ["codec 1 (git+https://example.invalid/codecs?rev=chosen#cccc)"]
[[package]]
name = "codec"
version = "1"
source = "git+https://example.invalid/codecs?rev=chosen#aaaa"
"#,
        );
        if second {
            text.push_str(
                r#"
[[package]]
name = "codec"
version = "1"
source = "git+https://example.invalid/codecs?rev=chosen#bbbb"
"#,
            );
        }
        LockGraph::parse(&text)
    };
    let unique = fixture(false);
    let selected = unique.dependency(&unique.workspace_package("consumer"), "codec");
    assert_eq!(selected.source.as_deref(), Some("git+https://example.invalid/codecs?rev=chosen#aaaa"));
    assert_eq!(selected, unique.dependency(&unique.workspace_package("explicit"), "codec"));
    assert!(std::panic::catch_unwind(|| unique.dependency(&unique.workspace_package("wrong"), "codec")).is_err());
    let ambiguous = fixture(true);
    assert!(std::panic::catch_unwind(|| ambiguous.dependency(&ambiguous.workspace_package("consumer"), "codec")).is_err());
    assert_eq!(selected, ambiguous.dependency(&ambiguous.workspace_package("explicit"), "codec"));
    assert!(!source_matches("registry+one", Some("registry+one#aaaa")), "registry sources remain exact");
    assert!(
        !source_matches("git+https://example.invalid/codecs?rev=other", selected.source.as_deref()),
        "requested Git revision remains part of the identity"
    );
}
