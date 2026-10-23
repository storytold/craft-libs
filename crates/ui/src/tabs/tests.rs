use super::*;
use egui::{Color32, Context, Event, FullOutput, Key, Modifiers, PointerButton, Pos2, RawInput, pos2, vec2};

fn total(fit: &TabFit) -> f32 {
    fit.shown.iter().map(|&(_, width)| width).sum::<f32>() + fit.overflow_width
}

#[test]
fn natural_widths_and_common_cap_preserve_tab_order() {
    let f = fit(&[60.0, 70.0, 80.0], 0, 400.0, 40.0, 18.0);
    assert_eq!(f.shown, [(0, 60.0), (1, 70.0), (2, 80.0)]);
    assert!(f.overflow.is_empty());
    assert_eq!(f.overflow_width, 0.0);
    let f = fit(&[50.0, 70.0, 120.0], 0, 200.0, 40.0, 18.0);
    assert!(f.overflow.is_empty());
    assert_eq!(f.shown[0], (0, 50.0));
    assert_eq!(f.shown[1], (1, 70.0));
    assert!((f.shown[2].1 - 80.0).abs() < 0.01);
    assert!((total(&f) - 200.0).abs() < 0.01);
}

#[test]
fn overflow_retains_selection_and_partitions_all_tabs() {
    let natural = [80.0, 90.0, 85.0, 95.0];
    for selected in [0, 1, 2, 3, usize::MAX] {
        for available in [0.0, 1.0, 5.0, 18.0, 19.0, 55.0, 120.0, 200.0, 500.0] {
            let f = fit(&natural, selected, available, 40.0, 18.0);
            let selected = selected.min(natural.len() - 1);
            assert!(f.shown.iter().any(|&(i, _)| i == selected), "{selected} at {available}: {f:?}");
            assert!(!f.overflow.contains(&selected));
            assert!(f.shown.windows(2).all(|w| w[0].0 < w[1].0));
            assert!(f.overflow.windows(2).all(|w| w[0] < w[1]));
            let mut all: Vec<usize> = f.shown.iter().map(|&(i, _)| i).chain(f.overflow.iter().copied()).collect();
            all.sort_unstable();
            assert_eq!(all, [0, 1, 2, 3]);
            assert!(total(&f) <= available + 0.001, "{available}: {f:?}");
        }
    }
}

#[test]
fn one_tab_never_reserves_an_unused_overflow_button() {
    let f = fit(&[80.0], 99, 10.0, 40.0, 18.0);
    assert_eq!(f.shown, [(0, 10.0)]);
    assert!(f.overflow.is_empty());
    assert_eq!(f.overflow_width, 0.0);
}

#[test]
fn malformed_dimensions_remain_finite_and_within_the_strip() {
    for available in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, 0.01, 10.0, 20.0, 1e20, f32::MAX] {
        for minimum in [f32::NAN, f32::INFINITY, -5.0, 0.0, 40.0, f32::MAX] {
            for overflow in [f32::NAN, f32::INFINITY, -5.0, 0.0, 18.0, f32::MAX] {
                for natural in [&[][..], &[f32::NAN, 50.0][..], &[f32::MAX, -5.0, 30.0, f32::INFINITY][..]] {
                    let f = fit(natural, usize::MAX, available, minimum, overflow);
                    assert!(f.shown.iter().all(|&(_, w)| w.is_finite() && w >= 0.0), "{f:?}");
                    assert!(f.overflow_width.is_finite() && f.overflow_width >= 0.0);
                    assert!(total(&f) <= extent(available) + 0.125, "{available} {minimum} {overflow}: {f:?}");
                    assert_eq!(f.shown.len() + f.overflow.len(), natural.len());
                }
            }
        }
    }
}

#[test]
fn many_tabs_keep_the_last_selected_without_quadratic_membership_scans() {
    let widths = vec![60.0; 10_000];
    let f = fit(&widths, widths.len() - 1, 240.0, 40.0, 18.0);
    assert!(f.shown.iter().any(|&(i, w)| i == widths.len() - 1 && w > 0.0));
    assert_eq!(f.shown.len() + f.overflow.len(), widths.len());
    assert!(total(&f) <= 240.001);
}

struct TabTest {
    ctx: Context,
    labels: [String; 2],
    selected: usize,
    enabled: bool,
    sense: Sense,
    rect: Rect,
    clicks: usize,
    paints: usize,
    time: f64,
}

impl TabTest {
    fn new() -> Self {
        let ctx = Context::default();
        ctx.enable_accesskit();
        ctx.global_style_mut(|s| s.animation_time = 0.0);
        Self {
            ctx,
            labels: ["Layers".into(), "Layers".into()],
            selected: 0,
            enabled: true,
            sense: Sense::click(),
            rect: Rect::from_min_size(pos2(20.0, 20.0), vec2(90.0, 26.0)),
            clicks: 0,
            paints: 0,
            time: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> (Vec<Response>, FullOutput) {
        self.time += 0.05;
        let ctx = self.ctx.clone();
        let mut responses = Vec::new();
        self.paints = 0;
        let raw =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 200.0))), events, time: Some(self.time), ..Default::default() };
        let mut output = ctx.run_ui(raw, |ui| {
            ui.add_enabled_ui(self.enabled, |ui| {
                for (i, label) in self.labels.iter().enumerate() {
                    let r = Tab::new(ui.scope_id().with(("tab", i)), label, self.selected == i)
                        .sense(self.sense)
                        .focus_stroke(Stroke::new(1.0, Color32::LIGHT_BLUE))
                        .show_at(ui, self.rect.translate(vec2(i as f32 * 100.0, 0.0)), |ui, response| {
                            self.paints += 1;
                            ui.painter().rect_filled(response.rect, 0.0, Color32::DARK_GRAY);
                        });
                    if r.clicked() {
                        self.clicks += 1;
                        self.selected = i;
                    }
                    responses.push(r);
                }
            });
        });
        output.textures_delta.clear();
        (responses, output)
    }

    fn settle(&mut self) -> Vec<Response> {
        self.frame(vec![]);
        self.frame(vec![]).0
    }
}

fn pointer(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn key(key: Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }
}

#[test]
fn duplicate_labels_have_distinct_stable_identity_and_tab_roles() {
    let mut h = TabTest::new();
    let responses = h.settle();
    assert_ne!(responses[0].id, responses[1].id);
    h.ctx.memory_mut(|m| m.request_focus(responses[1].id));
    h.labels[1] = "Calques".into();
    let (updated, output) = h.frame(vec![]);
    assert_eq!(responses[1].id, updated[1].id);
    assert!(updated[1].has_focus());
    let tree = output.platform_output.accesskit_update.unwrap();
    let (_, selected) = tree.nodes.iter().find(|(_, n)| n.label() == Some("Layers")).unwrap();
    assert_eq!(selected.role(), egui::accesskit::Role::Tab);
    assert_eq!(selected.is_selected(), Some(true));
    assert_eq!(selected.toggled(), None);
    let (_, other) = tree.nodes.iter().find(|(_, n)| n.label() == Some("Calques")).unwrap();
    assert_eq!(other.is_selected(), Some(false));
}

#[test]
fn pointer_and_keyboard_activate_once_and_disabled_tabs_do_not_activate() {
    for key_code in [Key::Enter, Key::Space] {
        let mut h = TabTest::new();
        let responses = h.settle();
        h.ctx.memory_mut(|m| m.request_focus(responses[1].id));
        h.frame(vec![]);
        h.frame(vec![key(key_code, true)]);
        h.frame(vec![key(key_code, false)]);
        assert_eq!((h.clicks, h.selected), (1, 1));
    }
    let mut h = TabTest::new();
    let center = h.settle()[1].rect.center();
    h.frame(vec![Event::PointerMoved(center), pointer(center, true)]);
    h.frame(vec![pointer(center, false)]);
    assert_eq!((h.clicks, h.selected), (1, 1));
    h.enabled = false;
    h.frame(vec![pointer(center, true), key(Key::Enter, true)]);
    let (responses, output) = h.frame(vec![pointer(center, false), key(Key::Enter, false)]);
    assert_eq!(h.clicks, 1);
    assert!(!responses[1].enabled());
    assert!(
        output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .filter(|(_, n)| n.role() == egui::accesskit::Role::Tab)
            .all(|(_, n)| n.is_disabled())
    );
}

#[test]
fn focus_is_visible_and_drag_responses_are_preserved_without_selecting() {
    let mut h = TabTest::new();
    h.sense = Sense::click_and_drag();
    let responses = h.settle();
    h.ctx.memory_mut(|m| m.request_focus(responses[1].id));
    let (_, output) = h.frame(vec![]);
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Rect(r)
        if r.rect == responses[1].rect.shrink(2.0) && r.stroke.color == Color32::LIGHT_BLUE)));
    let center = responses[1].rect.center();
    h.frame(vec![Event::PointerMoved(center), pointer(center, true)]);
    let to = center + vec2(40.0, 60.0);
    let (dragging, _) = h.frame(vec![Event::PointerMoved(to)]);
    assert!(dragging[1].drag_started() && dragging[1].dragged());
    let (stopped, _) = h.frame(vec![pointer(to, false)]);
    assert!(stopped[1].drag_stopped());
    assert_eq!((h.clicks, h.selected), (0, 0));
}

#[test]
fn invalid_rectangles_do_not_paint_or_accept_input() {
    for rect in [
        Rect::NOTHING,
        Rect::EVERYTHING,
        Rect::from_min_size(Pos2::ZERO, vec2(f32::NAN, 24.0)),
        Rect::from_min_size(Pos2::ZERO, vec2(0.0, 24.0)),
        Rect::from_min_size(Pos2::ZERO, vec2(1e20, 24.0)),
        Rect::from_min_max(pos2(50.0, 50.0), pos2(20.0, 20.0)),
    ] {
        let mut h = TabTest::new();
        h.rect = rect;
        let responses = h.settle();
        assert_eq!(h.paints, 0, "{rect:?}");
        assert!(responses.iter().all(|r| r.rect.is_finite() && !r.sense.senses_click()));
    }
}
