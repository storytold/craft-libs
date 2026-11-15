//! Parsing helpers the derives share.

use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Field, Fields, GenericArgument, PathArguments, Type};

/// The named fields of a struct, or an error naming `derive`.
pub(crate) fn named_fields(input: &DeriveInput, derive: &str) -> syn::Result<Vec<Field>> {
    match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => Ok(fields.named.iter().cloned().collect()),
            _ => Err(syn::Error::new_spanned(&input.ident, format!("`#[derive({derive})]` needs a struct with named fields"))),
        },
        _ => Err(syn::Error::new_spanned(&input.ident, format!("`#[derive({derive})]` needs a struct"))),
    }
}

/// The named fields of a struct, none for a unit struct, or an error naming `derive`.
pub(crate) fn fields_or_unit(input: &DeriveInput, derive: &str) -> syn::Result<Vec<Field>> {
    match &input.data {
        Data::Struct(data) if matches!(data.fields, Fields::Unit) => Ok(Vec::new()),
        _ => named_fields(input, derive),
    }
}

/// `T` of `Wrapper<T>` when `ty` is a path whose last segment is `wrapper` with one type argument.
pub(crate) fn wrapped<'a>(ty: &'a Type, wrapper: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else { return None };
    let segment = path.path.segments.last()?;
    if segment.ident != wrapper {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else { return None };
    match arguments.args.first()? {
        GenericArgument::Type(inner) if arguments.args.len() == 1 => Some(inner),
        _ => None,
    }
}

/// Whether `ty` is a path whose last segment is `name`.
pub(crate) fn is_named(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(path) if path.qself.is_none() && path.path.segments.last().is_some_and(|segment| segment.ident == name))
}

/// The path of the framework crate (`craft-ui-contract`, lib name `craft_ui`) as the crate being compiled reaches it.
///
/// Apps depend on the `craft` facade only, which re-exports it as `craft::ui`, so a hardcoded `::craft_ui` fails
/// there. A direct dependency on `craft-ui-contract` (or being it) gives `::craft_ui`; otherwise the facade gives
/// `::craft::ui`. With neither, `::craft_ui` is used so the error names the missing crate.
pub(crate) fn ui_crate() -> TokenStream {
    if crate_name("craft-ui-contract").is_ok() {
        return quote!(::craft_ui);
    }
    match crate_name("craft") {
        Ok(FoundCrate::Itself | FoundCrate::Name(_)) => quote!(::craft::ui),
        Err(_) => quote!(::craft_ui),
    }
}
