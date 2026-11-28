//! `SwatchTable`: a honeycomb's touching hexagon cells in rows that follow the count, picked by click and moved through
//! by the arrows; and the preview of the cell under the pointer.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::Preview;
use craft_theme::Srgba;
use craft_ui::choice::{SwatchArrangement, SwatchSpacing, SwatchTable};
use craft_ui::runtime::env::Label;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Pos2, Rect, vec2};

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
        let distances: Vec<f32> =
            rects.iter().enumerate().filter(|(other, _)| *other != at).map(|(_, o)| (o.center() - rect.center()).length()).collect();
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

/// The previews every pass reported since the log was last emptied, the passes that changed nothing left out.
fn reported(log: &RefCell<Vec<Preview<usize>>>) -> Vec<Preview<usize>> {
    log.borrow_mut().drain(..).filter(|preview| *preview != Preview::Unchanged).collect()
}

#[trial]
fn the_cell_under_the_pointer_is_previewed_once_until_the_pointer_leaves() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let entries: Vec<(Label, Srgba)> = (0..8).map(|i| (Label::from(format!("Cell {i}")), Srgba::rgb(i as f32 / 8.0, 0.4, 0.6))).collect();
    let mut fixture = Fixture::new({
        let log = Rc::clone(&log);
        move |ui| {
            let out = SwatchTable::new("Colors", &entries).per_row(4).show_all(ui);
            log.borrow_mut().push(out.preview);
        }
    });
    fixture.room(vec2(400.0, 300.0));
    fixture.settle();
    log.borrow_mut().clear();

    let cell = fixture.by_label("Cell 2").expect("a cell").rect;
    fixture.frame(vec![moved(cell.center())]);
    fixture.settle();
    assert_eq!(reported(&log), vec![Preview::Shows(2)], "moving onto a cell previews it, in one pass");

    fixture.frame(Vec::new());
    fixture.settle();
    assert!(!log.borrow().is_empty(), "the resting frames ran passes");
    assert_eq!(reported(&log), Vec::new(), "a pointer resting on the cell reports nothing more");

    let last = fixture.by_label("Cell 7").expect("a cell").rect;
    fixture.frame(vec![moved(Pos2::new(cell.center().x, last.max.y + 40.0))]);
    fixture.settle();
    assert_eq!(reported(&log), vec![Preview::Ended], "leaving the cells ends the preview, once");
}

/// The gaps between the cells of a grid of six cells three to a row, sitting as `spacing` says: across a row, and down
/// a column.
fn gaps(spacing: SwatchSpacing) -> (f32, f32) {
    let entries: Vec<(Label, Srgba)> = (0..6).map(|i| (Label::from(format!("Cell {i}")), Srgba::rgb(i as f32 / 6.0, 0.4, 0.6))).collect();
    let mut fixture = Fixture::new(move |ui| {
        let _ = SwatchTable::new("Colors", &entries).per_row(3).spacing(spacing).show_all(ui);
    });
    fixture.room(vec2(400.0, 300.0));
    fixture.settle();
    let cell = |i: usize| fixture.by_label(&format!("Cell {i}")).expect("every entry is a cell").rect;
    (cell(1).left() - cell(0).right(), cell(3).top() - cell(0).bottom())
}

/// Cells in rows sit apart across a row, nearer than spaced cells, and their rows lie further apart than that.
#[trial]
fn rows_hold_cells_a_little_apart_and_the_rows_further_apart() {
    let (touching, rows, spaced) = (gaps(SwatchSpacing::Touching), gaps(SwatchSpacing::Rows), gaps(SwatchSpacing::Spaced));
    assert!(rows.0 > touching.0 && rows.0 < spaced.0, "across a row: {touching:?} < {rows:?} < {spaced:?}");
    assert!(rows.1 > rows.0, "the rows lie further apart than the cells in a row: {rows:?}");
}

/// The widths of the outlines painted inside `cell`.
fn outline_widths(shapes: &[egui::Shape], cell: Rect, out: &mut Vec<f32>) {
    let inside = cell.expand(0.5);
    for shape in shapes {
        match shape {
            egui::Shape::Vec(inner) => outline_widths(inner, cell, out),
            egui::Shape::Rect(r) if inside.contains_rect(r.rect) && r.stroke.width > 0.0 && r.stroke.color.a() > 0 => out.push(r.stroke.width),
            egui::Shape::Path(path) if !path.points.is_empty() && path.points.iter().all(|p| inside.contains(*p)) && path.stroke.width > 0.0 => {
                out.push(path.stroke.width);
            }
            _ => {}
        }
    }
}

/// The widest outline painted round the second cell of a table of three, on a display of two pixels per point.
fn outline_width(hairline: bool) -> f32 {
    let entries = vec![
        (Label::from("Grey"), Srgba::rgb(0.5, 0.5, 0.5)),
        (Label::from("Black"), Srgba::rgb(0.0, 0.0, 0.0)),
        (Label::from("White"), Srgba::rgb(1.0, 1.0, 1.0)),
    ];
    let shown = entries.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = SwatchTable::new("Colors", &shown).per_row(3).spacing(SwatchSpacing::Spaced).hairline(hairline).show_all(ui);
    });
    fixture.room(vec2(200.0, 100.0));
    fixture.pixels_per_point(2.0).settle();
    let shapes = fixture.frame_shapes();
    let cell = fixture.by_label("Black").expect("a cell").rect;
    let mut widths = Vec::new();
    outline_widths(&shapes, cell, &mut widths);
    widths.into_iter().fold(None, |widest: Option<f32>, w| Some(widest.map_or(w, |m| m.max(w)))).expect("the cell has an outline")
}

/// A hairline table outlines its cells with a thinner line than a plain one.
#[trial]
fn hairline_cells_draw_a_thinner_outline() {
    let (plain, hairline) = (outline_width(false), outline_width(true));
    assert!(hairline < plain, "a hairline outline ({hairline}) is thinner than a thin one ({plain})");
}

/// The colours of the outlines painted inside `cell`.
fn outline_colors(shapes: &[egui::Shape], cell: Rect, out: &mut Vec<egui::Color32>) {
    let inside = cell.expand(0.5);
    for shape in shapes {
        match shape {
            egui::Shape::Vec(inner) => outline_colors(inner, cell, out),
            egui::Shape::Rect(r) if inside.contains_rect(r.rect) && r.stroke.width > 0.0 && r.stroke.color.a() > 0 => out.push(r.stroke.color),
            egui::Shape::Path(path) if !path.points.is_empty() && path.points.iter().all(|p| inside.contains(*p)) && path.stroke.width > 0.0 => {
                if let egui::epaint::ColorMode::Solid(color) = path.stroke.color {
                    out.push(color);
                }
            }
            _ => {}
        }
    }
}

/// The summed channels of the outline round the black cell of a table of three: higher is lighter.
fn outline_lightness(hairline: bool) -> u32 {
    let entries = vec![
        (Label::from("Grey"), Srgba::rgb(0.5, 0.5, 0.5)),
        (Label::from("Black"), Srgba::rgb(0.0, 0.0, 0.0)),
        (Label::from("White"), Srgba::rgb(1.0, 1.0, 1.0)),
    ];
    let shown = entries.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = SwatchTable::new("Colors", &shown).per_row(3).spacing(SwatchSpacing::Spaced).hairline(hairline).show_all(ui);
    });
    fixture.room(vec2(200.0, 100.0));
    fixture.pixels_per_point(2.0).settle();
    let shapes = fixture.frame_shapes();
    let cell = fixture.by_label("Black").expect("a cell").rect;
    let mut colors = Vec::new();
    outline_colors(&shapes, cell, &mut colors);
    let color = colors.first().copied().expect("the cell has an outline");
    u32::from(color.r()) + u32::from(color.g()) + u32::from(color.b())
}

/// On the light theme a hairline cell's outline is the lighter border colour, not the control edge, so a grid of
/// colours reads as colours rather than grey frames.
#[trial]
fn hairline_cells_take_the_lighter_border_colour() {
    let (plain, hairline) = (outline_lightness(false), outline_lightness(true));
    assert!(hairline > plain, "a hairline outline ({hairline}) is lighter than a plain one ({plain})");
}
