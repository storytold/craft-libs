//! Window › Arrange over the document windows.

use craft_behavior::dock::{Arrangement, DocumentGroup, DocumentWindows, arrange_rects};
use craft_ui::docking::{ArrangeCommands, ArrangeSpace, Arranged, arrange};
use craft_ui::testing::trial;
use egui::{Pos2, Rect, Vec2};

const AREA: Rect = Rect { min: Pos2::new(0.0, 40.0), max: Pos2::new(900.0, 640.0) };

/// Two documents docked in one group and a third in a floating window.
fn windows() -> DocumentWindows<u32> {
    let mut windows = DocumentWindows::default();
    windows.docked = vec![DocumentGroup::new(0, vec![1, 2])];
    windows.floating = vec![(DocumentGroup::new(1, vec![3]), Rect::from_min_size(Pos2::new(50.0, 80.0), Vec2::new(300.0, 200.0)))];
    windows
}

/// Tile All lays every document out in docked cells of the document area, as Tile does: no document is left in (or
/// moved into) a floating window, each gets its own group, and the cells run across or down the area.
#[trial]
fn tile_all_docks_every_document_in_its_own_cell_of_the_document_area() {
    let space = ArrangeSpace::new(AREA, Vec2::new(400.0, 300.0), 24.0, Some(1));
    for (command, arrangement) in [
        (ArrangeCommands::TileAllVertically, Arrangement::TileAllVertically),
        (ArrangeCommands::TileAllHorizontally, Arrangement::TileAllHorizontally),
    ] {
        let mut windows = windows();
        assert_eq!(arrange(&mut windows, command, &space), Arranged::Windows);
        assert!(windows.floating.is_empty(), "{command:?} leaves no floating window: {:?}", windows.floating);
        let groups: Vec<_> = windows.docked.iter().map(|group| group.documents.clone()).collect();
        assert_eq!(groups, [vec![1], vec![2], vec![3]], "{command:?} gives each document its own docked group");
        assert_eq!(windows.arrangement, Some(arrangement), "{command:?} is the arrangement the document area shows");
        let cells = arrange_rects(arrangement, AREA, groups.len()).expect("Tile All has cells for three documents");
        assert!(cells.iter().all(|cell| AREA.contains_rect(*cell)), "the cells lie in the document area: {cells:?}");
        let across = cells.windows(2).all(|pair| pair[1].left() >= pair[0].right() - 0.5);
        let down = cells.windows(2).all(|pair| pair[1].top() >= pair[0].bottom() - 0.5);
        match arrangement {
            Arrangement::TileAllVertically => assert!(across, "Tile All Vertically puts the cells side by side: {cells:?}"),
            _ => assert!(down, "Tile All Horizontally stacks the cells: {cells:?}"),
        }
    }
}
