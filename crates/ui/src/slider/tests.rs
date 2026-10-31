use super::*;
use egui::{Context, Event, FullOutput, Modifiers, PointerButton, Pos2, RawInput};

struct SliderTest {
    ctx: Context,
    style: SliderStyle,
    value: f64,
    log: bool,
    step: Option<f64>,
    field: bool,
    rect: Option<Rect>,
    changes: usize,
}

impl SliderTest {
    fn new(value: f64) -> Self {
        let ctx = Context::default();
        ctx.enable_accesskit();
        ctx.global_style_mut(|s| s.animation_time = 0.0);
        let style = SliderStyle::from_visuals(&egui::Visuals::light());
        Self { ctx, style, value, log: false, step: None, field: false, rect: None, changes: 0 }
    }

    fn frame(&mut self, events: Vec<Event>) -> (Response, FullOutput) {
        let ctx = self.ctx.clone();
        let mut response = None;
        let mut output =
            ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))), events, ..Default::default() }, |ui| {
                let mut s = Slider::new(&mut self.value, 10.0..=400.0, "Zoom", &self.style).width(200.0).mark(100.0);
                if self.log {
                    s = s.scale(Scale::Logarithmic);
                }
                if let Some(step) = self.step {
                    s = s.step(step);
                }
                if self.field {
                    s = s.editable(0, "%");
                }
                let r = s.show(ui);
                self.changes += usize::from(r.changed());
                self.rect = Some(r.rect);
                response = Some(r);
            });
        output.textures_delta.clear();
        (response.unwrap(), output)
    }

    /// The track's left and right thumb centres, from the first frame's layout.
    fn track(&mut self) -> (f32, f32, f32) {
        let (r, _) = self.frame(vec![]);
        (r.rect.left() + THUMB_RADIUS, r.rect.right() - THUMB_RADIUS, r.rect.center().y)
    }

    fn click(&mut self, x: f32, y: f32) {
        let p = pos2(x, y);
        self.frame(vec![Event::PointerMoved(p), pointer(p, true)]);
        self.frame(vec![pointer(p, false)]);
        self.frame(vec![]);
    }
}

fn pointer(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn key(key: Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

#[test]
fn clicking_the_track_sets_the_value_linearly_and_logarithmically() {
    let mut t = SliderTest::new(100.0);
    let (x0, x1, y) = t.track();
    t.click(x0 + (x1 - x0) * 0.5, y);
    assert!((t.value - 205.0).abs() < 1.0, "linear midpoint of 10–400: {}", t.value);
    assert!(t.changes > 0);

    let mut t = SliderTest::new(100.0);
    t.log = true;
    t.step = Some(1.0);
    let (x0, x1, y) = t.track();
    t.click(x0 + (x1 - x0) * 0.5, y);
    assert_eq!(t.value, 63.0, "logarithmic midpoint of 10–400 is √4000 ≈ 63, snapped to whole percent");
    t.click(x0, y);
    assert_eq!(t.value, 10.0);
    t.click(x1 + 50.0, y);
    assert_eq!(t.value, 10.0, "a click past the track isn't on the slider");
}

#[test]
fn double_click_returns_to_the_mark() {
    let mut t = SliderTest::new(250.0);
    t.log = true;
    let (x0, _, y) = t.track();
    let p = pos2(x0 + 20.0, y);
    t.frame(vec![Event::PointerMoved(p), pointer(p, true)]);
    t.frame(vec![pointer(p, false)]);
    t.frame(vec![pointer(p, true)]);
    t.frame(vec![pointer(p, false)]);
    t.frame(vec![]);
    assert_eq!(t.value, 100.0);
}

#[test]
fn keyboard_moves_by_step_page_and_to_the_ends() {
    let mut t = SliderTest::new(100.0);
    t.step = Some(1.0);
    let (r, _) = t.frame(vec![]);
    t.ctx.memory_mut(|m| m.request_focus(r.id));
    t.frame(vec![]);
    t.frame(vec![key(Key::ArrowRight)]);
    assert_eq!(t.value, 101.0);
    t.frame(vec![key(Key::PageDown)]);
    assert_eq!(t.value, 91.0);
    t.frame(vec![key(Key::End)]);
    assert_eq!(t.value, 400.0);
    t.frame(vec![key(Key::Home)]);
    assert_eq!(t.value, 10.0);
}

/// Home and End reach the range's ends even when they are off the step grid.
/// An editable slider's value field is named like the slider, so no input is unnamed.
#[test]
fn the_value_field_is_named_like_the_slider() {
    let mut t = SliderTest::new(100.0);
    t.field = true;
    let (_, out) = t.frame(vec![]);
    let update = out.platform_output.accesskit_update.expect("accesskit is on");
    let field = update.nodes.iter().map(|(_, n)| n).find(|n| n.role() == egui::accesskit::Role::SpinButton).expect("a value field");
    assert_eq!(field.label(), Some("Zoom"));
}

#[test]
fn home_and_end_reach_off_grid_ends() {
    let ctx = Context::default();
    let style = SliderStyle::from_visuals(&egui::Visuals::light());
    let mut v = 5.0;
    let run = |events: Vec<Event>, v: &mut f64| -> egui::Id {
        let mut id = None;
        let mut out = ctx.run_ui(RawInput { events, ..Default::default() }, |ui| {
            id = Some(Slider::new(v, 0.3..=10.3, "Level", &style).step(1.0).show(ui).id);
        });
        out.textures_delta.clear();
        id.expect("laid out")
    };
    let id = run(vec![], &mut v);
    ctx.memory_mut(|m| m.request_focus(id));
    run(vec![], &mut v);
    run(vec![key(Key::End)], &mut v);
    assert_eq!(v, 10.3);
    run(vec![key(Key::Home)], &mut v);
    assert_eq!(v, 0.3);
    run(vec![key(Key::ArrowRight)], &mut v);
    assert_eq!(v, 1.0, "arrows still snap to the step");
}

#[test]
fn the_field_keeps_its_width_and_the_track_does_not_move() {
    let mut t = SliderTest::new(99.0);
    t.field = true;
    let (a, _) = t.frame(vec![]);
    t.value = 100.0;
    let (b, _) = t.frame(vec![]);
    assert_eq!(a.rect, b.rect, "track and field together keep their size from 99% to 100%");
}

#[test]
fn non_finite_values_and_empty_ranges_are_harmless() {
    let mut t = SliderTest::new(f64::NAN);
    t.frame(vec![]);
    assert_eq!(t.value, 10.0);
    let style = SliderStyle::from_visuals(&egui::Visuals::dark());
    let ctx = Context::default();
    let mut v = 5.0;
    let mut out = ctx.run_ui(RawInput::default(), |ui| {
        Slider::new(&mut v, 3.0..=3.0, "Empty", &style).scale(Scale::Logarithmic).show(ui);
    });
    out.textures_delta.clear();
    assert_eq!(v, 5.0, "showing never changes a value nobody touched");
}

#[test]
fn assistive_technology_sees_a_labelled_slider() {
    let mut t = SliderTest::new(100.0);
    let (_, out) = t.frame(vec![]);
    let update = out.platform_output.accesskit_update.expect("accesskit is on");
    let node = update.nodes.iter().map(|(_, n)| n).find(|n| n.role() == egui::accesskit::Role::Slider).expect("a slider node");
    assert_eq!(node.numeric_value(), Some(100.0));
    assert_eq!(node.label(), Some("Zoom"));
    assert_eq!((node.min_numeric_value(), node.max_numeric_value()), (Some(10.0), Some(400.0)), "the range too");
}

#[test]
fn assistive_technology_reads_the_zoom_bar_in_percent() {
    let ctx = Context::default();
    ctx.enable_accesskit();
    let mut p = 100.0;
    let style = ZoomBarStyle {
        slider: SliderStyle::from_visuals(&egui::Visuals::light()),
        icon: Color32::GRAY,
        hover: Color32::LIGHT_GRAY,
        text: Color32::GRAY,
        font: 12.0,
    };
    let labels = ZoomBarLabels { zoom: "Zoom", zoom_in: "Zoom In", zoom_out: "Zoom Out" };
    let mut out = ctx.run_ui(RawInput::default(), |ui| {
        ZoomBar::new(&mut p, 10.0..=400.0, labels, &style).percent_label("Zoom level").show(ui);
    });
    out.textures_delta.clear();
    let update = out.platform_output.accesskit_update.expect("accesskit is on");
    let nodes: Vec<_> = update.nodes.iter().map(|(_, n)| n).collect();
    let slider = nodes.iter().find(|n| n.role() == egui::accesskit::Role::Slider).expect("a slider node");
    assert_eq!((slider.label(), slider.value()), (Some("Zoom"), Some("100%")));
    assert_eq!((slider.min_numeric_value(), slider.max_numeric_value(), slider.numeric_value_step()), (Some(10.0), Some(400.0), Some(1.0)));
    let level = nodes.iter().find(|n| n.label() == Some("Zoom level")).expect("the percentage is named");
    assert_eq!((level.role(), level.value()), (egui::accesskit::Role::Button, Some("100%")));
    assert!(nodes.iter().all(|n| !n.is_disabled()), "an enabled bar reads as enabled");

    // In a disabled Ui every part, buttons and percentage included, reads as disabled.
    let mut out = ctx.run_ui(RawInput::default(), |ui| {
        ui.disable();
        ZoomBar::new(&mut p, 10.0..=400.0, labels, &style).percent_label("Zoom level").show(ui);
    });
    out.textures_delta.clear();
    let update = out.platform_output.accesskit_update.expect("accesskit is on");
    for name in ["Zoom", "Zoom In", "Zoom Out", "Zoom level"] {
        let node = update.nodes.iter().map(|(_, n)| n).find(|n| n.label() == Some(name)).unwrap_or_else(|| panic!("{name}"));
        assert!(node.is_disabled(), "{name} reads as disabled");
    }
}

#[test]
fn the_field_stays_beside_the_track_in_a_vertical_layout() {
    let ctx = Context::default();
    let style = SliderStyle::from_visuals(&egui::Visuals::light());
    let mut v = 1.0;
    let mut rect = None;
    let mut out = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0))), ..Default::default() }, |ui| {
        ui.vertical(|ui| rect = Some(Slider::new(&mut v, 0.5..=2.0, "Speed", &style).editable(2, "×").show(ui).rect));
    });
    out.textures_delta.clear();
    assert!(rect.unwrap().height() <= HEIGHT + 4.0, "one row: {:?}", rect);
}

fn zoom_frame(ctx: &Context, percent: &mut f64, events: Vec<Event>) -> ZoomBarResponse {
    let style = ZoomBarStyle {
        slider: SliderStyle::from_visuals(&egui::Visuals::light()),
        icon: Color32::GRAY,
        hover: Color32::LIGHT_GRAY,
        text: Color32::GRAY,
        font: 12.0,
    };
    let labels = ZoomBarLabels { zoom: "Zoom", zoom_in: "Zoom In", zoom_out: "Zoom Out" };
    let mut out = None;
    let mut o = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 100.0))), events, ..Default::default() }, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            out = Some(ZoomBar::new(percent, 10.0..=500.0, labels, &style).stops(&[10.0, 25.0, 50.0, 75.0, 100.0, 150.0, 200.0, 500.0]).show(ui));
        });
    });
    o.textures_delta.clear();
    out.unwrap()
}

#[test]
fn zoom_bar_keeps_its_size_and_steps_through_its_stops() {
    let ctx = Context::default();
    let mut p = 99.0;
    let a = zoom_frame(&ctx, &mut p, vec![]).response.rect;
    p = 100.0;
    let b = zoom_frame(&ctx, &mut p, vec![]).response.rect;
    assert_eq!(a, b, "same block from 99% to 100%, in a right-to-left layout too");
    // The `+` button is the second 18 pt square from the right edge of the slider; find it by
    // clicking left to right across the block and watching the value.
    let y = b.center().y;
    let plus_x = b.left() + 18.0 + 110.0 + 9.0;
    for e in [vec![Event::PointerMoved(pos2(plus_x, y)), pointer(pos2(plus_x, y), true)], vec![pointer(pos2(plus_x, y), false)]] {
        zoom_frame(&ctx, &mut p, e);
    }
    assert_eq!(p, 150.0, "+ goes to the next stop");
    let minus_x = b.left() + 9.0;
    for e in [vec![Event::PointerMoved(pos2(minus_x, y)), pointer(pos2(minus_x, y), true)], vec![pointer(pos2(minus_x, y), false)]] {
        zoom_frame(&ctx, &mut p, e);
    }
    assert_eq!(p, 100.0, "- goes to the previous stop");
    // From between stops, + and - go to the nearest stop on that side, not past it.
    for (from, want) in [(99.7, 100.0), (100.3, 100.0)] {
        p = from;
        let x = if want > from { plus_x } else { minus_x };
        for e in [vec![Event::PointerMoved(pos2(x, y)), pointer(pos2(x, y), true)], vec![pointer(pos2(x, y), false)]] {
            zoom_frame(&ctx, &mut p, e);
        }
        assert_eq!(p, want, "from {from}");
    }
}

#[test]
fn typed_values_drop_the_suffix_even_when_it_holds_a_digit() {
    assert_eq!(parse_typed("120%", "%"), Some(120.0));
    assert_eq!(parse_typed(" 12,5 pt ", "pt"), Some(12.5));
    assert_eq!(parse_typed("24fps", "fps"), Some(24.0));
    assert_eq!(parse_typed("30 f/1s", "f/1s"), Some(30.0), "the suffix's own digit is not read as part of the number");
    assert_eq!(parse_typed("24 fp", "fps"), Some(24.0), "a half-typed suffix");
    assert_eq!(parse_typed("fast", "%"), None);
    // Anything else after the number is not a value; a trailing separator still is.
    assert_eq!(parse_typed("12xyz", "%"), None);
    assert_eq!(parse_typed("12 pt", "%"), None);
    assert_eq!(parse_typed("12%x", "%"), None);
    assert_eq!(parse_typed("12.", "%"), Some(12.0));
    assert_eq!(parse_typed("12,", "%"), Some(12.0));
    assert_eq!(parse_typed("12", ""), Some(12.0));
    assert_eq!(parse_typed("12x", ""), None);
}

#[test]
fn a_drag_reports_where_it_ended_to_a_caller_that_applies_on_release() {
    // The caller keeps its own value and re-seeds the slider with it every frame, applying only
    // when the drag ends: the release frame must hand back the dragged-to value.
    let ctx = Context::default();
    let style = SliderStyle::from_visuals(&egui::Visuals::light());
    let mut model = 100.0;
    let mut applied = Vec::new();
    let frame = |events: Vec<Event>, model: &mut f64, applied: &mut Vec<f64>| -> Rect {
        let mut rect = Rect::NOTHING;
        let mut out =
            ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))), events, ..Default::default() }, |ui| {
                let mut v = *model;
                let r = Slider::new(&mut v, 10.0..=400.0, "Size", &style).width(200.0).show(ui);
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    applied.push(v);
                    *model = v;
                }
                rect = r.rect;
            });
        out.textures_delta.clear();
        rect
    };
    let rect = frame(vec![], &mut model, &mut applied);
    let (x0, x1, y) = (rect.left() + THUMB_RADIUS, rect.right() - THUMB_RADIUS, rect.center().y);
    let from = pos2(x0 + 2.0, y);
    let to = pos2(x1, y);
    frame(vec![Event::PointerMoved(from), pointer(from, true)], &mut model, &mut applied);
    for k in 1..=6 {
        let p = from.lerp(to, k as f32 / 6.0);
        frame(vec![Event::PointerMoved(p)], &mut model, &mut applied);
    }
    assert!(applied.is_empty(), "nothing applies while the slider is held: {applied:?}");
    frame(vec![pointer(to, false)], &mut model, &mut applied);
    frame(vec![], &mut model, &mut applied);
    assert_eq!(applied.len(), 1, "{applied:?}");
    assert!((model - 400.0).abs() < 1e-6, "the release applies the end of the range, got {model}");
}
