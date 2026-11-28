//! Curves point spacing and the point fields (photocraft #14434): in a Curves dialog a moved point
//! keeps four input levels between it and a neighbour that is already four or more away — for
//! drags, arrow keys and the Input field alike, since all three go through `CurveRules::move_to` —
//! and the fields clamp a typed value to that range and keep the closest legal value. The gap is
//! the host's ask (`CurveRules::min_gap`): a bare rule set and a plain editor — the Contour
//! Editor, the raw Develop curve, a pressure-curve prefs editor — keep today's one-level
//! arithmetic, and a curve loaded with a closer pair keeps it under the Curves ask too.

use super::curve::{rig_gap, rig_with};
use craft_imaging::ui::adjust::{CurvesPanel, HostedAdjustDialog, ToneChannels, ToneCurves, ToneSpace};
use craft_imaging::ui::curve::{Curve, CurveRules};
use craft_ui::contract::Role;
use craft_ui::dialog::{Alert, DialogHost, HostedAlert};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::RangeAlert;
use egui::{Key, Modifiers, pos2, vec2};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const THREE: [[f32; 2]; 3] = [[0.0, 0.0], [128.0, 128.0], [255.0, 255.0]];
const FOUR: [[f32; 2]; 4] = [[0.0, 0.0], [128.0, 128.0], [140.0, 128.0], [255.0, 255.0]];

/// The rules a Curves dialog host builds ([`CurveRules::MIN_GAP`]).
fn rules() -> CurveRules {
    CurveRules::new(255.0, 16).min_gap(CurveRules::MIN_GAP)
}

// The model: drags, arrow keys and the fields all move points through `move_to`.

/// A fractional gap ask rounds up to the next whole level: bounds, clamp and stored input all
/// stay on whole levels, and the asked distance is still kept or exceeded.
#[trial]
fn a_fractional_gap_ask_still_moves_in_whole_levels() {
    let asking = || CurveRules::new(255.0, 16).min_gap(2.5);
    let mut points = vec![[0.0, 0.0], [8.0, 8.0], [16.0, 16.0]];
    assert!(asking().move_to(&mut points, 1, [0.0, 8.0]), "a move past the gap is held short of it");
    let input = points[1][0];
    assert_eq!(input, input.round(), "the point is held on a whole level, not on a fractional input: {input}");
    assert!(input >= 2.5 && 16.0 - input >= 2.5, "and the asked distance holds on both sides: {input}");
}

#[trial]
fn an_inner_point_move_keeps_four_levels_from_neighbours_that_far_off() {
    let mut points = FOUR.to_vec();
    assert!(rules().move_to(&mut points, 1, [136.0, 128.0]), "toward the point at 140");
    assert_eq!(points[1], [136.0, 128.0], "136 sits exactly four levels short");
    let mut points = FOUR.to_vec();
    assert!(rules().move_to(&mut points, 1, [137.0, 128.0]), "one level further is held back");
    assert_eq!(points[1], [136.0, 128.0], "at the four-level gap");
    let mut points = FOUR.to_vec();
    assert!(rules().move_to(&mut points, 2, [131.0, 128.0]), "and from the other side");
    assert_eq!(points[2], [132.0, 128.0], "the same gap holds");
    assert!(!rules().move_to(&mut points, 2, [130.0, 128.0]), "a step that stays clamped changes nothing");
    assert_eq!(points[2], [132.0, 128.0]);
    let mut points = FOUR.to_vec();
    assert!(rules().move_to(&mut points, 1, [104.0, 20.0]), "a far neighbour does not hold a point at all");
    assert_eq!(points[1], [104.0, 20.0]);
}

#[trial]
fn the_end_points_move_from_their_level_to_the_neighbours_gap() {
    let mut points = THREE.to_vec();
    assert!(rules().move_to(&mut points, 0, [3.0, 3.0]), "the end at 0 can move from 0");
    assert_eq!(points[0], [3.0, 3.0]);
    let mut points = THREE.to_vec();
    assert!(rules().move_to(&mut points, 0, [300.0, 20.0]), "and stops four levels short of the next point");
    assert_eq!(points[0], [124.0, 20.0]);
    let mut points = THREE.to_vec();
    assert!(!rules().move_to(&mut points, 0, [-20.0, 0.0]), "but its input never goes below 0");
    assert_eq!(points[0], [0.0, 0.0]);
    let mut points = THREE.to_vec();
    assert!(rules().move_to(&mut points, 2, [-40.0, 200.0]), "the end at 255 stops four levels above the point at 128");
    assert_eq!(points[2], [132.0, 200.0]);
    let mut points = THREE.to_vec();
    assert!(!rules().move_to(&mut points, 2, [260.0, 255.0]), "and its input never goes above 255");
    assert_eq!(points[2], [255.0, 255.0]);
    // An inner point between the ends takes the whole range minus the gaps: typing 300 keeps 251, typing 2 keeps 4.
    let mut points = THREE.to_vec();
    assert!(rules().move_to(&mut points, 1, [300.0, 128.0]), "the middle point stops at 251");
    assert_eq!(points[1], [251.0, 128.0]);
    let mut points = vec![[0.0, 0.0], [251.0, 128.0], [255.0, 255.0]];
    assert!(rules().move_to(&mut points, 1, [2.0, 128.0]), "and lands on 4 from below");
    assert_eq!(points[1], [4.0, 128.0]);
    let mut points = vec![[0.0, 0.0], [10.0, 50.0], [12.0, 60.0], [255.0, 255.0]];
    assert!(rules().move_to(&mut points, 3, [14.0, 255.0]), "a close pair elsewhere changes nothing for the four levels");
    assert_eq!(points[3], [16.0, 255.0], "the end stops four levels above the neighbour at 12");
}

/// A curve whose points are closer than four levels arrives from scripts and files; those pairs
/// keep today's one-level rule: a move never separates them and never lets them share an input.
#[trial]
fn a_curve_loaded_with_closer_points_keeps_the_one_level_rule() {
    let close = || vec![[0.0, 0.0], [10.0, 50.0], [12.0, 60.0], [255.0, 255.0]];
    let mut points = close();
    assert!(rules().move_to(&mut points, 2, [9.0, 60.0]), "a closer move lands one level in, not four");
    assert_eq!(points[2], [11.0, 60.0]);
    let mut points = close();
    assert!(rules().move_to(&mut points, 1, [13.0, 50.0]), "and the pair is not pushed apart");
    assert_eq!(points[1], [11.0, 50.0]);
    let mut points = vec![[0.0, 0.0], [10.0, 50.0], [10.0, 60.0], [255.0, 255.0]];
    assert!(rules().move_to(&mut points, 2, [10.0, 70.0]), "two points on one input are moved apart to one level");
    assert_eq!(points[2], [11.0, 70.0]);
}

/// The four-level gap is the Curves host's ask: a bare `CurveRules` — what the Contour Editor, the
/// raw Develop curve and a pressure-curve prefs editor get — keeps today's strictly-between
/// arithmetic.
#[trial]
fn a_bare_rule_set_still_keeps_todays_one_level_arithmetic() {
    let bare = || CurveRules::new(255.0, 16);
    let mut points = FOUR.to_vec();
    assert!(bare().move_to(&mut points, 2, [127.0, 128.0]), "a bare rule set moves strictly between");
    assert_eq!(points[2], [129.0, 128.0], "one level off the neighbour at 128, as today");
}

// The graph: a drag and the arrow keys run through the same rule.

/// The editor a host that asks for no gap builds — today's default.
fn rig_default(points: &[[f32; 2]]) -> super::curve::Rig {
    rig_with(points, false)
}

/// Drags the grabbed point to the screen position of levels `to`, from nodes calibrated on the
/// curve's own end points — levels, never measured pixels.
fn drag(rig: &mut super::curve::Rig, from: [f32; 2], to: [f32; 2]) {
    let n = rig.points().len();
    let (first, last) = (
        rig.fixture.by_label(&format!("Point 1 of {n}")).expect("the first point").center(),
        rig.fixture.by_label(&format!("Point {n} of {n}")).expect("the last point").center(),
    );
    let (points_first, points_last) = (rig.points()[0], rig.points()[n - 1]);
    let scale = vec2((last.x - first.x) / (points_last[0] - points_first[0]), (last.y - first.y) / (points_last[1] - points_first[1]));
    let at = |q: [f32; 2]| pos2(first.x + (q[0] - points_first[0]) * scale.x, first.y + (q[1] - points_first[1]) * scale.y);
    rig.press_at(from);
    let target = at(to);
    rig.fixture.frame(vec![craft_ui::testing::pointer::moved(target)]);
    rig.fixture.frame(vec![craft_ui::testing::pointer::release(target)]);
    rig.fixture.idle(1);
}

#[trial]
fn dragging_a_point_toward_a_neighbour_stops_four_levels_short() {
    let mut rig = rig_gap(&FOUR, false, Some(CurveRules::MIN_GAP));
    drag(&mut rig, [140.0, 128.0], [131.0, 128.0]);
    assert_eq!(rig.points(), vec![[0.0, 0.0], [128.0, 128.0], [132.0, 128.0], [255.0, 255.0]], "the drag stops at the four-level gap");
}

/// A host that asks for no gap — the default editor every other host builds — drags as today: the
/// point lands one level off its neighbour.
#[trial]
fn a_default_host_drag_still_lands_one_level_off_its_neighbour() {
    let mut rig = rig_default(&FOUR);
    drag(&mut rig, [140.0, 128.0], [127.0, 128.0]);
    assert_eq!(rig.points(), vec![[0.0, 0.0], [128.0, 128.0], [129.0, 128.0], [255.0, 255.0]], "one level off the point at 128, as today");
}

#[trial]
fn a_point_added_by_a_drag_is_clamped_the_same_way() {
    let mut rig = rig_gap(&THREE, false, Some(CurveRules::MIN_GAP));
    drag(&mut rig, [60.0, 60.0], [126.0, 130.0]);
    assert_eq!(
        rig.points(),
        vec![[0.0, 0.0], [124.0, 130.0], [128.0, 128.0], [255.0, 255.0]],
        "the added point stops four levels from its neighbour"
    );
}

#[trial]
fn arrow_key_steps_stop_at_the_four_level_gap() {
    let mut rig = rig_gap(&FOUR, false, Some(CurveRules::MIN_GAP));
    drag(&mut rig, [140.0, 128.0], [132.0, 128.0]);
    assert_eq!(rig.points()[2], [132.0, 128.0], "the drag lands exactly four levels from the point at 128");
    rig.press_at([132.0, 128.0]);
    rig.release_at([132.0, 128.0]);
    assert_eq!(rig.selected.get(), Some(2));
    for _ in 0..3 {
        rig.fixture.key(Key::ArrowLeft, Modifiers::NONE).idle(1);
    }
    assert_eq!(rig.points()[2], [132.0, 128.0], "three left steps are held at the four-level gap");
    rig.fixture.key(Key::ArrowLeft, Modifiers::SHIFT).idle(1);
    assert_eq!(rig.points()[2], [132.0, 128.0], "and a Shift step stays there");
    rig.fixture.key(Key::ArrowRight, Modifiers::NONE).idle(1);
    assert_eq!(rig.points()[2], [133.0, 128.0], "away from the neighbour the keys still work");
}

// The fields: the Input field's range is the same range, and a typed value outside it is kept at
// the closest legal level.

/// The Curves value of the trial: three points on the composite curve.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct HostCurves(ToneCurves);

impl Default for HostCurves {
    fn default() -> Self {
        let mut curves = ToneCurves::identity(&ToneChannels::of(ToneSpace::Rgb, false));
        curves.curves[0] = Curve::smooth(vec![[0.0, 0.0], [128.0, 160.0], [255.0, 255.0]]);
        HostCurves(curves)
    }
}

type CurvesDialog = HostedAdjustDialog<HostCurves, &'static str>;
type RangeAlerts = Arc<Mutex<Option<RangeAlert>>>;

/// The dialog open on its host, with the last pass's clamped-value report and a way to open it as
/// the host would. The host keeps the [`RangeAlert`] and words it with `Alert::range` on the next
/// pass, exactly as photocraft's `answer_requests` does for a hosted form dialog.
fn curves_host() -> (Rc<RefCell<DialogHost>>, Fixture, RangeAlerts, Arc<AtomicBool>) {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let alerts: RangeAlerts = Arc::new(Mutex::new(None));
    let alerts_out = Arc::clone(&alerts);
    let word_it = Arc::new(AtomicBool::new(false));
    let word_out = Arc::clone(&word_it);
    let shown = Rc::clone(&host);
    let panel_alerts = Arc::clone(&alerts);
    let word_it_run = Arc::clone(&word_it);
    let mut fixture = Fixture::new(move |ui| {
        shown.borrow_mut().run(ui);
        if word_it_run.load(Ordering::SeqCst) {
            let alert = alerts.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone().expect("a reported value to word");
            word_it_run.store(false, Ordering::SeqCst);
            shown.borrow_mut().open(HostedAlert::new(Alert::range("Curves", &alert, &Env::of(ui))));
        }
    });
    fixture.room(egui::Vec2::new(1400.0, 1000.0));
    host.borrow_mut().open(CurvesDialog::new("image.adjustments.curves", "Curves", HostCurves::default(), move |ui, value, cx| {
        let mut panel =
            CurvesPanel::new("Curves", craft_ui::contract::Bind::bound(&mut value.0), ToneChannels::of(ToneSpace::Rgb, false)).context(cx.context);
        if let Some(point) = cx.select_point {
            panel = panel.select(point);
        }
        if let Some(alert) = panel.show(ui).range_alert {
            *panel_alerts.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(alert);
        }
        None
    }));
    fixture.idle(10);
    if let Some(dialog) = host.borrow_mut().get_mut::<CurvesDialog>() {
        dialog.select_point(1);
    }
    fixture.settle();
    let _ = fixture.by_role(Role::DIALOG).first().expect("the Curves dialog shows");
    (host, fixture, alerts_out, word_out)
}

/// Types `text` into the field named `name` and commits it with Tab.
fn enter(fixture: &mut Fixture, name: &str, text: &str) {
    let field = fixture.by_label(name).unwrap_or_else(|| panic!("the {name} field"));
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text(text);
    fixture.key(Key::Tab, Modifiers::NONE).settle();
}

fn point(host: &Rc<RefCell<DialogHost>>) -> Option<[f32; 2]> {
    host.borrow().get::<CurvesDialog>().and_then(|dialog| dialog.value().0.curves[0].points().map(|points| points[1]))
}

#[trial]
fn typing_past_the_input_range_keeps_the_closest_level() {
    let (host, mut fixture, ..) = curves_host();
    enter(&mut fixture, "Input", "300");
    assert_eq!(point(&host), Some([251.0, 160.0]), "300 becomes 251: four levels below the end at 255");
    enter(&mut fixture, "Input", "2");
    assert_eq!(point(&host), Some([4.0, 160.0]), "and 2 becomes 4: four levels above the end at 0");
    enter(&mut fixture, "Output", "300");
    assert_eq!(point(&host), Some([4.0, 255.0]), "an output past 255 keeps 255");
    enter(&mut fixture, "Input", "96");
    assert_eq!(point(&host), Some([96.0, 255.0]), "a value inside the range moves the point as typed");
}

#[trial]
fn typing_past_a_range_reports_the_clamped_value_once() {
    let (_host, mut fixture, alerts, _) = curves_host();
    let taken = || alerts.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
    assert!(taken().is_none(), "nothing is reported until a value is held back");
    enter(&mut fixture, "Input", "300");
    let alert = taken().expect("the held input is reported");
    assert_eq!((alert.typed, alert.kept), (300.0, 251.0), "300 typed, the closest level kept");
    assert_eq!(alert.range, 4.0..=251.0, "the point's four-level range");
    assert!(alert.whole, "the field commits whole numbers");
    assert!(alert.unit.is_none(), "levels, no unit");
    enter(&mut fixture, "Input", "96");
    assert!(taken().is_none(), "a value in range reports nothing");
    enter(&mut fixture, "Output", "300");
    let alert = taken().expect("the held output is reported");
    assert_eq!((alert.typed, alert.kept), (300.0, 255.0), "300 typed, 255 kept");
    assert_eq!(alert.range, 0.0..=255.0, "the output's range");
}

#[trial]
fn the_range_alert_words_the_closest_value_sentence() {
    let (_host, mut fixture, alerts, word_it) = curves_host();
    enter(&mut fixture, "Input", "300");
    assert!(alerts.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_some(), "the clamped value is reported");
    word_it.store(true, Ordering::SeqCst);
    fixture.idle(3);
    let wanted = "An integer between 4 and 251 is required. Closest value inserted.";
    let shown = fixture.texts().iter().any(|text| text.text == wanted);
    assert!(shown, "the host-opened alert shows the whole sentence ({wanted})");
    assert!(fixture.texts().iter().any(|text| text.text == "251"), "and the field carries the kept value");
}
