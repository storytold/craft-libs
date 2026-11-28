//! A bounded reader for configuration lists, never an evaluator.

use super::{Config, ConfigExpr};

const MAX_DEPTH: usize = 32;
const MAX_NODES: usize = 8192;
const MAX_TEXT: usize = 1 << 20;

/// Keep retention policy distinct from malformed optional configuration syntax.
#[derive(Debug)]
pub(super) enum ConfigError {
    MetadataLimit,
    Cancelled,
    Invalid(String),
}

impl From<String> for ConfigError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}

impl From<&str> for ConfigError {
    fn from(message: &str) -> Self {
        Self::Invalid(message.into())
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("cancelled"),
            Self::MetadataLimit => formatter.write_str("effect configuration exceeds document metadata allowance"),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl From<ConfigError> for super::Error {
    fn from(error: ConfigError) -> Self {
        match error {
            ConfigError::Cancelled => Self::Source(crate::Error::Cancelled),
            ConfigError::MetadataLimit => Self::Limit(super::Limit::Storage),
            ConfigError::Invalid(message) => super::invalid(message),
        }
    }
}

/// Parses bounded named configuration lists as data, charging the shared metadata allowance without evaluating expressions.
pub(super) fn parse_config<'a>(raw: &'a str, budget: &mut usize, keep_going: &dyn Fn() -> bool) -> std::result::Result<Config<'a>, ConfigError> {
    if raw.len() > MAX_TEXT {
        return Err("effect configuration exceeds 1 MiB".into());
    }
    let mut reader = Reader { raw, pos: 0, nodes: 0, budget, keep_going, next_check: 0 };
    let forms = reader.sequence(0, false)?;
    Ok(Config { raw, forms })
}

/// Grow geometrically and charge retained capacity, including the allocator's
/// actual result, before moving any elements into the replacement allocation.
pub(super) fn reserve<T>(items: &mut Vec<T>, additional: usize, budget: &mut usize) -> std::result::Result<(), ConfigError> {
    let needed = items.len().checked_add(additional).ok_or(ConfigError::MetadataLimit)?;
    if needed <= items.capacity() {
        return Ok(());
    }
    let planned = needed.max(items.capacity().checked_mul(2).ok_or(ConfigError::MetadataLimit)?).max(4);
    let size = std::mem::size_of::<T>();
    let debit = planned.checked_sub(items.capacity()).and_then(|n| n.checked_mul(size)).ok_or(ConfigError::MetadataLimit)?;
    let before = *budget;
    *budget = before.checked_sub(debit).ok_or(ConfigError::MetadataLimit)?;
    let mut replacement = Vec::new();
    if replacement.try_reserve_exact(planned).is_err() {
        *budget = before;
        return Err(ConfigError::MetadataLimit);
    }
    let Some(extra) = replacement.capacity().checked_sub(planned).and_then(|n| n.checked_mul(size)) else {
        *budget = before;
        return Err(ConfigError::MetadataLimit);
    };
    let Some(left) = budget.checked_sub(extra) else {
        *budget = before;
        return Err(ConfigError::MetadataLimit);
    };
    *budget = left;
    replacement.append(items);
    *items = replacement;
    Ok(())
}

struct Reader<'a, 'b> {
    raw: &'a str,
    pos: usize,
    nodes: usize,
    budget: &'b mut usize,
    keep_going: &'b dyn Fn() -> bool,
    next_check: usize,
}

impl<'a> Reader<'a, '_> {
    /// Reads the next Unicode scalar through a checked UTF-8 boundary without advancing input.
    fn peek(&self) -> Option<char> {
        self.raw.get(self.pos..)?.chars().next()
    }

    /// Moves the byte cursor by a decoded character's UTF-8 length.
    fn advance(&mut self, character: char) -> std::result::Result<(), ConfigError> {
        if self.pos >= self.next_check {
            if !(self.keep_going)() {
                return Err(ConfigError::Cancelled);
            }
            self.next_check = self.pos.saturating_add(4096);
        }
        self.pos = self.pos.saturating_add(character.len_utf8());
        Ok(())
    }

    /// Skips ASCII whitespace and line comments without interpreting their contents.
    fn whitespace(&mut self) -> std::result::Result<(), ConfigError> {
        loop {
            match self.peek() {
                Some(character) if character.is_ascii_whitespace() => self.advance(character)?,
                Some('#') => {
                    while let Some(character) = self.peek() {
                        self.advance(character)?;
                        if character == '\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }

    /// Builds ordered list data under depth and node caps, rejecting unmatched delimiters and charging storage before insertion.
    fn sequence(&mut self, depth: usize, nested: bool) -> std::result::Result<Vec<ConfigExpr<'a>>, ConfigError> {
        if depth > MAX_DEPTH {
            return Err("effect configuration nesting exceeds 32".into());
        }
        let mut items = Vec::new();
        loop {
            self.whitespace()?;
            match self.peek() {
                None if nested => return Err("effect configuration has an unterminated list".into()),
                None => return Ok(items),
                Some(')') if nested => {
                    self.advance(')')?;
                    return Ok(items);
                }
                Some(')') => return Err("effect configuration has an unmatched closing parenthesis".into()),
                _ => {}
            }
            if depth == 0 && self.peek() != Some('(') {
                return Err("effect configuration must contain named lists".into());
            }
            self.nodes = self.nodes.checked_add(1).ok_or_else(|| "effect configuration node count overflow".to_string())?;
            if self.nodes > MAX_NODES {
                return Err("effect configuration exceeds 8,192 syntax nodes".into());
            }
            reserve(&mut items, 1, self.budget)?;
            let item = match self.peek() {
                Some('(') => {
                    self.advance('(')?;
                    ConfigExpr::List(self.sequence(depth + 1, true)?)
                }
                Some('"') => {
                    // Observed GimpConfig color tuples hold encoding, sample count,
                    // sample bytes, profile count and optional profile bytes. Their
                    // binary roles do not depend on whether octets happen to be UTF-8.
                    let binary = matches!(items.first(), Some(ConfigExpr::Symbol("color")))
                        && matches!(items.get(1), Some(ConfigExpr::String(_)))
                        && ((items.len() == 3 && matches!(items.get(2), Some(ConfigExpr::Number(_))))
                            || (items.len() == 5 && matches!(items.get(4), Some(ConfigExpr::Number(_)))));
                    self.string(binary)?
                }
                Some(_) => self.atom()?,
                None => return Err("effect configuration ended unexpectedly".into()),
            };
            if (depth == 0 && item.head().is_none()) || (depth == 1 && items.is_empty() && !matches!(item, ConfigExpr::Symbol(_))) {
                return Err("effect configuration must contain named lists".into());
            }
            items.push(item);
        }
    }

    /// Reads a symbol or finite number while rejecting control characters, empty atoms and nonfinite numeric input.
    fn atom(&mut self) -> std::result::Result<ConfigExpr<'a>, ConfigError> {
        let start = self.pos;
        while let Some(character) = self.peek() {
            if character.is_ascii_whitespace() || matches!(character, '(' | ')' | '#' | '"') {
                break;
            }
            if character.is_control() {
                return Err("effect configuration has a control character in an atom".into());
            }
            self.advance(character)?;
        }
        let atom = self.raw.get(start..self.pos).ok_or_else(|| "effect configuration atom bounds".to_string())?;
        if atom.is_empty() {
            return Err("effect configuration has an empty atom".into());
        }
        if let Ok(number) = atom.parse::<f64>() {
            if !number.is_finite() {
                return Err("effect configuration has a nonfinite number".into());
            }
            return Ok(ConfigExpr::Number(number));
        }
        Ok(ConfigExpr::Symbol(atom))
    }

    /// Decodes bounded quoted text and octal bytes, preserving binary payloads instead of forcing invalid UTF-8 into text.
    fn string(&mut self, binary: bool) -> std::result::Result<ConfigExpr<'a>, ConfigError> {
        self.advance('"')?;
        let mut text = Vec::new();
        loop {
            let character = self.peek().ok_or_else(|| "effect configuration has an unterminated string".to_string())?;
            self.advance(character)?;
            if character == '"' {
                if binary {
                    return Ok(ConfigExpr::Bytes(text));
                }
                return Ok(match String::from_utf8(text) {
                    Ok(text) if !text.contains('\0') => ConfigExpr::String(text),
                    Ok(text) => ConfigExpr::Bytes(text.into_bytes()),
                    Err(error) => ConfigExpr::Bytes(error.into_bytes()),
                });
            }
            let character = if character == '\\' {
                let escape = self.peek().ok_or_else(|| "effect configuration has an incomplete escape".to_string())?;
                self.advance(escape)?;
                match escape {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'b' => '\u{0008}',
                    'f' => '\u{000c}',
                    '"' => '"',
                    '\\' => '\\',
                    '0'..='7' => {
                        let mut value = escape.to_digit(8).unwrap_or(0);
                        for _ in 0..2 {
                            if let Some(next @ '0'..='7') = self.peek() {
                                self.advance(next)?;
                                value = value.saturating_mul(8).saturating_add(next.to_digit(8).unwrap_or(0));
                            } else {
                                break;
                            }
                        }
                        let byte = u8::try_from(value).map_err(|_| "effect configuration octal escape exceeds one byte".to_string())?;
                        reserve(&mut text, 1, self.budget)?;
                        text.push(byte);
                        continue;
                    }
                    _ => return Err("effect configuration uses an unsupported string escape".into()),
                }
            } else {
                character
            };
            let mut encoded = [0; 4];
            reserve(&mut text, character.len_utf8(), self.budget)?;
            text.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_entry_list_charges_its_retained_capacity_exactly() {
        let mut budget = usize::MAX;
        let parsed = parse_config("(x)", &mut budget, &|| true).unwrap();
        let ConfigExpr::List(inner) = &parsed.forms[0] else { panic!("list") };
        let cost = (parsed.forms.capacity() + inner.capacity()) * std::mem::size_of::<ConfigExpr<'_>>();
        assert_eq!(usize::MAX - budget, cost);
        let mut exact = cost;
        assert!(parse_config("(x)", &mut exact, &|| true).is_ok());
        assert_eq!(exact, 0);
        assert!(matches!(parse_config("(x)", &mut (cost - 1), &|| true), Err(ConfigError::MetadataLimit)));
    }

    #[test]
    fn one_byte_text_and_octal_charge_their_retained_capacity_exactly() {
        for raw in [r#"(x "a")"#, r#"(x "\141")"#] {
            let mut budget = usize::MAX;
            let parsed = parse_config(raw, &mut budget, &|| true).unwrap();
            let ConfigExpr::List(inner) = &parsed.forms[0] else { panic!("list") };
            let ConfigExpr::String(text) = &inner[1] else { panic!("string") };
            assert_eq!(text, "a");
            let cost = (parsed.forms.capacity() + inner.capacity()) * std::mem::size_of::<ConfigExpr<'_>>() + text.capacity();
            assert_eq!(usize::MAX - budget, cost);
            let mut exact = cost;
            assert!(parse_config(raw, &mut exact, &|| true).is_ok());
            assert_eq!(exact, 0);
            assert!(matches!(parse_config(raw, &mut (cost - 1), &|| true), Err(ConfigError::MetadataLimit)));
        }
    }

    #[test]
    fn repeated_forms_charge_actual_capacity_and_keep_source_order() {
        let raw = (0..1000).map(|n| format!("(x {n}) ")).collect::<String>();
        let mut budget = 1 << 20;
        let before = budget;
        let parsed = parse_config(&raw, &mut budget, &|| true).unwrap();
        let mut slots = parsed.forms.capacity();
        for (index, form) in parsed.forms.iter().enumerate() {
            let ConfigExpr::List(inner) = form else { panic!("list") };
            slots += inner.capacity();
            assert!(matches!(inner[1], ConfigExpr::Number(n) if n == index as f64));
        }
        assert_eq!(before - budget, slots * std::mem::size_of::<ConfigExpr<'_>>());
    }
    #[test]
    fn color_sample_and_profile_roles_preserve_utf8_octets_without_retyping_text() {
        // Wire shape observed in the archived producer's effect config color tuples.
        let raw = r#"(foreground (color "RGB u8" 3 "\101\102\103" 3 "icc")) (label "\101\102\103")"#;
        let parsed = parse_config(raw, &mut (1 << 20), &|| true).unwrap();
        let tuple = parsed.forms[0].as_list().unwrap()[1].as_list().unwrap();
        assert!(matches!(&tuple[1], ConfigExpr::String(v) if v == "RGB u8"));
        assert!(matches!(&tuple[3], ConfigExpr::Bytes(v) if v == b"ABC"));
        assert!(matches!(&tuple[5], ConfigExpr::Bytes(v) if v == b"icc"));
        assert!(matches!(&parsed.forms[1].as_list().unwrap()[1], ConfigExpr::String(v) if v == "ABC"));
    }
}
