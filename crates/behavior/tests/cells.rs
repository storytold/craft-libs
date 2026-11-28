//! The shared cell-selection rules.

use craft_behavior::{CellAdvance, CellAt, CellRange, CellSelection, CellSpace, Positions, SelectionModel};

fn c(row: u32, column: u32) -> CellAt<u32> {
    CellAt::new(row, column)
}

fn r(a: (u32, u32), b: (u32, u32)) -> CellRange<u32> {
    CellRange::cells(c(a.0, a.1), c(b.0, b.1))
}

// A range grows over a merge it crosses and again over a merge that one reaches; whole columns
// stay as they are in `CellSpace::cover`.
#[test]
fn ranges_grow_over_merged_cells_but_whole_lines_stay() {
    let merges = [r((1, 1), (2, 2)), r((3, 3), (3, 5))];
    let rows = Positions::new(10);
    let space = CellSpace::new(&rows, 8).merges(&merges);
    // B2:C3 holds the first merge; D4:F4 touches it only through the second merge.
    assert_eq!(space.cover(r((0, 0), (1, 1))), r((0, 0), (2, 2)));
    assert_eq!(space.cover(r((2, 2), (3, 3))), r((1, 1), (3, 5)));
    let whole = space.whole_columns(1, 1).expect("the grid has rows");
    assert_eq!(space.cover(whole.clone()), whole);
    assert_eq!(space.cover(space.whole_rows(1, 1)), space.whole_rows(1, 1));
}

// A stored selection with ranges but no active cell is refused, and so is an active cell with no ranges, where
// `advance` would otherwise index past the end; a stored selection keeps its order and current range through a
// round trip. The empty selection loads.
#[test]
fn a_stored_selection_is_checked_on_the_way_in() {
    assert_eq!(serde_json::from_str::<CellSelection<u32>>(r#"{"ranges":[]}"#).unwrap(), CellSelection::default());
    let no_ranges = r#"{"ranges":[],"active":{"row":0,"column":0},"anchor":{"row":0,"column":0}}"#;
    assert!(serde_json::from_str::<CellSelection<u32>>(no_ranges).is_err());
    let no_active = r#"{"ranges":[{"start":{"row":0,"column":0},"end":{"row":1,"column":1}}]}"#;
    assert!(serde_json::from_str::<CellSelection<u32>>(no_active).is_err());
    let outside = r#"{"ranges":[{"start":{"row":0,"column":0},"end":{"row":1,"column":1}}],"active":{"row":0,"column":4}}"#;
    assert!(serde_json::from_str::<CellSelection<u32>>(outside).is_err());
    let rows = Positions::new(10);
    let space = CellSpace::new(&rows, 10);
    let mut sel = CellSelection::from_ranges(vec![r((2, 2), (3, 3)), r((6, 6), (6, 7))], c(6, 6)).unwrap();
    sel.advance(CellAdvance::Tab, true, &space);
    let back: CellSelection<u32> = serde_json::from_str(&serde_json::to_string(&sel).unwrap()).unwrap();
    assert_eq!(back, sel);
}

// Cmd+click takes out a held cell between the first and last row of a range, which `apply` cannot see without the
// order, and the row stays selected while another of its cells is.
#[test]
fn toggle_takes_out_a_cell_between_the_rows_of_a_range() {
    let rows = Positions::new(20);
    let space = CellSpace::new(&rows, 6);
    let mut sel = CellSelection::<u32>::default();
    sel.select(c(2, 1), &space);
    sel.extend_to(c(9, 2), &space);
    assert!(sel.toggle(c(5, 1), &space));
    assert_eq!(sel.ranges().len(), 1);
    let placed = sel.placed(&rows);
    assert_eq!((placed.holds_cell(5, 1), placed.holds_cell(5, 2), placed.holds_row(5)), (false, true, true));
    assert!(sel.toggle(c(5, 2), &space));
    assert!(!sel.placed(&rows).holds_row(5));
    assert!(sel.toggle(c(5, 2), &space));
    assert!(sel.toggle(c(12, 4), &space));
    assert_eq!(sel.ranges().len(), 2);
}
