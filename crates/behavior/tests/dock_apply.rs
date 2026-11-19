//! Dock transactions and workspaces.

use craft_behavior::Axis;
use craft_behavior::dock::{
    Action, Column, DockEdge, DockError, DockPlacement, DockSide, Floating, Group, Layout, NamedWorkspace, Node, PanelPlace, PanelState, Permissions,
    SplitSize, Target, Workspace, WorkspaceChrome, WorkspaceKeeps, WorkspaceSet, node_at,
};
use emath::{Rect, pos2, vec2};

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(pos2(x, y), vec2(w, h))
}

fn window() -> Rect {
    rect(100.0, 100.0, 300.0, 400.0)
}

fn g(panels: &[u32], active: usize) -> Group<u32> {
    Group::new(panels.to_vec(), active)
}

fn tabs(panels: &[u32], active: usize) -> Node<u32> {
    Node::tabs(panels.to_vec(), active)
}

/// Canvas 1 above frame tabs [11, 12]; an end dock with column A ([2, 3], [4]) and column B ([5]); a floating
/// window with groups [6, 7] and [8].
fn docked() -> Layout<u32> {
    let middle = Node::split(Axis::Y, SplitSize::Ratio(0.7), tabs(&[1], 0), tabs(&[11, 12], 1));
    let dock = Node::dock(
        DockEdge::End,
        vec![Column::new(vec![g(&[2, 3], 1), g(&[4], 0)]).with_width(180.0), Column::new(vec![g(&[5], 0)]).with_width(120.0)],
    );
    let root = Node::split(Axis::X, SplitSize::FixedSecond(300.0), middle, dock);
    Layout::new(Some(root), vec![Floating::stack(vec![g(&[6, 7], 1), g(&[8], 0)], window()).unwrap()])
}

const ALL: [u32; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 11, 12];

fn allow(_: &u32) -> Permissions {
    Permissions::ALL
}

fn sorted(layout: &Layout<u32>) -> Vec<u32> {
    let mut panels: Vec<u32> = layout.panels().into_iter().copied().collect();
    panels.sort_unstable();
    panels
}

fn group_members(layout: &Layout<u32>, panel: u32) -> Vec<u32> {
    match layout.place(&panel).unwrap() {
        place @ PanelPlace::Node(_) => match layout.node_of(&place).unwrap() {
            Node::Tabs { panels, .. } => panels.clone(),
            other => panic!("{other:?}"),
        },
        place => layout.group(&place).unwrap().panels.clone(),
    }
}

fn end_dock(layout: &Layout<u32>) -> &Vec<Column<u32>> {
    let path = layout.dock_path(DockEdge::End).unwrap();
    let Some(Node::Dock { columns, .. }) = node_at(layout.root.as_ref().unwrap(), &path) else { panic!() };
    columns
}

#[test]
fn apply_all_changes_everything_or_nothing_and_reports_the_closed_panels() {
    let mut layout = docked();
    let closed = layout.apply_all([Action::close_group(3), Action::activate(4)], allow).unwrap();
    assert_eq!(closed, [2, 3]);
    assert!(!layout.contains(&2));

    let before = layout.clone();
    let keep = |p: &u32| if *p == 4 { Permissions::ALL.closable(false) } else { Permissions::ALL };
    assert_eq!(layout.apply_all([Action::close(8), Action::close(4)], keep), Err(DockError::NotAllowed));
    assert_eq!(layout, before);
}

#[test]
fn reconcile_drops_unknown_panels_and_places_missing_registered_ones() {
    let default = Workspace::new(docked());
    let mut workspace = default.clone();
    workspace.layout.apply(Action::close(5)).unwrap();
    workspace.layout.apply(Action::close(12)).unwrap();
    workspace.layout.apply(Action::open_at(99, 2, DockPlacement::tab(None))).unwrap();
    let stray = workspace.layout.location(&99).unwrap();
    workspace.hidden.push((98, stray));
    let registered = |p: &u32| *p < 90;
    workspace.reconcile(registered, allow, &default);
    workspace.validate().unwrap();
    // Unregistered panels wait in `hidden` and come back once registered.
    assert_eq!(workspace.hidden.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [98, 99]);
    assert_eq!(sorted(&workspace.layout), ALL);
    workspace.set_visible(99, true, None, false, allow).unwrap();
    assert_eq!(group_members(&workspace.layout, 2), [2, 3, 99]);
    assert_eq!(end_dock(&workspace.layout).len(), 2);
    assert_eq!(group_members(&workspace.layout, 11), [11, 12]);

    let mut broken = default.clone();
    broken.layout.floating[0].rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(-1.0, 400.0));
    broken.reconcile(registered, allow, &default);
    assert_eq!(broken.layout, default.layout);
}

#[test]
fn a_workspace_set_with_a_floating_group_round_trips_through_json() {
    let layout = Layout::new(Some(Node::tabs(vec![1u32, 2], 0)), vec![Floating::single(vec![3u32], 0, rect(1.0, 2.0, 300.0, 200.0)).with_id(5)]);
    let mut set = WorkspaceSet::new(vec![NamedWorkspace::new("essentials", "Essentials", Workspace::default())]);
    set.save("Mine", Workspace::new(layout), WorkspaceChrome::default(), WorkspaceKeeps::new(false, true, true)).unwrap();
    let saved = serde_json::to_string(&set).unwrap();
    let mut loaded: WorkspaceSet<u32> = serde_json::from_str(&saved).unwrap();
    assert!(loaded.builtin.is_empty(), "built-ins are code, not stored");
    loaded.builtin = set.builtin.clone();
    assert_eq!(loaded, set);
    assert!(loaded.saved[0].keeps.toolbar && loaded.saved[0].keeps.menus, "the captures are stored with the workspace");

    let older = saved.replace(",\"keeps_toolbar\":true", "");
    assert_ne!(older, saved, "the file names keeps_toolbar");
    let older: WorkspaceSet<u32> = serde_json::from_str(&older).unwrap();
    assert_eq!(older.saved[0].keeps, WorkspaceKeeps::new(false, true, false), "a file from before the toolbar box loads with it off");
}

/// A workspace whose layout has a bottom frame (Motion's Timeline) shows it when selected and drops it when another
/// workspace is selected; the host sees the switch in the live layout.
#[test]
fn a_workspace_with_a_bottom_pane_opens_and_closes_it_on_switch() {
    const TIMELINE: u32 = 9;
    let photography = Workspace::new(Layout::new(Some(tabs(&[1, 2], 0)), Vec::new()));
    let motion =
        Workspace::new(Layout::new(Some(Node::split(Axis::Y, SplitSize::FixedSecond(200.0), tabs(&[1, 2], 0), tabs(&[TIMELINE], 0))), Vec::new()));
    let mut set = WorkspaceSet::new(vec![
        NamedWorkspace::new("photography", "Photography", photography.clone()),
        NamedWorkspace::new("motion", "Motion", motion),
    ]);
    let mut live = photography;
    set.select("motion", &mut live).unwrap();
    let bottom = live.layout.place(&TIMELINE).unwrap();
    assert_eq!(bottom, PanelPlace::Node(vec![true]), "the Timeline is the second (bottom) frame of the split");
    set.select("photography", &mut live).unwrap();
    assert!(!live.layout.contains(&TIMELINE), "leaving Motion closes the Timeline");
    set.select("motion", &mut live).unwrap();
    assert!(live.layout.contains(&TIMELINE), "coming back opens it again");
}

/// Switching and resetting replace the live workspace and move it to a new revision; edits and selecting the shown
/// workspace keep it, and the revision is neither saved nor compared.
#[test]
fn only_a_replacement_changes_the_live_revision() {
    let essentials = Workspace::new(Layout::new(Some(tabs(&[1, 2], 0)), Vec::new()));
    let painting = Workspace::new(Layout::new(Some(tabs(&[3], 0)), Vec::new()));
    let mut set = WorkspaceSet::new(vec![
        NamedWorkspace::new("essentials", "Essentials", essentials.clone()),
        NamedWorkspace::new("painting", "Painting", painting),
    ]);
    let mut live = essentials.clone();
    let start = live.revision();
    live.apply(Action::activate(2), false, allow).unwrap();
    live.set_visible(1, false, None, false, allow).unwrap();
    set.select("essentials", &mut live).unwrap();
    assert_eq!(live.revision(), start, "edits and selecting the shown workspace keep the revision");
    set.select("painting", &mut live).unwrap();
    let switched = live.revision();
    assert!(switched != start && live.layout.contains(&3), "a switch replaces the live workspace");
    set.reset(&mut live).unwrap();
    assert!(live.revision() != switched, "a reset replaces it again");
    set.select("essentials", &mut live).unwrap();
    assert!(!live.layout.contains(&1), "the switch back shows the edited essentials");
    let loaded: Workspace<u32> = serde_json::from_str(&serde_json::to_string(&live).unwrap()).unwrap();
    assert_eq!(loaded, live, "the revision is not saved and not part of equality");
}

/// Open of a hidden panel follows its record: a floating panel returns to its window; a record whose anchor is gone
/// falls back to the anchor Open names.
#[test]
fn opening_a_closed_floating_panel_restores_its_window_place() {
    let mut workspace = Workspace::new(docked());
    workspace.apply(Action::close(8), false, allow).unwrap();
    assert!(workspace.layout.place(&8).is_none());
    workspace.apply(Action::open(8, Some(11)), false, allow).unwrap();
    assert!(matches!(workspace.layout.place(&8), Some(PanelPlace::Floating { .. })), "back in its window");
    assert!(workspace.hidden.is_empty());

    let mut stray = workspace.layout.location(&5).unwrap();
    stray.anchor = Some(99);
    workspace.hidden.push((50, stray));
    workspace.apply(Action::open(50, Some(11)), false, allow).unwrap();
    assert_eq!(group_members(&workspace.layout, 11), [11, 12, 50]);
}

/// A panel that floats keeps its docked place until it docks or closes; floating again from the float keeps the first.
#[test]
fn a_floated_panel_keeps_where_it_was_docked_until_it_docks_or_closes() {
    let mut workspace = Workspace::new(docked());
    let docked_at = workspace.layout.location(&2).unwrap();
    workspace.apply(Action::float(2, rect(10.0, 10.0, 200.0, 200.0)), false, allow).unwrap();
    assert_eq!(workspace.floated, [(2, docked_at.clone())]);
    workspace.apply(Action::float(2, rect(30.0, 30.0, 200.0, 200.0)), false, allow).unwrap();
    assert_eq!(workspace.floated, [(2, docked_at)], "floating again from the float keeps the first record");
    workspace.apply(Action::move_beside(2, 11, DockPlacement::tab(None)), false, allow).unwrap();
    assert!(workspace.floated.is_empty(), "docking drops the record");
    workspace.apply(Action::float(3, rect(10.0, 10.0, 200.0, 200.0)), false, allow).unwrap();
    assert_eq!(workspace.floated.len(), 1);
    workspace.apply(Action::close(3), false, allow).unwrap();
    assert!(workspace.floated.is_empty(), "closing drops the record");
}

/// Canvas 1; an end dock with an expanded column [2] and a collapsed column [3], [4]; a floating window [6].
fn columns() -> Layout<u32> {
    let dock = Node::dock(DockEdge::End, vec![Column::new(vec![g(&[2], 0)]), Column::new(vec![g(&[3], 0), g(&[4], 0)]).with_iconic(true)]);
    Layout::new(Some(Node::split(Axis::X, SplitSize::FixedSecond(300.0), tabs(&[1], 0), dock)), vec![Floating::single(vec![6], 0, window())])
}

fn floats(layout: &Layout<u32>, panel: u32) -> bool {
    matches!(layout.place(&panel), Some(PanelPlace::Floating { .. }))
}

fn column_of(layout: &Layout<u32>, panel: u32) -> &Column<u32> {
    layout.column(&layout.place(&panel).unwrap()).unwrap()
}

/// A floating window's edge: a save without one loads none, so the area takes the edge nearer the rect as before; a
/// move or a float of a collapsed column that sets one keeps and saves it; collapsing the column clears it; an
/// expanded column floated with one keeps none.
#[test]
fn a_floating_windows_edge_is_saved_set_by_collapsed_moves_and_floats_and_cleared_by_collapsing() {
    let saved = r#"{"root": {"Tabs": {"panels": [1], "active": 0}}, "floating": [{"id": 5, "panels": [6], "active": 0, "rect": [100.0, 100.0, 300.0, 400.0]}]}"#;
    let mut layout: Layout<u32> = serde_json::from_str(saved).unwrap();
    assert_eq!(layout.floating[0].edge, None, "a save without an edge keeps to the edge nearer its rect");

    layout.apply(Action::set_iconic(6, true)).unwrap();
    layout.apply(Action::move_floating_with_edge(6, window(), Some(DockEdge::End))).unwrap();
    assert_eq!(layout.floating[0].edge, Some(DockEdge::End), "a collapsed window's move sets its edge");
    let back: Layout<u32> = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
    assert_eq!(back, layout, "the edge is saved");
    layout.apply(Action::set_iconic(6, false)).unwrap();
    assert_eq!(layout.floating[0].edge, Some(DockEdge::End), "expanding keeps the edge the strip hung from");
    layout.apply(Action::set_iconic(6, true)).unwrap();
    assert_eq!(layout.floating[0].edge, None, "collapsing hangs the strip from the edge nearer the window");

    let mut moved = columns();
    moved.apply(Action::move_group(3, Target::float_with_edge(window(), Some(DockEdge::End)))).unwrap();
    let floated = moved.floating.iter().find(|window| window.contains(&3)).unwrap();
    assert_eq!(floated.edge, Some(DockEdge::End), "a collapsed column floats at the edge its strip was dropped near");
    let mut moved = columns();
    moved.apply(Action::move_group(2, Target::float_with_edge(window(), Some(DockEdge::End)))).unwrap();
    let floated = moved.floating.iter().find(|window| window.contains(&2)).unwrap();
    assert_eq!(floated.edge, None, "an expanded column keeps to the edge nearer its rect");
}

/// Collapse to icons is a state of every column: a floating window collapses and stays floating; groups leaving a
/// column float in its state; groups joining a column, docked or floating, take that column's state.
#[test]
fn a_column_keeps_its_collapsed_state_when_it_floats_and_a_joined_column_decides() {
    let mut layout = columns();
    layout.apply(Action::set_iconic(6, true)).unwrap();
    assert!(floats(&layout, 6) && column_of(&layout, 6).iconic);
    assert_eq!(layout.panel_state(&6), PanelState::Open, "a collapsed floating panel is open, not showing");
    layout.apply(Action::set_iconic(6, false)).unwrap();
    assert_eq!(layout.panel_state(&6), PanelState::Front);

    for action in [
        Action::move_column(3, Target::float(window())),
        Action::move_group(3, Target::float(window())),
        Action::move_panel(3, Target::float(window())),
    ] {
        let mut moved = columns();
        moved.apply(action.clone()).unwrap();
        assert!(floats(&moved, 3) && column_of(&moved, 3).iconic, "{action:?} floats as icons");
    }
    let mut moved = columns();
    moved.apply(Action::move_group(2, Target::float(window()))).unwrap();
    assert!(floats(&moved, 2) && !column_of(&moved, 2).iconic, "out of an expanded column it floats expanded");

    layout.apply(Action::move_group(2, Target::at(3, DockPlacement::group(true, None)))).unwrap();
    assert_eq!(layout.panel_state(&2), PanelState::Open, "into a collapsed column it shows as an icon");
    layout.apply(Action::move_group(2, Target::at(6, DockPlacement::group(false, None)))).unwrap();
    assert_eq!(layout.panel_state(&2), PanelState::Front, "into an expanded floating column it shows");

    layout.apply(Action::set_iconic(6, true)).unwrap();
    layout.apply(Action::move_group(3, Target::at(6, DockPlacement::Split(DockSide::Center)))).unwrap();
    assert_eq!(group_members(&layout, 6), [6, 3], "a group dropped on a floating icon group joins its tabs");
    assert_eq!(layout.panel_state(&3), PanelState::Open);
    layout.apply(Action::move_group(4, Target::float(window()))).unwrap();
    assert_eq!(layout.floating.len(), 2);
    layout.apply(Action::move_group(4, Target::at(6, DockPlacement::Split(DockSide::Center)))).unwrap();
    assert_eq!(layout.floating.len(), 1, "a floating icon group merged into another closes its window");
    assert_eq!(group_members(&layout, 6), [6, 4, 3], "it joins after the anchor");
}

/// A floating window is one column: drops above, below and into its groups land, and nothing goes beside it.
#[test]
fn a_floating_window_takes_drops_above_below_and_into_its_groups_and_none_beside() {
    let layout = columns();
    for side in [DockSide::Start, DockSide::End] {
        assert_eq!(DockPlacement::for_zone(&layout, &6, side), Err(DockError::InvalidPlacement), "{side:?}");
    }
    assert_eq!(DockPlacement::for_zone(&layout, &6, DockSide::Bottom), Ok(DockPlacement::group(true, None)));
    assert_eq!(DockPlacement::for_zone(&layout, &6, DockSide::Center), Ok(DockPlacement::Split(DockSide::Center)));
    for placement in [DockPlacement::column(true), DockPlacement::Split(DockSide::End), DockPlacement::Edge(DockSide::Start)] {
        let mut moved = layout.clone();
        assert_eq!(moved.apply(Action::move_group(2, Target::at(6, placement.clone()))), Err(DockError::InvalidPlacement), "{placement:?}");
        assert_eq!(moved, layout);
    }
}

/// A panel that does not collapse refuses a collapse of its column and a move into a collapsed one, so a drag's dry
/// run refuses alike; reconcile expands a saved collapsed column that holds it.
#[test]
fn a_panel_that_does_not_collapse_keeps_its_column_expanded() {
    let rigid = |p: &u32| if *p == 2 { Permissions::ALL.collapses(false) } else { Permissions::ALL };
    let mut layout = columns();
    let before = layout.clone();
    assert_eq!(layout.apply_with_permissions(Action::set_iconic(2, true), rigid), Err(DockError::NotAllowed));
    for to in [Target::at(3, DockPlacement::group(true, None)), Target::at(3, DockPlacement::tab(None))] {
        assert_eq!(layout.apply_with_permissions(Action::move_group(2, to), rigid), Err(DockError::NotAllowed));
    }
    assert_eq!(layout, before);
    layout.apply_with_permissions(Action::set_iconic(3, false), rigid).unwrap();
    layout.apply_with_permissions(Action::move_group(2, Target::at(3, DockPlacement::group(true, None))), rigid).unwrap();

    let mut saved = Workspace::new(columns());
    saved.layout.apply(Action::move_group(2, Target::at(3, DockPlacement::group(true, None)))).unwrap();
    assert!(column_of(&saved.layout, 2).iconic);
    saved.reconcile(|_| true, rigid, &Workspace::new(columns()));
    assert!(!column_of(&saved.layout, 2).iconic, "reconcile expands the column");
    assert_eq!(group_members(&saved.layout, 2), [2]);
}

/// A panel that shares no column (a toolbar) refuses every placement that puts it beside other panels in a column
/// or group, in either direction, and still moves to a new column, a new dock or a float; reconcile gives a saved
/// one a column of its own.
#[test]
fn a_panel_that_shares_no_column_moves_only_to_a_column_of_its_own() {
    let alone = |p: &u32| if *p == 2 { Permissions::ALL.shares_column(false) } else { Permissions::ALL };
    let layout = columns();
    let joins =
        [DockPlacement::tab(None), DockPlacement::Split(DockSide::Center), DockPlacement::group(true, None), DockPlacement::group(false, Some(80.0))];
    for placement in joins {
        for (panel, anchor) in [(2, 3), (3, 2), (2, 6), (6, 2)] {
            let mut moved = layout.clone();
            let action = Action::move_group(panel, Target::at(anchor, placement.clone()));
            assert_eq!(moved.apply_with_permissions(action, alone), Err(DockError::NotAllowed), "{panel} to {anchor}: {placement:?}");
        }
    }
    for to in [Target::at(3, DockPlacement::column(true)), Target::new_dock(DockEdge::Start), Target::float(window())] {
        let mut moved = layout.clone();
        moved.apply_with_permissions(Action::move_group(2, to.clone()), alone).unwrap_or_else(|e| panic!("{to:?}: {e}"));
        assert_eq!(column_of(&moved, 2).panels().count(), 1, "{to:?}");
    }

    let mut saved = Workspace::new(columns());
    saved.layout.apply(Action::move_group(2, Target::at(3, DockPlacement::group(true, None)))).unwrap();
    saved.reconcile(|_| true, alone, &Workspace::new(columns()));
    assert_eq!(column_of(&saved.layout, 2).panels().collect::<Vec<_>>(), [&2], "reconcile gives it a column of its own");
    assert_eq!(sorted(&saved.layout), [1, 2, 3, 4, 6]);
}

/// A panel that docks only in dock columns (a toolbar, photocraft#12517) refuses every placement that leaves it in a
/// frame of the docked tree: a split beside the canvas on any side and an edge of the docked area, so it never lies
/// across the window. A new dock, a column beside another and a float still take it, other panels still move while
/// one sits in a frame (only a new breach is refused), and reconcile moves a saved one into a dock column.
#[test]
fn a_panel_that_docks_only_in_columns_never_lands_in_a_frame() {
    let strip = |p: &u32| match p {
        2 => Permissions::ALL.shares_column(false).docks_outside_columns(false),
        1 => Permissions::PROTECTED.accepts_splits(true),
        _ => Permissions::ALL,
    };
    let layout = columns();
    let sides = [DockSide::Top, DockSide::Bottom, DockSide::Start, DockSide::End];
    let framed = sides.iter().map(|side| DockPlacement::Split(*side)).chain(sides.iter().map(|side| DockPlacement::Edge(*side)));
    for placement in framed {
        let mut moved = layout.clone();
        let action = Action::move_column(2, Target::at(1, placement.clone()));
        assert_eq!(moved.apply_with_permissions(action, strip), Err(DockError::NotAllowed), "{placement:?}");
        assert_eq!(moved, layout, "{placement:?}");
    }
    for to in [Target::new_dock(DockEdge::Start), Target::at(3, DockPlacement::column(true)), Target::float(window())] {
        let mut moved = layout.clone();
        moved.apply_with_permissions(Action::move_column(2, to.clone()), strip).unwrap_or_else(|e| panic!("{to:?}: {e}"));
        assert!(!matches!(moved.place(&2), Some(PanelPlace::Node(_))), "{to:?}");
    }
    // Without the right the same split takes it, so the refusal above is this right's.
    let mut split = layout.clone();
    split
        .apply_with_permissions(Action::move_column(2, Target::at(1, DockPlacement::Split(DockSide::Bottom))), |p: &u32| match p {
            2 => Permissions::ALL.shares_column(false),
            _ => Permissions::ALL,
        })
        .unwrap();
    assert!(matches!(split.place(&2), Some(PanelPlace::Node(_))));

    // A layout saved with the panel in a frame: other panels still move, and reconcile docks it in a column.
    let mut other = split.clone();
    other.apply_with_permissions(Action::move_group(6, Target::at(3, DockPlacement::column(false))), strip).unwrap();
    let mut saved = Workspace::new(split);
    saved.reconcile(|_| true, strip, &Workspace::new(columns()));
    assert!(matches!(saved.layout.place(&2), Some(PanelPlace::Dock { .. })), "reconcile docks it in a column: {:?}", saved.layout);
    assert_eq!(column_of(&saved.layout, 2).panels().collect::<Vec<_>>(), [&2]);
    assert_eq!(sorted(&saved.layout), [1, 2, 3, 4, 6]);
}

/// A stepped column (a toolbar's) keeps its step count docked, floating and through JSON, and its width moves only
/// by steps.
#[test]
fn a_stepped_column_keeps_its_step_count_and_refuses_a_width() {
    let mut layout = columns();
    assert_eq!(layout.apply(Action::set_column_steps(2, 0)), Err(DockError::InvalidGeometry));
    layout.apply(Action::set_column_steps(2, 2)).unwrap();
    assert_eq!(column_of(&layout, 2).steps, Some(2));
    let before = layout.clone();
    assert_eq!(layout.apply(Action::resize_column(2, 200.0)), Err(DockError::InvalidPlacement));
    assert_eq!(layout, before);
    let saved: Layout<u32> = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
    assert_eq!(saved, layout);

    layout.apply(Action::move_column(2, Target::float(window()))).unwrap();
    assert!(floats(&layout, 2));
    assert_eq!(column_of(&layout, 2).steps, Some(2), "a floated stepped column keeps its steps");
    let saved: Layout<u32> = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
    assert_eq!(saved, layout);
}
