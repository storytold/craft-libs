//! The Brush Preset picker's gear: Rename Brush… opens the browser's name field on the current preset, and the new
//! name comes back as the browser's rename.

use craft_draw::ui::brush_presets::{BrushPick, BrushPickerState, BrushPickerValues, BrushPresetPicker, BrushRowPaint, brush_row};
use craft_ui::contract::{Bind, Role};
use craft_ui::patterns::presets::PresetEvent;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
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
