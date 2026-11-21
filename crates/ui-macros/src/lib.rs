//! Macros of craft-ui: `#[derive(Component, Pattern, Props, ComponentStyle, TokenGroup, CommandSet, EnvValue, PlainData, MemoProps)]`,
//! `words!`, `#[story]`, `#[craft_ui::testing::trial]` and `register_contract!`.
#![warn(missing_docs)]

mod command_set;
mod component;
mod env;
mod memo;
mod pattern;
mod plain;
mod props;
mod registry;
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
/// application scope without one); `#[command(story)]` marks a story's set, which only an install that shows stories
/// loads (the gallery, the story fixtures), so its keys never reach an app. The prefix is one lower-camel segment naming the set's type, and ids are
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

/// Writes craft-tracked's `PlainData` marker and requires it of every field, so a field whose type can change behind
/// a shared reference (interior mutability, raw pointers, `dyn` objects, closures) does not compile; the error names
/// the field. Works on structs, tuple structs and enums. A field of a foreign plain type without an implementation
/// carries `#[plain_data(assume = "why the type is plain")]`, which skips the check for that field; the reason is
/// required.
#[proc_macro_derive(PlainData, attributes(plain_data))]
pub fn derive_plain_data(input: TokenStream) -> TokenStream {
    expanded(plain::derive(input.into()))
}

/// Writes `MemoProps`: every field feeds the memo key, in order (an enum's variant index first), and every field's type
/// must implement `MemoProps`, so a field holding interior mutability, a closure, a trait object or a raw pointer is a
/// compile error at that field. Works on structs, tuple structs and enums. A field of a foreign plain type without an
/// implementation carries `#[memo_props(hash = "why the type is plain")]`: it feeds the key through its `Hash`, and its
/// type must be `Hash + Sync` (which keeps out raw pointers, `NonNull`, `Cell` and `RefCell`), with a `Hash` that covers
/// everything the view shows and is prefix-free (two different values never feed bytes of which one starts the
/// other); the reason is required. A `&'static` reference to a static item (a registration) carries
/// `#[memo_props(address = "why")]` and feeds where it points. A field another field of the same variant stands for (a
/// closure a generation counter covers) carries `#[memo_props(covered_by = "<field>", reason = "why")]` and feeds
/// nothing; the named field must exist and feed the key itself. A type parameter is bound by `MemoProps`, except one
/// marked `#[memo_props(stateless)]`: the type of a callback held as `Stateless<E>`, which no fn item or closure
/// implements `MemoProps` for, is bound by `'static` only (`struct Bar<#[memo_props(stateless)] E> { english:
/// Stateless<E> }`). A type that is a handle to state written during the pass, or a context rebuilt each pass, carries
/// `#[memo_props(every_pass = "why")]`: it feeds no field, and a view whose key holds it runs in every pass.
#[proc_macro_derive(MemoProps, attributes(memo_props))]
pub fn derive_memo_props(input: TokenStream) -> TokenStream {
    expanded(memo::derive(input.into()))
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
/// The function takes no arguments and returns `()`; it fails by panicking. The crate registry includes it in the runner. An `#[ignore]` on the function
/// makes it run only under `--ignored`.
#[proc_macro_attribute]
pub fn trial(attr: TokenStream, item: TokenStream) -> TokenStream {
    let k = util::ui_crate();
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
    let gate = component::declares_suite().then(|| quote!(#[cfg(any(test, feature = "suite"))]));
    let helper = quote::format_ident!("__registry_trial_{}", ident);
    quote! {
        #gate
        #func

        #gate
        pub(crate) fn #helper() -> #k::testing::Trial {
            #k::testing::Trial::#register(module_path!(), #name, #ident)
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
    commands: Option<Expr>,
}

impl Parse for StoryArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let ty: Type = input.parse()?;
        let (mut covers, mut drive, mut disabled_text, mut title, mut group, mut commands) = (Vec::new(), None, None, None, None, None);
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
            } else if key == "commands" {
                commands = Some(input.parse::<Expr>()?);
            } else {
                return Err(syn::Error::new(
                    key.span(),
                    "`story` takes `covers = [..]`, `drive = ..`, `disabled_text = ..`, `title = ..`, `group = ..` and `commands = ..`",
                ));
            }
        }
        Ok(StoryArgs { ty, covers, drive, disabled_text, title, group, commands })
    }
}

/// Registers a function as a story of a component type: `#[story(Slider, covers = [Default, Mixed])]` on
/// `fn(&mut Ui, &mut StoryData) -> StoryOutput`. `drive = Drive::Step` names a drive other than the automatic one; `disabled_text = ".."` gives the story's control a
/// reason to show while disabled, which the story takes from `StoryData::tooltip`. `title = ".."` is the caption a
/// viewer shows over the story. `group = ".."` puts the story in a named group with the type's other stories that
/// name it, in declaration order; a viewer shows a group together, each story under its caption.
/// `commands = <fn>` names a `fn(&mut CommandStates)` that sets the app-scope command states the story runs under.
/// The type's registration and the story meet by the type's registered name.
#[proc_macro_attribute]
pub fn story(attr: TokenStream, item: TokenStream) -> TokenStream {
    let k = util::ui_crate();
    let args = parse_macro_input!(attr as StoryArgs);
    let mut func = parse_macro_input!(item as ItemFn);
    func.vis = syn::parse_quote!(pub(crate));
    let ident = &func.sig.ident;
    let name = ident.to_string();
    let ty = &args.ty;
    let covers = &args.covers;
    let drive = args.drive.as_ref().map(|drive| quote!(.drive(#drive)));
    let disabled_text = args.disabled_text.as_ref().map(|text| quote!(.disabled_text(#text)));
    let title = args.title.as_ref().map(|title| quote!(.title(#title)));
    let group = args.group.as_ref().map(|group| quote!(.group(#group)));
    let commands = args.commands.as_ref().map(|commands| quote!(.commands(#commands)));
    let gate = component::registration_gate();
    let helper = quote::format_ident!("__registry_story_{}", ident);
    let registration = quote! {
        #k::testing::StoryRegistration::new(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_CRATE_NAME"),
            ::std::any::type_name::<#ty>,
            <#ty as #k::testing::Registered>::registration,
            #k::testing::Story::new(#name, #ident).covers(&[#(#k::testing::Required::#covers),*]) #drive #disabled_text #title #group #commands,
        ).at(file!(), line!())
    };
    quote! {
        #gate
        #func

        #gate
        #k::__register_contract! {
            @suite
            pub(crate) fn #helper() -> #k::testing::StoryRegistration { #registration }
        }

    }
    .into()
}

/// Generates a crate registration function by reading its source modules.
#[proc_macro]
pub fn registry(input: TokenStream) -> TokenStream {
    if !input.is_empty() {
        return syn::Error::new(proc_macro2::Span::call_site(), "registry! takes no arguments").to_compile_error().into();
    }
    expanded(registry::expand())
}
