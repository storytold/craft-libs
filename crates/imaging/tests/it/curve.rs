//! The curve editor: pointer and key editing of the points, the fields, the pencil, the corners and the AccessKit
//! nodes, driven through the editor as a host would show it.

use craft_behavior::EditEvent;
use craft_imaging::ui::ChannelColor;
use craft_imaging::ui::curve::{Curve, CurveChannel, CurveEditor};
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
    rig_with(points, false)
}

/// [`rig`] with `tab_steps` and `end_delete_deselects` when `point_keys`.
fn rig_with(points: &[[f32; 2]], point_keys: bool) -> Rig {
    let curve = Curve::smooth(points.to_vec());
    let shared = Rc::new(RefCell::new(curve.clone()));
    let selected = Rc::new(Cell::new(None));
    let kinds = Rc::new(RefCell::new(Vec::new()));
    let (c, s, k) = (shared.clone(), selected.clone(), kinds.clone());
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
    });
    fixture.room(room());
    fixture.settle();
    let mut rig = Rig { fixture, curve: shared, selected, kinds, origin: Pos2::ZERO, scale: Vec2::ZERO };
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

const FOUR: [[f32; 2]; 4] = [[0.0, 0.0], [64.0, 64.0], [192.0, 192.0], [255.0, 255.0]];

/// Selects point `at` of `points` with a click on it, which also focuses the graph.
fn click_point(rig: &mut Rig, q: [f32; 2]) {
    rig.press_at(q);
    rig.release_at(q);
}

/// Ctrl+Tab steps right through the points and wraps; Ctrl+Shift+Tab steps left. The curve stays as it is.
#[trial]
fn tab_steps_selects_the_next_and_previous_point_round_the_curve() {
    let mut rig = rig_with(&FOUR, true);
    click_point(&mut rig, [64.0, 64.0]);
    assert_eq!(rig.selected.get(), Some(1));
    let mut seen = Vec::new();
    for _ in 0..3 {
        rig.fixture.key(egui::Key::Tab, egui::Modifiers::CTRL).idle(1);
        seen.push(rig.selected.get());
    }
    assert_eq!(seen, [Some(2), Some(3), Some(0)], "right, then round to the first");
    seen.clear();
    for _ in 0..3 {
        rig.fixture.key(egui::Key::Tab, egui::Modifiers::CTRL | egui::Modifiers::SHIFT).idle(1);
        seen.push(rig.selected.get());
    }
    assert_eq!(seen, [Some(3), Some(2), Some(1)], "round to the last, then left");
    assert_eq!(rig.points(), FOUR.to_vec(), "no point moved");
}

/// Without the option Ctrl+Tab leaves the selection.
#[trial]
fn without_tab_steps_ctrl_tab_leaves_the_selection() {
    let mut rig = rig(&FOUR);
    click_point(&mut rig, [64.0, 64.0]);
    rig.fixture.key(egui::Key::Tab, egui::Modifiers::CTRL).idle(1);
    assert_eq!(rig.selected.get(), Some(1));
}

/// Delete on an end point keeps the point; with the option it deselects it, without it the point stays selected.
/// Delete on an inner point removes it either way.
#[trial]
fn end_delete_deselects_an_end_point_and_inner_points_still_go() {
    for (point_keys, after) in [(false, Some(0)), (true, None)] {
        let mut rig = rig_with(&FOUR, point_keys);
        click_point(&mut rig, [0.0, 0.0]);
        assert_eq!(rig.selected.get(), Some(0));
        rig.fixture.key(egui::Key::Delete, egui::Modifiers::NONE).idle(1);
        assert_eq!(rig.points(), FOUR.to_vec(), "an end point stays");
        assert_eq!(rig.selected.get(), after, "end_delete_deselects: {point_keys}");
        click_point(&mut rig, [192.0, 192.0]);
        rig.fixture.key(egui::Key::Delete, egui::Modifiers::NONE).idle(1);
        assert_eq!(rig.points(), vec![[0.0, 0.0], [64.0, 64.0], [255.0, 255.0]], "an inner point goes");
    }
}
