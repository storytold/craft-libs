use super::dock_model::{Place, node_at};
use super::*;
use crate::layout::{SplitAxis, SplitSize};

pub(super) fn g(panels: &[u32], active: usize) -> Group<u32> {
    Group::new(panels.to_vec(), active)
}

fn tabs(panels: &[u32], active: usize) -> Node<u32> {
    Node::Tabs { panels: panels.to_vec(), active }
}

fn hsplit(size: SplitSize, first: Node<u32>, second: Node<u32>) -> Node<u32> {
    Node::Split { axis: SplitAxis::Horizontal, size, first: Box::new(first), second: Box::new(second) }
}

/// Canvas 1 above frame tabs [11, 12]; an end dock with column A ([2, 3], [4]) and column B
/// ([5]); a floating window with groups [6, 7] and [8].
pub(super) fn docked() -> Layout<u32> {
    let middle =
        Node::Split { axis: SplitAxis::Vertical, size: SplitSize::Ratio(0.7), first: Box::new(tabs(&[1], 0)), second: Box::new(tabs(&[11, 12], 1)) };
    let dock =
        Node::Dock { side: Side::End, columns: vec![Column::new(vec![g(&[2, 3], 1), g(&[4], 0)], 180.0), Column::new(vec![g(&[5], 0)], 120.0)] };
    Layout {
        root: Some(hsplit(SplitSize::FixedSecond(300.0), middle, dock)),
        floating: vec![Floating::stack(vec![g(&[6, 7], 1), g(&[8], 0)], [100.0, 100.0, 300.0, 400.0]).unwrap()],
    }
}

pub(super) const ALL: [u32; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 11, 12];

fn sorted(layout: &Layout<u32>) -> Vec<u32> {
    let mut panels: Vec<u32> = layout.panels().into_iter().copied().collect();
    panels.sort_unstable();
    panels
}

/// Members, active panel and minimized state of the group holding `panel`.
fn group_state(layout: &Layout<u32>, panel: u32) -> (Vec<u32>, u32, bool) {
    match layout.place(&panel).unwrap() {
        place @ (Place::Node(_) | Place::Floating { .. }) => match layout.node_of(&place).unwrap() {
            Node::Tabs { panels, active } => (panels.clone(), panels[*active], false),
            other => panic!("{other:?}"),
        },
        place => {
            let group = layout.group(&place).unwrap();
            (group.panels.clone(), group.panels[group.active], group.minimized)
        }
    }
}

fn dock(layout: &Layout<u32>, side: Side) -> &Vec<Column<u32>> {
    let path = layout.dock_path(side).unwrap();
    let Some(Node::Dock { columns, .. }) = node_at(layout.root.as_ref().unwrap(), &path) else { panic!() };
    columns
}

fn panels_of(groups: &[Group<u32>]) -> Vec<Vec<u32>> {
    groups.iter().map(|g| g.panels.clone()).collect()
}

#[test]
fn group_moves_keep_order_active_tab_and_minimized_state() {
    let targets = [
        Target::At { anchor: 5, placement: Placement::Group { below: true, height: None } },
        Target::At { anchor: 5, placement: Placement::Column { after: true } },
        Target::At { anchor: 8, placement: Placement::Group { below: false, height: None } },
        Target::NewDock { side: Side::Start },
        Target::Float { rect: Some([20.0, 20.0, 200.0, 300.0]) },
        Target::Float { rect: None },
    ];
    // A frame Tabs node, a column group and a floating group.
    for source in [11, 2, 6] {
        for to in &targets {
            let mut layout = docked();
            if source != 11 {
                layout.apply(Action::SetMinimized { panel: source, minimized: true }).unwrap();
            }
            let before = group_state(&layout, source);
            layout.apply(Action::MoveGroup { panel: source, to: to.clone() }).unwrap_or_else(|e| panic!("{source} {to:?}: {e}"));
            let after = group_state(&layout, source);
            assert_eq!((&after.0, after.1), (&before.0, before.1), "{source} {to:?}");
            // A group keeps its minimized state from dock to dock; a floating window's tree
            // has no minimized groups.
            if matches!((source, layout.place(&source)), (2, Some(Place::Dock { .. }))) {
                assert_eq!(after.2, before.2, "{source} {to:?}");
            }
            assert_eq!(sorted(&layout), ALL);
        }
    }
    // Into a frame split the group becomes a Tabs node with the same order and selection.
    let mut layout = docked();
    layout.apply(Action::MoveGroup { panel: 6, to: Target::At { anchor: 1, placement: Placement::Split(Zone::Left) } }).unwrap();
    assert_eq!(group_state(&layout, 6), (vec![6, 7], 7, false));
    assert_eq!(layout.floating[0].groups().len(), 1);
}

#[test]
fn group_merge_into_tabs_inserts_every_member_before_the_anchor_tab() {
    let mut layout = docked();
    layout.apply(Action::MoveGroup { panel: 6, to: Target::At { anchor: 2, placement: Placement::Tab { before: Some(3) } } }).unwrap();
    assert_eq!(group_state(&layout, 2), (vec![2, 6, 7, 3], 7, false));
    assert_eq!(panels_of(&layout.floating[0].groups()), [vec![8]]);

    let mut layout = docked();
    layout.apply(Action::MoveGroup { panel: 2, to: Target::At { anchor: 11, placement: Placement::Tab { before: Some(12) } } }).unwrap();
    assert_eq!(group_state(&layout, 11), (vec![11, 2, 3, 12], 3, false));
    assert_eq!(panels_of(&dock(&layout, Side::End)[0].groups), [vec![4]]);

    // A target inside the moving group is a no-op; a protected anchor refuses tabs.
    let before = docked();
    let mut layout = before.clone();
    layout.apply(Action::MoveGroup { panel: 2, to: Target::At { anchor: 3, placement: Placement::Tab { before: None } } }).unwrap();
    assert_eq!(layout, before);
    let protect = |p: &u32| if *p == 1 { Permissions::PROTECTED } else { Permissions::default() };
    let to = Target::At { anchor: 1, placement: Placement::Tab { before: None } };
    assert_eq!(layout.apply_with_permissions(Action::MoveGroup { panel: 2, to }, protect), Err(DockError::NotAllowed));
    assert_eq!(layout, before);
}

#[test]
fn column_moves_and_floats_keep_every_group() {
    let mut layout = docked();
    layout.apply(Action::SetIconic { panel: 4, iconic: true }).unwrap();
    layout.apply(Action::MoveColumn { panel: 2, to: Target::NewDock { side: Side::Start } }).unwrap();
    let Some(Node::Split { size, first, .. }) = &layout.root else { panic!() };
    assert_eq!(*size, SplitSize::FixedFirst(180.0));
    assert!(matches!(**first, Node::Dock { side: Side::Start, .. }));
    let start = dock(&layout, Side::Start);
    assert_eq!(start.len(), 1);
    assert_eq!(panels_of(&start[0].groups), [vec![2, 3], vec![4]]);
    assert_eq!((start[0].width, start[0].iconic, start[0].groups[0].active), (180.0, true, 1));
    assert_eq!(panels_of(&dock(&layout, Side::End)[0].groups), [vec![5]]);

    // The last column leaves the end dock, which collapses.
    layout.apply(Action::MoveColumn { panel: 5, to: Target::At { anchor: 2, placement: Placement::Column { after: false } } }).unwrap();
    assert!(layout.dock_path(Side::End).is_none());
    let start = dock(&layout, Side::Start);
    assert_eq!(start.iter().map(|c| c.width).collect::<Vec<_>>(), [120.0, 180.0]);

    layout.apply(Action::MoveColumn { panel: 3, to: Target::Float { rect: None } }).unwrap();
    let window = layout.floating.last().unwrap();
    assert_eq!(panels_of(&window.groups()), [vec![2, 3], vec![4]]);
    assert_eq!(window.rect[2], 180.0);
    assert_eq!(sorted(&layout), ALL);
    assert_eq!(layout.apply(Action::MoveColumn { panel: 11, to: Target::Float { rect: None } }), Err(DockError::InvalidPlacement));
}

#[test]
fn window_docks_as_stacked_groups_or_merges_as_tabs() {
    let mut layout = docked();
    layout.apply(Action::MoveWindow { panel: 8, to: Target::At { anchor: 4, placement: Placement::Group { below: true, height: None } } }).unwrap();
    assert_eq!(panels_of(&dock(&layout, Side::End)[0].groups), [vec![2, 3], vec![4], vec![6, 7], vec![8]]);
    assert!(layout.floating.is_empty());

    let mut layout = docked();
    layout.apply(Action::MoveWindow { panel: 8, to: Target::At { anchor: 11, placement: Placement::Tab { before: None } } }).unwrap();
    assert_eq!(group_state(&layout, 11), (vec![11, 12, 6, 7, 8], 7, false));

    let mut layout = docked();
    layout.apply(Action::MoveWindow { panel: 6, to: Target::NewDock { side: Side::Start } }).unwrap();
    let start = dock(&layout, Side::Start);
    assert_eq!((panels_of(&start[0].groups), start[0].width), (vec![vec![6, 7], vec![8]], 300.0));

    let before = docked();
    let mut layout = before.clone();
    layout.apply(Action::MoveWindow { panel: 7, to: Target::At { anchor: 8, placement: Placement::Tab { before: None } } }).unwrap();
    assert_eq!(layout, before);
    assert_eq!(layout.apply(Action::MoveWindow { panel: 2, to: Target::Float { rect: None } }), Err(DockError::InvalidPlacement));
}

#[test]
fn close_group_and_close_window_record_a_restorable_location_per_member() {
    let mut workspace = Workspace::new(docked());
    let allow = |_: &u32| Permissions::default();
    assert_eq!(workspace.apply(Action::CloseGroup { panel: 3 }, allow).unwrap(), [2, 3]);
    assert_eq!(workspace.hidden.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(panels_of(&dock(&workspace.layout, Side::End)[0].groups), [vec![4]]);
    workspace.set_visible(2, true, None, |_| Permissions::default()).unwrap();
    workspace.set_visible(3, true, None, |_| Permissions::default()).unwrap();
    assert_eq!(panels_of(&dock(&workspace.layout, Side::End)[0].groups), [vec![2, 3], vec![4]]);

    assert_eq!(workspace.layout.closed_by(&Action::CloseWindow { panel: 7 }).unwrap(), [6, 7, 8]);
    assert_eq!(workspace.apply(Action::CloseWindow { panel: 7 }, allow).unwrap(), [6, 7, 8]);
    assert!(workspace.layout.floating.is_empty());
    assert_eq!(workspace.hidden.len(), 3);
    for panel in [6, 7, 8] {
        workspace.set_visible(panel, true, None, |_| Permissions::default()).unwrap();
    }
    assert_eq!(workspace.layout.floating.len(), 1);
    assert_eq!(panels_of(&workspace.layout.floating[0].groups()), [vec![6, 7], vec![8]]);
    assert_eq!(workspace.layout.floating[0].rect, [100.0, 100.0, 300.0, 400.0]);

    workspace.apply(Action::CloseOthers { panel: 12 }, allow).unwrap();
    assert_eq!(group_state(&workspace.layout, 12).0, [12]);
    workspace.set_visible(11, true, None, |_| Permissions::default()).unwrap();
    assert_eq!(group_state(&workspace.layout, 12).0, [11, 12]);
    assert!(workspace.hidden.is_empty());
    assert_eq!(sorted(&workspace.layout), ALL);

    // A member that may not close keeps the whole group open.
    let keep = |p: &u32| if *p == 4 { Permissions { closable: false, ..Permissions::default() } } else { Permissions::default() };
    let before = workspace.clone();
    assert_eq!(workspace.apply(Action::CloseGroup { panel: 4 }, keep), Err(DockError::NotAllowed));
    assert_eq!(workspace, before);
}

#[test]
fn panels_closed_one_by_one_reopen_into_one_group() {
    let allow = |_: &u32| Permissions::default();
    let rect = [100.0, 100.0, 300.0, 400.0];
    let cycle = |workspace: &mut Workspace<u32>, close: &[u32], reopen: &[u32]| {
        for &panel in close {
            workspace.set_visible(panel, false, None, allow).unwrap();
        }
        for &panel in reopen {
            workspace.set_visible(panel, true, None, allow).unwrap_or_else(|e| panic!("{close:?} {reopen:?} {panel}: {e}"));
        }
        assert!(workspace.hidden.is_empty());
    };
    let orders2 = [[2, 3], [3, 2]];
    for close in orders2 {
        for reopen in orders2 {
            let mut workspace = Workspace::new(docked());
            cycle(&mut workspace, &close, &reopen);
            let columns = dock(&workspace.layout, Side::End);
            assert_eq!(
                columns.iter().map(|c| panels_of(&c.groups)).collect::<Vec<_>>(),
                [vec![vec![2, 3], vec![4]], vec![vec![5]]],
                "{close:?} {reopen:?}"
            );
        }
    }
    let orders2 = [[10, 20], [20, 10]];
    for close in orders2 {
        for reopen in orders2 {
            let mut workspace = Workspace::new(Layout { root: Some(tabs(&[1], 0)), floating: vec![Floating::single(vec![10, 20], 0, rect)] });
            cycle(&mut workspace, &close, &reopen);
            let [window] = &workspace.layout.floating[..] else { panic!("{close:?} {reopen:?}: {:?}", workspace.layout.floating) };
            assert_eq!((window.panels().copied().collect::<Vec<_>>(), window.rect), (vec![10, 20], rect), "{close:?} {reopen:?}");
        }
    }
    // Three tabs: the locations do not always record the order, only the window.
    let orders3 = [[10, 20, 30], [10, 30, 20], [20, 10, 30], [20, 30, 10], [30, 10, 20], [30, 20, 10]];
    for close in orders3 {
        for reopen in orders3 {
            let mut workspace = Workspace::new(Layout { root: Some(tabs(&[1], 0)), floating: vec![Floating::single(vec![10, 20, 30], 0, rect)] });
            cycle(&mut workspace, &close, &reopen);
            let [window] = &workspace.layout.floating[..] else { panic!("{close:?} {reopen:?}: {:?}", workspace.layout.floating) };
            let mut panels: Vec<u32> = window.panels().copied().collect();
            panels.sort_unstable();
            assert_eq!(panels, [10, 20, 30], "{close:?} {reopen:?}");
        }
    }
}

#[test]
fn restore_into_a_column_group_seam_and_new_column_round_trips() {
    let column_a = Column::new(vec![g(&[2, 3], 1), Group { minimized: true, height: Some(150.0), ..g(&[4], 0) }, g(&[9], 0)], 180.0);
    let column_b = Column { iconic: true, iconic_width: Some(48.0), ..Column::new(vec![g(&[5], 0)], 120.0) };
    let end = Node::Dock { side: Side::End, columns: vec![column_a, column_b] };
    let start = Node::Dock { side: Side::Start, columns: vec![Column::new(vec![g(&[10], 0)], 200.0)] };
    let original = Layout {
        root: Some(hsplit(SplitSize::FixedFirst(200.0), start, hsplit(SplitSize::FixedSecond(300.0), tabs(&[1], 0), end))),
        floating: vec![Floating::stack(vec![g(&[6], 0), Group { height: Some(90.0), ..g(&[7], 0) }], [10.0, 10.0, 200.0, 300.0]).unwrap()],
    };
    original.validate().unwrap();
    // 4: the seam between [2, 3] and [9]; 9: below [4]; 5: a new column after A; 10: alone in
    // its dock; 3: a tab; 7: below a floating group.
    for panel in [4, 9, 5, 10, 3, 7] {
        let mut layout = original.clone();
        let location = layout.location(&panel).unwrap();
        #[cfg(feature = "serde")]
        let location: Location<u32> = serde_json::from_str(&serde_json::to_string(&location).unwrap()).unwrap();
        layout.apply(Action::Close { panel }).unwrap();
        layout.restore(panel, &location).unwrap_or_else(|e| panic!("{panel}: {e} {location:?}"));
        assert_eq!(layout, original, "{panel}: {location:?}");
    }
}

#[test]
fn dock_validation_rejects_two_docks_per_side_nested_docks_empty_columns_and_bad_widths() {
    docked().validate().unwrap();
    let empty_dock = |side| Node::Dock { side, columns: Vec::new() };
    Layout { root: Some(hsplit(SplitSize::FixedFirst(10.0), empty_dock(Side::Start), tabs(&[1], 0))), floating: Vec::new() }.validate().unwrap();

    let two = hsplit(SplitSize::Ratio(0.5), empty_dock(Side::Start), empty_dock(Side::Start));
    let nested = Node::Split {
        axis: SplitAxis::Vertical,
        size: SplitSize::Ratio(0.5),
        first: Box::new(empty_dock(Side::Start)),
        second: Box::new(tabs(&[1], 0)),
    };
    let wrong_side = hsplit(SplitSize::Ratio(0.5), empty_dock(Side::End), tabs(&[1], 0));
    for root in [two, nested, wrong_side] {
        assert_eq!(Layout { root: Some(root), floating: Vec::new() }.validate(), Err(DockError::InvalidPlacement));
    }

    let with_column = |column: Column<u32>| Layout { root: Some(Node::Dock { side: Side::End, columns: vec![column] }), floating: Vec::new() };
    let bad = [
        Column::new(Vec::new(), 100.0),
        Column::new(vec![g(&[], 0)], 100.0),
        Column::new(vec![g(&[2], 1)], 100.0),
        Column::new(vec![Group { height: Some(f32::NAN), ..g(&[2], 0) }], 100.0),
        Column::new(vec![g(&[2], 0)], 0.0),
        Column::new(vec![g(&[2], 0)], -5.0),
        Column::new(vec![g(&[2], 0)], f32::INFINITY),
        Column::new(vec![g(&[2], 0)], 2.0e6),
        Column { iconic_width: Some(f32::NAN), ..Column::new(vec![g(&[2], 0)], 100.0) },
    ];
    for column in bad {
        assert!(with_column(column.clone()).validate().is_err(), "{column:?}");
    }

    let mut duplicate = docked();
    duplicate.floating[0].root = Node::Tabs { panels: vec![6, 7, 2], active: 0 };
    assert_eq!(duplicate.validate(), Err(DockError::DuplicatePanel));
    let mut empty_window = docked();
    empty_window.floating[0].root = Node::Tabs { panels: Vec::new(), active: 0 };
    assert_eq!(empty_window.validate(), Err(DockError::InvalidGeometry));
    let mut docked_window = docked();
    docked_window.floating[0].root = Node::Dock { side: Side::End, columns: vec![Column::new(vec![g(&[6, 7], 0)], 100.0)] };
    assert_eq!(docked_window.validate(), Err(DockError::InvalidPlacement));
    let mut native = docked();
    native.floating[0].native = Some([0.0, 0.0, -1.0, 10.0]);
    assert_eq!(native.validate(), Err(DockError::InvalidGeometry));
    let mut monitor = docked();
    monitor.floating[0].monitor = Some(super::Monitor { name: None, rect: [0.0, 0.0, 1920.0, 1080.0], scale: f32::NAN });
    assert_eq!(monitor.validate(), Err(DockError::InvalidGeometry));
    let mut monitor_size = docked();
    monitor_size.floating[0].monitor_size = Some([f32::NAN, 1080.0]);
    assert_eq!(monitor_size.validate(), Err(DockError::InvalidGeometry));
    let mut layout = docked();
    assert_eq!(layout.apply(Action::ResizeColumn { panel: 2, width: f32::NAN }), Err(DockError::InvalidGeometry));
    assert_eq!(layout.apply(Action::ResizeColumn { panel: 11, width: 50.0 }), Err(DockError::InvalidPlacement));
    assert_eq!(layout, docked());
}

#[test]
fn a_locked_workspace_records_its_native_windows() {
    let mut workspace = Workspace::new(docked());
    workspace.locked = true;
    for rect in [[0.0, 0.0, 300.0, 400.0], [40.0, 30.0, 300.0, 400.0]] {
        workspace.apply(Action::SetNative { panel: 6, rect: Some(rect) }, |_| Permissions::default()).unwrap();
        assert_eq!(workspace.layout.floating[0].native, Some(rect));
    }
}

#[test]
fn locked_workspace_rejects_structure_but_allows_activate_open_minimize_and_resize() {
    let mut workspace = Workspace::new(docked());
    let allow = |_: &u32| Permissions::default();
    workspace.apply(Action::Close { panel: 12 }, allow).unwrap();
    workspace.locked = true;
    let before = workspace.clone();
    let refused = [
        Action::Move { panel: 2, anchor: 5, placement: Placement::Tab { before: None } },
        Action::MoveGroup { panel: 2, to: Target::Float { rect: None } },
        Action::MoveColumn { panel: 5, to: Target::NewDock { side: Side::Start } },
        Action::MoveWindow { panel: 6, to: Target::At { anchor: 4, placement: Placement::Group { below: true, height: None } } },
        Action::Float { panel: 4, rect: [0.0, 0.0, 100.0, 100.0] },
        Action::MoveFloating { panel: 6, rect: [0.0, 0.0, 100.0, 100.0] },
        Action::Close { panel: 4 },
        Action::CloseGroup { panel: 4 },
        Action::CloseWindow { panel: 6 },
    ];
    for action in refused {
        assert_eq!(workspace.apply(action.clone(), allow), Err(DockError::NotAllowed), "{action:?}");
        assert_eq!(workspace, before);
    }
    assert_eq!(workspace.set_visible(4, false, None, |_| Permissions::default()), Err(DockError::NotAllowed));
    let allowed = [
        Action::Activate { panel: 2 },
        Action::Open { panel: 12, anchor: Some(11) },
        Action::SetMinimized { panel: 4, minimized: true },
        Action::SetIconic { panel: 5, iconic: true },
        Action::ResizeColumn { panel: 5, width: 160.0 },
        Action::ResizeGroup { panel: 4, height: Some(120.0) },
        Action::ResizeIconic { panel: 5, width: Some(60.0) },
        Action::ResizeSplit { path: Vec::new(), size: SplitSize::FixedSecond(320.0) },
    ];
    workspace.apply_all(allowed, allow).unwrap();
    assert!(workspace.hidden.is_empty());
    assert_eq!(workspace.layout.panel_state(&12), PanelState::Front);
    assert_eq!(workspace.layout.panel_state(&4), PanelState::Open);
    assert_eq!(workspace.layout.panel_state(&5), PanelState::Open);
    assert_eq!(workspace.layout.panel_state(&2), PanelState::Front);
    assert_eq!(workspace.layout.panel_state(&3), PanelState::Open);
    // Opening at a chosen place works too; moving a shown panel there does not.
    workspace.apply(Action::OpenAt { panel: 13, anchor: 11, placement: Placement::Split(Zone::Bottom) }, allow).unwrap();
    assert!(workspace.layout.contains(&13));
    assert_eq!(
        workspace.apply(Action::OpenAt { panel: 2, anchor: 11, placement: Placement::Split(Zone::Bottom) }, allow),
        Err(DockError::NotAllowed)
    );
}

#[test]
fn hiding_a_panel_that_may_not_close_is_refused() {
    let mut workspace = Workspace::new(docked());
    let canvas = |p: &u32| if *p == 1 { Permissions { accepts_splits: true, ..Permissions::PROTECTED } } else { Permissions::default() };
    assert_eq!(workspace.set_visible(1, false, None, canvas), Err(DockError::NotAllowed));
    assert!(workspace.layout.contains(&1) && workspace.hidden.is_empty());
    workspace.set_visible(2, false, None, canvas).unwrap();
    assert!(!workspace.layout.contains(&2));
}

#[test]
fn reconcile_drops_unknown_panels_and_places_missing_registered_ones() {
    let default = Workspace::new(docked());
    let mut workspace = default.clone();
    workspace.layout.apply(Action::Close { panel: 5 }).unwrap();
    workspace.layout.apply(Action::Close { panel: 12 }).unwrap();
    workspace.layout.apply(Action::OpenAt { panel: 99, anchor: 2, placement: Placement::Tab { before: None } }).unwrap();
    let stray = workspace.layout.location(&99).unwrap();
    workspace.hidden.push((98, stray));
    let registered = |p: &u32| *p < 90;
    workspace.reconcile(registered, |_| Permissions::default(), &default);
    workspace.validate().unwrap();
    // Unregistered panels wait in `hidden` and come back once registered.
    assert_eq!(workspace.hidden.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [98, 99]);
    assert_eq!(sorted(&workspace.layout), ALL);
    workspace.set_visible(99, true, None, |_| Permissions::default()).unwrap();
    assert_eq!(group_state(&workspace.layout, 2).0, [2, 3, 99]);
    assert_eq!(dock(&workspace.layout, Side::End).len(), 2);
    assert_eq!(group_state(&workspace.layout, 11).0, [11, 12]);

    let mut broken = default.clone();
    broken.layout.floating[0].rect[2] = -1.0;
    broken.reconcile(registered, |_| Permissions::default(), &default);
    assert_eq!(broken.layout, default.layout);
}

#[test]
fn reset_returns_to_saved_or_default() {
    let default = docked();
    let mut workspace = Workspace::new(default.clone());
    workspace.apply(Action::MoveGroup { panel: 6, to: Target::NewDock { side: Side::Start } }, |_| Permissions::default()).unwrap();
    workspace.reset(&default);
    assert_eq!(workspace.layout, default);

    let mut saved = default.clone();
    saved.apply(Action::Close { panel: 8 }).unwrap();
    saved.apply(Action::MoveGroup { panel: 11, to: Target::Float { rect: None } }).unwrap();
    workspace.saved = Some(saved.clone());
    workspace.reset(&default);
    assert_eq!(workspace.layout, saved);
    assert_eq!(workspace.hidden.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [8]);
    workspace.validate().unwrap();
}

#[cfg(feature = "serde")]
mod serde_tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn a_saved_stack_of_groups_loads_as_a_vertical_split_by_their_heights() {
        let saved = json!({
            "groups": [{"panels": [4], "active": 0, "height": 100.0}, {"panels": [5, 6], "active": 1, "height": 300.0}],
            "rect": [1.0, 2.0, 300.0, 400.0],
        });
        let window: Floating<u32> = serde_json::from_value(saved).unwrap();
        let expected = Node::Split {
            axis: SplitAxis::Vertical,
            size: SplitSize::Ratio(0.25),
            first: Box::new(Node::Tabs { panels: vec![4], active: 0 }),
            second: Box::new(Node::Tabs { panels: vec![5, 6], active: 1 }),
        };
        assert_eq!((window.root, window.minimized), (expected, false));
        let all_minimized = json!({"groups": [{"panels": [4], "active": 0, "minimized": true}], "rect": [1.0, 2.0, 300.0, 400.0]});
        let window: Floating<u32> = serde_json::from_value(all_minimized).unwrap();
        assert_eq!((window.root, window.minimized), (Node::Tabs { panels: vec![4], active: 0 }, true));
    }

    #[test]
    fn single_plain_floating_window_writes_the_v1_shape() {
        let rect = [1.0, 2.0, 300.0, 200.0];
        let plain = Floating::single(vec![4u32, 5], 1, rect);
        assert_eq!(serde_json::to_value(&plain).unwrap(), json!({"panels": [4, 5], "active": 1, "rect": rect}));
        let minimized = Floating { minimized: true, ..Floating::single(vec![4u32], 0, rect) };
        let stacked = Floating::stack(vec![g(&[4], 0), g(&[5], 0)], rect).unwrap();
        let native = Floating { native: Some(rect), ..Floating::single(vec![4u32], 0, rect) };
        for window in [minimized, stacked, native] {
            let value = serde_json::to_value(&window).unwrap();
            assert!(value.get("root").is_some() && value.get("panels").is_none(), "{value}");
            assert_eq!(serde_json::from_value::<Floating<u32>>(value).unwrap(), window);
        }
        assert!(serde_json::from_value::<Floating<u32>>(json!({"rect": rect})).is_err());
    }

    #[test]
    fn v2_round_trips() {
        let mut workspace = Workspace::new(docked());
        workspace.apply(Action::CloseGroup { panel: 4 }, |_| Permissions::default()).unwrap();
        workspace.apply(Action::SetNative { panel: 8, rect: Some([5.0, 5.0, 400.0, 300.0]) }, |_| Permissions::default()).unwrap();
        workspace.saved = Some(docked());
        workspace.locked = true;
        let text = serde_json::to_string(&workspace).unwrap();
        let parsed: Workspace<u32> = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, workspace);
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["version"], json!(2));

        let mut old = parsed.clone();
        old.version = 1;
        let mut value = serde_json::to_value(&old).unwrap();
        value.as_object_mut().unwrap().remove("version");
        assert_eq!(serde_json::from_value::<Workspace<u32>>(value).unwrap().version, 1);
        old.version = 3;
        assert_eq!(old.validate(), Err(DockError::UnsupportedVersion));
    }

    #[derive(Clone, Copy, Debug)]
    enum Kind {
        Layout,
        Workspace,
        Hidden,
    }

    /// App, file contents, and the JSON pointers to its docking values.
    type Fixture = (&'static str, &'static str, &'static [(&'static str, Kind)]);

    #[test]
    fn v1_layout_json_parses_unchanged() {
        use Kind::*;
        let fixtures: [Fixture; 7] = [
            (
                "designcraft",
                include_str!("../../tests/fixtures/v1/designcraft.json"),
                &[
                    ("/docking", Layout),
                    ("/dockingHidden", Hidden),
                    ("/customWorkspaces/0/docking", Layout),
                    ("/customWorkspaces/0/dockingHidden", Hidden),
                ],
            ),
            ("vectorcraft", include_str!("../../tests/fixtures/v1/vectorcraft.json"), &[("/docking", Layout), ("/docking_hidden", Hidden)]),
            ("lightcraft", include_str!("../../tests/fixtures/v1/lightcraft.json"), &[("/docking", Workspace)]),
            ("pdfcraft", include_str!("../../tests/fixtures/v1/pdfcraft.json"), &[("/docking", Workspace)]),
            ("cadcraft", include_str!("../../tests/fixtures/v1/cadcraft.json"), &[("/workspace/docking", Workspace)]),
            ("soundcraft", include_str!("../../tests/fixtures/v1/soundcraft.json"), &[("/docking", Workspace), ("/mix_docking", Workspace)]),
            (
                "photocraft",
                include_str!("../../tests/fixtures/v1/photocraft.json"),
                &[("/docking", Layout), ("/docking_hidden", Hidden), ("/panel_layout/docking", Layout), ("/panel_layout/dockingHidden", Hidden)],
            ),
        ];
        for (app, text, fields) in fixtures {
            let file: Value = serde_json::from_str(text).unwrap();
            for (pointer, kind) in fields {
                let value = file.pointer(pointer).unwrap_or_else(|| panic!("{app}{pointer}")).clone();
                let layout_value = match kind {
                    Layout => value.clone(),
                    Workspace => value["layout"].clone(),
                    Hidden => {
                        let hidden: std::collections::BTreeMap<String, Location<String>> = serde_json::from_value(value).unwrap();
                        assert!(!hidden.is_empty(), "{app}{pointer}");
                        continue;
                    }
                };
                if let Workspace = kind {
                    let workspace: super::Workspace<String> = serde_json::from_value(value).unwrap_or_else(|e| panic!("{app}{pointer}: {e}"));
                    workspace.validate().unwrap_or_else(|e| panic!("{app}{pointer}: {e}"));
                    assert_eq!(workspace.version, 1);
                    for (panel, location) in &workspace.hidden {
                        let mut layout = workspace.layout.clone();
                        layout.restore(panel.clone(), location).unwrap_or_else(|e| panic!("{app}{pointer} {panel}: {e}"));
                    }
                }
                let layout: super::Layout<String> = serde_json::from_value(layout_value.clone()).unwrap_or_else(|e| panic!("{app}{pointer}: {e}"));
                layout.validate().unwrap_or_else(|e| panic!("{app}{pointer}: {e}"));
                // Nothing new is in use, so the layout writes back exactly as read.
                assert_eq!(serde_json::to_value(&layout).unwrap(), layout_value, "{app}{pointer}");
            }
        }
    }

    #[test]
    fn every_app_fixture_reconciles_with_its_canvas_shown() {
        let s = |p: &str| p.to_owned();
        let permissions = |p: &String| if p == "canvas" { Permissions::PROTECTED } else { Permissions::default() };
        // The canvas between a start dock and the properties, which no fixture matches exactly.
        let beside = super::Layout {
            root: Some(Node::Split {
                axis: SplitAxis::Horizontal,
                size: SplitSize::FixedFirst(240.0),
                first: Box::new(Node::Dock {
                    side: Side::Start,
                    columns: vec![Column::new(vec![Group::new(vec![s("layers"), s("pages")], 0)], 240.0)],
                }),
                second: Box::new(Node::Split {
                    axis: SplitAxis::Horizontal,
                    size: SplitSize::Ratio(0.75),
                    first: Box::new(Node::Tabs { panels: vec![s("canvas")], active: 0 }),
                    second: Box::new(Node::Tabs { panels: vec![s("properties"), s("stroke")], active: 0 }),
                }),
            }),
            floating: Vec::new(),
        };
        // The canvas with no anchor to go back beside.
        let alone = super::Layout { root: Some(Node::Tabs { panels: vec![s("canvas")], active: 0 }), floating: Vec::new() };
        let fixtures = [
            ("designcraft", include_str!("../../tests/fixtures/v1/designcraft.json"), "/docking"),
            ("designcraft", include_str!("../../tests/fixtures/v1/designcraft.json"), "/customWorkspaces/0/docking"),
            ("vectorcraft", include_str!("../../tests/fixtures/v1/vectorcraft.json"), "/docking"),
            ("lightcraft", include_str!("../../tests/fixtures/v1/lightcraft.json"), "/docking/layout"),
            ("pdfcraft", include_str!("../../tests/fixtures/v1/pdfcraft.json"), "/docking/layout"),
            ("cadcraft", include_str!("../../tests/fixtures/v1/cadcraft.json"), "/workspace/docking/layout"),
            ("soundcraft", include_str!("../../tests/fixtures/v1/soundcraft.json"), "/docking/layout"),
            ("soundcraft", include_str!("../../tests/fixtures/v1/soundcraft.json"), "/mix_docking/layout"),
            ("photocraft", include_str!("../../tests/fixtures/v1/photocraft.json"), "/docking"),
            ("photocraft", include_str!("../../tests/fixtures/v1/photocraft.json"), "/panel_layout/docking"),
        ];
        for (app, text, pointer) in fixtures {
            let file: Value = serde_json::from_str(text).unwrap();
            let mut saved: super::Layout<String> = serde_json::from_value(file.pointer(pointer).unwrap().clone()).unwrap();
            let own = saved.contains(&s("canvas")).then(|| saved.clone());
            saved.take(&s("canvas"));
            for default in own.iter().chain([&beside, &alone]) {
                let mut workspace = super::Workspace::new(saved.clone());
                // A file written while the canvas could still be lost.
                if let Ok(location) = default.location(&s("canvas")) {
                    workspace.hidden.push((s("canvas"), location));
                }
                for mut workspace in [super::Workspace::new(saved.clone()), workspace] {
                    workspace.reconcile(|_| true, permissions, &super::Workspace::new(default.clone()));
                    workspace.validate().unwrap_or_else(|e| panic!("{app}{pointer}: {e}"));
                    assert!(workspace.layout.contains(&s("canvas")), "{app}{pointer}: {:?}", workspace.layout);
                    assert!(workspace.hidden.iter().all(|(p, _)| p != "canvas"), "{app}{pointer}");
                }
            }
        }
    }
}

#[test]
fn edge_placement_splits_the_part_between_the_docks_or_the_root() {
    // Dock layout: a floating window lands under the frames, beside the end dock.
    let mut layout = docked();
    layout.apply(Action::MoveWindow { panel: 6, to: Target::At { anchor: 1, placement: Placement::Edge(Zone::Bottom) } }).unwrap();
    let Some(Node::Split { first, second, .. }) = &layout.root else { panic!("{layout:?}") };
    assert!(matches!(&**second, Node::Dock { .. }));
    let Node::Split { axis: SplitAxis::Vertical, size: SplitSize::Ratio(size), second: bottom, .. } = &**first else { panic!("{first:?}") };
    assert_eq!(*size, 0.75);
    assert_eq!(**bottom, tabs(&[6, 7, 8], 1));
    assert!(layout.floating.is_empty());

    // Frame layout: one panel takes the whole left edge.
    let mut frames = Layout { root: Some(hsplit(SplitSize::Ratio(0.5), tabs(&[1], 0), tabs(&[2, 3], 0))), floating: Vec::new() };
    frames.apply(Action::Move { panel: 3, anchor: 1, placement: Placement::Edge(Zone::Left) }).unwrap();
    let Some(Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::Ratio(size), first, .. }) = &frames.root else { panic!() };
    assert_eq!((*size, &**first), (0.25, &tabs(&[3], 0)));

    let before = frames.clone();
    assert_eq!(frames.apply(Action::Move { panel: 2, anchor: 1, placement: Placement::Edge(Zone::Center) }), Err(DockError::InvalidPlacement));
    assert_eq!(frames, before);
}
