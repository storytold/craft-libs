use super::*;
use egui::{Context, Event, FullOutput, Key, Modifiers, PointerButton, Pos2, RawInput, pos2, vec2};

fn style() -> IconButtonStyle {
    let visuals = |fill, icon| ButtonVisuals { fill, stroke: Stroke::NONE, icon };
    IconButtonStyle {
        normal: visuals(Color32::TRANSPARENT, Color32::WHITE),
        hovered: visuals(Color32::DARK_GRAY, Color32::GREEN),
        pressed: Some(visuals(Color32::BLUE, Color32::RED)),
        selected: ButtonVisuals { fill: Color32::GRAY, stroke: Stroke::new(1.0, Color32::WHITE), icon: Color32::YELLOW },
        corner_radius: CornerRadius::same(4),
        focus_stroke: Stroke::new(1.0, Color32::LIGHT_BLUE),
    }
}

struct ButtonTest {
    ctx: Context,
    style: IconButtonStyle,
    enabled: bool,
    selected: bool,
    label: String,
    size: Vec2,
    clicks: usize,
    tint: Option<Color32>,
    animated_hover: bool,
    time: f64,
}

impl ButtonTest {
    fn new() -> Self {
        let ctx = Context::default();
        ctx.enable_accesskit();
        ctx.global_style_mut(|s| s.animation_time = 0.0);
        Self {
            ctx,
            style: style(),
            enabled: true,
            selected: false,
            label: "Inspect layers".into(),
            size: vec2(32.0, 32.0),
            clicks: 0,
            tint: None,
            animated_hover: false,
            time: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> (Response, FullOutput) {
        let ctx = self.ctx.clone();
        let mut response = None;
        self.tint = None;
        self.time += 1.0 / 60.0;
        let mut output = ctx.run_ui(
            RawInput { time: Some(self.time), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0))), events, ..Default::default() },
            |ui| {
                let r = ui
                    .add_enabled_ui(self.enabled, |ui| {
                        IconButton::new(&self.label, self.size, &self.style).selected(self.selected).animated_hover(self.animated_hover).show(
                            ui,
                            |ui, rect, tint| {
                                self.tint = Some(tint);
                                ui.painter().circle_filled(rect.center(), 3.0, tint);
                            },
                        )
                    })
                    .inner;
                self.clicks += usize::from(r.clicked());
                response = Some(r);
            },
        );
        output.textures_delta.clear();
        (response.unwrap(), output)
    }

    fn settle(&mut self) -> Response {
        self.frame(vec![]);
        self.frame(vec![]).0
    }
}

#[test]
fn opt_in_hover_fades_both_ways_but_press_and_selection_are_immediate() {
    let mut h = ButtonTest::new();
    h.animated_hover = true;
    h.ctx.global_style_mut(|s| s.animation_time = 0.3);
    let inside = h.settle().rect.center();
    h.frame(vec![Event::PointerMoved(inside)]);
    assert_ne!(h.tint, Some(Color32::GREEN), "hover must not jump directly to its target");
    for _ in 0..24 {
        h.frame(vec![]);
    }
    assert_eq!(h.tint, Some(Color32::GREEN));
    h.frame(vec![pointer(inside, true)]);
    assert_eq!(h.tint, Some(Color32::RED));
    h.selected = true;
    h.frame(vec![]);
    assert_eq!(h.tint, Some(Color32::YELLOW));
    h.selected = false;
    h.frame(vec![pointer(inside, false), Event::PointerMoved(pos2(300.0, 150.0))]);
    assert_ne!(h.tint, Some(Color32::WHITE), "leaving should fade instead of jumping");
    for _ in 0..24 {
        h.frame(vec![]);
    }
    assert_eq!(h.tint, Some(Color32::WHITE));
}

#[test]
fn motion_off_and_disabling_snap_even_in_the_middle_of_a_hover() {
    let mut h = ButtonTest::new();
    h.animated_hover = true;
    h.ctx.global_style_mut(|s| s.animation_time = 0.3);
    let inside = h.settle().rect.center();
    h.frame(vec![Event::PointerMoved(inside)]);
    h.enabled = false;
    h.frame(vec![]);
    assert_eq!(h.tint, Some(Color32::WHITE));
    h.enabled = true;
    // egui's hover hit testing uses the previous pass: register the re-enabled target first.
    h.frame(vec![]);
    h.ctx.global_style_mut(|s| s.animation_time = 0.0);
    let (response, _) = h.frame(vec![Event::PointerMoved(inside)]);
    assert!(response.hovered());
    assert_eq!(h.tint, Some(Color32::GREEN));
    h.frame(vec![Event::PointerMoved(pos2(300.0, 150.0))]);
    assert_eq!(h.tint, Some(Color32::WHITE));
}

fn pointer(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn key(key: Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }
}

#[test]
fn click_activates_once_and_selection_stays_caller_owned() {
    let mut h = ButtonTest::new();
    let center = h.settle().rect.center();
    h.frame(vec![Event::PointerMoved(center)]);
    assert_eq!(h.tint, Some(Color32::GREEN));
    h.frame(vec![pointer(center, true)]);
    assert_eq!(h.tint, Some(Color32::RED));
    assert_eq!(h.clicks, 0);
    h.frame(vec![pointer(center, false)]);
    h.frame(vec![]);
    assert_eq!(h.clicks, 1);
    assert!(!h.selected);
}

#[test]
fn both_pressed_styles_cancel_activation_after_dragging_away() {
    for separate_pressed in [false, true] {
        let mut h = ButtonTest::new();
        if !separate_pressed {
            h.style.pressed = None;
        }
        let inside = h.settle().rect.center();
        h.frame(vec![Event::PointerMoved(inside)]);
        h.frame(vec![pointer(inside, true)]);
        assert_eq!(h.tint, Some(if separate_pressed { Color32::RED } else { Color32::GREEN }));
        // egui cancels the click candidate once the pointer exceeds its drag threshold.
        let outside = pos2(300.0, 150.0);
        h.frame(vec![Event::PointerMoved(outside)]);
        assert_eq!(h.tint, Some(Color32::WHITE));
        h.frame(vec![pointer(outside, false)]);
        assert_eq!(h.clicks, 0, "releasing outside must not invoke a command");
    }
}

#[test]
fn selection_takes_precedence_and_is_exposed_to_accessibility() {
    let mut h = ButtonTest::new();
    h.selected = true;
    let center = h.settle().rect.center();
    let (_, output) = h.frame(vec![Event::PointerMoved(center), pointer(center, true)]);
    assert_eq!(h.tint, Some(Color32::YELLOW));
    let tree = output.platform_output.accesskit_update.unwrap();
    let (_, node) = tree.nodes.iter().find(|(_, n)| n.label() == Some("Inspect layers")).unwrap();
    assert_eq!(node.role(), egui::accesskit::Role::Button);
    assert_eq!(node.toggled(), Some(egui::accesskit::Toggled::True));
    assert!(!node.is_disabled());
}

#[test]
fn disabled_buttons_ignore_pointer_and_keyboard_activation() {
    let mut h = ButtonTest::new();
    let response = h.settle();
    h.ctx.memory_mut(|m| m.request_focus(response.id));
    h.enabled = false;
    let center = response.rect.center();
    h.frame(vec![Event::PointerMoved(center), pointer(center, true), key(Key::Enter, true)]);
    let (r, output) = h.frame(vec![pointer(center, false), key(Key::Enter, false)]);
    assert_eq!(h.clicks, 0);
    assert!(!r.enabled());
    let tree = output.platform_output.accesskit_update.unwrap();
    let (_, node) = tree.nodes.iter().find(|(_, n)| n.label() == Some("Inspect layers")).unwrap();
    assert!(node.is_disabled());
}

#[test]
fn enter_and_space_activate_once_per_press() {
    for key_code in [Key::Enter, Key::Space] {
        let mut h = ButtonTest::new();
        let response = h.settle();
        h.ctx.memory_mut(|m| m.request_focus(response.id));
        h.frame(vec![]);
        h.frame(vec![key(key_code, true)]);
        h.frame(vec![key(key_code, false)]);
        h.frame(vec![]);
        assert_eq!(h.clicks, 1, "{key_code:?}");
    }
}

#[test]
fn focus_has_an_inset_visible_outline() {
    let mut h = ButtonTest::new();
    let response = h.settle();
    h.ctx.memory_mut(|m| m.request_focus(response.id));
    let (r, output) = h.frame(vec![]);
    assert!(r.has_focus());
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Rect(shape)
        if shape.rect == r.rect.shrink(2.0) && shape.stroke == h.style.focus_stroke)));
}

#[test]
fn translating_a_label_preserves_id_and_focus() {
    let mut h = ButtonTest::new();
    let first = h.settle();
    h.ctx.memory_mut(|m| m.request_focus(first.id));
    h.label = "Inspecter les calques".into();
    let (second, output) = h.frame(vec![]);
    assert_eq!(first.id, second.id);
    assert!(second.has_focus());
    assert!(output.platform_output.accesskit_update.unwrap().nodes.iter().any(|(_, n)| n.label() == Some("Inspecter les calques")));
}

#[test]
fn duplicate_labels_have_distinct_ids_and_tab_visits_both() {
    let ctx = Context::default();
    let style = style();
    let mut ids = Vec::new();
    let mut frame = |events| {
        ids.clear();
        let mut out = ctx.run_ui(RawInput { events, ..Default::default() }, |ui| {
            for _ in 0..2 {
                let response = IconButton::new("Same label", Vec2::splat(24.0), &style).show(ui, |_, _, _| {});
                ids.push(response.id);
            }
        });
        out.textures_delta.clear();
        ids.clone()
    };
    let ids = frame(vec![]);
    assert_ne!(ids[0], ids[1]);
    ctx.memory_mut(|m| m.request_focus(ids[0]));
    frame(vec![]);
    frame(vec![key(Key::Tab, true)]);
    frame(vec![key(Key::Tab, false)]);
    assert_eq!(ctx.memory(|m| m.focused()), Some(ids[1]));
}

#[test]
fn hostile_sizes_are_inert_and_strokes_stay_finite() {
    for size in [vec2(f32::NAN, 24.0), vec2(24.0, f32::INFINITY), vec2(-1.0, 24.0), Vec2::ZERO, Vec2::splat(f32::MAX)] {
        let mut h = ButtonTest::new();
        h.size = size;
        let (r, _) = h.frame(vec![key(Key::Enter, true)]);
        assert_eq!(r.rect.size(), Vec2::ZERO);
        assert_eq!(h.tint, None);
        assert_eq!(h.clicks, 0);
        assert!(!r.sense.senses_click());
    }
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(20.0));
    for width in [f32::NAN, f32::INFINITY, -1.0] {
        assert_eq!(bounded_stroke(Stroke::new(width, Color32::WHITE), rect), Stroke::NONE);
    }
    assert_eq!(bounded_stroke(Stroke::new(f32::MAX, Color32::WHITE), rect).width, 10.0);
}

#[test]
fn single_shape_frame_preserves_original_rectangle_tessellation() {
    #[derive(Debug, PartialEq)]
    struct DrawMesh {
        clip: Rect,
        indices: Vec<u32>,
        vertices: Vec<(Pos2, Pos2, Color32)>,
    }
    fn meshes(mode: Option<bool>, enabled: bool, scale: f32) -> Vec<DrawMesh> {
        let ctx = Context::default();
        ctx.set_pixels_per_point(scale);
        let mut style = style();
        style.normal.fill = Color32::from_rgb(54, 54, 58);
        style.normal.stroke = Stroke::new(1.0, Color32::from_rgb(22, 22, 24));
        let mut output = FullOutput::default();
        for _ in 0..2 {
            output = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0))), ..Default::default() }, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    let size = vec2(36.0, 26.0);
                    if let Some(single) = mode {
                        let button = IconButton::new("Action", size, &style);
                        let button = if single { button.single_shape_frame() } else { button };
                        button.show(ui, |_, _, _| {});
                    } else {
                        let (rect, _) = ui.allocate_exact_size(size, Sense::click());
                        ui.painter().rect(rect, style.corner_radius, style.normal.fill, style.normal.stroke, StrokeKind::Inside);
                    }
                });
            });
            output.textures_delta.clear();
        }
        ctx.tessellate(output.shapes, output.pixels_per_point)
            .into_iter()
            .map(|clipped| {
                let egui::epaint::Primitive::Mesh(mesh) = clipped.primitive else { panic!("unexpected paint callback") };
                DrawMesh {
                    clip: clipped.clip_rect,
                    indices: mesh.indices,
                    vertices: mesh.vertices.into_iter().map(|v| (v.pos, v.uv, v.color)).collect(),
                }
            })
            .collect()
    }
    for scale in [1.0, 1.5, 2.0] {
        for enabled in [false, true] {
            let original = meshes(None, enabled, scale);
            assert_eq!(meshes(Some(true), enabled, scale), original, "enabled={enabled}, scale={scale}");
            assert_ne!(meshes(Some(false), enabled, scale), original, "separate shapes have a different edge mesh");
        }
    }
}
