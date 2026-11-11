//! Numbers, percentages and counts written for a formatting [`Region`], on ICU4X's decimal
//! formatter with CLDR data for every CLDR locale (`data/generate.sh`).

use std::fmt;

use fixed_decimal::{Decimal, Sign};
use icu_decimal::{DecimalFormatter, parts};
use icu_locale_core::Locale;
use icu_locale_core::extensions::unicode::{key, value};
use writeable::{Part, PartsWrite, Writeable};

use crate::Region;
use crate::data::{self, PERCENT};
use crate::number::{Marks, is_bidi_mark};

/// Which digits numbers are written in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Digits {
    /// The locale's own: Arabic-Indic in Egypt (`١٢٣`), Persian in Iran (`۱۲۳`), else `0`–`9`.
    #[default]
    Locale,
    /// `0`–`9` everywhere, for users who read Arabic or Persian with Latin digits.
    Latin,
}

/// How numbers are written in a formatting region: digits, decimal and grouping separators,
/// grouping sizes (`1,234,567`, Indian `12,34,567`) and the percent sign. Build it once per region
/// and keep it; [`crate::I18n`] holds one for counts, sizes and measures.
#[derive(Debug)]
pub struct Format {
    /// `None` only if the data doesn't load: numbers are then written as Rust writes them.
    decimal: Option<DecimalFormatter>,
    /// The system's separators where they differ from the locale's; an empty `group` is none.
    group: Option<String>,
    point: Option<String>,
    percent: (&'static str, &'static str),
    /// The marks [`Format::parse`] reads.
    pub(crate) marks: Marks,
    /// The decimal and group marks of the written form, read once from a number that shows both.
    separators: (char, Option<char>),
}

impl Format {
    /// The format for `region`'s number locale. With Latin digits, a decimal or grouping separator
    /// the region carries (Windows' and macOS's own settings) replaces the locale's.
    pub fn new(region: &Region, digits: Digits) -> Format {
        let mut locale = Locale::try_from_str(&region.numbers).unwrap_or(Locale::UNKNOWN);
        if digits == Digits::Latin {
            locale.extensions.unicode.keywords.set(key!("nu"), value!("latn"));
        }
        let decimal = data::provider().and_then(|p| DecimalFormatter::try_new_with_buffer_provider(p, (&locale).into(), Default::default()).ok());
        let mut format = Format { decimal, group: None, point: None, percent: ("", "%"), marks: Marks::default(), separators: ('.', None) };
        // The locale's own separators and digits, read from a number that shows them all.
        let probe = format.parts(&Decimal::try_from_str("-1234567.5").unwrap_or_default());
        let latin = probe.integer.starts_with(|c: char| c.is_ascii_digit());
        if latin {
            if let Some(point) = region.decimal.map(String::from).filter(|p| *p != probe.point) {
                format.point = Some(point);
            }
            if let Some(g) = region.grouping.map(|g| g.map(String::from).unwrap_or_default()).filter(|g| *g != probe.group) {
                format.group = Some(g);
            }
            // A grouping mark equal to the decimal mark (`1,234,5`) wouldn't read back. With a
            // decimal comma and the locale's grouping, `1,234.5` becomes `1.234,5`; a grouping
            // comma the system sets as well (Windows always sets one) leaves no grouping.
            let point = format.point.as_deref().unwrap_or(&probe.point);
            if format.group.as_deref().unwrap_or(&probe.group) == point {
                let swapped = region.grouping.is_none() && probe.point != point;
                format.group = Some(if swapped { probe.point.clone() } else { String::new() });
            }
        }
        format.percent = percent_affixes(&region.numbers, latin);
        let point = format.point.clone().unwrap_or(probe.point);
        let group = format.group.clone().unwrap_or(probe.group);
        // Latin digits typed into an Arabic-Indic format take the locale's Latin marks.
        let comma = match (latin, digits) {
            (true, _) => point == "," || group == ".",
            (false, Digits::Locale) => !Format::new(region, Digits::Latin).marks.point,
            (false, Digits::Latin) => false,
        };
        format.marks = Marks::new(&point, &group, !comma);
        format.separators = format.written_separators();
        format
    }

    /// The decimal and group marks numbers are written with, in the digits of the format. The group mark is
    /// `None` where the format writes none.
    pub fn separators(&self) -> (char, Option<char>) {
        self.separators
    }

    fn written_separators(&self) -> (char, Option<char>) {
        let sample = self.number(1_234_567.5, 1);
        let marks: Vec<char> = sample.chars().filter(|c| !c.is_alphanumeric() && !c.is_whitespace() && !is_bidi_mark(*c)).collect();
        let decimal = marks.last().copied().unwrap_or('.');
        let group = marks.first().copied().filter(|group| *group != decimal);
        (decimal, group)
    }

    /// The format for a locale tag (`de-CH`, `ar-EG`), with the locale's own digits.
    pub fn from_tag(tag: &str) -> Format {
        Format::new(&Region::from_tag(tag), Digits::Locale)
    }

    /// A whole number: `1,234,567`, `1.234.567`, `١٬٢٣٤٬٥٦٧`.
    pub fn integer(&self, n: impl Into<i128>) -> String {
        self.write(&Decimal::from(n.into()))
    }

    /// `value` rounded to `decimals` digits after the decimal separator; infinities and NaN are
    /// `∞`, `-∞` and `NaN`.
    pub fn number(&self, value: f64, decimals: usize) -> String {
        match to_decimal(value, decimals) {
            Some(d) => self.write(&d),
            None => non_finite(value).to_owned(),
        }
    }

    /// [`Self::number`] without grouping separators, for values such as pixel sizes and years:
    /// `1920`, `١٩٢٠`, `1234,5`.
    pub fn number_ungrouped(&self, value: f64, decimals: usize) -> String {
        match to_decimal(value, decimals) {
            Some(d) => {
                let p = self.parts_with(&d, Some(""));
                p.sign + &p.text
            }
            None => non_finite(value).to_owned(),
        }
    }

    /// `ratio` as a percentage with `decimals` digits after the decimal separator: `0.5` is
    /// `50%`, `50 %` (French, German), `%50` (Turkish), `٥٠٪؜` (Egyptian Arabic).
    pub fn percent(&self, ratio: f64, decimals: usize) -> String {
        let Some(d) = to_decimal(ratio * 100.0, decimals) else {
            return format!("{}{}{}", self.percent.0, non_finite(ratio), self.percent.1);
        };
        let p = self.parts(&d);
        format!("{}{}{}{}", p.sign, self.percent.0, p.text, self.percent.1)
    }

    /// `d` as text, the sign in place.
    pub(crate) fn write(&self, d: &Decimal) -> String {
        let p = self.parts(d);
        p.sign + &p.text
    }

    fn parts(&self, d: &Decimal) -> Written {
        self.parts_with(d, self.group.as_deref())
    }

    /// `d` taken apart, with `group` in place of the locale's grouping separator if set (`""`
    /// drops it).
    fn parts_with(&self, d: &Decimal, group: Option<&str>) -> Written {
        let mut sink = Sink { group, point: self.point.as_deref(), into: Target::Text, w: Written::default() };
        let written = match &self.decimal {
            Some(f) => f.format(d).write_to_parts(&mut sink),
            None => fmt::Write::write_str(&mut sink, &d.to_string()),
        };
        if written.is_err() {
            return Written { text: d.to_string(), ..Written::default() };
        }
        sink.w
    }
}

/// A formatted number taken apart: its sign, the rest, and the separators and integer digits as
/// the locale wrote them.
#[derive(Default)]
struct Written {
    sign: String,
    text: String,
    integer: String,
    group: String,
    point: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Target {
    Text,
    Sign,
    /// A separator the region replaces; the locale's goes to `group` or `point` only.
    Replaced,
}

/// Writes the formatter's parts into [`Written`], swapping in the region's separators.
struct Sink<'a> {
    group: Option<&'a str>,
    point: Option<&'a str>,
    into: Target,
    w: Written,
}

impl fmt::Write for Sink<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        match self.into {
            Target::Text => self.w.text.push_str(s),
            Target::Sign => self.w.sign.push_str(s),
            Target::Replaced => {}
        }
        Ok(())
    }
}

impl PartsWrite for Sink<'_> {
    type SubPartsWrite = Self;

    fn with_part(&mut self, part: Part, mut f: impl FnMut(&mut Self) -> fmt::Result) -> fmt::Result {
        let outer = self.into;
        let start = self.w.text.len();
        let replacement = match part {
            p if p == parts::GROUP => self.group,
            p if p == parts::DECIMAL => self.point,
            _ => None,
        };
        if let Some(r) = replacement {
            self.w.text.push_str(r);
            self.into = Target::Replaced;
        } else if part == parts::MINUS_SIGN || part == parts::PLUS_SIGN {
            self.into = Target::Sign;
        }
        let result = f(self);
        self.into = outer;
        if replacement.is_none() {
            let written = self.w.text.get(start..).unwrap_or_default();
            let slot = match part {
                p if p == parts::GROUP => &mut self.w.group,
                p if p == parts::DECIMAL => &mut self.w.point,
                p if p == parts::INTEGER => &mut self.w.integer,
                _ => return result,
            };
            if slot.is_empty() {
                *slot = written.to_owned();
            }
        }
        result
    }
}

/// `value` rounded to `decimals` digits after the point; `None` for infinities and NaN. A value
/// that rounds to zero has no sign.
pub(crate) fn to_decimal(value: f64, decimals: usize) -> Option<Decimal> {
    if !value.is_finite() {
        return None;
    }
    let mut d = Decimal::try_from_str(&format!("{value:.decimals$}")).ok()?;
    if d.absolute.is_zero() {
        d.sign = Sign::None;
    }
    Some(d)
}

/// CLDR's root symbols for a value that isn't finite.
fn non_finite(value: f64) -> &'static str {
    match value {
        f64::INFINITY => "∞",
        f64::NEG_INFINITY => "-∞",
        _ => "NaN",
    }
}

/// The percent affixes for a number locale (`ar-EG`), found by dropping trailing subtags.
fn percent_affixes(tag: &str, latin: bool) -> (&'static str, &'static str) {
    let mut tag = tag;
    loop {
        if let Some((_, _, before, after)) = PERCENT.iter().find(|(l, lat, _, _)| *lat == latin && l.eq_ignore_ascii_case(tag)) {
            return (before, after);
        }
        match tag.rsplit_once('-') {
            Some((parent, _)) => tag = parent,
            None => return ("", "%"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows always sets `sThousand`, so a decimal comma the user picks arrives beside a grouping
    /// comma: the numbers drop grouping rather than read `1,234,5`.
    #[test]
    fn a_system_grouping_mark_equal_to_the_decimal_mark_is_dropped() {
        let reg = |pairs: &'static [(&'static str, &'static str)]| move |n: &str| pairs.iter().find(|(k, _)| *k == n).map(|(_, v)| v.to_string());
        let region = crate::region::from_windows(reg(&[("LocaleName", "en-US"), ("sDecimal", ","), ("sThousand", ",")]), || None);
        let format = Format::new(&region, Digits::Locale);
        assert_eq!((format.number(1234.5, 1), format.integer(1_234_567)), ("1234,5".to_owned(), "1234567".to_owned()));
        let region = crate::region::from_windows(reg(&[("LocaleName", "de-DE"), ("sDecimal", ","), ("sThousand", ",")]), || None);
        assert_eq!(Format::new(&region, Digits::Locale).number(1234.5, 1), "1234,5");
        let region = crate::region::from_windows(reg(&[("LocaleName", "en-US"), ("sDecimal", "."), ("sThousand", ".")]), || None);
        assert_eq!(Format::new(&region, Digits::Locale).number(1234.5, 1), "1234.5");
    }
}
