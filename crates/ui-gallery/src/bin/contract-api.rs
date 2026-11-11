//! Writes `docs/contract-api.md`: every public item of craft-behavior and of craft-ui's shared modules (everything
//! but the family modules), with its path, its signature line and the first line of its doc comment.
//!
//! `cargo run -p craft-ui-gallery --bin contract-api` rewrites the file. The sources are parsed with `syn`, following
//! `mod`, `pub mod` and `pub use` from each crate root, so a re-export is listed under the path it is public at.
//! Setters that `#[derive(Props)]` writes and impls written by other derives have no source to parse; the item's
//! own docs and `Props` list them.

use quote::ToTokens;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use syn::{Attribute, Expr, ImplItem, Item, ItemUse, Lit, Meta, TraitItem, UseTree, Visibility};

const OUTPUT: &str = "docs/contract-api.md";

/// The crates listed, with the root module path of each and the modules of the second one left out: craft-ui's
/// family modules and its patterns and shell layers, which are not shared contract.
const CRATES: [(&str, &str); 2] = [("craft_behavior", "crates/behavior/src/lib.rs"), ("craft_ui", "crates/ui/src/lib.rs")];
const SHARED_UI_MODULES: [&str; 7] = ["contract", "layout", "layout_components", "primitives", "runtime", "style", "testing"];

/// The depth at which following a chain of re-exports stops.
const CHAIN_LIMIT: usize = 8;

#[derive(Clone)]
struct Member {
    signature: String,
    doc: String,
}

#[derive(Clone)]
struct Def {
    name: String,
    /// The line shown for the item: `pub struct Name<..>`, `pub fn name(..) -> ..`.
    signature: String,
    doc: String,
    public: bool,
    /// `#[doc(hidden)]`: kept so a re-export of it resolves, never listed.
    hidden: bool,
    members: Vec<Member>,
    is_type: bool,
}

struct Reexport {
    alias: String,
    /// The absolute path, or `None` when it leaves the parsed crates.
    target: Option<Vec<String>>,
    /// The path as written, for an external re-export.
    written: String,
    glob: bool,
    /// A `#[doc(hidden)]` re-export: followed to resolve names, never listed.
    hidden: bool,
}

struct Module {
    doc: String,
    /// Declared `pub mod` all the way from the crate root.
    reachable: bool,
    defs: Vec<Def>,
    reexports: Vec<Reexport>,
    children: Vec<String>,
}

struct Impl {
    type_name: String,
    module: String,
    members: Vec<Member>,
}

#[derive(Default)]
struct Tree {
    modules: BTreeMap<String, Module>,
    impls: Vec<Impl>,
    /// Exported macros, by crate root.
    macros: Vec<(String, Def)>,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn first_doc_line(attrs: &[Attribute]) -> String {
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let Meta::NameValue(pair) = &attr.meta
            && let Expr::Lit(literal) = &pair.value
            && let Lit::Str(text) = &literal.lit
        {
            let line = text.value();
            let line = line.trim();
            if !line.is_empty() {
                return line.to_owned();
            }
        }
    }
    String::new()
}

fn is_hidden(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let text = attr.to_token_stream().to_string();
        (attr.path().is_ident("doc") && text.contains("hidden")) || (attr.path().is_ident("cfg") && text.replace(' ', "") == "#[cfg(test)]")
    })
}

fn is_public(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

/// Token text with the spacing of source code.
fn tidy(tokens: impl ToTokens) -> String {
    let mut text = tokens.to_token_stream().to_string();
    for (from, to) in [
        (" :: ", "::"),
        (" ::", "::"),
        (":: ", "::"),
        ("& mut ", "&mut "),
        ("& ", "&"),
        (" ,", ","),
        (" ;", ";"),
        ("< ", "<"),
        (" <", "<"),
        (" >", ">"),
        (" :", ":"),
        ("( ", "("),
        (" )", ")"),
        (" (", "("),
        ("->(", "-> ("),
        ("[ ", "["),
        (" ]", "]"),
        ("? ", "?"),
        (" . ", "."),
        (" !", "!"),
    ] {
        text = text.replace(from, to);
    }
    text.replace("> >", ">>").replace("pub(", "pub (")
}

fn fn_line(vis: &Visibility, sig: &syn::Signature) -> String {
    let prefix = if is_public(vis) { "pub " } else { "" };
    format!("{prefix}{}", tidy(sig))
}

fn generics_and_where(generics: &syn::Generics) -> String {
    let mut text = tidy(generics);
    if let Some(clause) = &generics.where_clause {
        let _ = write!(text, " {}", tidy(clause));
    }
    text
}

fn fields_summary(fields: &syn::Fields) -> String {
    match fields {
        syn::Fields::Named(named) => {
            let shown: Vec<String> = named
                .named
                .iter()
                .filter(|field| is_public(&field.vis))
                .map(|field| format!("pub {}: {}", field.ident.as_ref().map(ToString::to_string).unwrap_or_default(), tidy(&field.ty)))
                .collect();
            if shown.is_empty() { String::new() } else { format!(" {{ {} }}", shown.join(", ")) }
        }
        syn::Fields::Unnamed(unnamed) => {
            let shown: Vec<String> =
                unnamed.unnamed.iter().filter(|field| is_public(&field.vis)).map(|field| format!("pub {}", tidy(&field.ty))).collect();
            if shown.is_empty() { String::new() } else { format!("({})", shown.join(", ")) }
        }
        syn::Fields::Unit => String::new(),
    }
}

fn variant_line(variant: &syn::Variant) -> String {
    let name = variant.ident.to_string();
    match &variant.fields {
        syn::Fields::Unit => name,
        syn::Fields::Unnamed(unnamed) => format!("{name}({})", unnamed.unnamed.iter().map(|field| tidy(&field.ty)).collect::<Vec<_>>().join(", ")),
        syn::Fields::Named(named) => format!(
            "{name} {{ {} }}",
            named
                .named
                .iter()
                .map(|field| format!("{}: {}", field.ident.as_ref().map(ToString::to_string).unwrap_or_default(), tidy(&field.ty)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn def_of(item: &Item) -> Option<Def> {
    let (name, signature, doc, vis, members, is_type) = match item {
        Item::Fn(function) => {
            (function.sig.ident.to_string(), fn_line(&function.vis, &function.sig), first_doc_line(&function.attrs), &function.vis, Vec::new(), false)
        }
        Item::Struct(item) => {
            let signature = format!("pub struct {}{}{}", item.ident, generics_and_where(&item.generics), fields_summary(&item.fields));
            (item.ident.to_string(), signature, first_doc_line(&item.attrs), &item.vis, Vec::new(), true)
        }
        Item::Enum(item) => {
            let variants: Vec<String> = item.variants.iter().map(variant_line).collect();
            let signature = format!("pub enum {}{} {{ {} }}", item.ident, generics_and_where(&item.generics), variants.join(", "));
            (item.ident.to_string(), signature, first_doc_line(&item.attrs), &item.vis, Vec::new(), true)
        }
        Item::Trait(item) => {
            let supers = if item.supertraits.is_empty() { String::new() } else { format!(": {}", tidy(&item.supertraits)) };
            let signature = format!("pub trait {}{}{}", item.ident, generics_and_where(&item.generics), supers);
            let members = item
                .items
                .iter()
                .filter_map(|member| match member {
                    TraitItem::Fn(function) => Some(Member { signature: tidy(&function.sig), doc: first_doc_line(&function.attrs) }),
                    TraitItem::Type(assoc) => Some(Member { signature: format!("type {}", assoc.ident), doc: first_doc_line(&assoc.attrs) }),
                    TraitItem::Const(assoc) => {
                        Some(Member { signature: format!("const {}: {}", assoc.ident, tidy(&assoc.ty)), doc: first_doc_line(&assoc.attrs) })
                    }
                    _ => None,
                })
                .collect();
            (item.ident.to_string(), signature, first_doc_line(&item.attrs), &item.vis, members, true)
        }
        Item::Type(item) => {
            let signature = format!("pub type {}{} = {}", item.ident, generics_and_where(&item.generics), tidy(&item.ty));
            (item.ident.to_string(), signature, first_doc_line(&item.attrs), &item.vis, Vec::new(), true)
        }
        Item::Const(item) => (
            item.ident.to_string(),
            format!("pub const {}: {}", item.ident, tidy(&item.ty)),
            first_doc_line(&item.attrs),
            &item.vis,
            Vec::new(),
            false,
        ),
        Item::Static(item) => (
            item.ident.to_string(),
            format!("pub static {}: {}", item.ident, tidy(&item.ty)),
            first_doc_line(&item.attrs),
            &item.vis,
            Vec::new(),
            false,
        ),
        _ => return None,
    };
    let hidden = is_hidden(attrs_of(item));
    Some(Def { name, signature, doc, public: is_public(vis) && !hidden, hidden, members, is_type })
}

/// The flag types a `bitflags!` invocation declares: `pub struct Name: Bits { .. }` with its first doc line.
fn bitflags_defs(tokens: &proc_macro2::TokenStream) -> Vec<Def> {
    let mut defs = Vec::new();
    let mut doc = String::new();
    let mut previous: Option<proc_macro2::TokenTree> = None;
    let mut iter = tokens.clone().into_iter().peekable();
    let mut public = false;
    while let Some(token) = iter.next() {
        match &token {
            proc_macro2::TokenTree::Group(group)
                if previous.as_ref().is_some_and(|t| matches!(t, proc_macro2::TokenTree::Punct(p) if p.as_char() == '#')) =>
            {
                let text = group.stream().to_string();
                if doc.is_empty()
                    && let Some(rest) = text.strip_prefix("doc = \"")
                {
                    doc = rest.trim_end_matches('"').trim().to_owned();
                }
            }
            proc_macro2::TokenTree::Ident(ident) if ident == "pub" => public = true,
            proc_macro2::TokenTree::Ident(ident) if ident == "struct" => {
                if let Some(proc_macro2::TokenTree::Ident(name)) = iter.next() {
                    let mut bits = String::new();
                    if matches!(iter.peek(), Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == ':') {
                        iter.next();
                        if let Some(proc_macro2::TokenTree::Ident(ty)) = iter.next() {
                            bits = format!(": {ty}");
                        }
                    }
                    defs.push(Def {
                        name: name.to_string(),
                        signature: format!("pub struct {name}{bits} (flag set)"),
                        doc: std::mem::take(&mut doc),
                        public,
                        hidden: false,
                        members: Vec::new(),
                        is_type: true,
                    });
                    public = false;
                }
            }
            _ => {}
        }
        previous = Some(token);
    }
    defs
}

fn attrs_of(item: &Item) -> &[Attribute] {
    match item {
        Item::Fn(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Const(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        _ => &[],
    }
}

/// Where `use` paths that start with `crate`, `self`, `super` or a sibling name start from.
fn absolute(path: &[String], here: &str, known: &dyn Fn(&str) -> bool) -> Option<Vec<String>> {
    let here_parts: Vec<String> = here.split("::").map(str::to_owned).collect();
    let (first, rest) = path.split_first()?;
    let mut base: Vec<String> = match first.as_str() {
        "crate" => here_parts.first().cloned().into_iter().collect(),
        "self" => here_parts,
        "super" => here_parts.get(..here_parts.len().saturating_sub(1))?.to_vec(),
        other if CRATES.iter().any(|(name, _)| *name == other) => vec![other.to_owned()],
        _ => {
            let mut relative = here_parts.clone();
            relative.extend(path.iter().cloned());
            let parent = relative.get(..relative.len().saturating_sub(1)).map(|parent| parent.join("::")).unwrap_or_default();
            return (known(&relative.join("::")) || known(&parent)).then_some(relative);
        }
    };
    let mut rest = rest;
    // `super::super::x`
    while base.len() > 1 && rest.first().is_some_and(|segment| segment == "super") {
        base.pop();
        rest = rest.get(1..)?;
    }
    base.extend(rest.iter().cloned());
    Some(base)
}

fn collect_use(tree: &UseTree, prefix: &mut Vec<String>, found: &mut Vec<(Vec<String>, String, bool)>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use(&path.tree, prefix, found);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut full = prefix.clone();
            let alias = name.ident.to_string();
            if alias == "self" {
                let last = full.last().cloned().unwrap_or_default();
                found.push((full, last, false));
            } else {
                full.push(alias.clone());
                found.push((full, alias, false));
            }
        }
        UseTree::Rename(rename) => {
            let mut full = prefix.clone();
            full.push(rename.ident.to_string());
            found.push((full, rename.rename.to_string(), false));
        }
        UseTree::Glob(_) => found.push((prefix.clone(), "*".to_owned(), true)),
        UseTree::Group(group) => {
            for inner in &group.items {
                collect_use(inner, prefix, found);
            }
        }
    }
}

/// A child module to walk: its name, its directory, whether it is public from the root and its inline content with doc.
type PendingModule = (String, PathBuf, bool, Option<(String, Vec<Item>)>);

struct Walker<'a> {
    tree: &'a mut Tree,
}

impl Walker<'_> {
    fn file(&mut self, path: &Path, module: &str, reachable: bool) {
        let Ok(source) = fs::read_to_string(path) else { return };
        let Ok(file) = syn::parse_file(&source) else { return };
        let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
        let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let dir = if matches!(stem, "lib" | "mod" | "main") { parent } else { parent.join(stem) };
        let doc = first_doc_line(&file.attrs);
        self.items(&file.items, module, &dir, reachable, doc);
    }

    fn items(&mut self, items: &[Item], module: &str, dir: &Path, reachable: bool, doc: String) {
        let mut entry = Module { doc, reachable, defs: Vec::new(), reexports: Vec::new(), children: Vec::new() };
        let mut pending: Vec<PendingModule> = Vec::new();
        for item in items {
            if is_hidden(attrs_of(item))
                && !matches!(
                    item,
                    Item::Use(_) | Item::Fn(_) | Item::Struct(_) | Item::Enum(_) | Item::Trait(_) | Item::Type(_) | Item::Const(_) | Item::Static(_)
                )
            {
                continue;
            }
            match item {
                Item::Mod(declared) => {
                    let name = declared.ident.to_string();
                    let public = is_public(&declared.vis);
                    entry.children.push(name.clone());
                    let inline = declared.content.as_ref().map(|(_, content)| content.clone());
                    pending.push((name, dir.to_path_buf(), reachable && public, inline.map(|content| (first_doc_line(&declared.attrs), content))));
                }
                Item::Use(import) => Self::uses(import, &mut entry, is_hidden(&import.attrs)),
                Item::Impl(imp) if imp.trait_.is_none() => {
                    if let syn::Type::Path(path) = &*imp.self_ty
                        && let Some(last) = path.path.segments.last()
                    {
                        let members = imp
                            .items
                            .iter()
                            .filter_map(|member| match member {
                                ImplItem::Fn(function) if is_public(&function.vis) && !is_hidden(&function.attrs) => {
                                    Some(Member { signature: fn_line(&function.vis, &function.sig), doc: first_doc_line(&function.attrs) })
                                }
                                ImplItem::Const(constant) if is_public(&constant.vis) => Some(Member {
                                    signature: format!("pub const {}: {}", constant.ident, tidy(&constant.ty)),
                                    doc: first_doc_line(&constant.attrs),
                                }),
                                _ => None,
                            })
                            .collect();
                        self.tree.impls.push(Impl { type_name: last.ident.to_string(), module: module.to_owned(), members });
                    }
                }
                Item::Macro(mac) if mac.mac.path.segments.last().is_some_and(|segment| segment.ident == "bitflags") => {
                    entry.defs.extend(bitflags_defs(&mac.mac.tokens));
                }
                Item::Macro(mac) if mac.attrs.iter().any(|attr| attr.path().is_ident("macro_export")) && mac.mac.path.is_ident("macro_rules") => {
                    if let Some(name) = &mac.ident {
                        let root = module.split("::").next().unwrap_or_default().to_owned();
                        let def = Def {
                            name: format!("{name}!"),
                            signature: format!("macro_rules! {name}"),
                            doc: first_doc_line(&mac.attrs),
                            public: true,
                            hidden: false,
                            members: Vec::new(),
                            is_type: false,
                        };
                        self.tree.macros.push((root, def));
                    }
                }
                other => {
                    if let Some(def) = def_of(other) {
                        entry.defs.push(def);
                    }
                }
            }
        }
        self.tree.modules.insert(module.to_owned(), entry);
        for (name, dir, reachable, inline) in pending {
            let child = format!("{module}::{name}");
            if let Some((doc, content)) = inline {
                self.items(&content, &child, &dir.join(&name), reachable, doc);
                continue;
            }
            let flat = dir.join(format!("{name}.rs"));
            let nested = dir.join(&name).join("mod.rs");
            if flat.exists() {
                self.file(&flat, &child, reachable);
            } else if nested.exists() {
                self.file(&nested, &child, reachable);
            }
        }
    }

    fn uses(import: &ItemUse, entry: &mut Module, hidden: bool) {
        if !is_public(&import.vis) {
            return;
        }
        let mut found = Vec::new();
        collect_use(&import.tree, &mut Vec::new(), &mut found);
        for (path, alias, glob) in found {
            let written = path.join("::");
            // The target resolves once every module is parsed.
            entry.reexports.push(Reexport { alias, target: Some(path), written, glob, hidden });
        }
    }
}

impl Tree {
    fn resolve_targets(&mut self) {
        let declared: Vec<String> = self.modules.keys().cloned().collect();
        for key in declared.clone() {
            let known = |path: &str| declared.iter().any(|module| module == path);
            if let Some(module) = self.modules.get_mut(&key) {
                for export in &mut module.reexports {
                    let Some(path) = export.target.take() else { continue };
                    export.target = absolute(&path, &key, &known);
                }
            }
        }
    }

    fn attach_impls(&mut self) {
        let impls = std::mem::take(&mut self.impls);
        for imp in impls {
            let same =
                self.modules.get_mut(&imp.module).and_then(|module| module.defs.iter_mut().find(|def| def.is_type && def.name == imp.type_name));
            if let Some(def) = same {
                def.members.extend(imp.members);
                continue;
            }
            // An impl written in a child module of the type's module.
            let mut parent = imp.module.as_str();
            let mut members = Some(imp.members);
            while let Some((up, _)) = parent.rsplit_once("::") {
                parent = up;
                if let Some(def) =
                    self.modules.get_mut(parent).and_then(|module| module.defs.iter_mut().find(|def| def.is_type && def.name == imp.type_name))
                    && let Some(members) = members.take()
                {
                    def.members.extend(members);
                    break;
                }
            }
        }
    }

    fn def(&self, module: &str, name: &str, depth: usize) -> Option<(String, &Def)> {
        let entry = self.modules.get(module)?;
        if let Some(def) = entry.defs.iter().find(|def| def.name == name) {
            return Some((module.to_owned(), def));
        }
        if depth >= CHAIN_LIMIT {
            return None;
        }
        for export in &entry.reexports {
            if export.glob {
                if let Some(target) = &export.target
                    && let Some(found) = self.def(&target.join("::"), name, depth + 1)
                {
                    return Some(found);
                }
                continue;
            }
            if export.alias == name
                && let Some(target) = &export.target
                && let Some((last, parent)) = target.split_last()
                && let Some(found) = self.def(&parent.join("::"), last, depth + 1)
            {
                return Some(found);
            }
        }
        None
    }

    /// The items a glob `use` of `module` brings in.
    fn glob_names(&self, module: &str, depth: usize) -> Vec<String> {
        let Some(entry) = self.modules.get(module) else { return Vec::new() };
        let mut names: Vec<String> = entry.defs.iter().filter(|def| def.public).map(|def| def.name.clone()).collect();
        if depth < CHAIN_LIMIT {
            for export in &entry.reexports {
                match (&export.target, export.glob) {
                    (Some(target), true) => names.extend(self.glob_names(&target.join("::"), depth + 1)),
                    (Some(_), false) => names.push(export.alias.clone()),
                    _ => {}
                }
            }
        }
        names
    }
}

fn describe(out: &mut String, path: &str, def: &Def) {
    let doc = if def.doc.is_empty() { String::new() } else { format!(" {}", def.doc) };
    let _ = writeln!(out, "- `{path}`: `{}`.{doc}", def.signature);
    for member in &def.members {
        let doc = if member.doc.is_empty() { String::new() } else { format!(" {}", member.doc) };
        let _ = writeln!(out, "  - `{}`.{doc}", member.signature);
    }
}

fn included(path: &str) -> bool {
    let mut parts = path.split("::");
    match (parts.next(), parts.next()) {
        (Some("craft_ui"), Some(top)) => SHARED_UI_MODULES.contains(&top),
        (Some("craft_ui"), None) | (Some("craft_behavior"), _) => true,
        _ => false,
    }
}

fn write_module(tree: &Tree, path: &str, out: &mut String) {
    let Some(module) = tree.modules.get(path) else { return };
    if !module.reachable {
        return;
    }
    let mut body = String::new();
    let mut defs: Vec<&Def> = module.defs.iter().filter(|def| def.public).collect();
    defs.sort_by(|a, b| a.name.cmp(&b.name));
    for def in defs {
        describe(&mut body, &format!("{path}::{}", def.name), def);
    }
    let is_root = !path.contains("::");
    if is_root {
        let mut seen: Vec<&str> = Vec::new();
        for (root, def) in &tree.macros {
            if root == path && !def.name.starts_with("__") && !seen.contains(&def.name.as_str()) {
                seen.push(&def.name);
                describe(&mut body, &format!("{path}::{}", def.name), def);
            }
        }
    }
    let mut exports: Vec<(String, String)> = Vec::new();
    for export in module.reexports.iter().filter(|export| !export.hidden) {
        let Some(target) = &export.target else {
            if !export.glob {
                exports.push((
                    export.alias.clone(),
                    format!("- `{path}::{}`: re-export of `{}` (outside these crates).\n", export.alias, export.written),
                ));
            }
            continue;
        };
        let names = if export.glob { tree.glob_names(&target.join("::"), 0) } else { vec![export.alias.clone()] };
        for name in names {
            let source = if export.glob {
                Some((target.join("::"), name.as_str()))
            } else {
                target.split_last().map(|(last, parent)| (parent.join("::"), last.as_str()))
            };
            let Some((from, original)) = source else { continue };
            let mut text = String::new();
            match tree.def(&from, original, 0) {
                Some((_, def)) if def.hidden => {}
                Some((home, def)) => {
                    let public_home = tree.modules.get(&home).is_some_and(|home_module| home_module.reachable) && included(&home) && def.public;
                    if public_home && home != path {
                        let _ = writeln!(text, "- `{path}::{name}`: re-export of `{home}::{}`.", def.name);
                    } else {
                        let mut shown = def.clone();
                        shown.name.clone_from(&name);
                        describe(&mut text, &format!("{path}::{name}"), &shown);
                    }
                }
                None => {
                    let known_module = tree.modules.contains_key(&format!("{from}::{original}"));
                    if known_module {
                        let _ = writeln!(text, "- `{path}::{name}`: re-export of the module `{from}::{original}`.");
                    } else if !export.glob {
                        let _ = writeln!(text, "- `{path}::{name}`: re-export of `{from}::{original}` (defined outside these crates).");
                    }
                }
            }
            if !text.is_empty() {
                exports.push((name, text));
            }
        }
    }
    exports.sort_by(|a, b| a.0.cmp(&b.0));
    exports.dedup_by(|a, b| a.0 == b.0);
    for (_, text) in exports {
        body.push_str(&text);
    }
    if !body.is_empty() {
        let _ = writeln!(out, "## `{path}`\n");
        if !module.doc.is_empty() {
            let _ = writeln!(out, "{}\n", module.doc);
        }
        out.push_str(&body);
        out.push('\n');
    }
    let mut children = module.children.clone();
    children.sort();
    for child in children {
        let child_path = format!("{path}::{child}");
        if included(&child_path) {
            write_module(tree, &child_path, out);
        }
    }
}

fn listing(root: &Path) -> String {
    let mut tree = Tree::default();
    for (name, file) in CRATES {
        let mut walker = Walker { tree: &mut tree };
        walker.file(&root.join(file), name, true);
    }
    tree.resolve_targets();
    tree.attach_impls();
    let mut out = String::new();
    out.push_str(
        "# Contract API\n\nGenerated by `cargo run -p craft-ui-gallery --bin contract-api`; do not edit. Every public item of\n\
         `craft_behavior` and of the shared modules of `craft_ui` (`contract`, `layout`, `layout_components`, `primitives`,\n\
         `runtime`, `style`, `testing` and the crate root), by public path, with its signature line and the first line of\n\
         its doc comment. Inherent methods and trait methods are indented under their type. Setters written by\n\
         `#[derive(Props)]` are not listed: each `#[prop]` field of a type is a setter of the same name.\n\n",
    );
    for (name, _) in CRATES {
        write_module(&tree, name, &mut out);
    }
    out
}

fn main() -> ExitCode {
    let root = workspace_root();
    let text = listing(&root);
    match fs::write(root.join(OUTPUT), text) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "cannot write {OUTPUT}: {error}");
            ExitCode::FAILURE
        }
    }
}
