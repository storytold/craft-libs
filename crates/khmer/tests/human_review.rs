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
    // Explicitly accepted rows, including the grouping which differs from ICU.
    for parts in [&["ក្នុង", "គោល", "ដៅ", "បង្កើន", "ការ", "យល់", "ដឹង"][..], &["ពណ៌", "សម្ដេច", "ថា", "ខ្មៅ"]]
    {
        let mut at = 0;
        let expected: Vec<_> = parts
            .iter()
            .take(parts.len() - 1)
            .map(|part| {
                at += part.len();
                at
            })
            .collect();
        assert_eq!(line_breaks(&parts.concat()), expected);
    }
}

#[test]
fn short_word_repair_preserves_known_words_and_correctable_misspellings() {
    let cases: &[(&str, &[&str])] = &[
        ("សណ្ដូកកបណ្ដែតខ្លួន", &["សណ្ដូក", "ក", "បណ្ដែត", "ខ្លួន"]),
        // A different suffix from the existing public synthetic corpus: the
        // rule is dictionary-driven, not an exception for the reviewed phrase.
        ("ជជែកលា", &["ជជែក", "លា"]),
        ("អភិវឌ្ឍន៍", &["អភិវឌ្ឍន៍"]),         // known whole word
        ("កសីករ", &["កសីករ"]),             // rare frequency entry, spelling correction remains possible
        ("សុន្ទរក", &["សុន្ទរក"]),             // competing one-edit spelling
        ("គហបតីករ័តន៍", &["គហបតីក", "រ័តន៍"]), // insertion into a longer dictionary word
    ];
    for &(text, expected) in cases {
        let actual: Vec<_> = craft_khmer::word_ranges(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(actual, expected, "{text}");
    }
    let text = "Latin 🦀 សណ្ដូកកបណ្ដែតខ្លួន!";
    assert!(line_breaks(text).contains(&"Latin 🦀 សណ្ដូក".len()));
    assert!(!line_breaks("សណ្ដូក\u{2060}កបណ្ដែតខ្លួន").contains(&"សណ្ដូក\u{2060}".len()));
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
