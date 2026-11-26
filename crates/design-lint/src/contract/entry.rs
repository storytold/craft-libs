//! `contract/entry`: in layers 5 and 6 a type is shown through the contract, not through an inherent entry point of its own.
//! Layer 8 (app panels) is not checked.

use syn::{ImplItem, Item, ReturnType, Type, Visibility};

use crate::report::Finding;

use super::Cx;
use super::sites::summarize;
use super::types::{flat_items, self_name};

pub const ID: &str = "contract/entry";

/// Inherent names that show a type without the pipeline.
const ENTRY_NAMES: [&str; 4] = ["show", "show_at", "show_in", "ui"];

fn is_public(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    if !matches!(cx.scope.layer(cx.path), Some(5 | 6)) {
        return;
    }
    for item in flat_items(&cx.file.items) {
        match item {
            Item::Impl(imp) if imp.trait_.is_none() => {
                let Some(owner) = self_name(imp) else { continue };
                if !cx.types.get(&owner).is_some_and(|f| f.defined && f.public && !f.component && !f.pattern) {
                    continue;
                }
                for m in imp.items.iter().filter_map(|i| if let ImplItem::Fn(m) = i { Some(m) } else { None }) {
                    if !is_public(&m.vis) {
                        continue;
                    }
                    if ENTRY_NAMES.contains(&m.sig.ident.to_string().as_str()) {
                        cx.flag_span(
                            out,
                            ID,
                            ID,
                            m.sig.ident.span(),
                            format!("`{owner}::{}` is an entry point of a type that implements neither `Component` nor `Pattern`", m.sig.ident),
                        );
                    } else if takes_ui(&m.sig) && returns_response(&m.sig.output) {
                        cx.flag_span(
                            out,
                            ID,
                            ID,
                            m.sig.ident.span(),
                            format!("`{owner}::{}` takes `&mut Ui` and returns a response; make `{owner}` a component", m.sig.ident),
                        );
                    }
                }
            }
            Item::Fn(f) if is_public(&f.vis) && takes_ui(&f.sig) && returns_response(&f.sig.output) => {
                cx.flag_span(
                    out,
                    ID,
                    ID,
                    f.sig.ident.span(),
                    format!("`{}` takes `&mut Ui` and returns a response; make it a component", f.sig.ident),
                );
            }
            _ => {}
        }
    }
}

fn takes_ui(sig: &syn::Signature) -> bool {
    sig.inputs.iter().any(|a| {
        matches!(a, syn::FnArg::Typed(t) if matches!(&*t.ty, Type::Reference(r) if r.mutability.is_some() && matches!(&*r.elem, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Ui"))))
    })
}

fn returns_response(output: &ReturnType) -> bool {
    let ReturnType::Type(_, ty) = output else { return false };
    let mut names = Vec::new();
    summarize(quote::ToTokens::to_token_stream(&**ty), &mut names);
    names.iter().any(|n| n.ends_with("Response"))
}
