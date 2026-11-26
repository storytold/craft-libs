//! `FindMatches` as pages report their text: following the start page, and the user's choice ending it.

use std::ops::Range;

use craft_text::find::FindMatches;
use craft_ui::testing::trial;

/// Ranges from `(start, end)` pairs.
fn ranges(pairs: &[(usize, usize)]) -> Vec<Range<usize>> {
    pairs.iter().map(|&(start, end)| start..end).collect()
}

/// The current match's page and range.
fn current(found: &FindMatches) -> Option<(usize, Range<usize>)> {
    found.current_match().map(|found| (found.page, found.range.clone()))
}

#[trial]
fn following_takes_the_first_match_from_the_start_page_whatever_order_pages_arrive_in() {
    let mut found = FindMatches::new(1);
    assert!(found.set_page(2, ranges(&[(0, 6)])), "the only known match is shown while the start page is pending");
    assert_eq!(current(&found), Some((2, 0..6)));
    assert!(found.set_page(1, ranges(&[(4, 10)])), "the start page's match takes over once its text arrives");
    assert_eq!(current(&found), Some((1, 4..10)));
    assert!(!found.set_page(0, ranges(&[(2, 8)])), "a match before the start page does not move the view");
    assert_eq!(current(&found), Some((1, 4..10)));
    assert!(!found.set_page(3, Vec::new()), "a page whose text failed changes nothing");
    assert_eq!(current(&found), Some((1, 4..10)));
    assert_eq!((found.len(), found.from()), (3, Some(1)), "every page's matches are listed and following goes on");
}

#[trial]
fn a_step_or_pick_ends_following_and_later_pages_keep_the_users_match() {
    let mut stepped = FindMatches::new(0);
    stepped.set_page(1, ranges(&[(0, 3), (5, 8)]));
    assert_eq!(stepped.step(false).map(|found| (found.page, found.range.clone())), Some((1, 5..8)));
    assert_eq!(stepped.from(), None, "a step ends following");
    assert!(!stepped.set_page(0, ranges(&[(1, 4)])), "a later page does not move the view");
    assert_eq!(current(&stepped), Some((1, 5..8)), "the stepped-to match stays current");

    let mut picked = FindMatches::new(0);
    picked.set_page(2, ranges(&[(0, 3)]));
    picked.set_page(1, ranges(&[(0, 3)]));
    assert_eq!(current(&picked), Some((1, 0..3)));
    picked.pick(1);
    assert_eq!(picked.from(), None, "a pick ends following");
    assert!(!picked.set_page(0, ranges(&[(0, 3)])), "a later page does not move the view");
    assert_eq!(current(&picked), Some((2, 0..3)), "the picked match stays current");
}
