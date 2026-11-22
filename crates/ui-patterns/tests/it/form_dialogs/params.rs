//! The parameter model: the spec-string parser, kinds, defaults and fit rules, and the rows' predicates, read-only
//! text and tooltips.

use craft_ui::contract::{Bind, Role};
use craft_ui::patterns::blend_keys::{BlendKeyEntry, BlendShortcut};
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, NumberUnit, Param, ParamDialog, ParamKind, ParamRows, set_param_field};
use craft_ui::patterns::presets::{PresetFns, PresetRow, PresetSource};
use craft_ui::runtime::env::{Label, Scale, UnitKind};
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
