//! `#[derive(MemoProps)]`: every field feeds the key, in order, and every field must implement `MemoProps`. An enum
//! feeds its variant's index before the variant's fields.
//!
//! The bounds and the escape come from the shared field check ([`plain`](crate::plain)). Its check item is left out:
//! the key body already uses every field through `MemoProps` (or `Hash`, for an escaped field), at the field's binding,
//! so each rejected field gives one error that names it.

use crate::plain::{Escape, FieldChecks, ParamEscape, escaped, field_checks};
use crate::util::ui_crate;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields};

/// The escape for a foreign plain type: the field feeds the key through its `Hash`.
const ESCAPE: Escape = Escape { attr: "memo_props", keyword: "hash" };

/// The escape for a callback's type parameter (`Stateless<E>`): bound by what `Stateless<E>: MemoProps` needs.
fn stateless() -> ParamEscape {
    ParamEscape { attr: "memo_props", keyword: "stateless", bound: quote!('static) }
}

/// The pattern binding every field of `fields` to a generated name (`{ a: __f0, b: __f1 }`, `(__f0, __f1)` or
/// nothing), so no field shadows the `key` parameter, and the statements feeding each.
fn feed(fields: &Fields, k: &TokenStream) -> syn::Result<(TokenStream, Vec<TokenStream>)> {
    // Spanned at the field's name, or a tuple field's type, so an error points at the field.
    let names: Vec<syn::Ident> = fields
        .iter()
        .enumerate()
        .map(|(at, field)| format_ident!("__f{at}", span = field.ident.as_ref().map_or_else(|| field.ty.span(), syn::Ident::span)))
        .collect();
    let pattern = match fields {
        Fields::Named(named) => {
            let fields = named.named.iter().filter_map(|field| field.ident.as_ref());
            quote!({ #(#fields: #names),* })
        }
        Fields::Unnamed(_) => quote!(( #(#names),* )),
        Fields::Unit => quote!(),
    };
    let mut statements = Vec::new();
    for (field, name) in fields.iter().zip(&names) {
        statements.push(if escaped(field, &ESCAPE)? { quote!(key.feed_hash(#name);) } else { quote!(#k::contract::MemoProps::key(#name, key);) });
    }
    Ok((pattern, statements))
}

pub(crate) fn derive(input: TokenStream) -> syn::Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    let k = ui_crate();
    let trait_path = quote!(#k::contract::MemoProps);
    let FieldChecks { generics, .. } = field_checks(&input, &trait_path, Some(&ESCAPE), Some(&stateless()))?;
    let body = match &input.data {
        Data::Struct(data) => {
            let (pattern, statements) = feed(&data.fields, &k)?;
            quote! {
                let Self #pattern = self;
                #(#statements)*
            }
        }
        Data::Enum(data) => {
            let mut arms = Vec::new();
            for (at, variant) in data.variants.iter().enumerate() {
                let name = &variant.ident;
                let (pattern, statements) = feed(&variant.fields, &k)?;
                let at = u32::try_from(at).map_err(|_| syn::Error::new_spanned(name, "too many variants"))?;
                arms.push(quote! {
                    Self::#name #pattern => {
                        key.write_u32(#at);
                        #(#statements)*
                    }
                });
            }
            quote! {
                match self {
                    #(#arms)*
                }
            }
        }
        // `field_checks` rejects a union.
        Data::Union(_) => quote!(),
    };
    let ident = &input.ident;
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let (_, type_generics, _) = input.generics.split_for_impl();
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #trait_path for #ident #type_generics #where_clause {
            #[allow(unused_variables)]
            fn key(&self, key: &mut #k::contract::MemoKey) {
                #body
            }
        }
    })
}
