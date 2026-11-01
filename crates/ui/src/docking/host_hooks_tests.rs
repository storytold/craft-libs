//! Host-owned chrome uses the same fitted geometry and preserves stock defaults.
use super::*;
use egui::{Context, Event, Id, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

fn input(events: Vec<Event>) -> RawInput {
    RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), focused: true, events, ..Default::default() }
}
fn button(at: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

#[test]
fn corner_resize_keeps_each_enabled_axis_including_single_axis_hosts() {
    for axes in [[true, true], [false, true], [true, false], [false, false]] {
        let ctx = Context::default();
        let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(300.0, 200.0));
        let frame = |events| {
            let mut resized = None;
            ctx.run_ui(input(events), |ui| {
                let style = chrome::ChromeStyle::from_ui(ui);
                resized = chrome::FloatingFrame::new(Id::unique("axes"), rect).resize_axes(axes).show(ui.ctx(), &style, |_, _| ()).resized;
            })
            .drop_without_applying_deltas();
            resized
        };
        frame(vec![]);
        frame(vec![]);
        let from = rect.right_bottom() - vec2(2.0, 2.0);
        frame(vec![Event::PointerMoved(from), button(from, true)]);
        frame(vec![Event::PointerMoved(from + vec2(10.0, 10.0))]);
        let result = frame(vec![Event::PointerMoved(from + vec2(40.0, 35.0))]);
        if axes == [false, false] {
            assert!(result.is_none());
        } else {
            let expected = rect.size() + vec2(if axes[0] { 40.0 } else { 0.0 }, if axes[1] { 35.0 } else { 0.0 });
            assert_eq!(result, Some(Rect::from_min_size(rect.min, expected)), "{axes:?}");
        }
    }
}

#[test]
fn the_top_gutter_is_reserved_by_default_and_can_be_owned_by_the_contents() {
    for reserve in [true, false] {
        let ctx = Context::default();
        let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(300.0, 200.0));
        let mut inner = Rect::NOTHING;
        ctx.run_ui(input(vec![]), |ui| {
            let style = chrome::ChromeStyle::from_ui(ui);
            inner = chrome::FloatingFrame::new(Id::unique("gutter"), rect).reserve_top_resize_band(reserve).show(ui.ctx(), &style, |_, r| r).inner;
        })
        .drop_without_applying_deltas();
        assert_eq!(inner.left(), rect.left());
        assert_eq!(inner.max, rect.max);
        assert_eq!(inner.top(), rect.top() + if reserve { chrome::RESIZE_BAND } else { 0.0 });
    }
}

#[derive(Default)]
struct Host {
    exclusive: bool,
    stock_group: Option<u32>,
    bodies: usize,
    columns: Vec<Rect>,
    iconic: Vec<Rect>,
    groups: std::collections::HashMap<u32, Rect>,
    collapsed: bool,
    protected: bool,
    frame_width: bool,
    split_width: bool,
    space: Option<FloatSpace<u32>>,
    style_calls: std::cell::Cell<usize>,
}
impl DockContent<u32> for Host {
    fn group_collapsed(&self, _: &[u32]) -> bool {
        self.collapsed
    }
    fn permissions(&self, _: &u32) -> Permissions {
        if self.protected { Permissions::PROTECTED } else { Permissions::default() }
    }
    fn owns_floating_split_width(&self, _: &u32) -> bool {
        self.split_width
    }
    fn owns_floating_width(&self, _: &u32) -> bool {
        self.frame_width
    }
    fn resize_handle_style(&self, _: crate::layout::ResizeHandleStyle) -> crate::layout::ResizeHandleStyle {
        self.style_calls.set(self.style_calls.get() + 1);
        crate::layout::ResizeHandleStyle {
            hovered: egui::Color32::TRANSPARENT,
            dragged: egui::Color32::TRANSPARENT,
            focused: egui::Color32::TRANSPARENT,
        }
    }
    fn body(&mut self, _: &mut egui::Ui, _: &u32) {
        self.bodies += 1;
    }
    fn custom_group(&mut self, _: &mut egui::Ui, _: &[u32], selected: &u32, rect: Rect, _: bool) -> bool {
        self.groups.insert(*selected, rect);
        self.exclusive && self.stock_group.as_ref() != Some(selected)
    }
    fn owns_iconic_column(&self, _: &u32) -> bool {
        self.exclusive
    }
    fn column_rect(&mut self, _: &Context, _: &Column<u32>, rect: Rect, _: Side) {
        self.columns.push(rect);
    }
    fn iconic_column(&mut self, _: &mut egui::Ui, _: &Column<u32>, rect: Rect, _: Side) {
        self.iconic.push(rect);
    }
}
fn render(layout: &Layout<u32>, host: &mut Host) -> Vec<(Hit<u32>, Rect)> {
    let ctx = Context::default();
    for _ in 0..3 {
        ctx.run_ui(input(vec![]), |ui| {
            host.bodies = 0;
            host.columns.clear();
            host.iconic.clear();
            let mut style = DockStyle::from_ui(ui);
            super::floating_tests::frame_style(&mut style);
            let protected = host.protected;
            let shown = DockArea::new(Id::unique("host")).show_customized(
                ui,
                layout,
                &style,
                |id| format!("Pane {id}"),
                |_| if protected { Permissions::PROTECTED } else { Permissions::default() },
                |_| PanelLimits::default(),
                host,
            );
            assert!(shown.error.is_none(), "{:?}", shown.error);
        })
        .drop_without_applying_deltas();
    }
    host.space = float_space(&ctx, Id::unique("host"));
    DockArea::hit_rects(&ctx, Id::unique("host"))
}

#[test]
fn an_exclusive_group_replaces_the_stock_body_and_tab_targets_through_overrides() {
    let layout = Layout { root: Some(Node::Tabs { panels: vec![1, 2], active: 0 }), floating: vec![] };
    for exclusive in [false, true] {
        let mut host = Host { exclusive, ..Default::default() };
        let hits = render(&layout, &mut host);
        assert_eq!(host.bodies, usize::from(!exclusive));
        assert_eq!(hits.iter().any(|(hit, _)| matches!(hit, Hit::Tab(_))), !exclusive);
    }
}

#[test]
fn an_iconic_host_receives_the_fitted_column_rect_without_a_second_stock_body() {
    let mut column = Column::new(vec![Group::new(vec![1, 2], 0)], 900.0);
    column.iconic = true;
    let layout = Layout { root: Some(Node::Dock { side: Side::Start, columns: vec![column] }), floating: vec![] };
    let mut host = Host { exclusive: true, ..Default::default() };
    render(&layout, &mut host);
    assert_eq!(host.columns.len(), 1);
    assert_eq!(host.columns, host.iconic);
    assert_eq!(host.bodies, 0);
    let rect = host.columns[0];
    assert!(rect.width() > 0.0 && rect.right() <= 800.0 && rect.bottom() <= 600.0);
}

#[test]
fn invalid_side_dock_gaps_are_rejected_before_host_painting() {
    for gap in [f32::NAN, f32::INFINITY, -1.0, 4097.0] {
        let ctx = Context::default();
        let layout = Layout { root: Some(Node::Tabs { panels: vec![1], active: 0 }), floating: vec![] };
        let mut host = Host::default();
        ctx.run_ui(input(vec![]), |ui| {
            let mut style = DockStyle::from_ui(ui);
            style.dock_gap = Some(gap);
            let shown = DockArea::new(Id::unique("invalid-gap")).show_content(ui, &layout, &style, &mut host);
            assert!(shown.error.is_some(), "{gap}");
            assert_eq!(host.bodies, 0);
        })
        .drop_without_applying_deltas();
    }
}

#[test]
fn host_owned_surfaces_record_fitted_float_geometry() {
    let mut host = Host { exclusive: true, ..Default::default() };
    let layout = Layout { root: Some(Node::Tabs { panels: vec![1, 2], active: 0 }), floating: vec![] };
    render(&layout, &mut host);
    let space = host.space.as_ref().unwrap();
    assert!(space.panel(&1).is_some_and(|s| s.x > 700.0 && s.y > 0.0));
    assert!(space.group(&2).is_some_and(|s| s.x > 700.0 && s.y > 0.0));
}

#[test]
fn host_owned_iconic_columns_record_fitted_float_geometry() {
    let mut host = Host { exclusive: true, ..Default::default() };
    let mut column = Column::new(vec![Group::new(vec![1, 2], 0)], 90.0);
    column.iconic = true;
    let layout = Layout { root: Some(Node::Dock { side: Side::Start, columns: vec![column] }), floating: vec![] };
    render(&layout, &mut host);
    let size = host.space.as_ref().unwrap().column(&1).expect("host column is recorded");
    assert_eq!(size.x, host.columns[0].width());
    assert!(size.y > 0.0);
}

#[test]
fn a_host_collapsed_protected_group_keeps_its_selection_without_drawing_a_body() {
    let layout = Layout { root: Some(Node::Tabs { panels: vec![1], active: 0 }), floating: vec![] };
    let mut host = Host { protected: true, collapsed: true, ..Default::default() };
    render(&layout, &mut host);
    assert_eq!(host.bodies, 0);
    assert!(matches!(layout.root, Some(Node::Tabs { active: 0, .. })));
    assert!(host.space.as_ref().unwrap().panel(&1).is_none());
}

#[test]
fn frame_width_ownership_keeps_internal_split_input_and_host_style() {
    use crate::layout::{SplitAxis, SplitSize};
    let ctx = Context::default();
    let mut window = Floating::single(vec![1], 0, [100.0, 100.0, 500.0, 350.0]);
    window.root = Node::Split {
        axis: SplitAxis::Horizontal,
        size: SplitSize::Ratio(0.5),
        first: Box::new(Node::Tabs { panels: vec![1], active: 0 }),
        second: Box::new(Node::Tabs { panels: vec![2], active: 0 }),
    };
    let layout = Layout { root: None, floating: vec![window] };
    let mut host = Host { frame_width: true, ..Default::default() };
    let mut step = |events| {
        let mut out = DockOutput::default();
        ctx.run_ui(input(events), |ui| {
            let mut style = DockStyle::from_ui(ui);
            super::floating_tests::frame_style(&mut style);
            out = DockArea::new(Id::unique("owned-width")).show_content(ui, &layout, &style, &mut host);
        })
        .drop_without_applying_deltas();
        out
    };
    step(vec![]);
    step(vec![]);
    step(vec![]);
    let hit = Hit::Divider(Divider::WindowSplit(1, vec![]));
    let hits = DockArea::<u32>::hit_rects(&ctx, Id::unique("owned-width"));
    let at = hits.iter().find(|(h, _)| *h == hit).expect("internal divider remains interactive").1.center();
    step(vec![Event::PointerMoved(at), button(at, true)]);
    step(vec![Event::PointerMoved(at + vec2(10.0, 0.0))]);
    let out = step(vec![Event::PointerMoved(at + vec2(40.0, 0.0))]);
    assert!(out.actions.iter().any(|a| matches!(a, Action::ResizeWindowSplit { .. })), "{:?}", out.actions);
    assert!(host.style_calls.get() > 0);
}

#[test]
fn internal_width_input_is_suppressed_only_when_explicitly_owned_through_overrides() {
    use crate::layout::{SplitAxis, SplitSize};
    let mut window = Floating::single(vec![1], 0, [100.0, 100.0, 500.0, 350.0]);
    window.root = Node::Split {
        axis: SplitAxis::Horizontal,
        size: SplitSize::Ratio(0.5),
        first: Box::new(Node::Tabs { panels: vec![1], active: 0 }),
        second: Box::new(Node::Tabs { panels: vec![2], active: 0 }),
    };
    let layout = Layout { root: None, floating: vec![window] };
    for owned in [false, true] {
        let mut host = Host { frame_width: true, split_width: owned, ..Default::default() };
        let hits = render(&layout, &mut host);
        assert_eq!(hits.iter().any(|(hit, _)| *hit == Hit::Divider(Divider::WindowSplit(1, vec![]))), !owned);
    }
}

#[test]
fn exclusive_docked_groups_skip_stock_body_bands_but_keep_column_edges() {
    for owned in [false, true] {
        for (zone, anchor, inset) in [
            (Zone::Top, egui::Align2::CENTER_TOP, vec2(0.0, 50.0)),
            (Zone::Bottom, egui::Align2::CENTER_BOTTOM, vec2(0.0, -20.0)),
            (Zone::Left, egui::Align2::LEFT_CENTER, vec2(20.0, 0.0)),
            (Zone::Right, egui::Align2::RIGHT_CENTER, vec2(-20.0, 0.0)),
        ] {
            let ctx = Context::default();
            let area = Id::unique("owned-dock-targets");
            let layout = Layout {
                root: Some(Node::Dock { side: Side::Start, columns: vec![Column::new(vec![Group::new(vec![1, 2], 0)], 240.0)] }),
                floating: vec![Floating::single(vec![99], 0, [500.0, 100.0, 200.0, 150.0])],
            };
            let mut host = Host { exclusive: owned, stock_group: Some(99), ..Default::default() };
            let mut step = |events, start: bool| {
                let mut out = DockOutput::default();
                ctx.run_ui(input(events), |ui| {
                    let mut style = DockStyle::from_ui(ui);
                    super::floating_tests::frame_style(&mut style);
                    out = DockArea::new(area).show_content(ui, &layout, &style, &mut host);
                    if start {
                        assert!(begin_drag(ui.ctx(), area, 99, Rect::from_min_size(pos2(500.0, 100.0), vec2(200.0, 150.0))));
                    }
                })
                .drop_without_applying_deltas();
                (out, host.groups.get(&1).copied())
            };
            step(vec![], false);
            step(vec![], false);
            let (_, target) = step(vec![], false);
            let target = target.expect("the docked group is drawn");
            let from = pos2(550.0, 140.0);
            step(vec![Event::PointerMoved(from), button(from, true)], true);
            let to = anchor.pos_in_rect(&target) + inset;
            step(vec![Event::PointerMoved(to)], false);
            let (out, _) = step(vec![Event::PointerMoved(to), button(to, false)], false);
            let group_drop = out.actions.iter().any(|a| {
                matches!(
                    a,
                    Action::MovePanel { to: Target::At { placement: Placement::Group { .. }, .. }, .. }
                        | Action::MoveGroup { to: Target::At { placement: Placement::Group { .. }, .. }, .. }
                )
            });
            if matches!(zone, Zone::Top | Zone::Bottom) {
                assert_eq!(group_drop, !owned, "{owned} {zone:?}: {:?}", out.actions);
            } else {
                assert!(!out.actions.is_empty(), "column edges still work: {owned} {zone:?}");
                assert!(!group_drop, "column edges must not become group bands");
            }
        }
    }
}

#[test]
fn owned_minimized_groups_skip_the_trailing_stock_target() {
    for owned in [false, true] {
        let ctx = Context::default();
        let area = Id::unique("owned-dock-targets");
        let mut layout = Layout {
            root: Some(Node::Dock { side: Side::Start, columns: vec![Column::new(vec![Group::new(vec![1, 2], 0)], 240.0)] }),
            floating: vec![Floating::single(vec![99], 0, [500.0, 100.0, 200.0, 150.0])],
        };
        if let Some(Node::Dock { columns, .. }) = &mut layout.root {
            columns[0].groups[0].minimized = true;
        }
        let mut host = Host { exclusive: owned, stock_group: Some(99), ..Default::default() };
        let mut step = |events, start: bool| {
            let mut out = DockOutput::default();
            ctx.run_ui(input(events), |ui| {
                let mut style = DockStyle::from_ui(ui);
                super::floating_tests::frame_style(&mut style);
                out = DockArea::new(area).show_content(ui, &layout, &style, &mut host);
                if start {
                    assert!(begin_drag(ui.ctx(), area, 99, Rect::from_min_size(pos2(500.0, 100.0), vec2(200.0, 150.0))));
                }
            })
            .drop_without_applying_deltas();
            (out, host.groups.get(&1).copied())
        };
        step(vec![], false);
        step(vec![], false);
        let (_, target) = step(vec![], false);
        let target = target.expect("the docked group is drawn");
        let from = pos2(550.0, 140.0);
        step(vec![Event::PointerMoved(from), button(from, true)], true);
        let to = pos2(target.center().x, target.bottom() + 60.0);
        step(vec![Event::PointerMoved(to)], false);
        let (out, _) = step(vec![Event::PointerMoved(to), button(to, false)], false);
        let group_drop = out.actions.iter().any(|a| {
            matches!(
                a,
                Action::MovePanel { to: Target::At { placement: Placement::Group { .. }, .. }, .. }
                    | Action::MoveGroup { to: Target::At { placement: Placement::Group { .. }, .. }, .. }
            )
        });
        assert_eq!(group_drop, !owned, "owned={owned}: {:?}", out.actions);
    }
}

#[test]
fn disabled_edge_axes_ignore_pointer_drags_and_enabled_edges_obey_limits() {
    for axes in [[true, true], [true, false], [false, true], [false, false]] {
        for horizontal in [true, false] {
            for grow in [true, false] {
                let ctx = Context::default();
                let rect = Rect::from_min_size(pos2(400.0, 300.0), vec2(300.0, 200.0));
                let bounds = Rect::from_min_max(pos2(0.0, 0.0), pos2(720.0, 520.0));
                let frame = |events| {
                    let mut resized = None;
                    ctx.run_ui(input(events), |ui| {
                        let style = chrome::ChromeStyle::from_ui(ui);
                        resized = chrome::FloatingFrame::new(Id::unique("edge-axis"), rect)
                            .resize_axes(axes)
                            .bounds(bounds)
                            .limits(PanelLimits { min: vec2(160.0, 120.0), max: vec2(310.0, 210.0) })
                            .show(ui.ctx(), &style, |_, _| ())
                            .resized;
                    })
                    .drop_without_applying_deltas();
                    resized
                };
                frame(vec![]);
                frame(vec![]);
                let from = if horizontal { pos2(rect.right() - 2.0, rect.center().y) } else { pos2(rect.center().x, rect.bottom() - 2.0) };
                frame(vec![Event::PointerMoved(from), button(from, true)]);
                let delta = if horizontal { vec2(10.0, 0.0) } else { vec2(0.0, 10.0) };
                frame(vec![Event::PointerMoved(from + delta)]);
                let result = frame(vec![Event::PointerMoved(from + delta * if grow { 100.0 } else { -100.0 })]);
                let enabled = if horizontal { axes[0] } else { axes[1] };
                let expected = if horizontal { vec2(if grow { 310.0 } else { 160.0 }, 200.0) } else { vec2(300.0, if grow { 210.0 } else { 120.0 }) };
                assert_eq!(result, enabled.then(|| Rect::from_min_size(rect.min, expected)), "{axes:?} horizontal={horizontal} grow={grow}");
            }
        }
    }
}

#[test]
fn frame_at_viewport_edge_cannot_resize_beyond_it() {
    let ctx = Context::default();
    let rect = Rect::from_min_max(pos2(400.0, 300.0), pos2(800.0, 600.0));
    let frame = |events| {
        let mut result = None;
        ctx.run_ui(input(events), |ui| {
            let style = chrome::ChromeStyle::from_ui(ui);
            let response = chrome::FloatingFrame::new(Id::unique("viewport-edge"), rect).show(ui.ctx(), &style, |_, _| ());
            assert_eq!(response.rect, rect);
            result = response.resized;
        })
        .drop_without_applying_deltas();
        result
    };
    frame(vec![]);
    frame(vec![]);
    let from = rect.max - vec2(2.0, 2.0);
    frame(vec![Event::PointerMoved(from), button(from, true)]);
    frame(vec![Event::PointerMoved(from + vec2(10.0, 10.0))]);
    assert_eq!(frame(vec![Event::PointerMoved(from + vec2(100.0, 100.0))]), None);
}
