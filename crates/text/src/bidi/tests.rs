use super::*;
use craft_ui::testing::trial;

fn rev(s: &str) -> String {
    s.chars().rev().collect()
}

/// The line as displayed, left to right, built from its visual runs.
fn shown(p: &Paragraph<'_>, line: Range<usize>) -> String {
    p.visual_runs(line).iter().map(|r| if r.is_rtl() { rev(&p.text()[r.range.clone()]) } else { p.text()[r.range.clone()].to_string() }).collect()
}

#[trial]
fn mixed_arabic_latin_numbers_and_brackets() {
    let s = "سنة 2024 (Retrait) فقط";
    let p = Paragraph::new(s, Direction::Rtl);
    let l = p.char_levels();
    let chars: Vec<char> = s.chars().collect();
    assert_eq!(l.len(), chars.len());
    // Arabic letters at the paragraph level 1; European digits and Latin at 2.
    assert_eq!(l[0], 1);
    assert_eq!(&l[4..8], &[2, 2, 2, 2]);
    assert_eq!(&l[10..17], &[2; 7]);
    assert_eq!(l[19], 1);
    // Brackets around Latin inside a right-to-left paragraph take the embedding direction (N0).
    assert_eq!(l[9] % 2, 1, "( is right to left");
    assert_eq!(l[17] % 2, 1, ") is right to left");
    // The Arabic words reversed, Latin and digits kept, the brackets swapped (the shaper mirrors
    // them back, rule L4). Char-level reordering and byte runs agree.
    let order = visual_order(&line_levels(&chars, &l, p.level()));
    let vis: String = order.iter().map(|&i| chars[i]).collect();
    assert_eq!(vis, "طقف )Retrait( 2024 ةنس");
    assert_eq!(shown(&p, 0..s.len()), vis);
}

#[trial]
fn hostile_text_never_panics() {
    // Explicit embeddings, isolates, unmatched pops, marks with no base, deep nesting.
    let rlo = "\u{202E}".repeat(200);
    let junk = [
        "\u{202B}\u{202B}\u{202C}\u{202C}\u{202C}abc\u{2067}سلام\u{2069}\u{2069}",
        "\u{064E}\u{064E}\u{200C}\u{200D}",
        "\u{200F}\u{200E}\t\u{200F}\n\u{200E}",
        "(((((((((سلام]]]]]]]",
        "a\u{2029}سلام\u{2029}b",
        &rlo,
    ];
    for s in junk {
        for base in [Direction::Ltr, Direction::Rtl, Direction::Auto] {
            let overrides = [Override { range: 0..s.len() / 2, rtl: true }, Override { range: s.len() / 2..s.len(), rtl: false }];
            for p in [Paragraph::new(s, base), Paragraph::with_overrides(s, base, &overrides)] {
                assert_eq!(p.levels().len(), s.len());
                let paras = p.paragraphs();
                assert_eq!((paras.first().map(|p| p.0.start), paras.last().map(|p| p.0.end)), (Some(0), Some(s.len())), "{s:?}");
                assert!(paras.windows(2).all(|w| w[0].0.end == w[1].0.start), "{s:?}");
                let bounds: Vec<usize> = (0..=s.len()).filter(|&b| s.is_char_boundary(b)).collect();
                for &b in &bounds {
                    for line in [0..b, b..s.len()] {
                        let runs = p.visual_runs(line.clone());
                        assert_eq!(runs.iter().map(|r| r.range.len()).sum::<usize>(), line.len(), "{s:?} {line:?}");
                        assert!(runs.iter().all(|r| !r.range.is_empty() && line.start <= r.range.start && r.range.end <= line.end));
                    }
                }
            }
        }
    }
}

#[trial]
fn runs_never_split_a_grapheme_cluster() {
    use unicode_segmentation::UnicodeSegmentation;
    let cases = [
        // Arabic with harakat, among Latin and digits.
        ("كَتَبَ abc مُحَمَّدٌ 123 بِسْمِ", Direction::Rtl),
        ("see كَتَبَ, مُحَمَّدٌ here", Direction::Ltr),
        // Hebrew with niqqud (and a dagesh-shin-dot stack).
        ("שָׁלוֹם hello עוֹלָם", Direction::Rtl),
        ("hello שָׁלוֹם world", Direction::Ltr),
        // A Devanagari conjunct (virama, and an explicit half form with ZWJ) in right-to-left text.
        ("سلام क्षत्रिय क्\u{200D}ष سلام", Direction::Rtl),
        ("سلام क्\u{200D} ", Direction::Rtl),
        // A combining mark after a Latin letter in a right-to-left paragraph, and after a Hebrew
        // letter in a left-to-right one, each at a run edge.
        ("שלום e\u{0301}te\u{0301} עולם", Direction::Rtl),
        ("שלום cafe\u{0301}", Direction::Rtl),
        ("café ש\u{05B8}\u{05BC} next", Direction::Ltr),
        ("abc ש\u{05B8}", Direction::Ltr),
        ("a\u{0301}\u{0302}ש", Direction::Auto),
    ];
    for (s, base) in cases {
        let p = Paragraph::new(s, base);
        let bounds: Vec<usize> = s.grapheme_indices(true).map(|(i, _)| i).chain([s.len()]).collect();
        for &a in &bounds {
            for &b in bounds.iter().filter(|&&b| b > a) {
                for r in p.visual_runs(a..b) {
                    assert!(bounds.contains(&r.range.start) && bounds.contains(&r.range.end), "{s:?} line {a}..{b}: run {r:?}");
                }
            }
        }
    }
    // The char-level L1 keeps a trailing joiner with its conjunct too.
    let s = "سلام क्\u{200D} ";
    let chars: Vec<char> = s.chars().collect();
    let l = line_levels(&chars, &Paragraph::new(s, Direction::Rtl).char_levels(), 1);
    assert_eq!((l[6], l[7], l[8]), (2, 2, 1));
}
