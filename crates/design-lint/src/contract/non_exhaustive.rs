//! `api/non-exhaustive`: public enums, struct-like variants and structs with public fields need `#[non_exhaustive]`.

use syn::{Attribute, Fields, Item, Visibility};

use crate::report::Finding;
use crate::rules::is_test_attr;

use super::Cx;

pub const ID: &str = "api/non-exhaustive";

fn has(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

fn doc_hidden(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("doc") && quote::ToTokens::to_token_stream(&a.meta).to_string().contains("hidden"))
}

fn is_pub(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    items(cx, &cx.file.items, out);
}

/// Walks the items reachable through `pub` modules; private modules, test code and hidden items and fields are not
/// public API.
fn items(cx: &Cx<'_>, list: &[Item], out: &mut Vec<Finding>) {
    for item in list {
        match item {
            Item::Mod(m) if is_pub(&m.vis) && !doc_hidden(&m.attrs) && !is_test_attr_any(&m.attrs) => {
                if let Some((_, inner)) = &m.content {
                    items(cx, inner, out);
                }
            }
            Item::Enum(e) if is_pub(&e.vis) && !doc_hidden(&e.attrs) && !is_test_attr_any(&e.attrs) && !cx.scope.is_closed(&e.ident.to_string()) => {
                if !has(&e.attrs, "non_exhaustive") {
                    cx.flag_span(
                        out,
                        ID,
                        ID,
                        e.ident.span(),
                        format!("public enum `{}` needs `#[non_exhaustive]` or a place on the closed list", e.ident),
                    );
                }
                for v in &e.variants {
                    if matches!(v.fields, Fields::Named(_)) && !has(&v.attrs, "non_exhaustive") {
                        cx.flag_span(
                            out,
                            ID,
                            ID,
                            v.ident.span(),
                            format!("struct-like variant `{}::{}` needs `#[non_exhaustive]`", e.ident, v.ident),
                        );
                    }
                }
            }
            Item::Struct(s)
                if is_pub(&s.vis) && !doc_hidden(&s.attrs) && !is_test_attr_any(&s.attrs) && !cx.scope.is_closed(&s.ident.to_string()) =>
            {
                let public_fields = match &s.fields {
                    Fields::Named(f) => f.named.iter().any(|f| is_pub(&f.vis) && !doc_hidden(&f.attrs)),
                    Fields::Unnamed(f) => f.unnamed.iter().any(|f| is_pub(&f.vis) && !doc_hidden(&f.attrs)),
                    Fields::Unit => false,
                };
                if public_fields && !has(&s.attrs, "non_exhaustive") {
                    cx.flag_span(
                        out,
                        ID,
                        ID,
                        s.ident.span(),
                        format!("public struct `{}` with public fields needs `#[non_exhaustive]` or a place on the closed list", s.ident),
                    );
                }
            }
            _ => {}
        }
    }
}

fn is_test_attr_any(attrs: &[Attribute]) -> bool {
    attrs.iter().any(is_test_attr)
}
