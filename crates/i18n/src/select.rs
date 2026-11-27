//! Selector messages: a small subset of Unicode's MessageFormat 2, for translations that must choose their words by a
//! number, an ordinal or a noun's gender.
//!
//! A translation is a selector message when it begins with `.input`, `.local` or `.match` ([`Message::is_complex`]); any other
//! translation is plain text with `{name}` placeholders, or the `|` forms of a `@plural` or `@ordinal` row. A selector message is
//! allowed in a plain row (a context that does not start with `@`) and in a `@plural` or `@ordinal` row, and in no other. The subset:
//!
//! - declarations `.input {$x :number}`, `.input {$x :integer}`, `.input {$x :number select=ordinal}`, `.input {$x :noun}` and
//!   `.local $y = {$x :noun form=genitive}`;
//! - one `.match` of one to three declared variables, then variants: a key for each selector (a plural or ordinal category, an
//!   exact whole number, a gender, or `*`) and a quoted pattern `{{…}}`; or no `.match` and one quoted pattern;
//! - in patterns, text, `{$x}`, `{$x :noun form=<name>}` and the escapes `\{`, `\}`, `\|` and `\\`.
//!
//! There is no markup, no other function, and no option beyond `select` and `form`. [`Message::parse`] reads a message under
//! the caps of [`crate::limits`]; [`check_message`] applies the rules a translation of a source text must meet (the checker
//! and the runtime share it, so they cannot disagree); the evaluator runs inside [`crate::I18n`].

use std::borrow::Cow;

use icu_plurals::{PluralCategory, PluralOperands, PluralRules};

use crate::limits::{MAX_ARG_CHARS, MAX_DECLARATIONS, MAX_MESSAGE_BYTES, MAX_PARTS, MAX_SELECTORS, MAX_VARIANTS};
use crate::noun::{Grammar, Noun, Nouns};
use crate::{Format, ltr};

/// Digits after the decimal point a number placeholder shows at most (an `f64` holds fewer).
pub(crate) const MAX_DECIMALS: usize = 20;

/// A CLDR plural category, in CLDR order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    /// `zero`.
    Zero,
    /// `one`.
    One,
    /// `two`.
    Two,
    /// `few`.
    Few,
    /// `many`.
    Many,
    /// `other`.
    Other,
}

impl Category {
    /// The six categories in CLDR order.
    pub const ALL: [Category; 6] = [Category::Zero, Category::One, Category::Two, Category::Few, Category::Many, Category::Other];

    /// The key a variant uses for it: `zero`, `one`, `two`, `few`, `many` or `other`.
    pub const fn key(self) -> &'static str {
        match self {
            Category::Zero => "zero",
            Category::One => "one",
            Category::Two => "two",
            Category::Few => "few",
            Category::Many => "many",
            Category::Other => "other",
        }
    }

    /// The category a key names; `None` for anything else.
    pub fn from_key(key: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.key() == key)
    }
}

pub(crate) fn from_icu(category: PluralCategory) -> Category {
    match category {
        PluralCategory::Zero => Category::Zero,
        PluralCategory::One => Category::One,
        PluralCategory::Two => Category::Two,
        PluralCategory::Few => Category::Few,
        PluralCategory::Many => Category::Many,
        PluralCategory::Other => Category::Other,
    }
}

/// What a language's plural rules say about whole numbers 0 to 1,000, from the CLDR data this crate embeds: the categories
/// they reach, the whole numbers of each ([`LocaleRules::whole_numbers`]) and the categories that hold exactly one. A checker
/// takes its counts from here, so it and the runtime agree.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct LocaleRules {
    /// The cardinal categories whole numbers reach, in CLDR order: the forms of a `@plural` row.
    pub cardinal: Vec<Category>,
    /// The ordinal categories whole numbers reach, in CLDR order: the forms of an `@ordinal` row.
    pub ordinal: Vec<Category>,
    /// The cardinal categories that hold exactly one whole number from 0 to 1,000 (Arabic `zero`, `one` and `two`; Polish `one`;
    /// English `one`).
    pub cardinal_single: Vec<Category>,
    /// The ordinal categories that hold exactly one whole number from 0 to 1,000 (French `one`).
    pub ordinal_single: Vec<Category>,
    /// The whole numbers 0 to 1,000 of each category, cardinal then ordinal, each list in order.
    whole: [[Vec<u16>; 6]; 2],
}

impl LocaleRules {
    /// The rules of `code` (a BCP 47 tag, any case); `None` for a language CLDR has no plural rules for.
    pub fn for_locale(code: &str) -> Option<LocaleRules> {
        LocaleRules::from_rules(crate::negotiation::plural_rules(code).as_ref(), crate::negotiation::ordinal_rules(code).as_ref())
    }

    /// The rules from the cardinal and ordinal rules a locale holds; `None` without cardinal rules.
    pub(crate) fn from_rules(cardinal: Option<&Rules>, ordinal: Option<&Rules>) -> Option<LocaleRules> {
        let (cardinal_rules, cardinal_categories) = cardinal?;
        let cardinal_categories: Vec<Category> = cardinal_categories.iter().copied().map(from_icu).collect();
        let cardinal_whole = whole_numbers_of(cardinal_rules);
        let (ordinal_categories, ordinal_whole) = match ordinal {
            Some((rules, categories)) => (categories.iter().copied().map(from_icu).collect(), whole_numbers_of(rules)),
            None => (vec![Category::Other], whole_numbers_of_other()),
        };
        Some(LocaleRules {
            cardinal_single: singles(&cardinal_whole, &cardinal_categories),
            ordinal_single: singles(&ordinal_whole, &ordinal_categories),
            cardinal: cardinal_categories,
            ordinal: ordinal_categories,
            whole: [cardinal_whole, ordinal_whole],
        })
    }

    /// The whole numbers from 0 to 1,000 that are in `category` under the cardinal (`Select::Plural`) or ordinal
    /// (`Select::Ordinal`) rules, in order: the German `one` is `[1]`, the Polish `few` starts `[2, 3, 4, 22, 23, 24, 32, …]`.
    pub fn whole_numbers(&self, select: Select, category: Category) -> &[u16] {
        let set = match select {
            Select::Plural => self.whole.first(),
            Select::Ordinal => self.whole.get(1),
        };
        set.and_then(|set| set.get(category_index(category))).map_or(&[], Vec::as_slice)
    }
}

fn category_index(category: Category) -> usize {
    Category::ALL.iter().position(|c| *c == category).unwrap_or(5)
}

/// A locale's rules and the categories whole numbers reach.
pub(crate) type Rules = (PluralRules, Vec<PluralCategory>);

/// Each category's whole numbers 0 to 1,000 under `rules`.
fn whole_numbers_of(rules: &PluralRules) -> [Vec<u16>; 6] {
    let mut whole: [Vec<u16>; 6] = Default::default();
    for n in 0..=1000_u16 {
        if let Some(list) = whole.get_mut(category_index(from_icu(rules.category_for(u32::from(n))))) {
            list.push(n);
        }
    }
    whole
}

/// Every whole number 0 to 1,000 is in `other`: the rules of a language with no ordinal data.
fn whole_numbers_of_other() -> [Vec<u16>; 6] {
    let mut whole: [Vec<u16>; 6] = Default::default();
    if let Some(list) = whole.get_mut(category_index(Category::Other)) {
        list.extend(0..=1000_u16);
    }
    whole
}

/// The categories of `reached` that hold exactly one whole number.
fn singles(whole: &[Vec<u16>; 6], reached: &[Category]) -> Vec<Category> {
    reached.iter().copied().filter(|c| whole.get(category_index(*c)).is_some_and(|numbers| numbers.len() == 1)).collect()
}

/// A typed value a message takes: the arguments of [`crate::I18n::tm`], and the values `trn` and `tord` bind to `n`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Arg<'a> {
    /// Text of the program's own (a command's label), inserted as it is.
    Text(&'a str),
    /// A person's text (a page, a file or a layer name): inserted like [`Arg::Text`], and cut to
    /// [`MAX_ARG_CHARS`] characters (ending with `…`).
    Name(&'a str),
    /// A count: written in the region's digits and grouping, and chooses a plural form.
    Count(u64),
    /// A number with the digits after its point it shows (`1.5`, with 1, shows `1,5` where the region says so); the plural form
    /// follows the digits shown.
    Number {
        /// The value.
        value: f64,
        /// The digits shown after the decimal separator.
        decimals: usize,
    },
    /// A noun, by its English name as the code names it (`page`): the language's `@noun` row gives its words and its gender.
    Noun(&'a str),
    /// A date or time, written for the region in the style.
    #[cfg(feature = "dates")]
    Date(crate::DateTime, crate::DateStyle),
}

impl<'a> Arg<'a> {
    /// [`Arg::Number`].
    pub const fn number(value: f64, decimals: usize) -> Arg<'a> {
        Arg::Number { value, decimals }
    }
}

/// What a source text's `{name}` placeholder is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    /// `{name}`: text.
    Text,
    /// `{n :number}`, and the `{n}` of a `@plural` or `@ordinal` row.
    Number,
    /// `{n :integer}`.
    Integer,
    /// `{when :date}`.
    Date,
    /// `{item :noun}`.
    Noun,
}

/// The placeholders of a source text, each once, in order of first appearance, with the type its annotation gives. A name stops
/// at the first white space; an unknown annotation is an error.
pub fn source_placeholders(source: &str) -> Result<Vec<(String, Type)>, String> {
    let mut found: Vec<(String, Type)> = Vec::new();
    let mut rest = source;
    while let Some((_, after)) = rest.split_once('{') {
        let Some((inside, tail)) = after.split_once('}') else { break };
        rest = tail;
        let mut words = inside.split_whitespace();
        let Some(name) = words.next() else { continue };
        let kind = match words.next() {
            None => Type::Text,
            Some(":number") => Type::Number,
            Some(":integer") => Type::Integer,
            Some(":date") => Type::Date,
            Some(":noun") => Type::Noun,
            Some(other) => return Err(format!("unknown placeholder type {other:?} (:number, :integer, :date or :noun)")),
        };
        if words.next().is_some() {
            return Err(format!("{{{inside}}}: a placeholder has a name and at most one type"));
        }
        match found.iter_mut().find(|(n, _)| n == name) {
            Some((_, known)) if *known == Type::Text => *known = kind,
            Some((_, known)) if kind != Type::Text && *known != kind => return Err(format!("{name:?} is typed two ways")),
            Some(_) => {}
            None => found.push((name.to_owned(), kind)),
        }
    }
    Ok(found)
}

/// A function a declaration applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Function {
    /// `:number`.
    Number,
    /// `:integer`.
    Integer,
    /// `:noun`.
    Noun,
}

/// What a number selects on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Select {
    /// The cardinal plural categories (the default).
    Plural,
    /// The ordinal categories (`select=ordinal`).
    Ordinal,
}

/// An `.input` or `.local` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Declaration {
    /// The name declared: the input itself, or the new local.
    pub name: String,
    /// `.local`: the variable it derives from (`.local $y = {$x …}` has `x`); `None` for an `.input`.
    pub from: Option<String>,
    /// The function applied.
    pub function: Function,
    /// `select=`, for a number.
    pub select: Select,
    /// `form=`, for a noun.
    pub form: Option<String>,
}

/// A variant's key for one selector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// `*`.
    Catchall,
    /// An exact whole number (`0`, `1`).
    Number(u64),
    /// A plural or ordinal category (`one`) or a gender (`feminine`).
    Name(String),
}

/// A piece of a pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// Text, its escapes resolved.
    Text(String),
    /// `{$name}`, or `{$name :noun form=…}` with its form.
    Var {
        /// The variable's name.
        name: String,
        /// The named form of a noun.
        form: Option<String>,
    },
}

/// A variant of a `.match`: one key for each selector, and its pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Variant {
    /// One key for each selector, in the selectors' order.
    pub keys: Vec<Key>,
    /// The pattern shown when the keys are the best match.
    pub pattern: Vec<Part>,
}

/// What comes after the declarations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// One quoted pattern.
    Pattern(Vec<Part>),
    /// A `.match` of declared variables and its variants.
    Match {
        /// The selector variables' names, one to [`MAX_SELECTORS`].
        selectors: Vec<String>,
        /// The variants, at most [`MAX_VARIANTS`].
        variants: Vec<Variant>,
    },
}

/// A parsed selector message.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Message {
    /// The declarations, in order.
    pub declarations: Vec<Declaration>,
    /// The pattern, or the `.match`.
    pub body: Body,
}

impl Message {
    /// Whether `text` is a selector message: it begins (after white space) with `.input`, `.local` or `.match`.
    pub fn is_complex(text: &str) -> bool {
        let text = text.trim_start();
        [".input", ".local", ".match"].iter().any(|keyword| text.starts_with(keyword))
    }

    /// Reads a selector message under the caps of [`crate::limits`]; the error says what is refused.
    pub fn parse(text: &str) -> Result<Message, String> {
        if text.len() > MAX_MESSAGE_BYTES {
            return Err(format!("a selector message is at most {MAX_MESSAGE_BYTES} bytes"));
        }
        Parser { text, at: 0 }.message()
    }

    /// The patterns of the message: the one pattern, or each variant's.
    pub fn patterns(&self) -> Vec<&[Part]> {
        match &self.body {
            Body::Pattern(pattern) => vec![pattern.as_slice()],
            Body::Match { variants, .. } => variants.iter().map(|v| v.pattern.as_slice()).collect(),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The parser

struct Parser<'a> {
    text: &'a str,
    at: usize,
}

type Parsed<T> = Result<T, String>;

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        self.text.get(self.at..).unwrap_or("")
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, token: &str) -> bool {
        if self.rest().starts_with(token) {
            self.at += token.len();
            true
        } else {
            false
        }
    }

    fn fail<T>(&self, what: impl std::fmt::Display) -> Parsed<T> {
        Err(format!("{what} (at byte {})", self.at))
    }

    /// White space: space, tab, line feed and carriage return. Returns whether any was skipped.
    fn space(&mut self) -> bool {
        let before = self.at;
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.at += 1;
        }
        self.at > before
    }

    fn expect(&mut self, token: &str) -> Parsed<()> {
        if self.eat(token) { Ok(()) } else { self.fail(format!("expected `{token}`")) }
    }

    /// A name: ASCII letters, digits, `_`, `-` and `.`, starting with a letter or `_`, at most 64 bytes.
    fn name(&mut self, what: &str) -> Parsed<String> {
        let start = self.at;
        if !self.peek().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
            return self.fail(format!("expected {what}"));
        }
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) {
            self.at += 1;
        }
        let name = self.text.get(start..self.at).unwrap_or("");
        if name.len() > 64 {
            return self.fail(format!("{what} longer than 64 bytes"));
        }
        Ok(name.to_owned())
    }

    fn variable(&mut self) -> Parsed<String> {
        self.expect("$")?;
        self.name("a variable name")
    }

    fn message(&mut self) -> Parsed<Message> {
        let mut declarations = Vec::new();
        self.space();
        loop {
            let declaration = if self.eat(".input") {
                self.input()?
            } else if self.eat(".local") {
                self.local()?
            } else {
                break;
            };
            if declarations.len() >= MAX_DECLARATIONS {
                return Err(format!("more than {MAX_DECLARATIONS} declarations"));
            }
            declarations.push(declaration);
            self.space();
        }
        let body = if self.eat(".match") {
            self.matcher()?
        } else if self.rest().starts_with("{{") {
            Body::Pattern(self.pattern()?)
        } else {
            return self.fail("expected a quoted pattern `{{…}}` or `.match`");
        };
        self.space();
        if !self.rest().is_empty() {
            return self.fail("unexpected text after the message");
        }
        Ok(Message { declarations, body })
    }

    fn input(&mut self) -> Parsed<Declaration> {
        self.space();
        self.expect("{")?;
        self.space();
        let name = self.variable()?;
        let (function, select, form) = self.annotation(true)?;
        self.expect("}")?;
        Ok(Declaration { name, from: None, function, select, form })
    }

    fn local(&mut self) -> Parsed<Declaration> {
        if !self.space() {
            return self.fail("expected white space after `.local`");
        }
        let name = self.variable()?;
        self.space();
        self.expect("=")?;
        self.space();
        self.expect("{")?;
        self.space();
        let from = self.variable()?;
        let (function, select, form) = self.annotation(true)?;
        self.expect("}")?;
        Ok(Declaration { name, from: Some(from), function, select, form })
    }

    /// `:function option=value…` up to the closing `}` of a declaration; the annotation is required there.
    fn annotation(&mut self, required: bool) -> Parsed<(Function, Select, Option<String>)> {
        self.space();
        if !self.eat(":") {
            return if required {
                self.fail("a declaration applies a function (`:number`, `:integer` or `:noun`)")
            } else {
                self.fail("expected `:`")
            };
        }
        let function = match self.name("a function name")?.as_str() {
            "number" => Function::Number,
            "integer" => Function::Integer,
            "noun" => Function::Noun,
            other => return Err(format!("unknown function :{other} (:number, :integer or :noun)")),
        };
        let mut select = None;
        let mut form = None;
        loop {
            if !self.space() || matches!(self.peek(), Some('}') | None) {
                break;
            }
            let option = self.name("an option name")?;
            if !matches!(option.as_str(), "select" | "form") {
                return Err(format!("unknown option {option} (select or form)"));
            }
            self.expect("=")?;
            let value = self.name("an option value")?;
            match option.as_str() {
                "select" if form.is_none() && select.is_none() => {
                    select = Some(match value.as_str() {
                        "plural" => Select::Plural,
                        "ordinal" => Select::Ordinal,
                        other => return Err(format!("select={other} is not supported (plural or ordinal)")),
                    });
                }
                "form" if form.is_none() => form = Some(value),
                "select" | "form" => return Err(format!("option {option} given twice")),
                other => return Err(format!("unknown option {other} (select or form)")),
            }
        }
        if select.is_some() && function == Function::Noun {
            return Err("select applies to a number, not a noun".into());
        }
        if form.is_some() && function != Function::Noun {
            return Err("form applies to a noun, not a number".into());
        }
        Ok((function, select.unwrap_or(Select::Plural), form))
    }

    fn matcher(&mut self) -> Parsed<Body> {
        let mut selectors = Vec::new();
        while self.space() && self.peek() == Some('$') {
            if selectors.len() >= MAX_SELECTORS {
                return Err(format!("more than {MAX_SELECTORS} selectors"));
            }
            let name = self.variable()?;
            if selectors.contains(&name) {
                return Err(format!("selector ${name} is listed twice"));
            }
            selectors.push(name);
        }
        if selectors.is_empty() {
            return self.fail("`.match` needs a selector variable");
        }
        let mut variants: Vec<Variant> = Vec::new();
        loop {
            self.space();
            if self.rest().is_empty() {
                break;
            }
            if variants.len() >= MAX_VARIANTS {
                return Err(format!("more than {MAX_VARIANTS} variants"));
            }
            let mut keys = Vec::new();
            while !self.rest().starts_with("{{") {
                if keys.len() >= selectors.len() {
                    return self.fail("a variant has a key for each selector, then a quoted pattern");
                }
                keys.push(self.key()?);
                self.space();
            }
            if keys.len() != selectors.len() {
                return Err(format!("a variant has {} keys; the `.match` has {} selectors", keys.len(), selectors.len()));
            }
            let pattern = self.pattern()?;
            variants.push(Variant { keys, pattern });
        }
        if variants.is_empty() {
            return self.fail("`.match` needs variants");
        }
        if !variants.iter().any(|v| v.keys.iter().all(|k| *k == Key::Catchall)) {
            return Err("no variant of only `*` keys: a `.match` needs a catch-all".into());
        }
        Ok(Body::Match { selectors, variants })
    }

    fn key(&mut self) -> Parsed<Key> {
        if self.eat("*") {
            return Ok(Key::Catchall);
        }
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) {
            self.at += 1;
        }
        let key = self.text.get(start..self.at).unwrap_or("");
        if key.is_empty() || key.len() > 64 {
            return self.fail("expected a variant key: a category, a whole number, a gender or `*`");
        }
        if key.bytes().all(|b| b.is_ascii_digit()) {
            return key.parse::<u64>().map(Key::Number).or_else(|_| self.fail("a whole number key too large"));
        }
        Ok(Key::Name(key.to_owned()))
    }

    /// `{{…}}`: text with its escapes, `{$x}` and `{$x :noun form=y}`.
    fn pattern(&mut self) -> Parsed<Vec<Part>> {
        self.expect("{{")?;
        let mut parts: Vec<Part> = Vec::new();
        let mut text = String::new();
        let push_text = |parts: &mut Vec<Part>, text: &mut String| -> Parsed<()> {
            if !text.is_empty() {
                if parts.len() >= MAX_PARTS {
                    return Err(format!("more than {MAX_PARTS} parts in a pattern"));
                }
                parts.push(Part::Text(std::mem::take(text)));
            }
            Ok(())
        };
        loop {
            match self.peek() {
                None => return self.fail("a pattern is not closed with `}}`"),
                Some('{') => {
                    push_text(&mut parts, &mut text)?;
                    if parts.len() >= MAX_PARTS {
                        return Err(format!("more than {MAX_PARTS} parts in a pattern"));
                    }
                    parts.push(self.placeholder()?);
                }
                Some('}') => {
                    if self.eat("}}") {
                        push_text(&mut parts, &mut text)?;
                        return Ok(parts);
                    }
                    return self.fail("a `}` in a pattern is written `\\}`");
                }
                Some('\\') => {
                    self.at += 1;
                    match self.bump() {
                        Some(c @ ('{' | '}' | '|' | '\\')) => text.push(c),
                        _ => return self.fail("an escape is `\\{`, `\\}`, `\\|` or `\\\\`"),
                    }
                }
                Some(c) => {
                    self.at += c.len_utf8();
                    text.push(c);
                }
            }
        }
    }

    fn placeholder(&mut self) -> Parsed<Part> {
        self.expect("{")?;
        self.space();
        match self.peek() {
            Some('#' | '/') => return Err("markup is not supported in a pattern".into()),
            Some('$') => {}
            _ => return Err("a placeholder in a pattern is a variable, `{$name}`".into()),
        }
        let name = self.variable()?;
        let mut form = None;
        if self.space() && self.peek() == Some(':') {
            let (function, _, parsed_form) = self.annotation(false)?;
            if function != Function::Noun || parsed_form.is_none() {
                return Err("in a pattern a variable takes only `:noun form=<name>`".into());
            }
            form = parsed_form;
            self.space();
        }
        self.expect("}")?;
        Ok(Part::Var { name, form })
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The rules a translation must meet

/// How a row's source text declares its count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// A plain row (any context other than `@plural` and `@ordinal`): its placeholders are typed in the source text.
    Plain,
    /// A `@plural` row: `{n}` is a count and the message selects on the cardinal categories.
    Plural,
    /// An `@ordinal` row: `{n}` is a count and the message selects on the ordinal categories.
    Ordinal,
}

/// What a locale provides to the rules: its plural rules and its declared grammar.
#[derive(Clone, Copy, Debug)]
pub struct Locale<'a> {
    /// The plural rules; `None` for a language CLDR has none for (only exact numbers and `*` can select there).
    pub rules: Option<&'a LocaleRules>,
    /// The genders and forms the language's `@grammar` rows declare.
    pub grammar: &'a Grammar,
}

/// Applies the rules of a selector message `translation` of `source` in `locale`, and returns it parsed:
///
/// 1. it parses (the caps of [`crate::limits`]);
/// 2. every variable is a placeholder of the source or declared; every `.input` names a source placeholder; every selector is
///    declared with a function that fits the source's type (`:number` or `:integer` for a number, `:noun` for a noun);
/// 3. keys are the locale's categories (cardinal, or ordinal with `select=ordinal`) or exact whole numbers for a number, and its
///    declared genders for a noun; `other` is never a key (`*` is the `other` category); a variant of only `*` exists; **every
///    category whole numbers 0 to 1,000 reach, except `other`, has a variant of its own**, or else an exact variant for each of
///    its whole numbers (German `one` is only 1);
/// 4. every placeholder of the source appears in every variant, but the selector's own number may be left out when the key is an
///    exact number or a category that holds exactly one such number;
/// 6. a `form=` names a form the locale declares;
/// 7. every pattern agrees with the source on a trailing `…`, and the text holds no bidi embedding, override or isolate (the
///    runtime adds the isolates).
///
/// (Rule 5, the form lists of `@plural` and `@ordinal` rows, is [`crate::I18n`]'s own check of those rows.)
pub fn check_message(kind: RowKind, source: &str, translation: &str, locale: &Locale<'_>) -> Result<Message, String> {
    let message = Message::parse(translation)?;
    // Rule 7: bidi controls.
    if let Some(c) = translation.chars().find(|c| matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')) {
        return Err(format!("U+{:04X} is a bidi embedding, override or isolate: the runtime adds the isolates", u32::from(c)));
    }
    // The source's placeholders, from the last form of a count row.
    let shown = match kind {
        RowKind::Plain => source,
        RowKind::Plural | RowKind::Ordinal => source.rsplit('|').next().unwrap_or(source),
    };
    let mut placeholders = source_placeholders(shown)?;
    if kind != RowKind::Plain {
        for (name, ty) in &mut placeholders {
            if name == "n" {
                *ty = Type::Number;
            }
        }
    }
    let source_type = |name: &str| placeholders.iter().find(|(n, _)| n == name).map(|(_, t)| *t);
    // Rule 2: declarations.
    struct Known<'m> {
        root: &'m str,
        function: Function,
        select: Select,
    }
    let mut known: Vec<(&str, Known<'_>)> = Vec::new();
    for d in &message.declarations {
        if known.iter().any(|(n, _)| *n == d.name) {
            return Err(format!("${} is declared twice", d.name));
        }
        let root = match &d.from {
            None => d.name.as_str(),
            Some(from) => match known.iter().find(|(n, _)| n == from) {
                Some((_, k)) => k.root,
                None => return Err(format!("${from} is used before it is declared")),
            },
        };
        let Some(ty) = source_type(root) else {
            return Err(format!("${root} is not a placeholder of the source text"));
        };
        let fits = match d.function {
            Function::Number | Function::Integer => matches!(ty, Type::Number | Type::Integer),
            Function::Noun => ty == Type::Noun,
        };
        if !fits {
            return Err(format!("${root} is {} in the source text; the function does not fit it", describe(ty)));
        }
        if let Some(form) = &d.form
            && !locale.grammar.forms.contains(form)
        {
            return Err(format!("form={form} is not declared by the language's @grammar forms"));
        }
        known.push((d.name.as_str(), Known { root, function: d.function, select: d.select }));
    }
    // Variables in patterns.
    for pattern in message.patterns() {
        for part in pattern {
            let Part::Var { name, form } = part else { continue };
            let root = match known.iter().find(|(n, _)| n == name) {
                Some((_, k)) => Some((k.root, k.function == Function::Noun)),
                None => source_type(name).map(|t| (name.as_str(), t == Type::Noun)),
            };
            let Some((_, is_noun)) = root else {
                return Err(format!("${name} is neither a placeholder of the source text nor declared"));
            };
            if let Some(form) = form {
                if !is_noun {
                    return Err(format!("${name} is not a noun: only a noun takes form="));
                }
                if !locale.grammar.forms.contains(form) {
                    return Err(format!("form={form} is not declared by the language's @grammar forms"));
                }
            }
        }
    }
    // Rule 3 and 4: the `.match`.
    if let Body::Match { selectors, variants } = &message.body {
        let mut chosen: Vec<(&Known<'_>, bool)> = Vec::new();
        for selector in selectors {
            let Some((_, k)) = known.iter().find(|(n, _)| n == selector) else {
                return Err(format!("selector ${selector} is not declared with `.input` or `.local`"));
            };
            match (kind, k.function, k.select) {
                (RowKind::Plural, Function::Number | Function::Integer, Select::Ordinal) => {
                    return Err("a @plural row selects on cardinal categories: leave out `select=ordinal`".into());
                }
                (RowKind::Ordinal, Function::Number | Function::Integer, Select::Plural) if k.root == "n" => {
                    return Err("an @ordinal row selects with `select=ordinal` on its count".into());
                }
                _ => {}
            }
            chosen.push((k, k.function == Function::Noun));
        }
        let none = LocaleRules { cardinal: vec![Category::Other], ordinal: vec![Category::Other], ..LocaleRules::default() };
        let rules = locale.rules.unwrap_or(&none);
        for (position, (k, is_noun)) in chosen.iter().enumerate() {
            let allowed: Vec<&str> = if *is_noun {
                locale.grammar.genders.iter().map(String::as_str).collect()
            } else {
                match k.select {
                    Select::Plural => rules.cardinal.iter().map(|c| c.key()).collect(),
                    Select::Ordinal => rules.ordinal.iter().map(|c| c.key()).collect(),
                }
            };
            for variant in variants {
                match variant.keys.get(position) {
                    Some(Key::Name(name)) if name == "other" && !*is_noun => {
                        return Err("`other` is never written as a key: `*` is the `other` category".into());
                    }
                    Some(Key::Name(name)) if !allowed.contains(&name.as_str()) => {
                        let what = if *is_noun { "a gender the language declares" } else { "a plural category the language reaches" };
                        return Err(format!("key {name:?} is not {what} (it has: {})", allowed.join(", ")));
                    }
                    Some(Key::Number(_)) if *is_noun => return Err("a noun selects on genders, not numbers".into()),
                    _ => {}
                }
            }
            if !*is_noun {
                for category in rules_categories(rules, k.select).iter().filter(|c| **c != Category::Other) {
                    let named = variants.iter().any(|v| matches!(v.keys.get(position), Some(Key::Name(n)) if n == category.key()));
                    let by_number = rules
                        .whole_numbers(k.select, *category)
                        .iter()
                        .all(|n| variants.iter().any(|v| matches!(v.keys.get(position), Some(Key::Number(exact)) if *exact == u64::from(*n))));
                    if !named && !by_number {
                        return Err(format!(
                            "no variant for `{}`, which whole numbers reach in this language (or one for each of its whole numbers): `*` stands for `other`",
                            category.key()
                        ));
                    }
                }
            }
        }
        // Rule 4 for each variant.
        for variant in variants {
            let mut exempt: Vec<&str> = Vec::new();
            for (position, (k, is_noun)) in chosen.iter().enumerate() {
                if *is_noun {
                    continue;
                }
                let singles = match k.select {
                    Select::Plural => &rules.cardinal_single,
                    Select::Ordinal => &rules.ordinal_single,
                };
                let single = match variant.keys.get(position) {
                    Some(Key::Number(_)) => true,
                    Some(Key::Name(name)) => Category::from_key(name).is_some_and(|c| singles.contains(&c)),
                    _ => false,
                };
                if single {
                    exempt.push(k.root);
                }
            }
            check_uses(&placeholders, &known.iter().map(|(n, k)| (*n, k.root)).collect::<Vec<_>>(), &variant.pattern, &exempt, &variant.keys)?;
        }
    } else if let Body::Pattern(pattern) = &message.body {
        check_uses(&placeholders, &known.iter().map(|(n, k)| (*n, k.root)).collect::<Vec<_>>(), pattern, &[], &[])?;
    }
    // Rule 7: the trailing `…`.
    let ellipsis = shown.ends_with('…');
    for pattern in message.patterns() {
        let ends = matches!(pattern.last(), Some(Part::Text(t)) if t.ends_with('…'));
        if ends != ellipsis {
            return Err("a variant disagrees with the source text on a trailing `…`".into());
        }
    }
    Ok(message)
}

fn rules_categories(rules: &LocaleRules, select: Select) -> &[Category] {
    match select {
        Select::Plural => &rules.cardinal,
        Select::Ordinal => &rules.ordinal,
    }
}

fn describe(ty: Type) -> &'static str {
    match ty {
        Type::Text => "text",
        Type::Number => "a number",
        Type::Integer => "an integer",
        Type::Date => "a date",
        Type::Noun => "a noun",
    }
}

/// Rule 4: every placeholder of the source appears in `pattern` (through a local that derives from it too), except `exempt`.
fn check_uses(placeholders: &[(String, Type)], roots: &[(&str, &str)], pattern: &[Part], exempt: &[&str], keys: &[Key]) -> Result<(), String> {
    let used: Vec<&str> = pattern
        .iter()
        .filter_map(|p| match p {
            Part::Var { name, .. } => Some(roots.iter().find(|(n, _)| n == name).map_or(name.as_str(), |(_, root)| *root)),
            Part::Text(_) => None,
        })
        .collect();
    for (name, _) in placeholders {
        if !used.contains(&name.as_str()) && !exempt.contains(&name.as_str()) {
            let which = keys
                .iter()
                .map(|k| match k {
                    Key::Catchall => "*".to_owned(),
                    Key::Number(n) => n.to_string(),
                    Key::Name(n) => n.clone(),
                })
                .collect::<Vec<_>>()
                .join(" ");
            return Err(if which.is_empty() {
                format!("the pattern leaves out {{${name}}}")
            } else {
                format!("the variant `{which}` leaves out {{${name}}}")
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// The evaluator

/// What evaluation reads from the locale of the row and the region.
pub(crate) struct Env<'a> {
    pub cardinal: Option<&'a PluralRules>,
    pub ordinal: Option<&'a PluralRules>,
    /// The locale's nouns; `None` when rendering English, where a noun shows as its English name.
    pub nouns: Option<&'a Nouns>,
    pub format: &'a Format,
    /// Written for a document or a file name: `0`–`9`, no grouping.
    pub plain: bool,
    #[cfg(feature = "dates")]
    pub dates: &'a crate::DateFormat,
}

/// A resolved value.
enum Var<'a> {
    Number {
        value: f64,
        decimals: usize,
        count: Option<u64>,
        ordinal: bool,
    },
    Text(Cow<'a, str>),
    Noun {
        noun: &'a Noun,
        form: Option<&'a str>,
    },
    #[cfg(feature = "dates")]
    Written(String),
}

/// `text` cut to `max` characters, the last of them `…` when it was cut.
pub(crate) fn cap_chars(text: &str, max: usize) -> Cow<'_, str> {
    match text.char_indices().nth(max) {
        None => Cow::Borrowed(text),
        Some((cut, _)) => {
            let keep = text.char_indices().nth(max.saturating_sub(1)).map_or(cut, |(i, _)| i);
            Cow::Owned(format!("{}…", text.get(..keep).unwrap_or("")))
        }
    }
}

/// `arg` as a value. Text is capped; a noun is found in the locale's nouns (`None` when it has no row for it).
fn value<'a>(arg: &Arg<'a>, env: &Env<'a>) -> Option<Var<'a>> {
    Some(match *arg {
        Arg::Text(s) | Arg::Name(s) => Var::Text(cap_chars(s, MAX_ARG_CHARS)),
        Arg::Count(n) => Var::Number { value: n as f64, decimals: 0, count: Some(n), ordinal: false },
        Arg::Number { value, decimals } => Var::Number { value, decimals: decimals.min(MAX_DECIMALS), count: None, ordinal: false },
        Arg::Noun(key) => match env.nouns {
            Some(nouns) => Var::Noun { noun: nouns.get(key)?, form: None },
            None => Var::Text(Cow::Borrowed(key)),
        },
        #[cfg(feature = "dates")]
        Arg::Date(at, style) => Var::Written(env.dates.format(at, style)),
    })
}

/// `arg` as the text a placeholder shows; `None` for a noun the locale has no row for.
pub(crate) fn shown(arg: &Arg<'_>, env: &Env<'_>) -> Option<String> {
    Some(value(arg, env)?.shown(env, None))
}

fn operands(value: f64, decimals: usize, count: Option<u64>) -> Option<PluralOperands> {
    match count {
        Some(n) if decimals == 0 => Some(PluralOperands::from(n)),
        _ => crate::format::to_decimal(value, decimals).map(|d| PluralOperands::from(&d)),
    }
}

impl Var<'_> {
    /// The text a placeholder shows.
    fn shown(&self, env: &Env<'_>, form: Option<&str>) -> String {
        match self {
            Var::Number { value, decimals, count, .. } => match count {
                Some(n) if env.plain && *decimals == 0 => n.to_string(),
                Some(n) if *decimals == 0 => ltr(&env.format.integer(*n)),
                _ if env.plain => env.format.number_ungrouped(*value, *decimals),
                _ => ltr(&env.format.number(*value, *decimals)),
            },
            Var::Text(text) => text.to_string(),
            Var::Noun { noun, form: own } => form.or(*own).map_or_else(|| noun.text.clone(), |f| noun.form(f).to_owned()),
            #[cfg(feature = "dates")]
            Var::Written(text) => text.clone(),
        }
    }

    /// The keys that match this value, most preferred first (`*` always matches last, implicitly).
    fn keys(&self, env: &Env<'_>) -> Vec<Key> {
        match self {
            Var::Number { value, decimals, count, ordinal } => {
                let mut keys = Vec::new();
                let whole = match count {
                    Some(n) if *decimals == 0 => Some(*n),
                    _ if value.is_finite() && value.fract() == 0.0 && (0.0..1e15).contains(value) => Some(*value as u64),
                    _ => None,
                };
                if let Some(whole) = whole {
                    keys.push(Key::Number(whole));
                }
                let rules = if *ordinal { env.ordinal } else { env.cardinal };
                let category = match (rules, operands(*value, *decimals, *count)) {
                    (Some(rules), Some(operands)) => from_icu(rules.category_for(operands)),
                    _ => Category::Other,
                };
                keys.push(Key::Name(category.key().to_owned()));
                keys
            }
            Var::Noun { noun, .. } => noun.gender.iter().map(|g| Key::Name(g.clone())).collect(),
            Var::Text(_) => Vec::new(),
            #[cfg(feature = "dates")]
            Var::Written(_) => Vec::new(),
        }
    }
}

impl Message {
    /// The pieces of the message for `args`: (text, whether it is an inserted value). `None` when the message cannot be shown:
    /// an argument is missing or of another type than its function takes, a noun has no row, or no variant matches.
    pub(crate) fn evaluate<'m>(&'m self, env: &Env<'m>, args: &[(&str, Arg<'m>)]) -> Option<Vec<(Cow<'m, str>, bool)>> {
        let mut vars: Vec<(&str, Var<'m>)> = Vec::new();
        for d in &self.declarations {
            let source = d.from.as_deref().unwrap_or(&d.name);
            let var = match d.from {
                None => value(&args.iter().find(|(n, _)| *n == source)?.1, env)?,
                Some(_) => match vars.iter().rev().find(|(n, _)| *n == source)?.1 {
                    Var::Number { value, decimals, count, ordinal } => Var::Number { value, decimals, count, ordinal },
                    Var::Noun { noun, form } => Var::Noun { noun, form },
                    Var::Text(_) => return None,
                    #[cfg(feature = "dates")]
                    Var::Written(_) => return None,
                },
            };
            let var = match (d.function, var) {
                (Function::Number, Var::Number { value, decimals, count, .. }) => {
                    Var::Number { value, decimals, count, ordinal: d.select == Select::Ordinal }
                }
                (Function::Integer, Var::Number { value, count, .. }) => {
                    Var::Number { value, decimals: 0, count, ordinal: d.select == Select::Ordinal }
                }
                (Function::Noun, Var::Noun { noun, form }) => Var::Noun { noun, form: d.form.as_deref().or(form) },
                _ => return None,
            };
            vars.push((d.name.as_str(), var));
        }
        let find = |name: &str| vars.iter().rev().find(|(n, _)| *n == name).map(|(_, v)| v);
        let pattern: &[Part] = match &self.body {
            Body::Pattern(pattern) => pattern,
            Body::Match { selectors, variants } => {
                let mut preferences: Vec<Vec<Key>> = Vec::new();
                for selector in selectors {
                    preferences.push(find(selector)?.keys(env));
                }
                &choose(variants, &preferences)?.pattern
            }
        };
        let mut out = Vec::with_capacity(pattern.len());
        for part in pattern {
            match part {
                Part::Text(text) => out.push((Cow::Borrowed(text.as_str()), false)),
                Part::Var { name, form } => {
                    let shown = match find(name) {
                        Some(var) => var.shown(env, form.as_deref()),
                        None => {
                            let arg = args.iter().find(|(n, _)| n == name)?;
                            value(&arg.1, env)?.shown(env, form.as_deref())
                        }
                    };
                    out.push((Cow::Owned(shown), true));
                }
            }
        }
        Some(out)
    }
}

/// The variant that matches best: for each selector in turn, an exact number, then the category (or the gender), then `*`.
fn choose<'m>(variants: &'m [Variant], preferences: &[Vec<Key>]) -> Option<&'m Variant> {
    let mut best: Option<(Vec<usize>, &Variant)> = None;
    'variants: for variant in variants {
        let mut ranks = Vec::with_capacity(preferences.len());
        for (key, preferred) in variant.keys.iter().zip(preferences) {
            let rank = match key {
                Key::Catchall => preferred.len(),
                other => match preferred.iter().position(|p| p == other) {
                    Some(rank) => rank,
                    None => continue 'variants,
                },
            };
            ranks.push(rank);
        }
        if best.as_ref().is_none_or(|(b, _)| ranks < *b) {
            best = Some((ranks, variant));
        }
    }
    best.map(|(_, variant)| variant)
}
