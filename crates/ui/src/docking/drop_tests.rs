//! Drop targets of the default profile over frames: a group body's thirds, the tab outline,
//! the fallback where a side has no room, and the root edge band.

use std::collections::HashMap;

use super::testing::DockDriver;
use super::*;
use crate::layout::{SplitAxis, SplitSize};
use egui::{Color32, Pos2, Rect, pos2, vec2};

/// Panel bodies by their rects; `rules` are the permissions of panels that differ from the
/// default, `min_width` every panel's least width.
#[derive(Default)]
struct Bodies {
    bodies: HashMap<u32, Rect>,
    rules: HashMap<u32, Permissions>,
    min_width: f32,
}

impl DockContent<u32> for Bodies {
    fn body(&mut self, ui: &mut egui::Ui, panel: &u32) {
        self.bodies.insert(*panel, ui.max_rect());
    }
    fn permissions(&self, panel: &u32) -> Permissions {
        self.rules.get(panel).copied().unwrap_or_default()
    }
    fn limits(&self, _: &u32) -> PanelLimits {
        PanelLimits { min: vec2(self.min_width, 0.0), ..PanelLimits::default() }
    }
}

type Driver = DockDriver<u32, Bodies>;

/// Frames `[1]` and `[2, 3]` side by side, each half of the area.
fn frames() -> Layout<u32> {
    Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::Ratio(0.5),
            first: Box::new(Node::Tabs { panels: vec![1], active: 0 }),
            second: Box::new(Node::Tabs { panels: vec![2, 3], active: 1 }),
        }),
        floating: Vec::new(),
    }
}

fn driver(rules: &[(u32, Permissions)]) -> Driver {
    DockDriver::new(frames(), Bodies { rules: rules.iter().copied().collect(), ..Bodies::default() })
}

/// The body of `panel` and its group's rect, the 26 pt strip included.
fn group(d: &Driver, panel: u32) -> (Rect, Rect) {
    let body = d.content.bodies.get(&panel).copied().unwrap_or_else(|| panic!("panel {panel} not drawn"));
    (body, Rect::from_min_max(pos2(body.left(), body.top() - 26.0), body.max))
}

fn tab(d: &Driver, panel: u32) -> Pos2 {
    d.hit(&Hit::Tab(panel)).unwrap_or_else(|| panic!("tab {panel}")).center()
}

/// The fill of the last frame's rect shape at `rect` that has no stroke.
fn fill_at(d: &Driver, rect: Rect) -> Option<Color32> {
    d.shapes().iter().find_map(|clipped| match &clipped.shape {
        egui::Shape::Rect(shape) if shape.rect == rect && shape.stroke.width == 0.0 && shape.fill != Color32::TRANSPARENT => Some(shape.fill),
        _ => None,
    })
}

/// The 4 pt line along `rect`'s edge on `zone`'s side, inside it.
fn line(rect: Rect, zone: Zone) -> Rect {
    match zone {
        Zone::Left => Rect::from_x_y_ranges(rect.left()..=(rect.left() + 4.0), rect.y_range()),
        Zone::Right => Rect::from_x_y_ranges((rect.right() - 4.0)..=rect.right(), rect.y_range()),
        Zone::Top => Rect::from_x_y_ranges(rect.x_range(), rect.top()..=(rect.top() + 4.0)),
        Zone::Bottom | Zone::Center => Rect::from_x_y_ranges(rect.x_range(), (rect.bottom() - 4.0)..=rect.bottom()),
    }
}

fn accent() -> Color32 {
    DockStyle::default().accent
}

#[test]
fn a_drop_on_a_groups_outer_third_docks_beside_it_with_its_rect_and_its_middle_joins_with_an_outline() {
    let d = driver(&[]);
    let (body, rect) = group(&d, 1);
    let (w, h) = (body.width(), body.height());
    let line = |zone| line(rect, zone);
    for (zone, at) in [
        (Zone::Left, pos2(body.left() + w * 0.3, body.center().y)),
        (Zone::Right, pos2(body.right() - w * 0.3, body.center().y)),
        (Zone::Top, pos2(body.center().x, body.top() + h * 0.3)),
        (Zone::Bottom, pos2(body.center().x, body.bottom() - h * 0.3)),
        // In the top left corner, the top is relatively nearer.
        (Zone::Top, pos2(body.left() + w * 0.25, body.top() + h * 0.2)),
    ] {
        let mut d = driver(&[]);
        d.hold(tab(&d, 3), at);
        assert!(!d.outlined(rect.shrink(1.0)), "{zone:?}");
        let out = previews_result(driver(&[]), tab(&d, 3), at, |d| group(d, 3).1);
        assert_eq!(out, vec![Action::Move { panel: 3, anchor: 1, placement: Placement::Split(zone) }], "{zone:?}");
    }
    // The middle and the tab strip join: the group's outline with a light fill, no line.
    for at in [body.center(), pos2(rect.center().x, rect.top() + 13.0)] {
        let mut d = driver(&[]);
        d.hold(tab(&d, 3), at);
        assert!(d.outlined(rect.shrink(1.0)), "{at:?}");
        assert_eq!(fill_at(&d, rect.shrink(1.0)), Some(accent().gamma_multiply(0.08)), "{at:?}: a light fill");
        assert!([Zone::Left, Zone::Right, Zone::Top, Zone::Bottom].into_iter().all(|z| !d.filled(line(z))), "{at:?}");
        let out = d.release(at);
        assert!(
            matches!(&out.actions[..], [Action::Move { panel: 3, anchor: 1, placement: Placement::Tab { .. } | Placement::Split(Zone::Center) }]),
            "{:?}",
            out.actions
        );
    }
    // A group's drag bar previews with the same painter.
    let mut d = driver(&[]);
    let bar = d.hit(&Hit::DragBar(2)).unwrap().center();
    d.hold(bar, body.center());
    assert!(d.outlined(rect.shrink(1.0)));
}

#[test]
fn without_new_dock_zones_a_group_that_takes_no_tabs_maps_its_whole_body_to_its_edges() {
    let canvas = Permissions { accepts_tabs: false, ..Permissions::default() };
    let start = || driver(&[(1, canvas)]).with_style(|style| style.new_dock_zones = false);
    let d = start();
    let (body, rect) = group(&d, 1);
    for (zone, at) in [(Zone::Left, body.center() - vec2(10.0, 0.0)), (Zone::Top, body.center() - vec2(0.0, 10.0))] {
        let out = previews_result(start(), tab(&d, 3), at, |d| group(d, 3).1);
        assert_eq!(out, vec![Action::Move { panel: 3, anchor: 1, placement: Placement::Split(zone) }], "{zone:?}");
    }
    // Its strip joins nothing.
    let mut d = start();
    let out = d.drag(tab(&d, 3), pos2(rect.center().x, rect.top() + 13.0));
    assert!(!out.actions.iter().any(|a| matches!(a, Action::Move { anchor: 1, placement: Placement::Tab { .. }, .. })), "{:?}", out.actions);
}

#[test]
fn a_side_without_room_joins_as_a_tab_or_offers_nothing() {
    // Two 400 pt frames of panels at least 300 wide: neither has room for two halves side by
    // side, but both have room for two halves stacked.
    let narrow = |rules: &[(u32, Permissions)]| {
        DockDriver::new(frames(), Bodies { rules: rules.iter().copied().collect(), min_width: 300.0, ..Bodies::default() })
    };
    let d = narrow(&[]);
    let (body, rect) = group(&d, 1);
    let left = pos2(body.left() + body.width() * 0.25, body.center().y);
    let mut d = narrow(&[]);
    d.hold(tab(&d, 3), left);
    assert!(d.outlined(rect.shrink(1.0)), "the tab outline");
    let out = d.release(left);
    assert_eq!(out.actions, vec![Action::Move { panel: 3, anchor: 1, placement: Placement::Split(Zone::Center) }]);
    let top = pos2(body.center().x, body.top() + body.height() * 0.2);
    let mut d = narrow(&[]);
    let out = d.drag(tab(&d, 3), top);
    assert_eq!(out.actions, vec![Action::Move { panel: 3, anchor: 1, placement: Placement::Split(Zone::Top) }]);
    // A group that takes no tabs offers nothing there.
    let mut d = narrow(&[(1, Permissions { accepts_tabs: false, ..Permissions::default() })]);
    d.hold(tab(&d, 3), left);
    assert!(!d.outlined(rect.shrink(1.0)));
    let out = d.release(left);
    assert!(!out.actions.iter().any(|a| matches!(a, Action::Move { anchor: 1, .. })), "{:?}", out.actions);
}

#[test]
fn the_root_edge_band_is_28_pt_and_previews_a_line_along_the_layouts_edge() {
    let d = driver(&[]);
    let (body, _) = group(&d, 1);
    let x = body.center().x;
    let window = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let out = previews_line(driver(&[]), tab(&d, 3), pos2(x, 573.0), line(window, Zone::Bottom));
    assert!(matches!(&out[..], [Action::Move { panel: 3, placement: Placement::Edge(Zone::Bottom), .. }]), "{out:?}");
    let mut d = driver(&[]);
    d.drag(tab(&d, 3), pos2(x, 573.0));
    d.settle();
    let (_, rect) = group(&d, 3);
    assert_eq!((rect.left(), rect.right(), rect.bottom()), (0.0, 800.0, 600.0), "along the layout's whole bottom edge");
    // Past the band, the group's own bottom third.
    let mut d = driver(&[]);
    let out = d.drag(tab(&d, 3), pos2(x, 571.0));
    assert_eq!(out.actions, vec![Action::Move { panel: 3, anchor: 1, placement: Placement::Split(Zone::Bottom) }]);
    // The band reaches only 4 pt past its line into a tab strip, which joins the group.
    let mut d = driver(&[]);
    let out = d.drag(tab(&d, 3), pos2(x, 12.0));
    assert!(matches!(&out.actions[..], [Action::Move { panel: 3, anchor: 1, placement: Placement::Tab { .. } }]), "{:?}", out.actions);
    let mut d = driver(&[]);
    let out = d.drag(tab(&d, 3), pos2(x, 7.0));
    assert!(matches!(&out.actions[..], [Action::Move { panel: 3, placement: Placement::Edge(Zone::Top), .. }]), "{:?}", out.actions);
}

fn tabs(panels: &[u32]) -> Node<u32> {
    Node::Tabs { panels: panels.to_vec(), active: 0 }
}

fn split(axis: SplitAxis, size: SplitSize, first: Node<u32>, second: Node<u32>) -> Node<u32> {
    Node::Split { axis, size, first: Box::new(first), second: Box::new(second) }
}

/// Side frame `[1]`, canvas 0 and side frame `[2, 3]` side by side, 200, 400 and 200 pt wide.
fn canvas_columns() -> Layout<u32> {
    let sides = split(SplitAxis::Horizontal, SplitSize::FixedSecond(200.0), tabs(&[0]), Node::Tabs { panels: vec![2, 3], active: 1 });
    Layout { root: Some(split(SplitAxis::Horizontal, SplitSize::FixedFirst(200.0), tabs(&[1]), sides)), floating: Vec::new() }
}

fn canvas_driver(canvas: Permissions, style: impl Fn(&mut DockStyle) + 'static) -> Driver {
    DockDriver::new(canvas_columns(), Bodies { rules: [(0, canvas)].into(), ..Bodies::default() }).with_style(style)
}

fn moves(out: &DockOutput<u32>) -> bool {
    out.actions.iter().any(|a| matches!(a, Action::Move { .. } | Action::MovePanel { .. }))
}

#[test]
fn between_side_frames_the_outer_edge_band_covers_only_the_canvas_column() {
    let d = canvas_driver(Permissions::PROTECTED, |_| {});
    let (canvas, _) = group(&d, 0);
    let (side, _) = group(&d, 1);
    let at = pos2(canvas.center().x, 590.0);
    let mut d = canvas_driver(Permissions::PROTECTED, |_| {});
    // The line along the canvas column's bottom.
    let out = previews_line(canvas_driver(Permissions::PROTECTED, |_| {}), tab(&d, 3), at, line(canvas, Zone::Bottom));
    assert_eq!(out, vec![Action::Move { panel: 3, anchor: 0, placement: Placement::Edge(Zone::Bottom) }]);
    d.drag(tab(&d, 3), at);
    // The side frames keep their full height.
    let column = split(SplitAxis::Vertical, SplitSize::Ratio(0.75), tabs(&[0]), tabs(&[3]));
    let sides = split(SplitAxis::Horizontal, SplitSize::FixedSecond(200.0), column, tabs(&[2]));
    assert_eq!(d.layout.root, Some(split(SplitAxis::Horizontal, SplitSize::FixedFirst(200.0), tabs(&[1]), sides)));
    // Under a side frame, its own bottom third.
    let mut d = canvas_driver(Permissions::PROTECTED, |_| {});
    let out = d.drag(tab(&d, 3), pos2(side.center().x, 590.0));
    assert_eq!(out.actions, vec![Action::Move { panel: 3, anchor: 1, placement: Placement::Split(Zone::Bottom) }]);
}

#[test]
fn between_docks_the_outer_edge_band_covers_the_part_between_them() {
    let dock = |side, panels: &[u32]| Node::Dock { side, columns: vec![Column::new(vec![Group::new(panels.to_vec(), 0)], 200.0)] };
    let middle = split(SplitAxis::Vertical, SplitSize::Ratio(0.7), tabs(&[0]), tabs(&[4]));
    let inner = split(SplitAxis::Horizontal, SplitSize::FixedSecond(200.0), middle.clone(), dock(Side::End, &[2, 3]));
    let layout =
        Layout { root: Some(split(SplitAxis::Horizontal, SplitSize::FixedFirst(200.0), dock(Side::Start, &[1]), inner)), floating: Vec::new() };
    let driver = || DockDriver::new(layout.clone(), Bodies { rules: [(0, Permissions::PROTECTED)].into(), ..Bodies::default() });
    let d = driver();
    let (canvas, _) = group(&d, 0);
    let (timeline, _) = group(&d, 4);
    let area = Rect::from_x_y_ranges(canvas.x_range(), (canvas.top() - 26.0)..=timeline.bottom());
    let at = pos2(canvas.center().x, 590.0);
    // The line along the bottom between the docks.
    let out = previews_line(driver(), tab(&d, 3), at, line(area, Zone::Bottom));
    assert!(matches!(&out[..], [Action::Move { panel: 3, placement: Placement::Edge(Zone::Bottom), .. }]), "{out:?}");
    let mut d = driver();
    d.drag(tab(&d, 3), at);
    let Some(Node::Split { second, .. }) = &d.layout.root else { panic!("{:?}", d.layout) };
    let Node::Split { first, .. } = &**second else { panic!("{:?}", d.layout) };
    assert_eq!(**first, split(SplitAxis::Vertical, SplitSize::Ratio(0.75), middle, tabs(&[3])));
}

/// The frame beside `panel`'s own: what an edge split with `panel` wrapped.
fn wrapped(layout: &Layout<u32>, panel: u32) -> Option<Node<u32>> {
    let Some(dock_model::Place::Node(path)) = layout.place(&panel) else { return None };
    let (last, parent) = path.split_last()?;
    layout.root.as_ref().and_then(|root| dock_model::node_at(root, &[parent, &[!last]].concat())).cloned()
}

#[test]
fn over_a_full_width_row_the_top_edge_band_covers_only_the_canvas_column_above_it() {
    // SoundCraft's Edit window: side frame 1, canvas 0 and side frame [2, 3] over a full-width
    // editor 4.
    let Some(upper) = canvas_columns().root else { panic!() };
    let root = split(SplitAxis::Vertical, SplitSize::Ratio(0.7), upper.clone(), tabs(&[4]));
    let layout = Layout { root: Some(root), floating: Vec::new() };
    let driver = || DockDriver::new(layout.clone(), Bodies { rules: [(0, Permissions::PROTECTED)].into(), ..Bodies::default() });
    let d = driver();
    let (canvas, _) = group(&d, 0);
    let at = pos2(canvas.center().x, 5.0);
    // The line along the canvas column's top, not the window's.
    let out = previews_line(driver(), tab(&d, 3), at, line(canvas, Zone::Top));
    assert_eq!(out, vec![Action::Move { panel: 3, anchor: 0, placement: Placement::Edge(Zone::Top) }]);
    let mut d = driver();
    d.drag(tab(&d, 3), at);
    assert_eq!(wrapped(&d.layout, 3), Some(tabs(&[0])), "{:?}", d.layout);
}

#[test]
fn an_outer_edge_drop_wraps_the_area_it_previews_when_the_app_applies_other_permissions() {
    // Frame [2, 3] and canvas 0 beside frame 1, which is protected only while drawing (PDFCraft's
    // bare tools mode): drawn, the edge band is not narrowed to the canvas.
    let inner = split(SplitAxis::Horizontal, SplitSize::Ratio(0.5), Node::Tabs { panels: vec![2, 3], active: 1 }, tabs(&[0]));
    let layout = Layout { root: Some(split(SplitAxis::Horizontal, SplitSize::Ratio(0.7), inner, tabs(&[1]))), floating: Vec::new() };
    let drawn = [(0, Permissions::PROTECTED), (1, Permissions::PROTECTED)];
    let driver = || DockDriver::new(layout.clone(), Bodies { rules: drawn.into(), ..Bodies::default() });
    let d = driver();
    let (canvas, _) = group(&d, 0);
    let at = pos2(canvas.center().x, 5.0);
    let mut d = driver();
    d.hold(tab(&d, 3), at);
    assert!(d.filled(line(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), Zone::Top)), "the line along the window's top");
    assert_eq!(result_preview(&d), None);
    let out = d.release(at);
    let mut applied = layout.clone();
    let apply = |p: &u32| if *p == 0 { Permissions::PROTECTED } else { Permissions::default() };
    applied.apply_all(out.actions.clone(), apply).unwrap_or_else(|e| panic!("{e}: {:?}", out.actions));
    let mut rest = layout;
    rest.apply(Action::Close { panel: 3 }).unwrap();
    assert_eq!(wrapped(&applied, 3), rest.root, "{:?}", out.actions);
}

#[test]
fn drop_edges_limit_the_outer_edges_and_the_canvas_sides() {
    let left_right = |style: &mut DockStyle| style.drop_edges = Sides::LEFT_RIGHT;
    let d = canvas_driver(Permissions::PROTECTED, left_right);
    // The canvas has no tab strip: its body is its frame.
    let (canvas, _) = group(&d, 0);
    let (side, _) = group(&d, 1);
    // Without top and bottom, no outer edge takes a drop: left and right are docks here.
    for at in [pos2(canvas.center().x, 590.0), pos2(canvas.left() + 5.0, 300.0), pos2(canvas.right() - 5.0, 300.0)] {
        let mut d = canvas_driver(Permissions::PROTECTED, left_right);
        d.hold(tab(&d, 3), at);
        assert!(!d.filled(line(canvas, Zone::Bottom)), "{at:?}");
        let out = d.release(at);
        assert!(!moves(&out), "{at:?}: {:?}", out.actions);
    }
    // A side frame keeps all four sides.
    for (zone, y) in [(Zone::Bottom, 590.0), (Zone::Top, side.top() + side.height() * 0.2)] {
        let mut d = canvas_driver(Permissions::PROTECTED, left_right);
        let out = d.drag(tab(&d, 3), pos2(side.center().x, y));
        assert_eq!(out.actions, vec![Action::Move { panel: 3, anchor: 1, placement: Placement::Split(zone) }], "{zone:?}");
    }
    // No drop edges: no outer edge takes a drop.
    for at in [pos2(canvas.center().x, 590.0), pos2(canvas.center().x, 5.0), pos2(canvas.left() + 5.0, 300.0), pos2(canvas.right() - 5.0, 300.0)] {
        let mut d = canvas_driver(Permissions::PROTECTED, |style| style.drop_edges = Sides::NONE);
        let out = d.drag(tab(&d, 3), at);
        assert!(!moves(&out), "{at:?}: {:?}", out.actions);
    }
}

#[test]
fn without_new_dock_zones_a_canvas_that_takes_splits_takes_them_only_on_its_drop_edges() {
    let beside = Permissions { accepts_splits: true, ..Permissions::PROTECTED };
    let d = canvas_driver(beside, |_| {});
    let (body, _) = group(&d, 0);
    // Nearer the bottom than the left, and right of center nearer the right than the top.
    let low = pos2(body.left() + body.width() * 0.3, body.bottom() - 40.0);
    let high = pos2(body.right() - body.width() * 0.3, body.top() + 40.0);
    for (edges, zones) in [
        (DockStyle::default().drop_edges, [Some(Zone::Bottom), Some(Zone::Top)]),
        (Sides::LEFT_RIGHT, [Some(Zone::Left), Some(Zone::Right)]),
        (Sides::NONE, [None, None]),
    ] {
        for (at, zone) in [low, high].into_iter().zip(zones) {
            let split_frames = move |style: &mut DockStyle| {
                style.new_dock_zones = false;
                style.drop_edges = edges;
            };
            let mut d = canvas_driver(beside, split_frames);
            let out = d.drag(tab(&d, 3), at);
            match zone {
                Some(zone) => assert_eq!(out.actions, vec![Action::Move { panel: 3, anchor: 0, placement: Placement::Split(zone) }], "{edges:?}"),
                None => assert!(!moves(&out), "{edges:?}: {:?}", out.actions),
            }
            // With new dock zones the canvas body takes no drop on any side.
            let mut d = canvas_driver(beside, move |style| style.drop_edges = edges);
            let out = d.drag(tab(&d, 3), at);
            assert!(!moves(&out), "{edges:?}: {:?}", out.actions);
        }
    }
}

/// Whether the last frame painted a drop preview: an accent line, outline or fill.
fn previews(d: &Driver) -> bool {
    let accent = accent();
    d.shapes().iter().any(|clipped| match &clipped.shape {
        egui::Shape::Rect(shape) => shape.fill == accent || shape.fill == accent.gamma_multiply(0.08) || shape.stroke.color == accent,
        egui::Shape::Path(path) => path.fill != Color32::TRANSPARENT && path.fill == accent.gamma_multiply(0.3),
        _ => false,
    })
}

fn ratio_split(axis: SplitAxis, ratio: f32, first: Node<u32>, second: Node<u32>) -> Node<u32> {
    Node::Split { axis, size: SplitSize::Ratio(ratio), first: Box::new(first), second: Box::new(second) }
}

#[test]
fn a_drop_that_changes_only_sizes_shows_nothing_and_does_nothing() {
    // CADCraft's sidebar: Layers (2) alone above Properties (3) and a third group (4), beside
    // the canvas (1); and the same with the column as one two-group split.
    let nested = ratio_split(SplitAxis::Vertical, 0.3, tabs(&[2]), ratio_split(SplitAxis::Vertical, 0.5, tabs(&[3]), tabs(&[4])));
    let flat = ratio_split(SplitAxis::Vertical, 0.3, tabs(&[2]), tabs(&[3]));
    for column in [nested, flat] {
        let layout = Layout { root: Some(ratio_split(SplitAxis::Horizontal, 0.6, tabs(&[1]), column)), floating: Vec::new() };
        let start = || DockDriver::new(layout.clone(), Bodies::default());
        let d = start();
        let (body, _) = group(&d, 3);
        let top = pos2(body.center().x, body.top() + body.height() * 0.2);
        // Properties' bottom third moves Layers, with the rect it takes.
        let below = pos2(body.center().x, body.bottom() - body.height() * 0.2);
        previews_result(start(), tab(&d, 2), below, |d| group(d, 2).1);
        // Its top third would put Layers back where it is.
        let mut d = start();
        d.hold(tab(&d, 2), top);
        assert!(!previews(&d), "no outline or fill");
        let out = d.release(top);
        assert!(out.actions.is_empty(), "{:?}", out.actions);
        assert_eq!(d.layout, layout);
    }
    // A lone frame's tab on the layout edge its frame already holds.
    let layout = Layout { root: Some(ratio_split(SplitAxis::Vertical, 0.75, tabs(&[1]), tabs(&[4]))), floating: Vec::new() };
    let mut d = DockDriver::new(layout.clone(), Bodies::default());
    let at = pos2(400.0, 590.0);
    d.hold(tab(&d, 4), at);
    assert!(!previews(&d));
    let out = d.release(at);
    assert!(out.actions.is_empty(), "{:?}", out.actions);
    assert_eq!(d.layout, layout);
}

#[test]
fn a_dock_drop_that_changes_only_heights_shows_nothing_and_does_nothing() {
    // A dock column of a sized lone group (2) above another (3), beside the canvas (1).
    let column = Column::new(vec![Group { height: Some(150.0), ..Group::new(vec![2], 0) }, Group::new(vec![3], 0)], 240.0);
    let dock = Node::Dock { side: Side::End, columns: vec![column] };
    let root = Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedSecond(240.0), first: Box::new(tabs(&[1])), second: Box::new(dock) };
    let layout = Layout { root: Some(root), floating: Vec::new() };
    let start = || DockDriver::new(layout.clone(), Bodies::default());
    let d = start();
    let (body, rect) = group(&d, 3);
    let (upper, _) = group(&d, 2);
    // Below the lower group moves the upper one, with the rect it takes.
    let mut d = start();
    d.hold(tab(&d, 2), pos2(body.center().x, body.bottom() - body.height() * 0.2));
    assert!(previews(&d));
    // The lower group's top third and the seam between them put the group back where it is.
    for at in [pos2(body.center().x, body.top() + body.height() * 0.2), pos2(body.center().x, (upper.bottom() + rect.top()) * 0.5)] {
        let mut d = start();
        d.hold(tab(&d, 2), at);
        assert!(!previews(&d), "{at:?}: no preview");
        let out = d.release(at);
        assert!(out.actions.is_empty(), "{at:?}: {:?}", out.actions);
        assert_eq!(d.layout, layout);
    }
}

#[test]
fn a_header_drag_of_a_windows_only_group_looks_through_its_window() {
    // Group 4 floats alone over frame 1's left part.
    let mut layout = frames();
    layout.floating.push(Floating::single(vec![4], 0, [40.0, 150.0, 300.0, 260.0]));
    let start = || DockDriver::new(layout.clone(), Bodies::default());
    let d = start();
    let (body, rect) = group(&d, 1);
    let window = Rect::from_min_size(pos2(40.0, 150.0), vec2(300.0, 260.0));
    let at = pos2(body.left() + body.width() * 0.25, body.center().y);
    assert!(window.contains(at));
    let bar = d.hit(&Hit::DragBar(4)).unwrap().center();
    // Frame 1's left third, behind the window.
    let out = previews_result(start(), bar, at, |d| {
        let (_, group) = group(d, 4);
        assert_eq!(group.left(), rect.left());
        group
    });
    assert_eq!(out, vec![Action::MoveGroup { panel: 4, to: Target::At { anchor: 1, placement: Placement::Split(Zone::Left) } }]);
}

#[test]
fn a_header_drag_in_a_window_of_two_groups_still_offers_the_other_group() {
    let mut layout = frames();
    let tree = ratio_split(SplitAxis::Vertical, 0.5, tabs(&[4]), tabs(&[5]));
    layout.floating.push(Floating::new(tree, [40.0, 100.0, 300.0, 400.0]));
    let start = || DockDriver::new(layout.clone(), Bodies::default());
    let d = start();
    let (body, rect) = group(&d, 5);
    let mut d = start();
    let bar = d.hit(&Hit::DragBar(4)).unwrap().center();
    d.hold(bar, body.center());
    assert!(d.outlined(rect.shrink(1.0)), "group 5 joins");
    let out = d.release(body.center());
    assert!(
        matches!(
            &out.actions[..],
            [Action::MoveGroup { panel: 4, to: Target::At { anchor: 5, placement: Placement::Tab { .. } | Placement::Split(Zone::Center) } }]
        ),
        "{:?}",
        out.actions
    );
}

#[test]
fn a_drop_beside_a_frame_too_narrow_to_halve_takes_room_from_its_neighbour() {
    // Frame 2 is 60 pt wide beside frame 1; group 4 floats over frame 1.
    let root =
        Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedSecond(60.0), first: Box::new(tabs(&[1])), second: Box::new(tabs(&[2])) };
    let layout = Layout { root: Some(root), floating: vec![Floating::single(vec![4], 0, [100.0, 100.0, 200.0, 200.0])] };
    let start = || DockDriver::new(layout.clone(), Bodies::default());
    let d = start();
    let (body, rect) = group(&d, 2);
    let at = pos2(body.left() + 8.0, body.center().y);
    // The preview is the rect the group takes left of frame 2.
    let out = previews_result(start(), tab(&d, 4), at, |d| group(d, 4).1);
    assert!(matches!(&out[..], [Action::Move { panel: 4, anchor: 2, placement: Placement::SplitSized { zone: Zone::Left, .. } }]), "{out:?}");
    let mut d = start();
    d.drag(tab(&d, 4), at);
    d.settle();
    assert!((group(&d, 2).1.width() - rect.width()).abs() < 1.0, "frame 2 keeps its width");
    assert!(group(&d, 4).1.width() >= 40.0);
}

/// The rect of the result preview the last frame painted: a 2 pt accent outline with a light
/// fill.
fn result_preview(d: &Driver) -> Option<Rect> {
    let fill = accent().gamma_multiply(0.08);
    d.shapes()
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(shape) if shape.fill == fill && shape.stroke.width == 0.0 => Some(shape.rect),
            _ => None,
        })
        .filter(|rect| d.outlined(*rect))
}

/// Drags `from` to `at` and releases: the preview while held is the drop line `line`, with no
/// result rect. Returns the drop's actions.
fn previews_line(mut d: Driver, from: Pos2, at: Pos2, line: Rect) -> Vec<Action<u32>> {
    d.hold(from, at);
    assert!(d.filled(line), "the line {line:?}");
    assert_eq!(result_preview(&d), None);
    let out = d.release(at);
    d.settle();
    out.actions
}

/// Drags `from` to `at` and back to rest: the preview while held is the rect `rect_of` reads
/// once the drop is applied, inset by its 1 pt margin. Returns the drop's actions.
fn previews_result(mut d: Driver, from: Pos2, at: Pos2, rect_of: impl Fn(&Driver) -> Rect) -> Vec<Action<u32>> {
    d.hold(from, at);
    let preview = result_preview(&d);
    let out = d.release(at);
    d.settle();
    assert_eq!(preview, Some(rect_of(&d).shrink(1.0)), "{:?}", out.actions);
    out.actions
}

#[test]
fn a_drop_beside_a_frame_previews_the_rect_the_group_takes() {
    let d = driver(&[]);
    let (body, _) = group(&d, 1);
    let at = pos2(body.left() + body.width() * 0.2, body.center().y);
    let out = previews_result(driver(&[]), tab(&d, 3), at, |d| group(d, 3).1);
    assert_eq!(out, vec![Action::Move { panel: 3, anchor: 1, placement: Placement::Split(Zone::Left) }]);
    // A whole group the same.
    let bar = d.hit(&Hit::DragBar(2)).unwrap().center();
    // Panel 3 is the group's selected tab, the one it draws.
    let out = previews_result(driver(&[]), bar, at, |d| group(d, 3).1);
    assert_eq!(out, vec![Action::MoveGroup { panel: 2, to: Target::At { anchor: 1, placement: Placement::Split(Zone::Left) } }]);
}

/// A 240 pt dock column at the start, group `[2]` above `[3, 4]`, beside frame 1.
fn dock_driver() -> Driver {
    let column = Column::new(vec![Group::new(vec![2], 0), Group::new(vec![3, 4], 0)], 240.0);
    let dock = Node::Dock { side: Side::Start, columns: vec![column] };
    let root = Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedFirst(240.0), first: Box::new(dock), second: Box::new(tabs(&[1])) };
    DockDriver::new(Layout { root: Some(root), floating: Vec::new() }, Bodies::default())
}

/// The rect of the dock column of `panel`'s group, its bar included, when that group fills it.
fn column_of(d: &Driver, panel: u32) -> Rect {
    let bar = d.hit(&Hit::ColumnBar(panel)).unwrap_or_else(|| panic!("column bar {panel}"));
    let (body, _) = group(d, panel);
    Rect::from_x_y_ranges(body.x_range(), bar.top()..=body.bottom())
}

#[test]
fn a_dock_drop_previews_the_rect_the_group_or_column_takes() {
    let d = dock_driver();
    let (upper, _) = group(&d, 2);
    // Above group 2: a new group at the column's top.
    let at = pos2(upper.center().x, upper.top() + 10.0);
    let out = previews_result(dock_driver(), tab(&d, 4), at, |d| group(d, 4).1);
    assert_eq!(out, vec![Action::Move { panel: 4, anchor: 2, placement: Placement::Group { below: false, height: None } }]);
    // The column's inner edge: a new column after it, the full height.
    let out = previews_result(dock_driver(), tab(&d, 4), pos2(upper.right() + 6.0, 200.0), |d| column_of(d, 4));
    assert_eq!(out, vec![Action::Move { panel: 4, anchor: 2, placement: Placement::Column { after: true } }]);
    // The window's right edge: a new dock, previewed by the line along that edge.
    let window = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let out = previews_line(dock_driver(), tab(&d, 4), pos2(795.0, 200.0), line(window, Zone::Right));
    assert_eq!(out, vec![Action::MovePanel { panel: 4, to: Target::NewDock { side: Side::End } }]);
}
