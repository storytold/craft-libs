//! `#[derive(EnvValue)]` with `#[env(style)]` and `#[env(root = ..)]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Expr};

pub(crate) fn derive(input: TokenStream) -> syn::Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    let (mut style, mut root) = (false, None::<Expr>);
    for attr in input.attrs.iter().filter(|attr| attr.path().is_ident("env")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("style") {
                style = true;
            } else if meta.path.is_ident("root") {
                root = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error("`env` takes `style` and `root = <expression>`"));
            }
            Ok(())
        })?;
    }
    let ident = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let root = match root {
        Some(expr) => quote!(#expr),
        None => quote!(<Self as ::core::default::Default>::default()),
    };
    let affects = style.then(|| {
        quote! {
            const AFFECTS_STYLE: bool = true;

            fn style_hash(&self) -> ::core::option::Option<u64> {
                use ::core::hash::{Hash, Hasher};
                let mut hasher = ::std::collections::hash_map::DefaultHasher::new();
                Hash::hash(self, &mut hasher);
                ::core::option::Option::Some(hasher.finish())
            }
        }
    });
    let marker = style.then(|| quote!(impl #impl_generics ::craft_ui::runtime::env::StyleValue for #ident #type_generics #where_clause {}));
    Ok(quote! {
        impl #impl_generics ::craft_ui::runtime::env::EnvValue for #ident #type_generics #where_clause {
            #affects

            fn root() -> Self {
                #root
            }
        }
        #marker
    })
}
