//! Trailing items: required items stay and optional text drops first.

use craft_behavior::IconId;
use craft_ui::list::{ListRow, TrailingFace, TrailingPart};
use craft_ui::testing::internals::place_row;
use craft_ui::testing::trial;

const PART: TrailingPart = TrailingPart::new(7);

/// Required items stay in a narrow row while an optional text drops, and the label keeps its least width.
#[trial]
fn required_trailing_items_stay_and_optional_text_drops_first() {
    let row = || {
        ListRow::new("Layer")
            .trailing_icon(IconId::new("lock"), "Locked")
            .trailing_part(PART, TrailingFace::disclosure(true), "Effects")
            .optional("Multiply")
    };
    let wide = place_row(&row(), 400.0, false);
    assert!(wide.items.iter().all(Option::is_some), "everything fits in a wide row");
    let narrow = place_row(&row(), 130.0, false);
    assert!(narrow.items[0].is_some() && narrow.items[1].is_some(), "the icon and the part stay");
    assert!(narrow.items[2].is_none(), "the optional text dropped");
    assert!(narrow.label.width() >= narrow.min_label);
}
