use super::*;

fn rev(s: &str) -> String {
    s.chars().rev().collect()
}

/// Byte offset of char `n` of `s`.
fn at(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map_or(s.len(), |(i, _)| i)
}

/// The line as displayed, left to right, built from its visual runs.
fn shown(p: &Paragraph<'_>, line: Range<usize>) -> String {
    p.visual_runs(line).iter().map(|r| if r.is_rtl() { rev(&p.text()[r.range.clone()]) } else { p.text()[r.range.clone()].to_string() }).collect()
}

fn shown_all(text: &str, base: Direction) -> String {
    shown(&Paragraph::new(text, base), 0..text.len())
}

#[test]
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

#[test]
fn levels_for_persian_with_latin_and_numbers() {
    let p = Paragraph::new("plain English", Direction::Ltr);
    assert!(!p.needs_reordering(), "pure left-to-right paragraphs skip the algorithm");
    assert_eq!(p.levels(), &[0; 13]);
    let s = "سلام Word ۱۴۰۳";
    let p = Paragraph::new(s, Direction::Rtl);
    assert_eq!(p.levels().len(), s.len());
    assert!(p.needs_reordering());
    assert_eq!(p.level_at(0), 1, "Persian letters are right to left");
    assert_eq!(p.level_at(at(s, 5)), 2, "Latin inside a right-to-left paragraph is embedded at level 2");
    assert_eq!(p.level_at(at(s, 10)), 2, "Persian digits are numbers: left to right inside right-to-left text");
    assert_eq!(p.level_at(s.len()), 1, "past the end: the paragraph level");
    // In a left-to-right paragraph Persian is embedded at level 1.
    let s = "Hello سلام";
    let p = Paragraph::new(s, Direction::Ltr);
    assert_eq!((p.level_at(0), p.level_at(at(s, 6))), (0, 1));
    // A right-to-left paragraph of Latin text keeps it left to right inside it.
    assert_eq!(Paragraph::new("ab", Direction::Rtl).char_levels(), vec![2, 2]);
}

#[test]
fn auto_direction_follows_the_first_strong_character() {
    assert!(Paragraph::new("سلام Word", Direction::Auto).is_rtl());
    assert!(!Paragraph::new("Word سلام", Direction::Auto).is_rtl());
    assert!(Paragraph::new("123 سلام", Direction::Auto).is_rtl(), "digits are weak");
    assert_eq!(Paragraph::new("123", Direction::Auto).level(), 0);
    assert_eq!(Paragraph::new("", Direction::Auto).level(), 0);
    assert_eq!(Paragraph::new("", Direction::Rtl).level(), 1);
    assert_eq!(Paragraph::new("Word", Direction::Rtl).level(), 1, "an explicit direction wins");
}

#[test]
fn whole_lines_read_in_display_order() {
    let s = "سلام دنیا";
    assert_eq!(shown_all(s, Direction::Rtl), rev(s));
    assert_eq!(shown_all("Hello سلام world", Direction::Ltr), "Hello مالس world");
    // "Price 250 toman" in Persian, with a Latin product name, Persian punctuation and digits:
    // numbers and Latin keep their order, the words around them read right to left, and the final
    // full stop is at the paragraph's end (the far left).
    let s = "قیمت 250 تومان برای Word، امسال ۱۴۰۳.";
    let want = format!(".۱۴۰۳ {} ،Word {} {} 250 {}", rev("امسال"), rev("برای"), rev("تومان"), rev("قیمت"));
    assert_eq!(shown_all(s, Direction::Rtl), want);
}

#[test]
fn each_line_is_reordered_on_its_own() {
    // A right-to-left paragraph of Latin words broken after "ab ": the trailing space takes the
    // paragraph level (rule L1), so it sits at the line's end edge, on the left.
    let p = Paragraph::new("ab cd", Direction::Rtl);
    assert_eq!(p.visual_runs(0..3), vec![Run { range: 2..3, level: 1 }, Run { range: 0..2, level: 2 }]);
    assert_eq!(p.visual_runs(3..5), vec![Run { range: 3..5, level: 2 }]);
    // Line ranges that aren't char-boundary ranges of the text give nothing.
    let p = Paragraph::new("سلام", Direction::Rtl);
    assert!(p.visual_runs(1..2).is_empty());
    assert!(p.line_levels(0..99).is_empty());
    #[allow(clippy::reversed_empty_ranges)]
    let backwards = 4..2;
    assert!(p.visual_runs(backwards).is_empty());
    assert!(p.visual_runs(2..2).is_empty());
}

#[test]
fn l1_resets_trailing_whitespace_and_tabs() {
    let chars: Vec<char> = "ab\tcd  ".chars().collect();
    assert_eq!(line_levels(&chars, &[2; 7], 1), vec![2, 2, 1, 2, 2, 1, 1]);
    assert_eq!(Paragraph::new("ab\tcd  ", Direction::Rtl).line_levels(0..7), vec![2, 2, 1, 2, 2, 1, 1]);
    // Short level slices never panic.
    assert_eq!(line_levels(&['a', 'b'], &[], 1), vec![1, 1]);
}

#[test]
fn visual_order_follows_rule_l2() {
    // The example from UAX #9 / unicode-bidi.
    assert_eq!(visual_order(&[0, 0, 0, 1, 1, 1, 2, 2]), vec![0, 1, 2, 6, 7, 5, 4, 3]);
    assert!(visual_order(&[]).is_empty());
    assert_eq!(visual_order(&[0, 0]), vec![0, 1]);
    assert_eq!(visual_order(&[1, 1, 1]), vec![2, 1, 0]);
    // Numbers (level 2) inside right-to-left text keep their order.
    assert_eq!(visual_order(&[1, 2, 2, 1]), vec![3, 1, 2, 0]);
    assert_eq!(visual_order(&[125, 125]), vec![1, 0]);
    assert_eq!(visual_order(&[255, 0, 255]), vec![0, 1, 2]);
}

#[test]
fn overrides_force_a_direction() {
    let p = Paragraph::with_overrides("abc def", Direction::Ltr, &[Override { range: 0..3, rtl: true }]);
    assert_eq!(p.levels().len(), 7);
    assert_eq!(shown(&p, 0..7), "cba def");
    let s = "سلام دنیا";
    let p = Paragraph::with_overrides(s, Direction::Rtl, &[Override { range: 0..at(s, 4), rtl: false }]);
    assert_eq!(shown(&p, 0..s.len()), format!("{} سلام", rev("دنیا")));
    // Overrides off char boundaries, empty, out of range or overlapping an earlier one are ignored.
    let bad = [Override { range: 1..2, rtl: false }, Override { range: 3..3, rtl: false }, Override { range: 0..99, rtl: false }];
    assert_eq!(Paragraph::with_overrides(s, Direction::Rtl, &bad).levels(), Paragraph::new(s, Direction::Rtl).levels());
    let overlapping = [Override { range: 2..5, rtl: true }, Override { range: 0..3, rtl: true }];
    assert_eq!(shown(&Paragraph::with_overrides("abcdef", Direction::Ltr, &overlapping), 0..6), "cbadef");
}

#[test]
fn each_paragraph_has_its_own_level() {
    // The second paragraph resolves right to left on its own, so rule L1 puts its trailing space at
    // level 1: at the line's end edge, on the left.
    let s = "a\nאב ";
    let p = Paragraph::new(s, Direction::Auto);
    assert_eq!(p.paragraphs(), &[(0..2, 0), (2..7, 1)]);
    assert_eq!((p.level(), p.is_rtl()), (0, false), "level() is the first paragraph's");
    assert_eq!((p.paragraph_level_at(1), p.paragraph_level_at(2), p.level_at(s.len())), (0, 1, 1));
    assert_eq!(p.line_levels(2..7), vec![1; 5]);
    assert_eq!(p.visual_runs(2..7), vec![Run { range: 2..7, level: 1 }]);
    assert_eq!(p.line_levels(0..2), vec![0, 0]);
    // Left-to-right text skips the algorithm but still splits paragraphs.
    assert_eq!(Paragraph::new("ab\u{2029}c", Direction::Ltr).paragraphs(), &[(0..5, 0), (5..6, 0)]);
    assert_eq!(Paragraph::new("", Direction::Auto).paragraphs(), &[(0..0, 0)]);
}

#[test]
fn overrides_continue_across_paragraph_separators() {
    let s = "ab\ncd";
    let p = Paragraph::with_overrides(s, Direction::Ltr, &[Override { range: 0..5, rtl: true }]);
    assert_eq!(p.paragraphs(), &[(0..3, 0), (3..5, 0)]);
    assert_eq!(p.line_levels(0..3), vec![1, 1, 0], "the separator keeps the paragraph level");
    assert_eq!(shown(&p, 0..3), "ba\n");
    assert_eq!(shown(&p, 3..5), "dc");
    // Each paragraph still resolves its own direction.
    let s = "a\nאב ";
    let p = Paragraph::with_overrides(s, Direction::Auto, &[Override { range: 0..1, rtl: false }]);
    assert_eq!(p.paragraphs(), &[(0..2, 0), (2..7, 1)]);
    assert_eq!(p.visual_runs(2..7), vec![Run { range: 2..7, level: 1 }]);
}

#[test]
fn mirrored_characters() {
    assert_eq!(mirror('('), Some(')'));
    assert_eq!(mirror(']'), Some('['));
    assert_eq!(mirror('«'), Some('»'));
    assert_eq!(mirror('a'), None);
    assert_eq!(mirror('∑'), None, "mirrored, but only by glyph");
}

#[test]
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

#[test]
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
