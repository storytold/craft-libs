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
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Data, DeriveInput};

/// The impl generics, with a bound for every checked field whose type names a type parameter, and the item that
/// checks every other checked field.
pub(crate) struct FieldChecks {
    pub(crate) generics: syn::Generics,
    pub(crate) checks: TokenStream,
}

/// Requires `trait_path` of every field of `input` (structs, tuple structs, enums; a union is an error). A field whose
/// type names a type parameter gets a bound on the impl; a field of a concrete type is checked by a generated
/// `check::<Field>()` spanned at the field's type, so the error points at the field, and a recursive type (a node
/// holding `Vec<Self>`) does not make its impl require itself. With `assume` set to an attribute name, a field
/// carrying `#[<assume>(assume = "reason")]` is not checked.
pub(crate) fn field_checks(input: &DeriveInput, trait_path: &TokenStream, assume: Option<&str>) -> syn::Result<FieldChecks> {
    let fields: Vec<&syn::Field> = match &input.data {
        Data::Struct(data) => data.fields.iter().collect(),
        Data::Enum(data) => data.variants.iter().flat_map(|variant| variant.fields.iter()).collect(),
        Data::Union(data) => return Err(syn::Error::new(data.union_token.span(), "a union cannot hold only plain data")),
    };
    let mut generics = input.generics.clone();
    let params: Vec<syn::Ident> = input.generics.type_params().map(|param| param.ident.clone()).collect();
    let where_clause = generics.make_where_clause();
    let mut concrete = Vec::new();
    for field in fields {
        if let Some(name) = assume
            && assumed(field, name)?
        {
            continue;
        }
        let ty = &field.ty;
        if mentions(ty, &params) {
            where_clause.predicates.push(syn::parse2(quote_spanned!(ty.span()=> #ty: #trait_path))?);
        } else {
            concrete.push(ty);
        }
    }
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let each = concrete.iter().map(|ty| quote_spanned!(ty.span()=> check::<#ty>();));
    let checks = quote! {
        const _: () = {
            #[allow(dead_code, clippy::extra_unused_type_parameters)]
            fn check<T: ?Sized + #trait_path>() {}
            #[allow(dead_code)]
            fn fields #impl_generics () #where_clause {
                #(#each)*
            }
        };
    };
    Ok(FieldChecks { generics, checks })
}

/// Whether `field` carries `#[<name>(assume = "reason")]`. Any other content of the attribute, or an empty reason, is
/// an error.
fn assumed(field: &syn::Field, name: &str) -> syn::Result<bool> {
    let mut found = false;
    for attr in field.attrs.iter().filter(|attr| attr.path().is_ident(name)) {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("assume") {
                return Err(meta.error(format!("expected `#[{name}(assume = \"why the type is plain\")]`")));
            }
            let reason: syn::LitStr = meta.value()?.parse()?;
            if reason.value().trim().is_empty() {
                return Err(syn::Error::new(reason.span(), "`assume` needs a reason: why this field's type is plain data"));
            }
            found = true;
            Ok(())
        })?;
    }
    Ok(found)
}

/// Whether `ty` names one of the type parameters `params`.
fn mentions(ty: &syn::Type, params: &[syn::Ident]) -> bool {
    struct Finder<'p> {
        params: &'p [syn::Ident],
        found: bool,
    }
    impl<'ast> Visit<'ast> for Finder<'_> {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            if path.leading_colon.is_none() && path.segments.first().is_some_and(|segment| self.params.contains(&segment.ident)) {
                self.found = true;
            }
            syn::visit::visit_path(self, path);
        }
    }
    let mut finder = Finder { params, found: false };
    finder.visit_type(ty);
    finder.found
}

/// The path of craft-tracked as the crate being compiled reaches it: the facade's `tracked` module, a direct
/// dependency (`::craft_tracked` in craft-tracked itself, which declares that name), or the framework's re-export in
/// `contract`.
fn tracked_crate() -> TokenStream {
    match crate_name("craft") {
        Ok(FoundCrate::Name(name)) => {
            let name = format_ident!("{name}");
            return quote!(::#name::tracked);
        }
        Ok(FoundCrate::Itself) => return quote!(::craft::tracked),
        Err(_) => {}
    }
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
    let FieldChecks { generics, checks } = field_checks(&input, &trait_path, Some("plain_data"))?;
    let ident = &input.ident;
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let (_, type_generics, _) = input.generics.split_for_impl();
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #trait_path for #ident #type_generics #where_clause {}

        #checks
    })
}
