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

struct Hits {
    found: Vec<(proc_macro2::Span, String)>,
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
            && path.segments.last().is_some_and(|s| s.ident == "MemoProps")
        {
            self.found.push((node.impl_token.span, "`MemoProps` is implemented by the derive only".to_owned()));
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
    let mut hits = Hits { found: Vec::new() };
    hits.visit_file(cx.file);
    for (span, message) in hits.found {
        cx.flag_span(out, ID, ID, span, message);
    }
}
