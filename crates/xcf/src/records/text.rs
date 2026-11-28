//! Bounded source text records. Pango markup and font requests remain inert;
//! this module does not shape text or choose native baseline/layout policy.

use std::borrow::Cow;

use super::{Budget, Scan, StopReason};
use crate::{Error, Result};

pub mod markup;

#[cfg(test)]
mod config_regressions;
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
    InvalidDirection,
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
/// singular field access matches ASCII-insensitive property symbols and refuses
/// duplicates instead of guessing. Exact source names and ordering stay intact.
#[derive(Debug)]
pub struct Config<'a> {
    pub forms: Vec<Value<'a>>,
}

impl Config<'_> {
    pub fn field(&self, name: &str) -> std::result::Result<Option<&[Value<'_>]>, FieldError> {
        let mut found = None;
        for form in &self.forms {
            let values = form.list()?;
            if values.first().ok_or(FieldError::ExpectedScalar)?.text()?.eq_ignore_ascii_case(name) {
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
        self.field(name)?
            .map(|values| match values {
                [Value::Atom { bytes, quoted, .. }] => {
                    let value = std::str::from_utf8(bytes).map_err(|_| FieldError::InvalidUtf8)?;
                    config_number(value, *quoted)
                }
                [Value::Atom { bytes: sign, quoted: false, .. }, Value::Atom { bytes, quoted: false, .. }] if sign.as_ref() == b"-" => {
                    let value = std::str::from_utf8(bytes).map_err(|_| FieldError::InvalidUtf8)?;
                    if value.starts_with(['+', '-']) {
                        return Err(FieldError::InvalidNumber);
                    }
                    config_number(value, false).map(|value| -value)
                }
                _ => Err(FieldError::ExpectedScalar),
            })
            .transpose()
    }
}

// GScanner's public lexical contract admits octal/hex integers. Only bare
// integer tokens use radix decoding; decimal fractions and quoted values retain
// their existing floating-point interpretation. Original atoms remain untouched.
fn config_number(source: &str, quoted: bool) -> std::result::Result<f64, FieldError> {
    if !quoted {
        let (negative, unsigned) = match source.as_bytes().first() {
            Some(b'-') => (true, source.get(1..).unwrap_or_default()),
            Some(b'+') => (false, source.get(1..).unwrap_or_default()),
            _ => (false, source),
        };
        let integer = if let Some(hex) = unsigned.strip_prefix("0x").or_else(|| unsigned.strip_prefix("0X")) {
            Some((hex, 16))
        } else if unsigned.len() > 1 && unsigned.starts_with('0') && !unsigned.contains(['.', 'e', 'E']) {
            Some((unsigned, 8))
        } else {
            None
        };
        if let Some((digits, radix)) = integer {
            // from_str_radix also accepts a sign; the lexical sign was already
            // separated above, so require digits rather than accepting 0x+1.
            if !digits.bytes().all(|byte| char::from(byte).is_digit(radix)) {
                return Err(FieldError::InvalidNumber);
            }
            let value = u64::from_str_radix(digits, radix).map_err(|_| FieldError::InvalidNumber)? as f64;
            return Ok(if negative { -value } else { value });
        }
    }
    source.parse::<f64>().ok().filter(|value| value.is_finite()).ok_or(FieldError::InvalidNumber)
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
    /// Compatibility string views of the eight fields. Explicit Latin-1
    /// lexical text is converted as before. Unavailable text, rotation or font string
    /// views use an empty slot; use [TextRecord::legacy_field_bytes] and
    /// [TextRecord::legacy_field_str] for exact source access.
    pub fields: Vec<Cow<'a, str>>,
    pub spans: Vec<Span>,
    /// Borrowed XLFD components, or empty when the request cannot be split.
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
    /// Effective paragraph policy. Duplicate direction forms use the safe Auto
    /// default; call `interpreted_direction` before applying this interpretation.
    pub direction: Direction,
    /// Saved writing/gravity direction, independent of paragraph autodetection.
    /// `None` means absent or uninterpretable; `interpreted_direction` distinguishes
    /// duplicates from absence while the original forms remain in `Config`.
    pub base_direction: Option<Direction>,
    /// Explicit `base-dir-auto` setting; absence remains distinct from `no`.
    /// Ambiguous direction forms leave both optional interpretations unset.
    pub automatic_direction: Option<bool>,
    pub source: Source<'a>,
}

impl TextRecord<'_> {
    /// Checked paragraph interpretation. Duplicate forms refuse a singular
    /// interpretation. For modern records, an omitted automatic flag is unresolved:
    /// its default depends on the producer era, which these neutral records do not
    /// assume. Saved writing/gravity direction remains in `base_direction`.
    pub fn interpreted_direction(&self) -> std::result::Result<Option<Direction>, FieldError> {
        if let Some(config) = self.config() {
            // Config and the compatibility fields are public. Revalidate current
            // forms rather than trusting cached interpretations after an edit.
            let (policy, _, automatic) = direction(config, &|| true).map_err(|_| FieldError::InvalidDirection)?;
            config.field("base-direction")?;
            config.field("base-dir-auto")?;
            return Ok(automatic.map(|_| policy));
        }
        Ok(Some(self.direction))
    }
    pub fn config(&self) -> Option<&Config<'_>> {
        match &self.source {
            Source::Gimp(config) => Some(config),
            _ => None,
        }
    }
    /// Exact lexical bytes of the requested GDyntext field.
    pub fn legacy_field_bytes(&self, index: usize) -> Option<&[u8]> {
        let Source::Gdyntext(legacy) = &self.source else { return None };
        let span = legacy.spans.get(index)?;
        self.raw.get(span.start.checked_sub(self.span.start)?..span.end.checked_sub(self.span.start)?)
    }

    /// UTF-8 view of the lexical GDyntext field, when one exists.
    pub fn legacy_field_str(&self, index: usize) -> Option<&str> {
        std::str::from_utf8(self.legacy_field_bytes(index)?).ok()
    }

    pub fn text(&self) -> std::result::Result<Option<&str>, FieldError> {
        match &self.source {
            Source::Gimp(config) => config
                .field("text")?
                .map(|values| match values {
                    [value] => gimp_string(value),
                    _ => Err(FieldError::ExpectedScalar),
                })
                .transpose(),
            Source::Gdyntext(legacy) => Ok(Some(&legacy.text)),
        }
    }
    /// Original Pango markup, including fragments without an explicit wrapper.
    /// `markup::tokenize` can expose inert contexts/runs; no native style is applied.
    pub fn markup(&self) -> std::result::Result<Option<&str>, FieldError> {
        self.config()
            .map(|config| config.field("markup")?.map(|values| gimp_string(values.first().ok_or(FieldError::ExpectedScalar)?)).transpose())
            .transpose()
            .map(Option::flatten)
    }
}

// GIMP string properties consume the decoded C-string prefix. Keep the whole
// atom in Config: truncation belongs only to known typed string accessors.
fn gimp_string<'a>(value: &'a Value<'_>) -> std::result::Result<&'a str, FieldError> {
    let bytes = value.bytes()?;
    let prefix = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
    std::str::from_utf8(prefix).map_err(|_| FieldError::InvalidUtf8)
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
            if self.bytes.get(self.at..self.at.saturating_add(2)) == Some(b"/*") {
                self.at += 2;
                loop {
                    self.tick()?;
                    if self.bytes.get(self.at..self.at.saturating_add(2)) == Some(b"*/") {
                        self.at += 2;
                        break;
                    }
                    if self.bytes.get(self.at).is_none() {
                        return Err(StopReason::Malformed.into());
                    }
                    self.at += 1;
                }
            } else if self.bytes.get(self.at) == Some(&b'#') {
                while self.bytes.get(self.at).is_some_and(|byte| *byte != b'\n') {
                    self.tick()?;
                    self.at += 1;
                }
            } else {
                return Ok(());
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
                            // One to three octal digits encode a single byte modulo 256.
                            value.to_le_bytes()[0]
                        }
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'b' => 8,
                        b'f' => 12,
                        b'v' if !require_quote => 11,
                        b'\\' | b'"' => escape,
                        _ => escape,
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
                while self.bytes.get(self.at).is_some_and(|byte| !byte.is_ascii_whitespace() && !b"()\"'#".contains(byte))
                    && self.bytes.get(self.at..self.at.saturating_add(2)) != Some(b"/*")
                {
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

fn base_direction_value(source: Option<&str>, quoted: bool, keep: &dyn Fn() -> bool) -> ReadResult<Option<Direction>> {
    Ok(match source {
        Some("ltr") => Some(Direction::Ltr),
        Some("rtl") => Some(Direction::Rtl),
        Some("ttb-rtl") => Some(Direction::TtbRtl),
        Some("ttb-rtl-upright") => Some(Direction::TtbRtlUpright),
        Some("ttb-ltr") => Some(Direction::TtbLtr),
        Some("ttb-ltr-upright") => Some(Direction::TtbLtrUpright),
        // Registered enum names are exact in GIMP. This reader also accepts
        // ASCII case variants as explicit leniency; property-symbol case folding
        // is separate. Keep every original value in Config for round trips.
        Some(value) if value.eq_ignore_ascii_case("GIMP_TEXT_DIRECTION_LTR") => Some(Direction::Ltr),
        Some(value) if value.eq_ignore_ascii_case("GIMP_TEXT_DIRECTION_RTL") => Some(Direction::Rtl),
        Some(value) if value.eq_ignore_ascii_case("GIMP_TEXT_DIRECTION_TTB_RTL") => Some(Direction::TtbRtl),
        Some(value) if value.eq_ignore_ascii_case("GIMP_TEXT_DIRECTION_TTB_RTL_UPRIGHT") => Some(Direction::TtbRtlUpright),
        Some(value) if value.eq_ignore_ascii_case("GIMP_TEXT_DIRECTION_TTB_LTR") => Some(Direction::TtbLtr),
        Some(value) if value.eq_ignore_ascii_case("GIMP_TEXT_DIRECTION_TTB_LTR_UPRIGHT") => Some(Direction::TtbLtrUpright),
        Some(value) if !quoted => Some(integer_direction(value, keep)?),
        None => None,
        _ => return Err(StopReason::Malformed.into()),
    })
}

fn direction(config: &Config<'_>, keep: &dyn Fn() -> bool) -> ReadResult<(Direction, Option<Direction>, Option<bool>)> {
    let (mut base, mut automatic) = (None, None);
    let (mut seen_base, mut seen_auto, mut duplicate) = (false, false, false);
    // Validate every occurrence independently: duplication is ambiguous, but
    // cannot hide a malformed setting in either field family. The bounded scan
    // polls even when thousands of unrelated short forms have no direction.
    for (index, form) in config.forms.iter().enumerate() {
        if index.is_multiple_of(64) && !keep() {
            return Err(Failure::Cancelled);
        }
        let values = form.list()?;
        let name = values.first().ok_or(FieldError::ExpectedScalar)?.text()?;
        if !name.eq_ignore_ascii_case("base-direction") && !name.eq_ignore_ascii_case("base-dir-auto") {
            continue;
        }
        let [Value::Atom { bytes, quoted, .. }] = values.get(1..).unwrap_or_default() else {
            return Err(StopReason::Malformed.into());
        };
        let value = std::str::from_utf8(bytes).map_err(|_| FieldError::InvalidUtf8)?;
        if name.eq_ignore_ascii_case("base-direction") {
            let parsed = base_direction_value(Some(value), *quoted, keep)?;
            duplicate |= seen_base;
            seen_base = true;
            base = parsed;
        } else {
            let parsed = if value.eq_ignore_ascii_case("yes") || value.eq_ignore_ascii_case("true") {
                true
            } else if value.eq_ignore_ascii_case("no") || value.eq_ignore_ascii_case("false") {
                false
            } else {
                return Err(StopReason::Malformed.into());
            };
            duplicate |= seen_auto;
            seen_auto = true;
            automatic = Some(parsed);
        }
    }
    if !keep() {
        return Err(Failure::Cancelled);
    }
    if duplicate {
        return Ok((Direction::Auto, None, None));
    }
    // Keep historical public fields as compatibility evidence; the checked
    // accessor leaves omitted automatic policy unresolved without producer context.
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

fn xlfd_fields(font: &[u8]) -> Option<[&str; 14]> {
    let mut parts = std::str::from_utf8(font).ok()?.strip_prefix('-')?.split('-');
    let mut fields = [""; 14];
    for field in &mut fields {
        *field = parts.next()?;
    }
    parts.next().is_none().then_some(fields)
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
        // The bounded font request is optional metadata. Only a complete XLFD
        // with an explicit charset can authorize Latin-1 text recovery.
        let font_fields = fields.last().and_then(|font| xlfd_fields(font));
        let latin1 = font_fields
            .as_ref()
            .is_some_and(|fields| fields.get(12).is_some_and(|charset| charset.eq_ignore_ascii_case("iso8859")) && fields.get(13) == Some(&"1"));
        let content = fields.first().copied().ok_or(Failure::Stop(StopReason::Malformed))?;
        let before = budget.retained_left;
        let mut parser = Parser { bytes: content, source_offset: 0, at: 0, checked_at: 0, nodes: 0, limits, budget, keep_going };
        let mut decoded = parser.quoted(false)?;
        for (index, byte) in decoded.iter().enumerate() {
            if index.is_multiple_of(256) {
                parser.check()?;
            }
            if *byte == 0 {
                // Keep the allocated bytes charged: only the typed value ends
                // here, while raw/lexical fields and their spans remain complete.
                decoded.truncate(index);
                break;
            }
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
                // The decoded text already passed validation. Its lexical bytes,
                // inactive rotation, and an unusable font request remain available through the spans.
                Err(_) if matches!(index, 0 | 3 | 7) => Cow::Borrowed(""),
                Err(_) => return Err(StopReason::Malformed.into()),
            };
            parser.budget.retain(&mut stored_fields, field)?;
        }
        for span in spans {
            parser.budget.retain(&mut stored_spans, span)?;
        }
        for field in font_fields.into_iter().flatten() {
            parser.budget.retain(&mut stored_font_fields, field)?;
        }
        parser.check()?;
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
        for raw in ["(base-dir-auto maybe)", "(base-direction sideways)"] {
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
        let raw = br#"(text "allocated") (bad "unterminated)"#;
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
        for _ in 0..32 {
            raw.extend([b'x'; 255]);
            raw.extend(b"\\n");
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
