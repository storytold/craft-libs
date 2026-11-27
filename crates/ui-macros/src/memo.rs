//! `#[derive(MemoProps)]`: every field feeds the key, in order, and every field must implement `MemoProps`. An enum
//! feeds its variant's index before the variant's fields.
//!
//! The bounds and the escapes come from the shared field check ([`plain`](crate::plain)). Its check item is left out:
//! the key body already uses every field through `MemoProps` (or `Hash`, for an escaped field), at the field's binding,
//! so each rejected field gives one error that names it. A field that feeds by its address or that another field stands
//! for is not checked: its key is written here.

use crate::plain::{EscapeRule, FieldChecks, ParamEscape, field_checks};
use crate::util::ui_crate;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, LitStr};

const ATTR: &str = "memo_props";

/// How one field feeds the key.
enum Feed {
    /// Through its `MemoProps`.
    Props,
    /// Through its `Hash`: `hash = "why the type is plain"`.
    Hash,
    /// By where it points: `address = "why"`, on a `&'static` reference to a static item.
    Address,
    /// Not at all: the named field of the same variant stands for it, `covered_by = "<field>", reason = "why"`.
    Covered(LitStr),
}

/// A reason given as `key = "reason"`; empty is an error.
fn reason(key: &str, value: &LitStr) -> syn::Result<()> {
    if value.value().trim().is_empty() {
        return Err(syn::Error::new(value.span(), format!("`{key}` needs a reason")));
    }
    Ok(())
}

/// How `field` feeds the key, from its `#[memo_props(..)]`.
fn feed_of(field: &syn::Field) -> syn::Result<Feed> {
    let mut feed = Feed::Props;
    let (mut covered, mut why): (Option<LitStr>, Option<LitStr>) = (None, None);
    for attr in field.attrs.iter().filter(|attr| attr.path().is_ident(ATTR)) {
        attr.parse_nested_meta(|meta| {
            let key = meta.path.get_ident().map(ToString::to_string).unwrap_or_default();
            let value: LitStr = meta.value()?.parse()?;
            let set = |feed: &mut Feed, to: Feed| {
                if !matches!(feed, Feed::Props) {
                    return Err(meta.error("a field feeds the key one way: `hash`, `address` or `covered_by`"));
                }
                *feed = to;
                Ok(())
            };
            match key.as_str() {
                "hash" => {
                    reason(&key, &value)?;
                    set(&mut feed, Feed::Hash)
                }
                "address" => {
                    reason(&key, &value)?;
                    set(&mut feed, Feed::Address)
                }
                "covered_by" => {
                    covered = Some(value.clone());
                    set(&mut feed, Feed::Covered(value))
                }
                "reason" => {
                    reason(&key, &value)?;
                    why = Some(value);
                    Ok(())
                }
                _ => Err(meta.error(format!(
                    "expected `#[{ATTR}(hash = \"why\")]`, `#[{ATTR}(address = \"why\")]` or `#[{ATTR}(covered_by = \"<field>\", reason = \"why\")]`"
                ))),
            }
        })?;
    }
    match (&covered, &why) {
        (Some(field), None) => return Err(syn::Error::new(field.span(), "`covered_by` needs a `reason`")),
        (None, Some(why)) => return Err(syn::Error::new(why.span(), "`reason` goes with `covered_by`")),
        _ => {}
    }
    if matches!(feed, Feed::Address) && !is_static_ref(&field.ty) {
        return Err(syn::Error::new(field.ty.span(), "`address` keys a `&'static` reference by where it points"));
    }
    Ok(feed)
}

/// Whether `ty` is `&'static T`.
fn is_static_ref(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Reference(reference) if reference.mutability.is_none() && reference.lifetime.as_ref().is_some_and(|l| l.ident == "static"))
}

/// The field escapes of `MemoProps`: `hash` binds a type parameter only such fields name by what `MemoKey::feed_hash`
/// needs (a dock's panel type); a `covered_by` field feeds nothing, so a type parameter only such fields name takes no
/// bound (the app state a shell slot's view holds); `address` fields are not checked and keep the trait's bound.
struct MemoEscapes;

impl EscapeRule for MemoEscapes {
    fn escape(&self, field: &syn::Field) -> syn::Result<Option<Option<TokenStream>>> {
        Ok(match feed_of(field)? {
            Feed::Props => None,
            Feed::Hash => Some(Some(quote!(::core::hash::Hash + ::core::marker::Sync))),
            Feed::Covered(_) => Some(Some(TokenStream::new())),
            Feed::Address => Some(None),
        })
    }
}

/// The escape for a callback's type parameter (`Stateless<E>`): bound by what `Stateless<E>: MemoProps` needs.
fn stateless() -> ParamEscape {
    ParamEscape { attr: ATTR, keyword: "stateless", bound: quote!('static) }
}

/// The reason of a type-level `#[memo_props(every_pass = "why")]`: the type is a handle to state written during the
/// pass, or a context rebuilt each pass, which no key can follow.
fn every_pass(input: &DeriveInput) -> syn::Result<Option<LitStr>> {
    let mut found = None;
    for attr in input.attrs.iter().filter(|attr| attr.path().is_ident(ATTR)) {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("every_pass") {
                return Err(meta.error(format!("expected `#[{ATTR}(every_pass = \"why\")]` on the type")));
            }
            let value: LitStr = meta.value()?.parse()?;
            reason("every_pass", &value)?;
            found = Some(value);
            Ok(())
        })?;
    }
    Ok(found)
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
    // A field is named by its name, or a tuple field by its index.
    let label = |at: usize, field: &syn::Field| field.ident.as_ref().map_or_else(|| at.to_string(), ToString::to_string);
    let feeds: Vec<Feed> = fields.iter().map(feed_of).collect::<syn::Result<_>>()?;
    let mut statements = Vec::new();
    for (at, ((field, name), feed)) in fields.iter().zip(&names).zip(&feeds).enumerate() {
        statements.push(match feed {
            Feed::Props => quote!(#k::contract::MemoProps::key(#name, key);),
            Feed::Hash => quote!(key.feed_hash(#name);),
            Feed::Address => quote!(#k::contract::MemoProps::key(&::core::ptr::from_ref(*#name).cast::<()>().addr(), key);),
            Feed::Covered(by) => {
                let target = by.value();
                let standing = fields.iter().enumerate().position(|(other, field)| label(other, field) == target);
                match standing.map(|other| (other, &feeds[other])) {
                    Some((other, _)) if other == at => return Err(syn::Error::new(by.span(), "a field cannot stand for itself")),
                    Some((_, Feed::Covered(_))) => {
                        return Err(syn::Error::new(
                            by.span(),
                            format!("`{target}` is covered by another field itself: name the field that feeds the key"),
                        ));
                    }
                    Some(_) => quote!(),
                    None => {
                        let what = field.ident.as_ref().map_or_else(|| "this field".to_owned(), |ident| format!("`{ident}`"));
                        return Err(syn::Error::new(by.span(), format!("no field `{target}` stands for {what}")));
                    }
                }
            }
        });
    }
    Ok((pattern, statements))
}

pub(crate) fn derive(input: TokenStream) -> syn::Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    let k = ui_crate();
    let trait_path = quote!(#k::contract::MemoProps);
    let ident = &input.ident;
    let (_, type_generics, _) = input.generics.split_for_impl();
    if every_pass(&input)?.is_some() {
        if let Some(attr) = memo_field_attrs(&input).next() {
            return Err(syn::Error::new_spanned(attr, "a type keyed in every pass feeds no field"));
        }
        let mut generics = input.generics.clone();
        for param in generics.type_params_mut() {
            param.attrs.clear();
        }
        let (impl_generics, _, where_clause) = generics.split_for_impl();
        return Ok(quote! {
            #[automatically_derived]
            impl #impl_generics #trait_path for #ident #type_generics #where_clause {
                fn key(&self, key: &mut #k::contract::MemoKey) {
                    key.changes_every_pass();
                }
            }
        });
    }
    let FieldChecks { generics, .. } = field_checks(&input, &trait_path, Some(&MemoEscapes), Some(&stateless()))?;
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
    let (impl_generics, _, where_clause) = generics.split_for_impl();
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

/// The `#[memo_props(..)]` attributes of the fields of `input`.
fn memo_field_attrs(input: &DeriveInput) -> impl Iterator<Item = &syn::Attribute> {
    let fields: Vec<&syn::Field> = match &input.data {
        Data::Struct(data) => data.fields.iter().collect(),
        Data::Enum(data) => data.variants.iter().flat_map(|variant| variant.fields.iter()).collect(),
        Data::Union(_) => Vec::new(),
    };
    fields.into_iter().flat_map(|field| field.attrs.iter()).filter(|attr| attr.path().is_ident(ATTR))
}
