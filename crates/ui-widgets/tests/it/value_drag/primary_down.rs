//! Physical primary track Down must produce its opt-in preview before movement or release.
use craft_behavior::{EditEvent, GestureId, Mixed, MotionMode};
use craft_ui::contract::{Bind, Props as _, Role, Show as _};
use craft_ui::form::{SliderField, SliderFieldLayout};
use craft_ui::runtime::focus;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::value_drag::Slider;
use egui::{Event, Id, Key, Modifiers, PointerButton, Pos2, Rect, UiBuilder, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

#[path = "primary_down_route.rs"]
mod route;

const START: f64 = 0.25;
const ROWS: [Rect; 2] =
    [Rect { min: Pos2::new(30.0, 30.0), max: Pos2::new(430.0, 100.0) }, Rect { min: Pos2::new(30.0, 130.0), max: Pos2::new(430.0, 200.0) }];
#[derive(Clone, Copy, Default)]
enum Probe {
    #[default]
    Real,
    Sizing,
    Measure,
}
struct State {
    values: [f64; 2],
    events: [Vec<EditEvent<f64>>; 2],
    nonreal: Vec<EditEvent<f64>>,
    enabled: bool,
    read_only: bool,
    probe: Probe,
    measurements: usize,
    reset: Option<f64>,
}
struct Geometry {
    owner: Id,
    thumb: Rect,
    slider: Rect,
}
impl Geometry {
    fn at(&self, fraction: f32) -> Pos2 {
        let travel = self.slider.width() - self.thumb.width();
        Pos2::new(self.slider.left() + self.thumb.width() / 2.0 + travel * fraction, self.thumb.center().y)
    }
}
struct Rig {
    fixture: Fixture,
    state: Rc<RefCell<State>>,
    fields: bool,
}
impl Rig {
    fn new(fields: bool, policy: Option<bool>) -> Self {
        Self::ranged(fields, policy, 0.0..=1.0, START, 0.01, None, false)
    }
    fn ranged(
        fields: bool,
        policy: Option<bool>,
        range: std::ops::RangeInclusive<f64>,
        start: f64,
        step: f64,
        grid: Option<f64>,
        fine_shift: bool,
    ) -> Self {
        let state = Rc::new(RefCell::new(State {
            values: [start; 2],
            events: [Vec::new(), Vec::new()],
            nonreal: Vec::new(),
            enabled: true,
            read_only: false,
            probe: Probe::Real,
            measurements: 0,
            reset: None,
        }));
        let shared = state.clone();
        let mut fixture = Fixture::new(move |root| {
            let probe = shared.borrow().probe;
            let builder = UiBuilder::new().scope_id(Id::unique("primary-down-physical-host")).max_rect(root.max_rect());
            let builder = if matches!(probe, Probe::Sizing) { builder.sizing_pass().invisible() } else { builder };
            let mut child = root.new_child(builder);
            let draw = |ui: &mut egui::Ui| {
                let mut state = shared.borrow_mut();
                for (row, &rect) in ROWS.iter().enumerate() {
                    let before = state.values[row];
                    let mut value = before;
                    let bind = Bind::bound(&mut value);
                    let bind = match state.reset {
                        Some(default) => bind.reset(default),
                        None => bind,
                    };
                    let policy = policy.filter(|_| row == 0);
                    let output = if fields {
                        let field = SliderField::new(if row == 0 { "Amount" } else { "Balance" }, bind, range.clone())
                            .id_salt(row)
                            .layout(SliderFieldLayout::Inline)
                            .step(step)
                            .enabled(state.enabled)
                            .read_only(state.read_only);
                        let mut field = route::field(field, policy, grid, fine_shift);
                        if matches!(probe, Probe::Measure) {
                            let size = field.measure_in(ui, craft_behavior::Proposal::Ideal);
                            assert!(size.x.is_finite() && size.y.is_finite() && size.x > 0.0 && size.y > 0.0);
                            None
                        } else {
                            Some(field.show_at(ui, rect).edit)
                        }
                    } else {
                        let slider = Slider::new(bind, range.clone())
                            .id_salt(row)
                            .label(if row == 0 { "Amount" } else { "Balance" })
                            .step(step)
                            .enabled(state.enabled)
                            .read_only(state.read_only);
                        let mut slider = route::slider(slider, policy, grid, fine_shift);
                        if matches!(probe, Probe::Measure) {
                            let size = slider.measure_in(ui, craft_behavior::Proposal::Ideal);
                            assert!(size.x.is_finite() && size.y.is_finite() && size.x > 0.0 && size.y > 0.0);
                            None
                        } else {
                            Some(slider.show_at(ui, rect))
                        }
                    };
                    if matches!(probe, Probe::Measure) || ui.is_sizing_pass() || craft_ui::runtime::measure::active(ui.ctx()) {
                        assert_eq!(value, before, "a nonreal callback must not write the binding");
                        if matches!(probe, Probe::Measure) {
                            state.measurements += 1;
                        }
                        if let Some(output) = output {
                            state.nonreal.extend(output.events);
                        }
                    } else {
                        state.values[row] = value;
                        state.events[row].extend(output.expect("a real show returns its actual edit response").events);
                    }
                }
            };
            draw(&mut child);
        });
        fixture.motion(MotionMode::Off).settle();
        Self { fixture, state, fields }
    }
    fn geometry(&mut self, row: usize) -> Geometry {
        for _ in 0..((row + 1) * if self.fields { 2 } else { 1 }) {
            self.fixture.key(Key::Tab, Modifiers::NONE).settle();
        }
        let owner = self.fixture.focused_stop().expect("naturally focused real Slider child").0;
        let record = focus::focused_rec(self.fixture.ctx()).expect("current genuine focused record");
        assert_eq!(record.stop.id, owner);
        assert!(!record.stop.text, "the numeric editor is not the slider");
        self.observed_geometry(row, owner)
    }
    fn observed_geometry(&mut self, row: usize, owner: Id) -> Geometry {
        let thumb = self.fixture.part_rect(owner, Slider::THUMB).expect("actual registered thumb");
        let slider = self.fixture.by_role(Role::SLIDER)[row].rect;
        assert!(slider.contains(thumb.center()));
        assert!(slider.width() - thumb.width() > 100.0);
        Geometry { owner, thumb, slider }
    }
    fn clear(&mut self) {
        self.state.borrow_mut().events = [Vec::new(), Vec::new()];
    }
    fn down(&mut self, geometry: &Geometry, at: Pos2) {
        assert!(!geometry.thumb.contains(at), "the actual initial press is track, not thumb");
        self.clear();
        self.fixture.frame(vec![pointer::moved(at), pointer::press(at)]);
        let targets = craft_ui::runtime::pointer::with_pass_pointer(self.fixture.ctx(), |pointer| pointer.press_targets);
        assert!(targets.click == Some(geometry.owner) || targets.drag == Some(geometry.owner), "real registered native owner");
    }
    fn open(&self, expected: f64, before: f64) -> GestureId {
        let state = self.state.borrow();
        assert_eq!(state.values[0], expected, "same-pass preview, before movement/release/idle");
        assert!(state.events[1].is_empty(), "no sibling operation");
        assert!(
            matches!(state.events[0].as_slice(),
            [EditEvent::Begin { id: begin, before: Mixed::Same(old), .. }, EditEvent::Update { id, value, .. }]
                if begin == id && *old == before && *value == expected),
            "one genuine Begin/update only: {:?}",
            state.events[0]
        );
        match &state.events[0][0] {
            EditEvent::Begin { id, .. } => *id,
            _ => unreachable!(),
        }
    }
}

#[trial]
fn primary_track_down_previews_immediately_and_release_commits_the_same_gesture_once() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(true));
        let geometry = rig.geometry(0);
        let at = geometry.at(0.75);
        rig.down(&geometry, at);
        let id = rig.open(0.75, START);
        rig.clear();
        rig.fixture.frame(vec![pointer::release(at)]);
        let state = rig.state.borrow();
        assert_eq!(state.values[0], 0.75);
        assert!(
            matches!(state.events[0].as_slice(), [EditEvent::Commit { id: commit, before: Mixed::Same(old), after, .. }]
            if *commit == id && *old == START && *after == 0.75),
            "one commit, no new Begin/Set: {:?}",
            state.events[0]
        );
    }
}

#[trial]
fn crossing_slop_after_primary_down_updates_without_a_second_begin() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(true));
        let geometry = rig.geometry(0);
        let at = geometry.at(0.75);
        rig.down(&geometry, at);
        let id = rig.open(0.75, START);
        rig.clear();
        let to = geometry.at(0.5);
        rig.fixture.frame(vec![pointer::moved(to)]);
        assert_eq!(rig.state.borrow().values[0], 0.5);
        assert!(
            matches!(rig.state.borrow().events[0].as_slice(), [EditEvent::Update { id: update, value, .. }]
            if *update == id && *value == 0.5),
            "slop crossing keeps the existing owner"
        );
        rig.clear();
        rig.fixture.frame(vec![pointer::release(to)]);
        assert!(matches!(rig.state.borrow().events[0].as_slice(), [EditEvent::Commit { id: commit, after, .. }]
            if *commit == id && *after == 0.5));
    }
}

#[trial]
fn escape_cancels_primary_down_without_old_release_replay_and_a_fresh_track_press_recovers() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(true));
        let geometry = rig.geometry(0);
        let at = geometry.at(0.75);
        rig.down(&geometry, at);
        let id = rig.open(0.75, START);
        rig.clear();
        rig.fixture.frame(vec![key_press(Key::Escape, Modifiers::NONE)]);
        assert_eq!(rig.state.borrow().values[0], START);
        assert!(matches!(rig.state.borrow().events[0].as_slice(), [EditEvent::Cancel { id: cancel, .. }] if *cancel == id));
        rig.clear();
        rig.fixture.frame(vec![
            pointer::release(at),
            Event::Key { key: Key::Escape, physical_key: Some(Key::Escape), pressed: false, repeat: false, modifiers: Modifiers::NONE },
        ]);
        assert!(rig.state.borrow().events.iter().all(Vec::is_empty));
        assert_eq!(rig.state.borrow().values[0], START);
        rig.down(&geometry, geometry.at(0.5));
        rig.open(0.5, START);
    }
}

#[trial]
fn omitted_and_false_primary_down_policy_keep_legacy_click_timing() {
    for fields in [false, true] {
        for policy in [None, Some(false)] {
            let mut rig = Rig::new(fields, policy);
            let geometry = rig.geometry(0);
            let at = geometry.at(0.75);
            rig.down(&geometry, at);
            assert_eq!(rig.state.borrow().values[0], START);
            assert!(rig.state.borrow().events.iter().all(Vec::is_empty));
            rig.fixture.frame(vec![pointer::release(at)]);
            assert!(matches!(rig.state.borrow().events[0].as_slice(), [EditEvent::Set { before: Mixed::Same(old), after, .. }]
            if *old == START && *after == 0.75));
        }
    }
}

#[trial]
fn primary_down_policy_leaves_thumb_press_and_begin_only_first_drag_unchanged() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(true));
        let geometry = rig.geometry(0);
        let at = geometry.thumb.center();
        rig.clear();
        rig.fixture.frame(vec![pointer::moved(at), pointer::press(at)]);
        assert_eq!(rig.state.borrow().values[0], START);
        assert!(rig.state.borrow().events.iter().all(Vec::is_empty));
        rig.fixture.frame(vec![pointer::moved(at + Vec2::new(50.0, 0.0))]);
        assert_eq!(rig.state.borrow().values[0], START);
        assert!(matches!(rig.state.borrow().events[0].as_slice(), [EditEvent::Begin { .. }]));
    }
}

#[trial]
fn disabled_and_read_only_initial_downs_are_inert_and_old_release_does_not_activate_after_reenable() {
    for fields in [false, true] {
        for read_only in [false, true] {
            let mut rig = Rig::new(fields, Some(true));
            let geometry = rig.geometry(0);
            let at = geometry.at(0.75);
            {
                let mut state = rig.state.borrow_mut();
                state.enabled = read_only;
                state.read_only = read_only;
            }
            rig.clear();
            rig.fixture.frame(vec![pointer::moved(at), pointer::press(at)]);
            assert_eq!(rig.state.borrow().values[0], START);
            assert!(rig.state.borrow().events.iter().all(Vec::is_empty));
            {
                let mut state = rig.state.borrow_mut();
                state.enabled = true;
                state.read_only = false;
            }
            rig.fixture.frame(vec![pointer::release(at)]);
            assert_eq!(rig.state.borrow().values[0], START);
            assert!(rig.state.borrow().events.iter().all(Vec::is_empty));
            rig.down(&geometry, geometry.at(0.5));
            rig.open(0.5, START);
        }
    }
}

#[trial]
fn sizing_and_measurement_initial_downs_are_inert_and_do_not_replay_on_real_restore() {
    for fields in [false, true] {
        for probe in [Probe::Sizing, Probe::Measure] {
            let mut rig = Rig::new(fields, Some(true));
            let geometry = rig.geometry(0);
            let at = geometry.at(0.75);
            rig.state.borrow_mut().probe = probe;
            rig.clear();
            rig.fixture.frame(vec![pointer::moved(at), pointer::press(at)]);
            assert_eq!(rig.state.borrow().values[0], START);
            assert!(rig.state.borrow().events.iter().all(Vec::is_empty));
            assert!(rig.state.borrow().nonreal.is_empty());
            if matches!(probe, Probe::Measure) {
                assert!(rig.state.borrow().measurements >= 2, "both actual public measurement callbacks ran");
            }
            rig.state.borrow_mut().probe = Probe::Real;
            rig.fixture.frame(vec![pointer::release(at)]);
            assert_eq!(rig.state.borrow().values[0], START);
            assert!(rig.state.borrow().events.iter().all(Vec::is_empty));
            rig.down(&geometry, geometry.at(0.5));
            rig.open(0.5, START);
        }
    }
}

#[trial]
fn newer_host_state_cancels_the_original_down_and_a_held_drag_cannot_restart_it() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(true));
        let geometry = rig.geometry(0);
        let at = geometry.at(0.75);
        rig.down(&geometry, at);
        let id = rig.open(0.75, START);
        rig.clear();
        rig.state.borrow_mut().values[0] = 0.5;
        let to = geometry.at(0.875);
        rig.fixture.frame(vec![pointer::moved(to)]);
        assert_eq!(rig.state.borrow().values[0], 0.5, "newer model value wins over the stale press");
        assert!(matches!(rig.state.borrow().events[0].as_slice(), [EditEvent::Cancel { id: cancel, .. }] if *cancel == id));
        rig.clear();
        rig.fixture.frame(vec![pointer::release(to)]);
        assert_eq!(rig.state.borrow().values[0], 0.5);
        assert!(rig.state.borrow().events.iter().all(Vec::is_empty));
        let recovered = rig.observed_geometry(0, geometry.owner);
        rig.down(&recovered, recovered.at(0.25));
        rig.open(0.25, 0.5);
    }
}

#[trial]
fn primary_down_opt_in_does_not_leak_to_the_other_default_row() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(true));
        let geometry = rig.geometry(1);
        let at = geometry.at(0.75);
        rig.down(&geometry, at);
        assert_eq!(rig.state.borrow().values, [START; 2]);
        assert!(rig.state.borrow().events.iter().all(Vec::is_empty));
        rig.fixture.frame(vec![pointer::release(at)]);
        assert_eq!(rig.state.borrow().values, [START, 0.75]);
        assert!(rig.state.borrow().events[0].is_empty());
        assert!(matches!(rig.state.borrow().events[1].as_slice(), [EditEvent::Set { .. }]));
    }
}

#[trial]
fn fine_track_travel_continues_from_the_real_down_landing_not_the_pre_press_value() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(true));
        let geometry = rig.geometry(0);
        rig.fixture.hold(Modifiers::ALT);
        rig.down(&geometry, geometry.at(0.75));
        let id = rig.open(0.75, START);
        rig.clear();
        rig.fixture.frame(vec![pointer::moved(geometry.at(0.25))]);
        // The landing plus fine travel; remultiplying a binary decimal grid changes this by one ULP.
        let expected = 0.75 + (0.25 - 0.75) * 0.1;
        assert_eq!(rig.state.borrow().values[0], expected);
        assert!(
            matches!(rig.state.borrow().events[0].as_slice(), [EditEvent::Update { id: update, value, .. }]
            if *update == id && *value == expected),
            "one owner continues from the newly landed 0.75"
        );
    }
}

#[trial]
fn secondary_track_down_is_not_promoted_and_a_fresh_primary_down_still_works() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(true));
        let geometry = rig.geometry(0);
        let at = geometry.at(0.75);
        rig.clear();
        rig.fixture.frame(vec![
            pointer::moved(at),
            Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
        ]);
        assert_eq!(rig.state.borrow().values[0], START);
        assert!(rig.state.borrow().events.iter().all(Vec::is_empty));
        rig.fixture.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE }]);
        rig.down(&geometry, geometry.at(0.5));
        rig.open(0.5, START);
    }
}

fn coarse_step_double_track_reset(fields: bool) {
    let mut rig = Rig::ranged(fields, Some(true), 0.0..=1.0, START, 0.5, None, false);
    rig.state.borrow_mut().reset = Some(START);
    rig.fixture.settle();
    let first_geometry = rig.geometry(0);
    let at = first_geometry.at(0.70);
    rig.down(&first_geometry, at);
    let first_id = rig.open(0.5, START);
    let first_down = rig.state.borrow().events[0].clone();
    let first_physical = craft_ui::runtime::pointer::with_pass_pointer(rig.fixture.ctx(), |pointer| pointer.events.clone());
    rig.clear();
    rig.fixture.frame(vec![pointer::release(at)]);
    let first_release = rig.state.borrow().events[0].clone();
    assert!(matches!(first_release.as_slice(), [EditEvent::Commit { id, after, .. }] if *id == first_id && *after == 0.5));
    let second_geometry = rig.observed_geometry(0, first_geometry.owner);
    assert!(!second_geometry.thumb.contains(at), "same physical point remains track after snapped first landing");
    rig.down(&second_geometry, at);
    let second_physical = craft_ui::runtime::pointer::with_pass_pointer(rig.fixture.ctx(), |pointer| pointer.events.clone());
    let clicks = |events: &[craft_behavior::PointerEvent]| {
        events
            .iter()
            .filter_map(|event| match event {
                craft_behavior::PointerEvent::Pressed { button: craft_behavior::PointerButton::Primary, clicks, .. } => Some(*clicks),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(clicks(&first_physical), vec![1], "real first primary Down click count");
    assert_eq!(clicks(&second_physical), vec![2], "real second same-position primary Down double-click count");
    let second_down = rig.state.borrow().events[0].clone();
    let down_value = rig.state.borrow().values[0];
    rig.clear();
    rig.fixture.frame(vec![pointer::release(at)]);
    let released_value = rig.state.borrow().values[0];
    let second_release = rig.state.borrow().events[0].clone();
    eprintln!(
        "COARSE_DOUBLE_TRACK_RESET fields={fields} owner={:?} at={at:?} first_thumb={:?} second_thumb={:?} first_physical={first_physical:?} first_down={first_down:?} first_release={first_release:?} second_physical={second_physical:?} second_down={second_down:?} down_value={down_value:?} second_release={second_release:?} released_value={released_value:?}",
        first_geometry.owner, first_geometry.thumb, second_geometry.thumb
    );
    assert_eq!(
        [down_value, released_value],
        [START, START],
        "double-click reset must survive second Down and release; fields={fields}; second_down={second_down:?}; second_release={second_release:?}"
    );
}

#[trial]
fn opt_in_slider_double_track_click_preserves_coarse_step_reset() {
    coarse_step_double_track_reset(false);
}

#[trial]
fn opt_in_slider_field_double_track_click_preserves_coarse_step_reset() {
    coarse_step_double_track_reset(true);
}

#[trial]
fn a_live_wheel_commits_before_opted_track_down_and_the_fresh_press_keeps_dragging() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(true));
        let geometry = rig.geometry(0);
        rig.clear();
        rig.fixture.frame(vec![pointer::moved(geometry.thumb.center()), pointer::wheel_lines(1.0)]);
        let (wheel_id, wheel_value) = {
            let state = rig.state.borrow();
            assert!(state.events[1].is_empty());
            assert!(state.values[0] > START && state.values[0] < 0.75, "a real wheel preview must be live: {:?}", state.events[0]);
            match state.events[0].as_slice() {
                [EditEvent::Begin { id: begin, before: Mixed::Same(before), .. }, EditEvent::Update { id, value, .. }]
                    if begin == id && *before == START && *value == state.values[0] =>
                {
                    (*id, *value)
                }
                events => panic!("the genuine wheel must remain open before the press: {events:?}"),
            }
        };
        let at = geometry.at(0.75);
        rig.down(&geometry, at);
        let press_id = {
            let state = rig.state.borrow();
            eprintln!("WHEEL_TO_PRIMARY_DOWN fields={fields} wheel_id={wheel_id:?} wheel_value={wheel_value:?} down={:?}", state.events[0]);
            assert_eq!(state.values[0], 0.75, "the opted press still previews immediately");
            assert!(state.events[1].is_empty());
            match state.events[0].as_slice() {
                [
                    EditEvent::Commit { id: ended, before: Mixed::Same(old), after, .. },
                    EditEvent::Begin { id: begin, before: Mixed::Same(start), .. },
                    EditEvent::Update { id, value, .. },
                ] if *ended == wheel_id
                    && *old == START
                    && *after == wheel_value
                    && begin != ended
                    && *start == wheel_value
                    && begin == id
                    && *value == 0.75 =>
                {
                    *id
                }
                events => panic!("end the old wheel at its own preview before starting a fresh press: {events:?}"),
            }
        };
        for (fraction, expected) in [(0.5, 0.5), (0.6, 0.6)] {
            rig.clear();
            rig.fixture.frame(vec![pointer::moved(geometry.at(fraction))]);
            let state = rig.state.borrow();
            assert_eq!(state.values[0], expected, "DragStart and Drag must keep moving the fresh press");
            assert!(
                matches!(state.events[0].as_slice(), [EditEvent::Update { id, value, .. }]
                if *id == press_id && *value == expected),
                "one update in the fresh press, not an early commit or second Begin: {:?}",
                state.events[0]
            );
            assert!(state.events[1].is_empty());
        }
        rig.clear();
        rig.fixture.frame(vec![pointer::release(geometry.at(0.6))]);
        {
            let state = rig.state.borrow();
            assert!(
                matches!(state.events[0].as_slice(), [EditEvent::Commit { id, before: Mixed::Same(before), after, .. }]
                if *id == press_id && *before == wheel_value && *after == 0.6),
                "only Release commits the fresh press: {:?}",
                state.events[0]
            );
            assert!(state.events[1].is_empty());
        }
        rig.clear();
        rig.fixture.frame(Vec::new());
        assert!(rig.state.borrow().events.iter().all(Vec::is_empty), "no delayed wheel-end replay");
        assert_eq!(rig.state.borrow().values[0], 0.6);
    }
}

/// A read-only `SliderField` keeps its value whatever its track press option: a click, a press held still and a drag on
/// its track change nothing and report no edit. Before, only a field with the press option passed read-only to its
/// slider, so a read-only field without it kept a writable track.
#[trial]
fn a_read_only_slider_field_ignores_track_presses_with_either_track_press_option() {
    for policy in [None, Some(false), Some(true)] {
        let mut rig = Rig::new(true, policy);
        let geometry = rig.geometry(0);
        rig.state.borrow_mut().read_only = true;
        rig.fixture.settle();
        rig.clear();
        rig.fixture.click_at(geometry.at(0.8));
        rig.fixture.settle();
        rig.fixture.drag_hold_at(geometry.at(0.6), Vec2::new(40.0, 0.0));
        rig.fixture.release_at(geometry.at(0.6) + Vec2::new(40.0, 0.0));
        rig.fixture.settle();
        assert_eq!(rig.state.borrow().values[0], START, "a read-only field keeps its value ({policy:?})");
        assert!(rig.state.borrow().events.iter().all(Vec::is_empty), "and reports no edit ({policy:?})");
    }
}
