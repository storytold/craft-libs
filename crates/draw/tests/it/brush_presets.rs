//! The Brush Preset picker: Rename Brush… in the gear opens the browser's name field on the current preset; the app
//! opens the picker at a point without its field; a resized picker keeps its size; the head edits angle and
//! roundness; Escape reverts the head's edits.

use craft_draw::ui::brush_presets::{
    BrushCardPart, BrushCardParts, BrushPick, BrushPickerSettings, BrushPickerState, BrushPickerValues, BrushPresetPicker, BrushRowPaint, brush_row,
};
use craft_draw::ui::brush_settings::TipAngle;
use craft_ui::contract::{Bind, Role, SaveKey};
use craft_ui::patterns::presets::{PresetEvent, PresetFns, PresetRow, PresetSource};
use craft_ui::primitives::FacePaint;
use craft_ui::runtime::env::Env;
use craft_ui::testing::pointer::{moved, press, release, secondary, wheel};
use craft_ui::testing::{Fixture, MemoRun, last_frame_memo_runs, trial};
use egui::{Key, Modifiers, Pos2, Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const HARD: u32 = 2;

/// Two presets, Soft Round and Hard Round, showing `parts`; their tip and stroke paint nothing, each face with a key of
/// its own.
fn two_rows(parts: &BrushCardParts, env: &Env) -> Vec<PresetRow<u32>> {
    let paint = BrushRowPaint::new(FacePaint::new(1, |_, _| {}), FacePaint::new(2, |_, _| {}));
    vec![brush_row(1, "Soft Round", 0, 45.0, *parts, env, &paint), brush_row(HARD, "Hard Round", 0, 12.0, *parts, env, &paint)]
}

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
        let mut host = shown.borrow_mut();
        let Host { current, state, picks } = &mut *host;
        let rows = PresetSource::new(state.parts, PresetFns::new(two_rows));
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
        let mut host = shown.borrow_mut();
        let Opened { current, state, values, open, tip_edits, at, offset } = &mut *host;
        ui.add_space(*offset);
        let rows = PresetSource::new(state.parts, PresetFns::new(two_rows));
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

/// A resized popover keeps only its size: each opening places it from its field or point at that size. Every edge
/// has a grip, the one at the field's start too: dragged, that edge moves out while the popover stays open.
#[trial]
fn a_resized_picker_reopens_at_the_size_its_grip_left() {
    let (mut fixture, host) = opened_rig(false, true);
    let field = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the closed field shows");
    fixture.click(&field);
    fixture.idle(3);
    let first = popover(&fixture);
    fixture.drag_at(first.left_center() + Vec2::new(1.0, 0.0), Vec2::new(-60.0, 0.0));
    fixture.idle(3);
    let widened = popover(&fixture);
    assert!(
        widened.width() > first.width() + 30.0 && widened.left() < first.left() - 30.0 && (widened.right() - first.right()).abs() < 1.0,
        "the edge at the field's start moves out: {first:?} -> {widened:?}"
    );
    fixture.drag_at(widened.right_bottom() - Vec2::splat(1.0), Vec2::new(60.0, 80.0));
    fixture.idle(3);
    let resized = popover(&fixture);
    assert!(
        resized.width() > widened.width() + 30.0 && resized.height() > widened.height() + 40.0,
        "the corner grip resizes: {widened:?} -> {resized:?}"
    );
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

/// The memoized views of `props` that ran in the last frame.
fn views_ran(fixture: &Fixture, props: &str) -> usize {
    last_frame_memo_runs(fixture.ctx()).into_iter().filter(|(name, _, run)| name.contains(props) && *run == MemoRun::Ran).count()
}

/// The memoized views of `props` that replayed in the last frame.
fn views_replayed(fixture: &Fixture, props: &str) -> usize {
    last_frame_memo_runs(fixture.ctx()).into_iter().filter(|(name, _, run)| name.contains(props) && *run == MemoRun::Replayed).count()
}

/// While nothing changes the open picker's head and preset list replay; each move of a Size drag runs the head's view
/// and replays the list.
#[trial]
fn a_size_drag_runs_the_head_and_replays_the_preset_list() {
    let (mut fixture, host) = opened_rig(true, false);
    fixture.frame(vec![]);
    assert_eq!((views_ran(&fixture, "HeadProps"), views_ran(&fixture, "ListProps")), (0, 0), "an idle frame runs neither view");
    let slider = fixture.by_role(Role::SLIDER).into_iter().find(|target| fixture.tree().name_of(target.node) == Some("Size"));
    let slider = slider.expect("the Size slider shows").rect;
    let from = slider.left_center() + Vec2::new(slider.width() * 0.3, 0.0);
    fixture.frame(vec![moved(from)]);
    fixture.frame(vec![press(from)]);
    for step in 1..=4 {
        fixture.frame(vec![moved(from + Vec2::new(8.0 * step as f32, 0.0))]);
        assert!(views_ran(&fixture, "HeadProps") > 0, "move {step} runs the head");
        assert_eq!(views_ran(&fixture, "ListProps"), 0, "move {step} runs no list view");
        assert!(views_replayed(&fixture, "ListProps") > 0, "move {step} replays the list");
    }
    fixture.frame(vec![release(from + Vec2::new(32.0, 0.0))]);
    assert_ne!(host.borrow().values.diameter, OPENED.diameter, "the drag edited the size");
}

/// The tip alone is a grid of bare tips: each tile square, no caption under it, the tiles side by side with no room
/// between them, and the name and size kept for the tooltip and the accessible name.
#[trial]
fn the_tip_grid_packs_square_tiles_without_captions() {
    let (mut fixture, host) = opened_rig(true, false);
    host.borrow_mut().state.parts = BrushCardParts { name: false, stroke: false, tip: true };
    fixture.settle();
    let tiles: Vec<Rect> = fixture
        .by_role(Role::LIST_ITEM)
        .into_iter()
        .filter(|target| fixture.tree().name_of(target.node).is_some_and(|name| name.contains("Round")))
        .map(|target| target.rect)
        .collect();
    assert_eq!(tiles.len(), 2, "both presets show as tiles: {tiles:?}");
    for tile in &tiles {
        assert!((tile.width() - tile.height()).abs() <= 2.0, "a square tile: {tile:?}");
    }
    let (a, b) = (tiles[0], tiles[1]);
    assert!((a.min.y - b.min.y).abs() < 1.0, "side by side: {a:?} {b:?}");
    assert!(b.min.x - a.max.x <= 2.0, "no room between them: {a:?} {b:?}");
    let painted: Vec<String> = fixture.texts().iter().map(|text| text.text.clone()).collect();
    assert!(!painted.iter().any(|text| text.contains("Soft Round") || text.contains("45 px")), "no caption is painted: {painted:?}");
}

/// Below a scale the presets show as the grid of tips whatever parts are chosen; above it, the chosen parts. The saved
/// parts stay the chosen ones.
#[trial]
fn a_small_scale_shows_the_grid_of_tips_and_a_larger_one_the_chosen_parts() {
    let mut state = BrushPickerState::<u32>::new();
    state.parts = BrushCardParts::ALL.set(BrushCardPart::Tip, false);
    state.browser.zoom = 1.0;
    assert_eq!(state.shown_parts(), state.parts, "the chosen parts at scale 1");
    state.browser.zoom = f64::from(*BrushPickerSettings::SCALE_RANGE.start());
    assert!(state.shown_parts().tip_only(), "the grid of tips at the smallest scale");
    assert_eq!(state.settings().parts, state.parts, "the saved parts stay the chosen ones");
}

/// Thirty presets, Brush 0 to Brush 29.
fn many_rows(parts: &BrushCardParts, env: &Env) -> Vec<PresetRow<u32>> {
    let paint = BrushRowPaint::new(FacePaint::new(1, |_, _| {}), FacePaint::new(2, |_, _| {}));
    (0..30).map(|n| brush_row(n, format!("Brush {n}"), 0, 12.0, *parts, env, &paint)).collect()
}

/// The app opens the picker from a right press on a canvas under it: the wheel over the picker scrolls its list at
/// once, with no click in it first.
#[trial]
fn the_wheel_scrolls_a_picker_a_right_press_opened_without_a_click() {
    let host = Rc::new(RefCell::new((Some(1u32), BrushPickerState::<u32>::new(), false, POINT)));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut host = shown.borrow_mut();
        let (current, state, open, at) = &mut *host;
        let rect = ui.max_rect();
        #[allow(deprecated)]
        let canvas = ui.interact(rect, egui::Id::new("canvas"), egui::Sense::click_and_drag());
        if canvas.hovered() && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary)) {
            *open = true;
            *at = ui.input(|i| i.pointer.interact_pos()).unwrap_or(POINT);
        }
        let rows = PresetSource::new(state.parts, PresetFns::new(many_rows));
        let picker =
            BrushPresetPicker::new("Brush Preset picker", Bind::bound(current), &rows, Bind::bound(state), OPENED).open_at(*at, Bind::bound(open));
        let _ = picker.show(ui);
    });
    fixture.room(Vec2::new(640.0, 760.0));
    fixture.settle();
    fixture.frame(vec![moved(POINT)]);
    fixture.frame(vec![secondary(POINT, true)]);
    fixture.frame(vec![secondary(POINT, false)]).settle();
    assert!(host.borrow().2, "the right press opens the picker");
    let items = |fixture: &Fixture| {
        let mut all = fixture.by_role(Role::LIST_ITEM);
        all.extend(fixture.by_role(Role::GRID_CELL));
        all.extend(fixture.by_role(Role::TREE_ITEM));
        assert!(!fixture.by_role(Role::DIALOG).is_empty(), "the popover shows");
        all
    };
    let first = items(&fixture).first().expect("the picker shows its presets").rect;
    let over = Pos2::new(first.center().x, first.center().y + 20.0);
    fixture.frame(vec![moved(over)]).settle();
    fixture.frame(vec![wheel(-200.0)]).settle();
    let after = items(&fixture).first().map(|target| target.rect);
    assert!(
        after.is_some_and(|rect| rect.top() < first.top() - 1.0) || after.is_some_and(|rect| rect != first),
        "the wheel scrolls the list: {first:?} -> {after:?}"
    );
}
