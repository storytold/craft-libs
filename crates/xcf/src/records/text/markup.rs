//! A restricted source tokenizer for Pango's XML markup, not a font/style engine.
//! Runs address normalized UTF-8 text; source ranges address this decoded markup
//! input, not the earlier escaped XCF configuration. DTDs/custom entities and
//! namespace interpretation are deliberately unsupported. No resources resolve.

use super::super::{Budget, Scan, StopReason};
use super::{Failure, ReadResult};
use crate::{Error, Result};
use std::{borrow::Cow, ops::Range};

#[cfg(test)]
mod regressions;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub bytes: usize,
    pub depth: usize,
    pub nodes: usize,
    pub attributes: usize,
    pub runs: usize,
    pub notices: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self { bytes: crate::MAX_STRING, depth: 32, nodes: 65_536, attributes: 65_536, runs: 65_536, notices: 128 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Markup,
    Span,
    Bold,
    Italic,
    Underline,
    Strike,
    Subscript,
    Superscript,
    Monospace,
    Bigger,
    Smaller,
    Unhandled,
}

/// Recognized source vocabulary, not validation or evaluation of native styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeKind {
    Font,
    FontDescription,
    Family,
    Weight,
    Style,
    Variant,
    Stretch,
    Rise,
    LetterSpacing,
    Features,
    Underline,
    TextTransform,
    Strikethrough,
    Foreground,
    Size,
    Alpha,
    Language,
    Unhandled,
}

#[derive(Debug)]
pub struct Attribute<'a> {
    pub name: &'a str,
    pub value: Cow<'a, str>,
    pub kind: AttributeKind,
    pub source: Range<usize>,
}

/// A persistent lightweight inheritance node. Attributes are stored once on
/// their original element, never cloned into every descendant text run.
#[derive(Debug)]
pub struct Context<'a> {
    pub parent: Option<usize>,
    pub name: &'a str,
    pub role: Role,
    pub attributes: Vec<Attribute<'a>>,
    /// Opening-tag range; 0..0 identifies the implicit fragment root.
    pub source: Range<usize>,
}

#[derive(Debug)]
pub struct Run {
    /// Byte range in Markup.text, always on UTF-8 scalar boundaries.
    pub text: Range<usize>,
    pub context: usize,
    /// Lexical character-data range in the decoded markup input.
    pub source: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    UnhandledElement,
    UnhandledAttribute,
}

#[derive(Debug)]
pub struct Notice {
    pub kind: NoticeKind,
    pub context: usize,
    pub attribute: Option<usize>,
    pub source: Range<usize>,
}

#[derive(Debug)]
pub struct Markup<'a> {
    pub raw: &'a str,
    pub text: String,
    pub runs: Vec<Run>,
    pub contexts: Vec<Context<'a>>,
    pub notices: Vec<Notice>,
    pub has_unhandled: bool,
    pub notices_truncated: bool,
}

pub struct Inheritance<'a, 'source> {
    contexts: &'a [Context<'source>],
    next: Option<usize>,
    left: usize,
}
impl<'a, 'source> Iterator for Inheritance<'a, 'source> {
    type Item = &'a Context<'source>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        let context = self.contexts.get(self.next?)?;
        self.next = context.parent;
        Some(context)
    }
}
impl Markup<'_> {
    /// Nearest element first, ending at the root. The defensive ceiling also
    /// prevents cycles if a caller edits the public context table after parsing.
    pub fn inherited(&self, context: usize) -> Inheritance<'_, '_> {
        Inheritance { contexts: &self.contexts, next: Some(context), left: 65 }
    }
}

fn role(name: &str) -> Role {
    match name {
        "markup" => Role::Markup,
        "span" => Role::Span,
        "b" => Role::Bold,
        "i" => Role::Italic,
        "u" => Role::Underline,
        "s" => Role::Strike,
        "sub" => Role::Subscript,
        "sup" => Role::Superscript,
        "tt" => Role::Monospace,
        "big" => Role::Bigger,
        "small" => Role::Smaller,
        _ => Role::Unhandled,
    }
}
fn attribute_kind(name: &str) -> AttributeKind {
    const KNOWN: &[(&str, AttributeKind)] = &[
        ("font", AttributeKind::Font),
        ("font_desc", AttributeKind::FontDescription),
        ("font_family", AttributeKind::Family),
        ("face", AttributeKind::Family),
        ("weight", AttributeKind::Weight),
        ("font_weight", AttributeKind::Weight),
        ("style", AttributeKind::Style),
        ("font_style", AttributeKind::Style),
        ("variant", AttributeKind::Variant),
        ("font_variant", AttributeKind::Variant),
        ("stretch", AttributeKind::Stretch),
        ("font_stretch", AttributeKind::Stretch),
        ("rise", AttributeKind::Rise),
        ("letter_spacing", AttributeKind::LetterSpacing),
        ("font_features", AttributeKind::Features),
        ("underline", AttributeKind::Underline),
        ("text_transform", AttributeKind::TextTransform),
        ("strikethrough", AttributeKind::Strikethrough),
        ("foreground", AttributeKind::Foreground),
        ("fgcolor", AttributeKind::Foreground),
        ("color", AttributeKind::Foreground),
        ("size", AttributeKind::Size),
        ("font_size", AttributeKind::Size),
        ("alpha", AttributeKind::Alpha),
        ("foreground_alpha", AttributeKind::Alpha),
        ("fgalpha", AttributeKind::Alpha),
        ("lang", AttributeKind::Language),
    ];
    KNOWN
        .iter()
        .find(|(key, _)| name.bytes().map(|byte| if byte == b'-' { b'_' } else { byte }).eq(key.bytes()))
        .map_or(AttributeKind::Unhandled, |(_, kind)| *kind)
}

fn xml_char(character: char) -> bool {
    matches!(character as u32, 9 | 10 | 13 | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)
}
// XML 1.0 productions 4/4a: https://www.w3.org/TR/xml/#NT-NameStartChar
fn name_start(character: char) -> bool {
    matches!(character as u32, 0x3a | 0x41..=0x5a | 0x5f | 0x61..=0x7a | 0xc0..=0xd6 | 0xd8..=0xf6 | 0xf8..=0x2ff |
        0x370..=0x37d | 0x37f..=0x1fff | 0x200c..=0x200d | 0x2070..=0x218f | 0x2c00..=0x2fef | 0x3001..=0xd7ff |
        0xf900..=0xfdcf | 0xfdf0..=0xfffd | 0x10000..=0xeffff)
}
fn name_char(character: char) -> bool {
    name_start(character) || matches!(character as u32, 0x2d | 0x2e | 0x30..=0x39 | 0xb7 | 0x300..=0x36f | 0x203f..=0x2040)
}
fn whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

struct Parser<'a, 'b> {
    raw: &'a str,
    at: usize,
    checked_at: usize,
    limits: Limits,
    budget: &'b mut Budget,
    keep: &'b dyn Fn() -> bool,
    output: Vec<u8>,
    runs: Vec<Run>,
    contexts: Vec<Context<'a>>,
    stack: Vec<usize>,
    stack_charge: usize,
    notices: Vec<Notice>,
    has_unhandled: bool,
    notices_truncated: bool,
    nodes: usize,
    attributes: usize,
    root_seen: bool,
    root_closed: bool,
    implicit_root: bool,
}
impl<'a> Parser<'a, '_> {
    fn check(&self) -> ReadResult<()> {
        if (self.keep)() { Ok(()) } else { Err(Failure::Cancelled) }
    }
    fn tick(&mut self) -> ReadResult<()> {
        if self.at.saturating_sub(self.checked_at) >= 256 {
            self.check()?;
            self.checked_at = self.at;
        }
        Ok(())
    }
    fn tail(&self) -> ReadResult<&'a str> {
        self.raw.get(self.at..).ok_or(Failure::Stop(StopReason::Malformed))
    }
    fn starts(&self, text: &str) -> bool {
        self.raw.get(self.at..).is_some_and(|tail| tail.starts_with(text))
    }
    fn space(&mut self) -> ReadResult<bool> {
        let before = self.at;
        while self.raw.as_bytes().get(self.at).is_some_and(|byte| whitespace(*byte)) {
            self.tick()?;
            self.at += 1;
        }
        Ok(before != self.at)
    }
    fn name(&mut self) -> ReadResult<&'a str> {
        let start = self.at;
        let first = self.tail()?.chars().next().ok_or(Failure::Stop(StopReason::Malformed))?;
        if !name_start(first) {
            return Err(StopReason::Malformed.into());
        }
        self.at += first.len_utf8();
        while let Some(character) = self.tail()?.chars().next().filter(|character| name_char(*character)) {
            self.tick()?;
            self.at += character.len_utf8();
        }
        self.raw.get(start..self.at).ok_or(Failure::Stop(StopReason::Malformed))
    }
    fn through(&mut self, end: &str) -> ReadResult<Range<usize>> {
        let start = self.at;
        while !self.starts(end) {
            self.tick()?;
            self.at += self.tail()?.chars().next().ok_or(Failure::Stop(StopReason::Malformed))?.len_utf8();
        }
        let range = start..self.at;
        self.at += end.len();
        Ok(range)
    }
    fn emit(&mut self, output: &mut Vec<u8>, character: char) -> ReadResult<()> {
        let mut encoded = [0; 4];
        for byte in character.encode_utf8(&mut encoded).as_bytes() {
            self.budget.retain(output, *byte)?;
        }
        Ok(())
    }
    fn decoded(&mut self, range: Range<usize>, attribute: bool, entities: bool) -> ReadResult<Cow<'a, str>> {
        let raw = self.raw.get(range).ok_or(Failure::Stop(StopReason::Malformed))?;
        let mut changed = false;
        let mut checked = 0;
        for (index, character) in raw.char_indices() {
            if index.saturating_sub(checked) >= 256 {
                self.check()?;
                checked = index;
            }
            if character == '\r' || (entities && character == '&') || (attribute && matches!(character, '\n' | '\t')) {
                changed = true;
                break;
            }
        }
        if !changed {
            return Ok(Cow::Borrowed(raw));
        }
        let mut output = Vec::new();
        let mut at = 0;
        checked = 0;
        while let Some(character) = raw.get(at..).and_then(|tail| tail.chars().next()) {
            if at.saturating_sub(checked) >= 256 {
                self.check()?;
                checked = at;
            }
            at += character.len_utf8();
            let character = if entities && character == '&' {
                let start = at;
                while raw.as_bytes().get(at).is_some_and(|byte| *byte != b';') {
                    if (at - start).is_multiple_of(256) {
                        self.check()?;
                    }
                    if at - start >= 32 && raw.as_bytes().get(start) != Some(&b'#') {
                        return Err(StopReason::Unsupported.into());
                    }
                    at += 1;
                }
                let name = raw.get(start..at).filter(|_| raw.as_bytes().get(at) == Some(&b';')).ok_or(Failure::Stop(StopReason::Malformed))?;
                at += 1;
                let value = match name {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "apos" => '\'',
                    "quot" => '"',
                    _ if name.starts_with('#') => {
                        let (digits, radix) = name.strip_prefix("#x").map_or_else(|| (name.get(1..).unwrap_or_default(), 10), |hex| (hex, 16));
                        if digits.is_empty() {
                            return Err(StopReason::Malformed.into());
                        }
                        let mut value = 0u32;
                        for (index, byte) in digits.bytes().enumerate() {
                            if index.is_multiple_of(256) {
                                self.check()?;
                            }
                            let digit = char::from(byte).to_digit(radix).ok_or(Failure::Stop(StopReason::Malformed))?;
                            value =
                                value.checked_mul(radix).and_then(|value| value.checked_add(digit)).ok_or(Failure::Stop(StopReason::Malformed))?;
                        }
                        char::from_u32(value).filter(|character| xml_char(*character)).ok_or(Failure::Stop(StopReason::Malformed))?
                    }
                    _ => {
                        if !name.chars().next().is_some_and(name_start) || !name.chars().all(name_char) {
                            return Err(StopReason::Malformed.into());
                        }
                        return Err(StopReason::Unsupported.into());
                    }
                };
                self.emit(&mut output, value)?;
                continue;
            } else if character == '\r' {
                if raw.as_bytes().get(at) == Some(&b'\n') {
                    at += 1;
                }
                if attribute { ' ' } else { '\n' }
            } else if attribute && matches!(character, '\n' | '\t') {
                ' '
            } else {
                character
            };
            self.emit(&mut output, character)?;
        }
        Ok(Cow::Owned(String::from_utf8(output).map_err(|_| Failure::Stop(StopReason::Malformed))?))
    }
    fn unique_attribute(&mut self, attributes: &[Attribute<'_>], key: &str) -> ReadResult<()> {
        for (index, previous) in attributes.iter().enumerate() {
            // The cursor does not move while comparing names. Byte-distance
            // polling cannot bound cancellation latency in this charged work.
            if index.is_multiple_of(64) {
                self.check()?;
            }
            self.budget.work(previous.name.len().saturating_add(key.len()).saturating_add(1))?;
            if previous.name == key {
                return Err(StopReason::Malformed.into());
            }
        }
        Ok(())
    }
    fn opening(&mut self) -> ReadResult<()> {
        let start = self.at;
        self.at += 1;
        let name = self.name()?;
        if name.contains(':') {
            return Err(StopReason::Unsupported.into());
        }
        if self.root_closed {
            return Err(StopReason::Malformed.into());
        }
        if self.stack.len().saturating_sub(usize::from(self.implicit_root)) >= self.limits.depth.min(64) {
            return Err(StopReason::RecordLimit.into());
        }
        let mut attributes: Vec<Attribute<'a>> = Vec::new();
        let empty = loop {
            let separated = self.space()?;
            if self.starts("/>") {
                self.at += 2;
                break true;
            }
            if self.starts(">") {
                self.at += 1;
                break false;
            }
            if !separated {
                return Err(StopReason::Malformed.into());
            }
            let attr_start = self.at;
            let key = self.name()?;
            if key == "xmlns" || key.contains(':') {
                return Err(StopReason::Unsupported.into());
            }
            self.attributes = self.attributes.checked_add(1).ok_or(Failure::Stop(StopReason::RecordLimit))?;
            if self.attributes > self.limits.attributes {
                return Err(StopReason::RecordLimit.into());
            }
            self.unique_attribute(&attributes, key)?;
            self.space()?;
            if !self.starts("=") {
                return Err(StopReason::Malformed.into());
            }
            self.at += 1;
            self.space()?;
            let quote =
                self.raw.as_bytes().get(self.at).copied().filter(|byte| matches!(byte, b'\'' | b'"')).ok_or(Failure::Stop(StopReason::Malformed))?;
            self.at += 1;
            let value_start = self.at;
            while self.raw.as_bytes().get(self.at) != Some(&quote) {
                self.tick()?;
                let character = self.tail()?.chars().next().ok_or(Failure::Stop(StopReason::Malformed))?;
                if character == '<' {
                    return Err(StopReason::Malformed.into());
                }
                self.at += character.len_utf8();
            }
            let range = value_start..self.at;
            self.at += 1;
            let value = self.decoded(range, true, true)?;
            self.budget.retain(&mut attributes, Attribute { name: key, value, kind: attribute_kind(key), source: attr_start..self.at })?;
        };
        let index = self.contexts.len();
        let role = role(name);
        self.has_unhandled |= role == Role::Unhandled || attributes.iter().any(|value| value.kind == AttributeKind::Unhandled);
        self.budget.retain(&mut self.contexts, Context { parent: self.stack.last().copied(), name, role, attributes, source: start..self.at })?;
        if self.stack.is_empty() {
            self.root_seen = true;
        }
        if !empty {
            let before = self.budget.retained_left;
            self.budget.retain(&mut self.stack, index)?;
            self.stack_charge = self.stack_charge.checked_add(before - self.budget.retained_left).ok_or(Failure::Stop(StopReason::RetainedLimit))?;
        } else if self.stack.is_empty() {
            self.root_closed = true;
        }
        Ok(())
    }
    fn closing(&mut self) -> ReadResult<()> {
        if self.implicit_root && self.stack.len() == 1 {
            return Err(StopReason::Malformed.into());
        }
        self.at += 2;
        let name = self.name()?;
        self.space()?;
        if !self.starts(">") {
            return Err(StopReason::Malformed.into());
        }
        self.at += 1;
        let index = self.stack.pop().ok_or(Failure::Stop(StopReason::Malformed))?;
        if self.contexts.get(index).is_none_or(|context| context.name != name) {
            return Err(StopReason::Malformed.into());
        }
        if self.stack.is_empty() {
            self.root_closed = true;
        }
        Ok(())
    }
    fn text(&mut self, range: Range<usize>, entities: bool) -> ReadResult<()> {
        let Some(context) = self.stack.last().copied() else {
            if self.raw.get(range).is_some_and(|text| text.bytes().all(whitespace)) {
                return Ok(());
            }
            return Err(StopReason::Malformed.into());
        };
        if entities && self.raw.get(range.clone()).is_some_and(|text| text.contains("]]>")) {
            return Err(StopReason::Malformed.into());
        }
        let retained = self.budget.retained_left;
        let value = self.decoded(range.clone(), false, entities)?;
        let temporary = retained - self.budget.retained_left;
        let start = self.output.len();
        for (index, byte) in value.bytes().enumerate() {
            if index.is_multiple_of(256) {
                self.check()?;
            }
            self.budget.retain(&mut self.output, byte)?;
        }
        drop(value);
        self.budget.retained_left += temporary;
        if self.output.len() != start {
            if self.runs.len() >= self.limits.runs {
                return Err(StopReason::RecordLimit.into());
            }
            self.budget.retain(&mut self.runs, Run { text: start..self.output.len(), context, source: range })?;
        }
        Ok(())
    }
    fn compact_wrapper(&mut self) -> ReadResult<()> {
        self.check()?;
        if !self.contexts.get(1).is_some_and(|context| context.name == "markup") {
            return Ok(());
        }
        for (index, context) in self.contexts.iter().enumerate().skip(2) {
            vector_poll(index, self.keep)?;
            if context.parent == Some(0) {
                return Ok(());
            }
        }
        for (index, run) in self.runs.iter().enumerate() {
            vector_poll(index, self.keep)?;
            if run.context == 0 {
                return Ok(());
            }
        }
        self.contexts.remove(0);
        for (index, context) in self.contexts.iter_mut().enumerate() {
            vector_poll(index, self.keep)?;
            context.parent = context.parent.and_then(|parent| parent.checked_sub(1));
        }
        for (index, run) in self.runs.iter_mut().enumerate() {
            vector_poll(index, self.keep)?;
            run.context = run.context.checked_sub(1).ok_or(Failure::Stop(StopReason::Malformed))?;
        }
        self.check()
    }
    fn parse(mut self) -> ReadResult<Markup<'a>> {
        // Pango treats the complete input as a fragment, even when its first
        // element is named markup: sibling elements and trailing text are valid.
        self.budget.retain(&mut self.contexts, Context { parent: None, name: "markup", role: Role::Markup, attributes: Vec::new(), source: 0..0 })?;
        let retained = self.budget.retained_left;
        self.budget.retain(&mut self.stack, 0)?;
        self.stack_charge = retained - self.budget.retained_left;
        self.implicit_root = true;
        self.root_seen = true;
        while self.at < self.raw.len() {
            self.check()?;
            // End tags do not create another source node or retained context.
            if !self.starts("</") {
                self.nodes = self.nodes.checked_add(1).ok_or(Failure::Stop(StopReason::RecordLimit))?;
            }
            if self.nodes > self.limits.nodes {
                return Err(StopReason::RecordLimit.into());
            }
            if self.starts("<!--") {
                self.at += 4;
                let source = self.through("-->")?;
                if self.raw.get(source).is_some_and(|body| body.contains("--") || body.ends_with('-')) {
                    return Err(StopReason::Malformed.into());
                }
            } else if self.starts("<![CDATA[") {
                if self.stack.is_empty() {
                    return Err(StopReason::Malformed.into());
                }
                self.at += 9;
                let source = self.through("]]>")?;
                self.text(source, false)?;
            } else if self.starts("<!") || self.starts("<?") {
                return Err(StopReason::Unsupported.into());
            } else if self.starts("</") {
                self.closing()?;
            } else if self.starts("<") {
                self.opening()?;
            } else {
                let start = self.at;
                while self.at < self.raw.len() && !self.starts("<") {
                    self.tick()?;
                    self.at += self.tail()?.chars().next().ok_or(Failure::Stop(StopReason::Malformed))?.len_utf8();
                }
                self.text(start..self.at, true)?;
            }
        }
        if if self.implicit_root { self.stack.len() != 1 } else { !self.root_seen || !self.root_closed || !self.stack.is_empty() } {
            return Err(StopReason::Malformed.into());
        }
        // Keep the established context IDs for exactly one explicit wrapper.
        // No outside run may inherit its styles; those fragments keep the root.
        self.compact_wrapper()?;
        drop(self.stack);
        self.budget.retained_left += self.stack_charge;
        Ok(Markup {
            raw: self.raw,
            text: String::from_utf8(self.output).map_err(|_| Failure::Stop(StopReason::Malformed))?,
            runs: self.runs,
            contexts: self.contexts,
            notices: self.notices,
            has_unhandled: self.has_unhandled,
            notices_truncated: self.notices_truncated,
        })
    }
}

fn vector_poll(index: usize, keep: &dyn Fn() -> bool) -> ReadResult<()> {
    if index.is_multiple_of(64) && !keep() { Err(Failure::Cancelled) } else { Ok(()) }
}

// Required record storage is admitted by the caller before optional notices.
fn record_notices(markup: &mut Markup<'_>, limit: usize, budget: &mut Budget, keep: &dyn Fn() -> bool) -> ReadResult<()> {
    for (index, context) in markup.contexts.iter().enumerate() {
        if !keep() {
            return Err(Failure::Cancelled);
        }
        if context.role == Role::Unhandled {
            let notice = Notice { kind: NoticeKind::UnhandledElement, context: index, attribute: None, source: context.source.clone() };
            if markup.notices.len() >= limit || budget.retain(&mut markup.notices, notice).is_err() {
                markup.notices_truncated = true;
            }
        }
        for (attribute, value) in context.attributes.iter().enumerate() {
            // Poll before filtering: recognized attributes still consume work.
            if attribute.is_multiple_of(64) && !keep() {
                return Err(Failure::Cancelled);
            }
            if value.kind != AttributeKind::Unhandled {
                continue;
            }
            let notice = Notice { kind: NoticeKind::UnhandledAttribute, context: index, attribute: Some(attribute), source: value.source.clone() };
            if markup.notices.len() >= limit || budget.retain(&mut markup.notices, notice).is_err() {
                markup.notices_truncated = true;
            }
        }
    }
    Ok(())
}

/// Tokenize one decoded source markup string. No DTD, PI, custom entity or
/// namespace interpretation is performed. Unsupported syntax declines the whole
/// optional record; unknown ordinary vocabulary stays available with notices.
/// Pango fragments may omit the markup wrapper; their contexts inherit a
/// synthetic Markup root with an empty source range, without copying the input.
pub fn tokenize<'a>(raw: &'a str, limits: Limits, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<Scan<Markup<'a>>> {
    let retained = budget.retained_left;
    let result = (|| {
        if !keep_going() {
            return Err(Failure::Cancelled);
        }
        if raw.len() > limits.bytes {
            return Err(StopReason::PayloadLimit.into());
        }
        budget.work(16)?;
        budget.work(raw.len().checked_mul(8).ok_or(Failure::Stop(StopReason::WorkLimit))?)?;
        let mut checked = 0;
        for (index, character) in raw.char_indices() {
            if index.saturating_sub(checked) >= 256 {
                if !keep_going() {
                    return Err(Failure::Cancelled);
                }
                checked = index;
            }
            if !xml_char(character) {
                return Err(StopReason::Malformed.into());
            }
        }
        let parser = Parser {
            raw,
            at: 0,
            checked_at: 0,
            limits,
            budget,
            keep: keep_going,
            output: Vec::new(),
            runs: Vec::new(),
            contexts: Vec::new(),
            stack: Vec::new(),
            stack_charge: 0,
            notices: Vec::new(),
            has_unhandled: false,
            notices_truncated: false,
            nodes: 0,
            attributes: 0,
            root_seen: false,
            root_closed: false,
            implicit_root: false,
        };
        let markup = parser.parse()?;
        let mut records = Vec::new();
        budget.retain(&mut records, markup)?;
        // Optional diagnostics cannot crowd out the required contexts, runs or
        // result slot. Derive them from the retained vocabulary only after all
        // usable markup storage has been admitted.
        if let Some(markup) = records.first_mut() {
            record_notices(markup, limits.notices, budget, keep_going)?;
        }
        Ok(Scan::complete(records, raw.len()))
    })();
    match result {
        Ok(scan) => Ok(scan),
        Err(Failure::Cancelled) => {
            budget.retained_left = retained;
            Err(Error::Cancelled)
        }
        Err(Failure::Stop(reason)) => {
            budget.retained_left = retained;
            Ok(Scan::stopped(Vec::new(), 0, reason))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Original synthetic markup, authored from Pango's documented element and
    // attribute vocabulary. It does not reference font assets or external data.
    #[test]
    fn normalized_multibyte_runs_keep_roles_attributes_and_parent_inheritance() {
        let raw = "<markup><span font-desc='Example Bold 12' foreground='red'>é<b>אב &amp; &#x1f600;</b>x</span></markup>";
        let scan = tokenize(raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        let markup = &scan.records[0];
        assert_eq!(markup.text, "éאב & 😀x");
        assert_eq!(markup.runs.len(), 3);
        let bold = &markup.runs[1];
        assert_eq!(&markup.text[bold.text.clone()], "אב & 😀");
        let inherited: Vec<_> = markup.inherited(bold.context).collect();
        assert_eq!(inherited.iter().map(|context| context.role).collect::<Vec<_>>(), [Role::Bold, Role::Span, Role::Markup]);
        let attribute = &inherited[1].attributes[0];
        assert_eq!(attribute.kind, AttributeKind::FontDescription);
        assert_eq!(attribute.value, "Example Bold 12");
        assert_eq!(&raw[attribute.source.clone()], "font-desc='Example Bold 12'");
        assert!(!markup.has_unhandled);
    }

    #[test]
    fn attribute_and_text_entities_follow_xml_line_normalization_without_expansion() {
        let raw = "<markup><span font='A\tB\r\nC&#10;&amp;&quot;'>a\r\nb&#13;<![CDATA[<x>&raw;\r\nc]]><!--ignored-->d</span></markup>";
        let scan = tokenize(raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        let markup = &scan.records[0];
        assert_eq!(markup.text, "a\nb\r<x>&raw;\ncd");
        assert_eq!(markup.contexts[1].attributes[0].value, "A B C\n&\"");
        assert!(markup.runs.iter().all(|run| markup.text.is_char_boundary(run.text.start) && markup.text.is_char_boundary(run.text.end)));
    }

    #[test]
    fn unknown_formatting_is_retained_once_without_inherited_string_amplification() {
        let value = "é".repeat(4096);
        let raw = format!("<markup><span mystery='{value}'>{}</span></markup>", "<b>x</b>".repeat(256));
        let scan = tokenize(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        let markup = &scan.records[0];
        assert!(markup.has_unhandled);
        assert_eq!(markup.text.len(), 256);
        assert_eq!(markup.runs.len(), 256);
        assert_eq!(markup.notices.len(), 1);
        assert_eq!(markup.notices[0].kind, NoticeKind::UnhandledAttribute);
        let attribute = &markup.contexts[1].attributes[0];
        assert!(matches!(attribute.value, std::borrow::Cow::Borrowed(_)));
        assert_eq!(attribute.value, value);
        assert!(markup.contexts.iter().skip(2).all(|context| context.attributes.is_empty() && context.parent == Some(1)));
    }

    #[test]
    fn malformed_or_unsupported_markup_declines_without_losing_budget_history() {
        for (raw, reason) in [
            ("<markup><b>x</markup>", StopReason::Malformed),
            ("<markup><span size='1' size='2'>x</span></markup>", StopReason::Malformed),
            ("<markup>&unknown;</markup>", StopReason::Unsupported),
            ("<!DOCTYPE markup [<!ENTITY x 'y'>]><markup>&x;</markup>", StopReason::Unsupported),
            ("<markup xmlns='elsewhere'>x</markup>", StopReason::Unsupported),
            ("<markup><foreign:b>x</foreign:b></markup>", StopReason::Unsupported),
            ("<markup>&#0;</markup>", StopReason::Malformed),
            ("<markup>&#+65;</markup>", StopReason::Malformed),
            ("<markup>&#x+41;</markup>", StopReason::Malformed),
            ("<markup>]]></markup>", StopReason::Malformed),
            ("<markup/><!-- bad -- comment -->", StopReason::Malformed),
        ] {
            let mut budget = Budget::default();
            let retained = budget.retained_remaining();
            let work = budget.work_remaining();
            let scan = tokenize(raw, Limits::default(), &mut budget, &|| true).unwrap();
            assert!(scan.records.is_empty(), "{raw}");
            assert_eq!(scan.stopped.unwrap().reason, reason, "{raw}");
            assert_eq!(budget.retained_remaining(), retained);
            assert!(budget.work_remaining() < work);
        }
    }

    #[test]
    fn bounded_notices_and_limits_do_not_claim_complete_formatting() {
        let raw = "<markup><odd unknown='one'>a</odd><other unknown='two'>b</other></markup>";
        let scan = tokenize(raw, Limits { notices: 1, ..Limits::default() }, &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        let markup = &scan.records[0];
        assert!(markup.has_unhandled && markup.notices_truncated);
        assert_eq!(markup.notices.len(), 1);
        assert_eq!(markup.text, "ab");
        for limits in
            [Limits { depth: 1, ..Limits::default() }, Limits { attributes: 0, ..Limits::default() }, Limits { runs: 0, ..Limits::default() }]
        {
            let scan = tokenize(raw, limits, &mut Budget::default(), &|| true).unwrap();
            assert_eq!(scan.stopped.unwrap().reason, StopReason::RecordLimit);
        }
    }

    #[test]
    fn shared_work_and_cancellation_bound_empty_records_and_long_values() {
        let raw = "<markup>text</markup>";
        let mut budget = Budget::new(16 + 8 * raw.len(), crate::MAX_METADATA_BYTES);
        assert!(tokenize(raw, Limits::default(), &mut budget, &|| true).unwrap().stopped.is_none());
        assert_eq!(budget.work_remaining(), 0);
        assert_eq!(tokenize(raw, Limits::default(), &mut budget, &|| true).unwrap().stopped.unwrap().reason, StopReason::WorkLimit);
        assert!(matches!(tokenize("", Limits::default(), &mut Budget::new(0, 0), &|| false), Err(crate::Error::Cancelled)));
        let raw = format!("<markup><span unknown='{}'>text</span></markup>", "x".repeat(4096));
        let checks = std::cell::Cell::new(0);
        let keep = || {
            checks.set(checks.get() + 1);
            checks.get() < 5
        };
        assert!(matches!(tokenize(&raw, Limits::default(), &mut Budget::default(), &keep), Err(crate::Error::Cancelled)));
    }

    #[test]
    fn duplicate_name_comparisons_are_charged_before_large_tag_work() {
        let attributes = (0..32).map(|index| format!(" x{index}='value'")).collect::<String>();
        let raw = format!("<markup{attributes}>text</markup>");
        let mut budget = Budget::new(16 + 8 * raw.len() + 32, crate::MAX_METADATA_BYTES);
        let retained = budget.retained_remaining();
        let scan = tokenize(&raw, Limits::default(), &mut budget, &|| true).unwrap();
        assert!(scan.records.is_empty());
        assert_eq!(scan.stopped.unwrap().reason, StopReason::WorkLimit);
        assert_eq!(budget.retained_remaining(), retained);
        assert!(budget.work_remaining() < 32);
    }

    #[test]
    fn unicode_unknown_names_and_truncated_markup_remain_bounded() {
        let raw = "<markup><任意 λ='é'>字&#65;</任意></markup>";
        let scan = tokenize(raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        assert_eq!(scan.records[0].text, "字A");
        assert!(scan.records[0].has_unhandled);
        assert_eq!(scan.records[0].notices.len(), 2);
        for (end, _) in raw.char_indices() {
            let scan = tokenize(&raw[..end], Limits::default(), &mut Budget::default(), &|| true).unwrap();
            if end == 0 {
                assert_eq!(scan.stopped, None, "the empty Pango fragment is valid");
                assert!(scan.records[0].text.is_empty());
                continue;
            }
            assert!(scan.stopped.is_some(), "truncated XML accepted at {end}");
        }
        let deep = format!("{}x{}", "<span>".repeat(65), "</span>".repeat(65));
        let scan = tokenize(&deep, Limits { depth: usize::MAX, ..Limits::default() }, &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, StopReason::RecordLimit);
    }
}
