//! The Brush Preset picker: Rename Brush… in the gear opens the browser's name field on the current preset; the app
//! opens the picker at a point without its field; a resized picker keeps its size; the head edits angle and
//! roundness; Escape reverts the head's edits.

use craft_draw::ui::brush_presets::{BrushPick, BrushPickerState, BrushPickerValues, BrushPresetPicker, BrushRowPaint, brush_row};
use craft_draw::ui::brush_settings::TipAngle;
use craft_ui::contract::{Bind, Role, SaveKey};
use craft_ui::patterns::presets::PresetEvent;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Pos2, Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const HARD: u32 = 2;

struct Host {
    current: Option<u32>,
    state: BrushPickerState<u32>,
    picks: Vec<BrushPick<u32>>,
}

#[trial]
fn rename_brush_opens_the_name_field_on_the_current_preset() {
    let host = Rc::new(RefCell::new(Host { current: Some(HARD), state: BrushPickerState::new(), picks: Vec::new() }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let env = Env::of(ui);
        let paint = BrushRowPaint::new(|_, _| {}, |_, _| {});
        let mut host = shown.borrow_mut();
        let Host { current, state, picks } = &mut *host;
        let rows =
            vec![brush_row(1, "Soft Round", 0, 45.0, state.parts, &env, &paint), brush_row(HARD, "Hard Round", 0, 12.0, state.parts, &env, &paint)];
        let values = BrushPickerValues::new(12.0, Some(1.0));
        let out = BrushPresetPicker::new("Brush Preset picker", Bind::bound(current), &rows, Bind::bound(state), values).show(ui);
        picks.extend(out.picks);
    });
    fixture.room(Vec2::new(480.0, 720.0));
    fixture.settle();

    let field = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the closed field shows");
    fixture.click(&field);
    fixture.idle(3);
    let gear = fixture.by_label("Brush Preset Options").expect("the gear sits in the search row");
    fixture.click(&gear);
    fixture.idle(3);
    let rename = fixture.by_label("Rename Brush…").expect("the gear menu shows");
    fixture.click(&rename);
    fixture.idle(4);

    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text("Hard Square");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(3);
    let host = host.borrow();
    let renamed = host.picks.iter().any(|pick| matches!(pick, BrushPick::List(PresetEvent::Rename { key: HARD, text, .. }) if text == "Hard Square"));
    assert!(renamed, "the typed name comes back as the browser's rename of the current preset: {:?}", host.picks);
}

/// The app's side of a picker it opens itself: the brush values it applies, the open state and what came back.
struct Opened {
    current: Option<u32>,
    state: BrushPickerState<u32>,
    values: BrushPickerValues,
    open: bool,
    tip_edits: usize,
    /// Where the app opens the picker without its field; `None` shows the field.
    at: Option<Pos2>,
    /// The space above the field.
    offset: f32,
}

const POINT: Pos2 = Pos2::new(160.0, 120.0);
const OPENED: BrushPickerValues = BrushPickerValues::new(12.0, Some(0.5)).with_tip(TipAngle::new(0.0, 1.0));

/// A picker over two presets showing `opened`'s values, opened by the app at [`POINT`] when `at_point`, else from its
/// field; with `remember`, resizable with its size saved. The host applies every edit and Escape's revert.
fn opened_rig(at_point: bool, remember: bool) -> (Fixture, Rc<RefCell<Opened>>) {
    let at = at_point.then_some(POINT);
    let host = Rc::new(RefCell::new(Opened {
        current: Some(HARD),
        state: BrushPickerState::new(),
        values: OPENED,
        open: at_point,
        tip_edits: 0,
        at,
        offset: 0.0,
    }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let env = Env::of(ui);
        let paint = BrushRowPaint::new(|_, _| {}, |_, _| {});
        let mut host = shown.borrow_mut();
        let Opened { current, state, values, open, tip_edits, at, offset } = &mut *host;
        ui.add_space(*offset);
        let rows =
            vec![brush_row(1, "Soft Round", 0, 45.0, state.parts, &env, &paint), brush_row(HARD, "Hard Round", 0, 12.0, state.parts, &env, &paint)];
        let mut picker = BrushPresetPicker::new("Brush Preset picker", Bind::bound(current), &rows, Bind::bound(state), *values);
        if remember {
            picker = picker.resizable(SaveKey::new("craft_draw_tests.brush_picker"));
        }
        if let Some(at) = *at {
            picker = picker.open_at(at, Bind::bound(open));
        }
        let out = picker.show(ui);
        if let Some(diameter) = out.diameter.landed() {
            values.diameter = *diameter;
        }
        if let Some(hardness) = out.hardness.landed() {
            values.hardness = Some(*hardness);
        }
        if let Some(tip) = out.tip.landed() {
            values.tip = Some(*tip);
        }
        *tip_edits += out.tip.len();
        if let Some(back) = out.reverted {
            *values = back;
        }
    });
    fixture.room(Vec2::new(640.0, 760.0));
    fixture.settle();
    (fixture, host)
}

fn popover(fixture: &Fixture) -> Rect {
    fixture.by_role(Role::DIALOG).first().map(|target| target.rect).expect("the popover shows")
}

#[trial]
fn the_app_opens_the_picker_at_a_point_without_its_field() {
    let (mut fixture, host) = opened_rig(true, false);
    let field = fixture.by_role(Role::COMBO_BOX);
    assert!(field.is_empty(), "no field runs: {field:?}");
    let rect = popover(&fixture);
    assert!((rect.min - POINT).length() < 32.0, "the popover hangs from the point: {rect:?}");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.idle(3);
    assert!(!host.borrow().open, "Escape writes the closed state back");
    assert!(fixture.by_role(Role::DIALOG).is_empty(), "the popover is gone");
}

/// A resized popover keeps only its size: each opening places it from its field or point at that size. Its grips
/// stand only on the edges its anchor leaves free.
#[trial]
fn a_resized_picker_reopens_at_the_size_its_grip_left() {
    let (mut fixture, host) = opened_rig(false, true);
    let field = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the closed field shows");
    fixture.click(&field);
    fixture.idle(3);
    let first = popover(&fixture);
    fixture.drag_at(first.left_center() + Vec2::new(1.0, 0.0), Vec2::new(-60.0, 0.0));
    fixture.idle(3);
    let held = popover(&fixture);
    assert!((held.width() - first.width()).abs() < 1.0, "the edge at the field's start has no grip: {first:?} -> {held:?}");
    fixture.drag_at(first.right_bottom() - Vec2::splat(1.0), Vec2::new(60.0, 80.0));
    fixture.idle(3);
    let resized = popover(&fixture);
    assert!(resized.width() > first.width() + 30.0 && resized.height() > first.height() + 40.0, "the corner grip resizes: {first:?} -> {resized:?}");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.idle(3);
    assert!(fixture.by_role(Role::DIALOG).is_empty(), "Escape closes the popover");

    host.borrow_mut().offset = 120.0;
    fixture.idle(3);
    let moved = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the field shows");
    assert!(moved.rect.min.y > field.rect.min.y + 100.0, "the field moved down: {:?} -> {:?}", field.rect, moved.rect);
    fixture.click(&moved);
    fixture.idle(3);
    let reopened = popover(&fixture);
    assert!((reopened.size() - resized.size()).abs().max_elem() < 1.0, "the reopened popover keeps the size: {resized:?} -> {reopened:?}");
    let under = reopened.min.y - moved.rect.max.y;
    assert!(
        (-1.0..16.0).contains(&under) && (reopened.min.x - moved.rect.min.x).abs() < 16.0,
        "it hangs under the moved field: {:?} / {reopened:?}",
        moved.rect
    );
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.idle(3);

    let point = Pos2::new(220.0, 60.0);
    {
        let mut host = host.borrow_mut();
        host.at = Some(point);
        host.open = true;
    }
    fixture.idle(3);
    let at_point = popover(&fixture);
    assert!((at_point.size() - resized.size()).abs().max_elem() < 1.0, "opened at a point it keeps the size: {resized:?} -> {at_point:?}");
    assert!((at_point.min - point).length() < 32.0, "it hangs from the new point: {at_point:?}");
}

#[trial]
fn the_head_shows_angle_and_roundness_beside_size() {
    let (mut fixture, host) = opened_rig(true, false);
    let tip = fixture.by_label("Tip angle and roundness").expect("the angle widget shows in the head");
    let size = fixture.by_label("Size").expect("the Size field shows");
    assert!(tip.rect.max.x <= size.rect.min.x + 1.0, "the angle widget comes first: {:?} / {:?}", tip.rect, size.rect);
    fixture.click_at(tip.rect.center() + Vec2::new(tip.rect.width() / 4.0, -tip.rect.height() / 4.0));
    fixture.idle(3);
    let host = host.borrow();
    assert!(host.tip_edits > 0, "the widget edits the tip");
    assert!(host.values.tip.is_some_and(|tip| tip.degrees != 0.0), "a press off the roundness dots turns the angle: {:?}", host.values.tip);
}

#[trial]
fn escape_reverts_size_hardness_angle_and_roundness() {
    let (mut fixture, host) = opened_rig(true, false);
    let tip = fixture.by_label("Tip angle and roundness").expect("the angle widget shows");
    fixture.click_at(tip.rect.center() + Vec2::new(tip.rect.width() / 4.0, -tip.rect.height() / 4.0));
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    for label in ["Size", "Hardness"] {
        let slider = fixture.by_role(Role::SLIDER).into_iter().find(|target| fixture.tree().name_of(target.node) == Some(label));
        let slider = slider.unwrap_or_else(|| panic!("the {label} slider shows"));
        fixture.click_at(slider.rect.left_center() + Vec2::new(slider.rect.width() * 0.8, 0.0));
    }
    fixture.idle(3);
    let edited = host.borrow().values;
    let tip_now = edited.tip.expect("the tip stays");
    assert!(edited.diameter != OPENED.diameter && edited.hardness != OPENED.hardness, "Size and Hardness changed: {edited:?}");
    assert!(tip_now.degrees != 0.0 && tip_now.roundness < 1.0, "angle and roundness changed: {edited:?}");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.idle(3);
    assert_eq!(host.borrow().values, OPENED, "Escape puts all four back");
}
