//! `BarItems`: a strip too short for its items moves whole groups into "More"; the every-item menu lists them all.

use craft_ui::bars::{BarItem, BarItems, MoreMenu};
use craft_ui::contract::{Role, Show as _};
use craft_ui::runtime::commands::EditCommands;
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;

fn strip(width: f32, more: MoreMenu) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let items = vec![
            BarItem::new(EditCommands::Undo),
            BarItem::new(EditCommands::Redo),
            BarItem::new(EditCommands::Cut).section(1),
            BarItem::new(EditCommands::Copy).section(1),
            BarItem::new(EditCommands::Paste).section(1),
        ];
        BarItems::new(items).more(more).show(ui);
    });
    fixture.room(Vec2::new(width, 80.0)).settle();
    fixture
}

/// How many of `labels` the strip shows.
fn shown(fixture: &Fixture, labels: &[&str]) -> usize {
    labels.iter().filter(|label| fixture.by_label(label).is_some()).count()
}

#[trial]
fn a_narrowing_strip_moves_whole_groups_into_more() {
    let (history, clipboard) = (["Undo", "Redo"], ["Cut", "Copy", "Paste"]);
    let wide = strip(600.0, MoreMenu::Overflow);
    assert_eq!(shown(&wide, &clipboard) + shown(&wide, &history), 5, "a wide strip shows every item");
    assert!(wide.by_label("More").is_none(), "a strip with room has no More");
    let mut split = false;
    for width in (40..=240).rev().step_by(4) {
        let fixture = strip(width as f32, MoreMenu::Overflow);
        let (h, c) = (shown(&fixture, &history), shown(&fixture, &clipboard));
        assert!(c == 0 || c == clipboard.len(), "{c} of the clipboard group show at {width}");
        assert!(h == 0 || h == history.len(), "{h} of the history group show at {width}");
        split |= h == history.len() && c == 0;
    }
    assert!(split, "no width shows the first group without the second");
}

#[trial]
fn the_every_item_menu_lists_the_items_on_the_strip_too() {
    let mut fixture = strip(600.0, MoreMenu::Every);
    assert!(fixture.by_label("Undo").is_some(), "a wide strip shows its items");
    let more = fixture.by_label("More").expect("More shows with the every-item menu");
    fixture.click(&more);
    fixture.settle();
    assert_eq!(fixture.by_role(Role::MENU_ITEM).len(), 5, "the menu lists every item");
}
