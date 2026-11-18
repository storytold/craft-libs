use super::*;
use crate::math::*;

fn omath(inner: &str) -> El {
    let x = format!(
        r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{inner}</m:oMath>"#
    );
    parse(&x).unwrap()
}

#[test]
fn reads_fraction_radical_and_scripts() {
    let e = omath(
        r#"<m:r><m:t>x=</m:t></m:r><m:f><m:num><m:r><m:t>-b±</m:t></m:r><m:rad><m:radPr><m:degHide m:val="1"/></m:radPr><m:deg/><m:e><m:sSup><m:e><m:r><m:t>b</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup><m:r><m:t>-4ac</m:t></m:r></m:e></m:rad></m:num><m:den><m:r><m:t>2a</m:t></m:r></m:den></m:f>"#,
    );
    let m = read_omath(&e, MathJc::CenterGroup);
    assert_eq!(m.nodes.len(), 2);
    let MNode::Frac { num, den, kind } = &m.nodes[1] else { panic!("{:#?}", m.nodes) };
    assert_eq!(*kind, FracKind::Bar);
    assert_eq!(den, &vec![MNode::Run(MRun::new("2a"))]);
    let MNode::Rad { deg_hide, e, .. } = &num[1] else { panic!() };
    assert!(*deg_hide);
    assert!(matches!(e[0], MNode::Script { kind: ScriptKind::Sup, .. }));
    assert_eq!(crate::math::to_linear(&m.nodes), "x=(-b±√(b^2-4ac))/2a");
    assert!(m.omml.starts_with("<m:oMath>"));
}

#[test]
fn reads_nary_delims_matrix_and_defaults() {
    let e = omath(
        r#"<m:nary><m:naryPr><m:chr m:val="∑"/><m:limLoc m:val="undOvr"/></m:naryPr><m:sub><m:r><m:t>i</m:t></m:r></m:sub><m:sup/><m:e><m:r><m:t>i</m:t></m:r></m:e></m:nary><m:d><m:dPr><m:begChr m:val="["/><m:endChr m:val=""/></m:dPr><m:e><m:r><m:t>a</m:t></m:r></m:e><m:e><m:r><m:t>b</m:t></m:r></m:e></m:d><m:nary><m:e/></m:nary><m:m><m:mr><m:e/><m:e/></m:mr></m:m>"#,
    );
    let m = read_omath(&e, MathJc::CenterGroup);
    let MNode::Nary { chr, lim_loc, .. } = &m.nodes[0] else { panic!() };
    assert_eq!((*chr, *lim_loc), ('∑', Some(LimLoc::UndOvr)));
    let MNode::Delim { beg, end, sep, elems, .. } = &m.nodes[1] else { panic!() };
    assert_eq!((*beg, *end, *sep, elems.len()), (Some('['), None, Some('|'), 2));
    assert!(matches!(m.nodes[2], MNode::Nary { chr: '∫', .. }));
    assert!(matches!(&m.nodes[3], MNode::Matrix { rows, .. } if rows.len() == 1 && rows[0].len() == 2));
}

/// Every run's text in the tree, in reading order.
fn texts(m: &mut Math) -> Vec<String> {
    let mut out = Vec::new();
    crate::math_tree::each_run_mut(&mut m.nodes, &mut |r| out.push(r.text.clone()));
    out
}

fn t(s: &str) -> String {
    format!("<m:r><m:t>{s}</m:t></m:r>")
}

/// #1273: one equation using every Office Math element of ECMA-376 §22.1 (each argument
/// holding a distinct marker) keeps every argument's text: nothing is silently dropped.
#[test]
fn every_omml_element_keeps_its_content() {
    let parts = [
        format!("<m:acc><m:accPr><m:chr m:val=\"̃\"/></m:accPr><m:e>{}</m:e></m:acc>", t("A1")),
        format!("<m:bar><m:barPr><m:pos m:val=\"top\"/></m:barPr><m:e>{}</m:e></m:bar>", t("B1")),
        format!("<m:box><m:boxPr><m:opEmu m:val=\"1\"/></m:boxPr><m:e>{}</m:e></m:box>", t("C1")),
        format!("<m:borderBox><m:borderBoxPr><m:strikeH/></m:borderBoxPr><m:e>{}</m:e></m:borderBox>", t("D1")),
        format!("<m:d><m:dPr><m:begChr m:val=\"[\"/></m:dPr><m:e>{}</m:e><m:e>{}</m:e></m:d>", t("E1"), t("E2")),
        format!("<m:eqArr><m:e>{}</m:e><m:e>{}</m:e></m:eqArr>", t("F1"), t("F2")),
        format!("<m:f><m:fPr><m:type m:val=\"skw\"/></m:fPr><m:num>{}</m:num><m:den>{}</m:den></m:f>", t("G1"), t("G2")),
        format!("<m:func><m:fName>{}</m:fName><m:e>{}</m:e></m:func>", t("H1"), t("H2")),
        format!("<m:groupChr><m:groupChrPr><m:chr m:val=\"⏞\"/><m:pos m:val=\"top\"/></m:groupChrPr><m:e>{}</m:e></m:groupChr>", t("I1")),
        format!("<m:limLow><m:e>{}</m:e><m:lim>{}</m:lim></m:limLow>", t("J1"), t("J2")),
        format!("<m:limUpp><m:e>{}</m:e><m:lim>{}</m:lim></m:limUpp>", t("K1"), t("K2")),
        format!("<m:m><m:mr><m:e>{}</m:e><m:e>{}</m:e></m:mr><m:mr><m:e>{}</m:e><m:e/></m:mr></m:m>", t("L1"), t("L2"), t("L3")),
        format!(
            "<m:nary><m:naryPr><m:chr m:val=\"∑\"/></m:naryPr><m:sub>{}</m:sub><m:sup>{}</m:sup><m:e>{}</m:e></m:nary>",
            t("M1"),
            t("M2"),
            t("M3")
        ),
        format!("<m:phant><m:phantPr><m:show m:val=\"0\"/></m:phantPr><m:e>{}</m:e></m:phant>", t("N1")),
        format!("<m:rad><m:deg>{}</m:deg><m:e>{}</m:e></m:rad>", t("O1"), t("O2")),
        format!("<m:sPre><m:sub>{}</m:sub><m:sup>{}</m:sup><m:e>{}</m:e></m:sPre>", t("P1"), t("P2"), t("P3")),
        format!("<m:sSub><m:e>{}</m:e><m:sub>{}</m:sub></m:sSub>", t("Q1"), t("Q2")),
        format!("<m:sSubSup><m:e>{}</m:e><m:sub>{}</m:sub><m:sup>{}</m:sup></m:sSubSup>", t("R1"), t("R2"), t("R3")),
        format!("<m:sSup><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>", t("S1"), t("S2")),
        // Run-level wrappers allowed in math (EG_PContentMath): hyperlinks, simple fields,
        // tracked insertions, content controls, markup compatibility.
        format!("<w:hyperlink w:anchor=\"x\">{}</w:hyperlink>", t("T1")),
        format!("<w:fldSimple w:instr=\"QUOTE\">{}</w:fldSimple>", t("T2")),
        format!("<w:ins w:id=\"1\" w:author=\"a\">{}</w:ins>", t("T3")),
        format!("<w:sdt><w:sdtContent>{}</w:sdtContent></w:sdt>", t("T4")),
        format!(
            "<mc:AlternateContent><mc:Choice Requires=\"m\">{}</mc:Choice><mc:Fallback><w:r><w:t>picture</w:t></w:r></mc:Fallback></mc:AlternateContent>",
            t("T5")
        ),
        "<w:r><w:t>T6</w:t></w:r>".to_string(),
    ];
    let x = format!(
        r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">{}</m:oMath>"#,
        parts.concat()
    );
    let e = parse(&x).unwrap();
    let mut m = read_omath(&e, MathJc::CenterGroup);
    let got = texts(&mut m).concat();
    let markers = [
        "A1", "B1", "C1", "D1", "E1", "E2", "F1", "F2", "G1", "G2", "H1", "H2", "I1", "J1", "J2", "K1", "K2", "L1", "L2", "L3", "M1", "M2", "M3",
        "N1", "O1", "O2", "P1", "P2", "P3", "Q1", "Q2", "R1", "R2", "R3", "S1", "S2", "T1", "T2", "T3", "T4", "T5", "T6",
    ];
    for k in markers {
        assert!(got.contains(k), "{k} dropped from {got}");
    }
    assert!(!got.contains("picture"), "the math choice wins over the fallback: {got}");
}

/// #1273: deleted text stays out; symbol characters and symbol-font text become Unicode.
#[test]
fn symbols_and_deletions_in_math() {
    let e = omath(
        r#"<m:r><m:t>x</m:t></m:r><w:del w:id="1" w:author="a"><m:r><m:t>gone</m:t></m:r></w:del><m:r><w:sym w:font="Symbol" w:char="F061"/></m:r><m:r><w:rPr><w:rFonts w:ascii="Symbol" w:hAnsi="Symbol"/></w:rPr><m:t>&#xF062;</m:t><w:noBreakHyphen/></m:r><w:r><w:rPr><w:rFonts w:ascii="Symbol"/></w:rPr><w:t>&#xF067;</w:t></w:r>"#,
    );
    let mut m = read_omath(&e, MathJc::CenterGroup);
    let got = texts(&mut m).concat();
    assert_eq!(got, "xαβ\u{2011}γ");
    let mut font = None;
    crate::math_tree::each_run_mut(&mut m.nodes, &mut |r| {
        if r.nor {
            font = Some(r.font.clone());
        }
    });
    assert_eq!(font, Some(None), "mapped symbol text takes the text font");
}

/// Hostile nesting through the wrappers is bounded (no stack overflow, no panic).
#[test]
fn deeply_nested_wrappers_are_bounded() {
    // Deeper than the math limit, within the XML parser's own depth limit.
    let n = 120;
    let inner = format!("{}{}{}", "<w:hyperlink>".repeat(n), t("deep"), "</w:hyperlink>".repeat(n));
    let e = omath(&inner);
    let _ = read_omath(&e, MathJc::CenterGroup);
    let inner = format!(
        "{}{}{}",
        "<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice Requires=\"m\">".repeat(n),
        t("deep"),
        "</mc:Choice></mc:AlternateContent>".repeat(n)
    );
    let e = omath(&inner);
    let _ = read_omath(&e, MathJc::CenterGroup);
}

#[test]
fn text_entry_points_find_the_equation_and_refuse_junk() {
    let ns = r#"xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#;
    // Any prefix, inside other markup, as a display paragraph with its justification.
    let x = r#"<p><o:oMathPara xmlns:o="http://schemas.openxmlformats.org/officeDocument/2006/math"><o:oMathParaPr><o:jc o:val="left"/></o:oMathParaPr><o:oMath><o:r><o:t>x</o:t></o:r></o:oMath></o:oMathPara></p>"#;
    let m = read_str(x).unwrap();
    assert_eq!((crate::math::to_linear(&m.nodes).as_str(), m.jc), ("x", MathJc::Left));
    // The strict OOXML namespace.
    let strict = r#"<m:oMath xmlns:m="http://purl.oclc.org/ooxml/officeDocument/math"><m:r><m:t>y</m:t></m:r></m:oMath>"#;
    assert_eq!(read_str(strict).map(|m| crate::math::to_linear(&m.nodes)).as_deref(), Some("y"));
    // Kept OMML, prefixes without declarations.
    assert_eq!(read_str("<m:oMath><m:r><m:t>z</m:t></m:r></m:oMath>").map(|m| crate::math::to_linear(&m.nodes)).as_deref(), Some("z"));
    for junk in ["", "<p/>", "not xml", "<m:oMath", "\u{0}"] {
        let _ = read_str(junk);
    }
    assert!(read_str("<p/>").is_none() && read_str("").is_none());
    // More elements than the reader keeps: refused, not read in part.
    let wide = format!("<m:oMath {ns}>{}</m:oMath>", "<m:r/>".repeat(MAX_ELEMENTS * 2 + 1));
    assert!(parse(&wide).is_none());
    let fits = format!("<m:oMath {ns}>{}</m:oMath>", "<m:r><m:t>a</m:t></m:r>".repeat(1000));
    assert!(parse(&fits).is_some());
    // Deep nesting is bounded.
    let deep = format!("<m:oMath {ns}>{}<m:r><m:t>x</m:t></m:r>{}</m:oMath>", "<m:box><m:e>".repeat(5000), "</m:e></m:box>".repeat(5000));
    let _ = read_str(&deep);
}

#[test]
fn kept_omml_is_written_back_only_when_well_formed() {
    let m = read_str(r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:r><m:t>x</m:t></m:r></m:oMath>"#).unwrap();
    assert_eq!(write_str("", false, &m, None), m.omml);
    let broken = Math { omml: "<m:oMath><m:r>".into(), ..m.clone() };
    let out = write_str("", false, &broken, None);
    assert!(out.starts_with("<m:oMath>") && out.ends_with("</m:oMath>") && out.contains(">x</m:t>"), "{out}");
    // A colour that differs from the kept one rewrites the equation from the tree, with the colour.
    let red = write_str("", false, &m, Some(Rgb(255, 0, 0)));
    assert!(red.contains(r#"<w:color w:val="FF0000"/>"#), "{red}");
}

proptest::proptest! {
    #[test]
    fn any_text_reads_without_panicking(s in "[<>/=\"a-z:m ]{0,120}") {
        let _ = read_str(&s);
        let _ = crate::mathml::parse(&s).map(|e| crate::mathml::to_tree(&e));
    }
}
