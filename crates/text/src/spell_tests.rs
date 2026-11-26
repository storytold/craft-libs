//! The spelling engine on small word lists and the bundled dictionary.

use crate::spell::Dictionary;
use craft_ui::testing::trial;

// "teh" suggests "the" first (a transposition), and nearer words come before more common ones.
#[trial]
fn suggestions_rank_by_distance_then_frequency() {
    let d = Dictionary::english();
    assert_eq!(d.suggest("teh", 3)[0], "the");
    assert!(d.suggest("recieve", 5).contains(&"receive".to_string()));
    assert_eq!(d.suggest("Wrold", 1), vec!["World".to_string()]);
    assert_eq!(d.suggest("brwon", 1), vec!["brown".to_string()]);
    assert!(d.suggest("HELO", 5)[0].chars().all(char::is_uppercase));
    // Among equal distances the lowest SCOWL level wins; a nearer word beats a more common one.
    let d = Dictionary::from_list("#10\ncart\n#50\ncare\ncarp\n");
    assert_eq!(d.suggest("carx", 3), vec!["cart", "care", "carp"]);
    let d = Dictionary::from_list("#10\nabcdef\n#50\nabcxyz\n");
    assert_eq!(d.suggest("abcxyf", 2), vec!["abcxyz", "abcdef"]);
}

// A word added with a curly apostrophe is accepted when the text uses either apostrophe, is stored
// straight, and is found again by the curly spelling for removal.
