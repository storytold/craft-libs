//! A honeycomb `SwatchTable`: touching hexagon cells in rows that follow the count, picked by click and moved through by
//! the arrows.

use std::cell::Cell;
use std::rc::Rc;

use craft_theme::Srgba;
use craft_ui::choice::{SwatchArrangement, SwatchTable};
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Rect, vec2};

/// Fifteen cells: the hexagon of side 3 (19 cells) cut short, rows of 3, 4, 5 and a short last row of 3 where the full
/// row has 4.
const CELLS: usize = 15;

fn honeycomb(picked: Rc<Cell<Option<usize>>>) -> Fixture {
    let entries: Vec<(Label, Srgba)> =
        (0..CELLS).map(|i| (Label::from(format!("Cell {i}")), Srgba::rgb(i as f32 / CELLS as f32, 0.4, 0.6))).collect();
    let mut fixture = Fixture::new(move |ui| {
        let out = SwatchTable::new("Colors", &entries).arrangement(SwatchArrangement::Honeycomb).show_all(ui);
        if let Some(at) = out.selected {
            picked.set(Some(at));
        }
    });
    fixture.room(vec2(400.0, 300.0));
    fixture.settle();
    fixture
}

fn rects(fixture: &Fixture) -> Vec<Rect> {
    (0..CELLS).map(|i| fixture.by_label(&format!("Cell {i}")).expect("every entry is a cell").rect).collect()
}

/// The entry Space selects after `key`, from the lead the last move left.
fn after(fixture: &mut Fixture, picked: &Cell<Option<usize>>, key: Key) -> Option<usize> {
    fixture.key(key, Modifiers::NONE);
    fixture.settle();
    picked.set(None);
    fixture.key(Key::Space, Modifiers::NONE);
    fixture.settle();
    picked.get()
}

#[trial]
fn honeycomb_cells_touch_without_overlap_and_arrows_follow_neighbours() {
    let picked = Rc::new(Cell::new(None));
    let mut fixture = honeycomb(Rc::clone(&picked));
    let rects = rects(&fixture);
    let width = rects[0].width();

    let mut rows: Vec<Vec<usize>> = Vec::new();
    for (at, rect) in rects.iter().enumerate() {
        match rows.last_mut() {
            Some(row) if (rects[row[0]].center().y - rect.center().y).abs() < 0.5 => row.push(at),
            _ => rows.push(vec![at]),
        }
    }
    assert_eq!(rows.iter().map(Vec::len).collect::<Vec<_>>(), [3, 4, 5, 3], "the rows follow the count: {rects:?}");
    for row in &rows {
        for pair in row.windows(2) {
            assert!((rects[pair[1]].min.x - rects[pair[0]].max.x).abs() < 0.5, "cells of a row touch: {:?}", rects[pair[0]]);
        }
    }
    let first = |row: usize| rects[rows[row][0]].center().x;
    assert!((first(0) - first(1) - width / 2.0).abs() < 0.5, "a longer row starts half a cell earlier: {} against {}", first(0), first(1));
    assert!((first(1) - first(2) - width / 2.0).abs() < 0.5, "the middle row starts half a cell earlier again");
    assert!((first(3) - first(2) - width / 2.0).abs() < 0.5, "the short last row continues the lattice, half a cell after the row above");
    for (at, rect) in rects.iter().enumerate() {
        let distances: Vec<f32> = rects.iter().enumerate().filter(|(other, _)| *other != at).map(|(_, o)| (o.center() - rect.center()).length()).collect();
        let nearest = distances.iter().copied().fold(f32::MAX, f32::min);
        assert!(distances.iter().all(|d| *d >= width - 0.5), "no two cells overlap: cell {at}");
        assert!((nearest - width).abs() < 0.5, "every cell touches a neighbour: cell {at} is {nearest} from the nearest");
    }

    let target = fixture.by_label("Cell 9").expect("a cell");
    fixture.click(&target);
    assert_eq!(picked.get(), Some(9), "a click picks the cell under the pointer");
    assert_eq!(after(&mut fixture, &picked, Key::ArrowRight), Some(10), "Right lands on the cell beside");
    assert_eq!(after(&mut fixture, &picked, Key::ArrowLeft), Some(9), "Left returns");
    assert_eq!(after(&mut fixture, &picked, Key::ArrowDown), Some(13), "Down into a shorter row goes to column c - 1");
    assert_eq!(after(&mut fixture, &picked, Key::ArrowUp), Some(8), "Up into a longer row keeps the column");
    assert_eq!(after(&mut fixture, &picked, Key::ArrowUp), Some(3), "Up into a shorter row goes to column c - 1");
    assert_eq!(after(&mut fixture, &picked, Key::ArrowUp), Some(0), "Up from the first column stays in it");
    assert_eq!(after(&mut fixture, &picked, Key::ArrowDown), Some(3), "Down from the first cell lands on the first cell of the next row");
}
