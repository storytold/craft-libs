//! The suite's text-size case lets a scroll region's view cut its rows, and no other part.

use craft_ui::contract::{Scroll, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::Stack;
use craft_ui::testing::story::{Story, StoryData, StoryOutput};
use craft_ui::testing::suite::text_size_case;
use craft_ui::testing::trial;
use egui::Ui;

/// More rows than the fixture's room holds, so one of them crosses the room's bottom edge.
const ROWS: usize = 40;

/// A stack that fills the room and shows `ROWS` rows, scrolling when `scroll` is set.
fn rows(ui: &mut Ui, scroll: bool) -> StoryOutput {
    let mut stack = Stack::new().fill();
    if scroll {
        stack = stack.scroll(Scroll::vertical());
    }
    stack.show(ui, |ui, _| {
        for row in 1..=ROWS {
            Text::new(format!("Row {row}")).show(ui);
        }
    });
    StoryOutput::default()
}

fn fixed_rows(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    rows(ui, false)
}

/// Rows that grow past a part that does not scroll leave it under the larger text size.
#[trial]
fn the_text_size_case_fails_text_that_grows_past_a_fixed_part() {
    let failed = text_size_case(&Story::new("fixed-rows", fixed_rows)).expect_err("the rows grow past the room");
    assert!(failed.contains("under text size"), "{failed}");
}
