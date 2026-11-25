//! The workspace's [`RuleScope`]: the layer map, `cargo metadata` and the allowances of spec 24.1 in one place.
//!
//! Paths the rules ask about are relative to the repository root. A file's module comes from the module
//! tree of its crate (`Index`), its layer from the layer map, and the egui facts from the crates'
//! dependencies and `[[test]]` tables.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::contract::{FS, RuleScope};
use crate::graph::{Metadata, Package};
use crate::layers::{LayerMaps, relative};
use crate::resolve::Index;
use crate::structure::{LAYERS_DIR, mapped_crates, structure_findings};
use crate::{Config, Error, Finding, exempt_sites};

/// Types whose cases or fields are fixed (spec 24.1, `api/non-exhaustive`): by meaning, or because a higher crate
/// of the library builds them with a struct expression or matches them exhaustively.
const CLOSED: &[&str] = &[
    // Fixed by meaning.
    "Mixed",
    "Politeness",
    "Numeric",
    "HitPriority",
    "ScrollPos",
    "PartRef",
    "SetPosition",
    "States",
    "StateMix",
    "ShowReadouts",     // the one Show Transformation Values flag: a bool, fixed by meaning
    "PenDraft",         // whose anchors a pen draft holds: own or foreign, fixed by meaning
    "CollectionEvent",  // built or matched in craft-ui-patterns
    "ColumnAlign",      // built or matched in craft-ui-widgets
    "ColumnEvent",      // built or matched in craft-ui-widgets
    "DialogOutput",     // built or matched in craft-ui-canvas
    "DocText",          // built or matched in craft-ui-widgets
    "DockEvent",        // built or matched in craft-ui-patterns
    "Drag",             // built or matched in craft-ui-canvas
    "Drive",            // built or matched in craft-ui-widgets
    "FirePhase",        // built or matched in craft-ui-canvas
    "FocusPolicy",      // built or matched in craft-ui-canvas
    "GridEvent",        // built or matched in craft-ui-patterns
    "GroupAt",          // built or matched in craft-ui-patterns
    "GroupItem",        // built or matched in craft-ui-patterns
    "Guide",            // built or matched in craft-ui-canvas
    "GuideColor",       // built or matched in craft-ui-patterns
    "GuideLine",        // built or matched in craft-ui-canvas
    "HAlign",           // built or matched in craft-ui-widgets
    "HostAnswer",       // built or matched in craft-ui-patterns
    "IconSpec",         // built or matched in craft-ui-widgets
    "Inner",            // built or matched in craft-ui-panels
    "MenuKey",          // built or matched in craft-ui-widgets
    "Mount",            // built or matched in craft-ui-panels
    "OpenChange",       // built or matched in craft-ui-patterns
    "OuterEdit",        // built or matched in craft-ui-widgets
    "Pending",          // built or matched in craft-ui-canvas
    "ProxyColor",       // built or matched in craft-ui-patterns
    "ReorderMove",      // built or matched in craft-ui-patterns
    "RowState",         // built or matched in craft-ui-widgets
    "RuleLook",         // built or matched in craft-ui-canvas
    "RulerFace",        // matched by every ruler (craft-text, craft-pages, craft-ui-canvas): a new face must be drawn by each
    "ShellRegions",     // built or matched in craft-ui-shell
    "SizeLimits",       // built or matched in craft-ui-panels
    "StoryOutput",      // built or matched in craft-ui-widgets
    "SurfaceStyle",     // built or matched in craft-ui-widgets
    "Target",           // built or matched in craft-ui-canvas
    "TextOptions",      // built or matched in craft-ui-widgets
    "TransformReach",   // `MoveOnly { points }` is built in PhotoCraft's type tool
    "GalleryItem",      // built in WordCraft's ui-egui, one literal per tile, with its key, label and face
    "RibbonOutput",     // built in WordCraft's ui-egui, which reports an empty ribbon output while no ribbon shows
    "LineBreakOptions", // built field by field in WordCraft's paragraph dialog, a record of check boxes that mirrors the format
    "AsianTypography",  // built field by field in WordCraft's paragraph dialog, a record of check boxes that mirrors the format
    "GapEdge",          // the start and the end of a column gap: two edges, fixed by meaning, matched in WordCraft's column ruler
    "StrokePlace",      // inside, centred on or outside an edge: three places, fixed by meaning
    "PagerStep",        // back or next: two steps, fixed by meaning, matched in craft-pages, craft-print and PdfCraft
    "PanePager",        // built in craft-pages, craft-print and PdfCraft's print and printer-marks dialogs
    "PageRevision",     // built in PdfCraft's ui-egui, which keys its page pictures by the document and page revision
    "GoTo",             // built in PdfCraft's ui-egui (link and zoom navigation) and craft-pages' tests, one per target kind
    "NotePopupMessage", // built in PdfCraft's ui-egui, one literal per message of a note's thread
    "DialogLayout",     // built in PdfCraft's ui-egui, which sets the dialogs' layout environment value
    "RunWord",          // built in PdfCraft's ui-egui, one literal per word of an OCR result
    "ScanEvent",        // matched in PdfCraft's ui-egui, whose scan tab answers each event
    "FolderListing",    // built in PdfCraft's ui-egui and craft-ui-shell's tests, one listing per pinned folder
    "ToastEnd",         // built or matched in craft-ui-widgets
    "TrackLook",        // built or matched in craft-ui-widgets
    "VariantParts",     // built or matched in craft-ui-widgets
];

/// Permission keys whose only allowed location is craft-ui-contract's `runtime::egui`.
const EGUI_KEYS: [&str; 9] = [
    "contract/egui-import",
    "contract/allocate",
    "contract/egui-widget",
    "contract/access",
    "contract/keys",
    "contract/pointer",
    "contract/motion",
    "contract/containers",
    "contract/platform",
];

/// One allowed location: files of `krate` whose module path starts with `module` (the whole crate when empty).
struct Place {
    krate: &'static str,
    module: &'static [&'static str],
}

const fn place(krate: &'static str, module: &'static [&'static str]) -> Place {
    Place { krate, module }
}

/// The locations that may use what `key` forbids. A key not listed here is allowed nowhere.
fn places(key: &str) -> Vec<Place> {
    match key {
        k if EGUI_KEYS.contains(&k) => vec![place("craft-ui-contract", &["runtime", "egui"])],
        "style-mutation" => {
            vec![place("craft-ui-contract", &["runtime", "egui", "style"]), place("craft-ui-contract", &["runtime", "egui", "scale"])]
        }
        "contract/text-edit" => {
            vec![
                place("craft-ui-contract", &["runtime", "egui"]),
                place("craft-ui-contract", &["primitives", "text_adapter"]),
                place("craft-ui-contract", &["testing"]),
            ]
        }
        "contract/memory" => vec![place("craft-ui-contract", &["runtime", "egui", "memory"])],
        "contract/layers" => vec![place("craft-ui-contract", &["runtime", "egui", "areas"])],
        "contract/motion:clock" => vec![place("craft-ui-contract", &["runtime", "motion"])],
        "contract/platform:target" => vec![place("craft-platform", &[]), place("craft-host", &["target"])],
        "behavior/pipeline-only" => {
            vec![place("craft-ui-contract", &["contract"]), place("craft-ui-contract", &["primitives"]), place("craft-ui-widgets", &["edit"])]
        }
        k if k.starts_with("style/") => vec![place("craft-ui-contract", &["style"])],
        "test/kit" => vec![place("craft-ui-contract", &["testing"])],
        _ => Vec::new(),
    }
}

/// Locations of `contract/no-global-state` where everything is allowed, as path prefixes relative to the repository
/// root, as in findings.
const NO_GLOBAL_STATE_PREFIXES: &[&str] = &[
    // Framework runtime: the state the memoized views read through environment keys.
    "crates/ui-contract/src/runtime",
    // Test helpers.
    "crates/ui-contract/src/testing",
    // Compile-time code, not a view: the proc macro reads the compiling crate's Cargo.toml.
    "crates/ui-macros/src/component.rs",
];

/// The reviewed per-item allowlist of `contract/no-global-state`: a file and what it may hold, either the identifier of
/// a `static` or `thread_local!` item or [`FS`] for `std::fs`. Each group names the settled fact, environment key or test-data
/// cache that covers it. A new item in these files is flagged.
const NO_GLOBAL_STATE: &[(&str, &str)] = &[
    // Compile-time registry discovery reads the defining crate's source, never a view input.
    ("crates/ui-macros/src/registry.rs", FS),
    // The trait's home crate: it implements `MemoProps` by hand for std and framework value types, and for the
    // framework's own keys the derive cannot write: a binding by its source's address, its check and the check's `fn`
    // (values.rs), an icon's glyph by the type of its `dyn Glyph` (icon.rs), and the write-only `Outbox` (memo_view.rs).
    ("crates/ui-contract/src/contract/memo.rs", "MemoProps"),
    ("crates/ui-contract/src/contract/values.rs", "MemoProps"),
    ("crates/ui-contract/src/contract/memo_view.rs", "MemoProps"),
    ("crates/ui-contract/src/primitives/icon.rs", "MemoProps"),
    // Settled fact: a card's view of a story's state keys by the story value's revision, whether the start is taken, the
    // dialog's flags and the reset count; the view of every shown story's state is one fixed-size digest of those keys.
    // Both are props held as `&mut`, which the derive does not take.
    // A card keys its owned story registration by the name and show function the card reads.
    ("crates/ui-gallery/src/chrome/card.rs", "MemoProps"),
    ("crates/ui-gallery/src/viewer/picks.rs", "MemoProps"),
    // Settled fact: the names of a preset group key by a walk over the group and its folders (`dyn EachName`), which
    // the derive cannot see through.
    ("crates/ui-patterns/src/patterns/presets/model/library.rs", "MemoProps"),
    // Settled fact: a name check keys by its inputs and its `fn` behind `dyn NameChecker`, as `Bind::check_with` does.
    ("crates/ui-patterns/src/patterns/presets/named.rs", "MemoProps"),
    // Settled fact: `PresetSource` erases its inputs behind a trait object; it keys on those inputs (themselves
    // `MemoProps`) and on the addresses of its capture-free `fn`s, which stand for the code that reads them.
    ("crates/ui-patterns/src/patterns/presets/source.rs", "MemoProps"),
    // Settled fact: constant font bytes, decompressed once from compile-time input and immutable after init.
    ("crates/ui-contract/src/fonts/defaults.rs", "HACK"),
    ("crates/ui-contract/src/fonts/defaults.rs", "EGUI_ICONS"),
    ("crates/ui-contract/src/fonts/defaults.rs", "UBUNTU_LIGHT"),
    ("crates/ui-contract/src/fonts/defaults.rs", "NOTO_EMOJI"),
    ("crates/ui-contract/src/fonts/defaults.rs", "EMOJI_ICON_FONT"),
    ("crates/ui-contract/src/fonts/defaults.rs", "INTER_REGULAR"),
    ("crates/ui-contract/src/fonts/defaults.rs", "INTER_SEMIBOLD"),
    // Environment fact (font environment): the system font index and the locale list derived from it, the font
    // providers, the system UI face and its repaint waiters.
    ("crates/ui-contract/src/fonts/system/fontique.rs", "INDEX"),
    ("crates/ui-contract/src/fonts/system/fontique.rs", "HAN_LOCALES"),
    ("crates/ui-contract/src/fonts/system/mod.rs", "PROVIDERS"),
    ("crates/ui-contract/src/fonts/system/ui_face.rs", "FACE"),
    ("crates/ui-contract/src/fonts/system/ui_face.rs", "WAITING"),
    // Environment fact (font environment): the scan of the font folders and the font files read from them.
    ("crates/ui-contract/src/fonts/on_demand/files.rs", "FILES"),
    ("crates/ui-contract/src/fonts/on_demand/files.rs", "SCAN"),
    ("crates/ui-contract/src/fonts/on_demand/files.rs", FS),
    ("crates/ui-contract/src/fonts/on_demand.rs", FS),
    // Environment fact: the icon-set registry, filled at app start before the first pass.
    ("crates/ui-contract/src/primitives/svg_set.rs", "APPS"),
    // Settled fact: the thumbnail texture cache, keyed by its content.
    ("crates/ui-contract/src/primitives/textures/thumbnails.rs", "MADE"),
    // Settled fact: the log-once set of unknown token names; it has no effect on output.
    ("crates/ui-contract/src/style/tokens.rs", "REPORTED"),
    // Settled fact: the viewer's component catalog, computed once from compile-time input and immutable after init.
    ("crates/ui-gallery/src/viewer/catalog.rs", "CATALOG"),
    // Settled fact: reads a file the user dropped, in the drop handler and not during the pass.
    ("crates/ui-gallery/src/chrome/shell.rs", FS),
    // Settled fact: the fallback text format, computed once from constants and immutable after init.
    ("crates/ui-canvas/src/canvas/tool.rs", "FALLBACK"),
    // Settled fact: the identity curve, computed once from constants and immutable after init.
    ("crates/imaging/src/ui/curve/preview.rs", "IDENTITY"),
    // Settled fact: the default layer-style draft and its preview paint, computed once from constants.
    ("crates/imaging/src/ui/layer_style/hosted.rs", "DRAFT"),
    ("crates/imaging/src/ui/layer_style/hosted.rs", "PAINT"),
    // Settled fact: a test fake of the font store; it writes a temporary folder.
    ("crates/font-manager/src/store/fake.rs", FS),
    // A cache of pure test data: MockImage is used only by stories and trials.
    ("crates/imaging/src/ui/mock_image.rs", "READS"),
];

/// Whether `what` (an item identifier or [`FS`]) may be declared or used in `file` for `contract/no-global-state`.
/// Stories hold constant demo data and pure caches of it (the owner's rule allows caches of pure test data).
fn no_global_state_allows(what: &str, file: &Path) -> bool {
    NO_GLOBAL_STATE_PREFIXES.iter().any(|prefix| file.starts_with(prefix))
        || file.components().any(|c| c.as_os_str() == "stories")
        || file.file_stem().is_some_and(|stem| stem == "stories")
        || NO_GLOBAL_STATE.iter().any(|(path, item)| *item == what && file == Path::new(path))
}

struct CrateInfo {
    name: String,
    dir: PathBuf,
    egui_linked: bool,
    /// Test sources of `harness = false` targets: a file, or the directory of a `main.rs`.
    harness_free: Vec<PathBuf>,
    /// Module path of each source file of the library tree.
    modules: BTreeMap<PathBuf, Vec<String>>,
}

/// A [`RuleScope`] read from the repository: `design/layers/`, `cargo metadata` and spec 24.1.
pub struct WorkspaceScope {
    maps: LayerMaps,
    crates: Vec<CrateInfo>,
    types: BTreeMap<PathBuf, crate::contract::Types>,
}

fn is_egui_crate(name: &str) -> bool {
    name == "eframe" || name.starts_with("egui") || name.starts_with("epaint")
}

/// The dependencies `package` builds with when `features` are on, each with the features the declaration asks
/// of it: the non-optional normal dependencies, and the optional ones a feature enables.
fn built_with(package: &Package, features: &BTreeSet<String>) -> BTreeMap<String, BTreeSet<String>> {
    let mut deps: BTreeSet<String> = package.dependencies.iter().filter(|d| d.normal && !d.optional).map(|d| d.name.clone()).collect();
    let mut asked: Vec<(String, String)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut todo: Vec<String> = features.iter().cloned().collect();
    while let Some(feature) = todo.pop() {
        if !seen.insert(feature.clone()) {
            continue;
        }
        for member in package.features.get(&feature).into_iter().flatten() {
            if let Some(dep) = member.strip_prefix("dep:") {
                deps.insert(dep.to_owned());
            } else if let Some((dep, sub)) = member.split_once('/') {
                if !dep.ends_with('?') {
                    deps.insert(dep.to_owned());
                }
                asked.push((dep.trim_end_matches('?').to_owned(), sub.to_owned()));
            } else {
                todo.push(member.clone());
            }
        }
    }
    let mut built = BTreeMap::new();
    for dep in package.dependencies.iter().filter(|d| d.normal && deps.contains(&d.name)) {
        let wanted: &mut BTreeSet<String> = built.entry(dep.name.clone()).or_default();
        wanted.extend(dep.features.iter().cloned());
        if dep.default_features {
            wanted.insert("default".to_owned());
        }
    }
    for (dep, sub) in asked {
        if let Some(wanted) = built.get_mut(&dep) {
            wanted.insert(sub);
        }
    }
    built
}

/// Whether `name`, built with `features`, depends on an egui crate, directly or through a workspace crate built
/// with the features its dependent asks for.
fn links_egui(meta: &Metadata, name: &str, features: &BTreeSet<String>, visiting: &mut Vec<String>) -> bool {
    let Some(package) = meta.package(name) else { return false };
    if visiting.iter().any(|v| v == name) {
        return false;
    }
    visiting.push(name.to_owned());
    let linked = built_with(package, features).into_iter().any(|(dep, wanted)| is_egui_crate(&dep) || links_egui(meta, &dep, &wanted, visiting));
    visiting.pop();
    linked
}

/// Crates that depend on an egui crate, directly or through another workspace crate. An optional dependency
/// counts only when a feature of the crate in use enables it (its default features, or what the dependent asks
/// for), so a crate that depends on `craft-keymap` without its `ui` feature does not link egui.
fn egui_linked(meta: &Metadata) -> BTreeSet<String> {
    let default = BTreeSet::from(["default".to_owned()]);
    meta.packages.iter().filter(|p| links_egui(meta, &p.name, &default, &mut Vec::new())).map(|p| p.name.clone()).collect()
}

/// Test and bench targets with `harness = false`, and the library's sources when its test target has
/// `harness = false`, as paths relative to the crate directory.
fn harness_free_targets(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let manifest = dir.join("Cargo.toml");
    let Ok(text) = std::fs::read_to_string(&manifest) else { return Ok(Vec::new()) };
    let table: toml::Table = toml::from_str(&text).map_err(|e| Error::Toml { path: manifest.clone(), message: e.to_string() })?;
    let mut out = Vec::new();
    if let Some(lib) = table.get("lib").and_then(toml::Value::as_table)
        && lib.get("harness").and_then(toml::Value::as_bool) == Some(false)
    {
        let path = Path::new(lib.get("path").and_then(toml::Value::as_str).unwrap_or("src/lib.rs"));
        out.push(path.parent().map(Path::to_path_buf).unwrap_or_default());
    }
    for (kind, folder) in [("test", "tests"), ("bench", "benches")] {
        for target in table.get(kind).and_then(toml::Value::as_array).into_iter().flatten().filter_map(toml::Value::as_table) {
            if target.get("harness").and_then(toml::Value::as_bool) != Some(false) {
                continue;
            }
            let name = target.get("name").and_then(toml::Value::as_str).unwrap_or_default();
            let path = match target.get("path").and_then(toml::Value::as_str) {
                Some(p) => PathBuf::from(p),
                None if dir.join(folder).join(name).join("main.rs").is_file() => Path::new(folder).join(name).join("main.rs"),
                None => Path::new(folder).join(format!("{name}.rs")),
            };
            out.push(if path.file_name().is_some_and(|f| f == "main.rs") { path.parent().map(Path::to_path_buf).unwrap_or_default() } else { path });
        }
    }
    Ok(out)
}

impl WorkspaceScope {
    /// Reads the layer maps below `root` and the module trees of the mapped crates.
    pub fn load(root: &Path, meta: &Metadata) -> Result<Self, Error> {
        let (maps, _) = LayerMaps::load(root, &root.join(LAYERS_DIR))?;
        let index = Index::build(&mapped_crates(meta, &maps))?;
        Self::with_index(root, meta, maps, &index)
    }

    /// The scope from layer maps and the index of their [`mapped_crates`], already loaded.
    fn with_index(root: &Path, meta: &Metadata, maps: LayerMaps, index: &Index) -> Result<Self, Error> {
        let canonical = std::fs::canonicalize(root).map_err(|source| Error::Io { path: root.to_owned(), source })?;
        let rel = |p: &Path| relative(&canonical, &std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned()));
        let linked = egui_linked(meta);
        let mut crates = Vec::new();
        for package in &meta.packages {
            let Some(dir) = package.dir.as_ref() else { continue };
            crates.push(CrateInfo {
                name: package.name.clone(),
                dir: rel(dir),
                egui_linked: linked.contains(&package.name),
                harness_free: harness_free_targets(dir)?.into_iter().map(|p| rel(dir).join(p)).collect(),
                modules: module_files(index, package, &rel),
            });
        }
        let types = crate::contract::build_facts(index, &rel);
        Ok(Self { maps, crates, types })
    }

    fn crate_of(&self, file: &Path) -> Option<&CrateInfo> {
        self.crates.iter().filter(|c| file.starts_with(&c.dir)).max_by_key(|c| c.dir.components().count())
    }
}

/// Each source file of the package's library tree with the path of the module it holds; for a file
/// that also holds inline modules, the shortest path.
fn module_files(index: &Index, package: &Package, rel: &dyn Fn(&Path) -> PathBuf) -> BTreeMap<PathBuf, Vec<String>> {
    let mut files: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    if let Some(tree) = index.trees.get(&package.name.replace('-', "_")) {
        for module in tree.modules.values() {
            let slot = files.entry(rel(&module.file)).or_insert_with(|| module.path.clone());
            if module.path.len() < slot.len() {
                slot.clone_from(&module.path);
            }
        }
    }
    files
}

fn has_prefix(path: &[String], prefix: &[&str]) -> bool {
    path.len() >= prefix.len() && path.iter().zip(prefix).all(|(a, b)| a == b)
}

impl RuleScope for WorkspaceScope {
    fn allows(&self, key: &str, file: &Path) -> bool {
        let Some(krate) = self.crate_of(file) else { return false };
        if let Some(what) = key.strip_prefix("contract/no-global-state:") {
            return no_global_state_allows(what, file);
        }
        if key == "test/layout" {
            return file.starts_with(krate.dir.join("tests").join("it"));
        }
        let module = krate.modules.get(file);
        places(key).iter().any(|p| p.krate == krate.name && (p.module.is_empty() || module.is_some_and(|m| has_prefix(m, p.module))))
    }

    fn layer(&self, file: &Path) -> Option<u8> {
        let krate = self.crate_of(file)?;
        let module = krate.modules.get(file)?;
        self.maps.get(&krate.name)?.layer_info(module).map(|i| i.layer)
    }

    fn egui_linked(&self, file: &Path) -> bool {
        self.crate_of(file).is_some_and(|c| c.egui_linked)
    }

    fn harness_free(&self, file: &Path) -> bool {
        self.crate_of(file).is_some_and(|c| c.harness_free.iter().any(|t| file.starts_with(t)))
    }

    fn is_closed(&self, name: &str) -> bool {
        CLOSED.contains(&name)
    }

    fn crate_types(&self, file: &Path) -> Option<&crate::contract::Types> {
        self.types.get(file)
    }
}

/// The whole check of a repository: the structure checks of [`crate::check_structure`] and the egui-ban and API
/// rules of [`crate::check_with_scope`] over the config's paths, with the layer map as the rules' scope. Both read
/// one module index.
///
/// Findings are sorted by path and line.
pub fn check_workspace(root: &Path, config: &Config, meta: &Metadata) -> Result<Vec<Finding>, Error> {
    let (maps, map_findings) = LayerMaps::load(root, &root.join(LAYERS_DIR))?;
    let index = Index::build(&mapped_crates(meta, &maps))?;
    let mut findings = structure_findings(root, meta, &maps, &index, map_findings, &exempt_sites(config))?;
    let scope = WorkspaceScope::with_index(root, meta, maps, &index)?;
    findings.extend(crate::check_with(config, root, Some(&scope), &index.files)?.findings);
    findings.sort_by(|a, b| (&a.path, a.line, a.rule).cmp(&(&b.path, b.line, b.rule)));
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_global_state_allowlist_is_per_item() {
        let memo = Path::new("crates/ui-contract/src/contract/memo.rs");
        assert!(no_global_state_allows("MemoProps", memo));
        assert!(!no_global_state_allows("MemoProps", Path::new("crates/ui-contract/src/contract/entry.rs")));
        assert!(!no_global_state_allows("SOMETHING", memo));
        let file = Path::new("crates/imaging/src/ui/curve/preview.rs");
        assert!(no_global_state_allows("IDENTITY", file));
        assert!(!no_global_state_allows("SECOND", file), "a new static in an allowlisted file is flagged");
        assert!(!no_global_state_allows(FS, file));
        assert!(no_global_state_allows("ANYTHING", Path::new("crates/ui-contract/src/runtime/focus.rs")));
        assert!(no_global_state_allows("ANYTHING", Path::new("crates/x/src/ui/stories.rs")));
    }

    fn package(name: &str, features: &str, dependencies: &[&str]) -> String {
        format!(r#"{{"name":"{name}","features":{{{features}}},"dependencies":[{}],"targets":[]}}"#, dependencies.join(","))
    }

    fn dep(name: &str, optional: bool, features: &[&str]) -> String {
        let features = features.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(",");
        format!(r#"{{"name":"{name}","kind":null,"optional":{optional},"features":[{features}],"uses_default_features":true}}"#)
    }

    fn linked(packages: &[String]) -> Vec<String> {
        let json = format!(r#"{{"packages":[{}]}}"#, packages.join(","));
        egui_linked(&Metadata::parse(Path::new("."), &json).unwrap()).into_iter().collect()
    }

    #[test]
    fn an_optional_dependency_links_egui_only_when_a_feature_in_use_enables_it() {
        let packages = [
            package("gated", r#""ui":["dep:egui"]"#, &[&dep("egui", true, &[])]),
            package("plain", "", &[&dep("gated", false, &[])]),
            package("plain_above", "", &[&dep("plain", false, &[])]),
            package("asks", "", &[&dep("gated", false, &["ui"])]),
            package("direct", "", &[&dep("egui", false, &[])]),
            package("above", "", &[&dep("direct", false, &[])]),
            package("by_default", r#""default":["ui"],"ui":["dep:egui"]"#, &[&dep("egui", true, &[])]),
        ];
        assert_eq!(linked(&packages), ["above", "asks", "by_default", "direct"]);
    }
}
