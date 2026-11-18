//! The in-ribbon gallery: its tile count follows the group's level, and its stepping stays in range.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_menu::{Gallery, GalleryFace, GalleryItem, GroupLevel};
use craft_ui::contract::Show as _;
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::env::{Label, with_env};
use craft_ui::testing::{Fixture, trial};
use egui::Rect;

/// The tiles of a gallery over `items`, six at the full level and three at the compact one: which of the first
/// `of` tiles show, by their labels.
fn shown(fixture: &Fixture, of: usize) -> Vec<usize> {
    (0..of).filter(|at| fixture.by_label(&format!("Tile {at}")).is_some()).collect()
}

#[trial]
fn the_tile_count_follows_the_group_level_and_stepping_stays_in_range_when_the_items_change() {
    let level = Rc::new(Cell::new(GroupLevel::Full));
    let count = Rc::new(Cell::new(10_usize));
    let picks = Rc::new(RefCell::new(Vec::<usize>::new()));
    let mut fixture = {
        let (level, count, picks) = (Rc::clone(&level), Rc::clone(&count), Rc::clone(&picks));
        Fixture::new(move |ui| {
            let paint = |_: &ContentPainterCx, _: Rect| {};
            let items: Vec<GalleryItem<'_, usize>> =
                (0..count.get()).map(|at| GalleryItem { key: at, label: Label::from(format!("Tile {at}")), face: GalleryFace::Paint(&paint) }).collect();
            let at_level = level.get();
            let out = with_env(ui, |set| {
                    set.values.set(at_level);
                },
                |ui| Gallery::new("Styles", &items, None).tiles(6, 3).show(ui));
            if let Some(key) = out.picked {
                picks.borrow_mut().push(key);
            }
        })
    };
    fixture.settle();
    assert_eq!(shown(&fixture, 10), vec![0, 1, 2, 3, 4, 5], "six tiles at the full level");

    let down = fixture.by_label("Next row").unwrap_or_else(|| panic!("the down button is named"));
    fixture.click(&down);
    fixture.settle();
    assert_eq!(shown(&fixture, 10), vec![4, 5, 6, 7, 8, 9], "a step down moves by the shown count, to the last full run of ten");
    let up = fixture.by_label("Previous row").unwrap_or_else(|| panic!("the up button is named"));
    fixture.click(&up);
    fixture.settle();
    assert_eq!(shown(&fixture, 10), vec![0, 1, 2, 3, 4, 5], "a step up goes back");

    level.set(GroupLevel::Compact);
    fixture.settle();
    assert_eq!(shown(&fixture, 10), vec![0, 1, 2], "three tiles at the compact level");

    level.set(GroupLevel::Full);
    fixture.settle();
    let down = fixture.by_label("Next row").unwrap_or_else(|| panic!("the down button is named"));
    fixture.click(&down);
    fixture.settle();
    assert_eq!(shown(&fixture, 10), vec![4, 5, 6, 7, 8, 9], "stepped to the end");
    count.set(3);
    fixture.settle();
    assert_eq!(shown(&fixture, 3), vec![0, 1, 2], "three items under a step past their end still show, none of the strip empty");

    let tile = fixture.by_label("Tile 1").unwrap_or_else(|| panic!("the second tile shows"));
    fixture.click(&tile);
    fixture.settle();
    assert_eq!(*picks.borrow(), vec![1], "a click on a tile picks its key");
}
