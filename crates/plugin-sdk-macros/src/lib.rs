#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! `#[craft_plugin]`: turns an inline module of handler functions into a craft-abi 1 plugin.
//!
//! The attribute reads `craft.toml` at compile time, checks it with the same manifest rules the
//! install gate uses, assigns each contribution its `entry` number (its index in
//! `contributions`), binds each contribution to a handler function by name, and emits:
//!
//! - the manifest as a `craft.manifest` custom section (wasm32 only),
//! - the `craft_abi`, `craft_alloc`, `craft_free`, `craft_invoke` and `craft_process` exports
//!   (wasm32 only, and only the exports the contributions need),
//! - a `craft_testing` module with the dispatch tables, so a native unit test can call a
//!   handler by contribution id.
//!
//! A manifest error, a missing handler, or an unsupported kind is a compile error that points at
//! the module.

use std::collections::BTreeMap;
use std::path::PathBuf;

use craft_plugin_abi::manifest::{Kind, Manifest};
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use serde_json::Value;
use syn::spanned::Spanned;
use syn::{Item, ItemMod, LitStr};

/// How the generated code runs one contribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Command,
    Hook,
    Panel,
    /// `format`: the file bytes or the pixels arrive in the blob.
    Format,
    Image,
    /// A kind this release does not know. The host never calls it.
    Unknown,
}

fn shape_of(kind: &Kind) -> Result<Shape, String> {
    match kind {
        Kind::Command | Kind::Menu => Ok(Shape::Command),
        Kind::Hook => Ok(Shape::Hook),
        Kind::Panel => Ok(Shape::Panel),
        Kind::Effect | Kind::Filter | Kind::ObjectFilter | Kind::LiveEffect => Ok(Shape::Image),
        Kind::Format => Ok(Shape::Format),
        Kind::AudioEffect => Err(format!("kind \"{}\" has no handler shape in craft-plugin-sdk yet", kind.name())),
        Kind::Unknown(_) => Ok(Shape::Unknown),
    }
}

struct Args {
    manifest: String,
}

fn parse_args(args: TokenStream2) -> syn::Result<Args> {
    let mut manifest = "craft.toml".to_owned();
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("manifest") {
            let lit: LitStr = meta.value()?.parse()?;
            manifest = lit.value();
            Ok(())
        } else {
            Err(meta.error("the only argument is manifest = \"path/to/craft.toml\""))
        }
    });
    syn::parse::Parser::parse2(parser, args)?;
    Ok(Args { manifest })
}

/// One contribution after the manifest was read.
struct Bound {
    id: String,
    kind: String,
    shape: Shape,
    handler: Option<syn::Ident>,
}

/// Read `craft.toml`, return the manifest JSON bytes (entries assigned, `handler` keys removed)
/// and the contributions with their handler names.
fn load_manifest(path: &str, span: Span) -> syn::Result<(Vec<u8>, Vec<Bound>)> {
    let root = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| syn::Error::new(span, "CARGO_MANIFEST_DIR is not set"))?;
    let full = PathBuf::from(root).join(path);
    let text = std::fs::read_to_string(&full).map_err(|e| syn::Error::new(span, format!("cannot read {}: {e}", full.display())))?;
    let mut doc: Value = toml::from_str(&text).map_err(|e| syn::Error::new(span, format!("{}: {e}", full.display())))?;
    let Some(root_obj) = doc.as_object_mut() else {
        return Err(syn::Error::new(span, "craft.toml must be a table"));
    };
    root_obj.entry("schema").or_insert(Value::from(1));
    root_obj.entry("abi").or_insert(serde_json::json!({ "craft": 1 }));
    let Some(list) = root_obj.get_mut("contributions").and_then(Value::as_array_mut) else {
        return Err(syn::Error::new(span, "craft.toml needs at least one [[contributions]] table"));
    };
    let mut handlers: Vec<Option<String>> = Vec::new();
    for (index, item) in list.iter_mut().enumerate() {
        let Some(obj) = item.as_object_mut() else {
            return Err(syn::Error::new(span, format!("contributions[{index}] must be a table")));
        };
        if obj.contains_key("entry") {
            return Err(syn::Error::new(span, format!("contributions[{index}].entry is assigned by #[craft_plugin]; remove it")));
        }
        let handler = match obj.remove("handler") {
            Some(Value::String(h)) => Some(h),
            Some(_) => return Err(syn::Error::new(span, format!("contributions[{index}].handler must be a string"))),
            None => None,
        };
        obj.insert("entry".to_owned(), Value::from(index as u64));
        handlers.push(handler);
    }
    let bytes = serde_json::to_vec(&doc).map_err(|e| syn::Error::new(span, e.to_string()))?;
    let manifest = Manifest::parse(&bytes).map_err(|e| syn::Error::new(span, format!("{}: {e}", full.display())))?;
    let mut bound = Vec::new();
    for (c, handler) in manifest.contributions.iter().zip(handlers) {
        let shape = shape_of(&c.kind).map_err(|m| syn::Error::new(span, format!("contribution \"{}\": {m}", c.id)))?;
        let handler = match (shape, handler) {
            (Shape::Unknown, None) => None,
            (Shape::Unknown, Some(_)) => {
                return Err(syn::Error::new(span, format!("contribution \"{}\" has an unknown kind, so it cannot have a handler", c.id)));
            }
            (_, Some(h)) => Some(
                syn::parse_str::<syn::Ident>(&h)
                    .map_err(|_| syn::Error::new(span, format!("contribution \"{}\": handler {h:?} is not a Rust identifier", c.id)))?,
            ),
            (_, None) => {
                return Err(syn::Error::new(span, format!("contribution \"{}\" (kind {}) needs a handler = \"function_name\"", c.id, c.kind.name())));
            }
        };
        bound.push(Bound { id: c.id.clone(), kind: c.kind.name().to_owned(), shape, handler });
    }
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, b) in bound.iter().enumerate() {
        if let Some(prev) = seen.insert(&b.id, i) {
            return Err(syn::Error::new(span, format!("contribution id \"{}\" appears at {prev} and {i}", b.id)));
        }
    }
    Ok((bytes, bound))
}

fn adapter(index: usize, b: &Bound) -> Option<(proc_macro2::Ident, TokenStream2)> {
    let handler = b.handler.as_ref()?;
    let name = format_ident!("__craft_entry_{index}");
    let sdk = quote!(::craft_plugin_sdk);
    let body = match b.shape {
        Shape::Command => quote! {
            fn #name(req: &#sdk::Value, _blob: &[u8]) -> ::core::result::Result<::core::option::Option<#sdk::Value>, #sdk::String> {
                #sdk::entry::command(req, #handler)
            }
        },
        Shape::Hook => quote! {
            fn #name(req: &#sdk::Value, _blob: &[u8]) -> ::core::result::Result<::core::option::Option<#sdk::Value>, #sdk::String> {
                #sdk::entry::hook(req, #handler)
            }
        },
        Shape::Panel => quote! {
            fn #name(req: &#sdk::Value, _blob: &[u8]) -> ::core::result::Result<::core::option::Option<#sdk::Value>, #sdk::String> {
                #sdk::entry::panel(req, #handler)
            }
        },
        Shape::Format => quote! {
            fn #name(req: &#sdk::Value, blob: &[u8]) -> ::core::result::Result<::core::option::Option<#sdk::Value>, #sdk::String> {
                #sdk::entry::format(req, blob, #handler)
            }
        },
        Shape::Image => quote! {
            fn #name(band: &mut #sdk::Band<'_>) -> ::core::result::Result<(), #sdk::String> {
                #sdk::entry::image(band, #handler)
            }
        },
        Shape::Unknown => return None,
    };
    Some((name, body))
}

fn expand(args: TokenStream2, item: TokenStream2) -> syn::Result<TokenStream2> {
    let args = parse_args(args)?;
    let mut module: ItemMod = syn::parse2(item)?;
    let span = module.span();
    let (bytes, bound) = load_manifest(&args.manifest, span)?;
    let Some((_, items)) = module.content.as_mut() else {
        return Err(syn::Error::new(span, "#[craft_plugin] needs an inline module: `mod name { ... }`"));
    };
    // Every handler must exist, so the error names the contribution instead of a deep type error.
    let defined: Vec<String> = items.iter().filter_map(|i| if let Item::Fn(f) = i { Some(f.sig.ident.to_string()) } else { None }).collect();
    for b in &bound {
        if let Some(h) = &b.handler
            && !defined.contains(&h.to_string())
        {
            return Err(syn::Error::new(span, format!("contribution \"{}\": the module has no function `{h}`", b.id)));
        }
    }
    let sdk = quote!(::craft_plugin_sdk);
    let n = bound.len();
    let has_control = bound.iter().any(|b| matches!(b.shape, Shape::Command | Shape::Hook | Shape::Panel | Shape::Format));
    let has_data = bound.iter().any(|b| b.shape == Shape::Image);

    let mut adapters = Vec::new();
    let mut control_slots = Vec::new();
    let mut data_slots = Vec::new();
    for (i, b) in bound.iter().enumerate() {
        match adapter(i, b) {
            Some((name, body)) => {
                adapters.push(body);
                if b.shape == Shape::Image {
                    control_slots.push(quote!(::core::option::Option::None));
                    data_slots.push(quote!(::core::option::Option::Some(#name as #sdk::DataFn)));
                } else {
                    control_slots.push(quote!(::core::option::Option::Some(#name as #sdk::ControlFn)));
                    data_slots.push(quote!(::core::option::Option::None));
                }
            }
            None => {
                control_slots.push(quote!(::core::option::Option::None));
                data_slots.push(quote!(::core::option::Option::None));
            }
        }
    }
    let ids: Vec<&str> = bound.iter().map(|b| b.id.as_str()).collect();
    let kinds: Vec<&str> = bound.iter().map(|b| b.kind.as_str()).collect();
    let json_text = String::from_utf8(bytes.clone()).map_err(|e| syn::Error::new(span, e.to_string()))?;
    let byte_lit = syn::LitByteStr::new(&bytes, span);
    let len = bytes.len();
    let manifest_path = args.manifest.clone();

    let control_export = if has_control {
        quote! {
            #[unsafe(no_mangle)]
            pub extern "C" fn craft_invoke(entry: i32, req_ptr: i32, req_len: i32, blob_ptr: i32, blob_len: i32) -> i64 {
                #sdk::rt::invoke(&super::__CRAFT_CONTROL, entry, req_ptr as u32, req_len as u32, blob_ptr as u32, blob_len as u32)
            }
        }
    } else {
        quote!()
    };
    let data_export = if has_data {
        quote! {
            #[unsafe(no_mangle)]
            pub extern "C" fn craft_process(desc_ptr: i32, desc_len: i32) -> i32 {
                #sdk::rt::process(&super::__CRAFT_DATA, desc_ptr as u32, desc_len as u32)
            }
        }
    } else {
        quote!()
    };
    let data_table = quote! { static __CRAFT_DATA: [::core::option::Option<#sdk::DataFn>; #n] = [#(#data_slots),*]; };
    let control_table = quote! { static __CRAFT_CONTROL: [::core::option::Option<#sdk::ControlFn>; #n] = [#(#control_slots),*]; };

    let generated: TokenStream2 = quote! {
        // Re-read the manifest whenever it changes.
        const _: &[u8] = ::core::include_bytes!(::core::concat!(::core::env!("CARGO_MANIFEST_DIR"), "/", #manifest_path));

        #(#adapters)*
        #control_table
        #data_table

        #[cfg(target_arch = "wasm32")]
        #[allow(unsafe_code)]
        #[unsafe(link_section = "craft.manifest")]
        #[used]
        static __CRAFT_MANIFEST: [u8; #len] = *#byte_lit;

        #[cfg(target_arch = "wasm32")]
        #[allow(unsafe_code)]
        mod __craft_exports {
            #[unsafe(no_mangle)]
            pub extern "C" fn craft_abi() -> i32 {
                1
            }
            #[unsafe(no_mangle)]
            pub extern "C" fn craft_alloc(len: i32, align: i32) -> i32 {
                #sdk::rt::alloc(len as u32, align as u32) as i32
            }
            #[unsafe(no_mangle)]
            pub extern "C" fn craft_free(ptr: i32, len: i32, align: i32) {
                #sdk::rt::free(ptr as u32, len as u32, align as u32)
            }
            #control_export
            #data_export
        }

        /// Native test access to this plugin: the manifest and the dispatch tables.
        #[allow(dead_code)]
        pub mod craft_testing {
            /// The manifest as the host reads it.
            pub const MANIFEST_JSON: &str = #json_text;
            /// `(contribution id, kind)` in entry order.
            pub const CONTRIBUTIONS: &[(&str, &str)] = &[#((#ids, #kinds)),*];

            fn entry_of(id: &str) -> ::core::result::Result<usize, #sdk::String> {
                CONTRIBUTIONS.iter().position(|(c, _)| *c == id).ok_or_else(|| #sdk::format!("no contribution {id:?}"))
            }

            /// Run a control plane contribution (`command`, `menu`, `hook`, `panel`) the way
            /// `craft_invoke` does, with `request` as the JSON the host would send.
            pub fn invoke(id: &str, request: #sdk::Value) -> ::core::result::Result<::core::option::Option<#sdk::Value>, #sdk::String> {
                let entry = entry_of(id)?;
                #sdk::rt::invoke_value(&super::__CRAFT_CONTROL, entry, &request, &[])
            }

            /// [`invoke`] with a blob, for a `format` contribution: the file bytes, or the RGBA8 pixels of a write.
            pub fn invoke_with_blob(id: &str, request: #sdk::Value, blob: &[u8]) -> ::core::result::Result<::core::option::Option<#sdk::Value>, #sdk::String> {
                let entry = entry_of(id)?;
                #sdk::rt::invoke_value(&super::__CRAFT_CONTROL, entry, &request, blob)
            }

            /// Run a data plane contribution on one band, the way `craft_process` does.
            pub fn process(id: &str, desc: &#sdk::abi::ProcessDescriptor, input: &[u8], params: #sdk::Value) -> ::core::result::Result<#sdk::Vec<u8>, #sdk::String> {
                let entry = entry_of(id)?;
                #sdk::rt::process_vec(&super::__CRAFT_DATA, entry, desc, input, params)
            }
        }
    };
    items.push(Item::Verbatim(generated));
    Ok(quote!(#module))
}

/// Turn an inline module of handlers into a craft-abi 1 plugin. See the crate docs.
#[proc_macro_attribute]
pub fn craft_plugin(args: TokenStream, item: TokenStream) -> TokenStream {
    match expand(args.into(), item.into()) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn run(manifest: &str, module: &str) -> Result<String, String> {
        let args: TokenStream2 = format!("manifest = \"tests/fixtures/{manifest}\"").parse().unwrap();
        let item: TokenStream2 = module.parse().unwrap();
        expand(args, item).map(|t| t.to_string()).map_err(|e| e.to_string())
    }

    const MODULE_A: &str = "mod m { fn a(_c: CommandCall) -> Result<Value, String> { Ok(Value::Null) } }";

    #[test]
    fn a_good_manifest_and_handler_expand_to_exports_and_a_manifest_section() {
        let out = run("good.toml", MODULE_A).unwrap();
        for needle in ["fn craft_abi", "fn craft_alloc", "fn craft_free", "fn craft_invoke", "craft.manifest", "craft_testing", "__craft_entry_0"] {
            assert!(out.contains(needle), "expansion lacks {needle}: {out}");
        }
        assert!(!out.contains("fn craft_process"), "a command-only plugin must not export craft_process");
    }

    #[test]
    fn a_data_plane_contribution_adds_craft_process_and_a_control_one_adds_craft_invoke() {
        // The fixture has only a command, so build the data case from the sdk's own fixture.
        let args: TokenStream2 = "manifest = \"../plugin-sdk/tests/fixtures/plugin.toml\"".parse().unwrap();
        let module = "mod m { fn echo(_c: CommandCall)->R{} fn fail(_c: CommandCall)->R{} fn gate(_h: HookCall)->V{} fn watcher(_h: HookCall)->V{} fn side(_e: PanelEvent)->O{} fn halve(_b:&mut Band)->R{} }";
        let out = expand(args, module.parse().unwrap()).unwrap().to_string();
        assert!(out.contains("fn craft_process") && out.contains("fn craft_invoke"));
    }

    #[test]
    fn a_format_contribution_gets_the_blob() {
        let out = run("format.toml", "mod m { fn f(_c: FormatCall) -> Result<FormatOutput, String> { todo!() } }").unwrap();
        assert!(out.contains("entry :: format") && out.contains("fn craft_invoke") && out.contains("invoke_with_blob"), "{out}");
    }

    #[test]
    fn an_unknown_kind_alone_needs_no_handler_and_no_invoke_export() {
        let out = run("only_unknown.toml", "mod m {}").unwrap();
        assert!(!out.contains("fn craft_invoke") && !out.contains("fn craft_process"), "{out}");
        assert!(out.contains("fn craft_abi"));
    }

    #[test]
    fn errors_name_the_problem() {
        let cases: [(&str, &str, &str); 12] = [
            ("no_handler.toml", MODULE_A, "needs a handler"),
            ("good.toml", "mod m { fn other() {} }", "no function `a`"),
            ("entry_given.toml", MODULE_A, "entry is assigned by #[craft_plugin]"),
            ("unknown_with_handler.toml", MODULE_A, "unknown kind, so it cannot have a handler"),
            ("audio.toml", MODULE_A, "no handler shape"),
            ("bad_permission.toml", MODULE_A, "permissions.required[0]"),
            ("duplicate_id.toml", MODULE_A, "appears at 0 and 1"),
            ("before_without_match.toml", MODULE_A, "before hook needs match"),
            ("bad_contribution_id.toml", MODULE_A, "contributions[0].id"),
            ("no_contributions.toml", MODULE_A, "[[contributions]]"),
            ("broken.toml", MODULE_A, "broken.toml"),
            ("missing-file.toml", MODULE_A, "cannot read"),
        ];
        for (file, module, want) in cases {
            let err = run(file, module).expect_err(file);
            assert!(err.contains(want), "{file}: wanted {want:?} in {err:?}");
        }
    }

    #[test]
    fn the_module_must_be_inline_and_the_args_must_be_known() {
        let err = run("good.toml", "mod m;").unwrap_err();
        assert!(err.contains("inline module"), "{err}");
        let args: TokenStream2 = "bogus = 1".parse().unwrap();
        let err = expand(args, MODULE_A.parse().unwrap()).unwrap_err().to_string();
        assert!(err.contains("the only argument is manifest"), "{err}");
    }
}
