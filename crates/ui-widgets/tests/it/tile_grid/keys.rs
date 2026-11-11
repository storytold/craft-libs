//! The tile grid's own keys and gestures: Bridge's rating and zoom keys, and the slow click that renames.

use craft_behavior::{EditEvent, keyed};
use craft_ui::contract::{CollectionEvent, ShowCollection as _};
use craft_ui::testing::{Fixture, trial};
use craft_ui::tile_grid::{GridEvent, RatingKeys, Tile, TileGrid, ZoomStep};
use egui::{Key, Modifiers, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const NAMES: [&str; 4] = ["Tile 1", "Tile 2", "Tile 3", "Tile 4"];

/// What the grid reported, kept across frames.
#[derive(Default)]
struct Seen {
    grid: Vec<GridEvent<usize>>,
    collection: Vec<CollectionEvent<usize, craft_ui::tile_grid::TilePartEvent>>,
}

fn rig(make: fn() -> TileGrid<'static, usize>) -> (Fixture, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2, 3];
        let source = keyed(&keys, |key: &usize| *key);
        let out = make().show_items(ui, &source, &mut |key, _| Tile::new(NAMES[*key]));
        let mut seen = held.borrow_mut();
        seen.grid.extend(out.events);
        seen.collection.extend(out.response.events);
    });
    fixture.room(Vec2::new(600.0, 400.0));
    fixture.settle();
    (fixture, seen)
}

/// With Bridge's keys, Cmd/Ctrl with a digit rates the selected tile and with 0 clears it; Cmd/Ctrl with `=` and `-`
/// step the zoom. Without the rating keys, Cmd/Ctrl+0 resets the zoom.
#[trial]
fn command_digits_rate_and_command_equals_and_minus_zoom() {
    let (mut fixture, seen) = rig(|| TileGrid::new("Library").rating_keys(RatingKeys::CommandDigits).zooms());
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    fixture.key(Key::ArrowRight, Modifiers::NONE).idle(2);
    seen.borrow_mut().grid.clear();
    for key in [Key::Num3, Key::Num0, Key::Equals, Key::Minus] {
        fixture.key(key, Modifiers::COMMAND).idle(1);
    }
    let reported: Vec<String> = std::mem::take(&mut seen.borrow_mut().grid)
        .into_iter()
        .map(|event| match event {
            GridEvent::Rate { keys, rating, .. } => format!("rate {keys:?} {rating}"),
            GridEvent::Zoom(step) => format!("zoom {step:?}"),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(reported, ["rate [0] 3", "rate [0] 0", "zoom In", "zoom Out"], "the selected tile is rated, then cleared; the zoom steps in and out");

    let (mut fixture, seen) = rig(|| TileGrid::new("Library").zooms());
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    fixture.key(Key::Num0, Modifiers::COMMAND).idle(1);
    assert!(
        matches!(seen.borrow().grid.as_slice(), [GridEvent::Zoom(ZoomStep::Reset)]),
        "without rating keys Cmd/Ctrl+0 resets the zoom: {:?}",
        seen.borrow().grid
    );
}

/// A click on the caption of the only selected tile opens its rename once the double-click interval passes; a double
/// click there activates the tile and opens no rename.
#[trial]
fn a_slow_click_on_the_caption_renames_and_a_double_click_activates() {
    let (mut fixture, seen) = rig(|| TileGrid::new("Library").rename(true));
    let tile = fixture.by_label("Tile 2").expect("the tile has a node");
    // The caption band runs under the picture, inside the card's bottom inset.
    let caption = tile.rect.center_bottom() - Vec2::new(0.0, 10.0);
    let renames = |seen: &Rc<RefCell<Seen>>| {
        seen.borrow().collection.iter().filter(|event| matches!(event, CollectionEvent::Rename(EditEvent::Begin { .. }))).count()
    };
    fixture.click_at(caption);
    fixture.advance(1.0).idle(1);
    fixture.click_at(caption);
    assert_eq!(renames(&seen), 0, "the rename waits for the double-click interval");
    fixture.advance(1.0).idle(2);
    assert_eq!(renames(&seen), 1, "a slow second click on the caption renames the tile");

    let (mut fixture, seen) = rig(|| TileGrid::new("Library").rename(true));
    fixture.click_at(caption);
    fixture.advance(1.0).idle(1);
    fixture.click_at(caption).click_at(caption);
    fixture.advance(1.0).idle(2);
    let activated = seen.borrow().collection.iter().any(|event| matches!(event, CollectionEvent::Activate(1)));
    assert!(activated && renames(&seen) == 0, "a double click activates and opens no rename");
}
