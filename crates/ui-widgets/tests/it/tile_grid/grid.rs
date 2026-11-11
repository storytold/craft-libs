//! The tile grid's layout: justified rows fill the width with whole cards (picture and what the tile adds around it).

use craft_ui::tile_grid::{Around, ArrangeSpacing, Tiling, arrange};

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
