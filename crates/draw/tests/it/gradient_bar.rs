//! The gradient stop bar: the stop rules and the default sampler, then the editor by pointer, keys, fields and
//! AccessKit.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_draw::ui::gradient::stops::{StopRules, sample};
use craft_draw::ui::gradient::{ColorStop, GradientEditor, GradientStops, OpacityStop, StopDrop, StopRef, StopRows};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;

const RED: Srgba = Srgba::rgb(1.0, 0.0, 0.0);
const BLUE: Srgba = Srgba::rgb(0.0, 0.0, 1.0);

fn red_to_blue() -> GradientStops {
    GradientStops::new(vec![ColorStop::new(0.0, RED).with_tag(7), ColorStop::new(1.0, BLUE).with_tag(9)])
}

fn close(a: Srgba, b: Srgba) -> bool {
    [a.r - b.r, a.g - b.g, a.b - b.b, a.a - b.a].iter().all(|d| d.abs() < 1e-5)
}

#[trial]
fn the_default_sampler_bends_each_segment_to_its_midpoint_and_splits_alpha_by_layout() {
    let mut stops = red_to_blue();
    stops.color[0].midpoint = 0.25;
    let half = sample(&stops, StopRows::Split, 0.25);
    assert!(close(half, Srgba::rgb(0.5, 0.0, 0.5)), "{half:?}");
    assert!(close(sample(&stops, StopRows::Split, -1.0), RED) && close(sample(&stops, StopRows::Split, 2.0), BLUE));

    // Split: alpha from the opacity stops, the colours' own ignored.
    stops.color[0].color.a = 0.0;
    stops.opacity = vec![OpacityStop::new(0.0, 0.0), OpacityStop::new(1.0, 1.0)];
    assert!((sample(&stops, StopRows::Split, 0.5).a - 0.5).abs() < 1e-5);
    stops.opacity.clear();
    assert_eq!(sample(&stops, StopRows::Split, 0.0).a, 1.0);
    // Combined: the colours' alpha.
    assert_eq!(sample(&stops, StopRows::Combined, 0.0).a, 0.0);
}

/// What the editor reported, pass by pass.
#[derive(Default)]
struct Seen {
    events: Vec<&'static str>,
    requested: Vec<usize>,
    dropped: Vec<StopDrop>,
    selected: Option<StopRef>,
}

struct Rig {
    fixture: Fixture,
    stops: Rc<RefCell<GradientStops>>,
    seen: Rc<RefCell<Seen>>,
}

fn kind<T>(event: &EditEvent<T>) -> &'static str {
    match event {
        EditEvent::Begin { .. } => "begin",
        EditEvent::Update { .. } => "update",
        EditEvent::Commit { .. } => "commit",
        EditEvent::Cancel { .. } => "cancel",
        EditEvent::Set { .. } => "set",
        _ => "other",
    }
}

fn rig(start: GradientStops, rules: StopRules, fields: bool) -> Rig {
    let stops = Rc::new(RefCell::new(start));
    let seen = Rc::new(RefCell::new(Seen::default()));
    let (shared, report) = (stops.clone(), seen.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut stops = shared.borrow_mut();
        let out = GradientEditor::new("Gradient", Bind::bound(&mut stops)).rules(rules.clone()).fields(fields).drop_target::<u32>().show(ui);
        let mut seen = report.borrow_mut();
        seen.events.extend(out.edit.events.iter().map(kind));
        seen.requested.extend(out.color_requested.iter().map(|c| c.index));
        seen.dropped.extend(out.dropped.iter().map(|(place, _)| *place));
        seen.selected = out.selected;
    });
    fixture.settle();
    Rig { fixture, stops, seen }
}

fn three() -> GradientStops {
    GradientStops::new(vec![
        ColorStop::new(0.0, RED).with_tag(1),
        ColorStop::new(0.5, Srgba::BLACK).with_tag(2),
        ColorStop::new(1.0, BLUE).with_tag(3),
    ])
}

impl Rig {
    fn marker(&self, name: &str) -> craft_ui::testing::Target {
        self.fixture.by_label(name).unwrap_or_else(|| panic!("{name} is in the tree"))
    }

    fn tags(&self) -> Vec<u64> {
        self.stops.borrow().color.iter().map(|s| s.tag).collect()
    }
}

#[trial]
fn a_drag_past_a_neighbour_is_one_gesture_and_the_selection_follows_the_stop() {
    let mut rig = rig(three(), StopRules::default(), false);
    let first = rig.marker("Color stop 1 of 3");
    rig.seen.borrow_mut().events.clear();
    rig.fixture.drag(&first, Vec2::new(250.0, 0.0));
    rig.fixture.idle(2);
    assert_eq!(rig.tags(), [2, 1, 3], "the first stop passed the middle one");
    assert_eq!(rig.seen.borrow().selected, Some(StopRef::Color(1)), "the selection followed the dragged stop");
    let events = rig.seen.borrow().events.clone();
    assert_eq!(events.first(), Some(&"begin"));
    assert_eq!(events.iter().filter(|e| **e == "begin").count(), 1, "one gesture");
    assert!(matches!(events.last(), Some(&"commit") | Some(&"set")), "{events:?}");
}
