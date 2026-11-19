//! The curve editor: pointer and key editing of the points, the fields, the pencil, the corners and the AccessKit
//! nodes, driven through the editor as a host would show it.

use craft_behavior::EditEvent;
use craft_imaging::ui::ChannelColor;
use craft_imaging::ui::adjust::{AdjustContext, CurvesPanel, ToneChannels, ToneCurves, ToneSpace};
use craft_imaging::ui::curve::{Curve, CurveChannel, CurveEditor};
use craft_theme::{Contrast, Mode};
use craft_ui::contract::Bind;
use craft_ui::testing::pointer::{moved, press, release, secondary};
use craft_ui::testing::{Fixture, Room, style, trial};
use egui::{Modifiers, Pos2, Vec2, pos2, vec2};
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

/// The rig, with [`CurveEditor::wide_grab`] when `wide_grab` is on.
fn rig_with(points: &[[f32; 2]], wide_grab: bool) -> Rig {
    let curve = Curve::smooth(points.to_vec());
    let shared = Rc::new(RefCell::new(curve.clone()));
    let selected = Rc::new(Cell::new(None));
    let kinds = Rc::new(RefCell::new(Vec::new()));
    let (c, s, k) = (shared.clone(), selected.clone(), kinds.clone());
    let channels = [CurveChannel::new("RGB", ChannelColor::Neutral), CurveChannel::new("Red", ChannelColor::Red)];
    let mut fixture = Fixture::new(move |ui| {
        let mut value = c.borrow().clone();
        let mut editor = CurveEditor::new("Curve", Bind::bound(&mut value)).channels(&channels).channel(0).selection(s.get());
        if wide_grab {
            editor = editor.wide_grab();
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

    /// A click `by` screen points from the level `q`: press and release in place.
    fn click_near(&mut self, q: [f32; 2], by: Vec2) {
        let pos = self.at(q) + by;
        click(&mut self.fixture, pos);
    }
}

fn click(fixture: &mut Fixture, pos: Pos2) {
    fixture.frame(vec![moved(pos)]);
    fixture.frame(vec![press(pos)]);
    fixture.frame(vec![release(pos)]);
    fixture.idle(1);
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

const MIDDLE: [[f32; 2]; 3] = [[0.0, 0.0], [128.0, 128.0], [255.0, 255.0]];

/// Photoshop's Curves dialog at UI scale 1: a press 8, 13 or 15 px from a point selects it and adds nothing, in any
/// direction.
#[trial]
fn a_wide_grab_takes_a_point_up_to_fifteen_px_away() {
    for by in [vec2(8.0, 0.0), vec2(13.0, 0.0), vec2(15.0, 0.0), vec2(-15.0, 0.0), vec2(9.0, -12.0)] {
        let mut rig = rig_with(&MIDDLE, true);
        rig.click_near([128.0, 128.0], by);
        assert_eq!(rig.points(), MIDDLE.to_vec(), "a click {by:?} from the point adds nothing and does not move it");
        assert_eq!(rig.selected.get(), Some(1), "a click {by:?} from the point selects it");
    }
}

/// A press 30 px from every point still adds one there and selects it.
#[trial]
fn a_wide_grab_still_adds_a_point_thirty_px_away() {
    let mut rig = rig_with(&MIDDLE, true);
    rig.click_near([128.0, 128.0], vec2(30.0, 0.0));
    let points = rig.points();
    assert_eq!(points.len(), 4, "{points:?}");
    assert_eq!(rig.selected.get(), Some(2), "the new point is selected: {points:?}");
}

/// A right click and a Ctrl press delete a point within one hit target only, with the wide grab on too: Photoshop was
/// measured for plain clicks. 7 px stays clear of the 8 px boundary, which screen coordinates round across.
#[trial]
fn a_wide_grab_keeps_the_delete_reach_of_one_hit_target() {
    let mut wrong = Vec::new();
    for (ctrl, by, deletes) in [(false, 7.0, true), (false, 13.0, false), (true, 7.0, true), (true, 13.0, false)] {
        let mut rig = rig_with(&MIDDLE, true);
        let pos = rig.at([128.0, 128.0]) + vec2(by, 0.0);
        rig.fixture.frame(vec![moved(pos)]);
        if ctrl {
            rig.fixture.hold(Modifiers::CTRL);
            rig.fixture.frame(vec![press(pos)]);
            rig.fixture.frame(vec![release(pos)]);
            rig.fixture.hold(Modifiers::NONE);
        } else {
            rig.fixture.frame(vec![secondary(pos, true)]);
            rig.fixture.frame(vec![secondary(pos, false)]);
        }
        rig.fixture.idle(1);
        let kept = if deletes { vec![[0.0, 0.0], [255.0, 255.0]] } else { MIDDLE.to_vec() };
        let how = if ctrl { "a Ctrl press" } else { "a right click" };
        if rig.points() != kept {
            wrong.push(format!("{how} {by} px from the point left {:?}", rig.points()));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Without the wide grab the editor keeps one hit target: a press 13 px from a point adds a second one.
#[trial]
fn without_a_wide_grab_a_press_thirteen_px_away_adds_a_point() {
    let mut rig = rig(&MIDDLE);
    rig.click_near([128.0, 128.0], vec2(13.0, 0.0));
    assert_eq!(rig.points().len(), 4, "{:?}", rig.points());
}

/// The point count of the composite curve after a click 15 px right of its middle point, in the Curves panel shown
/// in `context`, and the selection after it.
fn click_beside_the_middle_point(context: AdjustContext) -> (usize, Option<usize>) {
    let channels = ToneChannels::of(ToneSpace::Rgb, false);
    let mut start = ToneCurves::identity(&channels);
    start.curves[0] = Curve::smooth(MIDDLE.to_vec());
    let curves = Rc::new(RefCell::new(start));
    let selected = Rc::new(Cell::new(None));
    let (c, s) = (curves.clone(), selected.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut value = c.borrow().clone();
        let out = CurvesPanel::new("Curves", Bind::bound(&mut value), channels.clone()).context(context).show(ui);
        s.set(out.selected);
        *c.borrow_mut() = value;
    });
    fixture.room(room());
    fixture.settle();
    let at = fixture.by_label("Point 2 of 3").expect("the middle point").center();
    click(&mut fixture, at + vec2(15.0, 0.0));
    let count = curves.borrow().curves[0].points().map_or(0, <[_]>::len);
    (count, selected.get())
}

/// The Curves dialog takes a press 15 px from a point; the Properties panel, which was not measured, keeps one hit
/// target and adds a point there.
#[trial]
fn the_curves_dialog_takes_a_point_fifteen_px_away_and_properties_keeps_one_hit_target() {
    assert_eq!(click_beside_the_middle_point(AdjustContext::dialog(true)), (3, Some(1)), "the dialog selects the point");
    assert_eq!(click_beside_the_middle_point(AdjustContext::properties(true)), (4, Some(2)), "Properties adds a point");
}
