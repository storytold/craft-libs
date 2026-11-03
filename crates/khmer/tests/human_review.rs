#![cfg(craft_khmer_data)]
//! Selected visual-review feedback from @sungkhum, 2026-11-03.
//! These public/synthetic examples are negative boundary constraints, not a
//! claim that every unmarked position or every engine output was approved.
//! Offsets are original UTF-8 bytes. No private InDesign text is included.
use craft_khmer::line_breaks;

#[test]
fn human_rejected_boundaries_are_not_line_opportunities() {
    let cases: &[(&str, &[usize])] = &[
        ("អ្វីៗ", &[12]),
        ("ល្ងឹមរូប", &[12]),
        ("ទ្រីភេនម្ចាស់", &[12, 18, 27]),
        ("ថោរ", &[6]),
        ("កសីករ", &[3, 9]),
        ("Latin 🦀 ស្នីទ្ឋស្នាលស្រាប់ ១២៣!", &[23, 32]),
        ("សមិ្ដ", &[3, 12]),
        ("រដ្ឋការងារ", &[21]),
    ];
    for &(text, forbidden) in cases {
        let actual = line_breaks(text);
        for &at in forbidden {
            assert!(text.is_char_boundary(at) && at > 0 && at < text.len());
            assert!(!actual.contains(&at), "human-rejected boundary at byte {at}: {text}");
        }
    }
}

#[test]
fn human_accepted_finer_phrase_breaks_remain_available() {
    // The reviewer explicitly accepted this complete candidate row. Retain its
    // useful opportunities without prescribing all valid Khmer segmentations.
    let text = "ក្នុងគោលដៅបង្កើនការយល់ដឹង";
    assert_eq!(line_breaks(text), [15, 24, 30, 48, 57, 66]);
}

#[test]
fn documented_compound_keeps_a_useful_internal_opportunity() {
    // W3C sealreq issue 35 cites this optional compound division as useful for
    // avoiding excessive blank space. It is not a mandatory word definition.
    // https://github.com/w3c/sealreq/issues/35
    let text = "សហរដ្ឋអាមេរិក";
    assert!(line_breaks(text).contains(&"សហរដ្ឋ".len()));
    assert!(line_breaks("សហរដ្ឋ\u{2060}អាមេរិក").is_empty());
    assert!(line_breaks("សហរដ្ឋ\u{200b}អាមេរិក").contains(&"សហរដ្ឋ\u{200b}".len()));
}
