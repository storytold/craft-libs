//! Parsing helpers the derives share.

use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
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

/// The path of the framework crate (`craft-ui-contract`) as the crate being compiled reaches it.
///
/// Apps depend on the `craft` facade, which re-exports it as `craft::ui`; the facade is looked up first because no
/// framework crate depends on it, while an app may also list `craft-ui-contract` (for example as a dev-dependency,
/// which proc-macro-crate does not tell apart). Framework crates reach it as `::craft_ui`: each declares that name
/// (`extern crate self as craft_ui` in the crates that re-export it, an `extern crate` alias in the others). A
/// renamed dependency is reached by its own name. With neither crate, `::craft_ui` is used so the error names it.
pub(crate) fn ui_crate() -> TokenStream {
    match crate_name("craft") {
        Ok(FoundCrate::Name(name)) => {
            let name = format_ident!("{name}");
            return quote!(::#name::ui);
        }
        Ok(FoundCrate::Itself) => return quote!(::craft::ui),
        Err(_) => {}
    }
    match crate_name("craft-ui-contract") {
        Ok(FoundCrate::Name(name)) if name != "craft_ui_contract" => {
            let name = format_ident!("{name}");
            quote!(::#name)
        }
        _ => quote!(::craft_ui),
    }
}
