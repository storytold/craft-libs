//! The rules that ban egui APIs (spec 24.1) and the API-shape rules.
//!
//! Each rule is one module with a `check` function over the file's [`Cx`]. [`REGISTRY`] is the only place
//! that lists them. "Allowed in" locations come from the [`RuleScope`], never from this crate.

mod access;
mod allocate;
mod containers;
mod egui_import;
mod egui_widget;
mod entry;
mod facts;
mod keys;
mod layers;
mod memory;
mod motion;
mod non_exhaustive;
mod pipeline_only;
mod platform;
mod pointer;
mod props;
mod runtime_egui;
mod scope;
mod shape;
mod sites;
mod style;
mod test_rules;
mod text_edit;
mod types;
mod unsafe_allow;

use std::path::Path;

pub(crate) use facts::build as build_facts;
pub use scope::{RuleScope, StaticScope};
pub use types::Types;

use crate::report::Finding;
use sites::Site;

/// Every rule id of this module, for the marker and exemption validation in [`crate::RULES`].
pub(crate) const IDS: [&str; 39] = [
    egui_import::ID,
    runtime_egui::ID,
    text_edit::ID,
    allocate::ID,
    egui_widget::ID,
    access::ID,
    layers::ID,
    memory::ID,
    keys::ID,
    pointer::ID,
    motion::ID,
    containers::ID,
    platform::ID,
    test_rules::LAYOUT,
    test_rules::KIT,
    test_rules::HARNESS,
    test_rules::CFG_TEST,
    test_rules::LITERAL_SIZE,
    test_rules::IGNORED,
    non_exhaustive::ID,
    pipeline_only::ID,
    props::RESERVED,
    props::HAND_SETTER,
    props::OWN_FIELD,
    props::SHADOW,
    props::POINT_SIZE,
    style::STATE_PICK,
    style::MATCH_STATE,
    style::STATES_FIELD,
    style::SIZE_ARM,
    style::TOKEN_READ,
    style::INSTANCE_SETTER,
    style::TO_EGUI,
    entry::ID,
    shape::SHADOW,
    shape::WIDGET,
    shape::OVERLAY_FLAG,
    shape::REGISTRATION,
    unsafe_allow::ID,
];

type Check = fn(&Cx<'_>, &mut Vec<Finding>);

/// REGISTRY: add a rule by adding its `check` here and its ids to [`IDS`].
const REGISTRY: [Check; 21] = [
    egui_import::check,
    runtime_egui::check,
    text_edit::check,
    allocate::check,
    egui_widget::check,
    access::check,
    layers::check,
    memory::check,
    keys::check,
    pointer::check,
    motion::check,
    containers::check,
    platform::check,
    test_rules::check,
    non_exhaustive::check,
    pipeline_only::check,
    props::check,
    style::check,
    entry::check,
    shape::check,
    unsafe_allow::check,
];

/// One file as the rules see it.
pub(crate) struct Cx<'a> {
    pub path: &'a Path,
    pub scope: &'a dyn RuleScope,
    pub file: &'a syn::File,
    pub sites: Vec<Site>,
    pub types: types::Types,
}

impl Cx<'_> {
    /// Reports `rule` at `at` unless the scope allows `allow` (a rule id or `rule:sub-permission`) in this file.
    pub fn flag(&self, out: &mut Vec<Finding>, rule: &'static str, allow: &str, at: &Site, message: String) {
        if !self.scope.allows(allow, self.path) {
            out.push(Finding::new(rule, at.line, at.column, at.end_line, message));
        }
    }

    /// Reports `rule` at a source span unless the scope allows `allow` in this file.
    pub fn flag_span(&self, out: &mut Vec<Finding>, rule: &'static str, allow: &str, span: proc_macro2::Span, message: String) {
        if !self.scope.allows(allow, self.path) {
            let (start, end) = (span.start(), span.end());
            out.push(Finding::new(rule, start.line, start.column + 1, end.line, message));
        }
    }

    /// Method-call sites whose name satisfies `pick`.
    pub fn methods<'s>(&'s self, pick: impl Fn(&str, &sites::Args) -> bool + 's) -> impl Iterator<Item = (&'s Site, &'s str)> + 's {
        self.sites.iter().filter_map(move |s| s.method().filter(|(n, a)| pick(n, a)).map(|(n, _)| (s, n)))
    }

    /// Reports every method call that `pick` accepts; `why` completes the message.
    pub fn flag_methods(&self, out: &mut Vec<Finding>, rule: &'static str, allow: &str, why: &str, pick: impl Fn(&str, &sites::Args) -> bool) {
        for (site, name) in self.methods(pick) {
            self.flag(out, rule, allow, site, format!("{} {why}", call(name)));
        }
    }

    /// Reports every egui path (expression, pattern or type) that `pick` accepts; `why` completes the message.
    pub fn flag_paths(&self, out: &mut Vec<Finding>, rule: &'static str, allow: &str, why: &str, pick: impl Fn(&[String]) -> bool) {
        for (site, path) in self.egui_paths(pick) {
            self.flag(out, rule, allow, site, format!("`{path}` {why}"));
        }
    }

    /// Egui-rooted path sites (expressions, patterns, types) with a segment that satisfies `pick`.
    pub fn egui_paths<'s>(&'s self, pick: impl Fn(&[String]) -> bool + 's) -> impl Iterator<Item = (&'s Site, String)> + 's {
        self.sites.iter().filter_map(move |s| s.egui_path().filter(|p| pick(p)).map(|p| (s, p.join("::"))))
    }
}

/// Runs every rule over one file. `src` is parsed here; a file that does not parse yields no findings
/// (the base checker reports it).
pub fn analyze_contract(src: &str, path: &Path, scope: &dyn RuleScope, include_tests: bool) -> Vec<Finding> {
    syn::parse_file(src).map(|file| analyze_file(&file, path, scope, include_tests)).unwrap_or_default()
}

pub(crate) fn analyze_file(file: &syn::File, path: &Path, scope: &dyn RuleScope, include_tests: bool) -> Vec<Finding> {
    let cx = Cx {
        path,
        scope,
        file,
        sites: sites::collect(file, include_tests),
        types: types::collect(file).overlaid(&scope.crate_types(path).cloned().unwrap_or_default()),
    };
    let mut out = Vec::new();
    for check in REGISTRY {
        check(&cx, &mut out);
    }
    out.sort_by_key(|f| (f.line, f.column, f.rule));
    out.dedup_by(|a, b| a.rule == b.rule && a.line == b.line && a.column == b.column);
    out
}

/// A method name, for messages.
fn call(name: &str) -> String {
    format!("`.{name}(..)`")
}
