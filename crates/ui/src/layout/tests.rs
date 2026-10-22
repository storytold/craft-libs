use super::*;
use egui::{Color32, Context, Event, FullOutput, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2, accesskit, pos2, vec2};

fn sample() -> Rect {
    Rect::from_min_max(pos2(10.0, 20.0), pos2(410.0, 220.0))
}

#[test]
fn ordinary_splits_keep_relative_and_fixed_point_sizes() {
    for (request, first, second) in
        [(SplitSize::Ratio(0.25), 99.0, 297.0), (SplitSize::FixedFirst(150.0), 150.0, 246.0), (SplitSize::FixedSecond(150.0), 246.0, 150.0)]
    {
        let layout = split_rect(sample(), SplitAxis::Horizontal, request, 4.0, 20.0).unwrap();
        assert_eq!(layout.first.width(), first);
        assert_eq!(layout.second.width(), second);
        assert_eq!(layout.divider.width(), 4.0);
        assert_eq!(layout.first.max.x, layout.divider.min.x);
        assert_eq!(layout.divider.max.x, layout.second.min.x);
        assert_eq!(layout.first.min, sample().min);
        assert_eq!(layout.second.max, sample().max);
    }
    let layout = split_rect(sample(), SplitAxis::Vertical, SplitSize::Ratio(0.5), 2.0, 0.0).unwrap();
    assert_eq!(layout.first.height(), 99.0);
    assert_eq!(layout.second.height(), 99.0);
    assert_eq!(layout.divider.height(), 2.0);
}

#[test]
fn tiny_panes_never_invert_clamp_bounds_or_escape_the_parent() {
    for span in [0.0, 1.0, 15.0, 20.0, 35.0, 39.9, 40.0, 80.0, 1000.0] {
        for axis in [SplitAxis::Horizontal, SplitAxis::Vertical] {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(span));
            for gap in [0.0, 4.0, 2000.0] {
                for size in [
                    SplitSize::Ratio(-10.0),
                    SplitSize::Ratio(0.5),
                    SplitSize::Ratio(10.0),
                    SplitSize::FixedFirst(300.0),
                    SplitSize::FixedSecond(300.0),
                ] {
                    let layout = split_rect(rect, axis, size, gap, 20.0).unwrap();
                    for child in [layout.first, layout.divider, layout.second] {
                        assert!(rect.contains_rect(child), "{rect:?}: {child:?}");
                        assert!(child.width() >= 0.0 && child.height() >= 0.0);
                        assert!(child.min.is_finite() && child.max.is_finite());
                    }
                }
            }
        }
    }
}

#[test]
fn invalid_inputs_return_none_and_extreme_finite_requests_stay_bounded() {
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for size in [SplitSize::Ratio(invalid), SplitSize::FixedFirst(invalid), SplitSize::FixedSecond(invalid)] {
            assert!(split_rect(sample(), SplitAxis::Horizontal, size, 4.0, 20.0).is_none());
        }
        assert!(split_rect(sample(), SplitAxis::Horizontal, SplitSize::Ratio(0.5), invalid, 20.0).is_none());
        assert!(split_rect(sample(), SplitAxis::Horizontal, SplitSize::Ratio(0.5), 4.0, invalid).is_none());
        let mut rect = sample();
        rect.min.x = invalid;
        assert!(split_rect(rect, SplitAxis::Horizontal, SplitSize::Ratio(0.5), 4.0, 20.0).is_none());
    }
    assert!(split_rect(sample(), SplitAxis::Horizontal, SplitSize::Ratio(0.5), -1.0, 20.0).is_none());
    assert!(split_rect(sample(), SplitAxis::Horizontal, SplitSize::Ratio(0.5), 4.0, -1.0).is_none());
    assert!(split_rect(Rect { min: sample().max, max: sample().min }, SplitAxis::Horizontal, SplitSize::Ratio(0.5), 4.0, 20.0).is_none());
    let huge = Rect::from_min_max(pos2(-f32::MAX, 0.0), pos2(f32::MAX, 100.0));
    assert!(split_rect(huge, SplitAxis::Horizontal, SplitSize::Ratio(0.5), 4.0, 20.0).is_none());
    let huge = Rect::from_min_max(pos2(0.0, 0.0), pos2(f32::MAX, 100.0));
    for request in [SplitSize::Ratio(f32::MAX), SplitSize::FixedFirst(f32::MAX), SplitSize::FixedSecond(-f32::MAX)] {
        let layout = split_rect(huge, SplitAxis::Horizontal, request, 4.0, 20.0).unwrap();
        for child in [layout.first, layout.divider, layout.second] {
            assert!(huge.contains_rect(child));
            assert!(child.min.is_finite() && child.max.is_finite());
        }
    }
}

#[test]
fn resizing_retains_sizing_mode_and_bounded_minima() {
    let layout = split_rect(sample(), SplitAxis::Horizontal, SplitSize::FixedFirst(100.0), 4.0, 20.0).unwrap();
    assert_eq!(layout.resized(SplitSize::Ratio(0.25), 20.0, 40.0), Some(SplitSize::Ratio(120.0 / 396.0)));
    assert_eq!(layout.resized(SplitSize::FixedFirst(100.0), 20.0, 40.0), Some(SplitSize::FixedFirst(120.0)));
    assert_eq!(layout.resized(SplitSize::FixedSecond(296.0), 20.0, 40.0), Some(SplitSize::FixedSecond(276.0)));
    assert_eq!(layout.resized(SplitSize::FixedFirst(100.0), -1000.0, 40.0), Some(SplitSize::FixedFirst(40.0)));
    assert_eq!(layout.resized(SplitSize::FixedFirst(100.0), 1000.0, 40.0), Some(SplitSize::FixedFirst(356.0)));
    let tiny = split_rect(Rect::from_min_size(Pos2::ZERO, vec2(19.0, 100.0)), SplitAxis::Horizontal, SplitSize::Ratio(0.5), 4.0, 20.0).unwrap();
    assert_eq!(tiny.resized(SplitSize::FixedFirst(7.5), 100.0, 40.0), Some(SplitSize::FixedFirst(7.5)));
    assert_eq!(tiny.resized(SplitSize::FixedSecond(7.5), -100.0, 40.0), Some(SplitSize::FixedSecond(7.5)));
    assert!(tiny.resized(SplitSize::Ratio(0.5), f32::NAN, 40.0).is_none());
    let zero = split_rect(Rect::ZERO, SplitAxis::Horizontal, SplitSize::Ratio(0.5), 4.0, 20.0).unwrap();
    assert!(zero.resized(SplitSize::Ratio(0.5), 10.0, 40.0).is_none());
}

fn handle_style() -> ResizeHandleStyle {
    ResizeHandleStyle { hovered: Color32::GREEN, dragged: Color32::RED, focused: Color32::BLUE }
}

struct HandleTest {
    ctx: Context,
    axis: SplitAxis,
    rect: Rect,
    label: String,
    enabled: bool,
    value: f32,
}

impl HandleTest {
    fn new() -> Self {
        let ctx = Context::default();
        ctx.enable_accesskit();
        Self {
            ctx,
            axis: SplitAxis::Horizontal,
            rect: Rect::from_min_size(pos2(100.0, 20.0), vec2(4.0, 140.0)),
            label: "Resize panels".into(),
            enabled: true,
            value: 100.0,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> (ResizeResponse, FullOutput) {
        let ctx = self.ctx.clone();
        let mut result = None;
        let mut output =
            ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0))), events, ..Default::default() }, |ui| {
                result = Some(
                    ui.add_enabled_ui(self.enabled, |ui| {
                        ResizeHandle::new(Id::new("divider"), self.axis, self.rect, &self.label, handle_style())
                            .value(self.value, 0.0..=300.0)
                            .show(ui)
                    })
                    .inner,
                );
            });
        output.textures_delta.clear();
        (result.unwrap(), output)
    }

    fn settle(&mut self) -> ResizeResponse {
        self.frame(vec![]);
        self.frame(vec![]).0
    }
}

fn key(key: Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

fn pointer(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn accesskit_event(id: Id, action: accesskit::Action, data: Option<accesskit::ActionData>) -> Event {
    Event::AccessKitActionRequest(accesskit::ActionRequest { action, target_tree: accesskit::TreeId::ROOT, target_node: id.accesskit_id(), data })
}

#[test]
fn resize_handle_preserves_pointer_drag_lifecycle_and_reports_per_frame_delta() {
    let mut h = HandleTest::new();
    let center = h.rect.center();
    h.settle();
    h.frame(vec![Event::PointerMoved(center)]);
    h.frame(vec![pointer(center, true)]);
    let (response, _) = h.frame(vec![Event::PointerMoved(center + vec2(12.0, 0.0))]);
    assert!(response.response.dragged());
    assert_eq!(response.delta, 12.0);
    let (response, _) = h.frame(vec![Event::PointerMoved(center + vec2(16.0, 0.0))]);
    assert_eq!(response.delta, 4.0);
    let (response, _) = h.frame(vec![pointer(center + vec2(16.0, 0.0), false)]);
    assert!(response.response.drag_stopped());
    assert_eq!(response.delta, 0.0);
    assert!(!response.response.has_focus(), "pointer release must not leave a sticky focus highlight");
    let (_, output) = h.frame(vec![Event::PointerMoved(pos2(300.0, 180.0))]);
    assert!(
        !output
            .shapes
            .iter()
            .any(|shape| { matches!(&shape.shape, egui::epaint::Shape::Rect(rect) if rect.rect == h.rect && rect.fill == Color32::BLUE) })
    );
    assert_eq!(h.value, 100.0, "the component never owns pane state");
}

#[test]
fn focused_handle_resizes_with_axis_arrows_and_draws_visible_focus() {
    for (axis, forward, back) in [(SplitAxis::Horizontal, Key::ArrowRight, Key::ArrowLeft), (SplitAxis::Vertical, Key::ArrowDown, Key::ArrowUp)] {
        let mut h = HandleTest::new();
        h.axis = axis;
        let response = h.settle();
        h.ctx.memory_mut(|memory| memory.request_focus(response.response.id));
        h.frame(vec![]);
        let (response, output) = h.frame(vec![key(forward)]);
        assert_eq!(response.delta, 8.0);
        assert!(response.response.has_focus());
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::epaint::Shape::Rect(rect) if rect.rect == h.rect && rect.fill == Color32::BLUE))
        );
        assert_eq!(h.frame(vec![key(back)]).0.delta, -8.0);
    }
}

#[test]
fn unfocused_disabled_and_invalid_handles_do_not_resize() {
    let mut h = HandleTest::new();
    let response = h.settle();
    assert_eq!(h.frame(vec![key(Key::ArrowRight)]).0.delta, 0.0);
    h.ctx.memory_mut(|memory| memory.request_focus(response.response.id));
    h.enabled = false;
    let (response, output) = h.frame(vec![key(Key::ArrowRight)]);
    assert_eq!(response.delta, 0.0);
    assert!(!response.response.enabled());
    let tree = output.platform_output.accesskit_update.unwrap();
    let (_, node) = tree.nodes.iter().find(|(_, node)| node.label() == Some("Resize panels")).unwrap();
    assert!(node.is_disabled());
    h.enabled = true;
    h.rect.min.x = f32::NAN;
    assert_eq!(h.frame(vec![key(Key::ArrowRight)]).0.delta, 0.0);
}

#[test]
fn accessible_splitter_has_identity_orientation_value_and_actions() {
    let mut h = HandleTest::new();
    let response = h.settle();
    let id = response.response.id;
    h.label = "パネル幅を変更".into();
    let (response, output) = h.frame(vec![]);
    assert_eq!(response.response.id, id);
    let tree = output.platform_output.accesskit_update.unwrap();
    let (_, node) = tree.nodes.iter().find(|(_, node)| node.label() == Some(h.label.as_str())).unwrap();
    assert_eq!(node.role(), accesskit::Role::Splitter);
    assert_eq!(node.orientation(), Some(accesskit::Orientation::Vertical));
    assert_eq!(node.numeric_value(), Some(100.0));
    assert_eq!(node.min_numeric_value(), Some(0.0));
    assert_eq!(node.max_numeric_value(), Some(300.0));
    assert!(node.supports_action(accesskit::Action::Increment));
    assert!(node.supports_action(accesskit::Action::Decrement));
    assert!(node.supports_action(accesskit::Action::SetValue));
    assert_eq!(h.frame(vec![accesskit_event(id, accesskit::Action::Increment, None)]).0.delta, 8.0);
    assert_eq!(h.frame(vec![accesskit_event(id, accesskit::Action::Decrement, None)]).0.delta, -8.0);
    assert_eq!(h.frame(vec![accesskit_event(id, accesskit::Action::SetValue, Some(accesskit::ActionData::NumericValue(250.0)))]).0.delta, 150.0);
    assert_eq!(h.frame(vec![accesskit_event(id, accesskit::Action::SetValue, Some(accesskit::ActionData::NumericValue(f64::NAN)))]).0.delta, 0.0);
}
