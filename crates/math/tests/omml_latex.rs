//! OMML read into the tree and written as LaTeX means what GridCraft's direct OMML-to-LaTeX
//! reader (`crates/xlsx/src/omml.rs`) gave for its test cases, so GridCraft can read equations in
//! drawings through this crate. The LaTeX is compared by what it reads back as, not byte for byte.

use craft_math::{math, math_latex, omml};
fn tex(inner: &str) -> String {
    let x = format!(r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math">{inner}</m:oMath>"#);
    math_latex::to_latex(&omml::read_str(&x).unwrap().nodes)
}
#[test]
fn gridcraft_omml_cases_mean_the_same_latex() {
    let brace = |chr: &str, pos: &str| {
        format!(
            "<m:groupChr><m:groupChrPr><m:chr m:val=\"{chr}\"/><m:pos m:val=\"{pos}\"/></m:groupChrPr><m:e><m:r><m:t>x</m:t></m:r></m:e></m:groupChr>"
        )
    };
    let lim = "<m:lim><m:r><m:t>n</m:t></m:r></m:lim>";
    let under = format!("<m:limLow><m:e>{}</m:e>{lim}</m:limLow>", brace("\u{23DF}", "bot"));
    let cases: Vec<(String, &str)> = vec![
        ("<m:r><m:t>x+1</m:t></m:r>".into(), "x+1"),
        ("<m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f>".into(), "\\frac{a}{b}"),
        ("<m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup>".into(), "{x}^{2}"),
        ("<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e><m:r><m:t>π</m:t></m:r></m:e></m:rad>".into(), "\\sqrt{\\pi }"),
        ("<m:nary><m:naryPr><m:chr m:val=\"∑\"/><m:limLoc m:val=\"undOvr\"/></m:naryPr><m:sub><m:r><m:t>i=1</m:t></m:r></m:sub><m:sup><m:r><m:t>n</m:t></m:r></m:sup><m:e><m:r><m:t>i</m:t></m:r></m:e></m:nary>".into(), "\\sum_{i=1}^{n} {i}"),
        ("<m:nary><m:naryPr><m:subHide m:val=\"1\"/><m:supHide m:val=\"1\"/></m:naryPr><m:sub/><m:sup/><m:e><m:r><m:t>f</m:t></m:r></m:e></m:nary>".into(), "\\int {f}"),
        ("<m:d><m:dPr><m:begChr m:val=\"[\"/><m:endChr m:val=\"]\"/></m:dPr><m:e><m:m><m:mr><m:e><m:r><m:t>1</m:t></m:r></m:e><m:e><m:r><m:t>0</m:t></m:r></m:e></m:mr><m:mr><m:e><m:r><m:t>0</m:t></m:r></m:e><m:e><m:r><m:t>1</m:t></m:r></m:e></m:mr></m:m></m:e></m:d>".into(), "\\left[ \\begin{matrix}1 & 0 \\\\ 0 & 1\\end{matrix} \\right]"),
        ("<m:r><m:rPr><m:sty m:val=\"p\"/></m:rPr><m:t>sin</m:t></m:r><m:r><m:t>x</m:t></m:r>".into(), "\\sin x"),
        ("<m:r><m:rPr><m:scr m:val=\"double-struck\"/></m:rPr><m:t>R</m:t></m:r>".into(), "\\mathbb{R}"),
        ("<m:r><m:rPr><m:nor/></m:rPr><m:t>if {x}</m:t></m:r>".into(), "\\text{if \\{x\\}}"),
        ("<m:acc><m:accPr><m:chr m:val=\"\u{0302}\"/></m:accPr><m:e><m:r><m:t>x</m:t></m:r></m:e></m:acc>".into(), "\\hat{x}"),
        (under.clone(), "\\underbrace{x}_{n}"),
        (format!("<m:sSup><m:e>{under}</m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup>"), "{\\underbrace{x}_{n}}^{2}"),
        (format!("<m:limUpp><m:e>{}</m:e>{lim}</m:limUpp>", brace("\u{23DE}", "top")), "\\overbrace{x}^{n}"),
        (format!("<m:limLow><m:e><m:r><m:t>a</m:t></m:r></m:e>{lim}</m:limLow>"), "\\underset{n}{a}"),
        (format!("<m:limLow><m:e>{}<m:r><m:t>+1</m:t></m:r></m:e>{lim}</m:limLow>", brace("\u{23DF}", "bot")), "\\underset{n}{\\underbrace{x}+1}"),
    ];
    for (x, want) in cases {
        let got = tex(&x);
        assert_eq!(math::to_linear(&math_latex::parse_latex(&got)), math::to_linear(&math_latex::parse_latex(want)), "{x}: {got} for {want}");
    }
}
