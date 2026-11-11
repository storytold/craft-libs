//! `Menu` and `MenuItem`: the columns of an inline menu's rows.

mod bar;

mod native;

mod openers;

mod pick_focus;

use craft_behavior::IconId;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::IconSpec;
use craft_ui::testing::{Fixture, trial};

/// Regression: an inline menu inside a content-sized container shows by the record its container's measuring run
/// left, so its own body runs only for real; the rows before the first iconed or checked row still take the columns,
/// in every frame (a tab strip's overflow menu checks only the shown tab).
#[trial]
fn rows_of_an_inline_menu_shown_from_its_container_record_take_the_columns_of_later_rows() {
    let mut fixture = Fixture::new(|ui| {
        Stack::new().show(ui, |ui, _| {
            Menu::inline("Tabs").show(ui, |ui, _| {
                MenuItem::new("Layers").show(ui);
                MenuItem::new("History").icon(IconSpec::svg(IconId::new("history"))).show(ui);
                MenuItem::new("Swatches").check(true).show(ui);
            });
        });
    });
    let at = |fixture: &Fixture, label: &str| fixture.texts().iter().find(|text| text.text.replace('&', "") == label).map(|text| text.rect.left());
    let mut frames = 0;
    while at(&fixture, "Swatches").is_none() && frames < 8 {
        fixture.frame(vec![]);
        frames += 1;
    }
    let x = |fixture: &Fixture, label: &str| at(fixture, label).expect("the row is shown");
    for frame in 0..3 {
        assert_eq!(x(&fixture, "Layers"), x(&fixture, "Swatches"), "frame {frame}: a plain row takes the mark and icon columns");
        assert_eq!(x(&fixture, "History"), x(&fixture, "Swatches"), "frame {frame}: an iconed row takes the mark column");
        fixture.frame(vec![]);
    }
}
