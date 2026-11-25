use super::*;

#[test]
fn widthless_columns_do_not_change_cell_width_resolution() {
    let widths = |columns: &str, cells: &str| {
        let flow = parse(&format!("<table>{columns}<tr>{cells}</tr></table>"));
        let FBlock::Table(table) = &flow.blocks[0] else { panic!("expected table") };
        table.widths.clone()
    };
    let partial = "<td width='400'>x</td><td>y</td>";
    let widthless = "<colgroup><col><col></colgroup>";
    assert!(widths("", partial).is_empty());
    assert_eq!(widths(widthless, partial), widths("", partial));
    let complete = "<td width='40'>x</td><td width='80'>y</td>";
    assert_eq!(widths(widthless, complete), vec![30.0, 60.0]);
    // An explicitly declared column still enables the existing average-fill rule.
    assert_eq!(widths("<colgroup><col style='width:100pt'><col></colgroup>", "<td>x</td><td>y</td>"), vec![100.0, 100.0]);
}

#[test]
fn numeric_entity_rejects_sign() {
    assert_eq!(unescape("a&#+65;b"), "a&#+65;b");
    assert_eq!(unescape("a&#x+41;b"), "a&#x+41;b");
    assert_eq!(unescape("a&#65;&#x41;b"), "aAAb");
}

#[test]
fn important_declarations_win_the_cascade() {
    let color = |html: &str| fmt_of(paras(&parse(html))[0]).color;
    let red = Some(craft_wordmodel::Rgb(255, 0, 0));
    let blue = Some(craft_wordmodel::Rgb(0, 0, 255));
    assert_eq!(color("<style>p { color: red }</style><p style='color: blue'>x</p>"), blue);
    assert_eq!(color("<style>p { color: red !important }</style><p style='color: blue'>x</p>"), red);
    assert_eq!(color("<style>p { color: red !important }</style><p style='color: blue !important'>x</p>"), blue);
    assert_eq!(color("<p style='color: red !important; color: blue'>x</p>"), red);
}

#[test]
fn a_declared_charset_decodes_the_page() {
    // ISO 8859-2 (Central European): 0xA9 Š, 0xB9 š; named by the caller or the page.
    assert_eq!(decode_as(&[0xA9, b' ', 0xB9], Some("iso-8859-2")), "Š š");
    assert!(decode_as(b"<meta charset=iso-8859-2><p>\xA9</p>", None).contains("<p>Š</p>"));
    assert_eq!(decode_as(&[0x8A], Some("windows-1252")), "Š");
    // A page that says UTF-16 without a BOM is read as UTF-8, as browsers do.
    assert_eq!(decode_as("é".as_bytes(), Some("utf-16")), "é");
}

fn texts(f: &Flow) -> Vec<String> {
    f.blocks
        .iter()
        .map(|b| match b {
            FBlock::Para(p) => format!("{:?}{:?}:{}", p.kind, p.list.map(|l| (l.ordered, l.level)), p.text()),
            FBlock::Table(t) => format!("table{}x{}", t.rows.len(), t.cols()),
        })
        .collect()
}

fn table_style(html: &str) -> Option<String> {
    let doc = craft_flow::to_doc(&parse(html));
    doc.body.iter().find_map(|b| match &**b {
        craft_wordmodel::Block::Table(t) => Some(t.props.style.clone().unwrap_or_default()),
        _ => None,
    })
}

#[test]
fn borderless_tables_get_no_grid_style() {
    assert_eq!(table_style("<table><tr><td>a<td>b</table>").as_deref(), Some(""));
    assert_eq!(table_style("<table style='border:none'><tr><td style='border:0'>a</table>").as_deref(), Some(""));
    assert_eq!(table_style("<table border=0><tr><td style='border: 0px none'>a</table>").as_deref(), Some(""));
    assert_eq!(table_style("<table border=1><tr><td>a</table>").as_deref(), Some("TableGrid"));
    assert_eq!(table_style("<table style='border:1px solid #000'><tr><td>a</table>").as_deref(), Some("TableGrid"));
    assert_eq!(table_style("<table><tr><td style='border:1px solid #999'>a</table>").as_deref(), Some("TableGrid"));
    // The last declaration decides: a `style` attribute's over the style sheet's.
    assert_eq!(table_style("<style>table { border: 1px solid }</style><table><tr><td>a</table>").as_deref(), Some("TableGrid"));
    assert_eq!(table_style("<style>table { border: 1px solid }</style><table style='border:none'><tr><td>a</table>").as_deref(), Some(""));
}

#[test]
fn borderless_tables_export_without_cell_borders() {
    // #438: a table without grid style and borders writes no cell borders and reopens borderless.
    for (src, grid) in [
        ("<table style='border:none'><tr><td>A<td>B</table>", false),
        ("<table><tr><td>A<td>B</table>", false),
        ("<table border=1><tr><td>A<td>B</table>", true),
    ] {
        let out = export(&craft_flow::to_doc(&parse(src)));
        assert_eq!(out.contains("border:1px"), grid, "{src}: {out}");
        assert_eq!(table_style(&out).as_deref(), Some(if grid { "TableGrid" } else { "" }), "{src}");
        assert!(out.contains(">A</td>") && out.contains(">B</td>"), "{out}");
    }
}

/// storytold/wordcraft#5065: a DOCX bottom border without `w:sz` is width 0, and Word draws it at
/// its thinnest line. An empty paragraph carrying one is the horizontal rule HTML has.
#[test]
fn a_blank_paragraph_with_a_width_0_bottom_border_is_still_a_rule() {
    let mut p = craft_wordmodel::Paragraph::new();
    p.props.borders = Some(craft_wordmodel::props::Borders { bottom: Some(craft_wordmodel::props::Border::single(0.0)), ..Default::default() });
    let mut d = craft_wordmodel::Document::new();
    d.body = vec![craft_wordmodel::para_block(p)];
    assert!(export(&d).contains("<hr>"), "{}", export(&d));
}

#[test]
fn input_newlines_preserve_preformatted_lines() {
    for newline in ["\n", "\r\n", "\r"] {
        let f = parse(&format!("<pre>{newline}  Alpha{newline}  <b>Beta</b>{newline}{newline}  Gamma</pre>"));
        assert_eq!(texts(&f), ["CodeNone:  Alpha", "CodeNone:  Beta", "CodeNone:", "CodeNone:  Gamma"], "{newline:?}");
        let FBlock::Para(p) = &f.blocks[1] else { panic!() };
        assert!(matches!(p.inlines.last(), Some(Inline::Text(t, f)) if t == "Beta" && f.bold));
    }
}

#[test]
fn input_newlines_are_normalized_before_character_references() {
    let f = parse("<p style='white-space:pre'>A&#13;B\r\nC\rD</p><p>A\r\nB\rC</p>");
    assert_eq!(texts(&f), ["NormalNone:A\rB\nC\nD", "NormalNone:A B C"]);
    let f = parse("<p style='white-space:pre-wrap'>α\r\r\nβ&#13;γ</p><a href='https://example.test/a\r\nb\rc&#13;d'>link</a>");
    assert_eq!(texts(&f), ["NormalNone:α\n\nβ\rγ", "NormalNone:link"]);
    let FBlock::Para(p) = &f.blocks[1] else { panic!() };
    assert!(matches!(p.inlines.first(), Some(Inline::Text(_, f)) if f.link.as_deref() == Some("https://example.test/a\nb\nc\rd")));
}

#[test]
fn css_preformatted_paragraphs_keep_edge_spaces() {
    // #439: `white-space:pre` keeps leading and trailing spaces like `<pre>`; normal
    // paragraphs still collapse.
    let f = parse(
        "<p style=\"white-space:pre\">  Alpha  Beta  </p><p style=\"white-space:pre-wrap\">\n  Gamma  \n</p><pre>  Alpha  </pre><p>  Alpha  Beta  </p>",
    );
    assert_eq!(texts(&f), vec!["NormalNone:  Alpha  Beta  ", "NormalNone:  Gamma  ", "CodeNone:  Alpha  ", "NormalNone:Alpha Beta"]);
}

#[test]
fn tag_soup() {
    let f = parse(
        "<html><head><title>T &amp; U</title><style>p{}</style><script>if(a<b)x</script></head><body><h1>Head</h1><p>One <b>bold<p>Two <i>it</i>\n  text<ul><li>a<li>b<ol><li>c</ol></ul><table><tr><td>x<td colspan=2>y<tr><th>z</table><blockquote>q</blockquote><pre>l1\nl2</pre>",
    );
    assert_eq!(f.meta.title, "T & U");
    assert_eq!(
        texts(&f),
        vec![
            "Heading(1)None:Head",
            "NormalNone:One bold",
            "NormalNone:Two it text",
            "NormalSome((false, 0)):a",
            "NormalSome((false, 0)):b",
            "NormalSome((true, 1)):c",
            "table2x3",
            "QuoteNone:q",
            "CodeNone:l1",
            "CodeNone:l2",
        ]
    );
    let FBlock::Para(p) = &f.blocks[1] else { panic!() };
    assert!(matches!(&p.inlines[1], Inline::Text(t, f) if t == "bold" && f.bold));
}

#[test]
fn omitted_head_keeps_body_text() {
    let f = parse("<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>T</title>\n<body><p>Hello world</p><p>Second</p></body></html>");
    assert_eq!(f.meta.title, "T");
    assert_eq!(texts(&f), vec!["NormalNone:Hello world", "NormalNone:Second"]);

    for body in [
        "<p>Hello world</p>",
        "<div>Hello world</div>",
        "<span>Hello world</span>",
        "<a href='https://example.test'>Hello world</a>",
        "<custom>Hello world</custom>",
        "<textarea>ignored</textarea>Hello world",
    ] {
        let f = parse(&format!(
            "<html><head><title>T &amp; U</title><meta name='author' content='Ann'><style>p{{}}</style><script>ignored</script>{body}",
        ));
        assert_eq!(f.meta.title, "T & U");
        assert_eq!(f.meta.author, "Ann");
        assert_eq!(texts(&f), vec!["NormalNone:Hello world"], "{body}");
    }
}

#[test]
fn styles_and_links() {
    let f = parse("<p style='text-align:center'><span style=\"color:#ff0000;font-weight:700\">red</span> <a href=\"http://x\">l</a><br>n</p>");
    let FBlock::Para(p) = &f.blocks[0] else { panic!() };
    assert_eq!(p.align, Some(Align::Center));
    assert!(matches!(&p.inlines[0], Inline::Text(t, f) if t == "red" && f.bold && f.color == Some(craft_wordmodel::Rgb(255,0,0))));
    assert!(p.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "l" && f.link.as_deref() == Some("http://x"))));
    assert!(p.text().ends_with("l\nn"));
}

/// The paragraphs of every cell of the first table, row by row.
fn cell_paras(f: &Flow) -> Vec<Vec<Vec<Para>>> {
    let Some(FBlock::Table(t)) = f.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table") };
    t.rows
        .iter()
        .map(|r| r.iter().map(|c| c.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p.clone()) } else { None }).collect()).collect())
        .collect()
}

fn all_bold(p: &Para) -> bool {
    !p.inlines.is_empty() && p.inlines.iter().all(|i| matches!(i, Inline::Text(_, f) if f.bold))
}

fn any_bold(p: &Para) -> bool {
    p.inlines.iter().any(|i| matches!(i, Inline::Text(_, f) if f.bold))
}

#[test]
fn header_cells_are_bold_unless_their_css_says_otherwise() {
    // #1381: a <th> is bold, as in browsers and Word; `font-weight:normal` on it or inside it wins.
    let f = parse(
        "<table><thead><tr><th>Head</th><th style='font-weight:normal'>Plain</th><th><span style='font-weight:400'>Inner</span></th></tr></thead><tbody><tr><td>a<td>b<td>c</tbody></table>",
    );
    let rows = cell_paras(&f);
    assert!(all_bold(&rows[0][0][0]), "{:?}", rows[0][0]);
    assert!(!any_bold(&rows[0][1][0]), "{:?}", rows[0][1]);
    assert!(!any_bold(&rows[0][2][0]), "{:?}", rows[0][2]);
    assert!(!any_bold(&rows[1][0][0]), "td stays regular");
}

#[test]
fn cell_text_align_reaches_the_cell_paragraphs() {
    // #1381: `text-align` on a td/th, or inherited from its tr/thead/tbody/table, aligns the
    // cell's paragraphs; a paragraph's own text-align inside the cell wins.
    let f = parse(
        "<table style='text-align:center'><tbody style='text-align:justify'><tr style='text-align:right'>\
             <td style='text-align:right;'>r</td><td>inherited</td><td style='text-align:right'><p style='text-align:center'>own</p></td>\
             <td align=right>attr</td></tr><tr><td>body</td></tr></tbody><tr><td>table</td></tr></table>",
    );
    let rows = cell_paras(&f);
    assert_eq!(rows[0][0][0].align, Some(Align::Right), "td style");
    assert_eq!(rows[0][1][0].align, Some(Align::Right), "from tr");
    assert_eq!(rows[0][2][0].align, Some(Align::Center), "own paragraph wins");
    assert_eq!(rows[0][3][0].align, Some(Align::Right), "align attribute");
    assert_eq!(rows[1][0][0].align, Some(Align::Justify), "from tbody");
    assert_eq!(rows[2][0][0].align, Some(Align::Center), "from table");
    // `align` on a table places the table; it doesn't align the text in its cells.
    let f = parse("<table align=center><tr><td>x</td></tr></table>");
    assert_eq!(cell_paras(&f)[0][0][0].align, None);
}

#[test]
fn reporter_sample_header_bold_and_right_aligned_cells() {
    // #1381: the reporter's table.
    let f = parse(
        "<table border=\"1\" style=\"width:100%;border-collapse:collapse;\">\n<thead><tr style=\"background-color:#e8f0fa;\"><th>Indicatore</th><th>Valore atteso</th></tr></thead>\n<tbody>\n<tr><td>Audit</td><td style=\"text-align:right;\">4</td></tr>\n</tbody></table>",
    );
    let rows = cell_paras(&f);
    assert!(all_bold(&rows[0][0][0]) && all_bold(&rows[0][1][0]), "{:?}", rows[0]);
    assert_eq!(rows[1][1][0].align, Some(Align::Right));
    assert_eq!(rows[1][0][0].align, None);
}

#[test]
fn entities() {
    assert_eq!(unescape("a &lt;b&gt; &#65;&#x42; &bogus; &"), "a <b> AB &bogus; &");
    assert_eq!(unescape("&minus; &COPY; &EURO; &TRADE; &frac12; &frac34;"), "− © € ™ ½ ¾");
    assert_eq!(unescape("&theta; &lambda; &Delta; &nabla; &sum; &radic; &ne; &le; &ge; &hearts;"), "θ λ Δ ∇ ∑ √ ≠ ≤ ≥ ♥");
}

/// Browsers decode numeric and legacy named references without their `;` in text,
/// but keep `&name=` and `&nameX` intact inside attribute values (query strings).
#[test]
fn references_without_semicolon() {
    assert_eq!(unescape("&copy 2024 Acme"), "© 2024 Acme");
    assert_eq!(unescape("&#169 2024 Acme"), "© 2024 Acme");
    assert_eq!(unescape("&#xA9 2024 &amp &lt"), "© 2024 & <");
    assert_eq!(unescape("&bogus &# &#x &"), "&bogus &# &#x &");
    assert_eq!(unescape("&nbsp;x"), "\u{00A0}x");
    let f = parse("<p><a href=\"/q?a=1&copy=2&reg2=3&amp;b=&lt\">l</a> &copy 2024</p>");
    let FBlock::Para(p) = &f.blocks[0] else { panic!() };
    assert!(p.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "l" && f.link.as_deref() == Some("/q?a=1&copy=2&reg2=3&b=<"))));
    assert!(p.text().ends_with("© 2024"));
}

#[test]
fn wbr_word_break_opportunity() {
    let f = parse("<p>Super<wbr>cali<wbr>fragilistic</p>");
    let Some(FBlock::Para(p)) = f.blocks.first() else { panic!("expected para") };
    assert_eq!(p.text(), "Super\u{200B}cali\u{200B}fragilistic");
}

#[test]
fn page_break_after_and_before() {
    let f = parse("<p style='page-break-after: always'>First</p><p>Second</p>");
    let paras: Vec<_> = f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect();
    assert_eq!(paras.len(), 2);
    assert!(!paras[0].page_break);
    assert!(paras[1].page_break);

    let f2 = parse("<div style='break-after: page'><p>Part 1</p></div><p>Part 2</p>");
    let paras2: Vec<_> = f2.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect();
    assert_eq!(paras2.len(), 2);
    assert!(!paras2[0].page_break);
    assert!(paras2[1].page_break);
}

#[test]
fn font_size_relative_and_absolute() {
    let f = parse("<font size=\"+1\">larger</font><font size=\"-1\">smaller</font><font size=\"5\">huge</font>");
    let inls = match &f.blocks[0] {
        FBlock::Para(p) => &p.inlines,
        _ => panic!("expected para"),
    };
    let sizes: Vec<Option<f32>> = inls
        .iter()
        .map(|i| match i {
            Inline::Text(_, fmt) => fmt.size,
            _ => None,
        })
        .collect();
    assert_eq!(sizes, vec![Some(13.5), Some(10.0), Some(18.0)]);
}

#[test]
fn decode_with_charset_declarations() {
    let iso = b"<html><head><meta charset=\"iso-8859-1\"></head><body>\xE9l\xE8ve</body></html>";
    assert!(decode(iso).contains("élève"));

    let win = b"<html><head><meta charset='windows-1252'></head><body>\x80100</body></html>";
    assert!(decode(win).contains("€100"));

    let gbk = b"<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=gbk\"></head><body>\xC4\xE3\xBA\xC3</body></html>";
    assert!(decode(gbk).contains("你好"));

    let xml = b"<?xml version=\"1.0\" encoding=\"utf-8\"?><html><body>hello</body></html>";
    assert!(decode(xml).contains("hello"));
}

#[test]
fn raw_text_elements_skip_only_matching_end_tag() {
    let f = parse("<script>var a = \"</scripted>\"; var b = \"<b>malicious</b>\";</script><p>Hello</p>");
    assert_eq!(f.blocks.len(), 1);
    let p = match &f.blocks[0] {
        FBlock::Para(p) => p,
        b => panic!("expected para, got {b:?}"),
    };
    assert_eq!(p.text(), "Hello");

    let f = parse("<style>/* </styles> */ p { color: red; }</style><p>World</p>");
    assert_eq!(f.blocks.len(), 1);
    let p = match &f.blocks[0] {
        FBlock::Para(p) => p,
        b => panic!("expected para, got {b:?}"),
    };
    assert_eq!(p.text(), "World");
}

#[test]
fn leading_space_survives_round_trip() {
    let doc = Document::from_text(" lead");
    let html = export(&doc);
    assert!(!html.contains("<p> lead"), "{html}");
    let back = import(&html);
    assert_eq!(back.plain_text(Default::default()).replace('\u{a0}', " "), " lead");
}

#[test]
fn css_escapes_read_every_kind_of_newline() {
    // A backslash before CR LF, CR, LF or form feed continues the line; after hex digits, one
    // of them (CR LF as one) ends the escape.
    assert_eq!(css_unescape("A\\\r\nB\\\rC\\\nD\\\u{C}E"), "ABCDE");
    assert_eq!(css_unescape("\\41\r\nB \\42\u{C}C \\43 D"), "AB BC CD");
    assert_eq!(css_unescape("x\\'y\\"), "x'y");
}

#[test]
fn font_names_are_written_as_css_strings() {
    // A font name could end the CSS string or the declaration and add CSS of its own (#3739).
    let fonts = ["Arial\n;position:fixed", "Back\\slash", "O'Brien \"Sans\"", "A,B", "Times New Roman", "M+ 1p"];
    let mut p = Para::default();
    for (i, font) in fonts.iter().enumerate() {
        p.push_text(&format!("t{i}"), &Fmt { font: Some((*font).into()), ..Default::default() });
    }
    let html = export(&model::to_doc(&Flow { blocks: vec![FBlock::Para(p)], meta: Meta::default() }));
    assert!(html.contains("font-family:'Times New Roman'") && html.contains("font-family:'M+ 1p'"), "{html}");
    let styles: Vec<&str> = html.split(" style=\"").skip(1).filter_map(|s| s.split('"').next()).filter(|s| s.contains("font-family")).collect();
    assert_eq!(styles.len(), fonts.len(), "{html}");
    for s in styles {
        assert!(!s.contains([';', '\n']) && s.matches('\'').count() == 2, "{s}");
    }
    // Read back, each run has its font as it was.
    let back = parse(&html);
    let runs: Vec<(String, Option<String>)> = back
        .blocks
        .iter()
        .filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None })
        .flat_map(|p| p.inlines.iter())
        .filter_map(|i| if let Inline::Text(t, f) = i { Some((t.clone(), f.font.as_deref().map(String::from))) } else { None })
        .collect();
    for (i, font) in fonts.iter().enumerate() {
        assert!(runs.contains(&(format!("t{i}"), Some(font.to_string()))), "{font:?} in {runs:?}");
    }
}

/// A document that reads "I read " then a phonetic guide over `base`, then " daily.".
fn ruby_doc(base: &str, ruby: &str) -> Document {
    let props = craft_wordmodel::CharProps::default();
    let mut p = craft_wordmodel::Paragraph::with_text("I read ", props.clone());
    let n = p.len();
    p.insert_object(n, craft_flow::ruby_object(base.into(), ruby.into(), 20.0), &props).unwrap();
    let n = p.len();
    p.insert_text(n, " daily.", &props).unwrap();
    let mut d = Document::new();
    d.body = vec![craft_wordmodel::para_block(p)];
    d
}

/// The first flow ruby: its base text and reading.
fn ruby_of(f: &Flow) -> Option<(String, String)> {
    f.blocks.iter().find_map(|b| match b {
        FBlock::Para(p) => p.inlines.iter().find_map(|i| match i {
            Inline::Ruby { base, ruby, .. } => Some((base.clone(), ruby.clone())),
            _ => None,
        }),
        _ => None,
    })
}

/// The first document ruby: its base text and reading.
fn doc_ruby(d: &Document) -> Option<(String, String)> {
    d.body.iter().find_map(|b| match &**b {
        craft_wordmodel::Block::Para(p) => p.objects.iter().find_map(|o| match o {
            craft_wordmodel::InlineObject::Ruby { base, ruby, .. } => Some((base.clone(), ruby.clone())),
            _ => None,
        }),
        _ => None,
    })
}

/// The first document ruby: base size, ruby size, raise and alignment.
fn doc_ruby_attrs(d: &Document) -> Option<(f32, f32, f32, craft_wordmodel::para::RubyAlign)> {
    d.body.iter().find_map(|b| match &**b {
        craft_wordmodel::Block::Para(p) => p.objects.iter().find_map(|o| match o {
            craft_wordmodel::InlineObject::Ruby { base_size, size, raise, align, .. } => Some((*base_size, *size, *raise, *align)),
            _ => None,
        }),
        _ => None,
    })
}

#[test]
fn ruby_exports_as_ruby_markup() {
    // #1681: the base text with its reading in `<rt>`, not plain base text.
    let out = export(&ruby_doc("漢字", "かんじ"));
    assert!(out.contains("<p>I read <ruby>漢字<rt>かんじ</rt></ruby> daily.</p>"), "{out}");
    // The base and the reading are escaped like any other text.
    let out = export(&ruby_doc("A&B", "x<y"));
    assert!(out.contains("<ruby>A&amp;B<rt>x&lt;y</rt></ruby>"), "{out}");
}

#[test]
fn ruby_imports_with_its_surrounding_text() {
    let f = parse("<p>I read <ruby>漢字<rt>かんじ</rt></ruby> daily.</p>");
    assert_eq!(ruby_of(&f), Some(("漢字".into(), "かんじ".into())));
    assert_eq!(texts(&f), vec!["NormalNone:I read 漢字 daily."]);
    let FBlock::Para(p) = &f.blocks[0] else { panic!() };
    assert!(matches!(&p.inlines[0], Inline::Text(t, _) if t == "I read "), "{:?}", p.inlines);
    assert!(matches!(p.inlines.last(), Some(Inline::Text(t, _)) if t == " daily."), "{:?}", p.inlines);
    // One document object, holding the base text and the reading.
    assert_eq!(doc_ruby(&craft_flow::to_doc(&f)), Some(("漢字".into(), "かんじ".into())));
}

#[test]
fn ruby_rb_and_rp_fallbacks_import_as_one_guide() {
    // `<rb>` holds the base text and `<rp>` the fallback parentheses for browsers without
    // ruby: the parentheses are not text, and the base pieces join.
    let f = parse("<p><ruby><rb>漢</rb><rb>字</rb><rp>(</rp><rt>かんじ</rt><rp>)</rp></ruby></p>");
    assert_eq!(ruby_of(&f), Some(("漢字".into(), "かんじ".into())));
    assert_eq!(texts(&f), vec!["NormalNone:漢字"]);
    assert_eq!(doc_ruby(&craft_flow::to_doc(&f)), Some(("漢字".into(), "かんじ".into())));
}

#[test]
fn ruby_import_uses_the_phonetic_guide_defaults() {
    // HTML states no alignment, size or raise: the Phonetic Guide's own defaults over a
    // 20 pt run (half the base size, raised clear of the base text, centred)…
    let d = crate::doc::import("<p style='font-size:20pt'><ruby>漢字<rt>かんじ</rt></ruby></p>");
    let Some((base_size, size, raise, align)) = doc_ruby_attrs(&d) else { panic!("no ruby") };
    assert_eq!((base_size, size, align.ooxml()), (20.0, 10.0, "center"));
    assert!((raise - 18.0).abs() < 0.001, "{raise}");
    // …and the guide's 10.5 pt fallback base size when the source states none.
    let d = crate::doc::import("<p><ruby>漢字<rt>かんじ</rt></ruby></p>");
    let Some((base_size, size, raise, _)) = doc_ruby_attrs(&d) else { panic!("no ruby") };
    assert_eq!((base_size, size), (10.5, 5.0));
    assert!((raise - 9.4).abs() < 0.001, "{raise}");
}

#[test]
fn ruby_survives_an_html_round_trip() {
    let html = export(&ruby_doc("漢字", "かんじ"));
    assert_eq!(ruby_of(&parse(&html)), Some(("漢字".into(), "かんじ".into())));
    let back = crate::doc::import(&html);
    assert_eq!(doc_ruby(&back), Some(("漢字".into(), "かんじ".into())));
    assert_eq!(back.plain_text(Default::default()), "I read 漢字 daily.");
}

#[test]
fn ruby_keeps_the_run_formatting() {
    let props = craft_wordmodel::CharProps { bold: Some(true), ..Default::default() };
    let mut p = craft_wordmodel::Paragraph::with_text("I read ", props.clone());
    let n = p.len();
    p.insert_object(n, craft_flow::ruby_object("漢字".into(), "かんじ".into(), 20.0), &props).unwrap();
    let mut d = Document::new();
    d.body = vec![craft_wordmodel::para_block(p)];
    assert!(export(&d).contains("<strong><ruby>漢字<rt>かんじ</rt></ruby></strong>"), "{}", export(&d));
    // Importing it back keeps the formatting for the base text and the reading.
    let f = parse("<p><strong><ruby>漢字<rt>かんじ</rt></ruby></strong></p>");
    let FBlock::Para(p) = &f.blocks[0] else { panic!() };
    assert!(matches!(&p.inlines[0], Inline::Ruby { fmt, .. } if fmt.bold), "{:?}", p.inlines);
}

#[test]
fn malformed_ruby_keeps_its_text() {
    // No reading, an empty reading, a missing base, an unclosed element and a block boundary
    // closing it: none of them panics, and the text inside is kept.
    let cases: [(&str, &[&str]); 7] = [
        ("<p>a <ruby>漢字</ruby> b</p>", &["NormalNone:a 漢字 b"]),
        ("<p>a <ruby>漢字<rt></rt></ruby> b</p>", &["NormalNone:a 漢字 b"]),
        ("<p>a <ruby><rt>かんじ</rt></ruby> b</p>", &["NormalNone:a かんじ b"]),
        ("<p>a <ruby><rt></rt></ruby> b</p>", &["NormalNone:a b"]),
        ("<ruby>", &[]),
        ("<p>a <ruby>漢字<rt>かんじ</rt> b</p><p>next</p>", &["NormalNone:a 漢字 b", "NormalNone:next"]),
        ("<p>a <ruby>漢字<rt>かんじ</rt></p><p>next</p>", &["NormalNone:a 漢字", "NormalNone:next"]),
    ];
    for (src, want) in cases {
        assert_eq!(texts(&parse(src)), want, "{src}");
    }
    // Ruby in ruby: the inner element's text joins the outer guide's.
    assert_eq!(ruby_of(&parse("<p><ruby>漢字<ruby>字<rt>じ</rt></ruby><rt>かんじ</rt></ruby></p>")), Some(("漢字字".into(), "じかんじ".into())));
}

#[test]
fn small_big_and_css_font_sizes() {
    let f = parse(
        "<p><small>small</small> <big>big</big> <span style=\"font-size: xxx-large\">huge</span> <span style=\"font-size: smaller\">tiny</span></p>",
    );
    let FBlock::Para(p) = &f.blocks[0] else { panic!() };
    let sz = |i: usize| match &p.inlines[i] {
        Inline::Text(_, f) => f.size,
        _ => None,
    };
    assert_eq!(sz(0), Some(10.0));
    assert_eq!(sz(2), Some(14.0));
    assert_eq!(sz(4), Some(36.0));
    assert_eq!(sz(6), Some(10.0));
}

#[test]
fn css_important_casing_and_spacing() {
    let f = parse(
        "<p style='color: #ff0000 !IMPORTANT; font-size: 16pt ! important; font-weight: 700 !Important; text-align: center !IMPORTANT'>Styled</p>",
    );
    assert_eq!(f.blocks.len(), 1);
    let p = match &f.blocks[0] {
        FBlock::Para(p) => p,
        b => panic!("expected para, got {b:?}"),
    };
    assert_eq!(p.align, Some(Align::Center));
    match &p.inlines[0] {
        Inline::Text(_, fmt) => {
            assert_eq!(fmt.color, Some(craft_wordmodel::props::Rgb(0xFF, 0x00, 0x00)));
            assert_eq!(fmt.size, Some(16.0));
            assert!(fmt.bold);
        }
        i => panic!("expected text inline, got {i:?}"),
    }
}

#[test]
fn table_cell_vertical_alignment() {
    let f = parse("<table><tr><td valign=top>top</td><td valign=\"center\">mid</td><td style=\"vertical-align: bottom\">bot</td></tr></table>");
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    assert_eq!(t.rows[0][0].valign, Some(VAlign::Top));
    assert_eq!(t.rows[0][1].valign, Some(VAlign::Center));
    assert_eq!(t.rows[0][2].valign, Some(VAlign::Bottom));
    let doc = craft_flow::to_doc(&f);
    let craft_wordmodel::Block::Table(dt) = &*doc.body[0] else { panic!() };
    assert_eq!(dt.rows[0].cells[0].props.valign, Some(VAlign::Top));
    assert_eq!(dt.rows[0].cells[1].props.valign, Some(VAlign::Center));
    assert_eq!(dt.rows[0].cells[2].props.valign, Some(VAlign::Bottom));
    let html = export(&doc);
    assert!(html.contains("vertical-align:top"));
    assert!(html.contains("vertical-align:middle"));
    assert!(html.contains("vertical-align:bottom"));
}

#[test]
fn many_formatted_leading_breaks_trim_quickly() {
    // #3705: a paragraph opening with alternating bold and italic `<br>` was trimmed one
    // span at a time with `Vec::remove(0)`.
    let html = format!("<p>{}x</p>", "<b><br></b><i><br></i>".repeat(100_000));
    let t = std::time::Instant::now();
    let f = parse(&html);
    // About a second on a loaded machine; trimming one span at a time ran past 6 minutes.
    assert!(t.elapsed() < std::time::Duration::from_secs(20), "{:?}", t.elapsed());
    let texts: Vec<String> = f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p.text()) } else { None }).collect();
    assert_eq!(texts, ["x"]);
}

#[test]
fn font_size_at_the_integer_limits_does_not_overflow() {
    // `<font size>` offsets were added to 3 unchecked, so the extreme values panicked in debug builds.
    for size in ["+2147483647", "--2147483648", "-2147483647", "+99999999999"] {
        let f = parse(&format!("<font size=\"{size}\">x</font>"));
        assert!(texts(&f).iter().any(|t| t.ends_with(":x")), "size={size}");
    }
    // The offsets saturate, then clamp to the 1..=7 scale: 7 is 36 pt, 1 is 7.5 pt.
    let size = |html: &str| match &parse(html).blocks[0] {
        FBlock::Para(p) => p.inlines.iter().find_map(|i| match i {
            Inline::Text(_, fmt) => fmt.size,
            _ => None,
        }),
        _ => None,
    };
    assert_eq!(size("<font size=\"+2147483647\">x</font>"), Some(36.0));
    assert_eq!(size("<font size=\"--2147483648\">x</font>"), Some(36.0));
    assert_eq!(size("<font size=\"-2147483647\">x</font>"), Some(7.5));
}

#[test]
fn empty_comments_do_not_swallow_the_rest() {
    for html in ["<p>a<!-->b</p><p>c</p>", "<p>a<!--->b</p><p>c</p>"] {
        let doc = crate::doc::import(html);
        assert_eq!(doc.plain_text(Default::default()), "ab\nc", "{html}");
    }
}

fn paras(f: &Flow) -> Vec<&Para> {
    f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect()
}
fn fmt_of(p: &Para) -> Fmt {
    p.inlines.iter().find_map(|i| if let Inline::Text(_, f) = i { Some(f.clone()) } else { None }).unwrap_or_default()
}

#[test]
fn style_sheet_rules_apply_in_cascade_order() {
    let f = parse(
        "<style><!-- p { color: #00ff00; font-size: 10pt } .big { font-size: 20pt } p.big { font-weight: bold } \
         /* a comment { } */ @media print { p { color: #ff0000 } } div p, li:hover { font-style: italic } .000003 { text-align: center } --></style>\
         <p>plain</p><p class='big x'>big</p><p class=big style='font-size: 8pt'>inline wins</p><p class=000003>digits</p>",
    );
    let ps = paras(&f);
    let (plain, big, inline, digits) = (fmt_of(ps[0]), fmt_of(ps[1]), fmt_of(ps[2]), fmt_of(ps[3]));
    assert_eq!((plain.color, plain.size, plain.bold, plain.italic), (Some(craft_wordmodel::Rgb(0, 255, 0)), Some(10.0), false, false));
    assert_eq!((big.size, big.bold), (Some(20.0), true));
    assert_eq!(inline.size, Some(8.0));
    assert_eq!(ps[3].align, Some(Align::Center));
    assert_eq!(digits.size, Some(10.0));
}

#[test]
fn paragraph_css_states_spacing_indents_and_line_height() {
    let f = parse(
        "<style>p.a { margin: 6pt 1in 3pt; text-indent: -18pt; line-height: 150% }</style>\
         <p class=a>a</p><p style='margin-top: 14,4px; margin-bottom: auto; line-height: 20pt'>b</p><p>c</p>",
    );
    let ps = paras(&f);
    assert_eq!(
        ps[0].spacing,
        Spacing {
            before: Some(6.0),
            after: Some(3.0),
            left: Some(72.0),
            right: Some(72.0),
            first: Some(-18.0),
            line: Some(LineSpacing::Multiple(1.5)),
            ..Spacing::default()
        }
    );
    // Values that don't parse (a comma decimal, `auto`) state nothing.
    assert_eq!(ps[1].spacing, Spacing { line: Some(LineSpacing::Exactly(20.0)), ..Spacing::default() });
    assert_eq!(ps[2].spacing, Spacing::default());
}

#[test]
fn a_page_opens_as_word_opens_one() {
    let html = "<style>p.k { margin-bottom: 13px } span.z { margin: 0in }</style>\
                <p>first</p><h1>Head</h1><p class=k><span class=z>kept</span></p><p class=k>own</p><div>bare</div><p>&nbsp;</p>";
    let f = parse_in(html, &|_| None, true);
    let ps = paras(&f);
    // Times New Roman 12 pt on single lines; headings bold and black at a browser's sizes.
    let first = fmt_of(ps[0]);
    assert_eq!((first.font.as_deref(), first.size), (Some(PAGE_FONT), Some(12.0)));
    assert_eq!(ps[0].spacing.line, Some(LineSpacing::Multiple(1.0)));
    let cell = parse_in("<table><tr><td>cell</td></tr></table>", &|_| None, true);
    let FBlock::Table(t) = &cell.blocks[0] else { panic!() };
    let FBlock::Para(cp) = &t.rows[0][0].blocks[0] else { panic!() };
    assert_eq!(cp.spacing.line, Some(LineSpacing::Multiple(1.0)));
    let head = fmt_of(ps[1]);
    assert_eq!((head.bold, head.size, head.color), (true, Some(24.0), Some(craft_wordmodel::Rgb(0, 0, 0))));
    // A colour the heading inherits stays.
    // A generic family names a face on a page; pasted text keeps the document's font.
    let generic = parse_in("<p style='font-family: sans-serif'>a</p><p style='font-family: serif'>b</p>", &|_| None, true);
    let fonts: Vec<Option<String>> = paras(&generic).iter().map(|p| fmt_of(p).font.map(|f| f.to_string())).collect();
    assert_eq!(fonts, [Some("Arial".to_string()), Some(PAGE_FONT.to_string())]);
    assert_eq!(fmt_of(paras(&parse("<p style='font-family: sans-serif'>a</p>"))[0]).font, None);
    let inherited = parse_in("<body style='color: #000080'><h1>Head</h1></body>", &|_| None, true);
    assert_eq!(fmt_of(paras(&inherited)[0]).color, Some(craft_wordmodel::Rgb(0, 0, 0x80)));
    // Automatic spacing where a p or heading leaves its margins out (layout drops it at the
    // top of the page and of a cell).
    let auto = |p: &Para| (p.spacing.before_auto, p.spacing.after_auto, p.spacing.before, p.spacing.after);
    assert_eq!(auto(ps[0]), (true, true, None, None));
    assert_eq!(auto(ps[1]), (true, true, None, None));
    // A span's margin replaces the one its paragraph states; the one it leaves out stays automatic.
    assert_eq!(auto(ps[2]), (true, false, None, Some(0.0)));
    assert_eq!(auto(ps[3]), (true, false, None, Some(9.75)));
    // Other blocks get no automatic spacing.
    assert_eq!(auto(ps[4]), (false, false, None, None));
    // The document says so, and the empty paragraph's mark keeps its size.
    let d = model::to_doc(&f);
    let props = |i: usize| match &*d.body[i] {
        craft_wordmodel::Block::Para(p) => p.clone(),
        b => panic!("{b:?}"),
    };
    assert_eq!((props(0).props.space_before_auto, props(0).props.space_after_auto), (Some(true), Some(true)));
    assert_eq!((props(2).props.space_after_auto, props(2).props.space_after), (None, Some(0.0)));
    assert_eq!(props(5).mark.size, Some(12.0));
    // `<p>&nbsp;</p>` keeps the size of its space for its line.
    assert!(ps[5].inlines.is_empty());
    assert_eq!(ps[5].mark.as_ref().and_then(|m| m.size), Some(12.0));
    // A fragment (a paste) keeps the document's own formatting.
    let pasted = parse(html);
    assert_eq!(fmt_of(paras(&pasted)[0]).font, None);
    assert_eq!(paras(&pasted)[0].spacing, Spacing::default());
}

#[test]
fn cells_give_their_columns_widths() {
    let f = parse(
        "<style>td.n { width: 42px }</style><table border=1><tr><td colspan=3>t</td></tr><tr><td class=n>1</td><td width=200>a</td><td style='width: 2in'>b</td></tr></table>",
    );
    let FBlock::Table(t) = &f.blocks[0] else { panic!("{:?}", f.blocks) };
    assert_eq!(t.widths, [31.5, 150.0, 144.0]);
    // A column without a width leaves them all to the layout.
    let f = parse("<table><tr><td width=100>a</td><td>b</td></tr></table>");
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    assert!(t.widths.is_empty());
    // A cell beside a cell spanning rows from above gives its own column its width.
    let f =
        parse("<table><tr><td rowspan=2>a</td><td width=200>b</td></tr><tr><td width=300>c</td></tr><tr><td width=100>d</td><td>e</td></tr></table>");
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    assert_eq!(t.widths, [75.0, 150.0]);
}

#[test]
fn col_elements_give_table_column_widths() {
    let f = parse("<table><colgroup><col width=100><col style='width: 150pt'></colgroup><tr><td>A</td><td>B</td></tr></table>");
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    assert_eq!(t.widths, [75.0, 150.0]);

    // col with span
    let f2 = parse(
        "<table><colgroup><col span=2 style='width: 72pt'><col style='width: 144pt'></colgroup><tr><td>1</td><td>2</td><td>3</td></tr></table>",
    );
    let FBlock::Table(t2) = &f2.blocks[0] else { panic!() };
    assert_eq!(t2.widths, [72.0, 72.0, 144.0]);

    // Childless colgroup defines widths across declared span
    let f3 = parse("<table><colgroup span=2 width=100></colgroup><tr><td>1</td><td>2</td></tr></table>");
    let FBlock::Table(t3) = &f3.blocks[0] else { panic!() };
    assert_eq!(t3.widths, [75.0, 75.0]);

    // Child col elements override colgroup widths
    let f4 = parse("<table><colgroup width=100><col width=200></colgroup><tr><td>1</td></tr></table>");
    let FBlock::Table(t4) = &f4.blocks[0] else { panic!() };
    assert_eq!(t4.widths, [150.0]);

    // Mixed childless colgroup and colgroup with child col elements
    let f5 = parse(
        "<table><colgroup span=1 width=100></colgroup><colgroup><col style='width: 200pt'><col style='width: 300pt'></colgroup><tr><td>1</td><td>2</td><td>3</td></tr></table>",
    );
    let FBlock::Table(t5) = &f5.blocks[0] else { panic!() };
    assert_eq!(t5.widths, [75.0, 200.0, 300.0]);

    // Child col elements inherit parent colgroup width when unset
    let f6 = parse("<table><colgroup width=120><col><col></colgroup><tr><td>1</td><td>2</td></tr></table>");
    let FBlock::Table(t6) = &f6.blocks[0] else { panic!() };
    assert_eq!(t6.widths, [90.0, 90.0]);

    // Child col elements without width inherit group width while explicit child width takes precedence
    let f7 = parse("<table><colgroup width=120><col width=200><col></colgroup><tr><td>1</td><td>2</td></tr></table>");
    let FBlock::Table(t7) = &f7.blocks[0] else { panic!() };
    assert_eq!(t7.widths, [150.0, 90.0]);

    // Partial explicit col widths fall back to average for unset columns
    let f8 = parse("<table><colgroup><col style='width: 100pt'><col></colgroup><tr><td>1</td><td>2</td></tr></table>");
    let FBlock::Table(t8) = &f8.blocks[0] else { panic!() };
    assert_eq!(t8.widths, [100.0, 100.0]);

    // Export includes colgroup
    let doc = model::to_doc(&f);
    let html = export(&doc);
    assert!(html.contains("<colgroup>"), "html export includes colgroup: {html}");
    assert!(html.contains("<col style=\"width:75.00pt\">"), "{html}");
    assert!(html.contains("<col style=\"width:150.00pt\">"), "{html}");
}

#[test]
fn word_html_vertical_merge_is_preserved() {
    use craft_wordmodel::props::VMerge;
    let html = "<table>\
        <tr><td style='mso-yfti-vmerge:restart'>Top</td><td>B1</td></tr>\
        <tr><td style='mso-yfti-vmerge:continue'>Cont</td><td>B2</td></tr>\
    </table>";
    let f = parse(html);
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.rows[0].len(), 2);
    assert_eq!(t.rows[1].len(), 2, "second row must have exactly 2 cells, no duplicate synthetic cell");
    assert!(!t.rows[0][0].covered);
    assert!(t.rows[1][0].covered);

    let doc = model::to_doc(&f);
    let craft_wordmodel::Block::Table(tb) = &*doc.body[0] else { panic!() };
    assert_eq!(tb.rows[0].cells[0].props.vmerge, VMerge::Restart);
    assert_eq!(tb.rows[1].cells[0].props.vmerge, VMerge::Continue);
    assert_eq!(tb.rows[1].cells.len(), 2, "table model second row has exactly 2 cells");
    assert_eq!(tb.vertical_extent(0, 0), Some((0, 2)));
}

#[test]
fn html_multiple_colgroups_do_not_backfill_earlier_groups() {
    // Earlier colgroup with unset column must not be backfilled by later colgroup width.
    let html = "<table>\
        <colgroup><col></colgroup>\
        <colgroup style='width: 200pt'><col style='width: 50pt'></colgroup>\
        <tr><td>1</td><td>2</td></tr>\
    </table>";
    let f = parse(html);
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    assert_eq!(t.widths, [50.0, 50.0], "column 0 was not backfilled by group 2 width of 200pt");
}

#[test]
fn word_html_rowspan_with_explicit_vmerge_continue_no_duplicate_cell() {
    use craft_wordmodel::props::VMerge;
    let html = "<table>\
        <tr><td rowspan=2 style='mso-yfti-vmerge:restart'>A</td><td>B1</td></tr>\
        <tr><td style='mso-yfti-vmerge:continue'>A_cont</td><td>B2</td></tr>\
    </table>";
    let f = parse(html);
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.rows[0].len(), 2);
    assert_eq!(t.rows[1].len(), 2, "second row must have exactly 2 cells, not ragged");
    assert!(!t.rows[0][0].covered);
    assert!(t.rows[1][0].covered);

    let doc = model::to_doc(&f);
    let craft_wordmodel::Block::Table(tb) = &*doc.body[0] else { panic!() };
    assert_eq!(tb.rows[0].cells[0].props.vmerge, VMerge::Restart);
    assert_eq!(tb.rows[1].cells[0].props.vmerge, VMerge::Continue);
    assert_eq!(tb.rows[1].cells.len(), 2);
    assert_eq!(tb.vertical_extent(0, 0), Some((0, 2)));
}

#[test]
fn to_doc_normalises_row_0_continue_cell() {
    use craft_wordmodel::props::VMerge;
    // Case 1: Row 0 has continue and Row 1 has continue -> Row 0 normalised to Restart.
    let html_multi = "<table>\
        <tr><td style='mso-yfti-vmerge:continue'>TopCont</td><td>B1</td></tr>\
        <tr><td style='mso-yfti-vmerge:continue'>BottomCont</td><td>B2</td></tr>\
    </table>";
    let f_multi = parse(html_multi);
    let doc_multi = model::to_doc(&f_multi);
    let craft_wordmodel::Block::Table(tb_multi) = &*doc_multi.body[0] else { panic!() };
    assert_eq!(tb_multi.rows[0].cells[0].props.vmerge, VMerge::Restart, "row 0 continue normalised to restart");
    assert_eq!(tb_multi.rows[1].cells[0].props.vmerge, VMerge::Continue);

    // Case 2: Row 0 has continue but table has only 1 row (or no continuation) -> normalised to None.
    let html_single = "<table>\
        <tr><td style='mso-yfti-vmerge:continue'>SoloCont</td><td>B1</td></tr>\
    </table>";
    let f_single = parse(html_single);
    let doc_single = model::to_doc(&f_single);
    let craft_wordmodel::Block::Table(tb_single) = &*doc_single.body[0] else { panic!() };
    assert_eq!(tb_single.rows[0].cells[0].props.vmerge, VMerge::None, "orphaned row 0 continue normalised to none");

    // Case 3: Row 0 col 1 has continue, Row 1 has a spanning continue starting at col 0 -> col 1 normalised to None.
    let html_span = "<table>\
        <tr><td>A0</td><td style='mso-yfti-vmerge:continue'>B0</td></tr>\
        <tr><td colspan='2' style='mso-yfti-vmerge:continue'>SpanCont</td></tr>\
    </table>";
    let f_span = parse(html_span);
    let doc_span = model::to_doc(&f_span);
    let craft_wordmodel::Block::Table(tb_span) = &*doc_span.body[0] else { panic!() };
    assert_eq!(tb_span.rows[0].cells[1].props.vmerge, VMerge::None, "unaligned spanning continuation in row 1 must not mark row 0 cell as restart");
}

/// #1785: A covered continuation cell with `rowspan > 1` expands into the following row when
/// no restart cell above spans it.
#[test]
fn covered_continuation_with_rowspan_expands_into_next_row() {
    use craft_wordmodel::props::VMerge;
    let html = "<table>\
        <tr><td style='mso-yfti-vmerge:restart'>Top</td><td>B0</td></tr>\
        <tr><td rowspan='2' style='mso-yfti-vmerge:continue'>Mid</td><td>B1</td></tr>\
        <tr><td>B2</td></tr>\
    </table>";
    let f = parse(html);
    let doc = model::to_doc(&f);
    let craft_wordmodel::Block::Table(tb) = &*doc.body[0] else { panic!() };
    assert_eq!(tb.rows.len(), 3);
    assert_eq!(tb.rows[2].cells.len(), 2, "row 2 should have placeholder + B2");
    assert_eq!(tb.rows[2].cells[0].props.vmerge, VMerge::Continue);
}

/// #1785: A plain `rowspan=2` cell `A` in col 0 followed by `B` (`vmerge:restart`) and `D`
/// must still insert a placeholder for `A` in row 1 even when `B`'s explicit `vmerge:continue`
/// is the first `<td>` in row 1.
#[test]
fn rowspan_before_explicit_vmerge_continue_keeps_grid_columns() {
    use craft_wordmodel::props::VMerge;
    let html = "<table>\
        <tr><td rowspan='2'>A</td><td rowspan='2' style='mso-yfti-vmerge:restart'>B</td><td>D</td></tr>\
        <tr><td style='mso-yfti-vmerge:continue'>B_cont</td><td>D1</td></tr>\
    </table>";
    let f = parse(html);
    let doc = model::to_doc(&f);
    let craft_wordmodel::Block::Table(tb) = &*doc.body[0] else { panic!() };
    assert_eq!(tb.rows.len(), 2);
    assert_eq!(tb.rows[0].cells.len(), 3);
    assert_eq!(tb.rows[1].cells.len(), 3, "row 1 should have placeholder for A, B_cont, and D1");
    assert_eq!(tb.rows[1].cells[0].props.vmerge, VMerge::Continue);
    assert_eq!(tb.rows[1].cells[1].props.vmerge, VMerge::Continue);
    let craft_wordmodel::Block::Para(p1) = &*tb.rows[1].cells[1].blocks[0] else { panic!() };
    assert_eq!(p1.text, "B_cont");
    assert_eq!(tb.rows[1].cells[2].props.vmerge, VMerge::None);
    let craft_wordmodel::Block::Para(p2) = &*tb.rows[1].cells[2].blocks[0] else { panic!() };
    assert_eq!(p2.text, "D1");
}

#[test]
fn cells_take_their_css_borders() {
    use craft_wordmodel::props::{Border, BorderStyle};
    let f = parse(
        "<style>td.b { border-top: solid windowtext 1px; border-bottom: 2pt double #ff0000; border-left: none; border-right: 0px solid; vertical-align: bottom }</style><table><tr><td class=b>a</td><td>b</td></tr></table>",
    );
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    // The style sheet's other cell properties apply too.
    assert_eq!(t.rows[0][0].valign, Some(craft_wordmodel::props::VAlign::Bottom));
    let b = t.rows[0][0].borders.expect("borders");
    assert_eq!(b.top, Some(Border::single(0.75)));
    assert_eq!(b.bottom, Some(Border { style: BorderStyle::Double, width: 2.0, color: Some(craft_wordmodel::Rgb(255, 0, 0)), space: 0.0 }));
    assert_eq!(b.left.map(|l| l.style), Some(BorderStyle::None));
    // A border of no width is none (OOXML has no zero-width line).
    assert_eq!(b.right.map(|r| r.style), Some(BorderStyle::None));
    assert_eq!(t.rows[0][1].borders, None);
}

#[test]
fn style_sheet_edge_cases() {
    let f = parse_in(
        "<style>@charset \"utf-8\"; @import url(a.css); p.a { color: #0000ff } body { margin: 20px } p.b { margin: 10 5% medium }</style>\
         <body><p class=a>a</p>bare<p class=b>b</p></body>",
        &|_| None,
        true,
    );
    let ps = paras(&f);
    // A rule right after statement at-rules still applies.
    assert_eq!(fmt_of(ps[0]).color, Some(craft_wordmodel::Rgb(0, 0, 255)));
    // The body's margins aren't the paragraphs' indents.
    assert_eq!((ps[1].spacing.left, ps[1].spacing.before), (None, None));
    // Bare numbers, percentages and keywords state no margin: automatic spacing stays.
    assert_eq!((ps[2].spacing.before_auto, ps[2].spacing.left), (true, None));
    // The `style` attribute wins even over a sheet with more declarations than are read.
    let many = (0..600).map(|i| format!("x-{i}: 1")).collect::<Vec<_>>().join(";");
    let f = parse(&format!("<style>p {{ {many}; color: #ff0000 }}</style><p style='color: #00ff00'>c</p>"));
    assert_eq!(fmt_of(paras(&f)[0]).color, Some(craft_wordmodel::Rgb(0, 255, 0)));
}

#[test]
fn hostile_style_sheets_never_panic() {
    // One declaration block shared by many selectors isn't copied for each.
    let selectors = vec!["p"; 20_000].join(",");
    let huge = format!("<style>{selectors} {{ color: red; {} }}</style><p>x</p>", "x: y;".repeat(200_000));
    let _ = parse_in(&huge, &|_| None, true);
    let many = ".a { color: red } ".repeat(20_000);
    for css in ["{", "}", "p {", "@media { p { x } ", "p { color: red", "/* open", ".{}", "p..a{}", "\u{0}.é{x:y}", many.as_str()] {
        let _ = parse_in(&format!("<style>{css}</style><p class='a é'>x</p><td style='width: 1e40px'>y</td>"), &|_| None, true);
    }
}

/// A list item's further paragraph keeps its alignment (and page break) in its item.
#[test]
fn an_items_further_paragraph_keeps_its_alignment() {
    let mut one = Para { list: Some(model::ListInfo { ordered: true, ..Default::default() }), ..Default::default() };
    one.push_text("One", &Fmt::default());
    let mut cont = Para { item: Some(0), align: Some(Align::Center), ..Default::default() };
    cont.spacing.left = Some(36.0);
    cont.push_text("Cont", &Fmt::default());
    let html = export(&model::to_doc(&Flow { blocks: vec![FBlock::Para(one), FBlock::Para(cont)], meta: Meta::default() }));
    assert!(html.contains("<li>One<p style=\"text-align:center\">Cont</p></li>"), "{html}");
}

#[test]
fn conflicting_mso_vmerge_declarations_last_wins() {
    let html_restart_wins = "<table><tr><td style='mso-yfti-vmerge: continue; mso-yfti-vmerge: restart'>A</td></tr></table>";
    let f1 = parse(html_restart_wins);
    let FBlock::Table(t1) = &f1.blocks[0] else { panic!() };
    assert!(t1.rows[0][0].vmerge_restart);
    assert!(!t1.rows[0][0].covered);

    let html_continue_wins = "<table><tr><td style='mso-yfti-vmerge: restart; mso-yfti-vmerge: continue'>B</td></tr></table>";
    let f2 = parse(html_continue_wins);
    let FBlock::Table(t2) = &f2.blocks[0] else { panic!() };
    assert!(!t2.rows[0][0].vmerge_restart);
    assert!(t2.rows[0][0].covered);
}

#[test]
fn export_flow_derives_rowspan_for_vmerge_continuations() {
    let html = "<table>\
        <tr><td style='mso-yfti-vmerge:restart'>Top</td><td>B1</td></tr>\
        <tr><td style='mso-yfti-vmerge:continue'>Cont</td><td>B2</td></tr>\
    </table>";
    let f = parse(html);
    let exported = export_flow(&f, "en");
    assert!(exported.contains("<td rowspan=\"2\""), "must derive rowspan=\"2\" for restart cell: {exported}");
    assert!(!exported.contains("Cont</td>"), "covered continuation cell should not be emitted: {exported}");
    assert!(exported.contains(">B1</td>"), "{exported}");
    assert!(exported.contains(">B2</td>"), "{exported}");
}

#[test]
fn export_flow_preserves_orphan_covered_continuation_content() {
    let html = "<table>\
        <tr><td style='mso-yfti-vmerge:continue'>Orphan0</td><td>B0</td></tr>\
        <tr><td>A1</td><td>B1</td></tr>\
    </table>";
    let f = parse(html);
    let exported = export_flow(&f, "en");
    assert!(exported.contains("Orphan0</td>"), "orphan continuation content in row 0 must be preserved: {exported}");
}

#[test]
fn html_roundtrip_preserves_colgroups_widths_and_vmerge() {
    use craft_wordmodel::props::VMerge;

    let html = "<table>\
        <colgroup><col style='width: 80pt'></colgroup>\
        <colgroup><col style='width: 120pt'></colgroup>\
        <tr><td style='mso-yfti-vmerge:restart'>MergeA</td><td>B1</td></tr>\
        <tr><td style='mso-yfti-vmerge:continue'>MergeA_cont</td><td>B2</td></tr>\
    </table>";

    let f1 = parse(html);
    let FBlock::Table(t1) = &f1.blocks[0] else { panic!() };
    assert_eq!(t1.widths, [80.0, 120.0]);
    assert_eq!(t1.rows.len(), 2);
    assert_eq!(t1.rows[0].len(), 2);
    assert_eq!(t1.rows[1].len(), 2);
    assert!(!t1.rows[0][0].covered);
    assert!(t1.rows[0][0].vmerge_restart);
    assert!(t1.rows[1][0].covered);

    let doc1 = model::to_doc(&f1);
    let craft_wordmodel::Block::Table(tb1) = &*doc1.body[0] else { panic!() };
    assert_eq!(tb1.rows[0].cells[0].props.vmerge, VMerge::Restart);
    assert_eq!(tb1.rows[1].cells[0].props.vmerge, VMerge::Continue);
    assert_eq!(tb1.vertical_extent(0, 0), Some((0, 2)));

    // Export to HTML via export()
    let exported = export(&doc1);
    assert!(exported.contains("<colgroup>"), "exported HTML must contain colgroup: {exported}");
    assert!(exported.contains("width:80.00pt"), "{exported}");
    assert!(exported.contains("width:120.00pt"), "{exported}");
    assert!(exported.contains("rowspan=\"2\""), "exported HTML must emit rowspan=\"2\": {exported}");

    // Parse the exported HTML again
    let f2 = parse(&exported);
    let FBlock::Table(t2) = &f2.blocks[0] else { panic!() };
    assert_eq!(t2.widths, t1.widths, "column widths must match");
    assert_eq!(t2.rows.len(), t1.rows.len(), "row counts must match");
    assert_eq!(t2.rows[0].len(), t1.rows[0].len(), "row 0 cell count must match");
    assert_eq!(t2.rows[1].len(), t1.rows[1].len(), "row 1 cell count must match");
    assert_eq!(t2.rows[0][0].covered, t1.rows[0][0].covered, "row 0 cell 0 covered flag must match");
    assert_eq!(t2.rows[1][0].covered, t1.rows[1][0].covered, "row 1 cell 0 covered flag must match");

    let doc2 = model::to_doc(&f2);
    let craft_wordmodel::Block::Table(tb2) = &*doc2.body[0] else { panic!() };
    assert_eq!(tb2.rows[0].cells[0].props.vmerge, tb1.rows[0].cells[0].props.vmerge, "vmerge restart matches");
    assert_eq!(tb2.rows[1].cells[0].props.vmerge, tb1.rows[1].cells[0].props.vmerge, "vmerge continue matches");
    assert_eq!(tb2.vertical_extent(0, 0), tb1.vertical_extent(0, 0), "vertical merge extent matches");
}

fn assert_single_col_vmerge_table(tb: &craft_wordmodel::Table) {
    use craft_wordmodel::props::VMerge;
    assert_eq!(tb.rows.len(), 3, "table must have 3 rows");
    assert_eq!(tb.grid.len(), 1, "table must have 1 column");
    assert_eq!(tb.rows[0].cells.len(), 1);
    assert_eq!(tb.rows[1].cells.len(), 1);
    assert_eq!(tb.rows[2].cells.len(), 1);
    assert_eq!(tb.rows[0].cells[0].props.vmerge, VMerge::Restart);
    assert_eq!(tb.rows[1].cells[0].props.vmerge, VMerge::Continue);
    assert_eq!(tb.rows[2].cells[0].props.vmerge, VMerge::None);
    assert_eq!(tb.vertical_extent(0, 0), Some((0, 2)));
    assert_eq!(tb.grid_col(2, 0), 0, "Bottom cell must be at grid column 0");
    let bottom_text = match &*tb.rows[2].cells[0].blocks[0] {
        craft_wordmodel::Block::Para(p) => &p.text,
        _ => panic!("expected paragraph in Bottom cell"),
    };
    assert_eq!(bottom_text, "Bottom", "Bottom must be at row 2, cell 0");
}

#[test]
fn html_roundtrip_preserves_fully_covered_row_and_single_column_table() {
    // A one-column table with a Word vertical merge across the first two rows,
    // followed by an ordinary "Bottom" cell in row three.
    let html = "<table>\
        <tr><td style='mso-yfti-vmerge:restart'>Top</td></tr>\
        <tr><td style='mso-yfti-vmerge:continue'>Cont</td></tr>\
        <tr><td>Bottom</td></tr>\
    </table>";

    let f1 = parse(html);
    let FBlock::Table(t1) = &f1.blocks[0] else { panic!() };
    assert_eq!(t1.rows.len(), 3, "initial import must have 3 rows");
    assert_eq!(t1.cols(), 1, "initial import must have 1 column");
    assert_eq!(t1.rows[0].len(), 1);
    assert_eq!(t1.rows[1].len(), 1);
    assert_eq!(t1.rows[2].len(), 1);
    assert!(!t1.rows[0][0].covered);
    assert!(t1.rows[0][0].vmerge_restart);
    assert!(t1.rows[1][0].covered);
    assert!(!t1.rows[2][0].covered);

    let doc1 = model::to_doc(&f1);
    let craft_wordmodel::Block::Table(tb1) = &*doc1.body[0] else { panic!() };
    assert_single_col_vmerge_table(tb1);

    // Export to HTML via export()
    let exported = export(&doc1);
    assert!(exported.contains("rowspan=\"2\""), "exported HTML must emit rowspan=\"2\": {exported}");
    assert!(exported.contains("<tr></tr>"), "covered row with no visible cells must emit empty tr: {exported}");
    assert!(exported.contains("Bottom"), "exported HTML must contain Bottom: {exported}");

    // Parse the exported HTML again
    let f2 = parse(&exported);
    let FBlock::Table(t2) = &f2.blocks[0] else { panic!() };
    assert_eq!(t2.rows.len(), 3, "round-trip Flow must keep 3 rows");
    assert_eq!(t2.cols(), 1, "round-trip Flow must keep 1 column");
    assert_eq!(t2.rows[0].len(), 1, "row 0 has 1 cell");
    assert_eq!(t2.rows[1].len(), 1, "row 1 (fully covered row) has 1 cell");
    assert_eq!(t2.rows[2].len(), 1, "row 2 has 1 cell");
    assert!(!t2.rows[0][0].covered);
    assert!(t2.rows[1][0].covered, "row 1 cell must be covered placeholder");
    assert!(!t2.rows[2][0].covered, "row 2 cell (Bottom) must not be covered");

    // Convert round-tripped flow to document model
    let doc2 = model::to_doc(&f2);
    let craft_wordmodel::Block::Table(tb2) = &*doc2.body[0] else { panic!() };
    assert_single_col_vmerge_table(tb2);
}

#[test]
fn vmerge_validation_requires_matching_spans() {
    use craft_wordmodel::props::VMerge;

    // 1. Later-row check: row 0 has colspan=2, row 1 has continuation with colspan=1.
    // Mismatched spans must clear continuation to None and NOT promote row 0 cell to Restart.
    let ft_mismatched = FTable {
        rows: vec![
            vec![Cell { colspan: 2, ..Cell::default() }],
            vec![Cell { colspan: 1, covered: true, ..Cell::default() }, Cell { colspan: 1, ..Cell::default() }],
        ],
        widths: vec![50.0, 50.0],
        ..Default::default()
    };
    let doc = model::to_doc(&Flow { blocks: vec![FBlock::Table(ft_mismatched)], ..Default::default() });
    let craft_wordmodel::Block::Table(tb) = &*doc.body[0] else { panic!() };
    assert_eq!(tb.rows[0].cells[0].props.vmerge, VMerge::None, "mismatched span must not promote to Restart");
    assert_eq!(tb.rows[1].cells[0].props.vmerge, VMerge::None, "mismatched span must clear Continue to None");

    // 2. First-row check: row 0 has continuation with colspan=2, row 1 has continuation with colspan=1.
    // Mismatched spans must clear row 0 continuation to None, not promote to Restart.
    let ft_row0_mismatched = FTable {
        rows: vec![
            vec![Cell { colspan: 2, covered: true, ..Cell::default() }],
            vec![Cell { colspan: 1, covered: true, ..Cell::default() }, Cell { colspan: 1, ..Cell::default() }],
        ],
        widths: vec![50.0, 50.0],
        ..Default::default()
    };
    let doc0 = model::to_doc(&Flow { blocks: vec![FBlock::Table(ft_row0_mismatched)], ..Default::default() });
    let craft_wordmodel::Block::Table(tb0) = &*doc0.body[0] else { panic!() };
    assert_eq!(tb0.rows[0].cells[0].props.vmerge, VMerge::None, "mismatched span in row 0 must clear to None");
    assert_eq!(tb0.rows[1].cells[0].props.vmerge, VMerge::None, "mismatched span in row 1 must clear to None");

    // 3. Matching spans: row 0 has colspan=2, row 1 has continuation with colspan=2.
    // Matching spans must promote row 0 to Restart and keep row 1 as Continue.
    let ft_matching = FTable {
        rows: vec![vec![Cell { colspan: 2, ..Cell::default() }], vec![Cell { colspan: 2, covered: true, ..Cell::default() }]],
        widths: vec![50.0, 50.0],
        ..Default::default()
    };
    let doc_m = model::to_doc(&Flow { blocks: vec![FBlock::Table(ft_matching)], ..Default::default() });
    let craft_wordmodel::Block::Table(tb_m) = &*doc_m.body[0] else { panic!() };
    assert_eq!(tb_m.rows[0].cells[0].props.vmerge, VMerge::Restart);
    assert_eq!(tb_m.rows[1].cells[0].props.vmerge, VMerge::Continue);
}

#[test]
fn export_table_preserves_first_grid_column_alignment() {
    let mut doc = craft_wordmodel::Document::default();
    let mut table = craft_wordmodel::Table::new(2, 3, 240.0);
    table.grid = vec![40.0, 80.0, 120.0];
    table.rows[0].cells = vec![craft_wordmodel::Cell::with_text("A"), craft_wordmodel::Cell::with_text("B"), craft_wordmodel::Cell::with_text("C")];
    table.rows[1].props.grid_before = 1;
    table.rows[1].cells = vec![craft_wordmodel::Cell::with_text("D"), craft_wordmodel::Cell::with_text("E")];
    doc.body = vec![std::sync::Arc::new(craft_wordmodel::Block::Table(table))];

    // Flow representation should preserve the leading grid position
    let flow = model::from_doc(&doc);
    let FBlock::Table(ft) = &flow.blocks[0] else { panic!() };
    assert_eq!(ft.rows.len(), 2);
    assert_eq!(ft.rows[0].len(), 3);
    assert_eq!(ft.rows[1].len(), 3, "row 1 must have 3 cells (1 leading placeholder + 2 data cells)");
    assert_eq!(ft.widths, vec![40.0, 80.0, 120.0]);

    // HTML export should emit 3 colgroup cols and preserve cell alignment
    let html = export(&doc);
    assert!(html.contains("<col style=\"width:40.00pt\">"), "must contain col 0 width: {html}");
    assert!(html.contains("<col style=\"width:80.00pt\">"), "must contain col 1 width: {html}");
    assert!(html.contains("<col style=\"width:120.00pt\">"), "must contain col 2 width: {html}");
    assert!(html.contains(">D</td>"), "must contain cell D: {html}");
    assert!(html.contains(">E</td>"), "must contain cell E: {html}");
}

/// Ensure container elements (such as `div` and `p`) with background colors do not leak
/// character shading onto their child text runs (#1786).
#[test]
fn container_background_color_does_not_leak_into_character_shading() {
    let html = r#"<div style="background-color:#1e1e1e;color:#d4d4d4"><span>code</span></div>"#;
    let doc = import(html);
    let p = match &*doc.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert!(!p.runs.is_empty(), "expected nonempty runs");
    assert_eq!(&p.text, "code");
    assert_eq!(p.props.shading, Some(craft_wordmodel::Rgb(0x1e, 0x1e, 0x1e)), "container div background is preserved as paragraph shading");
    for r in &p.runs {
        assert_eq!(r.props.shading, None, "character should have no shading from container div");
    }
}

/// Verify that table background colors cascade down to cell shading through implicit or explicit
/// row groups (`tbody`, `thead`, `tfoot`), but do not leak into cell character shading.
#[test]
fn table_background_inherits_through_row_groups() {
    let html = r#"<table style="background-color:#e8f0fa"><tbody><tr><td>cell1</td></tr></tbody></table>
                  <table><tbody style="background-color:#d0e0f0"><tr><td>cell2</td></tr></tbody></table>
                  <table><thead style="background-color:#c0d0e0"><tr><th>head</th></tr></thead></table>"#;
    let doc = import(html);
    let tables: Vec<_> = doc
        .body
        .iter()
        .filter_map(|b| match &**b {
            craft_wordmodel::Block::Table(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 3);
    for (t, want_hex) in tables.iter().zip(["e8f0fa", "d0e0f0", "c0d0e0"]) {
        let cell = &t.rows[0].cells[0];
        assert_eq!(
            cell.props.shading.map(|c| c.hex().to_ascii_lowercase()),
            Some(want_hex.to_string()),
            "cell should inherit shading from table or row group"
        );
        let p = &cell.blocks[0];
        let craft_wordmodel::Block::Para(p) = &**p else {
            panic!("expected table cell block to be a paragraph");
        };
        assert!(!p.runs.is_empty(), "expected nonempty runs");
        for r in &p.runs {
            assert_eq!(r.props.shading, None, "cell text should have no character shading");
        }
    }
}

/// Verify that block-level container backgrounds (`p`, `pre`, `div`) are preserved as paragraph
/// shading on paragraphs rather than character shading on individual runs (#1786).
#[test]
fn block_element_backgrounds_preserve_paragraph_shading() {
    let html = r#"<p style="background:#1e1e1e;color:white">text</p><pre style="background:#2a2a2a">code</pre><div style="background:#3a3a3a"><p>inner</p></div>"#;
    let doc = import(html);
    assert_eq!(doc.body.len(), 3);
    let craft_wordmodel::Block::Para(p1) = &*doc.body[0] else { panic!("expected Para") };
    let craft_wordmodel::Block::Para(p2) = &*doc.body[1] else { panic!("expected Para") };
    let craft_wordmodel::Block::Para(p3) = &*doc.body[2] else { panic!("expected Para") };
    assert_eq!(p1.props.shading, Some(craft_wordmodel::Rgb(0x1e, 0x1e, 0x1e)));
    assert_eq!(p2.props.shading, Some(craft_wordmodel::Rgb(0x2a, 0x2a, 0x2a)));
    assert_eq!(p3.props.shading, Some(craft_wordmodel::Rgb(0x3a, 0x3a, 0x3a)));
    assert_eq!(&p1.text, "text");
    assert_eq!(&p2.text, "code");
    assert_eq!(&p3.text, "inner");
    assert!(!p1.runs.is_empty());
    assert!(!p2.runs.is_empty());
    assert!(!p3.runs.is_empty());
    for r in &p1.runs {
        assert_eq!(r.props.shading, None, "character should have no shading from paragraph background");
    }
    for r in &p2.runs {
        assert_eq!(r.props.shading, None, "character should have no shading from pre background");
    }
    for r in &p3.runs {
        assert_eq!(r.props.shading, None, "character should have no shading from container div background");
    }
}

/// Verify that paragraph shading created by a container background survives an HTML round trip (#1786).
#[test]
fn paragraph_shading_survives_an_html_round_trip() {
    let html = r#"<div style="background-color:#1e1e1e"><p>code</p></div><pre style="background-color:#2a2a2a">pre</pre>"#;
    let doc = import(html);
    let out = export(&doc);
    assert!(out.contains("background-color:#1E1E1E"), "paragraph shading is exported: {out}");
    assert!(out.contains("background-color:#2A2A2A"), "code block shading is exported: {out}");

    let back = import(&out);
    let craft_wordmodel::Block::Para(p1) = &*back.body[0] else { panic!("expected Para") };
    let craft_wordmodel::Block::Para(p2) = &*back.body[1] else { panic!("expected Para") };
    assert_eq!(p1.props.shading, Some(craft_wordmodel::Rgb(0x1e, 0x1e, 0x1e)));
    assert_eq!(p2.props.shading, Some(craft_wordmodel::Rgb(0x2a, 0x2a, 0x2a)));
}

/// Verify that paragraph shading inside a single-paragraph table cell survives an HTML round trip (#1786).
#[test]
fn table_cell_paragraph_shading_survives_an_html_round_trip() {
    let html = r#"<table><tr><td><p style="background-color:#123456">text</p></td></tr></table>"#;
    let doc = import(html);
    let out = export(&doc);
    assert!(out.contains("background-color:#123456"), "paragraph shading is exported: {out}");

    let back = import(&out);
    let table = back
        .body
        .iter()
        .find_map(|b| match &**b {
            craft_wordmodel::Block::Table(t) => Some(t),
            _ => None,
        })
        .expect("expected table");
    let cell = &table.rows[0].cells[0];
    assert_eq!(cell.props.shading, None, "paragraph shading must not become cell shading");
    let craft_wordmodel::Block::Para(p) = &*cell.blocks[0] else { panic!("expected Para") };
    assert_eq!(p.props.shading, Some(craft_wordmodel::Rgb(0x12, 0x34, 0x56)));
}

/// Verify that consecutive code paragraphs with different backgrounds export their own `<pre>`.
#[test]
fn consecutive_code_blocks_keep_their_own_shading() {
    let html = r#"<pre style="background-color:#111111">a</pre><pre style="background-color:#222222">b</pre>"#;
    let out = export(&import(html));
    assert!(out.contains("<pre style=\"background-color:#111111\"><code>a</code></pre>"), "{out}");
    assert!(out.contains("<pre style=\"background-color:#222222\"><code>b</code></pre>"), "{out}");
}

/// Verify that paragraph shading inherited from a paragraph style is exported (#1786).
#[test]
fn paragraph_shading_from_a_style_survives_html_export() {
    let mut doc = Document::new();
    std::sync::Arc::make_mut(&mut doc.styles).styles.push(craft_wordmodel::Style {
        id: "Shaded".into(),
        name: "Shaded".into(),
        para: craft_wordmodel::props::ParaProps { shading: Some(craft_wordmodel::Rgb(0x12, 0x34, 0x56)), ..Default::default() },
        ..Default::default()
    });
    let mut p = craft_wordmodel::Paragraph::with_text("text", craft_wordmodel::CharProps::default());
    p.props.style = Some("Shaded".into());
    doc.body = vec![craft_wordmodel::para_block(p)];

    let out = export(&doc);
    assert!(out.contains("background-color:#123456"), "style shading is exported: {out}");
}

/// A closing parenthesis without an opening one invalidates the declaration (#1786).
#[test]
fn background_shorthand_unbalanced_closing_paren_is_ignored() {
    let html = r#"<span style="background-color:#ff0000"><span style="background: ) none repeat">text</span></span>"#;
    let doc = import(html);
    let craft_wordmodel::Block::Para(p) = &*doc.body[0] else { panic!("expected Para") };
    assert_eq!(p.runs[0].props.shading, Some(craft_wordmodel::Rgb(0xff, 0x00, 0x00)), "invalid shorthand must not clear inherited shading");
}

/// A shaded paragraph keeps its shading on both sides of an internal page break (#1786).
#[test]
fn paragraph_shading_survives_an_internal_page_break() {
    let mut p = craft_wordmodel::Paragraph::with_text("before\u{000C}after", craft_wordmodel::CharProps::default());
    p.props.shading = Some(craft_wordmodel::Rgb(0x12, 0x34, 0x56));
    let mut doc = Document::new();
    doc.body = vec![craft_wordmodel::para_block(p)];
    let out = export(&doc);
    assert_eq!(out.matches("background-color:#123456").count(), 2, "{out}");
}

/// Verify that CSS-escaped quote characters in background shorthand URLs remain part of the URL
/// content and do not cause the shorthand declaration to be rejected.
#[test]
fn background_shorthand_escaped_quotes_in_url() {
    let doc = import(r#"<p><span style='background: #123456 url("a\"b.png")'>text</span></p>"#);
    let craft_wordmodel::Block::Para(p) = &*doc.body[0] else { panic!("expected Para") };
    assert_eq!(&p.text, "text");
    assert!(!p.runs.is_empty());
    assert_eq!(p.runs[0].props.shading, Some(craft_wordmodel::Rgb(0x12, 0x34, 0x56)));
}

/// Verify that valid multi-token CSS background shorthands without a color (such as `none repeat`)
/// reset character shading, while invalid declarations are ignored.
#[test]
fn background_shorthand_without_color_resets_background() {
    let html = r#"<span style="background-color: #ff0000"><span style="background: none repeat">plain</span></span>"#;
    let doc = import(html);
    let p = match &*doc.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p.runs[0].props.shading, None);

    let html_invalid = r#"<span style="background-color: #ff0000"><span style="background: none invalid_xyz">shaded</span></span>"#;
    let doc_invalid = import(html_invalid);
    let p_invalid = match &*doc_invalid.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_invalid.runs[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));
}

/// Verify that CSS `background: inherit` keeps inherited character shading instead of removing it (#1786).
#[test]
fn background_inherit_retains_inline_shading() {
    let html = r#"<span style="background-color: #ff0000"><span style="background: inherit">shaded</span></span>"#;
    let doc = import(html);
    let p = match &*doc.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p.runs[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)), "background: inherit should retain inherited character shading");

    // Also verify background-color: inherit retains shading.
    let html_color = r#"<span style="background-color: #ff0000"><span style="background-color: inherit">shaded</span></span>"#;
    let doc_color = import(html_color);
    let p_color = match &*doc_color.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(
        p_color.runs[0].props.shading,
        Some(craft_wordmodel::Rgb(255, 0, 0)),
        "background-color: inherit should retain inherited character shading"
    );

    // CSS-wide keyword inherit mixed with other values is invalid and ignored.
    let html_mixed = r#"<span style="background-color: #00ff00"><span style="background: red inherit">green</span></span>"#;
    let doc_mixed = import(html_mixed);
    let p_mixed = match &*doc_mixed.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(
        p_mixed.runs[0].props.shading,
        Some(craft_wordmodel::Rgb(0, 255, 0)),
        "invalid shorthand mixing inherit with values should be ignored"
    );
}

/// Verify that hostile background declarations with deeply nested slashes do not cause stack overflow (#1786).
#[test]
fn hostile_background_slashes_never_stack_overflow() {
    let slashes = "/".repeat(20_000);
    let html = format!(r#"<span style="background: {slashes}">hostile</span>"#);
    let doc = import(&html);
    let p = match &*doc.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p.runs[0].props.shading, None);
}

/// Verify that non-breaking space (U+00A0) is not treated as CSS whitespace when splitting tokens (#1786).
#[test]
fn background_whitespace_preserves_nbsp() {
    let html = "<span style=\"background-color: #ff0000\"><span style=\"background: none\u{00A0}repeat\">shaded</span></span>";
    let doc = import(html);
    let p = match &*doc.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(
        p.runs[0].props.shading,
        Some(craft_wordmodel::Rgb(255, 0, 0)),
        "non-breaking space is not CSS whitespace and should not clear shading"
    );
}

/// Verify that background shorthands exceeding the token limit are rejected and unapplied (#1786).
#[test]
fn background_shorthand_truncated_rejects_unapplied() {
    let mut layers = String::new();
    for i in 0..64 {
        layers.push_str(&format!("url(img{i}.png), "));
    }
    layers.push_str("#00ff00");
    let html = format!(r#"<span style="background-color: #ff0000"><span style="background: {layers}">retained</span></span>"#);
    let doc = import(&html);
    let p = match &*doc.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(
        p.runs[0].props.shading,
        Some(craft_wordmodel::Rgb(255, 0, 0)),
        "truncated shorthand should be rejected and leave inherited background untouched"
    );
}

/// Verify that shorthands with image-set or calc functions parse and preserve colors (#1786).
#[test]
fn background_shorthand_valid_functions_preserve_color() {
    let html_image_set = r#"<span style="background: #123456 image-set(url(a.png) 1x)">image-set</span>"#;
    let doc = import(html_image_set);
    let p = match &*doc.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p.runs[0].props.shading, Some(craft_wordmodel::Rgb(0x12, 0x34, 0x56)));

    let html_calc = r#"<span style="background: #123456 url(a.png) calc(50% - 2px)">calc</span>"#;
    let doc_calc = import(html_calc);
    let p_calc = match &*doc_calc.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_calc.runs[0].props.shading, Some(craft_wordmodel::Rgb(0x12, 0x34, 0x56)));
}

/// Verify that multi-layer background shorthands validate slashes per layer and preserve final-layer color (#1786).
#[test]
fn background_shorthand_multi_layer_slashes_apply_color() {
    let html = r#"<span style="background: url(a) center/cover, url(b) center/contain #123456">multi-layer</span>"#;
    let doc = import(html);
    let p = match &*doc.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p.runs[0].props.shading, Some(craft_wordmodel::Rgb(0x12, 0x34, 0x56)));

    // Color in a non-final layer is invalid CSS and must be ignored.
    let html_invalid_color = r#"<span style="background-color: #ff0000"><span style="background: #123456, url(b)">invalid</span></span>"#;
    let doc_invalid_color = import(html_invalid_color);
    let p_invalid_color = match &*doc_invalid_color.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_invalid_color.runs[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));

    // Multiple slashes in a single layer makes the declaration invalid.
    let html_multi_slash =
        r#"<span style="background-color: #ff0000"><span style="background: url(a) center/cover/extra, #123456">invalid</span></span>"#;
    let doc_multi_slash = import(html_multi_slash);
    let p_multi_slash = match &*doc_multi_slash.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_multi_slash.runs[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));
}

/// Verify that background shorthands with empty layers (leading, trailing, or double commas) are rejected (#1786).
#[test]
fn background_shorthand_rejects_empty_layers() {
    // Leading comma
    let html_leading = r#"<span style="background-color: #ff0000"><span style="background: , #00ff00">leading</span></span>"#;
    let doc_leading = import(html_leading);
    let p_leading = match &*doc_leading.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_leading.runs[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));

    // Trailing comma
    let html_trailing = r#"<span style="background-color: #ff0000"><span style="background: transparent,">trailing</span></span>"#;
    let doc_trailing = import(html_trailing);
    let p_trailing = match &*doc_trailing.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_trailing.runs[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));

    // Double comma
    let html_double = r#"<span style="background-color: #ff0000"><span style="background: #00ff00,,#0000ff">double</span></span>"#;
    let doc_double = import(html_double);
    let p_double = match &*doc_double.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_double.runs[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));
}

/// Verify that zero-alpha colors in background shorthands or declarations remain transparent (#1786).
#[test]
fn background_zero_alpha_colors_are_transparent() {
    for decl in ["rgb(0 0 0 / 0)", "rgba(0, 0, 0, 0)", "#0000", "#00000000", "#ffffff00", "hsla(120, 100%, 50%, 0)", "hsl(120 100% 50% / 0)"] {
        let html = format!(r#"<span style="background-color: #ff0000"><span style="background: {decl}">transparent</span></span>"#);
        let doc = import(&html);
        let p = match &*doc.body[0] {
            craft_wordmodel::Block::Para(p) => p,
            _ => panic!("expected paragraph"),
        };
        assert_eq!(p.runs[0].props.shading, None, "zero-alpha color `{decl}` must clear background rather than composite over white");
    }
}

/// Verify that ancestor table shading is preserved beneath transparent table layers (#1786).
#[test]
fn transparent_table_layers_preserve_ancestor_table_shading() {
    let html = r#"<table style="background-color:#e8f0fa">
        <tbody style="background-color:transparent">
            <tr style="background:transparent">
                <td>transparent_layers</td>
                <td style="background-color:#123456">explicit_cell</td>
            </tr>
        </tbody>
    </table>"#;
    let doc = import(html);
    let table = doc
        .body
        .iter()
        .find_map(|b| match &**b {
            craft_wordmodel::Block::Table(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        table.rows[0].cells[0].props.shading,
        Some(craft_wordmodel::Rgb(0xe8, 0xf0, 0xfa)),
        "table background must show through transparent tbody, tr, and td"
    );
    assert_eq!(
        table.rows[0].cells[1].props.shading,
        Some(craft_wordmodel::Rgb(0x12, 0x34, 0x56)),
        "cell with explicit background must override table background"
    );
}

/// Verify that block-level container background is available to immediate inline children using explicit `inherit` (#1786).
#[test]
fn block_background_explicit_inherit() {
    // When the immediate parent <p> has no background, an inline span inheriting background gets transparent (None):
    let html_indirect = r#"<div style="background-color: #ff0000"><p><span style="background-color: inherit">text</span></p></div>"#;
    let doc_indirect = import(html_indirect);
    let p_indirect = match &*doc_indirect.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_indirect.runs[0].props.shading, None);

    // When the span is an immediate child of the styled div, it inherits the div's computed background:
    let html_direct = r#"<div style="background-color: #ff0000"><span style="background-color: inherit">text</span></div>"#;
    let doc_direct = import(html_direct);
    let p_direct = match &*doc_direct.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_direct.runs[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));

    let html_shorthand = r#"<div style="background: #00ff00"><span style="background: inherit">text</span></div>"#;
    let doc_shorthand = import(html_shorthand);
    let p_shorthand = match &*doc_shorthand.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_shorthand.runs[0].props.shading, Some(craft_wordmodel::Rgb(0, 255, 0)));
}

/// Verify that CSS-escaped quotes inside background URLs do not close the quote or drop declaration colors (#1786).
#[test]
fn background_escaped_quote_url() {
    let html = r#"<span style='background: #123456 url("a\"b.png")'>text</span>"#;
    let doc = import(html);
    let p = match &*doc.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p.runs[0].props.shading, Some(craft_wordmodel::Rgb(0x12, 0x34, 0x56)));

    let html_single = r#"<span style="background: #654321 url('a\'b.png')">text</span>"#;
    let doc_single = import(html_single);
    let p_single = match &*doc_single.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_single.runs[0].props.shading, Some(craft_wordmodel::Rgb(0x65, 0x43, 0x21)));
}

/// Verify that invalid hex color declarations like `#ggg0` do not clear inherited shading (#1786).
#[test]
fn background_invalid_hex_retains_shading() {
    let html = r#"<span style="background-color: #ff0000"><span style="background-color: #ggg0">text</span></span>"#;
    let doc = import(html);
    let p = match &*doc.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p.runs[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));
}

/// Verify that background size keywords (`cover`, `contain`) without preceding position and slash are invalid and retain shading (#1786).
#[test]
fn background_cover_without_slash_invalid() {
    let html_cover = r#"<span style="background-color: #ff0000"><span style="background: cover">text</span></span>"#;
    let doc_cover = import(html_cover);
    let p_cover = match &*doc_cover.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_cover.runs[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));

    let html_contain = r#"<span style="background-color: #ff0000"><span style="background: contain">text</span></span>"#;
    let doc_contain = import(html_contain);
    let p_contain = match &*doc_contain.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_contain.runs[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));

    let html_valid = r#"<span style="background-color: #ff0000"><span style="background: center / cover">text</span></span>"#;
    let doc_valid = import(html_valid);
    let p_valid = match &*doc_valid.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_valid.runs[0].props.shading, None);
}

/// Verify that table backgrounds declared on thead or tbody are applied to cells (#1786).
#[test]
fn table_row_group_background_inheritance() {
    let html = r#"<table>
        <thead style="background-color: #00ff00">
            <tr><th>head</th></tr>
        </thead>
        <tbody style="background-color: #0000ff">
            <tr><td>body</td></tr>
        </tbody>
    </table>"#;
    let doc = import(html);
    let table = doc
        .body
        .iter()
        .find_map(|b| match &**b {
            craft_wordmodel::Block::Table(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(table.rows[0].cells[0].props.shading, Some(craft_wordmodel::Rgb(0, 255, 0)));
    assert_eq!(table.rows[1].cells[0].props.shading, Some(craft_wordmodel::Rgb(0, 0, 255)));
}

/// Verify that background shorthand with slashes inside URL functions correctly identifies the position/size slash (#1786).
#[test]
fn background_url_with_slashes_and_position_size() {
    let html = r#"<span style="background: url(/img/bg.png) center/cover #123456">text</span>"#;
    let doc = import(html);
    let p = match &*doc.body[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p.runs[0].props.shading, Some(craft_wordmodel::Rgb(0x12, 0x34, 0x56)));
}

/// Verify that a table inside an inline element with background shading does not turn character shading into table/cell background (#1786).
#[test]
fn table_inside_inline_element_does_not_inherit_character_shading() {
    let html = r#"<a style="background: #ff0000"><table><tr><td>cell</td></tr></table></a>"#;
    let doc = import(html);
    let table = doc
        .body
        .iter()
        .find_map(|b| match &**b {
            craft_wordmodel::Block::Table(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(table.rows[0].cells[0].props.shading, None);
}

/// Verify that a child with `background: inherit` inside an unstyled cell in a shaded table does not inherit the table fallback (#1786).
#[test]
fn cell_inherit_uses_cell_background_not_table_fallback() {
    let html = r#"<table style="background: #ff0000"><tr><td><span style="background: inherit">text</span></td></tr></table>"#;
    let doc = import(html);
    let table = doc
        .body
        .iter()
        .find_map(|b| match &**b {
            craft_wordmodel::Block::Table(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(table.rows[0].cells[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));
    let p = match &*table.rows[0].cells[0].blocks[0] {
        craft_wordmodel::Block::Para(p) => p,
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p.runs[0].props.shading, None);
}

/// Verify that CSS `background: transparent` overrides HTML `bgcolor` presentational attribute (#1786).
#[test]
fn bgcolor_overridden_by_css_transparent() {
    let html = r##"<table bgcolor="#ff0000" style="background: transparent"><tr><td>cell</td></tr></table>"##;
    let doc = import(html);
    let table = doc
        .body
        .iter()
        .find_map(|b| match &**b {
            craft_wordmodel::Block::Table(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(table.rows[0].cells[0].props.shading, None);
}

/// Verify that nested container shading is preserved when merging list continuation paragraphs (#1786).
#[test]
fn list_continuation_preserves_nested_container_shading() {
    let html = r#"<ul><li>before<div style="background:#111111;color:white">inside</div>after</li></ul>"#;
    let flow = parse(html);
    assert_eq!(flow.blocks.len(), 1);
    let FBlock::Para(p) = &flow.blocks[0] else { panic!("expected Para") };
    assert!(p.list.is_some(), "expected list item");
    let dark = Some(craft_wordmodel::Rgb(0x11, 0x11, 0x11));
    let inside = p
        .inlines
        .iter()
        .find_map(|i| match i {
            Inline::Text(t, fmt) if t.contains("inside") => Some(fmt.background),
            _ => None,
        })
        .expect("inside text inline");
    assert_eq!(inside, dark, "nested container shading should be preserved on merged run");

    let doc = import(html);
    let craft_wordmodel::Block::Para(p_doc) = &*doc.body[0] else { panic!("expected Para") };
    let mut offset = 0usize;
    let mut inside_shading = None;
    for r in &p_doc.runs {
        let text_slice = &p_doc.text[offset..offset + r.len];
        if text_slice.contains("inside") {
            inside_shading = Some(r.props.shading);
        } else if text_slice.contains("before") || text_slice.contains("after") {
            assert_eq!(r.props.shading, None);
        }
        offset += r.len;
    }
    assert_eq!(inside_shading, Some(dark));
}

/// Verify that starting a new row group implicitly closes any preceding sibling row group so
/// cell shading is not inherited from an omitted-end-tag preceding group (#1786).
#[test]
fn consecutive_row_groups_with_omitted_end_tag_do_not_leak_shading() {
    let html = r#"<table><tbody style="background:red"><tr><td>a</td></tr><tbody><tr><td>b</td></tr></table>"#;
    let doc = import(html);
    let table = doc
        .body
        .iter()
        .find_map(|b| match &**b {
            craft_wordmodel::Block::Table(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0].cells[0].props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));
    assert_eq!(table.rows[1].cells[0].props.shading, None, "second cell should not inherit shading from closed tbody");
}

/// Verify that a list item with a shaded container followed by unshaded text preserves separate
/// background regions so the trailing text does not inherit the container's background (#1786).
#[test]
fn shaded_container_first_in_list_item_preserves_unshaded_trailing_text() {
    let html = r#"<ul><li style="color:#111111"><div style="background:#111111;color:white">inside</div>after</li></ul>"#;
    let doc = import(html);
    assert_eq!(doc.body.len(), 1);
    let craft_wordmodel::Block::Para(p) = &*doc.body[0] else { panic!("expected Para") };
    assert_eq!(p.props.shading, None, "list paragraph as a whole should have no background when regions differ");
    let dark = Some(craft_wordmodel::Rgb(0x11, 0x11, 0x11));
    let mut offset = 0usize;
    let mut inside_shading = None;
    let mut after_shading = None;
    for r in &p.runs {
        let text_slice = &p.text[offset..offset + r.len];
        if text_slice.contains("inside") {
            inside_shading = Some(r.props.shading);
        } else if text_slice.contains("after") {
            after_shading = Some(r.props.shading);
        }
        offset += r.len;
    }
    assert_eq!(inside_shading, Some(dark));
    assert_eq!(after_shading, Some(None), "trailing text should have no shading");
}

/// Verify that container backgrounds behind tables are preserved on table cells (#1786).
#[test]
fn container_background_behind_table_preserved_on_cells() {
    let html = r#"<div style="background:#111111;color:white"><table><tr><td>text</td></tr></table></div>"#;
    let doc = import(html);
    let table = doc
        .body
        .iter()
        .find_map(|b| match &**b {
            craft_wordmodel::Block::Table(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(table.rows[0].cells[0].props.shading, Some(craft_wordmodel::Rgb(0x11, 0x11, 0x11)));

    // Nested table inside an unshaded cell stops at the cell boundary:
    let nested_html = r#"<div style="background:#111111"><table><tr><td><table><tr><td>nested</td></tr></table></td></tr></table></div>"#;
    let nested_doc = import(nested_html);
    let outer_table = nested_doc
        .body
        .iter()
        .find_map(|b| match &**b {
            craft_wordmodel::Block::Table(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(outer_table.rows[0].cells[0].props.shading, Some(craft_wordmodel::Rgb(0x11, 0x11, 0x11)));
    let inner_table = match &*outer_table.rows[0].cells[0].blocks[0] {
        craft_wordmodel::Block::Table(t) => t,
        _ => panic!("expected nested Table"),
    };
    assert_eq!(inner_table.rows[0].cells[0].props.shading, None, "nested table in unshaded cell must stop at cell boundary");
}

/// Verify that page imports preserve root (body/html) backgrounds while pasted fragments do not (#1786).
#[test]
fn root_background_preserved_on_page_import_only() {
    let html = r#"<body style="background:#111111;color:white"><p>text</p></body>"#;
    let page_doc = import_page(html, &|_| None);
    let craft_wordmodel::Block::Para(p_page) = &*page_doc.body[0] else { panic!("expected Para") };
    assert_eq!(p_page.props.shading, Some(craft_wordmodel::Rgb(0x11, 0x11, 0x11)), "page import must preserve body background on paragraphs");
    assert_eq!(
        p_page.runs[0].props.color,
        Some(craft_wordmodel::TextColor::Rgb(craft_wordmodel::Rgb(0xff, 0xff, 0xff))),
        "page import keeps the root foreground with its background"
    );

    let frag_doc = import(html);
    let craft_wordmodel::Block::Para(p_frag) = &*frag_doc.body[0] else { panic!("expected Para") };
    assert_eq!(p_frag.props.shading, None, "fragment paste must not turn body background into paragraph shading");
    assert_eq!(p_frag.runs[0].props.color, None, "fragment paste drops the root foreground with its background");
}

/// Verify that an inline child with `background: inherit` does not receive character shading from a pasted body background (#1786).
#[test]
fn pasted_body_background_not_inherited_by_inline_child() {
    let html = r#"<body style="background:#111111"><span style="background:inherit">text</span></body>"#;
    let frag_doc = import(html);
    let craft_wordmodel::Block::Para(p_frag) = &*frag_doc.body[0] else { panic!("expected Para") };
    assert_eq!(p_frag.runs[0].props.shading, None, "inline child in pasted fragment must not inherit character shading from body background");

    let page_doc = import_page(html, &|_| None);
    let craft_wordmodel::Block::Para(p_page) = &*page_doc.body[0] else { panic!("expected Para") };
    assert_eq!(
        p_page.runs[0].props.shading,
        Some(craft_wordmodel::Rgb(0x11, 0x11, 0x11)),
        "inline child in page import inherits body background via inherit"
    );
}

#[test]
fn root_background_on_html_and_foreground_on_body() {
    let html = r#"<html style="background:#111111"><body style="color:white"><p>text</p></body></html>"#;
    let frag_doc = import(html);
    let craft_wordmodel::Block::Para(p_frag) = &*frag_doc.body[0] else { panic!("expected Para") };
    assert_eq!(p_frag.props.shading, None, "fragment paste drops html background");
    assert_eq!(p_frag.runs[0].props.color, None, "fragment paste drops body foreground when html supplied suppressed background");

    let page_doc = import_page(html, &|_| None);
    let craft_wordmodel::Block::Para(p_page) = &*page_doc.body[0] else { panic!("expected Para") };
    assert_eq!(p_page.props.shading, Some(craft_wordmodel::Rgb(0x11, 0x11, 0x11)));
    assert_eq!(p_page.runs[0].props.color, Some(craft_wordmodel::TextColor::Rgb(craft_wordmodel::Rgb(0xff, 0xff, 0xff))));
}

#[test]
fn background_unitless_zero_lengths() {
    let html = r#"<div style="background:#123456 0.0 0"><p>text</p></div>"#;
    let doc = import(html);
    let craft_wordmodel::Block::Para(p) = &*doc.body[0] else { panic!("expected Para") };
    assert_eq!(p.props.shading, Some(craft_wordmodel::Rgb(0x12, 0x34, 0x56)));

    let html2 = r#"<div style="background:#654321 -0 +0.0"><p>text</p></div>"#;
    let doc2 = import(html2);
    let craft_wordmodel::Block::Para(p2) = &*doc2.body[0] else { panic!("expected Para") };
    assert_eq!(p2.props.shading, Some(craft_wordmodel::Rgb(0x65, 0x43, 0x21)));
}

#[test]
fn background_comments_skipped_outside_strings() {
    let html = r#"<div style="background:#111111 /* dark */;color:white"><p>text</p></div>"#;
    let doc = import(html);
    let craft_wordmodel::Block::Para(p) = &*doc.body[0] else { panic!("expected Para") };
    assert_eq!(p.props.shading, Some(craft_wordmodel::Rgb(0x11, 0x11, 0x11)));

    let html2 = r#"<div style="background:#222222 /* comment / with / slashes */"><p>text</p></div>"#;
    let doc2 = import(html2);
    let craft_wordmodel::Block::Para(p2) = &*doc2.body[0] else { panic!("expected Para") };
    assert_eq!(p2.props.shading, Some(craft_wordmodel::Rgb(0x22, 0x22, 0x22)));
}

#[test]
fn invalid_color_does_not_clear_preceding_background() {
    let html = r#"<div style="background-color:red;background-color:rgba(bogus / 0)"><p>text</p></div>"#;
    let doc = import(html);
    let craft_wordmodel::Block::Para(p) = &*doc.body[0] else { panic!("expected Para") };
    assert_eq!(p.props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));

    let html2 = r#"<div style="background:red;background:rgba(bogus / 0)"><p>text</p></div>"#;
    let doc2 = import(html2);
    let craft_wordmodel::Block::Para(p2) = &*doc2.body[0] else { panic!("expected Para") };
    assert_eq!(p2.props.shading, Some(craft_wordmodel::Rgb(255, 0, 0)));
}

#[test]
fn code_block_export_preserves_foreground_color() {
    let html = r#"<pre style="background:#000000;color:#ffffff"><code>const x = 1;</code></pre>"#;
    let doc = import_page(html, &|_| None);
    let exported = export(&doc);
    assert!(exported.contains("style=\"background-color:#000000\""), "exported pre must keep background: {}", exported);
    assert!(exported.contains("color:#FFFFFF"), "exported code inlines must keep foreground: {}", exported);
    assert!(exported.contains("const x = 1;"), "exported code text preserved: {}", exported);
}

#[test]
fn background_comments_inside_functions() {
    let html = r#"<div style="background:linear-gradient(/* stops */ #111111,#222222) #111111;color:white"><p>text</p></div>"#;
    let doc = import(html);
    let craft_wordmodel::Block::Para(p) = &*doc.body[0] else { panic!("expected Para") };
    assert_eq!(p.props.shading, Some(craft_wordmodel::Rgb(0x11, 0x11, 0x11)));
    assert_eq!(p.runs[0].props.color, Some(craft_wordmodel::TextColor::Rgb(craft_wordmodel::Rgb(0xff, 0xff, 0xff))));
}

#[test]
fn zero_alpha_colors_with_none_channels() {
    let html = r#"<div style="background-color:red;background-color:rgb(none none none / 0)"><p>text</p></div>"#;
    let doc = import(html);
    let craft_wordmodel::Block::Para(p) = &*doc.body[0] else { panic!("expected Para") };
    assert_eq!(p.props.shading, None);

    let html2 = r#"<div style="background:red;background:rgb(none none none / 0)"><p>text</p></div>"#;
    let doc2 = import(html2);
    let craft_wordmodel::Block::Para(p2) = &*doc2.body[0] else { panic!("expected Para") };
    assert_eq!(p2.props.shading, None);
}

#[test]
fn zero_rowspan_stops_at_the_row_group_boundary() {
    for boundary in ["</tbody><tbody>", "<tbody>", "</tbody>", "<caption>Heading</caption>", "<colgroup><col></colgroup>", "<col>"] {
        let html = format!("<table><tr><td rowspan=0>A<td>B<tr><td>C{boundary}<tr><td>D<td>E</table>");
        let flow = parse(&html);
        let table = flow
            .blocks
            .iter()
            .find_map(|block| match block {
                FBlock::Table(table) => Some(table),
                _ => None,
            })
            .unwrap();
        assert_eq!(table.rows.len(), 3, "{boundary}: expected three rows");
        assert_eq!(table.rows[0][0].rowspan, 2, "{boundary}: A spans the first two rows");
        assert!(table.rows[1][0].covered, "{boundary}: row two continues A");
        assert!(!table.rows[2][0].covered, "{boundary}: the next group starts at column one");
        assert_eq!(table.rows[2].len(), 2, "{boundary}: D and E retain their columns");
    }
}

#[test]
fn zero_rowspan_covers_an_empty_middle_row() {
    let flow = parse("<table><tbody><tr><td rowspan=0>A<td>B</tr><tr></tr><tr><td>C</tr></tbody></table>");
    let FBlock::Table(table) = &flow.blocks[0] else { panic!("expected a table") };

    assert_eq!(table.rows.len(), 3);
    assert_eq!(table.rows[0][0].rowspan, 3);
    assert!(table.rows[1][0].covered);
    assert!(table.rows[2][0].covered);
    assert_eq!(table.rows[2][1].text(), "C");
}

#[test]
fn zero_rowspan_covers_an_empty_trailing_row() {
    let flow = parse("<table><tbody><tr><td rowspan=0>A<td>B</tr><tr><td>C</tr><tr></tr></tbody><tbody><tr><td>D</tr></tbody></table>");
    let FBlock::Table(table) = &flow.blocks[0] else { panic!("expected a table") };

    assert_eq!(table.rows[0][0].rowspan, 3);
    assert_eq!(table.rows.len(), 4);
    assert!(table.rows[2][0].covered);
    assert!(!table.rows[3][0].covered);
    assert_eq!(table.rows[3][0].text(), "D");
}

/// A copy's HTML has no style sheet, so its top-level paragraphs and headings carry their own
/// spacing and indents (as margins and `text-indent`, in points), and a mail client lays the text
/// out as the app does. A page saved as HTML doesn't write them.
#[test]
fn the_fragment_writes_a_paragraphs_spacing_and_indents() {
    let chr = craft_wordmodel::CharProps::default();
    let mut spaced = craft_wordmodel::Paragraph::with_text("Spaced", chr.clone());
    spaced.props.space_before = Some(6.0);
    spaced.props.space_after = Some(12.0);
    spaced.props.indent_left = Some(36.0);
    spaced.props.indent_first = Some(-18.0);
    let mut plain = craft_wordmodel::Paragraph::with_text("Plain", chr.clone());
    plain.props.indent_right = Some(-0.0);
    plain.props.indent_first = Some(0.0);
    plain.props.space_after = Some(f32::NAN);
    // Values that round to zero print as 0 (never "-0"), and a first-line indent that does is left out.
    let mut tiny = craft_wordmodel::Paragraph::with_text("Tiny", chr.clone());
    tiny.props.indent_left = Some(-0.004);
    tiny.props.indent_first = Some(0.001);
    let mut heading = craft_wordmodel::Paragraph::with_text("Heading", chr).styled("Heading1");
    heading.props.space_before = Some(24.0);
    let mut d = Document::new();
    d.body = vec![
        craft_wordmodel::para_block(spaced),
        craft_wordmodel::para_block(plain),
        craft_wordmodel::para_block(tiny),
        craft_wordmodel::para_block(heading),
    ];
    let fragment = export_fragment(&d);
    assert!(fragment.contains("<p style=\"margin-top:6pt;margin-bottom:12pt;margin-left:36pt;text-indent:-18pt\">Spaced</p>"), "{fragment}");
    assert!(fragment.contains("<p style=\"margin-right:0pt\">Plain</p>"), "{fragment}");
    assert!(fragment.contains("<p style=\"margin-left:0pt\">Tiny</p>"), "{fragment}");
    assert!(fragment.contains("<h1 style=\"margin-top:24pt\">Heading</h1>"), "{fragment}");
    let page = export(&d);
    assert!(!page.contains("margin-") && !page.contains("text-indent"), "{page}");
}

/// Quotes, list items and code keep the fragment's plain markup, while a paragraph between them
/// still writes its spacing.
#[test]
fn the_fragment_spaces_only_top_level_paragraphs_and_headings() {
    let chr = craft_wordmodel::CharProps::default();
    let mut quote = craft_wordmodel::Paragraph::with_text("Quoted", chr.clone()).styled("Quote");
    quote.props.space_after = Some(10.0);
    let mut body = craft_wordmodel::Paragraph::with_text("Body", chr.clone());
    body.props.space_after = Some(8.0);
    let mut d = Document::new();
    let num = std::sync::Arc::make_mut(&mut d.numbering).add_list(craft_wordmodel::ListKind::Bullet);
    let mut item = craft_wordmodel::Paragraph::with_text("Item", chr).styled("ListParagraph");
    item.props.numbering = Some(craft_wordmodel::props::NumRef { num, level: 0 });
    item.props.space_after = Some(6.0);
    d.body = vec![craft_wordmodel::para_block(quote), craft_wordmodel::para_block(body), craft_wordmodel::para_block(item)];
    let fragment = export_fragment(&d);
    assert!(fragment.contains("<blockquote>\n<p>Quoted</p>\n</blockquote>"), "{fragment}");
    assert!(fragment.contains("<p style=\"margin-bottom:8pt\">Body</p>"), "{fragment}");
    assert!(fragment.contains("<ul><li>Item</li></ul>"), "{fragment}");
}

/// A list item's further paragraph keeps its alignment and page break in the fragment, without
/// spacing or indents, as the item itself.
#[test]
fn the_fragment_leaves_a_list_items_further_paragraph_unspaced() {
    let chr = craft_wordmodel::CharProps::default();
    let mut d = Document::new();
    let num = std::sync::Arc::make_mut(&mut d.numbering).add_list(craft_wordmodel::ListKind::Bullet);
    let mut item = craft_wordmodel::Paragraph::with_text("Item", chr.clone()).styled("ListParagraph");
    item.props.numbering = Some(craft_wordmodel::props::NumRef { num, level: 0 });
    let mut more = craft_wordmodel::Paragraph::with_text("More", chr);
    more.props.list_item = Some(0);
    more.props.align = Some(craft_wordmodel::Align::Center);
    more.props.page_break_before = Some(true);
    more.props.indent_left = Some(72.0);
    more.props.space_after = Some(6.0);
    d.body = vec![craft_wordmodel::para_block(item), craft_wordmodel::para_block(more)];
    let fragment = export_fragment(&d);
    let more = fragment.split("<li>Item").nth(1).and_then(|s| s.split("</li>").next()).unwrap();
    assert!(more.contains("More") && more.contains("text-align:center") && more.contains("break-before:page"), "{fragment}");
    assert!(!fragment.contains("margin-"), "{fragment}");
}

/// Code blocks and the paragraphs of a table cell keep the fragment's plain markup, whatever their
/// spacing.
#[test]
fn the_fragment_leaves_code_and_table_cells_unspaced() {
    let d = import(
        "<table><tr><td><p style='margin-left:20pt'>One</p><p style='margin-left:20pt'>Two</p></td></tr></table>         <pre style='margin-left:30pt'>code</pre>",
    );
    let fragment = export_fragment(&d);
    assert!(fragment.contains("One") && fragment.contains("code"), "{fragment}");
    assert!(!fragment.contains("margin-"), "{fragment}");
}
