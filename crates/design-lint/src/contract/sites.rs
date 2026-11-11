//! The call, path, macro and `cfg` sites of one file, collected once for all rules.
//!
//! Paths are resolved through the file's `use` items (any scope, one table), so `Order::Foreground` after
//! `use egui::Order` is the path `egui::Order::Foreground`.

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens as _;
use syn::spanned::Spanned as _;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr};

use crate::rules::{flatten_use, is_test_attr, path_idents};

/// Crates whose items are egui APIs for the purposes of the rules.
pub(crate) const EGUI_ROOTS: [&str; 7] = ["egui", "epaint", "emath", "ecolor", "egui_extras", "accesskit", "egui_kittest"];

/// What the arguments of a call contain.
#[derive(Debug, Default)]
pub(crate) struct Args {
    pub count: usize,
    /// Every identifier inside the arguments, nested ones included.
    pub idents: Vec<String>,
    /// Whether a numeric literal appears.
    pub has_literal: bool,
    pub has_closure: bool,
}

#[derive(Debug)]
pub(crate) enum Kind {
    /// `recv.name(args)`; `receiver` holds the identifiers of the receiver expression.
    Method { name: String, receiver: Vec<String>, args: Args },
    /// A path in an expression, pattern or type, or a `use` item. `call` is set when it is the callee.
    Path { path: Vec<String>, call: bool, in_use: bool },
    /// A macro call; `idents` are the identifiers and string literals of its tokens.
    Macro { name: String, idents: Vec<String> },
    /// A `#[cfg(..)]`, `#[cfg_attr(..)]` attribute; same token summary as a macro.
    Cfg { idents: Vec<String> },
}

#[derive(Debug)]
pub(crate) struct Site {
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub kind: Kind,
}

impl Site {
    /// The method name, when this is a method call.
    pub fn method(&self) -> Option<(&str, &Args)> {
        match &self.kind {
            Kind::Method { name, args, .. } => Some((name, args)),
            _ => None,
        }
    }

    /// The resolved path of an expression, pattern or type path (not a `use` item) when it is rooted in an egui crate.
    pub fn egui_path(&self) -> Option<&[String]> {
        match &self.kind {
            Kind::Path { path, in_use: false, .. } if is_egui(path) => Some(path),
            _ => None,
        }
    }
}

pub(crate) fn is_egui(path: &[String]) -> bool {
    path.first().is_some_and(|f| EGUI_ROOTS.contains(&f.as_str()))
}

struct Imports {
    named: Vec<(String, Vec<String>)>,
    egui_globs: Vec<Vec<String>>,
}

impl Imports {
    fn resolve(&self, path: &[String]) -> Vec<String> {
        let Some(first) = path.first() else { return Vec::new() };
        if matches!(first.as_str(), "crate" | "self" | "super") {
            return path.to_vec();
        }
        let hit = self.named.iter().filter(|(alias, _)| alias == first).map(|(_, full)| full).max_by_key(|full| is_egui(full));
        let prefix = hit.or_else(|| {
            let capitalised = first.chars().next().is_some_and(char::is_uppercase);
            capitalised.then(|| self.egui_globs.first()).flatten()
        });
        match prefix {
            Some(full) if hit.is_some() => full.iter().chain(&path[1..]).cloned().collect(),
            Some(glob) => glob.iter().chain(path).cloned().collect(),
            None => path.to_vec(),
        }
    }
}

struct Collector {
    imports: Imports,
    out: Vec<Site>,
    include_tests: bool,
}

pub(crate) fn collect(file: &syn::File, include_tests: bool) -> Vec<Site> {
    let mut gather = UseGatherer { named: Vec::new(), globs: Vec::new() };
    gather.visit_file(file);
    let egui_globs = gather.globs.into_iter().filter(|g| is_egui(g)).collect();
    let mut c = Collector { imports: Imports { named: gather.named, egui_globs }, out: Vec::new(), include_tests };
    c.visit_file(file);
    c.out.sort_by_key(|s| (s.line, s.column));
    c.out
}

struct UseGatherer {
    named: Vec<(String, Vec<String>)>,
    globs: Vec<Vec<String>>,
}

impl<'ast> Visit<'ast> for UseGatherer {
    fn visit_item_use(&mut self, i: &'ast syn::ItemUse) {
        let mut found = Vec::new();
        flatten_use(&mut Vec::new(), &i.tree, &mut found);
        for (alias, path) in found {
            match alias {
                Some(alias) => self.named.push((alias, path)),
                None => self.globs.push(path),
            }
        }
    }
}

pub(crate) fn summarize(tokens: TokenStream, out: &mut Vec<String>) {
    for tree in tokens {
        match tree {
            TokenTree::Ident(i) => out.push(i.to_string()),
            TokenTree::Literal(l) => out.push(l.to_string().trim_matches('"').to_owned()),
            TokenTree::Group(g) => summarize(g.stream(), out),
            TokenTree::Punct(_) => {}
        }
    }
}

fn args_of<'a>(args: impl IntoIterator<Item = &'a Expr>) -> Args {
    let mut out = Args::default();
    for e in args {
        out.count += 1;
        out.has_closure |= matches!(e, Expr::Closure(_));
        let tokens = e.to_token_stream();
        summarize(tokens.clone(), &mut out.idents);
        out.has_literal |= has_number(tokens);
    }
    out
}

/// Whether a numeric literal appears in the tokens.
fn has_number(tokens: TokenStream) -> bool {
    tokens.into_iter().any(|t| match t {
        TokenTree::Literal(l) => l.to_string().starts_with(|c: char| c.is_ascii_digit()),
        TokenTree::Group(g) => has_number(g.stream()),
        _ => false,
    })
}

impl Collector {
    fn add(&mut self, span: proc_macro2::Span, kind: Kind) {
        let (start, end) = (span.start(), span.end());
        self.out.push(Site { line: start.line, column: start.column + 1, end_line: end.line, kind });
    }

    fn skipped(&self, attrs: &[Attribute]) -> bool {
        !self.include_tests && attrs.iter().any(is_test_attr)
    }

    fn path(&mut self, span: proc_macro2::Span, path: &syn::Path, call: bool) {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        let resolved = self.imports.resolve(&segments);
        self.add(span, Kind::Path { path: resolved, call, in_use: false });
    }

    fn cfg_attr(&mut self, attr: &Attribute) {
        if attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr") {
            let mut idents = Vec::new();
            if let syn::Meta::List(l) = &attr.meta {
                summarize(l.tokens.clone(), &mut idents);
            }
            self.add(attr.span(), Kind::Cfg { idents });
        }
    }
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_attribute(&mut self, a: &'ast Attribute) {
        self.cfg_attr(a);
    }
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        if !self.skipped(&i.attrs) {
            visit::visit_item_mod(self, i);
        }
    }
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        if !self.skipped(&i.attrs) {
            visit::visit_item_fn(self, i);
        }
    }
    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        if !self.skipped(&i.attrs) {
            visit::visit_impl_item_fn(self, i);
        }
    }
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if !self.skipped(&i.attrs) {
            visit::visit_item_impl(self, i);
        }
    }
    fn visit_item_use(&mut self, i: &'ast syn::ItemUse) {
        if self.skipped(&i.attrs) {
            return;
        }
        let mut found = Vec::new();
        flatten_use(&mut Vec::new(), &i.tree, &mut found);
        for (_, path) in found {
            self.add(i.span(), Kind::Path { path, call: false, in_use: true });
        }
    }
    fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
        let mut receiver = Vec::new();
        summarize(e.receiver.to_token_stream(), &mut receiver);
        let args = args_of(&e.args);
        self.add(e.method.span(), Kind::Method { name: e.method.to_string(), receiver, args });
        visit::visit_expr_method_call(self, e);
    }
    fn visit_expr_call(&mut self, e: &'ast syn::ExprCall) {
        if let Expr::Path(p) = &*e.func
            && p.qself.is_none()
        {
            self.path(e.func.span(), &p.path, true);
            for a in &e.args {
                self.visit_expr(a);
            }
        } else {
            visit::visit_expr_call(self, e);
        }
    }
    fn visit_expr_path(&mut self, e: &'ast syn::ExprPath) {
        if path_idents(e).is_some() {
            self.path(e.span(), &e.path, false);
        }
    }
    fn visit_expr_struct(&mut self, e: &'ast syn::ExprStruct) {
        self.path(e.path.span(), &e.path, false);
        visit::visit_expr_struct(self, e);
    }
    fn visit_pat_struct(&mut self, p: &'ast syn::PatStruct) {
        self.path(p.path.span(), &p.path, false);
        visit::visit_pat_struct(self, p);
    }
    fn visit_pat_tuple_struct(&mut self, p: &'ast syn::PatTupleStruct) {
        self.path(p.path.span(), &p.path, false);
        visit::visit_pat_tuple_struct(self, p);
    }
    fn visit_type_path(&mut self, t: &'ast syn::TypePath) {
        if t.qself.is_none() {
            self.path(t.span(), &t.path, false);
        }
        visit::visit_type_path(self, t);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        let mut idents = Vec::new();
        summarize(m.tokens.clone(), &mut idents);
        if let Some(name) = m.path.segments.last() {
            self.add(m.span(), Kind::Macro { name: name.ident.to_string(), idents });
        }
        visit::visit_macro(self, m);
    }
}

/// Whether `idents` (from a `cfg` or macro summary) check the target OS (`target_os`) or a wasm arch or family.
pub(crate) fn names_platform(idents: &[String]) -> bool {
    let wasm = idents.iter().any(|i| i.starts_with("wasm")) && idents.iter().any(|i| matches!(i.as_str(), "target_arch" | "target_family"));
    wasm || idents.iter().any(|i| i == "target_os")
}
