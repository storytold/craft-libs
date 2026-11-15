//! The Layer Style and adjustment dialogs on a `DialogHost`: they open there, answer with their value at OK and
//! Cancel at Escape, and hand the app what they ask for.

use std::cell::RefCell;
use std::rc::Rc;

use craft_imaging::layer_style::{EffectId, EffectInstance, EffectKind, LayerStyle, LayerStyleDraft, LayerStylePage, StyleParams};
use craft_imaging::ui::adjust::{BrightnessContrast, BrightnessContrastEditor, HostedAdjustDialog};
use craft_imaging::ui::layer_style::{HostedLayerStyle, LayerStyleAsk};
use craft_ui::dialog::{DialogHost, DialogResult, StackDialog, dialog_id};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};

/// A fixture that runs `host` every pass.
fn fixture(host: &Rc<RefCell<DialogHost>>) -> Fixture {
    let shown = Rc::clone(host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture
}

fn layer_style() -> HostedLayerStyle {
    let mut draft = LayerStyleDraft::new(LayerStyle::default());
    draft.page = LayerStylePage::Styles;
    HostedLayerStyle::new("Layer Style", draft, vec!["R".to_owned(), "G".to_owned(), "B".to_owned()])
}

#[trial]
fn hosted_layer_style_queues_its_asks_and_answers_with_its_style() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut().open(layer_style());
    fixture.idle(10);
    let new_style = fixture.by_label("New Style…").expect("the Styles page offers New Style…");
    fixture.click(&new_style).idle(4);
    let asks = host.borrow_mut().get_mut::<HostedLayerStyle>().map(HostedLayerStyle::take_asks);
    assert_eq!(asks, Some(vec![LayerStyleAsk::NewStyle]), "New Style… waits for the app");

    let shadow = EffectInstance { id: EffectId(1), kind: EffectKind::DropShadow, on: true, params: StyleParams::default() };
    if let Some(dialog) = host.borrow_mut().get_mut::<HostedLayerStyle>() {
        dialog.draft_mut().style.effects.push(shadow.clone());
    }
    let ok = fixture.by_label("OK").expect("the OK button");
    fixture.click(&ok).idle(4);
    let effects = host.borrow_mut().take::<HostedLayerStyle>().and_then(DialogResult::ok).map(|style| style.effects);
    assert_eq!(effects, Some(vec![shadow]), "OK answers with the draft's style, the app's answers included");

    host.borrow_mut().open(layer_style());
    fixture.idle(10).key(Key::Escape, Modifiers::NONE).idle(4);
    assert_eq!(host.borrow_mut().take::<HostedLayerStyle>(), Some(DialogResult::Cancel), "a reopened dialog cancels at Escape");
}

type Adjust = HostedAdjustDialog<BrightnessContrast, &'static str>;

fn brightness_contrast() -> Adjust {
    let value = BrightnessContrast { brightness: 20.0, contrast: -10.0, legacy: false };
    HostedAdjustDialog::new("image.adjustments.brightnessContrast", "Brightness/Contrast", value, |ui, value, cx| {
        BrightnessContrastEditor::new("Brightness/Contrast", craft_ui::contract::Bind::bound(value)).context(cx.context).show(ui);
        Some("shown")
    })
}

#[trial]
fn a_hosted_adjustment_dialog_previews_and_answers_with_its_value() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut().open(brightness_contrast());
    fixture.idle(10);
    assert!(fixture.by_label("Brightness/Contrast").is_some(), "the dialog shows on the host");
    let asks = host.borrow_mut().get_mut::<Adjust>().map(Adjust::take_asks).unwrap_or_default();
    assert!(!asks.is_empty() && asks.iter().all(|ask| *ask == "shown"), "the editor's asks wait for the app");

    let preview = fixture.by_label("Preview").expect("the Preview box");
    fixture.click(&preview).idle(4);
    assert_eq!(host.borrow().get::<Adjust>().map(Adjust::is_previewing), Some(false), "the Preview box turns preview off");

    // The automation path: the value's serde fields, then a refused unknown field.
    let id = dialog_id("image.adjustments.brightnessContrast");
    let ctx = fixture.ctx().clone();
    let set = host.borrow_mut().set_json_field(&ctx, id, "contrast", serde_json::json!(35.0));
    assert_eq!(set, Ok(()), "the value's field takes a number");
    let refused = host.borrow_mut().set_json_field(&ctx, id, "noSuchField", serde_json::json!(1));
    assert!(refused.is_err(), "an unknown field is refused");
    // The view fields the dialog keeps beside its value take the same path and read back.
    for (field, value) in [("channel", serde_json::json!(1)), ("picker", serde_json::json!("gray")), ("preview", serde_json::json!(false))] {
        let set = host.borrow_mut().set_json_field(&ctx, id, field, value.clone());
        assert_eq!(set, Ok(()), "{field} is a field of the dialog");
        let read = host.borrow().get::<Adjust>().and_then(|dialog| dialog.json_fields()).map(|fields| fields[field].clone());
        assert_eq!(read, Some(value), "{field} reads back");
    }
    fixture.idle(4);
    let ok = fixture.by_label("OK").expect("the OK button");
    fixture.click(&ok).idle(4);
    let contrast = host.borrow_mut().take::<Adjust>().and_then(DialogResult::ok).map(|value| value.contrast);
    assert_eq!(contrast, Some(35.0), "OK answers with the value");

    host.borrow_mut().open(brightness_contrast());
    fixture.idle(10).key(Key::Escape, Modifiers::NONE).idle(4);
    assert_eq!(host.borrow_mut().take::<Adjust>(), Some(DialogResult::Cancel), "Escape cancels a reopened dialog");
}
