//! One-line cuts: each keeps the part of the text its place names and is no wider than its room.

use craft_i18n::Direction;
use craft_ui::contract::Show as _;
use craft_ui::display::{Elide, Text};
use craft_ui::testing::{Fixture, trial};
use egui::Shape;

/// The text `elide` shows of `text` in a room `width` wide in `direction`, and the width it takes.
fn shown(text: &str, elide: Elide, width: f32, direction: Direction) -> (String, f32) {
    let owned = text.to_owned();
    let mut fixture = Fixture::new(move |ui| {
        Text::new(owned.clone()).elide(elide).show(ui);
    });
    fixture.direction(direction).room(egui::vec2(width, 40.0)).settle();
    let galley = fixture
        .frame_shapes()
        .iter()
        .find_map(|shape| match shape {
            Shape::Text(text) => Some(text.galley.clone()),
            _ => None,
        })
        .expect("the text paints");
    (galley.job.text.clone(), galley.size().x)
}

#[trial]
fn each_cut_keeps_the_part_its_place_names_within_the_room() {
    let name = "IMG_2024_summer_holiday_final_edit_version_two.psd";
    let width = 120.0;
    let (start, at_start) = shown(name, Elide::Start, width, Direction::Ltr);
    assert!(start.starts_with('\u{2026}') && start.ends_with("two.psd"), "a start cut keeps the end: {start}");
    let (middle, at_middle) = shown(name, Elide::Middle, width, Direction::Ltr);
    let (head, tail) = middle.split_once('\u{2026}').expect("a middle cut shows the ellipsis");
    assert!(name.starts_with(head) && name.ends_with(tail) && !tail.is_empty(), "a middle cut keeps both ends: {middle}");
    assert!(head.chars().count() > tail.chars().count(), "a middle cut keeps more of the start than the end: {middle}");
    let path = "/home/ada/Pictures/2026/Summer/holiday.psd";
    let (folders, at_folders) = shown(path, Elide::Path, width, Direction::Ltr);
    assert!(
        folders.starts_with('\u{2026}') && folders.ends_with("/holiday.psd") && path.ends_with(&folders[3..]),
        "a path cut drops leading folders: {folders}"
    );
    for at in [at_start, at_middle, at_folders] {
        assert!(at <= width, "a cut fits its room");
    }
    let (whole, _) = shown("holiday.psd", Elide::Path, width, Direction::Ltr);
    assert_eq!(whole, "holiday.psd", "a text that fits is not cut");
}

#[trial]
fn an_end_cut_of_right_to_left_text_keeps_its_logical_start() {
    let name = "שם קובץ ארוך מאוד שלא נכנס בשורה אחת של הפאנל.psd";
    for direction in [Direction::Rtl, Direction::Ltr] {
        let (cut, _) = shown(name, Elide::End, 120.0, direction);
        let head = cut.trim_end_matches('\u{200f}').strip_suffix('\u{2026}').expect("the ellipsis ends the logical text");
        assert!(name.starts_with(head.trim_end()) && !head.is_empty(), "the cut keeps the start ({direction:?}): {cut}");
    }
}
