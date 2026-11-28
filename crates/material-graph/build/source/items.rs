//! Source files as items: functions, types, constants and macros, each with its tokens and what it names. Whatever
//! isn't recognised as an item ends up in [`Item::loose`] and counts for every result, so that not understanding some
//! syntax can only invalidate too much.

use super::tokens::{Token, tokenize};
use std::path::Path;

/// One thing an item names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mention {
    /// `name`, `.name`
    Name(String),
    /// `Owner::name`
    Path(String, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Function,
    /// A struct, enum, union, trait or type alias.
    Type,
    /// A constant, static or macro.
    Value,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub name: String,
    pub kind: ItemKind,
    /// The type an `impl` or `trait` block gives it.
    pub owner: Option<String>,
    /// Inside `impl Trait for Owner`: operators, `Drop`, `Default` and the like run without being named.
    pub trait_impl: bool,
    /// What the digest is taken over.
    pub tokens: Vec<Token>,
    pub mentions: Vec<Mention>,
    /// Counts for every result.
    pub loose: bool,
    /// A file `include_bytes!` and the like pull in, relative to the source file.
    pub includes: Vec<String>,
    /// The file it is in, for telling a developer what a version covers.
    pub file: String,
}

/// `use a::b as c`: `c` stands for `b`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    pub alias: String,
    pub target: String,
}

/// A match arm naming a node kind, cut out of a [`Dispatcher`]'s function.
#[derive(Debug, Clone)]
pub struct Arm {
    pub kinds: Vec<String>,
    /// How many arms that aren't literal-only (a guard, a binding) come before it in the match. The first arm that
    /// matches wins, so moving a literal arm past such an arm changes what runs.
    pub after: usize,
    pub tokens: Vec<Token>,
    pub mentions: Vec<Mention>,
}

/// A function whose outermost `match` picks by node kind. Each literal arm counts only for the kinds it names.
#[derive(Debug, Clone, Copy)]
pub struct Dispatcher<'a> {
    pub file: &'a str,
    pub function: &'a str,
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub items: Vec<Item>,
    pub aliases: Vec<Alias>,
    pub arms: Vec<Arm>,
}

const NOT_NAMES: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
    "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while",
    "async", "await", "dyn",
];

/// What `tokens` name. `owner` stands for `Self`.
pub fn mentions_in(tokens: &[Token], owner: Option<&str>) -> Vec<Mention> {
    let mut out = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        let Some(name) = t.ident() else { continue };
        let double_colon_at = |k: usize| tokens.get(k).is_some_and(|c| c.is_punct(':')) && tokens.get(k + 1).is_some_and(|c| c.is_punct(':'));
        // `Owner::name`, with the owner possibly `Self`
        if i >= 3 && double_colon_at(i - 2) {
            let before = tokens.get(i - 3).and_then(Token::ident);
            let before = if before == Some("Self") { owner } else { before };
            if let Some(path_owner) = before.filter(|b| !NOT_NAMES.contains(b)) {
                if !NOT_NAMES.contains(&name) {
                    out.push(Mention::Path(path_owner.to_string(), name.to_string()));
                }
                continue;
            }
        }
        if !NOT_NAMES.contains(&name) {
            out.push(Mention::Name(name.to_string()));
        }
    }
    if tokens.iter().any(|t| t.is("Self")) {
        out.extend(owner.map(|o| Mention::Name(o.to_string())));
    }
    out.sort();
    out.dedup();
    out
}

/// The files `include_bytes!("x")`, `include_str!`, `include!` in `tokens` name.
pub fn includes_in(tokens: &[Token]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if t.is("include_bytes") || t.is("include_str") || t.is("include") {
            let text = tokens.iter().skip(i + 1).take(3).find_map(|x| match x {
                Token::Text(s) | Token::Raw(s) => Some(s.clone()),
                _ => None,
            });
            out.extend(text);
        }
    }
    out
}

fn opens(t: &Token) -> bool {
    t.is_punct('{') || t.is_punct('(') || t.is_punct('[')
}

fn closes(t: &Token) -> bool {
    t.is_punct('}') || t.is_punct(')') || t.is_punct(']')
}

/// The index just past the bracket closing the one at `open`.
fn past_group(tokens: &[Token], open: usize) -> usize {
    let mut depth = 0usize;
    for (i, t) in tokens.iter().enumerate().skip(open) {
        if opens(t) {
            depth += 1;
        } else if closes(t) {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return i + 1;
            }
        }
    }
    tokens.len()
}

/// The index just past the item starting at `from`, which ends with `;` or, when `braces` allows, a `{…}` outside any
/// other bracket.
pub(super) fn past_item(tokens: &[Token], from: usize, braces: bool) -> usize {
    let mut i = from;
    while let Some(t) = tokens.get(i) {
        if t.is_punct(';') {
            return i + 1;
        }
        if t.is_punct('{') && braces {
            return past_group(tokens, i);
        }
        i = if opens(t) { past_group(tokens, i) } else { i + 1 };
    }
    tokens.len()
}

struct Parser<'a> {
    tokens: &'a [Token],
    file: &'a str,
    out: Parsed,
    /// Tokens no item took.
    loose: Vec<Token>,
    /// The attributes and name of the enclosing inline modules that have attributes: `#[cfg(unix)] mod backend`
    /// decides whether what is inside exists, so it goes into the digest of each item inside.
    scope: Vec<Token>,
}

impl Parser<'_> {
    fn add(&mut self, from: usize, to: usize, name: &str, kind: ItemKind, owner: Option<&str>, trait_impl: bool) {
        let tokens = self.tokens.get(from..to).unwrap_or(&[]);
        let includes = includes_in(tokens);
        let tokens = self.scoped(tokens);
        self.out.items.push(Item {
            name: name.to_string(),
            kind,
            owner: owner.map(str::to_string),
            trait_impl,
            mentions: mentions_in(&tokens, owner),
            tokens,
            loose: false,
            includes,
            file: self.file.to_string(),
        });
    }

    /// `tokens` after the attributes of the modules around them.
    fn scoped(&self, tokens: &[Token]) -> Vec<Token> {
        let mut out = self.scope.clone();
        out.extend_from_slice(tokens);
        out
    }

    /// Whether the item starting at `start` has attributes.
    fn has_attributes(&self, start: usize) -> bool {
        self.tokens.get(start).is_some_and(|t| t.is_punct('#'))
    }

    /// `use` declarations: their `as` renames.
    fn aliases(&mut self, from: usize, to: usize) {
        let tokens = self.tokens.get(from..to).unwrap_or(&[]);
        for (i, t) in tokens.iter().enumerate() {
            if t.is("as") && i > 0 {
                let target = tokens.get(i - 1).and_then(Token::ident);
                let alias = tokens.get(i + 1).and_then(Token::ident);
                if let (Some(target), Some(alias)) = (target, alias) {
                    self.out.aliases.push(Alias { alias: alias.to_string(), target: target.to_string() });
                }
            }
        }
    }

    /// The `use` declaration at `at` (its attributes start at `start`); the index past it.
    fn use_declaration(&mut self, start: usize, at: usize) -> usize {
        let end = past_item(self.tokens, at, false);
        self.aliases(at, end);
        if self.has_attributes(start) {
            // `#[cfg(unix)] use a::shade;` decides which `shade` runs: it counts for every result
            let tokens = self.tokens;
            self.loose.extend_from_slice(tokens.get(start..end).unwrap_or(&[]));
        }
        end
    }

    /// The `mod` at `at` (its attributes start at `start`); the index past it. An inline module's items are parsed.
    fn module(&mut self, start: usize, at: usize) -> usize {
        let end = past_item(self.tokens, at, true);
        if end > 0 && self.tokens.get(end - 1).is_some_and(|c| c.is_punct('}')) {
            let outer = self.scope.len();
            if self.has_attributes(start) {
                let tokens = self.tokens;
                self.scope.extend_from_slice(tokens.get(start..at + 2).unwrap_or(&[]));
            }
            self.items(at + 3, end - 1, None, false);
            self.scope.truncate(outer);
        }
        end
    }

    /// The items of a module or `impl` body, from `from` to `to`.
    fn items(&mut self, from: usize, to: usize, owner: Option<&str>, trait_impl: bool) {
        let mut i = from;
        let mut attributes = from;
        while i < to {
            let Some(t) = self.tokens.get(i) else { return };
            // attributes stay with their item; `#[cfg(test)]` takes it out
            if t.is_punct('#') && self.tokens.get(i + 1).is_some_and(|n| n.is_punct('[')) {
                let end = past_group(self.tokens, i + 1);
                let test = self.tokens.get(i + 2).is_some_and(|n| n.is("cfg")) && self.tokens.get(i + 4).is_some_and(|n| n.is("test"));
                if test {
                    i = past_item(self.tokens, end, true);
                    attributes = i;
                } else {
                    i = end;
                }
                continue;
            }
            if t.is("pub") {
                i += 1;
                if self.tokens.get(i).is_some_and(|n| n.is_punct('(')) {
                    i = past_group(self.tokens, i);
                }
                continue;
            }
            if t.is("unsafe") || t.is("async") || t.is("default") {
                i += 1;
                continue;
            }
            let next = self.tokens.get(i + 1);
            if t.is("extern") {
                // `extern "C" fn`, `extern crate x;`
                i += if matches!(next, Some(Token::Text(_))) { 2 } else { 1 };
                continue;
            }
            if t.is("const") && next.is_some_and(|n| n.is("fn") || n.is("unsafe") || n.is("extern")) {
                i += 1;
                continue;
            }
            let name = next.and_then(Token::ident).unwrap_or("").to_string();
            let start = attributes;
            let end = if t.is("fn") {
                let end = past_item(self.tokens, i, true);
                self.add(start, end, &name, ItemKind::Function, owner, trait_impl);
                end
            } else if t.is("struct") || t.is("enum") || t.is("union") {
                let end = past_item(self.tokens, i, true);
                self.add(start, end, &name, ItemKind::Type, owner, trait_impl);
                end
            } else if t.is("type") {
                let end = past_item(self.tokens, i, false);
                self.add(start, end, &name, ItemKind::Type, owner, trait_impl);
                end
            } else if t.is("const") || t.is("static") {
                let name =
                    if next.is_some_and(|n| n.is("mut")) { self.tokens.get(i + 2).and_then(Token::ident).unwrap_or("").to_string() } else { name };
                let end = past_item(self.tokens, i, false);
                self.add(start, end, &name, ItemKind::Value, owner, trait_impl);
                end
            } else if t.is("use") {
                self.use_declaration(start, i)
            } else if t.is("mod") {
                self.module(start, i)
            } else if t.is("trait") {
                let end = past_item(self.tokens, i, true);
                self.body_block(start, i, end, &name, ItemKind::Type, false);
                end
            } else if t.is("impl") {
                let end = past_item(self.tokens, i, true);
                let target = impl_target(self.tokens.get(i..end).unwrap_or(&[]));
                let first = self.out.items.len();
                self.body_block(start, i, end, &target.owner, ItemKind::Value, target.is_trait);
                if target.blanket {
                    self.out.items.iter_mut().skip(first).for_each(|item| item.loose = true);
                }
                end
            } else if t.is("macro_rules") {
                let name = self.tokens.get(i + 2).and_then(Token::ident).unwrap_or("").to_string();
                let end = past_item(self.tokens, i, true);
                self.add(start, end, &name, ItemKind::Value, owner, trait_impl);
                end
            } else if t.is("thread_local") {
                let open = i + 2;
                let end = past_group(self.tokens, open);
                self.items(open + 1, end.saturating_sub(1), owner, trait_impl);
                end + usize::from(self.tokens.get(end).is_some_and(|n| n.is_punct(';')))
            } else {
                self.loose.push(t.clone());
                i + 1
            };
            i = end;
            attributes = i;
        }
    }

    /// A `trait` or `impl` block: its header (and what isn't a function) is one item named `name`, its functions are
    /// items of their own.
    fn body_block(&mut self, start: usize, keyword: usize, end: usize, name: &str, kind: ItemKind, trait_impl: bool) {
        let Some(open) = (keyword..end).find(|&k| self.tokens.get(k).is_some_and(|t| t.is_punct('{'))) else { return };
        // the header's tokens with no body: a change of bounds or of the target counts
        let header = self.tokens.get(start..open).unwrap_or(&[]);
        let header = self.scoped(header);
        self.out.items.push(Item {
            name: name.to_string(),
            kind,
            owner: Some(name.to_string()),
            trait_impl,
            mentions: mentions_in(&header, Some(name)),
            tokens: header,
            loose: false,
            includes: Vec::new(),
            file: self.file.to_string(),
        });
        self.items(open + 1, end.saturating_sub(1), Some(name), trait_impl);
    }
}

struct ImplTarget {
    owner: String,
    is_trait: bool,
    /// `impl<T> X for T`: for every type, so for every result.
    blanket: bool,
}

/// The type an `impl` block's header is for: the last segment of the path after `for`, or else the first path.
fn impl_target(tokens: &[Token]) -> ImplTarget {
    let open = tokens.iter().position(|t| t.is_punct('{')).unwrap_or(tokens.len());
    let header = tokens.get(1..open).unwrap_or(&[]);
    // the generic parameters right after `impl`
    let mut params = Vec::new();
    let mut skip = 0;
    if header.first().is_some_and(|t| t.is_punct('<')) {
        let mut depth = 0i32;
        for (i, t) in header.iter().enumerate() {
            if t.is_punct('<') {
                depth += 1;
            } else if t.is_punct('>') && !header.get(i.wrapping_sub(1)).is_some_and(|p| p.is_punct('-')) {
                depth -= 1;
                if depth == 0 {
                    skip = i + 1;
                    break;
                }
            } else if depth == 1 && header.get(i.wrapping_sub(1)).is_some_and(|p| p.is_punct('<') || p.is_punct(',')) {
                params.extend(t.ident().map(str::to_string));
            }
        }
    }
    let rest = header.get(skip..).unwrap_or(&[]);
    let after_for = rest.iter().enumerate().find(|(i, t)| t.is("for") && !rest.get(i + 1).is_some_and(|n| n.is_punct('<'))).map(|(i, _)| i + 1);
    let target = rest.get(after_for.unwrap_or(0)..).unwrap_or(&[]);
    // the path's last segment before its generics
    let mut owner = String::new();
    for t in target {
        if t.is_punct('<') || t.is_punct('{') || t.is("where") {
            break;
        }
        if let Some(name) = t.ident().filter(|n| *n != "dyn" && *n != "mut") {
            owner = name.to_string();
        }
    }
    let blanket = params.contains(&owner);
    ImplTarget { owner, is_trait: after_for.is_some(), blanket }
}

/// One source file's items. `dispatchers` names functions to cut into arms.
pub fn parse(source: &str, file: &str, dispatchers: &[Dispatcher]) -> Parsed {
    let tokens = tokenize(source);
    let mut parser = Parser { tokens: &tokens, file, out: Parsed::default(), loose: Vec::new(), scope: Vec::new() };
    parser.items(0, tokens.len(), None, false);
    let Parser { out: mut parsed, loose, .. } = parser;
    if !loose.is_empty() {
        parsed.items.push(Item {
            name: String::new(),
            kind: ItemKind::Value,
            owner: None,
            trait_impl: false,
            mentions: mentions_in(&loose, None),
            tokens: loose,
            loose: true,
            includes: Vec::new(),
            file: file.to_string(),
        });
    }
    for d in dispatchers.iter().filter(|d| d.file == file || Path::new(file).ends_with(d.file)) {
        cut_arms(&mut parsed, d.function);
    }
    parsed
}

/// Cuts the literal arms out of function `function`'s outermost `match` over a kind: what remains of the function counts
/// for every kind, an arm for those it names.
fn cut_arms(parsed: &mut Parsed, function: &str) {
    let Some(item) = parsed.items.iter_mut().find(|i| i.name == function && i.kind == ItemKind::Function) else { return };
    let tokens = std::mem::take(&mut item.tokens);
    let Some(start) = dispatch_match(&tokens) else {
        item.tokens = tokens;
        return;
    };
    let end = past_group(&tokens, start);
    let mut kept: Vec<Token> = tokens.get(..=start).unwrap_or(&[]).to_vec();
    let mut i = start + 1;
    let last = end.saturating_sub(1);
    let mut after = 0;
    while i < last {
        let arm_end = past_arm(&tokens, i, last);
        let arm = tokens.get(i..arm_end).unwrap_or(&[]);
        match literal_kinds(arm) {
            Some(kinds) => parsed.arms.push(Arm { kinds, after, tokens: arm.to_vec(), mentions: mentions_in(arm, None) }),
            None => {
                after += 1;
                kept.extend_from_slice(arm);
            }
        }
        i = arm_end;
    }
    kept.extend_from_slice(tokens.get(last..).unwrap_or(&[]));
    item.mentions = mentions_in(&kept, item.owner.as_deref());
    item.tokens = kept;
}

/// The opening brace of the first `match` whose arms start with a string literal.
fn dispatch_match(tokens: &[Token]) -> Option<usize> {
    let mut i = 0;
    while let Some(t) = tokens.get(i) {
        if t.is("match") {
            let open = (i..tokens.len()).find(|&k| tokens.get(k).is_some_and(|x| x.is_punct('{')))?;
            if matches!(tokens.get(open + 1), Some(Token::Text(_))) {
                return Some(open);
            }
        }
        i += 1;
    }
    None
}

/// The index past the arm starting at `from`: its `=>`, then a block with an optional comma, or an expression up to a comma
/// outside brackets.
fn past_arm(tokens: &[Token], from: usize, last: usize) -> usize {
    let mut i = from;
    while i < last {
        let Some(t) = tokens.get(i) else { return last };
        if t.is_punct('=') && tokens.get(i + 1).is_some_and(|n| n.is_punct('>')) {
            i += 2;
            break;
        }
        i = if opens(t) { past_group(tokens, i) } else { i + 1 };
    }
    if tokens.get(i).is_some_and(|t| t.is_punct('{')) {
        let end = past_group(tokens, i);
        // a block that goes on as an expression (`{ … }.foo()`) runs up to its comma as well
        let ends_arm = end >= last || tokens.get(end).is_some_and(|t| t.is_punct(',') || !t.is_punct('.') && !t.is_punct('?'));
        if ends_arm {
            return (end + usize::from(tokens.get(end).is_some_and(|t| t.is_punct(',')))).min(last);
        }
        i = end;
    }
    while i < last {
        let Some(t) = tokens.get(i) else { return last };
        if t.is_punct(',') {
            return i + 1;
        }
        i = if opens(t) { past_group(tokens, i) } else { i + 1 };
    }
    last
}

/// The kinds of an arm whose pattern is only string literals (no guard).
fn literal_kinds(arm: &[Token]) -> Option<Vec<String>> {
    let mut kinds = Vec::new();
    for (i, t) in arm.iter().enumerate() {
        match t {
            Token::Text(s) => kinds.push(s.clone()),
            Token::Punct('|') => {}
            Token::Punct('=') if arm.get(i + 1).is_some_and(|n| n.is_punct('>')) => {
                return (!kinds.is_empty()).then_some(kinds);
            }
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(p: &Parsed) -> Vec<(&str, Option<&str>)> {
        p.items.iter().map(|i| (i.name.as_str(), i.owner.as_deref())).collect()
    }

    #[test]
    fn items_with_their_owners() {
        let p = parse(
            "pub fn a() -> u8 { b() } struct S { x: u8 } impl S { pub fn m(&self) {} const C: u8 = 1; } const K: [u8; 2] = [1, 2]; static mut V: u8 = 0;",
            "x.rs",
            &[],
        );
        assert_eq!(names(&p), [("a", None), ("S", None), ("S", Some("S")), ("m", Some("S")), ("C", Some("S")), ("K", None), ("V", None)]);
        assert!(p.items.iter().all(|i| !i.loose));
    }

    #[test]
    fn the_attributes_of_a_module_are_part_of_what_is_inside() {
        let of = |text: &str| {
            let p = parse(text, "x.rs", &[]);
            p.items.iter().map(|i| (i.name.clone(), i.tokens.clone(), i.loose)).collect::<Vec<_>>()
        };
        let one = of("#[cfg(unix)] mod backend { pub fn shade() { 1 } } #[cfg(not(unix))] mod backend { pub fn shade() { 2 } }");
        let swapped = of("#[cfg(not(unix))] mod backend { pub fn shade() { 1 } } #[cfg(unix)] mod backend { pub fn shade() { 2 } }");
        assert_ne!(one, swapped);
        // a module with no attributes brings nothing but its items
        assert_eq!(of("mod m { fn a() {} }"), of("pub mod m { fn a() {} }"));
        // a `use` under `cfg` counts for every result, a plain one does not
        assert!(of("#[cfg(unix)] use a::shade;").iter().any(|(_, _, loose)| *loose));
        assert!(of("use a::shade;").is_empty());
    }

    #[test]
    fn test_code_does_not_count() {
        let p = parse("fn a() {} #[cfg(test)] mod tests { fn b() {} } fn c() {}", "x.rs", &[]);
        assert_eq!(names(&p), [("a", None), ("c", None)]);
    }

    #[test]
    fn inline_modules_and_nested_bodies() {
        let p = parse("mod m { pub fn a() { fn inner() {} } } fn b() {}", "x.rs", &[]);
        assert_eq!(names(&p), [("a", None), ("b", None)]);
    }

    #[test]
    fn trait_impls_are_marked_and_blanket_ones_count_everywhere() {
        let p =
            parse("impl std::ops::Add for V { fn add(self, o: V) -> V { o } } impl<T: X> Y for T {} impl<'a, T> W<'a, T> { fn f() {} }", "x.rs", &[]);
        let add = p.items.iter().find(|i| i.name == "add").unwrap();
        assert!(add.trait_impl);
        assert_eq!(add.owner.as_deref(), Some("V"));
        assert!(p.items.iter().any(|i| i.name == "T" && i.loose));
        let blanket = parse("impl<T: X> Y for T { fn one(&self) {} fn two(&self) {} }", "x.rs", &[]);
        assert!(blanket.items.iter().all(|i| i.loose), "the whole block, not its last function");
        assert_eq!(p.items.iter().find(|i| i.name == "f").unwrap().owner.as_deref(), Some("W"));
    }

    #[test]
    fn what_is_not_understood_counts_everywhere() {
        let p = parse("some_macro!(generated_thing); fn a() {}", "x.rs", &[]);
        let loose = p.items.iter().find(|i| i.loose).unwrap();
        assert!(loose.tokens.iter().any(|t| t.is("generated_thing")));
    }

    #[test]
    fn renames_and_includes() {
        let p = parse(r#"use a::b as c; const D: &[u8] = include_bytes!("d.bin");"#, "x.rs", &[]);
        assert_eq!(p.aliases, [Alias { alias: "c".into(), target: "b".into() }]);
        assert_eq!(p.items[0].includes, ["d.bin"]);
    }

    #[test]
    fn mentions_tell_paths_from_names() {
        let tokens = tokenize("ops::blur(x); y.sample(); Self::go(); Tex::new()");
        let m = mentions_in(&tokens, Some("Me"));
        assert!(m.contains(&Mention::Path("ops".into(), "blur".into())));
        assert!(m.contains(&Mention::Name("sample".into())));
        assert!(m.contains(&Mention::Path("Me".into(), "go".into())));
        assert!(m.contains(&Mention::Path("Tex".into(), "new".into())));
        assert!(m.contains(&Mention::Name("Tex".into())), "an owner is named too");
    }

    #[test]
    fn arms_are_cut_out_of_a_dispatcher() {
        let source = r#"fn compute(kind: &str) -> u8 {
            let shared = helper();
            match kind {
                "a" | "b" => one(),
                "c" => { two(); 3 }
                other if other.len() > 3 => three(),
                _ => fallback(),
            }
        }"#;
        let p = parse(source, "x.rs", &[Dispatcher { file: "x.rs", function: "compute" }]);
        assert_eq!(p.arms.len(), 2);
        assert_eq!(p.arms[0].kinds, ["a", "b"]);
        assert!(p.arms[0].mentions.contains(&Mention::Name("one".into())));
        assert_eq!(p.arms[1].kinds, ["c"]);
        let compute = &p.items[0];
        for name in ["helper", "three", "fallback"] {
            assert!(compute.mentions.contains(&Mention::Name(name.into())), "{name} stays shared");
        }
        for name in ["one", "two"] {
            assert!(!compute.mentions.contains(&Mention::Name(name.into())), "{name} went with its arm");
        }
    }
}
