//! The curve editor: pointer and key editing of the points, the fields, the pencil, the corners and the AccessKit
//! nodes, driven through the editor as a host would show it.

use craft_behavior::EditEvent;
use craft_imaging::ui::ChannelColor;
use craft_imaging::ui::curve::{Curve, CurveChannel, CurveEditor, CurveEnd};
use craft_theme::{Contrast, Mode};
use craft_ui::contract::Bind;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, Room, style, trial};
use egui::{Pos2, Vec2, pos2, vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Rig {
    fixture: Fixture,
    curve: Rc<RefCell<Curve>>,
    selected: Rc<Cell<Option<usize>>>,
    /// The end the last pass reported for the clipping display.
    clipping: Rc<Cell<Option<CurveEnd>>>,
    /// The kind of each edit event, in order.
    kinds: Rc<RefCell<Vec<&'static str>>>,
    origin: Pos2,
    scale: Vec2,
}

fn kind(event: &EditEvent<Curve>) -> &'static str {
    match event {
        EditEvent::Begin { .. } => "begin",
        EditEvent::Update { .. } => "update",
        EditEvent::Commit { .. } => "commit",
        EditEvent::Cancel { .. } => "cancel",
        EditEvent::Set { .. } => "set",
        _ => "other",
    }
}

fn rig(points: &[[f32; 2]]) -> Rig {
    rig_with(points, Setup::default())
}

/// How a rig shows its editor.
#[derive(Clone, Copy)]
struct Setup {
    room: Vec2,
    input_sliders: bool,
    plot_height: Option<f32>,
    pencil: bool,
    show_clipping: bool,
}

impl Default for Setup {
    fn default() -> Self {
        Setup { room: room(), input_sliders: false, plot_height: None, pencil: false, show_clipping: false }
    }
}

/// The setup of the Curves adjustment: input sliders, in a room of `room`.
fn sliders_in(room: Vec2) -> Setup {
    Setup { room, input_sliders: true, plot_height: None, pencil: false, show_clipping: false }
}

fn rig_with(points: &[[f32; 2]], setup: Setup) -> Rig {
    let curve = Curve::smooth(points.to_vec());
    let shared = Rc::new(RefCell::new(curve.clone()));
    let selected = Rc::new(Cell::new(None));
    let clipping = Rc::new(Cell::new(None));
    let kinds = Rc::new(RefCell::new(Vec::new()));
    let (c, s, k, clip) = (shared.clone(), selected.clone(), kinds.clone(), clipping.clone());
    let channels = [CurveChannel::new("RGB", ChannelColor::Neutral), CurveChannel::new("Red", ChannelColor::Red)];
    let mut fixture = Fixture::new(move |ui| {
        let mut value = c.borrow().clone();
        let mut editor = CurveEditor::new("Curve", Bind::bound(&mut value)).channels(&channels).channel(0).selection(s.get());
        if setup.input_sliders {
            editor = editor.input_sliders();
        }
        if let Some(height) = setup.plot_height {
            editor = editor.plot_height(height);
        }
        if setup.pencil {
            editor = editor.pencil();
        }
        let out = editor.show_clipping(setup.show_clipping).show(ui);
        for event in &out.edit.events {
            k.borrow_mut().push(kind(event));
        }
        s.set(out.selected);
        clip.set(out.clipping);
        *c.borrow_mut() = value;
    });
    fixture.room(setup.room);
    fixture.settle();
    let mut rig = Rig { fixture, curve: shared, selected, clipping, kinds, origin: Pos2::ZERO, scale: Vec2::ZERO };
    if !setup.pencil {
        rig.calibrate(&curve);
    }
    rig
}

impl Rig {
    /// Maps levels to screen from the first and last point's nodes.
    fn calibrate(&mut self, curve: &Curve) {
        let points = curve.points().map(<[_]>::to_vec).unwrap_or_default();
        let (Some(first), Some(last)) = (points.first(), points.last()) else { return };
        let n = points.len();
        let a = self.fixture.by_label(&format!("Point 1 of {n}")).expect("the first point").center();
        let b = self.fixture.by_label(&format!("Point {n} of {n}")).expect("the last point").center();
        self.scale = vec2((b.x - a.x) / (last[0] - first[0]), (b.y - a.y) / (last[1] - first[1]));
        self.origin = pos2(a.x - first[0] * self.scale.x, a.y - first[1] * self.scale.y);
    }

    fn at(&self, q: [f32; 2]) -> Pos2 {
        pos2(self.origin.x + q[0] * self.scale.x, self.origin.y + q[1] * self.scale.y)
    }

    fn points(&self) -> Vec<[f32; 2]> {
        self.curve.borrow().points().map(<[_]>::to_vec).unwrap_or_default()
    }

    fn kinds(&self) -> Vec<&'static str> {
        self.kinds.borrow().clone()
    }

    fn press_at(&mut self, q: [f32; 2]) {
        let pos = self.at(q);
        self.fixture.frame(vec![moved(pos)]);
        self.fixture.frame(vec![press(pos)]);
    }

    fn move_to(&mut self, q: [f32; 2]) {
        self.fixture.frame(vec![moved(self.at(q))]);
    }

    fn release_at(&mut self, q: [f32; 2]) {
        self.fixture.frame(vec![release(self.at(q))]);
        self.fixture.idle(1);
    }
}

/// The room is this many plot widths wide and tall: the editor at its greatest size and the rows under it.
const WIDE_PLOTS: u8 = 5;
const TALL_PLOTS: u8 = 6;

fn room() -> Vec2 {
    let unit = style::tokens(Mode::Light, Contrast::Standard).size.plot.width;
    Vec2::new(unit * f32::from(WIDE_PLOTS), unit * f32::from(TALL_PLOTS))
}

const IDENTITY: [[f32; 2]; 2] = [[0.0, 0.0], [255.0, 255.0]];
const THREE: [[f32; 2]; 3] = [[0.0, 0.0], [100.0, 100.0], [255.0, 255.0]];

/// A press on the empty plot adds a point there, and the drag that follows is one gesture.
#[trial]
fn a_press_on_the_empty_graph_adds_a_point_and_its_drag_is_one_gesture() {
    let mut rig = rig(&IDENTITY);
    rig.press_at([128.0, 128.0]);
    rig.move_to([128.0, 180.0]);
    rig.move_to([130.0, 200.0]);
    rig.release_at([130.0, 200.0]);
    assert_eq!(rig.points(), vec![[0.0, 0.0], [130.0, 200.0], [255.0, 255.0]]);
    let kinds = rig.kinds();
    assert_eq!(kinds.iter().filter(|k| **k == "begin").count(), 1, "{kinds:?}");
    assert_eq!(kinds.last(), Some(&"commit"), "{kinds:?}");
    assert_eq!(rig.selected.get(), Some(1));
}

/// An interior point dragged far off the plot goes, and comes back on re-entry within the same drag.
#[trial]
fn a_point_dragged_off_the_graph_goes_and_comes_back_within_one_gesture() {
    let mut rig = rig(&THREE);
    rig.press_at([100.0, 100.0]);
    rig.move_to([100.0, 130.0]);
    let off = Pos2::new(rig.at([100.0, 100.0]).x, rig.origin.y + 600.0);
    rig.fixture.frame(vec![moved(off)]);
    assert_eq!(rig.points().len(), 2, "removed while off the graph");
    rig.move_to([140.0, 140.0]);
    rig.release_at([140.0, 140.0]);
    assert_eq!(rig.points(), vec![[0.0, 0.0], [140.0, 140.0], [255.0, 255.0]]);
    assert_eq!(rig.kinds().iter().filter(|k| **k == "begin").count(), 1);
}

/// Shown in a panel, the editor's rect holds the whole editor: the Input and Output fields and the hint lie under the
/// graph and inside the area the editor took, not clipped below it.
#[trial]
fn the_fields_and_the_hint_show_under_the_graph_inside_the_editor() {
    const HINT: &str = "Click to add a point";
    let curve = Rc::new(RefCell::new(Curve::smooth(THREE.to_vec())));
    let shared = curve.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut value = shared.borrow().clone();
        let _ = CurveEditor::new("Curve", Bind::bound(&mut value)).hint(HINT).show(ui);
    });
    fixture.in_room(Room::Panel);
    fixture.settle();
    let shown = fixture.shown();
    let corner = fixture.by_label("Point 1 of 3").expect("the first point").center();
    let input = fixture.by_label("Input").expect("the Input field").rect;
    let hint = fixture.texts().iter().find(|text| text.text == HINT).expect("the hint is painted").rect;
    assert!(input.top() > corner.y, "the fields sit under the graph: {input:?}, point at {corner:?}");
    assert!(hint.top() >= input.bottom(), "the hint sits under the fields: {hint:?}, {input:?}");
    for (part, rect) in [("Input", input), ("hint", hint)] {
        assert!(shown.contains_rect(rect), "{part} {rect:?} lies inside the editor's area {shown:?}");
    }
}

const BLACK_SLIDER: &str = "Input levels, Black input";
const WHITE_SLIDER: &str = "Input levels, White input";
const LIFTED: [[f32; 2]; 3] = [[0.0, 30.0], [128.0, 128.0], [255.0, 220.0]];

impl Rig {
    /// The centre of the input slider named `name`.
    fn slider(&self, name: &str) -> Pos2 {
        self.fixture.by_label(name).unwrap_or_else(|| panic!("the {name} slider shows")).center()
    }

    /// Drags the input slider `name` to input `level`, in steps, and releases it there.
    fn drag_slider(&mut self, name: &str, level: f32) {
        let from = self.slider(name);
        let to = pos2(self.at([level, 0.0]).x, from.y);
        self.fixture.frame(vec![moved(from)]);
        self.fixture.frame(vec![press(from)]);
        for step in 1..=4 {
            self.fixture.frame(vec![moved(from + (to - from) * step as f32 / 4.0)]);
        }
        self.fixture.frame(vec![release(to)]);
        self.fixture.idle(1);
    }
}

/// The black slider moves the black end point's input and keeps its output, as one gesture that selects the point;
/// the white slider does the same for the white end point.
#[trial]
fn the_input_sliders_move_the_end_inputs_and_keep_their_outputs() {
    let mut rig = rig_with(&LIFTED, sliders_in(room()));
    rig.drag_slider(BLACK_SLIDER, 40.0);
    assert_eq!(rig.points(), vec![[40.0, 30.0], [128.0, 128.0], [255.0, 220.0]]);
    assert_eq!(rig.selected.get(), Some(0), "the black end point is selected, so the Input field follows it");
    let kinds = rig.kinds();
    assert_eq!(kinds.iter().filter(|k| **k == "begin").count(), 1, "{kinds:?}");
    assert!(kinds.contains(&"update"), "the preview follows the drag: {kinds:?}");
    assert_eq!(kinds.last(), Some(&"commit"), "{kinds:?}");
    rig.drag_slider(WHITE_SLIDER, 200.0);
    assert_eq!(rig.points(), vec![[40.0, 30.0], [128.0, 128.0], [200.0, 220.0]]);
    assert_eq!(rig.selected.get(), Some(2));
}

/// An end point keeps 4 levels from its neighbour's input: an interior point, or the other end point of a straight curve.
#[trial]
fn an_end_point_keeps_four_levels_from_its_neighbour() {
    let mut rig = rig_with(&LIFTED, sliders_in(room()));
    rig.drag_slider(BLACK_SLIDER, 200.0);
    assert_eq!(rig.points()[0], [124.0, 30.0]);
    rig.drag_slider(WHITE_SLIDER, 20.0);
    assert_eq!(rig.points()[2], [132.0, 220.0]);
    let mut rig = rig_with(&IDENTITY, sliders_in(room()));
    rig.drag_slider(WHITE_SLIDER, 0.0);
    assert_eq!(rig.points(), vec![[0.0, 0.0], [4.0, 255.0]]);
    let mut rig = rig_with(&IDENTITY, sliders_in(room()));
    rig.drag_slider(BLACK_SLIDER, 255.0);
    assert_eq!(rig.points(), vec![[251.0, 0.0], [255.0, 255.0]]);
}

/// Pencil mode has no end points to move: no sliders.
#[trial]
fn pencil_mode_shows_no_input_sliders() {
    let rig = rig_with(&IDENTITY, Setup { pencil: true, ..sliders_in(room()) });
    assert!(rig.fixture.by_label(BLACK_SLIDER).is_none());
    assert!(rig.fixture.by_label(WHITE_SLIDER).is_none());
}

/// Alt on the drag of an end point, by its slider or by the point, reports that end for the clipping display; the
/// report ends with the release and with Alt. An interior point and a drag without Alt report nothing.
#[trial]
fn alt_on_an_end_reports_it_until_the_release_or_alt_ends() {
    let mut rig = rig_with(&LIFTED, sliders_in(room()));
    let black = rig.slider(BLACK_SLIDER);
    rig.fixture.hold(egui::Modifiers::ALT);
    rig.fixture.frame(vec![moved(black)]);
    rig.fixture.frame(vec![press(black)]);
    rig.fixture.frame(vec![moved(black + vec2(6.0, 0.0))]);
    assert_eq!(rig.clipping.get(), Some(CurveEnd::Black), "the black slider");
    rig.fixture.hold(egui::Modifiers::NONE);
    rig.fixture.frame(vec![moved(black + vec2(8.0, 0.0))]);
    assert_eq!(rig.clipping.get(), None, "Alt released while dragging");
    rig.fixture.frame(vec![release(black + vec2(8.0, 0.0))]);
    rig.fixture.idle(1);

    for (which, end) in [(2, Some(CurveEnd::White)), (0, Some(CurveEnd::Black)), (1, None)] {
        let q = rig.points()[which];
        let q = [q[0].max(3.0), q[1].max(3.0)];
        rig.fixture.hold(egui::Modifiers::ALT);
        rig.press_at(q);
        rig.move_to([q[0] - 3.0, q[1] - 3.0]);
        assert_eq!(rig.clipping.get(), end, "Alt-drag of the point at {q:?}");
        rig.release_at([q[0] - 3.0, q[1] - 3.0]);
        assert_eq!(rig.clipping.get(), None, "released at {q:?}");
        rig.fixture.hold(egui::Modifiers::NONE);
    }

    let white = rig.slider(WHITE_SLIDER);
    rig.fixture.frame(vec![moved(white)]);
    rig.fixture.frame(vec![press(white)]);
    rig.fixture.frame(vec![moved(white - vec2(6.0, 0.0))]);
    assert_eq!(rig.clipping.get(), None, "no Alt, no display");
    rig.fixture.frame(vec![release(white - vec2(6.0, 0.0))]);
}

/// Alt held on the press of an input slider reports its end from the press, before the pointer moves, and the report
/// ends with the release: an Alt-click shows the clipping display while the button is down, as Photoshop does.
#[trial]
fn alt_press_on_a_slider_reports_its_end_before_the_pointer_moves() {
    let mut rig = rig_with(&LIFTED, sliders_in(room()));
    for (name, end) in [(BLACK_SLIDER, CurveEnd::Black), (WHITE_SLIDER, CurveEnd::White)] {
        let at = rig.slider(name);
        rig.fixture.hold(egui::Modifiers::ALT);
        rig.fixture.frame(vec![moved(at)]);
        rig.fixture.frame(vec![press(at)]);
        assert_eq!(rig.clipping.get(), Some(end), "the press of the {name}");
        rig.fixture.frame(vec![]);
        assert_eq!(rig.clipping.get(), Some(end), "the {name} held still");
        rig.fixture.frame(vec![release(at)]);
        rig.fixture.idle(1);
        assert_eq!(rig.clipping.get(), None, "the {name} released");
        rig.fixture.hold(egui::Modifiers::NONE);
        rig.fixture.frame(vec![press(at)]);
        assert_eq!(rig.clipping.get(), None, "the {name} pressed without Alt");
        rig.fixture.frame(vec![release(at)]);
        rig.fixture.idle(1);
    }
}

/// With Show Clipping on, the press of an end, by its slider or its point, reports that end without Alt; an interior
/// point reports nothing.
#[trial]
fn show_clipping_reports_an_end_without_alt() {
    let mut rig = rig_with(&LIFTED, Setup { show_clipping: true, ..sliders_in(room()) });
    for (name, end) in [(BLACK_SLIDER, CurveEnd::Black), (WHITE_SLIDER, CurveEnd::White)] {
        let at = rig.slider(name);
        rig.fixture.frame(vec![moved(at)]);
        rig.fixture.frame(vec![press(at)]);
        assert_eq!(rig.clipping.get(), Some(end), "the press of the {name}");
        rig.fixture.frame(vec![release(at)]);
        rig.fixture.idle(1);
        assert_eq!(rig.clipping.get(), None, "the {name} released");
    }
    for (which, end) in [(0, Some(CurveEnd::Black)), (2, Some(CurveEnd::White)), (1, None)] {
        let q = rig.points()[which];
        let q = [q[0].max(3.0), q[1].max(3.0)];
        rig.press_at(q);
        assert_eq!(rig.clipping.get(), end, "the press of the point at {q:?}");
        rig.release_at(q);
    }
}

/// A mouse wheel turning `delta` points (a horizontal turn where `delta.x` is set), with `mods` held.
fn turn(delta: Vec2, mods: egui::Modifiers) -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        phase: egui::TouchPhase::Move,
        modifiers: mods,
        source: egui::MouseWheelSource::Wheel,
    }
}

/// Clicks the input slider at `at` without moving it: the sliders take the wheel once focused, as every value control
/// does, so a wheel over them does not steal the panel's scroll.
fn focus_sliders(rig: &mut Rig, at: Pos2) {
    rig.fixture.frame(vec![moved(at)]);
    rig.fixture.frame(vec![press(at)]);
    rig.fixture.frame(vec![release(at)]);
    rig.fixture.idle(1);
}

/// The wheel over the focused input sliders moves the end nearest the pointer by whole levels, whichever axis it turns
/// and with Alt held or not: on these sliders Alt is the clipping display, not the fine scale.
#[trial]
fn the_wheel_moves_the_end_nearest_the_pointer() {
    let mut rig = rig_with(&LIFTED, sliders_in(room()));
    let (black, white) = (rig.slider(BLACK_SLIDER), rig.slider(WHITE_SLIDER));
    let input = |rig: &Rig, i: usize| rig.points()[i][0];
    let last = rig.points().len() - 1;
    focus_sliders(&mut rig, white);
    rig.fixture.frame(vec![moved(white - vec2(6.0, 0.0))]);
    rig.fixture.frame(vec![turn(vec2(0.0, -10.0), egui::Modifiers::NONE)]);
    assert_eq!((input(&rig, 0), input(&rig, last)), (0.0, 254.0), "the white end, nearest the pointer, steps down");
    rig.fixture.hold(egui::Modifiers::ALT);
    rig.fixture.frame(vec![turn(vec2(0.0, -10.0), egui::Modifiers::ALT)]);
    assert_eq!(input(&rig, last), 253.0, "Alt held: a whole level");
    rig.fixture.frame(vec![turn(vec2(-10.0, 0.0), egui::Modifiers::ALT)]);
    assert_eq!(input(&rig, last), 252.0, "a horizontal turn, as Alt with a wheel may arrive");
    rig.fixture.hold(egui::Modifiers::NONE);
    rig.fixture.frame(vec![moved(black + vec2(6.0, 0.0))]);
    rig.fixture.frame(vec![turn(vec2(0.0, 10.0), egui::Modifiers::NONE)]);
    assert_eq!((input(&rig, 0), input(&rig, last)), (1.0, 252.0), "the black end, nearest the pointer now, steps up");
}

/// Alt held on the wheel over an input slider reports the end it moves while the wheel turns and for a moment after
/// its last turn; letting Alt go ends the report at once. Without Alt nothing reports.
#[trial]
fn alt_on_the_wheel_reports_the_end_it_moves() {
    let mut rig = rig_with(&LIFTED, sliders_in(room()));
    let white = rig.slider(WHITE_SLIDER);
    focus_sliders(&mut rig, white);
    rig.fixture.frame(vec![moved(white - vec2(6.0, 0.0))]);
    rig.fixture.frame(vec![turn(vec2(0.0, -10.0), egui::Modifiers::NONE)]);
    assert_eq!(rig.clipping.get(), None, "no Alt, no report");
    rig.fixture.hold(egui::Modifiers::ALT);
    rig.fixture.frame(vec![turn(vec2(0.0, -10.0), egui::Modifiers::ALT)]);
    assert_eq!(rig.clipping.get(), Some(CurveEnd::White), "the turn");
    rig.fixture.advance(0.2);
    rig.fixture.frame(vec![]);
    assert_eq!(rig.clipping.get(), Some(CurveEnd::White), "between turns");
    rig.fixture.advance(0.3);
    rig.fixture.frame(vec![]);
    assert_eq!(rig.clipping.get(), None, "the wheel stopped");
    rig.fixture.frame(vec![turn(vec2(0.0, -10.0), egui::Modifiers::ALT)]);
    assert_eq!(rig.clipping.get(), Some(CurveEnd::White), "turned again");
    rig.fixture.hold(egui::Modifiers::NONE);
    rig.fixture.frame(vec![]);
    assert_eq!(rig.clipping.get(), None, "Alt let go");
}

/// With Show Clipping on, the wheel over an input slider reports the end it moves without Alt, until a moment after
/// its last turn.
#[trial]
fn show_clipping_reports_the_end_the_wheel_moves() {
    let mut rig = rig_with(&LIFTED, Setup { show_clipping: true, ..sliders_in(room()) });
    let black = rig.slider(BLACK_SLIDER);
    focus_sliders(&mut rig, black);
    rig.fixture.advance(0.5);
    rig.fixture.idle(1);
    assert_eq!(rig.clipping.get(), None, "the click ended");
    rig.fixture.frame(vec![moved(black + vec2(6.0, 0.0))]);
    rig.fixture.frame(vec![turn(vec2(0.0, 10.0), egui::Modifiers::NONE)]);
    assert_eq!(rig.clipping.get(), Some(CurveEnd::Black), "the turn");
    rig.fixture.advance(0.5);
    rig.fixture.frame(vec![]);
    assert_eq!(rig.clipping.get(), None, "the wheel stopped");
}

/// A graph wider or taller than square (its plot height set, its width the room's) maps levels on each axis on its
/// own: a press adds a point at the levels under it, a press within the pick radius of a point takes that point, and
/// the sliders sit under the end points and map along the plot's width.
#[trial]
fn a_graph_that_is_not_square_maps_each_axis_on_its_own() {
    let unit = style::tokens(Mode::Light, Contrast::Standard).size.plot.width;
    let reach = style::tokens(Mode::Light, Contrast::Standard).size.hit.target / 2.0;
    for (name, room, plot) in [("wide", Vec2::new(unit * 4.0, unit * 3.0), unit * 0.9), ("tall", Vec2::new(unit * 1.2, unit * 5.0), unit * 3.0)] {
        let mut rig = rig_with(&THREE, Setup { plot_height: Some(plot), ..sliders_in(room) });
        let (w, h) = (rig.scale.x * 255.0, -rig.scale.y * 255.0);
        let ratio = w / h;
        match name {
            "wide" => assert!(ratio > 1.5, "{name}: the plot is wider than tall, {w}×{h}"),
            _ => assert!(ratio < 0.67, "{name}: the plot is taller than wide, {w}×{h}"),
        }
        for (end, q) in [(BLACK_SLIDER, THREE[0]), (WHITE_SLIDER, THREE[2])] {
            let x = rig.slider(end).x;
            assert!((x - rig.at(q).x).abs() < 0.5, "{name}: the {end} slider at x {x} sits under its point at {:?}", rig.at(q));
        }
        // Inside the reach of the middle point, on the diagonal: the press takes that point and moves it.
        let near = rig.at(THREE[1]) + vec2(reach * 0.6, -reach * 0.6);
        rig.fixture.frame(vec![moved(near)]);
        rig.fixture.frame(vec![press(near)]);
        rig.fixture.frame(vec![moved(near + vec2(0.0, -20.0))]);
        rig.fixture.frame(vec![release(near + vec2(0.0, -20.0))]);
        rig.fixture.idle(1);
        assert_eq!(rig.points().len(), 3, "{name}: the press took the middle point, no new one");
        assert_eq!(rig.selected.get(), Some(1), "{name}");
        // Far from every point: a new point at exactly the levels under the press.
        rig.press_at([200.0, 60.0]);
        rig.release_at([200.0, 60.0]);
        assert!(rig.points().contains(&[200.0, 60.0]), "{name}: {:?}", rig.points());
        rig.drag_slider(BLACK_SLIDER, 30.0);
        assert_eq!(rig.points()[0][0], 30.0, "{name}: the black slider maps along the plot's width");
    }
}

/// A Curves panel sized to its room: the graph takes the height the room leaves under the panel's other rows and the
/// host's reserve, so a taller room makes a taller graph; a room too short for the least plot keeps the rows under the
/// graph inside the panel and the text after it below them (the room scrolls instead).
#[trial]
fn a_panel_sized_to_its_room_follows_its_height_and_never_overlaps() {
    use craft_imaging::ui::adjust::{CurvesPanel, ToneChannels, ToneCurves, ToneSpace};
    const AFTER: &str = "Below the panel";
    let channels = ToneChannels::of(ToneSpace::Rgb, false);
    let mut heights = Vec::new();
    for room in [120.0, 700.0, 900.0] {
        let curves = Rc::new(RefCell::new(ToneCurves::identity(&channels)));
        let (shared, list) = (curves.clone(), channels.clone());
        let mut fixture = Fixture::new(move |ui| {
            let mut value = shared.borrow().clone();
            let _ = CurvesPanel::new("Curves", Bind::bound(&mut value), list.clone()).fill_height(40.0).show(ui);
            craft_ui::contract::Show::show(craft_ui::display::Text::new(AFTER), ui);
        });
        fixture.room(Vec2::new(360.0, room));
        fixture.settle();
        let top = fixture.by_label("Point 2 of 2").expect("the white end point").center().y;
        let bottom = fixture.by_label("Point 1 of 2").expect("the black end point").center().y;
        let input = fixture.by_label("Input").expect("the Input field").rect;
        let after = fixture.texts().iter().find(|text| text.text == AFTER).expect("the text after the panel").rect;
        assert!(input.top() > bottom, "{room}: the fields sit under the graph");
        assert!(after.top() >= input.bottom(), "{room}: what follows sits under the panel: {after:?}, fields {input:?}");
        if room > 500.0 {
            assert!(
                (after.bottom() - (room - 40.0)).abs() < 41.0,
                "{room}: the panel fills the room less the reserve, the text ends at {}",
                after.bottom()
            );
        }
        heights.push(bottom - top);
    }
    assert!(heights[1] > heights[0] && heights[2] > heights[1] + 150.0, "the graph grows with the room: {heights:?}");
}
