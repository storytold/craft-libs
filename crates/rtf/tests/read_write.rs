//! The reader and writer against the cases the apps' RTF code was tested with: DesignCraft's
//! text import, DeckCraft's outline import and export, PdfCraft's export, and WordCraft's
//! reader for code pages and `\u`.

use craft_rtf::{
    Align, Block, COLUMN_BREAK, Cell, CharFmt, Document, LINE_BREAK, MAX_COLORS, MAX_FONT_NAME, MAX_FONTS, MAX_GROUPS, NotRtf, PAGE_BREAK, Para, Run,
    Table, VertAlign, read, write,
};

fn text(rtf: &[u8]) -> String {
    read(rtf).unwrap().paras().map(Para::text).collect::<Vec<_>>().join("\n")
}

fn paras(rtf: &[u8]) -> Vec<Para> {
    read(rtf).unwrap().paras().cloned().collect()
}

/// The formatting of the run that holds `needle`.
fn fmt_of(doc: &Document, needle: &str) -> CharFmt {
    doc.paras().flat_map(|p| &p.runs).find(|r| r.text.contains(needle)).map(|r| r.fmt.clone()).unwrap()
}

fn para(s: &str) -> Para {
    Para { runs: vec![Run { text: s.into(), fmt: CharFmt::default() }], ..Para::default() }
}

#[test]
fn not_rtf_is_refused() {
    assert_eq!(read(b"PK\x03\x04"), Err(NotRtf));
    assert_eq!(read(b""), Err(NotRtf));
    assert!(read(b"  \n{\\rtf1 a}").is_ok());
    assert!(NotRtf.to_string().contains("{\\rtf"));
}

#[test]
fn text_and_formatting() {
    let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Times;}}{\colortbl;\red0\green0\blue0;}{\*\generator Test;}
\f0\fs24 Hello {\b bold} and {\i italic\i0  plain}\par
Caf\'e9 \u8364? price\tab tab\line next\par
}";
    let doc = read(rtf).unwrap();
    assert_eq!(text(rtf), "Hello bold and italic plain\nCafé € price\ttab\nnext");
    assert_eq!(doc.blocks.len(), 2, "a trailing \\par leaves no empty paragraph");
    assert!(fmt_of(&doc, "bold").bold && !fmt_of(&doc, "bold").italic);
    assert!(fmt_of(&doc, "italic").italic);
    let plain = fmt_of(&doc, " plain");
    assert!(!plain.italic && !plain.bold);
    assert_eq!(fmt_of(&doc, "Hello").half_points, Some(24));
    assert_eq!(fmt_of(&doc, "Hello").font.as_deref(), Some("Times"));
    assert!(!text(rtf).contains("Times") && !text(rtf).contains("Test"), "tables and starred destinations are not text");
}

#[test]
fn character_words() {
    let doc = read(br"{\rtf1{\colortbl;\red255\green0\blue0;\red0\green0\blue999;}\ul a\ulnone b\strike c\strike0 d\cf1 e\cf2 f\cf0 g\cf9 h\b\i\plain i\fs0 j\fs-8 k}").unwrap();
    assert!(fmt_of(&doc, "a").underline && !fmt_of(&doc, "b").underline);
    assert!(fmt_of(&doc, "c").strike && !fmt_of(&doc, "d").strike);
    assert_eq!(fmt_of(&doc, "e").color, Some([255, 0, 0]));
    assert_eq!(fmt_of(&doc, "f").color, Some([0, 0, 255]), "a component is clamped");
    assert_eq!(fmt_of(&doc, "g").color, None, "colour 0 is the default colour");
    assert_eq!(fmt_of(&doc, "h").color, None, "a colour the table does not have");
    assert_eq!(fmt_of(&doc, "i"), CharFmt::default(), "\\plain");
    assert_eq!((fmt_of(&doc, "j").half_points, fmt_of(&doc, "k").half_points), (None, None));
}

#[test]
fn special_characters_and_breaks() {
    assert_eq!(
        text(br"{\rtf1 a\~b\-c\_d\\e\{f\}g\emdash h\endash i\bullet j\lquote k\rquote l\ldblquote m\rdblquote}"),
        "a\u{A0}b\u{AD}c\u{2011}d\\e{f}g—h–i•j‘k’l“m”"
    );
    let breaks = format!("a{PAGE_BREAK}b{COLUMN_BREAK}c{LINE_BREAK}d\te");
    assert_eq!(text(br"{\rtf1 a\page b\column c\line d\tab e}"), breaks);
    // A backslash before a line end is a paragraph end; a bare line end is nothing.
    assert_eq!(text(b"{\\rtf1 a\\\nb\r\nc\\sect d}"), "a\nbc\nd");
}

#[test]
fn header_and_footer_variants_and_notes_are_skipped() {
    for w in ["header", "headerl", "headerr", "headerf", "footer", "footerl", "footerr", "footerf", "footnote", "annotation", "stylesheet", "info"] {
        let rtf = format!(r"{{\rtf1\ansi {{\{w} Page head\par}}Body text\par}}");
        assert_eq!(text(rtf.as_bytes()), "Body text", "{w}");
    }
    // Nothing inside a skipped destination comes back out of it, whatever it nests.
    assert_eq!(text(br"{\rtf1 {\header {\b {\fonttbl x} Head}}{\*\unknown {\field{\fldrslt hidden}}}Body}"), "Body");
}

#[test]
fn a_fields_result_is_its_text() {
    assert_eq!(text(br#"{\rtf1 See {\field{\*\fldinst HYPERLINK "https://example.com"}{\fldrslt {\ul the site}}}.}"#), "See the site.");
    assert_eq!(text(br"{\rtf1 {\field{\*\fldinst PAGE}{\*\fldrslt 3}} of 9}"), "3 of 9");
}

#[test]
fn pictures_and_objects_are_counted_not_read() {
    let doc = read(
        br"{\rtf1 a{\pict\pngblip 89504e47}b{\object\objemb{\*\objdata 0105}}{\*\shppict{\pict\pngblip 00}}{\nonshppict{\pict\wmetafile8 00}}c}",
    )
    .unwrap();
    assert_eq!(doc.pictures_left_out, 3, "a picture inside a skipped destination is not counted again");
    assert_eq!(doc.paras().map(Para::text).collect::<String>(), "abc");
    assert_eq!(text(b"{\\rtf1 a{\\pict\\bin4 }}{\\x}b}"), "ab", "binary data is not scanned for braces");
}

#[test]
fn out_of_range_unicode_wraps_and_hyphens_are_text() {
    assert_eq!(text(br"{\rtf1 a\u4294967361?b}"), "a\u{FFFF}b", "clamped to i32, then modulo 65536");
    assert_eq!(text(br"{\rtf1\ansi Rock\emdash-roll, x\tab-y, a\li-720 b}"), "Rock\u{2014}-roll, x\t-y, ab");
    assert_eq!(text(br"{\rtf1 {\b A\'4}B}"), "AB");
    assert_eq!(text(br"{\rtf1 a\'+fb}"), "a+fb");
}

#[test]
fn unicode_fallbacks() {
    assert_eq!(text(br"{\rtf1 \u8220\ldblquote Hi\u8221\rdblquote  x\u8212\emdash y}"), "\u{201C}Hi\u{201D} x\u{2014}y");
    assert_eq!(text(br"{\rtf1 \u8220\{Hi}"), "\u{201C}Hi", "a control symbol is one fallback character");
    assert_eq!(text(br"{\rtf1 \u8220\'93Hi}"), "\u{201C}Hi");
    assert_eq!(text(br"{\rtf1 \u8220{\b Hi}}"), "\u{201C}Hi", "a group is not a character");
    assert_eq!(text(br"{\rtf1 {\b \u8364} AB}"), "\u{20AC} AB", "the skip ends with the group");
    assert_eq!(text(br"{\rtf1 {\uc2\b \u8364} ABC}"), "\u{20AC} ABC");
    assert_eq!(text(br"{\rtf1\uc0 \u8364 5\uc2 \u8364xy5}"), "\u{20AC}5\u{20AC}5");
    assert_eq!(text(br"{\rtf1 \u8364\u8364?5}"), "\u{20AC}\u{20AC}5", "a \\u is never a fallback character");
    assert_eq!(text(br"{\rtf1 {\header \u8364}?x}"), "?x", "a \\u in a skipped destination leaves nothing to skip outside it");
}

#[test]
fn nul_is_not_text() {
    assert_eq!(text(br"{\rtf1 \u0?abc\par a\'00b\par {\b\'00}}"), "abc\nab");
    assert_eq!(text(b"{\\rtf1 a\0b\\par}\0\0"), "ab");
}

#[test]
fn surrogate_pairs() {
    assert_eq!(text(br"{\rtf1\ansi\uc1 A\u-10179?\u-8704?B \u-10174?\u-8265?\u30000?}"), "A\u{1F600}B \u{20BB7}\u{7530}");
    assert_eq!(text(br"{\rtf1 A\u-10179?B\u-8704?C}"), "A\u{FFFD}B\u{FFFD}C", "half a pair is U+FFFD, as in Word");
    assert_eq!(text(br"{\rtf1 A\u-10179?\par\u-8704?C}"), "A\u{FFFD}\n\u{FFFD}C", "a pair does not span a paragraph end");
    assert_eq!(text(br"{\rtf1 A\u-10179?}"), "A\u{FFFD}");
}

#[test]
fn code_pages() {
    assert_eq!(text(br"{\rtf1\ansi\ansicpg1251 \'cf\'f0\'e8\'e2\'e5\'f2}"), "Привет");
    assert_eq!(text(b"{\\rtf1\\ansi\\ansicpg1251 \xcf\xf0\xe8\xe2\xe5\xf2}"), "Привет", "raw bytes are text in the code page");
    assert_eq!(text(br"{\rtf1\mac Caf\'8e}"), "Café");
    assert_eq!(text(br"{\rtf1\pc Caf\'82}"), "Café");
    assert_eq!(text(br"{\rtf1\pca \'9d}"), "Ø");
    assert_eq!(text(br"{\rtf1\ansi Caf\'e9 \'80}"), "Café €", "Windows-1252 when the file names none");
    assert_eq!(text(br"{\rtf1\ansi\ansicpg9999 Caf\'e9}"), "Café", "an unknown code page is not taken");
    // The font's charset decides, then the default font's, then the document's.
    let rtf = br"{\rtf1\ansi\ansicpg1252\deff1{\fonttbl{\f0\fcharset204 Arial Cyr;}{\f1\fcharset161 Greek;}{\f2\cpg1250 CE;}{\f3 Plain;}}\'e1 \f0\'e1 \f2\'b3 \f3\'e1 \plain\'e1}";
    assert_eq!(text(rtf), "α б ł α α");
}

/// A symbol font (`\fcharset2`: Symbol, Wingdings) has no letters: a byte from 0x80 is the font's
/// own code, U+F000 + the byte, whatever the document's code page.
#[test]
fn a_symbol_fonts_bytes_are_its_own_codes() {
    for cpg in [1252, 1251] {
        let rtf = format!(r"{{\rtf1\ansi\ansicpg{cpg}{{\fonttbl{{\f0 Arial;}}{{\f1\fcharset2 Wingdings;}}}}\f1 J\'93\'d8\f0 \'93\f1\'fc\plain\'93}}");
        assert_eq!(text(rtf.as_bytes()), "J\u{F093}\u{F0D8}\u{201C}\u{F0FC}\u{201C}", "{cpg}");
    }
    // The symbol page is not for the font's own name in the table.
    let doc = read(br"{\rtf1{\fonttbl{\f0\fcharset2 Sym\'e9bol;}}\f0 x}").unwrap();
    assert_eq!(fmt_of(&doc, "x").font.as_deref(), Some("Symébol"));
    // A symbol default font's page is its own text's: not the text of a font chosen with \f,
    // and its again after \plain.
    let rtf = br"{\rtf1\ansi\deff0{\fonttbl{\f0\fcharset2 Symbol;}{\f1 Arial;}}\'b7 \f1\'b7 \plain\'b7}";
    assert_eq!(text(rtf), "\u{F0B7} \u{B7} \u{F0B7}");
    // A code page the entry names with \cpg is not replaced by the symbol charset.
    let rtf = br"{\rtf1{\fonttbl{\f0\cpg1251\fcharset2 Odd;}{\f1\fcharset2\cpg1251 Odd2;}}\f0\'c4\f1\'c4}";
    assert_eq!(text(rtf), "\u{414}\u{414}");
}

#[test]
fn east_asian_text_decodes_across_tokens() {
    // 日本語 in Shift-JIS: 93 FA 96 7B 8C EA. 0x7B is `{` as a byte, so it must be `\'7b`.
    assert_eq!(text(br"{\rtf1\ansi\ansicpg932 \'93\'fa\'96\'7b\'8c\'ea}"), "日本語");
    assert_eq!(text(b"{\\rtf1\\ansi\\ansicpg932 \x93\xfa\\'96\\'7b\x8c\xea ok}"), "日本語 ok");
    assert_eq!(text(br"{\rtf1{\fonttbl{\f0\fcharset134 SimSun;}}\f0\'c4\'e3\'ba\'c3}"), "你好");
    assert_eq!(text(br"{\rtf1\ansicpg932 a\'93}b}"), "a\u{FFFD}b", "a lead byte left at a group end");
    assert_eq!(text(br"{\rtf1\ansicpg932 a\'93\par b}"), "a\u{FFFD}\nb");
}

#[test]
fn font_table_forms() {
    // Entries as groups or bare, `\'3b` for a `;` in a name, an alternative name in a starred
    // destination, an entry without its `;`.
    let rtf =
        br"{\rtf1{\fonttbl\f0\froman Times New Roman;\f1 A\'3bB;{\f2\fnil{\*\falt Other}DejaVu Sans;}{\f3 NoSemicolon}}\f0 a\f1 b\f2 c\f3 d\f9 e}";
    let doc = read(rtf).unwrap();
    let font = |s: &str| fmt_of(&doc, s).font;
    assert_eq!(font("a").as_deref(), Some("Times New Roman"));
    assert_eq!(font("b").as_deref(), Some("A;B"));
    assert_eq!(font("c").as_deref(), Some("DejaVu Sans"));
    assert_eq!(font("d").as_deref(), Some("NoSemicolon"));
    assert_eq!(font("e"), None, "a font the table does not have");
}

#[test]
fn outline_levels_and_indents() {
    let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\*\generator X;}\pard\outlinelevel0\b Title one\b0\par\pard\li720 Point \'93a\'94\par\pard\outlinelevel2\li1440 Sub\par\outlinelevel9 Deep\par\pard\li-720 Back\par}";
    let got: Vec<(String, Option<u8>, i32)> = paras(rtf).iter().map(|p| (p.text(), p.outline_level, p.left_indent)).collect();
    assert_eq!(
        got,
        [
            ("Title one".to_string(), Some(0), 0),
            ("Point “a”".to_string(), None, 720),
            ("Sub".to_string(), Some(2), 1440),
            ("Deep".to_string(), None, 1440),
            ("Back".to_string(), None, -720)
        ]
    );
    let huge = paras(br"{\rtf1\pard\li9223372036854775807 deep\par\pard\li-9223372036854775808 flat\par}");
    assert!(huge[0].left_indent > 1_000_000_000 && huge[1].left_indent <= 0, "{huge:?}");
}

/// Word writes a list item's bullet as `{\listtext\pard\plain …\'b7\tab}` (older writers:
/// `{\pntext …}`) after the paragraph's own `\li`. That group is the rendered bullet, not text.
#[test]
fn list_labels_are_not_text_and_keep_the_indent() {
    let rtf = br"{\rtf1\ansi\pard Title\par\pard\fi-360\li720{\listtext\pard\plain\f1\'b7\tab}Point\par\pard\li720{\pntext\f1\'b7\tab}Old\par}";
    let got: Vec<(String, i32)> = paras(rtf).iter().map(|p| (p.text(), p.left_indent)).collect();
    assert_eq!(got, [("Title".to_string(), 0), ("Point".to_string(), 720), ("Old".to_string(), 720)]);
}

#[test]
fn alignment_indents_spacing_and_position() {
    let rtf = br"{\rtf1\pard\qc\li720\ri360\fi-360\sb120\sa240 x\super 2\sub i\nosupersub y\super z\super0 w\par\pard\qr r\par\pard\ql l\par\pard\qj j\par\pard n\par}";
    let doc = read(rtf).unwrap();
    let ps: Vec<&Para> = doc.paras().collect();
    assert_eq!((ps[0].left_indent, ps[0].right_indent, ps[0].first_line_indent, ps[0].space_before, ps[0].space_after), (720, 360, -360, 120, 240));
    assert_eq!(
        ps.iter().map(|p| p.align).collect::<Vec<_>>(),
        [Some(Align::Center), Some(Align::Right), Some(Align::Left), Some(Align::Justified), None]
    );
    assert_eq!((ps[4].left_indent, ps[4].space_after), (0, 0), "\\pard resets them");
    let va = |s: &str| fmt_of(&doc, s).vert_align;
    assert_eq!(
        (va("x"), va("2"), va("i"), va("y"), va("z"), va("w")),
        (VertAlign::Baseline, VertAlign::Superscript, VertAlign::Subscript, VertAlign::Baseline, VertAlign::Superscript, VertAlign::Baseline)
    );
}

#[test]
fn paragraph_properties_end_with_their_group() {
    let got: Vec<i32> = paras(br"{\rtf1 {\li720 a\par}b\par{\pard\li360 c}\par}").iter().map(|p| p.left_indent).collect();
    assert_eq!(got, [720, 0, 0]);
}

#[test]
fn empty_paragraphs_are_kept_except_the_last() {
    let doc = read(br"{\rtf1 a\par\par b\par\par}").unwrap();
    assert_eq!(doc.blocks.len(), 4);
    assert_eq!(read(br"{\rtf1 }").unwrap().blocks.len(), 0);
    assert_eq!(read(br"{\rtf1 a}").unwrap().blocks.len(), 1);
}

#[test]
fn tables() {
    let rtf = br"{\rtf1 Before\par\trowd\trgaph108\cellx1440\cellx4320{\intbl\fs20 A1}\cell{\intbl\b B1}\cell\row
\trowd\cellx1440\cellx2880\cellx4320\pard\intbl A2\par second\cell\cell C2\cell\row\pard After\par\trowd\cellx100 X\cell\row}";
    let doc = read(rtf).unwrap();
    let [Block::Para(before), Block::Table(t), Block::Para(after), Block::Table(last)] = doc.blocks.as_slice() else { panic!("{doc:?}") };
    assert_eq!((before.text(), after.text()), ("Before".to_string(), "After".to_string()));
    let cells = |row: &[Cell]| row.iter().map(|c| (c.right, c.paras.iter().map(Para::text).collect::<Vec<_>>().join("|"))).collect::<Vec<_>>();
    assert_eq!(cells(&t.rows[0]), [(1440, "A1".to_string()), (4320, "B1".to_string())]);
    assert_eq!(cells(&t.rows[1]), [(1440, "A2|second".to_string()), (2880, String::new()), (4320, "C2".to_string())]);
    assert!(t.rows[0][1].paras[0].runs[0].fmt.bold);
    assert_eq!(cells(&last.rows[0]), [(100, "X".to_string())]);
    // Reading order.
    assert_eq!(doc.paras().map(Para::text).collect::<Vec<_>>().join(" "), "Before A1 B1 A2 second  C2 After X");
    // A file cut inside a row keeps what the row had.
    let cut = read(br"{\rtf1\trowd\cellx100\intbl A\cell B").unwrap();
    let [Block::Table(t)] = cut.blocks.as_slice() else { panic!("{cut:?}") };
    assert_eq!(cells(&t.rows[0]), [(100, "A".to_string()), (0, "B".to_string())]);
}

#[test]
fn groups_nested_past_the_limit_are_read_flat() {
    let deep = MAX_GROUPS * 4;
    let mut rtf = String::from("{\\rtf1 ");
    rtf.push_str(&"{\\b ".repeat(deep));
    rtf.push_str("deep");
    rtf.push_str(&"}".repeat(deep));
    rtf.push_str(" after}");
    let doc = read(rtf.as_bytes()).unwrap();
    assert_eq!(doc.paras().map(Para::text).collect::<String>(), "deep after");
    assert!(fmt_of(&doc, "deep").bold && !fmt_of(&doc, "after").bold, "the groups within the limit still close in step");
    // A skipped destination deeper than the limit is still skipped.
    let mut rtf = String::from("{\\rtf1 ");
    rtf.push_str(&"{".repeat(MAX_GROUPS - 3));
    rtf.push_str("{\\info {{{{secret}}}}}shown");
    assert_eq!(text(rtf.as_bytes()), "shown");
    // Unbalanced closes are ignored.
    assert_eq!(text(b"{\\rtf1 a}}}}b{c"), "abc");
}

#[test]
fn font_and_colour_tables_are_capped() {
    let mut rtf = String::from("{\\rtf1{\\fonttbl");
    for i in 0..MAX_FONTS + 10 {
        rtf.push_str(&format!("{{\\f{i} F{i};}}"));
    }
    rtf.push_str(&format!("{{\\f9000 {};}}{{\\f9001 {};}}", "n".repeat(MAX_FONT_NAME), "n".repeat(MAX_FONT_NAME + 1)));
    rtf.push_str("}{\\colortbl;");
    for i in 0..MAX_COLORS + 10 {
        rtf.push_str(&format!("\\red{}\\green1\\blue1;", i % 256));
    }
    rtf.push_str(&format!("}}\\f{} a\\f{} b\\f9000 x\\f9001 y\\cf{} c\\cf{} d}}", MAX_FONTS - 1, MAX_FONTS, MAX_COLORS - 1, MAX_COLORS));
    let doc = read(rtf.as_bytes()).unwrap();
    assert_eq!(fmt_of(&doc, "a").font, Some(format!("F{}", MAX_FONTS - 1)));
    assert_eq!(fmt_of(&doc, "b").font, None, "past the font limit");
    assert_eq!(fmt_of(&doc, "x").font, None, "the table was full before the 256-byte name");
    assert_eq!(fmt_of(&doc, "y").font, None);
    assert_eq!(fmt_of(&doc, "c").color, Some([((MAX_COLORS - 2) % 256) as u8, 1, 1]));
    assert_eq!(fmt_of(&doc, "d").color, None, "past the colour limit");
    // The name limit on its own.
    let rtf = format!("{{\\rtf1{{\\fonttbl{{\\f0 {};}}{{\\f1 {};}}}}\\f0 a\\f1 b}}", "n".repeat(MAX_FONT_NAME), "n".repeat(MAX_FONT_NAME + 1));
    let doc = read(rtf.as_bytes()).unwrap();
    assert_eq!(fmt_of(&doc, "a").font.map(|f| f.len()), Some(MAX_FONT_NAME));
    assert_eq!(fmt_of(&doc, "b").font, None);
}

fn sample() -> Document {
    let fmt = |f: fn(&mut CharFmt)| {
        let mut c = CharFmt::default();
        f(&mut c);
        c
    };
    let run = |text: &str, fmt: CharFmt| Run { text: text.into(), fmt };
    let cell = |right, paras| Cell { right, paras };
    Document {
        blocks: vec![
            Block::Para(Para { runs: vec![run("Plan {Q3} \\ 100%", fmt(|c| c.bold = true))], outline_level: Some(0), ..Para::default() }),
            Block::Para(Para {
                runs: vec![
                    run(" Grow — fast 😀 ", CharFmt::default()),
                    run("Café\u{A0}résumé", fmt(|c| (c.italic, c.underline, c.half_points) = (true, true, Some(21)))),
                    run("\ttab\nline", fmt(|c| (c.strike, c.color) = (true, Some([255, 0, 128])))),
                    run("123 numbers first", fmt(|c| c.font = Some("Deja;Vu".into()))),
                    run("2", fmt(|c| c.vert_align = VertAlign::Superscript)),
                    run("i", fmt(|c| c.vert_align = VertAlign::Subscript)),
                ],
                outline_level: Some(8),
                left_indent: 720,
                right_indent: 360,
                first_line_indent: -360,
                space_before: 120,
                space_after: 240,
                align: Some(Align::Justified),
            }),
            Block::Para(Para::default()),
            Block::Table(Table {
                rows: vec![
                    vec![cell(1440, vec![para("A1")]), cell(4320, vec![para("B1"), Para::default(), para("B1 third")])],
                    vec![
                        cell(1440, vec![Para::default()]),
                        cell(2880, vec![para("日本語")]),
                        cell(4320, vec![Para { left_indent: -360, ..para("C2") }]),
                    ],
                ],
            }),
            Block::Para(Para { left_indent: -100, ..para(&format!("after{PAGE_BREAK}page{COLUMN_BREAK}column")) }),
            Block::Table(Table { rows: vec![vec![cell(50, vec![para("last")])]] }),
        ],
        pictures_left_out: 0,
    }
}

#[test]
fn what_is_written_reads_back() {
    let doc = sample();
    let rtf = write(&doc, "Helvetica");
    assert!(rtf.is_ascii(), "7-bit");
    assert_eq!(read(rtf.as_bytes()).unwrap(), doc, "{rtf}");
    assert_eq!(write(&read(rtf.as_bytes()).unwrap(), "Helvetica"), rtf, "and writes the same file again");
    assert_eq!(read(write(&Document::default(), "x").as_bytes()).unwrap(), Document::default());
}

#[test]
fn the_written_file() {
    let doc = Document {
        blocks: vec![
            Block::Para(Para {
                runs: vec![Run { text: "Title {1}".into(), fmt: CharFmt { bold: true, half_points: Some(32), ..CharFmt::default() } }],
                outline_level: Some(0),
                ..Para::default()
            }),
            Block::Para(Para { left_indent: 360, ..para("Body é") }),
            Block::Table(Table { rows: vec![vec![Cell { right: 1440, paras: vec![para("A")] }, Cell { right: 2880, paras: vec![] }], vec![]] }),
        ],
        pictures_left_out: 7,
    };
    let red = CharFmt { color: Some([255, 0, 0]), font: Some("Courier New".into()), ..CharFmt::default() };
    let mut colored = doc.clone();
    colored.blocks.push(Block::Para(Para { runs: vec![Run { text: "red".into(), fmt: red }], ..Para::default() }));
    assert_eq!(
        write(&doc, "Liberation Sans"),
        "{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1{\\fonttbl{\\f0\\fnil Liberation Sans;}}\n\\pard\\outlinelevel0 {\\b\\fs32 Title \\{1\\}}\\par\n\\pard\\li360 Body \\u233?\\par\n\\trowd\\trgaph108\\cellx1440\\cellx2880\\pard\\intbl A\\cell\\cell\\row\n\\pard}"
    );
    let rtf = write(&colored, "Liberation Sans");
    assert!(rtf.contains("{\\fonttbl{\\f0\\fnil Liberation Sans;}{\\f1\\fnil Courier New;}}\n{\\colortbl;\\red255\\green0\\blue0;}\n"), "{rtf}");
    assert!(rtf.ends_with("\\pard\\pard {\\f1\\cf1 red}\\par\n}"), "{rtf}");
    // The default font named on a run needs no word.
    let named = Document {
        blocks: vec![Block::Para(Para {
            runs: vec![Run { text: "x".into(), fmt: CharFmt { font: Some("liberation sans".into()), ..CharFmt::default() } }],
            ..Para::default()
        })],
        pictures_left_out: 0,
    };
    assert!(write(&named, "Liberation Sans").contains("\\pard x\\par"));
}

#[test]
fn escaping() {
    assert_eq!(craft_rtf::esc("a\\b{c}d\te\nf\u{B}g\u{2028}h"), "a\\\\b\\{c\\}d\\tab e\\line f\\line g\\line h");
    assert_eq!(craft_rtf::esc("\u{A0}\u{AD}\u{2011}\u{C}\u{E}\u{1}\r"), "\\~\\-\\_\\page \\column ");
    assert_eq!(craft_rtf::esc("é€😀"), "\\u233?\\u8364?\\u-10179?\\u-8704?");
    assert_eq!(craft_rtf::esc_font("A;B\n{C}"), "A\\'3bB\\{C\\}");
    let mut s = String::from("x");
    craft_rtf::push_esc(&mut s, "{");
    assert_eq!(s, "x\\{");
}

/// Every prefix of a file, and the file with each byte changed, reads without a panic.
#[test]
fn damaged_files_do_not_panic() {
    let rtf = write(&sample(), "Helvetica");
    let rtf = format!(
        "{rtf}{{\\rtf1\\ansicpg932{{\\fonttbl{{\\f0\\fcharset128 MS Mincho;}}}}{{\\info{{\\title x}}}}\\'93\\'fa\\u-10179?\\bin3 abc{{\\pict\\'00}}\\trowd\\cellx9 a\\cell\\row}}"
    );
    let bytes = rtf.as_bytes();
    for end in 0..=bytes.len() {
        let _ = read(&bytes[..end]);
    }
    for at in 0..bytes.len() {
        for b in [b'{', b'}', b'\\', b'\'', 0xFF, 0, b'9', b'-', b';'] {
            let mut damaged = bytes.to_vec();
            damaged[at] = b;
            let _ = read(&damaged);
        }
    }
}
