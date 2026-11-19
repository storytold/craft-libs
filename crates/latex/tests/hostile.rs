//! Hostile and generated input, from WordCraft's cross-format tests (`crates/formats/src/tests.rs`):
//! nesting, column specs and labels are bounded and linear, the export carries no TeX commands
//! from the document, and text survives a round trip.

use craft_latex::{export, import, linear_to_latex};
use craft_wordmodel::Document;
use std::time::{Duration, Instant};

fn text(tex: &str) -> String {
    import(tex.as_bytes()).plain_text(Default::default())
}

#[test]
fn deep_nesting_is_bounded() {
    assert!(text(&("{\\textbf{".repeat(50_000) + "x")).contains('x'));
    assert!(text(&("\\begin{itemize}\\item ".repeat(10_000) + "deep")).contains("deep"));
    let _ = import(("\\begin{tabular}{l}".repeat(2_000) + "cell").as_bytes());
    let _ = import(format!("${}x{}$", "\\frac{".repeat(10_000), "}".repeat(10_000)).as_bytes());
    // `*{n}{…}` column specs nested 100k deep, and repeats multiplying past what a table holds.
    assert!(text(&format!("\\begin{{tabular}}{{{}l{}}}x\\end{{tabular}}", "*{1}{".repeat(100_000), "}".repeat(100_000))).contains('x'));
    assert!(text(&format!("\\begin{{tabular}}{{{}p{{1cm}}{}}}x\\end{{tabular}}", "*{64}{".repeat(30), "}".repeat(30))).contains('x'));
    // An equation of unmatched `(` and one nested 100k deep convert in linear time.
    let started = Instant::now();
    assert_eq!(linear_to_latex(&"(".repeat(200_000)).len(), 200_000);
    let nested = format!("{}x{}", "(".repeat(100_000), ")".repeat(100_000));
    assert_eq!(linear_to_latex(&nested).len(), 200_001);
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    assert_eq!(linear_to_latex("(a)/(b)"), "\\frac{a}{b}");
    assert_eq!(linear_to_latex("((a)/(b))/(c)"), "\\frac{\\frac{a}{b}}{c}");
    // `\begin{` never closed.
    let started = Instant::now();
    let _ = import("\\begin{".repeat(300_000).as_bytes());
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
}

/// What a document says in an equation or a bookmark name is not written to the exported file as
/// TeX commands: compiling the export must not read files or run programs the document names.
#[test]
fn latex_export_carries_no_commands_from_the_document() {
    let tex = linear_to_latex("\\input secret.txt \\immediate\\write18\\bgroup id\\egroup \\csname x\\endcsname");
    for cmd in ["\\input", "\\immediate", "\\write", "\\bgroup", "\\egroup", "\\csname", "\\endcsname"] {
        assert!(!tex.contains(cmd), "{cmd} in {tex}");
    }
    assert!(tex.contains("\\backslash input"), "{tex}");
    for linear in ["a^^5cinput secret.txt ", "a^^^^5cinput", "x^(^^5cinput)", "^^5cinput"] {
        assert!(!linear_to_latex(linear).contains("^^"), "{linear}");
    }
    let deep = format!("{}^^5cinput x{}", "x^(".repeat(40), ")".repeat(40));
    assert!(!linear_to_latex(&deep).contains("^^"));
    assert_eq!(linear_to_latex("\\alpha+\\sin x+\\mathbb R"), "\\alpha+\\sin x+\\mathbb R");
    assert_eq!(linear_to_latex("x^2^3"), "x^2^3");
    let d = import("Text $\\input secret.txt $ and \\hypertarget{^^5cinput secret.txt ^^5c}{here}.".as_bytes());
    let out = export(&d);
    assert!(!out.contains("\\input") && !out.contains("^^"), "{out}");
}

/// The fastest of three reads of `src`.
fn fastest(src: &str) -> Duration {
    (0..3)
        .map(|_| {
            let started = Instant::now();
            assert!(import(src.as_bytes()).body.len() <= 2);
            started.elapsed()
        })
        .min()
        .unwrap_or_default()
}

/// Labels in a row are read in linear time: four times as many take well under the sixteen times
/// as long a quadratic read would (a ratio, so a busy machine does not fail it).
#[test]
fn latex_labels_in_a_row_are_read_in_linear_time() {
    let cases: [fn(usize) -> String; 3] = [|n| "\\label{a} ".repeat(n) + "x", |n| "\\label{a}".repeat(n) + "x", |n| "\\hypertarget{a}{} ".repeat(n)];
    for make in cases {
        let (small, large) = (fastest(&make(25_000)), fastest(&make(100_000)));
        assert!(large < small * 8 + Duration::from_millis(50), "{small:?} for 25,000, {large:?} for 100,000");
    }
    let d = import("\\label{one}First \\label{two}second \\label{three}third.".as_bytes());
    let p = d.para_at(&craft_wordmodel::Pos::body(0, 0)).unwrap();
    let names: Vec<&str> = p
        .objects
        .iter()
        .filter_map(|o| if let craft_wordmodel::InlineObject::BookmarkStart { name } = o { Some(name.as_str()) } else { None })
        .collect();
    assert_eq!(names, ["one", "two", "three"]);
    assert_eq!(d.plain_text(craft_wordmodel::StoryRef::Body).replace('\u{FFFC}', ""), "First second third.");
}

proptest::proptest! {
    #[test]
    fn fuzz_latex(s in "(\\\\(begin|end)\\{(itemize|tabular|verbatim|quote|center|x)\\}|\\\\[a-zA-Z]{1,8}\\*?|\\\\[^a-zA-Z]|[{}\\[\\]$&%~^_#]|\n\n|[a-z ]){0,150}") {
        let d = import(s.as_bytes());
        let _ = export(&d);
    }

    #[test]
    fn latex_text_round_trips(words in proptest::collection::vec("[a-zA-Z0-9{}$&%#_~^\\\\<>|`'.!-]{1,8}", 1..8)) {
        let text = words.join(" ");
        let tex = export(&Document::from_text(&text));
        let got = import(tex.as_bytes()).plain_text(Default::default());
        proptest::prop_assert_eq!(got.trim(), text.trim(), "tex: {}", tex);
    }
}
