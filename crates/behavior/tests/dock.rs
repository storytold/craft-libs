//! The dock model: validation, restore to the old place, permissions and the persisted shapes.

use craft_behavior::Axis;
use craft_behavior::dock::{Action, Column, DockEdge, DockError, Floating, Group, Layout, Location, Node, SplitSize};
use craft_behavior::placement::Placement;
use emath::{Pos2, Rect, Vec2};
use serde_json::json;

const WINDOW: Rect = Rect::from_min_max(Pos2::new(100.0, 100.0), Pos2::new(400.0, 500.0));

fn area(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(width, height))
}

fn g(panels: &[u32], active: usize) -> Group<u32> {
    Group::new(panels.to_vec(), active)
}

fn tabs(panels: &[u32], active: usize) -> Node<u32> {
    Node::tabs(panels.to_vec(), active)
}

fn hsplit(size: SplitSize, first: Node<u32>, second: Node<u32>) -> Node<u32> {
    Node::split(Axis::X, size, first, second)
}

/// Canvas 1 above frame tabs [11, 12]; an end dock with column A ([2, 3], [4]) and column B
/// ([5]); a floating window with groups [6, 7] and [8].
fn docked() -> Layout<u32> {
    let middle = Node::split(Axis::Y, SplitSize::Ratio(0.7), tabs(&[1], 0), tabs(&[11, 12], 1));
    let dock = Node::dock(
        DockEdge::End,
        vec![Column::new(vec![g(&[2, 3], 1), g(&[4], 0)]).with_width(180.0), Column::new(vec![g(&[5], 0)]).with_width(120.0)],
    );
    Layout::new(Some(hsplit(SplitSize::FixedSecond(300.0), middle, dock)), vec![Floating::stack(vec![g(&[6, 7], 1), g(&[8], 0)], WINDOW).unwrap()])
}

fn with_column(column: Column<u32>) -> Layout<u32> {
    Layout::new(Some(Node::dock(DockEdge::End, vec![column])), Vec::new())
}

#[test]
fn validation_rejects_two_docks_per_side_nested_docks_empty_columns_and_bad_widths() {
    docked().validate().unwrap();
    let empty_dock = |side| Node::dock(side, Vec::new());
    Layout::new(Some(hsplit(SplitSize::FixedFirst(10.0), empty_dock(DockEdge::Start), tabs(&[1], 0))), Vec::new()).validate().unwrap();

    let two = hsplit(SplitSize::Ratio(0.5), empty_dock(DockEdge::Start), empty_dock(DockEdge::Start));
    let nested = Node::split(Axis::Y, SplitSize::Ratio(0.5), empty_dock(DockEdge::Start), tabs(&[1], 0));
    let wrong_side = hsplit(SplitSize::Ratio(0.5), empty_dock(DockEdge::End), tabs(&[1], 0));
    for root in [two, nested, wrong_side] {
        assert_eq!(Layout::new(Some(root), Vec::new()).validate(), Err(DockError::InvalidPlacement));
    }

    let mut nan_height = g(&[2], 0);
    nan_height.height = Some(f32::NAN);
    let mut nan_iconic = Column::new(vec![g(&[2], 0)]).with_width(100.0);
    nan_iconic.iconic_width = Some(f32::NAN);
    let bad = [
        Column::new(Vec::new()).with_width(100.0),
        Column::new(vec![g(&[], 0)]).with_width(100.0),
        Column::new(vec![g(&[2], 1)]).with_width(100.0),
        Column::new(vec![nan_height]).with_width(100.0),
        Column::new(vec![g(&[2], 0)]).with_width(0.0),
        Column::new(vec![g(&[2], 0)]).with_width(-5.0),
        Column::new(vec![g(&[2], 0)]).with_width(f32::INFINITY),
        Column::new(vec![g(&[2], 0)]).with_width(2.0e6),
        nan_iconic,
    ];
    for column in bad {
        assert!(with_column(column.clone()).validate().is_err(), "{column:?}");
    }
}

/// A flyout width is validated like a column width, survives a close and restore, and is saved only when set.
#[test]
fn a_flyout_width_is_validated_restored_and_saved_only_when_set() {
    let mut iconic = Column::new(vec![g(&[1], 0), g(&[2], 0)]);
    iconic.iconic = true;
    let mut layout = with_column(iconic);
    let original = layout.clone();
    assert!(!serde_json::to_string(&original).unwrap().contains("flyout_width"), "an untouched width is not saved");
    for width in [0.0, -1.0, f32::NAN, f32::INFINITY, 2.0e6] {
        assert_eq!(layout.apply(Action::resize_flyout_width(1, Some(width))), Err(DockError::InvalidGeometry), "{width}");
        assert_eq!(layout, original);
    }
    let mut nan = g(&[1], 0);
    nan.flyout_width = Some(f32::NAN);
    assert!(with_column(Column::new(vec![nan])).validate().is_err());

    layout.apply(Action::resize_flyout_width(1, Some(400.0))).unwrap();
    let groups = |layout: &Layout<u32>| match layout.root.as_ref() {
        Some(Node::Dock { columns, .. }) => columns[0].groups.iter().map(|group| group.flyout_width).collect::<Vec<_>>(),
        root => panic!("{root:?}"),
    };
    assert_eq!(groups(&layout), [Some(400.0), None], "the width belongs to the panel's group alone");
    let saved: Layout<u32> = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
    assert_eq!(saved, layout);

    let location = layout.location(&1).unwrap();
    let location: Location<u32> = serde_json::from_str(&serde_json::to_string(&location).unwrap()).unwrap();
    layout.apply(Action::close(1)).unwrap();
    layout.restore(1, &location).unwrap();
    assert_eq!(groups(&layout), [Some(400.0), None], "a restore gives the group its flyout width back");

    layout.apply(Action::resize_flyout_width(1, None)).unwrap();
    assert_eq!(layout, original);
}

/// The layout of `restore_returns_each_panel_to_the_place_it_was_in` with `skip` absent.
fn without(skip: Option<u32>) -> Layout<u32> {
    let group = |panels: &[u32], active: usize| {
        let kept: Vec<u32> = panels.iter().copied().filter(|p| Some(*p) != skip).collect();
        (!kept.is_empty()).then(|| {
            let active = active.min(kept.len() - 1);
            Group::new(kept, active)
        })
    };
    let mut minimized = group(&[4], 0);
    if let Some(group) = &mut minimized {
        group.minimized = true;
        group.height = Some(150.0);
    }
    let mut tall = group(&[7], 0);
    if let Some(group) = &mut tall {
        group.height = Some(90.0);
    }
    let a: Vec<Group<u32>> = [group(&[2, 3], 1), minimized, group(&[9], 0)].into_iter().flatten().collect();
    let mut b = Column::new(group(&[5], 0).into_iter().collect()).with_width(120.0);
    b.iconic = true;
    b.iconic_width = Some(48.0);
    let columns: Vec<Column<u32>> = [Column::new(a).with_width(180.0), b].into_iter().filter(|c| !c.groups.is_empty()).collect();
    let end = Node::dock(DockEdge::End, columns);
    let windows: Vec<Group<u32>> = [group(&[6], 0), tall].into_iter().flatten().collect();
    Layout::new(
        Some(hsplit(SplitSize::FixedSecond(300.0), tabs(&[1], 0), end)),
        vec![Floating::stack(windows, area(10.0, 10.0, 200.0, 300.0)).unwrap()],
    )
}

#[test]
fn restore_returns_each_panel_to_the_place_it_was_in() {
    let original = without(None);
    original.validate().unwrap();
    // 4: the seam between [2, 3] and [9]; 9: below [4]; 5: a new column after A; 3: a tab;
    // 7: below a floating group.
    for panel in [4, 9, 5, 3, 7] {
        let location = original.location(&panel).unwrap();
        let location: Location<u32> = serde_json::from_str(&serde_json::to_string(&location).unwrap()).unwrap();
        let mut layout = without(Some(panel));
        layout.validate().unwrap();
        layout.restore(panel, &location).unwrap_or_else(|e| panic!("{panel}: {e} {location:?}"));
        assert_eq!(layout, original, "{panel}: {location:?}");
    }
}

#[test]
fn a_layout_saved_with_the_physical_names_loads_and_writes_back_in_reading_order() {
    let saved = r#"{
        "root": {"Split": {"axis": "Horizontal", "size": {"FixedSecond": 300.0},
            "first": {"Tabs": {"panels": [1, 2], "active": 1}},
            "second": {"Dock": {"side": "End", "columns": [
                {"groups": [{"panels": [3], "active": 0, "minimized": false, "height": null}], "width": 180.0, "iconic": false, "iconic_width": null}
            ]}}}},
        "floating": [{"id": 5, "panels": [4], "active": 0, "rect": [1.0, 2.0, 300.0, 200.0]}]
    }"#;
    let layout: Layout<u32> = serde_json::from_str(saved).unwrap();
    layout.validate().unwrap();
    let expected = Layout::new(
        Some(hsplit(
            SplitSize::FixedSecond(300.0),
            tabs(&[1, 2], 1),
            Node::dock(DockEdge::End, vec![Column::new(vec![g(&[3], 0)]).with_width(180.0)]),
        )),
        vec![Floating::single(vec![4], 0, area(1.0, 2.0, 300.0, 200.0)).with_id(5)],
    );
    assert_eq!(layout, expected);
    let written = serde_json::to_value(&layout).unwrap();
    assert_eq!(written["root"]["Split"]["axis"], json!("X"));
    let mut renamed: serde_json::Value = serde_json::from_str(saved).unwrap();
    renamed["root"]["Split"]["axis"] = json!("X");
    assert_eq!(written, renamed);
}

#[test]
fn a_saved_native_window_that_no_longer_parses_loads_with_default_placement() {
    let saved = r#"{
        "root": {"Tabs": {"panels": [1], "active": 0}},
        "floating": [
            {"id": 5, "root": {"Tabs": {"panels": [4], "active": 0}}, "rect": [1.0, 2.0, 300.0, 200.0],
             "native": [10.0, 20.0, 300.0, 200.0], "monitor": {"name": "DP-1", "rect": [0.0, 0.0, 1920.0, 1080.0], "scale": 1.0},
             "monitor_size": [1920.0, 1080.0]},
            {"id": 6, "root": {"Tabs": {"panels": [7], "active": 0}}, "rect": [1.0, 2.0, 300.0, 200.0],
             "native": {"normal": {"pos": [0.0, 0.0], "size": [0.0, 200.0]}}}
        ]
    }"#;
    let layout: Layout<u32> = serde_json::from_str(saved).unwrap();
    layout.validate().unwrap();
    assert_eq!(layout.floating.len(), 2);
    assert!(layout.floating.iter().all(|window| window.native.is_none()), "an old rect and a size that is no size give no placement");
    assert_eq!(layout.floating[0].rect, area(1.0, 2.0, 300.0, 200.0), "the rest of the window loads");

    let mut placed = layout.clone();
    placed.floating[0].native = Some(Placement::new([300.0, 200.0], 2.0));
    let back: Layout<u32> = serde_json::from_str(&serde_json::to_string(&placed).unwrap()).unwrap();
    assert_eq!(back, placed);
}
