//! Bounded source text records. Pango markup and font requests remain inert;
//! this module does not shape text or choose native baseline/layout policy.

use std::borrow::Cow;

use super::{Budget, Scan, StopReason};
use crate::{Error, Result};

pub mod markup;

#[cfg(test)]
mod regressions;

/// Limits for one optional configuration. Recursion has an absolute ceiling of
/// 64 even when a caller supplies a larger depth, so hostile data cannot exhaust
/// the process stack. Cumulative work and allocations use the shared Budget.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub bytes: usize,
    pub nodes: usize,
    pub depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { bytes: crate::MAX_STRING, nodes: 65_536, depth: 32 }
    }
}

/// Half-open byte range in the original inflated source, before unescaping.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug)]
pub enum Value<'a> {
    Atom { bytes: Cow<'a, [u8]>, quoted: bool, span: Span },
    List { values: Vec<Value<'a>>, span: Span },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldError {
    Duplicate,
    ExpectedList,
    ExpectedScalar,
    InvalidUtf8,
    InvalidNumber,
}

impl Value<'_> {
    pub fn span(&self) -> Span {
        match self {
            Self::Atom { span, .. } | Self::List { span, .. } => *span,
        }
    }
    pub fn bytes(&self) -> std::result::Result<&[u8], FieldError> {
        match self {
            Self::Atom { bytes, .. } => Ok(bytes),
            _ => Err(FieldError::ExpectedScalar),
        }
    }
    pub fn text(&self) -> std::result::Result<&str, FieldError> {
        std::str::from_utf8(self.bytes()?).map_err(|_| FieldError::InvalidUtf8)
    }
    pub fn list(&self) -> std::result::Result<&[Value<'_>], FieldError> {
        match self {
            Self::List { values, .. } => Ok(values),
            _ => Err(FieldError::ExpectedList),
        }
    }
}

/// Ordered named source forms. Unknown and duplicate fields are retained;
/// singular field access explicitly refuses duplicates instead of guessing.
#[derive(Debug)]
pub struct Config<'a> {
    pub forms: Vec<Value<'a>>,
}

impl Config<'_> {
    pub fn field(&self, name: &str) -> std::result::Result<Option<&[Value<'_>]>, FieldError> {
        let mut found = None;
        for form in &self.forms {
            let values = form.list()?;
            if values.first().ok_or(FieldError::ExpectedScalar)?.text()? == name {
                if found.is_some() {
                    return Err(FieldError::Duplicate);
                }
                found = Some(values.get(1..).unwrap_or_default());
            }
        }
        Ok(found)
    }
    pub fn scalar(&self, name: &str) -> std::result::Result<Option<&str>, FieldError> {
        self.field(name)?
            .map(|values| match values {
                [value] => value.text(),
                _ => Err(FieldError::ExpectedScalar),
            })
            .transpose()
    }
    pub fn number(&self, name: &str) -> std::result::Result<Option<f64>, FieldError> {
        self.scalar(name)?.map(|value| value.parse::<f64>().ok().filter(|value| value.is_finite()).ok_or(FieldError::InvalidNumber)).transpose()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Auto,
    Ltr,
    Rtl,
    TtbRtl,
    TtbRtlUpright,
    TtbLtr,
    TtbLtrUpright,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyEncoding {
    Utf8,
    Latin1,
}

/// All eight original GDyntext fields, including inactive rotation and the
/// unmodified XLFD request. No font is loaded or native unit conversion applied.
#[derive(Debug)]
pub struct Legacy<'a> {
    /// Original lexical fields; only a non-UTF-8 Latin-1 text field is converted.
    /// `TextRecord::raw` and `spans` always preserve its exact original bytes.
    pub fields: Vec<Cow<'a, str>>,
    pub spans: Vec<Span>,
    pub font_fields: Vec<&'a str>,
    pub text: String,
    pub encoding: LegacyEncoding,
}

#[derive(Debug)]
pub enum Source<'a> {
    Gimp(Config<'a>),
    Gdyntext(Legacy<'a>),
}

#[derive(Debug)]
pub struct TextRecord<'a> {
    pub raw: &'a [u8],
    pub span: Span,
    /// Effective paragraph policy, retaining the original compatibility behavior.
    pub direction: Direction,
    /// Saved writing/gravity direction, independent of paragraph autodetection.
    /// `None` means the source did not supply `base-direction`.
    pub base_direction: Option<Direction>,
    /// Explicit `base-dir-auto` setting; absence remains distinct from `no`.
    pub automatic_direction: Option<bool>,
    pub source: Source<'a>,
}

impl TextRecord<'_> {
    pub fn config(&self) -> Option<&Config<'_>> {
        match &self.source {
            Source::Gimp(config) => Some(config),
            _ => None,
        }
    }
    pub fn text(&self) -> std::result::Result<Option<&str>, FieldError> {
        match &self.source {
            Source::Gimp(config) => config.scalar("text"),
            Source::Gdyntext(legacy) => Ok(Some(&legacy.text)),
        }
    }
    /// Original Pango markup, including fragments without an explicit wrapper.
    /// `markup::tokenize` can expose inert contexts/runs; no native style is applied.
    pub fn markup(&self) -> std::result::Result<Option<&str>, FieldError> {
        self.config()
            .map(|config| config.field("markup")?.map(|values| values.first().ok_or(FieldError::ExpectedScalar)?.text()).transpose())
            .transpose()
            .map(Option::flatten)
    }
}

enum Failure {
    Stop(StopReason),
    Cancelled,
}
impl From<StopReason> for Failure {
    fn from(reason: StopReason) -> Self {
        Self::Stop(reason)
    }
}
impl From<FieldError> for Failure {
    fn from(_: FieldError) -> Self {
        Self::Stop(StopReason::Malformed)
    }
}
type ReadResult<T> = std::result::Result<T, Failure>;

struct Parser<'a, 'b> {
    bytes: &'a [u8],
    source_offset: usize,
    at: usize,
    checked_at: usize,
    nodes: usize,
    limits: Limits,
    budget: &'b mut Budget,
    keep_going: &'b dyn Fn() -> bool,
}

impl<'a> Parser<'a, '_> {
    fn check(&self) -> ReadResult<()> {
        if (self.keep_going)() { Ok(()) } else { Err(Failure::Cancelled) }
    }
    fn span(&self, start: usize, end: usize) -> ReadResult<Span> {
        Ok(Span {
            start: self.source_offset.checked_add(start).ok_or(Failure::Stop(StopReason::Malformed))?,
            end: self.source_offset.checked_add(end).ok_or(Failure::Stop(StopReason::Malformed))?,
        })
    }
    fn tick(&mut self) -> ReadResult<()> {
        // Escapes can step across exact byte multiples. Poll by distance instead
        // so hostile escapes cannot avoid checks inside a long quoted value.
        if self.at.saturating_sub(self.checked_at) >= 256 {
            self.check()?;
            self.checked_at = self.at;
        }
        Ok(())
    }
    fn space(&mut self) -> ReadResult<()> {
        loop {
            while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
                self.tick()?;
                self.at += 1;
            }
            if self.bytes.get(self.at) != Some(&b'#') {
                return Ok(());
            }
            while self.bytes.get(self.at).is_some_and(|byte| *byte != b'\n') {
                self.tick()?;
                self.at += 1;
            }
        }
    }
    fn quoted(&mut self, require_quote: bool) -> ReadResult<Vec<u8>> {
        let mut output = Vec::new();
        loop {
            self.tick()?;
            let Some(&byte) = self.bytes.get(self.at) else {
                return if require_quote { Err(StopReason::Malformed.into()) } else { Ok(output) };
            };
            self.at += 1;
            let byte = match byte {
                b'"' if require_quote => return Ok(output),
                0 => return Err(StopReason::Malformed.into()),
                b'\\' => {
                    let escape = self.bytes.get(self.at).copied().ok_or(Failure::Stop(StopReason::Malformed))?;
                    self.at += 1;
                    match escape {
                        b'0'..=b'7' => {
                            let mut value = u16::from(escape - b'0');
                            for _ in 0..2 {
                                if let Some(next) = self.bytes.get(self.at).filter(|byte| (b'0'..=b'7').contains(byte)) {
                                    value = value * 8 + u16::from(*next - b'0');
                                    self.at += 1;
                                } else {
                                    break;
                                }
                            }
                            u8::try_from(value).map_err(|_| Failure::Stop(StopReason::Malformed))?
                        }
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'b' => 8,
                        b'f' => 12,
                        b'v' if !require_quote => 11,
                        b'\\' | b'"' => escape,
                        _ if !require_quote => escape,
                        _ => return Err(StopReason::Malformed.into()),
                    }
                }
                byte => byte,
            };
            self.budget.retain(&mut output, byte)?;
        }
    }
    fn value(&mut self, depth: usize) -> ReadResult<Value<'a>> {
        self.check()?;
        self.space()?;
        self.nodes = self.nodes.checked_add(1).ok_or(Failure::Stop(StopReason::RecordLimit))?;
        if depth > self.limits.depth.min(64) || self.nodes > self.limits.nodes {
            return Err(StopReason::RecordLimit.into());
        }
        let start = self.at;
        match self.bytes.get(self.at).copied() {
            Some(b'(') => {
                self.at += 1;
                let mut values = Vec::new();
                loop {
                    self.space()?;
                    match self.bytes.get(self.at) {
                        Some(b')') => {
                            self.at += 1;
                            return Ok(Value::List { values, span: self.span(start, self.at)? });
                        }
                        None => return Err(StopReason::Malformed.into()),
                        _ => {
                            let value = self.value(depth + 1)?;
                            self.budget.retain(&mut values, value)?;
                        }
                    }
                }
            }
            Some(b'"') => {
                self.at += 1;
                let bytes = self.quoted(true)?;
                Ok(Value::Atom { bytes: Cow::Owned(bytes), quoted: true, span: self.span(start, self.at)? })
            }
            Some(b'\'') => {
                self.at += 1;
                let content = self.at;
                while self.bytes.get(self.at).is_some_and(|byte| *byte != b'\'') {
                    self.tick()?;
                    if self.bytes.get(self.at) == Some(&0) {
                        return Err(StopReason::Malformed.into());
                    }
                    self.at += 1;
                }
                if self.bytes.get(self.at) != Some(&b'\'') {
                    return Err(StopReason::Malformed.into());
                }
                let bytes = self.bytes.get(content..self.at).ok_or(Failure::Stop(StopReason::Malformed))?;
                self.at += 1;
                Ok(Value::Atom { bytes: Cow::Borrowed(bytes), quoted: true, span: self.span(start, self.at)? })
            }
            Some(b')') | None => Err(StopReason::Malformed.into()),
            Some(_) => {
                while self.bytes.get(self.at).is_some_and(|byte| !byte.is_ascii_whitespace() && !b"()\"'#".contains(byte)) {
                    self.tick()?;
                    self.at += 1;
                }
                let bytes = self.bytes.get(start..self.at).ok_or(Failure::Stop(StopReason::Malformed))?;
                Ok(Value::Atom { bytes: Cow::Borrowed(bytes), quoted: false, span: self.span(start, self.at)? })
            }
        }
    }
}

fn integer_direction(source: &str, keep: &dyn Fn() -> bool) -> ReadResult<Direction> {
    let (digits, radix) = if let Some(hex) = source.strip_prefix("0x").or_else(|| source.strip_prefix("0X")) {
        (hex, 16)
    } else if source.len() > 1 && source.starts_with('0') {
        (source, 8)
    } else {
        (source, 10)
    };
    if digits.is_empty() {
        return Err(StopReason::Malformed.into());
    }
    let mut value = 0u32;
    for (index, byte) in digits.bytes().enumerate() {
        if index.is_multiple_of(256) && !keep() {
            return Err(Failure::Cancelled);
        }
        let digit = char::from(byte).to_digit(radix).ok_or(Failure::Stop(StopReason::Malformed))?;
        value = value.checked_mul(radix).and_then(|value| value.checked_add(digit)).ok_or(Failure::Stop(StopReason::Malformed))?;
    }
    match value {
        0 => Ok(Direction::Ltr),
        1 => Ok(Direction::Rtl),
        2 => Ok(Direction::TtbRtl),
        3 => Ok(Direction::TtbRtlUpright),
        4 => Ok(Direction::TtbLtr),
        5 => Ok(Direction::TtbLtrUpright),
        _ => Err(StopReason::Malformed.into()),
    }
}

fn direction(config: &Config<'_>, keep: &dyn Fn() -> bool) -> ReadResult<(Direction, Option<Direction>, Option<bool>)> {
    let base = match config.scalar("base-direction")? {
        Some("ltr") => Some(Direction::Ltr),
        Some("rtl") => Some(Direction::Rtl),
        Some("ttb-rtl") => Some(Direction::TtbRtl),
        Some("ttb-rtl-upright") => Some(Direction::TtbRtlUpright),
        Some("ttb-ltr") => Some(Direction::TtbLtr),
        Some("ttb-ltr-upright") => Some(Direction::TtbLtrUpright),
        // Public GimpConfig enum deserialization also accepts registered names
        // case-insensitively. Keep the original atom in Config for round trips.
        Some(value) if value.eq_ignore_ascii_case("GIMP_TEXT_DIRECTION_LTR") => Some(Direction::Ltr),
        Some(value) if value.eq_ignore_ascii_case("GIMP_TEXT_DIRECTION_RTL") => Some(Direction::Rtl),
        Some(value) if value.eq_ignore_ascii_case("GIMP_TEXT_DIRECTION_TTB_RTL") => Some(Direction::TtbRtl),
        Some(value) if value.eq_ignore_ascii_case("GIMP_TEXT_DIRECTION_TTB_RTL_UPRIGHT") => Some(Direction::TtbRtlUpright),
        Some(value) if value.eq_ignore_ascii_case("GIMP_TEXT_DIRECTION_TTB_LTR") => Some(Direction::TtbLtr),
        Some(value) if value.eq_ignore_ascii_case("GIMP_TEXT_DIRECTION_TTB_LTR_UPRIGHT") => Some(Direction::TtbLtrUpright),
        Some(value) if matches!(config.field("base-direction")?, Some([Value::Atom { quoted: false, .. }])) => Some(integer_direction(value, keep)?),
        None => None,
        _ => return Err(StopReason::Malformed.into()),
    };
    let automatic = match config.scalar("base-dir-auto")? {
        Some(value) if value.eq_ignore_ascii_case("yes") || value.eq_ignore_ascii_case("true") => Some(true),
        Some(value) if value.eq_ignore_ascii_case("no") || value.eq_ignore_ascii_case("false") => Some(false),
        None => None,
        _ => return Err(StopReason::Malformed.into()),
    };
    let direction = match automatic {
        Some(true) => Direction::Auto,
        Some(false) => base.unwrap_or(Direction::Ltr),
        None => base.unwrap_or(Direction::Auto),
    };
    Ok((direction, base, automatic))
}

fn admit(bytes: &[u8], limits: Limits, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> ReadResult<()> {
    if !keep_going() {
        return Err(Failure::Cancelled);
    }
    if bytes.len() > limits.bytes {
        return Err(StopReason::PayloadLimit.into());
    }
    // Source parsing, unescaping and fixed source-field lookups are linear in
    // input size. Charge their work before parsing, including failed aliases.
    budget.work(16)?;
    budget.work(bytes.len().checked_mul(4).ok_or(Failure::Stop(StopReason::WorkLimit))?)?;
    Ok(())
}

fn finish<'a>(result: ReadResult<TextRecord<'a>>, source_offset: usize, retained: usize, budget: &mut Budget) -> Result<Scan<TextRecord<'a>>> {
    let result = result.and_then(|record| {
        let end = record.span.end;
        let mut records = Vec::new();
        budget.retain(&mut records, record)?;
        Ok(Scan::complete(records, end))
    });
    match result {
        Ok(scan) => Ok(scan),
        Err(Failure::Cancelled) => {
            budget.retained_left = retained;
            Err(Error::Cancelled)
        }
        Err(Failure::Stop(reason)) => {
            budget.retained_left = retained;
            Ok(Scan::stopped(Vec::new(), source_offset, reason))
        }
    }
}

/// Decode modern `gimp-text-layer` configuration as source fields. Unknown
/// formatting and Pango markup remain available without invoking a font engine.
pub fn gimp<'a>(
    bytes: &'a [u8],
    source_offset: usize,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<TextRecord<'a>>> {
    let retained = budget.retained_left;
    let result = (|| {
        admit(bytes, limits, budget, keep_going)?;
        let source = bytes.strip_suffix(&[0]).unwrap_or(bytes);
        for (index, byte) in source.iter().enumerate() {
            if index.is_multiple_of(256) && !keep_going() {
                return Err(Failure::Cancelled);
            }
            if *byte == 0 {
                return Err(StopReason::Malformed.into());
            }
        }
        let mut parser = Parser { bytes: source, source_offset, at: 0, checked_at: 0, nodes: 0, limits, budget, keep_going };
        let mut forms = Vec::new();
        loop {
            parser.space()?;
            if parser.at == source.len() {
                break;
            }
            let form = parser.value(0)?;
            form.list()?.first().ok_or(FieldError::ExpectedScalar)?.text()?;
            parser.budget.retain(&mut forms, form)?;
        }
        let config = Config { forms };
        let (direction, base_direction, automatic_direction) = direction(&config, keep_going)?;
        Ok(TextRecord {
            raw: bytes,
            span: parser.span(0, bytes.len())?,
            direction,
            base_direction,
            automatic_direction,
            source: Source::Gimp(config),
        })
    })();
    finish(result, source_offset, retained, budget)
}

fn legacy_separator(bytes: &[u8], keep_going: &dyn Fn() -> bool) -> ReadResult<Option<usize>> {
    for (index, byte) in bytes.iter().enumerate() {
        if index.is_multiple_of(256) && !keep_going() {
            return Err(Failure::Cancelled);
        }
        if *byte == 0 {
            return Err(StopReason::Malformed.into());
        }
        if *byte == b'}' && bytes.get(index + 1) == Some(&b'{') {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

/// Decode GDyntext's escaped content and preserve its eight original fields.
/// Latin-1 recovery is allowed only for an explicit ISO8859-1 XLFD declaration.
pub fn gdyntext<'a>(
    bytes: &'a [u8],
    source_offset: usize,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<TextRecord<'a>>> {
    let retained = budget.retained_left;
    let result = (|| {
        admit(bytes, limits, budget, keep_going)?;
        let raw = bytes.strip_suffix(&[0]).unwrap_or(bytes);
        let body = raw.strip_prefix(b"GDT10{").and_then(|source| source.strip_suffix(b"}")).ok_or(Failure::Stop(StopReason::Malformed))?;
        let mut fields: [&[u8]; 8] = [&[]; 8];
        let mut spans = [Span::default(); 8];
        let mut start = 0;
        for index in 0..8 {
            if !keep_going() {
                return Err(Failure::Cancelled);
            }
            let remaining = body.get(start..).ok_or(Failure::Stop(StopReason::Malformed))?;
            let length = if index == 7 {
                if legacy_separator(remaining, keep_going)?.is_some() {
                    return Err(StopReason::Malformed.into());
                }
                remaining.len()
            } else {
                legacy_separator(remaining, keep_going)?.ok_or(Failure::Stop(StopReason::Malformed))?
            };
            let end = start.checked_add(length).ok_or(Failure::Stop(StopReason::Malformed))?;
            if let Some(field) = fields.get_mut(index) {
                *field = body.get(start..end).ok_or(Failure::Stop(StopReason::Malformed))?;
            }
            if let Some(span) = spans.get_mut(index) {
                *span = Span {
                    start: source_offset.checked_add(6).and_then(|offset| offset.checked_add(start)).ok_or(Failure::Stop(StopReason::Malformed))?,
                    end: source_offset.checked_add(6).and_then(|offset| offset.checked_add(end)).ok_or(Failure::Stop(StopReason::Malformed))?,
                };
            }
            start = end.saturating_add(2);
        }
        // Metadata is UTF-8 independently of the legacy text field's declared
        // encoding. A raw Latin-1 text byte must reach the explicit XLFD fallback.
        let font = fields
            .get(7)
            .and_then(|font| std::str::from_utf8(font).ok())
            .and_then(|font| font.strip_prefix('-'))
            .ok_or(Failure::Stop(StopReason::Malformed))?;
        let mut font_fields = [""; 14];
        let mut parts = font.split('-');
        for field in &mut font_fields {
            *field = parts.next().filter(|field| field.len() <= 64).ok_or(Failure::Stop(StopReason::Malformed))?;
        }
        if parts.next().is_some() {
            return Err(StopReason::Malformed.into());
        }
        let latin1 = font_fields.get(12).is_some_and(|charset| charset.eq_ignore_ascii_case("iso8859")) && font_fields.get(13) == Some(&"1");
        let content = fields.first().copied().ok_or(Failure::Stop(StopReason::Malformed))?;
        let before = budget.retained_left;
        let mut parser = Parser { bytes: content, source_offset: 0, at: 0, checked_at: 0, nodes: 0, limits, budget, keep_going };
        let decoded = parser.quoted(false)?;
        if decoded.contains(&0) {
            return Err(StopReason::Malformed.into());
        }
        let decoded_storage = before - parser.budget.retained_left;
        let (text, encoding) = match String::from_utf8(decoded) {
            Ok(text) => (text, LegacyEncoding::Utf8),
            Err(error) if latin1 => {
                let decoded = error.into_bytes();
                let mut output = Vec::new();
                for byte in &decoded {
                    parser.check()?;
                    let mut encoded = [0; 4];
                    for byte in char::from(*byte).encode_utf8(&mut encoded).as_bytes() {
                        parser.budget.retain(&mut output, *byte)?;
                    }
                }
                drop(decoded);
                parser.budget.retained_left += decoded_storage;
                (String::from_utf8(output).map_err(|_| Failure::Stop(StopReason::Malformed))?, LegacyEncoding::Latin1)
            }
            Err(_) => return Err(StopReason::Malformed.into()),
        };
        let end = source_offset.checked_add(bytes.len()).ok_or(Failure::Stop(StopReason::Malformed))?;
        let mut stored_fields = Vec::new();
        let mut stored_spans = Vec::new();
        let mut stored_font_fields = Vec::new();
        for (index, field) in fields.into_iter().enumerate() {
            let field = match std::str::from_utf8(field) {
                Ok(field) => Cow::Borrowed(field),
                Err(_) if index == 0 && latin1 => {
                    let mut output = Vec::new();
                    for (offset, byte) in field.iter().enumerate() {
                        if offset.is_multiple_of(256) {
                            parser.check()?;
                        }
                        let mut encoded = [0; 4];
                        for byte in char::from(*byte).encode_utf8(&mut encoded).as_bytes() {
                            parser.budget.retain(&mut output, *byte)?;
                        }
                    }
                    Cow::Owned(String::from_utf8(output).map_err(|_| Failure::Stop(StopReason::Malformed))?)
                }
                Err(_) => return Err(StopReason::Malformed.into()),
            };
            parser.budget.retain(&mut stored_fields, field)?;
        }
        for span in spans {
            parser.budget.retain(&mut stored_spans, span)?;
        }
        for field in font_fields {
            parser.budget.retain(&mut stored_font_fields, field)?;
        }
        Ok(TextRecord {
            raw: bytes,
            span: Span { start: source_offset, end },
            direction: Direction::Auto,
            base_direction: None,
            automatic_direction: None,
            source: Source::Gdyntext(Legacy { fields: stored_fields, spans: stored_spans, font_fields: stored_font_fields, text, encoding }),
        })
    })();
    finish(result, source_offset, retained, budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Original synthetic source configurations from the documented XCF layouts;
    // no external font files, application assets or saved documents are used.

    #[test]
    fn source_fields_keep_order_bytes_units_offsets_and_lexical_spans() {
        let raw = br#"# comment
(font-size 18) (font-size-unit points) (offset-x -3) (offset-y 7)
(transformation (matrix 1 0.25 -0.5 2)) (unknown "\377\000")
(text "caf\303\251") (custom first) (custom last)"#;
        let scan = gimp(raw, 50, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        let record = &scan.records[0];
        assert_eq!(record.text().unwrap(), Some("café"));
        let config = record.config().unwrap();
        assert_eq!(config.scalar("font-size-unit").unwrap(), Some("points"));
        assert_eq!(config.number("offset-x").unwrap(), Some(-3.0));
        assert_eq!(config.number("offset-y").unwrap(), Some(7.0));
        assert_eq!(config.field("custom").unwrap_err(), FieldError::Duplicate);
        assert_eq!(config.forms.len(), 9);
        assert_eq!(config.field("unknown").unwrap().unwrap()[0].bytes().unwrap(), &[255, 0]);
        let value = &config.field("text").unwrap().unwrap()[0];
        assert_eq!(&raw[value.span().start - 50..value.span().end - 50], br#""caf\303\251""#);
        assert_eq!(record.span, Span { start: 50, end: 50 + raw.len() });
    }

    #[test]
    fn automatic_and_forced_source_directions_are_not_conflated() {
        for (fields, expected) in [
            ("", Direction::Auto),
            ("(base-direction ltr)", Direction::Ltr),
            ("(base-direction rtl)", Direction::Rtl),
            ("(base-direction rtl) (base-dir-auto yes)", Direction::Auto),
            ("(base-dir-auto no)", Direction::Ltr),
            ("(base-direction rtl) (base-dir-auto no)", Direction::Rtl),
            ("(base-direction ttb-rtl-upright)", Direction::TtbRtlUpright),
        ] {
            let raw = format!("(text \"אב 123, punctuation\") {fields}");
            let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
            assert_eq!(scan.stopped, None, "{fields}");
            assert_eq!(scan.records[0].direction, expected, "{fields}");
        }
        for raw in ["(base-dir-auto maybe)", "(base-direction sideways)", "(base-direction rtl) (base-direction ltr)"] {
            let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
            assert!(scan.records.is_empty());
            assert_eq!(scan.stopped.unwrap().reason, super::super::StopReason::Malformed);
        }
    }

    #[test]
    fn raw_markup_font_resources_and_source_matrix_are_not_native_layout() {
        let raw = br#"(markup "<markup><span font_desc=\"Example Bold 12\">ab</span></markup>" (markupfont key (font (family "Example")))) (transformation (matrix 1 2 3 4))"#;
        let scan = gimp(raw, 0, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        let record = &scan.records[0];
        assert_eq!(record.markup().unwrap(), Some("<markup><span font_desc=\"Example Bold 12\">ab</span></markup>"));
        assert_eq!(record.config().unwrap().field("markup").unwrap().unwrap().len(), 2);
        let matrix = record.config().unwrap().field("transformation").unwrap().unwrap()[0].list().unwrap();
        assert_eq!(matrix[0].text().unwrap(), "matrix");
        assert_eq!(matrix[3].text().unwrap(), "3");
    }

    pub(super) fn legacy_bytes(text: &str, charset: &str) -> Vec<u8> {
        format!("GDT10{{{text}}}{{1}}{{2}}{{17}}{{3}}{{0xff00aa}}{{0}}{{-source-example-medium-r-normal--12-120-75-75-p-60-{charset}-1}}")
            .into_bytes()
    }

    #[test]
    fn gdyntext_recovers_only_explicit_latin1_and_preserves_all_source_fields() {
        for (input, expected, encoding) in [("caf\\303\\251", "café", LegacyEncoding::Utf8), ("caf\\351", "café", LegacyEncoding::Latin1)] {
            let raw = legacy_bytes(input, "iso8859");
            let scan = gdyntext(&raw, 20, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
            assert_eq!(scan.stopped, None);
            let record = &scan.records[0];
            assert_eq!(record.text().unwrap(), Some(expected));
            let Source::Gdyntext(legacy) = &record.source else { panic!("legacy record expected") };
            assert_eq!(legacy.encoding, encoding);
            assert_eq!(legacy.fields[3], "17");
            assert_eq!(legacy.fields[4], "3");
            assert_eq!(legacy.fields[5], "0xff00aa");
            assert_eq!(legacy.fields.len(), 8);
            assert_eq!(legacy.font_fields.len(), 14);
            let span = legacy.spans[0];
            assert_eq!(&raw[span.start - 20..span.end - 20], input.as_bytes());
            assert_eq!(record.direction, Direction::Auto);
        }
        let raw = legacy_bytes("caf\\351", "unknown");
        let scan = gdyntext(&raw, 0, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
        assert!(scan.records.is_empty());
        assert_eq!(scan.stopped.unwrap().reason, super::super::StopReason::Malformed);
    }

    #[test]
    fn malformed_depth_node_and_retention_failures_refund_storage_not_work() {
        let raw = br#"(text "allocated") (bad "\q")"#;
        let mut budget = super::super::Budget::default();
        let retained = budget.retained_remaining();
        let work = budget.work_remaining();
        let scan = gimp(raw, 0, Limits::default(), &mut budget, &|| true).unwrap();
        assert!(scan.stopped.is_some());
        assert_eq!(budget.retained_remaining(), retained);
        assert!(budget.work_remaining() < work);
        for limits in [Limits { depth: 1, ..Limits::default() }, Limits { nodes: 2, ..Limits::default() }] {
            let scan = gimp(b"(x (y z))", 0, limits, &mut budget, &|| true).unwrap();
            assert_eq!(scan.stopped.unwrap().reason, super::super::StopReason::RecordLimit);
        }
        let scan = gimp(b"(text a)", 0, Limits::default(), &mut super::super::Budget::new(1024, 0), &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, super::super::StopReason::RetainedLimit);
        let scan = gimp(raw, 0, Limits::default(), &mut super::super::Budget::new(0, 1024), &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, super::super::StopReason::WorkLimit);
    }

    #[test]
    fn cancellation_precedes_spent_ledgers_and_runs_inside_large_strings() {
        assert!(matches!(gimp(b"", 0, Limits::default(), &mut super::super::Budget::new(0, 0), &|| false), Err(crate::Error::Cancelled)));
        assert!(matches!(gdyntext(b"", 0, Limits::default(), &mut super::super::Budget::new(0, 0), &|| false), Err(crate::Error::Cancelled)));
        let mut raw = b"(text \"".to_vec();
        while raw.len() < 8192 {
            if raw.len() % 256 == 255 {
                raw.extend(b"\\n");
            } else {
                raw.push(b'x');
            }
        }
        raw.extend(b"\")");
        let checks = std::cell::Cell::new(0);
        let keep = || {
            checks.set(checks.get() + 1);
            checks.get() < 5
        };
        // Admission still polls above; enter after the opening quote so this
        // escape-boundary case can only cancel inside the quoted decoder.
        let mut budget = super::super::Budget::default();
        let mut parser = Parser {
            bytes: &raw,
            source_offset: 0,
            at: 7,
            checked_at: 7,
            nodes: 0,
            limits: Limits::default(),
            budget: &mut budget,
            keep_going: &keep,
        };
        assert!(matches!(parser.quoted(true), Err(Failure::Cancelled)));
        assert!(parser.at < raw.len() - 2, "decoder cancellation precedes the closing quote");
        assert!(parser.budget.retained_remaining() < crate::MAX_METADATA_BYTES, "the decoder actually emitted and charged bytes");
    }

    #[test]
    fn modern_and_legacy_aliases_share_work_without_refunding_failed_attempts() {
        let raw = b"(text \"retained\")";
        let mut budget = super::super::Budget::new(16 + 4 * raw.len(), crate::MAX_METADATA_BYTES);
        let first = gimp(raw, 0, Limits::default(), &mut budget, &|| true).unwrap();
        assert_eq!(first.stopped, None);
        assert_eq!(budget.work_remaining(), 0);
        let retained = budget.retained_remaining();
        let old = legacy_bytes("text", "iso8859");
        for scan in [gimp(raw, 0, Limits::default(), &mut budget, &|| true), gdyntext(&old, 0, Limits::default(), &mut budget, &|| true)] {
            assert_eq!(scan.unwrap().stopped.unwrap().reason, super::super::StopReason::WorkLimit);
        }
        assert_eq!(budget.retained_remaining(), retained);
        let deep = format!("{}x{}", "(".repeat(100), ")".repeat(100));
        let scan =
            gimp(deep.as_bytes(), 0, Limits { depth: usize::MAX, ..Limits::default() }, &mut super::super::Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, super::super::StopReason::RecordLimit);
    }
}
