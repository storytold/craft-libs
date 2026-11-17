use egui::{Event, Key, Modifiers, PointerButton, Pos2, Sense, Vec2, pos2, vec2};
use egui_kittest::Harness;

use super::{MAX_DRAG_STEPS, click, drag, key, move_to, press, release, type_text, wheel};

/// The input events and held modifiers of every frame the app ran.
#[derive(Default)]
struct Frames {
    events: Vec<Vec<Event>>,
    modifiers: Vec<Modifiers>,
}

impl Frames {
    fn count(&self) -> usize {
        self.events.len()
    }

    fn pointer_buttons(&self) -> Vec<(Pos2, PointerButton, bool, Modifiers)> {
        let events = self.events.iter().flatten();
        events
            .filter_map(|event| match event {
                Event::PointerButton { pos, button, pressed, modifiers } => Some((*pos, *button, *pressed, *modifiers)),
                _ => None,
            })
            .collect()
    }

    fn moves(&self) -> Vec<Pos2> {
        self.events.iter().flatten().filter_map(|event| if let Event::PointerMoved(pos) = event { Some(*pos) } else { None }).collect()
    }
}

/// A harness that records its frames, with the frame `build` runs already cleared.
fn recorder() -> Harness<'static, Frames> {
    let mut harness = Harness::builder().with_size([200.0, 100.0]).build_ui_state(
        |ui, frames: &mut Frames| {
            let (events, modifiers) = ui.input(|i| (i.raw.events.clone(), i.modifiers));
            frames.events.push(events);
            frames.modifiers.push(modifiers);
        },
        Frames::default(),
    );
    *harness.state_mut() = Frames::default();
    harness
}

type Step = dyn Fn(&mut Harness<'static, Frames>);

#[test]
fn each_step_runs_its_documented_frames() {
    const AT: Pos2 = Pos2::new(40.0, 30.0);
    let cases: [(&str, usize, &Step); 9] = [
        ("move_to", 1, &|h| move_to(h, AT)),
        ("press", 2, &|h| press(h, AT, PointerButton::Primary, Modifiers::NONE)),
        ("release", 2, &|h| release(h, AT, PointerButton::Primary, Modifiers::NONE)),
        ("click", 3, &|h| click(h, AT, PointerButton::Primary, Modifiers::NONE)),
        ("drag 4", 7, &|h| drag(h, AT, pos2(80.0, 30.0), 4, Modifiers::NONE)),
        ("drag 0", 4, &|h| drag(h, AT, pos2(80.0, 30.0), 0, Modifiers::NONE)),
        ("key", 2, &|h| key(h, Key::A, Modifiers::NONE)),
        ("type_text", 2, &|h| type_text(h, "abc")),
        ("wheel", 2, &|h| wheel(h, vec2(0.0, -10.0), Modifiers::NONE)),
    ];
    for (name, frames, step) in cases {
        let mut harness = recorder();
        step(&mut harness);
        assert_eq!(harness.state().count(), frames, "{name}");
    }
}

#[test]
fn queued_events_add_frames_to_the_first_frame() {
    let mut harness = recorder();
    harness.event(Event::Text("a".to_owned()));
    harness.event(Event::Text("b".to_owned()));
    click(&mut harness, pos2(40.0, 30.0), PointerButton::Primary, Modifiers::NONE);
    let frames = harness.state();
    assert_eq!(frames.count(), 4);
    assert!(frames.events[0].contains(&Event::Text("a".to_owned())));
    assert!(frames.events[0].contains(&Event::PointerMoved(pos2(40.0, 30.0))));
    assert_eq!(frames.events[1], vec![Event::Text("b".to_owned())]);
}

#[test]
fn click_moves_presses_and_releases_in_order() {
    let at = pos2(40.0, 30.0);
    let mut harness = recorder();
    click(&mut harness, at, PointerButton::Primary, Modifiers::CTRL);
    let frames = harness.state();
    assert_eq!(
        frames.events[0],
        vec![
            Event::ModifiersChanged(Modifiers::CTRL),
            Event::PointerMoved(at),
            Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::CTRL },
        ]
    );
    assert_eq!(frames.events[1], vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::CTRL }]);
    assert_eq!(frames.events[2], vec![Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(frames.modifiers, [Modifiers::CTRL, Modifiers::CTRL, Modifiers::NONE]);
}

#[test]
fn click_reaches_a_button() {
    let mut harness = Harness::builder().with_size([200.0, 100.0]).build_ui_state(
        |ui, clicks: &mut (u32, u32)| {
            let response = ui.add_sized([100.0, 40.0], egui::Button::new("Open"));
            clicks.0 += u32::from(response.clicked());
            clicks.1 += u32::from(response.secondary_clicked());
        },
        (0, 0),
    );
    click(&mut harness, pos2(50.0, 20.0), PointerButton::Primary, Modifiers::NONE);
    click(&mut harness, pos2(50.0, 20.0), PointerButton::Secondary, Modifiers::NONE);
    press(&mut harness, pos2(50.0, 20.0), PointerButton::Secondary, Modifiers::NONE);
    release(&mut harness, pos2(50.0, 20.0), PointerButton::Secondary, Modifiers::NONE);
    assert_eq!(*harness.state(), (1, 2));
}

#[test]
fn press_keeps_buttons_and_modifiers_until_release() {
    let (from, to) = (pos2(10.0, 10.0), pos2(90.0, 50.0));
    let mut harness = recorder();
    press(&mut harness, from, PointerButton::Secondary, Modifiers::ALT);
    move_to(&mut harness, to);
    release(&mut harness, to, PointerButton::Secondary, Modifiers::ALT);
    let frames = harness.state();
    assert_eq!(
        frames.pointer_buttons(),
        [(from, PointerButton::Secondary, true, Modifiers::ALT), (to, PointerButton::Secondary, false, Modifiers::ALT)]
    );
    assert_eq!(frames.moves(), [from, to, to]);
    assert_eq!(frames.modifiers, [Modifiers::ALT, Modifiers::ALT, Modifiers::ALT, Modifiers::ALT, Modifiers::NONE]);
}

#[test]
fn drag_moves_evenly_and_ends_at_the_target() {
    let (from, to) = (pos2(10.0, 20.0), pos2(90.0, 60.0));
    let mut harness = recorder();
    drag(&mut harness, from, to, 4, Modifiers::SHIFT);
    let frames = harness.state();
    assert_eq!(frames.moves(), [from, pos2(30.0, 30.0), pos2(50.0, 40.0), pos2(70.0, 50.0), to]);
    assert_eq!(
        frames.pointer_buttons(),
        [(from, PointerButton::Primary, true, Modifiers::SHIFT), (to, PointerButton::Primary, false, Modifiers::SHIFT)]
    );
    // Held through the moves and the release, released in the last frame.
    assert_eq!(frames.modifiers[..6], [Modifiers::SHIFT; 6]);
    assert_eq!(frames.modifiers[6], Modifiers::NONE);
}

#[test]
fn drag_with_zero_steps_moves_once() {
    let (from, to) = (pos2(10.0, 20.0), pos2(90.0, 60.0));
    let mut harness = recorder();
    drag(&mut harness, from, to, 0, Modifiers::NONE);
    assert_eq!(harness.state().moves(), [from, to]);
}

#[test]
fn drag_steps_are_capped() {
    let mut harness = recorder();
    drag(&mut harness, pos2(0.0, 0.0), pos2(100.0, 0.0), usize::MAX, Modifiers::NONE);
    assert_eq!(harness.state().count(), MAX_DRAG_STEPS + 3);
    assert_eq!(harness.state().moves().last(), Some(&pos2(100.0, 0.0)));
}

#[test]
fn drag_moves_a_draggable_area() {
    let mut harness = Harness::builder().with_size([200.0, 100.0]).build_ui_state(
        |ui, moved: &mut Vec2| {
            let response = ui.allocate_response(vec2(200.0, 100.0), Sense::drag());
            *moved += response.drag_delta();
        },
        Vec2::ZERO,
    );
    drag(&mut harness, pos2(20.0, 20.0), pos2(120.0, 60.0), 5, Modifiers::NONE);
    let moved = *harness.state();
    assert!((moved - vec2(100.0, 40.0)).length() < 0.01, "{moved:?}");
}

#[test]
fn key_sends_down_and_up_with_modifiers() {
    let mut harness = recorder();
    key(&mut harness, Key::S, Modifiers::COMMAND);
    let frames = harness.state();
    let event = |pressed| Event::Key { key: Key::S, physical_key: None, pressed, repeat: false, modifiers: Modifiers::COMMAND };
    assert_eq!(frames.events[0], vec![Event::ModifiersChanged(Modifiers::COMMAND), event(true), event(false)]);
    assert_eq!(frames.modifiers, [Modifiers::COMMAND, Modifiers::NONE]);
}

#[test]
fn type_text_reaches_the_focused_field() {
    let mut harness = Harness::builder().with_size([200.0, 100.0]).build_ui_state(
        |ui, text: &mut String| {
            let label = ui.label("Name");
            let response = ui.text_edit_singleline(text).labelled_by(label.id);
            if ui.memory(|m| m.focused().is_none()) {
                response.request_focus();
            }
        },
        String::new(),
    );
    harness.run();
    type_text(&mut harness, "Craft");
    type_text(&mut harness, "");
    assert_eq!(harness.state(), "Craft");
}

#[test]
fn empty_text_sends_no_event() {
    let mut harness = recorder();
    type_text(&mut harness, "");
    assert!(harness.state().events.iter().all(Vec::is_empty));
}

#[test]
fn wheel_sends_points_with_its_modifiers() {
    let mut harness = recorder();
    wheel(&mut harness, vec2(0.0, -24.0), Modifiers::COMMAND);
    let frames = harness.state();
    assert_eq!(
        frames.events[0],
        vec![
            Event::ModifiersChanged(Modifiers::COMMAND),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -24.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: Modifiers::COMMAND,
            },
        ]
    );
    assert_eq!(frames.modifiers, [Modifiers::COMMAND, Modifiers::NONE]);
}
