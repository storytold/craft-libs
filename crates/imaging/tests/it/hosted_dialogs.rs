//! The Layer Style and adjustment dialogs on a `DialogHost`: they open there, answer with their value at OK and
//! Cancel at Escape, and hand the app what they ask for.

use std::cell::RefCell;
use std::rc::Rc;

use craft_imaging::layer_style::{EffectId, EffectInstance, EffectKind, LayerStyle, LayerStyleDraft, LayerStylePage, StyleParams};
use craft_imaging::ui::adjust::{BrightnessContrast, BrightnessContrastEditor, CurvesPanel, HostedAdjustDialog, ToneChannels, ToneCurves, ToneSpace};
use craft_imaging::ui::curve::Curve;
use craft_imaging::ui::layer_style::{HostedLayerStyle, LayerStyleAsk};
use craft_ui::contract::Role;
use craft_ui::dialog::{DialogHost, DialogResult, HeldOk, StackDialog, dialog_id};
use craft_ui::runtime::env::Label;
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

// The dialog's own edited signal: false at open, after a preview toggle and after a seed; true after an editor change,
// and after a field set of the value.
#[trial]
fn an_adjustment_dialog_counts_editor_changes_and_sets_as_edits_but_not_seeds_or_preview() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    let id = host.borrow_mut().open(brightness_contrast());
    fixture.idle(10);
    let edited = |host: &Rc<RefCell<DialogHost>>| host.borrow().get::<Adjust>().map(Adjust::edited);
    assert_eq!(edited(&host), Some(false), "a dialog opens unedited");
    let preview = fixture.by_label("Preview").expect("the Preview box");
    fixture.click(&preview).idle(4);
    if let Some(dialog) = host.borrow_mut().get_mut::<Adjust>() {
        dialog.seed(BrightnessContrast { brightness: 0.0, contrast: 0.0, legacy: false });
    }
    fixture.idle(4);
    assert_eq!(edited(&host), Some(false), "a preview toggle and a seed are not edits");
    let slider = fixture.by_role(craft_ui::contract::Role::SLIDER).first().cloned().expect("the Brightness slider");
    fixture.drag(&slider, Vec2::new(slider.rect.width() / 4.0, 0.0)).settle();
    assert_eq!(edited(&host), Some(true), "an editor change is an edit");

    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut again = self::fixture(&host);
    host.borrow_mut().open(brightness_contrast());
    again.idle(10);
    let ctx = again.ctx().clone();
    assert_eq!(host.borrow_mut().set_json_field(&ctx, id, "contrast", serde_json::json!(0.0)), Ok(()));
    assert_eq!(edited(&host), Some(true), "a field set of the value is an edit");
}

// A seed after the open moves the Reset baseline: Reset stays disabled after the seed, and after an edit it returns
// the value to the seed, not to the value the dialog opened with.
#[trial]
fn a_seed_moves_the_reset_baseline() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut().open(brightness_contrast());
    fixture.idle(10);
    let seeded = BrightnessContrast { brightness: 5.0, contrast: 5.0, legacy: false };
    if let Some(dialog) = host.borrow_mut().get_mut::<Adjust>() {
        dialog.seed(seeded);
    }
    fixture.idle(4).hold(Modifiers::ALT).idle(2);
    let reset = fixture.by_label("Reset").expect("Alt turns Cancel into Reset");
    assert!(fixture.tree().is_disabled_key(reset.node), "a seed is the baseline: nothing to reset");
    fixture.hold(Modifiers::NONE).idle(2);

    let slider = fixture.by_role(craft_ui::contract::Role::SLIDER).first().cloned().expect("the Brightness slider");
    fixture.drag(&slider, Vec2::new(slider.rect.width() / 4.0, 0.0)).settle();
    let value = |host: &Rc<RefCell<DialogHost>>| host.borrow().get::<Adjust>().map(|dialog| *dialog.value());
    assert_ne!(value(&host), Some(seeded), "the drag edits the value");
    fixture.hold(Modifiers::ALT).idle(2);
    let reset = fixture.by_label("Reset").expect("Alt turns Cancel into Reset");
    assert!(!fixture.tree().is_disabled_key(reset.node), "an edit enables Reset");
    fixture.click(&reset).idle(2);
    fixture.hold(Modifiers::NONE).settle();
    assert_eq!(value(&host), Some(seeded), "Reset returns to the seed");
}

// A dialog given a JSON form lists the form's keys for its value, with the dialog's own fields beside them, and the
// form sees the edited signal.
#[trial]
fn an_adjustment_dialog_with_a_json_form_lists_the_forms_keys() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    let form = |value: &BrightnessContrast, edited: bool| {
        let mut params = serde_json::Map::new();
        params.insert("b".to_owned(), serde_json::json!(value.brightness));
        params.insert("edited".to_owned(), serde_json::json!(edited));
        params
    };
    let id = host.borrow_mut().open(brightness_contrast().json_form(form));
    fixture.idle(10);
    let fields = || host.borrow().get::<Adjust>().and_then(|dialog| dialog.json_fields()).unwrap_or_default();
    let listed = fields();
    assert_eq!((listed["b"].clone(), listed["edited"].clone()), (serde_json::json!(20.0), serde_json::json!(false)));
    assert!(listed.get("brightness").is_none(), "the serde form does not list beside the app's");
    assert_eq!(listed["preview"], serde_json::json!(true), "the dialog's own fields list beside the form");
    let ctx = fixture.ctx().clone();
    assert_eq!(host.borrow_mut().set_json_field(&ctx, id, "brightness", serde_json::json!(5.0)), Ok(()));
    let listed = fields();
    assert_eq!((listed["b"].clone(), listed["edited"].clone()), (serde_json::json!(5.0), serde_json::json!(true)));
}

/// Why the app refuses the held OK.
const REFUSED: &str = "The layer is locked.";

// OK of a held adjustment dialog files its value and waits with OK disabled; a refusal keeps the dialog open with OK
// enabled and the reason shown until the value changes; an accepted OK closes it with the value.
#[trial]
fn a_held_adjustment_ok_refused_stays_open_and_an_accepted_one_closes() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    let id = host.borrow_mut().open(brightness_contrast().held_ok());
    fixture.idle(10);
    let on_dialog = |act: &mut dyn FnMut(&mut Adjust)| host.borrow_mut().get_mut::<Adjust>().map(act);

    let ok = fixture.by_label("OK").expect("the OK button");
    fixture.click(&ok).idle(4);
    let mut filed = None;
    on_dialog(&mut |dialog| filed = dialog.take_ok());
    assert_eq!(filed.map(|value| value.brightness), Some(20.0), "OK files the value");
    assert_eq!(host.borrow().held_state(id), Some(HeldOk::Waiting), "the filed OK waits for the app");
    let ok = fixture.by_label("OK").expect("the OK button");
    assert!(fixture.tree().is_disabled_key(ok.node), "OK waits disabled for the app's answer");

    on_dialog(&mut |dialog| dialog.answer_ok(Err(Label::from(REFUSED))));
    fixture.idle(4);
    assert!(host.borrow().is_open::<Adjust>(), "a refused OK keeps the dialog open");
    assert_eq!(host.borrow().held_state(id), Some(HeldOk::Refused(Label::from(REFUSED))));
    assert!(fixture.by_label(REFUSED).is_some(), "the refusal shows in the dialog");
    let ok = fixture.by_label("OK").expect("the OK button");
    assert!(!fixture.tree().is_disabled_key(ok.node), "a refusal enables OK again");
    on_dialog(&mut |dialog| dialog.value_mut().brightness = 5.0);
    fixture.idle(4);
    assert!(fixture.by_label(REFUSED).is_none(), "a changed value clears the refusal");

    let ctx = fixture.ctx().clone();
    assert!(!host.borrow_mut().confirm_by_id(&ctx, id), "confirm on a held dialog files the value and answers nothing yet");
    on_dialog(&mut |dialog| filed = dialog.take_ok());
    assert_eq!(filed.map(|value| value.brightness), Some(5.0), "confirm files the value as the button does");
    on_dialog(&mut |dialog| dialog.answer_ok(Ok(())));
    fixture.idle(4);
    let answered = host.borrow_mut().take_keyed::<Adjust>().and_then(|(result, _)| result.ok()).map(|value| value.brightness);
    assert_eq!(answered, Some(5.0), "the accepted OK closes the dialog with the held value");
}

/// The Curves value of the dialog trial: three points on the composite curve.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Curves(ToneCurves);

impl Default for Curves {
    fn default() -> Self {
        let mut curves = ToneCurves::identity(&ToneChannels::of(ToneSpace::Rgb, false));
        curves.curves[0] = Curve::smooth(vec![[0.0, 0.0], [128.0, 160.0], [255.0, 255.0]]);
        Curves(curves)
    }
}

type CurvesDialog = HostedAdjustDialog<Curves, &'static str>;

// The dialog sizes its content column from what the editor takes, and the graph is a square picture that shows across
// that column: everything under it (its tone bar, the selected point's Input and Output, the hint) lies inside the
// dialog, not pushed past its edge.
#[trial]
fn the_curves_dialog_shows_the_tone_bar_the_point_fields_and_the_hint() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut().open(CurvesDialog::new("image.adjustments.curves", "Curves", Curves::default(), |ui, value, cx| {
        let mut panel =
            CurvesPanel::new("Curves", craft_ui::contract::Bind::bound(&mut value.0), ToneChannels::of(ToneSpace::Rgb, false)).context(cx.context);
        if let Some(point) = cx.select_point {
            panel = panel.select(point);
        }
        let _ = panel.show(ui);
        None
    }));
    fixture.idle(10);
    if let Some(dialog) = host.borrow_mut().get_mut::<CurvesDialog>() {
        dialog.select_point(1);
    }
    fixture.settle();
    let dialog = fixture.by_role(Role::DIALOG).first().map(|dialog| dialog.rect).expect("the Curves dialog shows");
    let point = fixture.by_label("Point 2 of 3").expect("the middle point");
    let tree = fixture.tree();
    let graph = tree.parent_of_key(point.node).and_then(|graph| tree.bounds_of(graph)).expect("the graph holds its points");
    let input = fixture.by_label("Input").expect("the Input field").rect;
    let output = fixture.by_label("Output").expect("the Output field").rect;
    let hint = fixture.texts().iter().find(|text| text.text.starts_with("Click to add a point")).expect("the hint is painted").rect;
    for (part, rect) in [("the graph and its tone bar", graph), ("Input", input), ("Output", output), ("the hint", hint)] {
        assert!(dialog.contains_rect(rect), "{part} {rect:?} lies inside the dialog {dialog:?}");
    }
    assert!(input.top() >= graph.bottom() && hint.top() >= input.bottom(), "graph, fields, hint top to bottom: {graph:?} {input:?} {hint:?}");

    // The fields read the selected point, and Input moves it.
    let field = |fixture: &Fixture, name: &str| {
        let target = fixture.by_label(name).unwrap_or_else(|| panic!("the {name} field"));
        fixture.tree().value_of(target.node).map(str::to_owned)
    };
    assert_eq!(field(&fixture, "Input").as_deref(), Some("128"), "Input reads the selected point's input");
    assert_eq!(field(&fixture, "Output").as_deref(), Some("160"), "Output reads the selected point's output");
    let input = fixture.by_label("Input").expect("the Input field");
    fixture.click(&input).settle();
    // Tab commits the field; Enter would answer the dialog.
    fixture.key(Key::A, Modifiers::COMMAND).type_text("96");
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    let point = host.borrow().get::<CurvesDialog>().and_then(|dialog| dialog.value().0.curves[0].points().map(|points| points[1]));
    assert_eq!(point, Some([96.0, 160.0]), "setting Input moves the point");
}
