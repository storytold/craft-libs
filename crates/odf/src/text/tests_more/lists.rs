//! Lists (ODF 1.3 §5.3, §16.30–16.33): each `text:list` is a list of its own unless it continues
//! one; its list style gives each level's number format, prefix and suffix, the levels shown,
//! the bullet character and the start value.

use crate::text::tests::{NS_T, package};
use crate::text::{export, import};
use craft_wordmodel::Document;
use craft_wordmodel::section::NumFormat;

const NS_X: &str = r#"xmlns:xml="http://www.w3.org/XML/1998/namespace""#;

fn doc(styles: &str, body: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_X}><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

fn number(name: &str, format: &str, prefix: &str, suffix: &str, extra: &str) -> String {
    format!(
        r#"<text:list-style style:name="{name}"><text:list-level-style-number text:level="1" style:num-format="{format}" style:num-prefix="{prefix}" style:num-suffix="{suffix}" {extra}/><text:list-level-style-number text:level="2" style:num-format="a" style:num-suffix=")"/></text:list-style>"#
    )
}

fn bullet(name: &str, ch: &str) -> String {
    format!(r#"<text:list-style style:name="{name}"><text:list-level-style-bullet text:level="1" text:bullet-char="{ch}"/></text:list-style>"#)
}

fn list(style: &str, items: &[&str], attrs: &str) -> String {
    let items: String = items.iter().map(|t| format!("<text:list-item><text:p>{t}</text:p></text:list-item>")).collect();
    format!(r#"<text:list text:style-name="{style}" {attrs}>{items}</text:list>"#)
}

/// Each list paragraph: (its text, its list instance, level 0's format, label text and start).
fn items(d: &Document) -> Vec<(String, u32, NumFormat, String, u32)> {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .filter_map(|p| {
            let n = p.props.numbering?;
            let lv = d.numbering.level(n.num, 0)?;
            Some((p.text.clone(), n.num, lv.format, lv.text.clone(), lv.start))
        })
        .collect()
}

fn styles() -> String {
    [number("N1", "1", "", ".", ""), number("N2", "i", "", ".", ""), number("N3", "A", "(", ")", ""), bullet("B1", "–")].concat()
}

fn lists_body() -> String {
    [list("N1", &["One", "Two"], ""), list("N2", &["i one", "i two"], ""), list("N3", &["A one"], ""), list("B1", &["dash"], "")].concat()
}

#[test]
fn separate_lists_stay_separate_with_their_formats() {
    let d = doc(&styles(), &lists_body());
    let got = items(&d);
    let nums: Vec<u32> = got.iter().map(|i| i.1).collect();
    assert_eq!(nums[0], nums[1]);
    assert!(nums[1] != nums[2] && nums[3] != nums[4] && nums[4] != nums[5], "each text:list is its own list: {nums:?}");
    let shown: Vec<(NumFormat, &str)> = got.iter().map(|i| (i.2, i.3.as_str())).collect();
    assert_eq!(
        shown,
        [
            (NumFormat::Decimal, "%1."),
            (NumFormat::Decimal, "%1."),
            (NumFormat::LowerRoman, "%1."),
            (NumFormat::LowerRoman, "%1."),
            (NumFormat::UpperLetter, "(%1)"),
            (NumFormat::Bullet, "–"),
        ]
    );
}

#[test]
fn a_list_can_continue_another() {
    let body = [
        list("N1", &["One", "Two"], r#"xml:id="first""#),
        "<text:p>Between</text:p>".to_string(),
        list("N1", &["Three"], r#"text:continue-numbering="true""#),
        list("N2", &["i"], ""),
        list("N1", &["Four"], r#"text:continue-list="first""#),
    ]
    .concat();
    let d = doc(&styles(), &body);
    let got = items(&d);
    assert_eq!(got.len(), 5);
    assert_eq!(got[0].1, got[2].1, "continue-numbering goes on with the list before");
    assert_eq!(got[0].1, got[4].1, "continue-list goes on with the list it names");
    assert_ne!(got[0].1, got[3].1);
}

#[test]
fn start_values_and_levels_shown() {
    let styles = [number("S", "1", "", ".", r#"text:start-value="5" text:display-levels="1""#), {
        r#"<text:list-style style:name="D"><text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."/><text:list-level-style-number text:level="2" style:num-format="1" style:num-suffix="." text:display-levels="2"/></text:list-style>"#.to_string()
    }]
    .concat();
    let body = [list("S", &["five"], ""), r#"<text:list text:style-name="D"><text:list-item><text:p>1</text:p><text:list><text:list-item><text:p>1.1</text:p></text:list-item></text:list></text:list-item></text:list>"#.to_string()].concat();
    let d = doc(&styles, &body);
    let got = items(&d);
    assert_eq!(got[0].4, 5);
    let n = d.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == "1.1").and_then(|p| p.props.numbering).unwrap();
    assert_eq!(n.level, 1);
    assert_eq!(d.numbering.level(n.num, 1).unwrap().text, "%1.%2.");
}

#[test]
fn lists_round_trip() {
    let d =
        doc(&styles(), &[lists_body(), "<text:p>Between</text:p>".to_string(), list("N1", &["Three"], r#"text:continue-numbering="true""#)].concat());
    let back = import(&export(&d).unwrap()).unwrap();
    let strip = |v: Vec<(String, u32, NumFormat, String, u32)>| {
        // Instance ids may be renumbered; which items share one may not.
        let firsts: Vec<u32> = v.iter().map(|i| i.1).collect();
        let same: Vec<usize> = firsts.iter().map(|n| firsts.iter().position(|m| m == n).unwrap_or(0)).collect();
        v.into_iter().zip(same).map(|((t, _, f, l, s), k)| (t, k, f, l, s)).collect::<Vec<_>>()
    };
    assert_eq!(strip(items(&back)), strip(items(&d)));
}

#[test]
fn hostile_list_styles_are_bounded() {
    let styles = [
        number("H1", "?", "((((((((((((((((((((", "))))))))))))))))))", r#"text:start-value="99999999999" text:display-levels="99""#),
        r#"<text:list-style style:name="H2"><text:list-level-style-number text:level="999" style:num-format="1"/><text:list-level-style-bullet text:level="0" text:bullet-char=""/></text:list-style>"#.to_string(),
    ]
    .concat();
    let d = doc(&styles, &[list("H1", &["x"], ""), list("H2", &["y"], r#"text:continue-list="nope""#)].concat());
    let got = items(&d);
    assert_eq!(got.len(), 2);
    assert!(got[0].3.chars().count() <= 64, "{}", got[0].3);
    let _ = export(&d).unwrap();
}

#[test]
fn a_word_before_the_number_is_kept() {
    let d = doc(&number("C", "1", "Chapter ", ":", ""), &list("C", &["x"], ""));
    assert_eq!(items(&d)[0].3, "Chapter %1:");
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(items(&back)[0].3, "Chapter %1:");
}

#[test]
fn list_level_positions_are_read_and_written() {
    // Each level's indent, hanging and number alignment (ODF 1.3 §17.19–17.20,
    // `style:list-level-properties` in label-alignment mode).
    let styles = r#"<text:list-style style:name="P"><text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."><style:list-level-properties text:list-level-position-and-space-mode="label-alignment" fo:text-align="end"><style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="0.5in" fo:text-indent="-0.25in" fo:margin-left="0.5in"/></style:list-level-properties></text:list-level-style-number></text:list-style><text:list-style style:name="Q"><text:list-level-style-bullet text:level="1" text:bullet-char="•"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" fo:text-indent="-0.1in" fo:margin-left="0.2in"/></style:list-level-properties></text:list-level-style-bullet></text:list-style>"#;
    let d = doc(styles, &[list("P", &["one"], ""), list("Q", &["dot"], "")].concat());
    let level = |i: usize| {
        let p = d.body.iter().filter_map(|b| b.as_para()).nth(i).unwrap();
        let n = p.props.numbering.unwrap();
        d.numbering.level(n.num, n.level).unwrap().clone()
    };
    let (a, b) = (level(0), level(1));
    assert_eq!((a.indent, a.hanging, a.align), (36.0, 18.0, craft_wordmodel::Align::Right));
    assert_eq!((b.indent, b.hanging), (14.4, 7.2));
    let back = import(&export(&d).unwrap()).unwrap();
    let again = |i: usize| {
        let p = back.body.iter().filter_map(|b| b.as_para()).nth(i).unwrap();
        let n = p.props.numbering.unwrap();
        back.numbering.level(n.num, n.level).unwrap().clone()
    };
    let (c, e) = (again(0), again(1));
    assert_eq!((c.indent, c.hanging, c.align), (a.indent, a.hanging, a.align));
    assert!((e.indent - b.indent).abs() < 0.01 && (e.hanging - b.hanging).abs() < 0.01);
}

#[test]
fn a_list_items_further_paragraphs_are_paragraphs_with_no_number() {
    // LibreOffice writes an unnumbered entry as an item's second paragraph. It was joined to the
    // first by a line break, losing its paragraph, and so the first's markup too.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><text:list-style style:name="L"><text:list-level-style-number text:level="1" style:num-format="1"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment fo:margin-left="1in" fo:text-indent="-0.25in"/></style:list-level-properties></text:list-level-style-number></text:list-style></office:styles></office:document-styles>"#
    );
    let body = r#"<text:list text:style-name="L"><text:list-header><text:p>Header</text:p></text:list-header><text:list-item><text:p>One</text:p><text:p>Still one</text:p><text:p>And again</text:p></text:list-item><text:list-item><text:p>Two</text:p></text:list-item></text:list><text:p>After</text:p>"#;
    let content = format!("<office:document-content {NS_T}><office:body><office:text>{body}</office:text></office:body></office:document-content>");
    let shape = |d: &craft_wordmodel::Document| {
        d.body.iter().filter_map(|b| b.as_para()).map(|p| (p.text.clone(), p.props.numbering.is_some(), p.props.indent_left)).collect::<Vec<_>>()
    };
    let d = import(&package(&content, Some(&styles))).unwrap();
    let want = |a: &[(String, bool, Option<f32>)]| {
        let row = |t: &str| a.iter().find(|r| r.0 == t).unwrap_or_else(|| panic!("{t} in {a:?}"));
        assert_eq!(a.len(), 6, "each its own paragraph: {a:?}");
        for t in ["Header", "Still one", "And again"] {
            assert!(!row(t).1, "{t} has no number: {a:?}");
            assert_eq!(row(t).2, Some(72.0), "{t} at the item's text: {a:?}");
        }
        for t in ["One", "Two"] {
            assert!(row(t).1, "{t} is numbered: {a:?}");
        }
        assert!(!row("After").1);
    };
    want(&shape(&d));
    // The numbers go on past the unnumbered paragraph: one list.
    let nums: Vec<u32> = d.body.iter().filter_map(|b| b.as_para()).filter_map(|p| p.props.numbering.map(|n| n.num)).collect();
    assert_eq!(nums.len(), 2);
    assert_eq!(nums[0], nums[1], "{nums:?}");
    // And so do they written from the model, and written as read.
    for saved in [crate::text::tests::export_model(&d).unwrap(), export(&d).unwrap()] {
        let back = import(&saved).unwrap();
        want(&shape(&back));
        assert_eq!(back.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == "Header").unwrap().props.list_item, Some(0));
        let nums: Vec<u32> = back.body.iter().filter_map(|b| b.as_para()).filter_map(|p| p.props.numbering.map(|n| n.num)).collect();
        assert_eq!(nums[0], nums[1], "{nums:?}");
    }
    // Edited, an unnumbered paragraph is saved as edited, not as the list's kept markup said, and
    // the list around it stays one.
    for t in ["Header", "Still one"] {
        let mut d = import(&package(&content, Some(&styles))).unwrap();
        let i = d.body.iter().position(|b| b.as_para().is_some_and(|p| p.text == t)).unwrap();
        d.insert_text(&craft_wordmodel::Pos::body(i, 0), "Edited ", &Default::default()).unwrap();
        let back = import(&export(&d).unwrap()).unwrap();
        let rows = shape(&back);
        assert_eq!(rows.len(), 6, "each its own paragraph: {rows:?}");
        assert!(!rows.iter().any(|r| r.0 == t), "{t} was edited: {rows:?}");
        let row = rows.iter().find(|r| r.0 == format!("Edited {t}")).unwrap_or_else(|| panic!("the edit: {rows:?}"));
        assert_eq!((row.1, row.2), (false, Some(72.0)), "{t}: no number, at the item's text: {rows:?}");
        let nums: Vec<u32> = back.body.iter().filter_map(|b| b.as_para()).filter_map(|p| p.props.numbering.map(|n| n.num)).collect();
        assert_eq!(nums.len(), 2, "{rows:?}");
        assert_eq!(nums[0], nums[1], "{nums:?}");
    }
}

/// A level first used after an unnumbered paragraph is still the file's: a bulleted sublist stays
/// bulleted, a numbered one under bullets stays numbered, read and written from the model.
#[test]
fn a_sublist_after_an_unnumbered_paragraph_keeps_its_levels() {
    let open = |first: &str, second: &str| {
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:styles><text:list-style style:name="L">{first}{second}</text:list-style></office:styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {NS_T}><office:body><office:text><text:list text:style-name="L"><text:list-item><text:p>One</text:p><text:p>More</text:p><text:list><text:list-item><text:p>Sub</text:p></text:list-item></text:list></text:list-item></text:list></office:text></office:body></office:document-content>"#
        );
        import(&package(&content, Some(&styles))).unwrap()
    };
    let sub = |d: &Document| {
        let p = d.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == "Sub").unwrap();
        let n = p.props.numbering.unwrap();
        d.numbering.level(n.num, n.level).map(|l| (n.level, l.format, l.text.clone(), l.start)).unwrap()
    };
    let number = |level: u8, format: &str, start: u8| {
        format!(r#"<text:list-level-style-number text:level="{level}" style:num-format="{format}" style:num-suffix="." text:start-value="{start}"/>"#)
    };
    let bullet = |level: u8| format!(r#"<text:list-level-style-bullet text:level="{level}" text:bullet-char="–"/>"#);
    let d = open(&number(1, "1", 1), &bullet(2));
    assert_eq!(sub(&d), (1, NumFormat::Bullet, "–".to_string(), 1));
    assert_eq!(sub(&import(&crate::text::tests::export_model(&d).unwrap()).unwrap()), sub(&d), "written from the model");
    let d = open(&bullet(1), &number(2, "i", 3));
    let (level, format, _, start) = sub(&d);
    assert_eq!((level, format, start), (1, NumFormat::LowerRoman, 3));
    assert_eq!(sub(&import(&crate::text::tests::export_model(&d).unwrap()).unwrap()), sub(&d), "written from the model");
}

/// A list item's further paragraph stays in its item in HTML and Markdown: the list goes on after it.
#[test]
fn a_list_items_further_paragraph_stays_in_it_in_html_and_markdown() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><text:list-style style:name="L"><text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."/></text:list-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:list text:style-name="L"><text:list-item><text:p>One</text:p><text:p>Cont</text:p></text:list-item><text:list-item><text:p>Two</text:p></text:list-item></text:list></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let html = craft_html::doc::export(&d);
    assert_eq!(html.matches("<ol").count(), 1, "{html}");
    assert!(html.contains("<p>Cont</p>"), "{html}");
    let md = craft_markdown::export(&d);
    assert!(md.contains("1. One\n\n   Cont\n\n2. Two"), "{md}");
}

/// Only a list item's own further paragraph stays in it: an indented paragraph between two lists
/// joined by `text:continue-list` ends the list, and a last item's further paragraph (a one-item
/// list's too) stays in it.
#[test]
fn only_an_items_own_further_paragraph_stays_in_it() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><text:list-style style:name="L"><text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."/></text:list-style></office:styles></office:document-styles>"#
    );
    let open = |body: &str| {
        // (An indent of its own: LibreOffice writes one as an automatic style.)
        let auto = r#"<office:automatic-styles><style:style style:name="Ind" style:family="paragraph"><style:paragraph-properties fo:margin-left="0.5in" fo:text-indent="0in"/></style:style></office:automatic-styles>"#;
        let content =
            format!("<office:document-content {NS_T}>{auto}<office:body><office:text>{body}</office:text></office:body></office:document-content>");
        import(&package(&content, Some(&styles))).unwrap()
    };
    // A paragraph of its own between two parts of one list.
    let d = open(
        r#"<text:list xml:id="a" text:style-name="L"><text:list-item><text:p>One</text:p></text:list-item></text:list><text:p text:style-name="Ind">Between</text:p><text:list text:continue-list="a" text:style-name="L"><text:list-item><text:p>Two</text:p></text:list-item></text:list>"#,
    );
    let between = d.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == "Between").unwrap();
    assert_eq!(between.props.indent_left, Some(36.0));
    let html = craft_html::doc::export(&d);
    assert_eq!(html.matches("<ol").count(), 2, "{html}");
    assert!(!html.contains("<p>Between</p></li>") && !html.contains("<li>One<p>"), "{html}");
    let md = craft_markdown::export(&d);
    assert!(md.contains("\n\nBetween\n\n") && !md.contains(" Between"), "{md}");
    // A last item's further paragraph, and a one-item list's.
    for body in [
        r#"<text:list text:style-name="L"><text:list-item><text:p>One</text:p></text:list-item><text:list-item><text:p>Two</text:p><text:p>Cont</text:p></text:list-item></text:list><text:p>After</text:p>"#,
        r#"<text:list text:style-name="L"><text:list-item><text:p>Two</text:p><text:p>Cont</text:p></text:list-item></text:list><text:p>After</text:p>"#,
    ] {
        let d = open(body);
        let html = craft_html::doc::export(&d);
        assert!(html.contains("<li>Two<p>Cont</p></li>"), "{html}");
        let md = craft_markdown::export(&d);
        assert!(md.contains("1. Two\n\n   Cont\n\nAfter") || md.contains("2. Two\n\n   Cont\n\nAfter"), "{md}");
    }
}

/// Written from the model, a list item's further paragraph is in its `text:list-item`, after the
/// item's sublist, and the list stays one (#5723 review).
#[test]
fn a_further_paragraph_written_from_the_model_stays_in_its_item() {
    let d = craft_markdown::import("1. One\n\n   Cont\n    - Sub\n\n   More\n\n2. Two\n");
    let xml = crate::text::tests::content_of(&crate::text::tests::export_model(&d).unwrap());
    let one = xml.find(">One<").unwrap();
    let body = xml.get(one..).unwrap();
    assert!(body.contains("Cont</text:p><text:list>"), "{xml}");
    assert!(body.contains("Sub</text:p></text:list-item></text:list><text:p"), "{xml}");
    assert!(body.contains("More</text:p></text:list-item><text:list-item><text:p"), "{xml}");
    assert_eq!(xml.matches("<text:list xml:id").count(), 1, "one list: {xml}");
    let back = import(&crate::text::tests::export_model(&d).unwrap()).unwrap();
    let rows: Vec<_> =
        back.body.iter().filter_map(|b| b.as_para()).map(|p| (p.text.clone(), p.props.numbering.is_some(), p.props.list_item)).collect();
    assert_eq!(
        rows,
        [
            ("One".to_string(), true, None),
            ("Cont".to_string(), false, Some(0)),
            ("Sub".to_string(), true, None),
            ("More".to_string(), false, Some(0)),
            ("Two".to_string(), true, None)
        ],
        "{xml}"
    );
}

/// A list item's further paragraph opening or closing a section kept as read ends the list there,
/// as an item doing so does: the section opens before it, or closes after it, not somewhere past.
#[test]
fn a_further_paragraph_opening_or_closing_a_section_ends_the_list_there() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><text:list-style style:name="L"><text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."/></text:list-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T} {NS_X}><office:body><office:text><text:section text:name="S"><text:list xml:id="l1" text:style-name="L"><text:list-item><text:p>Zero</text:p></text:list-item><text:list-item><text:p>One</text:p><text:p>Cont</text:p></text:list-item></text:list></text:section><text:list text:style-name="L" text:continue-list="l1"><text:list-item><text:p>Two</text:p></text:list-item></text:list></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    // Edited, so the list is written from the model; the section's paragraphs are as read.
    let i = d.body.iter().position(|b| b.as_para().is_some_and(|p| p.text == "One")).unwrap();
    d.insert_text(&craft_wordmodel::Pos::body(i, 0), "Edited ", &Default::default()).unwrap();
    let c = crate::text::tests::content_of(&export(&d).unwrap());
    let (close, two) = (c.find("</text:section>"), c.find(">Two<"));
    assert!(close.is_some() && two.is_some() && close < two, "the section closes before Two: {c}");
    assert!(c.find(">Cont<") < close, "{c}");
    // And one opening a section starts after the list: the section opens before it.
    let content = format!(
        r#"<office:document-content {NS_T} {NS_X}><office:body><office:text><text:list xml:id="l1" text:style-name="L"><text:list-item><text:p>One</text:p></text:list-item></text:list><text:section text:name="S"><text:list text:style-name="L" text:continue-list="l1"><text:list-header><text:p>Header</text:p></text:list-header><text:list-item><text:p>Two</text:p></text:list-item></text:list></text:section></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    let i = d.body.iter().position(|b| b.as_para().is_some_and(|p| p.text == "One")).unwrap();
    d.insert_text(&craft_wordmodel::Pos::body(i, 0), "Edited ", &Default::default()).unwrap();
    let c = crate::text::tests::content_of(&export(&d).unwrap());
    let (open, header) = (c.find("<text:section"), c.find(">Header<"));
    assert!(open.is_some() && header.is_some() && open < header, "the section opens before Header: {c}");
}

/// A heading (`text:h`) in a list item is one of its further paragraphs: it stays in its item in
/// HTML, Markdown and ODT written from the model, and the list numbers on after it.
#[test]
fn a_heading_in_a_list_item_stays_in_it() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><text:list-style style:name="L"><text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."/></text:list-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T} {NS_X}><office:body><office:text><text:list text:style-name="L"><text:list-item><text:p>One</text:p><text:h text:outline-level="2">Head</text:h><text:p>After</text:p></text:list-item><text:list-item><text:p>Two</text:p></text:list-item></text:list></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    let html = craft_html::doc::export(&d);
    assert_eq!(html.matches("<ol").count(), 1, "{html}");
    assert!(html.contains("<li>One<h2>Head</h2><p>After</p></li>"), "{html}");
    let md = craft_markdown::export(&d);
    assert!(md.contains("1. One\n\n   ## Head\n\n   After\n\n2. Two"), "{md}");
    // Edited, so the list is written from the model.
    let i = d.body.iter().position(|b| b.as_para().is_some_and(|p| p.text == "One")).unwrap();
    d.insert_text(&craft_wordmodel::Pos::body(i, 0), "Edited ", &Default::default()).unwrap();
    let out = export(&d).unwrap();
    let c = crate::text::tests::content_of(&out);
    assert!(c.contains("Edited One</text:p><text:h"), "{c}");
    assert!(c.contains(">Head</text:h>"), "{c}");
    assert!(c.contains("After</text:p></text:list-item><text:list-item>"), "{c}");
    let back = import(&out).unwrap();
    let rows: Vec<_> =
        back.body.iter().filter_map(|b| b.as_para()).map(|p| (p.text.clone(), p.props.numbering.is_some(), p.props.list_item)).collect();
    assert_eq!(
        rows,
        [
            ("Edited One".to_string(), true, None),
            ("Head".to_string(), false, Some(0)),
            ("After".to_string(), false, Some(0)),
            ("Two".to_string(), true, None)
        ],
        "{c}"
    );
    let head = back.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == "Head").unwrap();
    assert_eq!(head.props.style.as_deref(), Some("Heading2"), "{c}");
}

#[test]
fn zero_indent_multi_paragraph_header_survives_model_export() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><text:list-style style:name="L"><text:list-level-style-number text:level="1" style:num-format="1"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment fo:margin-left="0in" fo:text-indent="0in"/></style:list-level-properties></text:list-level-style-number></text:list-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:list text:style-name="L"><text:list-header><text:p>Header</text:p><text:p>More header</text:p></text:list-header><text:list-item><text:p>One</text:p></text:list-item></text:list></office:text></office:body></office:document-content>"#
    );
    let doc = import(&package(&content, Some(&styles))).unwrap();
    let bytes = crate::text::tests::export_model(&doc).unwrap();
    let xml = crate::text::tests::entry(&bytes, "content.xml");
    assert_eq!(xml.matches("<text:list-header>").count(), 1);
    let back = import(&bytes).unwrap();
    for text in ["Header", "More header"] {
        let p = back.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == text).unwrap();
        assert_eq!(p.props.list_item, Some(0));
        assert!(p.props.numbering.is_none());
    }
}

/// A sublist's header, written from the model, stays with the sublist's items in the item the
/// sublist is in: alone or of several paragraphs, below another header, and before the item goes
/// on after the sublist.
#[test]
fn a_sublists_header_written_from_the_model_stays_in_its_parent_item() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><text:list-style style:name="L"><text:list-level-style-number text:level="1" style:num-format="1"/><text:list-level-style-number text:level="2" style:num-format="a"/><text:list-level-style-number text:level="3" style:num-format="i"/></text:list-style></office:styles></office:document-styles>"#
    );
    // The lists of `xml`, without attributes.
    let lists = |xml: &str| -> String {
        let (from, to) = (xml.find("<text:list ").unwrap(), xml.rfind("</text:list>").unwrap());
        let mut out = String::new();
        for part in xml.get(from..to).unwrap().split('<').skip(1) {
            let (tag, text) = part.split_once('>').unwrap();
            out.push_str(&format!("<{}>{text}", tag.split(' ').next().unwrap()));
        }
        out + "</text:list>"
    };
    let head = "<text:list-header><text:p>Sub header</text:p></text:list-header>";
    let sub = "<text:list-item><text:p>Sub</text:p></text:list-item>";
    for inner in [
        format!("<text:list>{head}{sub}<text:list-item><text:p>Sub two</text:p></text:list-item></text:list>"),
        format!(
            "<text:p>More</text:p><text:list><text:list-header><text:p>Sub header</text:p><text:p>Second line</text:p></text:list-header>{sub}</text:list>"
        ),
        format!("<text:list>{head}</text:list><text:p>Parent again</text:p>"),
        "<text:list><text:list-header><text:p>Sub header</text:p><text:list><text:list-header><text:p>Deeper</text:p></text:list-header><text:list-item><text:p>Deepest</text:p></text:list-item></text:list></text:list-header></text:list>".to_string(),
    ] {
        let list = |parent: &str, attrs: &str| {
            format!(
                "<text:list{attrs}><text:list-item><text:p>{parent}</text:p>{inner}</text:list-item><text:list-item><text:p>Next</text:p></text:list-item></text:list>"
            )
        };
        let content = format!(
            "<office:document-content {NS_T}><office:body><office:text>{}</office:text></office:body></office:document-content>",
            list("Parent", r#" text:style-name="L""#)
        );
        let mut d = import(&package(&content, Some(&styles))).unwrap();
        // Edited, so the list is written from the model.
        let i = d.body.iter().position(|b| b.as_para().is_some_and(|p| p.text == "Parent")).unwrap();
        d.insert_text(&craft_wordmodel::Pos::body(i, 0), "Edited ", &Default::default()).unwrap();
        let out = export(&d).unwrap();
        assert_eq!(lists(&crate::text::tests::content_of(&out)), list("Edited Parent", ""));
        let rows = |d: &Document| {
            let paras = d.body.iter().filter_map(|b| b.as_para());
            paras.map(|p| (p.text.clone(), p.props.numbering.map(|n| n.level), p.props.list_item)).collect::<Vec<_>>()
        };
        assert_eq!(rows(&import(&out).unwrap()), rows(&d));
    }
}
