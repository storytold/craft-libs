//! `quantity::Reader`: the one number and unit parser (18.3): locale digits, unit spellings, expressions and
//! relative input.

use crate::codec::NotAValue;

/// First code point of each supported digit block; each block has ten consecutive digits.
const DIGIT_BLOCKS: [u32; 5] = [0x30, 0x660, 0x6F0, 0x966, 0xFF10];

/// The value of a decimal digit of any supported script.
fn digit(c: char) -> Option<u32> {
    DIGIT_BLOCKS.iter().find_map(|&zero| (zero..zero + 10).contains(&(c as u32)).then(|| c as u32 - zero))
}

/// Whether `c` can be part of a unit spelling: a letter or a symbol that stands for a unit (`%`, `°`). `×` is the
/// product sign, never part of a unit.
fn is_unit_char(c: char) -> bool {
    c.is_alphabetic() || matches!(c, '%' | '\u{2030}' | '°' | '′' | '″')
}

/// Whether `spelling` can be registered with [`Reader::unit`]: letters and unit symbols only, so the reader can
/// tell it from a number or an operator.
pub fn is_unit_spelling(spelling: &str) -> bool {
    !spelling.is_empty() && spelling.chars().all(is_unit_char)
}

/// A unit spelling and the factor that turns its quantity into the base unit.
#[derive(Clone, Debug, PartialEq)]
struct Unit {
    spelling: String,
    factor: f64,
    /// What the caller named this unit by, handed back by [`Reader::read_tagged`].
    tag: Option<u32>,
    /// The factor of a number written right after the spelling ([`Reader::split_unit`]).
    rest: Option<f64>,
}

/// Reads numbers, expressions and unit quantities written in one locale.
///
/// Grammar: `expr = term (('+' | '-') term)*`, `term = factor (('*' | '/' | 'x' | 'X' | '×') factor)*`,
/// `factor = ('-' | '+')? (number unit? | '(' expr ')')`. A factor after `*`, `/`, `x` or `×` may be a percentage
/// (`3cm*50%` is 1.5 cm), whatever `%` means elsewhere. Text that starts with `+`, `*` or `/` is relative: it
/// applies to the start value (`+5` adds 5, `*2` doubles). A leading `-` is a negative number. A unit is the longest
/// registered spelling at the start of the letters after a number, so `2inx3` is `2in` times 3 and `2px` stays a
/// quantity when `px` is registered. A group separator counts only before exactly three digits.
#[derive(Clone, Debug, PartialEq)]
pub struct Reader {
    decimal: char,
    group: Option<char>,
    units: Vec<Unit>,
    /// The factor of a number with no unit ([`Reader::bare`]).
    bare: f64,
}

impl Default for Reader {
    fn default() -> Self {
        Reader { decimal: '.', group: None, units: Vec::new(), bare: 1.0 }
    }
}

impl Reader {
    /// A reader for plain numbers written with `.` as the decimal separator.
    pub fn new() -> Self {
        Self::default()
    }

    /// A number with no unit counts as `factor` of the reader's base unit (a picas field reads bare numbers as
    /// points: `1.0 / 12.0`). The factor applies to the first number of each term; a number after `*`, `/`, `x` or
    /// `×` is a plain multiplier, so `12*2` is two factors of twelve points.
    #[must_use]
    pub fn bare(mut self, factor: f64) -> Self {
        self.bare = factor;
        self
    }

    /// The decimal separator.
    #[must_use]
    pub fn decimal(mut self, decimal: char) -> Self {
        self.decimal = decimal;
        self
    }

    /// The digit-group separator, which the reader skips between digits.
    #[must_use]
    pub fn group(mut self, group: char) -> Self {
        self.group = Some(group);
        self
    }

    /// A unit spelling (case-insensitive; letters or the symbols `%`, `°`) and its factor to the base unit.
    /// Spaces between the number and the spelling are skipped, so "12 %" reads like "12%".
    #[must_use]
    pub fn unit(mut self, spelling: &str, factor: f64) -> Self {
        self.units.push(Unit { spelling: spelling.to_lowercase(), factor, tag: None, rest: None });
        self
    }

    /// Like [`Reader::unit`], with a `tag` that [`Reader::read_tagged`] returns when the text names this spelling.
    #[must_use]
    pub fn unit_tagged(mut self, spelling: &str, factor: f64, tag: u32) -> Self {
        self.units.push(Unit { spelling: spelling.to_lowercase(), factor, tag: Some(tag), rest: None });
        self
    }

    /// Like [`Reader::unit_tagged`], where a number written right after the spelling adds that many of `rest`: picas
    /// and points read as `2p6`, two picas and six points.
    #[must_use]
    pub fn split_unit(mut self, spelling: &str, factor: f64, rest: f64, tag: u32) -> Self {
        self.units.push(Unit { spelling: spelling.to_lowercase(), factor, tag: Some(tag), rest: Some(rest) });
        self
    }

    /// The decimal separator the reader reads and writes.
    pub fn decimal_separator(&self) -> char {
        self.decimal
    }

    /// The value of `text`. `start` is the value relative input applies to.
    pub fn read(&self, text: &str, start: Option<f64>) -> Result<f64, NotAValue> {
        self.read_tagged(text, start).map(|(value, _)| value)
    }

    /// The value of `text` and the tag of the unit its last quantity named (`None` for a bare number or an untagged
    /// unit).
    pub fn read_tagged(&self, text: &str, start: Option<f64>) -> Result<(f64, Option<u32>), NotAValue> {
        let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
        let mut parse = Parse { reader: self, chars: &chars, at: 0, tag: None };
        let value = match (chars.first(), start) {
            (Some('+' | '*' | '/'), Some(start)) => parse.expr_from(start)?,
            _ => parse.expr()?,
        };
        (parse.at == chars.len() && value.is_finite()).then_some((value, parse.tag)).ok_or(NotAValue)
    }

    /// The fraction a lone percentage writes (`50%` is 0.5): a number and `%`, with no sign, unit or operator;
    /// `None` for any other text.
    pub fn share(&self, text: &str) -> Option<f64> {
        let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
        let number = chars.strip_suffix(&['%'])?;
        let mut parse = Parse { reader: self, chars: number, at: 0, tag: None };
        let value = parse.number().ok()?;
        (parse.at == number.len()).then_some(value / 100.0)
    }
}

struct Parse<'a> {
    reader: &'a Reader,
    chars: &'a [char],
    at: usize,
    /// The tag of the unit the last quantity named.
    tag: Option<u32>,
}

impl Parse<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn expr(&mut self) -> Result<f64, NotAValue> {
        let first = self.term()?;
        self.sum(first)
    }

    /// An expression whose left operand is `start`, written without it.
    fn expr_from(&mut self, start: f64) -> Result<f64, NotAValue> {
        let term = self.product(start)?;
        self.sum(term)
    }

    fn sum(&mut self, mut left: f64) -> Result<f64, NotAValue> {
        while let Some(op @ ('+' | '-')) = self.peek() {
            self.at += 1;
            let right = self.term()?;
            left = if op == '+' { left + right } else { left - right };
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<f64, NotAValue> {
        let first = self.factor(true)?;
        self.product(first)
    }

    fn product(&mut self, mut left: f64) -> Result<f64, NotAValue> {
        // A bare relative `+5` has no product to run; the sum loop takes it.
        while let Some(op @ ('*' | '/' | 'x' | 'X' | '×')) = self.peek() {
            self.at += 1;
            // A multiplier is a plain number: the reader's bare factor does not scale it.
            let right = self.factor(false)?;
            left = if op == '/' { left / right } else { left * right };
        }
        Ok(left)
    }

    /// A factor; `scaled` says it starts a term, where a number with no unit counts as the reader's bare factor.
    /// Otherwise it is a multiplier, which may be a percentage.
    fn factor(&mut self, scaled: bool) -> Result<f64, NotAValue> {
        match self.peek() {
            Some('-') => {
                self.at += 1;
                self.factor(scaled).map(|value| -value)
            }
            Some('+') => {
                self.at += 1;
                self.factor(scaled)
            }
            Some('(') => {
                self.at += 1;
                let value = self.expr()?;
                (self.peek() == Some(')')).then(|| self.at += 1).ok_or(NotAValue)?;
                Ok(value)
            }
            _ => self.quantity(scaled),
        }
    }

    /// A number and the longest registered unit spelling at the start of the letters after it. Letters that start no
    /// unit stay unread: a product sign (`2x3`) takes them, and anything else leaves the text unread, which refuses it.
    fn quantity(&mut self, scaled: bool) -> Result<f64, NotAValue> {
        let number = self.number()?;
        self.tag = None;
        if !scaled && self.peek() == Some('%') {
            self.at += 1;
            return Ok(number / 100.0);
        }
        let reader = self.reader;
        let run: Vec<char> = self.chars[self.at..].iter().copied().take_while(|c| is_unit_char(*c)).collect();
        let unit = (1..=run.len()).rev().find_map(|len| {
            let lowered = run[..len].iter().collect::<String>().to_lowercase();
            reader.units.iter().find(|unit| unit.spelling == lowered).map(|unit| (len, unit))
        });
        match unit {
            Some((len, unit)) => {
                self.at += len;
                self.tag = unit.tag;
                let rest = match unit.rest {
                    Some(rest) if self.peek().is_some_and(|c| digit(c).is_some() || c == reader.decimal) => rest * self.number()?,
                    _ => 0.0,
                };
                Ok(number * unit.factor + rest)
            }
            None if scaled => Ok(number * reader.bare),
            None => Ok(number),
        }
    }

    /// Whether the group separator at `at` is followed by exactly three digits.
    fn three_digits_after(&self, at: usize) -> bool {
        self.chars[at + 1..].iter().take_while(|c| digit(**c).is_some()).count() == 3
    }

    fn number(&mut self) -> Result<f64, NotAValue> {
        let mut text = String::new();
        let mut seen_decimal = false;
        while let Some(c) = self.peek() {
            if let Some(d) = digit(c) {
                text.push(char::from_digit(d, 10).ok_or(NotAValue)?);
            } else if c == self.reader.decimal && !seen_decimal {
                seen_decimal = true;
                text.push('.');
            } else if Some(c) == self.reader.group && !seen_decimal && !text.is_empty() && self.three_digits_after(self.at) {
            } else {
                break;
            }
            self.at += 1;
        }
        if text.is_empty() || text == "." {
            return Err(NotAValue);
        }
        text.parse().map_err(|_| NotAValue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions_units_and_relative_input_read_through_one_grammar() {
        let reader = Reader::new().decimal(',').group('.').unit("in", 72.0).unit("pt", 1.0);
        assert_eq!(reader.read("2+3*4", None), Ok(14.0));
        assert_eq!(reader.read("(2+3)*4", None), Ok(20.0));
        assert_eq!(reader.read("1.234,5", None), Ok(1234.5), "group separator skipped, comma is the decimal");
        assert_eq!(reader.read("1in + 6pt", None), Ok(78.0));
        assert_eq!(reader.read("-3", Some(10.0)), Ok(-3.0), "a leading minus is a negative number");
        assert_eq!(reader.read("+5", Some(10.0)), Ok(15.0));
        assert_eq!(reader.read("*2", Some(10.0)), Ok(20.0));
        assert_eq!(reader.read("٣٤", None), Ok(34.0), "Arabic-Indic digits");
        let symbols = Reader::new().unit("%", 1.0).unit("°", 1.0);
        assert_eq!(symbols.read("12 %", None), Ok(12.0), "a symbol spelling after a space");
        assert_eq!(symbols.read("12°", None), Ok(12.0));
        assert_eq!(symbols.read("1.5×2", None), Ok(3.0), "× is the product sign");
        assert_eq!(symbols.read("10%+5%", None), Ok(15.0), "a symbol ends where the next operator starts");
        for bad in ["", "abc", "1 +", "2px", "1/0", "(1"] {
            assert_eq!(reader.read(bad, Some(1.0)), Err(NotAValue), "{bad:?} is not a value");
        }
    }

    #[test]
    fn times_signs_and_bare_numbers_read_through_the_grammar() {
        let near = |reader: &Reader, text: &str, want: f64| {
            let got = reader.read(text, None).unwrap_or(f64::NAN);
            assert!((got - want).abs() < 1e-9, "{text:?} read {got}, wanted {want}");
        };
        let lengths = Reader::new().unit("px", 1.0).unit("in", 72.0);
        for sign in ["x", "X", "×", "*"] {
            near(&lengths, &format!("2{sign}3"), 6.0);
        }
        near(&lengths, "2inx3", 432.0);
        near(&lengths, "2px", 2.0);
        near(&lengths, "2px×2", 4.0);

        let picas = Reader::new().bare(1.0 / 12.0).unit("pt", 1.0 / 12.0).unit("pc", 1.0);
        near(&picas, "12", 1.0);
        near(&picas, "1pt+12", 13.0 / 12.0);
        near(&picas, "1pc+12", 2.0);
        near(&picas, "12*2", 2.0);

        let split = Reader::new().bare(1.0 / 12.0).split_unit("p", 1.0, 1.0 / 12.0, 0);
        near(&split, "2p6", 2.5);
        near(&split, "-2p6", -2.5);
        near(&split, "2p", 2.0);

        let percent = Reader::new().unit("cm", 1.0).unit("%", 7.0);
        near(&percent, "3cm*50%", 1.5);
        near(&percent, "3cm/50%", 6.0);
        near(&percent, "2%", 14.0);
        assert_eq!(percent.read("*50%", Some(4.0)), Ok(2.0), "a relative share");

        let german = Reader::new().decimal(',').group('.');
        near(&german, "1.234,5", 1234.5);
        near(&german, "1.234.567", 1_234_567.0);
        for bad in ["1.2", "1.2345", "×3", "x3"] {
            assert_eq!(german.read(bad, None), Err(NotAValue), "{bad:?} is not a value");
        }
    }
}
