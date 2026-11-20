//! The icon picker's filter row: a category shows only its group's tiles, All shows every group, the search narrows
//! inside the category, and the category outlives a pass.

use craft_behavior::IconId;
use craft_ui::contract::Bind;
use craft_ui::pickers::{IconPicker, IconPickerGroup, IconPickerGroups, IconPickerItem};
use craft_ui::primitives::IconSpec;
use craft_ui::testing::{Fixture, Room, trial};

fn groups() -> Vec<IconPickerGroup<u8>> {
    vec![
        IconPickerGroup::new("Selection", vec![IconPickerItem::new(1, "Move"), IconPickerItem::new(2, "Marquee")]),
        IconPickerGroup::new("Drawing", vec![IconPickerItem::new(3, "Brush"), IconPickerItem::new(4, "Eraser")]),
    ]
}

fn shows(fixture: &Fixture, name: &str) -> bool {
    fixture.by_label(name).is_some()
}

#[trial]
fn the_category_row_filters_the_grid_and_search_stays_inside_it() {
    let mut fixture = Fixture::new(|ui| {
        let groups = groups();
        let mut value = None;
        IconPicker::new("Tool", Bind::bound(&mut value), &groups).group_display(IconPickerGroups::Filter).empty("Nothing here").show(ui);
    });
    fixture.in_room(Room::Dialog).settle();
    assert!(shows(&fixture, "Move") && shows(&fixture, "Brush"), "All shows every group");

    let drawing = fixture.by_label("Drawing").expect("the category's tab");
    fixture.click(&drawing).settle();
    assert!(shows(&fixture, "Brush") && shows(&fixture, "Eraser"), "the chosen category's tiles show");
    assert!(!shows(&fixture, "Move"), "another category's tiles are hidden");
    fixture.idle(2);
    assert!(!shows(&fixture, "Move"), "the category survives a pass");

    let search = fixture.by_label("Search icons").expect("the search field");
    fixture.click(&search).type_text("era").settle();
    assert!(shows(&fixture, "Eraser") && !shows(&fixture, "Brush"), "the search narrows inside the category");

    fixture.type_text("zzz").settle();
    assert!(fixture.texts().iter().any(|painted| painted.text == "Nothing here"), "no match shows the empty text");

    let all = fixture.by_label("All").expect("the All tab");
    fixture.click(&all).settle();
    assert!(!shows(&fixture, "Move"), "the search still applies under All");
}

fn icon_groups() -> Vec<IconPickerGroup<u8>> {
    let item = |key: u8, name: &str| IconPickerItem::new(key, name).icon(IconSpec::svg(IconId::new("settings")));
    vec![
        IconPickerGroup::new("Selection", vec![item(1, "Move"), item(2, "Marquee")]),
        IconPickerGroup::new("Drawing", vec![item(3, "Brush"), item(4, "Eraser")]),
    ]
}

/// A picker in a room, with or without large previews.
fn picker_fixture(large: bool) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let groups = icon_groups();
        let mut value = None;
        let picker = IconPicker::new("Tool", Bind::bound(&mut value), &groups).group_display(IconPickerGroups::Filter).columns(2);
        if large { picker.large_previews() } else { picker }.show(ui);
    });
    fixture.in_room(Room::Dialog).settle();
    fixture
}

#[trial]
fn large_previews_make_the_cells_larger() {
    let side = |large: bool| {
        let fixture = picker_fixture(large);
        fixture.by_label("Move").expect("the first tile").rect.width()
    };
    assert!(side(true) > side(false), "a large preview fills a larger cell");
}
