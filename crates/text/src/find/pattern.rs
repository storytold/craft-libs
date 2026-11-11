//! Find what text: options, errors and the compiled [`FindPattern`].

use std::ops::Range;

use super::PageMatches;

/// A paragraph mark in the searched text: `^p` finds it.
pub const PARAGRAPH_MARK: char = '\r';

/// The mark of a paragraph that ends a section, in the searched text only: `^b` finds it.
pub const SECTION_MARK: char = '\u{1D}';

/// The longest Find what text, in characters (Word's 255 is too short for pasted text; WordCraft
/// allows 1024).
pub const MAX_PATTERN: usize = 1024;

/// The largest count in a wildcard `{n,m}`.
pub const MAX_COUNT: u32 = 1000;

/// How the Find what text is read. The default is a plain search ignoring case.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FindOptions {
    #[allow(missing_docs)]
    pub match_case: bool,
    /// Whole words only: a match starts and ends at a word boundary.
    pub whole_word: bool,
    /// Match prefix: a match starts at a word's start.
    pub prefix: bool,
    /// Match suffix: a match ends at a word's end.
    pub suffix: bool,
    /// Ignore punctuation characters between the letters of the text found.
    pub ignore_punctuation: bool,
    /// Ignore white-space characters between the letters of the text found.
    pub ignore_space: bool,
    /// Word's Use wildcards. Wildcards always match case and turn off the five options above, as
    /// Word greys them out.
    pub wildcards: bool,
    /// A regular expression (WordCraft's own option; wins over `wildcards`).
    pub regex: bool,
    /// Take the text literally: no `^` codes (Acrobat and PowerPoint, which have none).
    pub literal: bool,
}

/// Why a Find what text can't be searched for. The app shows it under the field in its own
/// words (`FindLabels::error` in the `ui` module).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FindError {
    /// Longer than [`MAX_PATTERN`] characters.
    TooLong,
    /// A wildcard pattern Word would refuse.
    Wildcards(WildcardError),
    /// A regular expression the engine refuses.
    Regex(RegexError),
}

/// What is wrong with a wildcard pattern (WordCraft's engine names the same problems).
// wordcraft: crates/engine/src/cmd/find_pattern.rs:250,290,314,330 (its error strings).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WildcardError {
    /// A `{` with no closing `}`.
    UnclosedBrace,
    /// A `[` with no closing `]`.
    UnclosedBracket,
    /// `\1` in Find what: back-references belong in Replace with.
    BackReference,
    /// The text between braces is not a count (`{x}`, or an upper bound below the lower one).
    BadCount(String),
    /// A count above [`MAX_COUNT`].
    CountTooLarge,
    /// `[]` or `[!]`.
    EmptyClass,
    /// A range of a class that runs downwards (`[z-a]`).
    ReversedRange {
        /// The range's first character.
        from: char,
        /// The range's last character, which sorts before `from`.
        to: char,
    },
    /// The translated expression is one the engine refuses (an unbalanced parenthesis).
    Refused,
}

/// Why the engine refuses a regular expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegexError {
    /// A `(` with no `)`.
    UnclosedGroup,
    /// A `)` with no `(`.
    UnopenedGroup,
    /// A `[` with no `]`.
    UnclosedClass,
    /// A repetition (`*`, `+`, `?`, `{n}`) with nothing before it.
    NothingToRepeat,
    /// A `{n,m}` that is not a count, is unclosed, or runs downwards.
    BadRepetition,
    /// A class range that runs downwards (`[z-a]`).
    ReversedRange,
    /// A `\` followed by something that is not an escape.
    BadEscape,
    /// A group name that is empty, repeated or not a name.
    BadGroupName,
    /// Back-references (`\1`) and look-around, which the linear-time engine leaves out.
    Unsupported,
    /// A flag group (`(?i)`) the engine does not read.
    BadFlag,
    /// The expression is too big or too deeply nested to compile.
    TooBig,
    /// Any other refusal.
    Invalid,
}

impl RegexError {
    /// The kind of `pattern`'s refusal.
    fn of(pattern: &str, error: &regex::Error) -> RegexError {
        use regex_syntax::ast::ErrorKind as Ast;
        if matches!(error, regex::Error::CompiledTooBig(_)) {
            return RegexError::TooBig;
        }
        match regex_syntax::ParserBuilder::new().build().parse(pattern) {
            Err(regex_syntax::Error::Parse(parse)) => match parse.kind() {
                Ast::GroupUnclosed => RegexError::UnclosedGroup,
                Ast::GroupUnopened => RegexError::UnopenedGroup,
                Ast::ClassUnclosed => RegexError::UnclosedClass,
                Ast::RepetitionMissing => RegexError::NothingToRepeat,
                Ast::RepetitionCountInvalid
                | Ast::RepetitionCountUnclosed
                | Ast::DecimalEmpty
                | Ast::DecimalInvalid
                | Ast::SpecialWordOrRepetitionUnexpectedEof => RegexError::BadRepetition,
                Ast::ClassRangeInvalid | Ast::ClassRangeLiteral => RegexError::ReversedRange,
                Ast::EscapeUnrecognized
                | Ast::EscapeUnexpectedEof
                | Ast::EscapeHexEmpty
                | Ast::EscapeHexInvalid
                | Ast::EscapeHexInvalidDigit
                | Ast::ClassEscapeInvalid
                | Ast::SpecialWordBoundaryUnclosed
                | Ast::SpecialWordBoundaryUnrecognized => RegexError::BadEscape,
                Ast::GroupNameDuplicate { .. } | Ast::GroupNameEmpty | Ast::GroupNameInvalid | Ast::GroupNameUnexpectedEof => {
                    RegexError::BadGroupName
                }
                Ast::UnsupportedBackreference | Ast::UnsupportedLookAround => RegexError::Unsupported,
                Ast::FlagDanglingNegation
                | Ast::FlagDuplicate { .. }
                | Ast::FlagRepeatedNegation { .. }
                | Ast::FlagUnexpectedEof
                | Ast::FlagUnrecognized => RegexError::BadFlag,
                Ast::NestLimitExceeded(_) | Ast::CaptureLimitExceeded => RegexError::TooBig,
                _ => RegexError::Invalid,
            },
            _ => RegexError::Invalid,
        }
    }
}

impl std::fmt::Display for FindError {
    // The words shown to people are `FindLabels::error`; this is for logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindError::TooLong => write!(f, "the Find what text is longer than {MAX_PATTERN} characters"),
            FindError::Wildcards(kind) => write!(f, "wildcard pattern refused: {kind:?}"),
            FindError::Regex(kind) => write!(f, "regular expression refused: {kind:?}"),
        }
    }
}

impl std::error::Error for FindError {}

/// A Find what text compiled with its options. `Send + Sync`: one pattern searches the pages of a
/// document on several threads.
#[derive(Clone, Debug)]
pub struct FindPattern {
    options: FindOptions,
    /// The plain needle (after `^` codes), lowercased unless `match_case`, for the plain path.
    needle: Vec<char>,
    /// The translated expression for wildcards, regular expressions and the ignore options.
    expression: Option<String>,
    /// The pattern can match a paragraph mark, so the app must search across paragraphs.
    spans_paragraphs: bool,
    regex: Option<regex::Regex>,
}

impl FindPattern {
    /// The pattern `query` stands for. A plain query with no `^` codes and none of the ignore
    /// options takes the plain path (PdfCraft's prefix-table search over chars); everything else
    /// is translated to an expression: `^` codes as WordCraft's `find_code`, wildcards as its
    /// `wildcards`, the ignore options as optional runs of punctuation or spaces between the
    /// characters.
    // wordcraft: crates/engine/src/cmd/find_pattern.rs:142 `compile`, :174 `special`, :221
    // `wildcards`; pdfcraft: crates/render/src/text/search.rs (the plain search with case and
    // whole words).
    pub fn compile(query: &str, options: &FindOptions) -> Result<FindPattern, FindError> {
        if query.chars().count() > MAX_PATTERN {
            return Err(FindError::TooLong);
        }
        let options = *options;
        let empty = FindPattern { options, needle: Vec::new(), expression: None, spans_paragraphs: false, regex: None };
        let plain =
            !options.regex && !options.wildcards && !options.ignore_punctuation && !options.ignore_space && (options.literal || !query.contains('^'));
        if plain {
            let needle: Vec<char> = query.chars().map(|c| fold(c, options.match_case)).collect();
            return Ok(FindPattern { needle, ..empty });
        }
        let (pattern, spans) = if options.regex {
            let spans = ["\\r", "\\s", "\\x{1D}", "\r"].iter().any(|code| query.contains(code));
            (query.to_string(), spans)
        } else if options.literal {
            (escape(query), false)
        } else if options.wildcards {
            wildcards(query).map_err(FindError::Wildcards)?
        } else {
            special(query, &options)
        };
        if pattern.is_empty() {
            return Ok(empty);
        }
        let wrap = |p: String| format!("(?:{p})");
        let pattern = if options.wildcards && !options.regex {
            pattern
        } else if options.whole_word {
            format!(r"\b{}\b", wrap(pattern))
        } else {
            match (options.prefix, options.suffix) {
                (true, true) => format!(r"\b{}\b", wrap(pattern)),
                (true, false) => format!(r"\b{}", wrap(pattern)),
                (false, true) => format!(r"{}\b", wrap(pattern)),
                (false, false) => pattern,
            }
        };
        expression_pattern(pattern, spans, options, empty)
    }

    /// The options the pattern was built with.
    pub fn options(&self) -> &FindOptions {
        &self.options
    }

    /// Whether the pattern can match a paragraph mark (`^p`, `^b`, an expression with `\r`): the
    /// app then searches a whole story as one text instead of paragraph by paragraph.
    pub fn spans_paragraphs(&self) -> bool {
        self.spans_paragraphs
    }

    /// Whether the query was empty (or only white space with Ignore white-space): a search
    /// finds nothing and the find bar shows no count.
    pub fn is_empty(&self) -> bool {
        self.needle.is_empty() && self.expression.is_none()
    }

    /// The char ranges of the matches in `text`, in order, not overlapping, at most `limit`
    /// (PdfCraft caps a page at its `limit` so a one-letter search can't build millions).
    // pdfcraft: crates/render/src/text.rs:142 `find_opts_limited`; wordcraft:
    // crates/engine/src/cmd/find_pattern.rs:142 (the compiled expression's use).
    pub fn find_all(&self, text: &str, limit: usize) -> Vec<Range<usize>> {
        if limit == 0 {
            return Vec::new();
        }
        if let Some(regex) = &self.regex {
            let mut chars_before = 0;
            let mut byte = 0;
            let mut out = Vec::new();
            for found in regex.find_iter(text).filter(|m| !m.is_empty()) {
                chars_before += text.get(byte..found.start()).map_or(0, |s| s.chars().count());
                let length = found.as_str().chars().count();
                out.push(chars_before..chars_before + length);
                chars_before += length;
                byte = found.end();
                if out.len() == limit {
                    break;
                }
            }
            return out;
        }
        self.find_plain(text, limit)
    }

    /// One page's matches for [`FindMatches::set_page`]: [`find_all`](Self::find_all) with
    /// `limit`, and whether the page had more than `limit` matches (the bar then counts "12+").
    pub fn find_page(&self, text: &str, limit: usize) -> PageMatches {
        let mut ranges = self.find_all(text, limit.saturating_add(1));
        let limit_reached = ranges.len() > limit;
        ranges.truncate(limit);
        PageMatches { ranges, limit_reached }
    }

    /// The names of the pattern's named groups with their group numbers, in group order. Plain
    /// patterns have none. [`resolve_group_names`] takes [`group_index`](Self::group_index).
    pub fn group_names(&self) -> Vec<(usize, &str)> {
        let Some(regex) = &self.regex else { return Vec::new() };
        regex.capture_names().enumerate().filter_map(|(index, name)| name.map(|name| (index, name))).collect()
    }

    /// The number of the group named `name`, if the pattern has one.
    pub fn group_index(&self, name: &str) -> Option<usize> {
        self.group_names().into_iter().find(|(_, group)| *group == name).map(|(index, _)| index)
    }

    /// The match's groups for a replacement: group 0 is the match; `None` past the last group or
    /// for a group that took part in no match. Plain patterns have group 0 only.
    pub fn groups(&self, text: &str, found: Range<usize>) -> Vec<Option<Range<usize>>> {
        let whole = vec![Some(found.clone())];
        let Some(regex) = &self.regex else { return whole };
        let to_byte = |char_index: usize| text.char_indices().nth(char_index).map_or(text.len(), |(b, _)| b);
        let (start, end) = (to_byte(found.start), to_byte(found.end));
        let Some(captures) = regex.captures_at(text, start) else { return whole };
        if captures.get(0).is_none_or(|m| m.start() != start || m.end() != end) {
            return whole;
        }
        let to_char = |byte: usize| text.get(..byte).map_or(0, |s| s.chars().count());
        captures.iter().map(|group| group.map(|m| to_char(m.start())..to_char(m.end()))).collect()
    }

    /// The plain path: a prefix-table search over chars, non-overlapping, with the word rules.
    // pdfcraft: crates/render/src/text/search.rs:6 `find`.
    fn find_plain(&self, text: &str, limit: usize) -> Vec<Range<usize>> {
        let needle = &self.needle;
        if needle.is_empty() {
            return Vec::new();
        }
        let options = &self.options;
        let chars: Vec<char> = text.chars().collect();
        let folded: Vec<char> = chars.iter().map(|&c| fold(c, options.match_case)).collect();
        let mut prefix = vec![0; needle.len()];
        let mut matched = 0;
        for (i, &c) in needle.iter().enumerate().skip(1) {
            matched = advance(needle, &prefix, matched, c);
            if let Some(slot) = prefix.get_mut(i) {
                *slot = matched;
            }
        }
        matched = 0;
        let word = |i: Option<usize>| i.and_then(|i| chars.get(i)).is_some_and(|c| c.is_alphanumeric());
        let (starts_word, ends_word) = (options.whole_word || options.prefix, options.whole_word || options.suffix);
        let mut hits = Vec::new();
        for (i, &c) in folded.iter().enumerate() {
            matched = advance(needle, &prefix, matched, c);
            if matched != needle.len() {
                continue;
            }
            let end = i + 1;
            let start = end - needle.len();
            if (!starts_word || !word(start.checked_sub(1))) && (!ends_word || !word(Some(end))) {
                hits.push(start..end);
                if hits.len() == limit {
                    break;
                }
                matched = 0;
            } else {
                // A boundary rejection still allows an overlapping later candidate.
                matched = prefix.last().copied().unwrap_or_default();
            }
        }
        hits
    }
}

/// Any character except a paragraph mark (`?`, `^?`, `*`).
const ANY: &str = r"[^\r\x{1D}]";

fn escape(text: &str) -> String {
    regex::escape(text)
}

/// The char as the plain path compares it: lowercased unless `match_case`, one char to one char
/// so indices stay the text's.
fn fold(c: char, match_case: bool) -> char {
    if match_case {
        return c;
    }
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// Extend a matched prefix, falling back through shorter prefixes on a mismatch.
fn advance(needle: &[char], prefix: &[usize], mut matched: usize, c: char) -> usize {
    while matched > 0 && needle.get(matched) != Some(&c) {
        matched = prefix.get(matched - 1).copied().unwrap_or_default();
    }
    if needle.get(matched) == Some(&c) { matched + 1 } else { 0 }
}

/// Build the expression; `empty` is the pattern that finds nothing.
fn expression_pattern(pattern: String, spans: bool, options: FindOptions, empty: FindPattern) -> Result<FindPattern, FindError> {
    // Word's wildcard search always matches case.
    let case_insensitive = !options.match_case && (!options.wildcards || options.regex);
    let regex = regex::RegexBuilder::new(&pattern).case_insensitive(case_insensitive).build().map_err(|error| {
        if options.wildcards && !options.regex {
            FindError::Wildcards(WildcardError::Refused)
        } else {
            FindError::Regex(RegexError::of(&pattern, &error))
        }
    })?;
    Ok(FindPattern { expression: Some(pattern), spans_paragraphs: spans, regex: Some(regex), ..empty })
}

/// What a `^` code stands for in Find what.
// wordcraft: crates/engine/src/cmd/find_pattern.rs:44 `find_code`.
fn find_code(c: char) -> Option<&'static str> {
    Some(match c {
        'p' => r"\r",
        'b' => r"\x{1D}",
        't' => r"\t",
        '?' => ANY,
        '#' => "[0-9]",
        '$' => r"\p{L}",
        '^' => r"\^",
        'l' => r"\n",
        'm' => r"\x{0C}",
        'n' => r"\x{0E}",
        '+' => r"\x{2014}",
        '=' => r"\x{2013}",
        's' => r"\x{A0}",
        '~' => r"\x{2011}",
        '-' => r"\x{AD}",
        'w' => r"[ \t\x{A0}]+",
        'g' => r"\x{FFFC}",
        '%' => "§",
        'v' => "¶",
        _ => return None,
    })
}

/// A character code (`^13`, `^0151`, `^u8212`) as the character it stands for here.
fn code_char(n: u32) -> Option<char> {
    Some(match n {
        13 => PARAGRAPH_MARK,
        11 => '\n',
        12 => '\u{0C}',
        14 => '\u{0E}',
        30 => '\u{2011}',
        31 => '\u{AD}',
        n => char::from_u32(n).filter(|c| !c.is_control() || *c == '\t')?,
    })
}

/// A Windows-1252 code as its Unicode scalar: 128–159 hold typographic characters there (151 is
/// an em dash); the five undefined slots and every other code stay as they are.
fn ansi(n: u32) -> u32 {
    const HIGH: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018,
        0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    n.checked_sub(128).and_then(|k| HIGH.get(k as usize)).copied().unwrap_or(n)
}

/// Read a numeric code after `^` (`13`, `0151`, `u8212`) starting at `chars[i]`; returns the
/// character and how many characters it took.
pub(super) fn numeric_code(chars: &[char], i: usize) -> Option<(char, usize)> {
    let (start, max) = if chars.get(i) == Some(&'u') { (i + 1, 5) } else { (i, 4) };
    let digits: String = chars.get(start..).unwrap_or(&[]).iter().take(max).take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let n: u32 = digits.parse().ok()?;
    // `^0nnn` is a Windows-1252 code; `^nnn` and `^unnnn` are Unicode.
    let n = if start == i && digits.starts_with('0') { ansi(n) } else { n };
    Some((code_char(n)?, start - i + digits.chars().count()))
}

/// A literal character as an expression atom.
fn lit(c: char) -> String {
    if c == PARAGRAPH_MARK {
        return r"\r".into();
    }
    escape(c.encode_utf8(&mut [0; 4]))
}

/// Normal search: literal text with `^` codes, optionally skipping punctuation and white space.
// wordcraft: crates/engine/src/cmd/find_pattern.rs:174 `special`.
fn special(query: &str, options: &FindOptions) -> (String, bool) {
    let chars: Vec<char> = query.chars().collect();
    let mut atoms: Vec<String> = Vec::new();
    let mut spans = false;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        i += 1;
        if c == '^'
            && let Some(&n) = chars.get(i)
        {
            if let Some(code) = find_code(n) {
                spans |= matches!(n, 'p' | 'b');
                atoms.push(code.to_string());
                i += 1;
                continue;
            }
            if let Some((ch, used)) = numeric_code(&chars, i) {
                spans |= ch == PARAGRAPH_MARK;
                atoms.push(lit(ch));
                i += used;
                continue;
            }
            // Not a code: the caret itself.
        }
        if (options.ignore_punctuation && is_punct(c)) || (options.ignore_space && is_space(c)) {
            continue;
        }
        atoms.push(lit(c));
    }
    let filler = match (options.ignore_punctuation, options.ignore_space) {
        (true, true) => r"[\p{P} \t\x{A0}]*",
        (true, false) => r"\p{P}*",
        (false, true) => r"[ \t\x{A0}]*",
        (false, false) => "",
    };
    (atoms.join(filler), spans)
}

fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || matches!(c, '\u{2010}'..='\u{2027}' | '\u{00A1}' | '\u{00BF}' | '\u{00AB}' | '\u{00BB}')
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\u{A0}')
}

/// "Use wildcards" search.
// wordcraft: crates/engine/src/cmd/find_pattern.rs:221 `wildcards`.
fn wildcards(query: &str) -> Result<(String, bool), WildcardError> {
    let chars: Vec<char> = query.chars().collect();
    let mut out = String::new();
    let mut spans = false;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        i += 1;
        match c {
            '?' => out.push_str(ANY),
            '*' => {
                out.push_str(ANY);
                out.push_str("*?");
            }
            '@' => out.push('+'),
            '<' => out.push_str(r"\b{start}"),
            '>' => out.push_str(r"\b{end}"),
            '(' | ')' => out.push(c),
            '{' => {
                let end = chars.get(i..).and_then(|r| r.iter().position(|c| *c == '}')).ok_or(WildcardError::UnclosedBrace)?;
                let body: String = chars.get(i..i + end).unwrap_or(&[]).iter().collect();
                out.push_str(&count(&body)?);
                i += end + 1;
            }
            '[' => {
                let end = chars.get(i..).and_then(|r| r.iter().position(|c| *c == ']')).ok_or(WildcardError::UnclosedBracket)?;
                out.push_str(&class(chars.get(i..i + end).unwrap_or(&[]))?);
                i += end + 1;
            }
            '\\' => match chars.get(i) {
                Some(d) if d.is_ascii_digit() => return Err(WildcardError::BackReference),
                Some(&n) => {
                    out.push_str(&lit(n));
                    i += 1;
                }
                None => out.push_str(r"\\"),
            },
            '^' => {
                let Some(&n) = chars.get(i) else {
                    out.push_str(r"\^");
                    continue;
                };
                if let Some(code) = find_code(n) {
                    spans |= matches!(n, 'p' | 'b');
                    out.push_str(code);
                    i += 1;
                } else if let Some((ch, used)) = numeric_code(&chars, i) {
                    spans |= ch == PARAGRAPH_MARK;
                    out.push_str(&lit(ch));
                    i += used;
                } else {
                    out.push_str(r"\^");
                }
            }
            c => out.push_str(&lit(c)),
        }
    }
    Ok((out, spans))
}

/// `{n}`, `{n,}`, `{n,m}` (and `{,m}`).
fn count(body: &str) -> Result<String, WildcardError> {
    let bad = || WildcardError::BadCount(body.to_owned());
    let num = |s: &str| -> Result<Option<u32>, WildcardError> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(None);
        }
        let n: u32 = s.parse().map_err(|_| bad())?;
        if n > MAX_COUNT {
            return Err(WildcardError::CountTooLarge);
        }
        Ok(Some(n))
    };
    match body.split_once([',', ';']) {
        None => Ok(format!("{{{}}}", num(body)?.ok_or_else(bad)?)),
        Some((a, b)) => {
            let a = num(a)?.unwrap_or(0);
            match num(b)? {
                Some(b) if b < a => Err(bad()),
                Some(b) => Ok(format!("{{{a},{b}}}")),
                None => Ok(format!("{{{a},}}")),
            }
        }
    }
}

/// `[abc]`, `[a-z]`, `[!x]`: the characters inside are literal.
fn class(inner: &[char]) -> Result<String, WildcardError> {
    let (negated, inner) = match inner.split_first() {
        Some(('!', rest)) => (true, rest),
        _ => (false, inner),
    };
    if inner.is_empty() {
        return Err(WildcardError::EmptyClass);
    }
    let mut out = String::from(if negated { r"[^\r\x{1D}" } else { "[" });
    let mut i = 0;
    while let Some(&c) = inner.get(i) {
        let c = if c == '\\' && i + 1 < inner.len() {
            i += 1;
            inner.get(i).copied().unwrap_or(c)
        } else {
            c
        };
        i += 1;
        if inner.get(i) == Some(&'-')
            && let Some(&hi) = inner.get(i + 1)
        {
            if hi < c {
                return Err(WildcardError::ReversedRange { from: c, to: hi });
            }
            out.push_str(&format!("{}-{}", lit(c), lit(hi)));
            i += 2;
        } else {
            out.push_str(&lit(c));
        }
    }
    out.push(']');
    Ok(out)
}
