//! `SplitViews`: the panes of the arrangements in reading order, the divider lists that do not fit, and the dividers'
//! bounds and spans as the pointer drags them.

use craft_behavior::PartKey;
use craft_ui::canvas::{SplitAt, SplitLayout, SplitViews};
use craft_ui::contract::ShowContainer as _;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Pos2, Rect, Vec2};

/// The area the geometry rows lay out in, in points.
const FULL: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(600.0, 400.0));

/// The room the drag rows show the split views in: wide enough that a Tile of five keeps `least_pane` on both sides
/// of each column line.
const ROOM: Vec2 = Vec2::new(600.0, 400.0);

/// The slack of a coordinate comparison after f32 maths, in points.
const TOLERANCE: f32 = 0.5;

const PANE: PartKey = PartKey::new("pane");
const DIVIDER: PartKey = PartKey::new("divider");

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= TOLERANCE
}

/// The panes follow reading order (row by row, then along the row) and no two overlap.
fn in_reading_order_without_overlap(panes: &[Rect], what: &str) {
    for (i, a) in panes.iter().enumerate() {
        for (j, b) in panes.iter().enumerate().skip(i + 1) {
            let overlap = a.intersect(*b);
            assert!(overlap.width() <= TOLERANCE || overlap.height() <= TOLERANCE, "{what}: panes {i} and {j} overlap: {a:?} {b:?}");
            let later = b.center().y > a.bottom() - TOLERANCE || (near(a.top(), b.top()) && b.center().x > a.center().x);
            assert!(later, "{what}: pane {j} comes after pane {i} in reading order: {a:?} {b:?}");
        }
    }
}

fn widths(panes: &[Rect]) -> Vec<f32> {
    panes.iter().map(|pane| pane.width()).collect()
}

fn show(ui: &mut egui::Ui, data: &mut StoryData, layout: SplitLayout, count: usize) -> StoryOutput {
    let (at, at_at) = data.value_at("split", SplitAt::default());
    SplitViews::new("Views", layout, at_at.bind(at)).count(count).show(ui, |_, _| ()).output.into()
}

fn tiled(ui: &mut egui::Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, SplitLayout::Tile, 5)
}

fn three_stacked(ui: &mut egui::Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, SplitLayout::ThreeUpStacked, 3)
}

fn rect_of(fixture: &Fixture, part: PartKey, index: u64) -> Rect {
    let id = fixture.root_id().unwrap_or_else(|| panic!("the split views were shown"));
    fixture.part(id, part.at(index)).unwrap_or_else(|| panic!("part {index} is shown")).rect
}

fn drag_divider(fixture: &mut Fixture, index: u64, by: Vec2) {
    let id = fixture.root_id().unwrap_or_else(|| panic!("the split views were shown"));
    let grip = fixture.part(id, DIVIDER.at(index)).unwrap_or_else(|| panic!("divider {index} is shown"));
    fixture.drag(&grip, by);
}

#[trial]
fn grid_and_stacked_splits_lay_out_and_drag_their_dividers() {
    // Tile with five documents: three columns, two rows, the last row two cells.
    let even = SplitAt::lines(Vec::new(), Vec::new()).rects(SplitLayout::Tile, 5, FULL);
    assert_eq!(even.len(), 5, "five documents tile into five panes");
    in_reading_order_without_overlap(&even, "Tile of 5");
    assert!(even.iter().all(|pane| near(pane.width(), 200.0) && near(pane.height(), 200.0)), "even cells: {even:?}");
    let moved = SplitAt::lines(vec![0.2, 0.7], vec![0.4]).rects(SplitLayout::Tile, 5, FULL);
    in_reading_order_without_overlap(&moved, "Tile of 5, moved lines");
    assert!(near(moved[0].width(), 120.0) && near(moved[1].width(), 300.0) && near(moved[2].width(), 180.0), "{moved:?}");
    assert!(near(moved[3].width(), 120.0) && near(moved[3].height(), 240.0), "the second row follows the lines: {moved:?}");

    // A column list of the wrong length, out of order or outside 0–1 lays out even columns; the row list still applies.
    for columns in [vec![0.2], vec![0.7, 0.2], vec![0.2, 1.5], vec![0.2, f32::NAN]] {
        let panes = SplitAt::lines(columns.clone(), vec![0.4]).rects(SplitLayout::Tile, 5, FULL);
        assert!(widths(&panes).iter().all(|&width| near(width, 200.0)), "{columns:?} lays out even columns: {panes:?}");
        assert!(near(panes[0].height(), 160.0), "{columns:?} keeps the fitting row list: {panes:?}");
    }

    // Stacked documents: one row each, the full width.
    let stacked = SplitAt::lines(Vec::new(), vec![0.2, 0.5]).rects(SplitLayout::TileAllHorizontally, 3, FULL);
    in_reading_order_without_overlap(&stacked, "stacked");
    let heights: Vec<f32> = stacked.iter().map(|pane| pane.height()).collect();
    assert!(near(heights[0], 80.0) && near(heights[1], 120.0) && near(heights[2], 200.0), "{heights:?}");
    assert!(stacked.iter().all(|pane| near(pane.width(), FULL.width())), "{stacked:?}");

    // Two side by side: pane 0 at the start edge, mirrored under right-to-left.
    let two = SplitAt::lines(vec![0.3], Vec::new());
    let ltr = two.placed(SplitLayout::TwoUpVertical, 2, FULL, false);
    in_reading_order_without_overlap(&ltr, "two");
    assert!(near(ltr[0].left(), FULL.left()) && near(ltr[0].width(), 180.0), "{ltr:?}");
    let rtl = two.placed(SplitLayout::TwoUpVertical, 2, FULL, true);
    assert!(near(rtl[0].right(), FULL.right()) && near(rtl[0].width(), 180.0), "pane 0 sits at the right under RTL: {rtl:?}");
    assert!(near(rtl[1].left(), FULL.left()) && near(rtl[1].width(), 420.0), "{rtl:?}");

    // One document shows one pane over the whole area.
    assert_eq!(SplitAt::default().rects(SplitLayout::Tile, 1, FULL), vec![FULL]);

    // A dragged column line stops `least_pane` short of the area's edge and of the next line: the same gap both ways.
    let mut fixture = Fixture::story(&Story::new("tiled", tiled));
    fixture.room(ROOM).settle();
    drag_divider(&mut fixture, 0, Vec2::new(-1000.0, 0.0));
    let gap = rect_of(&fixture, PANE, 0).width();
    assert!(gap > 1.0, "the first pane keeps the least width: {gap}");
    drag_divider(&mut fixture, 0, Vec2::new(1000.0, 0.0));
    let second = rect_of(&fixture, PANE, 1).width();
    assert!(near(second, gap), "the line stops as far short of its neighbour as of the edge: {second} against {gap}");
    let kept = fixture.data().present::<SplitAt>("split").cloned();
    assert_eq!(kept.map(|at| at.columns.len()), Some(2), "a drag writes the whole column list");

    // 3-up Stacked: the row line spans only the second column.
    let mut fixture = Fixture::story(&Story::new("three stacked", three_stacked));
    fixture.room(ROOM).settle();
    let (tall, top) = (rect_of(&fixture, PANE, 0), rect_of(&fixture, PANE, 1));
    let row = rect_of(&fixture, DIVIDER, 1);
    assert!(row.left() >= tall.right() - TOLERANCE, "the row line starts at the second column: {row:?} beside {tall:?}");
    assert!(near(row.right(), top.right()), "the row line reaches the area's end: {row:?} {top:?}");
    drag_divider(&mut fixture, 1, Vec2::new(0.0, 40.0));
    let (tall_after, top_after) = (rect_of(&fixture, PANE, 0), rect_of(&fixture, PANE, 1));
    assert!(top_after.height() > top.height() + 1.0, "dragging the row line down grows the top cell: {top:?} to {top_after:?}");
    assert!(near(tall_after.height(), tall.height()), "the tall pane keeps its height: {tall:?} to {tall_after:?}");
}
