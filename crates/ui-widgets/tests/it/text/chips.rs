//! The chip field's draft rules.

use craft_behavior::Codec as _;
use craft_ui::testing::trial;
use craft_ui::text::ChipCodec;

fn chips(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// A draft names one chip per piece between separators, trimmed; pieces already in the list and empty pieces add
/// nothing, and the new chips follow the old ones in the order typed.
#[trial]
fn a_draft_adds_its_pieces_after_the_existing_chips() {
    let codec = ChipCodec::default();
    let start = chips(&["kite"]);
    assert_eq!(codec.parse("red, beach ;kite,, \n sky", &start), Ok(chips(&["kite", "red", "beach", "sky"])));
    assert_eq!(codec.parse("  ,; ", &start), Ok(start.clone()), "a draft of separators adds nothing");
    assert_eq!(codec.parse("red, red", &Vec::new()), Ok(chips(&["red"])), "a piece the draft names twice is added once");
    assert_eq!(codec.parse("two words", &Vec::new()), Ok(chips(&["two words"])), "a space does not end a chip");
}
