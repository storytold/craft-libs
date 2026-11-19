//! Reject suite registrations hidden from the source-tree registry walker.

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use std::str::FromStr as _;
use syn::spanned::Spanned as _;
use syn::visit::{self, Visit};

use super::Cx;
use crate::report::Finding;

/// Suite items inside expansions cannot be discovered by `registry!`.
pub const ID: &str = "registry/expansion";

fn attribute(tree: &TokenTree) -> Option<String> {
    match tree {
        TokenTree::Group(group) if group.delimiter() == Delimiter::Bracket => Some(group.stream().to_string()),
        _ => None,
    }
}

fn words(attr: &str) -> Vec<&str> {
    attr.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|s| !s.is_empty()).collect()
}

fn has_registration(tokens: TokenStream) -> bool {
    let trees: Vec<_> = tokens.into_iter().collect();
    let mut i = 0;
    while i < trees.len() {
        if matches!(&trees[i], TokenTree::Punct(p) if p.as_char() == '#') {
            let mut attrs = Vec::new();
            while i + 1 < trees.len() && matches!(&trees[i], TokenTree::Punct(p) if p.as_char() == '#') {
                let Some(attr) = attribute(&trees[i + 1]) else { break };
                attrs.push(attr);
                i += 2;
            }
            if attrs.iter().any(|attr| words(attr).first().is_some_and(|word| *word == "story" || *word == "trial")) {
                return true;
            }
            let unregistered = attrs.iter().any(|attr| {
                let words = words(attr);
                words.first() == Some(&"component") && words.contains(&"unregistered")
            });
            if !unregistered
                && attrs.iter().any(|attr| {
                    let words = words(attr);
                    words.first() == Some(&"derive") && (words.contains(&"Component") || words.contains(&"Pattern"))
                })
            {
                return true;
            }
            let story_set = attrs.iter().any(|attr| {
                let words = words(attr);
                words.first() == Some(&"command") && words.contains(&"story")
            });
            if story_set
                && attrs.iter().any(|attr| {
                    let words = words(attr);
                    words.first() == Some(&"derive") && words.contains(&"CommandSet")
                })
            {
                return true;
            }
            continue;
        }
        if let TokenTree::Ident(ident) = &trees[i] {
            if ident == "register_contract" && trees.get(i + 1).is_some_and(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == '!')) {
                return true;
            }
        }
        if let TokenTree::Group(group) = &trees[i] {
            if has_registration(group.stream()) {
                return true;
            }
        }
        i += 1;
    }
    false
}

struct Walk<'a, 'b> {
    cx: &'a Cx<'b>,
    out: &'a mut Vec<Finding>,
}

impl<'ast> Visit<'ast> for Walk<'_, '_> {
    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        let hidden = if item.mac.path.is_ident("macro_rules") {
            has_registration(item.mac.tokens.clone())
        } else if item.mac.path.is_ident("include") {
            syn::parse2::<syn::LitStr>(item.mac.tokens.clone())
                .ok()
                .and_then(|path| self.cx.path.parent().map(|parent| parent.join(path.value())))
                .and_then(|path| std::fs::read_to_string(path).ok())
                .and_then(|source| TokenStream::from_str(&source).ok())
                .is_some_and(has_registration)
        } else {
            false
        };
        if hidden {
            self.cx.flag_span(self.out, ID, ID, item.span(), "suite registration inside a macro or include is invisible to registry!".to_owned());
        }
        visit::visit_item_macro(self, item);
    }
}

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    Walk { cx, out }.visit_file(cx.file);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_hidden_registrations_and_allows_unregistered_derive() {
        let hidden = TokenStream::from_str("#[derive(Component)] struct A; #[trial] fn t() {} ").unwrap_or_default();
        let ordinary = TokenStream::from_str("#[derive(Component)] #[component(unregistered)] struct A; ").unwrap_or_default();
        assert!(has_registration(hidden));
        assert!(!has_registration(ordinary));
    }
}
