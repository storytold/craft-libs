#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! `#[command]`: write a command handler with a typed params struct and get its `CommandSpec` row
//! and a JSON Schema derived from that struct.
//!
//! ```ignore
//! #[derive(serde::Deserialize, schemars::JsonSchema)]
//! #[serde(deny_unknown_fields)]
//! struct NestParams { name: String, parent: Option<u32> }
//!
//! #[command(id = "clip.nest", label = "Nest", doc = r#"{"name":str,"parent":binId?}"#, menu = ["Clip"])]
//! fn nest(session: &mut Session, params: NestParams) -> Result<serde_json::Value, AppError> { ... }
//! ```
//!
//! For a function `nest` the macro keeps the function and adds, next to it and with its visibility:
//!
//! - `NEST_COMMAND`: a `craft_command::CommandSpec<Session, AppError>` row. Its `run` reads the
//!   JSON params into `NestParams` (so `deny_unknown_fields` and every serde rule apply) and calls
//!   `nest`. A params error goes through `bad_params` (default `From<String>` on the app error).
//! - `nest_schema()`: the `schemars` schema of `NestParams`, tagged `"x-craft-source": "derived"`.
//! - `NEST_SCHEMA`: `(id, nest_schema)`, one row for an app's schema side table.
//!
//! The row's `params` stays the `doc` string. `CommandSpec.params` is a `&'static str`, and a schema
//! needs `schemars` at run time, so the schema cannot sit in the row. `doc` is required so the row is
//! never left claiming an empty key list.
//!
//! The crate that uses the macro needs `craft-command`, `serde_json` and `schemars` as dependencies.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{Expr, ExprArray, FnArg, ItemFn, Lit, LitBool, LitStr, Path, PathArguments, ReturnType, Type};

#[derive(Default)]
struct Args {
    id: Option<LitStr>,
    label: Option<LitStr>,
    doc: Option<LitStr>,
    shortcut: Option<LitStr>,
    alt_shortcuts: Vec<LitStr>,
    menu: Vec<LitStr>,
    enabled: Option<Path>,
    bad_params: Option<Path>,
    journal: Option<LitBool>,
}

fn str_array(array: ExprArray) -> syn::Result<Vec<LitStr>> {
    array
        .elems
        .into_iter()
        .map(|e| match e {
            Expr::Lit(l) => match l.lit {
                Lit::Str(s) => Ok(s),
                other => Err(syn::Error::new_spanned(other, "menu entries are string literals")),
            },
            other => Err(syn::Error::new_spanned(other, "menu entries are string literals")),
        })
        .collect()
}

fn parse_args(tokens: TokenStream2) -> syn::Result<Args> {
    let mut args = Args::default();
    let parser = syn::meta::parser(|meta| {
        let ident = meta.path.get_ident().map(ToString::to_string).unwrap_or_default();
        match ident.as_str() {
            "id" => args.id = Some(meta.value()?.parse()?),
            "label" => args.label = Some(meta.value()?.parse()?),
            "doc" => args.doc = Some(meta.value()?.parse()?),
            "shortcut" => args.shortcut = Some(meta.value()?.parse()?),
            "alt_shortcuts" => args.alt_shortcuts = str_array(meta.value()?.parse()?)?,
            "menu" => args.menu = str_array(meta.value()?.parse()?)?,
            "enabled" => args.enabled = Some(meta.value()?.parse()?),
            "bad_params" => args.bad_params = Some(meta.value()?.parse()?),
            "journal" => args.journal = Some(meta.value()?.parse()?),
            _ => return Err(meta.error("unknown argument; expected id, label, doc, menu, shortcut, alt_shortcuts, enabled, bad_params or journal")),
        }
        Ok(())
    });
    syn::parse::Parser::parse2(parser, tokens)?;
    Ok(args)
}

fn need<'a>(value: &'a Option<LitStr>, name: &str) -> syn::Result<&'a LitStr> {
    value.as_ref().ok_or_else(|| syn::Error::new(Span::call_site(), format!("#[command] needs {name} = \"...\"")))
}

/// `&mut S` gives `S`.
fn session_type(arg: &FnArg) -> syn::Result<&Type> {
    if let FnArg::Typed(t) = arg
        && let Type::Reference(r) = &*t.ty
        && r.mutability.is_some()
    {
        return Ok(&r.elem);
    }
    Err(syn::Error::new_spanned(arg, "the first parameter must be `&mut Session`"))
}

fn params_type(arg: &FnArg) -> syn::Result<&Type> {
    match arg {
        FnArg::Typed(t) => Ok(&t.ty),
        FnArg::Receiver(_) => Err(syn::Error::new_spanned(arg, "a command is a free function, not a method")),
    }
}

/// `Result<T, E>` gives `E`.
fn error_type(ret: &ReturnType) -> syn::Result<&Type> {
    if let ReturnType::Type(_, ty) = ret
        && let Type::Path(p) = &**ty
        && let Some(seg) = p.path.segments.last()
        && seg.ident == "Result"
        && let PathArguments::AngleBracketed(a) = &seg.arguments
        && a.args.len() == 2
        && let Some(syn::GenericArgument::Type(e)) = a.args.last()
    {
        return Ok(e);
    }
    Err(syn::Error::new_spanned(ret, "the return type must be `Result<serde_json::Value, AppError>`"))
}

fn expand(args: TokenStream2, item: TokenStream2) -> syn::Result<TokenStream2> {
    let args = parse_args(args)?;
    let func: ItemFn = syn::parse2(item)?;
    if func.sig.asyncness.is_some() || !func.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(&func.sig, "a command cannot be async or generic"));
    }
    if func.sig.inputs.len() != 2 {
        return Err(syn::Error::new_spanned(&func.sig, "a command takes `(session: &mut Session, params: Params)`"));
    }
    let mut inputs = func.sig.inputs.iter();
    let (Some(first), Some(second)) = (inputs.next(), inputs.next()) else {
        return Err(syn::Error::new_spanned(&func.sig, "a command takes `(session: &mut Session, params: Params)`"));
    };
    let session = session_type(first)?;
    let params = params_type(second)?;
    let error = error_type(&func.sig.output)?;

    let id = need(&args.id, "id")?;
    let label = need(&args.label, "label")?;
    let doc = need(&args.doc, "doc")?;
    let name = &func.sig.ident;
    let vis = &func.vis;
    let upper = name.to_string().to_uppercase();
    let spec_name = format_ident!("{upper}_COMMAND");
    let schema_row = format_ident!("{upper}_SCHEMA");
    let schema_fn = format_ident!("{name}_schema");
    let run_name = format_ident!("__craft_run_{name}");
    let enabled_name = format_ident!("__craft_enabled_{name}");
    let menu = &args.menu;
    let alt_shortcuts = &args.alt_shortcuts;
    let shortcut = match &args.shortcut {
        Some(s) => quote!(::core::option::Option::Some(#s)),
        None => quote!(::core::option::Option::None),
    };
    let journal = args.journal.as_ref().is_none_or(|b| b.value);
    let convert = match &args.bad_params {
        Some(path) => quote!(#path),
        None => quote!(<#error as ::core::convert::From<::std::string::String>>::from),
    };
    let (enabled_fn, enabled_ref) = match &args.enabled {
        Some(path) => (quote!(), quote!(#path)),
        None => (
            // The default is "always enabled", which is what a row with no enablement rule means.
            quote! {
                #[doc(hidden)]
                fn #enabled_name(_session: &#session) -> ::core::result::Result<(), ::std::string::String> { ::core::result::Result::Ok(()) }
            },
            quote!(#enabled_name),
        ),
    };

    Ok(quote! {
        #func

        #enabled_fn

        #[doc(hidden)]
        fn #run_name(session: &mut #session, value: &::serde_json::Value) -> ::core::result::Result<::serde_json::Value, #error> {
            let parsed: #params = ::serde_json::from_value(value.clone())
                .map_err(|e| (#convert)(::std::format!("{}: bad params: {e}", #id)))?;
            #name(session, parsed)
        }

        #[doc = ::core::concat!("The `CommandSpec` row of `", #id, "`.")]
        #vis const #spec_name: ::craft_command::CommandSpec<#session, #error> = ::craft_command::CommandSpec {
            id: #id,
            label: #label,
            menu: &[#(#menu),*],
            shortcut: #shortcut,
            alt_shortcuts: &[#(#alt_shortcuts),*],
            params: #doc,
            enabled: #enabled_ref,
            run: #run_name,
            journal: #journal,
        };

        #[doc = ::core::concat!("The JSON Schema of the params of `", #id, "`, tagged `x-craft-source: derived`.")]
        #vis fn #schema_fn() -> ::serde_json::Value {
            let mut schema = ::serde_json::Value::from(::schemars::schema_for!(#params));
            if let ::serde_json::Value::Object(map) = &mut schema {
                map.insert(::std::string::String::from("x-craft-source"), ::serde_json::Value::from("derived"));
            }
            schema
        }

        #[doc = ::core::concat!("`(id, schema fn)` of `", #id, "` for a schema side table.")]
        #vis const #schema_row: (&'static str, fn() -> ::serde_json::Value) = (#id, #schema_fn);
    })
}

/// Generate the `CommandSpec` row and the derived schema for a command handler. See the crate docs.
#[proc_macro_attribute]
pub fn command(args: TokenStream, item: TokenStream) -> TokenStream {
    match expand(args.into(), item.into()) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
