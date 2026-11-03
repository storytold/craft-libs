#![cfg(craft_khmer_data)]
//! Reused public Aksara fixtures, pinned at
//! 31740e9d51d38171ae6119fc2d18a4a18fbfb2f1 (MIT; LICENSE-AKSARA).
//! Sources: lib/khmer-breaker.test.ts and new-khmer-linebreaking-update.md §6.
//! This tests the native range API; it does not pretend that it returns Latin
//! tokens or offers the web app's mutable user-dictionary/normalizer APIs.
use craft_khmer::{line_breaks, word_ranges};

#[test]
fn aksara_explicit_user_split_survives() {
    let text = "ព្រះ\u{200b}គម្ពីរ";
    let words: Vec<_> = word_ranges(text).iter().map(|r| text.get(r.clone()).unwrap()).collect();
    assert!(words.contains(&"ព្រះ"));
    assert!(words.contains(&"គម្ពីរ"));
    assert!(!words.contains(&"ព្រះគម្ពីរ"));
    assert!(line_breaks(text).contains(&"ព្រះ\u{200b}".len()));
}

#[test]
fn aksara_degenerate_inputs_keep_valid_nonoverlapping_ranges() {
    for text in ["", " ", "។", "\u{200b}", "្", "abc", "១២៣", "ក", "\n\t"] {
        let mut previous = 0;
        for range in word_ranges(text) {
            assert!(range.start >= previous && range.start < range.end);
            assert!(text.get(range.clone()).is_some());
            previous = range.end;
        }
        for at in line_breaks(text) {
            assert!(at > 0 && at < text.len() && text.is_char_boundary(at));
        }
    }
    assert!(word_ranges("Hello world").is_empty(), "native API returns Khmer spans only");
}

#[test]
fn aksara_documented_dictionary_and_tagger_examples() {
    let examples: &[(&str, &[&str])] = &[
        ("ការសិក្សា", &["ការ", "សិក្សា"]),
        ("ចំណេះដឹង", &["ចំណេះ", "ដឹង"]),
        ("រូបភាព", &["រូប", "ភាព"]),
        ("ចូលចិត្ត", &["ចូល", "ចិត្ត"]),
        ("មូលដ្ឋាន", &["មូលដ្ឋាន"]),
        ("ទីក្រុង", &["ទីក្រុង"]),
        ("រង់ចាំ", &["រង់ចាំ"]),
        ("អ្វីៗ", &["អ្វីៗ"]),
    ];
    for (text, expected) in examples {
        let words: Vec<_> = word_ranges(text).iter().map(|r| text.get(r.clone()).unwrap()).collect();
        assert_eq!(&words, expected, "{text}");
    }
}

#[test]
fn live_word_joiner_is_not_a_gold_corpus_annotation() {
    // Aksara's historical evaluation strips WJ as an annotation. Actual document
    // text must instead retain it and forbid adjacent line opportunities.
    let text = "ខ្មែរ\u{2060}ភាសា";
    assert!(line_breaks(text).is_empty());
    let at = text.find('\u{2060}').unwrap();
    assert_eq!(text.get(at..at + '\u{2060}'.len_utf8()), Some("\u{2060}"));
}
