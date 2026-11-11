//! `test/*`: where tests live, what they use, and what they assert (spec 22.2, 24.1).
//!
//! These rules look at test code, so a config that checks them sets `include-tests`.

use proc_macro2::TokenTree;
use quote::ToTokens as _;
use syn::spanned::Spanned as _;
use syn::visit::{self, Visit};
use syn::{Attribute, ItemFn, ItemMod};

use crate::report::Finding;
use crate::rules::is_test_attr;

use super::Cx;
use super::sites::{Kind, summarize};

/// An integration test file outside `tests/it/` in an egui-linked crate.
pub const LAYOUT: &str = "test/layout";
/// `egui_kittest` used outside the testing kit.
pub const KIT: &str = "test/kit";
/// A `#[test]` in a `harness = false` target.
pub const HARNESS: &str = "test/harness";
/// A `#[cfg(test)]` module in an egui-linked crate.
pub const CFG_TEST: &str = "test/cfg-test";
/// A float literal asserted in a test that shows a component or reads tokens.
pub const LITERAL_SIZE: &str = "test/literal-size";
/// A `#[test]` or `#[trial]` that is `#[ignore]`d, so never runs.
pub const IGNORED: &str = "test/ignored";

/// Identifiers that mean a test shows a component or reads token values.
const SHOWS: [&str; 8] = ["Fixture", "show", "Story", "story", "Tokens", "tokens", "by_label", "by_role"];

fn is_unit_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("test"))
}

fn is_test_fn(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("test") || a.path().segments.last().is_some_and(|s| s.ident == "trial"))
}

fn is_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cfg") && is_test_attr(a))
}

fn has_float(tokens: proc_macro2::TokenStream) -> bool {
    tokens.into_iter().any(|t| match t {
        TokenTree::Literal(l) => {
            let text = l.to_string();
            text.starts_with(|c: char| c.is_ascii_digit())
                && (text.contains('.') || text.ends_with("f32") || text.ends_with("f64"))
                && text.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '_')).any(|c| c.is_ascii_digit() && c != '0')
        }
        TokenTree::Group(g) => has_float(g.stream()),
        _ => false,
    })
}

struct Walk<'c, 'a> {
    cx: &'c Cx<'a>,
    out: &'c mut Vec<Finding>,
}

impl Walk<'_, '_> {
    fn function(&mut self, attrs: &[Attribute], ident: &syn::Ident, body: &syn::Block) {
        if self.cx.scope.harness_free(self.cx.path) && is_unit_test(attrs) {
            self.cx.flag_span(
                self.out,
                HARNESS,
                HARNESS,
                ident.span(),
                format!("`#[test]` fn `{ident}` in a `harness = false` target is never run; register it as a trial"),
            );
        }
        if is_test_fn(attrs) && attrs.iter().any(|a| a.path().is_ident("ignore")) {
            self.cx.flag_span(
                self.out,
                IGNORED,
                IGNORED,
                ident.span(),
                format!("test `{ident}` is `#[ignore]`d and never runs; run it or delete it"),
            );
        }
        if is_test_fn(attrs) && self.cx.scope.layer(self.cx.path).is_none_or(|l| l > 1) {
            let mut idents = Vec::new();
            summarize(body.to_token_stream(), &mut idents);
            if idents.iter().any(|i| SHOWS.contains(&i.as_str())) {
                let mut asserts = Asserts::default();
                asserts.visit_block(body);
                for span in asserts.floats {
                    self.cx.flag_span(
                        self.out,
                        LITERAL_SIZE,
                        LITERAL_SIZE,
                        span,
                        "asserts a literal size in a test that shows a component or reads tokens; assert a relation or `equals_token`".to_owned(),
                    );
                }
            }
        }
    }
}

#[derive(Default)]
struct Asserts {
    floats: Vec<proc_macro2::Span>,
}

impl<'ast> Visit<'ast> for Asserts {
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        let name = m.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
        if (name.starts_with("assert") || name.starts_with("debug_assert")) && has_float(m.tokens.clone()) {
            self.floats.push(m.span());
        }
    }
}

impl<'ast> Visit<'ast> for Walk<'_, '_> {
    fn visit_item_fn(&mut self, i: &'ast ItemFn) {
        self.function(&i.attrs, &i.sig.ident, &i.block);
        visit::visit_item_fn(self, i);
    }

    fn visit_item_mod(&mut self, i: &'ast ItemMod) {
        if self.cx.scope.egui_linked(self.cx.path) && is_cfg_test(&i.attrs) {
            self.cx.flag_span(
                self.out,
                CFG_TEST,
                CFG_TEST,
                i.ident.span(),
                format!("`#[cfg(test)] mod {}` in an egui-linked crate; tests live in the integration binary", i.ident),
            );
        }
        visit::visit_item_mod(self, i);
    }
}

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    Walk { cx, out }.visit_file(cx.file);
    for site in &cx.sites {
        if let Kind::Path { path, .. } = &site.kind
            && path.first().is_some_and(|f| f == "egui_kittest")
        {
            cx.flag(out, KIT, KIT, site, format!("`{}` is the egui test harness; use `craft_ui::testing`", path.join("::")));
        }
    }
    let parts: Vec<_> = cx.path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let misplaced = parts.iter().position(|p| p == "tests").is_some_and(|i| parts.get(i + 1).is_some_and(|n| n != "it"));
    if misplaced && cx.scope.egui_linked(cx.path) && !cx.scope.allows(LAYOUT, cx.path) {
        out.push(Finding::new(LAYOUT, 1, 1, 1, "integration test outside `tests/it/`; an egui-linked crate has one integration binary".to_owned()));
    }
}
