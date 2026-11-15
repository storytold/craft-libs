//! Stories of `ParamRows`.

use super::{DialogFields, ParamRows};
use crate::runtime::requests::PathKind;

use crate::contract::Bind;
use crate::patterns::blend_keys::{BlendKeyEntry, BlendShortcut};
use crate::patterns::form_dialogs::{NumberUnit, Param, ParamKind};
use crate::patterns::presets::PresetRow;
use crate::runtime::env::UnitKind;
use crate::testing::{Drive, StoryData, StoryOutput, story};

use egui::Ui;

const SPEC: &str = r#"{"radius":0.1..1000=1,"method":"spin|zoom","smooth":bool,"seed":u32=0,"tint":color=#336699}"#;

/// The patterns the pattern row offers: a group and two patterns in it.
fn patterns() -> Vec<PresetRow<String>> {
    vec![
        PresetRow::group("nature".to_owned(), "Nature", 0),
        PresetRow::item("grass".to_owned(), "Grass", 1),
        PresetRow::item("sand".to_owned(), "Sand", 1),
    ]
}

/// One row of every kind the rows show, a section, a row that waits for a check box, a row with a unit menu, a
/// multi-line text row, a searchable choice, a radio row with a disabled option, a blend row, a pattern row, a files
/// row in folder mode and a section of read-only rows.
fn params() -> Vec<Param> {
    let mut params = Param::parse_list(SPEC);
    let blend = vec![
        (BlendKeyEntry::new("normal".to_owned(), BlendShortcut::Normal), "Normal".into()),
        (BlendKeyEntry::new("dissolve".to_owned(), BlendShortcut::Dissolve), "Dissolve".into()),
        (BlendKeyEntry::new("multiply".to_owned(), BlendShortcut::Multiply), "Multiply".into()),
    ];
    let sizes = || vec![("small".to_owned(), "Small".into()), ("medium".to_owned(), "Medium".into()), ("large".to_owned(), "Large".into())];
    params.push(Param::new("blend", "Blend", ParamKind::blend(blend, None)));
    params.push(Param::new("pattern", "Pattern", ParamKind::pattern(Some("grass".to_owned()))));
    params.push(Param::files("files", "Files").folder_mode(true));
    params.push(Param::new("size", "Size", ParamKind::choice(sizes(), None)).searchable());
    params.push(Param::new("scale", "Scale", ParamKind::radio(sizes(), None)).disabled_options(["large"]));
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
    params.push(Param::new("notes", "Notes", ParamKind::text("")).multiline(3));
    params.push(Param::section("Camera Data"));
    params.push(Param::read_only_text("make", "Make", "Canon"));
    params.push(Param::read_only_text("exposure", "Exposure", "1/250 s at f/8"));
    params
}

#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = params();
    let patterns = patterns();
    let fields = data.value("fields", DialogFields::new());
    ParamRows::new("Filter", &params, Bind::bound(fields)).patterns(&patterns).show(ui).into()
}

#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_read_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = params();
    let patterns = patterns();
    let fields = data.value("fields", DialogFields::new());
    ParamRows::new("Filter", &params, Bind::bound(fields)).patterns(&patterns).read_only(true).show(ui).into()
}
