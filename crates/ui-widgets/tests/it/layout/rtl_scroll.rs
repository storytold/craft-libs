//! A horizontally scrolling row under right-to-left: its content-sized children keep their own extents.

use craft_i18n::Direction;
use craft_ui::contract::{Scroll, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::Row;
use craft_ui::testing::{Fixture, trial};

/// Two content-sized rows in a horizontally scrolling row do not overlap under right-to-left and follow the
/// left-to-right order mirrored. Guards against the first row taking no width in the unbounded axis, which put the
/// second on top of it.
#[trial]
fn content_sized_rows_in_a_horizontal_scroll_do_not_overlap_in_rtl() {
    let mut fixture = Fixture::new(|ui| {
        Row::new().scroll(Scroll::horizontal()).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Text::new("Alpha x").show(ui);
            });
            Row::new().show(ui, |ui, _| {
                Text::new("Beta x").show(ui);
            });
        });
    });
    fixture.direction(Direction::Rtl).room(egui::vec2(400.0, 200.0)).settle();
    let rect = |text: &str| fixture.texts().iter().find(|painted| painted.text == text).unwrap_or_else(|| panic!("no {text} painted")).rect;
    let (alpha, beta) = (rect("Alpha x"), rect("Beta x"));
    assert!(beta.max.x <= alpha.min.x, "Beta ends before Alpha starts in RTL: alpha {alpha:?}, beta {beta:?}");
}
