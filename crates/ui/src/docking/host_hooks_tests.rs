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
    bodies: usize,
    columns: Vec<Rect>,
    iconic: Vec<Rect>,
}
impl DockContent<u32> for Host {
    fn body(&mut self, _: &mut egui::Ui, _: &u32) {
        self.bodies += 1;
    }
    fn custom_group(&mut self, _: &mut egui::Ui, _: &[u32], _: &u32, _: Rect, _: bool) -> bool {
        self.exclusive
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
            let shown = DockArea::new(Id::unique("host")).show_customized(
                ui,
                layout,
                &style,
                |id| format!("Pane {id}"),
                |_| Permissions::default(),
                |_| PanelLimits::default(),
                host,
            );
            assert!(shown.error.is_none(), "{:?}", shown.error);
        })
        .drop_without_applying_deltas();
    }
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
