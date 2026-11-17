//! `GradientFillEditor`: each row reports its edit as one gesture of the whole fill.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_color_ui::ui::gradient::{ColorStop, GradientStops};
use craft_draw::ui::gradient::{GradientFill, GradientFillEditor, GradientGroup, GradientItem, GradientRef, GradientStyle};
use craft_theme::Srgba;
use craft_ui::contract::{Bind, Role};
use craft_ui::patterns::presets::PresetState;
use craft_ui::testing::{Fixture, trial};
use egui::vec2;

fn pair(from: f32, to: f32) -> GradientStops {
    GradientStops::new(vec![ColorStop::new(0.0, Srgba::rgb(from, from, from)), ColorStop::new(1.0, Srgba::rgb(to, to, to))])
}

fn preset() -> GradientStops {
    pair(0.9, 0.1)
}

struct Shown {
    fixture: Fixture,
    held: Rc<RefCell<GradientFill>>,
    log: Rc<RefCell<Vec<EditEvent<GradientFill>>>>,
}

impl Shown {
    /// The events reported since the last call.
    fn take(&mut self) -> Vec<EditEvent<GradientFill>> {
        self.fixture.settle();
        std::mem::take(&mut *self.log.borrow_mut())
    }
}

fn shown() -> Shown {
    let mut fill = GradientFill::new(GradientStops::new(vec![
        ColorStop::new(0.0, Srgba::rgb(0.0, 0.0, 0.0)).with_midpoint(0.3),
        ColorStop::new(1.0, Srgba::rgb(1.0, 1.0, 1.0)),
    ]));
    fill.dither = true;
    let held = Rc::new(RefCell::new(fill));
    let log = Rc::new(RefCell::new(Vec::new()));
    let state = Rc::new(RefCell::new(PresetState::<GradientRef>::new()));
    let (value, events, palette) = (Rc::clone(&held), Rc::clone(&log), state);
    let mut fixture = Fixture::new(move |ui| {
        let groups = [GradientGroup::new("Basics", vec![GradientItem::new("Red to blue", preset())])];
        let given = value.borrow().clone();
        let mut palette = palette.borrow_mut();
        let out = GradientFillEditor::new("Fill", Bind::given(given), &groups, Bind::bound(&mut *palette)).show(ui);
        for event in out.events.iter() {
            event.apply(&mut *value.borrow_mut());
            events.borrow_mut().push(event.clone());
        }
    });
    fixture.settle();
    Shown { fixture, held, log }
}

/// Asserts the events are one gesture (one `Commit` or `Set`, at most one `Begin`) in which every carried value
/// equals `start` once `neutral` copies the edited part back; returns the last value.
fn one_gesture(events: &[EditEvent<GradientFill>], start: &GradientFill, neutral: impl Fn(&mut GradientFill, &GradientFill)) -> GradientFill {
    let ended = events.iter().filter(|e| matches!(e, EditEvent::Commit { .. } | EditEvent::Set { .. })).count();
    let begun = events.iter().filter(|e| matches!(e, EditEvent::Begin { .. })).count();
    assert_eq!((ended, begun <= 1), (1, true), "one gesture, got {events:?}");
    for value in events.iter().filter_map(EditEvent::value) {
        let mut rest = value.clone();
        neutral(&mut rest, start);
        assert_eq!(&rest, start, "only the edited part differs");
    }
    let last = events.iter().rev().find_map(EditEvent::value).expect("a value").clone();
    assert_ne!(&last, start, "the edit changed the fill");
    last
}

#[trial]
fn each_row_reports_one_gesture_of_the_whole_fill() {
    let mut s = shown();

    let start = s.held.borrow().clone();
    let stop = s.fixture.by_label("Color stop 1 of 2").expect("the bar shows its stops");
    s.fixture.drag(&stop, vec2(30.0, 0.0));
    let events = s.take();
    one_gesture(&events, &start, |fill, from| fill.stops = from.stops.clone());

    let start = s.held.borrow().clone();
    let mut dials = s.fixture.by_role(Role::SLIDER);
    assert_eq!(dials.len(), 1, "the angle dial is the only slider");
    let dial = dials.remove(0);
    s.fixture.drag(&dial, vec2(30.0, 0.0));
    let events = s.take();
    one_gesture(&events, &start, |fill, from| fill.angle = from.angle);

    let start = s.held.borrow().clone();
    let radial = s.fixture.by_label("Radial").expect("the style entries");
    s.fixture.click(&radial);
    let events = s.take();
    let after = one_gesture(&events, &start, |fill, from| fill.style = from.style);
    assert_eq!(after.style, GradientStyle::Radial);

    let start = s.held.borrow().clone();
    let item = s.fixture.by_label("Red to blue").expect("the preset");
    s.fixture.click(&item);
    let events = s.take();
    assert!(!events.is_empty(), "the pick reports an edit");
    let mut expected = start.clone();
    expected.stops = preset();
    for value in events.iter().filter_map(EditEvent::value) {
        assert_eq!(value, &expected, "the preset replaces the stops with centred midpoints and keeps the rest");
    }
}
