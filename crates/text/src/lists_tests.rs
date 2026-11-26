//! List numbering and number styles.

use super::lists::{ListMarker, ListParagraph, NumberPunctuation, NumberStyle, markers};
use craft_ui::testing::trial;

// 1. 2. then a bullet at level 0 restarts; a level-1 run between level-0 numbers keeps them counting; empty paragraphs neither show nor count.
#[trial]
fn numbers_count_per_level_and_restart_after_an_outer_unnumbered_paragraph() {
    let number = |start| ListMarker::Number { style: NumberStyle::Arabic, start, punctuation: NumberPunctuation::Period };
    let mut empty = ListParagraph::new(0, Some(number(1)));
    empty.empty = true;
    let paragraphs = [
        ListParagraph::new(0, Some(number(1))),
        ListParagraph::new(1, Some(number(1))),
        ListParagraph::new(1, Some(number(1))),
        empty,
        ListParagraph::new(0, Some(number(1))),
        ListParagraph::new(0, Some(ListMarker::Bullet("•".into()))),
        ListParagraph::new(0, Some(number(5))),
        ListParagraph::new(0, None),
        ListParagraph::new(0, Some(number(1))),
    ];
    let shown = [Some("1."), Some("1."), Some("2."), None, Some("2."), Some("•"), Some("5."), None, Some("1.")];
    assert_eq!(markers(&paragraphs).iter().map(Option::as_deref).collect::<Vec<_>>(), shown);
}

// A level-2 paragraph without a margin starts at 72 pt; a hanging indent of -18 puts the first line 18 pt left of the rest.
