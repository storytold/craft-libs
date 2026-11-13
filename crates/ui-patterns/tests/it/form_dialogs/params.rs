//! The parameter model: the spec-string parser, kinds, defaults and fit rules, and the rows' predicates, read-only
//! text and tooltips.

use craft_ui::contract::Bind;
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind, ParamRows};
use craft_ui::testing::{Fixture, trial};
use craft_ui::value_drag::SliderScale;
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
    let source = ParamKind::choice(vec![("file".into(), "File".into()), ("folder".into(), "Folder".into())], None);
    let params = vec![
        Param::new("source", "Source", source),
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
