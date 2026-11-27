//! Stories of `ParamRows`.

use super::{DialogFields, ParamRows};
use crate::runtime::requests::PathKind;

use crate::contract::Show as _;
use crate::display::Text;
use crate::patterns::blend_keys::{BlendKeyEntry, BlendShortcut};
use crate::patterns::form_dialogs::{NumberUnit, Param, ParamKind};
use crate::patterns::presets::{PresetFns, PresetRow, PresetSource};
use crate::runtime::env::{Env, UnitKind};
use crate::testing::{Drive, StoryData, StoryOutput, story};

use egui::Ui;

const SPEC: &str = r#"{"radius":0.1..1000=1,"method":"spin|zoom","smooth":bool,"seed":u32=0,"tint":color=#336699}"#;

/// The patterns the pattern row offers: a group and two patterns in it.
fn pattern_rows(_: &(), _: &Env) -> Vec<PresetRow<String>> {
    vec![
        PresetRow::group("nature".to_owned(), "Nature", 0),
        PresetRow::item("grass".to_owned(), "Grass", 1),
        PresetRow::item("sand".to_owned(), "Sand", 1),
    ]
}

fn patterns() -> PresetSource<'static, String> {
    PresetSource::new((), PresetFns::new(pattern_rows))
}

/// One row of every kind the rows show, a section, a row that waits for a check box, a range row with a step of a half,
/// a row with a unit menu, a
/// multi-line text row, a text row forty characters wide, a searchable choice, a radio row with a disabled option, a radio row with its options in a row,
/// a blend row, a pattern row, a files row in folder mode, a color row with opacity and a section of read-only rows.
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
    params.push(Param::new("fit", "Fit", ParamKind::radio(sizes(), Some("medium".to_owned()))).options_in_row());
    params.push(Param::new("angle", "Angle", ParamKind::angle(30.0)));
    params.push(Param::new("centre", "Centre", ParamKind::point([50.0, 50.0])));
    params.push(Param::new("shadow", "Shadow", ParamKind::color_with_alpha(Some([0.0, 0.0, 0.0, 0.5]))));
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
    params.push(Param::new("gain", "Gain", ParamKind::range(-30.0, 30.0, 0.0, Default::default())).step(0.5));
    params.push(Param::new("notes", "Notes", ParamKind::text("")).multiline(3));
    params.push(Param::new("caption", "Caption", ParamKind::text("CONFIDENTIAL")).field_chars(40));
    params.push(Param::section("Camera Data"));
    params.push(Param::read_only_text("make", "Make", "Canon"));
    params.push(Param::read_only_text("exposure", "Exposure", "1/250 s at f/8"));
    params
}

#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = params();
    let patterns = patterns();
    let (fields, fields_at) = data.value_at("fields", DialogFields::new());
    ParamRows::new("Filter", &params, fields_at.bind(fields)).patterns(&patterns).show(ui).into()
}

/// Number, int and range rows with unit text no `UnitKind` names (`kbps`, `s`, `MiB`) beside a row in milliseconds.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_unit_labels(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = vec![
        Param::number("bitrate", "Bitrate", 6000.0, 50.0, 0).bounds(Some(100.0), Some(200_000.0)).unit_label("kbps"),
        Param::new("keyframes", "Keyframe interval", ParamKind::int(Some(1), Some(20), 2)).unit_label("s"),
        Param::new("offset", "Sync offset", ParamKind::number(Some(0.0), Some(20_000.0), 0.0, 1.0, 0)).unit(UnitKind::Milliseconds),
        Param::new("buffer", "Buffer", ParamKind::range(1.0, 512.0, 64.0, Default::default())).unit_label("MiB"),
    ];
    let (fields, fields_at) = data.value_at("fields", DialogFields::new());
    ParamRows::new("Output", &params, fields_at.bind(fields)).show(ui).into()
}

/// The rows of `params` over the story's values, which `setup` may prepare once.
fn rows_story(ui: &mut Ui, data: &mut StoryData, params: &[Param], setup: impl FnOnce(&mut DialogFields)) -> StoryOutput {
    if data.present::<DialogFields>("fields").is_none() {
        setup(data.value("fields", DialogFields::new()));
    }
    ParamRows::new("Filter", params, data.bind("fields", DialogFields::new())).show(ui).into()
}

/// Rows with a reset dot after their label while the value differs from the default.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_resettable(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = [
        Param::new("radius", "Radius", ParamKind::range(0.0, 100.0, 10.0, Default::default())).resettable(),
        Param::new("method", "Method", ParamKind::choice(vec![("spin".to_owned(), "Spin".into()), ("zoom".to_owned(), "Zoom".into())], None))
            .resettable(),
        Param::new("smooth", "Smooth", ParamKind::bool(false)).resettable(),
    ];
    rows_story(ui, data, &params, |fields| {
        fields.set("radius", 40.0);
        fields.set("method", "zoom");
    })
}

/// Rows whose keys differ over what is edited: they show blank and stay out of the answer until an edit.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = [
        Param::new("radius", "Radius", ParamKind::range(0.0, 100.0, 10.0, Default::default())),
        Param::number("width", "Width", 640.0, 1.0, 0),
        Param::new("method", "Method", ParamKind::choice(vec![("spin".to_owned(), "Spin".into()), ("zoom".to_owned(), "Zoom".into())], None)),
        Param::new("smooth", "Smooth", ParamKind::bool(false)),
    ];
    rows_story(ui, data, &params, |fields| {
        for key in ["radius", "width", "method", "smooth"] {
            fields.set_mixed(key);
        }
    })
}

/// A group that starts closed: its rows hide while it is closed and keep their values.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_closed_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = [
        Param::number("width", "Width", 640.0, 1.0, 0),
        Param::group("Advanced").initially_open(false),
        Param::number("height", "Height", 480.0, 1.0, 0),
        Param::new("smooth", "Smooth", ParamKind::bool(false)),
    ];
    rows_story(ui, data, &params, |_| {})
}

/// Rows with a help button after the control.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_with_help_link(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = [Param::number("width", "Width", 640.0, 1.0, 0).help_link("help/width"), Param::new("smooth", "Smooth", ParamKind::bool(false))];
    rows_story(ui, data, &params, |_| {})
}

/// Two sections side by side: the second stands beside the first and falls under it when the room is short.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_with_sections_side_by_side(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = [
        Param::section("Size"),
        Param::number("width", "Width", 640.0, 1.0, 0),
        Param::number("height", "Height", 480.0, 1.0, 0),
        Param::section("Colour").beside(),
        Param::new("tint", "Tint", ParamKind::color(Some([0.2, 0.4, 0.6]))),
        Param::new("smooth", "Smooth", ParamKind::bool(false)),
    ];
    rows_story(ui, data, &params, |_| {})
}

/// A section with a check box head, turned off: its rows take no input.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_with_a_checked_section_off(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = [
        Param::section_check("rays", "Rays", false),
        Param::new("count", "Count", ParamKind::range(1.0, 50.0, 12.0, Default::default())),
        Param::new("soft", "Soft", ParamKind::bool(true)),
    ];
    rows_story(ui, data, &params, |_| {})
}

/// A custom row that draws a picture, beside the three rows it belongs to.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_with_a_picture_beside(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = [
        Param::number("width", "Width", 640.0, 1.0, 0),
        Param::number("height", "Height", 480.0, 1.0, 0),
        Param::new("smooth", "Smooth", ParamKind::bool(false)),
        Param::custom("picture", "Picture", |ui, _, _| Text::new("Picture").show(ui)).beside(),
    ];
    rows_story(ui, data, &params, |_| {})
}

/// A radio row whose options carry a detail line.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_with_radio_details(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let kind = ParamKind::radio_with_details(
        vec![
            ("fast".to_owned(), "Fast".into(), "Lower quality, smaller file".into()),
            ("best".to_owned(), "Best".into(), "Full quality, largest file".into()),
        ],
        None,
    );
    let params = [Param::new("quality", "Quality", kind)];
    rows_story(ui, data, &params, |_| {})
}

/// Rows disabled by their values: Black Ink shows checked while Paper is on, Gap takes input only with dashes, and
/// Constrain only while exactly one of Width and Height is on.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_disabled_by_values(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = [
        Param::new("paper", "Simulate Paper", ParamKind::bool(true)),
        Param::new("ink", "Simulate Black Ink", ParamKind::bool(false)).enabled_when("paper", false).disabled_value(true),
        Param::new("dashes", "Dashes", ParamKind::int(Some(0), Some(6), 0)),
        Param::number("gap", "Gap", 2.0, 1.0, 0).enabled_above("dashes", 0.0),
        Param::new("width_on", "Width", ParamKind::bool(true)),
        Param::new("height_on", "Height", ParamKind::bool(false)),
        Param::new("constrain", "Constrain", ParamKind::bool(false)).enabled_when_differs("width_on", "height_on"),
    ];
    rows_story(ui, data, &params, |fields| {
        fields.set("paper", true);
        fields.set("width_on", true);
        fields.set("height_on", false);
    })
}

/// Rows that never take input (`Param::disabled`): shown greyed out whatever the other rows hold.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_enable_off(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = vec![
        Param::new("searchable", "Make Searchable", ParamKind::bool(true)),
        Param::new("engine", "Engine", ParamKind::choice(vec![("builtin".to_owned(), "Built-in".into())], None)).disabled(),
        Param::new("dpi", "Resolution", ParamKind::int(Some(72), Some(600), 300)).disabled(),
    ];
    ParamRows::new("Recognize Text", &params, data.bind("fields", DialogFields::new())).show(ui).into()
}

/// Options with descriptions and a disabled option with its reason, on a Choice row and a Radio row.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_option_descriptions(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let modes = || vec![("fast".to_owned(), "Fast".into()), ("accurate".to_owned(), "Accurate".into()), ("cloud".to_owned(), "Cloud".into())];
    let described = |param: Param| {
        param
            .option_description("fast", "Lower accuracy, finishes in seconds")
            .option_description("accurate", "Best for scanned books and small print")
            .disabled_option("cloud", "Needs a network connection")
    };
    let params = vec![
        described(Param::new("engine", "Engine", ParamKind::choice(modes(), Some("fast".to_owned())))),
        described(Param::new("quality", "Quality", ParamKind::radio(modes(), Some("accurate".to_owned())))),
    ];
    ParamRows::new("Recognize Text", &params, data.bind("fields", DialogFields::new())).show(ui).into()
}

/// A Choice row whose open list has a separator line between the standard paper sizes and the custom one.
#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_choice_separated(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sizes = vec![
        ("letter".to_owned(), "Letter".into()),
        ("legal".to_owned(), "Legal".into()),
        ("a4".to_owned(), "A4".into()),
        ("custom".to_owned(), "Custom".into()),
    ];
    let params = vec![Param::new("paper", "Paper Size", ParamKind::choice(sizes, Some("letter".to_owned()))).separated_at(3)];
    ParamRows::new("Page Setup", &params, data.bind("fields", DialogFields::new())).show(ui).into()
}

#[story(ParamRows<'static>, covers = [], drive = Drive::Never)]
fn param_rows_read_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = params();
    let patterns = patterns();
    let (fields, fields_at) = data.value_at("fields", DialogFields::new());
    ParamRows::new("Filter", &params, fields_at.bind(fields)).patterns(&patterns).read_only(true).show(ui).into()
}
