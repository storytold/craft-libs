//! `#[derive(ComponentStyle)]`: the trait from field attributes.

use crate::util::{named_fields, wrapped};
use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Attribute, DeriveInput, Expr, Field, Ident, Token};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Surface,
    Focus,
    Metrics,
    Text,
    Icon,
}

fn part_of(field: &Field) -> syn::Result<Option<Part>> {
    let mut part = None;
    for attr in field.attrs.iter().filter(|attr| attr.path().is_ident("style")) {
        attr.parse_nested_meta(|meta| {
            let found = if meta.path.is_ident("surface") {
                Part::Surface
            } else if meta.path.is_ident("focus") {
                Part::Focus
            } else if meta.path.is_ident("metrics") {
                Part::Metrics
            } else if meta.path.is_ident("text") {
                Part::Text
            } else if meta.path.is_ident("icon") {
                Part::Icon
            } else {
                return Err(
                    meta.error("a field is `#[style(surface)]`, `#[style(focus)]`, `#[style(metrics)]`, `#[style(text)]` or `#[style(icon)]`")
                );
            };
            if part.replace(found).is_some() {
                return Err(meta.error("a field carries one `#[style(..)]` part"));
            }
            Ok(())
        })?;
    }
    Ok(part)
}

fn variants(attrs: &[Attribute]) -> syn::Result<Option<Vec<Expr>>> {
    let mut found = None;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("style")) {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("variants") {
                return Err(meta.error("the type takes `#[style(variants = [..])]`"));
            }
            let list: syn::ExprArray = meta.value()?.parse()?;
            found = Some(list.elems.into_iter().collect());
            Ok(())
        })?;
    }
    Ok(found)
}

/// The accessor listing every field marked as one look kind, in field order; `None` when no field is.
fn collector(fields: &[&Field], name: &str, ty: TokenStream) -> Option<TokenStream> {
    if fields.is_empty() {
        return None;
    }
    let name = Ident::new(name, proc_macro2::Span::call_site());
    let adds = fields.iter().filter_map(|field| {
        let member = field.ident.as_ref()?;
        Some(if wrapped(&field.ty, "Option").is_some() { quote!(out.extend(self.#member.as_ref());) } else { quote!(out.push(&self.#member);) })
    });
    Some(quote! {
        fn #name(&self) -> ::std::vec::Vec<&#ty> {
            let mut out = ::std::vec::Vec::new();
            #(#adds)*
            out
        }
    })
}

pub(crate) fn derive(input: TokenStream) -> syn::Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    let fields = named_fields(&input, "ComponentStyle")?;
    let (mut surface, mut focus, mut metrics): (Option<&Field>, Option<&Field>, Option<&Field>) = (None, None, None);
    let (mut texts, mut icons): (Vec<&Field>, Vec<&Field>) = (Vec::new(), Vec::new());
    for field in &fields {
        let slot = match part_of(field)? {
            Some(Part::Text) => {
                texts.push(field);
                continue;
            }
            Some(Part::Icon) => {
                icons.push(field);
                continue;
            }
            Some(Part::Surface) => &mut surface,
            Some(Part::Focus) => &mut focus,
            Some(Part::Metrics) => &mut metrics,
            None => continue,
        };
        if slot.replace(field).is_some() {
            return Err(syn::Error::new_spanned(field, "at most one field carries each `#[style(..)]` part"));
        }
    }
    let ident = &input.ident;
    let metrics = metrics.ok_or_else(|| syn::Error::new_spanned(ident, "a style has one field marked `#[style(metrics)]`, of type `Metrics`"))?;
    let accessor = |field: Option<&Field>, name: &str, ty: TokenStream| -> Option<TokenStream> {
        let field = field?;
        let name = Ident::new(name, ident.span());
        let member = field.ident.as_ref()?;
        let body =
            if wrapped(&field.ty, "Option").is_some() { quote!(self.#member.as_ref()) } else { quote!(::core::option::Option::Some(&self.#member)) };
        Some(quote! {
            fn #name(&self) -> ::core::option::Option<&#ty> {
                #body
            }
        })
    };
    let surface = accessor(surface, "surface", quote!(::craft_ui::style::SurfaceLook));
    let focus = accessor(focus, "focus", quote!(::craft_ui::style::FocusLook));
    let texts = collector(&texts, "texts", quote!(::craft_ui::style::TextLook));
    let icons = collector(&icons, "icons", quote!(::craft_ui::style::IconLook));
    let metrics_field = &metrics.ident;
    let variants = variants(&input.attrs)?.map(|list| {
        let list = Punctuated::<Expr, Token![,]>::from_iter(list);
        quote!(const VARIANTS: &'static [::craft_ui::style::Variant] = &[#list];)
    });
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::craft_ui::style::ComponentStyle for #ident #type_generics #where_clause {
            #variants
            #surface
            #focus
            #texts
            #icons

            fn metrics(&self) -> ::craft_ui::style::Metrics {
                self.#metrics_field
            }
        }
    })
}
