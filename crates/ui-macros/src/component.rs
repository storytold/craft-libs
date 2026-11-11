//! `#[derive(Component)]` and `register_contract!`.

use crate::util::named_fields;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{Attribute, DeriveInput, Field, GenericParam, Generics, Ident, Lifetime, LitStr, Meta, Token, Type};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Leaf,
    Container,
    Collection,
}

/// What `#[component(..)]` and `register_contract!` declare about a type besides its shape.
#[derive(Default)]
struct Declared {
    overlay: bool,
    exempt: Vec<(LitStr, LitStr)>,
    /// The function that makes an instance of a leaf for the suite: `sample = path`.
    sample: Option<syn::Path>,
    /// The first sentence of the type's doc comment; empty for `register_contract!`.
    purpose: String,
}

#[derive(Default)]
struct Options {
    shape: Option<Shape>,
    widget: bool,
    canvas_control: bool,
    content_painter: bool,
    /// Implements `Meta` and registers nothing (see `#[component(unregistered)]` in the derive's doc).
    unregistered: bool,
    declared: Declared,
    probe: Vec<Type>,
}

impl Declared {
    /// Reads one option shared by the derive and `register_contract!`; `false` when `meta` is not one of them.
    fn read(&mut self, meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<bool> {
        let path = &meta.path;
        if path.is_ident("overlay") {
            self.overlay = true;
        } else if path.is_ident("exempt") {
            let (mut case, mut reason) = (None, None);
            meta.parse_nested_meta(|inner| {
                let slot = if inner.path.is_ident("case") {
                    &mut case
                } else if inner.path.is_ident("reason") {
                    &mut reason
                } else {
                    return Err(inner.error("`exempt` takes `case = \"..\"` and `reason = \"..\"`"));
                };
                *slot = Some(inner.value()?.parse::<LitStr>()?);
                Ok(())
            })?;
            match (case, reason) {
                (Some(case), Some(reason)) => self.exempt.push((case, reason)),
                _ => return Err(meta.error("`exempt` needs both `case = \"..\"` and `reason = \"..\"`")),
            }
        } else if path.is_ident("sample") {
            if self.sample.is_some() {
                return Err(meta.error("`sample` is declared once"));
            }
            self.sample = Some(meta.value()?.parse::<syn::Path>()?);
        } else {
            return Ok(false);
        }
        Ok(true)
    }
}

fn parse_options(attrs: &[Attribute]) -> syn::Result<Options> {
    let mut options = Options::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("component")) {
        if matches!(attr.meta, Meta::Path(_)) {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            let shape = if meta.path.is_ident("container") {
                Some(Shape::Container)
            } else if meta.path.is_ident("collection") {
                Some(Shape::Collection)
            } else {
                None
            };
            if let Some(shape) = shape {
                if options.shape.is_some_and(|earlier| earlier != shape) {
                    return Err(meta.error("a component is a container or a collection, not both"));
                }
                options.shape = Some(shape);
            } else if meta.path.is_ident("widget") {
                options.widget = true;
            } else if meta.path.is_ident("canvas_control") {
                options.canvas_control = true;
            } else if meta.path.is_ident("content_painter") {
                options.content_painter = true;
            } else if meta.path.is_ident("unregistered") {
                options.unregistered = true;
            } else if meta.path.is_ident("probe") {
                let content;
                syn::parenthesized!(content in meta.input);
                let types = Punctuated::<Type, Token![,]>::parse_terminated(&content)?;
                if types.is_empty() {
                    return Err(meta.error("`probe` lists the instantiations to register"));
                }
                options.probe.extend(types);
            } else if !options.declared.read(&meta)? {
                return Err(meta.error(
                    "unknown `component` option; expected `container`, `collection`, `widget`, `overlay`, `content_painter`, `canvas_control`, `unregistered`, `exempt(..)`, `sample = ..` or `probe(..)`",
                ));
            }
            Ok(())
        })?;
    }
    Ok(options)
}

/// A lifetime other than `'static` in a probe type.
struct StaticOnly(Option<syn::Error>);

impl Visit<'_> for StaticOnly {
    fn visit_lifetime(&mut self, lifetime: &Lifetime) {
        if lifetime.ident != "static" && self.0.is_none() {
            self.0 = Some(syn::Error::new(lifetime.span(), "a probe instantiates lifetimes as `'static`"));
        }
    }
}

/// The types to register: the type itself, or the instantiations `probe(..)` lists.
pub(crate) fn instantiations(ident: &Ident, generics: &Generics, probe: Vec<Type>, attribute: &str) -> syn::Result<Vec<Type>> {
    let parameterised = generics.params.iter().any(|param| !matches!(param, GenericParam::Lifetime(_)));
    if !parameterised {
        if let Some(first) = probe.first() {
            return Err(syn::Error::new_spanned(first, "`probe` is for a type with type parameters; this type registers itself"));
        }
        let lifetimes = generics.lifetimes().map(|_| quote!('static));
        let ty = if generics.params.is_empty() { quote!(#ident) } else { quote!(#ident<#(#lifetimes),*>) };
        return Ok(vec![syn::parse2(ty)?]);
    }
    if probe.is_empty() {
        return Err(syn::Error::new_spanned(
            ident,
            format!("`{ident}` has type parameters: list the instantiations to register with `#[{attribute}(probe({ident}<..>, ..))]`"),
        ));
    }
    for ty in &probe {
        let named =
            matches!(ty, Type::Path(path) if path.qself.is_none() && path.path.segments.last().is_some_and(|segment| segment.ident == *ident));
        if !named {
            return Err(syn::Error::new_spanned(ty, format!("`probe` lists instantiations of `{ident}`")));
        }
        let mut check = StaticOnly(None);
        check.visit_type(ty);
        if let Some(error) = check.0 {
            return Err(error);
        }
    }
    Ok(probe)
}

/// The first sentence of the first paragraph of the doc comment on `attrs`, without its final full stop.
pub(crate) fn purpose_of(attrs: &[Attribute]) -> String {
    let lines = attrs.iter().filter_map(|attr| match &attr.meta {
        Meta::NameValue(pair) if pair.path.is_ident("doc") => match &pair.value {
            syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(text), .. }) => Some(text.value()),
            _ => None,
        },
        _ => None,
    });
    let paragraph: Vec<String> = lines.map(|line| line.trim().to_owned()).take_while(|line| !line.is_empty()).collect();
    let text = paragraph.join(" ");
    let sentence = text.split_once(". ").map_or(text.as_str(), |(first, _)| first);
    sentence.trim_end_matches('.').to_owned()
}

/// The attributes on a registration block, from the crate being compiled. See `gate_for`.
fn registration_gate() -> TokenStream {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").and_then(|dir| std::fs::read_to_string(std::path::Path::new(&dir).join("Cargo.toml")).ok());
    gate_for(manifest.as_deref().unwrap_or_default())
}

/// The attributes on a registration block. The registration's types can exist only under the defining package's
/// `screen-testing` feature, which may be off while craft-ui's `testing` is on, so the block is gated on it (the cfg
/// is evaluated in the defining crate) when the package's `manifest` declares that feature, and is not gated otherwise.
pub(crate) fn gate_for(manifest: &str) -> TokenStream {
    let mut in_features = false;
    let declares = manifest.lines().map(str::trim).any(|line| {
        if line.starts_with('[') {
            in_features = line.trim_end_matches(|c: char| c.is_whitespace()) == "[features]";
            return false;
        }
        in_features && line.strip_prefix("screen-testing").is_some_and(|rest| rest.trim_start().starts_with('='))
    });
    if declares { quote!(#[cfg(feature = "screen-testing")]) } else { quote!() }
}

fn registrations(types: &[Type], declared: &Declared, shape: Shape) -> syn::Result<TokenStream> {
    if let Some(sample) = &declared.sample {
        if shape != Shape::Leaf {
            return Err(syn::Error::new_spanned(sample, "`sample` is for a leaf: a container or collection needs a body or a source"));
        }
        if types.len() != 1 {
            return Err(syn::Error::new_spanned(sample, "`sample` makes one type: a type registered through `probe(..)` has no single sample"));
        }
    }
    let sample: Vec<_> = declared.sample.iter().collect();
    let overlay = declared.overlay;
    let exempt: Vec<_> = declared.exempt.iter().map(|(case, reason)| quote!((#case, #reason))).collect();
    let shape = match shape {
        Shape::Leaf => quote!(leaf),
        Shape::Container => quote!(container),
        Shape::Collection => quote!(collection),
    };
    let purpose = &declared.purpose;
    let each = types.iter().map(|ty| quote!(::craft_ui::__register_contract!(#ty, #shape, #overlay, [#(#exempt),*], [#(#sample)*], #purpose);));
    let gate = registration_gate();
    Ok(quote! {
        #gate
        const _: () = {
            #(#each)*
        };
    })
}

/// The registration of each pattern type in `types`, with the `purpose` line of its doc comment.
///
/// `probe` is the path of the `PatternProbe` constructor of the pattern's kind.
pub(crate) fn pattern_registrations(types: &[Type], purpose: &str, probe: &TokenStream) -> TokenStream {
    let each = types.iter().map(|ty| quote!(::craft_ui::__register_pattern!(#ty, #purpose, #probe);));
    let gate = registration_gate();
    quote! {
        #gate
        const _: () = {
            #(#each)*
        };
    }
}

/// The source field's doc comment, or a generated one.
fn field_docs(name: &Ident, attrs: &[Attribute]) -> Vec<TokenStream> {
    let docs: Vec<_> = attrs.iter().filter(|attr| attr.path().is_ident("doc")).map(|attr| quote!(#attr)).collect();
    if docs.is_empty() {
        let text = format!("The `{name}` input of the style key.");
        return vec![quote!(#[doc = #text])];
    }
    docs
}

/// The generated key type: generic over the component's type and const parameters.
struct KeyType {
    ident: Ident,
    declaration: TokenStream,
    arguments: TokenStream,
    marker: Option<TokenStream>,
    fields: Vec<(Ident, Type, Vec<Attribute>)>,
}

impl KeyType {
    fn new(ident: &Ident, generics: &Generics, fields: Vec<(Ident, Type, Vec<Attribute>)>) -> Self {
        let mut declared = Vec::new();
        let mut arguments = Vec::new();
        let mut types = Vec::new();
        for param in &generics.params {
            match param {
                GenericParam::Type(param) => {
                    let name = &param.ident;
                    declared.push(quote!(#name: 'static));
                    arguments.push(quote!(#name));
                    types.push(quote!(#name));
                }
                GenericParam::Const(param) => {
                    let (name, ty) = (&param.ident, &param.ty);
                    declared.push(quote!(const #name: #ty));
                    arguments.push(quote!(#name));
                }
                GenericParam::Lifetime(_) => {}
            }
        }
        let wrap = |tokens: Vec<TokenStream>| if tokens.is_empty() { quote!() } else { quote!(<#(#tokens),*>) };
        let marker = (!types.is_empty()).then(|| quote!(::core::marker::PhantomData<fn() -> (#(#types,)*)>));
        KeyType { ident: format_ident!("{ident}StyleKey"), declaration: wrap(declared), arguments: wrap(arguments), marker, fields }
    }

    fn tokens(&self, vis: &syn::Visibility) -> TokenStream {
        let (name, declaration, arguments) = (&self.ident, &self.declaration, &self.arguments);
        let names: Vec<_> = self.fields.iter().map(|(field, _, _)| field).collect();
        let types: Vec<_> = self.fields.iter().map(|(_, ty, _)| ty).collect();
        let docs: Vec<Vec<TokenStream>> = self.fields.iter().map(|(name, _, docs)| field_docs(name, docs)).collect();
        let definition = if self.fields.is_empty() && self.marker.is_none() {
            quote! {
                #[doc = "The style key of the component."]
                #vis struct #name #declaration;
            }
        } else {
            let marker = self.marker.as_ref().map(|marker| quote!(__marker: #marker,));
            quote! {
                #[doc = "The style key of the component."]
                #[non_exhaustive]
                #vis struct #name #declaration { #(#(#docs)* pub #names: #types,)* #marker }
            }
        };
        let debug = {
            let text = name.to_string();
            quote!(f.debug_struct(#text) #(.field(stringify!(#names), &self.#names))* .finish())
        };
        quote! {
            #definition
            impl #declaration ::core::clone::Clone for #name #arguments {
                fn clone(&self) -> Self {
                    *self
                }
            }
            impl #declaration ::core::marker::Copy for #name #arguments {}
            impl #declaration ::core::cmp::PartialEq for #name #arguments {
                fn eq(&self, other: &Self) -> bool {
                    true #(&& self.#names == other.#names)*
                }
            }
            impl #declaration ::core::cmp::Eq for #name #arguments {}
            impl #declaration ::core::hash::Hash for #name #arguments {
                fn hash<H: ::core::hash::Hasher>(&self, state: &mut H) {
                    #(::core::hash::Hash::hash(&self.#names, state);)*
                    let _ = state;
                }
            }
            impl #declaration ::core::fmt::Debug for #name #arguments {
                fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                    #debug
                }
            }
            impl #declaration ::craft_ui::style::StyleKey for #name #arguments {}
        }
    }

    /// The expression that builds the key of `self`.
    fn value(&self) -> TokenStream {
        let name = &self.ident;
        let names: Vec<_> = self.fields.iter().map(|(field, _, _)| field).collect();
        if self.fields.is_empty() && self.marker.is_none() {
            return quote!(#name);
        }
        let marker = self.marker.as_ref().map(|_| quote!(__marker: ::core::marker::PhantomData,));
        quote!(#name { #(#names: self.#names,)* #marker })
    }
}

/// Whether `bounds` hold `'static`, and whether they hold `?Sized`.
fn bound_facts<'a>(bounds: impl Iterator<Item = &'a syn::TypeParamBound>) -> (bool, bool) {
    let (mut is_static, mut maybe_sized) = (false, false);
    for bound in bounds {
        match bound {
            syn::TypeParamBound::Lifetime(lifetime) if lifetime.ident == "static" => is_static = true,
            syn::TypeParamBound::Trait(bound) if matches!(bound.modifier, syn::TraitBoundModifier::Maybe(_)) => maybe_sized = true,
            _ => {}
        }
    }
    (is_static, maybe_sized)
}

/// 4.5: the style key names every type parameter, so each is `'static` and `Sized`.
fn check_params(generics: &Generics) -> syn::Result<()> {
    for param in generics.type_params() {
        let ident = &param.ident;
        let inline = bound_facts(param.bounds.iter());
        let mut clauses = generics.where_clause.iter().flat_map(|clause| clause.predicates.iter()).filter_map(|predicate| match predicate {
            syn::WherePredicate::Type(bounded) if matches!(&bounded.bounded_ty, Type::Path(path) if path.path.is_ident(ident)) => {
                Some(&bounded.bounds)
            }
            _ => None,
        });
        let (is_static, maybe_sized) = clauses
            .try_fold(inline, |(is_static, maybe), bounds| {
                let (more_static, more_maybe) = bound_facts(bounds.iter());
                Some((is_static || more_static, maybe || more_maybe))
            })
            .unwrap_or(inline);
        if maybe_sized {
            return Err(syn::Error::new_spanned(param, format!("4.5: the style key names `{ident}`, so it must be `Sized`")));
        }
        if !is_static {
            return Err(syn::Error::new_spanned(param, format!("4.5: the style key names `{ident}`, so it needs `{ident}: 'static`")));
        }
    }
    Ok(())
}

/// An associated type of a type parameter in a `#[style_key]` field: the key does not carry the bounds that name it.
struct AssociatedPath<'a> {
    params: Vec<&'a Ident>,
    found: Option<syn::Error>,
}

impl Visit<'_> for AssociatedPath<'_> {
    fn visit_type_path(&mut self, path: &syn::TypePath) {
        let associated =
            path.qself.is_some() || (path.path.segments.len() > 1 && self.params.iter().any(|param| path.path.segments[0].ident == **param));
        if associated && self.found.is_none() {
            self.found =
                Some(syn::Error::new_spanned(path, "a `#[style_key]` field names a concrete type, not an associated type of a type parameter"));
        }
        syn::visit::visit_type_path(self, path);
    }
}

fn check_key_fields(generics: &Generics, fields: &[&Field]) -> syn::Result<()> {
    for field in fields {
        let mut check = AssociatedPath { params: generics.type_params().map(|param| &param.ident).collect(), found: None };
        check.visit_type(&field.ty);
        if let Some(error) = check.found {
            return Err(error);
        }
    }
    Ok(())
}

struct Marked<'a> {
    common: &'a Field,
    container: Option<&'a Field>,
    style_key: Vec<&'a Field>,
}

fn marked<'a>(ident: &Ident, fields: &'a [Field], shape: Shape) -> syn::Result<Marked<'a>> {
    let (mut common, mut container, mut style_key) = (None, None, Vec::new());
    for field in fields {
        for attr in &field.attrs {
            let slot = if attr.path().is_ident("common") {
                &mut common
            } else if attr.path().is_ident("container") {
                &mut container
            } else if attr.path().is_ident("style_key") {
                attr.meta.require_path_only()?;
                style_key.push(field);
                continue;
            } else {
                continue;
            };
            if !attr.path().is_ident("container") {
                attr.meta.require_path_only()?;
            }
            if slot.replace(field).is_some() {
                return Err(syn::Error::new_spanned(attr, "at most one field carries this attribute"));
            }
        }
    }
    let common = common.ok_or_else(|| syn::Error::new_spanned(ident, "a component has one field marked `#[common]`, of type `Common`"))?;
    match (shape, container) {
        (Shape::Leaf, Some(field)) => {
            Err(syn::Error::new_spanned(field, "`#[container]` is for `#[component(container)]` and `#[component(collection)]`"))
        }
        (Shape::Container | Shape::Collection, None) => {
            Err(syn::Error::new_spanned(ident, "a container or collection has one field marked `#[container]`, of type `ContainerProps`"))
        }
        _ => Ok(Marked { common, container, style_key }),
    }
}

pub(crate) fn derive(input: TokenStream) -> syn::Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    let mut options = parse_options(&input.attrs)?;
    let shape = options.shape.unwrap_or(Shape::Leaf);
    if options.canvas_control && shape != Shape::Leaf {
        return Err(syn::Error::new_spanned(&input.ident, "`canvas_control` is for a leaf"));
    }
    if options.widget && shape != Shape::Leaf {
        return Err(syn::Error::new_spanned(&input.ident, "`widget` is for a leaf with `Output = Response`"));
    }
    check_params(&input.generics)?;
    let fields = named_fields(&input, "Component")?;
    let marked = marked(&input.ident, &fields, shape)?;
    let ident = &input.ident;
    let key_fields =
        marked.style_key.iter().filter_map(|field| field.ident.clone().map(|name| (name, field.ty.clone(), field.attrs.clone()))).collect();
    check_key_fields(&input.generics, &marked.style_key)?;
    let key = KeyType::new(ident, &input.generics, key_fields);
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let key_tokens = key.tokens(&input.vis);
    let (key_name, key_arguments, key_value) = (&key.ident, &key.arguments, key.value());
    let shape_type = match shape {
        Shape::Leaf => quote!(::craft_ui::contract::shape::Leaf),
        Shape::Container => quote!(::craft_ui::contract::shape::Body),
        Shape::Collection => quote!(::craft_ui::contract::shape::Items),
    };
    let canvas = options.canvas_control.then(|| {
        quote!(
            const CANVAS_CONTROL: bool = true;
        )
    });
    let (common, common_mut) = match &marked.common.ident {
        Some(field) => (quote!(&self.#field), quote!(&mut self.#field)),
        None => return Err(syn::Error::new_spanned(ident, "`#[common]` needs a named field")),
    };
    let has_props = marked.container.and_then(|field| field.ident.as_ref()).map(|field| {
        quote! {
            impl #impl_generics ::craft_ui::contract::HasProps for #ident #type_generics #where_clause {
                fn props(&self, _: ::craft_ui::contract::Internal) -> &::craft_ui::contract::ContainerProps {
                    &self.#field
                }
            }
        }
    });
    let content_painter =
        options.content_painter.then(|| quote!(impl #impl_generics ::craft_ui::contract::ContentPainter for #ident #type_generics #where_clause {}));
    let widget = options.widget.then(|| {
        let mut generics = input.generics.clone();
        generics
            .make_where_clause()
            .predicates
            .push(syn::parse_quote!(#ident #type_generics: ::craft_ui::contract::Component<Output = ::craft_ui::__private::egui::Response>));
        let (widget_generics, _, widget_where) = generics.split_for_impl();
        quote! {
            impl #widget_generics ::craft_ui::__private::egui::Widget for #ident #type_generics #widget_where {
                fn ui(self, ui: &mut ::craft_ui::__private::egui::Ui) -> ::craft_ui::__private::egui::Response {
                    ::craft_ui::contract::Show::show(self, ui)
                }
            }
        }
    });
    let registration = if options.unregistered {
        let declared = &options.declared;
        if declared.overlay || !declared.exempt.is_empty() || declared.sample.is_some() || !options.probe.is_empty() {
            return Err(syn::Error::new_spanned(
                ident,
                "`unregistered` registers nothing: `overlay`, `exempt`, `sample` and `probe` have no effect with it",
            ));
        }
        quote!()
    } else {
        let types = instantiations(ident, &input.generics, options.probe, "component")?;
        options.declared.purpose = purpose_of(&input.attrs);
        registrations(&types, &options.declared, shape)?
    };
    Ok(quote! {
        #key_tokens
        impl #impl_generics ::craft_ui::contract::Meta for #ident #type_generics #where_clause {
            type Shape = #shape_type;
            type StyleKey = #key_name #key_arguments;
            #canvas

            fn style_key(&self) -> Self::StyleKey {
                #key_value
            }

            fn common(&self, _: ::craft_ui::contract::Internal) -> &::craft_ui::contract::Common {
                #common
            }

            fn common_mut(&mut self, _: ::craft_ui::contract::Internal) -> &mut ::craft_ui::contract::Common {
                #common_mut
            }
        }
        #has_props
        #content_painter
        #widget
        #registration
    })
}

/// `register_contract!(Type, ..; options)`: the registration of an `impl Component` written without the derive.
struct Registered {
    types: Vec<Type>,
    declared: Declared,
    shape: Shape,
    /// The types are patterns: they register as `Pattern`s and no component probe applies.
    pattern: bool,
}

impl Parse for Registered {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut types = Vec::new();
        while !input.is_empty() && !input.peek(Token![;]) {
            types.push(input.parse::<Type>()?);
            if !input.peek(Token![,]) {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        if types.is_empty() {
            return Err(input.error("`register_contract!` takes the component types to register"));
        }
        let mut declared = Declared::default();
        let mut shape = Shape::Leaf;
        let mut pattern = false;
        if input.parse::<Option<Token![;]>>()?.is_some() {
            syn::meta::parser(|meta| {
                if meta.path.is_ident("container") {
                    shape = Shape::Container;
                } else if meta.path.is_ident("collection") {
                    shape = Shape::Collection;
                } else if meta.path.is_ident("pattern") {
                    pattern = true;
                } else if !declared.read(&meta)? {
                    return Err(meta.error("expected `container`, `collection`, `pattern`, `overlay`, `exempt(..)` or `sample = ..`"));
                }
                Ok(())
            })
            .parse2(input.parse::<TokenStream>()?)?;
        }
        if pattern && (shape != Shape::Leaf || declared.overlay || !declared.exempt.is_empty() || declared.sample.is_some()) {
            return Err(input.error("`pattern` takes no shape, `overlay`, `sample` or `exempt(..)`: a pattern has no pipeline"));
        }
        Ok(Registered { types, declared, shape, pattern })
    }
}

pub(crate) fn register(input: TokenStream) -> syn::Result<TokenStream> {
    let registered: Registered = syn::parse2(input)?;
    for ty in &registered.types {
        let mut check = StaticOnly(None);
        check.visit_type(ty);
        if let Some(error) = check.0 {
            return Err(error);
        }
    }
    if registered.pattern {
        return Ok(pattern_registrations(&registered.types, "", &quote!(::craft_ui::contract::PatternProbe::of)));
    }
    registrations(&registered.types, &registered.declared, registered.shape)
}
