//! SYLK (`.slk`): the symbolic link text format older spreadsheets and some legacy systems
//! export. One record per line, fields separated by `;` (`;;` is a literal semicolon):
//!
//! - `ID` starts the file; `E` ends it.
//! - `C;Y<row>;X<col>;K<value>;E<formula>` is a cell. `Y` and `X` may be left out to stay on the
//!   current row or column. `K` is a number, `"text"`, `TRUE`/`FALSE` or an error; `E` is a
//!   formula in R1C1 notation; `S;R<row>;C<col>` shares the formula of another cell.
//! - `P;P<code>` adds a number format to the file's list; `F;P<n>;Y<row>;X<col>` gives a cell the
//!   n-th format (0-based), and `F;W<first> <last> <width>` sets column widths in characters.
//!
//! Other records (fonts, options, window settings) are skipped. Formulas are converted to A1 with
//! the same relative and absolute references, as Excel does when it opens the file.

use std::collections::{BTreeMap, HashMap};

/// The largest SYLK file read: past it the file is refused.
pub const MAX_BYTES: usize = 512 << 20;
/// The grid cells are read into (Excel's): positions outside it make a record unreadable.
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;
/// Column-width records kept from one file; each can cover every column.
const MAX_WIDTH_RECORDS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The file is larger than [`MAX_BYTES`].
    #[error("the SYLK file is larger than {} MB", MAX_BYTES >> 20)]
    TooLarge,
    /// The text does not start with an `ID` record.
    #[error("not a SYLK file (it does not start with an ID record)")]
    NotSylk,
}

/// A cell's value.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Value {
    #[default]
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    /// An unquoted value that starts with `#`, as written (`#DIV/0!`). Which of these are error
    /// values is the caller's decision.
    Error(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cell {
    pub value: Value,
    /// The formula in A1 notation, without a leading `=`. It is the file's text with its
    /// references rewritten, not checked for syntax.
    pub formula: Option<String>,
    /// The cell's number format code, when the file gives it one other than `General`.
    pub format: Option<String>,
}

/// An `F;W` record: columns `first..=last` (zero-based) are `width` characters wide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnWidth {
    pub first: u32,
    pub last: u32,
    pub width: f64,
}

/// The one sheet a SYLK file holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sheet {
    /// Cells by zero-based (row, column).
    pub cells: BTreeMap<(u32, u32), Cell>,
    /// Column widths in file order (a later record overrides an earlier one).
    pub column_widths: Vec<ColumnWidth>,
    /// Cell records that could not be read (a position outside the grid) and were skipped.
    pub skipped: usize,
}

/// One-letter field tag and its argument.
fn tagged(field: &str) -> (&str, &str) {
    field.split_at(field.chars().next().map_or(0, char::len_utf8))
}

/// Reads a SYLK file.
pub fn read(bytes: &[u8]) -> Result<Sheet, Error> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        // Older exports are in a single-byte code page: keep each byte as its Latin-1 character.
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if !text.starts_with("ID") {
        return Err(Error::NotSylk);
    }
    let mut sheet = Sheet::default();
    let mut formats: Vec<String> = Vec::new();
    // Cells' number formats by index into `formats`, applied once every format is known.
    let mut cell_formats: Vec<((u32, u32), usize)> = Vec::new();
    // Each formula cell's R1C1 text, for the cells that share it.
    let mut formulas: HashMap<(u32, u32), String> = HashMap::new();
    let (mut row, mut col) = (0u32, 0u32);
    for line in text.lines() {
        let fields = split_fields(line);
        let Some((kind, rest)) = fields.split_first() else { continue };
        match kind.as_str() {
            "C" => cell_record(rest, &mut sheet, &mut formulas, &mut row, &mut col),
            "F" => {
                let mut format = None;
                for f in rest {
                    let (tag, arg) = tagged(f);
                    match tag {
                        "Y" => row = position(arg, MAX_ROWS).unwrap_or(row),
                        "X" => col = position(arg, MAX_COLS).unwrap_or(col),
                        "P" => format = arg.parse::<usize>().ok(),
                        "W" => {
                            let mut parts = arg.split_whitespace();
                            if let (Some(first), Some(last), Some(width)) = (parts.next(), parts.next(), parts.next())
                                && let (Some(first), Some(last), Ok(width)) =
                                    (position(first, MAX_COLS), position(last, MAX_COLS), width.parse::<f64>())
                                && first <= last
                                && (0.0..=255.0).contains(&width)
                                // A file of width records can't make its reader loop over every
                                // column again and again.
                                && sheet.column_widths.len() < MAX_WIDTH_RECORDS
                            {
                                sheet.column_widths.push(ColumnWidth { first, last, width });
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(format) = format {
                    cell_formats.push(((row, col), format));
                }
            }
            // Number formats only: `P;F…` and `P;E…` are fonts.
            "P" => {
                if let Some(code) = rest.first().and_then(|f| f.strip_prefix('P')) {
                    formats.push(code.to_string());
                }
            }
            "E" => break,
            _ => {}
        }
    }
    for (at, index) in cell_formats {
        let Some(code) = formats.get(index).filter(|code| !code.is_empty() && *code != "General") else { continue };
        sheet.cells.entry(at).or_default().format = Some(code.clone());
    }
    Ok(sheet)
}

/// A `C` record: a cell's value and formula at the current or the given position.
fn cell_record(fields: &[String], sheet: &mut Sheet, formulas: &mut HashMap<(u32, u32), String>, row: &mut u32, col: &mut u32) {
    let (mut value, mut formula, mut shared) = (None, None, None::<(Option<u32>, Option<u32>)>);
    let mut bad = false;
    for f in fields {
        let (tag, arg) = tagged(f);
        match tag {
            "Y" => match position(arg, MAX_ROWS) {
                Some(r) => *row = r,
                None => bad = true,
            },
            "X" => match position(arg, MAX_COLS) {
                Some(c) => *col = c,
                None => bad = true,
            },
            "K" => value = Some(parse_value(arg)),
            "E" => formula = Some(arg.to_string()),
            "S" => shared = Some((None, None)),
            // In a shared-formula cell, R and C name the cell whose formula it shares.
            "R" if shared.is_some() => shared = shared.map(|(_, c)| (position(arg, MAX_ROWS), c)),
            "C" if shared.is_some() => shared = shared.map(|(r, _)| (r, position(arg, MAX_COLS))),
            _ => {}
        }
    }
    if bad {
        sheet.skipped += 1;
        return;
    }
    let at = (*row, *col);
    if let Some((Some(r), Some(c))) = shared {
        formula = formulas.get(&(r, c)).cloned();
    }
    let cell = sheet.cells.entry(at).or_default();
    if let Some(v) = value {
        cell.value = v;
    }
    if let Some(r1c1) = formula {
        cell.formula = Some(r1c1_formula_to_a1(&r1c1, at));
        formulas.insert(at, r1c1);
    }
}

/// A record's fields: `;` separates them and `;;` is a literal semicolon.
fn split_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.trim_end_matches('\r').chars().peekable();
    while let Some(c) = chars.next() {
        if c == ';' {
            if chars.peek() == Some(&';') {
                chars.next();
                if let Some(f) = fields.last_mut() {
                    f.push(';');
                }
            } else {
                fields.push(String::new());
            }
        } else if let Some(f) = fields.last_mut() {
            f.push(c);
        }
    }
    fields
}

/// A 1-based row or column number → 0-based, within `max`.
fn position(arg: &str, max: u32) -> Option<u32> {
    arg.trim().parse::<u32>().ok().filter(|n| (1..=max).contains(n)).map(|n| n - 1)
}

/// A `K` value: `"text"` (everything between the first and the last quote, with Excel's
/// character escapes decoded), a boolean, a number, or other text (an error value when it starts
/// with `#`).
fn parse_value(arg: &str) -> Value {
    if let Some(text) = arg.strip_prefix('"') {
        let text = text.strip_suffix('"').unwrap_or(text);
        return Value::Text(decode_escapes(text));
    }
    if arg.eq_ignore_ascii_case("TRUE") || arg.eq_ignore_ascii_case("FALSE") {
        return Value::Bool(arg.eq_ignore_ascii_case("TRUE"));
    }
    if arg.starts_with('#') {
        return Value::Error(arg.to_string());
    }
    match arg.trim().parse::<f64>() {
        Ok(n) if n.is_finite() => Value::Number(n),
        _ => Value::Text(arg.to_string()),
    }
}

/// ISO 6937's diacritics (codes 0xC1–0xCF): each code, its combining mark, and the letters it
/// composes with, as pairs of letter and precomposed letter (from Unicode's canonical composition).
/// A diacritic on a SPACE is the spacing accent (`´`, `¨`, `ˇ`, ...), as in glibc's ISO_6937 map; the
/// grave, circumflex and tilde have none, since `` ` ``, `^` and `~` are plain ASCII.
const ISO6937_DIACRITICS: [(u8, char, &str); 13] = [
    (0xC1, '\u{0300}', "AÀaàEÈeèIÌiìNǸnǹOÒoòUÙuùWẀwẁYỲyỳ"), // grave
    (0xC2, '\u{0301}', " ´AÁaáCĆcćEÉeéGǴgǵIÍiíKḰkḱLĹlĺMḾmḿNŃnńOÓoóPṔpṕRŔrŕSŚsśUÚuúWẂwẃYÝyýZŹzź"), // acute
    (0xC3, '\u{0302}', "AÂaâCĈcĉEÊeêGĜgĝHĤhĥIÎiîJĴjĵOÔoôSŜsŝUÛuûWŴwŵYŶyŷZẐzẑ"), // circumflex
    (0xC4, '\u{0303}', "AÃaãEẼeẽIĨiĩNÑnñOÕoõUŨuũVṼvṽYỸyỹ"), // tilde
    (0xC5, '\u{0304}', " ¯AĀaāEĒeēGḠgḡIĪiīOŌoōUŪuūYȲyȳ"),   // macron
    (0xC6, '\u{0306}', " ˘AĂaăEĔeĕGĞgğIĬiĭOŎoŏUŬuŭ"),       // breve
    (0xC7, '\u{0307}', " ˙AȦaȧBḂbḃCĊcċDḊdḋEĖeėFḞfḟGĠgġHḢhḣIİMṀmṁNṄnṅOȮoȯPṖpṗRṘrṙSṠsṡTṪtṫWẆwẇXẊxẋYẎyẏZŻzż"), // dot above
    (0xC8, '\u{0308}', " ¨AÄaäEËeëHḦhḧIÏiïOÖoötẗUÜuüWẄwẅXẌxẍYŸyÿ"), // diaeresis
    (0xCA, '\u{030a}', " ˚AÅaåUŮuůwẘyẙ"),                   // ring
    (0xCB, '\u{0327}', " ¸CÇcçDḐdḑEȨeȩGĢgģHḨhḩKĶkķLĻlļNŅnņRŖrŗSŞsşTŢtţ"), // cedilla
    (0xCD, '\u{030b}', " ˝OŐoőUŰuű"),                       // double acute
    (0xCE, '\u{0328}', " ˛AĄaąEĘeęIĮiįOǪoǫUŲuų"),           // ogonek
    (0xCF, '\u{030c}', " ˇAǍaǎCČcčDĎdďEĚeěGǦgǧHȞhȟIǏiǐjǰKǨkǩLĽlľNŇnňOǑoǒRŘrřSŠsšTŤtťUǓuǔZŽzž"), // caron
];

/// ISO 6937's spacing characters above ASCII (codes 0xA1–0xFE that aren't diacritics).
const ISO6937_SPACING: [(u8, char); 71] = [
    (0xA1, '¡'),
    (0xA2, '¢'),
    (0xA3, '£'),
    (0xA5, '¥'),
    (0xA7, '§'),
    (0xA8, '¤'),
    (0xA9, '‘'),
    (0xAA, '“'),
    (0xAB, '«'),
    (0xAC, '←'),
    (0xAD, '↑'),
    (0xAE, '→'),
    (0xAF, '↓'),
    (0xB0, '°'),
    (0xB1, '±'),
    (0xB2, '²'),
    (0xB3, '³'),
    (0xB4, '×'),
    (0xB5, 'µ'),
    (0xB6, '¶'),
    (0xB7, '·'),
    (0xB8, '÷'),
    (0xB9, '’'),
    (0xBA, '”'),
    (0xBB, '»'),
    (0xBC, '¼'),
    (0xBD, '½'),
    (0xBE, '¾'),
    (0xBF, '¿'),
    (0xD0, '―'),
    (0xD1, '¹'),
    (0xD2, '®'),
    (0xD3, '©'),
    (0xD4, '™'),
    (0xD5, '♪'),
    (0xD6, '¬'),
    (0xD7, '¦'),
    (0xDC, '⅛'),
    (0xDD, '⅜'),
    (0xDE, '⅝'),
    (0xDF, '⅞'),
    (0xE0, 'Ω'),
    (0xE1, 'Æ'),
    (0xE2, 'Đ'),
    (0xE3, 'ª'),
    (0xE4, 'Ħ'),
    (0xE6, 'Ĳ'),
    (0xE7, 'Ŀ'),
    (0xE8, 'Ł'),
    (0xE9, 'Ø'),
    (0xEA, 'Œ'),
    (0xEB, 'º'),
    (0xEC, 'Þ'),
    (0xED, 'Ŧ'),
    (0xEE, 'Ŋ'),
    (0xEF, 'ŉ'),
    (0xF0, 'ĸ'),
    (0xF1, 'æ'),
    (0xF2, 'đ'),
    (0xF3, 'ð'),
    (0xF4, 'ħ'),
    (0xF5, 'ı'),
    (0xF6, 'ĳ'),
    (0xF7, 'ŀ'),
    (0xF8, 'ł'),
    (0xF9, 'ø'),
    (0xFA, 'œ'),
    (0xFB, 'ß'),
    (0xFC, 'þ'),
    (0xFD, 'ŧ'),
    (0xFE, 'ŋ'),
];

/// Text with Excel's SYLK character escapes decoded. Excel writes a character outside ASCII as
/// `ESC N` and one byte, the character's ISO 6937 code less 0x80. A diacritic's code is followed
/// by the letter it goes on (`ESC N H o` is `ö`, `ESC N {` is `ß`). A code ISO 6937 doesn't define,
/// or an `ESC` that isn't followed by `N` and a code, is kept as written.
fn decode_escapes(text: &str) -> String {
    if !text.contains('\u{1b}') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        let mut ahead = chars.clone();
        let code = (c == '\u{1b}' && ahead.next() == Some('N'))
            .then(|| ahead.next())
            .flatten()
            .and_then(|b| u8::try_from(b).ok())
            .filter(|b| (0x20..0x80).contains(b));
        let Some(code) = code.map(|b| b + 0x80) else {
            out.push(c);
            continue;
        };
        if let Some(&(_, mark, pairs)) = ISO6937_DIACRITICS.iter().find(|(d, _, _)| *d == code) {
            // The letter it goes on: precomposed where Unicode has the pair, else letter and mark.
            // The base is an ordinary character: another escape (or any control character) after a
            // diacritic leaves this escape as written, and the next one is decoded on its own.
            let Some(base) = ahead.next().filter(|b| !b.is_control()) else {
                out.push(c);
                continue;
            };
            let letters: Vec<char> = pairs.chars().collect();
            match letters.chunks(2).find(|p| p.first() == Some(&base)).and_then(|p| p.get(1)) {
                Some(&composed) => out.push(composed),
                None => {
                    out.push(base);
                    out.push(mark);
                }
            }
        } else if let Some(&(_, ch)) = ISO6937_SPACING.iter().find(|(s, _)| *s == code) {
            out.push(ch);
        } else {
            out.push(c);
            continue;
        }
        chars = ahead;
    }
    out
}

/// One part of an R1C1 reference: `R`/`C` followed by an absolute number, a bracketed offset or
/// nothing (the cell's own row or column).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    Absolute(u32),
    Relative(i64),
}

impl Part {
    /// The 0-based index and whether it is absolute, or `None` when it falls off the sheet.
    fn resolve(self, base: u32, max: u32) -> Option<(u32, bool)> {
        match self {
            Part::Absolute(n) => (1..=max).contains(&n).then(|| (n - 1, true)),
            Part::Relative(k) => i64::from(base).checked_add(k).and_then(|v| u32::try_from(v).ok()).filter(|v| *v < max).map(|v| (v, false)),
        }
    }
}

/// Parses the number after `R` or `C` at `i`: digits, `[offset]` or nothing. Returns the part and
/// the index after it, or `None` when the text isn't a reference part.
fn parse_part(chars: &[char], i: usize) -> Option<(Part, usize)> {
    match chars.get(i) {
        Some('[') => {
            let close = chars[i..].iter().position(|&c| c == ']')? + i;
            let inner: String = chars[i + 1..close].iter().collect();
            Some((Part::Relative(inner.parse().ok()?), close + 1))
        }
        Some(c) if c.is_ascii_digit() => {
            let end = chars[i..].iter().position(|c| !c.is_ascii_digit()).map_or(chars.len(), |p| p + i);
            let n: String = chars[i..end].iter().collect();
            Some((Part::Absolute(n.parse().unwrap_or(u32::MAX)), end))
        }
        _ => Some((Part::Relative(0), i)),
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '\\' | '?' | '$')
}

/// Rewrites the R1C1 references in a SYLK formula as A1 references for the cell at `at`
/// (zero-based row and column):
/// `RC[-1]` → `B2`, `R1C1` → `$A$1`, `R[-2]C:R[-1]C` → `D2:D3`, a whole column `C4` → `$D:$D` and
/// a whole row `R3` → `$3:$3`. Text in quotes and quoted sheet names are left alone; a reference
/// off the sheet becomes `#REF!`.
pub fn r1c1_formula_to_a1(formula: &str, at: (u32, u32)) -> String {
    let chars: Vec<char> = formula.chars().collect();
    let mut out = String::with_capacity(formula.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' {
            // Copy the quoted run as it is; a doubled quote stays inside it.
            let mut j = i + 1;
            while j < chars.len() && !(chars[j] == c && chars.get(j + 1) != Some(&c)) {
                j += if chars[j] == c { 2 } else { 1 };
            }
            let stop = (j + 1).min(chars.len());
            out.extend(&chars[i..stop]);
            i = stop;
            continue;
        }
        let starts_word = i == 0 || !is_word_char(chars[i - 1]);
        if starts_word
            && matches!(c, 'R' | 'r' | 'C' | 'c')
            && let Some((text, next)) = reference_at(&chars, i, at)
        {
            out.push_str(&text);
            i = next;
            continue;
        }
        // Copy the rest of a word as it is, so `ROUND` or `Sheet2` isn't read as a reference.
        if is_word_char(c) {
            let end = (i..chars.len()).find(|&j| !is_word_char(chars[j])).unwrap_or(chars.len());
            out.extend(&chars[i..end]);
            i = end;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// The A1 text of the R1C1 reference starting at `i`, and the index after it, or `None` when the
/// word there isn't a reference (a function or a name).
fn reference_at(chars: &[char], i: usize, at: (u32, u32)) -> Option<(String, usize)> {
    let mut j = i;
    let mut row = None;
    let mut col = None;
    if matches!(chars.get(j), Some('R' | 'r')) {
        let (part, next) = parse_part(chars, j + 1)?;
        row = Some(part);
        j = next;
    }
    if matches!(chars.get(j), Some('C' | 'c')) {
        let (part, next) = parse_part(chars, j + 1)?;
        col = Some(part);
        j = next;
    }
    if row.is_none() && col.is_none() {
        return None;
    }
    // A reference ends the word: `ROUND(`, `RATE` or `R2D2` are names.
    if chars.get(j).is_some_and(|&c| is_word_char(c) || c == '(' || c == '[' || c == '!') {
        return None;
    }
    let row_text = |p: Part| p.resolve(at.0, MAX_ROWS).map(|(r, abs)| format!("{}{}", if abs { "$" } else { "" }, r + 1));
    let col_text = |p: Part| p.resolve(at.1, MAX_COLS).map(|(c, abs)| format!("{}{}", if abs { "$" } else { "" }, col_letters(c)));
    // One side of a range (`R2:R4`, `C1:C3`) is written alone; a lone row or column is the whole of it.
    let prev = chars[..i].iter().rev().find(|c| !c.is_whitespace());
    let in_range = prev == Some(&':') || chars[j..].iter().find(|c| !c.is_whitespace()) == Some(&':');
    let text = match (row, col) {
        (Some(r), Some(c)) => row_text(r).zip(col_text(c)).map(|(r, c)| format!("{c}{r}")),
        (Some(r), None) => row_text(r).map(|r| if in_range { r } else { format!("{r}:{r}") }),
        (None, Some(c)) => col_text(c).map(|c| if in_range { c } else { format!("{c}:{c}") }),
        (None, None) => None,
    };
    Some((text.unwrap_or_else(|| "#REF!".into()), j))
}

/// 0 → `A`, 25 → `Z`, 26 → `AA`.
fn col_letters(col: u32) -> String {
    let mut n = u64::from(col) + 1;
    let mut out = Vec::new();
    while n > 0 {
        out.push(char::from(b'A' + ((n - 1) % 26) as u8));
        n = (n - 1) / 26;
    }
    out.iter().rev().collect()
}
