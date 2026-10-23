use super::*;
use crate::layout::{SplitAxis, SplitSize};
use egui::{Context, Event, Id, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

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
        floating: vec![Floating { panels: vec![5, 6], active: 1, rect: [80.0, 70.0, 300.0, 220.0] }],
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
    assert_eq!(layout.floating[0].panels, vec![6]);
    assert_eq!(layout.floating[0].active, 0);
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
    assert_eq!(layout.floating[0].panels, vec![6, 5]);
    assert_eq!(layout.floating[0].active, 1);
    layout.apply(move_to(6, 3, Zone::Center)).unwrap();
    layout.apply(move_to(5, 3, Zone::Center)).unwrap();
    assert!(layout.floating.is_empty());
    assert_eq!(layout.panels().len(), 6);
}

#[test]
fn invalid_moves_are_atomic_including_after_a_source_would_have_been_removed() {
    let layout = sample();
    for action in [
        move_to(5, 99, Zone::Center),
        move_to(99, 2, Zone::Center),
        move_to(5, 6, Zone::Left),
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
    ] {
        let mut next = layout.clone();
        assert_eq!(next.apply_with_permissions(action, permissions), Err(DockError::NotAllowed));
        assert_eq!(next, layout);
    }
    let mut next = layout;
    next.apply_with_permissions(Action::OpenAt { panel: 7, anchor: 1, placement: Placement::Split(Zone::Left) }, permissions).unwrap();
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
        floating: vec![Floating { panels: vec![3], active: 0, rect: [41.0, 52.0, 233.0, 244.0] }],
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
    duplicate.floating[0].panels.push(1);
    assert_eq!(duplicate.validate(), Err(DockError::DuplicatePanel));
    let mut invalid = sample();
    invalid.floating[0].active = usize::MAX;
    assert_eq!(invalid.validate(), Err(DockError::InvalidSelection));
    invalid.floating[0].active = 0;
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
    }
}

#[test]
fn pointer_targets_distinguish_reorder_center_split_and_occlusion() {
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 200.0));
    let group = target(2, &[2, 3], rect);
    let placement = resolve_drop(&5, pos2(90.0, 30.0), std::slice::from_ref(&group)).unwrap();
    assert_eq!(placement.placement, Placement::Tab { before: Some(3) });
    assert!(placement.insertion.unwrap().is_finite());
    assert_eq!(resolve_drop(&5, rect.center(), std::slice::from_ref(&group)).unwrap().placement, Placement::Split(Zone::Center));
    assert_eq!(resolve_drop(&5, pos2(11.0, 90.0), std::slice::from_ref(&group)).unwrap().placement, Placement::Split(Zone::Left));
    assert!(resolve_drop(&3, rect.center(), std::slice::from_ref(&group)).is_none());
    assert!(resolve_drop(&3, pos2(11.0, 90.0), std::slice::from_ref(&group)).is_some());
    let mut overlay = target(5, &[5], rect);
    overlay.accepts_tabs = false;
    overlay.allow_splits = false;
    assert!(resolve_drop(&3, rect.center(), &[group, overlay]).is_none());
    assert!(drop_zone(Rect::NOTHING, 24.0, pos2(0.0, 0.0)).is_none());
    assert!(drop_zone(rect, 24.0, pos2(f32::NAN, 0.0)).is_none());
}

#[test]
fn long_action_sequence_never_loses_or_duplicates_a_panel() {
    let mut layout = sample();
    let mut seed = 72u64;
    for step in 0..800 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let panel = (seed % 6 + 1) as u32;
        let anchor = ((seed >> 16) % 6 + 1) as u32;
        let action = if step % 7 == 0 {
            Action::Float { panel, rect: [10.0, 20.0, 200.0, 220.0] }
        } else {
            move_to(
                panel,
                anchor,
                match seed % 5 {
                    0 => Zone::Left,
                    1 => Zone::Right,
                    2 => Zone::Top,
                    3 => Zone::Bottom,
                    _ => Zone::Center,
                },
            )
        };
        let before = layout.clone();
        if layout.apply(action).is_err() {
            assert_eq!(layout, before);
        }
        layout.validate().unwrap();
        let mut panels: Vec<u32> = layout.panels().into_iter().copied().collect();
        panels.sort_unstable();
        assert_eq!(panels, vec![1, 2, 3, 4, 5, 6]);
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
    })
    .textures_delta
    .clear();
    (output, ids)
}

#[test]
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

fn pointer_button(pos: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

#[test]
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
    assert!(!layout.floating.iter().any(|g| g.panels.contains(&5)));
    assert_eq!(layout.panels().len(), 6);
    let Node::Split { second, .. } = layout.root.as_ref().unwrap() else { panic!() };
    let Node::Tabs { panels, active } = second.as_ref() else { panic!() };
    assert_eq!(panels.get(*active), Some(&5));
}

#[test]
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
fn focused_floating_resize_grip_accepts_both_arrow_axes_and_releases_tab() {
    let ctx = Context::default();
    let mut layout = sample();
    for _ in 0..3 {
        step(&ctx, &mut layout, Vec::new());
    }
    let id = Id::unique("test-area").with(("floating", &5u32)).with("resize");
    ctx.memory_mut(|memory| memory.request_focus(id));
    let key = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    step(&ctx, &mut layout, vec![key(egui::Key::ArrowRight), key(egui::Key::ArrowDown)]);
    assert_eq!(layout.floating[0].rect, [80.0, 70.0, 308.0, 228.0]);
    step(&ctx, &mut layout, vec![key(egui::Key::Tab)]);
    assert_ne!(ctx.memory(|memory| memory.focused()), Some(id));
}

#[test]
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
}

#[test]
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
    assert!((width - 260.0).abs() < 0.001, "sidebar width after inner-edge drag: {width}");
}

#[test]
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
fn floating_only_surface_does_not_allocate_the_callers_document_region() {
    let ctx = Context::default();
    let layout = Layout { root: None, floating: vec![Floating { panels: vec![1u32], active: 0, rect: [30.0, 40.0, 200.0, 200.0] }] };
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
    assert_eq!(layout.floating[0].panels, original.panels);
    assert_eq!(layout.floating[0].active, original.active);
    assert_ne!(layout.floating[0].rect, original.rect);
    step(&ctx, &mut layout, vec![pointer_button(start + vec2(50.0, 40.0), false)]);
    for _ in 0..2 {
        step(&ctx, &mut layout, vec![]);
    }
    let tab = ctx.read_response(Id::unique("test-area").with(("tab", &5u32))).unwrap().rect.center();
    step(&ctx, &mut layout, vec![Event::PointerMoved(tab), pointer_button(tab, true)]);
    step(&ctx, &mut layout, vec![Event::PointerMoved(tab + vec2(8.0, 0.0))]);
    step(&ctx, &mut layout, vec![Event::PointerMoved(tab + vec2(100.0, 100.0))]);
    let detached = step(&ctx, &mut layout, vec![pointer_button(tab + vec2(100.0, 100.0), false)]);
    assert!(detached.iter().any(|a| matches!(a, Action::Float { panel: 5, .. })));
    assert_eq!(layout.floating.len(), 2);
    assert_eq!(layout.floating[0].panels, vec![6]);
}

struct CustomContent {
    labels: Vec<String>,
    headers: usize,
    menus: usize,
    bodies: usize,
}

#[test]
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
    fn action_label(&mut self, ui: &egui::Ui, logical: &str, font: &egui::FontId, color: egui::Color32) -> egui::WidgetText {
        ui.painter().layout_no_wrap(format!("Action visual: {logical}"), font.clone(), color).into()
    }
    fn panel_menu(&mut self, ui: &mut egui::Ui, _: &u32) {
        self.menus += 1;
        ui.button("App menu").clicked();
    }
    fn group_header_width(&self, _: &[u32], _: &u32, _: bool) -> f32 {
        40.0
    }
    fn group_header(&mut self, ui: &mut egui::Ui, _: &[u32], _: &u32, _: bool) {
        self.headers += 1;
        assert!(ui.max_rect().width() <= 40.0);
        ui.label("…");
    }
}

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
fn custom_chrome_shaped_labels_and_programmatic_raise_are_available() {
    let ctx = Context::default();
    let layout = sample();
    let mut content = CustomContent { labels: vec![], headers: 0, menus: 0, bodies: 0 };
    for _ in 0..2 {
        custom_frame(&ctx, &layout, &mut content, vec![], &[], false);
    }
    let out = custom_frame(&ctx, &layout, &mut content, vec![], &[], true);
    assert_eq!(out.focused, Some(5));
    assert!(out.actions.is_empty());
    assert!(content.headers >= 3 && content.bodies >= 3);
    assert!(content.labels.iter().any(|s| s == "Logical 5"));
    let at = pos2(130.0, 82.0);
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
fn protected_group_member_blocks_floating_group_resize() {
    let moved = |actions: &[Action<u32>]| actions.iter().any(|a| matches!(a, Action::MoveFloating { .. }));
    assert!(moved(&resize_floating_group(|_| Permissions::default())), "the grip resizes a movable group");
    assert!(!moved(&resize_floating_group(|p| if *p == 6 { Permissions::PROTECTED } else { Permissions::default() })));
}

#[test]
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
    invalid.floating.push(Floating { panels: vec![3], active: 0, rect: [10.0, 10.0, 100.0, 100.0] });
    assert!(!DockArea::new(Id::unique("focus-area")).chrome_has_focus(&ctx, &invalid));
    assert!(!DockArea::<u32>::new(Id::unique("focus-area")).chrome_has_focus(&ctx, &Layout::default()));
    ctx.memory_mut(|memory| memory.request_focus(Id::unique("unrendered")));
    assert!(!DockArea::new(Id::unique("focus-area")).chrome_has_focus(&ctx, &layout));
}

#[test]
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
fn popup_action_labels_use_custom_visual_text_and_logical_accessibility() {
    use egui_kittest::{Harness, kittest::Queryable};
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
    let tab = Id::unique("action-label-test").with(("tab", &5_u32));
    let at = h.ctx.read_response(tab).unwrap().rect.center();
    h.input_mut().events.extend([
        Event::PointerMoved(at),
        Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(1);
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(egui::Popup::is_any_open(&h.ctx));
    for logical in ["Float panel", "Move to group", "Close panel"] {
        assert!(
            h.output().shapes.iter().any(|shape| paints(&shape.shape, &format!("Action visual: {logical}"))),
            "custom action must paint: {logical}"
        );
        // This sample also has a floating-group grip labeled Move to group.
        let expected = if logical == "Move to group" { 2 } else { 1 };
        assert_eq!(h.query_all_by_label(logical).count(), expected, "popup action must retain its own logical accessibility: {logical}");
        assert_eq!(
            h.query_all_by_label(&format!("Action visual: {logical}")).count(),
            0,
            "visual substitution must not replace logical accessibility"
        );
    }
}
