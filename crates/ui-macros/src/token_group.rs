//! `#[derive(TokenGroup)]` with `#[group(path = "..")]`.

use crate::util::{fields_or_unit, is_named};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, LitStr};

/// Theme colours, then the shape types of craft-theme.
const COLOUR: &str = "Rgba";
const SHAPE: [&str; 3] = ["Radius", "Stroke", "Elevation"];

/// `crate_prefix.name(.name)*`: a crate prefix in snake case, then at least one more segment.
fn valid_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('.').collect();
    segments.len() >= 2
        && segments.iter().all(|segment| {
            segment.chars().next().is_some_and(|first| first.is_ascii_lowercase())
                && segment.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

pub(crate) fn derive(input: TokenStream) -> syn::Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    let mut path: Option<LitStr> = None;
    for attr in input.attrs.iter().filter(|attr| attr.path().is_ident("group")) {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("path") {
                return Err(meta.error("`group` takes `path = \"crate_name.group\"`"));
            }
            path = Some(meta.value()?.parse()?);
            Ok(())
        })?;
    }
    let ident = &input.ident;
    let path = path.ok_or_else(|| syn::Error::new_spanned(ident, "a token group names its path: `#[group(path = \"crate_name.group\")]`"))?;
    if !valid_path(&path.value()) {
        return Err(syn::Error::new(path.span(), "a group path is a crate prefix and a name in snake case, such as `craft_timeline.clips`"));
    }
    let mut overlays = Vec::new();
    for field in fields_or_unit(&input, "TokenGroup")? {
        let Some(name) = &field.ident else { continue };
        let colour = is_named(&field.ty, COLOUR);
        if !colour && !SHAPE.iter().any(|shape| is_named(&field.ty, shape)) {
            return Err(syn::Error::new_spanned(&field.ty, "a token group holds colours (`Rgba`) and shapes (`Radius`, `Stroke`, `Elevation`)"));
        }
        if colour {
            let key = name.to_string();
            overlays.push(quote! {
                if let ::core::option::Option::Some(color) = values.color(Self::PATH, #key) {
                    self.#name = color;
                }
            });
        }
    }
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let body = if overlays.is_empty() { quote!(let _ = values;) } else { quote!(#(#overlays)*) };
    Ok(quote! {
        impl #impl_generics ::craft_ui::style::TokenGroup for #ident #type_generics #where_clause {
            const PATH: &'static str = #path;

            // Without an inherent `derive` on the type this call resolves to the trait method itself.
            #[deny(unconditional_recursion)]
            fn derive(tokens: &::craft_ui::style::Tokens) -> Self {
                <Self>::derive(tokens)
            }

            fn overlay(&mut self, values: &::craft_ui::style::GroupValues) {
                #body
            }
        }
    })
}
