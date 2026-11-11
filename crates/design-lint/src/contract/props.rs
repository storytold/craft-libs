//! `props/*`: the `Props` setters come from `#[derive(Props)]` and the `Props` trait, never from hand-written code (spec 3.2).

use proc_macro2::TokenTree;
use syn::spanned::Spanned as _;
use syn::{Expr, ImplItem, ImplItemFn, Item, ItemImpl, ItemStruct, Stmt, Type};

use crate::report::Finding;

use super::Cx;
use super::types::{derives, flat_items, self_name};

/// An inherent builder method named like a `Props` setter.
pub const RESERVED: &str = "props/reserved-setter";
/// An inherent builder method that only stores its argument in one field.
pub const HAND_SETTER: &str = "props/hand-setter";
/// A component field that `Common` already holds.
pub const OWN_FIELD: &str = "props/own-field";
/// A `#[prop]` field whose generated setter shadows a `Props` setter.
pub const SHADOW: &str = "props/shadow";
/// A size in points as a prop or a builder argument.
pub const POINT_SIZE: &str = "props/point-size";

/// The `Props` setters and the `Common` fields they write.
const SETTERS: [&str; 13] =
    ["id_salt", "id", "label", "enabled", "read_only", "size", "variant", "tooltip", "description", "selected", "doc_key", "command", "keeps_name"];

/// Fields a component takes from `Common`.
const COMMON_FIELDS: [&str; 3] = ["enabled", "id_salt", "tooltip"];

/// Identifiers a value may mention and still be a plain store: conversions and wrappers.
const WRAPPERS: [&str; 10] = ["Some", "into", "to_owned", "to_string", "Into", "from", "Box", "Arc", "new", "String"];

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    let component_layer = matches!(cx.scope.layer(cx.path), Some(5..=8));
    for item in flat_items(&cx.file.items) {
        match item {
            Item::Impl(imp) if imp.trait_.is_none() && component_layer && builds_props(cx, imp) => setters(cx, imp, out),
            Item::Enum(e) if component_layer => size_variants(cx, e, out),
            Item::Struct(s) => fields(cx, s, out),
            _ => {}
        }
    }
}

/// Whether the impl is of a type known to be a component or a `Props` type.
pub(super) fn builds_props(cx: &Cx<'_>, imp: &ItemImpl) -> bool {
    self_name(imp).is_some_and(|name| cx.types.get(&name).is_some_and(|f| f.component || f.props))
}

fn setters(cx: &Cx<'_>, imp: &ItemImpl, out: &mut Vec<Finding>) {
    let owner = self_name(imp).unwrap_or_default();
    for m in imp.items.iter().filter_map(|i| if let ImplItem::Fn(m) = i { Some(m) } else { None }) {
        if !is_builder(m, &owner) {
            continue;
        }
        let name = m.sig.ident.to_string();
        if SETTERS.contains(&name.as_str()) {
            cx.flag_span(out, RESERVED, RESERVED, m.sig.ident.span(), format!("`{owner}::{name}` is a `Props` setter name; `Props` provides it"));
        }
        for param in m.sig.inputs.iter().filter_map(|a| if let syn::FnArg::Typed(t) = a { Some(t) } else { None }) {
            let syn::Pat::Ident(pat) = &*param.pat else { continue };
            if is_point_size(&param.ty, &pat.ident.to_string()) {
                cx.flag_span(
                    out,
                    POINT_SIZE,
                    POINT_SIZE,
                    param.ty.span(),
                    format!("`{owner}::{name}` takes `{}` as a size; {SIZE_ADVICE}", pat.ident),
                );
            }
        }
        if only_stores(m) {
            cx.flag_span(
                out,
                HAND_SETTER,
                HAND_SETTER,
                m.sig.ident.span(),
                format!("`{owner}::{name}` only stores its argument; mark the field `#[prop]` and let `#[derive(Props)]` write the setter"),
            );
        }
    }
}

/// `fn name(self, ..) -> Self` (or the owner's name): a by-value receiver and a returned owner.
fn is_builder(m: &ImplItemFn, owner: &str) -> bool {
    let by_value = m.sig.receiver().is_some_and(|r| r.reference.is_none() && r.colon_token.is_none());
    let returns_owner = match &m.sig.output {
        syn::ReturnType::Type(_, ty) => {
            matches!(&**ty, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Self" || s.ident == owner))
        }
        syn::ReturnType::Default => false,
    };
    by_value && returns_owner
}

/// The body stores the one parameter, possibly converted or wrapped, in one field of `self`: `self.field = value; self` or
/// `Self { field: value, ..self }`.
fn only_stores(m: &ImplItemFn) -> bool {
    let mut params = m.sig.inputs.iter().filter_map(|a| if let syn::FnArg::Typed(t) = a { Some(t) } else { None });
    let (Some(param), None) = (params.next(), params.next()) else { return false };
    let syn::Pat::Ident(param) = &*param.pat else { return false };
    let param = param.ident.to_string();
    match m.block.stmts.as_slice() {
        [Stmt::Expr(Expr::Assign(assign), Some(_)), Stmt::Expr(Expr::Path(tail), None)] => {
            tail.path.is_ident("self") && is_self_field(&assign.left) && mentions_only(&assign.right, &param)
        }
        [Stmt::Expr(Expr::Struct(update), None)] => {
            let rest_is_self = matches!(update.rest.as_deref(), Some(Expr::Path(p)) if p.path.is_ident("self"));
            rest_is_self && update.qself.is_none() && update.fields.len() == 1 && mentions_only(&update.fields[0].expr, &param)
        }
        _ => false,
    }
}

/// Exactly `self.<field>`: the derive cannot write a nested path.
fn is_self_field(e: &Expr) -> bool {
    matches!(e, Expr::Field(f) if matches!(&*f.base, Expr::Path(p) if p.path.is_ident("self")))
}

fn mentions_only(e: &Expr, param: &str) -> bool {
    let mut idents = Vec::new();
    super::sites::summarize(quote::ToTokens::to_token_stream(e), &mut idents);
    idents.iter().all(|i| i == param || WRAPPERS.contains(&i.as_str()))
}

fn fields(cx: &Cx<'_>, s: &ItemStruct, out: &mut Vec<Finding>) {
    let named = |f: &syn::Field| f.ident.as_ref().map(ToString::to_string);
    let derived = derives(&s.attrs);
    // A plain options struct that derives only `Props` (such as `Segment`) has no `Common` and implements no `Props`
    // trait, so its setters shadow nothing.
    let component = derived.iter().any(|d| d == "Component") || s.fields.iter().any(|f| f.attrs.iter().any(|a| a.path().is_ident("common")));
    for f in &s.fields {
        let Some(field) = named(f) else { continue };
        if component && COMMON_FIELDS.contains(&field.as_str()) {
            cx.flag_span(out, OWN_FIELD, OWN_FIELD, f.span(), format!("`{}::{field}` duplicates the `Common` field of that name", s.ident));
        }
        let Some(prop) = f.attrs.iter().find(|a| a.path().is_ident("prop")) else { continue };
        let setter = setter_name(prop).unwrap_or_else(|| field.clone());
        if component && SETTERS.contains(&setter.as_str()) {
            cx.flag_span(out, SHADOW, SHADOW, f.span(), format!("the setter `{}::{setter}` shadows the `Props` setter of that name", s.ident));
        }
        if !declares_quantity(prop) && is_point_size(&f.ty, &field) {
            cx.flag_span(out, POINT_SIZE, POINT_SIZE, f.ty.span(), format!("`{}::{field}` is a size in points; {SIZE_ADVICE}", s.ident));
        }
    }
}

/// The `name = ".."` of a `#[prop(..)]` attribute.
fn setter_name(attr: &syn::Attribute) -> Option<String> {
    let syn::Meta::List(list) = &attr.meta else { return None };
    let tokens: Vec<TokenTree> = list.tokens.clone().into_iter().collect();
    tokens.windows(3).find_map(|w| match w {
        [TokenTree::Ident(name), TokenTree::Punct(eq), TokenTree::Literal(lit)] if name == "name" && eq.as_char() == '=' => {
            Some(lit.to_string().trim_matches('"').to_owned())
        }
        _ => None,
    })
}

const SIZE_ADVICE: &str =
    "sizes come from a token step (`ControlSize`), an aspect ratio or the content; a number that is not a size declares `#[prop(quantity = \"..\")]`";

/// Types that name a length in points.
const SIZE_TYPES: [&str; 6] = ["Px", "Points", "Pt", "Size", "Width", "Height"];

/// Words in a name that make an integer a size.
const SIZE_WORDS: [&str; 13] =
    ["width", "widths", "height", "heights", "size", "sizes", "side", "extent", "px", "points", "gap", "padding", "radius"];

/// `#[prop(quantity = "..")]`: the field declares what it measures, and it is not a size.
fn declares_quantity(attr: &syn::Attribute) -> bool {
    let syn::Meta::List(list) = &attr.meta else { return false };
    list.tokens.clone().into_iter().any(|t| matches!(t, TokenTree::Ident(i) if i == "quantity"))
}

/// Whether a value of type `ty` named `name` is a length in points: a float or `Vec2` of any name; a newtype named like
/// a size (`Px`, `Points`, `Size`, `Width`, `Height`); an integer named like a size. Looks through `Option`.
fn is_point_size(ty: &Type, name: &str) -> bool {
    let Type::Path(p) = ty else { return false };
    let Some(last) = p.path.segments.last() else { return false };
    let ident = last.ident.to_string();
    match ident.as_str() {
        "f32" | "f64" | "Vec2" => true,
        "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize" => name.split('_').any(|w| SIZE_WORDS.contains(&w)),
        "Option" => {
            matches!(&last.arguments, syn::PathArguments::AngleBracketed(a) if a.args.iter().any(|g| matches!(g, syn::GenericArgument::Type(t) if is_point_size(t, name))))
        }
        other => SIZE_TYPES.contains(&other),
    }
}

/// An enum variant named `Px`, `Points` or `Pt` that carries a number: a size chosen in points.
fn size_variants(cx: &Cx<'_>, e: &syn::ItemEnum, out: &mut Vec<Finding>) {
    for v in &e.variants {
        let named_like_points = matches!(v.ident.to_string().as_str(), "Px" | "Points" | "Pt");
        if named_like_points && v.fields.iter().any(|f| is_point_size(&f.ty, "px")) {
            cx.flag_span(out, POINT_SIZE, POINT_SIZE, v.ident.span(), format!("`{}::{}` is a size in points; {SIZE_ADVICE}", e.ident, v.ident));
        }
    }
}
