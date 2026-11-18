//! Page range text and From/To ranges.

use super::*;
use craft_ui::testing::trial;

#[trial]
fn range_text_parses_sorted_unique_and_bounded() {
    assert_eq!(parse_pages("5, 1-3; 2", 9), Ok(vec![0, 1, 2, 4]));
    assert_eq!(parse_pages("3-1", 9), Ok(vec![0, 1, 2]));
    assert_eq!(parse_pages("0", 9), Err(PageRangeError::OutOfRange { page: 0, count: 9 }));
    assert_eq!(parse_pages("12", 9), Err(PageRangeError::OutOfRange { page: 12, count: 9 }));
    assert_eq!(parse_pages("x", 9), Err(PageRangeError::NotAPage("x".into())));
    // A page number is ASCII digits only: no sign, no other script's digits.
    assert_eq!(parse_pages("+3", 9), Err(PageRangeError::NotAPage("+3".into())));
    assert_eq!(parse_pages("-3", 9), Err(PageRangeError::NotAPage("".into())));
    assert_eq!(parse_pages("1-+3", 9), Err(PageRangeError::NotAPage("+3".into())));
    assert_eq!(parse_pages("\u{0663}", 9), Err(PageRangeError::NotAPage("\u{0663}".into())));
    assert_eq!(parse_pages("  ", 9), Ok(vec![]));
    assert_eq!(parse_pages(&"1,".repeat(10_000), 9), Err(PageRangeError::TooLong));
    let overlaps = ["1-100000"; 11].join(",");
    assert_eq!(parse_pages(&overlaps, 100_000), Err(PageRangeError::TooLong));
    assert_eq!(PageRangeError::OutOfRange { page: 12, count: 9 }.to_string(), "page 12 is out of range (1\u{2013}9)");
}

/// The scope picks the pages and the subset filters them by page number: a slip sends a dialog's command to the
/// wrong pages without a word.
#[trial]
fn page_range_scopes_pick_pages_then_apply_the_subset() {
    let range = |scope, from, to, subset| PageRange { scope, from, to, subset };
    let all = PageRange::default();
    assert_eq!(all.pages(4, 2, &[]), vec![0, 1, 2, 3], "All is every page");
    assert_eq!(range(PageScope::All, 1, None, Subset::Even).pages(5, 0, &[]), vec![1, 3], "Even keeps pages 2 and 4");

    assert_eq!(range(PageScope::Current, 1, None, Subset::All).pages(9, 4, &[1, 2]), vec![4], "Current is the current page");

    let selection = range(PageScope::Selection, 1, None, Subset::All);
    assert_eq!(selection.pages(9, 0, &[5, 2, 5, 7, 2]), vec![5, 2, 7], "the selection keeps its order without repeats");
    assert_eq!(selection.pages(9, 3, &[]), vec![3], "an empty selection falls back to the current page");
    let odd_selection = range(PageScope::Selection, 1, None, Subset::Odd);
    assert_eq!(odd_selection.pages(9, 0, &[3, 0, 2, 1]), vec![0, 2], "the subset applies after the scope, by page number");

    assert_eq!(range(PageScope::Range, 3, None, Subset::All).pages(6, 0, &[]), vec![2, 3, 4, 5], "no `to` ends at the last page");
    assert_eq!(range(PageScope::Range, 2, Some(5), Subset::All).pages(9, 0, &[]), vec![1, 2, 3, 4], "`from` and `to` are inclusive");
    assert_eq!(range(PageScope::Range, 2, Some(50), Subset::All).pages(4, 0, &[]), vec![1, 2, 3], "`to` past the document is clamped");
    assert_eq!(range(PageScope::Range, 7, Some(9), Subset::All).pages(4, 0, &[]), Vec::<usize>::new(), "`from` past the document is no pages");
    assert_eq!(range(PageScope::Range, 1, Some(6), Subset::Odd).pages(9, 0, &[]), vec![0, 2, 4], "Odd keeps pages 1, 3 and 5 of the range");
}
