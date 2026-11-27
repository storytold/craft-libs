//! A Select steps over its stepped entries only. A value on an entry that is not stepped has no place among them: it
//! offers both steps, and a step forward lands on the first stop and a step back on the last.

use craft_ui::contract::{Bind, Entry, Role, Show as _};
use craft_ui::list::ThumbAspect;
use craft_ui::pickers::Select;
use craft_ui::testing::access::Declared;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Key, Modifiers, Rect, Ui};
use std::cell::RefCell;
use std::rc::Rc;

/// Entries 1 to 4; 2 and 4 are reached from the open list only.
fn select(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = [Entry::new(1u8, "One"), Entry::new(2u8, "Two").stepped(false), Entry::new(3u8, "Three"), Entry::new(4u8, "Four").stepped(false)];
    Select::new("Pick", data.bind("value", 4u8), &entries).show(ui).into()
}

/// The value after `key` on the focused, closed field holding `held`, and whether it offered Increment and Decrement.
fn step(held: u8, key: Key) -> (u8, (bool, bool)) {
    let mut fixture = Fixture::story(&Story::new("select_steps", select));
    *fixture.data().value("value", 4u8) = held;
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    let node = fixture.by_role(Role::COMBO_BOX).first().expect("the field").node;
    let tree = fixture.tree();
    let offered = (tree.declares_key(node, Declared::Increment), tree.declares_key(node, Declared::Decrement));
    fixture.key(key, Modifiers::NONE);
    fixture.settle();
    let value = *fixture.data().value("value", 4u8);
    (value, offered)
}

#[trial]
fn a_select_steps_over_its_stepped_entries_and_from_an_unstepped_one_to_an_end() {
    assert_eq!(step(1, Key::ArrowDown), (3, (true, false)), "the first stop skips the unstepped Two and offers no step back");
    assert_eq!(step(4, Key::ArrowDown), (1, (true, true)), "from an unstepped entry, forward lands on the first stop");
    assert_eq!(step(4, Key::ArrowUp), (3, (true, true)), "from an unstepped entry, back lands on the last stop");
}

#[trial]
fn the_closed_face_shows_an_entrys_closed_label_and_the_accessible_value_keeps_its_label() {
    let mut fixture = Fixture::new(|ui| {
        let entries = [Entry::new(1u8, "1 Dots").closed_label("Dots"), Entry::new(2u8, "2 Lines").closed_label("Lines")];
        Select::new("Leader", Bind::given(1u8), &entries).show(ui);
    });
    fixture.settle();
    let texts: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).collect();
    assert!(texts.iter().any(|text| text == "Dots"), "the face shows the closed label: {texts:?}");
    assert!(!texts.iter().any(|text| text == "1 Dots"), "and not the list text");
    let node = fixture.by_role(Role::COMBO_BOX).first().expect("the field").node;
    assert_eq!(fixture.tree().value_of(node), Some("1 Dots"), "the accessible value is the full label");
}

/// The rects a select's picture is painted into: on the closed face, then while the list is open (its rows and the
/// face), with the picture `aspect` times as wide as high, or with no aspect set.
fn picture_rects(aspect: Option<ThumbAspect>) -> (Vec<Rect>, Vec<Rect>) {
    let painted = Rc::new(RefCell::new(Vec::new()));
    let into = Rc::clone(&painted);
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(1u8, "Uniform"), Entry::new(2u8, "Taper")];
        let into = Rc::clone(&into);
        let select = Select::new("Profile", Bind::given(1u8), &entries).picture(move |_, _, rect| into.borrow_mut().push(rect)).picture_only();
        let select = match aspect {
            Some(aspect) => select.picture_aspect(aspect),
            None => select,
        };
        select.show(ui);
    });
    fixture.settle();
    let face = painted.take();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    assert!(!fixture.by_role(Role::LIST_BOX).is_empty(), "Alt+Down opens the list");
    (face, painted.take())
}

/// A picture aspect widens the picture on the face and in every row of the list; without one the picture is square.
#[trial]
fn a_picture_aspect_sets_the_pictures_width_on_the_face_and_in_the_rows() {
    let wide = |rect: &Rect| rect.height() > 0.0 && rect.width() > 3.5 * rect.height();
    let square = |rect: &Rect| rect.height() > 0.0 && (rect.width() - rect.height()).abs() < 0.5;
    let (face, rows) = picture_rects(Some(ThumbAspect::new(4, 1)));
    assert!(!face.is_empty() && face.iter().all(wide), "the face's picture is four times as wide as high: {face:?}");
    assert!(rows.len() >= 3 && rows.iter().all(wide), "each row's picture is four times as wide as high: {rows:?}");
    let (face, rows) = picture_rects(None);
    assert!(!face.is_empty() && face.iter().all(square), "without an aspect the face's picture is square: {face:?}");
    assert!(rows.len() >= 3 && rows.iter().all(square), "without an aspect each row's picture is square: {rows:?}");
}

/// A value that names no entry shows the placeholder on the face, and the field reads it as its value, as the face
/// shows it; a value that names an entry reads that entry. Guards against the shown placeholder leaving the field
/// with no value to read.
#[trial]
fn a_value_on_no_entry_reads_as_the_placeholder_the_face_shows() {
    for (held, read) in [(9u8, "Custom"), (1u8, "One")] {
        let mut fixture = Fixture::new(move |ui| {
            let entries = [Entry::new(1u8, "One"), Entry::new(2u8, "Two")];
            Select::new("Preset", Bind::given(held), &entries).placeholder("Custom").show(ui);
        });
        fixture.settle();
        let texts: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).collect();
        assert!(texts.iter().any(|text| text == read), "the face shows {read}: {texts:?}");
        let node = fixture.by_role(Role::COMBO_BOX).first().expect("the field").node;
        assert_eq!(fixture.tree().value_of(node), Some(read), "the field reads what its face shows");
    }
}
