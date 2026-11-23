//! The tile grid's layout: justified rows fill the width with whole cards (picture and what the tile adds around it).

use craft_behavior::keyed;
use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowCollection as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{ThumbAspect, ToggleState};
use craft_ui::testing::Fixture;
use craft_ui::tile_grid::{Around, ArrangeSpacing, GridEvent, GroupHeads, Tile, TileGrid, TileGroup, Tiling, arrange};
use egui::Vec2;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const WIDTH: f32 = 400.0;
const ROW: f32 = 100.0;
const GAP: f32 = 8.0;
const INSET: f32 = 4.0;
const HEADER: f32 = 30.0;
const EPS: f32 = 0.01;

fn spacing() -> ArrangeSpacing {
    ArrangeSpacing::new(GAP, INSET, HEADER)
}

#[craft_ui::testing::trial]
fn a_justified_row_but_the_last_fills_the_width_exactly() {
    let aspects = [1.5; 9];
    let around = Around::new(6.0, 2.0, 3.0, 20.0);
    let laid = arrange::<usize>(&aspects, &[], WIDTH, Tiling::justified(ROW), spacing(), around);
    assert!(laid.rows.len() > 1, "nine wide tiles do not fit one row");
    let content = WIDTH - 2.0 * INSET;
    for row in &laid.rows[..laid.rows.len() - 1] {
        let (first, last) = (laid.tiles[row.tiles.start], laid.tiles[row.tiles.end - 1]);
        let span = (last.right() + around.end) - (first.left() - around.start);
        assert!((span - content).abs() < EPS, "a full row of cards spans the content width");
        assert!(row.height >= ROW + around.height() - EPS, "scaling a full row up never shrinks it below the row height");
    }
    let last = laid.rows.last().expect("rows");
    assert!((last.height - ROW - around.height()).abs() < EPS, "the last row keeps the row height");
}

#[craft_ui::testing::trial]
fn a_leading_group_header_sits_at_the_top_edge_and_ungrouped_tiles_keep_the_inset() {
    let aspects = [1.0; 4];
    let groups = [TileGroup::new(0usize, 0, 4, "Folder")];
    let laid = arrange(&aspects, &groups, WIDTH, Tiling::justified(ROW), spacing(), Around::default());
    let header = laid.headers[0];
    assert!(header.top().abs() < EPS, "no inset above the first header");
    assert!((header.left() - INSET).abs() < EPS && (WIDTH - header.right() - INSET).abs() < EPS, "the header keeps the side insets");
    let loose = arrange::<usize>(&aspects, &[], WIDTH, Tiling::justified(ROW), spacing(), Around::default());
    assert!((loose.rows[0].top - INSET).abs() < EPS, "tiles that open the grid keep the inset above them");
}

/// Under `GroupHeads::Rule` a head is a divider: it paints no title, a click on it reports no group event, and a group
/// declared closed still shows its tiles; the group's title stays the heading's name.
#[craft_ui::testing::trial]
fn rule_group_heads_take_no_title_no_tab_stop_and_never_close() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let held = events.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys: Vec<usize> = (0..9).collect();
        let source = keyed(&keys, |key: &usize| *key);
        let groups =
            [TileGroup::new(100usize, 0, 3, "Alpha"), TileGroup::new(101usize, 3, 3, "Beta").open(false), TileGroup::new(102usize, 6, 3, "Gamma")];
        let out = TileGrid::new("Library")
            .groups(&groups)
            .collapsible()
            .group_heads(GroupHeads::Rule)
            .show_items(ui, &source, &mut |key, _| Tile::new(format!("Tile {}", key + 1)));
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(600.0, 600.0));
    fixture.settle();
    assert!(!fixture.texts().iter().any(|text| text.text == "Beta"), "a rule head paints no title");
    assert!(fixture.by_label("Tile 4").is_some(), "a group declared closed is open under Rule");
    let head = fixture.by_label("Beta").expect("the heading keeps its name").rect;
    fixture.click_at(head.center()).settle();
    let grouped = events
        .borrow()
        .iter()
        .any(|event| matches!(event, GridEvent::GroupOpen { .. } | GridEvent::GroupPicked { .. } | GridEvent::GroupMenu { .. }));
    assert!(!grouped, "a click on a rule reports no group event: {:?}", events.borrow());
}

/// A grid in a stack beside another takes its rows' height in the row's measuring run, so the stack holds the whole
/// grid and what follows it: a button under the grid takes clicks.
#[craft_ui::testing::trial]
fn a_grid_in_a_measured_stack_keeps_its_rows_and_what_follows_them() {
    let clicked = Rc::new(Cell::new(false));
    let held = clicked.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys: Vec<u64> = (0..3).collect();
        let source = keyed(&keys, |key: &u64| *key);
        Row::new().show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                let _ = TileGrid::new("Boards").checks(true).max_rows(3).show_items(ui, &source, &mut |key, _| {
                    Tile::new(format!("Board {key}")).check(ToggleState::Off).thumb(ThumbAspect::SQUARE, |_, _| {})
                });
                if Button::new("Clear").show(ui).clicked() {
                    held.set(true);
                }
            });
            Stack::new().show(ui, |ui, _| {
                Text::new("Options").show(ui);
            });
        });
    });
    fixture.room(Vec2::new(800.0, 600.0));
    fixture.settle();
    let (tile, clear) = (fixture.by_label("Board 2").expect("tile").rect, fixture.by_label("Clear").expect("button"));
    assert!(clear.rect.top() >= tile.bottom(), "the button follows the grid's rows: {tile:?} {:?}", clear.rect);
    fixture.click(&clear).settle();
    assert!(clicked.get(), "the button under the grid takes a click");
}
