//! `contract/no-global-state`: no process-level state in UI crates. A memoized view's inputs are its props and the
//! environment keys the framework tracks; a `static`, a `thread_local!`, a filesystem read or a hand-written `MemoProps`
//! impl is an input the framework cannot see.
//!
//! Clocks are covered by `contract/motion:clock`, and `unsafe_code` by the workspace `forbid` plus `contract/unsafe-allow`.
//! Build scripts and binaries (`build.rs`, `src/bin/`) are not views and are skipped.

use syn::visit::{self, Visit};
use syn::{Attribute, ItemImpl, ItemMacro, ItemMod, ItemStatic, ItemUse, Path as SynPath, UseTree};

use crate::report::Finding;

use super::Cx;

pub const ID: &str = "contract/no-global-state";

const PATH_FS: [&str; 2] = ["std", "fs"];

/// Permission keys of the two derive-only rules; no location is allowed.
const PLAIN_DATA: &str = "contract/no-global-state:plain-data";
const PROJECTION: &str = "contract/no-global-state:projection";

/// The hidden constructor behind `field!`; the lint reads source before macro expansion, so every textual hit is a direct call.
const FROM_FIELD_PATH: &str = "__from_field_path";

struct Hits {
    found: Vec<(proc_macro2::Span, String)>,
    derive_only: Vec<(proc_macro2::Span, String)>,
    projection: Vec<(proc_macro2::Span, String)>,
}

fn field_path_in(tokens: proc_macro2::TokenStream, out: &mut Vec<proc_macro2::Span>) {
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Ident(i) if i == FROM_FIELD_PATH => out.push(i.span()),
            proc_macro2::TokenTree::Group(g) => field_path_in(g.stream(), out),
            _ => {}
        }
    }
}

fn is_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && matches!(&a.meta, syn::Meta::List(list) if list.tokens.clone().into_iter().any(|t| matches!(t, proc_macro2::TokenTree::Ident(i) if i == "test")))
    })
}

fn names_fs(segments: &[String]) -> bool {
    segments.windows(2).any(|w| w[0] == PATH_FS[0] && w[1] == PATH_FS[1])
}

fn use_paths(tree: &UseTree, prefix: &mut Vec<String>, out: &mut Vec<(proc_macro2::Span, Vec<String>)>) {
    match tree {
        UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            use_paths(&p.tree, prefix, out);
            prefix.pop();
        }
        UseTree::Name(n) => {
            let mut full = prefix.clone();
            full.push(n.ident.to_string());
            out.push((n.ident.span(), full));
        }
        UseTree::Rename(n) => {
            let mut full = prefix.clone();
            full.push(n.ident.to_string());
            out.push((n.ident.span(), full));
        }
        UseTree::Glob(g) => out.push((g.star_token.span, prefix.clone())),
        UseTree::Group(g) => g.items.iter().for_each(|t| use_paths(t, prefix, out)),
    }
}

impl Visit<'_> for Hits {
    fn visit_item_mod(&mut self, node: &ItemMod) {
        if !is_cfg_test(&node.attrs) {
            visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_static(&mut self, node: &ItemStatic) {
        let what = if matches!(node.mutability, syn::StaticMutability::Mut(_)) { "static mut" } else { "static" };
        self.found.push((
            node.static_token.span,
            format!("`{what} {}` is process-level state; use `const`, or pass it as a prop or environment key", node.ident),
        ));
        visit::visit_item_static(self, node);
    }

    fn visit_item_macro(&mut self, node: &ItemMacro) {
        self.macro_path(&node.mac.path);
        visit::visit_item_macro(self, node);
    }

    fn visit_stmt_macro(&mut self, node: &syn::StmtMacro) {
        self.macro_path(&node.mac.path);
        visit::visit_stmt_macro(self, node);
    }

    fn visit_item_use(&mut self, node: &ItemUse) {
        let mut paths = Vec::new();
        use_paths(&node.tree, &mut Vec::new(), &mut paths);
        for (span, path) in paths {
            if names_fs(&path) {
                self.found.push((span, "`std::fs` reads the file system, which the framework does not track; read it outside the view".to_owned()));
            }
        }
        visit::visit_item_use(self, node);
    }

    fn visit_ident(&mut self, node: &proc_macro2::Ident) {
        if node == FROM_FIELD_PATH {
            self.projection.push((node.span(), format!("`{FROM_FIELD_PATH}` is called by `field!` only")));
        }
    }

    fn visit_macro(&mut self, node: &syn::Macro) {
        let mut spans = Vec::new();
        field_path_in(node.tokens.clone(), &mut spans);
        for span in spans {
            self.projection.push((span, format!("`{FROM_FIELD_PATH}` is called by `field!` only")));
        }
        visit::visit_macro(self, node);
    }

    fn visit_path(&mut self, node: &SynPath) {
        let names: Vec<String> = node.segments.iter().map(|s| s.ident.to_string()).collect();
        if names_fs(&names) {
            self.found.push((
                node.segments[0].ident.span(),
                "`std::fs` reads the file system, which the framework does not track; read it outside the view".to_owned(),
            ));
        }
        visit::visit_path(self, node);
    }

    fn visit_item_impl(&mut self, node: &ItemImpl) {
        if let Some((_, path, _)) = &node.trait_
            && let Some(name) = path.segments.last().map(|s| s.ident.to_string()).filter(|n| n == "MemoProps" || n == "PlainData")
        {
            let message = format!("`{name}` is implemented by the derive only");
            if name == "MemoProps" {
                self.found.push((node.impl_token.span, message));
            } else {
                self.derive_only.push((node.impl_token.span, message));
            }
        }
        visit::visit_item_impl(self, node);
    }
}

impl Hits {
    fn macro_path(&mut self, path: &SynPath) {
        if let Some(last) = path.segments.last()
            && matches!(last.ident.to_string().as_str(), "thread_local" | "lazy_static")
        {
            self.found.push((last.ident.span(), format!("`{}!` declares process-level state; pass it as a prop or environment key", last.ident)));
        }
    }
}

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    let normal = cx.path.to_string_lossy().replace('\\', "/");
    if normal.ends_with("/build.rs") || normal == "build.rs" || normal.contains("/src/bin/") {
        return;
    }
    let mut hits = Hits { found: Vec::new(), derive_only: Vec::new(), projection: Vec::new() };
    hits.visit_file(cx.file);
    for (span, message) in hits.found {
        cx.flag_span(out, ID, ID, span, message);
    }
    for (span, message) in hits.derive_only {
        cx.flag_span(out, ID, PLAIN_DATA, span, message);
    }
    for (span, message) in hits.projection {
        cx.flag_span(out, ID, PROJECTION, span, message);
    }
}
