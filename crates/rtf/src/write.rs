//! The model as RTF, and the escaping every RTF writer needs.

use std::fmt::Write;

use crate::model::{Align, Block, COLUMN_BREAK, CharFmt, Document, LINE_BREAK, PAGE_BREAK, Para, VertAlign};

/// RTF-escapes text: `\`, `{` and `}` escaped; tab, line, page and column breaks, the
/// non-breaking space, the soft hyphen and the non-breaking hyphen as their control words;
/// everything past ASCII as `\uN?` (UTF-16 code units as signed 16-bit numbers, with a `?`
/// fallback, so the file needs `\uc1`, the default). Other ASCII control characters are
/// dropped. U+000B and U+2028 are line breaks too.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    push_esc(&mut o, s);
    o
}

/// [`esc`], appended to `o`.
pub fn push_esc(o: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '{' => o.push_str("\\{"),
            '}' => o.push_str("\\}"),
            '\t' => o.push_str("\\tab "),
            LINE_BREAK | '\u{000B}' | '\u{2028}' => o.push_str("\\line "),
            PAGE_BREAK => o.push_str("\\page "),
            COLUMN_BREAK => o.push_str("\\column "),
            '\u{00A0}' => o.push_str("\\~"),
            '\u{00AD}' => o.push_str("\\-"),
            '\u{2011}' => o.push_str("\\_"),
            c if c.is_ascii_control() => {}
            c if c.is_ascii() => o.push(c),
            c => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    // Writing to a String never fails.
                    let _ = write!(o, "\\u{}?", *u as i16);
                }
            }
        }
    }
}

/// [`esc`] for a `\fonttbl` name: `;` would end the entry, so it is written `\'3b`, and control
/// characters, which [`esc`] writes as `\line` or `\tab`, are dropped.
pub fn esc_font(name: &str) -> String {
    esc(&name.replace(char::is_control, "")).replace(';', "\\'3b")
}

/// The fonts and colours a document uses, in the order of their tables. Font 0 is the default
/// font; colour 0 is the default (auto) colour, so the first real colour is `\cf1`.
struct Tables {
    fonts: Vec<String>,
    colors: Vec<[u8; 3]>,
}

impl Tables {
    fn of(doc: &Document, default_font: &str) -> Tables {
        let mut t = Tables { fonts: vec![default_font.to_string()], colors: Vec::new() };
        for fmt in doc.paras().flat_map(|p| &p.runs).map(|r| &r.fmt) {
            if let Some(f) = fmt.font.as_deref().filter(|f| t.font(f).is_none()) {
                t.fonts.push(f.to_string());
            }
            if let Some(c) = fmt.color.filter(|c| t.color(*c).is_none()) {
                t.colors.push(c);
            }
        }
        t
    }
    fn font(&self, name: &str) -> Option<usize> {
        self.fonts.iter().position(|f| f.eq_ignore_ascii_case(name))
    }
    fn color(&self, c: [u8; 3]) -> Option<usize> {
        self.colors.iter().position(|k| *k == c).map(|i| i + 1)
    }

    fn fmt_words(&self, f: &CharFmt) -> String {
        let mut w = String::new();
        for (on, word) in [(f.bold, "\\b"), (f.italic, "\\i"), (f.underline, "\\ul"), (f.strike, "\\strike")] {
            if on {
                w.push_str(word);
            }
        }
        match f.vert_align {
            VertAlign::Superscript => w.push_str("\\super"),
            VertAlign::Subscript => w.push_str("\\sub"),
            VertAlign::Baseline => {}
        }
        // Writing to a String never fails.
        if let Some(i) = f.font.as_deref().and_then(|n| self.font(n)).filter(|i| *i > 0) {
            let _ = write!(w, "\\f{i}");
        }
        if let Some(s) = f.half_points {
            let _ = write!(w, "\\fs{s}");
        }
        if let Some(i) = f.color.and_then(|c| self.color(c)) {
            let _ = write!(w, "\\cf{i}");
        }
        w
    }

    /// A paragraph's runs: a formatted run is a group, so its formatting ends with it.
    fn runs(&self, p: &Para, o: &mut String) {
        for r in &p.runs {
            let words = self.fmt_words(&r.fmt);
            if words.is_empty() {
                push_esc(o, &r.text);
            } else {
                o.push('{');
                o.push_str(&words);
                o.push(' ');
                push_esc(o, &r.text);
                o.push('}');
            }
        }
    }

    fn para_words(p: &Para, o: &mut String) {
        if let Some(l) = p.outline_level.filter(|l| *l <= 8) {
            let _ = write!(o, "\\outlinelevel{l}");
        }
        match p.align {
            Some(Align::Left) => o.push_str("\\ql"),
            Some(Align::Center) => o.push_str("\\qc"),
            Some(Align::Right) => o.push_str("\\qr"),
            Some(Align::Justified) => o.push_str("\\qj"),
            None => {}
        }
        for (word, twips) in
            [("li", p.left_indent), ("ri", p.right_indent), ("fi", p.first_line_indent), ("sb", p.space_before), ("sa", p.space_after)]
        {
            if twips != 0 {
                let _ = write!(o, "\\{word}{twips}");
            }
        }
    }
}

/// Writes a document as RTF. `default_font` is the font of text that names none.
///
/// The file is 7-bit: `\ansi\ansicpg1252\uc1`, a font table, a colour table when a run has a
/// colour, then one `\pard … \par` per paragraph and `\trowd … \row` per table row. What
/// [`read`](crate::read) reads from it is `doc` again, except that
/// [`Document::pictures_left_out`] is 0, a font equal to `default_font` reads as no font,
/// neighbouring runs of one formatting are one run, and empty runs, an empty last paragraph of a
/// cell and rows without cells are gone.
pub fn write(doc: &Document, default_font: &str) -> String {
    let t = Tables::of(doc, default_font);
    let mut o = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1{\\fonttbl");
    for (i, f) in t.fonts.iter().enumerate() {
        let _ = write!(o, "{{\\f{i}\\fnil {};}}", esc_font(f));
    }
    o.push_str("}\n");
    if !t.colors.is_empty() {
        o.push_str("{\\colortbl;");
        for [r, g, b] in &t.colors {
            let _ = write!(o, "\\red{r}\\green{g}\\blue{b};");
        }
        o.push_str("}\n");
    }
    for block in &doc.blocks {
        match block {
            Block::Para(p) => {
                o.push_str("\\pard");
                Tables::para_words(p, &mut o);
                o.push(' ');
                t.runs(p, &mut o);
                o.push_str("\\par\n");
            }
            Block::Table(table) => {
                for row in table.rows.iter().filter(|r| !r.is_empty()) {
                    o.push_str("\\trowd\\trgaph108");
                    for cell in row {
                        let _ = write!(o, "\\cellx{}", cell.right);
                    }
                    for cell in row {
                        for (i, p) in cell.paras.iter().enumerate() {
                            if i > 0 {
                                o.push_str("\\par");
                            }
                            o.push_str("\\pard\\intbl");
                            Tables::para_words(p, &mut o);
                            o.push(' ');
                            t.runs(p, &mut o);
                        }
                        o.push_str("\\cell");
                    }
                    o.push_str("\\row\n");
                }
                // The paragraph after a table is not in it.
                o.push_str("\\pard");
            }
        }
    }
    o.push('}');
    o
}
