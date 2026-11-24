//! Shared PostScript format primitives for EPS programs.
//!
//! [`Scanner`] handles bounded lexical parsing. [`OperandStack`], [`ExecutionBudget`], and
//! [`NumericOperator`] provide reusable language-level VM behavior while leaving values,
//! dictionaries, graphics state, rendering, and font policy to the host interpreter.

/// Maximum nesting depth for executable procedures in a source token.
pub const MAX_PROCEDURE_DEPTH: usize = 100;

/// Maximum number of operand values an interpreter should retain by default.
pub const MAX_OPERAND_STACK: usize = 100_000;

/// Maximum nested executable procedure calls supported by the bounded VM.
pub const MAX_CALL_DEPTH: usize = 256;

/// PostScript operand-stack operators whose semantics do not depend on the value model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackOperator {
    Pop,
    Exchange,
    Duplicate,
    CopyTop(usize),
    Index(usize),
    Roll { count: usize, shift: i64 },
    Clear,
    Count,
}

/// Result of applying an operand-stack operator.
#[derive(Debug, PartialEq, Eq)]
pub enum StackOutcome<T> {
    None,
    Popped(Option<T>),
    Count(usize),
    Copied(bool),
}

/// A bounded PostScript operand stack.
///
/// The stack is deliberately generic: an interpreter owns the concrete PostScript values and
/// graphics state, while this module provides the shared language-level stack operators. A push
/// past `limit` is ignored, matching the bounded EPS interpreter's defensive behavior.
#[derive(Clone, Debug)]
pub struct OperandStack<T> {
    values: Vec<T>,
    limit: usize,
}

impl<T: Clone> OperandStack<T> {
    /// Create an empty stack with a maximum number of values.
    pub fn new(limit: usize) -> Self {
        Self { values: Vec::new(), limit }
    }

    /// Number of values currently on the stack.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Borrow all stack values mutably from bottom to top.
    pub fn values_mut(&mut self) -> &mut [T] {
        &mut self.values
    }

    /// Remove and return the top value, if present.
    pub fn pop(&mut self) -> Option<T> {
        self.values.pop()
    }

    /// Remove every value.
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Remove values at and above `at`, returning them in bottom-to-top order. An index beyond
    /// the stack length removes nothing.
    pub fn split_off(&mut self, at: usize) -> Vec<T> {
        let at = at.min(self.values.len());
        self.values.split_off(at)
    }

    /// Push a value if the configured limit allows it; report whether it was pushed.
    pub fn push(&mut self, value: T) -> bool {
        if self.values.len() >= self.limit {
            return false;
        }
        self.values.push(value);
        true
    }

    /// Pop a value, or return the caller's language-specific underflow value.
    pub fn pop_or(&mut self, underflow: T) -> T {
        self.values.pop().unwrap_or(underflow)
    }

    /// Borrow the top value.
    pub fn top(&self) -> Option<&T> {
        self.values.last()
    }

    /// PostScript `dup`: duplicate the top value if one exists.
    pub fn duplicate(&mut self) {
        if let Some(value) = self.values.last().cloned() {
            let _ = self.push(value);
        }
    }

    /// PostScript `dup` with a caller-supplied underflow value.
    pub fn duplicate_or(&mut self, underflow: T) {
        let value = self.values.last().cloned().unwrap_or(underflow);
        let _ = self.push(value);
    }

    /// PostScript `exch`: exchange the top two values when both exist.
    pub fn exchange(&mut self) {
        let len = self.values.len();
        if len >= 2 {
            self.values.swap(len - 1, len - 2);
        }
    }

    /// PostScript `exch` with caller-supplied underflow values. Both operands are popped and
    /// restored even if either pop underflows, matching interpreters that use a null sentinel.
    pub fn exchange_or(&mut self, underflow: T) {
        let b = self.pop_or(underflow.clone());
        let a = self.pop_or(underflow);
        let _ = self.push(b);
        let _ = self.push(a);
    }

    /// PostScript `index`: copy the value `depth` below the top onto the stack.
    ///
    /// Invalid or negative depths are left to the caller to validate; an out-of-range depth is a
    /// no-op. This keeps the helper bounded even for hostile numeric operands.
    pub fn index(&mut self, depth: usize) {
        let len = self.values.len();
        if depth < len && len < self.limit {
            let value = self.values[len - depth - 1].clone();
            self.values.push(value);
        }
    }

    /// PostScript `index` with a caller-supplied underflow value, pushed even when `depth` is out
    /// of range. The configured stack limit still applies.
    pub fn index_or(&mut self, depth: usize, underflow: T) {
        let len = self.values.len();
        let value = if depth < len { self.values[len - depth - 1].clone() } else { underflow };
        let _ = self.push(value);
    }

    /// PostScript `n j roll`: rotate the top `count` values by signed `shift` places.
    ///
    /// Positive shifts move values toward the top; negative shifts move them toward the bottom.
    /// Counts larger than the stack and empty rolls are ignored.
    pub fn roll(&mut self, count: usize, shift: i64) {
        let len = self.values.len();
        if count == 0 || count > len {
            return;
        }
        let places = (i128::from(shift).rem_euclid(count as i128)) as usize;
        self.values[len - count..].rotate_right(places);
    }

    /// PostScript `copy` for an integer operand: append a copy of the top `count` values if it
    /// fits. Returns false when the range is invalid or the configured limit would be exceeded.
    pub fn copy_top(&mut self, count: usize) -> bool {
        let len = self.values.len();
        if count > len || len.saturating_add(count) > self.limit {
            return false;
        }
        let copied = self.values[len - count..].to_vec();
        self.values.extend(copied);
        true
    }

    /// Borrow all stack values from bottom to top.
    pub fn values(&self) -> &[T] {
        &self.values
    }

    /// Apply a PostScript operand-stack operator. `underflow` is the interpreter's fallback value
    /// for `exch`, `dup`, and `index`; the stack's configured limit remains in force.
    pub fn execute(&mut self, operator: StackOperator, underflow: T) -> StackOutcome<T> {
        match operator {
            StackOperator::Pop => StackOutcome::Popped(self.pop()),
            StackOperator::Exchange => {
                self.exchange_or(underflow);
                StackOutcome::None
            }
            StackOperator::Duplicate => {
                self.duplicate_or(underflow);
                StackOutcome::None
            }
            StackOperator::CopyTop(count) => StackOutcome::Copied(self.copy_top(count)),
            StackOperator::Index(depth) => {
                self.index_or(depth, underflow);
                StackOutcome::None
            }
            StackOperator::Roll { count, shift } => {
                self.roll(count, shift);
                StackOutcome::None
            }
            StackOperator::Clear => {
                self.clear();
                StackOutcome::None
            }
            StackOperator::Count => StackOutcome::Count(self.len()),
        }
    }
}

/// Bounded execution accounting shared by PostScript interpreters.
///
/// EPS programs can recurse or repeatedly execute procedures. A host can use this counter to
/// stop work before it consumes unbounded CPU, independently of its operator and graphics policy.
#[derive(Clone, Debug)]
pub struct ExecutionBudget {
    remaining: usize,
    call_depth: usize,
    max_call_depth: usize,
}

impl ExecutionBudget {
    /// Create a budget with an operation allowance and maximum nested procedure calls.
    pub fn new(operations: usize, max_call_depth: usize) -> Self {
        Self { remaining: operations, call_depth: 0, max_call_depth }
    }

    /// Units of work still available.
    pub fn remaining(&self) -> usize {
        self.remaining
    }

    /// Current nested procedure depth.
    pub fn call_depth(&self) -> usize {
        self.call_depth
    }

    /// Charge work atomically. On failure the remaining budget stays unchanged.
    pub fn spend(&mut self, units: usize) -> bool {
        let Some(remaining) = self.remaining.checked_sub(units) else {
            return false;
        };
        self.remaining = remaining;
        true
    }

    /// Charge work that is accounted lazily: saturate at zero so the caller's next strict
    /// [`Self::spend`] stops execution. This supports work whose current operator preserves the
    /// app's historical behavior after exhausting its budget.
    pub fn charge(&mut self, units: usize) {
        self.remaining = self.remaining.saturating_sub(units);
    }

    /// Enter a procedure call if it remains within the configured depth.
    pub fn enter_call(&mut self) -> bool {
        if self.call_depth >= self.max_call_depth {
            return false;
        }
        self.call_depth += 1;
        true
    }

    /// Leave a procedure call. Extra leaves are ignored to keep cleanup paths safe.
    pub fn leave_call(&mut self) {
        self.call_depth = self.call_depth.saturating_sub(1);
    }
}

/// Numeric operators used by PostScript's value-independent arithmetic core.
///
/// The host interpreter remains responsible for popping operands and selecting a result value,
/// but these operations define the shared arithmetic semantics. Non-finite results become zero,
/// matching the defensive finite-value policy used by the host interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    IntegerDivide,
    Modulo,
    Atan,
    Exponentiate,
    Maximum,
    Minimum,
    Negate,
    Absolute,
    SquareRoot,
    Sine,
    Cosine,
    NaturalLog,
    CommonLog,
    ToInteger,
    ToReal,
    Round,
    Floor,
    Ceiling,
    Truncate,
}

impl NumericOperator {
    /// Resolve a standard PostScript operator name to its numeric operation.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "add" => Self::Add,
            "sub" => Self::Subtract,
            "mul" => Self::Multiply,
            "div" => Self::Divide,
            "idiv" => Self::IntegerDivide,
            "mod" => Self::Modulo,
            "atan" => Self::Atan,
            "exp" => Self::Exponentiate,
            "max" => Self::Maximum,
            "min" => Self::Minimum,
            "neg" => Self::Negate,
            "abs" => Self::Absolute,
            "sqrt" => Self::SquareRoot,
            "sin" => Self::Sine,
            "cos" => Self::Cosine,
            "ln" => Self::NaturalLog,
            "log" => Self::CommonLog,
            "cvi" => Self::ToInteger,
            "cvr" => Self::ToReal,
            "round" => Self::Round,
            "floor" => Self::Floor,
            "ceiling" => Self::Ceiling,
            "truncate" => Self::Truncate,
            _ => return None,
        })
    }

    /// Number of operands the operation consumes.
    pub const fn arity(self) -> usize {
        match self {
            Self::Add
            | Self::Subtract
            | Self::Multiply
            | Self::Divide
            | Self::IntegerDivide
            | Self::Modulo
            | Self::Atan
            | Self::Exponentiate
            | Self::Maximum
            | Self::Minimum => 2,
            _ => 1,
        }
    }

    /// Apply a binary operator. Returns `None` for an operator that takes one operand.
    pub fn apply_binary(self, a: f64, b: f64) -> Option<f64> {
        let value = match self {
            Self::Add => a + b,
            Self::Subtract => a - b,
            Self::Multiply => a * b,
            Self::Divide => {
                if b == 0.0 {
                    0.0
                } else {
                    a / b
                }
            }
            // Rust's float-to-int cast saturates, as the app's previous VM did. checked_div/rem
            // also make both zero-divisor and i64::MIN / -1 cases deterministic.
            Self::IntegerDivide => (a as i64).checked_div(b as i64).unwrap_or(0) as f64,
            Self::Modulo => (a as i64).checked_rem(b as i64).unwrap_or(0) as f64,
            Self::Atan => a.atan2(b).to_degrees().rem_euclid(360.0),
            Self::Exponentiate => a.powf(b),
            Self::Maximum => a.max(b),
            Self::Minimum => a.min(b),
            _ => return None,
        };
        Some(finite_or_zero(value))
    }

    /// Apply a unary operator. Returns `None` for an operator that takes two operands.
    pub fn apply_unary(self, a: f64) -> Option<f64> {
        let value = match self {
            Self::Negate => -a,
            Self::Absolute => a.abs(),
            Self::SquareRoot => a.max(0.0).sqrt(),
            Self::Sine => a.to_radians().sin(),
            Self::Cosine => a.to_radians().cos(),
            Self::NaturalLog => a.ln(),
            Self::CommonLog => a.log10(),
            Self::ToInteger | Self::Truncate => a.trunc(),
            Self::ToReal => a,
            Self::Round => (a + 0.5).floor(),
            Self::Floor => a.floor(),
            Self::Ceiling => a.ceil(),
            _ => return None,
        };
        Some(finite_or_zero(value))
    }
}

fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

/// A lexical PostScript value. Names are left unevaluated for the caller's interpreter.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// A number accepted by the PostScript source reader.
    Number(f64),
    /// An executable name, including immediately evaluated `//name`.
    Name(String),
    /// A literal name such as `/name`.
    LiteralName(String),
    /// A parenthesized or hexadecimal byte string.
    String(Vec<u8>),
    /// The encoded bytes between `<~` and `~>` (including delimiters), not decoded data.
    Ascii85(Vec<u8>),
    /// A procedure whose nested values have already been tokenized.
    Procedure(Vec<Token>),
    /// A closing brace not consumed as part of a procedure, or another malformed delimiter.
    Null,
}

/// PostScript source bytes and the current byte offset.
pub struct Scanner<'a> {
    /// Source bytes. Public so an interpreter can implement `currentfile` operators.
    pub data: &'a [u8],
    /// Current byte offset in [`Scanner::data`].
    pub pos: usize,
}

impl<'a> Scanner<'a> {
    /// Start scanning at a byte offset, clamped to the source length.
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos: pos.min(data.len()) }
    }

    /// The next token; `{ … }` is returned as one procedure token.
    pub fn token(&mut self) -> Option<Token> {
        self.token_at_depth(0)
    }

    fn token_at_depth(&mut self, depth: usize) -> Option<Token> {
        self.skip_ws();
        let c = *self.data.get(self.pos)?;
        Some(match c {
            b'{' => {
                self.pos += 1;
                let mut body = Vec::new();
                loop {
                    self.skip_ws();
                    match self.data.get(self.pos) {
                        None => break,
                        Some(b'}') => {
                            self.pos += 1;
                            break;
                        }
                        _ if depth >= MAX_PROCEDURE_DEPTH => break,
                        _ => match self.token_at_depth(depth + 1) {
                            Some(t) => body.push(t),
                            None => break,
                        },
                    }
                }
                Token::Procedure(body)
            }
            b'}' => {
                self.pos += 1;
                Token::Null
            }
            b'/' => {
                self.pos += 1;
                let immediately_evaluated = self.data.get(self.pos) == Some(&b'/');
                if immediately_evaluated {
                    self.pos += 1;
                }
                let name = String::from_utf8_lossy(self.word()).into_owned();
                if immediately_evaluated { Token::Name(name) } else { Token::LiteralName(name) }
            }
            b'(' => Token::String(self.parenthesized()),
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                Token::Name("<<".to_owned())
            }
            b'>' if self.data.get(self.pos + 1) == Some(&b'>') => {
                self.pos += 2;
                Token::Name(">>".to_owned())
            }
            b'<' if self.data.get(self.pos + 1) == Some(&b'~') => self.ascii85(),
            b'<' => Token::String(self.hex_string()),
            b'[' | b']' => {
                self.pos += 1;
                Token::Name(char::from(c).to_string())
            }
            b')' | b'>' => {
                self.pos += 1;
                Token::Null
            }
            _ => {
                let word = self.word();
                if word.is_empty() {
                    self.pos += 1;
                    return Some(Token::Null);
                }
                match parse_number(word) {
                    Some(n) => Token::Number(n),
                    None => {
                        let name = String::from_utf8_lossy(word).into_owned();
                        if let Some((radix, digits)) = name.split_once('#') {
                            let radix = radix.parse::<u32>().ok().filter(|r| (2..=36).contains(r));
                            if let Some(number) = radix.and_then(|r| i64::from_str_radix(digits, r).ok()) {
                                return Some(Token::Number(number as f64));
                            }
                        }
                        Token::Name(name)
                    }
                }
            }
        })
    }

    /// Step over inline image data that follows an `image` operator reading `currentfile`.
    pub fn skip_image_data(&mut self) {
        self.skip_ws();
        let rest = self.data.get(self.pos..).unwrap_or_default();
        if rest.starts_with(b"<~") || self.data.get(self.pos).is_some_and(|c| (b'!'..=b'u').contains(c) && !c.is_ascii_hexdigit()) {
            self.pos = match rest.windows(2).position(|w| w == b"~>") {
                Some(p) => self.pos.saturating_add(p).saturating_add(2).min(self.data.len()),
                None => self.data.len(),
            };
            return;
        }
        while self.pos < self.data.len() && (self.data[self.pos].is_ascii_hexdigit() || is_white(self.data[self.pos])) {
            self.pos += 1;
        }
        if self.data.get(self.pos) == Some(&b'>') {
            self.pos += 1;
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.data.len() {
            let c = self.data[self.pos];
            if is_white(c) {
                self.pos += 1;
            } else if c == b'%' {
                let start = self.pos;
                while self.pos < self.data.len() && !matches!(self.data[self.pos], b'\n' | b'\r') {
                    self.pos += 1;
                }
                let line = &self.data[start..self.pos];
                let end_marker: Option<&[u8]> = if line.starts_with(b"%%BeginData:") {
                    Some(b"%%EndData")
                } else if line.starts_with(b"%%BeginBinary:") {
                    Some(b"%%EndBinary")
                } else if line.starts_with(b"%%BeginPreview") {
                    Some(b"%%EndPreview")
                } else {
                    None
                };
                if let Some(marker) = end_marker {
                    self.pos = match self.data.get(self.pos..).unwrap_or_default().windows(marker.len()).position(|w| w == marker) {
                        Some(offset) => self.pos.saturating_add(offset).min(self.data.len()),
                        None => self.data.len(),
                    };
                }
            } else {
                break;
            }
        }
    }

    fn word(&mut self) -> &[u8] {
        let start = self.pos;
        while self.pos < self.data.len() {
            let c = self.data[self.pos];
            if is_white(c) || matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%') {
                break;
            }
            self.pos += 1;
        }
        &self.data[start..self.pos]
    }

    fn parenthesized(&mut self) -> Vec<u8> {
        self.pos += 1;
        let mut out = Vec::new();
        let mut nesting = 1usize;
        while self.pos < self.data.len() {
            let c = self.data[self.pos];
            self.pos += 1;
            match c {
                b'\\' => {
                    let e = self.data.get(self.pos).copied().unwrap_or(b'\\');
                    self.pos = self.pos.saturating_add(1).min(self.data.len());
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'0'..=b'7' => {
                            let mut value = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.data.get(self.pos) {
                                    Some(d @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(*d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(value as u8);
                        }
                        b'\n' | b'\r' => {}
                        other => out.push(other),
                    }
                }
                b'(' => {
                    nesting = nesting.saturating_add(1);
                    out.push(c);
                }
                b')' => {
                    nesting = nesting.saturating_sub(1);
                    if nesting == 0 {
                        break;
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        out
    }

    fn hex_string(&mut self) -> Vec<u8> {
        self.pos += 1;
        let mut hex = Vec::new();
        while self.pos < self.data.len() && self.data[self.pos] != b'>' {
            if self.data[self.pos].is_ascii_hexdigit() {
                hex.push(self.data[self.pos]);
            }
            self.pos += 1;
        }
        if self.data.get(self.pos) == Some(&b'>') {
            self.pos += 1;
        }
        if hex.len() % 2 == 1 {
            hex.push(b'0');
        }
        hex.chunks(2).filter_map(|pair| Some((hex_value(*pair.first()?) << 4) | hex_value(*pair.get(1)?))).collect()
    }

    fn ascii85(&mut self) -> Token {
        let start = self.pos;
        self.pos = self.pos.saturating_add(2).min(self.data.len());
        while self.pos + 1 < self.data.len() && !(self.data[self.pos] == b'~' && self.data[self.pos + 1] == b'>') {
            self.pos += 1;
        }
        self.pos = self.pos.saturating_add(2).min(self.data.len());
        Token::Ascii85(self.data[start..self.pos].to_vec())
    }
}

fn is_white(c: u8) -> bool {
    matches!(c, 0 | 9 | 10 | 12 | 13 | 32)
}

fn hex_value(c: u8) -> u8 {
    (c as char).to_digit(16).unwrap_or(0) as u8
}

fn parse_number(word: &[u8]) -> Option<f64> {
    if word.is_empty() || !word.iter().all(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.')) || !word.iter().any(u8::is_ascii_digit) {
        return None;
    }
    let text = std::str::from_utf8(word).ok()?;
    text.parse::<f64>()
        .ok()
        .or_else(|| text.trim_start_matches(['+', '-']).parse::<f64>().ok().map(|value| if text.starts_with('-') { -value } else { value }))
}

#[cfg(test)]
mod stack_tests {
    use super::{ExecutionBudget, OperandStack, StackOperator, StackOutcome};

    #[test]
    fn stack_operators_follow_postscript_order_and_respect_limit() {
        let mut stack = OperandStack::new(8);
        assert!(stack.push(1));
        assert!(stack.push(2));
        assert!(stack.push(3));
        stack.exchange();
        assert_eq!(stack.values(), &[1, 3, 2]);
        stack.duplicate();
        assert_eq!(stack.values(), &[1, 3, 2, 2]);
        stack.roll(4, 1);
        assert_eq!(stack.values(), &[2, 1, 3, 2]);
        stack.roll(4, -1);
        assert_eq!(stack.values(), &[1, 3, 2, 2]);
        stack.index(2);
        assert_eq!(stack.values(), &[1, 3, 2, 2, 3]);
        assert!(stack.copy_top(2));
        assert_eq!(stack.values(), &[1, 3, 2, 2, 3, 2, 3]);
        stack.duplicate();
        assert_eq!(stack.len(), 8);
        assert!(!stack.push(7));
    }

    #[test]
    fn stack_underflow_and_invalid_rolls_are_safe() {
        let mut stack = OperandStack::new(2);
        assert_eq!(stack.pop_or(99), 99);
        stack.duplicate_or(99);
        assert_eq!(stack.values(), &[99]);
        stack.clear();
        stack.exchange_or(99);
        assert_eq!(stack.values(), &[99, 99]);
        stack.clear();
        stack.roll(1, i64::MAX);
        stack.index(usize::MAX);
        assert!(stack.is_empty());
        assert!(stack.copy_top(0));
        assert!(stack.is_empty());
    }

    #[test]
    fn stack_mutation_and_underflow_index_are_bounded() {
        let mut stack = OperandStack::new(4);
        assert!(stack.push(10));
        assert!(stack.push(20));
        stack.index_or(1, -1);
        stack.index_or(9, -1);
        assert_eq!(stack.values(), &[10, 20, 10, -1]);
        stack.index_or(0, -1);
        assert_eq!(stack.values(), &[10, 20, 10, -1]); // full stack silently drops the push

        assert_eq!(stack.top(), Some(&-1));
        stack.values_mut()[3] = 40;
        stack.roll(3, i64::MIN);
        assert_eq!(stack.values(), &[10, 40, 20, 10]);
        assert!(stack.split_off(usize::MAX).is_empty());
        assert_eq!(stack.split_off(1), [40, 20, 10]);
        assert_eq!(stack.values(), &[10]);
        assert_eq!(stack.pop(), Some(10));
        assert_eq!(stack.pop(), None);
        stack.clear();
        assert!(stack.is_empty());
    }

    #[test]
    fn shared_stack_operator_dispatch_reports_only_value_results() {
        let mut stack = OperandStack::new(4);
        assert!(stack.push(1));
        assert!(stack.push(2));
        assert_eq!(stack.execute(StackOperator::Count, 0), StackOutcome::Count(2));
        assert_eq!(stack.execute(StackOperator::Exchange, 0), StackOutcome::None);
        assert_eq!(stack.values(), &[2, 1]);
        assert_eq!(stack.execute(StackOperator::Duplicate, 0), StackOutcome::None);
        assert_eq!(stack.execute(StackOperator::Index(2), 0), StackOutcome::None);
        assert_eq!(stack.values(), &[2, 1, 1, 2]);
        assert_eq!(stack.execute(StackOperator::Roll { count: 3, shift: 1 }, 0), StackOutcome::None);
        assert_eq!(stack.values(), &[2, 2, 1, 1]);
        assert_eq!(stack.execute(StackOperator::CopyTop(2), 0), StackOutcome::Copied(false));
        assert_eq!(stack.execute(StackOperator::Pop, 0), StackOutcome::Popped(Some(1)));
        assert_eq!(stack.execute(StackOperator::Clear, 0), StackOutcome::None);
        assert!(stack.is_empty());
    }

    #[test]
    fn execution_budget_charges_atomically_and_bounds_call_depth() {
        let mut budget = ExecutionBudget::new(3, 1);
        assert!(budget.spend(2));
        assert_eq!(budget.remaining(), 1);
        assert!(!budget.spend(2));
        assert_eq!(budget.remaining(), 1);
        budget.charge(2);
        assert_eq!(budget.remaining(), 0);
        assert!(!budget.spend(1));
        assert!(budget.enter_call());
        assert_eq!(budget.call_depth(), 1);
        assert!(!budget.enter_call());
        budget.leave_call();
        budget.leave_call();
        assert_eq!(budget.call_depth(), 0);
        assert!(budget.enter_call());
    }
}

#[cfg(test)]
mod numeric_operator_tests {
    use super::NumericOperator as N;

    #[test]
    fn binary_numeric_operator_results_match_the_shared_vm_contract() {
        assert_eq!(N::from_name("add"), Some(N::Add));
        assert_eq!(N::from_name("idiv").map(N::arity), Some(2));
        assert_eq!(N::from_name("sqrt").map(N::arity), Some(1));
        assert_eq!(N::from_name("setgray"), None);
        assert_eq!(N::Add.apply_binary(2.0, 3.0), Some(5.0));
        assert_eq!(N::Subtract.apply_binary(2.0, 3.0), Some(-1.0));
        assert_eq!(N::Multiply.apply_binary(2.0, 3.0), Some(6.0));
        assert_eq!(N::Divide.apply_binary(7.0, 2.0), Some(3.5));
        assert_eq!(N::Divide.apply_binary(7.0, 0.0), Some(0.0));
        assert_eq!(N::IntegerDivide.apply_binary(-7.0, 2.0), Some(-3.0));
        assert_eq!(N::IntegerDivide.apply_binary(7.0, 0.0), Some(0.0));
        assert_eq!(N::IntegerDivide.apply_binary(i64::MIN as f64, -1.0), Some(0.0));
        assert_eq!(N::Modulo.apply_binary(-7.0, 2.0), Some(-1.0));
        assert_eq!(N::Modulo.apply_binary(7.0, 0.0), Some(0.0));
        assert_eq!(N::Atan.apply_binary(0.0, -1.0), Some(180.0));
        assert_eq!(N::Exponentiate.apply_binary(-2.0, 3.0), Some(-8.0));
        assert_eq!(N::Exponentiate.apply_binary(-2.0, 0.5), Some(0.0));
        assert_eq!(N::Maximum.apply_binary(2.0, 3.0), Some(3.0));
        assert_eq!(N::Minimum.apply_binary(2.0, 3.0), Some(2.0));
        assert_eq!(N::Negate.apply_binary(1.0, 2.0), None);
    }

    #[test]
    fn unary_numeric_operator_results_match_the_shared_vm_contract() {
        assert_eq!(N::Negate.apply_unary(2.0), Some(-2.0));
        assert_eq!(N::Absolute.apply_unary(-2.0), Some(2.0));
        assert_eq!(N::SquareRoot.apply_unary(-2.0), Some(0.0));
        assert_eq!(N::Sine.apply_unary(30.0).map(|v| (v - 0.5).abs() < 1e-12), Some(true));
        assert_eq!(N::Cosine.apply_unary(60.0).map(|v| (v - 0.5).abs() < 1e-12), Some(true));
        assert_eq!(N::NaturalLog.apply_unary(-1.0), Some(0.0));
        assert_eq!(N::CommonLog.apply_unary(100.0), Some(2.0));
        assert_eq!(N::ToInteger.apply_unary(-2.9), Some(-2.0));
        assert_eq!(N::ToReal.apply_unary(2.5), Some(2.5));
        assert_eq!(N::Round.apply_unary(-1.5), Some(-1.0));
        assert_eq!(N::Floor.apply_unary(1.9), Some(1.0));
        assert_eq!(N::Ceiling.apply_unary(1.1), Some(2.0));
        assert_eq!(N::Truncate.apply_unary(-2.9), Some(-2.0));
        assert_eq!(N::Add.apply_unary(1.0), None);
    }
}

#[cfg(test)]
mod tests {
    use super::{Scanner, Token};

    fn scan(source: &[u8]) -> Vec<Token> {
        let mut scanner = Scanner::new(source, 0);
        let mut tokens = Vec::new();
        while let Some(token) = scanner.token() {
            tokens.push(token);
        }
        tokens
    }

    #[test]
    fn scans_names_numbers_strings_and_procedures() {
        assert_eq!(
            scan(b"/x //y 16#ff { 2 (a\\n(b)) }"),
            vec![
                Token::LiteralName("x".into()),
                Token::Name("y".into()),
                Token::Number(255.0),
                Token::Procedure(vec![Token::Number(2.0), Token::String(b"a\n(b)".to_vec())]),
            ]
        );
    }

    #[test]
    fn skips_dsc_binary_data_and_preserves_inline_ascii85() {
        assert_eq!(scan(b"%%BeginBinary: 4\nxxxx%%EndBinary\n<~z~>"), vec![Token::Ascii85(b"<~z~>".to_vec())]);
    }

    #[test]
    fn malformed_delimiters_terminate_without_advancing_past_source() {
        let mut scanner = Scanner::new(b"<deadbeef", 0);
        assert_eq!(scanner.token(), Some(Token::String(vec![0xde, 0xad, 0xbe, 0xef])));
        assert_eq!(scanner.pos, scanner.data.len());
    }
}
