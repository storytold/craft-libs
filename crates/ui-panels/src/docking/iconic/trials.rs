//! Trials of `IconicColumn`: the letter of a panel without an icon, and the names a wide column shows.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use crate::contract::{Bind, ShowContainer as _};
use crate::docking::{GroupAt, IconicColumn, PanelSpec};
use crate::sections::Panel;
use crate::testing::memory::global;
use crate::testing::{Fixture, trial};
use craft_behavior::dock::{Column, Group};
use egui::Vec2;
use std::cell::Cell;
use std::rc::Rc;

fn spec(panel: &u8) -> PanelSpec {
    PanelSpec::new(if *panel == 0 { "Brushes" } else { "Swatches" })
}

/// A column `width` wide over two icon-less panels, and the panel whose flyout is open.
fn column(width: f32) -> (Fixture, Rc<Cell<Option<u8>>>) {
    let open = Rc::new(Cell::new(None));
    let seen = Rc::clone(&open);
    let mut fixture = Fixture::new(move |ui| {
        let column = Column::new(vec![Group::new(vec![0, 1], 0)]);
        let mut now = seen.get();
        IconicColumn::new("Panels", global("iconic column", "trial"), &column, Bind::bound(&mut now), &spec).show_flyouts(ui, |_, at, ui, _| {
            let GroupAt::Show(rect) = at else { return None };
            Panel::new("Flyout").show_at(ui, rect, |_, _| ());
            None
        });
        seen.set(now);
    });
    fixture.room(Vec2::new(width, 300.0)).settle();
    (fixture, open)
}

/// A panel without an icon shows the first letter of its name; a column wider than the label width shows the names
/// too, and a click on a name opens that panel's flyout as a click on its icon does.
#[trial]
fn an_icon_less_panel_shows_its_initial_and_a_wide_column_shows_names_that_open_flyouts() {
    let painted = |fixture: &Fixture, text: &str| fixture.texts().iter().any(|run| run.text == text);
    let (narrow, _) = column(40.0);
    assert!(painted(&narrow, "B") && painted(&narrow, "S"), "each icon-less panel shows its initial");
    assert!(!painted(&narrow, "Brushes"), "a narrow column shows no names");

    let (mut wide, open) = column(200.0);
    assert!(painted(&wide, "Brushes") && painted(&wide, "Swatches"), "a wide column shows the names");
    let icon = wide.by_label("Swatches").expect("the Swatches icon");
    assert!(icon.rect.width() < 200.0 / 2.0, "the icon keeps its square at the start: {:?}", icon.rect);
    // The name has no node of its own: the icon is its keyboard stop. Click on it, past the icon.
    wide.click_at(icon.rect.right_center() + Vec2::new(icon.rect.width(), 0.0)).idle(3);
    assert_eq!(open.get(), Some(1), "a click on the name opens its panel's flyout");
}
