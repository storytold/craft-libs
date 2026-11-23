//! Number formats: Excel format codes (`#,##0`, `0.0%`, `_-* # ##0 "₽"_-;…`, `General`) as chart
//! axes and data labels show them, in the chart's language (digit grouping and decimal mark).

use chrono::{Datelike, Days, NaiveDate};

/// Digit grouping and decimal mark for a language tag (`ru-RU`, `de-DE`, `en-US`).
fn marks(lang: Option<&str>) -> (char, char) {
    let l = lang.unwrap_or("en").split(['-', '_']).next().unwrap_or("en").to_ascii_lowercase();
    match l.as_str() {
        // Space-grouped, comma decimal.
        "ru" | "uk" | "be" | "kk" | "fr" | "pl" | "cs" | "sk" | "sv" | "fi" | "nb" | "nn" | "no" | "bg" | "lt" | "lv" | "et" | "hu" | "uz" | "hy"
        | "ka" | "az" => ('\u{a0}', ','),
        // Dot-grouped, comma decimal.
        "de" | "es" | "it" | "pt" | "nl" | "da" | "tr" | "el" | "ro" | "id" | "sr" | "hr" | "sl" | "vi" => ('.', ','),
        _ => (',', '.'),
    }
}

#[derive(Debug, PartialEq)]
enum Tok {
    /// Text shown as is.
    Lit(String),
    /// A digit placeholder: `0`, `#` or `?`.
    Digit(char),
    Dot,
    Comma,
    Percent,
    /// An unquoted, unescaped fraction separator.
    Slash,
}

/// A section's tokens (`[$₽-419]` gives its symbol; colours, conditions and locales drop out).
fn tokens(section: &str) -> Vec<Tok> {
    let mut out = vec![];
    let mut it = section.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                let mut s = String::new();
                for d in it.by_ref() {
                    if d == '"' {
                        break;
                    }
                    s.push(d);
                }
                out.push(Tok::Lit(s));
            }
            '\\' => {
                if let Some(d) = it.next() {
                    out.push(Tok::Lit(d.to_string()));
                }
            }
            // Padding the width of the next character: a space.
            '_' => {
                it.next();
                out.push(Tok::Lit(" ".into()));
            }
            // Repeat-to-fill: nothing on an axis.
            '*' => {
                it.next();
            }
            '[' => {
                let mut s = String::new();
                for d in it.by_ref() {
                    if d == ']' {
                        break;
                    }
                    s.push(d);
                }
                if let Some(sym) = s.strip_prefix('$') {
                    let sym = sym.split('-').next().unwrap_or("");
                    if !sym.is_empty() {
                        out.push(Tok::Lit(sym.to_string()));
                    }
                }
            }
            '0' | '#' | '?' => out.push(Tok::Digit(c)),
            '.' => out.push(Tok::Dot),
            ',' => out.push(Tok::Comma),
            '%' => out.push(Tok::Percent),
            '/' => out.push(Tok::Slash),
            '@' => {}
            _ => out.push(Tok::Lit(c.to_string())),
        }
    }
    out
}

/// The format's sections split at `;` (outside quotes, brackets and escapes).
fn sections(code: &str) -> Vec<&str> {
    let mut out = vec![];
    let (mut start, mut quoted, mut bracket, mut escaped) = (0, false, false, false);
    for (i, c) in code.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if !quoted => escaped = true,
            '"' => quoted = !quoted,
            '[' if !quoted => bracket = true,
            ']' if !quoted => bracket = false,
            ';' if !quoted && !bracket => {
                out.push(code.get(start..i).unwrap_or(""));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(code.get(start..).unwrap_or(""));
    out
}

/// Padding of a `?` placeholder; a space in the final text, but kept when the label is trimmed.
const PAD: char = '\u{1}';

/// `n` significant digits of `x` (rounded half to even on its exact binary value) and the decimal
/// exponent of the first.
fn significant(x: f64, n: usize) -> (Vec<u8>, i32) {
    let s = format!("{:.*e}", n.saturating_sub(1), x);
    let (m, e) = s.split_once('e').unwrap_or((s.as_str(), "0"));
    (m.bytes().filter(u8::is_ascii_digit).map(|b| b - b'0').collect(), e.parse().unwrap_or(0))
}

/// The decimal digits of `x` (finite, above zero) at Excel's 15 significant digits, and the decimal
/// exponent of the first one. Excel rounds what it shows from this decimal value, not from the
/// binary one: 0.145 is 0.145 to it, so `0.00` gives 0.15. A value that is exactly a tie at the
/// 16th digit (1234567890123455) goes toward zero, as in Excel.
fn digits15(x: f64) -> (Vec<u8>, i32) {
    let (d16, exp) = significant(x, 16);
    let tie = |ds: &[u8]| ds.get(15) == Some(&5) && ds.get(16..).is_some_and(|t| t.iter().all(|d| *d == 0));
    match d16.get(15).copied().unwrap_or(0) {
        0..=4 => (d16.get(..15).unwrap_or(&[]).to_vec(), exp),
        // A cheap screen at 30 digits, then the exact expansion.
        5 if tie(&significant(x, 30).0) && tie(&significant(x, 800).0) => (d16.get(..15).unwrap_or(&[]).to_vec(), exp),
        // Not a tie: round the exact value (the 16 digits may be a double rounding).
        5 => significant(x, 15),
        _ => {
            let (r, carry) = round_at(d16, 15);
            if carry { (r.get(..15).unwrap_or(&[]).to_vec(), exp + 1) } else { (r, exp) }
        }
    }
}

/// `ds` cut to `keep` digits, rounded half away from zero (a digit of 5 or more rounds up); `true`
/// when the carry added a leading digit.
fn round_at(mut ds: Vec<u8>, keep: usize) -> (Vec<u8>, bool) {
    if ds.len() <= keep {
        ds.resize(keep, 0);
        return (ds, false);
    }
    let up = ds.get(keep).is_some_and(|d| *d >= 5);
    ds.truncate(keep);
    if up {
        for d in ds.iter_mut().rev() {
            if *d == 9 {
                *d = 0;
            } else {
                *d += 1;
                return (ds, false);
            }
        }
        ds.insert(0, 1);
        return (ds, true);
    }
    (ds, false)
}

fn digit_text(ds: &[u8]) -> String {
    ds.iter().map(|d| char::from(b'0' + d)).collect()
}

/// `x` (finite, not negative) rounded half away from zero to `frac` decimals: the integer digits
/// and the `frac` fraction digits.
fn round_dec(x: f64, frac: usize) -> (String, String) {
    if x == 0.0 {
        return ("0".into(), "0".repeat(frac));
    }
    let (mut ds, exp) = digits15(x);
    let pos = exp + 1;
    let mut int_len = if pos > 0 { pos as usize } else { 0 };
    if pos < 0 {
        // Far below the last kept digit: zero, without building the leading zeros.
        if pos.unsigned_abs() as usize > frac + 1 {
            return ("0".into(), "0".repeat(frac));
        }
        let mut z = vec![0; pos.unsigned_abs() as usize];
        z.append(&mut ds);
        ds = z;
    }
    let (ds, carry) = round_at(ds, int_len + frac);
    if carry {
        int_len += 1;
    }
    let (i, f) = ds.split_at(int_len.min(ds.len()));
    (if i.is_empty() { "0".into() } else { digit_text(i) }, digit_text(f))
}

/// `v` in the General format: up to 10 significant digits, no grouping.
fn general(v: f64, dec: char) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    if !(1e-9..1e11).contains(&a) {
        let (ds, e) = digits15(a);
        let (ds, carry) = round_at(ds, 6);
        let e = if carry { e + 1 } else { e };
        let (lead, rest) = ds.split_at(1.min(ds.len()));
        let rest = digit_text(rest);
        let rest = rest.trim_end_matches('0');
        let m = if rest.is_empty() { digit_text(lead) } else { format!("{}{dec}{rest}", digit_text(lead)) };
        return format!("{m}E{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    let digits = (10 - (a.log10().floor() as i32 + 1)).clamp(0, 10) as usize;
    let (ip, fp) = round_dec(a, digits);
    let fp = fp.trim_end_matches('0');
    if fp.is_empty() { ip } else { format!("{ip}{dec}{fp}") }
}

/// Group an integer's digits in threes with `sep`.
fn group(digits: &str, sep: char) -> String {
    let n = digits.chars().count();
    let mut out = String::new();
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (n - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(d);
    }
    out
}

/// Longest number format applied, as in Excel (255 characters); a longer one formats as
/// General, so a chart cannot make every label expand a huge format.
pub const MAX_CODE_LEN: usize = 255;

/// The comparison of a conditional section (`[>=1000]`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

/// The condition in a section's brackets (`[>=1000]`, `[<0.5]`), if it has one.
fn condition(sec: &str) -> Option<(Op, f64)> {
    let (mut quoted, mut escaped) = (false, false);
    let mut it = sec.char_indices();
    while let Some((i, c)) = it.next() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' | '_' | '*' if !quoted => escaped = true,
            '"' => quoted = !quoted,
            '[' if !quoted => {
                let rest = sec.get(i + 1..)?;
                let raw = rest.split(']').next().unwrap_or("");
                let inner = raw.trim();
                let (op, num) = if let Some(n) = inner.strip_prefix("<=") {
                    (Op::Le, n)
                } else if let Some(n) = inner.strip_prefix(">=") {
                    (Op::Ge, n)
                } else if let Some(n) = inner.strip_prefix("<>") {
                    (Op::Ne, n)
                } else if let Some(n) = inner.strip_prefix('<') {
                    (Op::Lt, n)
                } else if let Some(n) = inner.strip_prefix('>') {
                    (Op::Gt, n)
                } else if let Some(n) = inner.strip_prefix('=') {
                    (Op::Eq, n)
                } else {
                    for _ in 0..raw.chars().count() + 1 {
                        it.next();
                    }
                    continue;
                };
                return num.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| (op, n));
            }
            _ => {}
        }
    }
    None
}

fn holds((op, n): (Op, f64), v: f64) -> bool {
    match op {
        Op::Lt => v < n,
        Op::Le => v <= n,
        Op::Gt => v > n,
        Op::Ge => v >= n,
        Op::Eq => v == n,
        Op::Ne => v != n,
    }
}

/// Which section a value takes when the first two sections carry conditions.
enum Pick {
    /// The section's index and whether a negative value shows its minus sign.
    Section(usize, bool),
    /// No condition holds and no section takes the rest: Excel shows hashes.
    NoMatch,
}

/// The section a value takes when the first two sections carry conditions (see [`Pick`]).
/// `None` without conditions (the sign picks the section).
/// An unconditioned first section takes `>0` (`>=0` in a two-section format), an unconditioned
/// second one the rest (`<0` in a three-section format), and a third section takes what is left.
/// As in Excel, only a format of one section adds the minus sign itself. A value that meets no
/// condition, with no third section to take it, matches nothing.
fn conditional(secs: &[&str], v: f64) -> Option<Pick> {
    let conds: Vec<Option<(Op, f64)>> = secs.iter().take(2).map(|s| condition(s)).collect();
    let (c1, c2) = (conds.first().copied().flatten(), conds.get(1).copied().flatten());
    if c1.is_none() && c2.is_none() {
        return None;
    }
    let n = secs.len().min(3);
    let minus = v < 0.0 && secs.len() < 2;
    if holds(c1.unwrap_or(if n >= 3 { (Op::Gt, 0.0) } else { (Op::Ge, 0.0) }), v) {
        return Some(Pick::Section(0, minus));
    }
    let d2 = c2.or(if n >= 3 { Some((Op::Lt, 0.0)) } else { None });
    if n >= 2 && d2.is_none_or(|d| holds(d, v)) {
        return Some(Pick::Section(1, false));
    }
    Some(if n >= 3 { Pick::Section(2, false) } else { Pick::NoMatch })
}

/// `sec` without its `[...]` parts (conditions, colours, locales) outside quotes.
fn without_brackets(sec: &str) -> String {
    let (mut out, mut quoted, mut bracket) = (String::new(), false, false);
    for c in sec.chars() {
        match c {
            '"' if !bracket => {
                quoted = !quoted;
                out.push(c);
            }
            '[' if !quoted => bracket = true,
            ']' if bracket => bracket = false,
            _ if !bracket => out.push(c),
            _ => {}
        }
    }
    out
}

/// `v` formatted with the Excel number format `code` in language `lang` (axis labels: the text
/// section of a format doesn't apply). Dates count from 1900 as Excel does.
pub fn format(code: &str, v: f64, lang: Option<&str>) -> String {
    format_dated(code, v, lang, false)
}

/// [`format`] with the workbook's date system: `date_1904` counts serial dates from 1 January
/// 1904 (`c:date1904`) instead of 1900.
pub fn format_dated(code: &str, v: f64, lang: Option<&str>, date_1904: bool) -> String {
    let code = if code.len() > MAX_CODE_LEN * 4 || code.chars().count() > MAX_CODE_LEN { "General" } else { code };
    let (sep, dec) = marks(lang);
    if !v.is_finite() {
        return String::new();
    }
    let secs = sections(code);
    // A date before its first day shows hashes whichever section takes it, so keep the value's own sign.
    let negative = v < 0.0;
    // The section for the value: by the conditions, else by the sign. A negative or zero section
    // shows the value unsigned.
    let (sec, v, minus) = match (conditional(&secs, v), secs.as_slice(), v) {
        (Some(Pick::NoMatch), ..) => return "#".repeat(8),
        (Some(Pick::Section(i, minus)), _, v) => (secs.get(i).copied().unwrap_or(""), v.abs(), minus),
        (None, [_, neg, ..], v) if v < 0.0 => (*neg, -v, false),
        (None, [_, _, zero, ..], v) if v == 0.0 => (*zero, v, false),
        (None, [pos, ..], v) => (*pos, v.abs(), v < 0.0),
        (None, [], v) => ("General", v.abs(), v < 0.0),
    };
    if without_brackets(sec).trim().eq_ignore_ascii_case("general") || (secs.len() == 1 && sec.trim().is_empty()) {
        let s = general(v, dec);
        return if minus { format!("-{s}") } else { s };
    }
    if let Some(dt) = date_tokens(sec) {
        // Excel shows a date before its first day as hashes.
        return if negative { "#".repeat(8) } else { date_text(&dt, v, date_1904, dec) };
    }
    let toks = tokens(sec);
    let first = toks.iter().position(|t| matches!(t, Tok::Digit(_)));
    let last = toks.iter().rposition(|t| matches!(t, Tok::Digit(_)));
    let (Some(first), Some(last)) = (first, last) else {
        // No digits: just the section's text ("-" for zero, a label).
        let s: String = toks
            .iter()
            .filter_map(|t| match t {
                Tok::Lit(s) => Some(s.as_str()),
                Tok::Slash => Some("/"),
                _ => None,
            })
            .collect();
        return s.trim().to_string();
    };
    // A percent sign scales the value before the fraction is picked, as in `fixed`.
    let fv = if toks.contains(&Tok::Percent) { v * 100.0 } else { v };
    let (out, zero) = match fraction(&toks, fv, sep) {
        Some(o) => o,
        None => match sci(&toks, v, sep, dec) {
            Some(o) => (o, false),
            None => fixed(&toks, first, last, v, sep, dec),
        },
    };
    let out = out.trim().replace(PAD, " ");
    // A negative number that rounds to zero shows no minus sign.
    if minus && !zero && !out.is_empty() { format!("-{out}") } else { out }
}

/// The text of the literal and percent tokens in `ts`.
fn lits(ts: &[Tok]) -> String {
    ts.iter()
        .filter_map(|t| match t {
            Tok::Lit(s) => Some(s.clone()),
            Tok::Percent => Some("%".into()),
            Tok::Slash => Some("/".into()),
            _ => None,
        })
        .collect()
}

/// Scientific notation (`0.00E+00`, `##0.0E-0`): the mantissa takes the placeholders before the `E`,
/// the exponent's sign and minimum width come from what follows it; with several integer
/// placeholders the exponent is a multiple of their count (engineering notation). `None` if the
/// section has no exponent part.
fn sci(toks: &[Tok], v: f64, sep: char, dec: char) -> Option<String> {
    let e = toks.iter().position(|t| matches!(t, Tok::Lit(s) if s.as_str() == "E" || s.as_str() == "e"))?;
    let e_char = if let Some(Tok::Lit(s)) = toks.get(e) { s.as_str() } else { "E" };
    let plus = match toks.get(e + 1)? {
        Tok::Lit(s) if s.as_str() == "+" => true,
        Tok::Lit(s) if s.as_str() == "-" => false,
        _ => return None,
    };
    let mant = toks.get(..e)?;
    let mfirst = mant.iter().position(|t| matches!(t, Tok::Digit(_)))?;
    let mlast = mant.iter().rposition(|t| matches!(t, Tok::Digit(_)))?;
    let exp_toks = toks.get(e + 2..)?;
    let exp_len = exp_toks.iter().take_while(|t| matches!(t, Tok::Digit(_))).count();
    if exp_len == 0 {
        return None;
    }
    let width = exp_toks.iter().take(exp_len).filter(|t| **t == Tok::Digit('0')).count();
    let rest = exp_toks.get(exp_len..).unwrap_or(&[]);
    let mant_frac = mant.iter().skip_while(|t| **t != Tok::Dot).filter(|t| matches!(t, Tok::Digit(_))).count();
    let step = mant.iter().take_while(|t| **t != Tok::Dot).filter(|t| matches!(t, Tok::Digit(_))).count().max(1) as i32;
    // The mantissa's digits: its decimal exponent is the value's, lowered to a multiple of `step`.
    let (ds, mut exp) = if v == 0.0 { (vec![0; 15], 0) } else { digits15(v) };
    let mut shift = exp.rem_euclid(step);
    exp -= shift;
    let (mut r, carry) = round_at(ds, shift as usize + 1 + mant_frac);
    if carry {
        // Rounding reached the next power of ten (9.999 as `0.00`): one, followed by zeros.
        let e2 = exp + shift + 1;
        shift = e2.rem_euclid(step);
        exp = e2 - shift;
        r = round_at(vec![1], shift as usize + 1 + mant_frac).0;
    }
    let (ip, fp) = r.split_at((shift as usize + 1).min(r.len()));
    let mantissa = fixed_with(mant, mfirst, mlast, sep, dec, |_| (digit_text(ip), digit_text(fp)));
    let sign = if exp < 0 {
        "-"
    } else if plus {
        "+"
    } else {
        ""
    };
    let digits = exp.unsigned_abs().to_string();
    let pad = "0".repeat(width.saturating_sub(digits.len()));
    Some(format!("{mantissa}{e_char}{sign}{pad}{digits}{}", lits(rest)))
}

/// `v` (not negative) in the fixed-point format whose tokens are `toks`, with its digits from
/// `first` to `last`, and whether it rounds to zero.
fn fixed(toks: &[Tok], first: usize, last: usize, v: f64, sep: char, dec: char) -> (String, bool) {
    let percent = toks.contains(&Tok::Percent);
    // Commas right after the last digit scale by a thousand each (`#,##0,` = thousands).
    let after = toks.get(last + 1..).unwrap_or(&[]);
    let scale_commas = after.iter().take_while(|t| **t == Tok::Comma).count();
    let mut x = v * if percent { 100.0 } else { 1.0 } / 1000f64.powi(scale_commas as i32);
    if !x.is_finite() {
        x = 0.0;
    }
    // Whether the number rounds to zero in the digits shown.
    let mut zero = false;
    let out = fixed_with(toks, first, last, sep, dec, |frac| {
        let r = round_dec(x, frac);
        zero = r.0.bytes().chain(r.1.bytes()).all(|d| d == b'0');
        r
    });
    (out, zero)
}

/// The layout of a fixed-point format; `round` gives the integer and fraction digits of the number
/// for the format's number of decimals.
fn fixed_with(toks: &[Tok], first: usize, last: usize, sep: char, dec: char, round: impl FnOnce(usize) -> (String, String)) -> String {
    let number = toks.get(first..=last).unwrap_or(&[]);
    let dot = number.iter().position(|t| *t == Tok::Dot);
    let (int_part, frac_part) = match dot {
        Some(d) => (number.get(..d).unwrap_or(&[]), number.get(d + 1..).unwrap_or(&[])),
        None => (number, &[][..]),
    };
    // Grouping: a comma (or the locale's space, written literally) between integer digits.
    let grouping = int_part.iter().any(|t| *t == Tok::Comma || matches!(t, Tok::Lit(s) if s.trim().is_empty()));
    let int_kinds: Vec<char> = int_part.iter().filter_map(|t| if let Tok::Digit(c) = t { Some(*c) } else { None }).collect();
    let frac_kinds: Vec<char> = frac_part.iter().filter_map(|t| if let Tok::Digit(c) = t { Some(*c) } else { None }).collect();
    let (ip, fp) = round(frac_kinds.len());
    let ip = ip.trim_start_matches('0');
    // Placeholders left of the number's digits: `0` shows a zero, `?` a space, `#` nothing.
    let missing = int_kinds.iter().rev().skip(ip.len());
    // The missing placeholders keep their order: `0???` shows 25 as `0 25`. Spaces before the first
    // zero stay outside the grouping.
    let missing: Vec<char> = missing.rev().copied().collect();
    let lead = missing.iter().take_while(|c| **c == '?').count();
    let rest: String = missing
        .iter()
        .skip(lead)
        .filter_map(|c| match c {
            '0' => Some('0'),
            '?' => Some(PAD),
            _ => None,
        })
        .collect();
    let ip = format!("{rest}{ip}");
    let mut num = if grouping { group(&ip, sep) } else { ip };
    num.insert_str(0, &PAD.to_string().repeat(lead));
    if dot.is_some() {
        num.push(dec);
        // The fraction's digits up to the last one that is not zero show; after it a `0` still
        // shows a zero, a `?` a space and a `#` nothing.
        let last_sig = fp.bytes().rposition(|d| d != b'0');
        for (i, kind) in frac_kinds.iter().enumerate() {
            let d = fp.as_bytes().get(i).map_or('0', |d| char::from(*d));
            if last_sig.is_some_and(|l| i <= l) || *kind == '0' {
                num.push(d);
            } else if *kind == '?' {
                num.push(PAD);
            }
        }
    }
    let before = lits(toks.get(..first).unwrap_or(&[]));
    let after = lits(toks.get(last + 1..).unwrap_or(&[]));
    format!("{before}{num}{after}")
}

/// A fraction format (`# ?/?`, `# ??/??`, `?/8`, `0/0`): the whole part, then the numerator over the
/// denominator closest to the value, with at most as many digits as the denominator has
/// placeholders (a fixed denominator as written), and whether it rounds to zero. `None` if the
/// section is no fraction format.
fn fraction(toks: &[Tok], v: f64, sep: char) -> Option<(String, bool)> {
    if toks.contains(&Tok::Dot) {
        return None;
    }
    let slash = toks.iter().position(|t| *t == Tok::Slash)?;
    let is_digit = |t: Option<&Tok>| matches!(t, Some(Tok::Digit(_)));
    // Numerator placeholders end at the slash.
    let mut ns = slash;
    while ns > 0 && is_digit(toks.get(ns - 1)) {
        ns -= 1;
    }
    let num_kinds: Vec<char> = toks.get(ns..slash)?.iter().filter_map(|t| if let Tok::Digit(c) = t { Some(*c) } else { None }).collect();
    if num_kinds.is_empty() {
        return None;
    }
    // Denominator: placeholders, or written digits.
    let mut de = slash + 1;
    let (mut den_kinds, mut fixed_den) = (vec![], String::new());
    while let Some(t) = toks.get(de) {
        match t {
            Tok::Digit('0') if !fixed_den.is_empty() => fixed_den.push('0'),
            Tok::Digit(c) if fixed_den.is_empty() => den_kinds.push(*c),
            Tok::Lit(s) if den_kinds.is_empty() && s.len() == 1 && s.bytes().all(|b| b.is_ascii_digit()) => fixed_den.push_str(s),
            _ => break,
        }
        de += 1;
    }
    if den_kinds.is_empty() && fixed_den.is_empty() {
        return None;
    }
    // Whole part: placeholders before the numerator, a space between.
    let mut first = ns;
    let mut whole_kinds: Vec<char> = vec![];
    let gap = ns.checked_sub(1).filter(|g| matches!(toks.get(*g), Some(Tok::Lit(s)) if s.trim().is_empty()));
    if let Some(g) = gap {
        let mut ws = g;
        while ws > 0 && matches!(toks.get(ws - 1), Some(Tok::Digit(_) | Tok::Comma)) {
            ws -= 1;
        }
        whole_kinds = toks.get(ws..g)?.iter().filter_map(|t| if let Tok::Digit(c) = t { Some(*c) } else { None }).collect();
        if !whole_kinds.is_empty() {
            first = ws;
        }
    }
    let grouping = toks.get(first..ns).is_some_and(|w| w.contains(&Tok::Comma));
    let has_whole = !whole_kinds.is_empty();
    let den_digits = den_kinds.len().clamp(1, 4);
    let x = v;
    let (mut whole, f) = if has_whole { (x.floor(), x - x.floor()) } else { (0.0, x) };
    let (mut n, d) = if let Ok(d) = fixed_den.parse::<u32>() {
        let d = d.max(1);
        let (ip, _) = round_dec(f * f64::from(d), 0);
        (ip.parse::<f64>().unwrap_or(0.0), f64::from(d))
    } else {
        let max = 10f64.powi(den_digits as i32) - 1.0;
        let (mut best, mut best_err) = ((0.0, 1.0), f64::INFINITY);
        let mut d = 1.0;
        while d <= max {
            let (ip, _) = round_dec(f * d, 0);
            let n = ip.parse::<f64>().unwrap_or(0.0);
            let err = (f - n / d).abs();
            if err < best_err - 1e-12 {
                (best, best_err) = ((n, d), err);
            }
            if err < 1e-12 {
                break;
            }
            d += 1.0;
        }
        best
    };
    if has_whole && n >= d {
        whole += 1.0;
        n -= d;
    }
    // The numerator is right-aligned and the denominator left-aligned in their placeholders.
    let pad_to = |s: String, kinds: &[char], left: bool| {
        let fill = kinds.len().saturating_sub(s.len());
        let c = if kinds.contains(&'?') {
            PAD
        } else if kinds.contains(&'0') && left {
            '0'
        } else {
            return s;
        };
        let fill = c.to_string().repeat(fill);
        if left { format!("{fill}{s}") } else { format!("{s}{fill}") }
    };
    let n_text = format!("{n:.0}");
    let d_text = if fixed_den.is_empty() { format!("{d:.0}") } else { fixed_den.clone() };
    let before = lits(toks.get(..first).unwrap_or(&[]));
    let after = lits(toks.get(de..).unwrap_or(&[]));
    let whole_text = if has_whole {
        let (ip, _) = round_dec(whole, 0);
        let ip = ip.trim_start_matches('0').to_string();
        let ip = if ip.is_empty() && (whole_kinds.contains(&'0') || n == 0.0) { "0".to_string() } else { ip };
        if grouping { group(&ip, sep) } else { ip }
    } else {
        String::new()
    };
    let body = if n == 0.0 && has_whole {
        // A whole value: the fraction's room stays blank.
        let room = num_kinds.len() + 1 + if fixed_den.is_empty() { den_digits } else { fixed_den.len() };
        format!("{whole_text} {}", PAD.to_string().repeat(room))
    } else {
        let frac = format!("{}/{}", pad_to(n_text, &num_kinds, true), pad_to(d_text, &den_kinds, false));
        // A blank whole part keeps the separator's space.
        if whole_text.is_empty() && has_whole {
            format!("{PAD}{frac}")
        } else if whole_text.is_empty() {
            frac
        } else {
            format!("{whole_text} {frac}")
        }
    };
    Some((format!("{before}{body}{after}"), n == 0.0 && whole == 0.0))
}

/// A token of a date or time format.
#[derive(Debug, PartialEq)]
enum DTok {
    Lit(String),
    Year(usize),
    /// `m` or `mm`: a month, or the minutes next to hours or seconds.
    Mm(usize),
    Month(usize),
    Min(usize),
    Day(usize),
    Hour(usize),
    Sec(usize),
    /// `AM/PM`: always upper case, whatever case it is written in.
    Ampm,
    /// `A/P`: the letters as written (`a/p` shows "a" and "p").
    Ap(char, char),
    /// Decimals of the second (`ss.00`).
    Frac(usize),
    /// Elapsed time (`[h]`, `[mm]`, `[ss]`): the unit and its minimum width.
    Elapsed(char, usize),
}

/// The tokens of a date or time section; `None` if it has no date or time part.
fn date_tokens(section: &str) -> Option<Vec<DTok>> {
    let cs: Vec<char> = section.chars().collect();
    let mut out: Vec<DTok> = vec![];
    let mut found = false;
    let mut i = 0;
    let run = |i: usize, c: char| cs.iter().skip(i).take_while(|d| d.eq_ignore_ascii_case(&c)).count();
    while let Some(&c) = cs.get(i) {
        let lc = c.to_ascii_lowercase();
        match lc {
            '"' => {
                let s: String = cs.iter().skip(i + 1).take_while(|d| **d != '"').collect();
                i += s.chars().count() + 2;
                out.push(DTok::Lit(s));
                continue;
            }
            '\\' | '_' | '*' => {
                if let Some(d) = cs.get(i + 1) {
                    if lc == '\\' {
                        out.push(DTok::Lit(d.to_string()));
                    } else if lc == '_' {
                        out.push(DTok::Lit(" ".into()));
                    }
                }
                i += 2;
                continue;
            }
            '[' => {
                let s: String = cs.iter().skip(i + 1).take_while(|d| **d != ']').collect();
                i += s.chars().count() + 2;
                let l = s.to_ascii_lowercase();
                match l.chars().next() {
                    Some(u @ ('h' | 'm' | 's')) if l.chars().all(|d| d == u) => {
                        found = true;
                        out.push(DTok::Elapsed(u, l.len()));
                    }
                    _ => {
                        if let Some(sym) = s.strip_prefix('$') {
                            let sym = sym.split('-').next().unwrap_or("");
                            if !sym.is_empty() {
                                out.push(DTok::Lit(sym.to_string()));
                            }
                        }
                    }
                }
                continue;
            }
            'g' if cs.iter().skip(i).take(7).collect::<String>().eq_ignore_ascii_case("general") => {
                out.push(DTok::Lit("General".into()));
                i += 7;
                continue;
            }
            // `e` alone is the year, as `yyyy`; before a sign it is the exponent of a scientific format.
            'e' if !matches!(cs.get(i + 1), Some('+' | '-')) => {
                found = true;
                out.push(DTok::Year(4));
                i += run(i, 'e');
                continue;
            }
            'y' | 'm' | 'd' | 'h' | 's' => {
                let n = run(i, lc);
                found = true;
                out.push(match lc {
                    'y' => DTok::Year(n),
                    'm' if n >= 3 => DTok::Month(n),
                    'm' => DTok::Mm(n),
                    'd' => DTok::Day(n),
                    'h' => DTok::Hour(n),
                    _ => DTok::Sec(n),
                });
                i += n;
                continue;
            }
            'a' => {
                let rest: String = cs.iter().skip(i).take(5).collect::<String>().to_ascii_lowercase();
                if rest == "am/pm" {
                    found = true;
                    out.push(DTok::Ampm);
                    i += 5;
                } else if rest.starts_with("a/p") {
                    found = true;
                    out.push(DTok::Ap(c, cs.get(i + 2).copied().unwrap_or('P')));
                    i += 3;
                } else {
                    out.push(DTok::Lit(c.to_string()));
                    i += 1;
                }
                continue;
            }
            '.' if matches!(out.last(), Some(DTok::Sec(_) | DTok::Elapsed('s', _))) && cs.get(i + 1) == Some(&'0') => {
                let n = run(i + 1, '0');
                out.push(DTok::Frac(n));
                i += 1 + n;
                continue;
            }
            _ => out.push(DTok::Lit(c.to_string())),
        }
        i += 1;
    }
    if !found {
        return None;
    }
    // `m` is minutes right after hours or right before seconds, else the month.
    let kind = |t: &DTok| matches!(t, DTok::Lit(_));
    for k in 0..out.len() {
        let Some(DTok::Mm(n)) = out.get(k) else { continue };
        let n = *n;
        let before = out.get(..k).and_then(|b| b.iter().rev().find(|t| !kind(t)));
        let after = out.get(k + 1..).and_then(|a| a.iter().find(|t| !kind(t)));
        let minute = matches!(before, Some(DTok::Hour(_) | DTok::Elapsed('h', _))) || matches!(after, Some(DTok::Sec(_) | DTok::Elapsed('s', _)));
        if let Some(t) = out.get_mut(k) {
            *t = if minute { DTok::Min(n) } else { DTok::Month(n) };
        }
    }
    Some(out)
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// The serial date `n` (not negative) as (year, month, day, weekday with 0 = Sunday), or `None`
/// when it is past what chrono can hold. In the 1900 system serial 0 is "1 January 1900" minus a
/// day, and serial 60 is the 29 February 1900 that Excel counts though it never existed; from 61
/// on the serial is the real calendar's. Excel's weekday before 1 March 1900 is one day behind
/// the real calendar, so the weekday of every serial comes from the same shifted date.
fn serial_date(n: i64, date_1904: bool) -> Option<(i64, i64, i64, usize)> {
    let days = Days::new(u64::try_from(n).ok()?);
    // Serial 0 is 1 January 1904, or 30 December 1899 so that serial 1 is 31 December 1899 and
    // 61 is 1 March 1900.
    let epoch = if date_1904 { NaiveDate::from_ymd_opt(1904, 1, 1)? } else { NaiveDate::from_ymd_opt(1899, 12, 30)? };
    let real = epoch.checked_add_days(days)?;
    let wd = real.weekday().num_days_from_sunday() as usize;
    if !date_1904 {
        match n {
            0 => return Some((1900, 1, 0, wd)),
            60 => return Some((1900, 2, 29, wd)),
            // Before the fake day Excel's dates are one day ahead of the real calendar's.
            1..=59 => {
                let shown = real.checked_add_days(Days::new(1))?;
                return Some((i64::from(shown.year()), i64::from(shown.month()), i64::from(shown.day()), wd));
            }
            _ => {}
        }
    }
    Some((i64::from(real.year()), i64::from(real.month()), i64::from(real.day()), wd))
}

/// The serial `v` (not negative) in a date or time format.
fn date_text(toks: &[DTok], v: f64, date_1904: bool, dec: char) -> String {
    // The last day Excel shows: 31 December 9999.
    if v >= if date_1904 { 2_957_004.0 } else { 2_958_466.0 } {
        return "#".repeat(8);
    }
    let subs = toks.iter().find_map(|t| if let DTok::Frac(n) = t { Some((*n).min(3)) } else { None }).unwrap_or(0);
    let exact = v * 86_400_000.0;
    // Excel rounds to whole seconds even when seconds are not displayed; fractional-second
    // tokens select a finer precision. Round once, with 0.01 ms of slack for serial float error.
    let unit = 10i64.pow(3 - subs as u32);
    let ms = (((exact + 0.01) / unit as f64 + 0.5).floor() as i64) * unit;
    let days = ms.div_euclid(86_400_000);
    // Rounding the time up can carry into the day after the last one Excel shows.
    if days > if date_1904 { 2_957_003 } else { 2_958_465 } {
        return "#".repeat(8);
    }
    let day_ms = ms.rem_euclid(86_400_000);
    let (hour, minute, second, milli) = (day_ms / 3_600_000, day_ms / 60_000 % 60, day_ms / 1000 % 60, day_ms % 1000);
    let Some((y, m, d, wd)) = serial_date(days, date_1904) else { return "#".repeat(8) };
    let ampm = toks.iter().any(|t| matches!(t, DTok::Ampm | DTok::Ap(..)));
    let mut out = String::new();
    for t in toks {
        match t {
            DTok::Lit(s) => out.push_str(s),
            DTok::Year(n) => out.push_str(&if *n <= 2 { format!("{:02}", y % 100) } else { format!("{y:04}") }),
            DTok::Month(n) | DTok::Mm(n) => {
                let name = MONTHS.get((m - 1).clamp(0, 11) as usize).copied().unwrap_or("");
                match *n {
                    1 => out.push_str(&m.to_string()),
                    2 => out.push_str(&format!("{m:02}")),
                    3 => out.extend(name.chars().take(3)),
                    4 => out.push_str(name),
                    _ => out.extend(name.chars().take(1)),
                }
            }
            DTok::Day(n) => {
                let name = DAYS.get(wd).copied().unwrap_or("");
                match *n {
                    1 => out.push_str(&d.to_string()),
                    2 => out.push_str(&format!("{d:02}")),
                    3 => out.extend(name.chars().take(3)),
                    _ => out.push_str(name),
                }
            }
            DTok::Hour(n) => {
                let h = if ampm { (hour + 11) % 12 + 1 } else { hour };
                out.push_str(&format!("{h:0w$}", w = (*n).min(2)));
            }
            DTok::Min(n) => out.push_str(&format!("{minute:0w$}", w = (*n).min(2))),
            DTok::Sec(n) => out.push_str(&format!("{second:0w$}", w = (*n).min(2))),
            DTok::Elapsed(u, n) => {
                let n = *n;
                let total = ms
                    / match u {
                        'h' => 3_600_000,
                        'm' => 60_000,
                        _ => 1000,
                    };
                out.push_str(&format!("{total:0n$}"));
            }
            DTok::Frac(n) => {
                out.push(dec);
                let digits = format!("{milli:03}");
                out.extend(digits.chars().chain(std::iter::repeat('0')).take(*n));
            }
            DTok::Ampm => out.push_str(if hour >= 12 { "PM" } else { "AM" }),
            DTok::Ap(a, p) => out.push(if hour >= 12 { *p } else { *a }),
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUB: &str = r#"_-* #\ ##0\ "₽"_-;\-* #\ ##0\ "₽"_-;_-* "-"??\ "₽"_-;_-@_-"#;

    #[test]
    fn an_overlong_format_is_general() {
        let code = "0".repeat(100_000);
        assert_eq!(format(&code, 12.5, None), format("General", 12.5, None));
        let ok = "0".repeat(MAX_CODE_LEN);
        assert_eq!(format(&ok, 7.0, None).len(), MAX_CODE_LEN);
    }

    #[test]
    fn scientific_subnormal() {
        assert_eq!(format("0.00E+00", 5e-324, None), "4.94E-324");
        assert_eq!(format("0.00E+00", f64::MIN_POSITIVE, None), "2.23E-308");
    }

    #[test]
    fn russian_accounting_roubles() {
        assert_eq!(format(RUB, 800_000.0, Some("ru-RU")), "800\u{a0}000 ₽");
        assert_eq!(format(RUB, 1_234_567.0, Some("ru-RU")), "1\u{a0}234\u{a0}567 ₽");
        assert_eq!(format(RUB, -5_000.0, Some("ru-RU")), "-5\u{a0}000 ₽");
        // Excel shows the two `??` of the zero section as spaces: "-", two spaces, the `\ ` space, "₽".
        assert_eq!(format(RUB, 0.0, Some("ru-RU")), "-   ₽");
    }

    #[test]
    fn common_codes() {
        assert_eq!(format("#,##0", 7000.0, Some("ru-RU")), "7\u{a0}000");
        assert_eq!(format("#,##0", 7000.0, Some("en-US")), "7,000");
        assert_eq!(format("#,##0", 7000.0, Some("de-DE")), "7.000");
        assert_eq!(format("0.0%", 0.125, Some("ru-RU")), "12,5%");
        assert_eq!(format("0.0%", 0.125, None), "12.5%");
        assert_eq!(format("0%", 0.0, None), "0%");
        assert_eq!(format("General", 2.5, Some("ru-RU")), "2,5");
        assert_eq!(format("General", 400_000.0, Some("ru-RU")), "400000");
        assert_eq!(format("General", 1.5e12, None), "1.5E+12");
        assert_eq!(format("General", 1e-10, None), "1E-10");
        assert_eq!(format("General", -2.5e-12, None), "-2.5E-12");
        assert_eq!(format("General", 1e100, None), "1E+100");
        assert_eq!(format("0.00", 1.23456, None), "1.23");
        assert_eq!(format("#,##0.0#", 1234.5, None), "1,234.5");
        assert_eq!(format("#,##0,", 12_345_678.0, None), "12,346");
        assert_eq!(format("[$₽-419]#,##0", 1500.0, Some("ru-RU")), "₽1\u{a0}500");
        assert_eq!(format("0;[Red]-0", -3.0, None), "-3");
        assert_eq!(format("0;0", -3.0, None), "3");
        assert_eq!(format("0", -3.0, None), "-3");
        assert_eq!(format("\"x\"0", 5.0, None), "x5");
    }

    #[test]
    fn trailing_commas_with_decimals() {
        assert_eq!(format("0.0,,", 15_000_000.0, Some("en-US")), "15.0");
        assert_eq!(format("0.00,\"K\"", 15_000.0, Some("en-US")), "15.00K");
    }

    #[test]
    fn explicit_empty_sections_hide_their_values() {
        for v in [3.0, -3.0, 0.0, -0.0] {
            assert_eq!(format(";;;", v, None), "", "all sections empty: {v}");
        }
        for (code, positive, negative, zero) in [
            (";0;0", "", "3", "0"),
            ("0;;0", "3", "", "0"),
            ("0;-0;", "3", "-3", ""),
            (";0", "", "3", ""),
            (r#"";"0;;"#, ";3", "", ""),
            (r"\;0;;", ";3", "", ""),
        ] {
            assert_eq!(format(code, 3.0, None), positive, "positive: {code}");
            assert_eq!(format(code, -3.0, None), negative, "negative: {code}");
            assert_eq!(format(code, 0.0, None), zero, "zero: {code}");
        }
        assert_eq!(format("0;-0;", -0.0, None), "");
    }

    #[test]
    fn blank_whole_codes_and_general_keep_their_fallback() {
        for code in ["", " \t", "General", " general "] {
            assert_eq!(format(code, 2.5, None), "2.5", "{code:?}");
            assert_eq!(format(code, -2.5, None), "-2.5", "{code:?}");
            assert_eq!(format(code, 0.0, None), "0", "{code:?}");
            assert_eq!(format(code, 2.5, Some("de-DE")), "2,5", "{code:?}");
        }
    }

    #[test]
    fn scientific_formats() {
        assert_eq!(format("0.00E+00", 1234.0, None), "1.23E+03");
        assert_eq!(format("0.00E+00", 1234.0, Some("ru-RU")), "1,23E+03");
        assert_eq!(format("0.00E+00", 0.00012, None), "1.20E-04");
        assert_eq!(format("0.00E+00", -1234.0, None), "-1.23E+03");
        assert_eq!(format("0.00E+00", 0.0, None), "0.00E+00");
        assert_eq!(format("0.0E+000", 5e10, None), "5.0E+010");
        assert_eq!(format("0.0E-0", 1234.0, None), "1.2E3");
        assert_eq!(format("0.0E-0", 0.0012, None), "1.2E-3");
        assert_eq!(format("0E+00", 9999.0, None), "1E+04");
        assert_eq!(format("0.00E+00", 9.999, None), "1.00E+01");
        assert_eq!(format("0.00e+00", 1234.0, None), "1.23e+03");
        assert_eq!(format("##0.0E+0", 12_345.0, None), "12.3E+3");
        assert_eq!(format("##0.0E+0", 0.012, None), "12.0E-3");
        assert_eq!(format("##0.0E+0", 999.96, None), "1.0E+3");
    }

    /// Each row: format code, value, what Excel shows.
    fn check(rows: &[(&str, f64, &str)]) {
        for (code, v, excel) in rows {
            assert_eq!(format(code, *v, None), *excel, "{code} with {v}");
        }
    }

    #[test]
    fn formats_without_seconds_round_seconds_before_selecting_the_day_and_minute() {
        // Independently observed in Excel with generated serial inputs in both date systems:
        // 0.49 seconds before a boundary carries, 0.51 seconds before it does not.
        let midnight = 45_292.0;
        let hour = midnight + 1.0 / 24.0;
        for date_1904 in [false, true] {
            let render = |code: &str, value| format_dated(code, value, None, date_1904);
            for seconds in [0.0002, 0.0006, 0.49] {
                let before_midnight = midnight - seconds / 86_400.0;
                let before_hour = hour - seconds / 86_400.0;
                assert_eq!(render("m/d/yyyy", before_midnight), render("m/d/yyyy", midnight));
                assert_eq!(render("hh:mm", before_midnight), "00:00");
                assert_eq!(render("hh:mm", before_hour), "01:00");
                assert_eq!(render("hh:mm:ss", before_hour), "01:00:00");
            }
            for seconds in [0.51, 1.0, 29.0] {
                let before_midnight = midnight - seconds / 86_400.0;
                let before_hour = hour - seconds / 86_400.0;
                assert_eq!(render("m/d/yyyy", before_midnight), render("m/d/yyyy", midnight - 1.0));
                assert_eq!(render("hh:mm", before_midnight), "23:59");
                assert_eq!(render("hh:mm", before_hour), "00:59");
            }
            // Explicit fractional seconds retain their precision rather than rounding to seconds.
            let before_hour = hour - 0.49 / 86_400.0;
            assert_eq!(render("hh:mm:ss.000", before_hour), "00:59:59.510");
        }
    }

    #[test]
    fn rounding_is_half_away_from_zero() {
        check(&[
            // code, value, Excel shows
            ("0", 2.5, "3"),
            ("0", 0.5, "1"),
            ("0", 1.5, "2"),
            ("0", 3.5, "4"),
            ("0", -2.5, "-3"),
            ("0", 2.4999, "2"),
            ("0%", 0.125, "13%"),
            ("0%", 0.005, "1%"),
            ("0.00", 0.145, "0.15"),
            ("0.00", 0.125, "0.13"),
            ("0.00", 1.005, "1.01"),
            ("0.00", 2.675, "2.68"),
            ("0.0", 0.25, "0.3"),
            ("0.0", -0.25, "-0.3"),
            ("0.0%", 0.1235, "12.4%"),
            ("#,##0", 2500.5, "2,501"),
            ("#,##0", 1234.5, "1,235"),
            ("#,##0,", 2500.0, "3"),
            ("#,##0,", 12_500.0, "13"),
            ("0.00E+00", 1.125, "1.13E+00"),
            ("0.0E+00", 0.0000125, "1.3E-05"),
            ("##0.0E+0", 12_250.0, "12.3E+3"),
            ("General", 0.1 + 0.2, "0.3"),
            ("General", 1.5e-10, "1.5E-10"),
            ("General", 2.5e15, "2.5E+15"),
            ("General", 1.0000000005, "1.000000001"),
        ]);
    }

    #[test]
    fn a_negative_number_that_rounds_to_zero_shows_no_minus() {
        check(&[
            // code, value, Excel shows
            ("0.00", -0.0033, "0.00"),
            ("0.00", -2.89e-9, "0.00"),
            ("0.00", -0.005, "-0.01"),
            ("0", -0.4, "0"),
            ("0", -0.5, "-1"),
            ("#,##0.00", -0.004, "0.00"),
            ("0.00%", -2.89e-9, "0.00%"),
            ("0.00%", -0.0033, "-0.33%"),
            ("0.00;(0.00)", -0.0033, "(0.00)"),
            ("0.00E+00", -2.89e-9, "-2.89E-09"),
        ]);
    }

    #[test]
    fn a_negative_fraction_that_rounds_to_zero_shows_no_minus() {
        check(&[
            // code, value, Excel shows
            ("# ?/?", -0.01, "0    "),
            ("?/?", -0.01, "0/1"),
            ("# ?/?", -0.5, "- 1/2"),
            ("# ?/?", -1.5, "-1 1/2"),
        ]);
    }

    #[test]
    fn a_value_meeting_no_condition_shows_hashes() {
        check(&[
            // code, value, Excel shows
            ("[>100]0;[<0]0", 50.0, "########"),
            ("[>100]0;[<0]0", 150.0, "150"),
            ("[>100]0;[<0]0", -5.0, "5"),
            ("[>100]0", 50.0, "########"),
            ("[>100]0", 150.0, "150"),
            // a second section without a condition takes the rest
            ("[>100]0;0.0", 50.0, "50.0"),
            // a third section takes the rest
            ("[>100]0;[<0]0;0.00", 50.0, "50.00"),
        ]);
    }

    #[test]
    fn a_negative_date_shows_hashes_in_every_section() {
        for code in ["m/d/yyyy", "m/d/yyyy;m/d/yyyy", "[<0]m/d/yyyy;m/d/yyyy", "m/d/yyyy;m/d/yyyy;m/d/yyyy"] {
            assert_eq!(format(code, -1.0, None), "########", "{code}");
            assert_eq!(format_dated(code, -1.0, None, true), "########", "{code} 1904");
            assert_eq!(format(code, 1.0, None), "1/1/1900", "{code}");
        }
    }

    #[test]
    fn an_exact_tie_at_the_sixteenth_digit_goes_toward_zero() {
        check(&[
            // code, value, Excel shows
            ("0", 1234567890123455.0, "1234567890123450"),
            ("0", 1234567890123415.0, "1234567890123410"),
            ("0", -1234567890123455.0, "-1234567890123450"),
            ("#,##0", 1234567890123455.0, "1,234,567,890,123,450"),
            ("0", 123456789012345.5, "123456789012345"),
            // not a tie: the nearer 15-digit value
            ("0", 1234567890123455.5, "1234567890123460"),
        ]);
    }

    #[test]
    fn question_marks_pad_with_spaces() {
        check(&[
            // code, value, Excel shows
            ("??0", 5.0, "  5"),
            ("??0", 1234.0, "1234"),
            ("0.0?", 1.5, "1.5 "),
            ("0.00?", 3.1, "3.10 "),
            ("0.00?", std::f64::consts::PI, "3.142"),
            ("#.??", 1.0, "1.  "),
            ("0.??", 0.5, "0.5 "),
            ("0.#", 1.0, "1."),
            ("0.##", 1.25, "1.25"),
            ("#,##0.0#", 1234.5, "1,234.5"),
        ]);
    }

    #[test]
    fn escaped_and_quoted_slashes_stay_literal() {
        assert_eq!(format(r"0\/8", 2.0, None), "2/8");
        assert_eq!(format(r#"0"/"8"#, 2.0, None), "2/8");
        assert_eq!(format("# ?/8", 2.5, None), "2 4/8");
    }

    #[test]
    fn fractions_show_the_closest_fraction() {
        check(&[
            // code, value, Excel shows
            ("# ?/?", 1.5, "1 1/2"),
            ("# ?/?", 3.25, "3 1/4"),
            ("# ?/?", 0.5, " 1/2"),
            ("# ?/?", 0.0, "0    "),
            ("# ?/?", -1.5, "-1 1/2"),
            ("# ?/?", std::f64::consts::PI, "3 1/7"),
            ("# ?/?", 2.0, "2    "),
            ("# ?/?", 2.9999, "3    "),
            ("# ??/??", std::f64::consts::PI, "3 14/99"),
            ("# ??/??", 0.5, "  1/2 "),
            ("# ??/??", 1.0 / 3.0, "  1/3 "),
            ("?/?", 0.75, "3/4"),
            ("?/?", 1.5, "3/2"),
            ("# ?/8", 3.5, "3 4/8"),
            ("# ??/16", 0.3125, "  5/16"),
        ]);
    }

    #[test]
    fn conditions_pick_the_section() {
        check(&[
            // code, value, Excel shows
            (r#"[>=1000]#,##0,"K";0"#, 12_500.0, "13K"),
            (r#"[>=1000]#,##0,"K";0"#, 1000.0, "1K"),
            (r#"[>=1000]#,##0,"K";0"#, 999.0, "999"),
            (r#"[>=1000000]0.0,,"M";[>=1000]0.0,"K";0"#, 2_500_000.0, "2.5M"),
            (r#"[>=1000000]0.0,,"M";[>=1000]0.0,"K";0"#, 1500.0, "1.5K"),
            (r#"[>=1000000]0.0,,"M";[>=1000]0.0,"K";0"#, 15.0, "15"),
            (r#"[=0]"none";0.0"#, 0.0, "none"),
            (r#"[=0]"none";0.0"#, 5.0, "5.0"),
            (r#"[Red][<0.5]0.00;0.0"#, 0.25, "0.25"),
            (r#"[Red][<0.5]0.00;0.0"#, 0.75, "0.8"),
            // Only a format of one section adds the minus sign itself
            ("[<1]0.00;0", -5.0, "5.00"),
            ("[<1]0.00;0", 5.0, "5"),
            (r#"[>=1000]0,"K";0"#, -5.0, "5"),
            (r#"[>=1000]0,"K""#, 5000.0, "5K"),
        ]);
    }

    #[test]
    fn general_with_condition_is_a_number() {
        check(&[
            ("[<0]General;0", -5.0, "5"),
            ("[<0]General;0", 5.0, "5"),
            ("[Red]General", 2.5, "2.5"),
            // Quoted text stays text.
            (r#"[<0]"General";0"#, -5.0, "General"),
        ]);
    }

    #[test]
    fn missing_integer_placeholders_keep_their_order() {
        check(&[("0???", 25.0, "0 25"), ("???0", 25.0, "  25")]);
    }

    #[test]
    fn percent_fraction_scales_before_the_fraction() {
        check(&[("# ?/?%", 0.125, "12 1/2%")]);
    }

    #[test]
    fn seconds_round_once_from_the_serial() {
        check(&[
            ("ss", 0.4996 / 86_400.0, "00"),
            ("ss", 0.5004 / 86_400.0, "01"),
            ("ss.0", 0.04996 / 86_400.0, "00.0"),
            ("ss.00", 45_292.0 + 0.005 / 86_400.0, "00.01"),
        ]);
    }

    #[test]
    fn dates_and_times() {
        // 45292 is 1 January 2024, a Monday.
        check(&[
            // code, value, Excel shows
            ("mmm-yy", 45292.0, "Jan-24"),
            ("m/d/yyyy", 45292.0, "1/1/2024"),
            ("mm/dd/yy", 45292.0, "01/01/24"),
            ("d-mmm-yy", 45292.0, "1-Jan-24"),
            ("mmmm d, yyyy", 45292.0, "January 1, 2024"),
            ("mmmmm", 45292.0, "J"),
            ("yyyy-mm-dd", 45323.0, "2024-02-01"),
            ("ddd", 45292.0, "Mon"),
            ("dddd", 45292.0, "Monday"),
            ("[$-409]mmm-yy", 45292.0, "Jan-24"),
            ("[$-F800]dddd, mmmm dd, yyyy", 45292.0, "Monday, January 01, 2024"),
            ("h:mm AM/PM", 45292.75, "6:00 PM"),
            ("h:mm AM/PM", 45292.5, "12:00 PM"),
            ("h:mm AM/PM", 45292.0, "12:00 AM"),
            ("h:mm am/pm", 45292.75, "6:00 PM"),
            ("h:mm a/p", 45292.75, "6:00 p"),
            ("h:mm A/P", 45292.25, "6:00 A"),
            ("e", 45292.0, "2024"),
            ("hh:mm:ss", 45292.75, "18:00:00"),
            ("yyyy-mm-dd hh:mm", 45292.75, "2024-01-01 18:00"),
            ("hh:mm", 0.5, "12:00"),
            ("mm:ss", 0.0001, "00:09"),
            ("h:mm:ss.00", 0.5 + 1.5 / 86_400.0, "12:00:01.50"),
            // Elapsed time
            ("[h]:mm", 1.5, "36:00"),
            ("[hh]:mm:ss", 2.25, "54:00:00"),
            ("[m]:ss", 0.01, "14:24"),
            ("[s]", 0.5, "43200"),
            // Excel counts a 29 February 1900 that never was
            ("m/d/yyyy", 1.0, "1/1/1900"),
            ("m/d/yyyy", 59.0, "2/28/1900"),
            ("m/d/yyyy", 60.0, "2/29/1900"),
            ("m/d/yyyy", 61.0, "3/1/1900"),
            ("dddd", 59.0, "Tuesday"),
            ("dddd", 60.0, "Wednesday"),
            ("dddd", 61.0, "Thursday"),
            ("m/d/yyyy", 2_958_465.0, "12/31/9999"),
            // Before the first day and after the last: hashes
            ("m/d/yyyy", -1.0, "########"),
            ("m/d/yyyy", 2_958_466.0, "########"),
        ]);
        assert_eq!(format("m/d/yyyy", 0.0, None), "1/0/1900");
    }

    #[test]
    fn serial_dates_match_a_day_by_day_walk_of_the_calendar() {
        // 1900 system: serial 1..=59 are 1 January..28 February 1900, 60 is Excel's fake 29
        // February, 61 is 1 March 1900 and the walk goes on to 31 December 9999.
        let mut day = NaiveDate::from_ymd_opt(1900, 1, 1).unwrap();
        for n in 1..=2_958_465_i64 {
            let got = serial_date(n, false).unwrap();
            if n == 60 {
                assert_eq!((got.0, got.1, got.2), (1900, 2, 29));
                day = NaiveDate::from_ymd_opt(1900, 3, 1).unwrap();
            } else {
                assert_eq!((got.0, got.1, got.2), (i64::from(day.year()), i64::from(day.month()), i64::from(day.day())), "1900: {n}");
                day = day.succ_opt().unwrap_or(day);
            }
            // Excel's weekday: serial 1 is a Sunday, whatever the real calendar says before March 1900.
            assert_eq!(got.3, ((n - 1) % 7) as usize, "1900 weekday: {n}");
        }
        assert_eq!(serial_date(0, false), Some((1900, 1, 0, 6)));
        // 1904 system: serial 0 is 1 January 1904, a Friday; the last day is 31 December 9999.
        let mut day = NaiveDate::from_ymd_opt(1904, 1, 1).unwrap();
        for n in 0..=2_957_003_i64 {
            let got = serial_date(n, true).unwrap();
            assert_eq!((got.0, got.1, got.2), (i64::from(day.year()), i64::from(day.month()), i64::from(day.day())), "1904: {n}");
            assert_eq!(got.3, ((n + 5) % 7) as usize, "1904 weekday: {n}");
            day = day.succ_opt().unwrap_or(day);
        }
        assert_eq!(serial_date(2_957_003, true).map(|s| (s.0, s.1, s.2)), Some((9999, 12, 31)));
        assert_eq!(serial_date(-1, true), None);
        assert_eq!(serial_date(-1, false), None);
    }

    #[test]
    fn leap_years_follow_the_gregorian_rule_and_excels_one_mistake() {
        check(&[
            // code, value, Excel shows
            // 1900 is no leap year in the calendar; Excel has a 29 February 1900 anyway (serial 60).
            ("m/d/yyyy", 365.0, "12/30/1900"),
            ("m/d/yyyy", 366.0, "12/31/1900"),
            ("m/d/yyyy", 367.0, "1/1/1901"),
            // 2000 is a leap year, 2100 is not.
            ("m/d/yyyy", 36_584.0, "2/28/2000"),
            ("m/d/yyyy", 36_585.0, "2/29/2000"),
            ("m/d/yyyy", 36_586.0, "3/1/2000"),
            ("m/d/yyyy", 73_109.0, "2/28/2100"),
            ("m/d/yyyy", 73_110.0, "3/1/2100"),
            ("m/d/yyyy", 2_958_464.0, "12/30/9999"),
        ]);
        // The same days in the 1904 system (1462 serials earlier).
        for (serial, excel) in [
            (35_122.0, "2/28/2000"),
            (35_123.0, "2/29/2000"),
            (35_124.0, "3/1/2000"),
            (71_647.0, "2/28/2100"),
            (71_648.0, "3/1/2100"),
            (2_957_003.0, "12/31/9999"),
        ] {
            assert_eq!(format_dated("m/d/yyyy", serial, None, true), excel, "1904: {serial}");
        }
        assert_eq!(format_dated("m/d/yyyy", 2_957_004.0, None, true), "########");
        assert_eq!(format_dated("m/d/yyyy", 2_958_466.0, None, false), "########");
    }

    #[test]
    fn rounding_the_time_up_past_the_last_day_shows_hashes() {
        // 2958465.99999999 is 0.86 ms before 1 January 10000; a format with seconds rounds up to it.
        for (date_1904, last) in [(false, 2_958_465.0), (true, 2_957_003.0)] {
            assert_eq!(format_dated("m/d/yyyy h:mm:ss", last + 0.99999999, None, date_1904), "########", "1904: {date_1904}");
            assert_eq!(format_dated("m/d/yyyy", last + 0.999_999_999_5, None, date_1904), "########", "1904: {date_1904}");
            // Time-only formats are cut off at the same place as every other format.
            assert_eq!(format_dated("h:mm:ss", last + 0.99999999, None, date_1904), "########", "1904: {date_1904}");
            // A time that stays on the last day shows.
            assert_eq!(format_dated("m/d/yyyy h:mm:ss", last + 0.999_988, None, date_1904), "12/31/9999 23:59:59", "1904: {date_1904}");
        }
    }

    #[test]
    fn the_1904_date_system_starts_four_years_later() {
        for (serial, excel) in [(0.0, "1/1/1904"), (43_830.0, "1/1/2024"), (1.0, "1/2/1904")] {
            assert_eq!(format_dated("m/d/yyyy", serial, None, true), excel, "1904: {serial}");
        }
        // Excel shows Friday for serial 0 of the 1904 system.
        assert_eq!(format_dated("dddd", 0.0, None, true), "Friday");
        // The same serial is 1462 days earlier in the 1900 system.
        assert_eq!(format_dated("m/d/yyyy", 43_830.0, None, false), "12/31/2019");
        // Numbers do not care.
        assert_eq!(format_dated("0.0", 2.25, None, true), "2.3");
    }

    #[test]
    fn date_codes_do_not_catch_number_formats() {
        check(&[
            // code, value, Excel shows
            ("0.00E+00", 1234.0, "1.23E+03"),
            (r#"0 "days""#, 5.0, "5 days"),
            (r#"0.0\ "s""#, 5.0, "5.0 s"),
            ("[Red]0.0", 5.0, "5.0"),
            ("[$₽-419]#,##0", 1500.0, "₽1,500"),
        ]);
    }

    #[test]
    fn engineering_formats_round_the_full_mantissa() {
        for (code, value, expected) in [
            ("##0.0E+00", 999.94, "999.9E+00"),
            ("##0.0E+00", 0.99994, "999.9E-03"),
            ("##0.0E+00", -999.94, "-999.9E+00"),
            ("#0.0E+00", 99.94, "99.9E+00"),
            ("##0.0E+00", 999.96, "1.0E+03"),
            ("#0.0E+00", 99.96, "1.0E+02"),
        ] {
            assert_eq!(format(code, value, None), expected, "{code}, {value}");
        }
        assert_eq!(format("##0.0E+00", 999.94, Some("de-DE")), "999,9E+00");
    }

    #[test]
    fn never_panics() {
        for code in [
            "",
            ";",
            ";;;",
            "\"",
            "[",
            "\\",
            "0.",
            ".",
            "#,",
            "%",
            "_",
            "*",
            "@",
            "0E+00",
            "[$",
            "\"unterminated",
            "[>",
            "[>=",
            "?/",
            "/",
            "# ?/",
            "?/?",
            "# ?/0",
            "yyyy",
            "[h",
            "mm",
            "s.0",
            "am/pm",
            "A/",
            "[<5]",
            "[<5];[>3]0",
        ] {
            for v in [0.0, -1.5, 1e300, f64::NAN, f64::INFINITY, 1e-300, 45_292.5, 3e6] {
                let _ = format(code, v, Some("ru"));
                let _ = format_dated(code, v, None, true);
            }
        }
    }
}
