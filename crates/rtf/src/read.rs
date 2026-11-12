//! RTF into the model.

use std::collections::HashMap;

use crate::codepage::{MAC_ROMAN_CODE_PAGE, SYMBOL_CODE_PAGE, charset_code_page, is_text_code_page};
use crate::lex::{Token, Tokens, is_rtf};
use crate::model::{Align, Block, COLUMN_BREAK, Cell, CharFmt, Document, LINE_BREAK, PAGE_BREAK, Para, Run, Table, VertAlign};
use crate::text::TextDecoder;

/// Deepest group nesting followed. Deeper groups are read in the state of the group around
/// them.
pub const MAX_GROUPS: usize = 512;
/// Most fonts kept from the font table.
pub const MAX_FONTS: usize = 4096;
/// Most colours kept from the colour table.
pub const MAX_COLORS: usize = 4096;
/// Longest font name kept, in bytes. A longer one is no font.
pub const MAX_FONT_NAME: usize = 256;

/// The file does not start with `{\rtf`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotRtf;

impl std::fmt::Display for NotRtf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not an RTF document (it doesn't start with {\\rtf)")
    }
}

impl std::error::Error for NotRtf {}

/// Destinations whose text is not body text. Any destination marked `\*` is skipped too.
const SKIP: [&str; 30] = [
    "stylesheet",
    "info",
    "header",
    "headerl",
    "headerr",
    "headerf",
    "footer",
    "footerl",
    "footerr",
    "footerf",
    "footnote",
    "listtable",
    "listoverridetable",
    "rsidtbl",
    "generator",
    "themedata",
    "colorschememapping",
    "latentstyles",
    "datastore",
    "xmlnstbl",
    "fldinst",
    "revtbl",
    "filetbl",
    "annotation",
    "bkmkstart",
    "bkmkend",
    // A list item's rendered bullet or number, written before its text (Word: listtext; older
    // writers: pntext and its parts). Its `\pard` must not reset the paragraph's own indent.
    "listtext",
    "pntext",
    "pntxta",
    "pntxtb",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Dest {
    #[default]
    Body,
    Skip,
    FontTbl,
    ColorTbl,
}

/// What a group saves and restores.
#[derive(Clone, Debug)]
struct St {
    dest: Dest,
    fmt: CharFmt,
    /// The code page of the current font, when its charset names one.
    cp: Option<u16>,
    /// A font was selected (`\f`) in this group or an enclosing one, and no `\plain` has since
    /// made the default font current again.
    font_set: bool,
    uc: usize,
    /// The paragraph properties in effect; their `runs` stay empty.
    para: Para,
    in_table: bool,
}

impl Default for St {
    fn default() -> St {
        St { dest: Dest::Body, fmt: CharFmt::default(), cp: None, font_set: false, uc: 1, para: Para::default(), in_table: false }
    }
}

#[derive(Default)]
struct Reader {
    st: St,
    stack: Vec<St>,
    /// Groups open beyond [`MAX_GROUPS`].
    overflow: usize,
    decoder: TextDecoder,
    /// `\ansicpg`, `\mac`, `\pc`, `\pca`; 0 until the file names one (Windows-1252 then).
    ansicpg: u16,
    deff: i32,
    fonts: HashMap<i32, String>,
    font_cps: HashMap<i32, u16>,
    font_cur: i32,
    font_cp_cur: Option<u16>,
    /// The font table entry being read named its code page with `\cpg`.
    font_cpg_named: bool,
    font_name: String,
    /// Where the text of a destination that keeps none goes.
    scrap: String,
    colors: Vec<Option<[u8; 3]>>,
    color_cur: ([u8; 3], bool),
    doc: Document,
    para: Para,
    text: String,
    rows: Vec<Vec<Cell>>,
    row: Vec<Cell>,
    cell: Vec<Para>,
    cellx: Vec<i32>,
}

/// Reads an RTF file.
///
/// Kept: body text in the file's code pages (`\ansicpg`, `\mac`, `\pc`, `\pca`, a font's
/// `\fcharset` or `\cpg`) and `\uN`; paragraphs (`\par`, `\sect`) with `\outlinelevel` and
/// `\li`, `\ri`, `\fi`, `\sb`, `\sa` and alignment; bold, italic, underline, strikethrough,
/// superscript and subscript, size, colour and font; tabs, line, page and
/// column breaks, the quote, dash and bullet words; tables (`\cellx`, `\cell`, `\row`).
///
/// Left out: headers, footers, footnotes, comments, field instructions (a field's result is
/// kept), list labels, the stylesheet, pictures and objects (counted in
/// [`Document::pictures_left_out`]), and every destination marked `\*`. A table nested in a
/// cell reads as that cell's text.
///
/// A trailing `\par` does not leave an empty last paragraph. Never fails on a file that starts
/// with `{\rtf`: what cannot be read is skipped.
pub fn read(bytes: &[u8]) -> Result<Document, NotRtf> {
    if !is_rtf(bytes) {
        return Err(NotRtf);
    }
    let mut r = Reader::default();
    let mut starred = false;
    for token in Tokens::new(bytes) {
        let star = std::mem::take(&mut starred);
        match token {
            Token::Open => r.open(),
            Token::Close => r.close(),
            Token::Star => starred = true,
            Token::Word { name, param } => r.word(name, param, star),
            Token::Hex(b) => r.hex(b),
            Token::Symbol(s) => r.symbol(s),
            Token::Text(t) => r.bytes(t),
            Token::Bin(_) => {}
        }
    }
    Ok(r.finish())
}

impl Reader {
    /// The decoder, and where text goes in the current destination.
    fn sink(&mut self) -> (&mut TextDecoder, &mut String) {
        self.scrap.clear();
        let out = match self.st.dest {
            Dest::Body => &mut self.text,
            Dest::FontTbl => &mut self.font_name,
            Dest::Skip | Dest::ColorTbl => &mut self.scrap,
        };
        (&mut self.decoder, out)
    }

    fn open(&mut self) {
        let (decoder, out) = self.sink();
        decoder.group(out);
        self.flush_run();
        if self.stack.len() >= MAX_GROUPS {
            self.overflow += 1;
            return;
        }
        self.stack.push(self.st.clone());
    }

    fn close(&mut self) {
        let (decoder, out) = self.sink();
        decoder.group(out);
        if self.overflow > 0 {
            self.overflow -= 1;
            return;
        }
        if self.st.dest == Dest::FontTbl {
            self.commit_font();
        }
        self.flush_run();
        if let Some(outer) = self.stack.pop() {
            self.st = outer;
        }
    }

    /// The code page 8-bit text is read in: the current font's, else the default font's, else
    /// the document's (Windows-1252 when none is named). The symbol page is for body text set in
    /// a symbol font, so it never applies outside the body (names in the font table), and a
    /// symbol default font's page does not apply to text in a font selected with `\f`.
    fn code_page(&self) -> u16 {
        let document = if self.ansicpg == 0 { 1252 } else { self.ansicpg };
        let body = self.st.dest == Dest::Body;
        let current = self.st.cp.filter(|&cp| cp != SYMBOL_CODE_PAGE || body);
        let default = self.font_cps.get(&self.deff).copied().filter(|&cp| cp != SYMBOL_CODE_PAGE || (body && !self.st.font_set));
        current.or(default).unwrap_or(document)
    }

    fn bytes(&mut self, chunk: &[u8]) {
        let cp = self.code_page();
        match self.st.dest {
            Dest::Body => self.decoder.bytes(chunk, cp, &mut self.text),
            Dest::FontTbl => {
                // `;` ends an entry; a name may be split around it by formatting.
                for (i, part) in chunk.split(|b| *b == b';').enumerate() {
                    if i > 0 {
                        self.commit_font();
                    }
                    self.decoder.bytes(part, cp, &mut self.font_name);
                }
            }
            Dest::ColorTbl => {
                for _ in chunk.iter().filter(|b| **b == b';') {
                    self.commit_color();
                }
            }
            Dest::Skip => {}
        }
    }

    fn hex(&mut self, b: u8) {
        let cp = self.code_page();
        // In a destination that keeps no text the byte still uses up a fallback position.
        let (decoder, out) = self.sink();
        decoder.byte(b, cp, out);
    }

    fn ch(&mut self, c: char) {
        let (decoder, out) = self.sink();
        decoder.char(c, out);
    }

    fn symbol(&mut self, s: u8) {
        if self.decoder.skip() {
            return;
        }
        match s {
            b'~' => self.ch('\u{00A0}'),
            b'-' => self.ch('\u{00AD}'),
            b'_' => self.ch('\u{2011}'),
            b'\\' | b'{' | b'}' => self.ch(s as char),
            b'\n' if self.st.dest == Dest::Body => self.end_para(),
            _ => {}
        }
    }

    fn commit_font(&mut self) {
        let name = std::mem::take(&mut self.font_name);
        let name = name.trim();
        let cp = self.font_cp_cur.take();
        if name.is_empty() || name.len() > MAX_FONT_NAME || (self.fonts.len() >= MAX_FONTS && !self.fonts.contains_key(&self.font_cur)) {
            return;
        }
        self.fonts.insert(self.font_cur, name.to_string());
        if let Some(cp) = cp {
            self.font_cps.insert(self.font_cur, cp);
        }
    }

    fn commit_color(&mut self) {
        let (rgb, set) = std::mem::take(&mut self.color_cur);
        if self.colors.len() < MAX_COLORS {
            self.colors.push(set.then_some(rgb));
        }
    }

    /// Ends the run being read: its text takes the formatting in effect now.
    fn flush_run(&mut self) {
        if self.text.is_empty() {
            return;
        }
        let mut text = std::mem::take(&mut self.text);
        // `\u0` and `\'00` are not text.
        text.retain(|c| c != '\0');
        if text.is_empty() {
            return;
        }
        match self.para.runs.last_mut() {
            Some(last) if last.fmt == self.st.fmt => last.text.push_str(&text),
            _ => self.para.runs.push(Run { text, fmt: self.st.fmt.clone() }),
        }
    }

    fn take_para(&mut self) -> Para {
        self.decoder.finish(&mut self.text);
        self.flush_run();
        let runs = std::mem::take(&mut self.para.runs);
        Para { runs, ..self.st.para.clone() }
    }

    /// The rows read so far become a table: something that is not a row follows them.
    fn end_table(&mut self) {
        if !self.rows.is_empty() {
            self.doc.blocks.push(Block::Table(Table { rows: std::mem::take(&mut self.rows) }));
        }
    }

    fn end_para(&mut self) {
        let para = self.take_para();
        if self.st.in_table || !self.cell.is_empty() {
            self.cell.push(para);
        } else {
            self.end_table();
            self.doc.blocks.push(Block::Para(para));
        }
    }

    fn end_cell(&mut self) {
        let para = self.take_para();
        if !para.runs.is_empty() || self.cell.is_empty() {
            self.cell.push(para);
        }
        self.row.push(Cell { right: 0, paras: std::mem::take(&mut self.cell) });
    }

    fn end_row(&mut self) {
        let mut row = std::mem::take(&mut self.row);
        for (cell, right) in row.iter_mut().zip(&self.cellx) {
            cell.right = *right;
        }
        if !row.is_empty() {
            self.rows.push(row);
        }
    }

    /// Changes the character formatting: the text read so far keeps the old one.
    fn restyle(&mut self, f: impl FnOnce(&mut CharFmt)) {
        self.flush_run();
        f(&mut self.st.fmt);
    }

    fn word(&mut self, name: &str, param: Option<i32>, starred: bool) {
        if name == "u" {
            if let Some(n) = param {
                let uc = self.st.uc;
                let (decoder, out) = self.sink();
                decoder.unicode(n, uc, out);
            }
            return;
        }
        if self.decoder.skip() {
            return;
        }
        if self.destination(name, starred) {
            return;
        }
        let pv = param.unwrap_or(0);
        match self.st.dest {
            Dest::Skip => {}
            Dest::ColorTbl => {
                let v = pv.clamp(0, 255) as u8;
                match name {
                    "red" => self.color_cur = ([v, self.color_cur.0[1], self.color_cur.0[2]], true),
                    "green" => self.color_cur = ([self.color_cur.0[0], v, self.color_cur.0[2]], true),
                    "blue" => self.color_cur = ([self.color_cur.0[0], self.color_cur.0[1], v], true),
                    _ => {}
                }
            }
            Dest::FontTbl => match name {
                "f" => {
                    self.font_cur = pv;
                    self.font_cpg_named = false;
                }
                "fcharset" => {
                    // The symbol charset doesn't replace a code page the entry named with `\cpg`.
                    let cp = charset_code_page(pv);
                    if !(self.font_cpg_named && cp == Some(SYMBOL_CODE_PAGE)) {
                        self.font_cp_cur = cp;
                    }
                }
                "cpg" => {
                    if let Some(cp) = u16::try_from(pv).ok().filter(|cp| is_text_code_page(*cp)) {
                        self.font_cp_cur = Some(cp);
                        self.font_cpg_named = true;
                    }
                }
                _ => {}
            },
            Dest::Body => self.body_word(name, param),
        }
    }

    /// Does `name` start a destination? Then the group's text goes there.
    fn destination(&mut self, name: &str, starred: bool) -> bool {
        let dest = match name {
            "fonttbl" => Dest::FontTbl,
            "colortbl" => Dest::ColorTbl,
            "pict" | "object" => {
                if self.st.dest != Dest::Skip {
                    self.doc.pictures_left_out += 1;
                }
                Dest::Skip
            }
            // A field's result is its text; `\*\fldrslt` is not a destination to drop.
            "fldrslt" => return true,
            _ if SKIP.contains(&name) => Dest::Skip,
            _ if starred => Dest::Skip,
            _ => return false,
        };
        self.flush_run();
        // Nothing inside a skipped destination comes back out of it.
        if self.st.dest != Dest::Skip {
            self.st.dest = dest;
        }
        true
    }

    fn body_word(&mut self, name: &str, param: Option<i32>) {
        let pv = param.unwrap_or(0);
        let on = param != Some(0);
        match name {
            "par" | "sect" => self.end_para(),
            "cell" => self.end_cell(),
            "row" => self.end_row(),
            "trowd" => self.cellx.clear(),
            "cellx" => self.cellx.push(pv),
            "intbl" => self.st.in_table = true,
            "pard" => (self.st.para, self.st.in_table) = (Para::default(), false),
            "outlinelevel" => self.st.para.outline_level = u8::try_from(pv).ok().filter(|l| *l <= 8),
            "li" => self.st.para.left_indent = pv,
            "ri" => self.st.para.right_indent = pv,
            "fi" => self.st.para.first_line_indent = pv,
            "sb" => self.st.para.space_before = pv,
            "sa" => self.st.para.space_after = pv,
            "ql" => self.st.para.align = Some(Align::Left),
            "qc" => self.st.para.align = Some(Align::Center),
            "qr" => self.st.para.align = Some(Align::Right),
            "qj" => self.st.para.align = Some(Align::Justified),
            "line" => self.ch(LINE_BREAK),
            "tab" => self.ch('\t'),
            "page" => self.ch(PAGE_BREAK),
            "column" => self.ch(COLUMN_BREAK),
            "emdash" => self.ch('\u{2014}'),
            "endash" => self.ch('\u{2013}'),
            "bullet" => self.ch('\u{2022}'),
            "lquote" => self.ch('\u{2018}'),
            "rquote" => self.ch('\u{2019}'),
            "ldblquote" => self.ch('\u{201C}'),
            "rdblquote" => self.ch('\u{201D}'),
            "emspace" => self.ch('\u{2003}'),
            "enspace" => self.ch('\u{2002}'),
            "b" => self.restyle(|f| f.bold = on),
            "i" => self.restyle(|f| f.italic = on),
            "ul" => self.restyle(|f| f.underline = on),
            "ulnone" => self.restyle(|f| f.underline = false),
            "strike" => self.restyle(|f| f.strike = on),
            "super" => self.restyle(|f| f.vert_align = if on { VertAlign::Superscript } else { VertAlign::Baseline }),
            "sub" => self.restyle(|f| f.vert_align = if on { VertAlign::Subscript } else { VertAlign::Baseline }),
            "nosupersub" => self.restyle(|f| f.vert_align = VertAlign::Baseline),
            "fs" => self.restyle(|f| f.half_points = u32::try_from(pv).ok().filter(|s| *s > 0)),
            "cf" => {
                let color = usize::try_from(pv).ok().and_then(|i| self.colors.get(i).copied().flatten());
                self.restyle(|f| f.color = color);
            }
            "f" => {
                let font = self.fonts.get(&pv).cloned();
                self.st.cp = self.font_cps.get(&pv).copied();
                self.st.font_set = true;
                self.restyle(|f| f.font = font);
            }
            "plain" => {
                // The default font is current again, and its code page with it.
                self.st.cp = None;
                self.st.font_set = false;
                self.restyle(|f| *f = CharFmt::default());
            }
            "uc" => self.st.uc = usize::try_from(pv).unwrap_or(0),
            "ansicpg" => {
                if let Some(cp) = u16::try_from(pv).ok().filter(|cp| is_text_code_page(*cp)) {
                    self.ansicpg = cp;
                }
            }
            "mac" if self.ansicpg == 0 => self.ansicpg = MAC_ROMAN_CODE_PAGE,
            "pc" if self.ansicpg == 0 => self.ansicpg = 437,
            "pca" if self.ansicpg == 0 => self.ansicpg = 850,
            "deff" => self.deff = pv,
            _ => {}
        }
    }

    fn finish(mut self) -> Document {
        let last = self.take_para();
        // A file cut inside a row keeps what the row had.
        if !self.row.is_empty() || !self.cell.is_empty() {
            if !last.runs.is_empty() || !self.cell.is_empty() {
                self.cell.push(last);
                self.row.push(Cell { right: 0, paras: std::mem::take(&mut self.cell) });
            }
            self.end_row();
            self.end_table();
        } else {
            self.end_table();
            if !last.runs.is_empty() {
                self.doc.blocks.push(Block::Para(last));
            }
        }
        self.doc
    }
}
