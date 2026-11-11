//! Macros of craft-ui: `#[derive(Component, Pattern, Props, ComponentStyle, TokenGroup, CommandSet, EnvValue)]`,
//! `words!`, `#[story]`, `#[craft_ui::testing::trial]` and `register_contract!`.

mod command_set;
mod component;
mod env;
mod pattern;
mod props;
mod style;
mod token_group;
mod util;
mod words;

#[cfg(test)]
mod tests;

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, Ident, ItemFn, ReturnType, Token, Type, parse_macro_input};

fn expanded(result: syn::Result<proc_macro2::TokenStream>) -> TokenStream {
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Writes `CommandSet` for an enum of unit variants and registers the set, which the command board loads at
/// install. `#[command(prefix = "..", scope = "..")]` on the enum gives the id prefix and the scope (the
/// application scope without one). The prefix is one lower-camel segment naming the set's type, and ids are
/// `prefix.variant_name`. Each variant takes `label`, `icon`, `kind`, `modal`, `edit` (the `EditCommand` a focused component takes the command as), `default`, `path` and `alias`.
#[proc_macro_derive(CommandSet, attributes(command))]
pub fn derive_command_set(input: TokenStream) -> TokenStream {
    expanded(command_set::derive(input.into()))
}

/// Writes `Meta`, the `<Type>StyleKey` type, `HasProps` for a container or collection, and the type's
/// registration and probe.
///
/// Attributes: `#[component]`, `#[component(container)]`, `#[component(collection)]`, `widget`, `overlay`,
/// `content_painter`, `canvas_control`, `unregistered`, `exempt(case = .., reason = ..)`, `probe(Type<..>, ..)` on the
/// type;
/// `#[common]`, `#[container]` and `#[style_key]` on fields.
///
/// `unregistered` implements `Meta` and registers nothing, for a face an app never composes on its own: a collection's
/// item or a leaf that only its parent shows. Its parent's stories and suite cases cover it. It is not a way to skip
/// stories for a type an app uses: such a type registers and has stories.
#[proc_macro_derive(Component, attributes(component, common, container, style_key))]
pub fn derive_component(input: TokenStream) -> TokenStream {
    expanded(component::derive(input.into()))
}

/// Writes `Pattern` for a struct that only assembles components, and registers it so the components index and the
/// suite list it. `#[pattern(probe(Type<..>, ..))]` lists the instantiations of a type with type parameters.
/// `#[pattern(tool)]` registers a `CanvasTool` and `#[pattern(dialog)]` a `StackDialog` instead: their stories show
/// them through a canvas or a dialog host, and the index lists them as a Tool or a Dialog.
#[proc_macro_derive(Pattern, attributes(pattern))]
pub fn derive_pattern(input: TokenStream) -> TokenStream {
    expanded(pattern::derive(input.into()))
}

/// Registers an `impl Component` written without the derive: `register_contract!(Type)`, with instantiations
/// for a generic type (`register_contract!(Field<'static, Plain>, Field<'static, Number>)`) and options after
/// a semicolon (`register_contract!(Menu; container, overlay)`; the shape is a leaf unless `container` or
/// `collection` is given).
#[proc_macro]
pub fn register_contract(input: TokenStream) -> TokenStream {
    expanded(component::register(input.into()))
}

/// Writes one setter per `#[prop]` field: `#[prop]`, `#[prop(into)]`, `#[prop(flag)]`, `#[prop(name = "..")]`.
/// The field's doc comment is the setter's. `#[prop(quantity = "..")]` names what a numeric prop measures; the
/// `props/point-size` lint reads it as the declaration that the number is not a size in points, and it changes no
/// generated code.
///
/// A field marked `#[collection]` (type `CollectionProps`) gets one forwarding setter per `CollectionProps`
/// builder, and a field marked `#[container]` (type `ContainerProps`) one per `ContainerProps` builder a component
/// takes under the builder's own name. The two lists are `COLLECTION` and `CONTAINER` in `props.rs`.
#[proc_macro_derive(Props, attributes(prop, collection, container))]
pub fn derive_props(input: TokenStream) -> TokenStream {
    expanded(props::derive(input.into()))
}

/// Writes `ComponentStyle` from `#[style(surface)]`, `#[style(focus)]` and `#[style(metrics)]` fields (one each),
/// any number of `#[style(text)]` (`TextLook`) and `#[style(icon)]` (`IconLook`) fields, and
/// `#[style(variants = [..])]` on the type.
#[proc_macro_derive(ComponentStyle, attributes(style))]
pub fn derive_component_style(input: TokenStream) -> TokenStream {
    expanded(style::derive(input.into()))
}

/// Writes `EnvValue`; `#[env(style)]` also writes `AFFECTS_STYLE`, the style hash and the `StyleValue` marker,
/// and `#[env(root = ..)]` gives the root value (`Default` otherwise).
#[proc_macro_derive(EnvValue, attributes(env))]
pub fn derive_env_value(input: TokenStream) -> TokenStream {
    expanded(env::derive(input.into()))
}

/// Writes `TokenGroup`'s `PATH` and theme overlay from `#[group(path = "..")]`; the author writes an inherent
/// `fn derive(tokens: &Tokens) -> Self`.
#[proc_macro_derive(TokenGroup, attributes(group))]
pub fn derive_token_group(input: TokenStream) -> TokenStream {
    expanded(token_group::derive(input.into()))
}

/// Writes a struct of built-in strings and its `WordSet` impl, one row per field. A row is
/// `field: "module.camelCase" = "English";`, or starts with `template`, `unit`, `count` (`"{n} item" | "{n} items"`),
/// `array`, `context` or `plain`. The macro checks ids, placeholders and that unit and count rows contain `{n}`.
#[proc_macro]
pub fn words(input: TokenStream) -> TokenStream {
    expanded(words::expand_words(input.into()))
}

/// Registers a function as a trial of its crate's test executable.
///
/// The function takes no arguments and returns `()`; it fails by panicking. The registration lists it in
/// `craft_ui::testing::trials()`, so the runner finds it without a hand-kept list. An `#[ignore]` on the function
/// makes it run only under `--ignored`.
#[proc_macro_attribute]
pub fn trial(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new(proc_macro2::Span::call_site(), "`trial` takes no arguments").to_compile_error().into();
    }
    let mut func = parse_macro_input!(item as ItemFn);
    // `#[ignore]` marks a fixture another trial runs as a subprocess; it means nothing on a plain fn.
    let attrs = func.attrs.len();
    func.attrs.retain(|attr| !attr.path().is_ident("ignore"));
    let register = if func.attrs.len() == attrs { quote!(new) } else { quote!(ignored) };
    let sig = &func.sig;
    let well_formed = sig.inputs.is_empty() && sig.asyncness.is_none() && sig.generics.params.is_empty() && matches!(sig.output, ReturnType::Default);
    if !well_formed {
        return syn::Error::new_spanned(sig, "a trial is `fn name()` with no arguments, generics or return value").to_compile_error().into();
    }
    let ident = &sig.ident;
    let name = ident.to_string();
    quote! {
        #func

        ::craft_ui::testing::__private::inventory::submit! {
            ::craft_ui::testing::Trial::#register(module_path!(), #name, #ident)
        }
    }
    .into()
}

/// The arguments of `#[story(Type, covers = [..], drive = ..)]`.
struct StoryArgs {
    ty: Type,
    covers: Vec<Ident>,
    drive: Option<Expr>,
    disabled_text: Option<Expr>,
    title: Option<Expr>,
    group: Option<Expr>,
}

impl Parse for StoryArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let ty: Type = input.parse()?;
        let (mut covers, mut drive, mut disabled_text, mut title, mut group) = (Vec::new(), None, None, None, None);
        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            if key == "covers" {
                let list;
                syn::bracketed!(list in input);
                covers = Punctuated::<Ident, Token![,]>::parse_terminated(&list)?.into_iter().collect();
            } else if key == "drive" {
                drive = Some(input.parse::<Expr>()?);
            } else if key == "disabled_text" {
                disabled_text = Some(input.parse::<Expr>()?);
            } else if key == "title" {
                title = Some(input.parse::<Expr>()?);
            } else if key == "group" {
                group = Some(input.parse::<Expr>()?);
            } else {
                return Err(syn::Error::new(
                    key.span(),
                    "`story` takes `covers = [..]`, `drive = ..`, `disabled_text = ..`, `title = ..` and `group = ..`",
                ));
            }
        }
        Ok(StoryArgs { ty, covers, drive, disabled_text, title, group })
    }
}

/// Registers a function as a story of a component type: `#[story(Slider, covers = [Default, Mixed])]` on
/// `fn(&mut Ui, &mut StoryData) -> StoryOutput`. `drive = Drive::Step` names a drive other than the automatic one; `disabled_text = ".."` gives the story's control a
/// reason to show while disabled, which the story takes from `StoryData::tooltip`. `title = ".."` is the caption a
/// viewer shows over the story. `group = ".."` puts the story in a named group with the type's other stories that
/// name it, in declaration order; a viewer shows a group together, each story under its caption.
/// The type's registration and the story meet by the type's registered name.
#[proc_macro_attribute]
pub fn story(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as StoryArgs);
    let func = parse_macro_input!(item as ItemFn);
    let ident = &func.sig.ident;
    let name = ident.to_string();
    let ty = &args.ty;
    let covers = &args.covers;
    let drive = args.drive.as_ref().map(|drive| quote!(.drive(#drive)));
    let disabled_text = args.disabled_text.as_ref().map(|text| quote!(.disabled_text(#text)));
    let title = args.title.as_ref().map(|title| quote!(.title(#title)));
    let group = args.group.as_ref().map(|group| quote!(.group(#group)));
    quote! {
        #func

        ::craft_ui::testing::__private::inventory::submit! {
            ::craft_ui::testing::StoryRegistration::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_CRATE_NAME"),
                ::std::any::type_name::<#ty>,
                ::craft_ui::testing::Story::new(#name, #ident).covers(&[#(::craft_ui::testing::Required::#covers),*]) #drive #disabled_text #title #group,
            )
            .at(file!(), line!())
        }
    }
    .into()
}
