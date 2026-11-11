//! `// design: allow(<reason>)` markers: finding them in the source text and applying them to findings.
//!
//! A marker may name the rules it covers: `// design: allow[literal-size/stroke, raw-colour](<reason>)`.

use std::ops::RangeInclusive;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens as _;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::report::{Finding, Severity};
use crate::rules::{RULES, STYLE_MUTATION, analyze_file, is_unmarkable, parse_failure, selector_matches};

pub const EMPTY: &str = "marker/empty";
pub const INVALID: &str = "marker/invalid";
pub const UNUSED: &str = "marker/unused";

/// Shortest accepted marker or exemption reason, in characters.
pub const MIN_REASON: usize = 3;

const PREFIX: &str = "design: allow";

/// The findings of one file after markers were applied.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Marked {
    /// Findings not covered by a marker, plus `marker/*` findings.
    pub findings: Vec<Finding>,
    /// Markers found in the file.
    pub markers: usize,
    /// `allow` and `expect` attributes naming a `clippy::disallowed_*` lint.
    pub clippy_allows: usize,
}

#[derive(Debug)]
enum Scope {
    /// `//! design: allow(..)`: every finding in the file.
    File,
    /// End-of-line marker: findings starting on `line`.
    Line,
    /// Alone on its line: findings starting in the next code line's statement or field.
    Next,
}

#[derive(Debug)]
struct Marker {
    line: usize,
    column: usize,
    scope: Scope,
    /// Rule ids or families the marker covers; empty means every rule.
    rules: Vec<String>,
    /// The rule and message of what is wrong with the marker, when something is.
    problem: Option<(&'static str, String)>,
}

impl Marker {
    fn covers_rule(&self, rule: &str) -> bool {
        self.rules.is_empty() || self.rules.iter().any(|r| selector_matches(r, rule))
    }
}

/// Checks one file's source text and applies its markers. The file is parsed once.
///
/// `exempt(rule)` tells whether a path exemption already silences a rule in this file;
/// those findings are dropped before markers are matched, so a marker that only covers them is unused.
/// `parse` findings are never exempt, and no marker covers them.
pub fn analyze_with_markers(src: &str, include_tests: bool, deny_unused: bool, exempt: &dyn Fn(&str) -> bool) -> Marked {
    analyze_with_extra(src, syn::parse_file(src).as_ref(), include_tests, deny_unused, exempt, &|_| Vec::new(), FilePlace::default())
}

/// Where a file sits, for the older rules that the scoped rules replace in some places.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FilePlace {
    /// The file may change the style and the interface scale (`style-mutation`).
    pub style_writer: bool,
    /// `test/literal-size` covers test code, so `literal-size/*` skips `#[cfg(test)]` items and, when `test_file` (a file under
    /// `tests/` or a `testing` module directory below `src/`), the whole file.
    pub tests_scoped: bool,
    pub test_file: bool,
}

/// [`analyze_with_markers`] with `extra` findings (the scoped `contract/*` rules) merged in before exemptions and markers apply.
/// `parsed` is the syntax tree of `src`.
pub(crate) fn analyze_with_extra(
    src: &str,
    parsed: Result<&syn::File, &syn::Error>,
    include_tests: bool,
    deny_unused: bool,
    exempt: &dyn Fn(&str) -> bool,
    extra: &dyn Fn(&syn::File) -> Vec<Finding>,
    place: FilePlace,
) -> Marked {
    let file = match parsed {
        Ok(file) => file,
        Err(e) => return Marked { findings: parse_failure(e).findings, markers: 0, clippy_allows: 0 },
    };
    let analysis = analyze_file(src, file, include_tests);
    let in_tests = |line: usize| place.tests_scoped && (place.test_file || analysis.test_code.iter().any(|r| r.contains(&line)));
    let older = analysis
        .findings
        .iter()
        .filter(|f| !(f.rule == STYLE_MUTATION && place.style_writer) && !(f.rule.starts_with("literal-size/") && in_tests(f.line)));
    let mut findings: Vec<Finding> = older.cloned().chain(extra(file)).filter(|f| !exempt(f.rule)).collect();
    let lines: Vec<&str> = src.lines().collect();
    let markers = find_markers(&lines, file);
    let mut spans: Option<Vec<RangeInclusive<usize>>> = None;
    let mut used = vec![false; markers.len()];
    let mut kept = Vec::new();
    // A finding is covered when it starts inside the range of a marker that covers its rule.
    let covering: Vec<Option<RangeInclusive<usize>>> = markers
        .iter()
        .map(|m| match m.scope {
            _ if m.problem.is_some() => None,
            Scope::File => Some(0..=usize::MAX),
            Scope::Line => Some(m.line..=m.line),
            Scope::Next => next_code_line(&lines, m.line).map(|line| {
                let item_spans = spans.get_or_insert_with(|| item_spans(file));
                let end = item_spans.iter().filter(|r| *r.start() == line).map(|r| *r.end()).max().unwrap_or(line);
                line..=end
            }),
        })
        .collect();
    for finding in findings.drain(..) {
        let hit = (!is_unmarkable(finding.rule))
            .then(|| {
                covering.iter().zip(&markers).position(|(c, m)| m.covers_rule(finding.rule) && c.as_ref().is_some_and(|r| r.contains(&finding.line)))
            })
            .flatten();
        match hit {
            Some(i) => used[i] = true,
            None => kept.push(finding),
        }
    }
    let severity = if deny_unused { Severity::Error } else { Severity::Warning };
    for (i, marker) in markers.iter().enumerate() {
        let (rule, message) = match &marker.problem {
            Some((rule, problem)) => (*rule, problem.clone()),
            None if used[i] || analysis.skipped.iter().any(|r| r.contains(&marker.line)) => continue,
            None => (UNUSED, "marker suppresses nothing; remove it".to_owned()),
        };
        if exempt(rule) {
            continue;
        }
        let mut finding = Finding::new(rule, marker.line, marker.column, marker.line, message);
        if rule == UNUSED {
            finding.severity = severity;
        }
        kept.push(finding);
    }
    kept.sort_by_key(|f| (f.line, f.column));
    Marked { findings: kept, markers: markers.len(), clippy_allows: analysis.clippy_allows }
}

/// The first line after `line` that holds code: not blank and not a comment.
fn next_code_line(lines: &[&str], line: usize) -> Option<usize> {
    (line + 1..=lines.len()).find(|&n| {
        let text = lines[n - 1].trim_start();
        !text.is_empty() && !text.starts_with("//")
    })
}

fn open_line(brace: &syn::token::Brace) -> usize {
    brace.span.open().start().line
}

/// The last line of a control-flow or block expression's header: up to its opening brace.
fn expr_header_end(e: &syn::Expr) -> Option<usize> {
    use syn::Expr;
    Some(match e {
        Expr::If(i) => open_line(&i.then_branch.brace_token),
        Expr::ForLoop(f) => open_line(&f.body.brace_token),
        Expr::While(w) => open_line(&w.body.brace_token),
        Expr::Loop(l) => open_line(&l.body.brace_token),
        Expr::Match(m) => open_line(&m.brace_token),
        Expr::Block(b) => open_line(&b.block.brace_token),
        Expr::Unsafe(u) => open_line(&u.block.brace_token),
        Expr::Async(a) => open_line(&a.block.brace_token),
        _ => return None,
    })
}

/// The last line of an item's header when it has a body of statements: up to the end of a fn signature or the opening brace.
fn item_header_end(i: &syn::Item) -> Option<usize> {
    use syn::Item;
    match i {
        Item::Fn(f) => Some(f.sig.span().end().line),
        Item::Impl(i) => Some(open_line(&i.brace_token)),
        Item::Trait(t) => Some(open_line(&t.brace_token)),
        Item::Mod(m) => m.content.as_ref().map(|(brace, _)| open_line(brace)),
        _ => None,
    }
}

/// Line ranges of every statement, item, match arm and field initialiser in the file.
///
/// A fn, impl, trait, module or control-flow statement counts as its header only; the statements in its body have their own ranges.
fn item_spans(file: &syn::File) -> Vec<RangeInclusive<usize>> {
    struct Spans(Vec<RangeInclusive<usize>>);
    impl Spans {
        fn add(&mut self, span: proc_macro2::Span, header_end: Option<usize>) {
            let end = header_end.unwrap_or_else(|| span.end().line);
            self.0.push(span.start().line..=end);
        }
    }
    impl<'ast> Visit<'ast> for Spans {
        fn visit_stmt(&mut self, i: &'ast syn::Stmt) {
            let header = match i {
                syn::Stmt::Expr(e, _) => expr_header_end(e),
                syn::Stmt::Item(item) => item_header_end(item),
                _ => None,
            };
            self.add(i.span(), header);
            visit::visit_stmt(self, i);
        }
        fn visit_item(&mut self, i: &'ast syn::Item) {
            self.add(i.span(), item_header_end(i));
            visit::visit_item(self, i);
        }
        fn visit_impl_item(&mut self, i: &'ast syn::ImplItem) {
            let header = match i {
                syn::ImplItem::Fn(f) => Some(f.sig.span().end().line),
                _ => None,
            };
            self.add(i.span(), header);
            visit::visit_impl_item(self, i);
        }
        fn visit_trait_item(&mut self, i: &'ast syn::TraitItem) {
            let header = match i {
                syn::TraitItem::Fn(f) if f.default.is_some() => Some(f.sig.span().end().line),
                _ => None,
            };
            self.add(i.span(), header);
            visit::visit_trait_item(self, i);
        }
        fn visit_arm(&mut self, i: &'ast syn::Arm) {
            self.add(i.span(), expr_header_end(&i.body));
            visit::visit_arm(self, i);
        }
        fn visit_field_value(&mut self, i: &'ast syn::FieldValue) {
            self.add(i.span(), None);
            visit::visit_field_value(self, i);
        }
    }
    let mut spans = Spans(Vec::new());
    spans.visit_file(file);
    spans.0
}

type LiteralSpan = ((usize, usize), (usize, usize));

/// `((line, column), (line, column))` of each string, char and byte literal token, columns in characters.
fn literal_spans(file: &syn::File, lines: &[&str]) -> Vec<LiteralSpan> {
    fn walk(stream: TokenStream, lines: &[&str], out: &mut Vec<LiteralSpan>) {
        for tree in stream {
            match tree {
                TokenTree::Group(g) => walk(g.stream(), lines, out),
                TokenTree::Literal(l) => {
                    let (s, e) = (l.span().start(), l.span().end());
                    // Doc comments lex to a literal spanning the comment itself.
                    let text: String = lines.get(s.line.wrapping_sub(1)).map(|t| t.chars().skip(s.column).take(2).collect()).unwrap_or_default();
                    if text != "//" && text != "/*" {
                        out.push(((s.line, s.column), (e.line, e.column)));
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(file.to_token_stream(), lines, &mut out);
    out
}

/// What follows `design: allow` in a comment: the rule list and the reason, or why they are malformed.
type Parsed = (Vec<String>, Option<(&'static str, String)>);

fn parse_marker(rest: &str) -> Option<Parsed> {
    let (list, after) = match rest.strip_prefix('[') {
        Some(inner) => {
            let Some(end) = inner.find(']') else {
                return Some((Vec::new(), Some((INVALID, "marker rule list is missing the closing `]`".to_owned()))));
            };
            (Some(&inner[..end]), &inner[end + 1..])
        }
        None => (None, rest),
    };
    let Some(reason) = after.strip_prefix('(') else {
        return list.is_some().then(|| (Vec::new(), Some((EMPTY, "marker needs a reason: `// design: allow[<rules>](<reason>)`".to_owned()))));
    };
    let rules: Vec<String> = list.map(|l| l.split(',').map(|r| r.trim().to_owned()).collect()).unwrap_or_default();
    let problem = if let Some(unknown) = rules.iter().find(|r| r.is_empty() || !RULES.iter().any(|known| selector_matches(r, known))) {
        Some((INVALID, format!("marker names unknown rule `{unknown}`")))
    } else {
        match reason.rfind(')') {
            None => Some((EMPTY, "marker is missing the closing `)`".to_owned())),
            Some(end) if reason[..end].trim().chars().count() < MIN_REASON => {
                Some((EMPTY, "marker needs a reason of at least three characters: `// design: allow(<reason>)`".to_owned()))
            }
            Some(_) => None,
        }
    };
    Some((rules, problem))
}

fn find_markers(lines: &[&str], file: &syn::File) -> Vec<Marker> {
    let literals = literal_spans(file, lines);
    let mut out = Vec::new();
    for (idx, text) in lines.iter().enumerate() {
        let line = idx + 1;
        let in_literal = |column: usize| literals.iter().any(|(s, e)| (s.0, s.1) <= (line, column) && (line, column) < (e.0, e.1));
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let Some(column) = (0..chars.len().saturating_sub(1)).find(|&c| chars[c].1 == '/' && chars[c + 1].1 == '/' && !in_literal(c)) else {
            continue;
        };
        let after = &text[chars[column].0 + 2..];
        let (file_level, body) = match after.strip_prefix('!') {
            Some(rest) => (true, rest),
            None if after.starts_with('/') => continue,
            None => (false, after),
        };
        let Some(rest) = body.trim().strip_prefix(PREFIX) else { continue };
        let Some((rules, problem)) = parse_marker(rest) else { continue };
        let alone = text[..chars[column].0].trim().is_empty();
        let scope = match (file_level, alone) {
            (true, _) => Scope::File,
            (false, true) => Scope::Next,
            (false, false) => Scope::Line,
        };
        out.push(Marker { line, column: column + 1, scope, rules, problem });
    }
    out
}
