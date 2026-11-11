//! Integration tests of craft-design-lint: rules.

#![allow(clippy::unwrap_used, reason = "tests")]

use craft_design_lint::{Finding, analyze_source};

fn run(src: &str) -> Vec<Finding> {
    analyze_source(src, false).findings
}

fn rules(src: &str) -> Vec<&'static str> {
    run(src).iter().map(|f| f.rule).collect()
}

fn flagged(src: &str, rule: &str) -> bool {
    rules(src) == [rule]
}

fn clean(src: &str) -> bool {
    let findings = run(src);
    findings.is_empty() || panic!("unexpected findings: {findings:#?}")
}

#[test]
fn literal_shapes() {
    for arg in ["-4.0", "(4.0)", "4 as f32", "4.0_f32", "2.0 * 3.0", "-(1.0 + 2.0)", "8"] {
        let src = format!("fn f(ui: &mut Ui) {{ ui.add_space({arg}); }}");
        assert!(flagged(&src, "literal-size/add-space"), "{arg}");
    }
    for arg in ["0.0", "-0.0", "0", "0.0 * 5.0", "gap", "self.gap", "tokens.space.sm", "pad()", "w / 2.0"] {
        let src = format!("fn f(ui: &mut Ui) {{ ui.add_space({arg}); }}");
        assert!(clean(&src), "{arg}");
    }
}

#[test]
fn consts_resolve_by_module_and_impl_scope() {
    // A module-level const of the same name in another module is a different const.
    let src = "mod a { const GAP: f32 = 0.0; fn f(ui: &mut Ui) { ui.add_space(GAP); } }\nmod b { const GAP: f32 = 4.0; fn g(ui: &mut Ui) { ui.add_space(GAP); } }\n";
    let found = run(src);
    assert_eq!(found.iter().map(|f| f.line).collect::<Vec<_>>(), [2]);

    // Shadowing: the innermost scope wins, so a zero outer const does not hide a non-zero inner one.
    let src =
        "const GAP: f32 = 0.0;\nmod m { const GAP: f32 = 4.0; fn f(ui: &mut Ui) { ui.add_space(GAP); } }\nfn g(ui: &mut Ui) { ui.add_space(GAP); }\n";
    assert_eq!(run(src).iter().map(|f| f.line).collect::<Vec<_>>(), [2]);

    // Self::X and Type::X name the impl's associated const; the message shows the path as written.
    let src = "struct A; struct B;\nimpl A { const PAD: f32 = 6.0; fn f(&self, ui: &mut Ui) { ui.add_space(Self::PAD); } }\nimpl B { const PAD: f32 = 0.0; fn f(&self, ui: &mut Ui) { ui.add_space(Self::PAD); ui.add_space(A::PAD); } }\n";
    let found = run(src);
    assert_eq!(found.iter().map(|f| f.line).collect::<Vec<_>>(), [2, 3]);
    assert!(found[0].message.contains("`Self::PAD` (const, line 2)"), "{}", found[0].message);
    assert!(found[1].message.contains("`A::PAD`"), "{}", found[1].message);

    // mod::X, self::X, super::X and crate::X.
    let src = "const TOP: f32 = 1.0;\nmod sizes { pub const BIG: f32 = 9.0; }\nmod ui { fn f(ui: &mut Ui) { ui.add_space(super::TOP); ui.add_space(crate::TOP); ui.add_space(crate::sizes::BIG); ui.add_space(self::NONE); } }\nconst NONE: f32 = 0.0;\n";
    assert_eq!(run(src).len(), 3);
}

#[test]
fn clippy_allow_attributes_are_counted_and_inner_ones_flagged() {
    let src = "#![allow(clippy::disallowed_methods, reason = \"stock widgets\")]\n#[expect(clippy::disallowed_types)]\nstruct S;\n#[allow(clippy::too_many_arguments)]\nfn f() {}\nmod m {\n    #![expect(clippy::disallowed_methods)]\n}\n";
    let analysis = analyze_source(src, false);
    assert_eq!(analysis.clippy_allows, 3);
    assert_eq!(analysis.findings.iter().map(|f| (f.rule, f.line)).collect::<Vec<_>>(), [("clippy/inner-allow", 1), ("clippy/inner-allow", 7)]);
    assert!(analysis.findings[0].message.contains("[[exempt]]"), "{}", analysis.findings[0].message);
}
