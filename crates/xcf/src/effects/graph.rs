//! Inert GEGL chain syntax, not an operation registry or evaluator.
//!
//! Source declarations type literals only. They never supply callbacks, defaults,
//! native commands or resources. Unknown names remain ordered data with indexed
//! diagnostics. Nodes are dependency-first; `source_ordinal` records textual order.
//! Label definitions are visible only after definition, throughout this graph
//! (including earlier auxiliary blocks), and never survive a separate parse.

use super::{Budget, Error, Limit, Result, invalid};

#[cfg(test)]
mod tests;

const MAX_BYTES: usize = 32768;
const MAX_TOKENS: usize = 2048;
const MAX_NODES: usize = 64;
const MAX_ARGUMENTS: usize = 256;
const MAX_DEPTH: usize = 8;

/// Format declarations supplied by the consumer, not native mapping policy.
#[derive(Clone, Copy, Debug)]
pub struct Operation<'a> {
    pub name: &'a str,
    pub fields: &'a [Field<'a>],
}
#[derive(Clone, Copy, Debug)]
pub struct Field<'a> {
    pub name: &'a str,
    pub kind: FieldType<'a>,
}
#[derive(Clone, Copy, Debug)]
pub enum FieldType<'a> {
    Number,
    Integer,
    UnsignedInteger,
    Boolean,
    Text,
    Color,
    Configuration,
    Choice(&'a [(&'a str, i32)]),
    Input,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Source,
    Node(usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    LinearRgb,
    Srgb,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub rgba: [f64; 4],
    pub space: ColorSpace,
}

/// Text/configuration/unknown values live in `Property::literal`, avoiding a
/// second copy. Numeric literals keep that source text as well as their value.
#[derive(Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Integer(i64),
    UnsignedInteger(u32),
    Boolean(bool),
    Choice(i32),
    Text,
    Color(Color),
    Configuration,
    Unknown,
    Input(Input),
}
#[derive(Debug, PartialEq)]
pub struct Property {
    pub name: String,
    pub literal: String,
    pub value: Value,
}
#[derive(Debug, PartialEq)]
pub struct Node {
    pub source_ordinal: usize,
    pub operation: String,
    /// None preserves a source generator/unspecified auxiliary input. The host
    /// must decide whether the operation can actually operate without an input.
    pub input: Option<Input>,
    pub properties: Vec<Property>,
}
#[derive(Debug, PartialEq)]
pub struct Label {
    pub name: String,
    pub input: Input,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Diagnostic {
    UnknownOperation { source_ordinal: usize },
    UnknownProperty { source_ordinal: usize, property: usize },
}
#[derive(Debug, PartialEq)]
pub struct Graph<'a> {
    pub raw: &'a str,
    pub nodes: Vec<Node>,
    pub labels: Vec<Label>,
    pub output: Input,
    pub diagnostics: Vec<Diagnostic>,
}

/// Decode one finite source graph with the effect document's same work budget.
/// Temporary/retained syntax storage is admitted conservatively before growth;
/// failed noncancelled parses refund it, but never the attempted decode work.
/// No declaration means no type guess, rather than a guessed native operation.
pub fn parse<'a>(raw: &'a str, declarations: &[Operation<'_>], budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<Graph<'a>> {
    if !keep_going() {
        return Err(crate::Error::Cancelled.into());
    }
    budget.work.charge(64)?;
    if raw.len() > MAX_BYTES {
        return Err(invalid("graph exceeds 32768 UTF-8 bytes"));
    }
    budget.work.charge(raw.len().saturating_mul(4))?;
    if raw.contains('\0') {
        return Err(invalid("graph contains NUL"));
    }
    if declarations.len() > 512 {
        return Err(invalid("graph source declarations exceed 512 operations"));
    }
    let before = budget.retained;
    let result = (|| {
        let tokens = tokenize(raw, budget, keep_going)?;
        if tokens.is_empty() {
            return Err(invalid("graph has no input or operation"));
        }
        let mut decoder = Decoder {
            tokens,
            cursor: 0,
            declarations,
            budget,
            keep_going,
            nodes: Vec::new(),
            labels: Vec::new(),
            diagnostics: Vec::new(),
            source_nodes: 0,
            assignments: 0,
        };
        let output = decoder.block(Some(Input::Source), 0)?;
        if decoder.peek().is_some() {
            return Err(invalid("graph has trailing structural syntax"));
        }
        Ok(Graph { raw, nodes: decoder.nodes, labels: decoder.labels, output, diagnostics: decoder.diagnostics })
    })();
    if result.as_ref().is_err_and(|error| !matches!(error, Error::Source(crate::Error::Cancelled))) {
        budget.retained = before;
    }
    result
}

fn check(keep_going: &dyn Fn() -> bool) -> Result<()> {
    if keep_going() { Ok(()) } else { Err(crate::Error::Cancelled.into()) }
}
fn reserve<T>(values: &mut Vec<T>, additional: usize, budget: &mut Budget) -> Result<()> {
    let needed = values.len().checked_add(additional).ok_or(Error::Limit(Limit::Storage))?;
    if needed > values.capacity() {
        let capacity = needed.max(values.capacity().saturating_mul(2));
        let bytes = capacity.saturating_sub(values.capacity()).checked_mul(std::mem::size_of::<T>()).ok_or(Error::Limit(Limit::Storage))?;
        budget.retained = budget.retained.checked_sub(bytes).ok_or(Error::Limit(Limit::Storage))?;
        values.try_reserve_exact(capacity.saturating_sub(values.len())).map_err(|_| Error::Limit(Limit::Storage))?;
    }
    Ok(())
}
fn push<T>(values: &mut Vec<T>, value: T, budget: &mut Budget) -> Result<()> {
    reserve(values, 1, budget)?;
    values.push(value);
    Ok(())
}
fn character(out: &mut String, ch: char, budget: &mut Budget) -> Result<()> {
    let needed = out.len().checked_add(ch.len_utf8()).ok_or(Error::Limit(Limit::Storage))?;
    if needed > out.capacity() {
        let capacity = needed.max(out.capacity().saturating_mul(2));
        budget.retained = budget.retained.checked_sub(capacity.saturating_sub(out.capacity())).ok_or(Error::Limit(Limit::Storage))?;
        out.try_reserve_exact(capacity.saturating_sub(out.len())).map_err(|_| Error::Limit(Limit::Storage))?;
    }
    out.push(ch);
    Ok(())
}
#[derive(Debug)]
enum Token {
    Word(String),
    Equal,
    Open,
    Close,
    Consumed,
}

// Public chain syntax uses inert shell quoting: no variable, command or path expansion.
fn tokenize(raw: &str, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut at = 0;
    let mut next_check = 0;
    let next = |at| raw.get(at..).and_then(|text| text.chars().next());
    while let Some(ch) = next(at) {
        if at >= next_check {
            check(keep_going)?;
            next_check = at.saturating_add(1024);
        }
        if ch.is_ascii_whitespace() {
            at += ch.len_utf8();
            continue;
        }
        if ch == '#' && (at == 0 || raw.as_bytes().get(at - 1).is_some_and(u8::is_ascii_whitespace)) {
            while next(at).is_some_and(|ch| ch != '\n') {
                if at >= next_check {
                    check(keep_going)?;
                    next_check = at.saturating_add(1024);
                }
                at += next(at).ok_or_else(|| invalid("graph comment boundary"))?.len_utf8();
            }
            continue;
        }
        if matches!(ch, '=' | '[' | ']') {
            if tokens.len() >= MAX_TOKENS {
                return Err(invalid("graph exceeds 2048 lexical items"));
            }
            push(
                &mut tokens,
                match ch {
                    '=' => Token::Equal,
                    '[' => Token::Open,
                    _ => Token::Close,
                },
                budget,
            )?;
            at += 1;
            continue;
        }
        let mut word = String::new();
        let mut started = false;
        while let Some(ch) = next(at) {
            if at >= next_check {
                check(keep_going)?;
                next_check = at.saturating_add(1024);
            }
            if ch.is_ascii_whitespace() || matches!(ch, '=' | '[' | ']') {
                break;
            }
            if matches!(ch, '\'' | '"') {
                started = true; // Explicit empty quotes still contribute a word.
                let quote = ch;
                at += ch.len_utf8();
                loop {
                    if at >= next_check {
                        check(keep_going)?;
                        next_check = at.saturating_add(1024);
                    }
                    let ch = next(at).ok_or_else(|| invalid("unterminated graph quoted literal"))?;
                    at += ch.len_utf8();
                    if ch == quote {
                        break;
                    }
                    if quote == '"' && ch == '\\' {
                        let escaped = next(at).ok_or_else(|| invalid("unterminated graph quoted escape"))?;
                        if matches!(escaped, '$' | '`' | '"' | '\\' | '\n') {
                            at += escaped.len_utf8();
                            // GLib's public shell parser keeps a quoted escaped newline;
                            // only an unquoted backslash-newline is a continuation.
                            character(&mut word, escaped, budget)?;
                        } else {
                            character(&mut word, '\\', budget)?;
                        }
                    } else {
                        character(&mut word, ch, budget)?;
                    }
                }
            } else if ch == '\\' {
                at += 1;
                let escaped = next(at).ok_or_else(|| invalid("unterminated graph literal escape"))?;
                at += escaped.len_utf8();
                if escaped != '\n' {
                    started = true;
                    character(&mut word, escaped, budget)?;
                }
            } else {
                started = true;
                character(&mut word, ch, budget)?;
                at += ch.len_utf8();
            }
        }
        if started {
            if tokens.len() >= MAX_TOKENS {
                return Err(invalid("graph exceeds 2048 lexical items"));
            }
            push(&mut tokens, Token::Word(word), budget)?;
        }
    }
    Ok(tokens)
}

struct Decoder<'b, 's> {
    tokens: Vec<Token>,
    cursor: usize,
    declarations: &'s [Operation<'s>],
    budget: &'b mut Budget,
    keep_going: &'b dyn Fn() -> bool,
    nodes: Vec<Node>,
    labels: Vec<Label>,
    diagnostics: Vec<Diagnostic>,
    source_nodes: usize,
    assignments: usize,
}
impl<'b, 's> Decoder<'b, 's> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.cursor)
    }
    fn assignment(&self) -> bool {
        matches!(self.tokens.get(self.cursor.saturating_add(1)), Some(Token::Equal))
    }
    fn take(&mut self) -> Result<Token> {
        let token = self.tokens.get_mut(self.cursor).ok_or_else(|| invalid("incomplete graph construction"))?;
        self.cursor = self.cursor.saturating_add(1);
        Ok(std::mem::replace(token, Token::Consumed))
    }
    fn word(&mut self) -> Result<String> {
        match self.take()? {
            Token::Word(word) => Ok(word),
            _ => Err(invalid("expected graph word")),
        }
    }
    fn equal(&mut self) -> Result<()> {
        if matches!(self.take()?, Token::Equal) { Ok(()) } else { Err(invalid("expected graph assignment")) }
    }
    fn name(&mut self) -> Result<String> {
        let word = self.word()?;
        if word.is_empty() || word.len() > 128 || word.chars().any(|ch| ch.is_whitespace() || ch.is_control() || matches!(ch, '=' | '[' | ']')) {
            return Err(invalid("graph name must have 1..128 UTF-8 bytes without structural delimiters"));
        }
        Ok(word)
    }
    fn declaration(&mut self, operation: &str) -> Result<Option<Operation<'s>>> {
        // GEGL's default namespace is optional. Other namespaces stay exact, and the
        // node continues to retain the operation spelling written in the source.
        let unqualified = if operation.contains(':') { operation.strip_prefix("gegl:").filter(|name| !name.contains(':')) } else { Some(operation) };
        for (index, declaration) in self.declarations.iter().enumerate() {
            if index % 64 == 0 {
                check(self.keep_going)?;
            }
            self.budget.work.charge(declaration.name.len().min(operation.len()).saturating_add(1).saturating_mul(3))?;
            if declaration.name == operation
                || unqualified.is_some_and(|name| declaration.name == name || declaration.name.strip_prefix("gegl:") == Some(name))
            {
                return Ok(Some(*declaration));
            }
        }
        Ok(None)
    }
    fn kind(&mut self, declaration: Option<Operation<'s>>, name: &str) -> Result<Option<FieldType<'s>>> {
        if let Some(declaration) = declaration {
            if declaration.fields.len() > 4096 {
                return Err(invalid("graph declaration exceeds 4096 fields"));
            }
            for (index, field) in declaration.fields.iter().enumerate() {
                if index % 64 == 0 {
                    check(self.keep_going)?;
                }
                self.budget.work.charge(field.name.len().min(name.len()).saturating_add(1))?;
                if field.name == name {
                    return Ok(Some(field.kind));
                }
            }
        }
        Ok(None)
    }
    fn block(&mut self, mut current: Option<Input>, depth: usize) -> Result<Input> {
        if depth > MAX_DEPTH {
            return Err(invalid("graph exceeds 8 auxiliary nesting levels"));
        }
        while self.peek().is_some() {
            check(self.keep_going)?;
            if matches!(self.peek(), Some(Token::Close)) {
                if depth == 0 {
                    return Err(invalid("unmatched graph closing bracket"));
                }
                break;
            }
            let special = match self.peek() {
                Some(Token::Word(word)) => word.as_str(),
                _ => "",
            };
            if matches!(special, "id" | "ref") {
                let defining = special == "id";
                self.word()?;
                self.equal()?;
                let name = self.name()?;
                if defining {
                    let input = current.ok_or_else(|| invalid("graph label has no preceding input"))?;
                    if self.labels.len() >= MAX_NODES {
                        return Err(invalid("excessive graph labels"));
                    }
                    for label in &self.labels {
                        self.budget.work.charge(label.name.len().min(name.len()).saturating_add(1))?;
                        if label.name == name {
                            return Err(invalid("duplicate graph label"));
                        }
                    }
                    push(&mut self.labels, Label { name, input }, self.budget)?;
                } else {
                    let mut found = None;
                    for label in &self.labels {
                        self.budget.work.charge(label.name.len().min(name.len()).saturating_add(1))?;
                        if label.name == name {
                            found = Some(label.input);
                            break;
                        }
                    }
                    current = Some(found.ok_or_else(|| invalid("graph reference is missing, forward or cyclic"))?);
                }
                continue;
            }
            let operation = self.name()?;
            if self.source_nodes >= MAX_NODES {
                return Err(invalid("graph exceeds 64 source nodes"));
            }
            let source_ordinal = self.source_nodes;
            self.source_nodes += 1;
            let declaration = self.declaration(&operation)?;
            if declaration.is_none() {
                push(&mut self.diagnostics, Diagnostic::UnknownOperation { source_ordinal }, self.budget)?;
            }
            let mut properties: Vec<Property> = Vec::new();
            while self.assignment() {
                check(self.keep_going)?;
                if matches!(self.peek(), Some(Token::Word(word)) if matches!(word.as_str(), "id" | "ref")) {
                    break;
                }
                let name = self.name()?;
                self.equal()?;
                if self.assignments >= MAX_ARGUMENTS {
                    return Err(invalid("excessive graph assignments"));
                }
                for property in &properties {
                    self.budget.work.charge(property.name.len().min(name.len()).saturating_add(1))?;
                    if property.name == name {
                        return Err(invalid("duplicate graph assignment"));
                    }
                }
                self.assignments += 1;
                let kind = self.kind(declaration, &name)?;
                if kind.is_none() {
                    push(&mut self.diagnostics, Diagnostic::UnknownProperty { source_ordinal, property: properties.len() }, self.budget)?;
                }
                let (literal, value) = if matches!(self.peek(), Some(Token::Open)) {
                    self.take()?;
                    let input = self.block(None, depth + 1)?;
                    if !matches!(self.take()?, Token::Close) {
                        return Err(invalid("unterminated graph auxiliary block"));
                    }
                    if kind.is_some_and(|kind| !matches!(kind, FieldType::Input)) {
                        return Err(invalid("scalar graph property has an auxiliary block"));
                    }
                    (String::new(), Value::Input(input))
                } else {
                    let literal = self.word()?;
                    let value = typed_value(&literal, kind, self.budget)?;
                    (literal, value)
                };
                push(&mut properties, Property { name, literal, value }, self.budget)?;
            }
            let index = self.nodes.len();
            push(&mut self.nodes, Node { source_ordinal, operation, input: current, properties }, self.budget)?;
            current = Some(Input::Node(index));
        }
        current.ok_or_else(|| invalid("graph auxiliary block has no input or operation"))
    }
}

fn typed_value(literal: &str, kind: Option<FieldType<'_>>, budget: &mut Budget) -> Result<Value> {
    Ok(match kind {
        None => Value::Unknown,
        Some(FieldType::Number) => {
            let value = literal.parse::<f64>().map_err(|_| invalid("graph scalar is not a number"))?;
            if !value.is_finite() {
                return Err(invalid("graph scalar is not finite"));
            }
            Value::Number(value)
        }
        Some(FieldType::Integer) => Value::Integer(literal.parse().map_err(|_| invalid("graph value is not an exact signed integer"))?),
        Some(FieldType::UnsignedInteger) => Value::UnsignedInteger(literal.parse().map_err(|_| invalid("graph value is not an exact u32 integer"))?),
        Some(FieldType::Boolean) => Value::Boolean(match literal {
            "true" => true,
            "false" => false,
            _ => return Err(invalid("graph boolean must be true or false")),
        }),
        Some(FieldType::Text) => Value::Text,
        Some(FieldType::Configuration) => Value::Configuration,
        Some(FieldType::Input) => return Err(invalid("graph input pad requires an auxiliary block")),
        Some(FieldType::Choice(choices)) => {
            if choices.len() > 256 {
                return Err(invalid("graph choice declaration exceeds 256 entries"));
            }
            let numeric = literal.parse::<i32>().ok();
            let mut found = None;
            let mut numeric_fallback = None;
            for (name, value) in choices {
                budget.work.charge(name.len().min(literal.len()).saturating_add(1))?;
                if *name == literal {
                    found = Some(*value);
                    break;
                }
                if Some(*value) == numeric {
                    numeric_fallback = Some(*value);
                }
            }
            Value::Choice(found.or(numeric_fallback).ok_or_else(|| invalid("unknown graph choice value"))?)
        }
        Some(FieldType::Color) => Value::Color(color(literal)?),
    })
}

fn color(value: &str) -> Result<Color> {
    if let Some(parts) = value
        .strip_prefix("rgba(")
        .and_then(|value| value.strip_suffix(')'))
        .map(|parts| (parts, 4))
        .or_else(|| value.strip_prefix("rgb(").and_then(|value| value.strip_suffix(')')).map(|parts| (parts, 3)))
    {
        let mut rgba = [0., 0., 0., 1.];
        let mut count = 0;
        for (index, item) in parts.0.split(',').enumerate() {
            let out = rgba.get_mut(index).ok_or_else(|| invalid("too many graph color components"))?;
            let value = item.trim().parse::<f64>().map_err(|_| invalid("invalid graph color component"))?;
            if !value.is_finite() || (index == 3 && !(0.0..=1.0).contains(&value)) {
                return Err(invalid("graph color exceeds finite/alpha domain"));
            }
            *out = value;
            count += 1;
        }
        if count != parts.1 {
            return Err(invalid("wrong graph color component count"));
        }
        return Ok(Color { rgba, space: ColorSpace::LinearRgb });
    }
    let mut rgba = [0, 0, 0, 255];
    if let Some(hex) = value.strip_prefix('#') {
        if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid("invalid graph hexadecimal color"));
        }
        match hex.len() {
            3 | 4 => {
                for (out, digit) in rgba.iter_mut().zip(hex.chars()) {
                    *out = digit.to_digit(16).ok_or_else(|| invalid("invalid graph hexadecimal color"))? as u8 * 17;
                }
            }
            6 | 8 => {
                for (index, out) in rgba.iter_mut().take(hex.len() / 2).enumerate() {
                    *out = hex
                        .get(index * 2..index * 2 + 2)
                        .and_then(|part| u8::from_str_radix(part, 16).ok())
                        .ok_or_else(|| invalid("invalid graph hexadecimal color"))?;
                }
            }
            _ => return Err(invalid("wrong graph hexadecimal color length")),
        }
    } else if value.eq_ignore_ascii_case("transparent") || value.eq_ignore_ascii_case("none") {
        rgba = [0; 4];
    } else {
        let named = [
            ("black", [0, 0, 0]),
            ("silver", [192, 192, 192]),
            ("gray", [128, 128, 128]),
            ("white", [255, 255, 255]),
            ("maroon", [128, 0, 0]),
            ("red", [255, 0, 0]),
            ("purple", [128, 0, 128]),
            ("fuchsia", [255, 0, 255]),
            ("green", [0, 128, 0]),
            ("lime", [0, 255, 0]),
            ("olive", [128, 128, 0]),
            ("yellow", [255, 255, 0]),
            ("navy", [0, 0, 128]),
            ("blue", [0, 0, 255]),
            ("teal", [0, 128, 128]),
            ("aqua", [0, 255, 255]),
        ];
        let rgb = named
            .iter()
            .find(|(name, _)| value.eq_ignore_ascii_case(name))
            .map(|(_, rgb)| *rgb)
            .ok_or_else(|| invalid("unsupported graph color literal"))?;
        rgba[..3].copy_from_slice(&rgb);
    }
    Ok(Color { rgba: rgba.map(|value| f64::from(value) / 255.), space: ColorSpace::Srgb })
}
