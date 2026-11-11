//! Find patterns, replacements and the matches across pages.

use crate::find::{FindError, FindOptions, FindPattern, PARAGRAPH_MARK};
use craft_ui::testing::trial;

fn span(start: usize, end: usize) -> std::ops::Range<usize> {
    start..end
}

#[expect(clippy::unwrap_used, reason = "a test pattern that does not compile fails the test")]
fn compile(query: &str, options: FindOptions) -> FindPattern {
    let compiled = FindPattern::compile(query, &options);
    assert!(compiled.is_ok(), "{query:?} does not compile: {compiled:?}");
    compiled.ok().unwrap()
}

// ^p matches a paragraph mark and makes the pattern span paragraphs; <(c?t)> with wildcards finds "cat" and "cut" as whole words; an unclosed [ is a Wildcards error.
#[trial]
fn word_codes_and_wildcards_compile() {
    let paragraph = compile("a^pb", FindOptions::default());
    assert!(paragraph.spans_paragraphs());
    assert_eq!(paragraph.find_all(&format!("xa{PARAGRAPH_MARK}b"), 10), [span(1, 4)]);

    let wild = FindOptions { wildcards: true, ..Default::default() };
    let pattern = compile("<(c?t)>", wild);
    assert_eq!(pattern.find_all("cat cut cart concat", 10), [0..3, 4..7]);

    assert!(matches!(FindPattern::compile("a[b", &wild), Err(FindError::Wildcards(_))));
    assert!(matches!(FindPattern::compile("(", &FindOptions { regex: true, ..Default::default() }), Err(FindError::Regex(_))));
    assert_eq!(FindPattern::compile(&"a".repeat(2000), &FindOptions::default()).unwrap_err(), FindError::TooLong);
}

// A page with more matches than the limit keeps `limit` of them and says so; setting the page again
// without the signal clears it.

// A pattern lists its named groups with their numbers; the lookup resolves `${name}` in a replacement.
