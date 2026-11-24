//! `GradientFillEditor`: each row reports its edit as one gesture of the whole fill.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_color_ui::ui::gradient::{ColorStop, GradientStops};
use craft_draw::ui::gradient::{
    FocusOutline, GradientFill, GradientFillEditor, GradientGroup, GradientItem, GradientRef, GradientStyle, GradientTile,
};
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

#[trial]
fn a_style_tile_samples_its_parameter_from_the_start_point_outward() {
    let tile = |style, angle| GradientTile::Style { style, angle };
    let t = |tile: GradientTile, x: f32, y: f32| tile.t_at(vec2(x, y));

    // Linear at -90 runs down: 0 at the top, 1 at the bottom, equal along a row.
    let down = tile(GradientStyle::Linear, -90.0);
    assert!(t(down, 0.5, 0.0) < 0.01 && t(down, 0.5, 1.0) > 0.99);
    assert!(t(down, 0.5, 0.25) < t(down, 0.5, 0.75));
    assert!((t(down, 0.1, 0.4) - t(down, 0.9, 0.4)).abs() < 1e-4);
    // At -45 the start is the top-left corner and the end the bottom-right.
    let diagonal = tile(GradientStyle::Linear, -45.0);
    assert!(t(diagonal, 0.0, 0.0) < 0.01 && t(diagonal, 1.0, 1.0) > 0.99);

    // Radial is 0 at the centre and equal on a circle round it.
    let radial = tile(GradientStyle::Radial, 0.0);
    assert!(t(radial, 0.5, 0.5) < 0.01);
    assert!(t(radial, 0.5, 0.5) < t(radial, 0.6, 0.5) && t(radial, 0.6, 0.5) < t(radial, 0.8, 0.5));
    assert!((t(radial, 0.7, 0.5) - t(radial, 0.5, 0.3)).abs() < 1e-4);

    // Reflected is mirrored about the centre.
    let mirrored = tile(GradientStyle::Reflected, 0.0);
    assert!(t(mirrored, 0.5, 0.5) < 0.01);
    assert!((t(mirrored, 0.8, 0.3) - t(mirrored, 0.2, 0.7)).abs() < 1e-4);
    assert!(t(mirrored, 0.6, 0.5) < t(mirrored, 0.9, 0.5));

    // Diamond isolines run through the points of equal |dx| + |dy|, which a circle does not.
    let diamond = tile(GradientStyle::Diamond, 0.0);
    assert!((t(diamond, 0.7, 0.5) - t(diamond, 0.6, 0.6)).abs() < 1e-4);
    assert!(t(diamond, 0.5, 0.5) < 0.01);

    // Angle sweeps counter-clockwise from the start direction, rising round the centre.
    let sweep = tile(GradientStyle::Angle, 0.0);
    assert!(t(sweep, 0.9, 0.5) < 0.01);
    assert!(t(sweep, 0.9, 0.5) < t(sweep, 0.5, 0.1), "up is a quarter turn counter-clockwise");
    assert!(t(sweep, 0.5, 0.1) < t(sweep, 0.1, 0.5));
    assert!(t(sweep, 0.1, 0.5) < t(sweep, 0.5, 0.9));

    // Focus: 0 at the focus centre, rising outward; circles round, rectangles square.
    let focus = egui::Rect::from_min_max(egui::pos2(0.25, 0.25), egui::pos2(0.75, 0.75));
    let circle = GradientTile::Focus { outline: FocusOutline::Circle, focus };
    let rect = GradientTile::Focus { outline: FocusOutline::Rect, focus };
    for tile in [circle, rect] {
        assert!(t(tile, 0.5, 0.5) < 0.01);
        assert!(t(tile, 0.5, 0.5) < t(tile, 0.6, 0.5) && t(tile, 0.6, 0.5) < t(tile, 0.9, 0.5));
        assert!((t(tile, 0.7, 0.5) - t(tile, 0.5, 0.3)).abs() < 1e-4);
    }
    // Both reach 1 at the farthest corner, so a diagonal step reads the same in both; along an axis the rectangle's
    // square isoline is nearer the centre than the circle's, so it rises faster there.
    assert!((t(circle, 0.7, 0.7) - t(rect, 0.7, 0.7)).abs() < 1e-4, "both outlines reach the corner together");
    assert!(t(rect, 0.9, 0.5) > t(circle, 0.9, 0.5) + 0.05, "along an axis a rectangle rises faster than a circle");
    assert!((t(rect, 0.7, 0.7) - t(rect, 0.7, 0.5)).abs() < 1e-4, "a rectangle's isoline is square");
}
