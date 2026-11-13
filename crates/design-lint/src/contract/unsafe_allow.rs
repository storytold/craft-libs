//! `contract/unsafe-allow`: no `allow(unsafe_code)` in source, except in craft-win32 and the benchmark's allocator
//! (`ALLOWED`). Every other crate keeps the workspace `forbid`.

use std::path::Path;

use syn::visit::{self, Visit};
use syn::{Attribute, Meta};

use crate::report::Finding;

use super::Cx;

pub const ID: &str = "contract/unsafe-allow";

/// The places that may allow `unsafe_code`, as path prefixes relative to the repository root, as in findings:
/// craft-win32, which holds the Windows FFI calls (the owner's exception, 2026-11-08), and the benchmark's global
/// allocator.
pub(crate) const ALLOWED: [&str; 2] = ["crates/win32/src", "crates/ui-bench/src/alloc.rs"];

struct Allows {
    spans: Vec<proc_macro2::Span>,
}

impl Visit<'_> for Allows {
    fn visit_attribute(&mut self, attr: &Attribute) {
        if matches!(attr.path().get_ident().map(|i| i.to_string()).as_deref(), Some("allow" | "expect")) && names_unsafe_code(&attr.meta) {
            self.spans.push(attr.pound_token.spans[0]);
        }
        visit::visit_attribute(self, attr);
    }
}

fn names_unsafe_code(meta: &Meta) -> bool {
    let Meta::List(list) = meta else { return false };
    list.tokens.clone().into_iter().any(|token| matches!(token, proc_macro2::TokenTree::Ident(ident) if ident == "unsafe_code"))
}

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    if ALLOWED.iter().any(|allowed| cx.path.starts_with(Path::new(allowed))) {
        return;
    }
    let mut allows = Allows { spans: Vec::new() };
    allows.visit_file(cx.file);
    for span in allows.spans {
        cx.flag_span(out, ID, ID, span, "`allow(unsafe_code)`; only craft-win32 and the benchmark allocator allow it".to_owned());
    }
}
