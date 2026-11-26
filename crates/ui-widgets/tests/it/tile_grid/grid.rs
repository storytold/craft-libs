//! The tile grid's layout: justified rows fill the width with whole cards (picture and what the tile adds around it);
//! a bound zoom that keeps the tile under the pointer in place; a drag whose tiles leave; insert gaps by display
//! position.

use craft_behavior::keyed;
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, CollectionEvent, ReorderRule, Show as _, ShowCollection as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{ThumbAspect, ToggleState};
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Fixture, pointer};
use craft_ui::tile_grid::{Around, ArrangeSpacing, GridEvent, GridZoom, GroupHeads, Tile, TileGrid, TileGroup, TileZoom, Tiling, ZoomStep, arrange};
use egui::{Key, Modifiers, Pos2, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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

/// Tiles of the shown grids: enough rows to scroll several views.
const TILES: usize = 120;

/// What a grid showed in its latest pass: each tile's rect, the grid's rect, and what it reported in every pass.
struct Shown {
    tiles: HashMap<usize, Rect>,
    view: Rect,
    grid: Vec<GridEvent<usize>>,
}

impl Shown {
    /// Nothing shown yet, shared between a fixture's frames and the trial.
    fn shared() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Shown { tiles: HashMap::new(), view: Rect::NOTHING, grid: Vec::new() }))
    }

    /// The tile whose centre is nearest `at`, and its rect.
    fn nearest(&self, at: Pos2) -> (usize, Rect) {
        let nearest = self.tiles.iter().min_by(|a, b| a.1.center().distance(at).total_cmp(&b.1.center().distance(at)));
        nearest.map(|(key, rect)| (*key, *rect)).expect("the grid shows tiles")
    }

    /// The rect tile `key` has now.
    fn tile(&self, key: usize) -> Rect {
        self.tiles.get(&key).copied().expect("the tile stays in view")
    }
}

/// A Ctrl/Cmd+wheel zoom and a Cmd/Ctrl+= step reflow the grid into another column count, and the tile under the
/// pointer keeps its top through both.
#[craft_ui::testing::trial]
fn a_wheel_zoom_keeps_the_tile_under_the_pointer_in_place() {
    let zoom = Rc::new(Cell::new(TileZoom::Ladder.default_value()));
    let shown = Shown::shared();
    let (held_zoom, held) = (zoom.clone(), shown.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys: Vec<usize> = (0..TILES).collect();
        let source = keyed(&keys, |key: &usize| *key);
        let mut value = held_zoom.get();
        held.borrow_mut().tiles.clear();
        let grid = TileGrid::new("Library").zoom(GridZoom::Bound(Bind::bound(&mut value), TileZoom::Ladder));
        let out = grid.show_items(ui, &source, &mut |key, icx| {
            held.borrow_mut().tiles.insert(*key, icx.rect);
            Tile::new(format!("Tile {key}"))
        });
        held_zoom.set(value);
        held.borrow_mut().view = out.response.response.rect;
    });
    fixture.room(Vec2::new(600.0, 400.0));
    fixture.settle();
    let view = shown.borrow().view;
    // A scrolled grid, so the kept tile has room to move either way.
    fixture.frame(vec![pointer::moved(view.center())]);
    fixture.frame(vec![pointer::wheel_lines(-2.0)]).settle();

    let (key, before) = shown.borrow().nearest(view.center());
    fixture.frame(vec![pointer::moved(before.center())]);
    let start = zoom.get();
    fixture.frame(vec![pointer::zoom_wheel(120.0)]).settle();
    assert!(zoom.get() != start, "the wheel zoom moved the bound value from {start}");
    {
        let seen = shown.borrow();
        let mut keys: Vec<usize> = seen.tiles.keys().copied().collect();
        keys.sort_unstable();
        eprintln!(
            "DIAG a_wheel_zoom_keeps_the_tile_under_the_pointer_in_place: key {key} before {before:?} zoom {start} -> {} view {:?} shown {keys:?}",
            zoom.get(),
            seen.view
        );
    }
    let after = shown.borrow().tile(key);
    assert!((after.top() - before.top()).abs() <= 1.0, "the tile under the pointer keeps its top: {before:?} to {after:?}");

    let (key, _) = shown.borrow().nearest(view.center());
    let lead = shown.borrow().tile(key);
    fixture.click_at(lead.center()).settle();
    let before = shown.borrow().tile(key);
    let start = zoom.get();
    fixture.key(Key::Equals, Modifiers::COMMAND).settle();
    // Keeping the tile in place reruns the key's pass inside its frame; the rerun takes no input, so one step.
    assert_eq!(zoom.get(), TileZoom::Ladder.step(start, ZoomStep::In), "Cmd/Ctrl+= raised the bound value one step from {start}");
    let after = shown.borrow().tile(key);
    assert!((after.top() - before.top()).abs() <= 1.0, "the lead tile under the pointer keeps its top: {before:?} to {after:?}");
}

/// A drag whose tiles leave the grid's source while it is held ends with no reorder; the same drag with its tiles
/// still there reorders.
#[craft_ui::testing::trial]
fn a_drag_whose_tiles_leave_ends_without_a_reorder() {
    let keys = Rc::new(RefCell::new((0..6_usize).collect::<Vec<_>>()));
    let reorders = Rc::new(Cell::new(0_usize));
    let (held_keys, held_reorders) = (keys.clone(), reorders.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys = held_keys.borrow().clone();
        let source = keyed(&keys, |key: &usize| *key);
        let grid = TileGrid::new("Library").reorder(ReorderRule::new());
        let out = grid.show_items(ui, &source, &mut |key, _| Tile::new(format!("Tile {key}")));
        let moved = out.response.events.iter().filter(|event| matches!(event, CollectionEvent::Reorder(_))).count();
        held_reorders.set(held_reorders.get() + moved);
    });
    fixture.room(Vec2::new(600.0, 400.0));
    fixture.settle();
    let rect = |fixture: &Fixture, key: usize| fixture.by_label(&format!("Tile {key}")).expect("the tile has a node").rect;
    let (from, onto) = (rect(&fixture, 1), rect(&fixture, 3));
    let by = onto.center() + Vec2::new(onto.width() / 4.0, 0.0) - from.center();
    fixture.drag_at(from.center(), by).settle();
    assert!(reorders.get() > 0, "a drag of a tile onto a later place reorders");

    fixture.drag_at_pausing(from.center(), by, |fixture| {
        keys.borrow_mut().retain(|key| *key != 1);
        fixture.idle(2);
        reorders.set(0);
    });
    fixture.settle();
    assert_eq!(reorders.get(), 0, "a drag whose tile left the source ends without a reorder");
}

/// With rows scrolled out of view, an insert gap is named by and reports the display position of the tile it
/// precedes, sits on that tile's start edge, and no gap shows for a position out of view or past the tiles in view.
#[craft_ui::testing::trial]
fn insert_gaps_keep_document_positions_when_rows_are_virtualised() {
    let shown = Shown::shared();
    let held = shown.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys: Vec<usize> = (0..TILES).collect();
        let source = keyed(&keys, |key: &usize| *key);
        let name = |at: usize| Label::from(format!("Insert before {}", at + 1));
        held.borrow_mut().tiles.clear();
        let grid = TileGrid::new("Library").insert_gaps(&name);
        let out = grid.show_items(ui, &source, &mut |key, icx| {
            held.borrow_mut().tiles.insert(*key, icx.rect);
            Tile::new(format!("Tile {key}"))
        });
        let mut seen = held.borrow_mut();
        seen.view = out.response.response.rect;
        seen.grid.extend(out.events);
    });
    fixture.room(Vec2::new(600.0, 300.0));
    fixture.settle();
    let first_gap = fixture.by_label("Insert before 1").expect("the first gap").rect;
    let first_tile = shown.borrow().tile(0);
    assert!(first_gap.width() >= first_tile.width() / 8.0, "the insert control must be discoverable: {first_gap:?} beside {first_tile:?}");
    let view = shown.borrow().view;
    fixture.frame(vec![pointer::moved(view.center())]);
    for _ in 0..4 {
        fixture.frame(vec![pointer::wheel_lines(-3.0)]).idle(1);
    }
    fixture.settle();

    let first = {
        let shown = shown.borrow();
        let inside = shown.tiles.iter().filter(|(_, rect)| rect.top() >= view.top() && rect.bottom() <= view.bottom());
        inside.map(|(key, _)| *key).min().expect("whole tiles show in the view")
    };
    assert!(first > 0, "the first rows scrolled out of view");
    assert!(fixture.by_label("Insert before 1").is_none(), "no gap shows before a tile out of view");
    assert!(fixture.by_label(&format!("Insert before {}", TILES + 1)).is_none(), "no end gap shows while the last tile is out of view");
    let gap = fixture.by_label(&format!("Insert before {}", first + 1)).expect("the first whole tile in view has a gap before it");
    let tile = shown.borrow().tile(first);
    let edge = gap.rect.center();
    assert!(
        (edge.x - tile.left()).abs() < tile.width() / 2.0 && tile.y_range().contains(edge.y),
        "the gap sits on the tile's start edge: {:?} beside {tile:?}",
        gap.rect
    );

    shown.borrow_mut().grid.clear();
    fixture.click(&gap).settle();
    let inserts: Vec<usize> =
        shown.borrow().grid.iter().filter_map(|event| if let GridEvent::Insert { at, .. } = event { Some(*at) } else { None }).collect();
    assert_eq!(inserts, [first], "the gap reports the display position it inserts before, not a row index");
}

/// An insert gap at rest is drawn muted: its circle's stroke differs from the one the pointer's gap takes.
#[craft_ui::testing::trial]
fn an_insert_gap_at_rest_is_muted() {
    let mut fixture = Fixture::new(move |ui| {
        let keys: Vec<usize> = (0..4).collect();
        let source = keyed(&keys, |key: &usize| *key);
        let name = |at: usize| Label::from(format!("Insert before {}", at + 1));
        let _ = TileGrid::new("Library").insert_gaps(&name).show_items(ui, &source, &mut |key, _| Tile::new(format!("Tile {key}")));
    });
    fixture.room(Vec2::new(600.0, 300.0));
    fixture.frame(vec![pointer::moved(egui::Pos2::new(590.0, 290.0))]).settle();
    fn strokes(shapes: &[egui::Shape]) -> Vec<egui::Color32> {
        shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Circle(circle) if circle.stroke.width > 0.0 => Some(circle.stroke.color),
                _ => None,
            })
            .collect()
    }
    let rest = strokes(&fixture.frame_shapes());
    assert!(!rest.is_empty(), "the gaps draw their circles");
    let gap = fixture.by_label("Insert before 1").expect("the first gap").rect;
    fixture.frame(vec![pointer::moved(gap.center())]).settle();
    let hovered = strokes(&fixture.frame_shapes());
    assert!(hovered.iter().any(|color| !rest.contains(color)), "the gap under the pointer is drawn in another colour: {rest:?} / {hovered:?}");
}
