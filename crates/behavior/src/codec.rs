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

    /// The value of `text` and the tag of the unit it named (`Reader::read_tagged`); the default names none.
    fn parse_tagged(&self, text: &str, start: &Self::Value) -> Result<(Self::Value, Option<u32>), NotAValue> {
        self.parse(text, start).map(|value| (value, None))
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

    fn parse_tagged(&self, text: &str, start: &Self::Value) -> Result<(Self::Value, Option<u32>), NotAValue> {
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
}

impl fmt::Debug for NumberCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NumberCodec")
            .field("reader", &self.reader)
            .field("decimals", &self.decimals)
            .field("fixed", &self.fixed)
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
        self.reader == other.reader && self.decimals == other.decimals && self.fixed == other.fixed && same_custom
    }
}

/// The decimal places a step needs to be written exactly: 2 for 0.25, 3 for 0.001, 0 for a whole step. A step with
/// no finite decimal form within [`MAX_STEP_DECIMALS`] places gets that many.
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
        NumberCodec { reader, decimals, fixed: false, custom: None }
    }

    /// The same codec writing every decimal place, trailing zeros included ("1.00"), as a field of a quantity shown
    /// at a set precision does.
    pub fn fixed(mut self) -> Self {
        self.fixed = true;
        self
    }

    /// The same codec with `parse` as a second reader: it runs when the reader rejects the text, so words of the
    /// field's context ("Auto") can stand for numbers.
    pub fn or_parse(mut self, parse: impl Fn(&str) -> Option<f64> + Send + Sync + 'static) -> Self {
        self.custom = Some(Arc::new(parse));
        self
    }

    /// The widest text the codec writes for a number in `range`: the longer of its two ends, with every decimal
    /// place, so a field over the range is sized for any value in it.
    pub fn widest(&self, range: &RangeInclusive<f64>) -> String {
        let text = |number: f64| format!("{number:.*}", self.decimals).replace('.', &self.reader.decimal_separator().to_string());
        let (low, high) = (text(*range.start()), text(*range.end()));
        if low.chars().count() > high.chars().count() { low } else { high }
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

    /// The tag is that of the unit the text named; `None` when it named no tagged unit or a custom parser read it.
    fn parse_tagged(&self, text: &str, start: &Option<f64>) -> Result<(Option<f64>, Option<u32>), NotAValue> {
        if text.trim().is_empty() {
            return Ok((None, None));
        }
        match self.reader.read_tagged(text, *start) {
            Ok((number, tag)) => Ok((Some(number), tag)),
            Err(rejected) => self.custom.as_ref().and_then(|parse| parse(text.trim())).map(|number| (Some(number), None)).ok_or(rejected),
        }
    }

    fn sample(&self, _current: &Option<f64>) -> Cow<'_, str> {
        Cow::Borrowed(NUMBER_SAMPLE)
    }
}
