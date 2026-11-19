//! Discovers suite items in a crate's source modules for `registry!()`.

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use std::path::{Path, PathBuf};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Attribute, Item, ItemMod, Meta, Token, Type};

struct FirstType(Type);
impl Parse for FirstType {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let first = input.parse()?;
        let _: TokenStream = input.parse()?;
        Ok(Self(first))
    }
}

fn attrs_cfg(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs.iter().filter(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr")).cloned().collect()
}

fn derives(attrs: &[Attribute], wanted: &str) -> bool {
    attrs.iter().filter(|a| a.path().is_ident("derive")).any(|a| {
        a.parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
            .is_ok_and(|paths| paths.iter().any(|p| p.segments.last().is_some_and(|s| s.ident == wanted)))
    })
}

fn marker(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.path().segments.last().is_some_and(|s| s.ident == name))
}

fn opt(attrs: &[Attribute], attr: &str, value: &str) -> bool {
    attrs.iter().filter(|a| a.path().is_ident(attr)).any(|a| match &a.meta {
        Meta::List(list) => list.tokens.to_string().split(|c: char| !c.is_alphanumeric() && c != '_').any(|word| word == value),
        _ => false,
    })
}

fn include_value(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Lit(expr) => match &expr.lit {
            syn::Lit::Str(value) => Some(value.value()),
            _ => None,
        },
        syn::Expr::Macro(expr) if expr.mac.path.is_ident("env") => {
            let name = syn::parse2::<syn::LitStr>(expr.mac.tokens.clone()).ok()?;
            (name.value() == "OUT_DIR").then(|| std::env::var("OUT_DIR").ok()).flatten()
        }
        syn::Expr::Macro(expr) if expr.mac.path.is_ident("concat") => {
            let args = expr.mac.parse_body_with(Punctuated::<syn::Expr, Token![,]>::parse_terminated).ok()?;
            args.iter().map(include_value).collect::<Option<Vec<_>>>().map(|parts| parts.concat())
        }
        _ => None,
    }
}

fn contains_suite_item(items: &[Item]) -> bool {
    items.iter().any(|item| match item {
        Item::Struct(item) => {
            (derives(&item.attrs, "Component") && !opt(&item.attrs, "component", "unregistered")) || derives(&item.attrs, "Pattern")
        }
        Item::Enum(item) => derives(&item.attrs, "CommandSet") && opt(&item.attrs, "command", "story"),
        Item::Fn(item) => marker(&item.attrs, "story") || marker(&item.attrs, "trial"),
        Item::Mod(item) => item.content.as_ref().is_some_and(|(_, items)| contains_suite_item(items)),
        Item::Macro(item) => {
            let name = item.mac.path.segments.last().map(|segment| segment.ident.to_string()).unwrap_or_default();
            let body = item.mac.tokens.to_string();
            name == "register_contract"
                || name == "include"
                || (name == "macro_rules"
                    && (body.contains("register_contract")
                        || body.contains("# [story")
                        || body.contains("# [trial")
                        || (body.contains("derive") && (body.contains("Component") || body.contains("Pattern")) && !body.contains("unregistered"))
                        || (body.contains("CommandSet") && body.contains("story"))))
        }
        _ => false,
    })
}

fn module_file(dir: &Path, path_dir: &Path, module: &ItemMod) -> Option<PathBuf> {
    for attr in &module.attrs {
        if attr.path().is_ident("path")
            && let Meta::NameValue(value) = &attr.meta
            && let syn::Expr::Lit(expr) = &value.value
            && let syn::Lit::Str(path) = &expr.lit
        {
            return Some(path_dir.join(path.value()));
        }
    }
    let plain = dir.join(format!("{}.rs", module.ident));
    if plain.is_file() {
        Some(plain)
    } else {
        let nested = dir.join(module.ident.to_string()).join("mod.rs");
        nested.is_file().then_some(nested)
    }
}

fn walk(
    items: &[Item],
    file: &Path,
    dir: &Path,
    path_dir: &Path,
    path: &[Ident],
    inherited: &[Attribute],
    calls: &mut Vec<TokenStream>,
) -> syn::Result<()> {
    for item in items {
        match item {
            Item::Mod(module) => {
                let mut cfg = inherited.to_vec();
                cfg.extend(attrs_cfg(&module.attrs));
                let mut child_path = path.to_vec();
                child_path.push(module.ident.clone());
                if let Some((_, content)) = &module.content {
                    let inline_dir = dir.join(module.ident.to_string());
                    walk(content, file, &inline_dir, &inline_dir, &child_path, &cfg, calls)?;
                } else if let Some(file) = module_file(dir, path_dir, module) {
                    let text = std::fs::read_to_string(&file)
                        .map_err(|e| syn::Error::new_spanned(module, format!("cannot read {}: {e}", file.display())))?;
                    let parsed =
                        syn::parse_file(&text).map_err(|e| syn::Error::new_spanned(module, format!("cannot parse {}: {e}", file.display())))?;
                    let child_dir = if file.file_name().is_some_and(|n| n == "mod.rs" || n == "main.rs" || n == "lib.rs") {
                        file.parent().unwrap_or(dir).to_path_buf()
                    } else {
                        file.parent().unwrap_or(dir).join(file.file_stem().unwrap_or_default())
                    };
                    walk(&parsed.items, &file, &child_dir, file.parent().unwrap_or(dir), &child_path, &cfg, calls)?;
                }
            }
            Item::Struct(item)
                if (derives(&item.attrs, "Component") && !opt(&item.attrs, "component", "unregistered")) || derives(&item.attrs, "Pattern") =>
            {
                let helper = format_ident!("__registry_component_{}", crate::command_set::snake(&item.ident.to_string()));
                let mut cfg = inherited.to_vec();
                cfg.extend(attrs_cfg(&item.attrs));
                calls.push(quote! { #(#cfg)* crate::#(#path::)*#helper(registry); });
            }
            Item::Fn(item) if marker(&item.attrs, "story") || marker(&item.attrs, "trial") => {
                let (prefix, method) =
                    if marker(&item.attrs, "story") { ("story", format_ident!("add_story")) } else { ("trial", format_ident!("add_trial")) };
                let helper = format_ident!("__registry_{}_{}", prefix, item.sig.ident);
                let mut cfg = inherited.to_vec();
                cfg.extend(attrs_cfg(&item.attrs));
                calls.push(quote! { #(#cfg)* registry.#method(crate::#(#path::)*#helper()); });
            }
            Item::Enum(item) if derives(&item.attrs, "CommandSet") && opt(&item.attrs, "command", "story") => {
                let helper = format_ident!("__registry_command_{}", crate::command_set::snake(&item.ident.to_string()));
                let mut cfg = inherited.to_vec();
                cfg.extend(attrs_cfg(&item.attrs));
                calls.push(quote! { #(#cfg)* registry.add_story_set(crate::#(#path::)*#helper()); });
            }
            Item::Macro(item) if item.mac.path.is_ident("include") => {
                let expression: syn::Expr = syn::parse2(item.mac.tokens.clone())?;
                let included =
                    include_value(&expression).ok_or_else(|| syn::Error::new_spanned(item, "registry! cannot inspect this include! path"))?;
                let included = file.parent().unwrap_or(dir).join(included);
                let text = std::fs::read_to_string(&included)
                    .map_err(|e| syn::Error::new_spanned(item, format!("cannot inspect {}: {e}", included.display())))?;
                let parsed =
                    syn::parse_file(&text).map_err(|e| syn::Error::new_spanned(item, format!("cannot inspect {}: {e}", included.display())))?;
                if contains_suite_item(&parsed.items) {
                    return Err(syn::Error::new_spanned(item, "suite registration inside include! is invisible to registry!"));
                }
            }
            Item::Macro(item) if item.mac.path.segments.last().is_some_and(|s| s.ident == "register_contract") => {
                let FirstType(ty) = syn::parse2(item.mac.tokens.clone())?;
                if let Type::Path(ty) = ty
                    && let Some(name) = ty.path.segments.last()
                {
                    let helper = format_ident!("__registry_component_{}", crate::command_set::snake(&name.ident.to_string()));
                    let mut cfg = inherited.to_vec();
                    cfg.extend(attrs_cfg(&item.attrs));
                    calls.push(quote! { #(#cfg)* crate::#(#path::)*#helper(registry); });
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn expand() -> syn::Result<TokenStream> {
    let invocation =
        proc_macro::Span::call_site().local_file().ok_or_else(|| syn::Error::new(Span::call_site(), "registry! needs a local source file"))?;
    let manifest =
        std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from).ok_or_else(|| syn::Error::new(Span::call_site(), "missing CARGO_MANIFEST_DIR"))?;
    let invocation = std::fs::canonicalize(&invocation).unwrap_or(invocation);
    let src = std::fs::canonicalize(manifest.join("src")).unwrap_or_else(|_| manifest.join("src"));
    let source = if invocation.starts_with(&src) { src.join("lib.rs") } else { invocation };
    let text = std::fs::read_to_string(&source).map_err(|e| syn::Error::new(Span::call_site(), format!("cannot read {}: {e}", source.display())))?;
    let parsed = syn::parse_file(&text)?;
    let mut calls = Vec::new();
    walk(&parsed.items, &source, source.parent().unwrap_or(&manifest), source.parent().unwrap_or(&manifest), &[], &[], &mut calls)?;
    let k = crate::util::ui_crate();
    Ok(quote! {
        /// Adds this crate's suite items to the caller's registry.
        pub fn register(registry: &mut #k::testing::Registry) {
            #(#calls)*
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_registration_inside_an_include() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry/root.rs");
        let parsed = syn::parse_file("include!(\"included.rs\");");
        assert!(parsed.is_ok());
        if let Ok(parsed) = parsed {
            let mut calls = Vec::new();
            let dir = root.parent().unwrap_or(Path::new(""));
            assert!(walk(&parsed.items, &root, dir, dir, &[], &[], &mut calls).is_err());
        }
    }

    #[test]
    fn follows_modules_and_keeps_their_conditions() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry/root.rs");
        let source = std::fs::read_to_string(&root).unwrap_or_default();
        let parsed = syn::parse_file(&source);
        assert!(parsed.is_ok());
        if let Ok(parsed) = parsed {
            let mut calls = Vec::new();
            let result =
                walk(&parsed.items, &root, root.parent().unwrap_or(Path::new("")), root.parent().unwrap_or(Path::new("")), &[], &[], &mut calls);
            assert!(result.is_ok());
            let generated = quote! { #(#calls)* }.to_string();
            assert!(generated.contains("feature = \"sample\""));
            assert!(generated.contains("__registry_story_view"));
            assert!(generated.contains("__registry_trial_changes"));
            assert!(generated.contains("__registry_trial_path_trial"));
        }
    }
}
