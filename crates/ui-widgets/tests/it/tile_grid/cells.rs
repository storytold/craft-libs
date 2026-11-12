//! Content cells: a grid sizes its cards from what its tiles measure, and keeps that measure while nothing it reads
//! changes.

use craft_behavior::{IconId, keyed};
use craft_ui::contract::ShowCollection as _;
use craft_ui::primitives::IconSpec;
use craft_ui::testing::{Fixture, pointer, trial, widest_runs};
use craft_ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::Vec2;

const KEYS: [usize; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

fn icon_grid(content_cells: bool) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let source = keyed(&KEYS, |key: &usize| *key);
        let grid = TileGrid::new("Icons").caption(CaptionPlace::None).rows_stamp(1);
        let grid = if content_cells { grid.content_cells() } else { grid };
        let _ = grid.show_items(ui, &source, &mut |key, _| Tile::new(format!("Icon {key}")).icon(IconSpec::svg(IconId::new("settings"))));
    });
    fixture.room(Vec2::new(600.0, 400.0));
    fixture.settle();
    fixture
}

/// Icon tiles in content cells are as large as their icon and its inset, so a row holds more of them than of the
/// thumbnail-sized cards a grid uses otherwise; a settled grid moves the pointer over them without measuring a tile.
#[trial]
fn content_cells_fit_the_icons_and_keep_their_measure() {
    let width = |fixture: &Fixture| fixture.by_label("Icon 0").expect("the first tile has a node").rect.width();
    let mut cells = icon_grid(true);
    let thumbs = icon_grid(false);
    assert!(width(&cells) < width(&thumbs), "content cells are smaller than thumbnail cards");
    let row = |fixture: &Fixture| {
        let top = fixture.by_label("Icon 0").expect("tile").rect.top();
        KEYS.iter().filter(|key| fixture.by_label(&format!("Icon {key}")).is_some_and(|tile| (tile.rect.top() - top).abs() < 0.5)).count()
    };
    assert!(row(&cells) > row(&thumbs), "a row holds more content cells than thumbnail cards");
    let over = cells.by_label("Icon 1").expect("tile").center();
    cells.frame(vec![pointer::moved(over)]);
    assert!(widest_runs(cells.ctx()).is_empty(), "a pointer move measured the tiles again: {:?}", widest_runs(cells.ctx()));
}
