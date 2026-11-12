//! Round trips over equaSHion's corpus (storytold/craft-libs#29): its typed LaTeX equations
//! (`corpus/equations.tsv`), the MathLive-style LaTeX of `corpus/latex.tsv` and its OMML samples
//! from WordCraft and DeckCraft files (`corpus/omml/`).

use craft_math::{Math, math, math_latex, mathml, omml};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus");

fn rows(file: &str) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(format!("{DIR}/{file}")).unwrap();
    text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).map(|l| l.split('\t').map(str::to_string).collect()).collect()
}

/// The equations of `equations.tsv`: (id, tree, display).
fn equations() -> Vec<(String, math::Arg, bool)> {
    let v: Vec<_> = rows("equations.tsv").into_iter().map(|f| (f[0].clone(), math_latex::parse_latex(&f[1]), f[2].trim() == "1")).collect();
    assert!(v.len() >= 40, "{} rows", v.len());
    v
}

#[test]
fn every_corpus_equation_reads_without_unknown_commands() {
    for (id, tree, _) in equations() {
        let lin = math::to_linear(&tree);
        assert!(!tree.is_empty() && !lin.contains('\\'), "{id}: {lin}");
    }
    for f in rows("latex.tsv") {
        let lin = math::to_linear(&math_latex::parse_latex(&f[0]));
        assert!(!lin.contains('\\'), "{}: {lin}", f[0]);
    }
}

#[test]
fn every_corpus_equation_survives_omml() {
    for (id, tree, display) in equations() {
        let eq = Math { nodes: tree.clone(), ..Default::default() };
        let xml = omml::write_str("", display, &eq, None);
        let back = omml::read_str(&xml).unwrap_or_else(|| panic!("{id}: {xml}"));
        // The writer states matrix column alignment the parser left implicit; the text is the same.
        assert_eq!(math::to_linear(&back.nodes), math::to_linear(&tree), "{id}: {xml}");
        // And the OMML written from what was read is the same OMML.
        let again = omml::write_str("", display, &Math { omml: String::new(), ..back }, None);
        assert_eq!(again, xml, "{id}");
    }
}

#[test]
fn every_corpus_equation_survives_mathml_and_latex() {
    for (id, tree, display) in equations() {
        let lin = math::to_linear(&tree);
        let ml = mathml::to_mathml(&tree, display, "");
        assert_eq!(mathml::parse(&ml).and_then(|m| mathml::read(&m)), Some((lin.clone(), display)), "{id}: {ml}");
        let tex = math_latex::to_latex(&tree);
        assert_eq!(math::to_linear(&math_latex::parse_latex(&tex)), lin, "{id}: {tex}");
    }
}

#[test]
fn omml_samples_read_and_write_back() {
    let mut n = 0;
    for e in std::fs::read_dir(format!("{DIR}/omml")).unwrap().flatten() {
        let xml = std::fs::read_to_string(e.path()).unwrap();
        let name = e.file_name();
        let m = omml::read_str(&xml).unwrap_or_else(|| panic!("{name:?}"));
        assert!(!m.nodes.is_empty() && m.omml.starts_with("<m:oMath"), "{name:?}");
        // Kept: the source goes back as it was read.
        let kept = omml::write_str("", false, &m, None);
        assert_eq!(kept, m.omml, "{name:?}");
        // Rewritten from the tree, it reads back to the same tree.
        let fresh = omml::write_str("", false, &Math { omml: String::new(), ..m.clone() }, None);
        assert_eq!(omml::read_str(&fresh).map(|b| b.nodes), Some(m.nodes.clone()), "{name:?}: {fresh}");
        n += 1;
    }
    assert_eq!(n, 6);
}
