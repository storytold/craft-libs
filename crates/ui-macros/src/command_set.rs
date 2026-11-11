//! `#[derive(CommandSet)]` with `#[command(prefix = "..", scope = "..")]` on the enum and
//! `#[command(label = "..", icon = "..", kind = "..", group = "..", modal = "..", edit = "..", default = "..", path = "..", alias = "..")]` on variants.
//! `edit = ".."` links the command to the `EditCommand` a focused component takes it as.
//! `default(mac = "..")` (or `windows`, `linux`) is a default of one platform: it replaces the unqualified defaults there.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Fields, LitStr, Variant};

#[derive(Default)]
struct SetAttrs {
    prefix: Option<LitStr>,
    scope: Option<LitStr>,
}

#[derive(Default)]
struct VariantAttrs {
    label: Option<LitStr>,
    icon: Option<LitStr>,
    kind: Option<LitStr>,
    group: Option<LitStr>,
    modal: Option<LitStr>,
    edit: Option<LitStr>,
    defaults: Vec<LitStr>,
    platform_defaults: Vec<(LitStr, LitStr)>,
    no_default_on: Vec<LitStr>,
    paths: Vec<LitStr>,
    aliases: Vec<LitStr>,
}

fn command_attrs(attrs: &[Attribute], mut read: impl FnMut(&syn::meta::ParseNestedMeta<'_>) -> syn::Result<bool>) -> syn::Result<()> {
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("command")) {
        attr.parse_nested_meta(|meta| if read(&meta)? { Ok(()) } else { Err(meta.error("unknown `command` option")) })?;
    }
    Ok(())
}

fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn kind_tokens(kind: &LitStr, group: Option<&LitStr>) -> syn::Result<TokenStream> {
    let group_tokens =
        group.map_or_else(|| quote!(::core::option::Option::None), |group| quote!(::core::option::Option::Some(::std::string::String::from(#group))));
    let plain = |tokens: TokenStream| match group {
        None => Ok(tokens),
        Some(group) => Err(syn::Error::new(group.span(), "`group` belongs to `kind = \"radio\"` and `kind = \"tool\"`")),
    };
    match kind.value().as_str() {
        "command" => plain(quote!(::craft_ui::runtime::commands::CommandKind::Command)),
        "toggle" => plain(quote!(::craft_ui::runtime::commands::CommandKind::Toggle)),
        "momentary" => plain(quote!(::craft_ui::runtime::commands::CommandKind::Momentary)),
        "radio" => Ok(quote!(::craft_ui::runtime::commands::CommandKind::Radio { group: #group_tokens })),
        "tool" => Ok(quote!(::craft_ui::runtime::commands::CommandKind::Tool { group: #group_tokens })),
        _ => Err(syn::Error::new(kind.span(), "`kind` is `\"command\"`, `\"toggle\"`, `\"radio\"`, `\"tool\"` or `\"momentary\"`")),
    }
}

/// The rules of `Scope::new`, checked where the derive reads the literal.
fn check_scope(scope: &LitStr) -> syn::Result<()> {
    let name = scope.value();
    let bad = name.is_empty() || name.len() > 64 || name.trim() != name || name.chars().any(|c| c == ':' || c.is_control());
    if bad {
        return Err(syn::Error::new(scope.span(), "a scope name is 1 to 64 bytes, with no `:`, control character or outer whitespace"));
    }
    Ok(())
}

/// A prefix is one lower-camel segment naming the set's type: ids read `<prefix>.<variant_name>`, so a dot or
/// underscore in the prefix would make a second segment.
fn check_prefix(prefix: &LitStr) -> syn::Result<()> {
    let name = prefix.value();
    let mut chars = name.chars();
    let lower_camel = chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_alphanumeric());
    if !lower_camel {
        return Err(syn::Error::new(
            prefix.span(),
            "a command prefix is one lower-camel segment naming the set's type, such as `canvasView`: no `.` or `_`",
        ));
    }
    Ok(())
}

fn platform_tokens(platform: &LitStr) -> syn::Result<TokenStream> {
    Ok(match platform.value().as_str() {
        "mac" => quote!(::craft_ui::runtime::commands::Platform::Mac),
        "windows" => quote!(::craft_ui::runtime::commands::Platform::Windows),
        "linux" => quote!(::craft_ui::runtime::commands::Platform::Linux),
        _ => return Err(syn::Error::new(platform.span(), "`no_default_on` is `\"mac\"`, `\"windows\"` or `\"linux\"`")),
    })
}

fn modal_tokens(modal: &LitStr) -> syn::Result<TokenStream> {
    Ok(match modal.value().as_str() {
        "blocked" => quote!(::craft_ui::runtime::commands::ModalRule::Blocked),
        "in_dialogs" => quote!(::craft_ui::runtime::commands::ModalRule::InDialogs),
        "always" => quote!(::craft_ui::runtime::commands::ModalRule::Always),
        "declared" => quote!(::craft_ui::runtime::commands::ModalRule::Declared),
        _ => return Err(syn::Error::new(modal.span(), "`modal` is `\"blocked\"`, `\"in_dialogs\"`, `\"always\"` or `\"declared\"`")),
    })
}

fn edit_tokens(edit: &LitStr) -> syn::Result<TokenStream> {
    let name = match edit.value().as_str() {
        "cut" => "Cut",
        "copy" => "Copy",
        "paste" => "Paste",
        "select_all" => "SelectAll",
        "undo" => "Undo",
        "redo" => "Redo",
        _ => return Err(syn::Error::new(edit.span(), "`edit` is `\"cut\"`, `\"copy\"`, `\"paste\"`, `\"select_all\"`, `\"undo\"` or `\"redo\"`")),
    };
    let name = syn::Ident::new(name, edit.span());
    Ok(quote!(::craft_ui::runtime::commands::EditCommand::#name))
}

fn variant(variant: &Variant) -> syn::Result<VariantAttrs> {
    if !matches!(variant.fields, Fields::Unit) {
        return Err(syn::Error::new_spanned(variant, "a command is a unit variant"));
    }
    let mut attrs = VariantAttrs::default();
    command_attrs(&variant.attrs, |meta| {
        let value = |meta: &syn::meta::ParseNestedMeta<'_>| meta.value()?.parse::<LitStr>();
        match () {
            () if meta.path.is_ident("label") => attrs.label = Some(value(meta)?),
            () if meta.path.is_ident("icon") => attrs.icon = Some(value(meta)?),
            () if meta.path.is_ident("kind") => attrs.kind = Some(value(meta)?),
            () if meta.path.is_ident("group") => attrs.group = Some(value(meta)?),
            () if meta.path.is_ident("modal") => attrs.modal = Some(value(meta)?),
            () if meta.path.is_ident("edit") => attrs.edit = Some(value(meta)?),
            () if meta.path.is_ident("default") && meta.input.peek(syn::Token![=]) => attrs.defaults.push(value(meta)?),
            () if meta.path.is_ident("default") => meta.parse_nested_meta(|inner| {
                let platform = inner.path.get_ident().map(|ident| LitStr::new(&ident.to_string(), ident.span()));
                match platform {
                    Some(platform) if matches!(platform.value().as_str(), "mac" | "windows" | "linux") => {
                        attrs.platform_defaults.push((platform, inner.value()?.parse()?));
                        Ok(())
                    }
                    _ => Err(inner.error("a platform default is `default(mac = \"..\")`, `default(windows = \"..\")` or `default(linux = \"..\")`")),
                }
            })?,
            () if meta.path.is_ident("no_default_on") => attrs.no_default_on.push(value(meta)?),
            () if meta.path.is_ident("path") => attrs.paths.push(value(meta)?),
            () if meta.path.is_ident("alias") => attrs.aliases.push(value(meta)?),
            () => return Ok(false),
        }
        Ok(true)
    })?;
    Ok(attrs)
}

/// The builder calls that carry a variant's icon, kind, modal, paths and aliases.
fn command_chain(attrs: &VariantAttrs) -> syn::Result<TokenStream> {
    let mut chain = quote!();
    if let Some(icon) = &attrs.icon {
        chain.extend(quote!(.with_icon(#icon)));
    }
    match (&attrs.kind, &attrs.group) {
        (Some(kind), group) => {
            let kind = kind_tokens(kind, group.as_ref())?;
            chain.extend(quote!(.with_kind(#kind)));
        }
        (None, Some(group)) => return Err(syn::Error::new(group.span(), "`group` needs `kind = \"radio\"` or `kind = \"tool\"`")),
        (None, None) => {}
    }
    if let Some(modal) = &attrs.modal {
        let modal = modal_tokens(modal)?;
        chain.extend(quote!(.with_modal(#modal)));
    }
    for path in &attrs.paths {
        let segments = path.value().split('/').map(str::to_owned).collect::<Vec<_>>();
        chain.extend(quote!(.with_path(::std::vec![#(::std::string::String::from(#segments)),*])));
    }
    for alias in &attrs.aliases {
        chain.extend(quote!(.with_alias(#alias)));
    }
    Ok(chain)
}

pub(crate) fn derive(input: TokenStream) -> syn::Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    let ident = &input.ident;
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(ident, "`#[derive(CommandSet)]` needs an enum of unit variants"));
    };
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(&input.generics, "a command set has no generic parameters"));
    }
    let mut set = SetAttrs::default();
    command_attrs(&input.attrs, |meta| {
        if meta.path.is_ident("prefix") {
            set.prefix = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("scope") {
            set.scope = Some(meta.value()?.parse()?);
        } else {
            return Ok(false);
        }
        Ok(true)
    })?;
    let prefix = set.prefix.ok_or_else(|| syn::Error::new_spanned(ident, "a command set names its id prefix: `#[command(prefix = \"..\")]`"))?;
    check_prefix(&prefix)?;
    if let Some(scope) = &set.scope {
        check_scope(scope)?;
    }
    let scope = match &set.scope {
        Some(scope) => quote!(::craft_ui::runtime::commands::CommandScope::new(#scope)),
        None => quote!(::craft_ui::runtime::commands::CommandScope::APP),
    };
    let (mut ids, mut arms, mut defs, mut bare, mut per_platform, mut edits) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for v in &data.variants {
        let attrs = variant(v)?;
        let name = &v.ident;
        let id = format!("{}.{}", prefix.value(), snake(&name.to_string()));
        if ids.contains(&id) {
            return Err(syn::Error::new_spanned(v, format!("two variants make the command id `{id}`")));
        }
        let chain = command_chain(&attrs)?;
        let label = attrs.label.ok_or_else(|| syn::Error::new_spanned(v, "a command has a label: `#[command(label = \"..\")]`"))?;
        let mut defaults = Vec::new();
        for text in &attrs.defaults {
            defaults.push(quote! {
                def = def.with_default(::craft_ui::runtime::commands::Binding::parse(#text).map_err(|error| ::craft_ui::runtime::commands::CommandDefError::binding(#id, error))?);
            });
        }
        for platform in &attrs.no_default_on {
            let platform = platform_tokens(platform)?;
            bare.push(quote!((#id, #platform)));
        }
        for (platform, text) in &attrs.platform_defaults {
            let platform = platform_tokens(platform)?;
            per_platform.push(quote! {
                out.push((#id, #platform, ::craft_ui::runtime::commands::Binding::parse(#text).map_err(|error| ::craft_ui::runtime::commands::CommandDefError::binding(#id, error))?));
            });
        }
        if let Some(edit) = &attrs.edit {
            let edit = edit_tokens(edit)?;
            edits.push(quote!((#id, #edit)));
        }
        arms.push(quote!(#id => ::core::option::Option::Some(Self::#name)));
        defs.push(quote! {
            {
                let scope = Self::SCOPE.scope().ok_or_else(|| ::craft_ui::runtime::commands::CommandDefError::scope(#id))?;
                let mut def = ::craft_ui::runtime::commands::CommandDef::new(::craft_ui::runtime::commands::CommandRef::new(#id), #label, scope) #chain;
                #(#defaults)*
                out.push(def);
            }
        });
        ids.push(id);
    }
    let names: Vec<_> = data.variants.iter().map(|v| &v.ident).collect();
    Ok(quote! {
        impl ::craft_ui::runtime::commands::CommandSet for #ident {
            const SCOPE: ::craft_ui::runtime::commands::CommandScope = #scope;

            fn from_ref(command: &::craft_ui::runtime::commands::CommandRef) -> ::core::option::Option<Self> {
                match command.id.as_str() {
                    #(#arms,)*
                    _ => ::core::option::Option::None,
                }
            }

            fn id(self) -> &'static str {
                match self {
                    #(Self::#names => #ids,)*
                }
            }

            fn defs() -> ::core::result::Result<::std::vec::Vec<::craft_ui::runtime::commands::CommandDef>, ::craft_ui::runtime::commands::CommandDefError> {
                let mut out = ::std::vec::Vec::new();
                #(#defs)*
                ::core::result::Result::Ok(out)
            }

            fn no_default_on() -> ::std::vec::Vec<(&'static str, ::craft_ui::runtime::commands::Platform)> {
                ::std::vec![#(#bare),*]
            }

            fn edits() -> ::std::vec::Vec<(&'static str, ::craft_ui::runtime::commands::EditCommand)> {
                ::std::vec![#(#edits),*]
            }

            fn platform_defaults() -> ::core::result::Result<
                ::std::vec::Vec<(&'static str, ::craft_ui::runtime::commands::Platform, ::craft_ui::runtime::commands::Binding)>,
                ::craft_ui::runtime::commands::CommandDefError,
            > {
                let mut out = ::std::vec::Vec::new();
                #(#per_platform)*
                ::core::result::Result::Ok(out)
            }
        }

        impl ::core::convert::From<#ident> for ::craft_ui::runtime::commands::CommandRef {
            fn from(command: #ident) -> Self {
                ::craft_ui::runtime::commands::CommandRef::new(::craft_ui::runtime::commands::CommandSet::id(command))
            }
        }

        const _: () = {
            ::craft_ui::__register_command_set!(#ident);
        };
    })
}
