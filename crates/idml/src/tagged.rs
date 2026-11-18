//! InDesign Tagged Text: plain text with `<Tag:value>` markup for paragraph and character styles
//! and local formatting, the text interchange format of layout applications.
//!
//! Written as ASCII (other characters as `<0xXXXX>`); read from ASCII, UTF-8 or UTF-16 files.
//! Paragraph styles, character styles, font family, font style and size are kept; other tags
//! are counted and skipped.

use std::collections::HashSet;
use std::fmt::Write as _;

/// Characters kept of a style or font name read from a file: each paragraph that uses a style
/// carries a copy of its name, so a long one would be multiplied by the paragraph count.
pub const MAX_NAME_CHARS: usize = 256;

/// The formatting of a run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharFormat {
    /// The character style (`CharStyle`); `None` for no character style.
    pub style: Option<String>,
    /// The font family (`cFont`).
    pub font: Option<String>,
    /// The font style (`cTypeface`: `Bold`, `Italic`, …).
    pub typeface: Option<String>,
    /// The size in points (`cSize`). Read as written when it is a finite number: whether a size
    /// makes sense is the caller's to say.
    pub size: Option<f64>,
}

/// Text with one formatting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub text: String,
    pub format: CharFormat,
}

/// A paragraph: its style (`ParaStyle`, empty for none) and its runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paragraph {
    pub style: String,
    pub runs: Vec<Run>,
}

/// A Tagged Text file, read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaggedText {
    pub paragraphs: Vec<Paragraph>,
    /// The paragraph styles the file defines or uses, in order of first appearance.
    pub para_styles: Vec<String>,
    /// The character styles the file defines or uses, in order of first appearance.
    pub char_styles: Vec<String>,
    /// Formatting tags that were not kept.
    pub skipped_tags: usize,
}

fn capped_name(mut name: String) -> String {
    if let Some((at, _)) = name.char_indices().nth(MAX_NAME_CHARS) {
        name.truncate(at);
    }
    name
}

fn escape(out: &mut String, c: char) {
    match c {
        '<' => out.push_str("\\<"),
        '>' => out.push_str("\\>"),
        '\\' => out.push_str("\\\\"),
        c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
        '\t' => out.push('\t'),
        c => {
            let mut buf = [0u16; 2];
            for u in c.encode_utf16(&mut buf) {
                let _ = write!(out, "<0x{u:04X}>");
            }
        }
    }
}

/// A tag value with `<`, `>` and `\` escaped by a backslash, so it keeps the tag intact.
fn escape_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        if matches!(c, '<' | '>' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A tag value (cut to [`MAX_NAME_CHARS`]) with the escapes `escape_value` writes (`\\`, `\<`,
/// `\>`) decoded. Any other backslash, including a trailing one, stays literal, as older files
/// and InDesign's `\:` expect.
fn unescape_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\'
            && let Some(&n) = it.peek()
            && matches!(n, '\\' | '<' | '>')
        {
            out.push(n);
            it.next();
        } else {
            out.push(c);
        }
    }
    capped_name(out)
}

/// The first format version whose `cFont` and `cTypeface` values are escaped like style names.
/// Older files hold them raw, so a backslash in them is literal.
const ESCAPED_FONTS_VERSION: u32 = 8;

/// A `cFont` or `cTypeface` value: decoded in files that escape it, literal in older ones.
fn font_value(v: &str, escaped: bool) -> String {
    if escaped { unescape_value(v) } else { capped_name(v.to_string()) }
}

/// Paragraphs as Tagged Text (`<ASCII-WIN>`, version 8, CRLF between paragraphs).
pub fn write(paragraphs: &[Paragraph]) -> String {
    let mut out = String::from("<ASCII-WIN>\r\n<Version:8>\r\n");
    for (i, p) in paragraphs.iter().enumerate() {
        if i > 0 {
            out.push_str("\r\n");
        }
        let _ = write!(out, "<ParaStyle:{}>", escape_value(&p.style));
        for Run { text, format: f } in &p.runs {
            if text.is_empty() {
                continue;
            }
            let mut open = Vec::new();
            for (tag, value) in [("CharStyle", &f.style), ("cFont", &f.font), ("cTypeface", &f.typeface)] {
                if let Some(v) = value {
                    let _ = write!(out, "<{tag}:{}>", escape_value(v));
                    open.push(tag);
                }
            }
            if let Some(v) = f.size.filter(|v| v.is_finite()) {
                let _ = write!(out, "<cSize:{v:.6}>");
                open.push("cSize");
            }
            text.chars().for_each(|c| escape(&mut out, c));
            for t in open.into_iter().rev() {
                let _ = write!(out, "<{t}:>");
            }
        }
    }
    out
}

/// The text of a Tagged Text file (ASCII, UTF-8, or UTF-16 by its byte-order mark).
pub fn decode(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| unit(*c)).collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        _ => String::from_utf8_lossy(bytes).trim_start_matches('\u{FEFF}').to_string(),
    }
}

/// Is this Tagged Text (by its header)?
pub fn is_tagged(bytes: &[u8]) -> bool {
    let head = decode(bytes.get(..64).unwrap_or(bytes));
    let h = head.trim_start();
    h.starts_with("<ASCII-") || h.starts_with("<UNICODE-") || h.starts_with("<ANSI-")
}

/// Joins UTF-16 code units that arrive one at a time (`<0xNNNN>`) into characters. A character
/// past U+FFFF comes as two units, a surrogate pair: the first half waits for the second.
#[derive(Default)]
struct Utf16Units {
    high: Option<u32>,
}

impl Utf16Units {
    /// The character that `unit` is or completes. Half a pair without its partner is no character.
    fn push(&mut self, unit: u32) -> Option<char> {
        match (self.high.take(), unit) {
            (Some(high), 0xDC00..=0xDFFF) => char::from_u32(0x10000 + ((high - 0xD800) << 10) + (unit - 0xDC00)),
            (_, 0xD800..=0xDBFF) => {
                self.high = Some(unit);
                None
            }
            _ => char::from_u32(unit),
        }
    }

    /// Other text came in between: a waiting first half has no partner.
    fn reset(&mut self) {
        self.high = None;
    }
}

/// Names in order of first appearance. The set keeps a file with very many styles linear.
#[derive(Default)]
struct Names {
    order: Vec<String>,
    seen: HashSet<String>,
}

impl Names {
    fn add(&mut self, name: &str) {
        if !name.is_empty() && self.seen.insert(name.to_string()) {
            self.order.push(name.to_string());
        }
    }
}

struct Reader {
    paragraphs: Vec<Paragraph>,
    format: CharFormat,
    units: Utf16Units,
}

impl Reader {
    fn push(&mut self, c: char) {
        let Some(p) = self.paragraphs.last_mut() else { return };
        match p.runs.last_mut() {
            Some(run) if run.format == self.format => run.text.push(c),
            _ => p.runs.push(Run { text: c.to_string(), format: self.format.clone() }),
        }
    }

    /// `<0xNNNN>`: a UTF-16 code unit. `from_str_radix` accepts a leading `+`, so the digits are
    /// checked first.
    fn code_unit(&mut self, digits: &str) {
        if !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_hexdigit())
            && let Ok(u) = u32::from_str_radix(digits, 16)
            && let Some(ch) = self.units.push(u)
        {
            self.push(ch);
        }
    }
}

/// The end of the tag that starts at `chars[start]` (a `<`): the index of its matching `>`
/// (tags nest in style definitions), or `chars.len()` when it never closes.
fn tag_end(chars: &[char], start: usize) -> usize {
    let mut depth = 0usize;
    let mut j = start;
    while let Some(c) = chars.get(j) {
        match c {
            '\\' => j += 1,
            '<' => depth += 1,
            '>' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
        j += 1;
    }
    chars.len()
}

/// Reads Tagged Text. Every input reads as something: unknown tags are skipped and counted.
pub fn parse(bytes: &[u8]) -> TaggedText {
    let text = decode(bytes);
    let chars: Vec<char> = text.chars().collect();
    let mut r = Reader { paragraphs: vec![Paragraph::default()], format: CharFormat::default(), units: Utf16Units::default() };
    let (mut para_styles, mut char_styles) = (Names::default(), Names::default());
    let mut started = false;
    let mut escaped_fonts = false;
    let mut skipped_tags = 0;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        match (c, chars.get(i + 1)) {
            ('\\', Some(&next)) => {
                r.units.reset();
                r.push(next);
                i += 2;
            }
            ('<', _) => {
                let end = tag_end(&chars, i);
                let tag: String = chars.get(i + 1..end).unwrap_or_default().iter().collect();
                i = end + 1;
                let (name, value) = tag.split_once(':').unwrap_or((tag.as_str(), ""));
                let non_empty = |v: String| (!v.is_empty()).then_some(v);
                match name {
                    n if n.starts_with("0x") => r.code_unit(n.get(2..).unwrap_or_default()),
                    "ParaStyle" => {
                        started = true;
                        let v = unescape_value(value);
                        para_styles.add(&v);
                        if let Some(p) = r.paragraphs.last_mut() {
                            p.style = v;
                        }
                    }
                    "CharStyle" => {
                        let v = unescape_value(value);
                        char_styles.add(&v);
                        r.format.style = non_empty(v);
                    }
                    "cFont" => r.format.font = non_empty(value.to_string()).map(|v| font_value(&v, escaped_fonts)),
                    "cTypeface" => r.format.typeface = non_empty(value.to_string()).map(|v| font_value(&v, escaped_fonts)),
                    "cSize" => r.format.size = value.trim().parse::<f64>().ok().filter(|v| v.is_finite()),
                    "DefineParaStyle" => para_styles.add(&capped_name(value.split('=').next().unwrap_or("").to_string())),
                    "DefineCharStyle" => char_styles.add(&capped_name(value.split('=').next().unwrap_or("").to_string())),
                    "Version" => escaped_fonts = value.trim().parse::<u32>().is_ok_and(|v| v >= ESCAPED_FONTS_VERSION),
                    n if n.starts_with("ASCII-") || n.starts_with("UNICODE-") || n.starts_with("ANSI-") => {}
                    _ => skipped_tags += 1,
                }
            }
            ('\r' | '\n', next) => {
                // A paragraph break (CRLF counts once).
                i += if c == '\r' && next == Some(&'\n') { 2 } else { 1 };
                // Header lines before any text don't start paragraphs.
                let header = !started && r.paragraphs.len() == 1 && r.paragraphs.first().is_some_and(|p| p.runs.is_empty());
                if !header {
                    let style = r.paragraphs.last().map(|p| p.style.clone()).unwrap_or_default();
                    r.paragraphs.push(Paragraph { style, runs: Vec::new() });
                }
            }
            _ => {
                r.units.reset();
                r.push(c);
                i += 1;
            }
        }
    }
    TaggedText { paragraphs: r.paragraphs, para_styles: para_styles.order, char_styles: char_styles.order, skipped_tags }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn text(t: &TaggedText) -> String {
        t.paragraphs.iter().map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    fn run<'a>(t: &'a TaggedText, word: &str) -> &'a Run {
        t.paragraphs.iter().flat_map(|p| &p.runs).find(|r| r.text.contains(word)).unwrap()
    }

    fn one(style: &str, text: &str, format: CharFormat) -> Vec<Paragraph> {
        vec![Paragraph { style: style.into(), runs: vec![Run { text: text.into(), format }] }]
    }

    /// Characters past U+FFFF are written as two tags, a UTF-16 surrogate pair.
    #[test]
    fn surrogate_pairs_become_one_character() {
        let t = parse("<ASCII-WIN>\r\n<Version:7>\r\n<ParaStyle:Body>A<0xD83D><0xDE00>B <0xD842><0xDFB7><0x7530>".as_bytes());
        assert_eq!(text(&t), "A\u{1F600}B \u{20BB7}\u{7530}");
        // What the writer writes comes back.
        assert_eq!(parse(write(&t.paragraphs).as_bytes()).paragraphs, t.paragraphs);
        // Half a pair is not a character: it is left out.
        assert_eq!(text(&parse("<ASCII-WIN>\r\n<ParaStyle:Body>A<0xD83D>B<0xDE00>C".as_bytes())), "ABC");
    }

    /// A style name is read back exactly as written, spaces included.
    #[test]
    fn style_names_keep_leading_and_trailing_spaces() {
        let t = parse("<ASCII-WIN>\r\n<Version:7>\r\n<ParaStyle:Body ><CharStyle: Em >Hello<CharStyle:>".as_bytes());
        assert_eq!(t.paragraphs[0].style, "Body ");
        assert_eq!(run(&t, "Hello").format.style.as_deref(), Some(" Em "));
        assert_eq!(t.para_styles, ["Body "]);
        assert_eq!(t.char_styles, [" Em "]);
    }

    /// A style defined with spaces in its name keeps them, so a paragraph that uses the name matches.
    #[test]
    fn defined_style_names_keep_leading_and_trailing_spaces() {
        let src =
            "<ASCII-WIN>\r\n<DefineParaStyle:Body =<cSize:12>><DefineCharStyle: Em =<cSize:12>><ParaStyle:Body ><CharStyle: Em >Hello<CharStyle:>";
        let t = parse(src.as_bytes());
        assert_eq!(t.para_styles, ["Body "]);
        assert_eq!(t.char_styles, [" Em "]);
        assert_eq!(t.skipped_tags, 0);
    }

    /// `from_str_radix` accepts a leading `+`, so `<0x+41>` would insert an 'A'. A signed or
    /// empty code is not a code point tag and is skipped.
    #[test]
    fn a_signed_hex_code_tag_is_skipped() {
        for tag in ["<0x+41>", "<0x-41>", "<0x>", "<0xZZ>"] {
            let src = format!("<ASCII-WIN>\r\n<ParaStyle:Body>A{tag}B");
            assert_eq!(text(&parse(src.as_bytes())), "AB", "{tag}");
        }
    }

    /// `f64::from_str` accepts "NaN" and "inf"; a size that is not finite is no size.
    #[test]
    fn non_finite_char_size_is_ignored() {
        for v in ["NaN", "inf", "infinity", "-inf", "big"] {
            let src = format!("<ASCII-WIN>\r\n<ParaStyle:Body><cSize:{v}>big<cSize:>");
            assert_eq!(run(&parse(src.as_bytes()), "big").format.size, None, "{v}");
        }
        let t = parse("<ASCII-WIN>\r\n<ParaStyle:Body><cSize:18>big<cSize:> rest".as_bytes());
        assert_eq!(run(&t, "big").format.size, Some(18.0));
        assert_eq!(run(&t, "rest").format.size, None);
    }

    #[test]
    fn a_leading_empty_paragraph_is_kept() {
        let paragraphs = vec![
            Paragraph { style: "Head".into(), runs: Vec::new() },
            Paragraph { style: "Body".into(), runs: vec![Run { text: "Text".into(), format: CharFormat::default() }] },
        ];
        assert_eq!(parse(write(&paragraphs).as_bytes()).paragraphs, paragraphs);
    }

    /// Style names with `:`, `<`, `>` or `\` keep those characters.
    #[test]
    fn style_names_with_tag_characters_round_trip() {
        for name in ["Body: Indent", "A<b>c", "x\\y", "end\\"] {
            let paragraphs = one(name, "Text", CharFormat { style: Some(name.into()), ..Default::default() });
            let out = write(&paragraphs);
            let back = parse(out.as_bytes());
            assert_eq!(back.paragraphs, paragraphs, "{out}");
            assert_eq!(back.para_styles, [name]);
            assert_eq!(back.char_styles, [name]);
        }
    }

    #[test]
    fn font_values_with_tag_characters_round_trip() {
        for name in ["Weird>Font", "A<b>c", "x\\y", "end\\"] {
            let paragraphs = one("", "hello", CharFormat { font: Some(name.into()), typeface: Some(name.into()), ..Default::default() });
            let out = write(&paragraphs);
            assert_eq!(parse(out.as_bytes()).paragraphs, paragraphs, "{out}");
        }
    }

    /// Version 7 files hold font values raw, so their backslashes stay literal.
    #[test]
    fn old_format_font_values_stay_literal() {
        let t = parse("<ASCII-WIN>\r\n<Version:7>\r\n<ParaStyle:Body><cFont:a\\b>x<cFont:><cTypeface:c\\d>y".as_bytes());
        assert_eq!(run(&t, "x").format.font.as_deref(), Some("a\\b"));
        assert_eq!(run(&t, "y").format.typeface.as_deref(), Some("c\\d"));
    }

    /// A backslash that isn't one of the writer's escapes (older files, InDesign's `\:`) stays literal.
    #[test]
    fn unknown_backslashes_in_style_names_stay_literal() {
        assert_eq!(unescape_value(r"x\y"), r"x\y");
        assert_eq!(unescape_value(r"a\:b"), r"a\:b");
        assert_eq!(unescape_value("end\\"), "end\\");
        assert_eq!(unescape_value(r"\<a\>\\"), r"<a>\");
    }

    #[test]
    fn tagged_text_round_trips() {
        let src = "<ASCII-WIN>\r\n<Version:7>\r\n<ParaStyle:Head>Caf<0x00E9> \\<1\\>\r\n<ParaStyle:Body>A <CharStyle:Em>word<CharStyle:> and <cSize:18.000000>big<cSize:> text<pUnknown:1>";
        assert!(is_tagged(src.as_bytes()));
        assert!(!is_tagged(b"plain text") && !is_tagged(b""));
        let t = parse(src.as_bytes());
        assert_eq!(text(&t), "Café <1>\nA word and big text");
        assert_eq!(t.paragraphs.iter().map(|p| p.style.as_str()).collect::<Vec<_>>(), ["Head", "Body"]);
        assert_eq!(run(&t, "word").format.style.as_deref(), Some("Em"));
        assert_eq!(run(&t, "big").format.size, Some(18.0));
        assert_eq!(t.para_styles, ["Head", "Body"]);
        assert_eq!(t.skipped_tags, 1);
        let out = write(&t.paragraphs);
        assert!(
            out.starts_with("<ASCII-WIN>\r\n<Version:8>\r\n") && out.contains("Caf<0x00E9>") && out.contains("<CharStyle:Em>word<CharStyle:>"),
            "{out}"
        );
        assert_eq!(parse(out.as_bytes()).paragraphs, t.paragraphs);
        // UTF-16 with a byte-order mark, either byte order.
        for le in [true, false] {
            let mut u16 = if le { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
            for u in "<UNICODE-WIN>\r\n<ParaStyle:X>Hi".encode_utf16() {
                u16.extend(if le { u.to_le_bytes() } else { u.to_be_bytes() });
            }
            assert!(is_tagged(&u16));
            assert_eq!(text(&parse(&u16)), "Hi");
        }
    }

    #[test]
    fn a_long_name_is_cut() {
        let name = "N".repeat(100_000);
        assert_eq!(unescape_value(&name).len(), MAX_NAME_CHARS);
        assert_eq!(font_value(&name, false).len(), MAX_NAME_CHARS);
        let t = parse(format!("<ASCII-WIN>\r\n<DefineParaStyle:{name}=><ParaStyle:{name}>x").as_bytes());
        assert_eq!(t.para_styles.len(), 1);
        assert_eq!(t.para_styles[0].len(), MAX_NAME_CHARS);
    }

    /// Input that is not Tagged Text, or is cut off inside a tag or an escape, still reads.
    #[test]
    fn hostile_input_reads_as_something() {
        for src in ["", "<", "<<<<", ">>>>", "\\", "a\\", "<ParaStyle:x", "<0x", "<ParaStyle:\\", "\r\n\r\n", "<a:<b:<c>>"] {
            let _ = parse(src.as_bytes());
        }
        let _ = parse(&[0xFF, 0xFE, 0x41]);
        let _ = parse(&[0xFF, 0xFF, 0xFF]);
        // Many unclosed tags and many distinct styles stay linear.
        let many: String = (0..50_000).map(|i| format!("<ParaStyle:s{i}>x\r\n")).collect();
        let t = parse(many.as_bytes());
        assert_eq!(t.para_styles.len(), 50_000);
        assert_eq!(parse("<".repeat(200_000).as_bytes()).skipped_tags, 1);
    }

    #[test]
    fn control_characters_and_empty_runs_are_written_safely() {
        let paragraphs = vec![Paragraph {
            style: "P".into(),
            runs: vec![
                Run { text: String::new(), format: CharFormat { size: Some(9.0), ..Default::default() } },
                Run { text: "a\tb\u{1}".into(), ..Default::default() },
            ],
        }];
        assert_eq!(write(&paragraphs), "<ASCII-WIN>\r\n<Version:8>\r\n<ParaStyle:P>a\tb<0x0001>");
        let nan = one("P", "x", CharFormat { size: Some(f64::NAN), ..Default::default() });
        assert!(!write(&nan).contains("cSize"));
    }
}
