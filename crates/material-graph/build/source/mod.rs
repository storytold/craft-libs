//! Engine versions from source: [`SourceTree`] reads Rust files into items, follows what an item names to what it
//! depends on, and takes a digest over that closure. A node kind's version covers the kind's match arms, everything
//! they reach and everything shared by all kinds, so changing a node changes the version of the kinds that reach the
//! change and of no others, with no version number to remember to bump.
//!
//! It errs towards invalidating, by construction:
//! - names are followed by name, not by type: `.sample()` reaches every function called `sample`;
//! - a type brings every `impl Trait for` it, since operators, `Drop` and the like run without being named;
//! - what the parser doesn't recognise as an item counts for every kind ([`items`]);
//! - a kind with no arm has no version of its own: callers use [`SourceTree::all`], which covers every file.
//!
//! Comments and layout change no version. What it can't see is listed in the result cache's documentation: the
//! dependencies and the compiler go into [`Toolchain`]; anything else that changes results goes into the epoch.

pub mod items;
pub mod modules;
pub mod tokens;

use crate::digest::{Digest, Fingerprint};
use items::{Alias, Arm, Dispatcher, Item, ItemKind, Mention, parse};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tokens::Token;

/// One source file.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Its path, as a stable name: relative to the workspace.
    pub name: String,
    pub text: String,
    /// The files it includes, by the name it uses, with their contents.
    pub included: Vec<(String, Vec<u8>)>,
}

/// The contents of the files `text` names in `include_bytes!` and the like, relative to `folder`. It reads the same
/// tokens as the parser, so a spacing like `include_bytes!( "x" )` finds the file the digest asks for.
pub fn included_files(text: &str, folder: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for name in items::includes_in(&tokens::tokenize(text)) {
        if let Ok(data) = std::fs::read(folder.join(&name)) {
            out.push((name, data));
        }
    }
    out
}

/// What a closure is made of, for telling a developer what a version covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Covered {
    pub functions: usize,
    pub files: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct SourceTree {
    items: Vec<Item>,
    /// By name, every item of that name whatever its owner.
    by_name: HashMap<String, Vec<usize>>,
    /// By owner and name.
    by_owner: HashMap<(String, String), Vec<usize>>,
    /// By type, its `impl Trait for` items.
    trait_impls: HashMap<String, Vec<usize>>,
    aliases: HashMap<String, Vec<String>>,
    arms: Vec<Arm>,
    arms_of: BTreeMap<String, Vec<usize>>,
    /// Items in every closure.
    loose: Vec<usize>,
    included: BTreeMap<(String, String), Vec<u8>>,
    /// Per item and arm, so that the closures of many kinds don't hash the same text over and over.
    digests: Vec<Digest>,
    arm_digests: Vec<Digest>,
}

impl SourceTree {
    /// Reads `files`; the functions of `dispatchers` are cut into arms.
    pub fn scan(files: &[SourceFile], dispatchers: &[Dispatcher]) -> SourceTree {
        let mut tree = SourceTree::default();
        for file in files {
            let parsed = parse(&file.text, &file.name, dispatchers);
            parsed.aliases.into_iter().for_each(|a| tree.alias(a));
            for arm in parsed.arms {
                let at = tree.arms.len();
                arm.kinds.iter().for_each(|k| tree.arms_of.entry(k.clone()).or_default().push(at));
                tree.arms.push(arm);
            }
            parsed.items.into_iter().for_each(|i| tree.add(i));
            for (name, data) in &file.included {
                tree.included.insert((file.name.clone(), name.clone()), data.clone());
            }
        }
        tree.digests = tree.items.iter().map(|i| tree.item_digest(i)).collect();
        tree.arm_digests = tree
            .arms
            .iter()
            .map(|a| {
                let mut f = Fingerprint::of("arm");
                f.u64(a.after as u64);
                let mut text = String::new();
                a.tokens.iter().for_each(|t: &Token| t.write(&mut text));
                f.text(&text);
                f.finish()
            })
            .collect();
        tree
    }

    fn alias(&mut self, a: Alias) {
        self.aliases.entry(a.alias).or_default().push(a.target);
    }

    fn add(&mut self, item: Item) {
        let at = self.items.len();
        self.by_name.entry(item.name.clone()).or_default().push(at);
        if let Some(owner) = &item.owner {
            self.by_owner.entry((owner.clone(), item.name.clone())).or_default().push(at);
            if item.trait_impl {
                self.trait_impls.entry(owner.clone()).or_default().push(at);
            }
        }
        if item.loose {
            self.loose.push(at);
        }
        self.items.push(item);
    }

    /// Every kind with an arm.
    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.arms_of.keys().map(String::as_str)
    }

    fn named(&self, name: &str) -> Vec<usize> {
        let mut out = self.by_name.get(name).cloned().unwrap_or_default();
        for target in self.aliases.get(name).into_iter().flatten() {
            out.extend(self.by_name.get(target).into_iter().flatten());
        }
        out
    }

    fn resolve(&self, mention: &Mention) -> Vec<usize> {
        match mention {
            Mention::Name(name) => self.named(name),
            // the owner's own item of that name where it has one, else anything of that name (a module's function)
            Mention::Path(owner, name) => {
                let owners = std::iter::once(owner).chain(self.aliases.get(owner).into_iter().flatten());
                let own: Vec<usize> = owners.filter_map(|o| self.by_owner.get(&(o.clone(), name.clone()))).flatten().copied().collect();
                if own.is_empty() { self.named(name) } else { own }
            }
        }
    }

    /// The items reached from `mentions`, with those in every closure.
    fn closure(&self, mentions: &[Mention], roots: &[usize]) -> BTreeSet<usize> {
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        let mut queue: Vec<usize> = roots.iter().chain(&self.loose).copied().collect();
        let mut asked: BTreeSet<&Mention> = BTreeSet::new();
        mentions.iter().for_each(|m| {
            if asked.insert(m) {
                queue.extend(self.resolve(m));
            }
        });
        while let Some(at) = queue.pop() {
            let Some(item) = self.items.get(at).filter(|_| seen.insert(at)) else { continue };
            for m in &item.mentions {
                if asked.insert(m) {
                    queue.extend(self.resolve(m));
                }
            }
            if item.kind == ItemKind::Type {
                queue.extend(self.trait_impls.get(&item.name).into_iter().flatten());
            }
        }
        seen
    }

    /// The digest of one item: where it is, its tokens and the files it includes.
    fn item_digest(&self, item: &Item) -> Digest {
        let mut f = Fingerprint::of("item");
        f.text(&item.file);
        let mut text = String::new();
        item.tokens.iter().for_each(|t: &Token| t.write(&mut text));
        f.text(&text);
        for include in &item.includes {
            f.text(include);
            match self.included.get(&(item.file.clone(), include.clone())) {
                Some(data) => f.bytes(data),
                None => f.text("missing"),
            }
        }
        f.finish()
    }

    fn digest_items(&self, purpose: &str, seen: &BTreeSet<usize>, arms: &[usize]) -> Digest {
        // sorted by content, not by index: a new item elsewhere moves every index and must not change this
        let mut parts: Vec<Digest> = seen.iter().filter_map(|at| self.digests.get(*at).copied()).collect();
        parts.extend(arms.iter().filter_map(|at| self.arm_digests.get(*at).copied()));
        parts.sort_unstable();
        let mut f = Fingerprint::of(purpose);
        parts.iter().for_each(|d| f.digest(d));
        f.finish()
    }

    /// The version of what `roots` (function or type names) reach: for a part of the program as a whole.
    pub fn version_of(&self, purpose: &str, roots: &[&str]) -> Digest {
        let mentions: Vec<Mention> = roots.iter().map(|r| Mention::Name((*r).to_string())).collect();
        self.digest_items(purpose, &self.closure(&mentions, &[]), &[])
    }

    /// The version of node kind `kind`: its arms, what they reach and what `shared` (names every kind goes through)
    /// reaches. `None` for a kind with no arm.
    pub fn kind_version(&self, kind: &str, shared: &[&str]) -> Option<Digest> {
        let arms = self.arms_of.get(kind)?;
        let mut mentions: Vec<Mention> = shared.iter().map(|r| Mention::Name((*r).to_string())).collect();
        for at in arms {
            mentions.extend(self.arms.get(*at).into_iter().flat_map(|a| a.mentions.iter().cloned()));
        }
        Some(self.digest_items(kind, &self.closure(&mentions, &[]), arms))
    }

    /// The names of the functions `kind`'s version covers (`None`: only what `shared` reaches), by file: for telling a
    /// developer what a version covers.
    pub fn functions_covered(&self, kind: Option<&str>, shared: &[&str]) -> BTreeMap<String, BTreeSet<String>> {
        let mut mentions: Vec<Mention> = shared.iter().map(|r| Mention::Name((*r).to_string())).collect();
        for at in kind.and_then(|k| self.arms_of.get(k)).into_iter().flatten() {
            mentions.extend(self.arms.get(*at).into_iter().flat_map(|a| a.mentions.iter().cloned()));
        }
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for item in self.closure(&mentions, &[]).iter().filter_map(|at| self.items.get(*at)) {
            if item.kind == ItemKind::Function {
                out.entry(item.file.clone()).or_default().insert(item.name.clone());
            }
        }
        out
    }

    /// What the kind's version covers.
    pub fn kind_covers(&self, kind: &str, shared: &[&str]) -> Option<Covered> {
        let arms = self.arms_of.get(kind)?;
        let mut mentions: Vec<Mention> = shared.iter().map(|r| Mention::Name((*r).to_string())).collect();
        for at in arms {
            mentions.extend(self.arms.get(*at).into_iter().flat_map(|a| a.mentions.iter().cloned()));
        }
        let seen = self.closure(&mentions, &[]);
        let items = seen.iter().filter_map(|at| self.items.get(*at));
        let (mut functions, mut files) = (0, BTreeSet::new());
        for item in items {
            functions += usize::from(item.kind == ItemKind::Function);
            files.insert(item.file.clone());
        }
        Some(Covered { functions, files })
    }

    /// The version of everything scanned: what a kind without arms, or a part nobody has split up, goes by.
    pub fn all(&self, purpose: &str) -> Digest {
        let everything: BTreeSet<usize> = (0..self.items.len()).collect();
        let arms: Vec<usize> = (0..self.arms.len()).collect();
        self.digest_items(purpose, &everything, &arms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISPATCH: &[Dispatcher] = &[Dispatcher { file: "a.rs", function: "compute" }];

    fn tree(a: &str, b: &str) -> SourceTree {
        let file = |name: &str, text: &str| SourceFile { name: name.into(), text: text.into(), included: Vec::new() };
        SourceTree::scan(&[file("a.rs", a), file("b.rs", b)], DISPATCH)
    }

    const A: &str = r#"
        fn eval() { compute("x"); }
        fn compute(kind: &str) -> f32 {
            let k = shared_helper();
            match kind {
                "blur" => blur(k),
                "sharpen" | "unsharp" => sharpen(k),
                "two_steps" => { let a = blur(k); sharpen(a) }
                _ => 0.0,
            }
        }
    "#;
    const B: &str = "fn shared_helper() -> f32 { 1.0 } fn blur(v: f32) -> f32 { v * 2.0 } fn sharpen(v: f32) -> f32 { v * 3.0 } fn unrelated() {}";
    const SHARED: &[&str] = &["eval"];

    fn versions(t: &SourceTree) -> BTreeMap<String, Digest> {
        t.kinds().map(|k| (k.to_string(), t.kind_version(k, SHARED).unwrap())).collect()
    }

    #[test]
    fn changing_a_node_changes_the_kinds_that_reach_it_and_no_others() {
        let before = versions(&tree(A, B));
        let after = versions(&tree(A, &B.replace("v * 2.0", "v * 2.5")));
        assert_ne!(before["blur"], after["blur"]);
        assert_ne!(before["two_steps"], after["two_steps"]);
        assert_eq!(before["sharpen"], after["sharpen"]);
        assert_eq!(before["unsharp"], after["unsharp"]);
    }

    #[test]
    fn changing_an_arm_changes_that_arm_only() {
        let before = versions(&tree(A, B));
        let after = versions(&tree(&A.replace("blur(k),", "blur(k) + 1.0,"), B));
        assert_ne!(before["blur"], after["blur"]);
        assert_eq!(before["sharpen"], after["sharpen"]);
        assert_eq!(before["two_steps"], after["two_steps"]);
    }

    #[test]
    fn changing_what_all_share_changes_all() {
        let before = versions(&tree(A, B));
        let after = versions(&tree(A, &B.replace("{ 1.0 }", "{ 1.5 }")));
        assert!(before.iter().all(|(k, v)| after[k] != *v));
    }

    #[test]
    fn comments_layout_and_unrelated_code_change_nothing() {
        let before = versions(&tree(A, B));
        let commented = B.replace("v * 2.0", "/* why */ v   *   2.0 // more\n");
        assert_eq!(before, versions(&tree(A, &commented)));
        let unrelated = B.replace("fn unrelated() {}", "fn unrelated() { let _ = 1; } fn added() {}");
        assert_eq!(before, versions(&tree(A, &unrelated)));
    }

    #[test]
    fn operators_are_reached_through_their_type() {
        let b = "struct V(f32); impl std::ops::Add for V { type Output = V; fn add(self, o: V) -> V { V(self.0 + o.0) } } fn blur(v: V) -> V { v } fn sharpen() {} fn shared_helper() {}";
        let before = versions(&tree(A, b));
        let after = versions(&tree(A, &b.replace("self.0 + o.0", "self.0 - o.0")));
        assert_ne!(before["blur"], after["blur"], "`+` names no function, but the operator is the type's");
        assert_eq!(before["sharpen"], after["sharpen"]);
    }

    #[test]
    fn renames_and_constants_are_followed() {
        let b =
            "use x::shared_helper as sh; const FACTOR: f32 = 2.0; fn blur(v: f32) -> f32 { v * FACTOR * sh() } fn sharpen() {} fn shared_helper() {}";
        let before = versions(&tree(A, b));
        assert_ne!(before["blur"], versions(&tree(A, &b.replace("= 2.0", "= 3.0")))["blur"]);
    }

    #[test]
    fn what_cannot_be_read_counts_for_all() {
        let before = versions(&tree(A, B));
        let with = format!("{B} mystery_macro!(one);");
        let changed = format!("{B} mystery_macro!(two);");
        let (with, changed) = (versions(&tree(A, &with)), versions(&tree(A, &changed)));
        assert_ne!(before["blur"], with["blur"]);
        assert!(with.iter().all(|(k, v)| changed[k] != *v));
    }

    #[test]
    fn included_files_count() {
        let source = r#"fn blur() { let _ = include_bytes!("table.bin"); } fn sharpen() {} fn shared_helper() {}"#;
        let of = |data: &[u8]| {
            let a = SourceFile { name: "a.rs".into(), text: A.into(), included: Vec::new() };
            let b = SourceFile { name: "b.rs".into(), text: source.into(), included: vec![("table.bin".into(), data.to_vec())] };
            versions(&SourceTree::scan(&[a, b], DISPATCH))
        };
        assert_ne!(of(b"one")["blur"], of(b"two")["blur"]);
        assert_eq!(of(b"one")["sharpen"], of(b"two")["sharpen"]);
    }

    #[test]
    fn an_include_is_found_however_it_is_spaced() {
        let folder = std::env::temp_dir().join(format!("craft_material_graph_include_{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("table.bin"), b"data").unwrap();
        for text in [
            r#"include_bytes!("table.bin")"#,
            r#"include_bytes!( "table.bin" )"#,
            "include_bytes! (\n    \"table.bin\",\n)",
            r##"include_bytes!(r#"table.bin"#)"##,
        ] {
            assert_eq!(included_files(text, &folder), [("table.bin".to_string(), b"data".to_vec())], "{text}");
        }
        assert!(included_files(r#"include_bytes!("absent.bin")"#, &folder).is_empty());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// The first arm that matches runs: a guarded arm in front of a literal one is not the same program as behind it.
    #[test]
    fn the_order_of_arms_that_can_overlap_counts() {
        let guarded = r#"fn eval() { compute("x", true); } fn compute(kind: &str, flag: bool) -> f32 { let k = shared_helper(); match kind { "blur" if flag => blur(k), "blur" => sharpen(k), _ => 0.0 } }"#;
        let swapped = r#"fn eval() { compute("x", true); } fn compute(kind: &str, flag: bool) -> f32 { let k = shared_helper(); match kind { "blur" => sharpen(k), "blur" if flag => blur(k), _ => 0.0 } }"#;
        assert_ne!(versions(&tree(guarded, B))["blur"], versions(&tree(swapped, B))["blur"]);
    }

    #[test]
    fn an_attribute_on_a_module_changes_what_is_inside() {
        let backend = |first: &str, second: &str| {
            format!(
                "#[cfg({first})] mod backend {{ pub fn shade() -> f32 {{ 1.0 }} }} #[cfg({second})] mod backend {{ pub fn shade() -> f32 {{ 2.0 }} }}"
            )
        };
        let b = |attrs: String| format!("{attrs} fn blur(v: f32) -> f32 {{ v * backend::shade() }} fn sharpen() {{}} fn shared_helper() {{}}");
        let (one, other) = (b(backend("unix", "not(unix)")), b(backend("not(unix)", "unix")));
        assert_ne!(versions(&tree(A, &one))["blur"], versions(&tree(A, &other))["blur"]);
    }

    #[test]
    fn every_source_change_reaches_the_version_of_everything() {
        let before = tree(A, B).all("all");
        assert_ne!(before, tree(A, &B.replace("fn unrelated() {}", "fn unrelated() { 1; }")).all("all"));
        assert_eq!(before, tree(A, &format!("{B} // a comment")).all("all"));
    }

    #[test]
    fn a_kind_with_no_arm_has_no_version_of_its_own() {
        assert_eq!(tree(A, B).kind_version("nobody", SHARED), None);
    }
}
