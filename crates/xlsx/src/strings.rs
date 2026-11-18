//! String items (`CT_Rst`, ECMA-376 Part 1, 18.4): shared strings, inline strings and comment
//! text, with the `_xHHHH_` escapes of `ST_Xstring` (22.9.2.19).

use std::sync::Arc;

use craft_xml::El;

use crate::styles::Font;

/// A formatted run of a string item (`<r>`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub text: String,
    /// `<rPr>`, `None` when the run has none.
    pub font: Option<Font>,
}

/// A phonetic reading over part of a string's text (`<rPh>`, 18.4.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneticRun {
    /// Offsets into the base text in UTF-16 code units, `start < end`, both within the text.
    pub start: u32,
    pub end: u32,
    pub text: String,
}

/// `<phoneticPr>` (18.4.7). `kind` and `alignment` are kept only when they are values the
/// schema defines (`ST_PhoneticType`, `ST_PhoneticAlignment`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhoneticProps {
    pub font_id: u32,
    pub kind: Option<String>,
    pub alignment: Option<String>,
}

const KINDS: &[&str] = &["halfwidthKatakana", "fullwidthKatakana", "Hiragana", "noConversion"];
const ALIGNMENTS: &[&str] = &["noControl", "left", "center", "distributed"];

/// `<t>`, with `xml:space="preserve"` when the text has edge whitespace or a line break, and
/// the text in `_xHHHH_` form ([`encode_xstring`]).
pub fn t_el(text: &str) -> El {
    let t = El::new("t");
    let ws = [' ', '\t', '\n', '\r'];
    let t = if text.starts_with(ws) || text.ends_with(ws) || text.contains('\n') { t.with_attr("xml:space", "preserve") } else { t };
    if text.is_empty() { t } else { t.with_text(encode_xstring(text)) }
}

/// One string item: a shared string (`<si>`), an inline string (`<is>`) or comment text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StringItem {
    /// The whole text, runs joined, phonetic readings left out.
    pub text: Arc<str>,
    /// The formatted runs, empty for a plain string.
    pub runs: Vec<TextRun>,
    pub phonetic_runs: Vec<PhoneticRun>,
    pub phonetic: Option<PhoneticProps>,
    /// `<rPh>` elements that were dropped: no offsets, `sb >= eb`, past the end of the text, or
    /// no reading.
    pub dropped_phonetic_runs: usize,
}

impl StringItem {
    pub(crate) fn read(e: &El) -> StringItem {
        let text = rich_text(e);
        let runs = e
            .children("r")
            .map(|r| TextRun { text: decode_xstring(&r.children("t").map(|t| t.text()).collect::<String>()), font: r.child("rPr").map(Font::read) })
            .collect();
        // The length in UTF-16 code units: the widest reading of "the total length of the base
        // text", so a run is only dropped when it cannot refer to the text at all.
        let len = u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX);
        let mut phonetic_runs = Vec::new();
        let mut seen = 0usize;
        for r in e.children("rPh") {
            seen += 1;
            let (Some(sb), Some(eb)) = (r.attr_u32("sb"), r.attr_u32("eb")) else { continue };
            let (start, end) = (sb.min(len), eb.min(len));
            let reading = r.child("t").map(|t| decode_xstring(&t.text())).unwrap_or_default();
            if start < end && !reading.is_empty() {
                phonetic_runs.push(PhoneticRun { start, end, text: reading });
            }
        }
        let phonetic = e.child("phoneticPr").map(|p| PhoneticProps {
            font_id: p.attr_u32("fontId").unwrap_or(0),
            kind: p.attr("type").map(str::trim).filter(|v| KINDS.contains(v)).map(str::to_string),
            alignment: p.attr("alignment").map(str::trim).filter(|v| ALIGNMENTS.contains(v)).map(str::to_string),
        });
        StringItem { dropped_phonetic_runs: seen - phonetic_runs.len(), text: Arc::from(text), runs, phonetic_runs, phonetic }
    }
}

impl StringItem {
    /// The item as `<name>`: `si` in the shared strings, `is` in a cell, `text` in a comment.
    /// The formatted runs when there are any, else the text; then the phonetic runs and
    /// properties, checked again as the reader checks them (a run inside the text with a
    /// reading, a kind and alignment the schema defines), so what is written is valid.
    pub fn to_el(&self, name: &str) -> El {
        let mut e = El::new(name);
        if self.runs.is_empty() {
            e.push(t_el(&self.text));
        }
        for r in &self.runs {
            let mut run = El::new("r");
            if let Some(f) = &r.font {
                run.push(f.to_el("rPr", true));
            }
            run.push(t_el(&r.text));
            e.push(run);
        }
        let len = u32::try_from(self.text.encode_utf16().count()).unwrap_or(u32::MAX);
        for r in &self.phonetic_runs {
            let (start, end) = (r.start.min(len), r.end.min(len));
            if start < end && !r.text.is_empty() {
                e.push(El::new("rPh").with_attr("sb", start).with_attr("eb", end).with_child(t_el(&r.text)));
            }
        }
        if let Some(p) = &self.phonetic {
            let mut pr = El::new("phoneticPr").with_attr("fontId", p.font_id);
            if let Some(k) = p.kind.as_deref().filter(|k| KINDS.contains(k)) {
                pr.set_attr("type", k);
            }
            if let Some(a) = p.alignment.as_deref().filter(|a| ALIGNMENTS.contains(a)) {
                pr.set_attr("alignment", a);
            }
            e.push(pr);
        }
        e
    }

    /// A plain string.
    pub fn plain(text: &str) -> StringItem {
        StringItem { text: Arc::from(text), ..StringItem::default() }
    }
}

/// The text of a string item: `<t>` and the `<t>` of each `<r>`, in order, with `_xHHHH_`
/// escapes decoded. Phonetic runs are not part of the text.
pub fn rich_text(e: &El) -> String {
    let mut s = String::new();
    for c in e.elements() {
        match c.name.as_str() {
            "t" => s.push_str(&c.text()),
            "r" => c.children("t").for_each(|t| s.push_str(&t.text())),
            _ => {}
        }
    }
    decode_xstring(&s)
}

fn is_escape(chars: &[char], i: usize) -> bool {
    chars.get(i) == Some(&'_')
        && chars.get(i + 1) == Some(&'x')
        && chars.get(i + 6) == Some(&'_')
        && (i + 2..i + 6).all(|k| chars.get(k).is_some_and(char::is_ascii_hexdigit))
}

fn escape_unit(chars: &[char], i: usize) -> Option<u32> {
    let hex: String = chars.get(i + 2..i + 6)?.iter().collect();
    u32::from_str_radix(&hex, 16).ok()
}

/// Text for a `<t>` or `<v>`: characters XML cannot carry and control characters other than tab
/// and line feed become `_xHHHH_`, and a literal `_xHHHH_` is protected with `_x005F_`. The
/// result still needs XML escaping.
pub fn encode_xstring(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut o = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if is_escape(&chars, i) {
            o.push_str("_x005F_");
        } else if !craft_xml::is_xml_char(c) || (c.is_control() && !matches!(c, '\t' | '\n')) {
            o.push_str(&format!("_x{:04X}_", c as u32 & 0xFFFF));
        } else {
            o.push(c);
        }
    }
    o
}

/// The inverse of [`encode_xstring`]. Two escapes that form a UTF-16 surrogate pair are one
/// character; an escape that is no character on its own stays as written.
pub fn decode_xstring(s: &str) -> String {
    if !s.contains("_x") {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut o = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if is_escape(&chars, i) {
            let unit = escape_unit(&chars, i);
            if let Some(ch) = unit.and_then(char::from_u32) {
                o.push(ch);
                i += 7;
                continue;
            }
            let pair = unit.filter(|h| (0xD800..=0xDBFF).contains(h) && is_escape(&chars, i + 7)).and_then(|high| {
                let low = escape_unit(&chars, i + 7).filter(|l| (0xDC00..=0xDFFF).contains(l))?;
                char::from_u32(0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00))
            });
            if let Some(ch) = pair {
                o.push(ch);
                i += 14;
                continue;
            }
        }
        o.push(c);
        i += 1;
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xstring() {
        assert_eq!(decode_xstring("a_x000D_b"), "a\rb");
        assert_eq!(encode_xstring("a\rb"), "a_x000D_b");
        assert_eq!(encode_xstring("_x000D_"), "_x005F_x000D_");
        assert_eq!(decode_xstring("_x005F_x000D_"), "_x000D_");
        assert_eq!(decode_xstring("_x12"), "_x12");
        assert_eq!(decode_xstring("_xD83E__xDD80_"), "\u{1F980}");
        // A lone surrogate is no character: kept as written.
        assert_eq!(decode_xstring("_xD83E_!"), "_xD83E_!");
        for s in ["plain", "tab\tline\n", "\u{1}\u{1F}", "_x0041_ _xZZZZ_ é"] {
            assert_eq!(decode_xstring(&encode_xstring(s)), s);
        }
    }

    fn item(xml: &str) -> StringItem {
        StringItem::read(&craft_xml::Options::local().parse(xml.as_bytes()).unwrap_or_default())
    }

    #[test]
    fn runs_and_phonetics() {
        let si = item(
            r#"<si><r><t>東</t></r><r><rPr><b/><rFont val="Arial"/><sz val="9"/></rPr><t xml:space="preserve">京 </t></r><rPh sb="0" eb="2"><t>トウキョウ</t></rPh><rPh sb="5" eb="9"><t>x</t></rPh><rPh sb="1" eb="1"><t>x</t></rPh><rPh sb="0" eb="1"/><phoneticPr fontId="3" type="Hiragana" alignment="sideways"/></si>"#,
        );
        assert_eq!(&*si.text, "東京 ");
        assert_eq!(si.runs.len(), 2);
        assert_eq!(si.runs[1].font.as_ref().map(|f| (f.bold, f.name.as_deref(), f.size)), Some((Some(true), Some("Arial"), Some(9.0))));
        assert_eq!(si.phonetic_runs, [PhoneticRun { start: 0, end: 2, text: "トウキョウ".into() }]);
        assert_eq!(si.dropped_phonetic_runs, 3);
        assert_eq!(si.phonetic, Some(PhoneticProps { font_id: 3, kind: Some("Hiragana".into()), alignment: None }));
        let plain = item("<si><t>a_x000A_b</t></si>");
        assert_eq!((&*plain.text, plain.runs.len()), ("a\nb", 0));
        // Written back: the dropped runs stay dropped, the bad alignment is left out.
        let again = item(&si.to_el("si").to_xml());
        assert_eq!((&again.text, &again.runs, &again.phonetic_runs, again.dropped_phonetic_runs), (&si.text, &si.runs, &si.phonetic_runs, 0));
        assert_eq!(again.phonetic, si.phonetic);
        assert_eq!(item(&plain.to_el("si").to_xml()), plain);
        assert_eq!(t_el(" a\r").to_xml(), r#"<t xml:space="preserve"> a_x000D_</t>"#);
        // A run that does not fit the text is not written.
        let bad = StringItem { phonetic_runs: vec![PhoneticRun { start: 3, end: 9, text: "x".into() }], ..StringItem::plain("ab") };
        assert_eq!(bad.to_el("si").to_xml(), "<si><t>ab</t></si>");
    }
}
