//! `style/*`: looks come from style parts built in `fn style` (spec 4.2, 4.3, 4.5). Component code never picks a colour from
//! response state, matches on `State` or `ControlSize`, holds per-state fields, reads tokens, or takes a per-instance style.
//!
//! The rules read layers 3 to 8; `craft_ui::style` is layer 2 and craft-theme layer 0, so the resolver and the
//! caches are outside them.

use proc_macro2::TokenStream;
use quote::ToTokens as _;
use syn::spanned::Spanned as _;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, ImplItem, Item, ItemImpl, ItemStruct, Type};

use crate::report::Finding;
use crate::rules::is_test_attr;

use super::Cx;
use super::sites::summarize;
use super::types::{derives, flat_items, self_name};

/// A colour chosen with an `if` on response state.
pub const STATE_PICK: &str = "style/state-pick";
/// A `match` with an arm on a `State` variant.
pub const MATCH_STATE: &str = "style/match-state";
/// A style struct with `States<_>` or per-state copy fields.
pub const STATES_FIELD: &str = "style/states-field";
/// A `match` with an arm on a `ControlSize` variant.
pub const SIZE_ARM: &str = "style/size-arm";
/// A token read outside a function that takes a `StyleCx`.
pub const TOKEN_READ: &str = "style/token-read";
/// A builder method that sets a style on one instance.
pub const INSTANCE_SETTER: &str = "style/instance-setter";
/// `.to_egui()` in a style builder.
pub const TO_EGUI: &str = "style/to-egui";

/// Response queries a hand-written colour pick branches on: hover, press and focus (not events such as `clicked`).
const RESPONSE_STATE: [&str; 6] = ["hovered", "is_hovered", "contains_pointer", "highlighted", "is_pointer_button_down_on", "has_focus"];

/// Egui calls that pick a colour from a response's state.
const INTERACT_CALLS: [&str; 2] = ["interact", "interact_selectable"];

const STATE_VARIANTS: [&str; 4] = ["Rest", "Hover", "Pressed", "Disabled"];

/// Methods that read tokens from an env or a `Ui`, and the receivers they are read from.
const TOKEN_METHODS: [&str; 3] = ["tokens", "ui_tokens", "token_group"];
const TOKEN_RECEIVERS: [&str; 3] = ["ui", "cx", "env"];

/// Builder names that set a style or palette on one instance.
const INSTANCE_NAMES: [&str; 12] = [
    "style",
    "with_style",
    "style_override",
    "override_style",
    "look",
    "with_look",
    "colors",
    "with_colors",
    "palette",
    "with_palette",
    "skin",
    "with_skin",
];

/// Style parts: a parameter of one of these is a per-instance style.
const STYLE_PARTS: [&str; 5] = ["SurfaceLook", "TextLook", "IconLook", "FocusLook", "VariantLook"];

/// Colour and shape types a style builder takes from tokens; `DataColor` (data) is not one of them.
const STYLE_VALUES: [&str; 7] = ["Color32", "Color", "Rgba", "StyleColor", "Stroke", "CornerRadius", "PathStroke"];

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    if !matches!(cx.scope.layer(cx.path), Some(3..=8)) {
        return;
    }
    for item in flat_items(&cx.file.items) {
        match item {
            Item::Struct(s) => style_fields(cx, s, out),
            Item::Impl(imp) if imp.trait_.is_none() => instance_setters(cx, imp, out),
            _ => {}
        }
    }
    let mut walk = Walk { cx, out, in_style: false };
    walk.visit_file(cx.file);
}

fn is_style_type(s: &ItemStruct) -> bool {
    let name = s.ident.to_string();
    name.ends_with("Style") || name.ends_with("Look") || derives(&s.attrs).iter().any(|d| d == "ComponentStyle")
}

fn per_state_name(name: &str) -> bool {
    ["hover", "pressed", "disabled"].iter().any(|s| name.ends_with(&format!("_{s}")) || name.starts_with(&format!("{s}_")))
        || name.starts_with("selected_")
        || name.ends_with("_selected")
}

fn style_fields(cx: &Cx<'_>, s: &ItemStruct, out: &mut Vec<Finding>) {
    if !is_style_type(s) {
        return;
    }
    for f in &s.fields {
        if let Some(name) = f.ident.as_ref().map(ToString::to_string).filter(|n| per_state_name(n)) {
            cx.flag_span(
                out,
                STATES_FIELD,
                STATES_FIELD,
                f.span(),
                format!("`{}::{name}` is a per-state copy field; hold one `Stateful<_>`", s.ident),
            );
        }
        if names_type(&f.ty, "States") {
            cx.flag_span(
                out,
                STATES_FIELD,
                STATES_FIELD,
                f.ty.span(),
                format!("`{}` holds `States<_>`; the one per-state type is `Stateful<_>`", s.ident),
            );
        }
    }
}

fn names_type(ty: &Type, name: &str) -> bool {
    struct Find<'n>(&'n str, bool);
    impl<'ast> Visit<'ast> for Find<'_> {
        fn visit_path_segment(&mut self, s: &'ast syn::PathSegment) {
            self.1 |= s.ident == self.0;
            visit::visit_path_segment(self, s);
        }
    }
    let mut find = Find(name, false);
    find.visit_type(ty);
    find.1
}

fn instance_setters(cx: &Cx<'_>, imp: &ItemImpl, out: &mut Vec<Finding>) {
    let owner = self_name(imp).unwrap_or_default();
    let Some(facts) = cx.types.get(&owner).filter(|f| f.component || f.props) else { return };
    for m in imp.items.iter().filter_map(|i| if let ImplItem::Fn(m) = i { Some(m) } else { None }) {
        let builder = m.sig.receiver().is_some_and(|r| r.reference.is_none() && r.colon_token.is_none())
            && matches!(&m.sig.output, syn::ReturnType::Type(_, t) if matches!(&**t, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Self" || s.ident == owner.as_str())));
        let name = m.sig.ident.to_string();
        let params: Vec<&Type> = m.sig.inputs.iter().filter_map(|a| if let syn::FnArg::Typed(t) = a { Some(&*t.ty) } else { None }).collect();
        let style_param = params.iter().any(|t| names_style(cx, facts, t));
        let value_param = matches!(params.as_slice(), [only] if names_any(only, &STYLE_VALUES) && !names_type(only, "DataColor"));
        if builder && (INSTANCE_NAMES.contains(&name.as_str()) || style_param || value_param) {
            cx.flag_span(
                out,
                INSTANCE_SETTER,
                INSTANCE_SETTER,
                m.sig.ident.span(),
                format!("`{owner}::{name}` sets a style on one instance; styles come from `fn style(cx, key)` and `Looks`"),
            );
        }
    }
}

/// Whether the type names the owner's `Style`, a `ComponentStyle` type or a style part.
fn names_style(cx: &Cx<'_>, owner: &super::types::TypeFacts, ty: &Type) -> bool {
    let mut text = Vec::new();
    summarize(ty.to_token_stream(), &mut text);
    text.iter().any(|t| {
        STYLE_PARTS.contains(&t.as_str()) || owner.style_type.as_deref() == Some(t.as_str()) || cx.types.get(t).is_some_and(|f| f.component_style)
    })
}

fn names_any(ty: &Type, names: &[&str]) -> bool {
    names.iter().any(|n| names_type(ty, n))
}

struct Walk<'c, 'a> {
    cx: &'c Cx<'a>,
    out: &'c mut Vec<Finding>,
    in_style: bool,
}

fn is_style_fn(sig: &syn::Signature) -> bool {
    sig.ident == "style" || sig.inputs.iter().any(|a| matches!(a, syn::FnArg::Typed(t) if names_type(&t.ty, "StyleCx")))
}

fn skipped(attrs: &[Attribute]) -> bool {
    attrs.iter().any(is_test_attr)
}

struct Paths(Vec<Vec<String>>);

impl<'ast> Visit<'ast> for Paths {
    fn visit_path(&mut self, p: &'ast syn::Path) {
        self.0.push(p.segments.iter().map(|s| s.ident.to_string()).collect());
    }
}

/// `State::<variant>` or `ControlSize::<variant>`: the owner and the variant of a path.
fn owner_variant(path: &[String]) -> Option<(&str, &str)> {
    let [.., owner, variant] = path else { return None };
    ((owner == "State" && STATE_VARIANTS.contains(&variant.as_str())) || owner == "ControlSize").then_some((owner.as_str(), variant.as_str()))
}

impl Walk<'_, '_> {
    fn in_fn(&mut self, sig: &syn::Signature, walk: impl FnOnce(&mut Self)) {
        let before = std::mem::replace(&mut self.in_style, is_style_fn(sig));
        walk(self);
        self.in_style = before;
    }

    fn report_variant(&mut self, owner: &str, variant: &str, span: proc_macro2::Span) {
        if owner == "State" {
            self.cx.flag_span(
                self.out,
                MATCH_STATE,
                MATCH_STATE,
                span,
                format!("`State::{variant}` test: read the look from `Stateful<_>`, not from a test on `State`"),
            );
        } else {
            self.cx.flag_span(
                self.out,
                SIZE_ARM,
                SIZE_ARM,
                span,
                format!("`ControlSize::{variant}` arm: sizes come from a `BySize` table and `ControlMetrics`"),
            );
        }
    }

    fn pattern_variants(&mut self, pat: &syn::Pat) {
        let mut paths = Paths(Vec::new());
        paths.visit_pat(pat);
        for p in paths.0 {
            if let Some((owner, variant)) = owner_variant(&p) {
                self.report_variant(owner, variant, pat.span());
            }
        }
    }

    fn pick_from_state(&mut self, span: proc_macro2::Span) {
        self.cx.flag_span(
            self.out,
            STATE_PICK,
            STATE_PICK,
            span,
            "a colour picked from response state; the style's `Stateful<_>` and `cx.resolve` do this".to_owned(),
        );
    }
}

fn idents(tokens: TokenStream) -> Vec<String> {
    let mut out = Vec::new();
    summarize(tokens, &mut out);
    out
}

fn asks_state(tokens: TokenStream) -> bool {
    idents(tokens).iter().any(|i| RESPONSE_STATE.contains(&i.as_str()))
}

/// A name that holds a colour: `Color32`, `StyleColor`, `hover_color`, `bg_fill`, `fill`.
fn is_colour_ident(i: &str) -> bool {
    let lower = i.to_ascii_lowercase();
    matches!(i, "Color32" | "Color" | "StyleColor" | "Rgba" | "DataColor")
        || lower.contains("color")
        || lower.contains("colour")
        || lower == "fill"
        || lower.ends_with("_fill")
}

/// The value a block ends in.
fn tail(block: &syn::Block) -> Option<&Expr> {
    match block.stmts.last() {
        Some(syn::Stmt::Expr(e, None)) => Some(e),
        _ => None,
    }
}

/// Whether the expression is a colour value: it names a colour.
fn is_colour_value(e: &Expr) -> bool {
    match e {
        Expr::Block(b) => tail(&b.block).is_some_and(is_colour_value),
        Expr::Closure(c) => is_colour_value(&c.body),
        _ => idents(e.to_token_stream()).iter().any(|i| is_colour_ident(i)),
    }
}

impl<'ast> Visit<'ast> for Walk<'_, '_> {
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        if !skipped(&i.attrs) {
            self.in_fn(&i.sig, |w| visit::visit_item_fn(w, i));
        }
    }
    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        if !skipped(&i.attrs) {
            self.in_fn(&i.sig, |w| visit::visit_impl_item_fn(w, i));
        }
    }
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        if !skipped(&i.attrs) {
            visit::visit_item_mod(self, i);
        }
    }
    fn visit_expr_if(&mut self, e: &'ast syn::ExprIf) {
        let picks = tail(&e.then_branch).is_some_and(is_colour_value)
            || matches!(e.else_branch.as_ref().map(|(_, other)| &**other), Some(Expr::Block(b)) if tail(&b.block).is_some_and(is_colour_value));
        if picks && asks_state(e.cond.to_token_stream()) {
            self.pick_from_state(e.if_token.span);
        }
        visit::visit_expr_if(self, e);
    }
    fn visit_expr_match(&mut self, e: &'ast syn::ExprMatch) {
        for arm in &e.arms {
            self.pattern_variants(&arm.pat);
        }
        if asks_state(e.expr.to_token_stream()) && e.arms.iter().any(|a| is_colour_value(&a.body)) {
            self.pick_from_state(e.match_token.span);
        }
        visit::visit_expr_match(self, e);
    }
    fn visit_expr_let(&mut self, e: &'ast syn::ExprLet) {
        self.pattern_variants(&e.pat);
        visit::visit_expr_let(self, e);
    }
    fn visit_expr_binary(&mut self, e: &'ast syn::ExprBinary) {
        if matches!(e.op, syn::BinOp::Eq(_) | syn::BinOp::Ne(_)) {
            for side in [&*e.left, &*e.right] {
                if let Expr::Path(p) = side {
                    let path: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
                    if let Some((owner, variant)) = owner_variant(&path).filter(|(o, _)| *o == "State") {
                        self.report_variant(owner, variant, p.span());
                    }
                }
            }
        }
        visit::visit_expr_binary(self, e);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        if m.path.is_ident("matches") {
            let tokens: Vec<String> = idents(m.tokens.clone());
            for w in tokens.windows(2) {
                if w[0] == "State" && STATE_VARIANTS.iter().any(|v| *v == w[1]) {
                    self.report_variant("State", &w[1], m.span());
                }
            }
        }
        visit::visit_macro(self, m);
    }
    fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
        let name = e.method.to_string();
        let receiver = idents(e.receiver.to_token_stream());
        if name == "to_egui" && self.in_style {
            self.cx.flag_span(
                self.out,
                TO_EGUI,
                TO_EGUI,
                e.method.span(),
                "`.to_egui()` in a style builder; `cx.color(..)` makes the `StyleColor`".to_owned(),
            );
        } else if TOKEN_METHODS.contains(&name.as_str()) && !self.in_style && receiver.iter().any(|r| TOKEN_RECEIVERS.contains(&r.as_str())) {
            self.cx.flag_span(
                self.out,
                TOKEN_READ,
                TOKEN_READ,
                e.method.span(),
                format!("`.{name}(..)` reads tokens outside `fn style`; read them in the style and keep the result in the `Style`"),
            );
        }
        let state_receiver = receiver.iter().any(|r| RESPONSE_STATE.contains(&r.as_str()));
        if matches!(name.as_str(), "then_some" | "then") && state_receiver && e.args.iter().any(is_colour_value) {
            self.pick_from_state(e.method.span());
        }
        let egui_style = receiver.iter().any(|r| matches!(r.as_str(), "style" | "visuals" | "widgets"));
        if name == "interact_selectable" || (INTERACT_CALLS.contains(&name.as_str()) && egui_style) {
            self.pick_from_state(e.method.span());
        }
        visit::visit_expr_method_call(self, e);
    }
    fn visit_expr_path(&mut self, e: &'ast syn::ExprPath) {
        let names: Vec<String> = e.path.segments.iter().map(|s| s.ident.to_string()).collect();
        let reads = names.windows(2).any(|w| matches!(w[0].as_str(), "Tokens" | "UiTokens") && w[1] == "of");
        if reads && !self.in_style {
            self.cx.flag_span(self.out, TOKEN_READ, TOKEN_READ, e.span(), format!("`{}` reads tokens outside `fn style`", names.join("::")));
        }
        visit::visit_expr_path(self, e);
    }
}
