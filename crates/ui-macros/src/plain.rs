//! The field check the plain-data derives share, and `#[derive(PlainData)]`.
//!
//! A derive that must hold only plain data (`PlainData`, `MemoProps`) requires its trait of every field. The trait has
//! no implementation for the types that can change behind a shared reference, so a field holding one is a compile
//! error at that field. The rejected types, the one list both traits keep out:
//!
//! - `Cell`, `RefCell`, `OnceCell`, `LazyCell`, `UnsafeCell`
//! - `Mutex`, `RwLock`, `OnceLock`, `LazyLock`, `Condvar` and every atomic
//! - raw pointers (`*const T`, `*mut T`)
//! - `dyn Trait` objects (`Box<dyn Fn()>` and every other) and closures
//! - containers and smart pointers of any of these (`Arc<Mutex<T>>`, `Vec<Cell<u8>>`)
//!
//! A field of a foreign plain type that has no implementation (an egui `Rect`, a `serde_json::Value`) carries
//! `#[plain_data(assume = "why the type is plain")]`, which skips the check for that field only. The reason is
//! required.

use crate::util::ui_crate;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::parse::Parser as _;
use syn::spanned::Spanned;
use syn::visit_mut::VisitMut;
use syn::{Data, DeriveInput};

/// What the plain-data check gives a derive.
pub(crate) struct FieldChecks {
    /// The impl generics: the type's own, with every type parameter bound by the derive's trait.
    pub(crate) generics: syn::Generics,
    /// The item that checks each field not escaped, under those bounds. A derive whose impl body already uses every
    /// field through its trait may leave it out.
    pub(crate) checks: TokenStream,
}

/// The escape a derive allows for one field: `#[<attr>(<keyword> = "reason")]`.
pub(crate) struct Escape {
    /// The attribute's name (`plain_data`).
    pub(crate) attr: &'static str,
    /// The one key it takes (`assume`).
    pub(crate) keyword: &'static str,
    /// What a type parameter that only escaped fields name is bound by instead of the derive's trait (`Hash + Sync`
    /// for a field that feeds a key through its `Hash`); `None` keeps the trait.
    pub(crate) bound: Option<TokenStream>,
}

/// Which fields a derive does not check, and what a type parameter that only those fields name is bound by.
pub(crate) trait EscapeRule {
    /// `None` when `field` is checked; else `Some(bound)`: what a type parameter that only escaped fields name is bound
    /// by instead of the derive's trait (`None` keeps the trait, an empty bound leaves the parameter unbound).
    fn escape(&self, field: &syn::Field) -> syn::Result<Option<Option<TokenStream>>>;
}

impl EscapeRule for Escape {
    fn escape(&self, field: &syn::Field) -> syn::Result<Option<Option<TokenStream>>> {
        Ok(escaped(field, self)?.then(|| self.bound.clone()))
    }
}

/// The escape a derive allows for one type parameter, `#[<attr>(<keyword>)]`: the parameter is bound by `bound`
/// instead of the derive's trait.
pub(crate) struct ParamEscape {
    /// The attribute's name (`memo_props`).
    pub(crate) attr: &'static str,
    /// The one flag it takes (`stateless`).
    pub(crate) keyword: &'static str,
    /// What the parameter is bound by instead (`'static`).
    pub(crate) bound: TokenStream,
}

/// Requires `trait_path` of every field of `input` (structs, tuple structs, enums; a union is an error). The impl
/// bounds each type parameter by `trait_path`; every field is checked under those bounds by a generated
/// `check::<Field>()` spanned at the field's type, so a rejected field (`Cell<T>` included) is an error at the derive
/// that names the field, and a recursive type (a node holding `Vec<Self>` or `Vec<Node<T>>`) does not make its impl
/// require itself. A field type written with `Self` is checked as the type itself. A field carrying `escape` is not
/// checked; a type parameter that only escaped fields name is bound by the escape's bound, when it has one. A type
/// parameter carrying `param_escape` is bound by its `bound` instead of `trait_path`; its fields are still checked under
/// that bound.
pub(crate) fn field_checks(
    input: &DeriveInput,
    trait_path: &TokenStream,
    escape: Option<&dyn EscapeRule>,
    param_escape: Option<&ParamEscape>,
) -> syn::Result<FieldChecks> {
    let fields: Vec<&syn::Field> = match &input.data {
        Data::Struct(data) => data.fields.iter().collect(),
        Data::Enum(data) => data.variants.iter().flat_map(|variant| variant.fields.iter()).collect(),
        Data::Union(data) => return Err(syn::Error::new(data.union_token.span(), "a union cannot hold only plain data")),
    };
    let mut generics = input.generics.clone();
    if let Some(param_escape) = param_escape {
        for param in &generics.params {
            let attrs = match param {
                syn::GenericParam::Type(_) => continue,
                syn::GenericParam::Lifetime(param) => &param.attrs,
                syn::GenericParam::Const(param) => &param.attrs,
            };
            if let Some(attr) = attrs.iter().find(|attr| attr.path().is_ident(param_escape.attr)) {
                let ParamEscape { attr: name, keyword, .. } = param_escape;
                return Err(syn::Error::new_spanned(attr, format!("`#[{name}({keyword})]` marks a type parameter")));
            }
        }
    }
    // The type parameters each kind of field names: a parameter only escaped fields name is bound by the escape's
    // bound, when it has one.
    let (mut named, mut named_escaped) = (Named::default(), Named::default());
    // The bound of the escaped fields that name each parameter, in field order.
    let mut escaped_bounds: Vec<(syn::Ident, TokenStream)> = Vec::new();
    for field in &fields {
        match escape.map(|escape| escape.escape(field)).transpose()?.flatten() {
            Some(bound) => {
                let mut names = Named::default();
                syn::visit::Visit::visit_type(&mut names, &field.ty);
                if let Some(bound) = bound {
                    escaped_bounds.extend(names.0.iter().map(|ident| (ident.clone(), bound.clone())));
                }
                named_escaped.0.extend(names.0);
            }
            None => syn::visit::Visit::visit_type(&mut named, &field.ty),
        }
    }
    // Each type parameter takes the derive's bound beside its own, in one place: a second place for it is a lint in
    // the type's crate (`clippy::multiple_bound_locations`).
    for param in generics.type_params_mut() {
        let escaped = match param_escape {
            Some(param_escape) => param_escaped(param, param_escape)?,
            None => false,
        };
        // A derive's helper attributes are not attributes of the impl.
        param.attrs.clear();
        let ident = &param.ident;
        let only_escaped = named_escaped.0.contains(ident) && !named.0.contains(ident);
        // The first escaped field's bound that binds anything; an empty one (no bound) only when no other is given.
        let bounds = || escaped_bounds.iter().filter(|(named, _)| named == ident).map(|(_, bound)| bound);
        let escape_bound = bounds().find(|bound| !bound.is_empty()).or_else(|| bounds().next());
        let bound = match (param_escape, escape_bound) {
            (Some(ParamEscape { bound, .. }), _) if escaped => quote!(#bound),
            (_, Some(bound)) if only_escaped => quote!(#bound),
            _ => quote!(#trait_path),
        };
        let bounds = syn::punctuated::Punctuated::<syn::TypeParamBound, syn::Token![+]>::parse_terminated.parse2(bound)?;
        param.bounds.extend(bounds);
    }
    let ident = &input.ident;
    let (_, type_generics, _) = input.generics.split_for_impl();
    let this: syn::Type = syn::parse2(quote!(#ident #type_generics))?;
    let mut checked = Vec::new();
    for field in fields {
        if let Some(escape) = escape
            && escape.escape(field)?.is_some()
        {
            continue;
        }
        let mut ty = field.ty.clone();
        ReplaceSelf(&this).visit_type_mut(&mut ty);
        checked.push(quote_spanned!(field.ty.span()=> check::<#ty>();));
    }
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let checks = quote! {
        const _: () = {
            #[allow(dead_code, clippy::extra_unused_type_parameters)]
            fn check<T: ?Sized + #trait_path>() {}
            #[allow(dead_code)]
            fn fields #impl_generics () #where_clause {
                #(#checked)*
            }
        };
    };
    Ok(FieldChecks { generics, checks })
}

/// The single-segment paths a type names, among them the type parameters it uses.
#[derive(Default)]
struct Named(Vec<syn::Ident>);

impl<'ast> syn::visit::Visit<'ast> for Named {
    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        if path.qself.is_none()
            && let Some(ident) = path.path.get_ident()
        {
            self.0.push(ident.clone());
        }
        syn::visit::visit_type_path(self, path);
    }
}

/// Replaces the type `Self` with the deriving type, which the generated free function can't name as `Self`.
struct ReplaceSelf<'t>(&'t syn::Type);

impl VisitMut for ReplaceSelf<'_> {
    fn visit_type_mut(&mut self, ty: &mut syn::Type) {
        if let syn::Type::Path(path) = ty
            && path.qself.is_none()
            && path.path.is_ident("Self")
        {
            *ty = self.0.clone();
            return;
        }
        syn::visit_mut::visit_type_mut(self, ty);
    }
}

/// Whether `param` carries `#[<attr>(<keyword>)]`. Any other content of the attribute is an error.
fn param_escaped(param: &syn::TypeParam, escape: &ParamEscape) -> syn::Result<bool> {
    let mut found = false;
    for attr in param.attrs.iter().filter(|attr| attr.path().is_ident(escape.attr)) {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident(escape.keyword) {
                return Err(meta.error(format!("expected `#[{}({})]`", escape.attr, escape.keyword)));
            }
            found = true;
            Ok(())
        })?;
    }
    Ok(found)
}

/// Whether `field` carries `#[<attr>(<keyword> = "reason")]`. Any other content of the attribute, or an empty
/// reason, is an error.
pub(crate) fn escaped(field: &syn::Field, escape: &Escape) -> syn::Result<bool> {
    let Escape { attr: name, keyword, .. } = escape;
    let mut found = false;
    for attr in field.attrs.iter().filter(|attr| attr.path().is_ident(name)) {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident(keyword) {
                return Err(meta.error(format!("expected `#[{name}({keyword} = \"reason\")]`")));
            }
            let reason: syn::LitStr = meta.value()?.parse()?;
            if reason.value().trim().is_empty() {
                return Err(syn::Error::new(reason.span(), format!("`{keyword}` needs a reason: why this field's type is plain data")));
            }
            found = true;
            Ok(())
        })?;
    }
    Ok(found)
}

/// The path of craft-tracked as the crate being compiled reaches it: a direct dependency (`::craft_tracked` in
/// craft-tracked itself, which declares that name), else the framework's re-export in `contract`. Through the
/// facade that is `craft::ui::contract`, which every `ui` build has without the facade's `tracked` feature.
fn tracked_crate() -> TokenStream {
    match crate_name("craft-tracked") {
        Ok(FoundCrate::Name(name)) => {
            let name = format_ident!("{name}");
            quote!(::#name)
        }
        Ok(FoundCrate::Itself) => quote!(::craft_tracked),
        Err(_) => {
            let k = ui_crate();
            quote!(#k::contract)
        }
    }
}

/// `#[derive(PlainData)]`: the marker, required of every field not marked `#[plain_data(assume = "..")]`.
pub(crate) fn derive(input: TokenStream) -> syn::Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    let k = tracked_crate();
    let trait_path = quote!(#k::PlainData);
    let FieldChecks { generics, checks } =
        field_checks(&input, &trait_path, Some(&Escape { attr: "plain_data", keyword: "assume", bound: None }), None)?;
    let ident = &input.ident;
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let (_, type_generics, _) = input.generics.split_for_impl();
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #trait_path for #ident #type_generics #where_clause {}

        #checks
    })
}
