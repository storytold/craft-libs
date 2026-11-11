//! The document table geometry.

use craft_behavior::{CellAt, CellRange};
use craft_pages::ui::table::TableGeometry;
use craft_ui::testing::trial;
use egui::pos2;

#[trial]
fn table_geometry_finds_cells_and_clamps_past_the_last_edge() {
    let geometry = TableGeometry::new(vec![0.0, 20.0, 50.0], vec![0.0, 40.0, 80.0, 120.0]);
    assert_eq!(geometry.cell_at(pos2(40.0, 19.9)), Some(CellAt::new(0, 1)), "a band owns its leading edge");
    assert_eq!(geometry.cell_at(pos2(39.9, 20.0)), Some(CellAt::new(1, 0)));
    assert_eq!(geometry.cell_at(pos2(120.0, 10.0)), None, "the last edge is outside");
    let past = CellRange::new(1, 4, 1..3, 2);
    assert_eq!(geometry.rect(&past), egui::Rect::from_min_max(pos2(40.0, 20.0), pos2(120.0, 50.0)));
}
