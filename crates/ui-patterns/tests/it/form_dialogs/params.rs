//! The parameter model: the spec-string parser, kinds, defaults and fit rules.

use craft_ui::patterns::form_dialogs::{FieldValue, Param, ParamKind};
use craft_ui::testing::trial;
use craft_ui::value_drag::SliderScale;

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
