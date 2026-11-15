//! `Codec`: how a text part turns a value into text and text into a value (18.2), with the library codecs of this
//! crate.

use crate::quantity::Reader;
use std::borrow::Cow;
use std::fmt;
use std::ops::RangeInclusive;
use std::sync::Arc;

/// The text is not a value of the codec's type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotAValue;

/// What a parse learned beside the value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ParseNote {
    /// The tag of the unit the text named.
    pub unit: Option<u32>,
    /// The number read before the codec held it to its range.
    pub held: Option<f64>,
    /// The text held more than the codec reads; the value is what came before the cut.
    pub cut: bool,
}

/// Where the caret goes when the part takes focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Caret {
    /// The whole text is selected.
    All,
    /// The caret sits after the last character.
    End,
}

/// A value's text form.
pub trait Codec {
    /// The value the text stands for.
    type Value: Clone + PartialEq + Send + Sync + 'static;

    /// Every draft is a value (plain text): a draft applies as typed and never rejects.
    const DRAFTS_ARE_VALUES: bool = false;

    /// The text of `v`.
    fn format(&self, v: &Self::Value) -> String;

    /// The value of `text`; `start` is the value when editing began, for relative input.
    fn parse(&self, text: &str, start: &Self::Value) -> Result<Self::Value, NotAValue>;

    /// The value of `text` and what the parse learned beside it (`Reader::read_tagged`); the default notes nothing.
    fn parse_tagged(&self, text: &str, start: &Self::Value) -> Result<(Self::Value, ParseNote), NotAValue> {
        self.parse(text, start).map(|value| (value, ParseNote::default()))
    }

    /// Whether `text` applies as soon as it is typed rather than on commit.
    fn applies_as_typed(&self, _text: &str) -> bool {
        false
    }

    /// Whether the codec takes `c` as typed.
    fn filter(&self, _c: char) -> bool {
        true
    }

    /// The text the field shows after committing `v`, which was parsed from `text`.
    fn normalise(&self, _text: &str, v: &Self::Value) -> String {
        self.format(v)
    }

    /// Where the caret goes when the part takes focus with this text.
    fn caret_on_focus(&self, _text: &str) -> Caret {
        Caret::All
    }

    /// The widest typical text, for the field's ideal width. Codecs with bounded text only; `PlainText` returns
    /// empty text, so a plain field never sizes from what the user typed.
    fn sample(&self, current: &Self::Value) -> Cow<'_, str> {
        self.format(current).into()
    }
}

impl<C: Codec + ?Sized> Codec for &C {
    type Value = C::Value;
    const DRAFTS_ARE_VALUES: bool = C::DRAFTS_ARE_VALUES;

    fn format(&self, v: &Self::Value) -> String {
        (**self).format(v)
    }

    fn parse(&self, text: &str, start: &Self::Value) -> Result<Self::Value, NotAValue> {
        (**self).parse(text, start)
    }

    fn parse_tagged(&self, text: &str, start: &Self::Value) -> Result<(Self::Value, ParseNote), NotAValue> {
        (**self).parse_tagged(text, start)
    }

    fn applies_as_typed(&self, text: &str) -> bool {
        (**self).applies_as_typed(text)
    }

    fn filter(&self, c: char) -> bool {
        (**self).filter(c)
    }

    fn normalise(&self, text: &str, v: &Self::Value) -> String {
        (**self).normalise(text, v)
    }

    fn caret_on_focus(&self, text: &str) -> Caret {
        (**self).caret_on_focus(text)
    }

    fn sample(&self, current: &Self::Value) -> Cow<'_, str> {
        (**self).sample(current)
    }
}

/// Plain text: every draft is a value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlainText;

impl Codec for PlainText {
    type Value = String;
    const DRAFTS_ARE_VALUES: bool = true;

    fn format(&self, v: &String) -> String {
        v.clone()
    }

    fn parse(&self, text: &str, _start: &String) -> Result<String, NotAValue> {
        Ok(text.to_owned())
    }

    fn applies_as_typed(&self, _text: &str) -> bool {
        true
    }

    fn caret_on_focus(&self, _text: &str) -> Caret {
        Caret::End
    }

    fn sample(&self, _current: &String) -> Cow<'_, str> {
        Cow::Borrowed("")
    }
}

/// A reader a caller adds for words that stand for numbers.
type Custom = Arc<dyn Fn(&str) -> Option<f64> + Send + Sync>;

/// An optional number: empty text is `None`; anything else reads through a [`Reader`].
#[derive(Clone)]
pub struct NumberCodec {
    reader: Reader,
    decimals: usize,
    /// Every decimal place is written, trailing zeros included.
    fixed: bool,
    custom: Option<Custom>,
    /// Typing rules of a zoom field ([`NumberCodec::zoom_typing`]).
    zoom: bool,
}

impl fmt::Debug for NumberCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NumberCodec")
            .field("reader", &self.reader)
            .field("decimals", &self.decimals)
            .field("fixed", &self.fixed)
            .field("zoom", &self.zoom)
            .field("custom", &self.custom.is_some())
            .finish()
    }
}

impl PartialEq for NumberCodec {
    fn eq(&self, other: &Self) -> bool {
        let same_custom = match (&self.custom, &other.custom) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        self.reader == other.reader && self.decimals == other.decimals && self.fixed == other.fixed && self.zoom == other.zoom && same_custom
    }
}

/// The decimal places a step needs to be written exactly: 2 for 0.25, 3 for 0.001, 0 for a whole step. A step with
/// no finite decimal form within `MAX_STEP_DECIMALS` places gets that many.
pub fn decimals_for_step(step: f64) -> usize {
    let step = step.abs();
    if !step.is_finite() || step == 0.0 {
        return 0;
    }
    let mut scale = 1.0;
    for places in 0..MAX_STEP_DECIMALS {
        let scaled = step * scale;
        if (scaled - scaled.round()).abs() <= scaled.max(1.0) * STEP_TOLERANCE {
            return places;
        }
        scale *= 10.0;
    }
    MAX_STEP_DECIMALS
}

/// The most places `decimals_for_step` reports.
const MAX_STEP_DECIMALS: usize = 12;

/// How far a scaled step may sit from a whole number, relative to its size, and still count as whole.
const STEP_TOLERANCE: f64 = 1e-9;

/// The widest number a field is sized for.
const NUMBER_SAMPLE: &str = "0000.00";

impl NumberCodec {
    /// A codec reading with `reader` and writing at most `decimals` decimal places.
    pub fn new(reader: Reader, decimals: usize) -> Self {
        NumberCodec { reader, decimals, fixed: false, custom: None, zoom: false }
    }

    /// The same codec writing every decimal place, trailing zeros included ("1.00"), as a field of a quantity shown
    /// at a set precision does.
    pub fn fixed(mut self) -> Self {
        self.fixed = true;
        self
    }

    /// The same codec with the typing rules of a zoom field: digits, `/` and the decimal separator, `a/b` as the
    /// percentage `a ÷ b × 100`, and a second separator cutting the text.
    pub fn zoom_typing(mut self) -> Self {
        self.zoom = true;
        self
    }

    /// The zoom reading of trimmed, non-empty `text`: the text before a second decimal separator, then `a/b` as a
    /// percentage or a plain number. The filter keeps `+`, `-` and `*` out, so the reader's expressions never run.
    fn read_zoom(&self, text: &str) -> Result<(f64, ParseNote), NotAValue> {
        let separator = self.reader.decimal_separator();
        let second = text.char_indices().filter(|&(_, c)| c == separator).nth(1).map(|(at, _)| at);
        let (kept, cut) = match second {
            Some(at) => (&text[..at], true),
            None => (text, false),
        };
        let number = match kept.split_once('/') {
            Some((part, whole)) => {
                let part = self.reader.read(part, None)?;
                let whole = self.reader.read(whole, None)?;
                if whole == 0.0 {
                    return Err(NotAValue);
                }
                part / whole * 100.0
            }
            None => self.reader.read(kept, None)?,
        };
        Ok((number, ParseNote { cut, ..ParseNote::default() }))
    }

    /// The same codec writing at most `decimals` decimal places.
    pub fn at_most(mut self, decimals: usize) -> Self {
        self.decimals = self.decimals.min(decimals);
        self
    }

    /// The same codec with `parse` as a second reader: it runs when the reader rejects the text, so words of the
    /// field's context ("Auto") can stand for numbers.
    pub fn or_parse(mut self, parse: impl Fn(&str) -> Option<f64> + Send + Sync + 'static) -> Self {
        self.custom = Some(Arc::new(parse));
        self
    }

    /// The widest text the codec writes for a number in `range`: the longest of each end and the number one place of
    /// the last decimal inside it ("100" and "99.9" over 0 to 100 at one decimal), so a field over the range is sized
    /// for any value it shows in it.
    pub fn widest(&self, range: &RangeInclusive<f64>) -> String {
        let (low, high) = (*range.start(), *range.end());
        let place = 10f64.powi(-i32::try_from(self.decimals).unwrap_or(i32::MAX));
        let inside = |end: f64, other: f64| (self.decimals > 0 && (other - end).abs() >= place).then(|| end + place.copysign(other - end));
        [Some(low), Some(high), inside(low, high), inside(high, low)]
            .into_iter()
            .flatten()
            .map(|number| self.format(&Some(number)))
            .max_by_key(|text| text.chars().count())
            .unwrap_or_default()
    }

    /// The most decimal places the codec writes.
    pub fn decimals(&self) -> usize {
        self.decimals
    }

    /// The reader the codec parses with.
    pub fn reader(&self) -> &Reader {
        &self.reader
    }
}

impl Default for NumberCodec {
    fn default() -> Self {
        NumberCodec::new(Reader::new(), 3)
    }
}

impl Codec for NumberCodec {
    type Value = Option<f64>;

    fn format(&self, v: &Option<f64>) -> String {
        let Some(number) = v else { return String::new() };
        let fixed = format!("{number:.*}", self.decimals);
        let trimmed = if self.fixed {
            &fixed
        } else if fixed.contains('.') {
            fixed.trim_end_matches('0').trim_end_matches('.')
        } else {
            &fixed
        };
        trimmed.replace('.', &self.reader.decimal_separator().to_string())
    }

    fn parse(&self, text: &str, start: &Option<f64>) -> Result<Option<f64>, NotAValue> {
        self.parse_tagged(text, start).map(|(value, _)| value)
    }

    /// The note's unit is the tag of the unit the text named; `None` when it named no tagged unit or a custom parser
    /// read it. With zoom typing the note's `cut` says a second decimal separator cut the text.
    fn parse_tagged(&self, text: &str, start: &Option<f64>) -> Result<(Option<f64>, ParseNote), NotAValue> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok((None, ParseNote::default()));
        }
        if self.zoom {
            return self.read_zoom(trimmed).map(|(number, note)| (Some(number), note));
        }
        match self.reader.read_tagged(text, *start) {
            Ok((number, tag)) => Ok((Some(number), ParseNote { unit: tag, ..ParseNote::default() })),
            Err(rejected) => {
                self.custom.as_ref().and_then(|parse| parse(text.trim())).map(|number| (Some(number), ParseNote::default())).ok_or(rejected)
            }
        }
    }

    /// Zoom typing takes ASCII digits, `/` and the decimal separator; every other number field takes every character.
    fn filter(&self, c: char) -> bool {
        !self.zoom || c.is_ascii_digit() || c == '/' || c == self.reader.decimal_separator()
    }

    fn sample(&self, _current: &Option<f64>) -> Cow<'_, str> {
        Cow::Borrowed(NUMBER_SAMPLE)
    }
}
