//! The tile grid's own keys and gestures: Bridge's rating and zoom keys, and the slow click that renames.

use craft_behavior::{EditEvent, Selection, keyed};
use craft_ui::contract::{Bind, CollectionEvent, ShowCollection as _};
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::tile_grid::{GridEvent, GridZoom, RatingKeys, Tile, TileGrid, TileGroup, TileZoom};
use egui::{Key, Modifiers, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const NAMES: [&str; 4] = ["Tile 1", "Tile 2", "Tile 3", "Tile 4"];

#[trial]
fn keyboard_selects_a_selectable_group_header() {
    let selected = Rc::new(RefCell::new(Selection::single(1_usize)));
    let held = selected.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [1_usize, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let groups = [TileGroup::new(0, 0, 2, "Folder").selectable()];
        let mut selection = held.borrow_mut();
        let _ =
            TileGrid::new("Library")
                .groups(&groups)
                .collapsible()
                .selection(Bind::bound(&mut *selection))
                .show_items(ui, &source, &mut |_, _| Tile::new("Tile"));
    });
    fixture.room(Vec2::new(600.0, 400.0)).settle();
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    fixture.key(Key::Home, Modifiers::NONE).idle(2);
    fixture.key(Key::ArrowUp, Modifiers::NONE).idle(2);
    assert_eq!(selected.borrow().items, [0], "the header is selected when the lead reaches it");
    fixture.key(Key::ArrowDown, Modifiers::SHIFT).idle(2);
    assert_eq!(selected.borrow().items, [0, 1], "a range includes the selectable header");
}

#[trial]
fn keyboard_lead_on_a_group_header_is_reported() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [1_usize, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let groups = [TileGroup::new(0, 0, 2, "Folder")];
        let out = TileGrid::new("Library").groups(&groups).collapsible().show_items(ui, &source, &mut |_, _| Tile::new("Tile"));
        held.borrow_mut().extend(out.response.events);
    });
    fixture.room(Vec2::new(600.0, 400.0)).settle();
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    fixture.key(Key::Home, Modifiers::NONE).idle(2);
    fixture.key(Key::ArrowUp, Modifiers::NONE).idle(2);
    assert!(seen.borrow().iter().any(|event| matches!(event, CollectionEvent::LeadMoved { key: 0, .. })));
}

/// What the grid reported, kept across frames, and the zoom value a bound grid writes.
#[derive(Default)]
struct Seen {
    grid: Vec<GridEvent<usize>>,
    collection: Vec<CollectionEvent<usize, craft_ui::tile_grid::TilePartEvent>>,
    zoom: f64,
}

/// A four-tile grid `make` builds over the zoom value the rig keeps, which starts at the ladder's default.
fn rig(make: for<'z> fn(&'z mut f64) -> TileGrid<'z, usize>) -> (Fixture, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen { zoom: TileZoom::Ladder.default_value(), ..Seen::default() }));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2, 3];
        let source = keyed(&keys, |key: &usize| *key);
        let mut zoom = held.borrow().zoom;
        let out = make(&mut zoom).show_items(ui, &source, &mut |key, _| Tile::new(NAMES[*key]));
        let mut seen = held.borrow_mut();
        seen.zoom = zoom;
        seen.grid.extend(out.events);
        seen.collection.extend(out.response.events);
    });
    fixture.room(Vec2::new(600.0, 400.0));
    fixture.settle();
    (fixture, seen)
}

/// The ladder zoom bound to `zoom`.
fn bound(zoom: &mut f64) -> GridZoom<'_> {
    GridZoom::Bound(Bind::bound(zoom), TileZoom::Ladder)
}

/// With Bridge's keys, Cmd/Ctrl with a digit rates the selected tile and with 0 clears it, leaving the zoom alone;
/// Cmd/Ctrl with `=` raises the bound zoom and with `-` lowers it. Without the rating keys, Cmd/Ctrl+0 returns the
/// zoom to its default.
#[trial]
fn command_digits_rate_and_command_equals_and_minus_zoom() {
    let (mut fixture, seen) = rig(|zoom| TileGrid::new("Library").rating_keys(RatingKeys::CommandDigits).zoom(bound(zoom)));
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    fixture.key(Key::ArrowRight, Modifiers::NONE).idle(2);
    seen.borrow_mut().grid.clear();
    let start = seen.borrow().zoom;
    for key in [Key::Num3, Key::Num0] {
        fixture.key(key, Modifiers::COMMAND).idle(1);
    }
    let reported: Vec<String> = std::mem::take(&mut seen.borrow_mut().grid)
        .into_iter()
        .map(|event| match event {
            GridEvent::Rate { keys, rating, .. } => format!("rate {keys:?} {rating}"),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(reported, ["rate [0] 3", "rate [0] 0"], "the selected tile is rated, then cleared");
    assert_eq!(seen.borrow().zoom, start, "Cmd/Ctrl+0 is a rating key here and leaves the zoom");
    fixture.key(Key::Equals, Modifiers::COMMAND).idle(1);
    let raised = seen.borrow().zoom;
    assert!(raised > start, "Cmd/Ctrl+= raises the zoom: {start} to {raised}");
    fixture.key(Key::Minus, Modifiers::COMMAND).idle(1);
    let lowered = seen.borrow().zoom;
    assert!(lowered < raised, "Cmd/Ctrl+- lowers it: {raised} to {lowered}");
    assert!(seen.borrow().grid.is_empty(), "a zoom step is no grid event: {:?}", seen.borrow().grid);

    let (mut fixture, seen) = rig(|zoom| TileGrid::new("Library").zoom(bound(zoom)));
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    fixture.key(Key::Equals, Modifiers::COMMAND).idle(1);
    fixture.key(Key::Equals, Modifiers::COMMAND).idle(1);
    assert!(seen.borrow().zoom > TileZoom::Ladder.default_value(), "two steps in leave the default");
    fixture.key(Key::Num0, Modifiers::COMMAND).idle(1);
    assert_eq!(seen.borrow().zoom, TileZoom::Ladder.default_value(), "without rating keys Cmd/Ctrl+0 returns the zoom to its default");
    assert!(seen.borrow().grid.is_empty(), "and rates nothing: {:?}", seen.borrow().grid);
}

/// A click on the caption of the only selected tile opens its rename once the double-click interval passes; a double
/// click there activates the tile and opens no rename.
#[trial]
fn a_slow_click_on_the_caption_renames_and_a_double_click_activates() {
    let (mut fixture, seen) = rig(|_| TileGrid::new("Library").rename(true));
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

    let (mut fixture, seen) = rig(|_| TileGrid::new("Library").rename(true));
    fixture.click_at(caption);
    fixture.advance(1.0).idle(1);
    fixture.click_at(caption).click_at(caption);
    fixture.advance(1.0).idle(2);
    let activated = seen.borrow().collection.iter().any(|event| matches!(event, CollectionEvent::Activate(1)));
    assert!(activated && renames(&seen) == 0, "a double click activates and opens no rename");
}

/// A double click on the grid's background reports `EmptyActivated`; a double click on a tile activates it instead.
#[trial]
fn a_double_click_on_the_background_activates_the_empty_space() {
    let (mut fixture, seen) = rig(|_| TileGrid::new("Library"));
    let tile = fixture.by_label("Tile 2").expect("the tile has a node").rect;
    let empty_space = egui::pos2(tile.center().x, tile.max.y + 80.0);
    let count = |seen: &Rc<RefCell<Seen>>, of: fn(&CollectionEvent<usize, craft_ui::tile_grid::TilePartEvent>) -> bool| {
        seen.borrow().collection.iter().filter(|event| of(event)).count()
    };
    fixture.click_at(empty_space).click_at(empty_space).idle(1);
    assert_eq!(count(&seen, |event| matches!(event, CollectionEvent::EmptyActivated)), 1, "the background's double click is reported once");
    assert_eq!(count(&seen, |event| matches!(event, CollectionEvent::Activate(_))), 0, "no tile activates");
    fixture.advance(1.0).idle(1);
    fixture.click_at(tile.center()).click_at(tile.center()).idle(1);
    assert_eq!(count(&seen, |event| matches!(event, CollectionEvent::Activate(1))), 1, "a double click on a tile activates it");
    assert_eq!(count(&seen, |event| matches!(event, CollectionEvent::EmptyActivated)), 1, "and is no background double click");
}

/// One wheel notch over a grid that overflows scrolls a third of its view.
#[trial]
fn a_wheel_notch_scrolls_a_third_of_the_view() {
    let seen = Rc::new(RefCell::new((0..40usize).map(|_| egui::Rect::NOTHING).collect::<Vec<_>>()));
    let held = seen.clone();
    let view = Rc::new(std::cell::Cell::new(egui::Rect::NOTHING));
    let shown = view.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys: Vec<usize> = (0..40).collect();
        let source = keyed(&keys, |key: &usize| *key);
        let out = TileGrid::new("Library").show_items(ui, &source, &mut |key, icx| {
            held.borrow_mut()[*key] = icx.rect;
            Tile::new(format!("Tile {key}"))
        });
        shown.set(out.response.response.rect);
    });
    fixture.room(Vec2::new(600.0, 300.0));
    fixture.settle();
    let top = seen.borrow()[0].min.y;
    fixture.frame(vec![pointer::moved(view.get().center())]);
    fixture.frame(vec![pointer::wheel_lines(-1.0)]).idle(2);
    let moved = top - seen.borrow()[0].min.y;
    let share = moved / view.get().height();
    assert!((0.3..0.37).contains(&share), "one notch scrolls a third of the view: {moved} of {}", view.get().height());
}

/// Stars clicked on a selected tile rate every selected tile; on a tile outside the selection they rate that tile
/// alone. A selected tile shows its badge bar, so its stars can be clicked without hovering it first.
#[trial]
fn stars_on_a_selected_tile_rate_the_selection() {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2, 3];
        let source = keyed(&keys, |key: &usize| *key);
        let out = TileGrid::new("Library").show_items(ui, &source, &mut |key, _| Tile::new(NAMES[*key]).rating(0));
        held.borrow_mut().grid.extend(out.events);
    });
    fixture.room(Vec2::new(600.0, 400.0));
    fixture.settle();
    for (name, mods) in [("Tile 1", Modifiers::NONE), ("Tile 2", Modifiers::SHIFT)] {
        let tile = fixture.by_label(name).expect("the tile has a node");
        fixture.hold(mods).click(&tile).hold(Modifiers::NONE).settle();
    }
    let stars = fixture.by_label("Rating of Tile 2").expect("a selected tile shows its stars");
    fixture.click(&stars).settle();
    let tile = fixture.by_label("Tile 4").expect("the tile has a node");
    fixture.frame(vec![pointer::moved(tile.rect.center())]).settle();
    let stars = fixture.by_label("Rating of Tile 4").expect("the hovered tile shows its stars");
    fixture.click(&stars).settle();
    let rated: Vec<Vec<usize>> =
        seen.borrow().grid.iter().filter_map(|event| if let GridEvent::Rate { keys, .. } = event { Some(keys.clone()) } else { None }).collect();
    assert_eq!(rated, [vec![0, 1], vec![3]], "the selection is rated together, an unselected tile alone");
}
