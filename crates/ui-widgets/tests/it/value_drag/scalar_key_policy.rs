//! Opt-in scalar keys use real rows and the normal key stage; the native application baseline is separate.

use crate::overlay::kit::{Opened, Opener, Seen};
use craft_behavior::{EditEvent, MotionMode};
use craft_ui::contract::{Bind, Entry, Overlay, Props as _, Role, Show as _, Trigger};
use craft_ui::form::{SliderField, SliderFieldLayout};
use craft_ui::pickers::Select;
use craft_ui::runtime::focus::{FocusTarget, Initial, KeyUse, host_canvas, request};
use craft_ui::runtime::overlay::{OpenFilter, any_open};
use craft_ui::testing::focus::{AccessActionKind, key_release};
use craft_ui::testing::input::{key_press, key_repeat};
use craft_ui::testing::{Fixture, Frames, pointer, trial};
use craft_ui::value_drag::{ScalarKeyPolicy, ScalarKeyTrigger, Slider};
use egui::{Event, Id, ImeEvent, Key, Modifiers, Pos2, Rect, Ui, Vec2, ViewportId};
use std::cell::RefCell;
use std::rc::Rc;

const ROWS: [Rect; 2] =
    [Rect { min: Pos2::new(20.0, 20.0), max: Pos2::new(420.0, 80.0) }, Rect { min: Pos2::new(20.0, 100.0), max: Pos2::new(420.0, 160.0) }];
const HOST: Rect = Rect { min: Pos2::new(500.0, 20.0), max: Pos2::new(700.0, 75.0) };
const NATIVE_TEXT: Rect = Rect { min: Pos2::new(500.0, 100.0), max: Pos2::new(700.0, 135.0) };

fn policy() -> ScalarKeyPolicy {
    ScalarKeyPolicy::new(0.04).trigger(ScalarKeyTrigger::HoveredOrFocused)
}

fn close(left: f64, right: f64) {
    assert!((left - right).abs() < 1e-9, "expected {right}, got {left}");
}

fn modal_first() -> Overlay {
    let mut overlay = Overlay::modal();
    overlay.focus = overlay.focus.initial(Initial::First);
    overlay
}

struct Scene {
    values: [[f64; 2]; 2],
    selected: usize,
    fields: bool,
    policy: Option<ScalarKeyPolicy>,
    pointer_native: bool,
    pointer_model_step: Option<f64>,
    shown: [bool; 2],
    enabled: bool,
    read_only: bool,
    clip: Option<Rect>,
    ids: [Id; 2],
    events: [Vec<EditEvent<f64>>; 2],
    canvas: bool,
    canvas_id: Id,
    native_text: bool,
    text: String,
    raw_press: bool,
    raw_release: bool,
    overlay: u8,
    open: bool,
    combo: bool,
    pick: u8,
}

struct Rig {
    fixture: Fixture,
    scene: Rc<RefCell<Scene>>,
}

impl Rig {
    fn new(fields: bool, policy: Option<ScalarKeyPolicy>) -> Self {
        Self::with_initial(fields, policy, 0.01)
    }

    fn with_initial(fields: bool, policy: Option<ScalarKeyPolicy>, initial: f64) -> Self {
        let scene = Rc::new(RefCell::new(Scene {
            values: [[initial, 3.0], [-0.7, 17.0]],
            selected: 0,
            fields,
            policy,
            pointer_native: false,
            pointer_model_step: None,
            shown: [true; 2],
            enabled: true,
            read_only: false,
            clip: None,
            ids: [Id::NULL; 2],
            events: [Vec::new(), Vec::new()],
            canvas: false,
            canvas_id: Id::NULL,
            native_text: false,
            text: String::new(),
            raw_press: false,
            raw_release: false,
            overlay: 0,
            open: false,
            combo: false,
            pick: 1,
        }));
        let held = Rc::clone(&scene);
        let opened = Opened::default();
        let content = RefCell::new(Seen::default());
        let mut fixture = Fixture::new(move |ui| {
            let mut scene = held.borrow_mut();
            let original_clip = ui.clip_rect();
            if let Some(clip) = scene.clip {
                ui.set_clip_rect(original_clip.intersect(clip));
            }
            let selected = scene.selected;
            ui.push_id(("selected-model", selected), |ui| {
                let shows = scene.shown;
                for (row, shown) in shows.iter().copied().enumerate() {
                    if !shown {
                        continue;
                    }
                    let name = if row == 0 { "Exposure" } else { "Contrast" };
                    let (range, grid) = if row == 0 { (-4.0..=4.0, 0.01) } else { (-100.0..=100.0, 1.0) };
                    let mut value = scene.values[selected][row];
                    let row_policy = scene
                        .policy
                        .map(|policy| if row == 0 { policy } else { ScalarKeyPolicy::new(1.0).trigger(ScalarKeyTrigger::HoveredOrFocused) });
                    let output = if scene.fields {
                        let field = SliderField::new(name, Bind::bound(&mut value), range)
                            .step(grid)
                            .layout(SliderFieldLayout::Stacked)
                            .id_salt(("row", row))
                            .enabled(scene.enabled)
                            .read_only(scene.read_only);
                        let field = match row_policy {
                            Some(policy) => field.key_policy(policy),
                            None => field,
                        };
                        pointer_grid_field(field, scene.pointer_native, scene.pointer_model_step).show_at(ui, ROWS[row]).edit
                    } else {
                        let slider = Slider::new(Bind::bound(&mut value), range)
                            .step(grid)
                            .wheel_step(0.02)
                            .label(name)
                            .id_salt(("row", row))
                            .enabled(scene.enabled)
                            .read_only(scene.read_only);
                        let slider = match row_policy {
                            Some(policy) => slider.key_policy(policy),
                            None => slider,
                        };
                        pointer_grid_slider(slider, scene.pointer_native, scene.pointer_model_step).show_at(ui, ROWS[row])
                    };
                    scene.values[selected][row] = value;
                    scene.ids[row] = output.response.id;
                    scene.events[row].extend(output.events.iter().cloned());
                }
            });
            ui.set_clip_rect(original_clip);
            if scene.canvas {
                let response = ui.interact(HOST, ui.scope_id().with("native-canvas"), egui::Sense::click_and_drag());
                host_canvas(ui.ctx(), response.id, KeyUse::CANVAS);
                scene.canvas_id = response.id;
            }
            if scene.native_text {
                ui.put(NATIVE_TEXT, egui::TextEdit::singleline(&mut scene.text));
            }
            if scene.combo {
                let entries = [Entry::new(1u8, "One"), Entry::new(2u8, "Two")];
                Select::new("Pick", Bind::bound(&mut scene.pick), &entries).show_at(ui, HOST);
            }
            if scene.overlay != 0 {
                let make = if scene.overlay == 1 { Overlay::popover as fn() -> Overlay } else { modal_first };
                Opener::new(&opened, &content, Trigger::Manual, make).open(scene.open).id_salt("guard-opener").show_at(ui, HOST);
                scene.open = false;
            }
            ui.input(|input| {
                for event in &input.events {
                    if let Event::Key { key: Key::ArrowUp, pressed, .. } = event {
                        if *pressed {
                            scene.raw_press = true;
                        } else {
                            scene.raw_release = true;
                        }
                    }
                }
            });
        });
        fixture.motion(MotionMode::Off).settle();
        Self { fixture, scene }
    }

    fn focus_slider(&mut self, row: usize) {
        let target = self.fixture.by_role(Role::SLIDER)[row];
        self.fixture.access(AccessActionKind::Focus, &target).settle();
    }

    fn hover_label(&mut self, row: usize) {
        self.fixture.frame(vec![pointer::moved(ROWS[row].min + Vec2::new(8.0, 8.0))]).settle();
    }

    fn set(&mut self, value: f64) {
        let selected = self.scene.borrow().selected;
        self.scene.borrow_mut().values[selected][0] = value;
        self.fixture.settle();
    }
}

#[trial]
fn scalar_policy_none_keeps_original_vertical_shift_and_accessibility_steps() {
    let mut rig = Rig::new(false, None);
    rig.focus_slider(0);
    rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    close(rig.scene.borrow().values[0][0], 0.02);
    rig.set(0.01);
    rig.fixture.key(Key::ArrowUp, Modifiers::SHIFT).settle();
    close(rig.scene.borrow().values[0][0], 0.11);
    rig.set(0.01);
    let target = rig.fixture.by_role(Role::SLIDER)[0];
    rig.fixture.access(AccessActionKind::Increment, &target).settle();
    close(rig.scene.borrow().values[0][0], 0.02);
}

#[trial]
fn scalar_policy_adds_before_original_grid_snapping_in_both_directions() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(policy()));
        rig.focus_slider(0);
        for (key, mods, expected) in [
            (Key::ArrowUp, Modifiers::NONE, 0.05),
            (Key::ArrowUp, Modifiers::SHIFT, 0.21),
            (Key::ArrowDown, Modifiers::NONE, -0.03),
            (Key::ArrowDown, Modifiers::SHIFT, -0.19),
        ] {
            rig.set(0.01);
            rig.fixture.key(key, mods).settle();
            close(rig.scene.borrow().values[0][0], expected);
        }
    }
}

#[trial]
fn scalar_policy_whole_row_label_value_and_track_hover_use_one_model_nudge() {
    for at in [ROWS[0].min + Vec2::new(8.0, 8.0), Pos2::new(400.0, 30.0), Pos2::new(200.0, 70.0)] {
        let mut rig = Rig::new(true, Some(policy()));
        let before_focus = rig.fixture.focused_stop();
        rig.fixture.frame(vec![pointer::moved(at)]).key(Key::ArrowUp, Modifiers::NONE).settle();
        close(rig.scene.borrow().values[0][0], 0.05);
        close(rig.scene.borrow().values[0][1], 3.0);
        assert_eq!(rig.fixture.focused_stop(), before_focus, "hover never moves focus");
    }
}

#[trial]
fn scalar_policy_focused_row_wins_the_arrow_over_a_hovered_row() {
    for focused in [0, 1] {
        let mut rig = Rig::new(true, Some(policy()));
        rig.focus_slider(focused);
        let before_focus = rig.fixture.focused_stop();
        rig.hover_label(1 - focused);
        rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
        let values = rig.scene.borrow().values;
        let (edited, other) = if focused == 0 { (values[0][0], values[0][1]) } else { (values[0][1], values[0][0]) };
        assert!(edited > if focused == 0 { 0.01 } else { 3.0 }, "the focused row took the arrow: {values:?}");
        assert_eq!(other, if focused == 0 { 3.0 } else { 0.01 }, "the hovered row left it: {values:?}");
        assert_eq!(rig.fixture.focused_stop(), before_focus);
    }
}

#[trial]
fn scalar_policy_hovered_row_leaves_the_arrows_to_a_focused_list() {
    let mut rig = Rig::new(true, Some(policy()));
    rig.scene.borrow_mut().combo = true;
    rig.fixture.settle();
    let target = rig.fixture.by_role(Role::COMBO_BOX)[0];
    rig.fixture.access(AccessActionKind::Focus, &target).settle();
    let focused = rig.fixture.focused_stop();
    assert!(focused.is_some(), "the select holds the focus");
    rig.hover_label(0);
    let before = rig.scene.borrow().values;
    rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    assert_eq!(rig.scene.borrow().values, before, "a focused list that acts on the moves keeps Up from a hovered row");
    assert_eq!(rig.fixture.focused_stop(), focused);
}

#[trial]
fn scalar_policy_focused_trigger_does_not_let_hovered_rows_compete() {
    let mut rig = Rig::new(true, Some(ScalarKeyPolicy::new(0.04)));
    rig.focus_slider(1);
    rig.hover_label(0);
    rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    close(rig.scene.borrow().values[0][0], 0.01);
    close(rig.scene.borrow().values[0][1], 4.0);
}

#[trial]
fn scalar_policy_accessibility_increment_decrement_are_not_lost_as_horizontal_moves() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(policy()));
        let target = rig.fixture.by_role(Role::SLIDER)[0];
        close(rig.fixture.tree().numeric_of(target.node).expect("numeric slider").step.expect("step"), 0.04);
        rig.fixture.access(AccessActionKind::Increment, &target).settle();
        close(rig.scene.borrow().values[0][0], 0.05);
        rig.fixture.access(AccessActionKind::Decrement, &target).settle();
        close(rig.scene.borrow().values[0][0], 0.01);
    }
}

#[trial]
fn scalar_policy_numeric_child_accessibility_delegates_one_composite_edit() {
    let mut rig = Rig::new(true, Some(policy()));
    let target = rig.fixture.by_role(Role::SPIN_BUTTON)[0];
    close(rig.fixture.tree().numeric_of(target.node).expect("numeric child").step.expect("step"), 0.04);
    rig.fixture.access(AccessActionKind::Increment, &target).settle();
    close(rig.scene.borrow().values[0][0], 0.05);
    rig.fixture.access(AccessActionKind::Decrement, &target).settle();
    close(rig.scene.borrow().values[0][0], 0.01);
}

#[trial]
fn scalar_policy_pointer_wheel_and_horizontal_keys_keep_the_original_steps() {
    fn geometry(rig: &Rig, initial: f64) -> (Id, Rect, Pos2) {
        close(rig.scene.borrow().values[0][0], initial);
        let id = rig.scene.borrow().ids[0];
        let thumb = rig.fixture.part_rect(id, Slider::THUMB).expect("real thumb");
        let inset = thumb.width() / 2.0;
        let travel = ROWS[0].width() - 2.0 * inset;
        let expected_x = ROWS[0].left() + inset + ((initial + 4.0) / 8.0) as f32 * travel;
        assert!(
            (thumb.center().x - expected_x).abs() < 0.1,
            "first-draw real thumb must represent starting model {initial}: {thumb:?}, expected center {expected_x}"
        );
        let at = Pos2::new(ROWS[0].left() + inset + 0.51375 * travel, thumb.center().y);
        (id, thumb, at)
    }

    let cases = [("None", None), ("Some", Some(policy()))];
    let mut shared_at = None;
    let mut thumb_values = Vec::new();
    for (name, policy) in cases {
        // The original input is independently reproduced for BOTH policies before any track/key/wheel assertion.
        let mut rig = Rig::with_initial(false, policy, 0.01);
        let (id, thumb, at) = geometry(&rig, 0.01);
        if let Some(expected) = shared_at {
            assert_eq!(at, expected);
        } else {
            shared_at = Some(at);
        }
        assert!(thumb.contains(at), "{name}: original point really hits the thumb");
        rig.fixture.click_at(at).settle();
        let after = rig.scene.borrow().values[0][0];
        eprintln!("scalar fresh thumb {name}: initial=.01, thumb={thumb:?}, at={at:?}, owner={:?}, after={after}", rig.fixture.focused_stop());
        assert_eq!(rig.fixture.focused_stop().map(|(owner, _)| owner), Some(id));
        close(after, 0.01);
        thumb_values.push(after);
    }
    assert_eq!(thumb_values[0], thumb_values[1]);

    let mut track_values = Vec::new();
    for (name, policy) in cases {
        // -2 is installed BEFORE this rig's FIRST draw. No post-draw reset or reused thumb/click state.
        let mut rig = Rig::with_initial(false, policy, -2.0);
        let (id, thumb, at) = geometry(&rig, -2.0);
        assert_eq!(Some(at), shared_at, "same real pointer point under both starts and policies");
        assert!(!thumb.contains(at), "{name}: genuine track click cannot hit the real thumb");
        rig.fixture.click_at(at).settle();
        let after = rig.scene.borrow().values[0][0];
        eprintln!("scalar fresh track {name}: initial=-2, thumb={thumb:?}, at={at:?}, owner={:?}, after={after}", rig.fixture.focused_stop());
        assert_eq!(rig.fixture.focused_stop().map(|(owner, _)| owner), Some(id));
        close(after, 0.11);
        track_values.push(after);
    }
    assert_eq!(track_values[0], track_values[1]);

    let mut key_values = Vec::new();
    for (name, policy) in cases {
        let mut rig = Rig::with_initial(false, policy, 0.11);
        geometry(&rig, 0.11);
        rig.focus_slider(0);
        rig.fixture.key(Key::ArrowRight, Modifiers::NONE).settle();
        let after = rig.scene.borrow().values[0][0];
        eprintln!("scalar fresh horizontal {name}: initial=.11, after={after}");
        close(after, 0.12);
        key_values.push(after);
    }
    assert_eq!(key_values[0], key_values[1]);

    let mut wheel_values = Vec::new();
    for (name, policy) in cases {
        let mut rig = Rig::with_initial(false, policy, 0.12);
        let (id, thumb, _) = geometry(&rig, 0.12);
        rig.focus_slider(0);
        let focused = rig.fixture.focused_stop();
        assert_eq!(focused.map(|(owner, _)| owner), Some(id), "real focus makes the slider wheel-eligible");
        let at = thumb.center();
        rig.fixture.frame(vec![pointer::moved(at)]).frame(vec![pointer::wheel_lines(1.0)]).advance(1.0).settle();
        let after = rig.scene.borrow().values[0][0];
        eprintln!("scalar fresh wheel {name}: initial=.12, focused={focused:?}, at={at:?}, after={after}");
        close((after - 0.12).abs(), 0.02);
        wheel_values.push(after);
    }
    assert_eq!(wheel_values[0], wheel_values[1]);
}

#[trial]
fn scalar_policy_typed_values_keep_original_decimal_grid() {
    let mut rig = Rig::new(true, Some(policy()));
    let target = rig.fixture.by_role(Role::SPIN_BUTTON)[0];
    rig.fixture.click(&target).key(Key::A, Modifiers::COMMAND).type_text("0.13").key(Key::Enter, Modifiers::NONE).settle();
    close(rig.scene.borrow().values[0][0], 0.13);
}

#[trial]
fn scalar_policy_preserves_existing_numeric_editor_priority_in_both_row_orders() {
    for edited in [0, 1] {
        let mut plain = Rig::new(true, None);
        let mut opted = Rig::new(true, Some(policy()));
        for rig in [&mut plain, &mut opted] {
            let target = rig.fixture.by_role(Role::SPIN_BUTTON)[edited];
            rig.fixture.click(&target).key(Key::A, Modifiers::COMMAND).type_text(if edited == 0 { "0.13" } else { "7" }).settle();
            let before_focus = rig.fixture.focused_stop();
            let other = rig.scene.borrow().values[0][1 - edited];
            rig.hover_label(1 - edited);
            rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
            close(rig.scene.borrow().values[0][1 - edited], other);
            assert_eq!(rig.fixture.focused_stop(), before_focus);
        }
        assert_eq!(plain.scene.borrow().values, opted.scene.borrow().values, "the editor, not row-level hover, owns its key");
    }
}

#[trial]
fn scalar_policy_does_not_steal_a_real_native_text_editors_arrow() {
    let mut rig = Rig::new(true, Some(policy()));
    rig.scene.borrow_mut().native_text = true;
    rig.fixture.settle().click_at(NATIVE_TEXT.center()).type_text("native draft").settle();
    let values = rig.scene.borrow().values;
    rig.hover_label(0);
    rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    assert_eq!(rig.scene.borrow().values, values);
    assert_eq!(rig.scene.borrow().text, "native draft");
}

#[trial]
fn scalar_policy_ime_composition_keeps_hovered_other_row_out() {
    let mut rig = Rig::new(true, Some(policy()));
    let target = rig.fixture.by_role(Role::SPIN_BUTTON)[0];
    rig.fixture.click(&target).frame(vec![Event::Ime(ImeEvent::Preedit { text: "0".to_owned(), active_range_chars: Some(0..1) })]).settle();
    let other = rig.scene.borrow().values[0][1];
    rig.hover_label(1);
    rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    close(rig.scene.borrow().values[0][1], other);
}

#[trial]
fn scalar_policy_captured_drag_keeps_the_other_row_quiet_in_both_orders() {
    for dragged in [0, 1] {
        let mut rig = Rig::new(true, Some(policy()));
        rig.focus_slider(dragged);
        let id = rig.fixture.focused_stop().expect("actual slider focus").0;
        let thumb = rig.fixture.part_rect(id, Slider::THUMB).expect("real slider thumb").center();
        rig.fixture.frame(vec![pointer::moved(thumb)]).frame(vec![pointer::press(thumb)]);
        rig.fixture.frame(vec![pointer::moved(thumb + Vec2::new(12.0, 0.0))]);
        let over_other = ROWS[1 - dragged].min + Vec2::new(8.0, 8.0);
        rig.fixture.frame(vec![pointer::moved(over_other)]);
        let before = rig.scene.borrow().values;
        let other_events = rig.scene.borrow().events[1 - dragged].len();
        rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
        assert_eq!(rig.scene.borrow().values, before, "one captured gesture keeps its key priority");
        assert_eq!(rig.scene.borrow().events[1 - dragged].len(), other_events);
        rig.fixture.frame(vec![pointer::release(over_other)]).settle();
    }
}

#[trial]
fn scalar_policy_disabled_read_only_unmounted_and_clipped_rows_cannot_nudge() {
    for mode in 0..4 {
        let mut rig = Rig::new(true, Some(policy()));
        {
            let mut scene = rig.scene.borrow_mut();
            match mode {
                0 => scene.enabled = false,
                1 => scene.read_only = true,
                2 => scene.shown = [false; 2],
                _ => scene.clip = Some(Rect::from_min_max(Pos2::new(20.0, 50.0), Pos2::new(420.0, 80.0))),
            }
        }
        rig.fixture.settle();
        rig.hover_label(0);
        rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
        close(rig.scene.borrow().values[0][0], 0.01);
        close(rig.scene.borrow().values[0][1], 3.0);
    }
}

#[trial]
fn scalar_policy_model_switch_with_neutral_focus_reacquires_only_the_new_real_track() {
    let mut rig = Rig::new(true, Some(policy()));
    rig.focus_slider(0);
    rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    close(rig.scene.borrow().values[0][0], 0.05);
    // Keep genuine neutral host focus across the switch. Canonical SDK focus repair is not a new policy contract.
    rig.scene.borrow_mut().canvas = true;
    rig.fixture.settle();
    let host = rig.scene.borrow().canvas_id;
    request(rig.fixture.ctx(), FocusTarget::Id(host));
    rig.fixture.settle();
    rig.scene.borrow_mut().selected = 1;
    rig.fixture.frame(vec![Event::PointerGone]).settle();
    let before = rig.scene.borrow().values;
    rig.fixture.key(Key::ArrowUp, Modifiers::SHIFT).settle();
    assert_eq!(rig.scene.borrow().values, before, "a neutral focus and absent hover do not become a new mask row input");
    rig.focus_slider(0);
    rig.fixture.key(Key::ArrowUp, Modifiers::SHIFT).settle();
    close(rig.scene.borrow().values[1][0], -0.5);
    close(rig.scene.borrow().values[0][0], 0.05);
}

#[trial]
fn scalar_policy_hovered_row_with_native_canvas_focus_withdraws_press_and_release() {
    for same_batch in [false, true] {
        let mut rig = Rig::new(true, Some(policy()));
        rig.scene.borrow_mut().canvas = true;
        rig.fixture.settle();
        let id = rig.scene.borrow().canvas_id;
        request(rig.fixture.ctx(), FocusTarget::Id(id));
        rig.fixture.settle();
        let before_focus = rig.fixture.focused_stop();
        assert_eq!(before_focus.map(|(owner, _)| owner), Some(id));
        rig.hover_label(0);
        if same_batch {
            rig.fixture.frame(vec![key_press(Key::ArrowUp, Modifiers::NONE), key_release(Key::ArrowUp, Modifiers::NONE)]);
        } else {
            rig.fixture.frame(vec![key_press(Key::ArrowUp, Modifiers::NONE)]).frame(vec![key_release(Key::ArrowUp, Modifiers::NONE)]);
        }
        rig.fixture.settle();
        close(rig.scene.borrow().values[0][0], 0.05);
        assert!(!rig.scene.borrow().raw_press && !rig.scene.borrow().raw_release, "the native canvas sees neither half");
        assert_eq!(rig.fixture.focused_stop(), before_focus);
    }
}

#[trial]
fn scalar_policy_none_keeps_the_real_native_canvas_input_path() {
    let mut rig = Rig::new(true, None);
    rig.scene.borrow_mut().canvas = true;
    rig.fixture.settle();
    let id = rig.scene.borrow().canvas_id;
    request(rig.fixture.ctx(), FocusTarget::Id(id));
    rig.fixture.settle();
    rig.hover_label(0);
    rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    assert!(rig.scene.borrow().raw_press && rig.scene.borrow().raw_release);
    close(rig.scene.borrow().values[0][0], 0.01);
}

#[trial]
fn scalar_policy_repeat_and_mixed_shift_events_each_apply_once_in_order() {
    let mut rig = Rig::new(true, Some(policy()));
    rig.focus_slider(0);
    rig.fixture.frame(vec![key_press(Key::ArrowUp, Modifiers::NONE)]).frame(vec![key_repeat(Key::ArrowUp, Modifiers::NONE)]);
    rig.fixture.frame(vec![key_release(Key::ArrowUp, Modifiers::NONE)]).settle();
    close(rig.scene.borrow().values[0][0], 0.09);
    rig.set(0.01);
    rig.fixture
        .frame(vec![
            key_press(Key::ArrowUp, Modifiers::NONE),
            key_release(Key::ArrowUp, Modifiers::NONE),
            key_press(Key::ArrowUp, Modifiers::SHIFT),
            key_release(Key::ArrowUp, Modifiers::SHIFT),
        ])
        .settle();
    close(rig.scene.borrow().values[0][0], 0.25);
}

#[trial]
fn scalar_policy_popup_and_modal_keep_hovered_background_rows_quiet() {
    for mode in [1, 2] {
        let mut rig = Rig::new(true, Some(policy()));
        rig.scene.borrow_mut().overlay = mode;
        rig.scene.borrow_mut().open = true;
        rig.fixture.settle();
        assert!(any_open(rig.fixture.ctx(), OpenFilter::Keys), "real key-owning overlay is open");
        let before = rig.scene.borrow().values;
        rig.hover_label(0);
        rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
        assert_eq!(rig.scene.borrow().values, before);
    }
}

#[trial]
fn scalar_policy_invalid_parameters_keep_the_existing_slider_behavior() {
    for policy in [ScalarKeyPolicy::new(0.0), ScalarKeyPolicy::new(f64::NAN), ScalarKeyPolicy::new(0.04).shift_factor(f64::INFINITY)] {
        let mut rig = Rig::new(false, Some(policy));
        rig.focus_slider(0);
        rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
        close(rig.scene.borrow().values[0][0], 0.02);
    }
}

#[trial]
fn scalar_policy_interest_and_keys_are_viewport_local() {
    let mut frames = Frames::new();
    let first = RefCell::new(0.01);
    let second = RefCell::new(-0.7);
    let mut root = |ui: &mut Ui| {
        SliderField::new("Shared name", Bind::bound(&mut *first.borrow_mut()), -4.0..=4.0)
            .step(0.01)
            .key_policy(policy())
            .id_salt("same-row")
            .show_at(ui, ROWS[0]);
    };
    let mut child = |ui: &mut Ui| {
        SliderField::new("Shared name", Bind::bound(&mut *second.borrow_mut()), -4.0..=4.0)
            .step(0.01)
            .key_policy(policy())
            .id_salt("same-row")
            .show_at(ui, ROWS[0]);
    };
    let viewport = ViewportId::from_hash_of("scalar-child");
    for _ in 0..3 {
        frames.frame(vec![], &mut root);
        frames.frame_in_viewport(viewport, vec![], &mut child);
    }
    frames.frame_in_viewport(viewport, vec![pointer::moved(ROWS[0].min + Vec2::new(8.0, 8.0))], &mut child);
    frames.frame_in_viewport(viewport, vec![key_press(Key::ArrowUp, Modifiers::NONE)], &mut child);
    frames.frame_in_viewport(viewport, vec![key_release(Key::ArrowUp, Modifiers::NONE)], &mut child);
    close(*first.borrow(), 0.01);
    close(*second.borrow(), -0.66);
}

#[trial]
fn scalar_policy_each_key_emits_one_real_set_and_no_other_row_edit() {
    for fields in [false, true] {
        let mut rig = Rig::new(fields, Some(policy()));
        rig.focus_slider(0);
        rig.scene.borrow_mut().events = [Vec::new(), Vec::new()];
        rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
        let scene = rig.scene.borrow();
        let sets = scene.events[0].iter().filter(|event| matches!(event, EditEvent::Set { .. })).count();
        assert_eq!(sets, 1, "one setter for one key: {:?}", scene.events[0]);
        assert!(scene.events[1].is_empty(), "no second row edits: {:?}", scene.events[1]);
    }
}

#[trial]
fn scalar_policy_custom_curve_keeps_additive_model_units_and_model_bounds() {
    fn position(value: f64, min: f64, max: f64) -> f64 {
        ((value - min) / (max - min)).powi(2)
    }
    fn value_at(t: f64, min: f64, max: f64) -> f64 {
        min + t.sqrt() * (max - min)
    }
    for fields in [false, true] {
        let value = Rc::new(RefCell::new(0.01));
        let held = Rc::clone(&value);
        let mut fixture = Fixture::new(move |ui| {
            let mut value = held.borrow_mut();
            let scale = craft_ui::value_drag::SliderScale::custom(position, value_at);
            if fields {
                SliderField::new("Curve", Bind::bound(&mut *value), -4.0..=4.0).step(0.01).scale(scale).key_policy(policy()).show_at(ui, ROWS[0]);
            } else {
                Slider::new(Bind::bound(&mut *value), -4.0..=4.0).step(0.01).scale(scale).key_policy(policy()).show_at(ui, ROWS[0]);
            }
        });
        fixture.motion(MotionMode::Off).settle();
        let target = fixture.by_role(Role::SLIDER)[0];
        fixture.access(AccessActionKind::Focus, &target).key(Key::ArrowUp, Modifiers::NONE).settle();
        close(*value.borrow(), 0.05);
        *value.borrow_mut() = 3.99;
        fixture.settle().key(Key::ArrowUp, Modifiers::SHIFT).settle();
        close(*value.borrow(), 4.0);
    }
}

// Explicit model-grid producer: the correction changes only this builder, never any trial assertion or input.
fn model_grid_policy(nudge: f64, step: f64) -> ScalarKeyPolicy {
    ScalarKeyPolicy::new(nudge).trigger(ScalarKeyTrigger::HoveredOrFocused).model_step(step)
}

fn model_grid_signature(rig: &Rig) -> Vec<(u8, Option<u64>)> {
    rig.scene.borrow().events[0]
        .iter()
        .map(|event| {
            let kind = match event {
                EditEvent::Begin { .. } => 0,
                EditEvent::Update { .. } => 1,
                EditEvent::Commit { .. } => 2,
                EditEvent::Cancel { .. } => 3,
                EditEvent::Set { .. } => 4,
                _ => 5,
            };
            (kind, event.landed().map(|value| value.to_bits()))
        })
        .collect()
}

#[trial]
fn scalar_model_grid_exact_native_landings_are_not_near_comparisons() {
    for fields in [false, true] {
        for nudge in [0.04, (-3.96_f64) - (-4.0_f64)] {
            for (initial, key, mods, expected) in [
                (0.01, Key::ArrowUp, Modifiers::SHIFT, 0.21),
                (0.01, Key::ArrowUp, Modifiers::NONE, 0.05),
                (0.01, Key::ArrowDown, Modifiers::NONE, -0.03),
                (0.01, Key::ArrowDown, Modifiers::SHIFT, -0.19),
                (3.99, Key::ArrowUp, Modifiers::SHIFT, 4.0),
                (-3.99, Key::ArrowDown, Modifiers::SHIFT, -4.0),
                (0.012, Key::ArrowUp, Modifiers::NONE, 0.05),
            ] {
                let mut rig = Rig::with_initial(fields, Some(model_grid_policy(nudge, 0.01)), initial);
                rig.focus_slider(0);
                rig.scene.borrow_mut().events[0].clear();
                rig.fixture.key(key, mods).settle();
                let scene = rig.scene.borrow();
                assert_eq!(
                    scene.values[0][0], expected,
                    "native model-step landing; fields={fields}, nudge={nudge:?}, initial={initial}, mods={mods:?}"
                );
                assert_eq!(scene.values[0][1], 3.0);
                assert_eq!(scene.events[0].len(), 1, "one independent accepted key remains one Set");
                assert!(
                    matches!(&scene.events[0][0], EditEvent::Set { before: craft_behavior::Mixed::Same(before), after, .. } if *before == initial && *after == expected)
                );
            }
        }
    }
}

#[trial]
fn scalar_model_grid_default_none_retains_exact_track_grid_bits() {
    for fields in [false, true] {
        for nudge in [0.04, (-3.96_f64) - (-4.0_f64)] {
            let mut rig = Rig::new(fields, Some(ScalarKeyPolicy::new(nudge)));
            rig.focus_slider(0);
            rig.scene.borrow_mut().events[0].clear();
            rig.fixture.key(Key::ArrowUp, Modifiers::SHIFT).settle();
            let raw = 0.01 + nudge * 5.0;
            assert_ne!(raw, 0.21, "this is the deliberate unchanged default bit pattern");
            assert_eq!(rig.scene.borrow().values[0][0].to_bits(), raw.to_bits());
            assert_eq!(model_grid_signature(&rig), vec![(4, Some(raw.to_bits()))]);
        }
    }
}

#[trial]
fn scalar_model_grid_accessibility_uses_the_same_exact_opt_in() {
    for fields in [false, true] {
        for numeric_child in [false, true] {
            if numeric_child && !fields {
                continue;
            }
            let nudge = (-3.96_f64) - (-4.0_f64);
            let mut rig = Rig::new(fields, Some(model_grid_policy(nudge, 0.01)));
            let role = if numeric_child { Role::SPIN_BUTTON } else { Role::SLIDER };
            let target = rig.fixture.by_role(role)[0];
            rig.scene.borrow_mut().events[0].clear();
            rig.fixture.access(AccessActionKind::Increment, &target).settle();
            assert_eq!(rig.scene.borrow().values[0][0], 0.05);
            rig.fixture.access(AccessActionKind::Decrement, &target).settle();
            assert_eq!(rig.scene.borrow().values[0][0], 0.01);
            assert_eq!(model_grid_signature(&rig), vec![(4, Some(0.05_f64.to_bits())), (4, Some(0.01_f64.to_bits()))]);
        }
    }
}

#[trial]
fn scalar_model_grid_pointer_wheel_horizontal_and_typed_paths_are_bit_unchanged() {
    for fields in [false, true] {
        for action in ["pointer", "wheel", "horizontal", "typed"] {
            if action == "typed" && !fields {
                continue;
            }
            let mut observations = Vec::new();
            for opted in [false, true] {
                let selected = if opted { model_grid_policy(0.04, 0.01) } else { ScalarKeyPolicy::new(0.04) };
                let mut rig = Rig::new(fields, Some(selected));
                rig.focus_slider(0);
                let slider = rig.fixture.focused_stop().expect("real slider focus").0;
                let thumb = rig.fixture.part_rect(slider, Slider::THUMB).expect("real thumb").center();
                rig.scene.borrow_mut().events[0].clear();
                match action {
                    "pointer" => {
                        rig.fixture.hold(Modifiers::ALT).drag_at(thumb, Vec2::new(18.0, 0.0)).settle();
                    }
                    "wheel" => {
                        rig.fixture.frame(vec![pointer::moved(thumb)]).frame(vec![pointer::wheel_lines(1.0)]).advance(1.0).settle();
                    }
                    "horizontal" => {
                        rig.fixture.key(Key::ArrowRight, Modifiers::NONE).settle();
                    }
                    "typed" => {
                        let target = rig.fixture.by_role(Role::SPIN_BUTTON)[0];
                        rig.fixture.click(&target).key(Key::A, Modifiers::COMMAND).type_text("0.13").key(Key::Enter, Modifiers::NONE).settle();
                    }
                    _ => unreachable!(),
                }
                let values = rig.scene.borrow().values;
                observations.push((values, model_grid_signature(&rig)));
            }
            assert_eq!(observations[0], observations[1], "model-step must not alter {action}; fields={fields}");
        }
    }
}

#[trial]
fn scalar_model_grid_invalid_quantums_do_not_activate_and_extremes_stay_finite_and_bounded() {
    for step in [0.0, -0.01, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let selected = model_grid_policy(0.04, step);
        assert!(!selected.is_valid(), "invalid model step must make this optional policy inactive: {step}");
    }
    for fields in [false, true] {
        for step in [f64::from_bits(1), f64::MIN_POSITIVE, f64::MAX] {
            let selected = model_grid_policy(0.04, step);
            assert!(selected.is_valid());
            let mut rig = Rig::new(fields, Some(selected));
            rig.focus_slider(0);
            rig.fixture.key(Key::ArrowUp, Modifiers::SHIFT).settle();
            let value = rig.scene.borrow().values[0][0];
            assert!(
                value.is_finite() && (-4.0..=4.0).contains(&value),
                "valid quantization must stay finite and bounded: step={step}, value={value}"
            );
        }
    }
}

// Baseline producer: existing public Shift-fine binding, but no pointer model-grid option yet.
fn pointer_grid_slider(slider: Slider<'_>, native: bool, step: Option<f64>) -> Slider<'_> {
    let slider =
        if native { slider.pointer_modifiers(craft_behavior::DragModifiers::new(Some(craft_behavior::DragModifier::Shift), None)) } else { slider };
    match step {
        Some(step) => slider.pointer_model_step(step),
        None => slider,
    }
}

fn pointer_grid_field(field: SliderField<'_>, native: bool, step: Option<f64>) -> SliderField<'_> {
    let field =
        if native { field.pointer_modifiers(craft_behavior::DragModifiers::new(Some(craft_behavior::DragModifier::Shift), None)) } else { field };
    match step {
        Some(step) => field.pointer_model_step(step),
        None => field,
    }
}

fn pointer_grid_rig(fields: bool, step: Option<f64>) -> Rig {
    let mut rig = Rig::new(fields, Some(model_grid_policy(0.04, 0.01)));
    rig.scene.borrow_mut().pointer_native = true;
    rig.scene.borrow_mut().pointer_model_step = step;
    rig.fixture.settle();
    rig
}

// The public AX Focus action addresses the observed real slider; its resulting id only locates its real thumb.
// No id, focus state or model value is manufactured to make a drag pass.
fn pointer_grid_path(rig: &mut Rig, fraction: f32) -> (Pos2, Pos2) {
    rig.focus_slider(0);
    let slider = rig.fixture.by_role(Role::SLIDER)[0];
    let owner = rig.fixture.ctx().memory(|memory| memory.focused()).expect("real focused slider");
    let thumb = rig.fixture.part_rect(owner, Slider::THUMB).expect("real thumb part");
    let travel = slider.rect.width() - thumb.width();
    assert!(travel > 0.0);
    (thumb.center(), thumb.center() + Vec2::new(travel * fraction, 0.0))
}

fn pointer_grid_hold(rig: &mut Rig, from: Pos2, to: Pos2, mods: Modifiers) -> f64 {
    rig.fixture.hold(mods).frame(vec![pointer::moved(from)]).frame(vec![pointer::press(from)]);
    for frame in 1..=4 {
        rig.fixture.frame(vec![pointer::moved(from + (to - from) * (frame as f32 / 4.0))]);
    }
    rig.fixture.settle();
    rig.scene.borrow().values[0][0]
}

fn pointer_grid_release(rig: &mut Rig, at: Pos2) -> f64 {
    rig.fixture.frame(vec![pointer::release(at)]).settle();
    rig.scene.borrow().values[0][0]
}

#[trial]
fn pointer_model_grid_exact_native_thumb_held_and_release_plain_shift_alt_both_widgets_and_signs() {
    for fields in [false, true] {
        for sign in [1.0_f64, -1.0] {
            for (mods, expected) in [(Modifiers::NONE, 0.17), (Modifiers::SHIFT, 0.03), (Modifiers::ALT, 0.17)] {
                let mut rig = pointer_grid_rig(fields, Some(0.01));
                rig.set(sign * 0.01);
                let (from, to) = pointer_grid_path(&mut rig, sign as f32 * 0.02);
                rig.scene.borrow_mut().events[0].clear();
                assert_eq!(
                    pointer_grid_hold(&mut rig, from, to, mods),
                    sign * expected,
                    "exact native held; fields={fields}, sign={sign}, mods={mods:?}"
                );
                assert_eq!(pointer_grid_release(&mut rig, to), sign * expected, "exact native release");
                let scene = rig.scene.borrow();
                assert_eq!(scene.values[0][1], 3.0);
                assert_eq!(scene.values[1], [-0.7, 17.0]);
                assert_eq!(scene.events[0].iter().filter(|event| matches!(event, EditEvent::Commit { .. })).count(), 1);
                assert_eq!(scene.events[0].iter().filter(|event| matches!(event, EditEvent::Begin { .. })).count(), 1);
                assert!(!scene.events[0].iter().any(|event| matches!(event, EditEvent::Set { .. })));
            }
        }
    }
}

#[trial]
fn pointer_model_grid_default_none_keeps_original_bits_even_with_key_model_step_selected() {
    for fields in [false, true] {
        let mut rig = pointer_grid_rig(fields, None);
        let (from, to) = pointer_grid_path(&mut rig, 0.02);
        let expected = 0.16999999999999993_f64;
        assert_eq!(pointer_grid_hold(&mut rig, from, to, Modifiers::NONE).to_bits(), expected.to_bits());
        assert_eq!(pointer_grid_release(&mut rig, to).to_bits(), expected.to_bits());
    }
}

#[trial]
fn pointer_model_grid_independent_drags_keep_distinct_gestures_exact_originals_and_complete_other_values() {
    for fields in [false, true] {
        let mut rig = pointer_grid_rig(fields, Some(0.01));
        for expected in [0.17_f64, 0.33] {
            let (from, to) = pointer_grid_path(&mut rig, 0.02);
            assert_eq!(pointer_grid_hold(&mut rig, from, to, Modifiers::NONE), expected);
            assert_eq!(pointer_grid_release(&mut rig, to), expected);
        }
        let scene = rig.scene.borrow();
        let commits: Vec<_> = scene.events[0]
            .iter()
            .filter_map(|event| {
                if let EditEvent::Commit { id, before: craft_behavior::Mixed::Same(before), after, .. } = event {
                    Some((*id, *before, *after))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(commits.len(), 2);
        assert_ne!(commits[0].0, commits[1].0, "separate physical drags are independent Undo candidates");
        assert_eq!((commits[0].1, commits[0].2), (0.01, 0.17));
        assert_eq!((commits[1].1, commits[1].2), (0.17, 0.33));
        assert_eq!(scene.values, [[0.33, 3.0], [-0.7, 17.0]]);
        assert!(scene.events[1].is_empty());
    }
}

#[trial]
fn pointer_model_grid_track_click_uses_the_same_exact_model_grid_without_an_atomic_operation() {
    for fields in [false, true] {
        let mut rig = pointer_grid_rig(fields, Some(0.01));
        let (from, _) = pointer_grid_path(&mut rig, 0.0);
        let target = rig.fixture.by_role(Role::SLIDER)[0];
        let owner = rig.fixture.ctx().memory(|memory| memory.focused()).expect("real slider focus");
        let thumb = rig.fixture.part_rect(owner, Slider::THUMB).expect("real thumb");
        let at = Pos2::new(target.rect.left() + thumb.width() * 0.5 + (target.rect.width() - thumb.width()) * 0.65, from.y);
        rig.scene.borrow_mut().events[0].clear();
        rig.fixture.frame(vec![pointer::moved(at)]).frame(vec![pointer::press(at)]).frame(vec![pointer::release(at)]).settle();
        let scene = rig.scene.borrow();
        assert_eq!(scene.values[0], [1.2, 3.0]);
        assert_eq!(scene.events[0].len(), 1);
        assert!(
            matches!(&scene.events[0][0], EditEvent::Set { before: craft_behavior::Mixed::Same(before), after, .. } if *before == 0.01 && *after == 1.2)
        );
    }
}

#[trial]
fn pointer_model_grid_non_pointer_keys_wheel_accessibility_and_typing_are_bit_and_event_unchanged() {
    for fields in [false, true] {
        for action in ["scalar", "horizontal", "wheel", "accessibility", "typed"] {
            if action == "typed" && !fields {
                continue;
            }
            let mut observations = Vec::new();
            for step in [None, Some(0.01)] {
                let mut rig = pointer_grid_rig(fields, step);
                let (thumb, _) = pointer_grid_path(&mut rig, 0.0);
                rig.scene.borrow_mut().events[0].clear();
                match action {
                    "scalar" => {
                        rig.fixture.key(Key::ArrowUp, Modifiers::SHIFT).settle();
                    }
                    "horizontal" => {
                        rig.fixture.key(Key::ArrowRight, Modifiers::NONE).settle();
                    }
                    "wheel" => {
                        rig.fixture.frame(vec![pointer::moved(thumb)]).frame(vec![pointer::wheel_lines(1.0)]).advance(1.0).settle();
                    }
                    "accessibility" => {
                        let target = rig.fixture.by_role(Role::SLIDER)[0];
                        rig.fixture.access(AccessActionKind::Increment, &target).settle();
                    }
                    "typed" => {
                        let target = rig.fixture.by_role(Role::SPIN_BUTTON)[0];
                        rig.fixture.click(&target).key(Key::A, Modifiers::COMMAND).type_text("0.13").key(Key::Enter, Modifiers::NONE).settle();
                    }
                    _ => unreachable!(),
                }
                observations.push((rig.scene.borrow().values.map(|model| model.map(f64::to_bits)), model_grid_signature(&rig)));
            }
            assert_eq!(observations[0], observations[1], "pointer option cannot alter {action}; fields={fields}");
        }
    }
}

#[trial]
fn pointer_model_grid_escape_and_disabled_readonly_do_not_commit_or_replay_a_quantized_proposal() {
    for fields in [false, true] {
        let mut rig = pointer_grid_rig(fields, Some(0.01));
        let original = rig.scene.borrow().values;
        let (from, to) = pointer_grid_path(&mut rig, 0.02);
        assert_eq!(pointer_grid_hold(&mut rig, from, to, Modifiers::NONE), 0.17);
        rig.fixture.key(Key::Escape, Modifiers::NONE).settle();
        assert_eq!(rig.scene.borrow().values, original);
        pointer_grid_release(&mut rig, to);
        assert_eq!(rig.scene.borrow().values, original);
        assert_eq!(rig.scene.borrow().events[0].iter().filter(|event| matches!(event, EditEvent::Cancel { .. })).count(), 1);
        assert!(!rig.scene.borrow().events[0].iter().any(|event| matches!(event, EditEvent::Commit { .. })));
        for read_only in [false, true] {
            let mut blocked = pointer_grid_rig(fields, Some(0.01));
            let before = blocked.scene.borrow().values;
            let (from, to) = pointer_grid_path(&mut blocked, 0.02);
            if read_only {
                blocked.scene.borrow_mut().read_only = true;
            } else {
                blocked.scene.borrow_mut().enabled = false;
            }
            blocked.fixture.settle();
            pointer_grid_hold(&mut blocked, from, to, Modifiers::NONE);
            pointer_grid_release(&mut blocked, to);
            assert_eq!(blocked.scene.borrow().values, before);
            assert!(blocked.scene.borrow().events.iter().all(Vec::is_empty));
        }
    }
}

#[trial]
fn pointer_model_grid_invalid_quantums_keep_default_and_valid_extremes_are_finite_and_bounded() {
    for fields in [false, true] {
        for step in [0.0, -0.01, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut rig = pointer_grid_rig(fields, Some(step));
            let (from, to) = pointer_grid_path(&mut rig, 0.02);
            let expected = 0.16999999999999993_f64;
            assert_eq!(pointer_grid_hold(&mut rig, from, to, Modifiers::NONE).to_bits(), expected.to_bits());
            assert_eq!(pointer_grid_release(&mut rig, to).to_bits(), expected.to_bits());
        }
        for step in [f64::MIN_POSITIVE, f64::from_bits(1), f64::MAX, 0.01] {
            for initial in [-3.99_f64, 0.01, 3.99] {
                let mut rig = pointer_grid_rig(fields, Some(step));
                rig.set(initial);
                let (from, to) = pointer_grid_path(&mut rig, if initial < 0.0 { -0.02 } else { 0.02 });
                let held = pointer_grid_hold(&mut rig, from, to, Modifiers::NONE);
                let released = pointer_grid_release(&mut rig, to);
                assert!(held.is_finite() && (-4.0..=4.0).contains(&held));
                assert!(released.is_finite() && (-4.0..=4.0).contains(&released));
            }
        }
    }
}
