use super::*;
use crate::model::Run;

/// A paragraph of `n` runs, each in its own font and colour.
fn varied(n: usize) -> Document {
    let runs = (0..n)
        .map(|i| Run {
            text: "x".into(),
            fmt: CharFmt { font: Some(format!("Font {i}")), color: Some([(i >> 16) as u8, (i >> 8) as u8, i as u8]), ..CharFmt::default() },
        })
        .collect();
    Document { blocks: vec![Block::Para(Para { runs, ..Para::default() })], pictures_left_out: 0 }
}

#[test]
fn many_distinct_fonts_and_colours_take_constant_steps_each() {
    let n = 3000;
    let doc = varied(n);
    let mut out = String::new();
    let steps = lookup_steps(|| out = write(&doc, "Helvetica"));
    // The default font, then two lookups per run to build the tables and two to write it.
    assert!(steps <= 4 * n + 1, "{steps} steps for {n} runs");
    assert!(out.contains(&format!("{{\\f{n}\\fnil Font {};}}", n - 1)), "the last font has the last slot");
    assert!(out.contains(&format!("\\f{n}\\cf{n} x")), "the last run names the last font and colour");
}

#[test]
fn fonts_match_without_regard_to_ascii_case() {
    let run = |font: &str| Run { text: "x".into(), fmt: CharFmt { font: Some(font.into()), ..CharFmt::default() } };
    let doc = Document {
        blocks: vec![Block::Para(Para { runs: vec![run("Arial"), run("ARIAL"), run("helvetica")], ..Para::default() })],
        pictures_left_out: 0,
    };
    let out = write(&doc, "Helvetica");
    assert!(out.contains("{\\fonttbl{\\f0\\fnil Helvetica;}{\\f1\\fnil Arial;}}"), "{out}");
    // Both spellings of Arial are font 1; the default font's run names no font.
    assert!(out.contains("{\\f1 x}{\\f1 x}x\\par"), "{out}");
}
