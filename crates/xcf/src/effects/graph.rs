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
    /// Finite source components, including alpha, without destination clamping.
    pub rgba: [f64; 4],
    pub space: ColorSpace,
}

/// Text/configuration/unknown values live in `Property::literal`, avoiding a
/// second copy. Numeric literals keep that source text as well as their value.
#[derive(Debug, PartialEq)]
pub enum Value {
    Number(f64),
    /// A finite `rel` coefficient, not yet scaled by the consumer's reference dimension.
    /// Applies to any declared numeric field without rounding to its eventual integer type.
    RelativeNumber(f64),
    Integer(i64),
    UnsignedInteger(u32),
    Boolean(bool),
    Choice(i32),
    Text,
    Color(Color),
    /// Source CMYK components and alpha in their written units. No profile conversion,
    /// component normalization, clamping or native color policy is applied here.
    CmykColor {
        cmyk: [f64; 4],
        alpha: f64,
    },
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
    UnknownOperation {
        source_ordinal: usize,
    },
    UnknownProperty {
        source_ordinal: usize,
        property: usize,
    },
    /// A known integer field retains its literal without a guessed value when absolute
    /// numeric syntax or destination range is unsupported by this bounded reader.
    UnsupportedInteger {
        source_ordinal: usize,
        property: usize,
    },
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
            // The root already has its source input; an empty/comment-only chain
            // passes it through. Still observe cancellation after scanning the source.
            check(keep_going)?;
        }
        let mut decoder = Decoder {
            tokens,
            cursor: 0,
            declarations,
            budget,
            keep_going,
            nodes: Vec::new(),
            labels: Vec::new(),
            label_sources: Vec::new(),
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

fn token(tokens: &mut Vec<Token>, value: Token, budget: &mut Budget) -> Result<()> {
    if tokens.len() >= MAX_TOKENS {
        return Err(invalid("graph exceeds 2048 lexical items"));
    }
    push(tokens, value, budget)
}

/// Emit a decoded shell word, splitting only a whole-word assignment. A word
/// already lexed as a scalar value must retain every embedded '=' literally.
fn word_tokens(
    tokens: &mut Vec<Token>,
    mut word: String,
    scalar: bool,
    block_value: bool,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<()> {
    if !scalar && let Some(equal) = word.find('=') {
        // Shell quoting may surround an entire assignment, not just its value.
        // Copy the bounded name fallibly, then keep the value in the original
        // prepaid string allocation. Never split a second '=' in the value.
        let mut name = String::new();
        for (index, ch) in word.get(..equal).ok_or_else(|| invalid("invalid graph assignment boundary"))?.chars().enumerate() {
            if index % 1024 == 0 {
                check(keep_going)?;
            }
            character(&mut name, ch, budget)?;
        }
        let start = equal.checked_add(1).ok_or_else(|| invalid("invalid graph assignment boundary"))?;
        let mut offset = 0usize;
        word.retain(|ch| {
            let keep = offset >= start;
            offset = offset.saturating_add(ch.len_utf8());
            keep
        });
        token(tokens, Token::Word(name), budget)?;
        token(tokens, Token::Equal, budget)?;
        if !block_value {
            token(tokens, Token::Word(word), budget)?;
        }
    } else {
        token(tokens, Token::Word(word), budget)?;
    }
    Ok(())
}

fn word_character(word: &mut String, ch: char, equal: &mut bool, value: &mut bool, budget: &mut Budget) -> Result<()> {
    if *equal {
        *value = true;
    } else if ch == '=' {
        *equal = true;
    }
    character(word, ch, budget)
}

// Public chain syntax uses inert shell quoting: no variable, command or path expansion.
// Only the first assignment '=' separates name and scalar value. Auxiliary delimiters
// apply at the value's start/end; brackets in the middle of a scalar remain literal data.
fn tokenize(raw: &str, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut at = 0;
    let mut next_check = 0;
    let mut expect_value = false;
    let next = |at| raw.get(at..).and_then(|text| text.chars().next());
    while let Some(ch) = next(at) {
        if at >= next_check {
            check(keep_going)?;
            next_check = at.saturating_add(1024);
        }
        // Remove an unquoted continuation before deciding whether the value begins
        // an auxiliary block. Escaped/quoted brackets are still handled by the word lexer.
        if raw.get(at..).is_some_and(|tail| tail.starts_with("\\\n")) {
            at += 2;
            continue;
        }
        if ch.is_ascii_whitespace() {
            if expect_value {
                token(&mut tokens, Token::Word(String::new()), budget)?;
                expect_value = false;
            }
            at += ch.len_utf8();
            continue;
        }
        if !expect_value && ch == '#' && (at == 0 || raw.as_bytes().get(at - 1).is_some_and(u8::is_ascii_whitespace)) {
            while next(at).is_some_and(|ch| ch != '\n') {
                if at >= next_check {
                    check(keep_going)?;
                    next_check = at.saturating_add(1024);
                }
                at += next(at).ok_or_else(|| invalid("graph comment boundary"))?.len_utf8();
            }
            continue;
        }
        if ch == '[' || !expect_value && matches!(ch, '=' | ']') {
            token(
                &mut tokens,
                match ch {
                    '=' => Token::Equal,
                    '[' => Token::Open,
                    _ => Token::Close,
                },
                budget,
            )?;
            expect_value = ch == '=';
            at += 1;
            continue;
        }
        let scalar = expect_value;
        expect_value = false;
        let mut word = String::new();
        let mut started = false;
        // A scalar's '=' was already emitted by the outer lexer. Empty quotes
        // still leave it awaiting its first value byte, just like a quoted prefix.
        let mut decoded_equal = scalar;
        let mut decoded_value = false;
        let mut trailing_closes = 0usize;
        while let Some(ch) = next(at) {
            if at >= next_check {
                check(keep_going)?;
                next_check = at.saturating_add(1024);
            }
            if ch.is_ascii_whitespace() || !scalar && !decoded_equal && matches!(ch, '=' | '[' | ']') || decoded_equal && !decoded_value && ch == '['
            {
                break;
            }
            if ch == '{' && (!started || decoded_equal && !decoded_value) {
                return Err(invalid("graph keyframe syntax is unsupported"));
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
                            // Quoted escaped newlines are literal; only unquoted ones continue.
                            trailing_closes = 0;
                            word_character(&mut word, escaped, &mut decoded_equal, &mut decoded_value, budget)?;
                        } else {
                            trailing_closes = 0;
                            word_character(&mut word, '\\', &mut decoded_equal, &mut decoded_value, budget)?;
                        }
                    } else {
                        trailing_closes = 0;
                        word_character(&mut word, ch, &mut decoded_equal, &mut decoded_value, budget)?;
                    }
                }
            } else if ch == '\\' {
                at += 1;
                let escaped = next(at).ok_or_else(|| invalid("unterminated graph literal escape"))?;
                at += escaped.len_utf8();
                if escaped != '\n' {
                    started = true;
                    trailing_closes = 0;
                    word_character(&mut word, escaped, &mut decoded_equal, &mut decoded_value, budget)?;
                }
            } else {
                started = true;
                trailing_closes = if (scalar || decoded_equal) && ch == ']' { trailing_closes.saturating_add(1) } else { 0 };
                word_character(&mut word, ch, &mut decoded_equal, &mut decoded_value, budget)?;
                at += ch.len_utf8();
            }
        }
        // Each removed character is an ASCII ']', so this remains a UTF-8 boundary.
        let length = word.len().checked_sub(trailing_closes).ok_or_else(|| invalid("invalid graph scalar boundary"))?;
        word.truncate(length);
        // An assignment prefix still accepts an unquoted opener while no value byte
        // followed '='. If '=' is already a token, empty quotes emit no scalar here.
        let block_value = decoded_equal && !decoded_value && next(at) == Some('[');
        if (started || scalar) && !(scalar && block_value) {
            word_tokens(&mut tokens, word, scalar, block_value, budget, keep_going)?;
        }
        for index in 0..trailing_closes {
            if index % 64 == 0 {
                check(keep_going)?;
            }
            token(&mut tokens, Token::Close, budget)?;
        }
    }
    if expect_value {
        token(&mut tokens, Token::Word(String::new()), budget)?;
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
    // Some(source ordinal) marks a label on a node whose auxiliary dependencies
    // are still being parsed. Its placeholder input must never resolve a reference.
    label_sources: Vec<Option<usize>>,
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
    fn declaration(
        declarations: &'s [Operation<'s>],
        operation: &str,
        budget: &mut Budget,
        keep_going: &dyn Fn() -> bool,
    ) -> Result<Option<Operation<'s>>> {
        // GEGL's default namespace is optional. Other namespaces stay exact, and the
        // node continues to retain the operation spelling written in the source.
        let unqualified = if operation.contains(':') { operation.strip_prefix("gegl:").filter(|name| !name.contains(':')) } else { Some(operation) };
        for (index, declaration) in declarations.iter().enumerate() {
            if index % 64 == 0 {
                check(keep_going)?;
            }
            budget.work.charge(declaration.name.len().min(operation.len()).saturating_add(1).saturating_mul(3))?;
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
    fn define_label(&mut self, name: String, input: Input, pending_source: Option<usize>) -> Result<()> {
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
        push(&mut self.label_sources, pending_source, self.budget)
    }
    fn reference(&mut self, name: &str) -> Result<Input> {
        for (index, label) in self.labels.iter().enumerate() {
            self.budget.work.charge(label.name.len().min(name.len()).saturating_add(1))?;
            if label.name == name {
                if self.label_sources.get(index).is_some_and(Option::is_some) {
                    return Err(invalid("graph reference is missing, forward or cyclic"));
                }
                return Ok(label.input);
            }
        }
        Err(invalid("graph reference is missing, forward or cyclic"))
    }
    fn finish_labels(&mut self, source_ordinal: usize, index: usize) -> Result<()> {
        for (label, pending) in self.labels.iter_mut().zip(&mut self.label_sources) {
            self.budget.work.charge(1)?;
            if *pending == Some(source_ordinal) {
                label.input = Input::Node(index);
                *pending = None;
            }
        }
        Ok(())
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
                    self.define_label(name, input, None)?;
                } else {
                    current = Some(self.reference(&name)?);
                }
                continue;
            }
            if self.assignment() {
                let Some(Input::Node(index)) = current else { return Err(invalid("graph assignment has no referenced operation")) };
                let node = self.nodes.get_mut(index).ok_or_else(|| invalid("missing referenced graph operation"))?;
                let declaration = Self::declaration(self.declarations, &node.operation, self.budget, self.keep_going)?;
                let source_ordinal = node.source_ordinal;
                let mut properties = std::mem::take(&mut node.properties);
                self.properties(&mut properties, declaration, source_ordinal, None)?;
                self.nodes.get_mut(index).ok_or_else(|| invalid("missing referenced graph operation"))?.properties = properties;
                continue;
            }
            let operation = self.name()?;
            if self.source_nodes >= MAX_NODES {
                return Err(invalid("graph exceeds 64 source nodes"));
            }
            let source_ordinal = self.source_nodes;
            self.source_nodes += 1;
            let declaration = Self::declaration(self.declarations, &operation, self.budget, self.keep_going)?;
            if declaration.is_none() {
                push(&mut self.diagnostics, Diagnostic::UnknownOperation { source_ordinal }, self.budget)?;
            }
            let mut properties = Vec::new();
            self.properties(&mut properties, declaration, source_ordinal, Some(depth))?;
            let index = self.nodes.len();
            push(&mut self.nodes, Node { source_ordinal, operation, input: current, properties }, self.budget)?;
            self.finish_labels(source_ordinal, index)?;
            current = Some(Input::Node(index));
        }
        current.ok_or_else(|| invalid("graph auxiliary block has no input or operation"))
    }
    /// A new node may bind labels and introduce auxiliary dependencies. A
    /// resolved node accepts scalar edits only, preserving dependency order.
    fn properties(
        &mut self,
        properties: &mut Vec<Property>,
        declaration: Option<Operation<'s>>,
        source_ordinal: usize,
        depth: Option<usize>,
    ) -> Result<()> {
        while self.assignment() {
            check(self.keep_going)?;
            if matches!(self.peek(), Some(Token::Word(word)) if word == "ref") {
                break;
            }
            if matches!(self.peek(), Some(Token::Word(word)) if word == "id") {
                if depth.is_none() {
                    break; // The block already has the resolved input for this label.
                }
                self.word()?;
                self.equal()?;
                let name = self.name()?;
                // Finalize the index only after the pending node's dependencies.
                self.define_label(name, Input::Source, Some(source_ordinal))?;
                continue;
            }
            let name = self.name()?;
            self.equal()?;
            if self.assignments >= MAX_ARGUMENTS {
                return Err(invalid("excessive graph assignments"));
            }
            let mut slot = properties.len();
            for (index, property) in properties.iter().enumerate() {
                self.budget.work.charge(property.name.len().min(name.len()).saturating_add(1))?;
                if property.name == name {
                    if depth.is_some() {
                        return Err(invalid("duplicate graph assignment"));
                    }
                    if matches!(property.value, Value::Input(_)) {
                        return Err(invalid("a scalar assignment cannot replace an auxiliary input"));
                    }
                    slot = index;
                    break;
                }
            }
            self.assignments += 1;
            let kind = self.kind(declaration, &name)?;
            if slot < properties.len() {
                // A resolved scalar reassignment replaces the value and its
                // diagnostic. Graph::raw retains the original source sequence.
                self.budget.work.charge(self.diagnostics.len())?;
                self.diagnostics.retain(|entry| {
                    !matches!(entry,
                        Diagnostic::UnknownProperty { source_ordinal: owner, property }
                        | Diagnostic::UnsupportedInteger { source_ordinal: owner, property }
                        if *owner == source_ordinal && *property == slot
                    )
                });
            }
            if kind.is_none() {
                push(&mut self.diagnostics, Diagnostic::UnknownProperty { source_ordinal, property: slot }, self.budget)?;
            }
            let (literal, value) = if matches!(self.peek(), Some(Token::Open)) {
                let Some(depth) = depth else { return Err(invalid("a referenced graph node cannot acquire an auxiliary block")) };
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
                let value = typed_value(&literal, kind, self.budget, self.keep_going)?;
                check(self.keep_going)?;
                (literal, value)
            };
            if matches!(kind, Some(FieldType::Integer | FieldType::UnsignedInteger)) && matches!(value, Value::Unknown) {
                push(&mut self.diagnostics, Diagnostic::UnsupportedInteger { source_ordinal, property: slot }, self.budget)?;
            }
            let property = Property { name, literal, value };
            if let Some(previous) = properties.get_mut(slot) {
                *previous = property;
            } else {
                push(properties, property, self.budget)?;
            }
        }
        Ok(())
    }
}

fn typed_value(literal: &str, kind: Option<FieldType<'_>>, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<Value> {
    let numeric = matches!(kind, Some(FieldType::Number | FieldType::Integer | FieldType::UnsignedInteger));
    let number = if numeric { literal.trim_ascii() } else { literal };
    if numeric && let Some(coefficient) = number.strip_suffix("rel") {
        let coefficient = coefficient.parse::<f64>().map_err(|_| invalid("relative graph coefficient is not a number"))?;
        if !coefficient.is_finite() {
            return Err(invalid("relative graph coefficient is not finite"));
        }
        return Ok(Value::RelativeNumber(coefficient));
    }
    // A numeric coefficient followed by malformed relative syntax is not an absolute
    // integer fallback. Keep its existing syntax error instead of disguising it as data.
    if matches!(kind, Some(FieldType::Integer | FieldType::UnsignedInteger))
        && number.find("rel").is_some_and(|at| number.get(..at).is_some_and(|prefix| prefix.parse::<f64>().is_ok()))
    {
        return Err(invalid("malformed relative graph coefficient"));
    }
    Ok(match kind {
        None => Value::Unknown,
        Some(FieldType::Number) => {
            let value = number.parse::<f64>().map_err(|_| invalid("graph scalar is not a number"))?;
            if !value.is_finite() {
                return Err(invalid("graph scalar is not finite"));
            }
            Value::Number(value)
        }
        Some(FieldType::Integer) => signed_integer(number).map_or(Value::Unknown, Value::Integer),
        Some(FieldType::UnsignedInteger) => unsigned_integer(number).map_or(Value::Unknown, Value::UnsignedInteger),
        // GEGL 0.4.62's public chain API uses this explicit true set, not general
        // case folding (for example, "on" is true but "ON" is false).
        Some(FieldType::Boolean) => Value::Boolean(matches!(literal, "true" | "True" | "TRUE" | "1" | "yes" | "Yes" | "YES" | "on" | "y" | "Y")),
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
            for (index, (name, value)) in choices.iter().enumerate() {
                if index % 64 == 0 {
                    check(keep_going)?;
                }
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
        Some(FieldType::Color) => match cmyk_color(literal)? {
            Some(value) => value,
            None => Value::Color(color(literal)?),
        },
    })
}

/// GEGL parses decimal/scientific integer spellings through a double and truncates
/// toward zero. Exact integer text keeps its full source precision before that fallback.
/// Reject nonfinite/overflow values rather than reproducing platform-dependent C casts.
fn integer_scalar(literal: &str) -> Result<f64> {
    let value = literal.parse::<f64>().map_err(|_| invalid("graph integer is not a finite number"))?;
    if !value.is_finite() {
        return Err(invalid("graph integer is not finite"));
    }
    Ok(value.trunc())
}

fn exact_integer_spelling(literal: &str) -> bool {
    let digits = literal.strip_prefix('+').or_else(|| literal.strip_prefix('-')).unwrap_or(literal);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn signed_integer(literal: &str) -> Result<i64> {
    if let Ok(value) = literal.parse() {
        return Ok(value);
    }
    // A rejected exact integer must not round back into range through f64. Decimal
    // and scientific spellings still use their separately checked numeric fallback.
    if exact_integer_spelling(literal) {
        return Err(invalid("graph exact signed integer exceeds i64 range"));
    }
    let value = integer_scalar(literal)?;
    // i64::MAX rounds up to 2^63 as f64, so the upper comparison must be exclusive.
    if !(i64::MIN as f64..-(i64::MIN as f64)).contains(&value) {
        return Err(invalid("graph signed integer exceeds i64 range"));
    }
    Ok(value as i64)
}

fn unsigned_integer(literal: &str) -> Result<u32> {
    if let Ok(value) = literal.parse() {
        return Ok(value);
    }
    if !literal.starts_with('-') && exact_integer_spelling(literal) {
        return Err(invalid("graph exact unsigned integer exceeds u32 range"));
    }
    // Preserve the existing checked negative-zero/fractional fallback; negative
    // values whose truncated result is below zero remain unsupported.
    let value = integer_scalar(literal)?;
    if !(0.0..=f64::from(u32::MAX)).contains(&value) {
        return Err(invalid("graph unsigned integer exceeds u32 range"));
    }
    Ok(value as u32)
}

/// GEGL color functions have an ASCII-case-insensitive name and may separate it
/// from '(' with ASCII whitespace. Keep component parsing and source units separate.
fn color_function(literal: &str) -> Option<(&str, &str)> {
    // Trim only the scanner view for functions. Hex/named colors still inspect the
    // original literal, and Property::literal keeps every source whitespace byte.
    let literal = literal.trim_matches(|ch: char| ch.is_ascii_whitespace());
    let (name, arguments) = literal.split_once('(')?;
    let name = name.trim_end_matches(|ch: char| ch.is_ascii_whitespace());
    Some((name, arguments.strip_suffix(')')?))
}

fn cmyk_color(literal: &str) -> Result<Option<Value>> {
    let Some((name, parts)) = color_function(literal) else { return Ok(None) };
    let count = if name.eq_ignore_ascii_case("cmyka") {
        5
    } else if name.eq_ignore_ascii_case("cmyk") {
        4
    } else {
        return Ok(None);
    };
    let mut components: [f64; 5] = [0., 0., 0., 0., 1.];
    let mut parsed = 0;
    for (index, item) in parts.split(',').enumerate() {
        let component = components.get_mut(index).ok_or_else(|| invalid("too many graph CMYK components"))?;
        *component = item.trim().parse().map_err(|_| invalid("invalid graph CMYK component"))?;
        if !component.is_finite() {
            return Err(invalid("graph CMYK component is not finite"));
        }
        parsed += 1;
    }
    if parsed != count {
        return Err(invalid("wrong graph CMYK component count"));
    }
    let [c, m, y, k, alpha] = components;
    Ok(Some(Value::CmykColor { cmyk: [c, m, y, k], alpha }))
}

fn color(value: &str) -> Result<Color> {
    if let Some(parts) = color_function(value).and_then(|(name, parts)| {
        if name.eq_ignore_ascii_case("rgba") {
            Some((parts, 4))
        } else if name.eq_ignore_ascii_case("rgb") {
            Some((parts, 3))
        } else {
            None
        }
    }) {
        let mut rgba = [0., 0., 0., 1.];
        let mut count = 0;
        for (index, item) in parts.0.split(',').enumerate() {
            let out = rgba.get_mut(index).ok_or_else(|| invalid("too many graph color components"))?;
            let value = item.trim().parse::<f64>().map_err(|_| invalid("invalid graph color component"))?;
            if !value.is_finite() {
                return Err(invalid("graph color component is not finite"));
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
