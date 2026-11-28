//! Data styles (ODF 1.3 Part 3, section 16.29: `number:number-style`, `number:currency-style`,
//! `number:percentage-style`, `number:date-style`, `number:time-style`, `number:boolean-style`,
//! `number:text-style`) as spreadsheet format codes (`#,##0.00;[Red]-#,##0.00`).
//!
//! Each data style is read into sections of parts (digits, date fields, literal text, …); a style
//! whose `style:map` conditions apply other styles becomes one code with a section per condition,
//! in the order a spreadsheet tries them. One printer spells the sections as a code and alone
//! decides what is quoted. A colour the code cannot name, a currency's locale and a quarter's
//! field (written as `Q`) are not kept.
//!
//! The trees are named with ODF's usual prefixes (`number:`, `style:`, `loext:`).

use std::collections::HashMap;
use std::fmt::Write as _;

use craft_xml::El;

/// The colours a format code names, with the RGB they stand for.
const COLORS: [(&str, &str); 8] = [
    ("Red", "#ff0000"),
    ("Blue", "#0000ff"),
    ("Green", "#00ff00"),
    ("Black", "#000000"),
    ("White", "#ffffff"),
    ("Yellow", "#ffff00"),
    ("Magenta", "#ff00ff"),
    ("Cyan", "#00ffff"),
];

/// The most sections a code has.
const MAX_SECTIONS: usize = 4;
/// The most digits of any one kind read from an attribute.
const MAX_DIGITS: usize = 30;
/// The most parts kept for one data style.
const MAX_PARTS: usize = 256;

/// The condition office suites write for "any number".
const ANY: &str = "<=1.7976931348623157E+308";

/// What a data style formats, by its element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Number,
    Currency,
    Percentage,
    Date,
    Time,
    Boolean,
    Text,
}

/// The order of day, month and year in a short date that follows the locale
/// (`number:automatic-order`), which the caller knows and the file does not say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DateOrder {
    /// As the elements are written.
    #[default]
    AsWritten,
    MonthDayYear,
    DayMonthYear,
    YearMonthDay,
}

// ----------------------------------------------------------------------------------- the model

/// One section of a format code.
#[derive(Clone, Debug, Default, PartialEq)]
struct Section {
    /// A colour name from [`COLORS`].
    color: Option<&'static str>,
    /// An explicit condition (`>=100`): operator and number, as written.
    cond: Option<String>,
    parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq)]
enum Part {
    /// The standard number format.
    General,
    Number(Number),
    /// `0.00E+00`.
    Scientific {
        number: Number,
        exp_digits: usize,
        plus: bool,
    },
    /// `# ?/8`: the whole part (if any), numerator digits and denominator.
    Fraction {
        int: Option<IntMask>,
        numerator: usize,
        denominator: Denominator,
    },
    Date {
        field: Field,
        long: bool,
        elapsed: bool,
    },
    AmPm,
    /// Text shown as is.
    Lit(String),
    /// The percent sign, which scales the value by 100.
    Percent,
    /// `$`, or `[$€]`.
    Currency {
        symbol: String,
    },
    /// `@`: the cell's text.
    Text,
    /// `*c`: the character repeated to fill the cell.
    Fill(char),
    /// `_c`: a space as wide as the character.
    Blank(char),
}

/// The integer digits of a number: how many always show (`0` or `?`), how many of those show as
/// spaces when not needed (`?`), whether thousands are grouped, and how many digits the code
/// draws (`#` included).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct IntMask {
    min: usize,
    blank: usize,
    grouping: bool,
    width: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Number {
    int: IntMask,
    decimals: usize,
    min_decimals: usize,
    /// Trailing commas: each scales the value by a thousandth.
    thousands: usize,
    /// Text among the integer digits (`000-00-0000`), by the number of integer digits to its
    /// right.
    embedded: Vec<(usize, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Denominator {
    Fixed(u32),
    /// As many digits as needed, up to this many.
    Digits(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Field {
    Year,
    Month { textual: bool },
    Day,
    Weekday,
    Era,
    Hours,
    Minutes,
    Seconds { decimals: usize },
}

// ------------------------------------------------------------------------- sections → a code

fn plain_literal(c: char, date: bool) -> bool {
    matches!(c, ' ' | '-' | '+' | '(' | ')' | ':') || (date && matches!(c, ',' | '/' | '.'))
}

/// Literal text in a code: bare when it can't be mistaken for format characters, else quoted (a
/// quote itself is escaped, as quotes can't hold one).
fn print_literal(out: &mut String, text: &str, date: bool) {
    // An empty text (`""`) shows nothing, which only quotes can say.
    if text.is_empty() {
        out.push_str("\"\"");
        return;
    }
    if text.chars().all(|c| plain_literal(c, date)) {
        out.push_str(text);
        return;
    }
    for (k, piece) in text.split('"').enumerate() {
        if k > 0 {
            out.push_str("\\\"");
        }
        if !piece.is_empty() {
            let _ = write!(out, "\"{piece}\"");
        }
    }
}

fn print_int(out: &mut String, int: &IntMask, embedded: &[(usize, String)]) {
    // A digit stands left of every embedded text, or the text would lead the number.
    let width = int.width.max(int.min).max(if int.grouping { 4 } else { 1 }).max(embedded.iter().map(|(p, _)| *p + 1).max().unwrap_or(0));
    for pos in (0..width).rev() {
        for (_, text) in embedded.iter().filter(|(p, _)| *p == pos + 1) {
            print_literal(out, text, false);
        }
        // Blank digits (`?`) are the leading ones of the minimum integer digits.
        out.push(if pos < int.min && pos + int.blank >= int.min {
            '?'
        } else if pos < int.min {
            '0'
        } else {
            '#'
        });
        if int.grouping && pos == 3 {
            out.push(',');
        }
    }
    for (_, text) in embedded.iter().filter(|(p, _)| *p == 0) {
        print_literal(out, text, false);
    }
}

fn print_number(out: &mut String, n: &Number) {
    print_int(out, &n.int, &n.embedded);
    if n.decimals > 0 {
        out.push('.');
        for i in 0..n.decimals {
            out.push(if i < n.min_decimals { '0' } else { '#' });
        }
    }
    out.push_str(&",".repeat(n.thousands.min(6)));
}

fn date_token(field: Field, long: bool) -> &'static str {
    match (field, long) {
        (Field::Year, true) => "yyyy",
        (Field::Year, false) => "yy",
        (Field::Month { textual: true }, true) => "mmmm",
        (Field::Month { textual: true }, false) => "mmm",
        (Field::Month { textual: false } | Field::Minutes, true) => "mm",
        (Field::Month { textual: false } | Field::Minutes, false) => "m",
        (Field::Day, true) => "dd",
        (Field::Day, false) => "d",
        (Field::Weekday, true) => "dddd",
        (Field::Weekday, false) => "ddd",
        (Field::Era, true) => "ggg",
        (Field::Era, false) => "g",
        (Field::Hours, true) => "hh",
        (Field::Hours, false) => "h",
        (Field::Seconds { .. }, true) => "ss",
        (Field::Seconds { .. }, false) => "s",
    }
}

fn print_part(out: &mut String, p: &Part, date: bool) {
    match p {
        Part::General => out.push_str("General"),
        Part::Number(n) => print_number(out, n),
        Part::Scientific { number, exp_digits, plus } => {
            print_number(out, number);
            out.push_str(if *plus { "E+" } else { "E-" });
            out.push_str(&"0".repeat(*exp_digits));
        }
        Part::Fraction { int, numerator, denominator } => {
            if let Some(int) = int {
                print_int(out, &IntMask { width: int.min, ..*int }, &[]);
                out.push(' ');
            }
            out.push_str(&"?".repeat(*numerator));
            out.push('/');
            match denominator {
                Denominator::Fixed(d) => {
                    let _ = write!(out, "{d}");
                }
                Denominator::Digits(n) => out.push_str(&"?".repeat(*n)),
            }
        }
        Part::Date { field, long, elapsed } => {
            let token = date_token(*field, *long);
            if *elapsed {
                let _ = write!(out, "[{token}]");
            } else {
                out.push_str(token);
            }
            if let Field::Seconds { decimals } = field
                && *decimals > 0
            {
                out.push('.');
                out.push_str(&"0".repeat(*decimals));
            }
        }
        Part::AmPm => out.push_str("AM/PM"),
        Part::Lit(t) => print_literal(out, t, date),
        Part::Percent => out.push('%'),
        Part::Currency { symbol } if symbol == "$" => out.push('$'),
        Part::Currency { symbol } => {
            let _ = write!(out, "[${symbol}]");
        }
        Part::Text => out.push('@'),
        Part::Fill(c) => {
            out.push('*');
            out.push(*c);
        }
        Part::Blank(c) => {
            out.push('_');
            out.push(*c);
        }
    }
}

fn print_code(sections: &[Section]) -> String {
    let mut out = String::new();
    for (k, s) in sections.iter().enumerate() {
        if k > 0 {
            out.push(';');
        }
        if let Some(c) = s.color {
            let _ = write!(out, "[{c}]");
        }
        if let Some(c) = &s.cond {
            let _ = write!(out, "[{c}]");
        }
        let date = s.parts.iter().any(|p| matches!(p, Part::Date { .. } | Part::AmPm));
        for p in &s.parts {
            print_part(&mut out, p, date);
        }
    }
    out
}

/// The condition true exactly where `c` is false (`>5` → `<=5`).
fn complement(c: &str) -> Option<String> {
    let op_len = c.chars().take_while(|c| matches!(c, '<' | '>' | '=')).count();
    let (op, num) = c.split_at(op_len);
    let neg = match op {
        ">" => "<=",
        ">=" => "<",
        "<" => ">=",
        "<=" => ">",
        "=" => "<>",
        "<>" => "=",
        _ => return None,
    };
    Some(format!("{neg}{num}"))
}

/// `value()>=0` → `>=0`; "any number" however its bound is spelled → [`ANY`].
fn condition(c: &str) -> Option<String> {
    let rest = c.trim().strip_prefix("value()")?.trim();
    let op_len = rest.chars().take_while(|c| matches!(c, '<' | '>' | '=' | '!')).count();
    let op = rest.get(..op_len)?;
    let num = rest.get(op_len..)?.trim();
    let n = num.parse::<f64>().ok().filter(|v| !v.is_nan())?;
    // Older writers round the bound past the largest number.
    if op == "<=" && n >= f64::MAX {
        return Some(ANY.into());
    }
    let op = match op {
        "==" => "=",
        "!=" => "<>",
        o @ ("<" | ">" | "<=" | ">=" | "=" | "<>") => o,
        _ => return None,
    };
    Some(format!("{op}{num}"))
}

// ------------------------------------------------------------------------- elements → sections

/// One data style as read: its own sections (one, or a boolean style's three), and the
/// conditions applying other styles.
#[derive(Clone, Debug)]
struct DataStyle {
    kind: Kind,
    sections: Vec<Section>,
    maps: Vec<(String, String)>,
    /// A short date in the locale's order: its day, month and year parts, `long` each.
    short_date: Option<[bool; 3]>,
}

/// The data styles of a document, by name. See [`DataStyles::add`].
#[derive(Clone, Debug, Default)]
pub struct DataStyles(HashMap<String, DataStyle>);

fn local(e: &El) -> Option<&str> {
    e.name.strip_prefix("number:")
}

fn attr<'e>(e: &'e El, name: &str) -> Option<&'e str> {
    e.attr(&format!("number:{name}"))
}

fn flag(e: &El, name: &str) -> bool {
    attr(e, name) == Some("true")
}

fn int_attr(e: &El, name: &str) -> Option<usize> {
    attr(e, name).or_else(|| e.attr(&format!("loext:{name}"))).and_then(|v| v.trim().parse::<usize>().ok()).map(|n| n.min(MAX_DIGITS))
}

/// The integer digits an element states (`min_default` when it states no minimum).
fn read_int(e: &El, min_default: usize) -> IntMask {
    let min = int_attr(e, "min-integer-digits").unwrap_or(min_default);
    IntMask { min, blank: int_attr(e, "max-blank-integer-digits").unwrap_or(0).min(min), grouping: flag(e, "grouping"), width: min }
}

/// A `number:number` or `number:scientific-number`'s digits.
fn read_number(e: &El, scientific: bool) -> Number {
    let decimals = int_attr(e, "decimal-places").unwrap_or(if scientific { 2 } else { 0 });
    // Each trailing comma scales by a thousand; a factor past 1000^6 (or not a finite number) is
    // no format anyone writes.
    let mut thousands = 0;
    if let Some(mut f) = attr(e, "display-factor").and_then(|f| f.parse::<f64>().ok()).filter(|f| f.is_finite()) {
        while f >= 1000.0 && thousands < 6 {
            thousands += 1;
            f /= 1000.0;
        }
    }
    let mut embedded: Vec<(usize, String)> = e
        .children("number:embedded-text")
        .take(MAX_DIGITS)
        .filter_map(|t| Some((attr(t, "position")?.trim().parse::<usize>().ok()?.min(MAX_DIGITS), t.text().into_owned())))
        .collect();
    embedded.sort_by_key(|e| std::cmp::Reverse(e.0));
    Number {
        int: read_int(e, if scientific { 1 } else { 0 }),
        decimals,
        min_decimals: int_attr(e, "min-decimal-places").unwrap_or(decimals).min(decimals),
        thousands,
        embedded,
    }
}

/// `number:number` alone without decimal places is the standard format.
fn is_standard(n: &El) -> bool {
    attr(n, "decimal-places").is_none()
        && !flag(n, "grouping")
        && attr(n, "display-factor").is_none()
        && int_attr(n, "min-integer-digits").unwrap_or(1) == 1
        && n.children("number:embedded-text").next().is_none()
}

/// A short date in the locale's order (`number:automatic-order`) of day, month and year only:
/// whether each is long.
fn short_date(style: &El) -> Option<[bool; 3]> {
    if !flag(style, "automatic-order") {
        return None;
    }
    let fields: Vec<&El> = style.elements().filter(|e| local(e).is_some_and(|n| n != "text")).collect();
    let long = |e: &El| attr(e, "style") == Some("long");
    let find = |n: &str| fields.iter().find(|e| local(e) == Some(n)).copied();
    let (Some(d), Some(m), Some(y)) = (find("day"), find("month"), find("year")) else { return None };
    (fields.len() == 3 && !flag(m, "textual")).then(|| [long(d), long(m), long(y)])
}

fn kind_of(name: &str) -> Option<Kind> {
    Some(match name {
        "number:number-style" => Kind::Number,
        "number:currency-style" => Kind::Currency,
        "number:percentage-style" => Kind::Percentage,
        "number:date-style" => Kind::Date,
        "number:time-style" => Kind::Time,
        "number:boolean-style" => Kind::Boolean,
        "number:text-style" => Kind::Text,
        _ => return None,
    })
}

fn lit(s: &str) -> Part {
    Part::Lit(s.to_string())
}

/// A data style element's own section.
fn read_section(style: &El, kind: Kind) -> Section {
    let color = style
        .child("style:text-properties")
        .and_then(|t| t.attr("fo:color"))
        .and_then(|rgb| COLORS.iter().find(|(_, c)| c.eq_ignore_ascii_case(rgb.trim())).map(|(n, _)| *n));
    // In an elapsed-time style the first time part is the one that isn't cut at its unit.
    let mut elapsed = attr(style, "truncate-on-overflow") == Some("false");
    let mut parts = Vec::new();
    let mut percent_seen = false;
    for e in style.elements().take(MAX_PARTS) {
        let Some(name) = local(e) else { continue };
        let long = attr(e, "style") == Some("long");
        let mut date = |field: Field| {
            let first = elapsed && matches!(field, Field::Hours | Field::Minutes | Field::Seconds { .. });
            if first {
                elapsed = false;
            }
            Part::Date { field, long, elapsed: first }
        };
        let part = match name {
            "number" if is_standard(e) => Part::General,
            "number" => Part::Number(read_number(e, false)),
            "scientific-number" => {
                let mut number = read_number(e, true);
                number.int.width = number.int.width.max(int_attr(e, "exponent-interval").unwrap_or(1));
                Part::Scientific {
                    number,
                    exp_digits: int_attr(e, "min-exponent-digits").unwrap_or(2).max(1),
                    plus: attr(e, "forced-exponent-sign") != Some("false"),
                }
            }
            "fraction" => Part::Fraction {
                int: attr(e, "min-integer-digits").map(|_| read_int(e, 0)),
                numerator: int_attr(e, "min-numerator-digits").unwrap_or(1).max(1),
                denominator: match attr(e, "denominator-value").and_then(|d| d.trim().parse::<u32>().ok()).filter(|d| *d > 0) {
                    Some(d) => Denominator::Fixed(d),
                    None => Denominator::Digits(
                        int_attr(e, "min-denominator-digits").or_else(|| attr(e, "max-denominator-value").map(str::len)).unwrap_or(1).clamp(1, 9),
                    ),
                },
            },
            "currency-symbol" => {
                let symbol = e.text();
                // An empty symbol is only a locale.
                if symbol.is_empty() {
                    continue;
                }
                Part::Currency { symbol: symbol.replace(']', "") }
            }
            "text" => {
                let t = e.text();
                match e.attr("loext:blank-width-char").and_then(|c| c.chars().next()) {
                    Some(c) if t == " " => Part::Blank(c),
                    // In a percentage style the first `%` is the percent sign; any other is text.
                    _ if kind == Kind::Percentage && !percent_seen && t.contains('%') => {
                        percent_seen = true;
                        let (before, after) = t.split_once('%').unwrap_or((&t, ""));
                        if !before.is_empty() {
                            parts.push(lit(before));
                        }
                        parts.push(Part::Percent);
                        if after.is_empty() {
                            continue;
                        }
                        lit(after)
                    }
                    _ => Part::Lit(t.into_owned()),
                }
            }
            "fill-character" => Part::Fill(e.text().chars().next().unwrap_or(' ')),
            "text-content" => Part::Text,
            "day" => date(Field::Day),
            "month" => date(Field::Month { textual: flag(e, "textual") }),
            "year" => date(Field::Year),
            "day-of-week" => date(Field::Weekday),
            "era" => date(Field::Era),
            "hours" => date(Field::Hours),
            "minutes" => date(Field::Minutes),
            "seconds" => date(Field::Seconds { decimals: int_attr(e, "decimal-places").unwrap_or(0) }),
            "am-pm" => Part::AmPm,
            // A quarter has no format code token.
            "quarter" => lit("Q"),
            _ => continue,
        };
        parts.push(part);
    }
    Section { color, cond: None, parts }
}

impl DataStyles {
    /// Adds the data styles among the children of `list` (an `office:styles` or
    /// `office:automatic-styles`). A later style of the same name replaces an earlier one.
    pub fn add(&mut self, list: &El) {
        for e in list.elements() {
            let Some(kind) = kind_of(&e.name) else { continue };
            let Some(name) = e.attr("style:name") else { continue };
            // Numbers in a boolean style show TRUE for anything but zero.
            let sections = if kind == Kind::Boolean {
                let word = |w: &str| Section { parts: vec![lit(w)], ..Section::default() };
                vec![word("TRUE"), word("TRUE"), word("FALSE")]
            } else {
                vec![read_section(e, kind)]
            };
            let maps = e
                .children("style:map")
                .take(MAX_SECTIONS)
                .filter_map(|m| Some((m.attr("style:condition")?.to_string(), m.attr("style:apply-style-name")?.to_string())))
                .collect();
            let short_date = if kind == Kind::Date { short_date(e) } else { None };
            self.0.insert(name.to_string(), DataStyle { kind, sections, maps, short_date });
        }
    }

    /// What the data style `name` formats.
    pub fn kind(&self, name: &str) -> Option<Kind> {
        self.0.get(name).map(|s| s.kind)
    }

    /// The data style `name` as a format code, with the sections its conditions apply. A short
    /// date in the locale's order keeps the order its elements are written in.
    pub fn format_code(&self, name: &str) -> Option<String> {
        self.format_code_with(name, DateOrder::AsWritten)
    }

    /// [`DataStyles::format_code`], with a short date in the locale's order spelled in `order`
    /// with `/` between its fields.
    pub fn format_code_with(&self, name: &str, order: DateOrder) -> Option<String> {
        let sections = |s: &DataStyle| -> Vec<Section> {
            match (s.short_date, order) {
                (Some(long), order) if order != DateOrder::AsWritten => {
                    let [d, m, y] = long;
                    let day = Part::Date { field: Field::Day, long: d, elapsed: false };
                    let month = Part::Date { field: Field::Month { textual: false }, long: m, elapsed: false };
                    let year = Part::Date { field: Field::Year, long: y, elapsed: false };
                    let fields = match order {
                        DateOrder::MonthDayYear => [month, day, year],
                        DateOrder::DayMonthYear => [day, month, year],
                        _ => [year, month, day],
                    };
                    let [a, b, c] = fields;
                    vec![Section { parts: vec![a, lit("/"), b, lit("/"), c], ..Section::default() }]
                }
                _ => s.sections.clone(),
            }
        };
        let main = self.0.get(name)?;
        let own_sections = sections(main);
        let Some(own) = own_sections.first() else { return Some("General".into()) };
        let general = Section { parts: vec![Part::General], ..Section::default() };
        if main.maps.is_empty() {
            // A style that shows nothing on its own is the standard one.
            if own_sections.len() == 1 && own.parts.is_empty() {
                return Some(print_code(&[Section { parts: vec![Part::General], ..own.clone() }]));
            }
            return Some(print_code(&own_sections));
        }
        let mapped: Vec<(Option<String>, Section)> = main
            .maps
            .iter()
            .take(MAX_SECTIONS - 1)
            .map(|(cond, apply)| {
                let s = self.0.get(apply).and_then(|s| sections(s).into_iter().next()).unwrap_or_else(|| general.clone());
                (condition(cond), s)
            })
            .collect();
        let conds: Vec<&str> = mapped.iter().map(|(c, _)| c.as_deref().unwrap_or("?")).collect();
        // A code's implicit sections (positive;negative[;zero[;text]]) need no conditions; with a
        // text section last, positive-and-zero;negative;text.
        let text_main = own.parts.contains(&Part::Text);
        let implicit = match conds.as_slice() {
            [">=0"] | [">0", "<0"] => !text_main,
            [">0", "<0", "=0"] => true,
            [c] if *c == ANY => text_main,
            [">=0", "<0"] => text_main,
            _ => false,
        };
        let mut out: Vec<Section> = mapped.into_iter().map(|(cond, s)| Section { cond: if implicit { None } else { cond }, ..s }).collect();
        // Before a text section, a last arm of plain General covering what the others leave is no
        // section of its own (the other values show so anyway).
        if text_main
            && !implicit
            && let Some(last) = out.last()
        {
            let first = out.first().and_then(|s| s.cond.as_deref()).and_then(complement);
            let covers = last.cond.as_deref() == Some(ANY) || (out.len() == 2 && last.cond.is_some() && last.cond == first);
            if covers && (Section { cond: None, ..last.clone() }) == general {
                out.pop();
            } else if last.cond.as_deref() == Some(ANY)
                && let Some(last) = out.last_mut()
            {
                last.cond = None;
            }
        }
        if text_main || !out.iter().all(|s| s.cond.is_some()) || *own != general {
            out.push(own.clone());
        }
        Some(print_code(&out))
    }
}
