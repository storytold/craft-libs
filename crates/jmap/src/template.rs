//! Address templates: RFC 6570 level 1 (simple string expansion), as JMAP uses them for `downloadUrl`,
//! `uploadUrl` and `eventSourceUrl` (RFC 8620 §2, §6.1, §6.2, §7.3).
//!
//! A template is read once, when the session is read, and checked then; expanding it later percent-encodes
//! each value (RFC 6570 §3.2.1), resolves the result against the session address (RFC 3986 §5.2) and checks it
//! again under the transport rule.
//!
//! Stricter than RFC 6570, on purpose:
//!
//! * only level 1 is accepted: `{name}`, no operator, no list, no modifier;
//! * expressions may only stand in the path or the query. The scheme, the host and the port come from the
//!   server's literal text, never from a value; an expression in the fragment would be dropped, so it is
//!   refused too;
//! * the template must be absolute, network-path (`//host/…`) or absolute-path (`/…`): a relative-path
//!   template has no stable meaning;
//! * every variable must receive a value: an undefined variable would expand to nothing (RFC 6570 §3.2.1)
//!   and silently change the shape of the path;
//! * a value that lands in a path segment may not be empty, `.` or `..`, and no segment holding a value may
//!   end up being `.` or `..` (RFC 3986 §3.3), since dot-segment removal would move the request elsewhere.

use std::fmt;

use crate::address::{Address, AddressError, MAX_ADDRESS_LEN};
use crate::percent::{encode_component, hex_value};

/// The most expressions a template may hold.
const MAX_EXPRESSIONS: usize = 16;
/// The longest variable name accepted.
const MAX_VARIABLE_LEN: usize = 64;

/// Why a template cannot be read or expanded.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TemplateError {
    /// The template text is empty, too long, or holds characters RFC 6570 §2.1 does not allow.
    Syntax,
    /// An expression uses an operator, a list or a modifier (RFC 6570 levels 2 to 4).
    UnsupportedLevel,
    /// An expression stands in the scheme, the host, the port or the fragment.
    ExpressionOutsidePathOrQuery,
    /// A relative-path template.
    RelativeTemplate,
    /// More than the accepted number of expressions.
    TooManyExpressions,
    /// A variable the template needs has no value. The name is a short checked identifier.
    MissingValue(String),
    /// The template uses a variable that its role does not define. The name is a short checked identifier.
    UnknownVariable(String),
    /// The template lacks a variable its role requires.
    MissingVariable(&'static str),
    /// A value lands in a path segment and is empty, `.` or `..`, or makes a segment a dot-segment.
    UnsafePathValue,
    /// The expanded address is not usable.
    Address(AddressError),
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax => {
                f.write_str("the server's address template is malformed (an unmatched brace, a bad variable name or a forbidden character)")
            }
            Self::UnsupportedLevel => {
                f.write_str("the server's address template uses operators beyond plain {name} substitution, which JMAP does not use")
            }
            Self::ExpressionOutsidePathOrQuery => {
                f.write_str("the server's address template would let a value choose the scheme, server, port or fragment, which is refused")
            }
            Self::RelativeTemplate => {
                f.write_str("the server's address template neither starts with a scheme nor with /, so its target cannot be known")
            }
            Self::TooManyExpressions => f.write_str("the server's address template has more than 16 variables"),
            Self::MissingValue(name) => write!(f, "no value was supplied for the template variable {name}"),
            Self::UnknownVariable(name) => write!(f, "the server's address template uses the variable {name}, which JMAP does not define for it"),
            Self::MissingVariable(name) => write!(f, "the server's address template lacks the variable {name}, which JMAP requires"),
            Self::UnsafePathValue => f.write_str(
                "a file name or type placed in the address path is empty, \".\" or \"..\", which would change the address; use another name",
            ),
            Self::Address(error) => write!(f, "the address built from the server's template cannot be used: {error}"),
        }
    }
}

impl std::error::Error for TemplateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Address(error) => Some(error),
            _ => None,
        }
    }
}

impl From<AddressError> for TemplateError {
    fn from(error: AddressError) -> Self {
        Self::Address(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Path,
    Query,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Literal(String),
    Variable { name: String, place: Place },
}

/// A checked RFC 6570 level 1 template, tied to the address it is resolved against.
#[derive(Clone, PartialEq, Eq)]
pub struct AddressTemplate {
    pieces: Vec<Piece>,
    base: Address,
}

/// Characters RFC 6570 §2.1 allows in literals, restricted to visible ASCII (`%` is checked separately).
fn is_literal_char(b: u8) -> bool {
    (0x21..=0x7E).contains(&b) && !matches!(b, b'"' | b'\'' | b'<' | b'>' | b'\\' | b'^' | b'`' | b'{' | b'|' | b'}')
}

/// RFC 6570 §2.3 `varname`: `varchar *( ["."] varchar )`, `varchar = ALPHA / DIGIT / "_" / pct-encoded`.
fn is_varname(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_VARIABLE_LEN || name.starts_with('.') || name.ends_with('.') || name.contains("..") {
        return false;
    }
    let mut bytes = name.bytes();
    while let Some(b) = bytes.next() {
        let ok = match b {
            b'%' => bytes.next().and_then(hex_value).is_some() && bytes.next().and_then(hex_value).is_some(),
            b'.' | b'_' => true,
            _ => b.is_ascii_alphanumeric(),
        };
        if !ok {
            return false;
        }
    }
    true
}

/// The length of the literal scheme-and-authority prefix of a template, or `None` for an absolute-path one.
fn authority_end(template: &str) -> Result<usize, TemplateError> {
    let after_scheme = match template.split_once("://") {
        Some((scheme, _)) if !scheme.is_empty() && scheme.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')) => {
            scheme.len().saturating_add(3)
        }
        _ if template.starts_with("//") => 2,
        _ if template.starts_with('/') => return Ok(0),
        _ => return Err(TemplateError::RelativeTemplate),
    };
    let rest = template.get(after_scheme..).ok_or(TemplateError::Syntax)?;
    let end = rest.find(['/', '?', '#', '{']).unwrap_or(rest.len());
    if rest.get(end..).is_some_and(|tail| tail.starts_with('{')) {
        return Err(TemplateError::ExpressionOutsidePathOrQuery);
    }
    Ok(after_scheme.saturating_add(end))
}

fn is_dot_segment(segment: &str) -> bool {
    let decoded = segment.replace("%2E", ".").replace("%2e", ".");
    decoded == "." || decoded == ".."
}

impl AddressTemplate {
    /// Reads a template that will be resolved against `base` once expanded.
    pub fn parse(template: &str, base: &Address) -> Result<Self, TemplateError> {
        if template.is_empty() || template.len() > MAX_ADDRESS_LEN {
            return Err(TemplateError::Syntax);
        }
        if !template.bytes().all(|b| is_literal_char(b) || b == b'{' || b == b'}') {
            return Err(TemplateError::Syntax);
        }
        let prefix_len = authority_end(template)?;
        let (prefix, rest) = template.split_at_checked(prefix_len).ok_or(TemplateError::Syntax)?;
        if prefix.contains('}') {
            return Err(TemplateError::Syntax);
        }
        let mut pieces = vec![Piece::Literal(prefix.to_owned())];
        let mut place = Place::Path;
        let mut in_fragment = false;
        let mut expressions = 0usize;
        let mut literal = String::new();
        let mut chars = rest.char_indices();
        while let Some((index, c)) = chars.next() {
            match c {
                '{' => {
                    let after = index.checked_add(1).ok_or(TemplateError::Syntax)?;
                    let tail = rest.get(after..).ok_or(TemplateError::Syntax)?;
                    let close = tail.find('}').ok_or(TemplateError::Syntax)?;
                    let name = tail.get(..close).ok_or(TemplateError::Syntax)?;
                    if name.starts_with(['+', '#', '.', '/', ';', '?', '&', '=', ',', '!', '@', '|']) || name.contains([',', ':', '*']) {
                        return Err(TemplateError::UnsupportedLevel);
                    }
                    if !is_varname(name) {
                        return Err(TemplateError::Syntax);
                    }
                    if in_fragment {
                        return Err(TemplateError::ExpressionOutsidePathOrQuery);
                    }
                    expressions = expressions.saturating_add(1);
                    if expressions > MAX_EXPRESSIONS {
                        return Err(TemplateError::TooManyExpressions);
                    }
                    if !literal.is_empty() {
                        pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                    }
                    pieces.push(Piece::Variable { name: name.to_owned(), place });
                    for _ in 0..=close {
                        chars.next();
                    }
                }
                '}' => return Err(TemplateError::Syntax),
                '%' => {
                    let mut ok = true;
                    for _ in 0..2 {
                        match chars.next() {
                            Some((_, digit)) if digit.is_ascii_hexdigit() => literal.push(digit),
                            _ => ok = false,
                        }
                    }
                    if !ok {
                        return Err(TemplateError::Syntax);
                    }
                    literal.insert(literal.len().saturating_sub(2), '%');
                }
                _ => {
                    if c == '?' && place == Place::Path && !in_fragment {
                        place = Place::Query;
                    }
                    if c == '#' {
                        in_fragment = true;
                    }
                    literal.push(c);
                }
            }
        }
        if !literal.is_empty() {
            pieces.push(Piece::Literal(literal));
        }
        Ok(Self { pieces, base: base.clone() })
    }

    /// The variable names the template uses, in order of appearance (a repeated name appears again).
    pub fn variables(&self) -> impl Iterator<Item = &str> {
        self.pieces.iter().filter_map(|piece| match piece {
            Piece::Variable { name, .. } => Some(name.as_str()),
            Piece::Literal(_) => None,
        })
    }

    /// Whether the template uses the variable `name`.
    pub fn has_variable(&self, name: &str) -> bool {
        self.variables().any(|variable| variable == name)
    }

    /// The address templates are resolved against (the session address).
    pub fn base(&self) -> &Address {
        &self.base
    }

    /// Checks that the template uses every name of `required` and nothing outside `allowed`.
    pub(crate) fn check_variables(&self, required: &[&'static str], allowed: &[&str]) -> Result<(), TemplateError> {
        for &name in required {
            if !self.has_variable(name) {
                return Err(TemplateError::MissingVariable(name));
            }
        }
        if let Some(unknown) = self.variables().find(|name| !allowed.contains(name)) {
            return Err(TemplateError::UnknownVariable(unknown.to_owned()));
        }
        Ok(())
    }

    /// Expands the template with `values` (pairs of variable name and raw value), resolves the result against
    /// the base address, and checks it with [`Address::check_link`] from the base.
    pub fn expand(&self, values: &[(&str, &str)]) -> Result<Address, TemplateError> {
        let mut expanded = String::new();
        let mut segment_start = 0usize;
        let mut segment_has_value = false;
        let mut place = Place::Path;
        let mut finished_path = false;
        for piece in &self.pieces {
            match piece {
                Piece::Literal(text) => {
                    for c in text.chars() {
                        if place == Place::Path && !finished_path && matches!(c, '/' | '?' | '#') {
                            let segment = expanded.get(segment_start..).unwrap_or("");
                            if segment_has_value && is_dot_segment(segment) {
                                return Err(TemplateError::UnsafePathValue);
                            }
                            segment_has_value = false;
                            if c == '/' {
                                segment_start = expanded.len().saturating_add(1);
                            } else {
                                finished_path = true;
                                place = Place::Query;
                            }
                        }
                        expanded.push(c);
                    }
                }
                Piece::Variable { name, place: variable_place } => {
                    let value = values
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, value)| *value)
                        .ok_or_else(|| TemplateError::MissingValue(name.clone()))?;
                    if *variable_place == Place::Path && matches!(value, "" | "." | "..") {
                        return Err(TemplateError::UnsafePathValue);
                    }
                    if *variable_place == Place::Path {
                        segment_has_value = true;
                    }
                    if value.len() > MAX_ADDRESS_LEN {
                        return Err(TemplateError::Address(AddressError::TooLong));
                    }
                    expanded.push_str(&encode_component(value));
                    if expanded.len() > MAX_ADDRESS_LEN {
                        return Err(TemplateError::Address(AddressError::TooLong));
                    }
                }
            }
        }
        if !finished_path && segment_has_value && is_dot_segment(expanded.get(segment_start..).unwrap_or("")) {
            return Err(TemplateError::UnsafePathValue);
        }
        let address = self.base.resolve(&expanded)?;
        self.base.check_link(&address)?;
        Ok(address)
    }
}

impl fmt::Debug for AddressTemplate {
    /// Shows the base origin and the variable names, not the template's path.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AddressTemplate").field("base", &self.base.origin()).field("variables", &self.variables().collect::<Vec<_>>()).finish()
    }
}

#[cfg(test)]
mod tests;
