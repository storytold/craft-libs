//! `#[derive(Pattern)]`: the marker impl and the registration.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, DeriveInput, Type};

/// What a `#[derive(Pattern)]` declares: the instantiations a `#[pattern(probe(Type<..>, ..))]` lists, and the path
/// of the `PatternProbe` constructor of its kind (`#[pattern(tool)]`, `#[pattern(dialog)]`; `PatternProbe::of`
/// without either). A kind's constructor lives beside the kind's trait and is bounded by it.
struct Options {
    types: Vec<Type>,
    probe: TokenStream,
}

fn options(attrs: &[Attribute]) -> syn::Result<Options> {
    let mut types = Vec::new();
    let mut probe = quote!(::craft_ui::contract::PatternProbe::of);
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("pattern")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("tool") {
                probe = quote!(::craft_ui::canvas::tool_probe);
                return Ok(());
            }
            if meta.path.is_ident("dialog") {
                probe = quote!(::craft_ui::dialog::dialog_probe);
                return Ok(());
            }
            if !meta.path.is_ident("probe") {
                return Err(meta.error("unknown `pattern` option; expected `probe(..)`, `tool` or `dialog`"));
            }
            let content;
            syn::parenthesized!(content in meta.input);
            let listed = syn::punctuated::Punctuated::<Type, syn::Token![,]>::parse_terminated(&content)?;
            if listed.is_empty() {
                return Err(meta.error("`probe` lists the instantiations to register"));
            }
            types.extend(listed);
            Ok(())
        })?;
    }
    Ok(Options { types, probe })
}

pub(crate) fn derive(input: TokenStream) -> syn::Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    if !matches!(input.data, syn::Data::Struct(_)) {
        return Err(syn::Error::new_spanned(&input.ident, "`#[derive(Pattern)]` needs a struct"));
    }
    let ident = &input.ident;
    let Options { types, probe } = options(&input.attrs)?;
    let types = crate::component::instantiations(ident, &input.generics, types, "pattern")?;
    let purpose = crate::component::purpose_of(&input.attrs);
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let registration = crate::component::pattern_registrations(&types, &purpose, &probe);
    Ok(quote! {
        impl #impl_generics ::craft_ui::contract::Pattern for #ident #type_generics #where_clause {}
        #registration
    })
}
