use super::*;
use crate::layout::{SplitAxis, SplitSize};
use egui::{Context, Event, Id, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

fn tabs(panels: &[u32], active: usize) -> Node<u32> {
    Node::Tabs { panels: panels.to_vec(), active }
}

fn sample() -> Layout<u32> {
    Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedSecond(240.0),
            first: Box::new(tabs(&[1], 0)),
            second: Box::new(tabs(&[2, 3, 4], 1)),
        }),
        floating: vec![Floating::single(vec![5, 6], 1, [80.0, 70.0, 300.0, 220.0])],
    }
}

fn move_to(panel: u32, anchor: u32, zone: Zone) -> Action<u32> {
    Action::Move { panel, anchor, placement: Placement::Split(zone) }
}

#[test]
fn floating_brush_panel_joins_main_tabs_without_losing_other_panels() {
    let mut layout = sample();
    layout.apply(Action::Move { panel: 5, anchor: 3, placement: Placement::Tab { before: Some(4) } }).unwrap();
    let Node::Split { size, second, .. } = layout.root.as_ref().unwrap() else { panic!() };
    assert_eq!(*size, SplitSize::FixedSecond(240.0));
    assert_eq!(**second, tabs(&[2, 3, 5, 4], 2));
    assert_eq!(layout.floating[0].groups()[0].panels, vec![6]);
    assert_eq!(layout.floating[0].groups()[0].active, 0);
    layout.validate().unwrap();
    assert_eq!(layout.panels().len(), 6);
}

#[test]
fn detach_redock_merge_and_collapse_empty_source_containers() {
    let mut layout = sample();
    layout.apply(Action::Float { panel: 1, rect: [30.0, 40.0, 250.0, 200.0] }).unwrap();
    assert_eq!(layout.root, Some(tabs(&[2, 3, 4], 1)));
    layout.apply(move_to(1, 2, Zone::Left)).unwrap();
    assert_eq!(layout.floating.len(), 1);
    layout.apply(move_to(5, 6, Zone::Center)).unwrap();
    assert_eq!(layout.floating[0].groups()[0].panels, vec![6, 5]);
    assert_eq!(layout.floating[0].groups()[0].active, 1);
    layout.apply(move_to(6, 3, Zone::Center)).unwrap();
    layout.apply(move_to(5, 3, Zone::Center)).unwrap();
    assert!(layout.floating.is_empty());
    assert_eq!(layout.panels().len(), 6);
}

#[test]
fn move_targets_skip_the_panel_itself_and_protected_anchors() {
    let layout = sample();
    let all = |_: &u32| Permissions::default();
    let targets = layout.move_targets(&1, all);
    assert_eq!(targets.len(), 5);
    assert!(!targets.contains(&&1));
    let guarded = |p: &u32| if *p == 3 { Permissions::PROTECTED } else { Permissions::default() };
    assert!(!layout.move_targets(&1, guarded).contains(&&3));

    // A lone panel has nowhere to go — its "Move Panel to Group" menu hides
    // (storytold/lightcraft#1768).
    let single = Layout { root: Some(tabs(&[1], 0)), floating: Vec::new() };
    assert!(single.move_targets(&1, all).is_empty());
}

#[test]
fn move_targets_skip_the_panels_own_group() {
    let layout = sample();
    let all = |_: &u32| Permissions::default();
    assert_eq!(layout.move_targets(&3, all), vec![&1, &5, &6]);
    let pair = Layout { root: Some(tabs(&[1, 2], 0)), floating: Vec::new() };
    assert!(pair.move_targets(&1, all).is_empty());
}

#[test]
fn invalid_moves_are_atomic_including_after_a_source_would_have_been_removed() {
    let layout = sample();
    for action in [
        move_to(5, 99, Zone::Center),
        move_to(99, 2, Zone::Center),
        Action::Move { panel: 5, anchor: 6, placement: Placement::Column { after: true } },
        move_to(5, 5, Zone::Center),
        Action::Move { panel: 5, anchor: 2, placement: Placement::Tab { before: Some(6) } },
        Action::Float { panel: 3, rect: [f32::NAN, 0.0, 100.0, 100.0] },
        Action::Float { panel: 3, rect: [0.0, 0.0, 0.0, 100.0] },
        Action::ResizeSplit { path: vec![false], size: SplitSize::Ratio(0.3) },
    ] {
        let mut changed = layout.clone();
        assert!(changed.apply(action).is_err());
        assert_eq!(changed, layout);
    }
}

#[test]
fn removing_an_earlier_tab_preserves_selected_identity() {
    let mut layout = Layout { root: Some(tabs(&[1, 2, 3, 4], 2)), floating: Vec::new() };
    layout.apply(Action::Close { panel: 1 }).unwrap();
    assert_eq!(layout.root, Some(tabs(&[2, 3, 4], 1)));
    layout.apply(Action::Close { panel: 3 }).unwrap();
    assert_eq!(layout.root, Some(tabs(&[2, 4], 1)));
}

#[test]
fn reordering_tabs_uses_destination_identity_after_removal() {
    let mut layout = Layout { root: Some(tabs(&[1, 2, 3], 0)), floating: Vec::new() };
    layout.apply(Action::Move { panel: 1, anchor: 2, placement: Placement::Tab { before: None } }).unwrap();
    assert_eq!(layout.root, Some(tabs(&[2, 3, 1], 2)));
    layout.apply(Action::Move { panel: 1, anchor: 2, placement: Placement::Tab { before: Some(2) } }).unwrap();
    assert_eq!(layout.root, Some(tabs(&[1, 2, 3], 0)));
    let before = layout.clone();
    layout.apply(Action::Move { panel: 1, anchor: 2, placement: Placement::Tab { before: Some(1) } }).unwrap();
    assert_eq!(layout, before);
}

#[test]
fn accordion_open_state_and_custom_height_survive_reorder() {
    let mut layout = Layout {
        root: Some(Node::Stack {
            entries: vec![StackEntry { panel: 1, open: false, height: Some(123.0) }, StackEntry { panel: 2, open: true, height: None }],
        }),
        floating: Vec::new(),
    };
    layout.apply(Action::Move { panel: 1, anchor: 2, placement: Placement::Tab { before: None } }).unwrap();
    let Node::Stack { entries } = layout.root.as_ref().unwrap() else { panic!() };
    assert_eq!(entries[1], StackEntry { panel: 1, open: false, height: Some(123.0) });
    let location = layout.location(&1).unwrap();
    layout.apply(Action::Close { panel: 1 }).unwrap();
    layout.restore(1, &location).unwrap();
    let Node::Stack { entries } = layout.root.as_ref().unwrap() else { panic!() };
    assert_eq!(entries[1], StackEntry { panel: 1, open: false, height: Some(123.0) });
}

#[test]
fn protected_canvas_remains_present_and_cannot_be_grouped_with_panels() {
    let permissions = |panel: &u32| if *panel == 1 { Permissions::PROTECTED } else { Permissions::default() };
    let layout = sample();
    for action in [
        Action::Close { panel: 1 },
        Action::Float { panel: 1, rect: [10.0, 10.0, 200.0, 200.0] },
        move_to(1, 3, Zone::Center),
        move_to(3, 1, Zone::Center),
        Action::OpenAt { panel: 1, anchor: 3, placement: Placement::Tab { before: None } },
        Action::OpenAt { panel: 7, anchor: 1, placement: Placement::Split(Zone::Left) },
    ] {
        let mut next = layout.clone();
        assert_eq!(next.apply_with_permissions(action, permissions), Err(DockError::NotAllowed));
        assert_eq!(next, layout);
    }
    let beside = |panel: &u32| if *panel == 1 { Permissions { accepts_splits: true, ..Permissions::PROTECTED } } else { Permissions::default() };
    let mut next = layout;
    next.apply_with_permissions(Action::OpenAt { panel: 7, anchor: 1, placement: Placement::Split(Zone::Left) }, beside).unwrap();
    assert!(next.contains(&1) && next.contains(&7));
}

#[test]
fn moving_a_floating_group_needs_every_member_movable() {
    // sample() floats 5 and 6 together; 6 may not move, so naming 5 cannot move it either.
    let permissions = |panel: &u32| Permissions { movable: *panel != 6, ..Permissions::default() };
    let layout = sample();
    let mut next = layout.clone();
    assert_eq!(
        next.apply_with_permissions(Action::MoveFloating { panel: 5, rect: [0.0, 0.0, 300.0, 220.0] }, permissions),
        Err(DockError::NotAllowed)
    );
    assert_eq!(next, layout);
    next.apply_with_permissions(Action::MoveFloating { panel: 5, rect: [0.0, 0.0, 300.0, 220.0] }, |_: &u32| Permissions::default()).unwrap();
    assert_eq!(next.floating[0].rect, [0.0, 0.0, 300.0, 220.0]);

    // The same holds when 6 sits in another group of the window's tree.
    let mut split = sample();
    split.floating[0].root =
        Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::Ratio(0.5), first: Box::new(tabs(&[5], 0)), second: Box::new(tabs(&[6], 0)) };
    let before = split.clone();
    assert_eq!(
        split.apply_with_permissions(Action::MoveFloating { panel: 5, rect: [0.0, 0.0, 300.0, 220.0] }, permissions),
        Err(DockError::NotAllowed)
    );
    assert_eq!(split, before);
}

#[test]
fn close_and_restore_preserve_fixed_side_and_floating_geometry() {
    let mut layout = Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedFirst(180.0),
            first: Box::new(tabs(&[1], 0)),
            second: Box::new(tabs(&[2], 0)),
        }),
        floating: vec![Floating::single(vec![3], 0, [41.0, 52.0, 233.0, 244.0])],
    };
    let saved = layout.clone();
    let first = layout.location(&1).unwrap();
    let float = layout.location(&3).unwrap();
    layout.apply(Action::Close { panel: 1 }).unwrap();
    layout.apply(Action::Close { panel: 3 }).unwrap();
    layout.restore(1, &first).unwrap();
    layout.restore(3, &float).unwrap();
    assert_eq!(layout, saved);
    let invalid = Location { anchor: Some(99), ..first };
    layout.apply(Action::Close { panel: 1 }).unwrap();
    let before = layout.clone();
    assert_eq!(layout.restore(1, &invalid), Err(DockError::MissingTarget));
    assert_eq!(layout, before);
}

#[test]
fn closing_and_reopening_an_entire_accordion_restores_its_kind_and_heights() {
    let mut layout = Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedSecond(220.0),
            first: Box::new(tabs(&[1], 0)),
            second: Box::new(Node::Stack {
                entries: vec![StackEntry { panel: 2, open: false, height: Some(90.0) }, StackEntry { panel: 3, open: true, height: Some(180.0) }],
            }),
        }),
        floating: Vec::new(),
    };
    let before = layout.clone();
    let second = layout.location(&2).unwrap();
    layout.apply(Action::Close { panel: 2 }).unwrap();
    let third = layout.location(&3).unwrap();
    layout.apply(Action::Close { panel: 3 }).unwrap();
    layout.restore(3, &third).unwrap();
    layout.restore(2, &second).unwrap();
    assert_eq!(layout, before);
}

#[test]
fn invalid_saved_layouts_reject_duplicates_selection_depth_and_nonfinite_geometry() {
    let mut duplicate = sample();
    if let Node::Tabs { panels, .. } = &mut duplicate.floating[0].root {
        panels.push(1);
    }
    assert_eq!(duplicate.validate(), Err(DockError::DuplicatePanel));
    let mut invalid = sample();
    if let Node::Tabs { active, .. } = &mut invalid.floating[0].root {
        *active = usize::MAX;
    }
    assert_eq!(invalid.validate(), Err(DockError::InvalidSelection));
    if let Node::Tabs { active, .. } = &mut invalid.floating[0].root {
        *active = 0;
    }
    invalid.floating[0].rect[3] = f32::INFINITY;
    assert_eq!(invalid.validate(), Err(DockError::InvalidGeometry));
    let mut deep = tabs(&[0], 0);
    for panel in 1..=MAX_DEPTH as u32 {
        deep = Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::Ratio(0.5), first: Box::new(deep), second: Box::new(tabs(&[panel], 0)) };
    }
    let mut deep = Layout { root: Some(deep), floating: Vec::new() };
    assert_eq!(deep.validate(), Err(DockError::LimitExceeded));
    assert_eq!(deep.apply(Action::Activate { panel: 0 }), Err(DockError::LimitExceeded));
    let oversized = Layout { root: Some(Node::Tabs { panels: (0..=MAX_PANELS as u32).collect(), active: 0 }), floating: Vec::new() };
    assert_eq!(oversized.validate(), Err(DockError::LimitExceeded));
    for empty in [tabs(&[], 0), Node::Stack { entries: Vec::new() }] {
        let root = Layout { root: Some(empty.clone()), floating: Vec::new() };
        assert_eq!(root.validate(), Err(DockError::InvalidGeometry));
        let beside =
            Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::Ratio(0.5), first: Box::new(tabs(&[1], 0)), second: Box::new(empty) };
        assert_eq!(Layout { root: Some(beside), floating: Vec::new() }.validate(), Err(DockError::InvalidGeometry));
    }
}

fn target(anchor: u32, members: &[u32], rect: Rect) -> DropTarget<u32> {
    let strip = Rect::from_min_size(rect.min, vec2(rect.width(), 24.0));
    DropTarget {
        anchor,
        rect,
        tab_strip: Some(strip),
        tabs: members
            .iter()
            .enumerate()
            .map(|(i, panel)| (*panel, Rect::from_min_size(rect.min + vec2(i as f32 * 60.0, 0.0), vec2(60.0, 24.0))))
            .collect(),
        allow_splits: true,
        accepts_tabs: true,
        right_to_left: false,
    }
}

#[test]
fn pointer_targets_distinguish_reorder_center_split_and_occlusion() {
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 200.0));
    let group = target(2, &[2, 3], rect);
    let placement = resolve_drop(&5, pos2(90.0, 30.0), std::slice::from_ref(&group), 40.0).unwrap();
    assert_eq!(placement.placement, Placement::Tab { before: Some(3) });
    assert!(placement.insertion.unwrap().is_finite());
    assert_eq!(resolve_drop(&5, rect.center(), std::slice::from_ref(&group), 40.0).unwrap().placement, Placement::Split(Zone::Center));
    assert_eq!(resolve_drop(&5, pos2(11.0, 90.0), std::slice::from_ref(&group), 40.0).unwrap().placement, Placement::Split(Zone::Left));
    assert!(resolve_drop(&3, rect.center(), std::slice::from_ref(&group), 40.0).unwrap().noop);
    assert!(resolve_drop(&3, pos2(11.0, 90.0), std::slice::from_ref(&group), 40.0).is_some());
    let mut overlay = target(5, &[5], rect);
    overlay.accepts_tabs = false;
    overlay.allow_splits = false;
    assert!(resolve_drop(&3, rect.center(), &[group, overlay], 40.0).is_none());
    assert!(drop_zone(Rect::NOTHING, 24.0, 40.0, pos2(0.0, 0.0)).is_none());
    assert!(drop_zone(rect, 24.0, 40.0, pos2(f32::NAN, 0.0)).is_none());
}

#[test]
fn side_edge_zones_are_40_pt_deep_or_a_quarter_of_the_bodys_smaller_side() {
    let depth = SIDE_EDGE_ZONE_DEPTH;
    assert_eq!(depth, 40.0);
    // The inner corner of the left trapezoid is the depth of both bands.
    let inner = |rect: Rect| zone_shape(rect, 0.0, depth, Zone::Left).unwrap()[1] - rect.min;
    for size in [vec2(600.0, 400.0), vec2(160.0, 160.0)] {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), size);
        assert_eq!(inner(rect), vec2(40.0, 40.0), "{size:?}");
        let y = rect.center().y;
        assert_eq!(drop_zone(rect, 0.0, depth, pos2(rect.left() + 39.0, y)), Some(Zone::Left), "{size:?}");
        assert_eq!(drop_zone(rect, 0.0, depth, pos2(rect.left() + 41.0, y)), Some(Zone::Center), "{size:?}");
    }
    // A 90 pt body keeps a 45 pt center.
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(90.0, 90.0));
    assert_eq!(inner(rect), vec2(22.5, 22.5));
    assert_eq!(drop_zone(rect, 0.0, depth, rect.center()), Some(Zone::Center));
    assert_eq!(drop_zone(rect, 0.0, depth, pos2(rect.left() + 22.0, rect.center().y)), Some(Zone::Left));
    // A tall narrow body: its top and bottom zones are as deep as its left and right ones.
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(120.0, 600.0));
    assert_eq!(inner(rect), vec2(30.0, 30.0));
    let top = zone_shape(rect, 0.0, depth, Zone::Top).unwrap();
    assert_eq!(top[2].y - rect.top(), 30.0);
    assert_eq!(drop_zone(rect, 0.0, depth, pos2(rect.center().x, rect.top() + 29.0)), Some(Zone::Top));
    assert_eq!(drop_zone(rect, 0.0, depth, pos2(rect.center().x, rect.top() + 31.0)), Some(Zone::Center));
    // The bands start below the tab header.
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(600.0, 400.0));
    assert_eq!(drop_zone(rect, 24.0, depth, pos2(rect.center().x, rect.top() + 60.0)), Some(Zone::Top));
    assert_eq!(zone_shape(rect, 24.0, depth, Zone::Top).unwrap()[0], rect.left_top() + vec2(0.0, 24.0));
    // Without depth there are no sides.
    assert_eq!(drop_zone(rect, 0.0, 0.0, pos2(rect.left() + 1.0, rect.center().y)), Some(Zone::Center));
    assert!(zone_shape(rect, 0.0, 0.0, Zone::Left).is_none());
}

#[test]
fn a_bodys_outer_thirds_are_its_sides_and_the_relatively_nearer_side_wins_at_a_corner() {
    let fraction = DockStyle::default().side_zone_fraction;
    assert!((fraction - 1.0 / 3.0).abs() < 1e-6);
    // A 24 pt header over a 600 x 300 body.
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(600.0, 324.0));
    let body = Rect::from_min_max(pos2(10.0, 44.0), rect.max);
    let zone = |p| body_zone(rect, 24.0, fraction, true, p);
    let y = body.center().y;
    assert_eq!(zone(pos2(body.left() + 199.0, y)), Some(Zone::Left));
    assert_eq!(zone(pos2(body.left() + 201.0, y)), Some(Zone::Center));
    assert_eq!(zone(pos2(body.right() - 199.0, y)), Some(Zone::Right));
    assert_eq!(zone(pos2(body.center().x, body.top() + 99.0)), Some(Zone::Top));
    assert_eq!(zone(pos2(body.center().x, body.top() + 101.0)), Some(Zone::Center));
    assert_eq!(zone(pos2(body.center().x, body.bottom() - 99.0)), Some(Zone::Bottom));
    assert_eq!(zone(pos2(body.left() + 2.0, rect.top() + 10.0)), Some(Zone::Center), "the header joins");
    // In a corner: a quarter of the width from the left is further than a fifth of the height
    // from the top, a tenth of the width nearer than three tenths of the height.
    assert_eq!(zone(pos2(body.left() + 150.0, body.top() + 60.0)), Some(Zone::Top));
    assert_eq!(zone(pos2(body.left() + 60.0, body.top() + 90.0)), Some(Zone::Left));
    // Without a middle, the relatively nearest side takes every point.
    let edges = |p| body_zone(rect, 24.0, fraction, false, p);
    assert_eq!(edges(body.center() - vec2(10.0, 0.0)), Some(Zone::Left));
    assert_eq!(edges(body.center() - vec2(0.0, 10.0)), Some(Zone::Top));
    assert_eq!(edges(body.center() + vec2(0.0, 10.0)), Some(Zone::Bottom));
    // No fraction: only the middle. Outside or invalid: nothing.
    assert_eq!(body_zone(rect, 24.0, 0.0, true, pos2(body.left() + 1.0, y)), Some(Zone::Center));
    assert!(body_zone(rect, 24.0, fraction, true, pos2(0.0, 0.0)).is_none());
    assert!(body_zone(Rect::NOTHING, 24.0, fraction, true, pos2(0.0, 0.0)).is_none());
}

#[test]
fn a_drop_beside_a_group_previews_a_line_along_its_edge_and_a_group_without_tabs_has_no_middle() {
    let style = DockStyle::from_visuals(&egui::Visuals::dark());
    let rule = style.zone_rule();
    assert_eq!(rule, ZoneRule::Fraction { fraction: 1.0 / 3.0, line: 4.0 });
    assert_eq!(DockStyle { side_edge_zones: true, ..style }.zone_rule(), ZoneRule::Depth(SIDE_EDGE_ZONE_DEPTH));
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 224.0));
    let group = target(2, &[2, 3], rect);
    let resolve = |at, group: &DropTarget<u32>| resolve_drop_by(&5, at, std::slice::from_ref(group), rule);
    let left = resolve(pos2(20.0, 140.0), &group).unwrap();
    assert_eq!(left.placement, Placement::Split(Zone::Left));
    assert_eq!(left.preview, Rect::from_x_y_ranges(10.0..=14.0, rect.y_range()), "the group's whole left edge");
    let bottom = resolve(pos2(160.0, 240.0), &group).unwrap();
    assert_eq!(bottom.placement, Placement::Split(Zone::Bottom));
    assert_eq!(bottom.preview, Rect::from_x_y_ranges(rect.x_range(), 240.0..=244.0));
    let middle = resolve(pos2(160.0, 144.0), &group).unwrap();
    assert_eq!((middle.placement, middle.preview), (Placement::Split(Zone::Center), rect));
    // A canvas: its middle goes to the nearest edge, and its strip takes nothing.
    let canvas = DropTarget { accepts_tabs: false, ..group };
    assert_eq!(resolve(pos2(150.0, 144.0), &canvas).unwrap().placement, Placement::Split(Zone::Left));
    assert!(resolve(pos2(90.0, 30.0), &canvas).is_none());
}

#[test]
fn a_style_needs_a_drop_line_and_side_zones_of_at_most_half_the_body() {
    let style = DockStyle::from_visuals(&egui::Visuals::dark());
    assert!(style.valid());
    assert!(DockStyle { side_zone_fraction: 0.0, ..style.clone() }.valid());
    assert!(DockStyle { side_zone_fraction: 0.5, ..style.clone() }.valid());
    assert!(!DockStyle { side_zone_fraction: 0.6, ..style.clone() }.valid());
    assert!(!DockStyle { drop_line_width: 0.0, ..style }.valid());
}

#[test]
fn long_action_sequence_never_loses_or_duplicates_a_panel() {
    for (start, ids) in [(sample(), vec![1, 2, 3, 4, 5, 6]), (super::model_tests::docked(), super::model_tests::ALL.to_vec())] {
        let mut workspace = Workspace::new(start);
        let mut seed = 72u64;
        for step in 0..800 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let pick = |bits: u64| ids[(bits % ids.len() as u64) as usize];
            let (panel, anchor) = (pick(seed >> 8), pick(seed >> 24));
            let placement = match (seed >> 40) % 7 {
                0 => Placement::Split(Zone::Left),
                1 => Placement::Split(Zone::Right),
                2 => Placement::Split(Zone::Top),
                3 => Placement::Split(Zone::Bottom),
                4 => Placement::Split(Zone::Center),
                5 => Placement::Group { below: seed & 1 == 0, height: None },
                _ => Placement::Column { after: seed & 2 == 0 },
            };
            let to = match (seed >> 32) % 4 {
                0 => Target::Float { rect: None },
                1 => Target::NewDock { side: if seed & 4 == 0 { Side::Start } else { Side::End } },
                _ => Target::At { anchor, placement: placement.clone() },
            };
            let action = match step % 13 {
                0 => Action::Float { panel, rect: [10.0, 20.0, 200.0, 220.0] },
                1 => Action::MoveGroup { panel, to },
                2 => Action::MoveColumn { panel, to },
                3 => Action::MoveWindow { panel, to },
                4 => Action::CloseGroup { panel },
                5 => Action::CloseWindow { panel },
                6 => Action::CloseOthers { panel },
                7 => Action::Close { panel },
                8 => Action::SetMinimized { panel, minimized: seed & 8 == 0 },
                9 => Action::SetIconic { panel, iconic: seed & 16 == 0 },
                _ => Action::Move { panel, anchor, placement },
            };
            let before = workspace.clone();
            if workspace.apply(action, |_| Permissions::default()).is_err() {
                assert_eq!(workspace, before);
            }
            if step % 3 == 0
                && let Some((hidden, _)) = workspace.hidden.first().cloned()
            {
                let fallback = workspace.layout.panels().first().map(|p| **p);
                let before = workspace.clone();
                if workspace.set_visible(hidden, true, fallback, |_| Permissions::default()).is_err() {
                    assert_eq!(workspace, before);
                }
            }
            workspace.validate().unwrap();
            let mut panels: Vec<u32> = workspace.layout.panels().into_iter().copied().chain(workspace.hidden.iter().map(|(p, _)| *p)).collect();
            panels.sort_unstable();
            assert_eq!(panels, ids, "step {step}");
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn saved_layout_and_hidden_location_round_trip() {
    let layout = sample();
    let value = serde_json::to_string(&layout).unwrap();
    let restored: Layout<u32> = serde_json::from_str(&value).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored, layout);
    let location = layout.location(&3).unwrap();
    let encoded = serde_json::to_string(&location).unwrap();
    assert_eq!(serde_json::from_str::<Location<u32>>(&encoded).unwrap(), location);
}

#[cfg(feature = "serde")]
#[test]
/// Deserialize an older hidden `Location` without a `sibling` field; the
/// missing list stays empty and the anchor panel is preserved.
fn old_hidden_locations_accept_panel_ids_without_a_default_identity() {
    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    enum Panel {
        Layers,
        Brushes,
    }
    let location: Location<Panel> = serde_json::from_value(serde_json::json!({
        "floating": null, "anchor": "Layers", "placement": {"Tab": {"before": null}},
        "size": null, "stack": null
    }))
    .unwrap();
    assert!(location.sibling.is_empty());
    assert_eq!(location.anchor, Some(Panel::Layers));
}

/// One 800×600 `DockArea::show` frame with actual body labels. Returns `DockOutput` plus the body
/// scope ids and records the final cursor icon in temp data; clears texture deltas.
fn frame(ctx: &Context, layout: &Layout<u32>, events: Vec<Event>, focused: bool) -> (DockOutput<u32>, Vec<Id>) {
    let mut output = DockOutput::default();
    let mut ids = Vec::new();
    let input = RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), events, focused, ..Default::default() };
    ctx.run_ui(input, |ui| {
        let style = DockStyle::from_ui(ui);
        output = DockArea::new(Id::unique("test-area")).show(
            ui,
            layout,
            &style,
            |p| format!("Panel {p}"),
            |_| Permissions::default(),
            |ui, _| {
                ids.push(ui.scope_id());
                ui.label("Actual panel content");
            },
        );
        let cursor = ctx.output(|o| o.cursor_icon);
        ctx.data_mut(|d| d.insert_temp(Id::unique("test-cursor"), cursor));
    })
    .textures_delta
    .clear();
    (output, ids)
}

#[test]
/// Moving a panel from a root tab into a floating window keeps its body scope
/// id across frames.
fn renderer_keeps_content_identity_when_panel_moves_into_a_float() {
    let ctx = Context::default();
    let mut layout = Layout { root: Some(tabs(&[1], 0)), floating: Vec::new() };
    let (output, before) = frame(&ctx, &layout, Vec::new(), true);
    assert!(output.error.is_none());
    layout.apply(Action::Float { panel: 1, rect: [100.0, 100.0, 320.0, 260.0] }).unwrap();
    let (output, after) = frame(&ctx, &layout, Vec::new(), true);
    assert!(output.error.is_none());
    assert_eq!(before, after);
}

#[test]
/// Escape and window focus loss each discard an in-progress drag; a subsequent release emits no
/// actions.
fn cancelling_or_losing_focus_during_a_drag_does_not_move_panels() {
    let ctx = Context::default();
    let layout = sample();
    let source = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0));
    let press = Event::PointerButton { pos: pos2(30.0, 30.0), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE };
    frame(&ctx, &layout, vec![Event::PointerMoved(pos2(30.0, 30.0)), press], true);
    assert!(begin_drag(&ctx, Id::unique("test-area"), 2u32, source));
    let (out, _) = frame(
        &ctx,
        &layout,
        vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
        true,
    );
    assert!(out.actions.is_empty());
    assert!(begin_drag(&ctx, Id::unique("test-area"), 2u32, source));
    let (out, _) = frame(&ctx, &layout, Vec::new(), false);
    assert!(out.actions.is_empty());
    let (out, _) = frame(
        &ctx,
        &layout,
        vec![Event::PointerButton { pos: pos2(700.0, 400.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }],
        true,
    );
    assert!(out.actions.is_empty());
}

fn step(ctx: &Context, layout: &mut Layout<u32>, events: Vec<Event>) -> Vec<Action<u32>> {
    let (out, _) = frame(ctx, layout, events, true);
    assert!(out.error.is_none());
    for action in &out.actions {
        layout.apply(action.clone()).unwrap();
    }
    out.actions
}

/// A primary-button pointer event at `pos` with `pressed` and no modifiers.
fn pointer_button(pos: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

#[test]
/// Real pointer input drags a floating panel's tab into the main tab strip, emits a `Move`, removes
/// it from the float, preserves the panel count, and activates it in the target group.
fn actual_pointer_drag_docks_a_floating_panel_into_the_main_tab_strip() {
    let ctx = Context::default();
    let mut layout = sample();
    for _ in 0..3 {
        step(&ctx, &mut layout, Vec::new());
    }
    let source = ctx.read_response(Id::unique("test-area").with(("tab", &5u32))).unwrap().rect.center();
    let target = ctx.read_response(Id::unique("test-area").with(("tab", &3u32))).unwrap().rect.center();
    step(&ctx, &mut layout, vec![Event::PointerMoved(source), pointer_button(source, true)]);
    step(&ctx, &mut layout, vec![Event::PointerMoved(source + vec2(8.0, 0.0))]);
    step(&ctx, &mut layout, vec![Event::PointerMoved(target)]);
    let actions = step(&ctx, &mut layout, vec![pointer_button(target, false)]);
    assert!(actions.iter().any(|a| matches!(a, Action::Move { panel: 5, .. })), "{actions:?}");
    assert!(!layout.floating.iter().any(|g| g.contains(&5)));
    assert_eq!(layout.panels().len(), 6);
    let Node::Split { second, .. } = layout.root.as_ref().unwrap() else { panic!() };
    let Node::Tabs { panels, active } = second.as_ref() else { panic!() };
    assert_eq!(panels.get(*active), Some(&5));
}

/// On X11 and Windows a release outside the window never reaches egui (egui-winit drops a
/// button event once the pointer has left), so a tab dragged out of the window used to stay on
/// the pointer for good. Leaving the window detaches the panel where it left, even over a dock
/// target, and ends the drag.
#[test]
fn tab_dragged_out_of_the_window_floats_where_it_left() {
    let ctx = Context::default();
    let mut layout = sample();
    for _ in 0..3 {
        step(&ctx, &mut layout, Vec::new());
    }
    let tab = ctx.read_response(Id::unique("test-area").with(("tab", &3u32))).unwrap().rect.center();
    let exit = pos2(4.0, 300.0);
    step(&ctx, &mut layout, vec![Event::PointerMoved(tab), pointer_button(tab, true)]);
    step(&ctx, &mut layout, vec![Event::PointerMoved(tab + vec2(-8.0, 0.0))]);
    step(&ctx, &mut layout, vec![Event::PointerMoved(exit)]);
    let actions = step(&ctx, &mut layout, vec![Event::PointerGone]);
    assert!(actions.iter().any(|a| matches!(a, Action::Float { panel: 3, .. })), "{actions:?}");
    assert!(layout.floating.iter().any(|g| g.panels().eq([3].iter())), "{layout:?}");
    let back = pos2(400.0, 300.0);
    assert!(step(&ctx, &mut layout, vec![Event::PointerMoved(back)]).is_empty());
    assert!(step(&ctx, &mut layout, vec![pointer_button(back, false)]).is_empty(), "the drag ended when the pointer left");
}

#[test]
/// At wide, narrow and wide-again sizes, every rendered body rect stays horizontally inside the
/// viewport and, at the wide sizes, panel min/max limits bound its width, without mutating the
/// layout's stored preferred widths.
fn panel_size_limits_clamp_geometry_without_overwriting_preferred_widths() {
    let layout = Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedFirst(450.0),
            first: Box::new(tabs(&[1], 0)),
            second: Box::new(Node::Split {
                axis: SplitAxis::Horizontal,
                size: SplitSize::FixedSecond(500.0),
                first: Box::new(tabs(&[0], 0)),
                second: Box::new(tabs(&[2], 0)),
            }),
        }),
        floating: Vec::new(),
    };
    let unchanged = layout.clone();
    let ctx = Context::default();
    let mut widths = Vec::new();
    for width in [1400.0, 900.0, 300.0, 1400.0] {
        let mut rects = std::collections::BTreeMap::new();
        let mut out = DockOutput::default();
        ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 600.0))), ..Default::default() }, |ui| {
            out = DockArea::new(Id::unique("bounds")).show_with_limits(
                ui,
                &layout,
                &DockStyle::from_ui(ui),
                |p| p.to_string(),
                |p| if *p == 0 { Permissions::PROTECTED } else { Permissions::default() },
                |p| match *p {
                    1 => PanelLimits { min: vec2(200.0, 40.0), max: vec2(480.0, 1_000_000.0) },
                    2 => PanelLimits { min: vec2(250.0, 40.0), max: vec2(520.0, 1_000_000.0) },
                    _ => PanelLimits { min: vec2(360.0, 40.0), ..Default::default() },
                },
                |ui, p| {
                    rects.insert(*p, ui.max_rect());
                },
            );
        })
        .textures_delta
        .clear();
        assert!(out.error.is_none());
        assert!(out.actions.is_empty());
        for rect in rects.values() {
            assert!(rect.is_finite() && rect.is_positive());
            assert!(rect.left() >= 0.0 && rect.right() <= width);
        }
        if width >= 900.0 {
            assert!((200.0..=480.0).contains(&rects[&1].width()));
            assert!((250.0..=520.0).contains(&rects[&2].width()));
            assert!(rects[&0].width() >= 360.0);
        }
        widths.push(rects[&1].width());
    }
    assert_eq!(widths.first(), widths.last());
    assert_eq!(layout, unchanged);
}

#[test]
/// A focused bottom-right floating resize grip moves width and height with Right/Down arrows and
/// then loses focus to Tab normally.
fn focused_floating_resize_grip_accepts_both_arrow_axes_and_releases_tab() {
    let ctx = Context::default();
    let mut layout = sample();
    for _ in 0..3 {
        step(&ctx, &mut layout, Vec::new());
    }
    let id = DockArea::hit_id(Id::unique("test-area"), &Hit::Resize(5u32, ResizeEdge::BottomRight));
    let layer = DockArea::floating_layer(Id::unique("test-area"), &layout.floating[0]).unwrap();
    let [x, y, width, height] = layout.floating[0].rect;
    assert_eq!(ctx.memory(|m| m.area_rect(layer.id)), Some(Rect::from_min_size(pos2(x, y), vec2(width, height))));
    ctx.memory_mut(|memory| memory.request_focus(id));
    let key = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    step(&ctx, &mut layout, vec![key(egui::Key::ArrowRight), key(egui::Key::ArrowDown)]);
    assert_eq!(layout.floating[0].rect, [80.0, 70.0, 308.0, 228.0]);
    step(&ctx, &mut layout, vec![key(egui::Key::Tab)]);
    assert_ne!(ctx.memory(|memory| memory.focused()), Some(id));
}

#[test]
/// A stack divider dragged by pointer grows the open body above and shrinks the adjacent open body
/// while closed entries and unrelated bodies keep their heights; keyboard arrows continue resizing.
fn stack_dividers_resize_adjacent_open_bodies_by_pointer_and_keyboard() {
    let ctx = Context::default();
    let mut layout = Layout {
        root: Some(Node::Stack {
            entries: vec![
                StackEntry { panel: 1, open: true, height: Some(150.0) },
                StackEntry { panel: 2, open: false, height: Some(91.0) },
                StackEntry { panel: 3, open: true, height: Some(150.0) },
                StackEntry { panel: 4, open: true, height: Some(120.0) },
            ],
        }),
        floating: Vec::new(),
    };
    for _ in 0..3 {
        step(&ctx, &mut layout, Vec::new());
    }
    let id = Id::unique("test-area").with(("stack-resize", &1u32));
    let start = ctx.read_response(id).unwrap().rect.center();
    step(&ctx, &mut layout, vec![Event::PointerMoved(start), pointer_button(start, true)]);
    step(&ctx, &mut layout, vec![Event::PointerMoved(start + vec2(0.0, 24.0))]);
    step(&ctx, &mut layout, vec![pointer_button(start + vec2(0.0, 24.0), false)]);
    let Node::Stack { entries } = layout.root.as_ref().unwrap() else { panic!() };
    let after_pointer = entries[0].height.unwrap();
    assert!(after_pointer > 150.0);
    assert!((entries[0].height.unwrap() + entries[2].height.unwrap() - 300.0).abs() < 0.001);
    assert_eq!(entries[1].height, Some(91.0));
    assert!(!entries[1].open);
    assert_eq!(entries[3].height, Some(120.0));
    ctx.memory_mut(|memory| memory.request_focus(id));
    step(
        &ctx,
        &mut layout,
        vec![Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
    );
    let Node::Stack { entries } = layout.root.as_ref().unwrap() else { panic!() };
    assert!(entries[0].height.unwrap() > after_pointer);
    assert!((entries[0].height.unwrap() + entries[2].height.unwrap() - 300.0).abs() < 0.001);
    // Dragged past the next body's minimum, the one after it gives way too, down to the same
    // minimum; the closed entry keeps its height.
    let start = ctx.read_response(id).unwrap().rect.center();
    step(&ctx, &mut layout, vec![Event::PointerMoved(start), pointer_button(start, true)]);
    step(&ctx, &mut layout, vec![Event::PointerMoved(start + vec2(0.0, 600.0))]);
    step(&ctx, &mut layout, vec![pointer_button(start + vec2(0.0, 600.0), false)]);
    let Node::Stack { entries } = layout.root.as_ref().unwrap() else { panic!() };
    let heights = entries.iter().map(|entry| entry.height.unwrap()).collect::<Vec<_>>();
    assert!((heights[2] - heights[3]).abs() < 0.01 && heights[3] < 120.0, "{heights:?}");
    assert!((heights[0] + heights[2] + heights[3] - 420.0).abs() < 0.01, "{heights:?}");
    assert_eq!(heights[1], 91.0);
}

#[test]
/// A horizontal split's inner edge inside a scrollable dock remains hit-testable and draggable next
/// to a scrollbar.
fn split_inner_edge_remains_draggable_beside_a_scrollbar() {
    let ctx = Context::default();
    let mut layout = Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedFirst(220.0),
            first: Box::new(tabs(&[1], 0)),
            second: Box::new(tabs(&[2], 0)),
        }),
        floating: Vec::new(),
    };
    let frame = |layout: &mut Layout<u32>, events| {
        let mut output = DockOutput::default();
        let mut sidebar = Rect::NOTHING;
        ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), events, ..Default::default() }, |ui| {
            output = DockArea::new(Id::unique("scrolling-dock")).show(
                ui,
                layout,
                &DockStyle::from_ui(ui),
                |p| format!("Panel {p}"),
                |_| Permissions::default(),
                |ui, panel| {
                    if *panel == 1 {
                        sidebar = ui.max_rect();
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                            .show(ui, |ui| {
                                for row in 0..100 {
                                    ui.label(format!("Library item {row}"));
                                }
                            });
                    } else {
                        ui.label("Document");
                    }
                },
            );
        })
        .textures_delta
        .clear();
        assert!(output.error.is_none());
        for action in output.actions {
            layout.apply(action).unwrap();
        }
        sidebar
    };
    for _ in 0..3 {
        frame(&mut layout, Vec::new());
    }
    let sidebar = frame(&mut layout, Vec::new());
    let start = pos2(sidebar.right() - 2.0, sidebar.center().y);
    frame(&mut layout, vec![Event::PointerMoved(start), pointer_button(start, true)]);
    frame(&mut layout, vec![Event::PointerMoved(start + vec2(40.0, 0.0))]);
    frame(&mut layout, vec![pointer_button(start + vec2(40.0, 0.0), false)]);
    let Node::Split { size: SplitSize::FixedFirst(width), .. } = layout.root.unwrap() else { panic!() };
    assert!((width - 260.0).abs() < 0.01, "sidebar width after inner-edge drag: {width}");
}

#[test]
/// Closing a panel in a nested vertical/horizontal layout and restoring its
/// saved location reproduces the complete layout.
fn restoring_a_panel_beside_a_nested_subtree_preserves_the_complete_workspace() {
    let mut layout = Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Vertical,
            size: SplitSize::FixedSecond(330.0),
            first: Box::new(tabs(&[1], 0)),
            second: Box::new(Node::Split {
                axis: SplitAxis::Horizontal,
                size: SplitSize::FixedFirst(220.0),
                first: Box::new(tabs(&[2], 0)),
                second: Box::new(tabs(&[3], 0)),
            }),
        }),
        floating: Vec::new(),
    };
    let expected = layout.clone();
    let location = layout.location(&1).unwrap();
    layout.apply(Action::Close { panel: 1 }).unwrap();
    layout.restore(1, &location).unwrap();
    assert_eq!(layout, expected);
}

#[test]
/// A layout with only floating windows leaves the caller's available document rect unchanged.
fn floating_only_surface_does_not_allocate_the_callers_document_region() {
    let ctx = Context::default();
    let layout = Layout { root: None, floating: vec![Floating::single(vec![1u32], 0, [30.0, 40.0, 200.0, 200.0])] };
    ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), ..Default::default() }, |ui| {
        let before = ui.available_rect_before_wrap();
        let out = DockArea::new(Id::unique("float-only")).show(
            ui,
            &layout,
            &DockStyle::from_ui(ui),
            |p| p.to_string(),
            |_| Permissions::default(),
            |ui, _| {
                ui.label("Panel");
            },
        );
        assert!(out.error.is_none());
        assert_eq!(ui.available_rect_before_wrap(), before);
    })
    .textures_delta
    .clear();
}

#[test]
/// Dragging the floating group grip emits `MoveFloating` and changes the window rect while the
/// group's panels and active panel stay the same; dragging a tab detaches only that panel.
fn floating_group_grip_moves_every_member_while_tab_drag_detaches_one() {
    let ctx = Context::default();
    let mut layout = sample();
    let original = layout.floating[0].clone();
    for _ in 0..2 {
        step(&ctx, &mut layout, vec![]);
    }
    let start = pos2(86.0, 82.0);
    step(&ctx, &mut layout, vec![Event::PointerMoved(start), pointer_button(start, true)]);
    let actions = step(&ctx, &mut layout, vec![Event::PointerMoved(start + vec2(50.0, 40.0))]);
    assert!(actions.iter().any(|a| matches!(a, Action::MoveFloating { .. })));
    assert_eq!(layout.floating[0].groups()[0].panels, original.groups()[0].panels);
    assert_eq!(layout.floating[0].groups()[0].active, original.groups()[0].active);
    assert_ne!(layout.floating[0].rect, original.rect);
    step(&ctx, &mut layout, vec![pointer_button(start + vec2(50.0, 40.0), false)]);
    for _ in 0..2 {
        step(&ctx, &mut layout, vec![]);
    }
    let tab = ctx.read_response(Id::unique("test-area").with(("tab", &5u32))).unwrap().rect.center();
    step(&ctx, &mut layout, vec![Event::PointerMoved(tab), pointer_button(tab, true)]);
    step(&ctx, &mut layout, vec![Event::PointerMoved(tab + vec2(8.0, 0.0))]);
    // Inside the source window the release is a no-op; outside it floats.
    let outside = pos2(780.0, 650.0);
    step(&ctx, &mut layout, vec![Event::PointerMoved(outside)]);
    let detached = step(&ctx, &mut layout, vec![pointer_button(outside, false)]);
    assert!(detached.iter().any(|a| matches!(a, Action::Float { panel: 5, .. })));
    assert_eq!(layout.floating.len(), 2);
    assert_eq!(layout.floating[0].groups()[0].panels, vec![6]);
}

#[test]
fn floating_group_drag_ends_on_release_or_pointer_exit() {
    for (released, leaves) in [(false, true), (true, true), (true, false)] {
        let ctx = Context::default();
        let mut layout = sample();
        for _ in 0..2 {
            step(&ctx, &mut layout, vec![]);
        }
        let start = pos2(86.0, 82.0);
        step(&ctx, &mut layout, vec![Event::PointerMoved(start), pointer_button(start, true)]);
        let moved = start + vec2(50.0, 40.0);
        let actions = step(&ctx, &mut layout, vec![Event::PointerMoved(moved)]);
        assert!(actions.iter().any(|action| matches!(action, Action::MoveFloating { .. })));
        let last = layout.clone();
        let mut events = vec![];
        if released {
            events.push(pointer_button(moved, false));
        }
        if leaves {
            events.push(Event::PointerGone);
        }
        let (out, _) = frame(&ctx, &layout, events, true);
        assert_eq!(out.group_drop, if leaves { None } else { Some((5, moved)) });
        for action in out.actions {
            layout.apply(action).unwrap();
        }
        assert_eq!(layout, last);
        for pointer in [moved + vec2(20.0, 20.0), moved + vec2(30.0, 30.0)] {
            let (out, _) = frame(&ctx, &layout, vec![Event::PointerMoved(pointer)], true);
            assert!(out.actions.is_empty(), "{out:?}");
            assert!(out.group_drag.is_none() && out.group_drop.is_none());
        }
        assert!(out.group_drag.is_none());
        assert_eq!(ctx.dragged_id(), None);
    }
}

#[test]
fn floating_group_touch_release_reports_drop_after_pointer_gone() {
    for web_order in [false, true] {
        let ctx = Context::default();
        let mut layout = sample();
        for _ in 0..2 {
            step(&ctx, &mut layout, vec![]);
        }
        let touch = |phase, pos| Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(1), phase, pos, force: None };
        let start = pos2(86.0, 82.0);
        let press = pointer_button(start, true);
        let events = if web_order {
            vec![press, touch(egui::TouchPhase::Start, start)]
        } else {
            vec![touch(egui::TouchPhase::Start, start), Event::PointerMoved(start), press]
        };
        step(&ctx, &mut layout, events);
        let moved = start + vec2(50.0, 40.0);
        let events = if web_order {
            vec![Event::PointerMoved(moved), touch(egui::TouchPhase::Move, moved)]
        } else {
            vec![touch(egui::TouchPhase::Move, moved), Event::PointerMoved(moved)]
        };
        assert!(step(&ctx, &mut layout, events).iter().any(|action| matches!(action, Action::MoveFloating { .. })));
        // egui-winit emits Touch before release/gone; eframe web emits it afterwards.
        let events = if web_order {
            vec![pointer_button(moved, false), Event::PointerGone, touch(egui::TouchPhase::End, moved)]
        } else {
            vec![touch(egui::TouchPhase::End, moved), pointer_button(moved, false), Event::PointerGone]
        };
        let (out, _) = frame(&ctx, &layout, events, true);
        assert!(!ctx.input(|i| i.pointer.has_pointer()));
        assert_eq!(out.group_drop, Some((5, moved)));
        assert!(out.group_drag.is_none());
        assert_eq!(ctx.dragged_id(), None);
        assert!(step(&ctx, &mut layout, vec![Event::PointerMoved(moved + vec2(30.0, 30.0))]).is_empty());
    }
}

#[test]
/// A floating group drag cancelled by pointer loss, with or without Escape, restores the
/// original floating rect and emits no group drag or drop.
fn floating_group_exit_with_escape_or_focus_loss_restores_original_geometry() {
    for escape in [false, true] {
        let ctx = Context::default();
        let mut layout = sample();
        let original = layout.clone();
        for _ in 0..2 {
            step(&ctx, &mut layout, vec![]);
        }
        let start = pos2(86.0, 82.0);
        step(&ctx, &mut layout, vec![Event::PointerMoved(start), pointer_button(start, true)]);
        step(&ctx, &mut layout, vec![Event::PointerMoved(start + vec2(50.0, 40.0))]);
        assert_ne!(layout.floating[0].rect, original.floating[0].rect);
        let mut events = vec![Event::PointerGone];
        if escape {
            events.push(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        }
        let (out, _) = frame(&ctx, &layout, events, escape);
        assert!(out.group_drag.is_none() && out.group_drop.is_none());
        for action in out.actions {
            layout.apply(action).unwrap();
        }
        assert_eq!(layout.floating[0].rect, original.floating[0].rect);
    }
}

struct CustomContent {
    labels: Vec<String>,
    headers: usize,
    menus: usize,
    bodies: usize,
}

#[test]
/// `cancel_drag` discards a floating group's move before the caller replaces the layout, so later
/// pointer movement and release emit nothing and the replacement stays intact.
fn host_cancel_discards_group_drag_before_workspace_replacement() {
    let ctx = Context::default();
    let mut layout = sample();
    for _ in 0..2 {
        step(&ctx, &mut layout, vec![]);
    }
    let start = pos2(86.0, 82.0);
    step(&ctx, &mut layout, vec![Event::PointerMoved(start), pointer_button(start, true)]);
    let moved = start + vec2(50.0, 40.0);
    let actions = step(&ctx, &mut layout, vec![Event::PointerMoved(moved)]);
    assert!(actions.iter().any(|action| matches!(action, Action::MoveFloating { .. })));

    cancel_drag::<u32>(&ctx, Id::unique("test-area"));
    layout = sample();
    let replacement = layout.clone();
    let actions = step(&ctx, &mut layout, vec![Event::PointerMoved(moved + vec2(20.0, 10.0))]);
    assert!(actions.is_empty());
    assert_eq!(layout, replacement);
    let mut content = CustomContent { labels: vec![], headers: 0, menus: 0, bodies: 0 };
    let output = custom_frame(&ctx, &layout, &mut content, vec![pointer_button(moved + vec2(20.0, 10.0), false)], &[], false);
    assert!(output.group_drag.is_none());
    assert!(output.group_drop.is_none());
}
impl DockContent<u32> for CustomContent {
    fn body(&mut self, ui: &mut egui::Ui, _: &u32) {
        self.bodies += 1;
        ui.label("Editable content");
    }
    fn label(&mut self, ui: &egui::Ui, _: &u32, logical: &str, font: &egui::FontId, color: egui::Color32) -> egui::WidgetText {
        self.labels.push(logical.to_owned());
        ui.painter().layout_no_wrap("Custom visual label".into(), font.clone(), color).into()
    }
    /// Custom visual action label that prefixes the logical text with
    /// `Action visual: `.
    fn action_label(&mut self, ui: &egui::Ui, logical: &str, font: &egui::FontId, color: egui::Color32) -> egui::WidgetText {
        ui.painter().layout_no_wrap(format!("Action visual: {logical}"), font.clone(), color).into()
    }
    /// Record an app menu draw and emit a clickable `App menu` button.
    fn panel_menu(&mut self, ui: &mut egui::Ui, _: &u32) {
        self.menus += 1;
        ui.button("App menu").clicked();
    }
    /// Test custom chrome declares a 40 pt group header width.
    fn group_header_width(&self, _: &[u32], _: &u32, _: bool, _: f32) -> f32 {
        40.0
    }
    /// Custom header: records the call, asserts it receives at most the declared 40 pt width, and
    /// draws its own glyph.
    fn group_header(&mut self, ui: &mut egui::Ui, _: &[u32], _: &u32, _: bool) {
        self.headers += 1;
        assert!(ui.max_rect().width() <= 40.0);
        ui.label("…");
    }
}

/// Run one 800×600 customized dock frame with optional blockers and a
/// programmatic raise of panel 5; returns the output with texture deltas cleared.
fn custom_frame(
    ctx: &Context,
    layout: &Layout<u32>,
    content: &mut CustomContent,
    events: Vec<Event>,
    blockers: &[Rect],
    raise: bool,
) -> DockOutput<u32> {
    let mut out = DockOutput::default();
    ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), events, ..Default::default() }, |ui| {
        let style = DockStyle::from_ui(ui);
        let mut area = DockArea::new(Id::unique("test-area")).blocked_rects(blockers);
        if raise {
            area = area.raise_panel(5);
        }
        out = area.show_customized(ui, layout, &style, |p| format!("Logical {p}"), |_| Permissions::default(), |_| PanelLimits::default(), content);
    })
    .textures_delta
    .clear();
    out
}

#[test]
/// Custom chrome receives logical labels, draws custom headers and bodies and app-menu
/// popups, while a programmatic raise sets no user focus or focus ring and emits
/// no action.
fn custom_chrome_shaped_labels_and_programmatic_raise_are_available() {
    let ctx = Context::default();
    let layout = sample();
    let mut content = CustomContent { labels: vec![], headers: 0, menus: 0, bodies: 0 };
    for _ in 0..2 {
        custom_frame(&ctx, &layout, &mut content, vec![], &[], false);
    }
    let out = custom_frame(&ctx, &layout, &mut content, vec![], &[], true);
    assert_eq!(out.focused, None, "a programmatic raise is not user focus");
    assert_eq!(ctx.memory(|m| m.focused()), None, "a raise draws no keyboard-focus ring");
    assert!(out.actions.is_empty());
    assert!(content.headers >= 3 && content.bodies >= 3);
    assert!(content.labels.iter().any(|s| s == "Logical 5"));
    let at = pos2(130.0, 96.0);
    custom_frame(
        &ctx,
        &layout,
        &mut content,
        vec![Event::PointerMoved(at), Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE }],
        &[],
        false,
    );
    custom_frame(
        &ctx,
        &layout,
        &mut content,
        vec![Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE }],
        &[],
        false,
    );
    assert!(content.menus > 0);
}

#[test]
/// An overlay appearing mid-drag blocks the release's float fallback; later frames emit no action.
fn overlay_appearing_mid_drag_blocks_release_and_float_fallback() {
    let ctx = Context::default();
    let layout = sample();
    let mut content = CustomContent { labels: vec![], headers: 0, menus: 0, bodies: 0 };
    let source = Rect::from_min_size(pos2(80.0, 70.0), vec2(300.0, 220.0));
    custom_frame(&ctx, &layout, &mut content, vec![Event::PointerMoved(pos2(130.0, 82.0)), pointer_button(pos2(130.0, 82.0), true)], &[], false);
    assert!(begin_drag(&ctx, Id::unique("test-area"), 5, source));
    let at = pos2(700.0, 400.0);
    let blocker = Rect::from_min_size(pos2(650.0, 350.0), vec2(100.0, 100.0));
    let out = custom_frame(&ctx, &layout, &mut content, vec![Event::PointerMoved(at), pointer_button(at, false)], &[blocker], false);
    assert!(out.actions.is_empty());
    let out = custom_frame(&ctx, &layout, &mut content, vec![], &[], false);
    assert!(out.actions.is_empty());
}

#[test]
/// Escape during a floating group move restores the original floating rect.
fn group_move_escape_restores_original_geometry() {
    let ctx = Context::default();
    let mut layout = sample();
    let original = layout.floating[0].rect;
    for _ in 0..2 {
        step(&ctx, &mut layout, vec![]);
    }
    let start = pos2(86.0, 82.0);
    step(&ctx, &mut layout, vec![Event::PointerMoved(start), pointer_button(start, true)]);
    step(&ctx, &mut layout, vec![Event::PointerMoved(start + vec2(50.0, 40.0))]);
    assert_ne!(layout.floating[0].rect, original);
    step(
        &ctx,
        &mut layout,
        vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
    );
    assert_eq!(layout.floating[0].rect, original);
}

#[test]
/// Custom headers and labels render at 0.75×, 1×, 1.5× and 2×: headers and bodies are produced and
/// panel 6's label is rendered with its logical text.
fn custom_headers_and_labels_render_at_small_and_large_scales() {
    for scale in [0.75, 1.0, 1.5, 2.0] {
        let ctx = Context::default();
        ctx.set_pixels_per_point(scale);
        let mut content = CustomContent { labels: vec![], headers: 0, menus: 0, bodies: 0 };
        let layout = Layout { root: Some(tabs(&[1, 2, 3, 4, 5, 6], 5)), floating: vec![] };
        let mut out = DockOutput::default();
        ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(120.0 * scale, 160.0))), ..Default::default() }, |ui| {
            let style = DockStyle::from_ui(ui);
            out = DockArea::new(Id::unique("scaled")).show_customized(
                ui,
                &layout,
                &style,
                |p| format!("Logical {p}"),
                |_| Permissions::default(),
                |_| PanelLimits::default(),
                &mut content,
            );
        })
        .textures_delta
        .clear();
        assert!(out.error.is_none());
        assert!(content.headers > 0 && content.bodies > 0);
        assert!(content.labels.iter().any(|s| s == "Logical 6"));
    }
}

#[test]
/// A protected member blocks whole-group gestures and host drops: drag/drop state stays empty and
/// no `MoveFloating` action is emitted.
fn protected_group_member_blocks_whole_group_gesture_and_host_drop() {
    let ctx = Context::default();
    let layout = sample();
    let start = pos2(86.0, 82.0);
    for events in [
        vec![],
        vec![],
        vec![Event::PointerMoved(start), pointer_button(start, true)],
        vec![Event::PointerMoved(pos2(300.0, 300.0))],
        vec![pointer_button(pos2(300.0, 300.0), false)],
    ] {
        let mut out = DockOutput::default();
        ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), events, ..Default::default() }, |ui| {
            let style = DockStyle::from_ui(ui);
            out = DockArea::new(Id::unique("test-area")).show(
                ui,
                &layout,
                &style,
                |p| format!("Panel {p}"),
                |p| if *p == 6 { Permissions::PROTECTED } else { Permissions::default() },
                |ui, _| {
                    ui.label("Body");
                },
            );
        })
        .textures_delta
        .clear();
        assert!(out.group_drag.is_none() && out.group_drop.is_none());
        assert!(!out.actions.iter().any(|a| matches!(a, Action::MoveFloating { .. })));
    }
}

/// Drags the floating group's resize grip in `sample()` and returns every action emitted.
fn resize_floating_group(permissions: impl Fn(&u32) -> Permissions + Copy) -> Vec<Action<u32>> {
    let ctx = Context::default();
    let layout = sample();
    // sample() floats 5 and 6 at [80, 70, 300, 220]; the grip is its bottom-right corner.
    let (start, end) = (pos2(374.0, 284.0), pos2(398.0, 308.0));
    let mut actions = Vec::new();
    for events in [
        vec![],
        vec![],
        vec![Event::PointerMoved(start), pointer_button(start, true)],
        vec![Event::PointerMoved(end)],
        vec![pointer_button(end, false)],
    ] {
        let mut out = DockOutput::default();
        ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), events, ..Default::default() }, |ui| {
            let style = DockStyle::from_ui(ui);
            out = DockArea::new(Id::unique("resize-area")).show(
                ui,
                &layout,
                &style,
                |p| format!("Panel {p}"),
                permissions,
                |ui, _| {
                    ui.label("Body");
                },
            );
        })
        .textures_delta
        .clear();
        actions.extend(out.actions);
    }
    actions
}

#[test]
/// A floating group's grip resizes a movable group, but a group containing
/// protected panel 6 blocks the `MoveFloating` action.
fn protected_group_member_blocks_floating_group_resize() {
    let moved = |actions: &[Action<u32>]| actions.iter().any(|a| matches!(a, Action::MoveFloating { .. }));
    assert!(moved(&resize_floating_group(|_| Permissions::default())), "the grip resizes a movable group");
    assert!(!moved(&resize_floating_group(|p| if *p == 6 { Permissions::PROTECTED } else { Permissions::default() })));
}

#[test]
/// Chrome focus recognizes a rendered tab but rejects a focused body text field, an invalid layout,
/// an empty layout and an unrendered id.
fn chrome_focus_classifies_rendered_tab_but_excludes_body_text_field_and_invalid_layout() {
    let ctx = Context::default();
    let layout = sample();
    let tab = Id::unique("focus-area").with(("tab", 3u32));
    let text = Id::unique("body-text");
    let mut value = String::new();
    let mut paint = |focus: Id| {
        let mut output =
            ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), ..Default::default() }, |ui| {
                DockArea::new(Id::unique("focus-area")).show(
                    ui,
                    &layout,
                    &DockStyle::from_ui(ui),
                    |p| format!("Panel {p}"),
                    |_| Permissions::default(),
                    |ui, p| {
                        if *p == 3 {
                            ui.add(egui::TextEdit::singleline(&mut value).id(text));
                        }
                    },
                );
                ui.memory_mut(|memory| memory.request_focus(focus));
            });
        output.textures_delta.clear();
    };
    paint(tab);
    assert!(DockArea::new(Id::unique("focus-area")).chrome_has_focus(&ctx, &layout));
    paint(text);
    assert!(!DockArea::new(Id::unique("focus-area")).chrome_has_focus(&ctx, &layout));
    paint(tab);
    let mut invalid = layout.clone();
    invalid.floating.push(Floating::single(vec![3], 0, [10.0, 10.0, 100.0, 100.0]));
    assert!(!DockArea::new(Id::unique("focus-area")).chrome_has_focus(&ctx, &invalid));
    assert!(!DockArea::<u32>::new(Id::unique("focus-area")).chrome_has_focus(&ctx, &Layout::default()));
    ctx.memory_mut(|memory| memory.request_focus(Id::unique("unrendered")));
    assert!(!DockArea::new(Id::unique("focus-area")).chrome_has_focus(&ctx, &layout));
}

#[test]
/// Chrome focus recognizes rendered tabs, collapse buttons, stack resizers, split dividers and
/// floating resizers.
fn chrome_focus_classifies_rendered_accordion_and_split_and_floating_resizers() {
    let ctx = Context::default();
    let area = Id::unique("focus-chrome-controls");
    let accordion = Layout {
        root: Some(Node::Stack {
            entries: vec![StackEntry { panel: 1u32, open: true, height: Some(150.0) }, StackEntry { panel: 2u32, open: true, height: Some(150.0) }],
        }),
        floating: Vec::new(),
    };
    for (layout, id) in [
        (accordion.clone(), area.with(("tab", 1u32))),
        (accordion.clone(), area.with(("collapse", 1u32))),
        (accordion, area.with(("stack-resize", 1u32))),
        (sample(), area.with(("split", Vec::<bool>::new()))),
        (sample(), area.with(("floating", 5u32)).with("resize")),
    ] {
        let mut output =
            ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), ..Default::default() }, |ui| {
                DockArea::new(area).show(
                    ui,
                    &layout,
                    &DockStyle::from_ui(ui),
                    |p| format!("Panel {p}"),
                    |_| Permissions::default(),
                    |ui, _| {
                        ui.label("Body");
                    },
                );
                ui.memory_mut(|memory| memory.request_focus(id));
            });
        output.textures_delta.clear();
        assert!(ctx.read_response(id).is_some(), "the fixture must render the control");
        assert!(DockArea::new(area).chrome_has_focus(&ctx, &layout));
    }
}

#[test]
/// `cancel_drag` ends an owned split/stack/floating resize until a new press: held movement stops
/// producing actions and the layout stays at the cancellation snapshot, then a fresh press resizes
/// again.
fn workspace_cancel_stops_owned_resize_controls_until_new_press() {
    let area = Id::unique("test-area");
    let fixtures = [
        (sample(), area.with(("split", Vec::<bool>::new())), vec2(24.0, 0.0)),
        (
            Layout {
                root: Some(Node::Stack {
                    entries: vec![StackEntry { panel: 1, open: true, height: Some(150.0) }, StackEntry { panel: 2, open: true, height: Some(150.0) }],
                }),
                floating: vec![],
            },
            area.with(("stack-resize", &1u32)),
            vec2(0.0, 24.0),
        ),
        (sample(), area.with(("floating", &5u32)).with("resize"), vec2(24.0, 24.0)),
    ];
    for (mut layout, id, delta) in fixtures {
        let ctx = Context::default();
        for _ in 0..3 {
            step(&ctx, &mut layout, vec![]);
        }
        let start = ctx.read_response(id).unwrap().rect.center();
        step(&ctx, &mut layout, vec![Event::PointerMoved(start), pointer_button(start, true)]);
        let actions = step(&ctx, &mut layout, vec![Event::PointerMoved(start + delta)]);
        assert!(!actions.is_empty(), "gesture fixture must actually resize: {id:?}");
        assert_eq!(ctx.dragged_id(), Some(id));
        // Keep all identities present in a replacement snapshot.
        let replacement = layout.clone();
        cancel_drag::<u32>(&ctx, area);
        assert_eq!(ctx.dragged_id(), None);
        for multiplier in [2.0, 3.0] {
            let actions = step(&ctx, &mut layout, vec![Event::PointerMoved(start + delta * multiplier)]);
            assert!(actions.is_empty(), "held old gesture produced {actions:?}");
            assert_eq!(layout, replacement);
        }
        step(&ctx, &mut layout, vec![pointer_button(start + delta * 3.0, false)]);
        for _ in 0..2 {
            step(&ctx, &mut layout, vec![]);
        }
        let start = ctx.read_response(id).unwrap().rect.center();
        step(&ctx, &mut layout, vec![Event::PointerMoved(start), pointer_button(start, true)]);
        assert!(!step(&ctx, &mut layout, vec![Event::PointerMoved(start + delta)]).is_empty());
    }
}

#[test]
/// Cancelling one area's drag leaves another area's split divider or a body-owned drag in control,
/// and that gesture continues producing its effects.
fn workspace_cancel_preserves_other_area_and_body_drag_ownership() {
    for body_drag in [false, true] {
        let ctx = Context::default();
        let area = Id::unique("cancel-area");
        let other = Id::unique("other-area");
        let layout = Layout {
            root: Some(Node::Split {
                axis: SplitAxis::Horizontal,
                size: SplitSize::FixedFirst(180.0),
                first: Box::new(tabs(&[1], 0)),
                second: Box::new(tabs(&[2], 0)),
            }),
            floating: vec![],
        };
        let body_id = Id::unique("application-body-drag");
        let mut value = 0.0;
        let mut run = |events: Vec<Event>| {
            let mut outputs = Vec::new();
            ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), events, ..Default::default() }, |ui| {
                let style = DockStyle::from_ui(ui);
                for (id, x) in [(area, 0.0), (other, 400.0)] {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(x, 0.0), vec2(390.0, 580.0))));
                    outputs.push(DockArea::new(id).show(
                        &mut child,
                        &layout,
                        &style,
                        |p| format!("Panel {p}"),
                        |_| Permissions::default(),
                        |ui, p| {
                            if body_drag && id == area && *p == 1 {
                                let rect = Rect::from_min_size(ui.max_rect().min + vec2(10.0, 10.0), vec2(100.0, 40.0));
                                let response = ui.interact(rect, body_id, egui::Sense::drag());
                                if response.dragged() {
                                    value += response.drag_delta().x;
                                }
                            }
                        },
                    ));
                }
            })
            .textures_delta
            .clear();
            outputs
        };
        for _ in 0..3 {
            run(vec![]);
        }
        let owned = if body_drag { body_id } else { other.with(("split", Vec::<bool>::new())) };
        let start = ctx.read_response(owned).unwrap().rect.center();
        run(vec![Event::PointerMoved(start), pointer_button(start, true)]);
        run(vec![Event::PointerMoved(start + vec2(20.0, 0.0))]);
        assert_eq!(ctx.dragged_id(), Some(owned));
        cancel_drag::<u32>(&ctx, area);
        assert_eq!(ctx.dragged_id(), Some(owned));
        let outputs = run(vec![Event::PointerMoved(start + vec2(40.0, 0.0))]);
        assert_eq!(ctx.dragged_id(), Some(owned));
        if body_drag {
            assert!(value >= 40.0);
        } else {
            assert!(!outputs[1].actions.is_empty());
        }
    }
}

#[test]
/// Custom action labels paint their visual substitution while the popup buttons keep their own
/// logical accessibility names; the visual text is not exposed as an accessibility label.
fn popup_action_labels_use_custom_visual_text_and_logical_accessibility() {
    use egui_kittest::{Harness, kittest::Queryable};
    /// Recursively search a shape or shape vector for a text galley whose text is
    /// exactly `text`.
    fn paints(shape: &egui::Shape, text: &str) -> bool {
        match shape {
            egui::Shape::Text(shape) => shape.galley.text() == text,
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| paints(shape, text)),
            _ => false,
        }
    }
    let layout = sample();
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        move |ui, content: &mut CustomContent| {
            DockArea::new(Id::unique("action-label-test")).show_customized(
                ui,
                &layout,
                &DockStyle::from_ui(ui),
                |p| format!("Logical {p}"),
                |_| Permissions::default(),
                |_| PanelLimits::default(),
                content,
            );
        },
        CustomContent { labels: vec![], headers: 0, menus: 0, bodies: 0 },
    );
    h.run_steps(2);
    let tab = Id::unique("action-label-test").with(("tab", &6_u32));
    let at = h.ctx.read_response(tab).unwrap().rect.center();
    h.input_mut().events.extend([
        Event::PointerMoved(at),
        Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(1);
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(egui::Popup::is_any_open(&h.ctx));
    for logical in ["Float Panel", "Move Panel to Group", "Close Panel"] {
        assert!(
            h.output().shapes.iter().any(|shape| paints(&shape.shape, &format!("Action visual: {logical}"))),
            "custom action must paint: {logical}"
        );
        assert_eq!(h.query_all_by_label(logical).count(), 1, "popup action must retain its own logical accessibility: {logical}");
        assert_eq!(
            h.query_all_by_label(&format!("Action visual: {logical}")).count(),
            0,
            "visual substitution must not replace logical accessibility"
        );
    }
}

#[test]
/// At the minimum tab width, only the tabs that fit beside the drag bar and overflow control are
/// shown, the active last tab replaces the last fitting tab, and the overflow menu lists exactly the
/// hidden panels.
fn a_strip_too_narrow_for_its_tabs_at_their_minimum_width_lists_the_others_in_the_overflow_menu() {
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};
    let area = Id::unique("whole-tabs");
    let layout = Layout { root: Some(tabs(&[1, 2, 3, 4, 5, 6], 5)), floating: vec![] };
    let mut h = Harness::builder().with_size(vec2(260.0, 240.0)).build_ui(move |ui| {
        DockArea::new(area).show(
            ui,
            &layout,
            &DockStyle::from_ui(ui),
            |panel| format!("Panel {panel}"),
            |_| Permissions::default(),
            |_: &mut egui::Ui, _: &u32| {},
        );
    });
    h.run_steps(2);
    let shown: Vec<u32> = (1..=6).filter(|p| h.ctx.read_response(area.with(("tab", p))).is_some()).collect();
    assert!((2..6).contains(&shown.len()), "{shown:?}");
    // The active last tab takes the place of the last tab that fits; the others keep their order.
    let mut expected: Vec<u32> = (1..shown.len() as u32).collect();
    expected.push(6);
    assert_eq!(shown, expected);
    // As many tabs show as fit at the minimum width beside the drag bar and the "…" button.
    let style = DockStyle::default();
    let room = 260.0 - style.drag_bar_width - 22.0;
    assert!(shown.len() as f32 * style.min_tab_width <= room && (shown.len() + 1) as f32 * style.min_tab_width > room, "{shown:?}");
    h.get_by_role_and_label(Role::Button, "Panels").click();
    h.run_steps(2);
    let listed: Vec<u32> = (1..=6).filter(|p| h.query_by_role_and_label(Role::Button, &format!("Panel {p}")).is_some()).collect();
    let hidden: Vec<u32> = (1..=6).filter(|p| !shown.contains(p)).collect();
    assert_eq!(listed, hidden, "the overflow menu lists exactly the hidden panels");
}

#[test]
/// Long tab labels are truncated with an ellipsis inside their tab and expose the full label in a
/// tooltip on hover.
fn tab_labels_that_do_not_fit_are_truncated_inside_their_tab_and_named_in_a_tooltip() {
    let ctx = Context::default();
    ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
    let layout = Layout { root: Some(tabs(&[1, 2, 3], 0)), floating: vec![] };
    let label = |p: &u32| format!("Eigenschaften und Anpassungen {p}");
    let run = |events: Vec<Event>| {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(240.0, 200.0))), events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            DockArea::new(Id::unique("truncate")).show(
                ui,
                &layout,
                &DockStyle::from_ui(ui),
                label,
                |_| Permissions::default(),
                |ui, _| {
                    ui.label("Body");
                },
            );
        });
        output.textures_delta.clear();
        output.shapes
    };
    run(vec![]);
    let shapes = run(vec![]);
    let tab = |p: u32| ctx.read_response(Id::unique("truncate").with(("tab", &p))).expect("every tab shrinks to fit").rect;
    for p in 1..=3 {
        let tab = tab(p);
        // The clip rect would hide an overflowing label; what is laid out must itself fit.
        let painted: Vec<&egui::epaint::TextShape> = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if tab.contains(text.visual_bounding_rect().center()) => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(painted.len(), 1, "tab {p}: {painted:?}");
        assert!(painted[0].galley.elided, "tab {p}");
        assert!(tab.contains_rect(painted[0].visual_bounding_rect()), "tab {p}: label {:?} overflows {tab:?}", painted[0].visual_bounding_rect());
    }
    run(vec![Event::PointerMoved(tab(2).center())]);
    let shapes = run(vec![]);
    let full = label(&2);
    // An elided galley still reports its whole text, so the cut tab label alone would match.
    assert!(
        shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == full && !text.galley.elided)),
        "a truncated tab names its panel in full on hover"
    );
}

// Ported from storytold/craft-libs#110 by @Zenodeon.

#[test]
/// A floating resize keeps drag ownership when another host control rewrites the owned-interaction
/// list; `cancel_drag` then ends it and further movement is ignored.
fn resize_cancellation_retains_ownership_across_other_host_rendering() {
    let ctx = Context::default();
    let area = Id::unique("test-area");
    let mut layout = sample();
    for _ in 0..3 {
        step(&ctx, &mut layout, vec![]);
    }
    let id = area.with(("floating", &5u32)).with("resize-right");
    let start = ctx.read_response(id).unwrap().rect.center();
    step(&ctx, &mut layout, vec![Event::PointerMoved(start), pointer_button(start, true)]);
    step(&ctx, &mut layout, vec![Event::PointerMoved(start + vec2(20.0, 0.0))]);
    assert_eq!(ctx.dragged_id(), Some(id));
    ctx.data_mut(|data| data.insert_temp(area.with("owned-interactions"), vec![Id::unique("another-host-control")]));
    cancel_drag::<u32>(&ctx, area);
    assert!(ctx.dragged_id().is_none());
    let before = layout.clone();
    assert!(step(&ctx, &mut layout, vec![Event::PointerMoved(start + vec2(100.0, 0.0))]).is_empty());
    assert_eq!(layout, before);
}

#[test]
/// A collapsed 26 pt floating window still resizes its width with the Right arrow under a height
/// cap, growing to the minimum height instead of ignoring the gesture.
fn collapsed_floating_height_cap_does_not_disable_keyboard_width_resizing() {
    let ctx = Context::default();
    let mut layout = Layout { root: None, floating: vec![Floating::single(vec![1u32], 0, [80.0, 70.0, 300.0, 26.0])] };
    let id = Id::unique("collapsed-resize").with(("floating", &1u32)).with("resize-right");
    let mut frame = |events| {
        let mut actions = Vec::new();
        ctx.run_ui(
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), focused: true, events, ..Default::default() },
            |ui| {
                let mut style = DockStyle::from_ui(ui);
                style.min_pane = 0.0;
                actions = DockArea::new(Id::unique("collapsed-resize"))
                    .show_with_limits(
                        ui,
                        &layout,
                        &style,
                        |p| p.to_string(),
                        |_| Permissions::default(),
                        |_| PanelLimits { min: vec2(220.0, 0.0), max: vec2(1000.0, 0.0) },
                        |_, _| {},
                    )
                    .actions;
            },
        )
        .textures_delta
        .clear();
        for action in actions {
            layout.apply(action).unwrap();
        }
    };
    for _ in 0..3 {
        frame(vec![]);
    }
    ctx.memory_mut(|memory| memory.request_focus(id));
    frame(vec![Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    assert_eq!(layout.floating[0].rect, [80.0, 70.0, 308.0, 44.0]);
}

#[test]
/// Dropping a panel back on its own singleton or multi-tab source body is an
/// explicit no-op with no insertion marker, with or without split permission.
fn returning_to_a_singleton_or_floating_source_body_is_an_explicit_noop() {
    for panels in [vec![1u32], vec![1, 2, 3]] {
        for allow_splits in [false, true] {
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(300.0, 300.0));
            let target = DropTarget {
                anchor: 1,
                rect,
                tab_strip: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 26.0))),
                tabs: panels.iter().map(|id| (*id, Rect::NOTHING)).collect(),
                allow_splits,
                accepts_tabs: true,
                right_to_left: false,
            };
            let proposal = resolve_drop(&1, rect.center(), &[target], 40.0).unwrap();
            assert!(proposal.noop);
            assert!(proposal.insertion.is_none());
        }
    }
}

#[test]
/// Dragging each floating resize grip with the pointer past a viewport edge leaves every asserted
/// edge unchanged when the pointer moves partway back while still outside, and returning to the
/// start point restores the original geometry. `resize-right` and `resize` assert the clamped
/// right edge and fixed left edge; `resize-left` and `resize-bottom-left` assert only their fixed
/// right edge. `resize-bottom` asserts the clamped bottom edge and fixed top edge.
fn pointer_floating_resize_stops_at_edges_and_waits_for_cursor_to_return() {
    for name in ["resize-right", "resize-left", "resize-bottom", "resize", "resize-bottom-left"] {
        let ctx = Context::default();
        let mut layout = sample();
        for _ in 0..3 {
            step(&ctx, &mut layout, vec![]);
        }
        let id = Id::unique("test-area").with(("floating", &5u32)).with(name);
        let start = ctx.read_response(id).unwrap().rect.center();
        let original = layout.floating[0].rect;
        let horizontal = name != "resize-bottom";
        let left = name.contains("left");
        step(&ctx, &mut layout, vec![Event::PointerMoved(start), pointer_button(start, true)]);
        let outside = if horizontal { pos2(if left { -200.0 } else { 1100.0 }, start.y) } else { pos2(start.x, 1000.0) };
        step(&ctx, &mut layout, vec![Event::PointerMoved(outside)]);
        let stopped = layout.floating[0].rect;
        if horizontal {
            if left {
                assert_eq!(stopped[0] + stopped[2], original[0] + original[2]);
            } else {
                assert_eq!(stopped[0], original[0]);
                assert_eq!(stopped[0] + stopped[2], 800.0);
            }
        } else {
            assert_eq!(stopped[1], original[1]);
            assert_eq!(stopped[1] + stopped[3], 600.0);
        }
        let still_outside = if horizontal { outside + vec2(if left { 100.0 } else { -100.0 }, 0.0) } else { outside - vec2(0.0, 100.0) };
        step(&ctx, &mut layout, vec![Event::PointerMoved(still_outside)]);
        assert_eq!(layout.floating[0].rect, stopped, "{name}: reversal outside the edge must not resize");
        step(&ctx, &mut layout, vec![Event::PointerMoved(start)]);
        assert_eq!(layout.floating[0].rect, original, "{name}: returning to the original handle restores its geometry");
        step(&ctx, &mut layout, vec![pointer_button(start, false)]);
        assert!(ctx.memory(|memory| memory.focused()).is_none(), "{name}: the grip gives up focus after a pointer drag");
    }
}

#[test]
/// Dropping a panel within its existing tab slots is a no-op whose insertion
/// marker stays inside the strip; positions outside its own slots move it.
fn unchanged_tab_slots_are_explicit_noops_with_bounded_insertion_markers() {
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 200.0));
    let target = DropTarget {
        anchor: 1,
        rect,
        tab_strip: Some(Rect::from_min_size(rect.min, vec2(300.0, 30.0))),
        tabs: vec![
            (1, Rect::from_min_size(pos2(0.0, 0.0), vec2(60.0, 30.0))),
            (2, Rect::from_min_size(pos2(60.0, 0.0), vec2(60.0, 30.0))),
            (3, Rect::from_min_size(pos2(120.0, 0.0), vec2(60.0, 30.0))),
        ],
        allow_splits: true,
        accepts_tabs: true,
        right_to_left: false,
    };
    for (panel, positions) in [(1, [10.0, 50.0]), (2, [70.0, 110.0]), (3, [130.0, 210.0])] {
        for x in positions {
            let proposal = resolve_drop(&panel, pos2(x, 12.0), std::slice::from_ref(&target), 40.0).unwrap();
            assert!(proposal.noop, "{panel}/{x}");
            assert!(target.tab_strip.unwrap().contains_rect(proposal.insertion.unwrap()));
        }
    }
    for (panel, x) in [(1, 140.0), (3, 10.0), (2, 210.0)] {
        let proposal = resolve_drop(&panel, pos2(x, 12.0), std::slice::from_ref(&target), 40.0).unwrap();
        assert!(!proposal.noop);
        assert!(proposal.insertion.is_some());
    }
}

#[test]
/// A host drag handler that accepts the release as a no-op consumes it, so no fallback float is
/// emitted and the host observes one release.
fn host_owned_noop_consumes_release_without_a_fallback_float() {
    struct Content(usize);
    impl DockContent<u32> for Content {
        fn body(&mut self, _ui: &mut egui::Ui, _panel: &u32) {}
        /// Test drop handler that counts releases and accepts every request with
        /// default feedback, consuming host-owned drops.
        fn resolve_drag(&mut self, _ctx: &Context, _style: &DockStyle, request: &DropRequest<'_, u32>) -> Option<DropFeedback> {
            if request.released {
                self.0 += 1;
            }
            Some(DropFeedback::default())
        }
    }
    let ctx = Context::default();
    let area = Id::unique("host-owned");
    let layout = sample();
    let mut content = Content(0);
    let from = pos2(30.0, 20.0);
    let to = pos2(400.0, 300.0);
    let render = |events: Vec<Event>, content: &mut Content| {
        let mut output = DockOutput::default();
        ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() }, |ui| {
            output = DockArea::new(area).show_customized(
                ui,
                &layout,
                &DockStyle::from_ui(ui),
                |p| p.to_string(),
                |_| Permissions::default(),
                |_| PanelLimits::default(),
                content,
            );
        })
        .textures_delta
        .clear();
        output
    };
    render(
        vec![
            Event::PointerMoved(from),
            Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        ],
        &mut content,
    );
    assert!(begin_drag(&ctx, area, 1u32, Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0))));
    render(vec![Event::PointerMoved(to)], &mut content);
    let output = render(
        vec![Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }],
        &mut content,
    );
    assert_eq!(content.0, 1);
    assert!(output.actions.iter().all(|action| !matches!(action, Action::Move { .. } | Action::Float { .. })));
}

#[test]
fn overflow_noop_detection_uses_logical_order_including_hidden_tabs() {
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(140.0, 200.0));
    let target = DropTarget {
        anchor: 1,
        rect,
        tab_strip: Some(Rect::from_min_size(Pos2::ZERO, vec2(140.0, 30.0))),
        tabs: vec![
            (1, Rect::from_min_size(Pos2::ZERO, vec2(60.0, 30.0))),
            (2, Rect::NOTHING),
            (3, Rect::from_min_size(pos2(60.0, 0.0), vec2(60.0, 30.0))),
        ],
        allow_splits: true,
        accepts_tabs: true,
        right_to_left: false,
    };
    assert!(resolve_drop(&1, pos2(10.0, 12.0), std::slice::from_ref(&target), 40.0).unwrap().noop);
    // Before the next visible tab crosses hidden tab 2 and really changes the order.
    assert!(!resolve_drop(&1, pos2(65.0, 12.0), std::slice::from_ref(&target), 40.0).unwrap().noop);
    assert!(resolve_drop(&3, pos2(130.0, 12.0), std::slice::from_ref(&target), 40.0).unwrap().noop);
}

#[test]
/// For clipped and singleton strips of several widths, a no-op drop keeps its
/// insertion marker inside the visible strip and at most 2 pt wide.
fn insertion_markers_stay_inside_clipped_and_singleton_strips() {
    for width in [0.5, 1.0, 20.0, 140.0] {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(width, 200.0));
        let strip = Rect::from_min_size(rect.min, vec2(width, 30.0));
        for tab_left in [-100.0, 10.0, 300.0] {
            let target = DropTarget {
                anchor: 1,
                rect,
                tab_strip: Some(strip.expand(5.0)),
                tabs: vec![(1, Rect::from_min_size(pos2(tab_left, 20.0), vec2(60.0, 30.0)))],
                allow_splits: true,
                accepts_tabs: true,
                right_to_left: false,
            };
            for x in [rect.left(), rect.center().x, rect.right()] {
                let proposal = resolve_drop(&1, pos2(x, 25.0), std::slice::from_ref(&target), 40.0).unwrap();
                assert!(proposal.noop);
                let marker = proposal.insertion.unwrap();
                assert!(rect.intersect(target.tab_strip.unwrap()).contains_rect(marker));
                assert!(marker.width() > 0.0 && marker.width() <= 2.0);
            }
        }
    }
}

#[test]
/// A subpixel-wide tab strip crossing zero still yields a no-op insertion
/// marker contained by the strip.
fn insertion_marker_handles_subpixel_strip_crossing_zero_without_panicking() {
    let rect = Rect::from_min_max(pos2(-0.001, 0.0), pos2(0.002, 30.0));
    let target =
        DropTarget { anchor: 1, rect, tab_strip: Some(rect), tabs: vec![(1, rect)], allow_splits: true, accepts_tabs: true, right_to_left: false };
    let proposal = resolve_drop(&1, pos2(0.0, 12.0), &[target], 40.0).unwrap();
    assert!(proposal.noop);
    assert!(rect.contains_rect(proposal.insertion.unwrap()));
}

#[test]
/// A strip narrower than its tabs paints at least one ellipsis overflow symbol.
fn overflowing_strip_paints_a_single_ellipsis_symbol() {
    use egui_kittest::Harness;
    fn paints(shape: &egui::Shape, text: &str) -> bool {
        match shape {
            egui::Shape::Text(shape) => shape.galley.text() == text,
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| paints(shape, text)),
            _ => false,
        }
    }
    let layout = Layout { root: Some(tabs(&[1, 2, 3, 4, 5, 6], 0)), floating: vec![] };
    let mut h = Harness::builder().with_size(vec2(150.0, 240.0)).build_ui(move |ui| {
        DockArea::new(Id::unique("overflow-symbol")).show(
            ui,
            &layout,
            &DockStyle::from_ui(ui),
            |panel| format!("Panel {panel}"),
            |_| Permissions::default(),
            |_ui: &mut egui::Ui, _panel: &u32| {},
        );
    });
    h.run_steps(2);
    assert!(h.output().shapes.iter().any(|shape| paints(&shape.shape, "\u{2026}")));
}

struct Plain;

impl DockContent<u32> for Plain {
    fn body(&mut self, _: &mut egui::Ui, _: &u32) {}
}

/// `sample` with its docked frames swapped: every panel stays, in other places.
fn swapped() -> Layout<u32> {
    let mut layout = sample();
    layout.root = Some(Node::Split {
        axis: SplitAxis::Horizontal,
        size: SplitSize::FixedFirst(240.0),
        first: Box::new(tabs(&[2, 3, 4], 1)),
        second: Box::new(tabs(&[1], 0)),
    });
    layout
}

#[test]
/// Replacing the layout during a tab drag ends the drag, so the new layout is not modified.
fn replacing_the_layout_ends_a_drag_in_progress() {
    use super::testing::DockDriver;
    let area = Id::unique("replaced");
    let driver = || {
        let mut d = DockDriver::new(sample(), Plain);
        d.id = area;
        d.settle();
        d
    };
    // Over frame [1], clear of the floating window.
    let over = pos2(500.0, 450.0);
    for replace in [false, true] {
        let mut d = driver();
        let tab = d.hit(&Hit::Tab(2)).unwrap().center();
        d.hold(tab, over);
        assert!(is_dragging::<u32>(&d.ctx, area));
        if replace {
            d.layout = swapped();
        }
        d.move_to(over + vec2(4.0, 0.0));
        assert_eq!(is_dragging::<u32>(&d.ctx, area), !replace);
        let out = d.release(over);
        if replace {
            assert!(out.actions.is_empty(), "{:?}", out.actions);
            assert_eq!(d.layout, swapped());
        } else {
            assert!(matches!(out.actions.as_slice(), [Action::Move { panel: 2, anchor: 1, .. }]), "{:?}", out.actions);
        }
    }

    // A window follows its bar through its own moves, and through an app that adjusts the
    // moved rectangle or a split's size, and stops when the app switches workspaces.
    let mut d = driver();
    let bar = d.hit(&Hit::Title(5)).unwrap().center();
    d.hold(bar, bar + vec2(60.0, 40.0));
    d.move_to(bar + vec2(90.0, 60.0));
    assert!(is_dragging::<u32>(&d.ctx, area));
    assert_ne!(d.layout.floating[0].rect, sample().floating[0].rect);
    d.layout.floating[0].rect[0] += 3.0;
    if let Some(Node::Split { size, .. }) = &mut d.layout.root {
        *size = SplitSize::FixedSecond(260.0);
    }
    d.move_to(bar + vec2(100.0, 70.0));
    assert!(is_dragging::<u32>(&d.ctx, area));
    d.layout = swapped();
    let out = d.move_to(bar + vec2(120.0, 80.0));
    assert!(!is_dragging::<u32>(&d.ctx, area));
    assert!(out.actions.is_empty(), "{:?}", out.actions);
    d.release(bar + vec2(120.0, 80.0));
    assert_eq!(d.layout, swapped());
}
