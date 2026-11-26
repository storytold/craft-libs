//! What one file says about its types and impls, shared by the rules that read item shapes.
//!
//! A type is a component when the file derives or attributes it as one, or implements `Component` for it.
//! Types defined in another file are not known; rules that need a type's definition skip them.

use std::collections::HashMap;

use syn::{Attribute, Item, ItemImpl, Visibility};

use crate::rules::is_test_attr;

/// Facts about one type name.
#[derive(Clone, Debug, Default)]
pub struct TypeFacts {
    /// The facts come from the type's definition.
    pub defined: bool,
    /// Declared `pub` without a restriction.
    pub public: bool,
    /// Derived, attributed or implemented as a `Component`.
    pub component: bool,
    /// Carries `#[derive(Component)]`, which registers the type.
    pub derives_component: bool,
    /// Carries `#[derive(Pattern)]`, which registers the type.
    pub derives_pattern: bool,
    /// Named in a `register_contract!(..)` anywhere in its crate.
    pub registered: bool,
    /// Derives `Props` or has a `#[prop]` field.
    pub props: bool,
    /// Implements or derives the `Pattern` marker.
    pub pattern: bool,
    /// Carries `#[component(.., overlay, ..)]`.
    pub overlay_attr: bool,
    /// Derives `ComponentStyle`.
    pub component_style: bool,
    /// The `type Style = ..` of its `impl Component`.
    pub style_type: Option<String>,
}

/// The facts of the type names a file mentions.
#[derive(Clone, Debug, Default)]
pub struct Types {
    by_name: HashMap<String, TypeFacts>,
}

impl Types {
    pub(crate) fn get(&self, name: &str) -> Option<&TypeFacts> {
        self.by_name.get(name)
    }

    pub(crate) fn insert(&mut self, name: String, facts: TypeFacts) {
        self.by_name.insert(name, facts);
    }

    /// `self` with every entry of `over` replacing the entry of that name.
    pub(crate) fn overlaid(mut self, over: &Types) -> Self {
        self.by_name.extend(over.by_name.iter().map(|(k, v)| (k.clone(), v.clone())));
        self
    }
}

/// Every item of the file, inline modules included; test code is left out.
pub(crate) fn flat_items<'a>(items: impl IntoIterator<Item = &'a Item>) -> Vec<&'a Item> {
    let mut out = Vec::new();
    walk(items, &mut out);
    out
}

fn walk<'a>(items: impl IntoIterator<Item = &'a Item>, out: &mut Vec<&'a Item>) {
    for item in items {
        if attrs_of(item).iter().any(is_test_attr) {
            continue;
        }
        out.push(item);
        if let Item::Mod(m) = item
            && let Some((_, inner)) = &m.content
        {
            walk(inner, out);
        }
    }
}

fn attrs_of(item: &Item) -> &[Attribute] {
    match item {
        Item::Struct(i) => &i.attrs,
        Item::Enum(i) => &i.attrs,
        Item::Impl(i) => &i.attrs,
        Item::Mod(i) => &i.attrs,
        Item::Fn(i) => &i.attrs,
        _ => &[],
    }
}

/// The last identifier of a `derive(..)` list entry, for every derive attribute in `attrs`.
pub(crate) fn derives(attrs: &[Attribute]) -> Vec<String> {
    let mut names = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("derive")) {
        let _ = attr.parse_nested_meta(|meta| {
            if let Some(last) = meta.path.segments.last() {
                names.push(last.ident.to_string());
            }
            Ok(())
        });
    }
    names
}

/// Whether the attribute is `name(..)` with `word` as one of its own top-level identifiers (not one nested in a group).
pub(crate) fn attr_names(attr: &Attribute, name: &str, word: &str) -> bool {
    attr.path().is_ident(name)
        && matches!(&attr.meta, syn::Meta::List(l) if l.tokens.clone().into_iter().any(|t| matches!(t, proc_macro2::TokenTree::Ident(i) if i == word)))
}

/// The last identifier of an impl's self type.
pub(crate) fn self_name(imp: &ItemImpl) -> Option<String> {
    let mut ty = &*imp.self_ty;
    while let syn::Type::Reference(r) = ty {
        ty = &r.elem;
    }
    match ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

/// The last identifier of an impl's trait path.
pub(crate) fn trait_name(imp: &ItemImpl) -> Option<String> {
    imp.trait_.as_ref().and_then(|(_, path, _)| path.segments.last()).map(|s| s.ident.to_string())
}

fn is_public(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

/// The facts a type's definition gives.
pub(crate) fn definition(item: &Item) -> Option<(String, TypeFacts)> {
    let (ident, attrs, vis) = match item {
        Item::Struct(s) => (&s.ident, &s.attrs, &s.vis),
        Item::Enum(e) => (&e.ident, &e.attrs, &e.vis),
        _ => return None,
    };
    let derived = derives(attrs);
    let has_prop_field = matches!(item, Item::Struct(s) if s.fields.iter().any(|f| f.attrs.iter().any(|a| a.path().is_ident("prop"))));
    let facts = TypeFacts {
        defined: true,
        public: is_public(vis),
        component: derived.iter().any(|d| d == "Component") || attrs.iter().any(|a| a.path().is_ident("component")),
        derives_component: derived.iter().any(|d| d == "Component"),
        registered: false,
        props: derived.iter().any(|d| d == "Props") || has_prop_field,
        derives_pattern: derived.iter().any(|d| d == "Pattern"),
        pattern: derived.iter().any(|d| d == "Pattern"),
        overlay_attr: attrs.iter().any(|a| attr_names(a, "component", "overlay")),
        component_style: derived.iter().any(|d| d == "ComponentStyle"),
        style_type: None,
    };
    Some((ident.to_string(), facts))
}

/// What a trait impl adds to its self type's facts.
pub(crate) fn add_impl(facts: &mut TypeFacts, imp: &ItemImpl) {
    match trait_name(imp).as_deref() {
        Some("Component") => {
            facts.component = true;
            facts.style_type = imp.items.iter().find_map(|i| match i {
                syn::ImplItem::Type(t) if t.ident == "Style" => match &t.ty {
                    syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
                    _ => None,
                },
                _ => None,
            });
        }
        Some("Pattern") => facts.pattern = true,
        _ => {}
    }
}

/// The identifiers inside the `register_contract!(..)` invocations among `items`.
pub(crate) fn registered_idents<'a>(items: impl IntoIterator<Item = &'a Item>) -> Vec<String> {
    fn idents(tokens: proc_macro2::TokenStream, names: &mut Vec<String>) {
        for token in tokens {
            match token {
                proc_macro2::TokenTree::Ident(ident) => names.push(ident.to_string()),
                proc_macro2::TokenTree::Group(group) => idents(group.stream(), names),
                _ => {}
            }
        }
    }
    let mut names = Vec::new();
    for item in flat_items(items) {
        if let Item::Macro(m) = item
            && m.mac.path.segments.last().is_some_and(|s| s.ident == "register_contract")
        {
            idents(m.mac.tokens.clone(), &mut names);
        }
    }
    names
}

/// Facts from this file alone: the fallback when no crate-wide facts are known.
pub(crate) fn collect(file: &syn::File) -> Types {
    let mut types = Types::default();
    let items = flat_items(&file.items);
    for (name, facts) in items.iter().filter_map(|i| definition(i)) {
        types.by_name.insert(name, facts);
    }
    for item in &items {
        if let Item::Impl(imp) = item
            && let Some(name) = self_name(imp)
        {
            add_impl(types.by_name.entry(name).or_default(), imp);
        }
    }
    for name in registered_idents(&file.items) {
        if let Some(facts) = types.by_name.get_mut(&name) {
            facts.registered = true;
        }
    }
    types
}
