//! Public scalar contracts: real sensed thumbs, not synthetic response coordinates.
use craft_behavior::{CancelReason, EditEvent, MotionMode};
use craft_ui::contract::{Bind, Props as _, Role, Show as _};
use craft_ui::form::{SliderField, SliderFieldLayout};
use craft_ui::runtime::edit::cancel_all;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::value_drag::{DragModifier, DragModifiers, Slider};
use egui::{Id, Key, Modifiers, Pos2, Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const ROWS: [Rect; 2] =
    [Rect { min: Pos2::new(20.0, 20.0), max: Pos2::new(420.0, 90.0) }, Rect { min: Pos2::new(20.0, 120.0), max: Pos2::new(420.0, 190.0) }];
fn fine_shift() -> DragModifiers {
    DragModifiers::new(Some(DragModifier::Shift), None)
}

struct State {
    values: [[f64; 2]; 2],
    owner: usize,
    shown: bool,
    enabled: bool,
    read_only: bool,
    events: [Vec<EditEvent<f64>>; 2],
}
struct Rig {
    fixture: Fixture,
    state: Rc<RefCell<State>>,
}
impl Rig {
    fn new(fields: bool, policy: Option<DragModifiers>) -> Self {
        let state = Rc::new(RefCell::new(State {
            values: [[50.0; 2]; 2],
            owner: 0,
            shown: true,
            enabled: true,
            read_only: false,
            events: [Vec::new(), Vec::new()],
        }));
        let held = Rc::clone(&state);
        let mut fixture = Fixture::new(move |ui| {
            let mut state = held.borrow_mut();
            if !state.shown {
                return;
            }
            let owner = state.owner;
            ui.push_id(("scalar-model", owner), |ui| {
                let starts = state.values[owner];
                for (row, start) in starts.iter().copied().enumerate() {
                    let mut value = start;
                    let edit = if fields {
                        let field = SliderField::new(if row == 0 { "Amount" } else { "Balance" }, Bind::bound(&mut value), 0.0..=100.0)
                            .step(0.01)
                            .layout(SliderFieldLayout::Stacked)
                            .id_salt(row)
                            .enabled(state.enabled)
                            .read_only(state.read_only);
                        let field = if row == 0 {
                            match policy {
                                Some(policy) => field.pointer_modifiers(policy),
                                None => field,
                            }
                        } else {
                            field
                        };
                        field.show_at(ui, ROWS[row]).edit
                    } else {
                        let slider = Slider::new(Bind::bound(&mut value), 0.0..=100.0)
                            .step(0.01)
                            .id_salt(row)
                            .enabled(state.enabled)
                            .read_only(state.read_only);
                        let slider = if row == 0 {
                            match policy {
                                Some(policy) => slider.pointer_modifiers(policy),
                                None => slider,
                            }
                        } else {
                            slider
                        };
                        slider.show_at(ui, ROWS[row])
                    };
                    state.values[owner][row] = value;
                    state.events[row].extend(edit.events);
                }
            });
        });
        fixture.motion(MotionMode::Off).settle();
        Self { fixture, state }
    }
    fn thumb(&mut self, row: usize) -> Pos2 {
        let target = self.fixture.by_role(Role::SLIDER)[row];
        self.fixture.access(AccessActionKind::Focus, &target).settle();
        let id = self.fixture.focused_stop().expect("actual slider focus").0;
        assert_ne!(id, Id::NULL);
        self.fixture.part_rect(id, Slider::THUMB).expect("real sensed thumb").center()
    }
    fn drag(&mut self, row: usize, mods: Modifiers) -> f64 {
        let thumb = self.thumb(row);
        self.fixture.hold(mods).drag_at(thumb, Vec2::new(18.0, 0.0)).settle();
        self.state.borrow().values[0][row]
    }
}

#[trial]
fn scalar_contracts_pointer_shift_fine_alt_normal_are_exact_default_swaps() {
    for fields in [false, true] {
        let plain = Rig::new(fields, None).drag(0, Modifiers::NONE);
        let fine = Rig::new(fields, None).drag(0, Modifiers::ALT);
        let coarse = Rig::new(fields, None).drag(0, Modifiers::SHIFT);
        assert!(fine > 50.0 && fine < plain && plain < coarse);
        assert_eq!(Rig::new(fields, Some(fine_shift())).drag(0, Modifiers::NONE), plain);
        assert_eq!(Rig::new(fields, Some(fine_shift())).drag(0, Modifiers::SHIFT), fine);
        assert_eq!(Rig::new(fields, Some(fine_shift())).drag(0, Modifiers::ALT), plain);
        assert_eq!(Rig::new(fields, None).drag(0, Modifiers::ALT | Modifiers::SHIFT), fine);
        assert_eq!(Rig::new(fields, Some(fine_shift())).drag(0, Modifiers::ALT | Modifiers::SHIFT), fine);
    }
}

#[trial]
fn scalar_contracts_pointer_policy_never_leaks_to_other_row_or_keyboard() {
    for fields in [false, true] {
        let baseline = Rig::new(fields, None).drag(1, Modifiers::SHIFT);
        let mut rig = Rig::new(fields, Some(fine_shift()));
        assert_eq!(rig.drag(1, Modifiers::SHIFT), baseline);
        assert_eq!(rig.state.borrow().values[0][0], 50.0);
        for mods in [Modifiers::NONE, Modifiers::SHIFT, Modifiers::ALT] {
            let mut base = Rig::new(fields, None);
            let mut selected = Rig::new(fields, Some(fine_shift()));
            base.thumb(0);
            selected.thumb(0);
            base.fixture.key(Key::ArrowUp, mods).settle();
            selected.fixture.key(Key::ArrowUp, mods).settle();
            assert_eq!(base.state.borrow().values, selected.state.borrow().values);
        }
    }
}

#[trial]
fn scalar_contracts_pointer_disabled_read_only_and_absent_are_inert() {
    for fields in [false, true] {
        for mode in 0..3 {
            let mut rig = Rig::new(fields, Some(fine_shift()));
            let thumb = rig.thumb(0);
            match mode {
                0 => rig.state.borrow_mut().enabled = false,
                1 => rig.state.borrow_mut().read_only = true,
                _ => rig.state.borrow_mut().shown = false,
            }
            rig.fixture.settle().hold(Modifiers::SHIFT).drag_at(thumb, Vec2::new(18.0, 0.0)).settle();
            assert_eq!(rig.state.borrow().values, [[50.0; 2]; 2]);
            assert!(rig.state.borrow().events.iter().all(Vec::is_empty));
        }
    }
}

#[trial]
fn scalar_contracts_pointer_cancel_does_not_replay_on_release_or_reappearance() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(fine_shift()));
        let thumb = rig.thumb(0);
        let end = thumb + Vec2::new(18.0, 0.0);
        rig.fixture.hold(Modifiers::SHIFT).drag_hold_at(thumb, end - thumb);
        assert!(rig.state.borrow().values[0][0] > 50.0);
        cancel_all(rig.fixture.ctx(), CancelReason::Escape);
        rig.fixture.settle().release_at(end).settle();
        assert_eq!(rig.state.borrow().values, [[50.0; 2]; 2]);
        rig.state.borrow_mut().shown = false;
        rig.fixture.settle();
        rig.state.borrow_mut().shown = true;
        rig.fixture.settle();
        assert_eq!(rig.state.borrow().values, [[50.0; 2]; 2]);
    }
}

#[trial]
fn scalar_contracts_pointer_owner_switch_cannot_capture_or_replay_into_new_owner() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(fine_shift()));
        let thumb = rig.thumb(0);
        let end = thumb + Vec2::new(18.0, 0.0);
        rig.fixture.hold(Modifiers::SHIFT).drag_hold_at(thumb, end - thumb);
        rig.state.borrow_mut().owner = 1;
        rig.fixture.settle().release_at(end).settle();
        assert_eq!(rig.state.borrow().values[1], [50.0; 2]);
        rig.state.borrow_mut().owner = 0;
        rig.fixture.settle();
        let current = rig.state.borrow().values;
        rig.fixture.frame(vec![pointer::moved(end)]).settle();
        assert_eq!(rig.state.borrow().values, current);
    }
}
