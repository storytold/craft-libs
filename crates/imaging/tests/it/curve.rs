//! The curve editor: pointer and key editing of the points, the fields, the pencil, the corners and the AccessKit
//! nodes, driven through the editor as a host would show it.

use craft_behavior::EditEvent;
use craft_imaging::ui::ChannelColor;
use craft_imaging::ui::curve::{Curve, CurveChannel, CurveEditor};
use craft_theme::{Contrast, Mode};
use craft_ui::contract::Bind;
use craft_ui::testing::pointer::{moved, press, release, secondary};
use craft_ui::testing::{Fixture, Room, style, trial};
use egui::{Pos2, Vec2, pos2, vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(crate) struct Rig {
    pub(crate) fixture: Fixture,
    curve: Rc<RefCell<Curve>>,
    pub(crate) selected: Rc<Cell<Option<usize>>>,
    /// The kind of each edit event, in order.
    kinds: Rc<RefCell<Vec<&'static str>>>,
    /// The key presses left in egui's input after the editor, which the app reads, in order.
    pub(crate) app_keys: Rc<RefCell<Vec<egui::Key>>>,
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
    rig_with(points, false)
}

/// [`rig`] with `tab_steps` and `end_delete_deselects` when `point_keys`.
pub(crate) fn rig_with(points: &[[f32; 2]], point_keys: bool) -> Rig {
    let curve = Curve::smooth(points.to_vec());
    let shared = Rc::new(RefCell::new(curve.clone()));
    let selected = Rc::new(Cell::new(None));
    let kinds = Rc::new(RefCell::new(Vec::new()));
    let app_keys = Rc::new(RefCell::new(Vec::new()));
    let (c, s, k, a) = (shared.clone(), selected.clone(), kinds.clone(), app_keys.clone());
    let channels = [CurveChannel::new("RGB", ChannelColor::Neutral), CurveChannel::new("Red", ChannelColor::Red)];
    let mut fixture = Fixture::new(move |ui| {
        let mut value = c.borrow().clone();
        let mut editor = CurveEditor::new("Curve", Bind::bound(&mut value)).channels(&channels).channel(0).selection(s.get());
        if point_keys {
            editor = editor.tab_steps().end_delete_deselects();
        }
        let out = editor.show(ui);
        for event in &out.edit.events {
            k.borrow_mut().push(kind(event));
        }
        s.set(out.selected);
        *c.borrow_mut() = value;
        let pressed = ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| if let egui::Event::Key { key, pressed: true, .. } = event { Some(*key) } else { None })
                .collect::<Vec<_>>()
        });
        a.borrow_mut().extend(pressed);
    });
    fixture.room(room());
    fixture.settle();
    let mut rig = Rig { fixture, curve: shared, selected, kinds, app_keys, origin: Pos2::ZERO, scale: Vec2::ZERO };
    rig.calibrate(&curve);
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

    pub(crate) fn points(&self) -> Vec<[f32; 2]> {
        self.curve.borrow().points().map(<[_]>::to_vec).unwrap_or_default()
    }

    fn kinds(&self) -> Vec<&'static str> {
        self.kinds.borrow().clone()
    }

    pub(crate) fn press_at(&mut self, q: [f32; 2]) {
        let pos = self.at(q);
        self.fixture.frame(vec![moved(pos)]);
        self.fixture.frame(vec![press(pos)]);
    }

    /// Presses `px` screen pixels from the position of levels `q`.
    fn press_near(&mut self, q: [f32; 2], px: Vec2) {
        let pos = self.at(q) + px;
        self.fixture.frame(vec![moved(pos)]);
        self.fixture.frame(vec![press(pos)]);
    }

    fn release_near(&mut self, q: [f32; 2], px: Vec2) {
        self.fixture.frame(vec![release(self.at(q) + px)]);
        self.fixture.idle(1);
    }

    fn move_to(&mut self, q: [f32; 2]) {
        self.fixture.frame(vec![moved(self.at(q))]);
    }

    pub(crate) fn release_at(&mut self, q: [f32; 2]) {
        self.fixture.frame(vec![release(self.at(q))]);
        self.fixture.idle(1);
    }

    fn right_click_at(&mut self, q: [f32; 2]) {
        let pos = self.at(q);
        self.fixture.frame(vec![moved(pos)]);
        self.fixture.frame(vec![secondary(pos, true)]);
        self.fixture.frame(vec![secondary(pos, false)]);
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

    // Past the grab radius below the plot it is removed too.
    let mut rig = self::rig(&[[0.0, 0.0], [100.0, 0.0], [255.0, 255.0]]);
    rig.press_at([100.0, 0.0]);
    rig.move_to([100.0, 0.0]);
    rig.fixture.frame(vec![moved(rig.at([100.0, 0.0]) + vec2(0.0, 40.0))]);
    assert_eq!(rig.points().len(), 2, "removed past the radius");
}

/// A press up to 15 px from a point (straight or diagonal) grabs it; a press 30 px away adds a new one.
#[trial]
fn a_press_near_a_point_grabs_it_and_a_press_far_from_it_adds_one() {
    for offset in [vec2(15.0, 0.0), vec2(0.0, -15.0), vec2(10.0, 10.0)] {
        let mut rig = rig(&THREE);
        rig.press_near([100.0, 100.0], offset);
        rig.release_near([100.0, 100.0], offset);
        assert_eq!(rig.points().len(), 3, "grabbed at {offset:?}");
        assert_eq!(rig.selected.get(), Some(1), "grabbed at {offset:?}");
    }
    let mut rig = rig(&THREE);
    rig.press_near([100.0, 100.0], vec2(30.0, 0.0));
    rig.release_near([100.0, 100.0], vec2(30.0, 0.0));
    assert_eq!(rig.points().len(), 4, "a press 30 px away adds a point");
}

/// A right-click on the empty graph drops the selection and adds no point; a right-click on an inner point removes it.
#[trial]
fn a_right_click_on_the_empty_graph_deselects_and_on_a_point_removes_it() {
    let mut rig = rig(&THREE);
    rig.press_at([100.0, 100.0]);
    rig.release_at([100.0, 100.0]);
    assert_eq!(rig.selected.get(), Some(1));
    rig.right_click_at([180.0, 20.0]);
    assert_eq!(rig.selected.get(), None, "a right-click on the empty graph deselects");
    assert_eq!(rig.points(), THREE.to_vec(), "and adds no point");
    rig.right_click_at([100.0, 100.0]);
    assert_eq!(rig.points(), vec![[0.0, 0.0], [255.0, 255.0]], "a right-click on a point removes it");
}

/// Where two points are closer than the grab radius, a press nearer the second grabs the second.
#[trial]
fn where_grab_areas_overlap_the_nearest_point_is_grabbed() {
    let per_px = 1.0 / rig(&THREE).scale.x;
    let points = [[0.0, 0.0], [100.0, 100.0], [100.0 + 8.0 * per_px, 100.0], [255.0, 255.0]];
    let mut rig = rig(&points);
    rig.press_near(points[2], vec2(3.0, 0.0));
    rig.release_near(points[2], vec2(3.0, 0.0));
    assert_eq!(rig.points().len(), 4);
    assert_eq!(rig.selected.get(), Some(2));
}

/// A stationary click that grabs a point outside the plot keeps the point.
#[trial]
fn a_click_that_grabs_a_point_outside_the_plot_keeps_it() {
    let points = [[0.0, 0.0], [100.0, 0.0], [255.0, 255.0]];
    let mut rig = rig(&points);
    let below = vec2(0.0, 15.0);
    rig.press_near([100.0, 0.0], below);
    rig.release_near([100.0, 0.0], below);
    assert_eq!(rig.points().len(), 3);
    assert_eq!(rig.selected.get(), Some(1));
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
