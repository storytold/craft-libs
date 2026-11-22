//! The gradient stop bar: the stop rules and the default sampler, then the editor by pointer, keys, fields and
//! AccessKit.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_color_ui::ui::gradient::stops::{AltDrop, StopRules, sample};
use craft_color_ui::ui::gradient::{ColorStop, GradientEditor, GradientStops, OpacityStop, StopDrop, StopRef, StopRows};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, Event, Key, Modifiers, Rect, Shape, TextureId, Vec2, vec2};

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
    requested: Vec<(usize, Option<Rect>)>,
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
    rig_below(start, rules, fields, 0.0)
}

/// A rig whose editor sits `above` points down a room tall enough for a drag far above and far below the bar.
fn rig_below(start: GradientStops, rules: StopRules, fields: bool, above: f32) -> Rig {
    let stops = Rc::new(RefCell::new(start));
    let seen = Rc::new(RefCell::new(Seen::default()));
    let (shared, report) = (stops.clone(), seen.clone());
    let mut fixture = Fixture::new(move |ui| {
        ui.add_space(above);
        let mut stops = shared.borrow_mut();
        let out = GradientEditor::new("Gradient", Bind::bound(&mut stops)).rules(rules.clone()).fields(fields).drop_target::<u32>().show(ui);
        let mut seen = report.borrow_mut();
        seen.events.extend(out.edit.events.iter().map(kind));
        seen.requested.extend(out.color_requested.iter().map(|c| (c.index, c.rect)));
        seen.dropped.extend(out.dropped.iter().map(|(place, _)| *place));
        seen.selected = out.selected;
    });
    if above > 0.0 {
        fixture.room(vec2(400.0, 2.0 * above + 300.0));
    }
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

/// The host opens its picker where the request came from: under the stop's marker for Enter on it, at the Color well
/// for a click on the well.
#[trial]
fn a_colour_request_carries_the_rect_of_what_asked() {
    let mut rig = rig(three(), StopRules::default(), true);
    let last = rig.marker("Color stop 3 of 3");
    rig.fixture.click(&last).key(Key::Enter, Modifiers::NONE).idle(1);
    let requested = std::mem::take(&mut rig.seen.borrow_mut().requested);
    let Some(&(2, Some(rect))) = requested.last() else { panic!("Enter asks for the last stop's colour at a place: {requested:?}") };
    assert!(rect.x_range().contains(last.center().x), "under the last stop's marker: {rect:?}, marker {:?}", last.rect);
    let well = rig.marker("Color");
    rig.fixture.click(&well).idle(1);
    let requested = std::mem::take(&mut rig.seen.borrow_mut().requested);
    let Some(&(2, Some(rect))) = requested.last() else { panic!("the well asks for the selected stop's colour at a place: {requested:?}") };
    assert!(rect.intersects(well.rect), "at the well: {rect:?}, well {:?}", well.rect);
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

#[trial]
fn an_alt_drag_released_on_another_stop_swaps_their_colours_and_keeps_their_places() {
    // Alt-drags the first stop of `three()` (the first one half transparent) to `share` of the way to the last.
    let alt_drag = |rules: StopRules, share: f32| {
        let mut start = three();
        start.color[0].color.a = 0.5;
        let mut rig = rig(start, rules, false);
        let (from, last) = (rig.marker("Color stop 1 of 3").center(), rig.marker("Color stop 3 of 3").center());
        rig.fixture.hold(Modifiers::ALT);
        rig.seen.borrow_mut().events.clear();
        rig.fixture.drag_at(from, vec2((last.x - from.x) * share, 0.0));
        rig.fixture.hold(Modifiers::NONE);
        rig.fixture.idle(2);
        rig
    };
    let swap = StopRules::new(StopRows::Combined).alt_drop(AltDrop::Swap);

    let rig = alt_drag(swap.clone(), 1.0);
    let stops = rig.stops.borrow().clone();
    assert_eq!(rig.tags(), [3, 2, 1], "the tags went with the colours");
    assert_eq!(stops.color[0].color, Srgba { a: 0.5, ..BLUE }, "the first stop took the blue and kept its alpha");
    assert_eq!(stops.color[2].color, RED, "the last stop took the red and kept its alpha");
    assert_eq!(stops.color.iter().map(|s| s.location).collect::<Vec<_>>(), [0.0, 0.5, 1.0], "the places stay");
    assert_eq!(rig.seen.borrow().selected, Some(StopRef::Color(0)), "the selection stays on the dragged stop");
    let events = rig.seen.borrow().events.clone();
    assert_eq!(events.iter().filter(|e| **e == "begin").count(), 1, "one step: {events:?}");

    // Between stops it copies, and so does the default rule onto a stop.
    assert_eq!(alt_drag(swap, 0.25).tags(), [1, 1, 2, 3], "released between stops it copies");
    assert_eq!(alt_drag(StopRules::new(StopRows::Combined), 1.0).tags(), [1, 2, 3, 1], "under Copy it copies onto the stop");
}

/// Three colour stops (tags 1..=3) and three opacity stops (tags 11..=13).
fn three_and_three() -> GradientStops {
    GradientStops::with_opacity(
        three().color,
        vec![OpacityStop::new(0.0, 1.0).with_tag(11), OpacityStop::new(0.5, 0.5).with_tag(12), OpacityStop::new(1.0, 1.0).with_tag(13)],
    )
}

impl Rig {
    fn opacity_tags(&self) -> Vec<u64> {
        self.stops.borrow().opacity.iter().map(|s| s.tag).collect()
    }
}

#[trial]
fn a_stop_dragged_off_the_bar_comes_off_unless_it_returns_or_is_one_of_the_last_two() {
    // The pointer goes this far past the bar's edge: well beyond the tear-off distance, which is a share of the
    // control height and so of the bar's own height.
    const ABOVE: f32 = 500.0;
    let far = |rig: &Rig| rig.marker("Gradient").rect.height() * 4.0;

    // A colour stop dragged down and released there comes off, in one gesture; the opacity row is untouched.
    let mut rig = rig_below(three_and_three(), StopRules::default(), false, ABOVE);
    let from = rig.marker("Color stop 2 of 3").center();
    let by = vec2(0.0, far(&rig));
    rig.seen.borrow_mut().events.clear();
    rig.fixture.drag_at(from, by);
    rig.fixture.idle(2);
    assert_eq!(rig.tags(), [1, 3], "the dragged colour stop came off");
    assert_eq!(rig.opacity_tags(), [11, 12, 13], "the opacity row kept its stops");
    let events = rig.seen.borrow().events.clone();
    assert_eq!(events.iter().filter(|e| **e == "begin").count(), 1, "one gesture: {events:?}");

    // An opacity stop dragged up off the bar comes off the same way.
    let mut rig = rig_below(three_and_three(), StopRules::default(), false, ABOVE);
    let from = rig.marker("Opacity stop 2 of 3").center();
    let by = vec2(0.0, -far(&rig));
    rig.fixture.drag_at(from, by);
    rig.fixture.idle(2);
    assert_eq!(rig.opacity_tags(), [11, 13], "the dragged opacity stop came off");
    assert_eq!(rig.tags(), [1, 2, 3], "the colour row kept its stops");

    // The release frame decides: a stop taken off the bar and brought back before the release stays.
    let mut rig = rig_below(three_and_three(), StopRules::default(), false, ABOVE);
    let from = rig.marker("Color stop 2 of 3").center();
    let by = vec2(0.0, far(&rig));
    rig.fixture.drag_hold_at(from, by);
    rig.fixture.frame(vec![Event::PointerMoved(from)]);
    rig.fixture.release_at(from);
    rig.fixture.idle(2);
    assert_eq!(rig.tags(), [1, 2, 3], "the stop returned to the bar before the release and stays");

    // At the row's minimum a stop dragged off the bar stays, in both rows.
    let mut rig = rig_below(red_to_blue(), StopRules::default(), false, ABOVE);
    let before = rig.stops.borrow().clone();
    let (down, up) = (rig.marker("Color stop 1 of 2").center(), rig.marker("Opacity stop 1 of 2").center());
    let by = vec2(0.0, far(&rig));
    rig.fixture.drag_at(down, by);
    rig.fixture.drag_at(up, -by);
    rig.fixture.idle(2);
    assert_eq!(*rig.stops.borrow(), before, "the last two stops of a row cannot be torn off");
}

/// Four colour stops, the middle two at the same location: tags 1 at 0, 2 and 3 at 0.5, 4 at 1.
fn tied_in_the_middle() -> GradientStops {
    GradientStops::new(vec![
        ColorStop::new(0.0, RED).with_tag(1),
        ColorStop::new(0.5, Srgba::BLACK).with_tag(2),
        ColorStop::new(0.5, Srgba::BLACK).with_tag(3),
        ColorStop::new(1.0, BLUE).with_tag(4),
    ])
}

#[trial]
fn a_stop_keeps_its_place_among_stops_at_its_location_unless_it_crossed_them() {
    // Drags colour stop `name` to the horizontal position of the tied pair (`share` of the way from the first stop
    // to the last, read from the markers) and returns the tags in order and the selection.
    let drag = |name: &str, share: f32| {
        let mut rig = rig(tied_in_the_middle(), StopRules::default(), false);
        let (first, last) = (rig.marker("Color stop 1 of 4").center().x, rig.marker("Color stop 4 of 4").center().x);
        let from = rig.marker(name).center();
        let by = vec2(first + (last - first) * share - from.x, 0.0);
        rig.fixture.drag_at(from, by);
        rig.fixture.idle(2);
        (rig.tags(), rig.seen.borrow().selected)
    };

    // Moving up onto the tied location, the stop stays before them; moving down onto it, after them.
    assert_eq!(drag("Color stop 1 of 4", 0.5), (vec![1, 2, 3, 4], Some(StopRef::Color(0))), "reached the tie from below");
    assert_eq!(drag("Color stop 4 of 4", 0.5), (vec![1, 2, 3, 4], Some(StopRef::Color(3))), "reached the tie from above");
    // Past the tie it crosses both stops and the selection goes with it.
    assert_eq!(drag("Color stop 1 of 4", 0.75), (vec![2, 3, 1, 4], Some(StopRef::Color(2))), "crossed the tie upward");
    assert_eq!(drag("Color stop 4 of 4", 0.25), (vec![1, 4, 2, 3], Some(StopRef::Color(1))), "crossed the tie downward");
}

#[trial]
fn a_split_bar_holds_its_midpoint_diamonds_inside_its_rect() {
    let rig = rig(red_to_blue(), StopRules::default(), false);
    let bar = rig.marker("Gradient").rect;
    for name in ["Midpoint 1", "Opacity midpoint 1"] {
        let diamond = rig.marker(name).rect;
        assert!(bar.contains_rect(diamond), "{name} {diamond:?} lies inside the bar {bar:?}");
    }
}

/// The vertex colours of a red-to-blue editor's ramp, split into its red and its blue ones: the untextured meshes'
/// vertices of a pure red or blue hue (the markers' tips are meshes too, in theme colours).
fn ramp_ends(enabled: bool) -> (Vec<Color32>, Vec<Color32>) {
    let mut fixture = Fixture::new(move |ui| {
        let mut stops = red_to_blue();
        let _ = GradientEditor::new("Gradient", Bind::bound(&mut stops)).fields(false).enabled(enabled).show(ui);
    });
    fixture.settle();
    let colors: Vec<Color32> = fixture
        .frame_shapes()
        .iter()
        .filter_map(|shape| match shape {
            Shape::Mesh(mesh) if mesh.texture_id == TextureId::default() => Some(mesh.vertices.iter().map(|vertex| vertex.color)),
            _ => None,
        })
        .flatten()
        .collect();
    let hue = |keep: fn(Color32) -> bool| colors.iter().copied().filter(|&c| c.a() > 0 && keep(c)).collect::<Vec<_>>();
    (hue(|c| c.r() > 0 && c.g() == 0 && c.b() == 0), hue(|c| c.b() > 0 && c.r() == 0 && c.g() == 0))
}

/// A disabled editor paints its ramp, data painted with its content painter, dimmer than an enabled one, and the
/// ramp's two ends keep their hues.
#[trial]
fn a_disabled_editor_dims_its_ramp_and_keeps_its_hues() {
    let strongest = |colors: &[Color32]| colors.iter().map(|c| c.a()).max();
    let (red, blue) = ramp_ends(true);
    let (dim_red, dim_blue) = ramp_ends(false);
    assert!(strongest(&red).is_some() && strongest(&blue).is_some(), "the enabled ramp paints its red and blue ends");
    assert!(strongest(&dim_red) < strongest(&red), "the disabled ramp's red {dim_red:?} is dimmer than the enabled {red:?}");
    assert!(strongest(&dim_blue) < strongest(&blue), "the disabled ramp's blue {dim_blue:?} is dimmer than the enabled {blue:?}");
    assert!(!dim_red.is_empty() && !dim_blue.is_empty(), "the disabled ramp keeps a red and a blue end");
}
