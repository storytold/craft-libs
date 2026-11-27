//! The parameter model: the spec-string parser, kinds, defaults and fit rules, and the rows' predicates, read-only
//! text and tooltips.

use craft_behavior::{EditEvent, GesturePhase};
use craft_ui::contract::{Bind, LabelScheme, Role};
use craft_ui::dialog::DialogHost;
use craft_ui::patterns::blend_keys::{BlendKeyEntry, BlendShortcut};
use craft_ui::patterns::form_dialogs::{
    DialogFields, FieldValue, HostedParamDialog, NumberUnit, Param, ParamDialog, ParamKind, ParamRows, open_fields, set_color_field, set_param_field,
};
use craft_ui::patterns::presets::{PresetFns, PresetRow, PresetSource};
use craft_ui::runtime::env::{Label, Scale, UnitKind};
use craft_ui::testing::relations::{above, left_of};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberLook;
use craft_ui::value_drag::SliderScale;
use egui::{Key, Modifiers, Vec2};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

const SPEC: &str = r#"{"radius":0.1..1000=1,"method":"spin|zoom"="zoom","monochromatic":bool,"seed":u32=0,"cellSize":px=4,"tint":color=#ff8000,"kernel":int[25],"layer":layer id,"curve":json,"note":text} -> {"x":1}"#;
const KERNEL_CELLS: usize = 25;

/// The registry notation gives the kinds, bounds, defaults and labels the rows show; entries the rows do not edit are
/// left out.
#[trial]
fn the_registry_notation_parses_into_params() {
    let params = Param::parse_list(SPEC);
    let keys: Vec<_> = params.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["radius", "method", "monochromatic", "seed", "cellSize", "tint", "kernel", "note"]);
    assert_eq!(params[0].label.as_str(), "Radius");
    assert_eq!(params[4].label.as_str(), "Cell Size");

    assert_eq!(params[0].kind, ParamKind::range(0.1, 1000.0, 1.0, SliderScale::Log));
    let ParamKind::Choice { options, default, .. } = &params[1].kind else { panic!("a choice") };
    assert_eq!(options.iter().map(|(id, label)| (id.as_str(), label.as_str())).collect::<Vec<_>>(), [("spin", "Spin"), ("zoom", "Zoom")]);
    assert_eq!(default.as_deref(), Some("zoom"));
    assert_eq!(params[3].kind, ParamKind::int(None, None, 0));
    assert_eq!(params[5].kind.default_value(), Some(FieldValue::Color([1.0, 128.0 / 255.0, 0.0])));

    let kernel = params[6].kind.default_value().expect("a grid has a default");
    assert_eq!(kernel, FieldValue::Grid((0..KERNEL_CELLS).map(|i| i64::from(i == KERNEL_CELLS / 2)).collect()));
    assert!(params[6].kind.fits(&kernel));
    assert!(!params[0].kind.fits(&FieldValue::Number(5000.0)));
    assert!(!params[1].kind.fits(&FieldValue::Text("wobble".into())));

    // Bounds with a unit after them, keys with a space, fixed labels, colours with and without a default, the log
    // slider's threshold and a fractional default.
    let spec = r#"{"amount":0..100 %=50,"width":1..4096 px=100,"ratio":1..500,"wide":1..501,"zero":0..5000,"Black Point":bool,"bpc":bool,"plain":color,"tint":color=#ff0000,"centerX":0..1=0.5} (a note about the result)"#;
    let params = Param::parse_list(spec);
    let by = |key: &str| params.iter().find(|p| p.key == key).unwrap_or_else(|| panic!("a row {key}"));
    assert_eq!(params.len(), 10, "the note after the object adds no row");
    assert_eq!(by("amount").kind, ParamKind::range(0.0, 100.0, 50.0, SliderScale::Linear));
    assert_eq!(by("width").kind, ParamKind::range(1.0, 4096.0, 100.0, SliderScale::Log));
    assert_eq!(by("ratio").kind, ParamKind::range(1.0, 500.0, 1.0, SliderScale::Linear), "a ratio of exactly 500 stays linear");
    assert_eq!(by("wide").kind, ParamKind::range(1.0, 501.0, 1.0, SliderScale::Log));
    assert_eq!(by("zero").kind, ParamKind::range(0.0, 5000.0, 0.0, SliderScale::Linear), "a minimum of 0 stays linear");
    assert_eq!(by("Black Point").label.as_str(), "Black Point", "a key with a space is kept");
    assert_eq!(by("bpc").label.as_str(), "Black Point Compensation");
    assert_eq!(by("plain").kind.default_value(), None, "a colour without a default is unset");
    assert_eq!(by("tint").kind.default_value(), Some(FieldValue::Color([1.0, 0.0, 0.0])));
    let centre = by("centerX");
    assert_eq!(centre.kind, ParamKind::range(0.0, 1.0, 0.5, SliderScale::Linear), "a 0..1 range keeps its fractional default");
    assert_eq!(centre.kind.default_value(), Some(FieldValue::Number(0.5)));
}

/// A fixture showing `params` as rows over `values`.
fn rows(params: Vec<Param>, values: &Rc<RefCell<DialogFields>>) -> Fixture {
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = ParamRows::new("Form", &params, Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.settle();
    fixture
}

#[trial]
fn a_hidden_caption_keeps_the_controls_name() {
    let lengths = craft_ui::runtime::env::LengthUnits::new(UnitKind::Points, UnitKind::Points, 72.0);
    let fixture = rows(
        vec![
            Param::number("width", "Width", 100.0, 1.0, 2).length(lengths).hide_caption(),
            Param::new("range", "Range", ParamKind::text("1-3")).hide_caption(),
            Param::new("height", "Height", ParamKind::int(None, None, 100)),
            Param::new("amount", "Amount", ParamKind::range(0.0, 10.0, 5.0, SliderScale::Linear)).hide_caption().resettable(),
        ],
        &Rc::new(RefCell::new(DialogFields::new())),
    );
    let tree = fixture.tree();
    assert!(fixture.by_role(Role::SPIN_BUTTON).iter().any(|target| tree.name_of(target.node) == Some("Width")));
    assert!(fixture.by_role(Role::TEXT_INPUT).iter().any(|target| tree.name_of(target.node) == Some("Range")));
    let texts = fixture.texts();
    let paints = |caption: &str| texts.iter().any(|run| run.text.trim_end_matches(':') == caption);
    assert!(paints("Height"), "ordinary rows still paint their caption");
    assert!(paints("Amount"), "resettable rows keep the caption with the reset dot");
    assert!(!paints("Width") && !paints("Range"), "hidden captions use no paint");
}

/// The opt-in range keeps its numeric value and only opens its slider from the field's arrow.
#[trial]
fn a_popup_slider_opens_from_a_typed_range_row() {
    let params = Param::parse_list(r#"{"scale":1..1000=100}"#);
    let popup = vec![params[0].clone().popup_slider().unit(UnitKind::Percent)];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(popup.clone(), &values);
    assert!(fixture.by_role(Role::SLIDER).is_empty(), "no permanent slider");
    let chevron = fixture.by_label("Slider").expect("the slider arrow");
    let unit = fixture.texts().iter().find(|run| run.text == "%").expect("the percent unit");
    assert!(unit.rect.left() >= chevron.rect.right(), "the unit follows the arrow");
    fixture.click(&chevron).settle();
    let slider = fixture.by_role(Role::SLIDER).into_iter().next().expect("the popup slider");
    fixture.click_at(slider.center()).settle();
    let value = values.borrow().number("scale").unwrap();
    assert!((value - 500.5).abs() <= 4.0, "linear midpoint: {value}");

    assert!(set_param_field(&popup, &mut values.borrow_mut(), "scale", json!("125")).is_err());
    assert!(set_param_field(&popup, &mut values.borrow_mut(), "scale", json!(1001)).is_err());
    set_param_field(&popup, &mut values.borrow_mut(), "scale", json!(125.5)).unwrap();
    assert_eq!(ParamDialog::remembered(&popup, &values.borrow()).number("scale"), Some(125.5));

    let standard = rows(params, &Rc::new(RefCell::new(DialogFields::new())));
    assert_eq!(standard.by_role(Role::SLIDER).len(), 1, "standard range uses its visible slider");
}

#[trial]
fn a_short_popup_slider_can_select_its_midpoint_without_explicit_precision() {
    let param = Param::new("scale", "Scale", ParamKind::range(0.0, 0.001, 0.0, SliderScale::Linear)).popup_slider();
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(vec![param], &values);
    let chevron = fixture.by_label("Slider").expect("the slider arrow");
    fixture.click(&chevron).settle();
    let slider = fixture.by_role(Role::SLIDER).into_iter().next().expect("the popup slider");

    fixture.click_at(slider.center()).settle();

    let value = values.borrow().number("scale").expect("the selected value");
    assert!((value - 0.0005).abs() <= 0.00002, "linear midpoint: {value}");
}

/// A range row with a step moves by it on the slider's arrow keys, snaps a typed value to it, and keeps it when its
/// slider is a popup; a step that is not positive leaves the row as it was.
#[trial]
fn a_range_row_moves_and_snaps_by_its_step() {
    let gain = || Param::new("gain", "Gain", ParamKind::range(-30.0, 30.0, 0.0, SliderScale::Linear));
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(vec![gain().step(0.5)], &values);
    let slider = fixture.by_role(Role::SLIDER).into_iter().next().expect("the slider");
    fixture.click_at(slider.center()).settle();
    assert_eq!(values.borrow().number("gain"), Some(0.0), "the middle of the track is on a step");
    fixture.key(Key::ArrowRight, Modifiers::NONE).settle();
    assert_eq!(values.borrow().number("gain"), Some(0.5), "Right moves one step");

    let field = fixture.by_role(Role::SPIN_BUTTON).into_iter().next().expect("the number field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("3.3");
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(values.borrow().number("gain"), Some(3.5), "a typed value snaps to the step");

    let popup = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(vec![gain().popup_slider().step(0.5)], &popup);
    let field = fixture.by_role(Role::SPIN_BUTTON).into_iter().next().expect("the popup row's field");
    fixture.click(&field).settle();
    fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    assert_eq!(popup.borrow().number("gain"), Some(0.5), "Up on the popup row's field moves one step");

    assert_eq!(gain().step(0.0), gain(), "a zero step is ignored");
    assert_eq!(gain().step(f64::NAN), gain(), "a step that is no number is ignored");
    assert_eq!(Param::new("on", "On", ParamKind::bool(false)).step(0.5), Param::new("on", "On", ParamKind::bool(false)), "other kinds are unchanged");
}

// A row with `visible_when` is there only while its condition holds, and one with `enabled_when` takes input only
// while its condition holds; a tooltip is the description of its row.
#[trial]
fn rows_follow_their_predicates_and_carry_their_tooltip() {
    let source = ParamKind::choice(vec![("file".into(), "File".into()), ("folder".into(), "Folder".into()), ("url".into(), "Link".into())], None);
    let params = vec![
        Param::new("source", "Source", source),
        Param::new("detail", "Detail", ParamKind::int(Some(0), Some(9), 1)).enabled_when_in("source", ["file", "folder"]),
        Param::new("input", "Input Folder", ParamKind::text("")).visible_when("source", "folder"),
        Param::new("custom", "Custom", ParamKind::bool(false)).tooltip("Use your own quality"),
        Param::new("quality", "Quality", ParamKind::int(Some(0), Some(12), 8)).enabled_when("custom", true),
    ];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(params, &values);
    assert!(fixture.by_label("Input Folder").is_none(), "hidden while the source is a file");
    let quality = fixture.by_label("Quality").expect("the quality row");
    assert!(fixture.tree().is_disabled_key(quality.node), "disabled until Custom is on");
    let custom = fixture.by_label("Custom").expect("the custom row");
    assert_eq!(fixture.tree().description_of(custom.node), Some("Use your own quality"));

    fixture.click(&custom).settle();
    let quality = fixture.by_label("Quality").expect("the quality row");
    assert!(!fixture.tree().is_disabled_key(quality.node), "enabled with Custom on");
    values.borrow_mut().set("source", FieldValue::Text("folder".into()));
    fixture.settle();
    assert!(fixture.by_label("Input Folder").is_some(), "shown once the source is a folder");

    // A second condition on the same row (`visible_when_in`) narrows it: it shows only while both hold.
    let params = vec![
        Param::new("source", "Source", ParamKind::choice(vec![("file".into(), "File".into()), ("folder".into(), "Folder".into())], None)),
        Param::new("both", "Both", ParamKind::bool(false)),
        Param::new("extra", "Extra", ParamKind::text("")).visible_when_in("source", ["file", "folder"]).visible_when("both", true),
    ];
    let narrow = Rc::new(RefCell::new(DialogFields::new()));
    let mut narrowed = rows(params, &narrow);
    assert!(narrowed.by_label("Extra").is_none(), "hidden while one of its conditions fails");
    narrow.borrow_mut().set("both", FieldValue::Bool(true));
    narrowed.settle();
    assert!(narrowed.by_label("Extra").is_some(), "shown once every condition holds");

    // A row enabled for two of three values takes input for both and greys out for the third.
    let detail = fixture.by_label("Detail").expect("the detail row");
    assert!(!fixture.tree().is_disabled_key(detail.node), "enabled for the folder source");
    values.borrow_mut().set("source", FieldValue::Text("url".into()));
    fixture.settle();
    let detail = fixture.by_label("Detail").expect("the detail row");
    assert!(fixture.tree().is_disabled_key(detail.node), "disabled for the third value");
}

#[trial]
fn a_row_checks_all_its_enable_conditions_on_screen_and_in_automation() {
    let params = vec![
        Param::new("hyphenate", "Hyphenation", ParamKind::bool(false)),
        Param::new("zone", "Hyphenation Zone", ParamKind::int(Some(0), None, 0))
            .enabled_when("hyphenate", true)
            .and_enabled_when_in("composer", ["singleLine", "standardSingleLine"]),
    ];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(params.clone(), &values);
    for (hyphenate, composer, enabled) in [
        (true, None, false),
        (false, Some("singleLine"), false),
        (true, Some("everyLine"), false),
        (true, Some("singleLine"), true),
        (true, Some("standardSingleLine"), true),
    ] {
        {
            let mut fields = values.borrow_mut();
            fields.set("hyphenate", hyphenate);
            if let Some(composer) = composer {
                fields.set("composer", composer);
            }
        }
        fixture.settle();
        let zone = fixture.by_label("Hyphenation Zone").expect("the row stays visible");
        assert_eq!(!fixture.tree().is_disabled_key(zone.node), enabled, "{hyphenate}, {composer:?}");
        let mut fields = values.borrow().clone();
        assert_eq!(set_param_field(&params, &mut fields, "zone", json!(8)).is_ok(), enabled, "{hyphenate}, {composer:?}");
    }
}

// A row with two `visible_when` conditions is there only while both hold: the second call adds a condition, not
// replaces the first. A `disabled` row takes no input whatever the fields hold, from a click or from automation.
#[trial]
fn a_row_with_two_visible_conditions_needs_both_and_a_disabled_row_takes_no_input() {
    let params = vec![
        Param::new("custom", "Custom", ParamKind::bool(false)),
        Param::new("advanced", "Advanced", ParamKind::bool(false)),
        Param::new("quality", "Quality", ParamKind::int(Some(0), Some(12), 8)).visible_when("custom", true).visible_when("advanced", true),
        Param::new("locked", "Locked", ParamKind::bool(false)).disabled(),
    ];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(params.clone(), &values);
    let click = |fixture: &mut Fixture, label: &str| {
        let target = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} row"));
        fixture.click(&target).settle();
    };
    assert!(fixture.by_label("Quality").is_none(), "hidden while neither condition holds");
    click(&mut fixture, "Custom");
    assert!(fixture.by_label("Quality").is_none(), "the first condition alone keeps the row hidden");
    click(&mut fixture, "Advanced");
    assert!(fixture.by_label("Quality").is_some(), "shown once both hold");
    click(&mut fixture, "Custom");
    assert!(fixture.by_label("Quality").is_none(), "the second condition alone keeps the row hidden");

    let locked = fixture.by_label("Locked").expect("a disabled row shows");
    assert!(fixture.tree().is_disabled_key(locked.node), "a disabled row is a disabled node");
    fixture.click(&locked).settle();
    assert_ne!(values.borrow().bool("locked"), Some(true), "a click on a disabled row changes nothing");
    let mut fields = values.borrow().clone();
    assert!(set_param_field(&params, &mut fields, "locked", json!(true)).is_err(), "automation is refused a disabled row");
    assert_ne!(fields.bool("locked"), Some(true), "the refused value is not set");
}

// A row that resets its dependents removes them when its value changes, from the row or from automation before any
// pass; the value it already holds keeps them.
#[trial]
fn a_row_that_resets_drops_its_dependents_when_its_value_changes() {
    let documents = ParamKind::radio(vec![("0".into(), "First".into()), ("1".into(), "Second".into())], Some("0".into()));
    let params = vec![Param::new("source", "Source", documents).resets(["layer"]), Param::new("layer", "Layer", ParamKind::int(Some(0), None, 0))];
    let mut fields = DialogFields::new();
    set_param_field(&params, &mut fields, "source", json!("0")).expect("the source row takes an option");
    set_param_field(&params, &mut fields, "layer", json!(7)).expect("the layer row takes a number");
    set_param_field(&params, &mut fields, "source", json!("0")).expect("the same option");
    assert_eq!(fields.int("layer"), Some(7), "setting the held value keeps the dependent");
    set_param_field(&params, &mut fields, "source", json!("1")).expect("another option");
    assert!(!fields.contains("layer"), "another value removes the dependent before any pass");

    let values = Rc::new(RefCell::new(DialogFields::from_pairs([("source", FieldValue::Text("0".into())), ("layer", FieldValue::Int(7))])));
    let mut fixture = rows(params, &values);
    let first = fixture.by_label("First").expect("the first option");
    fixture.click(&first).settle();
    assert_eq!(values.borrow().int("layer"), Some(7), "a click on the chosen option keeps the dependent");
    let second = fixture.by_label("Second").expect("the second option");
    fixture.click(&second).settle();
    assert_eq!(values.borrow().text("source"), Some("1"));
    assert_ne!(values.borrow().int("layer"), Some(7), "a click on another option resets the dependent");
}

// A radio row is captioned by its label with its options in a column; with `options_in_row` its options stand side by
// side with no caption, and still pick the value.
#[trial]
fn a_radio_row_in_a_row_stands_its_options_side_by_side_without_a_caption() {
    let layouts =
        || ParamKind::radio(vec![("diagonal".into(), "Diagonal".into()), ("horizontal".into(), "Horizontal".into())], Some("diagonal".into()));
    let painted = |fixture: &Fixture, text: &str| fixture.texts().iter().any(|painted| painted.text == text);

    let values = Rc::new(RefCell::new(DialogFields::new()));
    let fixture = rows(vec![Param::new("layout", "Layout", layouts())], &values);
    let (first, second) = (fixture.by_label("Diagonal").expect("the first option"), fixture.by_label("Horizontal").expect("the second option"));
    assert!(painted(&fixture, "Layout"), "a radio row is captioned by its label");
    assert!(second.rect.top() >= first.rect.bottom(), "the options stand in a column");

    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(vec![Param::new("layout", "Layout", layouts()).options_in_row()], &values);
    let (first, second) = (fixture.by_label("Diagonal").expect("the first option"), fixture.by_label("Horizontal").expect("the second option"));
    assert!(!painted(&fixture, "Layout"), "no caption over the options");
    assert!(second.rect.left() >= first.rect.right(), "the second option stands after the first");
    assert!((second.rect.center().y - first.rect.center().y).abs() < 1.0, "the options share one line");
    fixture.click(&second).settle();
    assert_eq!(values.borrow().text("layout"), Some("horizontal"));
}

// A text row with `field_chars` asks for that many characters instead of the field length: a wide row's field is wider
// than a plain row's, and `0` keeps the field length.
#[trial]
fn a_text_row_takes_the_width_in_characters_it_asks_for() {
    let params = vec![
        Param::new("name", "Name", ParamKind::text("")),
        Param::new("text", "Text", ParamKind::text("CONFIDENTIAL")).field_chars(48),
        Param::new("plain", "Plain", ParamKind::text("")).field_chars(0),
    ];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let fixture = rows(params, &values);
    let width = |label: &str| fixture.by_label(label).unwrap_or_else(|| panic!("the {label} field")).rect.width();
    let (name, text, plain) = (width("Name"), width("Text"), width("Plain"));
    assert!(text > name, "the wide row's field is wider than the field length: {text} against {name}");
    assert!((plain - name).abs() < 0.5, "0 characters keep the field length: {plain} against {name}");
}

// A read-only row shows its text, selectable, and keeps no value.
#[trial]
fn a_read_only_row_shows_its_text_and_keeps_no_value() {
    let params = vec![Param::section("Camera Data"), Param::read_only_text("make", "Make", "Canon")];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let fixture = rows(params, &values);
    let tree = fixture.tree();
    let shown = tree.keys().into_iter().any(|key| tree.value_of(key) == Some("Canon"));
    assert!(shown, "the text is the value of a node");
    assert!(!values.borrow().contains("make"), "the row keeps no value");
}

/// The blend keys work inside the dialog's modal layer, on the first Blend row only: Shift+= steps to the next mode,
/// Shift+Alt+M picks Multiply, and a second Blend row does not move.
#[trial]
fn a_blend_row_steps_and_picks_by_its_keys_inside_the_dialog() {
    let modes = || -> Vec<(BlendKeyEntry<String>, Label)> {
        vec![
            (BlendKeyEntry::new("normal".to_owned(), BlendShortcut::Normal), "Normal".into()),
            (BlendKeyEntry::new("dissolve".to_owned(), BlendShortcut::Dissolve), "Dissolve".into()),
            (BlendKeyEntry::new("multiply".to_owned(), BlendShortcut::Multiply), "Multiply".into()),
        ]
    };
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        // Every frame runs a second pass: a key press steps the mode once, because the pass that takes the press is
        // the only one that applies it.
        if ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("a second pass in the frame");
        }
        let params =
            vec![Param::new("mode", "Mode", ParamKind::blend(modes(), None)), Param::new("other", "Other Mode", ParamKind::blend(modes(), None))];
        let _ = ParamDialog::new("blend", "Blend", params).values(Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.settle();
    let held = |key: &str| values.borrow().text(key).map(str::to_owned);
    assert_eq!(held("mode").as_deref(), Some("normal"), "a Blend row opens on its first mode");

    fixture.key(Key::Equals, Modifiers::SHIFT).settle();
    assert_eq!(held("mode").as_deref(), Some("dissolve"), "Shift+= steps to the next mode");
    fixture.key(Key::M, Modifiers::SHIFT | Modifiers::ALT).settle();
    assert_eq!(held("mode").as_deref(), Some("multiply"), "Shift+Alt+M picks Multiply");
    assert_eq!(held("other").as_deref(), Some("normal"), "a second Blend row does not move");

    // A greyed mode (#13852): the keys step past it and its letter picks nothing; the closed list's Down skips it too.
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let params = vec![Param::new("mode", "Mode", ParamKind::blend(modes(), None)).disabled_options(["dissolve"])];
        let _ = ParamDialog::new("blend", "Blend", params).values(Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.settle();
    let held = || values.borrow().text("mode").map(str::to_owned);
    fixture.key(Key::Equals, Modifiers::SHIFT).settle();
    assert_eq!(held().as_deref(), Some("multiply"), "Shift+= steps past the greyed Dissolve");
    fixture.key(Key::Minus, Modifiers::SHIFT).settle();
    assert_eq!(held().as_deref(), Some("normal"), "Shift+- steps back past it");
    fixture.key(Key::I, Modifiers::SHIFT | Modifiers::ALT).settle();
    assert_eq!(held().as_deref(), Some("normal"), "the greyed mode's letter picks nothing");
    // Tab to the Mode list: the closed list's role types ahead and moves with the arrows.
    for _ in 0..4 {
        if fixture.focused_role().is_some_and(|role| role.type_ahead && role.moves) {
            break;
        }
        fixture.key(Key::Tab, Modifiers::NONE).settle();
    }
    fixture.key(Key::ArrowDown, Modifiers::NONE).settle();
    assert_eq!(held().as_deref(), Some("multiply"), "Down on the closed list skips the greyed mode");
}

#[trial]
fn paired_parameter_fields_15539_keep_their_controls_and_values() {
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let modes = vec![
            (BlendKeyEntry::new("normal".to_owned(), BlendShortcut::Normal), "Normal".into()),
            (BlendKeyEntry::new("multiply".to_owned(), BlendShortcut::Multiply), "Multiply".into()),
        ];
        let params = vec![
            Param::new("name", "Name", ParamKind::text("Layer 1")),
            Param::new("mode", "Mode", ParamKind::blend(modes, Some("normal".into()))).beside_next(),
            Param::new("opacity", "Opacity", ParamKind::number(Some(0.0), Some(100.0), 100.0, 1.0, 0)).enabled_when("mode", "multiply"),
            Param::new("neutral", "Fill with neutral color", ParamKind::bool(false)).enabled_when_in("mode", ["multiply".to_owned()]),
        ];
        let _ = ParamDialog::new("newLayer", "New Layer", params).values(Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.scale(Scale::new(1.25)).room(Vec2::new(1280.0, 800.0)).settle();

    let mode = fixture.by_role(Role::COMBO_BOX).into_iter().find(|target| fixture.tree().name_of(target.node) == Some("Mode")).expect("mode");
    let opacity =
        fixture.by_role(Role::SPIN_BUTTON).into_iter().find(|target| fixture.tree().name_of(target.node) == Some("Opacity")).expect("opacity");
    assert!((mode.center().y - opacity.center().y).abs() < 1.0, "Mode and Opacity share a row");
    assert!(mode.rect.right() <= opacity.rect.left(), "Mode and Opacity have separate hit areas");

    assert!(fixture.tree().is_disabled_key(opacity.node), "Normal has a disabled paired Opacity field");
    fixture.click_at(opacity.center()).settle();
    fixture.type_text("42").settle();
    assert_eq!(values.borrow().number("opacity"), Some(100.0), "a disabled neighbor ignores input");

    fixture.click(&mode).settle();
    let multiply = fixture.by_label("Multiply").expect("the mode option");
    fixture.click(&multiply).settle();
    let opacity = fixture.by_label("Opacity").expect("the paired field remains present");
    assert!(!fixture.tree().is_disabled_key(opacity.node), "the original predicate enables Opacity");
    assert_eq!(values.borrow().text("mode"), Some("multiply"));
    fixture.click(&opacity).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("42").key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(values.borrow().number("opacity"), Some(42.0));
    assert_eq!(values.borrow().text("mode"), Some("multiply"));
}

#[trial]
fn paired_parameter_fields_15539_leave_no_gap_when_both_are_hidden() {
    let before = Param::new("before", "Before", ParamKind::text("one"));
    let after = Param::new("after", "After", ParamKind::text("two"));
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let hidden = rows(
        vec![
            before.clone(),
            Param::new("mode", "Mode", ParamKind::text("Normal")).visible_when("show", true).beside_next(),
            Param::new("opacity", "Opacity", ParamKind::int(Some(0), Some(100), 100)).visible_when("show", true),
            after.clone(),
        ],
        &values,
    );
    let center_y = |fixture: &Fixture, name: &str| fixture.by_label(name).expect(name).center().y;
    let gap = |fixture: &Fixture| center_y(fixture, "After") - center_y(fixture, "Before");
    assert!(hidden.by_label("Mode").is_none() && hidden.by_label("Opacity").is_none());
    let absent = rows(vec![before, after], &Rc::new(RefCell::new(DialogFields::new())));
    assert_eq!(gap(&hidden), gap(&absent), "a hidden pair uses no space");
}

/// `separated_at(index)` draws the framework's list separator before that option of a choice row's open list and
/// between no other two: a separator placed by the wrong index, or one drawn on every option, would split the standard
/// paper sizes from the wrong neighbour.
#[trial]
fn a_choice_row_draws_its_separator_before_the_named_option() {
    let sizes: Vec<(String, Label)> = [("letter", "Letter"), ("legal", "Legal"), ("a4", "A4"), ("custom", "Custom")]
        .into_iter()
        .map(|(id, label)| (id.to_owned(), label.into()))
        .collect();
    let params = vec![Param::new("paper", "Paper Size", ParamKind::choice(sizes, Some("letter".to_owned()))).separated_at(3)];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = ParamRows::new("Page Setup", &params, Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 800.0));
    fixture.settle();
    let field = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the choice field shows");
    fixture.click(&field);
    fixture.idle(3);

    let line_of = |fixture: &Fixture, text: &str| {
        let run = fixture.texts().iter().rev().find(|run| run.text == text).unwrap_or_else(|| panic!("the list shows {text}"));
        run.rect
    };
    let (letter, legal, a4, custom) = (line_of(&fixture, "Letter"), line_of(&fixture, "Legal"), line_of(&fixture, "A4"), line_of(&fixture, "Custom"));
    // The separator is a row of its own in the open list: the step from A4 to Custom is wider than the step between two
    // options with none between them, and those two steps are equal.
    let plain = legal.top() - letter.top();
    assert!((a4.top() - legal.top() - plain).abs() < 0.5, "no separator between Legal and A4: {letter:?} {legal:?} {a4:?}");
    assert!(custom.top() - a4.top() > plain + 1.0, "a separator between A4 and Custom: A4 {a4:?}, Custom {custom:?}, plain step {plain}");
}

/// A pattern row shows the first pattern for a stored id the rows lack without writing it, a pick writes the pattern's
/// id, and a group header takes no pick.
#[trial]
fn a_pattern_row_picks_its_pattern() {
    let rows = PresetSource::new(
        (),
        PresetFns::new(|_, _| {
            vec![
                PresetRow::group("nature".to_owned(), "Nature", 0),
                PresetRow::item("grass".to_owned(), "Grass", 1),
                PresetRow::item("sand".to_owned(), "Sand", 1),
            ]
        }),
    );
    let values = Rc::new(RefCell::new(DialogFields::from_pairs([("fill", "ghost")])));
    let shown = values.clone();
    let params = vec![Param::new("fill", "Fill", ParamKind::pattern(None))];
    let mut fixture = Fixture::new(move |ui| {
        let _ = ParamRows::new("Form", &params, Bind::bound(&mut *shown.borrow_mut())).patterns(&rows).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 800.0));
    fixture.settle();
    let held = || values.borrow().text("fill").map(str::to_owned);
    assert_eq!(held().as_deref(), Some("ghost"), "showing the first pattern writes nothing");

    let field = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the pattern field shows");
    assert_eq!(fixture.tree().value_of(field.node), Some("Grass"), "a stored id the rows lack shows the first pattern");
    fixture.click(&field);
    fixture.idle(3);
    let sand = fixture.by_label("Sand").expect("the popover lists the patterns");
    fixture.click(&sand);
    fixture.idle(3);
    assert_eq!(held().as_deref(), Some("sand"), "a pick writes the pattern's id");

    fixture.click(&field);
    fixture.idle(3);
    let group = fixture.by_label("Nature").expect("the popover lists the group");
    fixture.click(&group);
    fixture.idle(3);
    assert_eq!(held().as_deref(), Some("sand"), "a group header writes nothing");
}

/// A color row with opacity holds four components: its default, an automation set and a picked color carry the
/// opacity, a row without opacity keeps three, and stored values read back in their own shapes.
#[trial]
fn a_color_row_with_opacity_holds_four_components() {
    let params = vec![
        Param::new("fill", "Fill", ParamKind::color_with_alpha(Some([1.0, 0.0, 0.0, 0.5]))),
        Param::new("stroke", "Stroke", ParamKind::color_with_alpha(None)),
        Param::new("tint", "Tint", ParamKind::color(Some([0.0, 0.0, 1.0]))),
    ];
    let mut fields = DialogFields::new();
    open_fields(&params, &mut fields);
    assert_eq!(fields.get("fill"), Some(&FieldValue::Rgba([1.0, 0.0, 0.0, 0.5])), "the default opens with its opacity");
    assert_eq!(fields.get("stroke"), None, "a row without a default stays unset");
    assert_eq!(fields.get("tint"), Some(&FieldValue::Color([0.0, 0.0, 1.0])), "a row without opacity keeps three components");
    assert!(!params[0].accepts("fill", &FieldValue::Color([1.0, 0.0, 0.0])), "a row with opacity refuses three components");
    assert!(!params[2].accepts("tint", &FieldValue::Rgba([0.0, 0.0, 1.0, 0.5])), "a row without opacity refuses four");

    set_param_field(&params, &mut fields, "fill", json!([0.0, 1.0, 0.0, 0.25])).expect("the row takes four components");
    assert_eq!(fields.rgba("fill"), Some([0.0, 1.0, 0.0, 0.25]));
    assert!(set_param_field(&params, &mut fields, "fill", json!([0.0, 1.0, 0.0])).is_err(), "three components are refused");
    assert!(set_param_field(&params, &mut fields, "fill", json!([0.0, 1.0, 0.0, 1.5])).is_err(), "an opacity over 1 is refused");
    assert!(set_param_field(&params, &mut fields, "fill", json!([0.0, 1.0, 0.0, -0.5])).is_err(), "a negative opacity is refused");

    set_color_field(&params, &mut fields, "fill", craft_theme::Srgba::rgb(0.0, 0.0, 1.0));
    assert_eq!(fields.rgba("fill"), Some([0.0, 0.0, 1.0, 0.25]), "a picked color keeps the row's opacity");
    set_color_field(&params, &mut fields, "stroke", craft_theme::Srgba::new(1.0, 1.0, 1.0, 0.1));
    assert_eq!(fields.rgba("stroke"), Some([1.0, 1.0, 1.0, 1.0]), "an unset row takes its starting opacity, not the picker's");

    fields.set("kernel", FieldValue::Grid(vec![1, 0, 0, 1]));
    let stored = serde_json::to_string(&fields).expect("the fields serialize");
    let read: DialogFields = serde_json::from_str(&stored).expect("the fields read back");
    assert_eq!(read, fields, "four components read back as a color with opacity, four integers as a grid");

    let written: DialogFields = serde_json::from_str(r#"{"fill": [1, 0, 0, 1], "stroke": [0, 0, 1, 2]}"#).expect("the map reads");
    assert_eq!(written.rgba("fill"), Some([1.0, 0.0, 0.0, 1.0]), "four integers read as a color with opacity");
    let kept = ParamDialog::remembered(&params, &written);
    assert_eq!(kept.get("fill"), Some(&FieldValue::Rgba([1.0, 0.0, 0.0, 1.0])), "a stored opaque color written as integers is kept");
    assert_eq!(kept.get("stroke"), None, "a stored opacity over 1 is dropped");
}

/// The opacity field beside a color row with opacity sets the alpha and keeps the color, held to 0 to 100 percent;
/// it takes no input while the row has no color, and a row without opacity has no such field.
#[trial]
fn the_opacity_field_of_a_color_row_sets_its_alpha() {
    let params = vec![
        Param::new("fill", "Fill", ParamKind::color_with_alpha(Some([1.0, 0.0, 0.0, 0.5]))),
        Param::new("stroke", "Stroke", ParamKind::color_with_alpha(None)),
        Param::new("tint", "Tint", ParamKind::color(Some([0.0, 0.0, 1.0]))),
    ];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(params, &values);
    let enter = |fixture: &mut Fixture, label: &str, text: &str| {
        let field = fixture.by_label(label).expect("the opacity field");
        fixture.click(&field).settle();
        fixture.key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Tab, Modifiers::NONE).settle();
    };
    enter(&mut fixture, "Fill opacity", "25");
    assert_eq!(values.borrow().rgba("fill"), Some([1.0, 0.0, 0.0, 0.25]), "the typed opacity sets the alpha");
    enter(&mut fixture, "Fill opacity", "150");
    assert_eq!(values.borrow().rgba("fill"), Some([1.0, 0.0, 0.0, 1.0]), "an opacity over 100 percent is held to opaque");
    enter(&mut fixture, "Stroke opacity", "40");
    assert_eq!(values.borrow().get("stroke"), None, "an unset row takes no opacity");
    assert!(fixture.by_label("Tint opacity").is_none(), "a row without opacity has no opacity field");
}

/// Rows that follow one unit choice show the same base value in the chosen unit and store the base value; a typed
/// zero on a row with no lower bound reaches the value.
#[trial]
fn rows_in_a_shared_unit_follow_the_units_row() {
    let units =
        || vec![NumberUnit::new("cm", UnitKind::Centimetres, "Centimeters", 72.0 / 2.54), NumberUnit::new("in", UnitKind::Inches, "Inches", 72.0)];
    let choice = ParamKind::choice(vec![("cm".into(), "Centimeters".into()), ("in".into(), "Inches".into())], Some("cm".into()));
    let params = vec![
        Param::new("units", "Units", choice),
        Param::number_in_units("width", "Width", 72.0, 1.0, 2, "units", units()),
        Param::number_in_units("height", "Height", 144.0, 1.0, 2, "units", units()),
    ];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(params, &values);
    let enter = |fixture: &mut Fixture, label: &str, text: &str| {
        let field = fixture.by_label(label).expect("the number row");
        fixture.click(&field).settle();
        fixture.key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Enter, Modifiers::NONE).settle();
    };
    enter(&mut fixture, "Width", "2.54");
    assert!((values.borrow().number("width").unwrap() - 72.0).abs() < 1e-6, "2.54 cm is 72 points");
    values.borrow_mut().set("units", FieldValue::Text("in".into()));
    fixture.settle();
    assert_eq!(values.borrow().number("height"), Some(144.0), "the unit change keeps the stored value");
    enter(&mut fixture, "Height", "3");
    assert!((values.borrow().number("height").unwrap() - 216.0).abs() < 1e-6, "3 in is 216 points");
    enter(&mut fixture, "Width", "0");
    assert_eq!(values.borrow().number("width"), Some(0.0), "a typed zero reaches the value");
}

/// An optional number row has no value until its Custom box is on; a typed value is held to the bounds, and the value
/// typed before the box went off comes back when it goes on again.
#[trial]
fn an_optional_number_is_held_to_its_bounds_and_remembered_while_off() {
    let params = vec![Param::new("quality", "Quality", ParamKind::optional_number(0.0, 12.0, None, 5.0, 0))];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(params, &values);
    assert_eq!(values.borrow().number("quality"), None, "the default is no value");
    let toggle = |fixture: &mut Fixture| {
        let custom = fixture.by_label("Custom").expect("the Custom check box");
        fixture.click(&custom).settle();
    };
    let enter = |fixture: &mut Fixture, text: &str| {
        let field = fixture.by_label("Quality").expect("the number field");
        fixture.click(&field).settle();
        fixture.key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Tab, Modifiers::NONE).settle();
    };
    toggle(&mut fixture);
    assert_eq!(values.borrow().number("quality"), Some(5.0), "the box on starts from the initial value");
    enter(&mut fixture, "-5");
    assert_eq!(values.borrow().number("quality"), Some(0.0), "a value below the bounds is held to the least");
    enter(&mut fixture, "99");
    assert_eq!(values.borrow().number("quality"), Some(12.0), "a value above the bounds is held to the greatest");
    enter(&mut fixture, "7");
    toggle(&mut fixture);
    assert_eq!(values.borrow().number("quality"), None, "the box off leaves no value");
    toggle(&mut fixture);
    assert_eq!(values.borrow().number("quality"), Some(7.0), "the box on again brings back the value typed before");
}

// A text row with `max_chars` stops typing at its limit, and the host's `set` refuses longer text; the limit counts
// characters, not bytes.
#[trial]
fn a_text_row_with_a_limit_refuses_longer_text() {
    let params = vec![Param::new("code", "Code", ParamKind::text("")).max_chars(4).mono()];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(params.clone(), &values);
    let field = fixture.by_label("Code").expect("the code row has a field");
    fixture.click(&field).settle();
    fixture.type_text("abcdefgh").settle();
    assert_eq!(values.borrow().text("code"), Some("abcd"), "the field stops at the limit");

    assert!(params[0].accepts("code", &FieldValue::Text("abcd".into())), "text at the limit fits");
    assert!(params[0].accepts("code", &FieldValue::Text("éééé".into())), "four characters of two bytes each fit");
    assert!(!params[0].accepts("code", &FieldValue::Text("abcde".into())), "text past the limit is refused");

    let mut fields = DialogFields::new();
    set_param_field(&params, &mut fields, "code", json!("wxyz")).expect("text at the limit is set");
    let refused = set_param_field(&params, &mut fields, "code", json!("wxyz1"));
    assert!(refused.is_err(), "longer text is refused: {refused:?}");
    assert_eq!(fields.text("code"), Some("wxyz"), "the refused text is not set");
}

/// photocraft#15540: a parameter dialog opens with its first field's whole text selected, so typing right away replaces
/// the value: a text row (New Layer's Name) and a slider row's number (Gaussian Blur's Radius).
#[trial]
fn typing_in_a_freshly_opened_parameter_dialog_replaces_its_first_field() {
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let params = vec![
            Param::new("name", "Name", ParamKind::text("Layer 1")),
            Param::new("radius", "Radius", ParamKind::range(0.1, 1000.0, 1.0, SliderScale::Log)),
        ];
        let _ = ParamDialog::new("newLayer", "New Layer", params).values(Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.settle();
    fixture.type_text("Sky").settle();
    assert_eq!(values.borrow().text("name"), Some("Sky"), "typing replaced the name");

    let values = Rc::new(RefCell::new(DialogFields::new()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let params = vec![Param::new("radius", "Radius", ParamKind::range(0.1, 1000.0, 1.0, SliderScale::Log))];
        let _ = ParamDialog::new("gaussianBlur", "Gaussian Blur", params).values(Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.settle();
    fixture.type_text("7").key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(values.borrow().number("radius"), Some(7.0), "typing replaced the radius");
}

/// A fixture showing `params` as rows over `values`, logging the edit events of every pass as `(phase, gesture)`. While
/// `twice` is on, every frame runs a second pass.
fn logged_rows(params: Vec<Param>, values: &Rc<RefCell<DialogFields>>, twice: bool) -> (Fixture, Rc<RefCell<Vec<EditEvent<DialogFields>>>>) {
    let (shown, log) = (values.clone(), Rc::new(RefCell::new(Vec::new())));
    let out = log.clone();
    let mut fixture = Fixture::new(move |ui| {
        if twice && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("a second pass in the frame");
        }
        let shown = ParamRows::new("Form", &params, Bind::bound(&mut *shown.borrow_mut())).show(ui);
        out.borrow_mut().extend(shown.events);
    });
    fixture.settle();
    (fixture, log)
}

// A resettable row's label has a reset dot while the value differs from the default; a double press on the label (the
// dot's gesture) restores the default as one edit, and one on an unchanged row changes nothing.
#[trial]
fn a_resettable_row_shows_its_dot_only_while_changed_and_its_press_restores_the_default() {
    let params = vec![Param::new("radius", "Radius", ParamKind::range(0.0, 100.0, 10.0, SliderScale::Linear)).resettable()];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let (mut fixture, log) = logged_rows(params, &values, false);
    let label_at =
        |fixture: &Fixture| fixture.texts().iter().find(|text| text.text == "Radius").map(|text| text.rect.center()).expect("the row's label");
    let reset = |fixture: &mut Fixture| {
        let at = label_at(fixture);
        fixture.click_at(at);
        fixture.click_at(at).settle();
    };
    reset(&mut fixture);
    assert_eq!(values.borrow().number("radius"), Some(10.0), "a reset of an unchanged row changes nothing");
    assert!(log.borrow().is_empty(), "and files no edit");

    values.borrow_mut().set("radius", FieldValue::Number(40.0));
    fixture.settle();
    reset(&mut fixture);
    assert_eq!(values.borrow().number("radius"), Some(10.0), "the press restores the default");
    assert_eq!(log.borrow().len(), 1, "as one edit: {:?}", log.borrow());
}

/// What a probe row saw: nothing yet, its key unmarked or its key marked mixed.
const UNSEEN: u8 = 0;
const SEEN_PLAIN: u8 = 1;
const SEEN_MIXED: u8 = 2;

/// A custom row that records whether its key was marked mixed when it drew.
fn mixed_probe(seen: &Arc<AtomicU8>) -> Param {
    let seen = Arc::clone(seen);
    Param::custom("probe", "Probe", move |ui, fields, _| {
        seen.store(if fields.is_mixed("probe") { SEEN_MIXED } else { SEEN_PLAIN }, Ordering::Relaxed);
        ui.label("probe")
    })
}

// A key marked mixed shows blank, stays out of the answer and the opening fill; an edit makes it concrete; Reset marks
// it again; a custom row reads the mark from its fields until an edit.
#[trial]
fn a_mixed_key_shows_mixed_stays_out_of_the_answer_and_a_custom_row_sees_it_until_an_edit() {
    let seen = Arc::new(AtomicU8::new(UNSEEN));
    let params = vec![
        Param::number("width", "Width", 640.0, 1.0, 0),
        Param::new("method", "Method", ParamKind::choice(vec![("spin".into(), "Spin".into()), ("zoom".into(), "Zoom".into())], None)),
        mixed_probe(&seen),
    ];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    for key in ["width", "method", "probe"] {
        values.borrow_mut().set_mixed(key);
    }
    values.borrow_mut().snapshot();
    let (mut fixture, _) = logged_rows(params, &values, false);
    assert!(["width", "method"].iter().all(|key| !values.borrow().contains(key)), "the opening fill skips a marked key");
    assert!(
        serde_json::to_value(&*values.borrow()).is_ok_and(|answer| answer.as_object().is_some_and(|map| !map.contains_key("width"))),
        "a marked key is absent from the answer"
    );
    assert_eq!(seen.load(Ordering::Relaxed), SEEN_MIXED, "a custom row reads the mark from its fields");

    let field = fixture.by_label("Width").expect("the width row");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("5").key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(values.borrow().number("width"), Some(5.0), "an edit makes the key concrete");
    assert!(!values.borrow().is_mixed("width"), "and clears its mark");

    values.borrow_mut().set("probe", FieldValue::Int(1));
    fixture.settle();
    assert_eq!(seen.load(Ordering::Relaxed), SEEN_PLAIN, "the custom row sees the mark gone after an edit");

    assert!(values.borrow_mut().reset(), "Reset changes the values");
    assert!(values.borrow().is_mixed("width") && !values.borrow().contains("width"), "Reset marks the key again");
}

// A drag of a slider is one gesture: one begin, updates and one commit with one gesture id, a second pass of every
// frame lifts nothing new, Escape cancels with the whole values from before, and a typed value, which applies as it is
// typed, lifts as the field's one gesture.
#[trial]
fn a_row_drag_lifts_to_one_gesture_and_a_typed_value_to_the_fields_one() {
    let params =
        vec![Param::new("radius", "Radius", ParamKind::range(0.0, 100.0, 10.0, SliderScale::Linear)), Param::number("width", "Width", 640.0, 1.0, 0)];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let (mut fixture, log) = logged_rows(params, &values, true);
    let slider = fixture.by_role(Role::SLIDER).into_iter().next().expect("the radius row has a slider");
    fixture.drag(&slider, Vec2::new(60.0, 0.0)).settle();
    let phases = |log: &Vec<EditEvent<DialogFields>>| log.iter().filter_map(EditEvent::phase).collect::<Vec<_>>();
    let ids: std::collections::BTreeSet<_> = log.borrow().iter().filter_map(|event| event.id().map(|id| id.0)).collect();
    let events = log.borrow();
    let phases_seen = phases(&events);
    let count = |wanted: GesturePhase| phases_seen.iter().filter(|phase| **phase == wanted).count();
    assert_eq!((count(GesturePhase::Begin), count(GesturePhase::Commit)), (1, 1), "one begin and one commit: {phases_seen:?}");
    assert_eq!(ids.len(), 1, "one gesture id");
    assert_ne!(values.borrow().number("radius"), Some(10.0), "the drag moved the radius");
    drop(events);

    log.borrow_mut().clear();
    let before = values.borrow().number("radius");
    fixture.drag_hold_at(slider.rect.center(), Vec2::new(-30.0, 0.0));
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    let cancelled = log.borrow().iter().find(|event| event.phase() == Some(GesturePhase::Cancel)).cloned().expect("Escape cancels the gesture");
    assert_eq!(cancelled.landed().and_then(|fields| fields.number("radius")), before, "the cancel carries the whole values from before");

    log.borrow_mut().clear();
    let field = fixture.by_label("Width").expect("the width row");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("7").key(Key::Enter, Modifiers::NONE).settle();
    let typed = phases(&log.borrow());
    let count = |wanted: GesturePhase| typed.iter().filter(|phase| **phase == wanted).count();
    assert_eq!((count(GesturePhase::Begin), count(GesturePhase::Commit)), (1, 1), "a typed value is one gesture: {:?}", log.borrow());
    assert_eq!(values.borrow().number("width"), Some(7.0), "that lands the typed value");
}

/// The rects of two rows' fields in a form of `params` shown in a room of `room`.
fn placed(params: Vec<Param>, room: Vec2, first: &str, second: &str) -> [egui::Rect; 2] {
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(params, &values);
    fixture.room(room).settle();
    let first = fixture.by_label(first).expect("the first row").rect;
    let second = fixture.by_label(second).expect("the second row").rect;
    [first, second]
}

// A section beside the one before it, and a full-width custom row beside its section's rows, stand side by side with
// room and fall under in a room that ends before the pair does.
#[trial]
fn a_beside_row_sits_beside_its_section_with_room_and_under_it_without() {
    let sections = || {
        vec![
            Param::section("First"),
            Param::number("a", "Alpha", 1.0, 1.0, 0),
            Param::section("Second").beside(),
            Param::number("b", "Beta", 2.0, 1.0, 0),
        ]
    };
    let [alpha, beta] = placed(sections(), Vec2::new(1400.0, 600.0), "Alpha", "Beta");
    assert!(left_of(alpha, beta), "with room the second section stands beside the first");
    let [alpha, beta] = placed(sections(), Vec2::new(beta.max.x - 1.0, 900.0), "Alpha", "Beta");
    assert!(above(alpha, beta), "without room it falls under");

    let picture = || {
        vec![Param::section("Size"), Param::number("a", "Alpha", 1.0, 1.0, 0), Param::custom("pic", "Pic", |ui, _, _| ui.button("Picture")).beside()]
    };
    let [alpha, pic] = placed(picture(), Vec2::new(1400.0, 600.0), "Alpha", "Picture");
    assert!(left_of(alpha, pic), "with room the picture stands beside the rows");
    let [alpha, pic] = placed(picture(), Vec2::new(pic.max.x - 1.0, 900.0), "Alpha", "Picture");
    assert!(above(alpha, pic), "without room it falls under");
}

/// A dialog with one colour row.
fn tinted(fields: &mut DialogFields) -> ParamDialog<'_> {
    ParamDialog::new("tint", "Tint", vec![Param::new("tint", "Tint", ParamKind::color(None))]).values(Bind::bound(fields))
}

// A colour row's press goes to the host's one routing: the host names the top dialog and the request, and its answer is
// written into the row's key in the next pass.
#[trial]
fn a_hosted_dialogs_colour_request_goes_through_the_host_and_its_answer_comes_back() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1200.0, 900.0));
    let id = host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "tint", DialogFields::new(), tinted));
    fixture.idle(6);
    let swatch = fixture.by_label("Tint").expect("the colour row has a swatch");
    fixture.click(&swatch).idle(4);
    let (named, request) = host.borrow_mut().take_color_request().expect("the press filed a request");
    assert_eq!((named, request.key.as_str()), (id, "tint"), "the host names the dialog and the row");
    assert!(host.borrow_mut().take_color_request().is_none(), "the request comes once");

    host.borrow_mut().answer_color(id, "tint", craft_theme::Srgba::new(1.0, 0.0, 0.0, 1.0));
    fixture.idle(4);
    assert_eq!(on_fields(&host, |fields| fields.color("tint")), Some([1.0, 0.0, 0.0]), "the answer is written into the row's key");
}

/// `read` of the open hosted dialog's values.
fn on_fields<T>(host: &Rc<RefCell<DialogHost>>, read: impl FnOnce(&DialogFields) -> Option<T>) -> Option<T> {
    host.borrow().get::<HostedParamDialog>().and_then(|dialog| read(dialog.fields()))
}

#[trial]
fn parameter_dialogs_15539_show_declared_units_and_keep_parsing() {
    let opacity = || Param::new("opacity", "Opacity", ParamKind::number(Some(0.0), Some(100.0), 100.0, 1.0, 0)).unit(UnitKind::Percent);
    let new_layer = vec![Param::new("name", "Name", ParamKind::text("Layer 1")), opacity()];
    let stroke = vec![
        Param::new("copies", "Copies", ParamKind::int(Some(1), Some(5), 1)),
        Param::new("width", "Width", ParamKind::number(Some(1.0), Some(250.0), 1.0, 1.0, 0)).unit(UnitKind::Pixels),
        opacity(),
        Param::new("angle", "Angle", ParamKind::int(Some(0), Some(90), 4)).unit(UnitKind::Degrees),
        Param::new("offset", "Offset", ParamKind::optional_number(0.0, 10.0, Some(2.5), 2.5, 2)).unit(UnitKind::Points),
    ];
    for (title, params, units) in [
        ("New Layer", new_layer, vec![("Opacity", "%", "42%")]),
        ("Stroke", stroke, vec![("Width", "px", "12 px"), ("Opacity", "%", "42%"), ("Angle", "°", "15°"), ("Offset", "pt", "3.25 pt")]),
    ] {
        let values = Rc::new(RefCell::new(DialogFields::new()));
        let shown = values.clone();
        let mut fixture = Fixture::new(move |ui| {
            let _ = ParamDialog::new(title, title, params.clone()).values(Bind::bound(&mut *shown.borrow_mut())).show(ui);
        });
        fixture.room(Vec2::new(1280.0, 800.0)).settle();
        for (label, suffix, _) in &units {
            let field = fixture.by_label(label).expect("the number field");
            let painted = fixture
                .texts()
                .iter()
                .filter(|text| text.text == *suffix && !text.elided && text.clip.contains_rect(text.rect) && field.rect.contains_rect(text.rect))
                .count();
            assert_eq!(painted, 1, "{title}: one visible {suffix} on {label}");
        }
        for (label, _, text) in units {
            let field = fixture.by_label(label).expect("the number field");
            fixture.click(&field).settle();
            fixture.key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Tab, Modifiers::NONE).settle();
        }
        let held = values.borrow();
        assert_eq!(held.number("opacity"), Some(42.0));
        if title == "Stroke" {
            assert_eq!(held.number("width"), Some(12.0));
            assert_eq!(held.int("angle"), Some(15));
            assert_eq!(held.number("offset"), Some(3.25));
            assert_eq!(held.int("copies"), Some(1));
        }
    }
}

/// Unit text shows after the number of number, int, range and popup range rows, in place of the symbol of a row's unit
/// kind when it has one, and the kind still reads typed text; a row in milliseconds shows `ms` and reads `250 ms`.
#[trial]
fn rows_show_unit_text_and_milliseconds() {
    let params = vec![
        Param::number("bitrate", "Bitrate", 6000.0, 50.0, 0).unit_label("kbps"),
        Param::new("keyframes", "Keyframe interval", ParamKind::int(Some(1), Some(20), 2)).unit_label("s"),
        Param::new("offset", "Sync offset", ParamKind::number(Some(0.0), Some(20_000.0), 0.0, 1.0, 0)).unit(UnitKind::Milliseconds),
        Param::new("delay", "Delay", ParamKind::number(Some(0.0), Some(1000.0), 0.0, 1.0, 0)).unit(UnitKind::Milliseconds).unit_label("msec"),
        Param::new("buffer", "Buffer", ParamKind::range(1.0, 512.0, 64.0, SliderScale::Linear)).unit_label("MiB"),
        Param::new("size", "Size", ParamKind::range(1.0, 100.0, 10.0, SliderScale::Linear)).popup_slider().unit_label("GiB"),
    ];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = ParamRows::new("Form", &params, Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.room(Vec2::new(1280.0, 800.0)).settle();
    for unit in ["kbps", "s", "ms", "msec", "MiB", "GiB"] {
        let painted = fixture.texts().iter().filter(|text| text.text == unit && !text.elided && text.clip.contains_rect(text.rect)).count();
        assert_eq!(painted, 1, "one visible {unit}");
    }
    for (label, typed) in [("Bitrate", "8000"), ("Sync offset", "250 ms"), ("Delay", "120ms")] {
        let field = fixture.by_label(label).expect("the number field");
        fixture.click(&field).settle();
        fixture.key(Key::A, Modifiers::COMMAND).type_text(typed).key(Key::Tab, Modifiers::NONE).settle();
    }
    let held = values.borrow();
    assert_eq!(held.number("bitrate"), Some(8000.0));
    assert_eq!(held.number("offset"), Some(250.0), "the milliseconds row reads its spelling");
    assert_eq!(held.number("delay"), Some(120.0), "a row with unit text still reads its kind's spelling");
}

#[trial]
fn length_rows_show_custom_unit_text_without_changing_their_readers() {
    let lengths = craft_ui::runtime::env::LengthUnits::new(UnitKind::Points, UnitKind::Points, 72.0);
    let doubled = craft_behavior::NumberCodec::new(craft_behavior::Reader::new().bare(2.0), 2);
    let params = vec![
        Param::number("plain", "Plain", 0.0, 1.0, 2).length(lengths).unit_label("length label"),
        Param::number("coded", "Coded", 0.0, 1.0, 2).length(lengths).codec(doubled).unit_label("codec label"),
        Param::number("hidden", "Hidden unit", 0.0, 1.0, 2).length(lengths).unit_label(""),
    ];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(params, &values);
    fixture.room(Vec2::new(1280.0, 800.0)).settle();
    for unit in ["length label", "codec label"] {
        assert!(fixture.texts().iter().any(|text| text.text == unit && !text.elided && text.clip.contains_rect(text.rect)), "{unit}");
    }
    assert!(!fixture.texts().iter().any(|text| text.text == "pt" && !text.elided));
    for (label, typed) in [("Plain", "3pt"), ("Coded", "3"), ("Hidden unit", "3pt")] {
        let field = fixture.by_label(label).expect("the length row");
        fixture.click(&field).settle();
        fixture.key(Key::A, Modifiers::COMMAND).type_text(typed).key(Key::Enter, Modifiers::NONE).settle();
    }
    let (plain, coded) = (values.borrow().number("plain"), values.borrow().number("coded"));
    assert_eq!(plain, Some(3.0));
    assert_eq!(values.borrow().number("hidden"), plain);
    assert!(plain.zip(coded).is_some_and(|(plain, coded)| coded == 2.0 * plain), "{plain:?}, {coded:?}");
}

/// A count per inch shows `/in` after its number, apart in the field or in the number's own run in the inline look, and
/// a typed number or an edit of that run is read back as the count.
#[trial]
fn a_count_per_inch_row_shows_per_inch_and_reads_the_count() {
    let detail = vec![Param::new("detail", "Detail", ParamKind::number(Some(0.0), Some(100.0), 10.0, 1.0, 0)).unit(UnitKind::PerInch)];
    for inline in [false, true] {
        let values = Rc::new(RefCell::new(DialogFields::new()));
        let shown = values.clone();
        let params = detail.clone();
        let mut fixture = Fixture::new(move |ui| {
            let _ = ParamDialog::new("Distort", "Distort", params.clone()).values(Bind::bound(&mut *shown.borrow_mut())).show(ui);
        });
        if inline {
            fixture.root_value(NumberLook::Inline);
        }
        fixture.room(Vec2::new(1280.0, 800.0)).settle();
        let field = fixture.by_label("Detail").expect("the number field");
        let runs: Vec<String> = fixture
            .texts()
            .iter()
            .filter(|text| !text.elided && text.clip.contains_rect(text.rect) && field.rect.contains_rect(text.rect))
            .map(|text| text.text.clone())
            .collect();
        let unit = runs.iter().filter(|run| run.ends_with("/in")).count();
        assert_eq!(unit, 1, "inline {inline}: one visible /in on Detail: {runs:?}");
        assert!(runs.iter().any(|run| run.starts_with("10")), "inline {inline}: the count shows: {runs:?}");

        fixture.click(&field).settle();
        if inline {
            // The run holds "10/in": an edit at its start keeps the unit's text, which is read back with the number.
            fixture.key(Key::Home, Modifiers::NONE).key(Key::Delete, Modifiers::NONE).type_text("4");
        } else {
            fixture.key(Key::A, Modifiers::COMMAND).type_text("40");
        }
        fixture.key(Key::Tab, Modifiers::NONE).settle();
        assert_eq!(values.borrow().number("detail"), Some(40.0), "inline {inline}: the edited count commits");
    }
}

/// In the default look, where the field draws `/in` apart, a typed `/in` after the count is read as the unit, and a `/`
/// before a number still divides.
#[trial]
fn a_count_per_inch_row_reads_a_typed_per_inch() {
    let detail = vec![Param::new("detail", "Detail", ParamKind::number(Some(0.0), Some(100.0), 10.0, 1.0, 0)).unit(UnitKind::PerInch)];
    for typed in ["40/in", "40 /IN", "80/2"] {
        let values = Rc::new(RefCell::new(DialogFields::new()));
        let shown = values.clone();
        let params = detail.clone();
        let mut fixture = Fixture::new(move |ui| {
            let _ = ParamDialog::new("Distort", "Distort", params.clone()).values(Bind::bound(&mut *shown.borrow_mut())).show(ui);
        });
        fixture.room(Vec2::new(1280.0, 800.0)).settle();
        let field = fixture.by_label("Detail").expect("the number field");
        fixture.click(&field).settle();
        fixture.key(Key::A, Modifiers::COMMAND).type_text(typed).key(Key::Tab, Modifiers::NONE).settle();
        assert_eq!(values.borrow().number("detail"), Some(40.0), "{typed:?} commits the count");
    }
}

/// Scale's merge: a `{w, h}` object keeps what it leaves out from the held value; anything else is refused.
fn merge_scale(fields: &mut DialogFields, key: &str, value: serde_json::Value) -> Result<(), String> {
    let serde_json::Value::Object(incoming) = value else { return Err(format!("{key} takes an object")) };
    let mut merged = match fields.get(key) {
        Some(FieldValue::Json(serde_json::Value::Object(held))) => held.clone(),
        _ => serde_json::Map::new(),
    };
    merged.extend(incoming.into_iter().filter(|(_, v)| !v.is_null()));
    fields.set(key, FieldValue::Json(merged.into()));
    Ok(())
}

// A custom row's set-time rule writes a value set from outside in place of the default write, from the held values,
// for its key and its companions; its refusal is the answer and leaves the values as they were.
#[trial]
fn a_custom_rows_set_rule_merges_a_set_value_with_the_held_one() {
    let params = vec![
        Param::custom("scale", "Scale", |ui, _, _| ui.label("scale")).companions(["link"]).on_set(merge_scale),
        Param::custom("plain", "Plain", |ui, _, _| ui.label("plain")),
    ];
    let mut fields = DialogFields::new();
    set_param_field(&params, &mut fields, "scale", json!({"w": 50, "h": 50})).expect("an object is taken");
    set_param_field(&params, &mut fields, "scale", json!({"w": 80, "h": null})).expect("a partial object is taken");
    assert_eq!(fields.get("scale"), Some(&FieldValue::Json(json!({"w": 80, "h": 50}))), "the rule kept the held height");
    set_param_field(&params, &mut fields, "link", json!({"on": true})).expect("a companion goes through the rule");
    assert_eq!(fields.get("link"), Some(&FieldValue::Json(json!({"on": true}))));
    let refused = set_param_field(&params, &mut fields, "scale", json!(3));
    assert!(refused.is_err_and(|why| why.contains("takes an object")), "the rule's refusal is the answer");
    assert_eq!(fields.get("scale"), Some(&FieldValue::Json(json!({"w": 80, "h": 50}))), "a refusal changes nothing");
    set_param_field(&params, &mut fields, "plain", json!({"w": 1})).expect("a row without a rule takes the value");
    set_param_field(&params, &mut fields, "plain", json!({"h": 2})).expect("and replaces it");
    assert_eq!(fields.get("plain"), Some(&FieldValue::Json(json!({"h": 2}))), "without a rule the value replaces the held one");
}

// `enabled_above` and `enabled_when_differs` disable a row from the values, for the rows and for automation; turning
// the second switch on disables the constrained row.
#[trial]
fn rows_enabled_above_a_bound_or_while_two_keys_differ_follow_their_keys() {
    let params = vec![
        Param::new("width_on", "Width", ParamKind::bool(false)),
        Param::new("height_on", "Height", ParamKind::bool(false)),
        Param::new("constrain", "Constrain", ParamKind::bool(false)).enabled_when_differs("width_on", "height_on"),
        Param::new("dashes", "Dashes", ParamKind::int(Some(0), None, 0)),
        Param::new("gap", "Gap", ParamKind::number(Some(0.0), None, 1.0, 1.0, 1)).enabled_above("dashes", 0.0),
    ];
    let mut fields = DialogFields::from_pairs([("width_on", FieldValue::Bool(false)), ("height_on", FieldValue::Bool(false))]);
    assert!(set_param_field(&params, &mut fields, "constrain", json!(true)).is_err(), "both off: refused");
    assert!(set_param_field(&params, &mut fields, "gap", json!(2.0)).is_err(), "no dashes: refused");
    set_param_field(&params, &mut fields, "dashes", json!(2)).expect("dashes");
    set_param_field(&params, &mut fields, "gap", json!(2.0)).expect("an int above the bound enables the row");

    let values = Rc::new(RefCell::new(fields));
    let mut fixture = rows(params, &values);
    let disabled = |fixture: &Fixture| fixture.tree().is_disabled_key(fixture.by_label("Constrain").expect("the row").node);
    assert!(disabled(&fixture), "both switches off");
    let width = fixture.by_label("Width").expect("the width switch");
    fixture.click(&width).settle();
    assert!(!disabled(&fixture), "exactly one switch on");
    let height = fixture.by_label("Height").expect("the height switch");
    fixture.click(&height).settle();
    assert!(disabled(&fixture), "turning the second switch on disables the row");
}

// A disabled row shows its `disabled_value` and keeps its own value, which it shows again once enabled.
#[trial]
fn a_disabled_row_shows_its_disabled_value_and_keeps_its_own() {
    let params = vec![
        Param::new("paper", "Paper", ParamKind::bool(true)),
        Param::new("ink", "Ink", ParamKind::bool(false)).enabled_when("paper", false).disabled_value(true),
    ];
    let values = Rc::new(RefCell::new(DialogFields::from_pairs([("paper", FieldValue::Bool(true)), ("ink", FieldValue::Bool(false))])));
    let mut fixture = rows(params, &values);
    let ink = |fixture: &Fixture| {
        let tree = fixture.tree();
        let check = tree.with_role(Role::CHECK_BOX).into_iter().find(|key| tree.name_of(*key) == Some("Ink")).expect("the ink check box");
        tree.toggled_of(check)
    };
    assert_eq!(ink(&fixture), Some(craft_behavior::Mixed::Same(true)), "shown with the derived value while disabled");
    assert_eq!(values.borrow().bool("ink"), Some(false), "its own value stays");
    let paper = fixture.by_label("Paper").expect("the paper switch");
    fixture.click(&paper).settle();
    assert_eq!(ink(&fixture), Some(craft_behavior::Mixed::Same(false)), "enabled, it shows its own value again");
}

// A length row reads typed text with the row's codec when it has one, as a plain number row does.
#[trial]
fn a_length_row_reads_with_its_codec() {
    let lengths = craft_ui::runtime::env::LengthUnits::new(UnitKind::Points, UnitKind::Points, 72.0);
    // A codec that reads a bare number as twice itself.
    let doubled = craft_behavior::NumberCodec::new(craft_behavior::Reader::new().bare(2.0), 2);
    let params = vec![
        Param::number("plain", "Plain", 0.0, 1.0, 2).length(lengths),
        Param::number("coded", "Coded", 0.0, 1.0, 2).length(lengths).codec(doubled),
    ];
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let mut fixture = rows(params, &values);
    for label in ["Plain", "Coded"] {
        let field = fixture.by_label(label).expect("the length row");
        fixture.click(&field).settle();
        fixture.key(Key::A, Modifiers::COMMAND).type_text("3").key(Key::Enter, Modifiers::NONE).settle();
    }
    let (plain, coded) = (values.borrow().number("plain"), values.borrow().number("coded"));
    assert!(plain.zip(coded).is_some_and(|(plain, coded)| coded > plain), "the codec read the typed text: {plain:?} against {coded:?}");
}

/// A row marked `beside_previous` stands on the line of the row before it, where an unmarked one stands below.
#[trial]
fn a_row_beside_the_previous_one_shares_its_line() {
    let top = |beside: bool| {
        let second = Param::number("second", "Second", 1.0, 1.0, 0);
        let params = vec![Param::number("first", "First", 1.0, 1.0, 0), if beside { second.beside_previous() } else { second }];
        let values = Rc::new(RefCell::new(DialogFields::new()));
        let shown = values.clone();
        let mut fixture = Fixture::new(move |ui| {
            let _ = ParamRows::new("Form", &params, Bind::bound(&mut *shown.borrow_mut())).labels(LabelScheme::Inline).show(ui);
        });
        fixture.settle();
        let at = |name: &str| fixture.texts().iter().find(|text| text.text.contains(name)).map(|text| text.rect.center().y);
        at("First").zip(at("Second")).map(|(first, second)| second - first)
    };
    let (stacked, beside) = (top(false), top(true));
    assert!(stacked.is_some_and(|gap| gap > 10.0), "unmarked rows stack: {stacked:?}");
    assert!(beside.is_some_and(|gap| gap.abs() < 3.0), "a beside row shares the line: {beside:?}");
}

/// The texts of a document form (`LabelScheme::Inline`) over `params`, with their rects.
fn inline_texts(params: Vec<Param>) -> Vec<(String, egui::Rect)> {
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = ParamRows::new("Form", &params, Bind::bound(&mut *shown.borrow_mut())).labels(LabelScheme::Inline).show(ui);
    });
    fixture.settle();
    fixture.texts().iter().map(|text| (text.text.clone(), text.rect)).collect()
}

/// The rect of the first text that contains `name`.
fn text_at(texts: &[(String, egui::Rect)], name: &str) -> Option<egui::Rect> {
    texts.iter().find(|(text, _)| text.contains(name)).map(|(_, rect)| *rect)
}

/// Production's Split: a radio row with `stacked_options` stands its options in a column with no caption, also in a
/// document form, and a row marked `under_option` stands under its option, before the next option.
#[trial]
fn stacked_options_nest_each_rows_under_its_option() {
    let ways = vec![("pages".to_owned(), Label::from("Number of pages")), ("size".to_owned(), Label::from("File size"))];
    let params = vec![
        Param::new("mode", "Split by", ParamKind::radio(ways, Some("pages".to_owned()))).stacked_options(),
        Param::number("pages", "Pages per file", 1.0, 1.0, 0).under_option("pages"),
        Param::number("size", "At most", 2.0, 0.1, 1).under_option("size"),
    ];
    let texts = inline_texts(params);
    assert!(text_at(&texts, "Split by").is_none(), "no caption over or beside stacked options: {texts:?}");
    let (pages, per_file, size, at_most) = (
        text_at(&texts, "Number of pages").expect("the first option"),
        text_at(&texts, "Pages per file").expect("the first option's row"),
        text_at(&texts, "File size").expect("the second option"),
        text_at(&texts, "At most").expect("the second option's row"),
    );
    assert!(
        pages.bottom() <= per_file.top() + 1.0 && per_file.bottom() <= size.top() + 1.0,
        "the row stands between its option and the next: {texts:?}"
    );
    assert!(size.bottom() <= at_most.top() + 1.0, "the second row stands under the second option: {texts:?}");
}

/// Production's Background: a row under an option of a row of options stands right after that option on its line,
/// its control alone (no caption), before the next option.
#[trial]
fn a_row_under_an_option_in_a_row_follows_it_with_no_caption() {
    let sources = vec![("colour".to_owned(), Label::from("From colour")), ("file".to_owned(), Label::from("File"))];
    let params = vec![
        Param::new("source", "Source", ParamKind::radio(sources, Some("colour".to_owned()))).options_in_row(),
        Param::number("strength", "Strength", 3.0, 1.0, 0).under_option("colour"),
    ];
    let texts = inline_texts(params);
    assert!(text_at(&texts, "Strength").is_none(), "the nested control shows no caption: {texts:?}");
    let (colour, file) = (text_at(&texts, "From colour").expect("the first option"), text_at(&texts, "File").expect("the second option"));
    let value = texts.iter().find(|(text, _)| text.trim() == "3").map(|(_, rect)| *rect).expect("the nested field shows its value");
    assert!((value.center().y - colour.center().y).abs() < 6.0, "on the options' line: {texts:?}");
    assert!(value.left() > colour.right() && value.right() < file.left(), "between its option and the next: {texts:?}");
}

/// In a document form a group row under a heading of its own name shows the name once (production's Enhance: Pages
/// over the page range options), as a radio row does.
#[trial]
fn a_group_row_under_a_heading_of_its_name_shows_the_name_once() {
    let params = vec![Param::section("Pages"), Param::custom("pages", "Pages", |ui, _, _| ui.label("The options"))];
    let texts = inline_texts(params);
    let names = texts.iter().filter(|(text, _)| text.trim() == "Pages").count();
    assert_eq!(names, 1, "the heading alone names the group: {texts:?}");
    assert!(text_at(&texts, "The options").is_some(), "the group shows: {texts:?}");
}

#[trial]
fn hiding_side_captions_keeps_the_heading_above_a_custom_row() {
    let params = vec![Param::custom("pages", "Pages", |ui, _, _| ui.label("The options")).hide_caption()];
    let texts = inline_texts(params);
    assert!(text_at(&texts, "Pages").unwrap().bottom() <= text_at(&texts, "The options").unwrap().top());
}

#[trial]
fn a_hidden_caption_does_not_indent_an_inline_field_without_a_shared_column() {
    let params = [
        Param::new("first", "A long visible caption", ParamKind::text("first")),
        Param::new("second", "Second", ParamKind::text("second")).hide_caption(),
    ];
    let mut values = DialogFields::new();
    let mut fixture = Fixture::new(move |ui| {
        ParamRows::new("Form", &params, Bind::bound(&mut values)).labels(LabelScheme::Inline).show(ui);
    });
    fixture.settle();
    let name = fixture.texts().iter().find(|text| text.text.contains("A long visible caption")).unwrap();
    let hidden = fixture.by_role(Role::TEXT_INPUT).into_iter().find(|target| fixture.tree().name_of(target.node) == Some("Second")).unwrap();
    assert!((name.rect.left() - hidden.rect.left()).abs() <= 1.0, "the hidden row starts at the leading edge: {hidden:?}");
}

/// A text row with `select_on_focus` selects its whole text when it takes the focus, so typing replaces it (PdfCraft's
/// Rename opens on the name selected); a row without it keeps the text and types at the caret.
#[trial]
fn a_text_row_that_selects_on_focus_is_replaced_by_typing() {
    for (select, expected) in [(true, "Draft"), (false, "")] {
        let param = Param::new("name", "Name", ParamKind::text("Report"));
        let params = vec![if select { param.select_on_focus() } else { param }];
        let values = Rc::new(RefCell::new(DialogFields::new()));
        values.borrow_mut().set("name", FieldValue::Text("Report".into()));
        let mut fixture = rows(params, &values);
        let field = fixture.by_label("Name").expect("the name row has a field");
        fixture.click(&field).settle();
        fixture.type_text("Draft").settle();
        let typed = values.borrow().text("name").map(str::to_owned);
        if select {
            assert_eq!(typed.as_deref(), Some(expected), "typing replaced the selected name");
        } else {
            assert!(typed.as_deref().is_some_and(|text| text.contains("Report") && text.contains("Draft")), "typing kept the name: {typed:?}");
        }
    }
}

/// Production's Opacity row: a short slider and a number field as wide as its value, the row as wide as its content.
/// An unmarked range row's slider takes the room the form offers.
#[trial]
fn a_short_slider_row_is_as_wide_as_its_content() {
    let widths = |short: bool| {
        let row = Param::new("opacity", "Opacity", ParamKind::range(0.0, 100.0, 100.0, Default::default())).decimals(1);
        let params = vec![if short { row.short_slider() } else { row }];
        let values = Rc::new(RefCell::new(DialogFields::new()));
        let shown = values.clone();
        let mut fixture = Fixture::new(move |ui| {
            let _ = ParamRows::new("Form", &params, Bind::bound(&mut *shown.borrow_mut())).labels(LabelScheme::Inline).show(ui);
        });
        fixture.room(egui::vec2(800.0, 400.0));
        fixture.settle();
        let slider = fixture.by_role(Role::SLIDER).first().map(|slider| slider.rect.width());
        let field = fixture.by_role(Role::SPIN_BUTTON).first().map(|field| field.rect.width());
        (slider.unwrap_or(f32::NAN), field.unwrap_or(f32::NAN))
    };
    let ((long, wide), (short, narrow)) = (widths(false), widths(true));
    assert!(long > 300.0, "an unmarked slider fills the row: {long}");
    assert!(short < 120.0 && short > 40.0, "a short slider is about a short field long: {short}");
    assert!(narrow <= wide, "the short row's number field ({narrow}) is no wider than the unmarked one's ({wide})");
}
