//! The `layer/import` and `layer/internal` rules.
//!
//! [`Index`] holds each crate's module tree with the names every module defines, re-exports (`pub use`)
//! and globs. A path is resolved through that index to the module that defines the item, and the layer
//! map judges that module, so a re-export in a low layer does not hide an item from a high one.
//! Visibility is not checked: rustc already does that.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use proc_macro2::LineColumn;
use syn::spanned::Spanned as _;
use syn::visit::{self, Visit};
use syn::{Attribute, Item, ItemUse, UseTree};

use crate::Error;
use crate::layers::{LayerMaps, finding, relative};
use crate::report::Finding;
use crate::rules::is_test_attr;

/// A path that names an item in a higher layer than the module that uses it.
pub const IMPORT: &str = "layer/import";
/// A path from layers above an internal module's range into that module.
pub const INTERNAL: &str = "layer/internal";

#[derive(Debug)]
pub(crate) enum Name {
    Defined,
    Module,
    Use(Vec<String>),
}

pub(crate) struct Module {
    pub(crate) path: Vec<String>,
    pub(crate) file: PathBuf,
    /// The inline `mod name { .. }` blocks, outermost first, that hold the module inside `file`; empty for the file's own.
    inline: Vec<String>,
    pub(crate) names: HashMap<String, Name>,
    globs: Vec<Vec<String>>,
}

/// A source file of the index: its text and its syntax tree, which every check of the run reads.
pub(crate) struct Parsed {
    pub(crate) src: String,
    pub(crate) file: syn::File,
}

pub(crate) struct CrateTree {
    pub(crate) name: String,
    pub(crate) modules: BTreeMap<Vec<String>, Module>,
}

/// The module that defines the item a path names, or the module itself when the path names a module.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Resolved {
    pub(crate) krate: String,
    pub(crate) module: Vec<String>,
    pub(crate) is_module: bool,
}

impl Resolved {
    fn shown(&self) -> String {
        std::iter::once(self.krate.as_str()).chain(self.module.iter().map(String::as_str)).collect::<Vec<_>>().join("::")
    }
}

/// A lookup's progress on one `(crate, module, segments)` state.
enum Step {
    /// Being resolved further up the chain: reaching it again is a cycle of re-exports or globs.
    Open,
    Done(Option<Resolved>),
}

/// The states one lookup has entered. Glob imports form cycles (`use child::*` beside the child's `use super::*`), so a
/// state is resolved once per lookup and a state reached again on its own chain resolves to nothing.
type Lookup = HashMap<(String, Vec<String>, Vec<String>), Step>;

/// The module trees of the crates that have a layer map, keyed by Rust identifier, and their parsed files.
#[derive(Default)]
pub(crate) struct Index {
    pub(crate) trees: BTreeMap<String, CrateTree>,
    /// Each module file, by the path its crate's `lib.rs` path leads to.
    pub(crate) files: BTreeMap<PathBuf, Parsed>,
}

fn io_error(path: &Path, message: String) -> Error {
    Error::Io { path: path.to_owned(), source: std::io::Error::new(std::io::ErrorKind::InvalidData, message) }
}

fn is_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(is_test_attr)
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Fn(i) => &i.attrs,
        Item::Mod(i) => &i.attrs,
        Item::Impl(i) => &i.attrs,
        Item::Struct(i) => &i.attrs,
        Item::Enum(i) => &i.attrs,
        Item::Use(i) => &i.attrs,
        Item::Const(i) => &i.attrs,
        Item::Static(i) => &i.attrs,
        Item::Trait(i) => &i.attrs,
        Item::Type(i) => &i.attrs,
        _ => &[],
    }
}

impl Index {
    /// Parses the module tree of each `(package name, lib.rs path)`.
    pub(crate) fn build(crates: &[(String, PathBuf)]) -> Result<Self, Error> {
        let mut index = Self::default();
        for (name, root_file) in crates {
            let tree = CrateTree::load(name, root_file, &mut index.files)?;
            index.trees.insert(name.replace('-', "_"), tree);
        }
        Ok(index)
    }

    /// The items of `module` outside test code.
    pub(crate) fn items(&self, module: &Module) -> Vec<&Item> {
        module_items(&self.files, module)
    }

    fn module(&self, krate: &str, path: &[String]) -> Option<&Module> {
        self.trees.get(krate)?.modules.get(path)
    }

    pub(crate) fn resolve(&self, krate: &str, from: &[String], segs: &[String]) -> Option<Resolved> {
        self.resolve_in(krate, from, segs, &mut Lookup::new())
    }

    fn resolve_in(&self, krate: &str, from: &[String], segs: &[String], lookup: &mut Lookup) -> Option<Resolved> {
        if segs.is_empty() {
            return None;
        }
        let (krate, module, rest) = self.start(krate, from, segs)?;
        self.walk(&krate, &module, rest, lookup)
    }

    /// The crate and module a path's first segment leads to, and the segments left.
    fn start<'s>(&self, krate: &str, from: &[String], segs: &'s [String]) -> Option<(String, Vec<String>, &'s [String])> {
        let here = |rest| Some((krate.to_owned(), from.to_vec(), rest));
        match segs[0].as_str() {
            "crate" => Some((krate.to_owned(), Vec::new(), &segs[1..])),
            "self" => here(&segs[1..]),
            "super" => {
                let mut module = from.to_vec();
                let mut rest = segs;
                while rest.first().is_some_and(|s| s == "super") {
                    module.pop()?;
                    rest = &rest[1..];
                }
                Some((krate.to_owned(), module, rest))
            }
            first => {
                let module = self.module(krate, from)?;
                if module.names.contains_key(first) {
                    here(segs)
                } else if self.trees.contains_key(first) {
                    Some((first.to_owned(), Vec::new(), &segs[1..]))
                } else if !module.globs.is_empty() {
                    here(segs)
                } else {
                    None
                }
            }
        }
    }

    fn walk(&self, krate: &str, module_path: &[String], segs: &[String], lookup: &mut Lookup) -> Option<Resolved> {
        if segs.is_empty() {
            return Some(Resolved { krate: krate.to_owned(), module: module_path.to_vec(), is_module: true });
        }
        let state = (krate.to_owned(), module_path.to_vec(), segs.to_vec());
        match lookup.get(&state) {
            Some(Step::Open) => return None,
            Some(Step::Done(found)) => return found.clone(),
            None => {}
        }
        lookup.insert(state.clone(), Step::Open);
        let found = self.step(krate, module_path, segs, lookup);
        lookup.insert(state, Step::Done(found.clone()));
        found
    }

    /// [`Self::walk`] past a state it has not entered: the first segment looked up in `module_path`.
    fn step(&self, krate: &str, module_path: &[String], segs: &[String], lookup: &mut Lookup) -> Option<Resolved> {
        let (first, rest) = segs.split_first()?;
        let module = self.module(krate, module_path)?;
        match module.names.get(first) {
            Some(Name::Module) => {
                let mut child = module_path.to_vec();
                child.push(first.clone());
                self.walk(krate, &child, rest, lookup)
            }
            Some(Name::Defined) => Some(Resolved { krate: krate.to_owned(), module: module_path.to_vec(), is_module: false }),
            Some(Name::Use(target)) => {
                let target = self.resolve_in(krate, module_path, target, lookup)?;
                if rest.is_empty() || !target.is_module { Some(target) } else { self.walk(&target.krate, &target.module, rest, lookup) }
            }
            None => module.globs.iter().find_map(|glob| {
                let target = self.resolve_in(krate, module_path, glob, lookup)?;
                if target.is_module { self.walk(&target.krate, &target.module, segs, lookup) } else { Some(target) }
            }),
        }
    }

    /// Every item and module a glob import of `target` brings in, resolved to where it is defined.
    fn expand(&self, target: &Resolved) -> Vec<Resolved> {
        let mut out = Vec::new();
        self.expand_into(target, &mut HashSet::new(), &mut out);
        out
    }

    /// [`Self::expand`] of one module; `expanded` holds the modules already expanded, which glob cycles reach again.
    fn expand_into(&self, target: &Resolved, expanded: &mut HashSet<(String, Vec<String>)>, out: &mut Vec<Resolved>) {
        let Some(module) = self.module(&target.krate, &target.module).filter(|_| target.is_module) else { return };
        if !expanded.insert((target.krate.clone(), target.module.clone())) {
            return;
        }
        for (name, kind) in &module.names {
            let resolved = match kind {
                Name::Defined => Some(Resolved { krate: target.krate.clone(), module: target.module.clone(), is_module: false }),
                Name::Module => {
                    let mut child = target.module.clone();
                    child.push(name.clone());
                    Some(Resolved { krate: target.krate.clone(), module: child, is_module: true })
                }
                Name::Use(path) => self.resolve(&target.krate, &target.module, path),
            };
            out.extend(resolved);
        }
        for glob in &module.globs {
            if let Some(inner) = self.resolve(&target.krate, &target.module, glob) {
                self.expand_into(&inner, expanded, out);
                out.push(inner);
            }
        }
    }
}

impl CrateTree {
    fn load(name: &str, root_file: &Path, files: &mut BTreeMap<PathBuf, Parsed>) -> Result<Self, Error> {
        let mut tree = Self { name: name.to_owned(), modules: BTreeMap::new() };
        parse_into(files, root_file)?;
        let dir = root_file.parent().unwrap_or(Path::new("")).to_owned();
        tree.add(files, Vec::new(), root_file, Vec::new(), &dir)?;
        Ok(tree)
    }

    /// Adds the module at `path`, held in `file` (parsed into `files`) inside the inline modules `inline`, and its
    /// children; `child_dir` is where `mod x;` files live.
    fn add(
        &mut self,
        files: &mut BTreeMap<PathBuf, Parsed>,
        path: Vec<String>,
        file: &Path,
        inline: Vec<String>,
        child_dir: &Path,
    ) -> Result<(), Error> {
        let mut names = HashMap::new();
        let mut globs = Vec::new();
        let mut children = Vec::new();
        let module = Module { path, file: file.to_owned(), inline, names: HashMap::new(), globs: Vec::new() };
        for item in module_items(files, &module) {
            match item {
                Item::Use(u) => collect_use(u, &mut names, &mut globs),
                Item::Mod(m) => {
                    names.insert(m.ident.to_string(), Name::Module);
                    children.push((m.ident.to_string(), m.content.is_some()));
                }
                other => {
                    if let Some(ident) = defined_name(other) {
                        names.insert(ident, Name::Defined);
                    }
                }
            }
        }
        for (name, is_inline) in children {
            let mut child_path = module.path.clone();
            child_path.push(name.clone());
            if is_inline {
                let mut inner = module.inline.clone();
                inner.push(name.clone());
                self.add(files, child_path, file, inner, &child_dir.join(&name))?;
            } else {
                let (child_file, grand_dir) = module_file(child_dir, &name, file)?;
                parse_into(files, &child_file)?;
                self.add(files, child_path, &child_file, Vec::new(), &grand_dir)?;
            }
        }
        self.modules.insert(module.path.clone(), Module { names, globs, ..module });
        Ok(())
    }
}

/// The items of `module` outside test code, found in its parsed file.
fn module_items<'f>(files: &'f BTreeMap<PathBuf, Parsed>, module: &Module) -> Vec<&'f Item> {
    let Some(parsed) = files.get(&module.file) else { return Vec::new() };
    let mut items = &parsed.file.items[..];
    for name in &module.inline {
        let content = items.iter().find_map(|i| match i {
            Item::Mod(m) if m.ident == name && !is_test(&m.attrs) => m.content.as_ref(),
            _ => None,
        });
        let Some((_, inner)) = content else { return Vec::new() };
        items = inner;
    }
    items.iter().filter(|i| !is_test(item_attrs(i))).collect()
}

/// Reads and parses `file` into `files`.
fn parse_into(files: &mut BTreeMap<PathBuf, Parsed>, file: &Path) -> Result<(), Error> {
    if files.contains_key(file) {
        return Ok(());
    }
    let src = std::fs::read_to_string(file).map_err(|source| Error::Io { path: file.to_owned(), source })?;
    let parsed = syn::parse_file(&src).map_err(|e| io_error(file, format!("does not parse: {e}")))?;
    files.insert(file.to_owned(), Parsed { src, file: parsed });
    Ok(())
}

/// `dir/name.rs` or `dir/name/mod.rs`, and the directory its own `mod x;` files live in.
fn module_file(dir: &Path, name: &str, parent: &Path) -> Result<(PathBuf, PathBuf), Error> {
    let flat = dir.join(format!("{name}.rs"));
    let nested = dir.join(name).join("mod.rs");
    if flat.is_file() {
        Ok((flat, dir.join(name)))
    } else if nested.is_file() {
        Ok((nested, dir.join(name)))
    } else {
        Err(io_error(parent, format!("`mod {name};` has no file {} or {}", flat.display(), nested.display())))
    }
}

fn defined_name(item: &Item) -> Option<String> {
    let ident = match item {
        Item::Fn(i) => &i.sig.ident,
        Item::Struct(i) => &i.ident,
        Item::Enum(i) => &i.ident,
        Item::Union(i) => &i.ident,
        Item::Trait(i) => &i.ident,
        Item::TraitAlias(i) => &i.ident,
        Item::Type(i) => &i.ident,
        Item::Const(i) => &i.ident,
        Item::Static(i) => &i.ident,
        _ => return None,
    };
    Some(ident.to_string())
}

/// A leaf of a `use` tree: a path with the name it binds, or a glob.
enum Leaf {
    Path(Vec<String>, Option<String>),
    Glob(Vec<String>),
}

fn leaves(tree: &UseTree, prefix: &mut Vec<String>, out: &mut Vec<Leaf>) {
    match tree {
        UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            leaves(&p.tree, prefix, out);
            prefix.pop();
        }
        UseTree::Name(n) => out.push(leaf(prefix, &n.ident.to_string(), None)),
        UseTree::Rename(r) => out.push(leaf(prefix, &r.ident.to_string(), Some(r.rename.to_string()))),
        UseTree::Glob(_) => out.push(Leaf::Glob(prefix.clone())),
        UseTree::Group(g) => g.items.iter().for_each(|t| leaves(t, prefix, out)),
    }
}

/// `use a::b::{self}` binds `b` to the path `a::b`.
fn leaf(prefix: &[String], ident: &str, alias: Option<String>) -> Leaf {
    let mut segs = prefix.to_vec();
    if ident != "self" || segs.is_empty() {
        segs.push(ident.to_owned());
    }
    Leaf::Path(segs, alias)
}

fn collect_use(item: &ItemUse, names: &mut HashMap<String, Name>, globs: &mut Vec<Vec<String>>) {
    let mut out = Vec::new();
    leaves(&item.tree, &mut Vec::new(), &mut out);
    for l in out {
        match l {
            Leaf::Glob(segs) => globs.push(segs),
            Leaf::Path(segs, alias) => {
                let bound = alias.or_else(|| segs.last().cloned()).unwrap_or_default();
                if bound != "_" {
                    names.insert(bound, Name::Use(segs));
                }
            }
        }
    }
}

struct Judge<'a> {
    index: &'a Index,
    maps: &'a LayerMaps,
    krate: &'a str,
    module: &'a Module,
    layer: u8,
    shown: PathBuf,
    out: Vec<Finding>,
    seen: HashSet<(usize, usize, &'static str, String)>,
}

impl Judge<'_> {
    fn judge_path(&mut self, segs: &[String], at: LineColumn) {
        if let Some(r) = self.index.resolve(self.krate, &self.module.path, segs) {
            self.judge(&r, &segs.join("::"), at);
        }
    }

    fn judge_glob(&mut self, segs: &[String], at: LineColumn) {
        let Some(target) = self.index.resolve(self.krate, &self.module.path, segs) else { return };
        let spelled = format!("{}::*", segs.join("::"));
        for r in self.index.expand(&target) {
            self.judge(&r, &spelled, at);
        }
        self.judge(&target, &spelled, at);
    }

    fn judge(&mut self, r: &Resolved, spelled: &str, at: LineColumn) {
        let Some(info) = self.maps.get_by_ident(&r.krate).and_then(|c| c.layer_info(&r.module)) else { return };
        let (rule, message) = if info.layer > self.layer {
            (IMPORT, format!("`{spelled}` is defined in `{}` (layer {}); layer {} may not import a higher layer", r.shown(), info.layer, self.layer))
        } else if let Some((lo, hi)) = info.internal.filter(|&(_, hi)| self.layer > hi) {
            (INTERNAL, format!("`{spelled}` is defined in `{}`, internal to layers {lo} to {hi}; layer {} may not use it", r.shown(), self.layer))
        } else {
            return;
        };
        if self.seen.insert((at.line, at.column, rule, r.shown())) {
            let mut f = Finding::new(rule, at.line, at.column + 1, at.line, message);
            f.path.clone_from(&self.shown);
            f.area = self.krate.to_owned();
            self.out.push(f);
        }
    }
}

impl<'ast> Visit<'ast> for Judge<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if !is_test(item_attrs(item)) {
            visit::visit_item(self, item);
        }
    }

    /// Nested modules are judged on their own, with their own layer.
    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {}

    fn visit_attribute(&mut self, _: &'ast Attribute) {}

    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        let at = item.span().start();
        let mut out = Vec::new();
        leaves(&item.tree, &mut Vec::new(), &mut out);
        for l in out {
            match l {
                Leaf::Path(segs, _) => self.judge_path(&segs, at),
                Leaf::Glob(segs) => self.judge_glob(&segs, at),
            }
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if path.segments.len() >= 2 {
            let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
            self.judge_path(&segs, path.span().start());
        }
        visit::visit_path(self, path);
    }
}

/// `layer/import` and `layer/internal` findings for every crate in `index` that has a layer map.
pub(crate) fn check_imports(index: &Index, maps: &LayerMaps, root: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    for (ident, tree) in &index.trees {
        let Some(layers) = maps.get(&tree.name) else { continue };
        for module in tree.modules.values() {
            let Some(info) = layers.layer_info(&module.path) else { continue };
            let mut judge = Judge {
                index,
                maps,
                krate: ident,
                module,
                layer: info.layer,
                shown: relative(root, &module.file),
                out: Vec::new(),
                seen: HashSet::new(),
            };
            for item in index.items(module) {
                judge.visit_item(item);
            }
            out.append(&mut judge.out);
        }
    }
    out
}

/// `layer/unmapped` findings: modules no map entry covers in a crate with several layers.
pub(crate) fn check_coverage(index: &Index, maps: &LayerMaps, root: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    for tree in index.trees.values() {
        let Some(layers) = maps.get(&tree.name) else { continue };
        for module in tree.modules.values().filter(|m| layers.layer_of(&m.path).is_none()) {
            let shown = if module.path.is_empty() { "crate".to_owned() } else { module.path.join("::") };
            let message = format!("module `{shown}` of `{}` is in no layer-map entry", tree.name);
            out.push(finding(crate::layers::UNMAPPED, relative(root, &module.file), message));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A crate whose `palette` module globs six children that each glob `palette` and a sibling: a dense glob cycle.
    fn cyclic() -> Index {
        let children: String = (0..6)
            .map(|i| {
                let item = if i == 3 { "pub struct Swatch;" } else { "" };
                format!("mod m{i} {{ use super::*; use super::m{}::*; {item} }}", (i + 1) % 6)
            })
            .collect();
        let globs: String = (0..6).map(|i| format!("use m{i}::*;")).collect();
        let src = format!("pub mod palette {{ use craft_theme::Hsb; {children} {globs} }}");
        let lib = Path::new("src/lib.rs");
        let mut files = BTreeMap::from([(lib.to_owned(), Parsed { file: syn::parse_file(&src).unwrap(), src })]);
        let mut tree = CrateTree { name: "demo".to_owned(), modules: BTreeMap::new() };
        tree.add(&mut files, Vec::new(), lib, Vec::new(), Path::new("src")).unwrap();
        Index { trees: BTreeMap::from([("demo".to_owned(), tree)]), files }
    }

    fn path(segs: &[&str]) -> Vec<String> {
        segs.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn names_resolve_through_a_glob_cycle_and_misses_end() {
        let index = cyclic();
        let swatch = Resolved { krate: "demo".to_owned(), module: path(&["palette", "m3"]), is_module: false };
        assert_eq!(index.resolve("demo", &path(&["palette"]), &path(&["Swatch"])), Some(swatch.clone()));
        assert_eq!(index.resolve("demo", &path(&["palette", "m0"]), &path(&["Swatch"])), Some(swatch.clone()));
        assert_eq!(index.resolve("demo", &path(&["palette", "m1"]), &path(&["Missing"])), None);
        assert_eq!(index.resolve("demo", &path(&["palette"]), &path(&["craft_theme", "Hsb"])), None);
        let palette = Resolved { krate: "demo".to_owned(), module: path(&["palette"]), is_module: true };
        assert!(index.expand(&palette).contains(&swatch));
    }
}
