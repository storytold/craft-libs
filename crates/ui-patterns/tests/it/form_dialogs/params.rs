//! The parameter model: the spec-string parser, kinds, defaults and fit rules, and the rows' predicates, read-only
//! text and tooltips.

use craft_ui::contract::{Bind, Role};
use craft_ui::patterns::blend_keys::{BlendKeyEntry, BlendShortcut};
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, NumberUnit, Param, ParamDialog, ParamKind, ParamRows, set_param_field};
use craft_ui::patterns::presets::PresetRow;
use craft_ui::runtime::env::{Label, UnitKind};
use craft_ui::testing::{Fixture, trial};
use craft_ui::value_drag::SliderScale;
use egui::{Key, Modifiers, Vec2};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

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

    // A row enabled for two of three values takes input for both and greys out for the third.
    let detail = fixture.by_label("Detail").expect("the detail row");
    assert!(!fixture.tree().is_disabled_key(detail.node), "enabled for the folder source");
    values.borrow_mut().set("source", FieldValue::Text("url".into()));
    fixture.settle();
    let detail = fixture.by_label("Detail").expect("the detail row");
    assert!(fixture.tree().is_disabled_key(detail.node), "disabled for the third value");
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
}

/// A pattern row shows the first pattern for a stored id the rows lack without writing it, a pick writes the pattern's
/// id, and a group header takes no pick.
#[trial]
fn a_pattern_row_picks_its_pattern() {
    let rows = vec![
        PresetRow::group("nature".to_owned(), "Nature", 0),
        PresetRow::item("grass".to_owned(), "Grass", 1),
        PresetRow::item("sand".to_owned(), "Sand", 1),
    ];
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
