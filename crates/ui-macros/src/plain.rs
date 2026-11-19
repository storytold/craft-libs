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
}

/// Requires `trait_path` of every field of `input` (structs, tuple structs, enums; a union is an error). The impl
/// bounds each type parameter by `trait_path`; every field is checked under those bounds by a generated
/// `check::<Field>()` spanned at the field's type, so a rejected field (`Cell<T>` included) is an error at the derive
/// that names the field, and a recursive type (a node holding `Vec<Self>` or `Vec<Node<T>>`) does not make its impl
/// require itself. A field type written with `Self` is checked as the type itself. A field carrying `escape` is not
/// checked.
pub(crate) fn field_checks(input: &DeriveInput, trait_path: &TokenStream, escape: Option<&Escape>) -> syn::Result<FieldChecks> {
    let fields: Vec<&syn::Field> = match &input.data {
        Data::Struct(data) => data.fields.iter().collect(),
        Data::Enum(data) => data.variants.iter().flat_map(|variant| variant.fields.iter()).collect(),
        Data::Union(data) => return Err(syn::Error::new(data.union_token.span(), "a union cannot hold only plain data")),
    };
    let mut generics = input.generics.clone();
    let params: Vec<syn::Ident> = input.generics.type_params().map(|param| param.ident.clone()).collect();
    let where_clause = generics.make_where_clause();
    for param in &params {
        where_clause.predicates.push(syn::parse2(quote!(#param: #trait_path))?);
    }
    let ident = &input.ident;
    let (_, type_generics, _) = input.generics.split_for_impl();
    let this: syn::Type = syn::parse2(quote!(#ident #type_generics))?;
    let mut checked = Vec::new();
    for field in fields {
        if let Some(escape) = escape
            && escaped(field, escape)?
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

/// Whether `field` carries `#[<attr>(<keyword> = "reason")]`. Any other content of the attribute, or an empty
/// reason, is an error.
pub(crate) fn escaped(field: &syn::Field, escape: &Escape) -> syn::Result<bool> {
    let Escape { attr: name, keyword } = *escape;
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
    let FieldChecks { generics, checks } = field_checks(&input, &trait_path, Some(&Escape { attr: "plain_data", keyword: "assume" }))?;
    let ident = &input.ident;
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let (_, type_generics, _) = input.generics.split_for_impl();
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #trait_path for #ident #type_generics #where_clause {}

        #checks
    })
}
