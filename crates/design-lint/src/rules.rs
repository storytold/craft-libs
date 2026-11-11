//! The `syn`-based rules: literal sizes and named colour constants.

use std::ops::RangeInclusive;

use syn::punctuated::Punctuated;
use syn::spanned::Spanned as _;
use syn::visit::{self, Visit};
use syn::{AttrStyle, Attribute, BinOp, Expr, ExprCall, ExprMethodCall, ExprPath, ExprStruct, Lit, Meta, Token, UnOp};

use crate::report::Finding;

pub const VEC2: &str = "literal-size/vec2";
pub const ADD_SPACE: &str = "literal-size/add-space";
pub const CORNER_RADIUS: &str = "literal-size/corner-radius";
pub const STROKE: &str = "literal-size/stroke";
pub const MARGIN: &str = "literal-size/margin";
pub const FONT_ID: &str = "literal-size/font-id";
pub const RAW_COLOUR_CONST: &str = "raw-colour/const";
pub const STYLE_MUTATION: &str = "style-mutation";
pub const TOOLTIP: &str = "tooltip/stock";
pub const INNER_ALLOW: &str = "clippy/inner-allow";
pub const PARSE: &str = "parse";

const OLDER: [&str; 15] = [
    VEC2,
    ADD_SPACE,
    CORNER_RADIUS,
    STROKE,
    MARGIN,
    FONT_ID,
    RAW_COLOUR_CONST,
    STYLE_MUTATION,
    TOOLTIP,
    INNER_ALLOW,
    PARSE,
    crate::markers::EMPTY,
    crate::markers::INVALID,
    crate::markers::UNUSED,
    crate::UNUSED_EXEMPTION,
];

const SCOPED: usize = crate::contract::IDS.len();

/// Every rule id the checker emits, including the marker and exemption rules and the scoped rules
/// (`contract/*`, `test/*`, `api/*`, `behavior/*`, `props/*`, `style/*`) that `check_with_scope` runs.
pub const RULES: [&str; OLDER.len() + SCOPED] = {
    let mut all = [""; OLDER.len() + SCOPED];
    let mut i = 0;
    while i < OLDER.len() {
        all[i] = OLDER[i];
        i += 1;
    }
    let mut j = 0;
    while j < SCOPED {
        all[OLDER.len() + j] = crate::contract::IDS[j];
        j += 1;
    }
    all
};

/// `selector` is the rule id itself or its family (`literal-size` matches `literal-size/vec2`).
pub(crate) fn selector_matches(selector: &str, rule: &str) -> bool {
    rule == selector || rule.strip_prefix(selector).is_some_and(|rest| rest.starts_with('/'))
}

/// Findings of these rules can be silenced neither by markers nor, for `parse`, by exemptions.
pub(crate) fn is_unmarkable(rule: &str) -> bool {
    rule == PARSE || rule == INNER_ALLOW
}

/// Methods that change the style, visuals, spacing or zoom of a context or `Ui` from app code.
const STYLE_MUTATORS: [&str; 6] = ["style_mut", "spacing_mut", "visuals_mut", "set_zoom_factor", "set_style", "set_visuals"];

/// Methods and functions whose arguments may be a `(width, colour)` stroke tuple.
const STROKE_TAKERS: [&str; 11] =
    ["stroke", "rect", "rect_stroke", "line_segment", "circle", "circle_stroke", "hline", "vline", "line", "closed_line", "dotted_line"];

/// egui's tooltip methods on `Response`, each with the craft-ui contract call that replaces it.
const STOCK_TOOLTIPS: [(&str, &str); 8] = [
    ("on_hover_text", "craft_ui::contract::Props::tooltip"),
    ("on_disabled_hover_text", "craft_ui::contract::Props::tooltip"),
    ("on_hover_ui", "craft_ui::contract::Show::show_with_tip"),
    ("on_disabled_hover_ui", "craft_ui::contract::Show::show_with_tip"),
    ("on_hover_text_at_pointer", "craft_ui::contract::Props::tooltip with TooltipSpec::at(TooltipAt::Pointer)"),
    ("on_hover_ui_at_pointer", "craft_ui::contract::Show::show_with_tip"),
    ("show_tooltip_text", "craft_ui::contract::Props::tooltip with TooltipSpec::when(TooltipWhen::Now)"),
    ("show_tooltip_ui", "craft_ui::contract::Show::show_with_tip"),
];

/// The colour constants of egui's `Rgba`.
const RGBA_COLOURS: [&str; 5] = ["WHITE", "BLACK", "RED", "GREEN", "BLUE"];

const NAMED_COLOURS: [&str; 21] = [
    "WHITE",
    "BLACK",
    "RED",
    "GREEN",
    "BLUE",
    "YELLOW",
    "GRAY",
    "DARK_GRAY",
    "LIGHT_GRAY",
    "LIGHT_RED",
    "DARK_RED",
    "ORANGE",
    "GOLD",
    "BROWN",
    "LIGHT_YELLOW",
    "LIGHT_GREEN",
    "DARK_GREEN",
    "LIGHT_BLUE",
    "DARK_BLUE",
    "PURPLE",
    "KHAKI",
];

/// The findings of one source file and the line ranges skipped as test code.
#[derive(Debug, Default)]
pub struct Analysis {
    /// Findings in source order; `path` and `area` are empty until the caller fills them.
    pub findings: Vec<Finding>,
    /// Line ranges (1-based, inclusive) of `#[cfg(test)]` and `#[test]` items that were not checked.
    pub skipped: Vec<RangeInclusive<usize>>,
    /// Line ranges of `#[cfg(test)]` and `#[test]` items, whether they were checked or not.
    pub test_code: Vec<RangeInclusive<usize>>,
    /// `allow` and `expect` attributes naming a `clippy::disallowed_*` lint, inner and outer.
    pub clippy_allows: usize,
}

/// Checks one file's source text. A file that does not parse yields one `parse` finding.
pub fn analyze_source(src: &str, include_tests: bool) -> Analysis {
    match syn::parse_file(src) {
        Ok(file) => analyze_file(src, &file, include_tests),
        Err(e) => parse_failure(&e),
    }
}

/// The analysis of a file that did not parse: one `parse` finding.
pub(crate) fn parse_failure(e: &syn::Error) -> Analysis {
    let start = e.span().start();
    let finding = Finding::new(PARSE, start.line.max(1), start.column + 1, start.line.max(1), e.to_string());
    Analysis { findings: vec![finding], ..Analysis::default() }
}

/// Checks a parsed file; `src` is its text, used for finding snippets.
pub(crate) fn analyze_file(src: &str, file: &syn::File, include_tests: bool) -> Analysis {
    let mut linter = Linter {
        lines: src.lines().collect(),
        defs: Vec::new(),
        imports: Vec::new(),
        fns: Vec::new(),
        scope: Vec::new(),
        out: Analysis::default(),
        include_tests,
    };
    if linter.skips(&file.attrs, file.span()) {
        return linter.out;
    }
    linter.resolve_consts(file);
    linter.visit_file(file);
    linter.out.findings.sort_by_key(|f| (f.line, f.column));
    linter.out
}

/// A module, or the type (or trait) an `impl` block is for, that encloses an item.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Frame {
    Module(String),
    Type(String),
}

impl Frame {
    fn name(&self) -> &str {
        match self {
            Self::Module(n) | Self::Type(n) => n,
        }
    }
}

/// A `const` or `static` item (module-level, associated or local) and its value when it is a numeric literal expression.
struct ConstDef {
    scope: Vec<Frame>,
    name: String,
    line: usize,
    expr: Expr,
    value: Option<f64>,
}

/// A `use` declaration: `alias` is the name it brings in, `None` for a glob (`path::*`).
struct Import {
    /// The module scope the `use` is in.
    scope: Vec<Frame>,
    alias: Option<String>,
    path: Vec<String>,
}

struct Linter<'a> {
    lines: Vec<&'a str>,
    defs: Vec<ConstDef>,
    imports: Vec<Import>,
    /// Names of fns with the module scope they are defined in.
    fns: Vec<(Vec<Frame>, String)>,
    /// The modules and impl blocks around the item being visited.
    scope: Vec<Frame>,
    out: Analysis,
    include_tests: bool,
}

/// Collects `const NAME: T = <expr>` and `static` items in any scope, with their enclosing modules and impl blocks.
struct ConstCollector {
    defs: Vec<ConstDef>,
    imports: Vec<Import>,
    fns: Vec<(Vec<Frame>, String)>,
    scope: Vec<Frame>,
    include_tests: bool,
}

impl ConstCollector {
    fn add(&mut self, name: &syn::Ident, expr: &Expr) {
        self.defs.push(ConstDef {
            scope: self.scope.clone(),
            name: name.to_string(),
            line: name.span().start().line,
            expr: expr.clone(),
            value: None,
        });
    }

    fn skipped(&self, attrs: &[Attribute]) -> bool {
        !self.include_tests && attrs.iter().any(is_test_attr)
    }
}

impl<'ast> Visit<'ast> for ConstCollector {
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        if self.skipped(&i.attrs) {
            return;
        }
        self.scope.push(Frame::Module(i.ident.to_string()));
        visit::visit_item_mod(self, i);
        self.scope.pop();
    }
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if self.skipped(&i.attrs) {
            return;
        }
        self.scope.push(Frame::Type(type_name(&i.self_ty)));
        visit::visit_item_impl(self, i);
        self.scope.pop();
    }
    fn visit_item_trait(&mut self, i: &'ast syn::ItemTrait) {
        self.scope.push(Frame::Type(i.ident.to_string()));
        visit::visit_item_trait(self, i);
        self.scope.pop();
    }
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        if !self.skipped(&i.attrs) {
            self.fns.push((module_scope(&self.scope).to_vec(), i.sig.ident.to_string()));
            visit::visit_item_fn(self, i);
        }
    }
    fn visit_item_use(&mut self, i: &'ast syn::ItemUse) {
        if !self.skipped(&i.attrs) {
            let mut found = Vec::new();
            flatten_use(&mut Vec::new(), &i.tree, &mut found);
            let scope = module_scope(&self.scope).to_vec();
            self.imports.extend(found.into_iter().map(|(alias, path)| Import { scope: scope.clone(), alias, path }));
        }
    }
    fn visit_item_const(&mut self, i: &'ast syn::ItemConst) {
        self.add(&i.ident, &i.expr);
        visit::visit_item_const(self, i);
    }
    fn visit_item_static(&mut self, i: &'ast syn::ItemStatic) {
        self.add(&i.ident, &i.expr);
        visit::visit_item_static(self, i);
    }
    fn visit_impl_item_const(&mut self, i: &'ast syn::ImplItemConst) {
        self.add(&i.ident, &i.expr);
        visit::visit_impl_item_const(self, i);
    }
}

/// The names a `use` tree brings in with their full paths; `None` is a glob import of the path.
pub(crate) fn flatten_use(prefix: &mut Vec<String>, tree: &syn::UseTree, out: &mut Vec<(Option<String>, Vec<String>)>) {
    use syn::UseTree;
    match tree {
        UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            flatten_use(prefix, &p.tree, out);
            prefix.pop();
        }
        UseTree::Name(n) => {
            let mut path = prefix.clone();
            if n.ident != "self" {
                path.push(n.ident.to_string());
            }
            if let Some(alias) = path.last().cloned() {
                out.push((Some(alias), path));
            }
        }
        UseTree::Rename(r) => {
            let mut path = prefix.clone();
            if r.ident != "self" {
                path.push(r.ident.to_string());
            }
            if r.rename != "_" {
                out.push((Some(r.rename.to_string()), path));
            }
        }
        UseTree::Glob(_) => out.push((None, prefix.clone())),
        UseTree::Group(g) => {
            for t in &g.items {
                flatten_use(prefix, t, out);
            }
        }
    }
}

/// The last identifier of a type path (`Button` for `crate::ui::Button<T>`); empty for other types.
fn type_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(p) => last_ident(&p.path).unwrap_or_default(),
        syn::Type::Group(g) => type_name(&g.elem),
        syn::Type::Paren(p) => type_name(&p.elem),
        _ => String::new(),
    }
}

/// The identifiers of a path expression without `<..>` arguments or a leading `<T as Trait>`.
pub(crate) fn path_idents(e: &ExprPath) -> Option<Vec<String>> {
    (e.qself.is_none() && e.path.segments.iter().all(|s| s.arguments.is_none()))
        .then(|| e.path.segments.iter().map(|s| s.ident.to_string()).collect())
}

/// The scope up to and including its innermost module.
fn module_scope(scope: &[Frame]) -> &[Frame] {
    let end = scope.iter().rposition(|f| matches!(f, Frame::Module(_))).map_or(0, |i| i + 1);
    &scope[..end]
}

/// The last two segments of a path, `("Vec2", "new")` for `egui::Vec2::new`.
fn last_two(path: &syn::Path) -> Option<(String, String)> {
    let n = path.segments.len();
    (n >= 2).then(|| (path.segments[n - 2].ident.to_string(), path.segments[n - 1].ident.to_string()))
}

fn last_ident(path: &syn::Path) -> Option<String> {
    path.segments.last().map(|s| s.ident.to_string())
}

fn cfg_is_test(meta: &Meta) -> bool {
    match meta {
        Meta::Path(p) => p.is_ident("test"),
        Meta::List(l) if l.path.is_ident("all") => {
            l.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated).is_ok_and(|inner| inner.iter().any(cfg_is_test))
        }
        _ => false,
    }
}

pub(crate) fn is_test_attr(attr: &Attribute) -> bool {
    if attr.path().is_ident("test") {
        return true;
    }
    if !attr.path().is_ident("cfg") {
        return false;
    }
    match &attr.meta {
        Meta::List(l) => l.parse_args::<Meta>().is_ok_and(|m| cfg_is_test(&m)),
        _ => false,
    }
}

/// Whether the attribute is `allow(..)` or `expect(..)` naming a `clippy::disallowed_*` lint.
fn is_disallowed_clippy_allow(attr: &Attribute) -> bool {
    if !(attr.path().is_ident("allow") || attr.path().is_ident("expect")) {
        return false;
    }
    let Meta::List(list) = &attr.meta else { return false };
    let Ok(lints) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else { return false };
    lints.iter().any(|m| {
        let Meta::Path(p) = m else { return false };
        let mut segments = p.segments.iter();
        matches!((segments.next(), segments.next(), segments.next()), (Some(a), Some(b), None) if a.ident == "clippy" && b.ident.to_string().starts_with("disallowed_"))
    })
}

/// Whether a method call chain starts at `RichText::..`.
fn rooted_in_rich_text(e: &Expr) -> bool {
    match e {
        Expr::MethodCall(m) => rooted_in_rich_text(&m.receiver),
        Expr::Paren(p) => rooted_in_rich_text(&p.expr),
        Expr::Call(c) => matches!(&*c.func, Expr::Path(p) if last_two(&p.path).is_some_and(|(ty, _)| ty == "RichText")),
        _ => false,
    }
}

fn peel(e: &Expr) -> &Expr {
    match e {
        Expr::Paren(p) => peel(&p.expr),
        Expr::Group(g) => peel(&g.expr),
        other => other,
    }
}

impl Linter<'_> {
    /// Whether an item with these attributes is test code to skip; records its lines when it is test code.
    fn skips(&mut self, attrs: &[Attribute], span: proc_macro2::Span) -> bool {
        if !attrs.iter().any(is_test_attr) {
            return false;
        }
        let lines = span.start().line..=span.end().line;
        self.out.test_code.push(lines.clone());
        if self.include_tests {
            return false;
        }
        self.out.skipped.push(lines);
        true
    }

    fn resolve_consts(&mut self, file: &syn::File) {
        let mut collector =
            ConstCollector { defs: Vec::new(), imports: Vec::new(), fns: Vec::new(), scope: Vec::new(), include_tests: self.include_tests };
        collector.visit_file(file);
        self.defs = collector.defs;
        self.imports = collector.imports;
        self.fns = collector.fns;
        loop {
            let resolved: Vec<(usize, f64)> = self
                .defs
                .iter()
                .enumerate()
                .filter(|(_, d)| d.value.is_none())
                .filter_map(|(i, d)| self.eval_in(&d.expr, &d.scope).map(|v| (i, v)))
                .collect();
            if resolved.is_empty() {
                break;
            }
            for (i, v) in resolved {
                self.defs[i].value = Some(v);
            }
        }
    }

    /// The full paths a path written here can stand for, through the file's `use` items.
    ///
    /// A first segment that is an imported name (also renamed, or grouped) is replaced by the imported path.
    /// Otherwise the path as written comes first, then, through each glob import in scope, the path under the glob's
    /// prefix. A bare name that a fn in scope defines is not extended through globs: the local fn wins.
    fn expand_path(&self, path: &syn::Path) -> Vec<Vec<String>> {
        let written: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        let Some(first) = written.first() else { return Vec::new() };
        let module = module_scope(&self.scope);
        let visible = || self.imports.iter().filter(|i| module.starts_with(&i.scope));
        let explicit = visible().filter(|i| i.alias.as_deref() == Some(first.as_str())).max_by_key(|i| i.scope.len());
        if let Some(import) = explicit {
            return vec![import.path.iter().chain(&written[1..]).cloned().collect()];
        }
        let mut out = vec![written.clone()];
        let local_fn = written.len() == 1 && self.fns.iter().any(|(scope, name)| name == first && module.starts_with(scope));
        if !local_fn {
            out.extend(visible().filter(|i| i.alias.is_none()).map(|i| i.path.iter().chain(&written).cloned().collect::<Vec<_>>()));
        }
        out
    }

    /// The consts called `name` that have a literal value, with their indices.
    fn named<'s>(&'s self, name: &'s str) -> impl Iterator<Item = (usize, &'s ConstDef)> + 's {
        self.defs.iter().enumerate().filter(move |(_, d)| d.name == name && d.value.is_some())
    }

    /// The constants a path expression can name when written in `scope`.
    ///
    /// A bare name is looked up in the enclosing scopes, innermost first, then in every module-level const of that name
    /// (it may have been imported). `Self::X`, `self::X`, `super::X`, `crate::X` and `Type::X` / `module::X` (the last
    /// two segments) are looked up in the scope they name. Only consts with a literal value are candidates.
    fn candidates(&self, p: &ExprPath, scope: &[Frame]) -> Vec<usize> {
        let Some(idents) = path_idents(p) else { return Vec::new() };
        match idents.as_slice() {
            [name] => {
                let visible: Vec<(usize, &ConstDef)> = self.named(name).filter(|(_, d)| scope.starts_with(&d.scope)).collect();
                if let Some(depth) = visible.iter().map(|(_, d)| d.scope.len()).max() {
                    return visible.into_iter().filter(|(_, d)| d.scope.len() == depth).map(|(i, _)| i).collect();
                }
                self.named(name).filter(|(_, d)| !d.scope.iter().any(|f| matches!(f, Frame::Type(_)))).map(|(i, _)| i).collect()
            }
            [.., qualifier, name] => {
                let wanted: Option<Vec<Frame>> = match qualifier.as_str() {
                    "Self" => scope.iter().rposition(|f| matches!(f, Frame::Type(_))).map(|i| scope[..=i].to_vec()),
                    "self" => Some(module_scope(scope).to_vec()),
                    "super" => {
                        let module = module_scope(scope);
                        Some(module[..module.len().saturating_sub(1)].to_vec())
                    }
                    "crate" => Some(Vec::new()),
                    _ => None,
                };
                match wanted {
                    Some(wanted) => self.named(name).filter(|(_, d)| d.scope == wanted).map(|(i, _)| i).collect(),
                    None if qualifier == "Self" => Vec::new(),
                    None => self.named(name).filter(|(_, d)| d.scope.last().is_some_and(|f| f.name() == qualifier)).map(|(i, _)| i).collect(),
                }
            }
            [] => Vec::new(),
        }
    }

    /// The value of an expression made of numeric literals, consts, negation, casts, parentheses and arithmetic, written in `scope`.
    ///
    /// A name with several candidate consts takes the first non-zero value, so it is flagged when any candidate is non-zero.
    fn eval_in(&self, e: &Expr, scope: &[Frame]) -> Option<f64> {
        match e {
            Expr::Lit(l) => match &l.lit {
                Lit::Float(f) => f.base10_parse().ok(),
                Lit::Int(i) => i.base10_parse().ok(),
                _ => None,
            },
            Expr::Unary(u) if matches!(u.op, UnOp::Neg(_)) => self.eval_in(&u.expr, scope).map(|v| -v),
            Expr::Paren(p) => self.eval_in(&p.expr, scope),
            Expr::Group(g) => self.eval_in(&g.expr, scope),
            Expr::Cast(c) => self.eval_in(&c.expr, scope),
            Expr::Binary(b) => {
                let (l, r) = (self.eval_in(&b.left, scope)?, self.eval_in(&b.right, scope)?);
                match b.op {
                    BinOp::Add(_) => Some(l + r),
                    BinOp::Sub(_) => Some(l - r),
                    BinOp::Mul(_) => Some(l * r),
                    BinOp::Div(_) => Some(l / r),
                    BinOp::Rem(_) => Some(l % r),
                    _ => None,
                }
            }
            Expr::Path(p) => {
                let values: Vec<f64> = self.candidates(p, scope).into_iter().filter_map(|i| self.defs[i].value).collect();
                values.iter().copied().find(|v| *v != 0.0).or_else(|| values.first().copied())
            }
            _ => None,
        }
    }

    fn eval(&self, e: &Expr) -> Option<f64> {
        self.eval_in(e, &self.scope)
    }

    /// The const a path argument names, as written, and the line it is defined on, when its value is non-zero.
    fn const_name(&self, e: &Expr) -> Option<(String, usize)> {
        let Expr::Path(p) = peel(e) else { return None };
        let def = self.candidates(p, &self.scope).into_iter().map(|i| &self.defs[i]).find(|d| d.value.is_some_and(|v| v != 0.0))?;
        let idents = path_idents(p)?;
        let shown = idents[idents.len().saturating_sub(2)..].join("::");
        Some((shown, def.line))
    }

    fn is_flagged_size(&self, e: &Expr) -> bool {
        self.eval(e).is_some_and(|v| v != 0.0)
    }

    /// Reports `what` when any of `args` is a non-zero literal size.
    fn size_rule<'e>(&mut self, rule: &'static str, what: proc_macro2::Span, args: impl IntoIterator<Item = &'e Expr>) {
        let flagged: Vec<&Expr> = args.into_iter().filter(|a| self.is_flagged_size(a)).collect();
        if flagged.is_empty() {
            return;
        }
        let snippet = self.snippet(what);
        let named: Vec<String> = flagged.iter().filter_map(|a| self.const_name(a)).map(|(n, l)| format!("`{n}` (const, line {l})")).collect();
        let uses = if named.is_empty() { "literal sizes".to_owned() } else { format!("{}, a literal size", named.join(", ")) };
        let message = format!("`{snippet}` uses {uses}; {}", hint(rule));
        self.report(rule, what, message);
    }

    /// Reports each `(width, colour)` tuple among `args` whose width is a non-zero literal size.
    fn stroke_tuples<'e>(&mut self, args: impl IntoIterator<Item = &'e Expr>) {
        for arg in args {
            let Expr::Tuple(t) = peel(arg) else { continue };
            let mut elems = t.elems.iter();
            let (Some(width), Some(colour), None) = (elems.next(), elems.next(), elems.next()) else { continue };
            if self.eval(colour).is_none() {
                self.size_rule(STROKE, t.span(), [width]);
            }
        }
    }

    fn report(&mut self, rule: &'static str, span: proc_macro2::Span, message: String) {
        let (start, end) = (span.start(), span.end());
        self.out.findings.push(Finding::new(rule, start.line, start.column + 1, end.line, message));
    }

    /// The source text of a span; the first line and an ellipsis when it spans several lines.
    fn snippet(&self, span: proc_macro2::Span) -> String {
        let (start, end) = (span.start(), span.end());
        let Some(line) = self.lines.get(start.line.wrapping_sub(1)) else { return String::new() };
        let chars: Vec<char> = line.chars().collect();
        if start.line == end.line {
            chars.get(start.column..end.column.min(chars.len())).map(|s| s.iter().collect()).unwrap_or_default()
        } else {
            let first: String = chars.get(start.column..).map(|s| s.iter().collect()).unwrap_or_default();
            format!("{} …", first.trim_end())
        }
    }

    fn check_call(&mut self, call: &ExprCall) {
        let Expr::Path(func) = &*call.func else { return };
        let candidates = self.expand_path(&func.path);
        if candidates.iter().any(|c| c.last().is_some_and(|f| STROKE_TAKERS.contains(&f.as_str()))) {
            self.stroke_tuples(call.args.iter());
        }
        for idents in &candidates {
            if self.apply_call(call, idents) {
                return;
            }
        }
    }

    /// Applies the rule for a call to the function at `idents`; whether one matched.
    fn apply_call(&mut self, call: &ExprCall, idents: &[String]) -> bool {
        let span = call.span();
        let args = call.args.iter();
        if idents.last().map(String::as_str) == Some("vec2") {
            self.size_rule(VEC2, span, args);
            return true;
        }
        let [.., ty, f] = idents else { return false };
        match (ty.as_str(), f.as_str()) {
            ("Vec2", "new" | "splat") => self.size_rule(VEC2, span, args),
            ("CornerRadius", "same" | "from") | ("theme", "corner_radius") => self.size_rule(CORNER_RADIUS, span, args),
            ("Shape" | "RectShape", "rect_filled" | "rect_stroke" | "new" | "filled" | "stroke") => {
                self.size_rule(CORNER_RADIUS, span, args.skip(1).take(1));
            }
            ("Stroke" | "PathStroke", "new") | ("theme", "stroke") => self.size_rule(STROKE, span, args.take(1)),
            ("Stroke" | "PathStroke", "from") => self.stroke_tuples(args),
            ("Margin", "same" | "symmetric" | "from") | ("theme", "margin" | "margin_x") => self.size_rule(MARGIN, span, args),
            ("FontId", "proportional" | "monospace" | "new") => self.size_rule(FONT_ID, span, args.take(1)),
            ("Tooltip", "for_enabled" | "for_disabled" | "for_widget" | "always_open") => {
                let message = format!("`{ty}::{f}` builds a stock egui tooltip; {}", hint(TOOLTIP));
                self.report(TOOLTIP, span, message);
            }
            _ => return false,
        }
        true
    }

    fn check_method(&mut self, call: &ExprMethodCall) {
        let name = call.method.to_string();
        let end = call.paren_token.span.close();
        let span = call.method.span().join(end).unwrap_or_else(|| call.method.span());
        if STROKE_TAKERS.contains(&name.as_str()) {
            self.stroke_tuples(call.args.iter());
        }
        match name.as_str() {
            "add_space" => self.size_rule(ADD_SPACE, span, call.args.iter()),
            "corner_radius" => self.size_rule(CORNER_RADIUS, span, call.args.iter()),
            "rect" | "rect_filled" | "rect_stroke" => self.size_rule(CORNER_RADIUS, span, call.args.iter().skip(1).take(1)),
            "inner_margin" | "outer_margin" => self.size_rule(MARGIN, span, call.args.iter()),
            "size" if rooted_in_rich_text(&call.receiver) => self.size_rule(FONT_ID, span, call.args.iter()),
            "into" if call.args.is_empty() => self.stroke_tuples([&*call.receiver]),
            n if STOCK_TOOLTIPS.iter().any(|(stock, _)| *stock == n) => {
                let replacement = STOCK_TOOLTIPS.iter().find(|(stock, _)| *stock == n).map_or("craft_ui::contract::Props::tooltip", |(_, r)| r);
                self.report(TOOLTIP, span, format!("`{n}` is a stock egui tooltip; use `{replacement}` or mark `// design: allow(<reason>)`"));
            }
            n if STYLE_MUTATORS.contains(&n) || n.starts_with("override_") => {
                let snippet = self.snippet(span);
                let message = if n == "set_zoom_factor" {
                    format!("`{snippet}` changes the interface scale from app code; set it with `InterfaceScale::apply` (craft_ui::interface_scale)")
                } else {
                    format!("`{snippet}` changes the style from app code; {}", hint(STYLE_MUTATION))
                };
                self.report(STYLE_MUTATION, span, message);
            }
            _ => {}
        }
    }

    fn check_struct(&mut self, s: &ExprStruct) {
        let (rule, fields): (&'static str, &[&str]) = match self
            .expand_path(&s.path)
            .iter()
            .filter_map(|c| c.last())
            .map(String::as_str)
            .find(|t| matches!(*t, "Vec2" | "CornerRadius" | "Stroke" | "PathStroke" | "Margin"))
        {
            Some("Vec2") => (VEC2, &["x", "y"]),
            Some("CornerRadius") => (CORNER_RADIUS, &["nw", "ne", "sw", "se"]),
            Some("Stroke" | "PathStroke") => (STROKE, &["width"]),
            Some("Margin") => (MARGIN, &["left", "right", "top", "bottom"]),
            _ => return,
        };
        let values =
            s.fields.iter().filter(|f| matches!(&f.member, syn::Member::Named(n) if fields.contains(&n.to_string().as_str()))).map(|f| &f.expr);
        self.size_rule(rule, s.span(), values);
    }

    fn check_path(&mut self, p: &ExprPath) {
        let named = self.expand_path(&p.path).iter().any(|c| match c.as_slice() {
            [.., ty, name] if ty == "Color32" => NAMED_COLOURS.contains(&name.as_str()),
            [.., ty, name] if ty == "Rgba" => RGBA_COLOURS.contains(&name.as_str()),
            _ => false,
        });
        if named {
            let snippet = self.snippet(p.span());
            let message = format!(
                "`{snippet}`; take the colour from a semantic token (`Tokens::of(ui).color.text.normal.to_egui()`) or mark `// design: allow(<reason>)`"
            );
            self.report(RAW_COLOUR_CONST, p.span(), message);
        }
    }
}

fn hint(rule: &str) -> &'static str {
    match rule {
        VEC2 => "use a size token (`Tokens::of(ui).size.control.md`, `space.gap.inline`) or mark `// design: allow(<reason>)`",
        ADD_SPACE => "use a spacing token (`Tokens::of(ui).space.gap.inline`) or mark `// design: allow(<reason>)`",
        CORNER_RADIUS => "use a radius token from `Tokens::of(ui)` or mark `// design: allow(<reason>)`",
        STROKE => "use a stroke width token from `Tokens::of(ui)` or mark `// design: allow(<reason>)`",
        MARGIN => "use a spacing token (`Tokens::of(ui).space`) or mark `// design: allow(<reason>)`",
        TOOLTIP => "use `craft_ui::contract::Props::tooltip` or `craft_ui::contract::Show::show_with_tip` or mark `// design: allow(<reason>)`",
        STYLE_MUTATION => "set styles through the theme (craft-ui installs them from the tokens) or mark `// design: allow(<reason>)`",
        _ => "use a type-size token from `Tokens::of(ui)` or mark `// design: allow(<reason>)`",
    }
}

impl<'ast> Visit<'ast> for Linter<'_> {
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        if !self.skips(&i.attrs, i.span()) {
            self.scope.push(Frame::Module(i.ident.to_string()));
            visit::visit_item_mod(self, i);
            self.scope.pop();
        }
    }
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        if !self.skips(&i.attrs, i.span()) {
            visit::visit_item_fn(self, i);
        }
    }
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if !self.skips(&i.attrs, i.span()) {
            self.scope.push(Frame::Type(type_name(&i.self_ty)));
            visit::visit_item_impl(self, i);
            self.scope.pop();
        }
    }
    fn visit_item_trait(&mut self, i: &'ast syn::ItemTrait) {
        self.scope.push(Frame::Type(i.ident.to_string()));
        visit::visit_item_trait(self, i);
        self.scope.pop();
    }
    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        if !self.skips(&i.attrs, i.span()) {
            visit::visit_impl_item_fn(self, i);
        }
    }
    fn visit_trait_item_fn(&mut self, i: &'ast syn::TraitItemFn) {
        if !self.skips(&i.attrs, i.span()) {
            visit::visit_trait_item_fn(self, i);
        }
    }
    fn visit_item_const(&mut self, i: &'ast syn::ItemConst) {
        if !self.skips(&i.attrs, i.span()) {
            visit::visit_item_const(self, i);
        }
    }
    fn visit_item_static(&mut self, i: &'ast syn::ItemStatic) {
        if !self.skips(&i.attrs, i.span()) {
            visit::visit_item_static(self, i);
        }
    }

    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        if !is_disallowed_clippy_allow(attr) {
            return;
        }
        self.out.clippy_allows += 1;
        if matches!(attr.style, AttrStyle::Inner(_)) {
            let message = format!(
                "an inner `allow` or `expect` of a `clippy::disallowed_*` lint silences the design-system clippy rules for its whole scope; allow the single item instead, or list the file in an `[[exempt]]` for `{INNER_ALLOW}`"
            );
            self.report(INNER_ALLOW, attr.span(), message);
        }
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        self.check_call(call);
        visit::visit_expr_call(self, call);
    }
    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        self.check_method(call);
        visit::visit_expr_method_call(self, call);
    }
    fn visit_expr_struct(&mut self, s: &'ast ExprStruct) {
        self.check_struct(s);
        visit::visit_expr_struct(self, s);
    }
    fn visit_expr_path(&mut self, p: &'ast ExprPath) {
        self.check_path(p);
        visit::visit_expr_path(self, p);
    }

    /// Macro arguments are checked when they parse as comma-separated expressions (or `expr; expr`).
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        if let Ok(args) = m.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
            for arg in &args {
                self.visit_expr(arg);
            }
        } else if let Ok((a, b)) = m.parse_body_with(|input: syn::parse::ParseStream| {
            let a: Expr = input.parse()?;
            input.parse::<Token![;]>()?;
            Ok((a, input.parse::<Expr>()?))
        }) {
            self.visit_expr(&a);
            self.visit_expr(&b);
        }
    }
}
