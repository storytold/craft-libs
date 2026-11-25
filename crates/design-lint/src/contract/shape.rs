//! `contract/shadow`, `contract/widget`, `contract/overlay-flag`: what the derive writes is not written by hand (spec 1.2).

use syn::{ImplItem, Item};

use crate::report::Finding;
use crate::rules::flatten_use;

use super::Cx;
use super::sites::is_egui;
use super::types::{flat_items, self_name, trait_name};

/// An inherent show function on a component.
pub const SHADOW: &str = "contract/shadow";
/// A hand-written `impl egui::Widget`.
pub const WIDGET: &str = "contract/widget";
/// An overridden `overlay` on a type without `#[component(overlay)]`.
pub const OVERLAY_FLAG: &str = "contract/overlay-flag";
/// An `impl Component for` or `impl Pattern for` a type that neither derives it nor appears in `register_contract!`.
pub const REGISTRATION: &str = "contract/registration";

/// The names of the sealed show traits' functions; an inherent function of one of them hides the trait's.
const SHOW_NAMES: [&str; 5] = ["show", "show_at", "show_in", "show_with_tip", "show_at_with_tip"];

fn check_registration(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    for item in flat_items(&cx.file.items) {
        let Item::Impl(imp) = item else { continue };
        let contract = match trait_name(imp).as_deref() {
            Some("Component") => "Component",
            Some("Pattern") => "Pattern",
            _ => continue,
        };
        let Some(owner) = self_name(imp) else { continue };
        let unregistered = cx.types.get(&owner).is_some_and(|f| {
            let derives = if contract == "Component" { f.derives_component } else { f.derives_pattern };
            f.defined && !derives && !f.registered
        });
        if unregistered {
            cx.flag_span(out, REGISTRATION, REGISTRATION, imp.impl_token.span, {
                let option = if contract == "Pattern" { "; pattern" } else { "" };
                format!("`impl {contract} for {owner}` without `#[derive({contract})]`: register it with `register_contract!({owner}{option})`")
            });
        }
    }
}

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    check_registration(cx, out);
    let widget_names = egui_widget_names(cx);
    for item in flat_items(&cx.file.items) {
        let Item::Impl(imp) = item else { continue };
        let Some(owner) = self_name(imp) else { continue };
        let fns = imp.items.iter().filter_map(|i| if let ImplItem::Fn(m) = i { Some(m) } else { None });
        match trait_name(imp).as_deref() {
            None if cx.types.get(&owner).is_some_and(|f| f.component) => {
                for m in fns.filter(|m| SHOW_NAMES.contains(&m.sig.ident.to_string().as_str())) {
                    cx.flag_span(
                        out,
                        SHADOW,
                        SHADOW,
                        m.sig.ident.span(),
                        format!("`{owner}::{}` hides the library's `show`; the pipeline writes it", m.sig.ident),
                    );
                }
            }
            Some(_) if is_egui_widget(imp, &widget_names) => {
                cx.flag_span(out, WIDGET, WIDGET, imp.impl_token.span, format!("`impl egui::Widget for {owner}`; `#[component(widget)]` writes it"));
            }
            Some("Component") => {
                let flagged = cx.types.get(&owner).is_some_and(|f| f.defined && !f.overlay_attr);
                if let Some(m) = fns.into_iter().find(|m| m.sig.ident == "overlay").filter(|_| flagged) {
                    cx.flag_span(
                        out,
                        OVERLAY_FLAG,
                        OVERLAY_FLAG,
                        m.sig.ident.span(),
                        format!("`{owner}` overrides `overlay` without `#[component(overlay)]`"),
                    );
                }
            }
            _ => {}
        }
    }
}

/// The names the file binds to `egui::..::Widget`, and whether it globs an egui module.
struct WidgetNames {
    bound: Vec<String>,
    glob: bool,
}

fn egui_widget_names(cx: &Cx<'_>) -> WidgetNames {
    let mut names = WidgetNames { bound: Vec::new(), glob: false };
    for item in flat_items(&cx.file.items) {
        let Item::Use(u) = item else { continue };
        let mut found = Vec::new();
        flatten_use(&mut Vec::new(), &u.tree, &mut found);
        for (alias, path) in found.into_iter().filter(|(_, p)| is_egui(p)) {
            match alias {
                Some(alias) if path.last().is_some_and(|l| l == "Widget") => names.bound.push(alias),
                Some(_) => {}
                None => names.glob = true,
            }
        }
    }
    names
}

fn is_egui_widget(imp: &syn::ItemImpl, names: &WidgetNames) -> bool {
    let Some((_, path, _)) = &imp.trait_ else { return false };
    let Some(first) = path.segments.first().map(|s| s.ident.to_string()) else { return false };
    if path.segments.len() > 1 {
        return super::sites::EGUI_ROOTS.contains(&first.as_str()) && path.segments.last().is_some_and(|last| last.ident == "Widget");
    }
    names.bound.contains(&first) || (names.glob && first == "Widget")
}
