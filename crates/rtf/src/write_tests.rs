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

#[test]
fn outline_writer_emits_powerpoint_headings_runs_and_unicode() {
    let mut w = OutlineWriter::new("Liberation Sans");
    w.begin_title();
    w.run("First", true, false, false);
    w.line_break();
    w.run("Café 😀", false, true, true);
    w.end_title();
    w.begin_body(0);
    w.run("Body", false, false, false);
    w.end_body();
    w.begin_body(12);
    w.run("Deep", false, false, false);
    w.end_body();
    // Empty untitled slide title.
    w.begin_title();
    w.run("", false, false, false);
    w.end_title();
    let rtf = w.finish();

    assert!(rtf.contains("{\\f0\\fswiss Liberation Sans;}"), "{rtf}");
    assert!(rtf.contains("{\\s9\\li3240\\fi-360\\level9 heading 9;}"), "{rtf}");
    assert!(rtf.contains("\\pard\\s1\\outlinelevel0\\sb240\\b\\fs32 {\\b First}\\line {\\i\\ul Caf\\u233? \\u-10179?\\u-8704?}\\b0\\par\n"), "{rtf}");
    assert!(rtf.contains("\\pard\\s2\\outlinelevel1\\li360\\fs24 Body\\par\n"), "{rtf}");
    assert!(rtf.contains("\\pard\\s9\\outlinelevel9\\li3240\\fs24 Deep\\par\n"), "{rtf}");
    assert!(rtf.contains("\\pard\\s1\\outlinelevel0\\sb240\\b\\fs32 \\b0\\par\n"), "{rtf}");

    let doc = crate::read(rtf.as_bytes()).unwrap();
    assert_eq!(doc.blocks.len(), 4);
}
