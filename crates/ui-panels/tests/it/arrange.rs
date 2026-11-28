//! Window › Arrange over the document windows.

use craft_behavior::dock::{Arrangement, DocumentGroup, DocumentWindows, arrange_rects};
use craft_ui::docking::{ArrangeCommands, ArrangeSpace, Arranged, arrange, arrange_states};
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

/// The Arrange commands published as enabled with `documents` documents in `groups` docked groups and `floating`
/// floating document windows.
fn enabled_commands(documents: usize, groups: usize, floating: usize) -> Vec<ArrangeCommands> {
    arrange_states(documents, groups, floating).into_iter().filter(|(_, state)| state.is_enabled()).map(|(command, _)| command).collect()
}

fn all_on(list: &[ArrangeCommands], commands: &[ArrangeCommands], with: &str, state: &[ArrangeCommands]) {
    for command in commands {
        assert!(list.contains(command), "{with}: {command:?} is enabled: {state:?}");
    }
}

fn all_off(list: &[ArrangeCommands], commands: &[ArrangeCommands], with: &str, state: &[ArrangeCommands]) {
    for command in commands {
        assert!(!list.contains(command), "{with}: {command:?} is greyed: {state:?}");
    }
}

const THREE_UP: [ArrangeCommands; 3] = [ArrangeCommands::ThreeUpHorizontal, ArrangeCommands::ThreeUpVertical, ArrangeCommands::ThreeUpStacked];
const BIG_UP: [ArrangeCommands; 2] = [ArrangeCommands::FourUp, ArrangeCommands::SixUp];

/// An n-up entry greys while its cells could not be filled: 3-up needs three documents, and 4-up and 6-up are
/// measured greyed at two and three documents and enabled at six.
#[trial]
fn an_n_up_entry_greys_until_its_cells_can_be_filled() {
    let two = enabled_commands(2, 1, 0);
    all_on(&two, &[ArrangeCommands::TwoUpHorizontal, ArrangeCommands::TwoUpVertical], "two documents", &two);
    all_off(&two, &THREE_UP, "two documents", &two);
    all_off(&two, &BIG_UP, "two documents", &two);

    let three = enabled_commands(3, 1, 0);
    all_on(&three, &THREE_UP, "three documents", &three);
    all_off(&three, &BIG_UP, "three documents", &three);

    let six = enabled_commands(6, 1, 0);
    all_on(&six, &BIG_UP, "six documents", &six);
}

/// Four and five documents were not measured for 4-up and 6-up, so they keep the state those entries had: enabled.
#[trial]
fn four_and_five_documents_keep_four_up_and_six_up_enabled() {
    for documents in [4_usize, 5] {
        let state = enabled_commands(documents, 1, 0);
        all_on(&state, &BIG_UP, "{documents} documents (not measured, kept as today)", &state);
    }
}

/// Consolidate All to Tabs and the Match entries work from one document; Cascade waits for a floating document
/// window, and Match Rotation keeps its rule of two documents.
#[trial]
fn one_tabbed_document_greys_only_cascade_and_match_rotation() {
    let one = enabled_commands(1, 1, 0);
    all_on(
        &one,
        &[ArrangeCommands::ConsolidateAllToTabs, ArrangeCommands::MatchZoom, ArrangeCommands::MatchLocation, ArrangeCommands::MatchAll],
        "one tabbed document",
        &one,
    );
    all_off(&one, &[ArrangeCommands::Cascade], "one tabbed document", &one);
    all_off(&one, &[ArrangeCommands::MatchRotation], "one tabbed document (not measured, kept as today)", &one);
    let two = enabled_commands(2, 1, 0);
    all_on(&two, &[ArrangeCommands::MatchRotation], "two documents (not measured, kept as today)", &two);

    let floated = enabled_commands(1, 0, 1);
    all_on(&floated, &[ArrangeCommands::Cascade, ArrangeCommands::ConsolidateAllToTabs], "one floating document window", &floated);
    // Consolidate All to Tabs puts that window back into the one tab group, and Cascade goes grey again.
    let consolidated = enabled_commands(1, 1, 0);
    all_off(&consolidated, &[ArrangeCommands::Cascade], "after Consolidate All to Tabs", &consolidated);
}

/// The window entries keep their own rules: with no document every entry is greyed, and Float in Window waits for a
/// docked document while Tile, Tile All, Float All and New Window work from one document on.
#[trial]
fn the_window_entries_keep_their_own_rules() {
    assert!(enabled_commands(0, 0, 0).is_empty(), "with no document every Arrange entry is greyed");
    let docked = enabled_commands(1, 1, 0);
    all_on(
        &docked,
        &[
            ArrangeCommands::FloatInWindow,
            ArrangeCommands::FloatAllInWindows,
            ArrangeCommands::Tile,
            ArrangeCommands::TileAllVertically,
            ArrangeCommands::NewWindowForDocument,
        ],
        "one tabbed document",
        &docked,
    );
    let floating = enabled_commands(2, 0, 2);
    all_off(&floating, &[ArrangeCommands::FloatInWindow], "every document in a floating window", &floating);
    all_on(&floating, &[ArrangeCommands::Cascade, ArrangeCommands::FloatAllInWindows], "every document in a floating window", &floating);
}
