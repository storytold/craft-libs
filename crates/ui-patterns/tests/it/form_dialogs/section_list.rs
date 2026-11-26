//! `SectionList`: a pick is reported once, and the list stays where it is while the body scrolls.

use craft_behavior::{DisplayOrder, ItemSource};
use craft_ui::contract::Show as _;
use craft_ui::display::Text;
use craft_ui::list::ListRow;
use craft_ui::patterns::form_dialogs::SectionList;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Pos2, Rect, Vec2};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const NAMES: [&str; 4] = ["General", "Compression", "Marks", "Output"];

/// The lines of a body: more than the room shows.
const LINES: usize = 60;

/// The list's rows, keyed by position.
struct Names;

impl DisplayOrder<usize> for Names {
    fn len(&self) -> usize {
        NAMES.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        NAMES.get(at).map(|name| Cow::Borrowed(*name))
    }
}

impl ItemSource<usize> for Names {}

/// The rect of the node named `label`.
fn rect_of(fixture: &Fixture, label: &str) -> Option<Rect> {
    fixture.by_label(label).map(|target| target.rect)
}

#[trial]
fn a_pick_returns_its_key_once_and_the_list_stays_put_while_the_body_scrolls() {
    let (shown, picks) = (Rc::new(Cell::new(0_usize)), Rc::new(RefCell::new(Vec::new())));
    let (at, picked) = (Rc::clone(&shown), Rc::clone(&picks));
    let mut fixture = Fixture::new(move |ui| {
        let section = at.get();
        let key = SectionList::new("Sections", &Names, section).show(
            ui,
            |key| ListRow::new(Label::from(NAMES.get(*key).copied().unwrap_or_default())),
            |ui| {
                for line in 1..=LINES {
                    Text::new(format!("{} line {line}", NAMES.get(section).copied().unwrap_or_default())).show(ui);
                }
            },
        );
        if let Some(key) = key {
            at.set(key);
            picked.borrow_mut().push(key);
        }
    });
    let room = Vec2::new(600.0, 300.0);
    fixture.room(room);
    fixture.settle();

    let list_before = rect_of(&fixture, "Compression").expect("the list shows its rows");
    let body_before = rect_of(&fixture, "General line 1");
    let over_body = Pos2::new(room.x - 60.0, room.y / 2.0);
    for _ in 0..6 {
        fixture.frame(vec![pointer::moved(over_body), pointer::wheel(-120.0)]);
    }
    fixture.settle();
    assert_ne!(rect_of(&fixture, "General line 1"), body_before, "the wheel scrolls the body");
    assert_eq!(rect_of(&fixture, "Compression"), Some(list_before), "the list stays put while the body scrolls");
    assert!(picks.borrow().is_empty(), "scrolling picks nothing");

    let row = fixture.by_label("Compression").expect("the Compression row");
    fixture.click(&row).settle();
    assert_eq!(*picks.borrow(), vec![1], "the pick is reported once, in the pass of the pick");
    assert_eq!(shown.get(), 1, "the caller keeps the picked section");
}
