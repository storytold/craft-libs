//! The wheel over a Blend Mode dropdown in Layer Style steps the mode with no click first, on the Blending Options
//! page and on an effect's page, as Photoshop's does and as an adjustment dialog's controls take the wheel.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::GESTURE_GAP;
use craft_imaging::layer_style::{EffectId, EffectInstance, EffectKind, EffectSpec, LayerStyle, LayerStyleDraft, LayerStylePage, StyleValue};
use craft_imaging::ui::layer_style::LayerStyleDialog;
use craft_ui::contract::{Entry, Role};
use craft_ui::testing::access::name;
use craft_ui::testing::pointer::{moved, wheel_lines};
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;

const MODES: [&str; 4] = ["Normal", "Dissolve", "Darken", "Multiply"];

/// A dialog over `draft` with the four `MODES`, applying every edit to the draft it shows.
fn rig(draft: LayerStyleDraft, specs: Vec<EffectSpec>) -> (Fixture, Rc<RefCell<LayerStyleDraft>>) {
    let draft = Rc::new(RefCell::new(draft));
    let shown = draft.clone();
    let channels: Vec<String> = ["R", "G", "B"].map(String::from).to_vec();
    let modes: Vec<Entry<String>> = MODES.iter().map(|m| Entry::new((*m).to_owned(), (*m).to_owned())).collect();
    let mut fixture = Fixture::new(move |ui| {
        let mut draft = shown.borrow_mut();
        let out = LayerStyleDialog::new("Layer Style", &draft, &channels).specs(&specs).blend_modes(&modes).show(ui);
        out.apply_view(&mut draft);
        for event in out.edit {
            if let Some(value) = event.value() {
                draft.style = value.clone();
            }
        }
    });
    fixture.room(Vec2::new(900.0, 1400.0)).settle();
    (fixture, draft)
}

/// One wheel turn of `lines` (negative is down) over the dialog's Blend Mode dropdown, then the pause that ends it.
fn wheel_blend(fixture: &mut Fixture, lines: f32) {
    let select = fixture
        .by_role(Role::COMBO_BOX)
        .into_iter()
        .find(|target| name(fixture.tree(), target.node) == Some("Blend Mode"))
        .expect("a Blend Mode dropdown");
    fixture.frame(vec![moved(select.rect.center())]);
    fixture.frame(vec![wheel_lines(lines)]);
    fixture.advance(GESTURE_GAP * 2.0).idle(3);
}

#[trial]
fn the_wheel_steps_the_layers_blend_mode_without_a_click() {
    let (mut fixture, draft) = rig(LayerStyleDraft::new(LayerStyle::default()), Vec::new());
    assert_eq!(draft.borrow().page, LayerStylePage::Blending);
    assert_eq!(draft.borrow().style.blending.blend, "Normal");
    wheel_blend(&mut fixture, -1.0);
    assert_eq!(draft.borrow().style.blending.blend, "Dissolve", "a notch down steps to the next mode");
    wheel_blend(&mut fixture, 1.0);
    assert_eq!(draft.borrow().style.blending.blend, "Normal", "a notch up steps back");
}

#[trial]
fn the_wheel_steps_an_effects_blend_mode_without_a_click() {
    let spec = EffectSpec::photoshop(EffectKind::DropShadow);
    let mut params = spec.defaults.clone();
    params.set("blend", StyleValue::Text("Darken".into()));
    let effects = vec![EffectInstance { id: EffectId(1), kind: EffectKind::DropShadow, on: true, params }];
    let mut draft = LayerStyleDraft::new(LayerStyle { effects, ..LayerStyle::default() });
    draft.page = LayerStylePage::Effect(EffectId(1));
    let (mut fixture, draft) = rig(draft, vec![spec]);
    let blend = |draft: &Rc<RefCell<LayerStyleDraft>>| draft.borrow().style.effect(EffectId(1)).and_then(|e| e.params.get("blend").cloned());
    wheel_blend(&mut fixture, -1.0);
    assert_eq!(blend(&draft), Some(StyleValue::Text("Multiply".into())), "a notch down steps the shadow's mode");
    wheel_blend(&mut fixture, 1.0);
    assert_eq!(blend(&draft), Some(StyleValue::Text("Darken".into())), "a notch up steps back");
}
