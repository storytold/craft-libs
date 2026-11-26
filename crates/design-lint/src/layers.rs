//! The layer map: `design/layers/<crate>/crate.toml` and `design/layers/<crate>/<family>.toml`.
//!
//! `crate.toml` gives the crate's lowest and highest layer (and, for a layer-0 crate, its row), or
//! `outside = true` for a crate outside the layers (proc macros, a gallery app).
//! Each family file lists module paths with the layer the module belongs to, or `outside = true`.
//! Every file starts with `config_version = 1`. The layer is written
//! down, not derived from directories, because modules move between directories.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::Error;
use crate::report::Finding;

/// A layer-map file that is malformed or contradicts the crate's layer range or another file.
pub const MAP: &str = "layer/map";
/// A module of the source tree that no layer-map entry (and no single-layer crate) covers.
pub const UNMAPPED: &str = "layer/unmapped";

/// The highest layer number (layer 8 exists only in app repositories).
pub const MAX_LAYER: u8 = 8;

/// The file that holds a crate's layer range.
const CRATE_FILE: &str = "crate.toml";

/// Which row of layer 0 a crate sits in.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Row {
    /// Primitives, seeds and tokens.
    Tokens,
    /// Host, words, settings, key types and OS services.
    Services,
}

/// One module path and its layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleLayer {
    /// Module path below the crate root; empty for the root itself.
    pub path: Vec<String>,
    /// The module's layer; `None` for a module outside the layers (testing helpers, macros).
    pub layer: Option<u8>,
    /// Set when the module is internal to layers `lo` to `hi`: layers above `hi` may not use it.
    pub internal: Option<(u8, u8)>,
    /// The family file the entry came from.
    pub family: String,
}

/// The layers of one crate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrateLayers {
    /// Package name, for example `craft-ui`.
    pub name: String,
    /// Lowest and highest layer of the crate's modules; `None` for a crate outside the layers.
    pub range: Option<(u8, u8)>,
    /// Row within layer 0; set exactly when the crate is a layer-0 crate.
    pub row: Option<Row>,
    /// Module entries from every family file.
    pub modules: Vec<ModuleLayer>,
}

/// Where the map puts a module.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Placement {
    /// In a layer.
    Layer(LayerInfo),
    /// Outside the layers: not judged as importer or as import target.
    Outside,
}

/// A module's layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayerInfo {
    /// The module's layer.
    pub layer: u8,
    /// The module's internal range, when it has one.
    pub internal: Option<(u8, u8)>,
}

impl CrateLayers {
    /// The layer of the module at `path`: the longest entry that is a prefix of it (an entry for `crate`
    /// covers the root file only). A module with no entry takes the crate's only layer when
    /// `lowest == highest`; the root file, which declares and re-exports, takes the highest layer.
    pub fn layer_of(&self, path: &[String]) -> Option<Placement> {
        let Some((lowest, highest)) = self.range else { return Some(Placement::Outside) };
        let covers = |m: &&ModuleLayer| if m.path.is_empty() { path.is_empty() } else { path.starts_with(&m.path) };
        let best = self.modules.iter().filter(covers).max_by_key(|m| m.path.len());
        let layered = |layer, internal| Some(Placement::Layer(LayerInfo { layer, internal }));
        match best {
            Some(ModuleLayer { layer: Some(layer), internal, .. }) => layered(*layer, *internal),
            Some(_) => Some(Placement::Outside),
            None if lowest == highest => layered(lowest, None),
            None if path.is_empty() => layered(highest, None),
            None => None,
        }
    }

    /// The layer of the module at `path`, when it is in one.
    pub fn layer_info(&self, path: &[String]) -> Option<LayerInfo> {
        match self.layer_of(path)? {
            Placement::Layer(info) => Some(info),
            Placement::Outside => None,
        }
    }
}

/// The layer maps of every crate that has one.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LayerMaps {
    crates: BTreeMap<String, CrateLayers>,
}

impl LayerMaps {
    /// The map of the package called `name`.
    pub fn get(&self, name: &str) -> Option<&CrateLayers> {
        self.crates.get(name)
    }

    /// Every crate with a map, by package name.
    pub fn crates(&self) -> impl Iterator<Item = &CrateLayers> {
        self.crates.values()
    }

    /// The map of the crate whose Rust identifier is `ident` (`craft_ui` for `craft-ui`).
    pub fn get_by_ident(&self, ident: &str) -> Option<&CrateLayers> {
        self.crates.values().find(|c| c.name.replace('-', "_") == ident)
    }

    /// Reads `dir` (`design/layers`); a missing directory is an empty map.
    ///
    /// Returns the maps and the `layer/map` findings for files that contradict each other or the crate's
    /// range. A file that does not parse is an error. Finding paths are relative to `root`.
    pub fn load(root: &Path, dir: &Path) -> Result<(Self, Vec<Finding>), Error> {
        let mut maps = Self::default();
        let mut findings = Vec::new();
        for crate_dir in sorted_entries(dir)?.into_iter().filter(|p| p.is_dir()) {
            let name = crate_dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
            let (layers, mut found) = load_crate(root, &crate_dir, &name)?;
            findings.append(&mut found);
            if let Some(layers) = layers {
                maps.crates.insert(name, layers);
            }
        }
        Ok((maps, findings))
    }
}

pub(crate) fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let io = |source| Error::Io { path: dir.to_owned(), source };
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).map_err(io)?.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>().map_err(io)?;
    entries.sort();
    Ok(entries)
}

/// The `config_version` every design file carries.
pub const CONFIG_VERSION: i64 = 1;

/// Reads a design file: it must carry `config_version = 1`; the other keys deserialize into `T`.
pub(crate) fn read_toml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, Error> {
    let fail = |message: String| Error::Toml { path: path.to_owned(), message };
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io { path: path.to_owned(), source })?;
    let mut table: toml::Table = toml::from_str(&text).map_err(|e| fail(e.to_string()))?;
    match table.remove("config_version") {
        Some(toml::Value::Integer(CONFIG_VERSION)) => {}
        Some(other) => return Err(fail(format!("unsupported config_version {other:?}; this tool reads {CONFIG_VERSION}"))),
        None => return Err(fail(format!("missing `config_version = {CONFIG_VERSION}`"))),
    }
    toml::Value::Table(table).try_into().map_err(|e| fail(e.to_string()))
}

pub(crate) fn relative(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_owned()
}

pub(crate) fn finding(rule: &'static str, path: PathBuf, message: String) -> Finding {
    let mut f = Finding::new(rule, 1, 1, 1, message);
    f.path = path;
    f.area = "(design)".to_owned();
    f
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCrate {
    lowest: Option<u8>,
    highest: Option<u8>,
    row: Option<Row>,
    #[serde(default)]
    outside: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFamily {
    #[serde(default)]
    module: Vec<RawModule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModule {
    path: String,
    layer: Option<u8>,
    #[serde(default)]
    outside: bool,
    internal: Option<[u8; 2]>,
}

fn load_crate(root: &Path, dir: &Path, name: &str) -> Result<(Option<CrateLayers>, Vec<Finding>), Error> {
    let mut findings = Vec::new();
    let crate_file = dir.join(CRATE_FILE);
    if !crate_file.is_file() {
        findings.push(finding(
            MAP,
            relative(root, dir),
            format!("`{name}` has layer files but no `{CRATE_FILE}` giving its lowest and highest layer"),
        ));
        return Ok((None, findings));
    }
    let raw: RawCrate = read_toml(&crate_file)?;
    let shown = relative(root, &crate_file);
    let range = check_crate_range(&raw, &shown, &mut findings);
    let mut layers = CrateLayers { name: name.to_owned(), range, row: raw.row, modules: Vec::new() };
    for file in sorted_entries(dir)? {
        let is_family = file.extension().is_some_and(|e| e == "toml") && file.file_name().is_some_and(|n| n != CRATE_FILE);
        if is_family {
            load_family(root, &file, &mut layers, &mut findings)?;
        }
    }
    Ok((Some(layers), findings))
}

/// The crate's layer range, or `None` for `outside = true` or a range that was reported as wrong.
fn check_crate_range(raw: &RawCrate, file: &Path, findings: &mut Vec<Finding>) -> Option<(u8, u8)> {
    let mut bad = |message: String| {
        findings.push(finding(MAP, file.to_owned(), message));
        None
    };
    match (raw.outside, raw.lowest, raw.highest) {
        (true, None, None) if raw.row.is_none() => None,
        (true, ..) => bad("`outside` excludes `lowest`, `highest` and `row`".to_owned()),
        (false, Some(lowest), Some(highest)) if lowest > highest || highest > MAX_LAYER => {
            bad(format!("layer range {lowest} to {highest} is not within 0 to {MAX_LAYER}"))
        }
        (false, Some(lowest), Some(highest)) => match (lowest == 0 && highest == 0, raw.row) {
            (true, None) => bad("a layer-0 crate names its `row` (`tokens` or `services`)".to_owned()),
            (false, Some(_)) => bad("`row` applies to layer-0 crates only".to_owned()),
            _ => Some((lowest, highest)),
        },
        (false, ..) => bad("`lowest` and `highest` are both required (or `outside = true`)".to_owned()),
    }
}

fn load_family(root: &Path, file: &Path, layers: &mut CrateLayers, findings: &mut Vec<Finding>) -> Result<(), Error> {
    let raw: RawFamily = read_toml(file)?;
    let shown = relative(root, file);
    let family = file.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_owned();
    for entry in raw.module {
        match module_entry(&entry, layers, &family) {
            Ok(module) => layers.modules.push(module),
            Err(message) => findings.push(finding(MAP, shown.clone(), message)),
        }
    }
    Ok(())
}

fn module_entry(entry: &RawModule, layers: &CrateLayers, family: &str) -> Result<ModuleLayer, String> {
    let path = parse_module_path(&entry.path)?;
    let shown = &entry.path;
    let layer = match (entry.layer, entry.outside) {
        (Some(_), true) | (None, false) => return Err(format!("`{shown}` has either `layer` or `outside = true`")),
        (layer, _) => layer,
    };
    if let (Some(layer), Some((lowest, highest))) = (layer, layers.range)
        && (layer < lowest || layer > highest)
    {
        return Err(format!("`{shown}` is in layer {layer}, outside the crate's layers {lowest} to {highest}"));
    }
    let internal = match (entry.internal, layer) {
        (Some([lo, hi]), Some(layer)) if lo <= hi && (lo..=hi).contains(&layer) => Some((lo, hi)),
        (Some([lo, hi]), _) => return Err(format!("`{shown}`: internal range {lo} to {hi} must be ordered and contain the module's layer")),
        (None, _) => None,
    };
    if let Some(other) = layers.modules.iter().find(|m| m.path == path) {
        return Err(format!("`{shown}` is already mapped in `{}.toml`", other.family));
    }
    Ok(ModuleLayer { path, layer, internal, family: family.to_owned() })
}

/// `crate` is the root; anything else is `a::b::c` with identifier segments.
fn parse_module_path(text: &str) -> Result<Vec<String>, String> {
    if text == "crate" {
        return Ok(Vec::new());
    }
    let segments: Vec<String> = text.split("::").map(str::to_owned).collect();
    let valid = |s: &String| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_') && !s.starts_with(|c: char| c.is_ascii_digit());
    if segments.iter().all(valid) && segments[0] != "crate" {
        Ok(segments)
    } else {
        Err(format!("`{text}` is not a module path (`crate` or `a::b`, relative to the crate root)"))
    }
}
