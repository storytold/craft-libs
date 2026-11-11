//! Stories of `ParamRows`.

use super::{DialogFields, ParamRows};
use crate::runtime::requests::PathKind;

use crate::contract::Bind;
use crate::patterns::form_dialogs::{NumberUnit, Param, ParamKind};
use crate::runtime::env::UnitKind;
use crate::testing::{Drive, StoryData, StoryOutput, story};

use egui::Ui;

const SPEC: &str = r#"{"radius":0.1..1000=1,"method":"spin|zoom","smooth":bool,"seed":u32=0,"tint":color=#336699}"#;

/// One row of every kind the rows show, a section, a row that waits for a check box and a row with a unit menu.
fn params() -> Vec<Param> {
    let mut params = Param::parse_list(SPEC);
    params.push(Param::new("angle", "Angle", ParamKind::angle(30.0)));
    params.push(Param::new("centre", "Centre", ParamKind::point([50.0, 50.0])));
    params.push(Param::section("Output"));
    params.push(Param::number("width", "Width", 640.0, 1.0, 0).bounds(Some(1.0), None).unit(UnitKind::Pixels));
    params.push(Param::number_with_units(
        "resolution",
        "Resolution",
        72.0,
        1.0,
        0,
        vec![
            NumberUnit::new("ppi", UnitKind::PixelsPerInch, "pixels/inch", 1.0),
            NumberUnit::new("ppcm", UnitKind::PixelsPerCentimetre, "pixels/cm", 2.54),
        ],
    ));
    params.push(Param::new("quality", "Quality", ParamKind::optional_number(1.0, 100.0, None, 75.0, 0)));
    params.push(Param::new("folder", "Folder", ParamKind::text("")).hint("Choose a folder").browse(PathKind::Folder));
    params.push(Param::new("kernel", "Kernel", ParamKind::grid(9, 0)));
    params.push(Param::new("sharpen", "Sharpen", ParamKind::bool(false)));
    params.push(Param::new("amount", "Amount", ParamKind::range(0.0, 100.0, 50.0, Default::default())).enabled_when("sharpen", true));
    params
}

#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = params();
    let fields = data.value("fields", DialogFields::new());
    ParamRows::new("Filter", &params, Bind::bound(fields)).show(ui).into()
}

#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_read_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = params();
    let fields = data.value("fields", DialogFields::new());
    ParamRows::new("Filter", &params, Bind::bound(fields)).read_only(true).show(ui).into()
}
