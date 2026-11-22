//! Stories of `PrintPanel` and `PrintDialog`.

use super::dialog::PrintPreview;
use super::{
    Duplex, PrintDialog, PrintLayout, PrintMarks, PrintMode, PrintPanel, PrintPanelLayout, PrintPresets, PrintSection, PrintSheet, Printer, Settings,
};
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::patterns::form_dialogs::{DialogFields, Param};
use craft_ui::patterns::presets::{PresetError, PresetRow};
use craft_ui::primitives::TextureCache;
use craft_ui::runtime::env::Message;
use craft_ui::testing::pictures::sample_picture;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Rect, Ui, Vec2, pos2};

fn printers() -> Vec<Printer> {
    vec![
        Printer::new("_192_0_2_15").label("192.0.2.15").is_default(true),
        Printer::new("Office_1").label("Office"),
        Printer::new("Office_2").label("Office"),
    ]
}

#[story(PrintPanel<'static>, covers = [], drive = Drive::Never)]
fn print_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Settings { printer: "Office_1".to_owned(), duplex: Some(Duplex::LongEdge), ..Settings::default() };
    let printers = printers();
    PrintPanel::new(data.bind("settings", start), &printers).show(ui).into()
}

#[story(PrintPanel<'static>, covers = [], drive = Drive::Never)]
fn print_panel_discovering(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PrintPanel::new(data.bind("settings", Settings::default()), &[]).discovering(true).show(ui).into()
}

#[story(PrintPanel<'static>, covers = [], drive = Drive::Never)]
fn print_panel_printer_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Settings { printer: "Office_1".to_owned(), ..Settings::default() };
    let printers = printers();
    PrintPanel::new(data.bind("settings", start), &printers).layout(PrintPanelLayout::PrinterOnly).driver(true).show(ui).into()
}

const PICTURE: [usize; 2] = [240, 160];

/// A letter sheet with the image box inset one inch, the printable area and, with `marks`, corner crop and registration
/// marks.
fn sheet(ui: &Ui, marks: bool) -> PrintSheet {
    let id = TextureCache::texture(ui.ctx(), "print-dialog-story", || ColorImage::filled(PICTURE, Color32::from_rgb(0x4a, 0x7f, 0xc1)));
    let picture = SizedTexture::new(id, Vec2::new(PICTURE[0] as f32, PICTURE[1] as f32));
    // design: allow[literal-size/vec2](page and paper sizes are document points, story data)
    let image = Rect::from_min_size(pos2(72.0, 72.0), Vec2::new(468.0, 312.0));
    // design: allow[literal-size/vec2](page and paper sizes are document points, story data)
    let sheet = PrintSheet::new(Vec2::new(612.0, 792.0), image, picture, "468 × 312 pt".into())
        .printable(Rect::from_min_size(pos2(18.0, 18.0), Vec2::new(576.0, 756.0)));
    if marks { sheet.marks(PrintMarks { corner_crop: true, registration: true, ..PrintMarks::default() }) } else { sheet }
}

/// The sheet of the preview story: Letter paper, the image box centred over a sample photo.
fn preview_sheet(ui: &Ui) -> PrintSheet {
    let id = TextureCache::texture(ui.ctx(), "print-preview-story", || sample_picture(1, [240, 320]));
    // design: allow[literal-size/vec2](page and paper sizes are document points, story data)
    let image = Rect::from_center_size(pos2(306.0, 396.0), Vec2::new(360.0, 480.0));
    // design: allow[literal-size/vec2](page and paper sizes are document points, story data)
    PrintSheet::new(Vec2::new(612.0, 792.0), image, SizedTexture::new(id, Vec2::new(240.0, 320.0)), "360 × 480 pt".into())
}

#[story(PrintPreview<'static>, covers = [Default])]
fn print_preview(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sheet = preview_sheet(ui);
    let (position, position_at) = data.value_at("position", sheet.image.min);
    PrintPreview::new("Print preview", &sheet, position_at.bind(position)).show(ui).into()
}

/// The image box hangs over the paper's right edge, a position the host refuses.
#[story(PrintPreview<'static>, covers = [Invalid], drive = Drive::Never)]
fn print_preview_refused(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sheet = preview_sheet(ui);
    let paper = Rect::from_min_size(pos2(0.0, 0.0), sheet.paper);
    let size = sheet.image.size();
    let position = data.bind("position", pos2(400.0, 156.0)).check_with((paper, size), |(paper, size), at| {
        if paper.contains_rect(Rect::from_min_size(*at, *size)) { Validity::Ok } else { Validity::Refuse(Message::new("Off the paper")) }
    });
    PrintPreview::new("Print preview", &sheet, position).show(ui).into()
}

#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Settings { printer: "Office_1".to_owned(), duplex: Some(Duplex::LongEdge), ..Settings::default() };
    let printers = printers();
    let sheet = sheet(ui, true);
    PrintDialog::new(data.bind("settings", start), &printers, Bind::given(DialogFields::default())).sheet(&sheet).ready(true).show(ui).into()
}

/// The artwork outlines of the sections story's document, in document points: the page inset half an inch and the
/// image box of [`sheet`].
const SECTIONS_OUTLINES: [Rect; 2] =
    [Rect::from_min_max(pos2(36.0, 36.0), pos2(576.0, 756.0)), Rect::from_min_max(pos2(72.0, 72.0), pos2(540.0, 384.0))];

/// The sections beside the shown one, with a preset row, a one-line header and a document on three sheets.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_sections(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Settings { printer: "Office_1".to_owned(), ..Settings::default() };
    let printers = printers();
    let sheet = sheet(ui, true).count(3).outlines(SECTIONS_OUTLINES.to_vec());
    let rows = [PresetRow::item("Default".to_owned(), "Default", 0), PresetRow::item("Proof".to_owned(), "Proof", 0)];
    let built_in = ["Default".to_owned()];
    let header = [Param::number("pagesPerSheet", "Pages per sheet", 1.0, 1.0, 0)];
    let sections = [
        PrintSection::new("Position and Size", vec![Param::number("scale", "Scale", 100.0, 1.0, 0)]),
        PrintSection::new("Printing Marks", vec![Param::number("markWeight", "Mark weight", 0.25, 0.25, 2)]),
    ];
    PrintDialog::new(data.bind("settings", start), &printers, Bind::given(DialogFields::default()))
        .layout(PrintLayout::Sections)
        .sheet(&sheet)
        .presets(PrintPresets::new(&rows, Some("Proof".to_owned()), &built_in))
        .header(&header)
        .sections(&sections)
        .driver(true)
        .ready(true)
        .show(ui)
        .into()
}

/// The sections layout while the host has no sheet to show: its message stands where the preview would.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_sections_no_sheet(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let printers = printers();
    let sections = [PrintSection::new("Position and Size", vec![Param::number("scale", "Scale", 100.0, 1.0, 0)])];
    PrintDialog::new(data.bind("settings", Settings::default()), &printers, Bind::given(DialogFields::default()))
        .layout(PrintLayout::Sections)
        .message("The document has no printable artwork")
        .sections(&sections)
        .show(ui)
        .into()
}

/// The preset editor: a name field whose refusal shows under it.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_preset_editor(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sections = [PrintSection::new("Position and Size", vec![Param::number("scale", "Scale", 100.0, 1.0, 0)])];
    let ((settings, settings_at), (name, name_at)) = data.pair_at(("settings", Settings::default()), ("name", "Default".to_owned()));
    PrintDialog::new(settings_at.bind(settings), &[], Bind::given(DialogFields::default()))
        .layout(PrintLayout::Sections)
        .mode(PrintMode::PresetEditor)
        .preset_name(name_at.bind(name))
        .check_name(|name| name.eq_ignore_ascii_case("Default").then_some(PresetError::NameTaken))
        .sections(&sections)
        .show(ui)
        .into()
}

#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_discovering(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sheet = sheet(ui, false);
    PrintDialog::new(data.bind("settings", Settings::default()), &[], Bind::given(DialogFields::default()))
        .sheet(&sheet)
        .discovering(true)
        .show(ui)
        .into()
}

#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_busy(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Settings { printer: "Office_1".to_owned(), ..Settings::default() };
    let printers = printers();
    let sheet = sheet(ui, false);
    PrintDialog::new(data.bind("settings", start), &printers, Bind::given(DialogFields::default())).sheet(&sheet).busy(true).show(ui).into()
}
